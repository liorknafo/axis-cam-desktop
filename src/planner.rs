//! The one-button planner: asks the model for a proposal, checks it with the
//! real generators and export checks, and asks for repairs. Nothing from the
//! model is used until it has passed every check.
use crate::{
    analysis::{self, PartReport},
    look,
    mesh::Mesh,
    model::Document,
    openai::{ApiError, Item, MAX_CALLS, Part, Reply, Role, Tool, ToolChoice},
    plan_check::{self, CheckReport},
    planner_tools::{self, ToolState},
    proposal::{self, Problem, Proposal},
};
use std::{future::Future, sync::Arc};

/// Model requests per run: one proposal and up to three corrections.
pub const MAX_REQUESTS: usize = 4;
/// Pictures the model may ask for within one planner request.
pub const MAX_PICTURES_PER_REQUEST: usize = 6;
/// Pictures the model may ask for in a whole run.
pub const MAX_PICTURES_PER_RUN: usize = 20;
/// The most recent pictures sent as images; older ones are sent as their
/// descriptions. The overview counts here.
pub const KEPT_PICTURES: usize = 8;
/// Replies with tool calls one planner request accepts before it fails.
pub const MAX_TOOL_ROUNDS: usize = 16;

/// Anything that can answer a conversation: the OpenAI client, or a script
/// in tests.
pub trait Backend {
    /// One send of the conversation with the planner tools it offers, none
    /// when `tools` is empty, and how the model may use them. With `edit`
    /// the planner changes the current plan, and its reply follows
    /// `proposal::schema_for(true)`.
    fn reply(
        &self,
        messages: &[Item],
        tools: &[Tool],
        choice: ToolChoice,
        edit: bool,
    ) -> impl Future<Output = Result<Reply, ApiError>> + Send;
    /// Whether this run may show the model pictures.
    fn pictures(&self) -> bool {
        false
    }
    /// Whether the run's machine has a rotary A axis, so its proposals
    /// follow `proposal::schema_on` (with the chuck and the A angles), told
    /// once the part report is made; a backend that asks for structured
    /// output sends that schema. The default ignores it.
    fn rotary(&self, _rotary: bool) {}
}
/// The stage a run is in; `n` counts model requests from 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Analyzing,
    /// The first request.
    Asking(usize),
    /// A request after a proposal failed the checks or could not be read.
    Repairing(usize),
    /// The one request about faces a passing plan leaves covered.
    Extending(usize),
    Checking(usize),
    /// The plan passed; the 3D view of the stock is being built.
    Previewing,
}
/// What a run reports as it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A stage starts.
    Stage(Progress),
    /// The outcome of request `n`'s reply: its check, or why it was not read.
    Checked(usize, StepResult),
    /// The overview picture the first request carries.
    Overview(Look),
    /// A picture the model asked for in request `n`, or why none was drawn.
    Looked(usize, Look),
    /// Request `n` was refused and is sent again without this.
    Dropped(usize, Dropped),
}
/// A picture as the run reports it: what it shows (or why it could not be
/// drawn, with no `png`) and the model's reason for asking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look {
    pub description: String,
    pub purpose: String,
    pub png: Option<Arc<Vec<u8>>>,
}
/// What a run stops sending after OpenAI refused a request carrying it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dropped {
    /// Every planner tool; the pictures already sent stay.
    Tool,
    /// Every picture: each is sent as its description.
    Pictures,
}
/// How a reply fared: the check passed, failed with these problems, or the
/// reply was not a proposal at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepResult {
    Passed,
    /// The first `SHOWN_PROBLEMS` problems, one line each, and how many more.
    Problems {
        shown: Vec<String>,
        more: usize,
    },
    /// The parse error, one line of at most 300 characters.
    Unreadable(String),
}
/// Problems a check result lists before saying how many more there were.
const SHOWN_PROBLEMS: usize = 5;
impl StepResult {
    fn of(check: &CheckReport) -> Self {
        if check.passed {
            return Self::Passed;
        }
        Self::Problems {
            shown: check
                .problems
                .iter()
                .take(SHOWN_PROBLEMS)
                .map(Problem::line)
                .collect(),
            more: check.problems.len().saturating_sub(SHOWN_PROBLEMS),
        }
    }
}
/// Counts the blocking stages still running, including those of runs that
/// were cancelled: blocking work cannot be interrupted.
#[derive(Debug, Clone, Default)]
pub struct Busy(std::sync::Arc<std::sync::atomic::AtomicUsize>);
/// One running blocking stage; it stops counting when dropped.
pub struct BusyGuard(std::sync::Arc<std::sync::atomic::AtomicUsize>);
impl Busy {
    pub fn running(&self) -> usize {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub fn enter(&self) -> BusyGuard {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        BusyGuard(self.0.clone())
    }
}
impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}
/// Runs `work` on the blocking pool, counted by `busy` until the work itself
/// ends, even when the future waiting for it is dropped.
pub async fn blocking<T: Send + 'static>(
    busy: &Busy,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, tokio::task::JoinError> {
    let guard = busy.enter();
    tokio::task::spawn_blocking(move || {
        let _guard = guard;
        work()
    })
    .await
}
/// A proposal that passed every check, and the job it builds.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub proposal: Proposal,
    pub check: CheckReport,
    pub job: Document,
    pub report: PartReport,
    /// Model requests made in the run.
    pub requests: usize,
    /// The request whose proposal this is.
    pub chosen: usize,
    /// Warnings and unmachined entries left out of `proposal` by `tidy`.
    pub more_warnings: usize,
    pub more_unmachined: usize,
    /// The error of a follow-up request that failed after this plan passed.
    pub note: Option<ApiError>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub message: String,
    /// The last proposal's problems, when the model answered but never passed.
    pub problems: Vec<Problem>,
    pub api: Option<ApiError>,
    pub requests: usize,
}
/// One exchange of the planner chat: what the user asked, and what the
/// planner answered (its summary of the plan, or why it found none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub request: String,
    pub answer: String,
}
/// What the user asks the planner in the chat, and the exchanges before it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chat {
    pub turns: Vec<Turn>,
    pub request: String,
}
/// The most of one request, or of one earlier answer, the model is sent.
pub const MAX_REQUEST_CHARS: usize = 2000;
/// The earlier exchanges of the chat the model is sent, the latest.
pub const KEPT_TURNS: usize = 8;
impl Chat {
    /// The messages after the part report and the overview: the earlier
    /// exchanges, then the request, with the plan to change (`current`, the
    /// JSON of `proposal::describe`) when there is one. None without a
    /// request.
    fn messages(&self, current: Option<&str>) -> Vec<Item> {
        let request = proposal::clip(&self.request, MAX_REQUEST_CHARS);
        if request.is_empty() {
            return vec![];
        }
        let mut items = vec![];
        let skip = self.turns.len().saturating_sub(KEPT_TURNS);
        for turn in &self.turns[skip..] {
            let asked = proposal::clip(&turn.request, MAX_REQUEST_CHARS);
            let answer = proposal::clip(&turn.answer, MAX_REQUEST_CHARS);
            items.push(Item::text(
                Role::User,
                format!("Earlier the user asked: {asked}"),
            ));
            items.push(Item::text(
                Role::Assistant,
                if answer.is_empty() {
                    "(no answer)".into()
                } else {
                    answer
                },
            ));
        }
        items.push(Item::text(
            Role::User,
            match current {
                Some(current) => format!("The job's current plan: {current}\n\nThe user asks for this change: {request}"),
                None => format!("Plan the job, and follow this request of the user where the rules allow it: {request}"),
            },
        ));
        items
    }
}
/// Whether a run with `chat` changes the current plan of `doc` rather than
/// planning from scratch: the user asked something, and the job has an
/// operation.
pub fn editing(doc: &Document, chat: &Chat) -> bool {
    !proposal::clip(&chat.request, MAX_REQUEST_CHARS).is_empty()
        && doc.setups.iter().any(|s| !s.operations.is_empty())
}
/// The section the instructions end with when the planner changes the
/// current plan.
const EDITING: &str = "

CHANGING THE CURRENT PLAN
- The job already has a plan, sent as `current_plan` in the format of your reply, and the user asks for a change. Reply with the whole revised plan: every setup and operation it is to have, in order.
- Each setup and operation of `current_plan` carries `from`, its number there. Copy `from` onto every setup and operation you keep or change, and set it to null on a new one. Leave out what is to be removed.
- Change only what the user asks for, and what the checks then require. Copy every other setup and operation as it is, with the same values, names and order: the program keeps an operation you did not change exactly as the user left it, with settings the format does not show, and writes only the values you changed. Fields `current_plan` leaves null stay null unless you change them.
- An operation keeps its `from` only in the setup it came from, and only while that setup keeps its `from` and orientation; to move one to another setup, add it there with `from` null. Each `from` may be used once. Setup 1 stays first, with `from` 1.
- `kept_operations` lists operations of the current plan that you cannot see or change (strategies the format does not offer). They stay in their setups at their place: keep those setups, with their `from` and orientation, and do not count them against the limits.
- Say in `summary`, in a sentence or two for the user, what you changed. When the change cannot be made with these tools and strategies, reply with the current plan unchanged and say why in `summary`.
- The earlier messages of the chat, if any, are for context only: `current_plan` is the job as it stands now.";
/// The part report's `rotary` bullet, on a machine with a rotary A axis.
const ROTARY_PART: &str =
    "- `rotary`, present when the machine has a rotary A axis: see ROTARY A AXIS.\n";
/// The ROTARY A AXIS section, with its blank line after it.
const ROTARY: &str = "ROTARY A AXIS
- `rotary` in the part report is the machine's A axis, a chuck on the table that turns the stock about X: `centre_height` is the A centreline above the table, `max_diameter` the widest stock (across its corners) and `max_length` the longest (from the chuck's face) that fits, `a_min` and `a_max` the A limits in degrees, `grip_min` and `grip_max` what the jaws grip (a round bar's diameter; a box's thinner side up to its diagonal), and `centreline` says whether the A centreline was `measured` by the operator or is only `assumed`. When it is `assumed`, add a warning that the centreline must be probed on the Control page before cutting.
- Hold the stock in the chuck instead of a vise when the part is long and slim, or has features on several sides that A can turn to instead of re-clamping. Then give `chuck`, otherwise null: `stock` is `Round bar` (give `diameter`) or `Box` (give `width` across Y and `height` in Z), `length` long along X; `jaws` `3-jaw` for a round bar and `4-jaw` for a box; `stickout` is how much of the stock reaches out of the jaws, `tailstock` is `Retracted`, `Supports the stock` or `Off the table`; `free_end` is the stock left beyond the part's free end (null: 2 mm). The program turns setup 1 to its `orientation`, which may be any of the six, and places the part on the stock's axis in the stick-out: the part must lie within the stick-out and the jaws must grip at least half their length of the stock, so cut only the stick-out. The report's orientations and stock margins are made for the project's own stock; the check report's `stocks` have the chuck's. Setup 1's `origin` is `A axis at stock end` or `A axis at chuck face` (X0 on the stock's free end face, or on the chuck's face; Y0 Z0 on the A centreline).
- A later setup with `a_angles` (degrees, within `a_min` to `a_max`, such as [0, 90, 180, -90] on a -180 to 180 range) is not re-clamped: A turns the stock in setup 1's chuck, which it keeps along with its work zero. Every operation of that setup runs at each of its angles in turn, on the stock the angles before left. Its `orientation` is the part axis that faces up at its first angle; a different one is refused, with the right one. Positive A turns +Y towards +Z (the right-hand rule about +X), so with `+Z` up in setup 1 the first angle 90 brings `+Y` up, 180 brings `-Z` up and -90 brings `-Y` up (0 keeps `+Z`; 270, where the A range reaches it, is the same as -90). Its `origin` is ignored and its `holder` is null; it uses no work offset of its own. Use it instead of a flip: when the same operations are right for every angle (an outside Contour all round, a Parallel or Waterline finish) list the angles in one setup and keep `surfaces` empty; to turn the bar into a square (or any prism) face its sides: one Face in a setup of all the angles, whose `surfaces` are the top plane's IDs of every orientation those angles bring up (+Z, +Y, -Z and -Y for [0, 90, 180, 270] with `+Z` up in setup 1), which the program faces at each angle where it is on top, with the tool's centre out to each face's edges (`facing_area` and the stock's `stock_margin` do not apply). Size the chuck's box so that the faces leave 2 mm or more to cut, and give the part a shoulder to the bar: the part's end face towards the jaws is left covered, so list it in `unmachined` as joined to the bar, never part it off the bar there (the program refuses a loose piece, a web under 3 mm or too weak for the cut, and a part-off the jaws do not hold); when each side needs its own operations make one setup per side with a single angle. Give a setup after the last indexed one `a_angles` null to clamp again (usually not needed).
- The program refuses a plan that does not fit, with the check's message and the stage `export` or `workholding`, by setup and operation: the cut's depth below the Z travel and the retract above it (turning A needs room over everything that swings, the stock, the part and the jaws), X Y and Z travel, the A range, the stock too big for the unit or the jaws, and the tool or its holder within `gap` of the chuck, its jaws, the rotary head and the tailstock, at each angle and while A turns. Answer those with a smaller or shorter stock, a shorter stick-out, a shorter tool or `Off the table`, a lower `clearance`, or leave the feature in `unmachined`. Check a draft with `check_plan`, as for the other setups.

";
/// The PLANNING THE HOLDING section, with its blank line after it: how the
/// planner is to hold every setup, which stock it starts from and what a holder
/// needs of it. With `tools`, a last rule names the planner tools that test a
/// draft's holding; they are named nowhere else while they are not offered.
fn holding_rules(tools: bool) -> String {
    let tools = if tools {
        "\n- Before you answer, check a draft with `check_plan`: it reports the problems and the stock before each setup. Use `stock_after` to see what the earlier setups left of a setup's stock and `check_holding` to test a library entry on it, and decide the order of the setups and each `holder` with them."
    } else {
        ""
    };
    format!("PLANNING THE HOLDING
- Decide, before you plan any operation, how every setup is held and what stock it starts from. A later setup starts from the stock the earlier setups left, not from the whole block: a faced top makes the flipped stock shorter, and cut sides make it narrower. A holder that does not fit that stock, or has nothing to grip on it, makes the plan fail.
- `workholding_library` lists the vises and clamps you may choose from, by `number` from 1, with their sizes in mm and no names. A `Vise` has `jaw_width`, `jaw_height`, `jaw_thickness`, `max_opening` (the widest stock it opens to across its jaws) and `grip` (how far up from the stock's bottom the jaws hold it). A `Clamp` has `length`, `width`, `height` and `bite` (how far it reaches onto the stock top). A round bar's holders: a `V-block` (`angle`, `v_width`, `width`, `height`, `length`, `clamp_width`, `clamp_height`) holds a bar lying along X, `Soft jaws` (`bore`, the diameter they are bored to, and a vise's sizes) one lying or standing, a `Collet block` (`er`, `block`, `height`) one standing; choose them only for a round bar.
- Each setup's `holder` is a `number` from `workholding_library`, or null. Setup 1 is held by the user's own entry, whose `number` is given in the report's `workholding` (which is absent when setup 1 models no vise or clamps): its `holder` is null or that `number` and cannot be changed. For a later setup null means the same holder as setup 1 when setup 1 has an entry, and no modelled holding when it has none; choose a `number` when the stock it starts from suits another holder better, for example clamps when the earlier setups took the material a vise would grip, or a vise that opens wider. The program lays the chosen holder out (a vise's axis and grip, the clamps' sides and positions) for the setup's toolpaths: you choose only the entry.
- A vise closes on the material in its grip band, from the stock's bottom up to the grip: across the jaws, the stock is as wide as that band. Walls an earlier setup cut through the whole band, such as walls an outside Contour finished, can be gripped, even under stock left wider above them; sides cut through only part of the band cannot, as the jaws then close on the wider stock with part of their face on air.
- A vise needs, in the stock the setup starts from: material at both jaws from the bottom up to the grip (at least 60 % of each jaw's contact face), material under the band to rest on (at least half of it), stock at least 1 mm above the grip, and a band no wider across the jaws than `max_opening`. Clamps need material under each clamp's bite (at least 60 % of it). A part cut free of the material the holder grips cannot be held either.
- Plan the earlier setups for that. A later setup turned over grips what was their top: cut the sides there through the whole band (an outside Contour down past it) or leave them whole, never part of the way. A later setup facing the same way grips what was their bottom: stop an outside Contour above its grip with `bottom_offset` and finish the rest in a later setup. Order the setups so that each starts from stock its holder can hold.
- A setup its holder cannot hold comes back as a problem with the stage `workholding` on that setup; it names the earlier operation that removed the material. Change that operation, the order of the setups or the `holder`.{tools}

")
}
/// What the model is told before it sees the part, without pictures, as
/// the tests read it.
#[cfg(test)]
pub fn instructions() -> String {
    instructions_with(false, false, false)
}
/// `instructions_for` for a machine without a rotary A axis, as the tests read it.
#[cfg(test)]
pub fn instructions_with(pictures: bool, tool: bool, overview: bool) -> String {
    instructions_for(pictures, tool, overview, false)
}
/// What the model is told before it sees the part. Every limit the builder
/// and the generators enforce is stated, so a correct plan passes first time.
/// With `tool` (the planner tools are offered), a TOOLS section lists them
/// with their limits, `view_model` only with `pictures`, and PLANNING THE
/// HOLDING names the tools that test a draft's holding. With `pictures`, a
/// PICTURES section before RULES explains the pictures: with `overview`, the
/// overview the conversation carries; with `tool`, the view_model tool and its
/// limits.
/// With `rotary` (the machine has an A axis), the report's `rotary` and a ROTARY
/// A AXIS section say how to use it; without, the text is the same as it was.
pub fn instructions_for(pictures: bool, tool: bool, overview: bool, rotary: bool) -> String {
    let (rotary_part, rotary) = if rotary {
        (ROTARY_PART, ROTARY)
    } else {
        ("", "")
    };
    let tools = if tool {
        planner_tools::instructions(pictures)
    } else {
        String::new()
    };
    let pictures = if pictures {
        // Without its overview (it could not be drawn) the section does not
        // promise one.
        let first = if overview {
            "After the part report you get an overview picture drawn by the program: setup 1's orientation from the front-left, with the stock (amber, see-through) and setup 1's vise or clamps (grey)."
        } else {
            "The pictures are drawn by the program, with the stock (amber, see-through) and setup 1's vise or clamps (grey)."
        };
        let (look, limits) = if tool {
            (
                "- Call `view_model` to see more: another orientation or angle, zoomed on surfaces, surfaces highlighted in blue, layers on or off, and, once a proposal has been checked, one of its operations (`operation`, counted from 1 as in the check report): its toolpaths, and the stock left after it (cut stock green); and cut open on a section (`section`, square to an axis or through a surface) to see inside: depths, wall and floor thicknesses, stock left in a pocket or under the part.\n",
                format!(
                    "You get at most {MAX_PICTURES_PER_REQUEST} pictures per request (the part report, or a check report the program sends back) and {MAX_PICTURES_PER_RUN} in the run. "
                ),
            )
        } else {
            ("", String::new())
        };
        format!("PICTURES
- {first} Tags in white boxes give each listed plane's surface IDs (at most three, then \"…\") and each hole's `diameter` (as Ø6.0). The caption names the view and has a scale bar in mm; the triad shows +X (red), +Y (green), +Z (blue).
{look}- {limits}Older pictures are replaced by their descriptions. The pictures are drawn from the same model as the report; when they seem to disagree, the report's numbers are right.

")
    } else {
        String::new()
    };
    let holding = holding_rules(tool);
    let (per_setup, total) = (proposal::MAX_SETUP_OPERATIONS, proposal::MAX_OPERATIONS);
    let samples = format!(
        "{},{:03},{:03}",
        crate::cam::CEILING / 1_000_000,
        crate::cam::CEILING / 1000 % 1000,
        crate::cam::CEILING % 1000
    );
    format!("You plan CNC milling jobs for AXIS CAM Studio, a 3-axis CAM program. You receive a JSON part report and reply with one JSON proposal that matches the given schema. The program then generates every operation, simulates the stock, runs its export safety checks and tells you what failed, so you can correct the proposal.

THE PART REPORT
- Lengths are millimetres. `tools` lists the only tools you may use, by `number`, with their `diameter`, `flutes`, flute length and stickout, and a chamfer mill's `tip_angle`. `machine` gives the maximum rpm and feed, and in `coolants` the coolant choices the machine has outputs for. `material` is the project's material with its cutting data (see SPEEDS AND FEEDS).
- `orientations` lists six ways to clamp the part, named by the part axis that points up. The first entry is setup 1, where the stock is defined; your first setup must use it. Each orientation may be used by one setup only, so a plan has at most 6 setups. Add another setup only for features that cannot be reached otherwise.
- In each orientation, `planes` are the horizontal faces visible from above, with their `surfaces` IDs. `kind` is `top` for the highest face and `floor` for lower faces. `depth_below_top` is measured from the model's top in that orientation. `stock_margin` is the untouched stock beyond the model: top, bottom, -X, +X, -Y, +Y. Faces that an earlier setup machined have no stock left on them.
- A floor's `max_cutter_diameter` is the largest flat cutter, up to 50 mm, that has somewhere to cut on it: everything of the part above the floor keeps the cutter out, but past an edge where the part falls away below the floor (a ledge, a notch through a wall) the cutter may stand over the lower area, so such a floor takes a large cutter. A floor's `openings` are the holes through it, of any shape, with nothing of the part over or under them (cut-outs through a plate), each with its box, its `depth` from the floor down to where its walls end, and the largest flat cutter that fits inside it (`max_cutter_diameter`). A plane's `holes` give the largest drill that fits (`diameter`, a bore's nominal size: a drill of exactly that diameter fits), the opening diameter (`opening_diameter`, larger at a counterbore or a countersink), the depth and whether the hole goes through. `countersink` is null, or the cone the opening narrows down in: its included `angle` in degrees and its `depth` below the plane.
- `freeform_area` is the visible area that is neither horizontal nor vertical; only Parallel finishing follows it.
- `truncated` true means the orientation has more planes or holes than are listed; only listed surfaces can be used.
- `workholding`, when present, is setup 1's modelled vise or clamps, with its `number` in `workholding_library`, or its chuck (see WORKHOLDING).
- `workholding_library` lists the vises and clamps you may choose from, by `number`, with their sizes (see PLANNING THE HOLDING).
- `sheet`, when present, says setup 1's stock is a sheet of `size` (X, Y, thickness) lying on a spoilboard, for a flat part cut from it: a cut through the sheet may go on into the board by `spoilboard_allowance` mm, never deeper, so in setup 1 `bottom_offset` and a through drill's `breakthrough` may take a cut that far below the model's bottom. `holddown` is what holds the sheet: `Screws` through its waste, whose heads are setup 1's `workholding` boxes (kind `Hold-down screws`), or `Tape`. `needs_tabs` true says the screws hold only the waste: an outside Contour that cuts through must leave `tabs`. On a sheet the top plane lists `openings` too: the part's cut-outs, which a Contour `inside_holes` cuts (a Pocket needs a floor).
{rotary_part}
STRATEGIES (the only six)
- Face: flattens the top. `surfaces` = the top plane's IDs. Tool: flat end mill or face mill. `stepdown` at most the flute length, `stepover_percent` 1-100. Facing needs stock above the face: use it only where the orientation's `stock_margin[0]` is greater than `bottom_offset`. `bottom_offset` above 0 leaves that much stock on the face. `facing_area` `stock` (or null) faces the whole stock top, `model` only over the model's outline seen from above, a little past it, leaving the stock beyond for later operations. Face makes levels × passes strokes: levels = (`stock_margin[0]` − `bottom_offset`) ÷ `stepdown`, rounded up; passes = (the faced width in Y + the tool diameter) ÷ the stepover in mm, rounded up, plus 1. The faced width is the stock's (the model's Y size + `stock_margin[4]` + `stock_margin[5]`), or the model's Y size with `facing_area` `model`, which also goes once round the outline at each level.
- Pocket: clears from the stock top down to each floor it lists, each to its own depth, the highest first; a deeper floor inside a shallower one starts where that one stops. `surfaces` = all IDs of one or more floor planes of the orientation. Tool: a flat end mill no wider than the largest `max_cutter_diameter` of those floors; a floor it does not fit is skipped. A Pocket only goes where earlier operations left material, so the same Pocket again with a smaller cutter is a rest pass: it cleans the corners and narrow places a larger cutter could not reach, in a fraction of the time; give it `finish_passes` 0. `through` true also cuts each of the floors' `openings` down to where its walls end plus `breakthrough` (null means 1 mm), and the ones narrower than the cutter are left for a smaller one; their corners keep the cutter's radius, so finish cut-outs that need tight corners (switch cut-outs) with a rest pass of the smallest cutter that reaches; with `through` false or null the openings are cleared over at the floor's height only. `stepover_percent` at most 50, `stepdown` at most the flute length. The floor's depth plus the stock above the top must be less than the tool's stickout. `radial_allowance` and `axial_allowance` leave stock. `pocket_entry` is `Profile ramp` or `Plunge`; `ramp_angle` null or 0.1-15 degrees, also with `Plunge`. `finish_passes` a whole number 0-20. Its moves are about levels × the points of all its offset rings, and three settings multiply them: a smaller `stepover_percent` adds rings, each finish pass repeats the outermost ring at every level, and a smaller `ramp_angle` adds ramp moves at every entry (the tool stays down from ring to ring and enters again only for the deepest loop of each separate spot).
- Contour, `contour_mode` `outside`: one path around the whole model outline, from the stock top down to the model's bottom plus `bottom_offset`. `surfaces` empty. Flat end mill, `stepdown` at most the flute length, `finish_passes` a whole number 0-20. The cut depth from the stock top must be less than the stickout; for a tall part use `bottom_offset` to stop higher and finish from the opposite side. It removes one tool width around the outline, not all surrounding stock. Its moves are about levels × the outline's points.
- Contour, `contour_mode` `inside_floor`: the walls of one floor. `surfaces` = that floor's IDs. The tool and depth rules of Pocket apply: a flat end mill no wider than the floor's `max_cutter_diameter`, depth within the stickout, `stepdown` at most the flute length, `finish_passes` a whole number 0-20; `pocket_entry` and `ramp_angle` work as for Pocket, but its stepover is fixed at 40 %, so `stepover_percent` changes nothing. Its moves are about levels × the points of its rings, as for Pocket: each finish pass repeats the outermost ring at every level, and a smaller `ramp_angle` adds ramp moves at every entry (the tool stays down from ring to ring and enters again only for the deepest loop of each separate spot).
- Contour, `contour_mode` `inside_holes`: inside every hole through the model's outline seen from above (a flat part's cut-outs, round holes too), from the stock top down to the model's bottom plus `bottom_offset`. `surfaces` empty. Flat end mill. `hole_diameter` null cuts every hole the tool fits in; a number cuts only the holes at least that wide, leaving the smaller ones to a Drill. The slugs come free.
- Contour `tabs`, for `contour_mode` `outside` only (null otherwise): holding tabs that keep the part joined to the stock round it where the cut goes through, `count` (1-50) on each outline, each `width` wide and `height` thick, placed by the program on the outline's straight sides and gentle curves, never on corners, short segments or tight arcs: each needs its width plus the tool's diameter plus 2 mm of outline that turns by 29° at most. Tabs need stock beside the part to join.
- Drill: drills the holes of one plane. `surfaces` = that plane's IDs. Tool: a drill no wider than the hole's `diameter` (one of exactly that diameter fits). With a chamfer mill instead it countersinks each hole: its point goes down until its cone is as wide as the hole's `opening_diameter` at the plane, so choose one at least that wide, and drill the holes before it. A hole with a `countersink` needs a chamfer mill whose `tip_angle` is the countersink's `angle` (within 0.5°); another is refused. Countersink every hole that has one, or list it in `unmachined`. `hole_diameter` limits it to holes whose `diameter` equals it (not `opening_diameter`); null drills every hole of the plane. The drill's depth is measured from the stock top: `stock_margin[0]` + the plane's `depth_below_top` + the hole's `depth` + `breakthrough` must be less than the drill's flute length. `drill_cycle` is `Drilling`, `Chip breaking`, `Deep drilling` (both need `peck`), `Counterboring`, `Helical milling`, `Thread milling` or `Tapping`.
- Drill, `drill_cycle` `Helical milling`: mills each hole of the plane with a flat end mill instead of a drill, down a helix of `stepdown` per turn to a flat floor, so one end mill cuts holes of any size it fits and blind holes keep flat bottoms (a drill leaves its 118° point). Tool: a flat end mill no wider than the smallest selected hole's `diameter`; use `hole_diameter` to leave out the holes it does not fit. A hole over twice the cutter's diameter is cut in rings `stepover_percent` (1-100) apart. `stepdown` at most the flute length; the depth rule of drilling applies with the flute length. Not on a manual mill.
- Threads: a hole with a `thread` is tapped or thread-milled after it is drilled. First drill it with a drill of its `tap_drill` (or Helical milling to that size), through its `length` and, when blind, a pitch and a half deeper. Then a Drill of the same plane with `drill_cycle` `Thread milling` on a CNC: tool a thread mill whose `pitches` range holds the thread's `pitch`, whose `thread_angle` is the thread's, narrower than `tap_drill` and whose `reach` is at least the thread's `length` plus half a pitch; it cuts the size and hand modeled. `Tapping` is only for a manual mill or a lathe (the CNC cannot tap): tool a tap of the thread's size. Use `hole_diameter` (the hole's `diameter`) to keep each to the holes of one thread size. Thread every hole that has a `thread`, or list it in `unmachined`.
- Drill, through holes: `through` may be true only in setup 1, and only when every hole the operation selects is a through hole in the report; use `hole_diameter` to select only those. Setups after the first rest on the face where a drill would exit, so drill through holes in setup 1, or set `through` false and list the exit in `unmachined`. With `through` true, `breakthrough` extends the drill below the hole; null means 1 mm. It is at most half the drill diameter plus 1 mm and never more than the stock below the model (`stock_margin[1]`).
- Parallel: surface-following finishing of the whole model from above. `surfaces` empty. Ball or flat end mill. `surface_stepover` at most the tool radius, `tolerance` 0.01-1. The pass count times the samples per pass must stay under {samples}: (the model's Y size ÷ `surface_stepover`) × (the model's X size ÷ step), where step = min(`surface_stepover`, √(8 × tool radius × `tolerance`), 1 mm) and must be at least 0.05 mm.
- Waterline: constant-height passes around the model, always from the model's top down to its bottom plus `bottom_offset`. `surfaces` empty. Flat end mill, `stepdown` at most the flute length. That height must be less than the tool's stickout. `surface_stepover` is the most its neighbouring layers' loops may lie apart across a sloping face: over a face of slope θ (flatter than 1° excepted) layers are at most `surface_stepover` × tan θ apart instead of `stepdown`. It cuts about that height ÷ `stepdown` layers, plus each sloping face's width seen from above ÷ `surface_stepover` layers, its moves about layers × the outline's points.

WORKHOLDING
- `workholding` in the part report, when present, describes what holds the stock in setup 1. `kind` is `Vise` (two jaws on opposite sides of the stock), `Clamps`, `Chuck`, or for a round bar `V-block`, `Soft jaws` or `Collet block`, with `round_bar` giving its `diameter`, `length` and `along` (`X` lying, `Z` standing); a lying bar's work zero may be on its axis (`Bar axis at left end`, `Bar axis center`). `boxes` lists each jaw or clamp as [x0, y0, z0, x1, y1, z1] in setup 1's axes, in mm from the model's -X/-Y/bottom corner. A clamp lies over the stock top; a jaw reaches from the stock's bottom up to its top, which is its z1.
- A `Chuck` holds the stock on the machine's rotary A axis, along X, by its -X end: `chuck` gives its `jaws` (3 or 4), its `chuck_diameter`, the `jaw_length`, the `gripped_length` of stock in the jaws, the `stickout` beyond them towards +X, the `bar_diameter` of a round bar, where the `tailstock` is and the `a_angle` every operation runs at (0: indexed angles are not planned). Its `boxes` are the chuck body, each jaw, the rotary head, the tailstock's centre, quill and body when it is on the table, and the stock gripped in the jaws: no tool may come within `gap` of the gripped stock either, so cut only the stick-out, from its +X end to `gap` plus the tool's radius short of the jaws.
- The boxes are keep-out zones. The program grows each box by `gap` mm on every side and refuses any move, rapids and retracts included, that brings the cutter or the tool holder within `gap` of a box. A refused move comes back as a problem with the stage `workholding`, by setup and operation. Parallels, bolts, the table and the machine body are not checked.
- The later setups (every setup after the first) are held by the library entry their `holder` names, or by setup 1's own entry when it is null (see PLANNING THE HOLDING). That entry is laid out by the program once it has generated the setup's toolpaths: it picks the sides and positions that keep them clearest of those toolpaths, and the preview shows the result. What still comes within `gap` of it is reported with the stage `workholding` too.
- Keep the plan clear of the boxes. Do not face or contour where a clamp sits over the stock edge, and do not let a tool work beside a jaw below the jaws' top. Keep outside contours above the jaws' top by giving them a `bottom_offset` greater than the jaw's z1 plus `gap`, and finish the walls below from the setup turned over, whose vise can grip the walls finished above (see PLANNING THE HOLDING); or move them to a setup where the sides are free.
- `clearance_above_stock_top` is present when a box and its gap reach above the stock top, and says how far. The default `clearance` of 15 already covers a height below 15, so leave `clearance` null then. When the height is 15 or more, give every operation a `clearance` greater than it: a machine that retracts to a clearance plane refuses a plan whose plane (the highest `clearance` in a setup) is not above the boxes and their gap, and rapids then pass over the boxes. Otherwise leave `clearance` null.
- A `workholding` problem names its place in the setup's work coordinates (X, Y and Z measured from the setup's work zero, Z up), not from the model's corner as the report's boxes are: convert before comparing.

{rotary}{holding}SPEEDS AND FEEDS
- `material` in the report holds the cutting data for carbide tools: `vc_min_m_per_min` and `vc_max_m_per_min` give the surface speed range in m/min, and `chip_load_mm` the chip load per tooth in mm in five bands by tool diameter. A band holds the diameters up to its `up_to_mm` (and above the previous band's), with its `min` and `max`; the last band, with `up_to_mm` null, holds every larger diameter.
- The program computes, for every operation, the surface speed Vc = π × D × rpm ÷ 1000 (m/min) and the chip load fz = feed ÷ (rpm × flutes) (mm per tooth), from the tool's diameter D and `flutes` in `tools` and the operation's `rpm` and `feed`. Vc must lie between `vc_min_m_per_min` and `vc_max_m_per_min`, and fz between the `min` and `max` of the band of the tool's diameter; under 3 mm the band's `min` and `max` shrink in proportion to the diameter (a Ø1 tool takes a third of them, a Ø0.5 a sixth). Choose a Vc in range and set rpm = Vc × 1000 ÷ (π × D), then choose an fz in the band and set feed = fz × rpm × flutes. For example, a 3-flute Ø10 mm tool at 8000 rpm and 1000 mm/min runs at Vc = 251 m/min with fz = 0.042 mm per tooth.
- Drills are checked like end mills: a Drill's `feed` is divided by rpm × flutes in the same way and must give a chip load inside the band of the drill's diameter, so choose drill feeds inside those bands. A small drill needs a high rpm to reach the surface speed.
- When even the machine's maximum rpm (`machine.max_rpm`) gives a tool less than the minimum surface speed, running at that maximum rpm counts as in range. When the minimum chip load needs more than the machine's maximum feed (`machine.max_feed`), running at that maximum feed counts as in range. A value above the material's maximum is never excused, and rounding gets 1% of slack on each limit.
- Plunge and ramp feeds are not checked. Plans with a spindle speed or feed outside the material's cutting data are refused: each such operation comes back as a problem with the stage `cutting data`, together with the generation, export, tool-axis and gouge problems.

{tools}{pictures}RULES
- Use only tool numbers from `tools` and surface IDs from the chosen orientation's `planes`. Never invent either.
- Every setup needs at least one operation. At most {per_setup} operations per setup and {total} in total. Spindle speed is at least 100 rpm and feed at least 1 mm/min, both within the machine's maximums.
- Every number must be finite.
- Every operation is limited to {samples} moves (Parallel to {samples} samples, as above): what memory holds, far past any real job. A finer stepdown or stepover multiplies the moves and the cutting time: choose them for the cut, not to stay under the limit.
- The program refuses any move whose tool centre line passes through the part, and any move below the bottom of the stock. In setups after the first, nothing may go below the part's underside.
- `bottom_offset`, `radial_allowance` and `axial_allowance` cannot be negative, but for a `sheet` in setup 1 `bottom_offset` may be down to -`spoilboard_allowance`. Only a through drill's breakthrough in setup 1 goes below the model, into the stock beneath it. On a `sheet` a cut in setup 1 may also go into the spoilboard, by its allowance at most.
- Order operations in a setup: Face, then Pocket (larger cutters before the rest passes of smaller ones), then Contour, then Drill (drills before chamfer mills), then finishing.
- No tool tip may go deeper below the stock top than the tool's stickout.
- When `machine.clearance` is present, the program also refuses a stock that stands taller than `gantry_height` (the vise's height under it plus the stock's height) or wider than `gantry_width`, and a tool whose stick-out and holder, from the highest the tip travels, reach past `nose_to_table` (the spindle nose at Z top, over the table). Stand a tall part on its shorter side or use a shorter tool; a figure named in `assumed` was not measured, so say so when a plan is close to it.
- Choose each operation's spindle speed and feed from the material's cutting data (see SPEEDS AND FEEDS), within the machine's limits.
- Set each operation's `coolant` to one of `machine.coolants`: coolant the machine has no output for is refused.
- Set the fields a strategy does not use to null. A null field takes the program's default: `stepdown` 2, `stepover_percent` 40, `surface_stepover` 1, `tolerance` 0.01, `peck` 3, `finish_passes` 1, `ramp_angle` 2, `breakthrough` 1, `clearance` 15, allowances and `bottom_offset` 0.
- For later setups choose the work zero: `Model origin` is accepted; the program tells the operator where it lies relative to the part, and to the stock top while no earlier setup has cut it. Prefer a stock-based zero when the stock it is touched off on is untouched. A stock-based zero `Stock top <spot>` or `Stock bottom <spot>` is a point of the box the setup starts from (what the earlier setups leave), <spot> being `center`, a corner (`front-left`, `front-right`, `back-left`, `back-right`) or the middle of an edge (`front`, `back`, `left`, `right`); front is -Y, left -X. It is touched off on the stock top over the point (a zero on the bottom as the stock's height above it), and X0 Y0 from the sides it names (both sides of an axis for a point midway along it) between 1 mm and 10 mm below the top, near the end of each side nearer the zero (5% to 15% of its length from it) or near its middle. Corners are touched within 10 mm of the corner, at least 1 mm in from both edges. On stock under 40 mm tall the sides are touched within a quarter of its height below the top, and under 40 mm across the corner area is a quarter of the shorter top side. A stock-based zero is refused when earlier setups have cut the stock at one of those spots, or under a zero on the bottom.
- Give each setup a short `workholding` note. The jaws or clamps in the report are modelled; say what else must be kept clear.
- `clearance` is the height above the stock top, in mm, at which the tool travels between cuts: 15 when null. It can only be raised, never below 15 and never above the machine's Z travel (`machine.travel`, its third value; 300 when that is 0). Raise it only for the reason given under WORKHOLDING.
- List every feature you cannot machine with these tools and strategies in `unmachined`, with the reason. Do not leave it out silently, and do not add an operation that cannot work.
- Put risks the operator should know about in `warnings`.
- In the check report, `covered_other` lists the remaining surface by `facing` (+X, -X, +Y, -Y, top, bottom), named in setup 1's frame, which can differ from the orientation names when setup 1 is rotated.
- In the check report, `stocks` gives the stock each setup starts from, in that setup's axes: its `size`, its box from the model's -X/-Y/bottom corner (`box_from_part_corner`), how much the earlier setups `removed` whole from each side, and `by` which operations.
- Reply with the JSON proposal only.")
}
fn feedback(check: &CheckReport) -> String {
    let report = serde_json::to_string(check).unwrap_or_default();
    if check.passed {
        format!(
            "The proposal passes every check. Its check report follows. Some faces are still covered by stock (`covered_planes`, and `covered_other` by the direction it faces in setup 1's frame). If these tools and strategies can machine them, extend the proposal, adding a setup when needed. Otherwise keep the plan and list them in `unmachined` with the reason. Reply with the full proposal.\n{report}"
        )
    } else {
        format!(
            "The proposal did not pass. Its check report follows; `problems` lists what failed, by setup and operation (counted from 1). Correct those points and reply with the full proposal. If a feature cannot be machined with these tools and strategies, remove its operation and list it in `unmachined`.\n{report}"
        )
    }
}
/// Unicode 16's combining marks (general category M: accents, vowel signs,
/// viramas, tone marks, enclosing marks) as sorted half-open ranges: first,
/// last + 1, first, last + 1, …
const MARKS: [u32; 642] = [
    0x0300, 0x0370, 0x0483, 0x048A, 0x0591, 0x05BE, 0x05BF, 0x05C0, 0x05C1, 0x05C3, 0x05C4, 0x05C6,
    0x05C7, 0x05C8, 0x0610, 0x061B, 0x064B, 0x0660, 0x0670, 0x0671, 0x06D6, 0x06DD, 0x06DF, 0x06E5,
    0x06E7, 0x06E9, 0x06EA, 0x06EE, 0x0711, 0x0712, 0x0730, 0x074B, 0x07A6, 0x07B1, 0x07EB, 0x07F4,
    0x07FD, 0x07FE, 0x0816, 0x081A, 0x081B, 0x0824, 0x0825, 0x0828, 0x0829, 0x082E, 0x0859, 0x085C,
    0x0897, 0x08A0, 0x08CA, 0x08E2, 0x08E3, 0x0904, 0x093A, 0x093D, 0x093E, 0x0950, 0x0951, 0x0958,
    0x0962, 0x0964, 0x0981, 0x0984, 0x09BC, 0x09BD, 0x09BE, 0x09C5, 0x09C7, 0x09C9, 0x09CB, 0x09CE,
    0x09D7, 0x09D8, 0x09E2, 0x09E4, 0x09FE, 0x09FF, 0x0A01, 0x0A04, 0x0A3C, 0x0A3D, 0x0A3E, 0x0A43,
    0x0A47, 0x0A49, 0x0A4B, 0x0A4E, 0x0A51, 0x0A52, 0x0A70, 0x0A72, 0x0A75, 0x0A76, 0x0A81, 0x0A84,
    0x0ABC, 0x0ABD, 0x0ABE, 0x0AC6, 0x0AC7, 0x0ACA, 0x0ACB, 0x0ACE, 0x0AE2, 0x0AE4, 0x0AFA, 0x0B00,
    0x0B01, 0x0B04, 0x0B3C, 0x0B3D, 0x0B3E, 0x0B45, 0x0B47, 0x0B49, 0x0B4B, 0x0B4E, 0x0B55, 0x0B58,
    0x0B62, 0x0B64, 0x0B82, 0x0B83, 0x0BBE, 0x0BC3, 0x0BC6, 0x0BC9, 0x0BCA, 0x0BCE, 0x0BD7, 0x0BD8,
    0x0C00, 0x0C05, 0x0C3C, 0x0C3D, 0x0C3E, 0x0C45, 0x0C46, 0x0C49, 0x0C4A, 0x0C4E, 0x0C55, 0x0C57,
    0x0C62, 0x0C64, 0x0C81, 0x0C84, 0x0CBC, 0x0CBD, 0x0CBE, 0x0CC5, 0x0CC6, 0x0CC9, 0x0CCA, 0x0CCE,
    0x0CD5, 0x0CD7, 0x0CE2, 0x0CE4, 0x0CF3, 0x0CF4, 0x0D00, 0x0D04, 0x0D3B, 0x0D3D, 0x0D3E, 0x0D45,
    0x0D46, 0x0D49, 0x0D4A, 0x0D4E, 0x0D57, 0x0D58, 0x0D62, 0x0D64, 0x0D81, 0x0D84, 0x0DCA, 0x0DCB,
    0x0DCF, 0x0DD5, 0x0DD6, 0x0DD7, 0x0DD8, 0x0DE0, 0x0DF2, 0x0DF4, 0x0E31, 0x0E32, 0x0E34, 0x0E3B,
    0x0E47, 0x0E4F, 0x0EB1, 0x0EB2, 0x0EB4, 0x0EBD, 0x0EC8, 0x0ECF, 0x0F18, 0x0F1A, 0x0F35, 0x0F36,
    0x0F37, 0x0F38, 0x0F39, 0x0F3A, 0x0F3E, 0x0F40, 0x0F71, 0x0F85, 0x0F86, 0x0F88, 0x0F8D, 0x0F98,
    0x0F99, 0x0FBD, 0x0FC6, 0x0FC7, 0x102B, 0x103F, 0x1056, 0x105A, 0x105E, 0x1061, 0x1062, 0x1065,
    0x1067, 0x106E, 0x1071, 0x1075, 0x1082, 0x108E, 0x108F, 0x1090, 0x109A, 0x109E, 0x135D, 0x1360,
    0x1712, 0x1716, 0x1732, 0x1735, 0x1752, 0x1754, 0x1772, 0x1774, 0x17B4, 0x17D4, 0x17DD, 0x17DE,
    0x180B, 0x180E, 0x180F, 0x1810, 0x1885, 0x1887, 0x18A9, 0x18AA, 0x1920, 0x192C, 0x1930, 0x193C,
    0x1A17, 0x1A1C, 0x1A55, 0x1A5F, 0x1A60, 0x1A7D, 0x1A7F, 0x1A80, 0x1AB0, 0x1ACF, 0x1B00, 0x1B05,
    0x1B34, 0x1B45, 0x1B6B, 0x1B74, 0x1B80, 0x1B83, 0x1BA1, 0x1BAE, 0x1BE6, 0x1BF4, 0x1C24, 0x1C38,
    0x1CD0, 0x1CD3, 0x1CD4, 0x1CE9, 0x1CED, 0x1CEE, 0x1CF4, 0x1CF5, 0x1CF7, 0x1CFA, 0x1DC0, 0x1E00,
    0x20D0, 0x20F1, 0x2CEF, 0x2CF2, 0x2D7F, 0x2D80, 0x2DE0, 0x2E00, 0x302A, 0x3030, 0x3099, 0x309B,
    0xA66F, 0xA673, 0xA674, 0xA67E, 0xA69E, 0xA6A0, 0xA6F0, 0xA6F2, 0xA802, 0xA803, 0xA806, 0xA807,
    0xA80B, 0xA80C, 0xA823, 0xA828, 0xA82C, 0xA82D, 0xA880, 0xA882, 0xA8B4, 0xA8C6, 0xA8E0, 0xA8F2,
    0xA8FF, 0xA900, 0xA926, 0xA92E, 0xA947, 0xA954, 0xA980, 0xA984, 0xA9B3, 0xA9C1, 0xA9E5, 0xA9E6,
    0xAA29, 0xAA37, 0xAA43, 0xAA44, 0xAA4C, 0xAA4E, 0xAA7B, 0xAA7E, 0xAAB0, 0xAAB1, 0xAAB2, 0xAAB5,
    0xAAB7, 0xAAB9, 0xAABE, 0xAAC0, 0xAAC1, 0xAAC2, 0xAAEB, 0xAAF0, 0xAAF5, 0xAAF7, 0xABE3, 0xABEB,
    0xABEC, 0xABEE, 0xFB1E, 0xFB1F, 0xFE00, 0xFE10, 0xFE20, 0xFE30, 0x101FD, 0x101FE, 0x102E0,
    0x102E1, 0x10376, 0x1037B, 0x10A01, 0x10A04, 0x10A05, 0x10A07, 0x10A0C, 0x10A10, 0x10A38,
    0x10A3B, 0x10A3F, 0x10A40, 0x10AE5, 0x10AE7, 0x10D24, 0x10D28, 0x10D69, 0x10D6E, 0x10EAB,
    0x10EAD, 0x10EFC, 0x10F00, 0x10F46, 0x10F51, 0x10F82, 0x10F86, 0x11000, 0x11003, 0x11038,
    0x11047, 0x11070, 0x11071, 0x11073, 0x11075, 0x1107F, 0x11083, 0x110B0, 0x110BB, 0x110C2,
    0x110C3, 0x11100, 0x11103, 0x11127, 0x11135, 0x11145, 0x11147, 0x11173, 0x11174, 0x11180,
    0x11183, 0x111B3, 0x111C1, 0x111C9, 0x111CD, 0x111CE, 0x111D0, 0x1122C, 0x11238, 0x1123E,
    0x1123F, 0x11241, 0x11242, 0x112DF, 0x112EB, 0x11300, 0x11304, 0x1133B, 0x1133D, 0x1133E,
    0x11345, 0x11347, 0x11349, 0x1134B, 0x1134E, 0x11357, 0x11358, 0x11362, 0x11364, 0x11366,
    0x1136D, 0x11370, 0x11375, 0x113B8, 0x113C1, 0x113C2, 0x113C3, 0x113C5, 0x113C6, 0x113C7,
    0x113CB, 0x113CC, 0x113D1, 0x113D2, 0x113D3, 0x113E1, 0x113E3, 0x11435, 0x11447, 0x1145E,
    0x1145F, 0x114B0, 0x114C4, 0x115AF, 0x115B6, 0x115B8, 0x115C1, 0x115DC, 0x115DE, 0x11630,
    0x11641, 0x116AB, 0x116B8, 0x1171D, 0x1172C, 0x1182C, 0x1183B, 0x11930, 0x11936, 0x11937,
    0x11939, 0x1193B, 0x1193F, 0x11940, 0x11941, 0x11942, 0x11944, 0x119D1, 0x119D8, 0x119DA,
    0x119E1, 0x119E4, 0x119E5, 0x11A01, 0x11A0B, 0x11A33, 0x11A3A, 0x11A3B, 0x11A3F, 0x11A47,
    0x11A48, 0x11A51, 0x11A5C, 0x11A8A, 0x11A9A, 0x11C2F, 0x11C37, 0x11C38, 0x11C40, 0x11C92,
    0x11CA8, 0x11CA9, 0x11CB7, 0x11D31, 0x11D37, 0x11D3A, 0x11D3B, 0x11D3C, 0x11D3E, 0x11D3F,
    0x11D46, 0x11D47, 0x11D48, 0x11D8A, 0x11D8F, 0x11D90, 0x11D92, 0x11D93, 0x11D98, 0x11EF3,
    0x11EF7, 0x11F00, 0x11F02, 0x11F03, 0x11F04, 0x11F34, 0x11F3B, 0x11F3E, 0x11F43, 0x11F5A,
    0x11F5B, 0x13440, 0x13441, 0x13447, 0x13456, 0x1611E, 0x16130, 0x16AF0, 0x16AF5, 0x16B30,
    0x16B37, 0x16F4F, 0x16F50, 0x16F51, 0x16F88, 0x16F8F, 0x16F93, 0x16FE4, 0x16FE5, 0x16FF0,
    0x16FF2, 0x1BC9D, 0x1BC9F, 0x1CF00, 0x1CF2E, 0x1CF30, 0x1CF47, 0x1D165, 0x1D16A, 0x1D16D,
    0x1D173, 0x1D17B, 0x1D183, 0x1D185, 0x1D18C, 0x1D1AA, 0x1D1AE, 0x1D242, 0x1D245, 0x1DA00,
    0x1DA37, 0x1DA3B, 0x1DA6D, 0x1DA75, 0x1DA76, 0x1DA84, 0x1DA85, 0x1DA9B, 0x1DAA0, 0x1DAA1,
    0x1DAB0, 0x1E000, 0x1E007, 0x1E008, 0x1E019, 0x1E01B, 0x1E022, 0x1E023, 0x1E025, 0x1E026,
    0x1E02B, 0x1E08F, 0x1E090, 0x1E130, 0x1E137, 0x1E2AE, 0x1E2AF, 0x1E2EC, 0x1E2F0, 0x1E4EC,
    0x1E4F0, 0x1E5EE, 0x1E5F0, 0x1E8D0, 0x1E8D7, 0x1E944, 0x1E94B, 0xE0100, 0xE01F0,
];
/// A combining mark, drawn on the character before it.
fn combining(c: char) -> bool {
    // An odd count of bounds at or below `c` puts it inside a range.
    MARKS.partition_point(|&bound| bound <= u32::from(c)) % 2 == 1
}
/// A letter of any script that a name keeps. Letters drawn as quotation
/// marks, primes or apostrophes are not kept: the modifier letters (ʺ ˮ ʹ ʻ
/// ʼ), the Greek numeral sign, ՙ, ߴ ߵ, ꞈ, Ꞌ ꞌ, the Canadian syllabics finals
/// ᐟ ᐠ ᐥ, ŉ and the Duployan affixes.
fn name_letter(c: char) -> bool {
    c.is_alphabetic()
        && !combining(c)
        && !matches!(
            c,
            '\u{149}'
                | '\u{2b0}'..='\u{2ff}'
                | '\u{374}'
                | '\u{559}'
                | '\u{7f4}'
                | '\u{7f5}'
                | '\u{141f}'
                | '\u{1420}'
                | '\u{1425}'
                | '\u{a788}'
                | '\u{a78b}'
                | '\u{a78c}'
                | '\u{1bc80}'..='\u{1bc93}'
        )
}
/// A format character that joins or breaks words without being drawn: the
/// soft hyphen, the zero-width space, non-joiner and joiner, the direction
/// marks, the word joiner, the invisible operators and the byte order mark.
fn invisible(c: char) -> bool {
    matches!(c, '\u{ad}' | '\u{200b}'..='\u{200f}' | '\u{2060}'..='\u{2064}' | '\u{feff}')
}
/// A character as a name reads it: fullwidth forms of ASCII (！ to ～)
/// become ASCII, except the fullwidth quotation marks ＂ and ＇, which stay
/// fullwidth and go; dashes and minus signs (U+2010–2015, U+2212, U+FE58,
/// U+FE63 and, through its ASCII form, U+FF0D) become -; the wave dash 〜
/// (and, through its ASCII form, ～) becomes ~; the Arabic decimal and
/// thousands separators and percent sign (U+066B, U+066C, U+066A) become
/// . , and %; the fraction and division slashes (U+2044, U+2215) become /;
/// ∅ (U+2205) becomes ⌀; and × becomes x.
fn name_form(c: char) -> char {
    let c = match c {
        '\u{ff02}' | '\u{ff07}' => c,
        '\u{ff01}'..='\u{ff5e}' => char::from_u32(u32::from(c) - 0xfee0).unwrap_or(c),
        _ => c,
    };
    match c {
        '\u{2010}'..='\u{2015}' | '\u{2212}' | '\u{fe58}' | '\u{fe63}' => '-',
        '\u{301c}' => '~',
        '\u{66b}' => '.',
        '\u{66c}' => ',',
        '\u{66a}' => '%',
        '\u{2044}' | '\u{2215}' => '/',
        '\u{2205}' => '\u{2300}',
        '\u{d7}' => 'x',
        _ => c,
    }
}
/// A setup or operation name the model chose, as one cleaned line of at most
/// 60 characters. Each character is first read as `name_form` gives it, so
/// "1–4", "1～4", "۳٫۵", "Z−0.5", "10×10" and "（左）" keep their meaning. The
/// name keeps letters and digits of any script, the combining marks on a
/// kept letter and - _ . , / ( ) # + : & % ~ < > = ± ° ⌀ ≤ ≥. A * between
/// two digits, white space aside, becomes x ("10 * 10" is "10 x 10"); any
/// other * is removed, so "**Roughing**" is "Roughing". An apostrophe
/// (' or ’) stays between a letter or digit and a letter, so "Bob's" and
/// "Op 2's" do too; combining marks are skipped back to the letter they are
/// on, so a decomposed "José's" keeps it as well. `invisible` format
/// characters go without a trace, so a Persian word written with a
/// zero-width non-joiner, or "Bohrung" with a soft hyphen, stays one word,
/// and a middle dot · inside Catalan l·l goes the same way ("paral·lel" is
/// "parallel"); elsewhere it separates words ("드릴·탭" is "드릴 탭"). Every other character becomes a space, and runs of spaces
/// become one, so words and numbers never run together, and no quotation or
/// check mark, or listed look-alike, can close the app's quotes around the
/// name or pass for the app's check lines or the " · " between their fields.
pub(crate) fn name(text: &str) -> String {
    let mut kept = String::new();
    let mut chars = text
        .chars()
        .filter(|&c| !invisible(c))
        .map(name_form)
        .peekable();
    // The last character that is not a combining mark, and whether it is a
    // kept letter, whose marks stay.
    let (mut base, mut on_letter) = (None::<char>, false);
    // The last character that is not white space.
    let mut before = None::<char>;
    while let Some(c) = chars.next() {
        let previous = before;
        if !c.is_whitespace() {
            before = Some(c);
        }
        // A * between two digits, white space aside, is a times sign; any
        // other * is removed like other symbols.
        let c = match c {
            '*' if previous.is_some_and(char::is_numeric)
                && chars
                    .clone()
                    .find(|next| !next.is_whitespace())
                    .is_some_and(char::is_numeric) =>
            {
                'x'
            }
            _ => c,
        };
        if combining(c) {
            kept.push(if on_letter { c } else { ' ' });
            continue;
        }
        // A middle dot inside Catalan l·l goes without a space; anywhere
        // else it separates words, as in Korean "드릴·탭" (drill and tap).
        if c == '\u{b7}'
            && matches!(base, Some('l' | 'L'))
            && matches!(chars.peek(), Some('l' | 'L'))
        {
            continue;
        }
        let apostrophe = matches!(c, '\'' | '\u{2019}')
            && base.is_some_and(|b| name_letter(b) || b.is_numeric())
            && chars.peek().is_some_and(|&next| name_letter(next));
        on_letter = name_letter(c);
        let keep = on_letter
            || c.is_numeric()
            || apostrophe
            || matches!(
                c,
                '-' | '_'
                    | '.'
                    | ','
                    | '/'
                    | '('
                    | ')'
                    | '#'
                    | '+'
                    | ':'
                    | '&'
                    | '%'
                    | '~'
                    | '<'
                    | '>'
                    | '='
                    | '\u{b1}'
                    | '\u{b0}'
                    | '\u{2300}'
                    | '\u{2264}'
                    | '\u{2265}'
            );
        kept.push(if keep { c } else { ' ' });
        base = Some(c);
    }
    // One line: runs of spaces become one, and none is left at either end.
    proposal::clip(&kept, 60)
}
/// The model's free text, cleaned and bounded once, before anything keeps or
/// shows it, and before the check builds the job from it. Returns how many
/// warnings and unmachined entries were left out.
pub(crate) fn tidy(mut p: Proposal) -> (Proposal, usize, usize) {
    use proposal::clip;
    p.summary = clip(&p.summary, 600);
    let more_warnings = p.warnings.len().saturating_sub(12);
    p.warnings.truncate(12);
    for warning in &mut p.warnings {
        *warning = clip(warning, 240);
    }
    let more_unmachined = p.unmachined.len().saturating_sub(20);
    p.unmachined.truncate(20);
    for item in &mut p.unmachined {
        item.feature = clip(&item.feature, 80);
        item.reason = clip(&item.reason, 240);
    }
    for setup in &mut p.setups {
        setup.name = name(&setup.name);
        setup.workholding = clip(&setup.workholding, 500);
        for op in &mut setup.operations {
            op.name = name(&op.name);
            op.reason = clip(&op.reason, 160);
        }
    }
    (p, more_warnings, more_unmachined)
}
/// What joins the planner's workholding note to setup 1's own notes.
const PLANNER_NOTE: &str = " · Planner: ";
/// Setup 1's parameter that keeps the planner note the last Apply added,
/// saved with the project, so a later Apply finds that exact text again.
pub(crate) const PLANNER_NOTE_KEY: &str = "planner_note";
/// Setup 1's notes after Apply, and the planner note to keep in its
/// `planner_note` parameter (empty when there is none). The planner's
/// workholding note joins the notes on one line, "… · Planner: …", or
/// "Planner: …" alone when nothing else is left. `earlier` is the note an
/// earlier Apply added, removed only where that exact text still stands
/// whole, so notes do not pile up: the last " · Planner: <earlier>" followed
/// by the end of the notes or by white space, or else the last
/// "Planner: <earlier>" that is also at the start of the notes or after
/// white space, with the white space after it. When `earlier` does not end
/// a sentence (. ! ? or 。), only the end of the notes or a line break may
/// follow it, since words after it on the same line may go on with its
/// sentence. Everything else stays, including text the user typed before or
/// after the planner's note, notes the user started with "Planner: ", and a
/// planner note the user changed or went on with, mid-word or not. The
/// planner's part, line breaks turned into spaces, is at most 500
/// characters; the user's own notes are never cut.
pub(crate) fn setup_1_notes(notes: &str, earlier: &str, note: &str) -> (String, String) {
    let joined = format!("{PLANNER_NOTE}{earlier}");
    let alone = format!("Planner: {earlier}");
    // Whether the notes end at `end` or white space follows it (a line
    // break, unless `earlier` ends a sentence), and whether they start at
    // `at` or white space comes before it.
    let sentence = earlier.ends_with(['.', '!', '?', '\u{3002}']);
    let ends_whole = |end: usize| {
        notes[end..].chars().next().is_none_or(|next| {
            if sentence {
                next.is_whitespace()
            } else {
                matches!(
                    next,
                    '\n' | '\u{b}' | '\u{c}' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'
                )
            }
        })
    };
    let starts_whole = |at: usize| {
        notes[..at]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace)
    };
    let own = if earlier.is_empty() {
        notes.to_string()
    } else if let Some((at, _)) = notes
        .rmatch_indices(joined.as_str())
        .find(|&(at, _)| ends_whole(at + joined.len()))
    {
        format!("{}{}", &notes[..at], &notes[at + joined.len()..])
    } else if let Some((at, _)) = notes
        .rmatch_indices(alone.as_str())
        .find(|&(at, _)| starts_whole(at) && ends_whole(at + alone.len()))
    {
        let (before, after) = (&notes[..at], notes[at + alone.len()..].trim_start());
        if before.trim().is_empty() {
            after.to_string()
        } else {
            format!("{before}{after}")
        }
    } else {
        notes.to_string()
    };
    let own = own.trim_end();
    // One cleaned line; with "Planner: " it makes 500 characters at most.
    let note = proposal::clip(note, 500 - "Planner: ".len());
    let notes = match (own.is_empty(), note.is_empty()) {
        (_, true) => own.to_string(),
        (true, false) => format!("Planner: {note}"),
        (false, false) => format!("{own}{PLANNER_NOTE}{note}"),
    };
    (notes, note)
}
/// The job Apply puts in place of the current one: the outcome's, with the
/// planner's workholding note for setup 1 joined to setup 1's own notes on
/// the same line, in place of the exact note an earlier Apply added, which
/// setup 1 keeps for this (`setup_1_notes`). A change asked for in the chat
/// while a preview is open starts from this too.
pub fn applied(outcome: &Outcome) -> Document {
    let mut job = outcome.job.clone();
    let note = outcome.proposal.setups.first();
    if let Some(first) = job.setups.first_mut() {
        let note = note.map_or("", |s| s.workholding.as_str());
        let earlier = crate::model::get(&first.p, PLANNER_NOTE_KEY);
        let (notes, added) = setup_1_notes(crate::model::get(&first.p, "notes"), earlier, note);
        first.p.insert("notes".into(), notes);
        if added.is_empty() {
            first.p.remove(PLANNER_NOTE_KEY);
        } else {
            first.p.insert(PLANNER_NOTE_KEY.into(), added);
        }
    }
    job
}
/// Problems as a failed run keeps them: each message one line of at most
/// 300 characters.
fn bounded(mut problems: Vec<Problem>) -> Vec<Problem> {
    for problem in &mut problems {
        problem.message = proposal::clip(&problem.message, 300);
    }
    problems
}
/// The proposal without what models sometimes put around it: a Markdown
/// code fence, or sentences before or after the outermost braces.
pub(crate) fn json_object(text: &str) -> &str {
    match (text.find('{'), text.rfind('}')) {
        (Some(start), Some(end)) if start < end => &text[start..=end],
        _ => text,
    }
}
/// The document the checks run on: no project title, and every setup named
/// by its number, so no check message carries the title or a setup name to
/// OpenAI. The cutting-data messages name the project's material, which the
/// part report sends anyway; both carry its name clipped to 60 characters,
/// and each message is clipped to 300 characters.
fn anonymized(doc: &Document) -> Document {
    let mut doc = doc.clone();
    doc.title.clear();
    for (i, setup) in doc.setups.iter_mut().enumerate() {
        setup.p.insert("name".into(), format!("Setup {}", i + 1));
    }
    doc
}
/// The conversation as it is sent: images beyond the `KEPT_PICTURES` most
/// recent, or all of them once pictures are off, become their description;
/// once the tools are gone, their calls and outputs become plain text.
fn outgoing(conversation: &[Item], pictures: bool, tool: bool) -> Vec<Item> {
    // The tool each call named, to word its output.
    let called: std::collections::HashMap<&str, &str> = conversation
        .iter()
        .filter_map(|item| match item {
            Item::Call(call) => Some((call.call_id.as_str(), call.name.as_str())),
            _ => None,
        })
        .collect();
    let total = conversation
        .iter()
        .map(|i| match i {
            Item::Message(_, parts) => parts
                .iter()
                .filter(|p| matches!(p, Part::Image { .. }))
                .count(),
            _ => 0,
        })
        .sum::<usize>();
    let mut seen = 0;
    conversation
        .iter()
        .map(|item| match item {
            Item::Message(role, parts) => Item::Message(
                *role,
                parts
                    .iter()
                    .map(|part| match part {
                        Part::Image { alt, .. } => {
                            seen += 1;
                            if !pictures {
                                Part::Text(format!("(Picture not sent: {alt})"))
                            } else if total - seen >= KEPT_PICTURES {
                                Part::Text(format!("(Earlier picture, not resent: {alt})"))
                            } else {
                                part.clone()
                            }
                        }
                        text => text.clone(),
                    })
                    .collect(),
            ),
            Item::Call(call) if !tool => Item::text(
                Role::Assistant,
                if call.name == look::NAME {
                    format!("(Asked for a picture: {})", call.arguments)
                } else {
                    format!("(Called {}: {})", call.name, call.arguments)
                },
            ),
            Item::Output { call_id, output } if !tool => Item::text(
                Role::User,
                match called.get(call_id.as_str()) {
                    Some(&name) if name != look::NAME => {
                        format!("({name} answered: {output})")
                    }
                    _ => format!("(Picture tool answered: {output})"),
                },
            ),
            other => other.clone(),
        })
        .collect()
}
/// What the run may still show the model.
struct Sight {
    /// Pictures are sent as images; off once OpenAI refused them.
    pictures: bool,
    /// The planner tools are offered; off once OpenAI refused them.
    tool: bool,
    /// The conversation carries the overview: it was drawn.
    overview: bool,
    /// The planner changes the current plan the conversation carries.
    edit: bool,
}
/// Whether the text of an HTTP 400 (its message, and its parameter when it
/// names one) says the tools or the pictures are what was refused. Any other
/// 400, such as a context that is too long, would fail again without them.
fn refuses_tools_or_pictures(message: &str) -> bool {
    let message = message.to_lowercase();
    ["tool", "function", "image", "picture"]
        .iter()
        .any(|word| message.contains(word))
}
/// What a refused send should be made again without. A 400 that names only
/// pictures drops the pictures first, as it would fail again with the same
/// ones; any other refusal, and `Unsupported` with no text to go by, drops the
/// tools first. What is already off is skipped.
fn dropped_for(error: &ApiError, sight: &Sight) -> Option<Dropped> {
    let pictures_first = match error {
        ApiError::Unsupported(_) => false,
        ApiError::BadRequest(m) => {
            if !refuses_tools_or_pictures(m) {
                return None;
            }
            let m = m.to_lowercase();
            let names = |words: [&str; 2]| words.iter().any(|word| m.contains(word));
            names(["image", "picture"]) && !names(["tool", "function"])
        }
        _ => return None,
    };
    let order = if pictures_first {
        [Dropped::Pictures, Dropped::Tool]
    } else {
        [Dropped::Tool, Dropped::Pictures]
    };
    order.into_iter().find(|dropped| match dropped {
        Dropped::Tool => sight.tool,
        Dropped::Pictures => sight.pictures,
    })
}
/// One planner request: sends the conversation, answers the model's tool
/// calls, and returns the text of the first reply without calls. A send
/// OpenAI refuses for its tools or pictures is made again without what it
/// names: the tools, or the pictures when only they are named, and then the
/// other.
async fn ask(
    backend: &(impl Backend + Sync),
    sight: &mut Sight,
    conversation: &mut Vec<Item>,
    // `&mut`: a shared borrow of the !Sync mesh would make the planner's future non-Send.
    state: &mut ToolState,
    busy: &Busy,
    request: usize,
    progress: &mut (impl FnMut(Event) + Send),
) -> Result<String, ApiError> {
    state.next_request();
    let mut rounds = 0;
    loop {
        // Once the rounds are used up the tools stay offered, but may not be called.
        let choice = if rounds >= MAX_TOOL_ROUNDS {
            ToolChoice::None
        } else {
            ToolChoice::Auto
        };
        let report = &state.scene.report;
        let tools = if sight.tool {
            planner_tools::offered_for(
                sight.pictures,
                proposal::schema_with(sight.edit, report.rotary.is_some()),
            )
        } else {
            vec![]
        };
        let mut system = instructions_for(
            sight.pictures,
            sight.tool,
            sight.overview,
            report.rotary.is_some(),
        );
        if sight.edit {
            system.push_str(EDITING);
        }
        let mut messages = vec![Item::text(Role::System, system)];
        messages.extend(outgoing(conversation, sight.pictures, sight.tool));
        let reply = match backend.reply(&messages, &tools, choice, sight.edit).await {
            Ok(reply) => reply,
            Err(error) => {
                let Some(dropped) = dropped_for(&error, sight) else {
                    return Err(error);
                };
                match dropped {
                    Dropped::Tool => sight.tool = false,
                    Dropped::Pictures => sight.pictures = false,
                }
                progress(Event::Dropped(request, dropped));
                continue;
            }
        };
        if reply.calls.is_empty() {
            return Ok(reply.text);
        }
        rounds += 1;
        if rounds > MAX_TOOL_ROUNDS {
            return Err(ApiError::Other(
                "The model kept calling tools without replying with a plan.".into(),
            ));
        }
        let mut pictures = vec![];
        for (index, call) in reply.calls.into_iter().enumerate() {
            // The model sees an answer for every call it made; the ones past
            // the limit are not run.
            let answer = if index >= MAX_CALLS {
                planner_tools::over_the_limit()
            } else {
                // The tool works on a copy of the state (which shares the part
                // and the drafts), and a call that panics is caught there: the
                // copy comes back as the tool left it, so a draft check or a view
                // counts against its limits even when it fails (see
                // `planner_tools::answer`).
                let (name, arguments, mut copy) =
                    (call.name.clone(), call.arguments.clone(), state.clone());
                let answer = blocking(busy, move || {
                    let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        planner_tools::answer(&name, &arguments, &mut copy)
                    }));
                    (answer.ok(), copy)
                })
                .await;
                match answer {
                    Ok((answer, copy)) => {
                        *state = copy;
                        answer.unwrap_or_else(|| planner_tools::crashed(&call.name))
                    }
                    Err(_) => planner_tools::crashed(&call.name),
                }
            };
            conversation.push(Item::Call(call.clone()));
            conversation.push(Item::Output {
                call_id: call.call_id,
                output: answer.output,
            });
            // A line of the timeline: the picture with the model's reason, or
            // a sentence with no picture.
            progress(Event::Looked(
                request,
                Look {
                    description: answer.line,
                    purpose: answer
                        .picture
                        .as_ref()
                        .map(|p| p.purpose.clone())
                        .unwrap_or_default(),
                    png: answer.picture.as_ref().map(|p| p.png.clone()),
                },
            ));
            pictures.extend(answer.picture);
        }
        // One message holds this round's pictures, each after its description.
        if !pictures.is_empty() {
            let parts = pictures
                .into_iter()
                .flat_map(|p| {
                    [
                        Part::Text(p.description.clone()),
                        Part::Image {
                            png: p.png,
                            alt: p.description,
                        },
                    ]
                })
                .collect();
            conversation.push(Item::Message(Role::User, parts));
        }
    }
}
/// Runs the planner. `progress` is called as each stage starts and with each
/// check's result. The job and mesh are taken by value so the returned
/// future can move between threads.
#[cfg(test)]
pub async fn plan(
    backend: &(impl Backend + Sync),
    doc: Document,
    mesh: Mesh,
    busy: &Busy,
    progress: impl FnMut(Event) + Send,
) -> Result<Outcome, Failure> {
    plan_chat(backend, doc, mesh, Chat::default(), busy, progress).await
}
/// `plan` with what the user asked in the chat: a change to the job's
/// current plan when it has one (see [`editing`]), else what to plan.
pub async fn plan_chat(
    backend: &(impl Backend + Sync),
    doc: Document,
    mesh: Mesh,
    chat: Chat,
    busy: &Busy,
    progress: impl FnMut(Event) + Send,
) -> Result<Outcome, Failure> {
    plan_with_chat(
        backend,
        doc,
        mesh,
        chat,
        busy,
        progress,
        analysis::report,
        plan_check::evaluate_full,
    )
    .await
}
/// What the plan check of `plan_with` returns: the report, the job it
/// built and what the planner's tools read of it (`plan_check::evaluate_full`).
type Checking = (CheckReport, Option<Document>, Option<plan_check::Checked>);
/// `plan` with its two blocking stages given: the part report (`analyze`)
/// and the plan check (`check`, which also returns what the planner's tools
/// read of the checked job). Tests use it to hold a stage while it runs, or
/// to make a check fail to run.
#[cfg(test)]
async fn plan_with<A, C>(
    backend: &(impl Backend + Sync),
    doc: Document,
    mesh: Mesh,
    busy: &Busy,
    progress: impl FnMut(Event) + Send,
    analyze: A,
    check: C,
) -> Result<Outcome, Failure>
where
    A: FnOnce(&Document, &Mesh) -> Result<PartReport, String> + Send + 'static,
    C: Fn(&Document, &Mesh, &PartReport, &Proposal) -> Checking + Clone + Send + 'static,
{
    let chat = Chat::default();
    plan_with_chat(backend, doc, mesh, chat, busy, progress, analyze, check).await
}
/// `plan_with` with what the user asked in the chat.
#[allow(clippy::too_many_arguments)]
async fn plan_with_chat<A, C>(
    backend: &(impl Backend + Sync),
    doc: Document,
    mesh: Mesh,
    chat: Chat,
    busy: &Busy,
    mut progress: impl FnMut(Event) + Send,
    analyze: A,
    check: C,
) -> Result<Outcome, Failure>
where
    A: FnOnce(&Document, &Mesh) -> Result<PartReport, String> + Send + 'static,
    C: Fn(&Document, &Mesh, &PartReport, &Proposal) -> Checking + Clone + Send + 'static,
{
    let edit = editing(&doc, &chat);
    let real = doc;
    let doc = anonymized(&real);
    let fail = |message: String, problems, api, requests| Failure {
        message,
        problems,
        api,
        requests,
    };
    progress(Event::Stage(Progress::Analyzing));
    let (part_doc, part_mesh) = (doc.clone(), mesh.clone());
    let report = blocking(busy, move || analyze(&part_doc, &part_mesh))
        .await
        .map_err(|e| {
            fail(
                format!("The part analysis failed to run: {e}"),
                vec![],
                None,
                0,
            )
        })?
        .map_err(|e| fail(e, vec![], None, 0))?;
    backend.rotary(report.rotary.is_some());
    let part = serde_json::to_string(&report).map_err(|e| fail(e.to_string(), vec![], None, 0))?;
    // The plan to change, read before anything is sent.
    let current = if edit {
        let current = proposal::describe(&doc, &report).map_err(|e| fail(e, vec![], None, 0))?;
        Some(serde_json::to_string(&current).map_err(|e| fail(e.to_string(), vec![], None, 0))?)
    } else {
        None
    };
    // The system message is added to each send, as the pictures allow.
    let mut conversation = vec![Item::text(Role::User, part)];
    let mut state = ToolState::new(look::Scene {
        doc: Arc::new(doc.clone()),
        mesh: Arc::new(mesh.clone()),
        report: Arc::new(report.clone()),
        job: None,
    });
    // The tools are offered with or without pictures.
    let mut sight = Sight {
        pictures: backend.pictures(),
        tool: true,
        overview: false,
        edit,
    };
    if sight.pictures {
        let s = state.scene.clone();
        // Without its overview the run goes on: the report is what counts.
        if let Ok(Ok(picture)) = blocking(busy, move || look::overview(&s)).await {
            progress(Event::Overview(Look {
                description: picture.description.clone(),
                purpose: String::new(),
                png: Some(picture.png.clone()),
            }));
            conversation.push(Item::Message(
                Role::User,
                vec![
                    Part::Text(format!(
                        "The program's overview picture: {}",
                        picture.description
                    )),
                    Part::Image {
                        png: picture.png,
                        alt: picture.description,
                    },
                ],
            ));
            sight.overview = true;
        }
    }
    conversation.extend(chat.messages(current.as_deref()));
    let mut best: Option<Outcome> = None;
    let mut problems = vec![];
    let mut asked_for_coverage = false;
    let mut made = 0;
    let mut note = None;
    let mut stage: fn(usize) -> Progress = Progress::Asking;
    for request in 1..=MAX_REQUESTS {
        progress(Event::Stage(stage(request)));
        made = request;
        let asked = ask(
            backend,
            &mut sight,
            &mut conversation,
            &mut state,
            busy,
            request,
            &mut progress,
        );
        let text = match asked.await {
            Ok(text) => text,
            // A plan that already passed is still good if a later request
            // fails; the error is kept for the account and the preview.
            Err(error) if best.is_some() => {
                note = Some(error);
                break;
            }
            // The problems an earlier proposal failed with are kept.
            Err(error) => {
                return Err(fail(
                    error.message(),
                    bounded(problems.clone()),
                    Some(error),
                    request,
                ));
            }
        };
        let empty = text.trim().is_empty();
        let parsed = if empty {
            Err("The reply was empty.".to_string())
        } else {
            proposal::parse(json_object(&text))
        };
        if let Err(error) = &parsed {
            let unreadable = StepResult::Unreadable(proposal::clip(error, 300));
            progress(Event::Checked(request, unreadable));
        }
        let (proposal, more_warnings, more_unmachined) = match parsed {
            Ok(proposal) => tidy(proposal),
            Err(_) if best.is_some() => break,
            Err(error) => {
                problems = vec![Problem {
                    setup: 0,
                    operation: None,
                    stage: "format",
                    message: error.clone(),
                }];
                // The API may refuse an assistant message with no content.
                let echoed = if empty { "(empty reply)".into() } else { text };
                conversation.push(Item::text(Role::Assistant, echoed));
                conversation.push(Item::text(Role::User, format!("{error} Reply with the full proposal as one JSON object that matches the schema.")));
                stage = Progress::Repairing;
                continue;
            }
        };
        progress(Event::Stage(Progress::Checking(request)));
        let (doc, mesh, part, candidate, check) = (
            doc.clone(),
            mesh.clone(),
            report.clone(),
            proposal.clone(),
            check.clone(),
        );
        let checked = blocking(busy, move || check(&doc, &mesh, &part, &candidate)).await;
        let (check, job, checked) = match checked {
            Ok(checked) => checked,
            // A check that could not run says nothing against a plan in hand.
            Err(_) if best.is_some() => break,
            Err(e) => {
                return Err(fail(
                    format!("The plan check failed to run: {e}"),
                    bounded(problems.clone()),
                    None,
                    request,
                ));
            }
        };
        // This answer is the latest draft now, passed or not: operation
        // views draw its job, and `stock_after` and `check_holding` read it.
        state.scene.job = job.clone().map(Arc::new);
        state.checked = checked.map(Arc::new);
        progress(Event::Checked(request, StepResult::of(&check)));
        let next = feedback(&check);
        if let (true, Some(mut job)) = (check.passed, job) {
            // The job is the user's again: Apply keeps the real names of
            // setup 1 and of every setup a change kept and did not rename.
            job.title = real.title.clone();
            for setup in &mut job.setups {
                if let Some((i, user)) = real
                    .setups
                    .iter()
                    .enumerate()
                    .find(|(_, u)| u.id == setup.id)
                    && setup.name() == format!("Setup {}", i + 1)
                {
                    setup.p.insert("name".into(), user.name().into());
                }
            }
            let exposed = |c: &CheckReport| c.coverage.as_ref().map_or(0., |c| c.exposed_percent);
            let covered = check
                .coverage
                .as_ref()
                .is_some_and(|c| !c.covered_planes.is_empty() || !c.covered_other.is_empty());
            if best
                .as_ref()
                .is_none_or(|b| exposed(&check) >= exposed(&b.check))
            {
                best = Some(Outcome {
                    proposal,
                    check,
                    job,
                    report: report.clone(),
                    requests: request,
                    chosen: request,
                    more_warnings,
                    more_unmachined,
                    note: None,
                });
            }
            // One round to machine what is still covered; then the best plan
            // stands. A change asked for is not added to.
            if !covered || asked_for_coverage || edit {
                break;
            }
            asked_for_coverage = true;
            stage = Progress::Extending;
        } else {
            if best.is_some() {
                break;
            }
            problems = check.problems;
            stage = Progress::Repairing;
        }
        conversation.push(Item::text(Role::Assistant, text));
        conversation.push(Item::text(Role::User, next));
    }
    match best {
        Some(outcome) => Ok(Outcome {
            requests: made,
            note,
            ..outcome
        }),
        None => Err(fail(
            if edit {
                format!(
                    "The planner could not change the plan so that it passes every check in {MAX_REQUESTS} attempts."
                )
            } else {
                format!(
                    "The planner could not produce a plan that passes every check in {MAX_REQUESTS} attempts."
                )
            },
            bounded(problems),
            None,
            made,
        )),
    }
}

/// The signed-in ChatGPT plan as the planner's backend.
pub struct OpenAiBackend {
    client: std::sync::Arc<crate::openai::Client>,
    model: String,
    /// Set once the plan route refused structured output: later requests of
    /// the run ask for plain JSON at once.
    plain: std::sync::atomic::AtomicBool,
    /// The text of the current reply streamed so far, for the page.
    stream: Arc<std::sync::Mutex<String>>,
    /// Whether the run may show the model pictures.
    pictures: bool,
    /// Whether the run's machine has a rotary A axis (`Backend::rotary`).
    rotary: std::sync::atomic::AtomicBool,
}
impl OpenAiBackend {
    pub fn new(
        client: Arc<crate::openai::Client>,
        model: String,
        stream: Arc<std::sync::Mutex<String>>,
        pictures: bool,
    ) -> Self {
        Self {
            client,
            model,
            plain: false.into(),
            stream,
            pictures,
            rotary: false.into(),
        }
    }
}
impl Backend for OpenAiBackend {
    async fn reply(
        &self,
        messages: &[Item],
        tools: &[Tool],
        choice: ToolChoice,
        edit: bool,
    ) -> Result<Reply, ApiError> {
        use std::sync::atomic::Ordering;
        let request = crate::openai::Request {
            model: self.model.clone(),
            messages: messages.to_vec(),
            schema_name: "machining_plan",
            schema: proposal::schema_with(edit, self.rotary.load(Ordering::Relaxed)),
            tools: tools.to_vec(),
            tool_choice: choice,
        };
        if self.plain.load(Ordering::Relaxed) {
            let reply = self.client.respond_plain_streaming(&request, &self.stream);
            return reply.await;
        }
        let reply = self
            .client
            .respond_streaming(&request, &self.stream)
            .await?;
        if !reply.structured {
            self.plain.store(true, Ordering::Relaxed);
        }
        Ok(reply)
    }
    fn pictures(&self) -> bool {
        self.pictures
    }
    fn rotary(&self, rotary: bool) {
        self.rotary
            .store(rotary, std::sync::atomic::Ordering::Relaxed);
    }
}
/// The stock after the whole job, meshed at Standard detail for the
/// preview's 3D view.
pub fn final_view(job: &Document, mesh: &Mesh) -> Option<crate::simulation::Simulation> {
    final_view_at(job, mesh, crate::stock_surface::Detail::Standard)
}
/// `final_view` meshed at `detail`; tests use Draft, which is faster.
pub(crate) fn final_view_at(
    job: &Document,
    mesh: &Mesh,
    detail: crate::stock_surface::Detail,
) -> Option<crate::simulation::Simulation> {
    let si = job.setups.len().checked_sub(1)?;
    let oi = job.setups[si].operations.iter().rposition(|o| o.enabled)?;
    view_after(job, mesh, si, oi, detail)
}
/// The stock after operation `oi` of setup `si` of `job`, meshed at `detail`.
pub(crate) fn view_after(
    job: &Document,
    mesh: &Mesh,
    si: usize,
    oi: usize,
    detail: crate::stock_surface::Detail,
) -> Option<crate::simulation::Simulation> {
    let mut sim = crate::simulation::prepare(job, mesh, si, oi).ok()?;
    sim.seek(1.);
    let volume = sim.volume.as_ref()?;
    let surface = crate::stock_surface::Snapshot::of(volume, detail)
        .build()
        .ok()?;
    sim.stock_surface = Some(std::sync::Arc::new(surface));
    sim.stock_surface_version = volume.version;
    sim.stock_surface_detail = detail;
    sim.stock_surface_seconds = sim.seconds;
    Some(sim)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{Setup, get};
    use crate::openai::ToolCall;
    use crate::proposal::tests::{block, good};
    use std::sync::Mutex;
    /// Replies in order and records every conversation it was sent.
    pub struct Script {
        replies: Mutex<std::collections::VecDeque<Result<Reply, ApiError>>>,
        /// Each send's messages as role and text; calls and outputs are skipped.
        pub seen: Mutex<Vec<Vec<(Role, String)>>>,
        /// Each send's items and how the tools were offered: None when
        /// there were none.
        pub turns: Mutex<Vec<(Vec<Item>, Option<ToolChoice>)>>,
        /// The names of the tools each send offered.
        pub offered: Mutex<Vec<Vec<&'static str>>>,
        /// Whether each send was for changing the current plan.
        pub edits: Mutex<Vec<bool>>,
        /// The draft schema of each send's `check_plan` tool: None when it
        /// was not offered.
        pub check_schemas: Mutex<Vec<Option<serde_json::Value>>>,
        pictures: bool,
    }
    impl Script {
        /// Text replies, without pictures.
        pub fn new(replies: Vec<Result<String, ApiError>>) -> Self {
            let replies = replies.into_iter().map(|reply| {
                reply.map(|text| Reply {
                    text,
                    structured: true,
                    calls: vec![],
                })
            });
            Self {
                replies: Mutex::new(replies.collect()),
                seen: Mutex::new(vec![]),
                turns: Mutex::new(vec![]),
                offered: Mutex::new(vec![]),
                edits: Mutex::new(vec![]),
                check_schemas: Mutex::new(vec![]),
                pictures: false,
            }
        }
        /// Replies that may call the tools, with pictures on.
        pub fn calls(replies: Vec<Result<Reply, ApiError>>) -> Self {
            Self {
                pictures: true,
                ..Self::calling(replies)
            }
        }
        /// Replies that may call the tools, with pictures off.
        pub fn calling(replies: Vec<Result<Reply, ApiError>>) -> Self {
            Self {
                replies: Mutex::new(replies.into()),
                seen: Mutex::new(vec![]),
                turns: Mutex::new(vec![]),
                offered: Mutex::new(vec![]),
                edits: Mutex::new(vec![]),
                check_schemas: Mutex::new(vec![]),
                pictures: false,
            }
        }
    }
    impl Backend for Script {
        async fn reply(
            &self,
            messages: &[Item],
            tools: &[Tool],
            choice: ToolChoice,
            edit: bool,
        ) -> Result<Reply, ApiError> {
            self.edits.lock().unwrap().push(edit);
            self.check_schemas.lock().unwrap().push(
                tools
                    .iter()
                    .find(|t| t.name == "check_plan")
                    .map(|t| t.parameters["properties"]["proposal"].clone()),
            );
            let texts = messages.iter().filter_map(Item::as_text).collect();
            self.seen.lock().unwrap().push(texts);
            let offered = (!tools.is_empty()).then_some(choice);
            self.turns
                .lock()
                .unwrap()
                .push((messages.to_vec(), offered));
            self.offered
                .lock()
                .unwrap()
                .push(tools.iter().map(|t| t.name).collect());
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Err(ApiError::Other("script exhausted".into())))
        }
        fn pictures(&self) -> bool {
            self.pictures
        }
    }
    /// The tools offered with or without pictures, in order.
    const PART_TOOLS: [&str; 5] = [
        "check_plan",
        "stock_after",
        "check_holding",
        "cutting_data",
        "surface_info",
    ];
    fn json(proposal: &Proposal) -> Result<String, ApiError> {
        Ok(serde_json::to_string(proposal).unwrap())
    }
    /// The stages of a run's events, in order.
    fn stages(events: &[Event]) -> Vec<Progress> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Stage(stage) => Some(*stage),
                _ => None,
            })
            .collect()
    }
    fn bad(report: &PartReport) -> Proposal {
        let mut bad = good(report);
        bad.setups[0].operations[0].tool = 9;
        bad
    }
    #[tokio::test]
    async fn a_good_first_proposal_gets_one_coverage_round_and_the_best_plan_wins() {
        let (doc, mesh, report) = block();
        let mut smaller = good(&report);
        smaller.setups.truncate(1);
        let script = Script::new(vec![json(&smaller), json(&good(&report))]);
        let mut events = vec![];
        let outcome = plan(&script, doc.clone(), mesh.clone(), &Busy::default(), |e| {
            events.push(e)
        })
        .await
        .unwrap();
        assert_eq!((outcome.requests, outcome.chosen), (2, 2));
        assert_eq!(outcome.proposal, good(&report));
        assert_eq!(outcome.job.setups.len(), 2);
        assert!(outcome.check.passed);
        assert_eq!(
            stages(&events),
            vec![
                Progress::Analyzing,
                Progress::Asking(1),
                Progress::Checking(1),
                Progress::Extending(2),
                Progress::Checking(2)
            ]
        );
        let seen = script.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[1].len(), 4);
        assert_eq!(seen[1][2].0, Role::Assistant);
        assert!(
            seen[1][3].1.starts_with("The proposal passes every check.")
                && seen[1][3].1.contains("\"covered_planes\"")
        );
    }
    #[tokio::test]
    async fn the_first_request_carries_the_report_and_no_private_data() {
        let (mut doc, mesh, report) = block();
        doc.title = "Secret customer bracket".into();
        let script = Script::new(vec![json(&good(&report)), json(&good(&report))]);
        plan(&script, doc.clone(), mesh.clone(), &Busy::default(), |_| {})
            .await
            .unwrap();
        let seen = script.seen.lock().unwrap();
        let (system, user) = (&seen[0][0], &seen[0][1]);
        assert_eq!((system.0, user.0), (Role::System, Role::User));
        for needed in [
            "Face",
            "Pocket",
            "Contour",
            "Drill",
            "Parallel",
            "Waterline",
            "unmachined",
            "stickout",
        ] {
            assert!(system.1.contains(needed), "{needed}");
        }
        let sent: serde_json::Value = serde_json::from_str(&user.1).unwrap();
        assert_eq!(sent["orientations"].as_array().unwrap().len(), 6);
        assert_eq!(sent["tools"][0]["number"], 1);
        let all: String = seen
            .iter()
            .flatten()
            .map(|(_, text)| text.as_str())
            .collect();
        let path = doc.file.as_ref().unwrap().path.to_str().unwrap().to_owned();
        for private in [
            "Secret customer bracket",
            path.as_str(),
            "four-setup-block",
            &mesh.fingerprint,
        ] {
            assert!(!all.contains(private), "{private}");
        }
    }
    #[tokio::test]
    async fn correction_rounds_send_no_title_setup_name_or_file_name() {
        let (mut doc, mesh, report) = block();
        doc.title = "Secret customer bracket".into();
        doc.setups[0]
            .p
            .insert("name".into(), "ZZ-SECRET-FIXTURE".into());
        doc.setups[0]
            .machine
            .p
            .insert("post_air".into(), "Not available".into());
        // The first proposal fails generation, the second the export stage.
        let mut generation = good(&report);
        generation.setups[0].operations[1].stepover_percent = Some(80.);
        let mut export = good(&report);
        export.setups[0].operations[0].rpm = 8000.5;
        export.setups[1].operations[0].coolant = "Air".into();
        let script = Script::new(vec![
            json(&generation),
            json(&export),
            json(&good(&report)),
            json(&good(&report)),
        ]);
        let outcome = plan(&script, doc.clone(), mesh.clone(), &Busy::default(), |_| {})
            .await
            .unwrap();
        assert_eq!(outcome.job.setups[0].name(), "ZZ-SECRET-FIXTURE");
        assert_eq!(outcome.job.title, "Secret customer bracket");
        let seen = script.seen.lock().unwrap();
        assert!(seen[1][3].1.contains("\"stage\":\"generation\""));
        assert!(seen[2][5].1.contains("\"stage\":\"export\""));
        let all: String = seen
            .iter()
            .flatten()
            .map(|(_, text)| text.as_str())
            .collect();
        let file = doc.file.as_ref().unwrap();
        for private in [
            "Secret customer bracket",
            "ZZ-SECRET-FIXTURE",
            file.name.as_str(),
            file.path.to_str().unwrap(),
            ".ngc",
            &mesh.fingerprint,
        ] {
            assert!(!all.contains(private), "{private}");
        }
    }
    #[test]
    fn the_instructions_state_the_limits_the_program_enforces() {
        let text = instructions();
        let limit = crate::cam::CEILING;
        for needed in [
            // Setups and orientations.
            "Each orientation may be used by one setup only".to_string(),
            "at most 6 setups".into(),
            format!(
                "At most {} operations per setup and {} in total",
                proposal::MAX_SETUP_OPERATIONS,
                proposal::MAX_OPERATIONS
            ),
            // Numbers.
            "Every number must be finite".into(),
            "cannot be negative".into(),
            "`finish_passes` a whole number 0-20".into(),
            "`ramp_angle` null or 0.1-15".into(),
            // Face, Contour, Drill, Parallel and Waterline rules.
            "`stock_margin[0]` is greater than `bottom_offset`".into(),
            "`facing_area`".into(),
            "Contour, `contour_mode` `outside`".into(),
            "The tool and depth rules of Pocket apply".into(),
            "not `opening_diameter`".into(),
            "the countersink's `angle`".into(),
            "measured from the stock top".into(),
            "`through` may be true only in setup 1".into(),
            "null means 1 mm".into(),
            "half the drill diameter plus 1 mm".into(),
            "`stock_margin[1]`".into(),
            format!(
                "{},{:03},{:03}",
                limit / 1_000_000,
                limit / 1000 % 1000,
                limit % 1000
            ),
            "√(8 × tool radius × `tolerance`)".into(),
            "at least 0.05 mm".into(),
            "layers, its moves about layers × the outline's points".into(),
            "`surface_stepover` × tan θ".into(),
            // The report and the check report.
            "`truncated`".into(),
            "setup 1's frame".into(),
        ] {
            assert!(text.contains(&needed), "{needed}");
        }
        // Contour's stepdown limit sits in its own line.
        let contour = text
            .lines()
            .find(|l| l.contains("`contour_mode` `outside`"))
            .unwrap();
        assert!(contour.contains("`stepdown` at most the flute length"));
    }
    #[test]
    fn the_instructions_explain_the_workholding_boxes_and_the_gap() {
        let text = instructions();
        for needed in [
            "\nWORKHOLDING\n- `workholding` in the part report, when present, describes",
            "keep-out zones",
            "grows each box by `gap` mm",
            "from the model's -X/-Y/bottom corner",
            "later setups",
            "laid out by the program",
            "`clearance_above_stock_top`",
            "`clearance` greater than it",
            "already covers a height below 15",
            "When the height is 15 or more",
            "work coordinates (X, Y and Z measured from the setup's work zero",
            "It can only be raised, never below 15 and never above the machine's Z travel (`machine.travel`, its third value; 300 when that is 0).",
            "above the jaws' top",
            "stage `workholding`",
            "are not checked",
        ] {
            assert!(text.contains(needed), "{needed}");
        }
        // The old line that said clamps are not modelled is gone.
        assert!(!text.contains("Clamps are not modelled"));
    }
    /// The text of the section called `title`: from its heading to the
    /// blank line that ends it.
    fn section(text: &str, title: &str) -> String {
        let at = text
            .find(&format!("\n{title}\n"))
            .unwrap_or_else(|| panic!("no {title} section"))
            + 1;
        let end = text[at..].find("\n\n").map_or(text.len(), |n| at + n);
        text[at..end].to_string()
    }
    #[test]
    fn the_instructions_teach_planning_the_holding_before_any_operation() {
        let text = instructions();
        let at = |title: &str| text.find(&format!("\n{title}\n")).unwrap();
        // After WORKHOLDING, which describes setup 1's boxes, and before the rules.
        assert!(at("WORKHOLDING") < at("PLANNING THE HOLDING"));
        assert!(at("PLANNING THE HOLDING") < at("SPEEDS AND FEEDS"));
        let holding = section(&text, "PLANNING THE HOLDING");
        for needed in [
            "before you plan any operation",
            "the stock the earlier setups left",
            "a faced top makes the flipped stock shorter",
            "cut sides make it narrower",
            "`workholding_library`",
            "`number`",
            "`holder`",
            "`max_opening`",
            "`grip`",
            "`bite`",
            "null",
            "Setup 1",
            "material at both jaws",
            "up to the grip",
            "the bottom",
            "at least 1 mm above the grip",
            "material under each clamp's bite",
            "`bottom_offset`",
            "in a later setup",
            "stage `workholding`",
            "names the earlier operation",
            // The vise closes on its grip band: walls finished through it
            // can be gripped, sides cut through part of it cannot.
            "A vise closes on the material in its grip band",
            "the stock is as wide as that band",
            "Walls an earlier setup cut through the whole band",
            "can be gripped",
            "only part of the band cannot",
            "material under the band to rest on",
            "a band no wider across the jaws than `max_opening`",
            "A later setup turned over grips what was their top",
            "never part of the way",
        ] {
            assert!(holding.contains(needed), "{needed}\n{holding}");
        }
        // Gone: advice the band contradicts, and a facing rule that was never
        // one (a faced top is what a flipped setup rests on).
        for gone in [
            "do not cut the sides a vise will grip",
            "do not face away the top",
        ] {
            assert!(!text.contains(gone), "{gone}");
        }
        let workholding = section(&text, "WORKHOLDING");
        assert!(!workholding.contains("(finish them from a later setup)"));
        assert!(workholding.contains("finish the walls below from the setup turned over, whose vise can grip the walls finished above"));
        // The tools are named only when they are offered.
        for tool in ["check_plan", "stock_after", "check_holding"] {
            assert!(!text.contains(tool), "{tool}");
            assert!(holding_rules(true).contains(tool), "{tool}");
            assert!(!holding_rules(false).contains(tool), "{tool}");
        }
        assert_eq!(holding_rules(false), format!("{holding}\n\n"));
        // The report's key is explained, and WORKHOLDING no longer says every
        // later setup gets setup 1's entry.
        assert!(text.contains("- `workholding_library` lists"));
        let workholding = section(&text, "WORKHOLDING");
        assert!(!workholding.contains("get the same kind of workholding and library entry"));
        assert!(workholding.contains("`holder`") && workholding.contains("null"));
        assert!(workholding.contains("laid out by the program"));
    }
    #[tokio::test]
    async fn the_report_and_the_workholding_problems_carry_no_entry_name_or_note() {
        use crate::proposal::tests::hold;
        let (mut doc, mesh, _) = block();
        for key in ["stock_left", "stock_right", "stock_front", "stock_back"] {
            doc.setups[0].p.insert(key.into(), "36".into());
        }
        hold(&mut doc, "Clamp", &[("clamps", "-X:0;+X:0")]);
        for h in &mut doc.workholding {
            h.p.insert("name".into(), "ZZ-SECRET-HOLDER".into());
            h.p.insert("notes".into(), "ZZ-SECRET-NOTE".into());
        }
        doc.setups[0]
            .p
            .insert("notes".into(), "ZZ-SECRET-NOTES".into());
        let report = analysis::report(&doc, &mesh).unwrap();
        // Facing the whole stock runs through the clamps; facing the model's
        // outline stays clear of them.
        let through = good(&report);
        let mut clear = good(&report);
        for setup in &mut clear.setups {
            setup.operations[0].facing_area = Some("model".into());
        }
        let script = Script::new(vec![json(&through), json(&clear), json(&clear)]);
        let outcome = plan(&script, doc.clone(), mesh.clone(), &Busy::default(), |_| {}).await;
        assert!(outcome.is_ok(), "{:?}", outcome.err());
        let seen = script.seen.lock().unwrap();
        // The report names the kind, the boxes and the gap, and that is all.
        let part = &seen[0][1].1;
        assert!(
            part.contains("\"workholding\":{\"kind\":\"Clamps\",\"number\":2,\"boxes\":[["),
            "{part}"
        );
        assert!(part.contains("\"gap\":2.0"), "{part}");
        // The library is told by number and size: both starters, no name.
        assert!(
            part.contains("\"workholding_library\":[{\"number\":1,\"kind\":\"Vise\",\"grip\":8.0,"),
            "{part}"
        );
        assert!(part.contains("{\"number\":2,\"kind\":\"Clamp\","), "{part}");
        assert!(seen[1][3].1.contains("\"stage\":\"workholding\""));
        let all: String = seen
            .iter()
            .flatten()
            .map(|(_, text)| text.as_str())
            .collect();
        for private in ["ZZ-SECRET-HOLDER", "ZZ-SECRET-NOTE", "ZZ-SECRET-NOTES"] {
            assert!(!all.contains(private), "{private}");
        }
    }
    #[test]
    fn the_instructions_explain_the_speed_and_feed_check() {
        let text = instructions();
        for needed in [
            "SPEEDS AND FEEDS",
            "surface speed",
            "chip load",
            "Vc = π × D × rpm ÷ 1000",
            "fz = feed ÷ (rpm × flutes)",
            "`vc_min_m_per_min`",
            "`chip_load_mm`",
            "`up_to_mm`",
            "`flutes`",
            "Drills are checked like end mills",
            "feed ÷ (rpm × flutes)",
            "maximum rpm",
            "maximum feed",
            "outside the material's cutting data are refused",
            "`cutting data`",
        ] {
            assert!(text.contains(needed), "{needed}");
        }
        assert!(text.contains('π') || text.contains("pi"));
        // The older sentence about conservative values is replaced by the check.
        assert!(!text.contains("Choose conservative spindle speeds"));
        // The example in the text is what the program computes.
        assert!(text.contains(
            "a 3-flute Ø10 mm tool at 8000 rpm and 1000 mm/min runs at Vc = 251 m/min with fz = 0.042 mm per tooth"
        ));
        let (doc, _, _) = block();
        let tool = &doc.tools[0];
        let mut op = crate::model::Operation::new(crate::model::Strategy::Pocket, tool.id.clone());
        op.p.insert("rpm".into(), "8000".into());
        op.p.insert("feed".into(), "1000".into());
        let crate::material::SpeedFeed::InRange { vc, fz } = crate::material::check(
            doc.selected_material().unwrap(),
            tool,
            &op,
            &doc.setups[0].machine,
        ) else {
            panic!("the example is in range")
        };
        assert_eq!((vc.round(), (fz * 1000.).round() / 1000.), (251., 0.042));
    }
    #[test]
    fn a_parallel_stepover_the_old_sample_limit_refused_now_generates() {
        // At 0.5 mm on the 120 × 80 mm fixture: some 38,000 samples, past
        // the 30,000 the preview once allowed.
        let (doc, mesh, report) = block();
        let generate = |stepover: f32| {
            let mut proposal = good(&report);
            let mut parallel = proposal::tests::op("Parallel", 2, vec![]);
            parallel.surface_stepover = Some(stepover);
            parallel.tolerance = Some(0.02);
            proposal.setups[0].operations.push(parallel);
            let job = proposal::build(&doc, &mesh, &report, &proposal).unwrap();
            let setup = &job.setups[0];
            let op = setup.operations.last().unwrap();
            let tool = job.tools.iter().find(|t| t.id == op.tool).unwrap();
            crate::cam::generate(setup, op, tool, &mesh)
        };
        assert!(!instructions().contains("On a 120 × 80 mm part with a Ø6 ball end mill"));
        assert!(generate(0.6).is_ok());
        assert!(generate(0.5).is_ok());
    }
    /// Builds `edit`'s change to the passing plan and generates operation
    /// `oi` of setup 1 with the stock the operations before it leave, as the
    /// plan check does.
    fn generated_in_setup_1(
        fixture: &(Document, Mesh, PartReport),
        edit: impl FnOnce(&mut Proposal) -> usize,
    ) -> Result<(), String> {
        moves_in_setup_1(fixture, edit).map(|_| ())
    }
    /// `generated_in_setup_1`, returning how many moves the operation makes.
    fn moves_in_setup_1(
        (doc, mesh, report): &(Document, Mesh, PartReport),
        edit: impl FnOnce(&mut Proposal) -> usize,
    ) -> Result<usize, String> {
        let mut proposal = good(report);
        let oi = edit(&mut proposal);
        let job = proposal::build(doc, mesh, report, &proposal).map_err(|p| format!("{p:?}"))?;
        crate::simulation::prepare(&job, mesh, 0, oi).map(|s| s.plan.moves.len())
    }
    #[test]
    fn fine_stepdowns_the_old_move_limit_refused_now_generate() {
        let fixture = block();
        let text = instructions();
        assert!(text.contains("Every operation is limited to 5,000,000 moves"));
        assert!(!text.contains("keep `stepdown` at or above about 0.5 mm"));
        // Pocket, outside Contour and Waterline at the stepdowns the 30,000
        // move limit refused on the fixture.
        let stepdown = |strategy: &'static str, value: f32| {
            generated_in_setup_1(&fixture, |p| {
                let ops = &mut p.setups[0].operations;
                let oi = match strategy {
                    "Pocket" => 1,
                    "Contour" => 2,
                    _ => {
                        ops.push(proposal::tests::op("Waterline", 1, vec![]));
                        ops.len() - 1
                    }
                };
                ops[oi].stepdown = Some(value);
                oi
            })
        };
        for (strategy, refused) in [("Pocket", 0.2), ("Contour", 0.3), ("Waterline", 0.4)] {
            assert_eq!(stepdown(strategy, refused), Ok(()), "{strategy} {refused}");
        }
    }
    #[test]
    fn finish_passes_ramp_and_stepover_multiply_pocket_and_inside_floor_moves() {
        let fixture = block();
        let text = instructions();
        let line = |start: &str| {
            text.lines()
                .find(|l| l.starts_with(start))
                .unwrap_or_else(|| panic!("{start}"))
                .to_string()
        };
        let pocket = line("- Pocket:");
        let inside = line("- Contour, `contour_mode` `inside_floor`:");
        for l in [&pocket, &inside] {
            assert!(
                l.contains("each finish pass repeats the outermost ring at every level"),
                "{l}"
            );
            assert!(
                l.contains("a smaller `ramp_angle` adds ramp moves at every entry (the tool stays down from ring to ring and enters again only for the deepest loop of each separate spot)"),
                "{l}"
            );
        }
        assert!(pocket.contains("a smaller `stepover_percent` adds rings"));
        assert!(
            inside.contains("its stepover is fixed at 40 %, so `stepover_percent` changes nothing")
        );
        // On the fixture, with Face first, at a 0.5 mm stepdown: a Pocket, or
        // an inside-floor Contour of the same floor after it.
        let at_half = |inside_floor: bool, edit: &dyn Fn(&mut proposal::ProposedOp)| {
            generated_in_setup_1(&fixture, |p| {
                let ops = &mut p.setups[0].operations;
                let oi = if inside_floor {
                    let mut walls = proposal::tests::op("Contour", 1, ops[1].surfaces.clone());
                    walls.contour_mode = Some("inside_floor".into());
                    ops[2] = walls;
                    2
                } else {
                    1
                };
                ops[oi].stepdown = Some(0.5);
                edit(&mut ops[oi]);
                oi
            })
        };
        // The finish passes that went over the old 30,000 move limit
        // generate now, as does every ramp angle.
        for (inside_floor, finish) in [(false, 4), (true, 5)] {
            let name = if inside_floor {
                "inside floor"
            } else {
                "Pocket"
            };
            assert_eq!(at_half(inside_floor, &|_| {}), Ok(()), "{name}");
            assert_eq!(
                at_half(inside_floor, &|o| o.finish_passes = Some(finish - 1)),
                Ok(()),
                "{name}, {} finish passes",
                finish - 1
            );
            assert_eq!(
                at_half(inside_floor, &|o| o.ramp_angle = Some(1.)),
                Ok(()),
                "{name}, ramp 1°"
            );
            assert_eq!(
                at_half(inside_floor, &|o| o.finish_passes = Some(finish)),
                Ok(()),
                "{name}, {finish} finish passes"
            );
            assert_eq!(
                at_half(inside_floor, &|o| o.ramp_angle = Some(0.1)),
                Ok(()),
                "{name}, ramp 0.1°"
            );
        }
        // A smaller stepover adds Pocket rings; an inside-floor Contour keeps 40 %.
        assert_eq!(at_half(false, &|o| o.stepover_percent = Some(12.)), Ok(()));
        assert_eq!(at_half(false, &|o| o.stepover_percent = Some(10.)), Ok(()));
        assert_eq!(
            at_half(true, &|o| o.stepover_percent = Some(10.)),
            Ok(()),
            "stepover changes nothing for an inside-floor Contour"
        );
    }
    #[test]
    fn the_facing_peck_and_setup_rules_in_the_instructions_hold_on_the_fixture() {
        let fixture = block();
        let report = &fixture.2;
        let text = instructions();
        // Face and drilling: the strokes and pecks the old limits refused
        // generate, and the instructions no longer set them.
        assert!(text.contains("Face makes levels × passes strokes: levels"));
        assert!(!text.contains("at most 2,000") && !text.contains("÷ 1,000"));
        let o = &report.orientations[0];
        let face = generated_in_setup_1(&fixture, |p| {
            let face = &mut p.setups[0].operations[0];
            face.stepdown = Some(0.38);
            face.stepover_percent = Some(5.);
            0
        });
        assert_eq!(face, Ok(()), "Face at 0.38 mm and 5 %");
        let top = o.planes.iter().find(|p| p.kind == "top").unwrap();
        let deepest = top.holes.iter().map(|h| h.depth).fold(0., f32::max);
        let depth = o.stock_margin[0] + top.depth_below_top + deepest;
        let pecks = generated_in_setup_1(&fixture, |p| {
            let ops = &mut p.setups[0].operations;
            let mut drill = ops.remove(3);
            drill.peck = Some(depth / 1000. * 0.98);
            ops.insert(0, drill);
            0
        });
        assert_eq!(pecks, Ok(()), "over 1,000 pecks a hole");
        // Every setup needs at least one operation.
        assert!(text.contains("Every setup needs at least one operation."));
        let mut empty = good(report);
        empty.setups[1].operations.clear();
        let problems = proposal::build(&fixture.0, &fixture.1, report, &empty).unwrap_err();
        assert!(
            problems
                .iter()
                .any(|p| p.setup == 2 && p.message == "The setup has no operations."),
            "{problems:?}"
        );
        // Only a through drill's breakthrough goes below the model.
        assert!(!text.contains("never cuts below the model's bottom"));
        assert!(text.contains(
            "Only a through drill's breakthrough in setup 1 goes below the model, into the stock beneath it."
        ));
    }
    #[test]
    fn a_waterline_past_the_old_300_layers_generates() {
        // A 20 mm high box: 301 layers, once refused.
        let (doc, mesh, report) = open_box();
        let mut proposal = complete(&report);
        let mut op = proposal::tests::op("Waterline", 1, vec![]);
        op.stepdown = Some(20. / 300.5);
        proposal.setups[0].operations.push(op);
        let job = proposal::build(&doc, &mesh, &report, &proposal).unwrap();
        let levels = crate::simulation::prepare(&job, &mesh, 0, 2)
            .unwrap()
            .plan
            .levels;
        assert_eq!(levels, 301);
    }
    #[tokio::test]
    async fn a_failed_check_is_sent_back_and_the_repair_is_accepted() {
        let (doc, mesh, report) = block();
        let script = Script::new(vec![
            json(&bad(&report)),
            json(&good(&report)),
            json(&good(&report)),
        ]);
        let mut events = vec![];
        let outcome = plan(&script, doc.clone(), mesh.clone(), &Busy::default(), |e| {
            events.push(e)
        })
        .await
        .unwrap();
        assert_eq!(outcome.requests, 3, "repair, then one coverage round");
        assert_eq!(outcome.chosen, 3);
        assert_eq!(
            stages(&events),
            vec![
                Progress::Analyzing,
                Progress::Asking(1),
                Progress::Checking(1),
                Progress::Repairing(2),
                Progress::Checking(2),
                Progress::Extending(3),
                Progress::Checking(3)
            ]
        );
        let seen = script.seen.lock().unwrap();
        let repair = &seen[1][3].1;
        assert!(repair.starts_with("The proposal did not pass."));
        assert!(
            repair.contains("T9 is not in the tool library")
                && repair.contains("\"stage\":\"structure\"")
        );
    }
    #[tokio::test]
    async fn a_speed_outside_the_materials_data_is_sent_back_and_the_repair_is_accepted() {
        let (doc, mesh, report) = block();
        let mut fast = good(&report);
        fast.setups[0].operations[0].rpm = 2000.;
        let script = Script::new(vec![
            json(&fast),
            json(&good(&report)),
            json(&good(&report)),
        ]);
        let outcome = plan(&script, doc.clone(), mesh.clone(), &Busy::default(), |_| {})
            .await
            .unwrap();
        assert!(outcome.check.passed);
        assert_eq!(outcome.proposal, good(&report));
        let seen = script.seen.lock().unwrap();
        // The model sees the material's data in the report and the problem,
        // by setup and operation, in the correction.
        assert!(seen[0][1].1.contains("\"vc_min_m_per_min\":150.0"));
        let repair = &seen[1][3].1;
        assert!(repair.starts_with("The proposal did not pass."));
        assert!(
            repair.contains("\"stage\":\"cutting data\"")
                && repair.contains("\"setup\":1,\"operation\":1,")
                && repair
                    .contains("Surface speed 63 m/min is below Aluminium 6061-T6's 150–300 m/min.")
        );
    }
    #[tokio::test]
    async fn four_failures_end_the_run_with_the_last_problems() {
        let (doc, mesh, report) = block();
        let script = Script::new(vec![
            json(&bad(&report)),
            Ok("Here is my plan!".into()),
            json(&bad(&report)),
            Ok("oops".into()),
            json(&good(&report)),
        ]);
        let mut events = vec![];
        let failure = plan(&script, doc.clone(), mesh.clone(), &Busy::default(), |e| {
            events.push(e)
        })
        .await
        .unwrap_err();
        assert_eq!(failure.requests, 4);
        assert!(failure.message.contains("could not produce a plan"));
        // The last reply's problem, not the first's.
        assert_eq!(failure.problems.len(), 1);
        assert_eq!(failure.problems[0].stage, "format");
        assert!(failure.problems[0].message.contains("not valid JSON"));
        assert_eq!(failure.api, None);
        assert_eq!(
            stages(&events),
            vec![
                Progress::Analyzing,
                Progress::Asking(1),
                Progress::Checking(1),
                Progress::Repairing(2),
                Progress::Repairing(3),
                Progress::Checking(3),
                Progress::Repairing(4)
            ]
        );
        let seen = script.seen.lock().unwrap();
        assert_eq!(seen.len(), 4);
        assert!(
            seen[2][5].1.contains("not valid JSON")
                && seen[2][5].1.contains("Reply with the full proposal")
        );
    }
    #[tokio::test]
    async fn a_run_reports_each_check_result() {
        let (doc, mesh, report) = block();
        let script = Script::new(vec![
            json(&bad(&report)),
            json(&good(&report)),
            json(&good(&report)),
        ]);
        let mut events = vec![];
        plan(&script, doc, mesh, &Busy::default(), |e| events.push(e))
            .await
            .unwrap();
        let problems = StepResult::Problems {
            shown: vec!["Setup 1, operation 1: T9 is not in the tool library.".into()],
            more: 0,
        };
        // Each result follows its check's stage and comes before the next stage.
        assert_eq!(
            events,
            vec![
                Event::Stage(Progress::Analyzing),
                Event::Stage(Progress::Asking(1)),
                Event::Stage(Progress::Checking(1)),
                Event::Checked(1, problems),
                Event::Stage(Progress::Repairing(2)),
                Event::Stage(Progress::Checking(2)),
                Event::Checked(2, StepResult::Passed),
                Event::Stage(Progress::Extending(3)),
                Event::Stage(Progress::Checking(3)),
                Event::Checked(3, StepResult::Passed),
            ]
        );
    }
    #[tokio::test]
    async fn an_unreadable_reply_is_reported() {
        let (doc, mesh, report) = block();
        let script = Script::new(vec![Ok("Here is my plan!".into()), json(&good(&report))]);
        let mut events = vec![];
        plan(&script, doc.clone(), mesh.clone(), &Busy::default(), |e| {
            events.push(e)
        })
        .await
        .unwrap();
        let Some(Event::Checked(1, StepResult::Unreadable(why))) = events
            .iter()
            .find(|e| matches!(e, Event::Checked(1, _)))
            .cloned()
        else {
            panic!("{events:?}")
        };
        assert!(
            why.contains("not valid JSON") && why.chars().count() <= 300,
            "{why}"
        );
        assert_eq!(
            events[..4],
            [
                Event::Stage(Progress::Analyzing),
                Event::Stage(Progress::Asking(1)),
                Event::Checked(1, StepResult::Unreadable(why)),
                Event::Stage(Progress::Repairing(2)),
            ]
        );
        assert!(events.contains(&Event::Checked(2, StepResult::Passed)));
        // A reply that cannot be read after a plan passed is reported too.
        let script = Script::new(vec![json(&good(&report)), Ok("oops".into())]);
        let mut events = vec![];
        plan(&script, doc, mesh, &Busy::default(), |e| events.push(e))
            .await
            .unwrap();
        assert!(events.contains(&Event::Checked(1, StepResult::Passed)));
        assert!(
            matches!(
                events.last(),
                Some(Event::Checked(2, StepResult::Unreadable(_)))
            ),
            "{events:?}"
        );
        // The model's text is bounded: a long reply's error says at most 300 characters.
        let long = format!("{{\"summary\": \"{}", "x".repeat(2000));
        let script = Script::new(vec![Ok(long), Err(ApiError::UsageLimit)]);
        let (doc, mesh, _) = block();
        let mut events = vec![];
        let _ = plan(&script, doc, mesh, &Busy::default(), |e| events.push(e)).await;
        let Some(Event::Checked(1, StepResult::Unreadable(why))) =
            events.iter().find(|e| matches!(e, Event::Checked(1, _)))
        else {
            panic!("{events:?}")
        };
        assert!(why.chars().count() <= 300 && !why.contains('\n'), "{why}");
    }
    #[tokio::test]
    async fn a_check_result_lists_five_problems_and_counts_the_rest() {
        let (doc, mesh, report) = block();
        let script = Script::new(vec![json(&good(&report))]);
        let mut events = vec![];
        let failure = plan_with(
            &script,
            doc,
            mesh,
            &Busy::default(),
            |e| events.push(e),
            analysis::report,
            |_: &Document, _: &Mesh, _: &PartReport, _: &Proposal| {
                let problems = (1..=8)
                    .map(|i| Problem {
                        setup: 1,
                        operation: Some(i),
                        stage: "generation",
                        message: format!("Problem {i} {}", "x".repeat(400)),
                    })
                    .collect();
                let report = CheckReport {
                    passed: false,
                    problems,
                    operations: vec![],
                    coverage: None,
                    total_minutes: 0.,
                    lathe_minutes: 0.,
                    manual_minutes: 0.,
                    cnc_minutes: 0.,
                    move_minutes: 0.,
                    zeros: vec![],
                    stocks: vec![],
                };
                (report, None, None)
            },
        )
        .await
        .unwrap_err();
        assert_eq!(failure.problems.len(), 8, "the failure keeps every problem");
        let Some(Event::Checked(1, StepResult::Problems { shown, more })) =
            events.iter().find(|e| matches!(e, Event::Checked(1, _)))
        else {
            panic!("{events:?}")
        };
        assert_eq!((shown.len(), *more), (5, 3));
        let place = "Setup 1, operation 1: ".len();
        assert!(shown[0].starts_with("Setup 1, operation 1: Problem 1 xxx"));
        for line in shown {
            assert_eq!(line.chars().count(), place + 300, "{line}");
        }
        // Exactly five problems leave nothing to count.
        let script = Script::new(vec![json(&good(&report))]);
        let (doc, mesh, _) = block();
        let mut events = vec![];
        let _ = plan_with(
            &script,
            doc,
            mesh,
            &Busy::default(),
            |e| events.push(e),
            analysis::report,
            |_: &Document, _: &Mesh, _: &PartReport, _: &Proposal| {
                let problems = (0..5)
                    .map(|_| Problem {
                        setup: 0,
                        operation: None,
                        stage: "format",
                        message: "Bad.".into(),
                    })
                    .collect();
                let report = CheckReport {
                    passed: false,
                    problems,
                    operations: vec![],
                    coverage: None,
                    total_minutes: 0.,
                    lathe_minutes: 0.,
                    manual_minutes: 0.,
                    cnc_minutes: 0.,
                    move_minutes: 0.,
                    zeros: vec![],
                    stocks: vec![],
                };
                (report, None, None)
            },
        )
        .await;
        let found = events.iter().find_map(|e| match e {
            Event::Checked(1, StepResult::Problems { shown, more }) => Some((shown.clone(), *more)),
            _ => None,
        });
        assert_eq!(found, Some((vec!["Plan: Bad.".to_string(); 5], 0)));
    }
    #[test]
    fn the_instructions_say_which_problems_come_with_the_cutting_data_ones() {
        let text = instructions();
        assert!(
            text.contains("together with the generation, export, tool-axis and gouge problems."),
            "{text}"
        );
        assert!(!text.contains("together with the other problems"));
    }
    /// The plan check, except that it panics, as a check that fails to run
    /// would, on a proposal whose summary is "boom".
    fn check_or_boom(
        doc: &Document,
        mesh: &Mesh,
        report: &PartReport,
        proposal: &Proposal,
    ) -> (CheckReport, Option<Document>, Option<plan_check::Checked>) {
        assert_ne!(proposal.summary, "boom", "the check fails to run");
        let (report, job) = plan_check::check(doc, mesh, report, proposal);
        (report, job, None)
    }
    fn boom(report: &PartReport) -> Proposal {
        Proposal {
            summary: "boom".into(),
            ..good(report)
        }
    }
    #[tokio::test]
    async fn an_api_error_or_a_check_that_fails_to_run_keeps_the_last_problems() {
        let (doc, mesh, report) = block();
        let t9 = |failure: &Failure| {
            failure
                .problems
                .iter()
                .any(|p| p.message.contains("T9 is not in the tool library"))
        };
        // An API error after a failed check.
        let script = Script::new(vec![json(&bad(&report)), Err(ApiError::UsageLimit)]);
        let failure = plan(&script, doc.clone(), mesh.clone(), &Busy::default(), |_| {})
            .await
            .unwrap_err();
        assert_eq!(
            (failure.api.clone(), failure.requests),
            (Some(ApiError::UsageLimit), 2)
        );
        assert!(t9(&failure), "{:?}", failure.problems);
        // A check that fails to run after a failed check.
        let script = Script::new(vec![json(&bad(&report)), json(&boom(&report))]);
        let failure = plan_with(
            &script,
            doc.clone(),
            mesh.clone(),
            &Busy::default(),
            |_| {},
            analysis::report,
            check_or_boom,
        )
        .await
        .unwrap_err();
        assert!(
            failure.message.starts_with("The plan check failed to run"),
            "{}",
            failure.message
        );
        assert_eq!((failure.api.clone(), failure.requests), (None, 2));
        assert!(t9(&failure), "{:?}", failure.problems);
        // A check that fails to run first: nothing was found yet.
        let script = Script::new(vec![json(&boom(&report))]);
        let failure = plan_with(
            &script,
            doc.clone(),
            mesh.clone(),
            &Busy::default(),
            |_| {},
            analysis::report,
            check_or_boom,
        )
        .await
        .unwrap_err();
        assert!(failure.message.starts_with("The plan check failed to run"));
        assert!(failure.problems.is_empty());
        // A check that fails to run after a plan passed ends the run with it.
        let script = Script::new(vec![json(&good(&report)), json(&boom(&report))]);
        let outcome = plan_with(
            &script,
            doc,
            mesh,
            &Busy::default(),
            |_| {},
            analysis::report,
            check_or_boom,
        )
        .await
        .unwrap();
        assert_eq!((outcome.requests, outcome.chosen), (2, 1));
        assert_eq!(outcome.note, None);
        // The problems kept are bounded like those of four failures.
        let (doc, mesh, report) = block();
        let mut wrong = good(&report);
        wrong.setups[0].operations[0].strategy = "z".repeat(5000);
        for follow_up in [Err(ApiError::UsageLimit), json(&boom(&report))] {
            let script = Script::new(vec![json(&wrong), follow_up]);
            let failure = plan_with(
                &script,
                doc.clone(),
                mesh.clone(),
                &Busy::default(),
                |_| {},
                analysis::report,
                check_or_boom,
            )
            .await
            .unwrap_err();
            assert!(!failure.problems.is_empty());
            assert!(
                failure
                    .problems
                    .iter()
                    .all(|p| p.message.chars().count() <= 300)
            );
        }
    }
    #[tokio::test]
    async fn a_request_that_errors_or_a_check_that_fails_to_run_ends_the_events_without_a_result() {
        use Progress::*;
        let (doc, mesh, report) = block();
        let stage = Event::Stage;
        let t9 = StepResult::Problems {
            shown: vec!["Setup 1, operation 1: T9 is not in the tool library.".into()],
            more: 0,
        };
        let passed = Event::Checked(1, StepResult::Passed);
        let wrong = Event::Checked(1, t9);
        // An API error on a request: its stage is the last event, with no
        // Checked after it, on the first request, after a failed check and
        // on the follow-up of a plan that passed.
        for (replies, expected) in [
            (
                vec![Err(ApiError::UsageLimit)],
                vec![stage(Analyzing), stage(Asking(1))],
            ),
            (
                vec![json(&bad(&report)), Err(ApiError::UsageLimit)],
                vec![
                    stage(Analyzing),
                    stage(Asking(1)),
                    stage(Checking(1)),
                    wrong.clone(),
                    stage(Repairing(2)),
                ],
            ),
            (
                vec![json(&good(&report)), Err(ApiError::UsageLimit)],
                vec![
                    stage(Analyzing),
                    stage(Asking(1)),
                    stage(Checking(1)),
                    passed.clone(),
                    stage(Extending(2)),
                ],
            ),
        ] {
            let mut events = vec![];
            let _ = plan(
                &Script::new(replies),
                doc.clone(),
                mesh.clone(),
                &Busy::default(),
                |e| events.push(e),
            )
            .await;
            assert_eq!(events, expected);
        }
        // A check that fails to run: its stage is the last event, on the
        // first proposal, after a failed check and after a plan that passed.
        for (replies, expected) in [
            (
                vec![json(&boom(&report))],
                vec![stage(Analyzing), stage(Asking(1)), stage(Checking(1))],
            ),
            (
                vec![json(&bad(&report)), json(&boom(&report))],
                vec![
                    stage(Analyzing),
                    stage(Asking(1)),
                    stage(Checking(1)),
                    wrong.clone(),
                    stage(Repairing(2)),
                    stage(Checking(2)),
                ],
            ),
            (
                vec![json(&good(&report)), json(&boom(&report))],
                vec![
                    stage(Analyzing),
                    stage(Asking(1)),
                    stage(Checking(1)),
                    passed.clone(),
                    stage(Extending(2)),
                    stage(Checking(2)),
                ],
            ),
        ] {
            let mut events = vec![];
            let _ = plan_with(
                &Script::new(replies),
                doc.clone(),
                mesh.clone(),
                &Busy::default(),
                |e| events.push(e),
                analysis::report,
                check_or_boom,
            )
            .await;
            assert_eq!(events, expected);
        }
    }
    #[tokio::test]
    async fn the_last_of_two_different_check_failures_is_reported() {
        let (doc, mesh, report) = block();
        let tool = |n: u32| {
            let mut wrong = good(&report);
            wrong.setups[0].operations[0].tool = n;
            json(&wrong)
        };
        let script = Script::new(vec![tool(9), tool(8), tool(9), tool(8)]);
        let failure = plan(&script, doc, mesh, &Busy::default(), |_| {})
            .await
            .unwrap_err();
        assert_eq!(failure.requests, 4);
        let messages: Vec<&str> = failure
            .problems
            .iter()
            .map(|p| p.message.as_str())
            .collect();
        assert!(
            messages
                .iter()
                .any(|m| m.contains("T8 is not in the tool library")),
            "{messages:?}"
        );
        assert!(!messages.iter().any(|m| m.contains("T9")), "{messages:?}");
    }
    #[tokio::test]
    async fn a_passing_plan_survives_a_worse_or_failed_follow_up() {
        let (doc, mesh, report) = block();
        for follow_up in [
            json(&bad(&report)),
            Ok("oops".into()),
            Err(ApiError::UsageLimit),
        ] {
            let api = follow_up.clone().err();
            let script = Script::new(vec![json(&good(&report)), follow_up]);
            let outcome = plan(&script, doc.clone(), mesh.clone(), &Busy::default(), |_| {})
                .await
                .unwrap();
            assert_eq!(outcome.proposal, good(&report));
            assert_eq!(script.seen.lock().unwrap().len(), 2);
            assert_eq!((outcome.requests, outcome.chosen), (2, 1));
            // Only an API error is kept, for the account and the preview.
            assert_eq!(outcome.note, api);
        }
        let mut smaller = good(&report);
        smaller.setups.truncate(1);
        let script = Script::new(vec![json(&good(&report)), json(&smaller)]);
        let outcome = plan(&script, doc.clone(), mesh.clone(), &Busy::default(), |_| {})
            .await
            .unwrap();
        assert_eq!(outcome.proposal, good(&report));
        assert_eq!((outcome.requests, outcome.chosen), (2, 1));
    }
    #[tokio::test]
    async fn a_refusal_ends_the_run_with_a_clean_bounded_message() {
        let (doc, mesh, _) = block();
        let hostile = format!("No\nway \u{202e}\u{7}{}", "x".repeat(5000));
        let script = Script::new(vec![Err(ApiError::Refused(hostile))]);
        let failure = plan(&script, doc, mesh, &Busy::default(), |_| {})
            .await
            .unwrap_err();
        assert!(!failure.message.contains(['\n', '\u{202e}', '\u{7}']));
        assert!(
            failure.message.chars().count() <= 340,
            "{}",
            failure.message.len()
        );
    }
    #[tokio::test]
    async fn model_text_is_cleaned_and_bounded_before_it_is_kept() {
        let (doc, mesh, report) = block();
        let mut long = good(&report);
        long.summary = format!("\u{202e}{}", "s".repeat(5_000_000));
        long.warnings = (0..15)
            .map(|i| format!("warning {i}\n{}", "w".repeat(500)))
            .collect();
        long.unmachined = (0..25)
            .map(|i| proposal::Unmachined {
                feature: format!("feature {i} {}", "f".repeat(200)),
                reason: "r".repeat(500),
            })
            .collect();
        long.setups[0].workholding = "h".repeat(900);
        long.setups[0].operations[0].name = "n".repeat(200);
        long.setups[0].operations[0].reason = "\u{7}".to_string() + &"q".repeat(400);
        let script = Script::new(vec![json(&long), json(&long)]);
        let outcome = plan(&script, doc, mesh, &Busy::default(), |_| {})
            .await
            .unwrap();
        let p = &outcome.proposal;
        assert!(p.summary.len() == 600 && p.summary.chars().all(|c| c == 's'));
        assert_eq!(p.warnings.len(), 12);
        assert!(
            p.warnings
                .iter()
                .all(|w| w.chars().count() == 240 && !w.contains('\n'))
        );
        assert_eq!(outcome.more_warnings, 3);
        assert_eq!(p.unmachined.len(), 20);
        assert_eq!(outcome.more_unmachined, 5);
        assert!(
            p.unmachined
                .iter()
                .all(|u| u.feature.chars().count() == 80 && u.reason.chars().count() == 240)
        );
        assert_eq!(p.setups[0].workholding.chars().count(), 500);
        assert_eq!(p.setups[0].operations[0].name.chars().count(), 60);
        assert_eq!(p.setups[0].operations[0].reason, "q".repeat(160));
    }
    #[tokio::test]
    async fn names_lose_quotation_and_check_marks_before_the_check_builds_the_job() {
        let (doc, mesh, report) = block();
        let mut named = good(&report);
        named.setups[1].name = "Under\u{201d} \u{2713} Checked by \"AXIS\"".into();
        let names = [
            (
                0,
                0,
                "\u{201c}Face\u{201d} \u{2714} \u{201e}a\u{ab}b\u{bb}c\u{2039}d\u{203a}e'f\u{2018}g\u{2019} \u{2611}\u{2717}\u{2718}\u{2612}\u{d7} top",
                "Face a b c d e'f g x top",
            ),
            // The rest of Unicode's initial and final quotation marks (Pi, Pf),
            // each replaced by a space.
            (
                0,
                1,
                "\u{201b}a\u{201f}b\u{2e02}c\u{2e03}d\u{2e04}e\u{2e05}f\u{2e09}g\u{2e0a}h\u{2e0c}i\u{2e0d}j\u{2e1c}k\u{2e1d}l\u{2e20}m\u{2e21}n\u{201a}",
                "a b c d e f g h i j k l m n",
            ),
            // Primes, fullwidth and ornamental quotation marks.
            (
                0,
                2,
                "6\u{2032}\u{2033}\u{2034}\u{2035}\u{2036}\u{2037} \u{ff02}wide\u{ff07} \u{275b}\u{275c}\u{275d}\u{275e}\u{301d}\u{301e}\u{301f}walls",
                "6 wide walls",
            ),
            // Check and cross marks; × becomes x, an en dash a hyphen, and a
            // diameter sign stays.
            (
                0,
                3,
                "Drill \u{2705}\u{274c}\u{274e}\u{2715}\u{2716}\u{221a}\u{2713}\u{2714}\u{2611}\u{2717}\u{2718}\u{2612} 10\u{d7}10 grid, holes 1\u{2013}4 \u{2300}6.8",
                "Drill 10x10 grid, holes 1-4 \u{2300}6.8",
            ),
            // An apostrophe between two letters stays; other ' ‘ ’ go.
            (
                1,
                0,
                "Bob's 'fixture' Bob\u{2019}s \u{2018}rock'n'roll\u{2019} 6' 'x",
                "Bob's fixture Bob\u{2019}s rock'n'roll 6 x",
            ),
            // A combining accent or a digit before an apostrophe counts as
            // its letter: a decomposed "José's" and "Op 2's" keep theirs.
            (
                1,
                1,
                "Jose\u{301}'s 10\u{d7}10 pocket, Op 2's floor",
                "Jose\u{301}'s 10x10 pocket, Op 2's floor",
            ),
        ];
        for &(si, oi, raw, _) in &names {
            named.setups[si].operations[oi].name = raw.into();
        }
        let script = Script::new(vec![json(&named), json(&named)]);
        let outcome = plan(&script, doc, mesh, &Busy::default(), |_| {})
            .await
            .unwrap();
        assert_eq!(outcome.proposal.setups[1].name, "Under Checked by AXIS");
        // The job is built from the cleaned names.
        assert_eq!(outcome.job.setups[1].name(), "Under Checked by AXIS");
        for &(si, oi, raw, clean) in &names {
            assert_eq!(
                outcome.proposal.setups[si].operations[oi].name, clean,
                "{raw}"
            );
            assert_eq!(outcome.job.setups[si].operations[oi].name(), clean, "{raw}");
        }
        // Other model text keeps its quotes.
        let mut quoted = good(&report);
        quoted.summary = "A \"quoted\" summary \u{2713}".into();
        let script = Script::new(vec![json(&quoted), json(&quoted)]);
        let (doc, mesh, _) = block();
        let outcome = plan(&script, doc, mesh, &Busy::default(), |_| {})
            .await
            .unwrap();
        assert_eq!(outcome.proposal.summary, "A \"quoted\" summary \u{2713}");
    }
    #[test]
    fn names_keep_letters_digits_and_a_short_punctuation_list() {
        // Look-alikes of quotation and check marks outside any list are gone:
        // modifier letters, Hebrew geresh and gershayim, the ditto mark,
        // corner brackets, the double low-reversed-9 quotation mark and the
        // ballot and check marks.
        for (raw, clean) in [
            ("Face \u{2ee}Check\u{2ee}", "Face Check"),
            ("Face \u{2ba}x\u{2ba}", "Face x"),
            ("\u{5f4}x\u{5f4}", "x"),
            ("\u{3003}x", "x"),
            ("\u{2e42}x", "x"),
            ("\u{300c}x\u{300d}", "x"),
            ("\u{1f5f8} passed", "passed"),
            ("\u{1f5f9} ok", "ok"),
            ("a\u{2bb}b\u{2bc}c\u{5f3}d", "a b c d"),
            ("\u{300e}x\u{300f} \u{ff62}y\u{ff63}", "x y"),
            ("\u{1f5f4}\u{1f5f5}\u{1f5f6}\u{1f5f7} z", "z"),
            (
                "Face \u{2ee} \u{b7} T1 New tool \u{b7} 0.1 min",
                "Face T1 New tool 0.1 min",
            ),
            // Canadian syllabics finals drawn like primes, n preceded by an
            // apostrophe and the Duployan affixes.
            ("Face \u{1425}Checked\u{1425}", "Face Checked"),
            (
                "Face \u{141f}Checked\u{141f} \u{1420}x\u{1420}",
                "Face Checked x",
            ),
            ("\u{149}Check", "Check"),
            ("\u{1bc80}x\u{1bc88} y\u{1bc93}", "x y"),
            // Fullwidth quotation marks go, even between letters.
            ("Bob\u{ff07}s \u{ff02}x\u{ff02}", "Bob s x"),
            // A combining mark stays only on a kept letter.
            ("\"\u{301}x 1\u{20e3} \u{301}", "x 1"),
            // Any other removed character becomes a space, so words and
            // numbers never run together.
            ("Pocket\u{2192}drill 1\u{2713}2", "Pocket drill 1 2"),
        ] {
            assert_eq!(name(raw), clean, "{raw}");
        }
        // Ordinary names keep their meaning, in any script.
        for ordinary in [
            "Face top",
            "Pocket \u{d8}8 (left)",
            "Drill 4x M6",
            "Waterline 0.5 mm",
            "Kontur au\u{df}en",
            "\u{7c97}\u{52a0}\u{5de5} 1",
            "Cafe\u{301} slot_2 - 50% & 1:2, #3 + 1/4 in.",
            "\u{921}\u{94d}\u{930}\u{93f}\u{932} 6",
            "\u{e23}\u{e48}\u{e2d}\u{e07} 2",
            // Comparison, unit and tolerance signs.
            "Slot <6 mm",
            "Fillets R\u{2264}2",
            "Boss >3 = R\u{2265}1",
            "Chamfer 45\u{b0}",
            "Drill \u{2300}6.8",
            "Bore \u{d8}10 \u{b1}0.02",
            // Halfwidth katakana keep their voiced sound mark.
            "\u{ff76}\u{ff9e}\u{ff72}\u{ff84}\u{ff9e} 3",
        ] {
            assert_eq!(name(ordinary), ordinary);
        }
        // Dashes and minus signs become -, * between two digits becomes x,
        // fullwidth forms become ASCII first, and removed punctuation between
        // words a space.
        for (raw, clean) in [
            ("Holes 1\u{2013}4", "Holes 1-4"),
            ("Face to Z\u{2212}0.5", "Face to Z-0.5"),
            ("10*10 pocket", "10x10 pocket"),
            ("10 * 10", "10 x 10"),
            // Elsewhere * is removed like other symbols, so Markdown
            // emphasis and footnote stars leave no letters behind.
            ("**Roughing** pass", "Roughing pass"),
            ("Face *top*", "Face top"),
            ("Drill 6* holes", "Drill 6 holes"),
            ("Pocket R2*", "Pocket R2"),
            ("*5 slot", "5 slot"),
            ("Pocket\u{2014}left", "Pocket-left"),
            ("Op 2\u{2013}3\u{2019}s", "Op 2-3\u{2019}s"),
            (
                "a\u{2010}b\u{2011}c\u{2012}d\u{2015}e\u{fe58}f\u{fe63}g\u{ff0d}h",
                "a-b-c-d-e-f-g-h",
            ),
            (
                "\u{7c97}\u{52a0}\u{5de5}\u{ff08}\u{5de6}\u{ff09}",
                "\u{7c97}\u{52a0}\u{5de5}(\u{5de6})",
            ),
            (
                "\u{30d5}\u{30a7}\u{30a4}\u{30b9}\u{30fb}\u{30c8}\u{30c3}\u{30d7}",
                "\u{30d5}\u{30a7}\u{30a4}\u{30b9} \u{30c8}\u{30c3}\u{30d7}",
            ),
            (
                "\u{ff26}\u{ff41}\u{ff43}\u{ff45} \u{ff11}\u{ff10}\u{ff0a}\u{ff15}",
                "Face 10x5",
            ),
        ] {
            assert_eq!(name(raw), clean, "{raw}");
        }
        // Range, decimal, percent, fraction and diameter marks of other
        // scripts keep their meaning: ～ and 〜 become ~, which stays, the
        // Arabic decimal and thousands separators and percent sign become
        // . , %, the fraction and division slashes become / and ∅ becomes ⌀.
        for (raw, clean) in [
            (
                "\u{7a74}\u{3042}\u{3051} 1\u{ff5e}4",
                "\u{7a74}\u{3042}\u{3051} 1~4",
            ),
            (
                "\u{30bf}\u{30c3}\u{30d7} M6\u{301c}M10",
                "\u{30bf}\u{30c3}\u{30d7} M6~M10",
            ),
            ("M6~M10 tap", "M6~M10 tap"),
            (
                "\u{645}\u{62a}\u{647} \u{6f3}\u{66b}\u{6f5}",
                "\u{645}\u{62a}\u{647} \u{6f3}.\u{6f5}",
            ),
            (
                "\u{6af}\u{627}\u{645} \u{6f5}\u{6f0}\u{66a}",
                "\u{6af}\u{627}\u{645} \u{6f5}\u{6f0}%",
            ),
            (
                "\u{661}\u{66c}\u{662}\u{660}\u{660}",
                "\u{661},\u{662}\u{660}\u{660}",
            ),
            ("Drill \u{2205}10", "Drill \u{2300}10"),
            ("1\u{2044}4 in drill", "1/4 in drill"),
            ("1\u{2215}2 in slot", "1/2 in slot"),
        ] {
            assert_eq!(name(raw), clean, "{raw}");
        }
        // Format characters join or break words without being seen, so they
        // go without a space: a Persian word written with a zero-width
        // non-joiner and a soft hyphen stay one word. A middle dot inside
        // Catalan l·l goes the same way; anywhere else it leaves a space, so
        // paired terms stay apart and it cannot imitate the " · " separators.
        for (raw, clean) in [
            (
                "\u{633}\u{648}\u{631}\u{627}\u{62e}\u{200c}\u{6a9}\u{627}\u{631}\u{6cc} \u{6f6}",
                "\u{633}\u{648}\u{631}\u{627}\u{62e}\u{6a9}\u{627}\u{631}\u{6cc} \u{6f6}",
            ),
            ("Bohr\u{ad}ung", "Bohrung"),
            (
                "a\u{200b}b\u{200d}c\u{200e}d\u{200f}e\u{2060}f\u{2064}g\u{feff}h",
                "abcdefgh",
            ),
            ("Fresat paral\u{b7}lel", "Fresat parallel"),
            ("Face \u{b7} top", "Face top"),
            (
                "\u{b4dc}\u{b9b4}\u{b7}\u{d0ed} M6",
                "\u{b4dc}\u{b9b4} \u{d0ed} M6",
            ),
            ("Drill\u{b7}Tap M6", "Drill Tap M6"),
            ("Op 1\u{b7}2 \u{b7}x y\u{b7}", "Op 1 2 x y"),
        ] {
            assert_eq!(name(raw), clean, "{raw}");
        }
        // White space becomes one space; × becomes x.
        assert_eq!(name(" Face\t\u{a0} top  10\u{d7}10 "), "Face top 10x10");
        // A digit before an apostrophe counts like a letter; one after does not.
        assert_eq!(name("Op 2's 'n' x'"), "Op 2's n x");
    }
    #[test]
    fn the_combining_mark_ranges_are_sorted_and_find_marks_of_any_script() {
        assert!(MARKS.windows(2).all(|pair| pair[0] < pair[1]));
        // An accent, a virama, a vowel sign, a Thai tone mark, a kana voicing
        // mark, a Hebrew point, an enclosing keycap and the last variation
        // selector.
        for mark in [
            '\u{301}',
            '\u{94d}',
            '\u{93f}',
            '\u{e48}',
            '\u{3099}',
            '\u{5b0}',
            '\u{20e3}',
            '\u{e01ef}',
        ] {
            assert!(combining(mark), "U+{:04X}", u32::from(mark));
        }
        for other in ['a', '1', ' ', '\u{2ff}', '\u{370}', '\u{e23}', '\u{e01f0}'] {
            assert!(!combining(other), "U+{:04X}", u32::from(other));
        }
    }
    #[test]
    fn setup_1_notes_keep_the_planners_note_on_one_line() {
        let added = |text: &str, note: &str| (text.to_string(), note.to_string());
        assert_eq!(
            setup_1_notes("Soft jaws.", "", "Vise on\nthe long\r\nsides."),
            added(
                "Soft jaws. \u{b7} Planner: Vise on the long sides.",
                "Vise on the long sides."
            )
        );
        assert_eq!(
            setup_1_notes(" \n", "", "Vise."),
            added("Planner: Vise.", "Vise.")
        );
        assert_eq!(
            setup_1_notes("Soft jaws.\n", "", "Vise."),
            added("Soft jaws. \u{b7} Planner: Vise.", "Vise.")
        );
        // A second Apply replaces its own earlier note, in either form.
        assert_eq!(
            setup_1_notes("Soft jaws. \u{b7} Planner: Vise.", "Vise.", "Clamps."),
            added("Soft jaws. \u{b7} Planner: Clamps.", "Clamps.")
        );
        assert_eq!(
            setup_1_notes("Planner: Vise.", "Vise.", "Clamps."),
            added("Planner: Clamps.", "Clamps.")
        );
        // Text the user typed after the planner's note survives.
        assert_eq!(
            setup_1_notes(
                "Soft jaws. \u{b7} Planner: Vise. Torque 40 Nm.",
                "Vise.",
                "Clamps."
            ),
            added(
                "Soft jaws. Torque 40 Nm. \u{b7} Planner: Clamps.",
                "Clamps."
            )
        );
        assert_eq!(
            setup_1_notes("Planner: Vise. Torque 40 Nm.", "Vise.", "Clamps."),
            added("Torque 40 Nm. \u{b7} Planner: Clamps.", "Clamps.")
        );
        // Notes the user starts with "Planner: " survive, as does a planner
        // note the user changed: only the exact text Apply added goes.
        assert_eq!(
            setup_1_notes("Planner: Alice checks jaws.", "", "Clamps."),
            added(
                "Planner: Alice checks jaws. \u{b7} Planner: Clamps.",
                "Clamps."
            )
        );
        assert_eq!(
            setup_1_notes("Planner: Alice checks jaws.", "Vise.", "Clamps."),
            added(
                "Planner: Alice checks jaws. \u{b7} Planner: Clamps.",
                "Clamps."
            )
        );
        assert_eq!(
            setup_1_notes(
                "Soft jaws. \u{b7} Planner: Vise, 40 Nm.",
                "Vise.",
                "Clamps."
            ),
            added(
                "Soft jaws. \u{b7} Planner: Vise, 40 Nm. \u{b7} Planner: Clamps.",
                "Clamps."
            )
        );
        // The earlier note counts only where it stands whole: followed by
        // the end of the notes or by white space and, alone, at the start or
        // after white space. A planner note the user went on with mid-word or
        // mid-number is the user's and stays.
        assert_eq!(
            setup_1_notes(
                "Soft jaws. \u{b7} Planner: Vises on both ends",
                "Vise",
                "Clamps."
            ),
            added(
                "Soft jaws. \u{b7} Planner: Vises on both ends \u{b7} Planner: Clamps.",
                "Clamps."
            )
        );
        assert_eq!(
            setup_1_notes("Planner: Vises on both ends", "Vise", "Clamps."),
            added(
                "Planner: Vises on both ends \u{b7} Planner: Clamps.",
                "Clamps."
            )
        );
        assert_eq!(
            setup_1_notes(
                "Soft jaws. \u{b7} Planner: Clamp in vise on the long sides, 40 Nm",
                "Clamp in vise on the long sides",
                "Clamps."
            ),
            added(
                "Soft jaws. \u{b7} Planner: Clamp in vise on the long sides, 40 Nm \u{b7} Planner: Clamps.",
                "Clamps."
            )
        );
        assert_eq!(
            setup_1_notes("XPlanner: Vise.", "Vise.", "Clamps."),
            added("XPlanner: Vise. \u{b7} Planner: Clamps.", "Clamps.")
        );
        // An earlier note that does not end a sentence (. ! ? or 。) counts
        // as whole only before the end of the notes or a line break: words
        // after it on the same line may continue its sentence, so the note
        // stays and the new one is added.
        assert_eq!(
            setup_1_notes("Planner: Vise grips here", "Vise", "Clamps."),
            added(
                "Planner: Vise grips here \u{b7} Planner: Clamps.",
                "Clamps."
            )
        );
        assert_eq!(
            setup_1_notes(
                "Soft jaws. \u{b7} Planner: Vise grips here",
                "Vise",
                "Clamps."
            ),
            added(
                "Soft jaws. \u{b7} Planner: Vise grips here \u{b7} Planner: Clamps.",
                "Clamps."
            )
        );
        assert_eq!(
            setup_1_notes("Soft jaws. \u{b7} Planner: Vise", "Vise", "Clamps."),
            added("Soft jaws. \u{b7} Planner: Clamps.", "Clamps.")
        );
        assert_eq!(
            setup_1_notes(
                "Soft jaws. \u{b7} Planner: Vise\nTorque 40 Nm.",
                "Vise",
                "Clamps."
            ),
            added(
                "Soft jaws.\nTorque 40 Nm. \u{b7} Planner: Clamps.",
                "Clamps."
            )
        );
        assert_eq!(
            setup_1_notes("Planner: Vise\r\nTorque 40 Nm.", "Vise", "Clamps."),
            added("Torque 40 Nm. \u{b7} Planner: Clamps.", "Clamps.")
        );
        // Of two copies, the last whole one goes.
        assert_eq!(
            setup_1_notes(
                "A \u{b7} Planner: Vise\nB \u{b7} Planner: Vise grips",
                "Vise",
                "Clamps."
            ),
            added(
                "A\nB \u{b7} Planner: Vise grips \u{b7} Planner: Clamps.",
                "Clamps."
            )
        );
        // A note that ends a sentence is whole before white space too.
        assert_eq!(
            setup_1_notes("Planner: Vise! Torque 40 Nm.", "Vise!", "Clamps."),
            added("Torque 40 Nm. \u{b7} Planner: Clamps.", "Clamps.")
        );
        assert_eq!(
            setup_1_notes(
                "Soft jaws. \u{b7} Planner: Vise? Torque 40 Nm.",
                "Vise?",
                "Clamps."
            ),
            added(
                "Soft jaws. Torque 40 Nm. \u{b7} Planner: Clamps.",
                "Clamps."
            )
        );
        assert_eq!(
            setup_1_notes(
                "Planner: \u{4e07}\u{529b}\u{3002} \u{30c8}\u{30eb}\u{30af}",
                "\u{4e07}\u{529b}\u{3002}",
                "Clamps."
            ),
            added(
                "\u{30c8}\u{30eb}\u{30af} \u{b7} Planner: Clamps.",
                "Clamps."
            )
        );
        // The last whole copy, the one Apply appended, is replaced.
        assert_eq!(
            setup_1_notes(
                "A \u{b7} Planner: Vise. B \u{b7} Planner: Vise.",
                "Vise.",
                "Clamps."
            ),
            added(
                "A \u{b7} Planner: Vise. B \u{b7} Planner: Clamps.",
                "Clamps."
            )
        );
        // A note Apply wrote alone is replaced after text the user typed
        // before it, so setup 1 does not carry two planner notes.
        assert_eq!(
            setup_1_notes(
                "Use soft jaws. Planner: Vise on the long sides.",
                "Vise on the long sides.",
                "Clamp on the short sides."
            ),
            added(
                "Use soft jaws. \u{b7} Planner: Clamp on the short sides.",
                "Clamp on the short sides."
            )
        );
        assert_eq!(
            setup_1_notes(
                "Use soft jaws.\nPlanner: Vise. Torque 40 Nm.",
                "Vise.",
                "Clamps."
            ),
            added(
                "Use soft jaws.\nTorque 40 Nm. \u{b7} Planner: Clamps.",
                "Clamps."
            )
        );
        assert_eq!(
            setup_1_notes(" \n Planner: Vise. Torque 40 Nm.", "Vise.", "Clamps."),
            added("Torque 40 Nm. \u{b7} Planner: Clamps.", "Clamps.")
        );
        // A plan without a note for setup 1 leaves no stale one behind.
        assert_eq!(
            setup_1_notes("Soft jaws. \u{b7} Planner: Vise.", "Vise.", ""),
            added("Soft jaws.", "")
        );
        assert_eq!(setup_1_notes("Planner: Vise.", "Vise.", " "), added("", ""));
        assert_eq!(setup_1_notes("Soft jaws.", "", ""), added("Soft jaws.", ""));
        // The planner's part is at most 500 characters; the user's notes are
        // never cut.
        let own = "o".repeat(700);
        let (text, note) = setup_1_notes(&own, "", &"w".repeat(900));
        assert!(!text.contains('\n'));
        let part = text.strip_prefix(&format!("{own} \u{b7} ")).unwrap();
        assert!(part.starts_with("Planner: www"));
        assert_eq!(part.chars().count(), 500);
        assert_eq!(part.strip_prefix("Planner: "), Some(note.as_str()));
        // That note is found again by the next Apply.
        assert_eq!(
            setup_1_notes(&text, &note, "Vise."),
            added(&format!("{own} \u{b7} Planner: Vise."), "Vise.")
        );
    }
    #[tokio::test]
    async fn failure_problems_are_bounded() {
        let (doc, mesh, report) = block();
        let mut wrong = good(&report);
        wrong.setups[0].operations[0].strategy = "z".repeat(5000);
        let script = Script::new(vec![json(&wrong); 4]);
        let failure = plan(&script, doc, mesh, &Busy::default(), |_| {})
            .await
            .unwrap_err();
        assert!(!failure.problems.is_empty());
        assert!(
            failure
                .problems
                .iter()
                .all(|p| p.message.chars().count() <= 300)
        );
    }
    /// A 60 × 40 × 20 mm box with no stock below it: a Face and an outside
    /// Contour leave nothing covered.
    pub(crate) fn open_box() -> (Document, Mesh, PartReport) {
        let (mut doc, _) = crate::post::fixture::job();
        crate::proposal::tests::aluminium(&mut doc);
        let size = [60f32, 40., 20.];
        let v = |i: usize| -> [f32; 3] {
            std::array::from_fn(|k| if i >> k & 1 == 0 { 0. } else { size[k] })
        };
        let mut triangles = vec![];
        for q in [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ] {
            triangles.push([v(q[0]), v(q[1]), v(q[2])]);
            triangles.push([v(q[0]), v(q[2]), v(q[3])]);
        }
        let mesh = crate::mesh::normalize(triangles).unwrap();
        doc.setups[0].operations.clear();
        doc.setups[0].p.insert("stock_bottom".into(), "0".into());
        let report = analysis::report(&doc, &mesh).unwrap();
        (doc, mesh, report)
    }
    /// Face and an outside Contour: the whole box in one request.
    pub(crate) fn complete(report: &PartReport) -> Proposal {
        let top = report.orientations[0]
            .planes
            .iter()
            .find(|p| p.kind == "top")
            .unwrap()
            .surfaces
            .clone();
        Proposal {
            summary: "Face and contour.".into(),
            chuck: None,
            setups: vec![proposal::ProposedSetup {
                orientation: report.orientations[0].up.into(),
                name: "Top".into(),
                origin: "Stock top center".into(),
                workholding: "Vise.".into(),
                holder: None,
                a_angles: None,
                operations: vec![
                    proposal::tests::op("Face", 1, top),
                    proposal::tests::op("Contour", 1, vec![]),
                ],
                from: None,
            }],
            unmachined: vec![],
            warnings: vec![],
        }
    }
    #[tokio::test]
    async fn a_first_plan_with_nothing_covered_makes_one_request() {
        let (doc, mesh, report) = open_box();
        let script = Script::new(vec![json(&complete(&report)), json(&complete(&report))]);
        let mut events = vec![];
        let outcome = plan(&script, doc, mesh, &Busy::default(), |e| events.push(e))
            .await
            .unwrap();
        let cover = outcome.check.coverage.as_ref().unwrap();
        assert!(
            cover.covered_planes.is_empty() && cover.covered_other.is_empty(),
            "{cover:?}"
        );
        assert_eq!((outcome.requests, outcome.chosen), (1, 1));
        assert_eq!(script.seen.lock().unwrap().len(), 1);
        assert_eq!(
            stages(&events),
            vec![
                Progress::Analyzing,
                Progress::Asking(1),
                Progress::Checking(1)
            ]
        );
    }
    #[tokio::test]
    async fn a_proposal_in_a_code_fence_or_between_sentences_is_read() {
        let (doc, mesh, report) = open_box();
        let body = serde_json::to_string(&complete(&report)).unwrap();
        for text in [
            format!("```json\n{body}\n```"),
            format!("Here is the plan:\n{body}\nHope this helps!"),
        ] {
            let script = Script::new(vec![Ok(text)]);
            let outcome = plan(&script, doc.clone(), mesh.clone(), &Busy::default(), |_| {})
                .await
                .unwrap();
            assert_eq!(outcome.requests, 1);
        }
        let script = Script::new(vec![Ok("No plan.".into()), json(&complete(&report))]);
        let mut events = vec![];
        plan(&script, doc, mesh, &Busy::default(), |e| events.push(e))
            .await
            .unwrap();
        assert_eq!(
            stages(&events),
            vec![
                Progress::Analyzing,
                Progress::Asking(1),
                Progress::Repairing(2),
                Progress::Checking(2)
            ]
        );
    }
    #[tokio::test]
    async fn the_openai_backend_sends_the_conversation_and_remembers_plain_json() {
        use crate::fake_openai::{Fake, Resp};
        use crate::session::SessionStore;
        let fake = Fake::start(|req| {
            if req.json()["text"].is_object() {
                Resp::error(400, "subscription_sharing_unsupported_capability")
            } else {
                Resp::answer("{}")
            }
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(crate::session::FileStore(dir.path().join("s.json")));
        let session = crate::session::sample();
        store.save(&session).unwrap();
        let client = crate::openai::Client::new(
            crate::oauth::Endpoints::local(&fake.base),
            "oaiapp_1".into(),
            store,
            session,
        );
        let stream = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let backend = OpenAiBackend::new(
            std::sync::Arc::new(client),
            "gpt-6.1-sol".into(),
            stream.clone(),
            true,
        );
        assert!(backend.pictures());
        let messages = vec![
            Item::text(Role::System, "Plan."),
            Item::text(Role::User, "Part."),
            Item::text(Role::Assistant, "{}"),
            Item::text(Role::User, "Fix it."),
        ];
        let text = || stream.lock().unwrap().clone();
        assert_eq!(
            backend
                .reply(&messages, &[], ToolChoice::Auto, false)
                .await
                .unwrap()
                .text,
            "{}"
        );
        assert_eq!(text(), "{}", "the structured refusal streamed nothing");
        *stream.lock().unwrap() = "x".repeat(40);
        let tools = planner_tools::offered(true);
        let reply = backend.reply(&messages, &tools, ToolChoice::None, false);
        assert_eq!(reply.await.unwrap().text, "{}");
        assert_eq!(text(), "{}", "the plain request starts the text again");
        let sent = fake.seen("/v1/responses");
        assert_eq!(
            sent.len(),
            3,
            "the second reply skips the structured attempt"
        );
        // The tools are offered only when given, all of them, with their choice.
        assert!(sent[0].json()["tools"].is_null() && sent[1].json()["tools"].is_null());
        let offered = sent[2].json();
        let names: Vec<&str> = offered["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names[0], crate::look::NAME);
        assert_eq!(names[1..], PART_TOOLS);
        assert_eq!(offered["tool_choice"], "none");
        let first = sent[0].json();
        assert_eq!(first["model"], "gpt-6.1-sol");
        assert_eq!(first["text"]["format"]["name"], "machining_plan");
        assert_eq!(first["text"]["format"]["schema"], proposal::schema());
        let roles: Vec<&str> = first["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["system", "user", "assistant", "user"]);
        assert!(sent[1].json()["text"].is_null() && sent[2].json()["text"].is_null());
    }
    #[tokio::test]
    async fn an_empty_reply_is_not_echoed_back_as_an_empty_assistant_message() {
        use crate::fake_openai::{Fake, Resp};
        use crate::session::SessionStore;
        // Every reply completes with no text and no calls.
        let fake = Fake::start(|_| {
            Resp::sse(&[
                serde_json::json!({"type": "response.created"}),
                serde_json::json!({"type": "response.completed", "response": {"status": "completed"}}),
            ])
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(crate::session::FileStore(dir.path().join("s.json")));
        let session = crate::session::sample();
        store.save(&session).unwrap();
        let client = crate::openai::Client::new(
            crate::oauth::Endpoints::local(&fake.base),
            "oaiapp_1".into(),
            store,
            session,
        );
        let backend = OpenAiBackend::new(
            std::sync::Arc::new(client),
            "gpt-6.1-sol".into(),
            std::sync::Arc::new(std::sync::Mutex::new(String::new())),
            false,
        );
        let (doc, mesh, _) = block();
        let mut events = vec![];
        let _ = plan(&backend, doc, mesh, &Busy::default(), |e| events.push(e)).await;
        let Some(Event::Checked(1, StepResult::Unreadable(why))) =
            events.iter().find(|e| matches!(e, Event::Checked(1, _)))
        else {
            panic!("{events:?}")
        };
        assert!(why.contains("empty"), "{why}");
        let sent = fake.seen("/v1/responses");
        assert!(sent.len() >= 2, "the correction round was sent");
        let second = sent[1].json();
        let input = second["input"].as_array().unwrap();
        assert!(
            input.iter().all(|m| {
                m["role"] != "assistant" || m["content"].as_str().is_none_or(|c| !c.is_empty())
            }),
            "{input:?}"
        );
        let last = input.last().unwrap();
        assert_eq!(last["role"], "user");
        assert!(last["content"].to_string().contains("empty"), "{last}");
    }
    #[tokio::test]
    async fn blocking_work_is_counted_until_it_ends_even_when_nobody_waits() {
        let busy = Busy::default();
        let (started, release) = (
            std::sync::Arc::new(std::sync::Barrier::new(2)),
            std::sync::Arc::new(std::sync::Barrier::new(2)),
        );
        let (s, r) = (started.clone(), release.clone());
        let waiting = tokio::spawn({
            let busy = busy.clone();
            async move {
                blocking(&busy, move || {
                    s.wait();
                    r.wait();
                })
                .await
            }
        });
        tokio::task::spawn_blocking({
            let started = started.clone();
            move || started.wait()
        })
        .await
        .unwrap();
        assert_eq!(busy.running(), 1);
        // Cancelling the run drops the waiting future, not the work.
        waiting.abort();
        let _ = waiting.await;
        assert_eq!(busy.running(), 1);
        tokio::task::spawn_blocking(move || release.wait())
            .await
            .unwrap();
        for _ in 0..200 {
            if busy.running() == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(busy.running(), 0);
    }
    #[test]
    fn queued_blocking_work_is_counted_before_it_starts() {
        // One blocking thread, held by a first stage: a second stage queues
        // behind it and must already count, so Plan cannot start a run while
        // it waits.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(1)
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            use std::sync::{
                Arc,
                atomic::{AtomicBool, Ordering::SeqCst},
            };
            let (busy, gate) = (Busy::default(), Gate::default());
            let releases = Releases(&gate);
            let first = tokio::spawn({
                let (busy, held) = (busy.clone(), gate.clone());
                async move { blocking(&busy, move || held.hold()).await }
            });
            while !gate.entered.load(SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
            let started = Arc::new(AtomicBool::new(false));
            let second = tokio::spawn({
                let (busy, started) = (busy.clone(), started.clone());
                async move { blocking(&busy, move || started.store(true, SeqCst)).await }
            });
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            assert!(!started.load(SeqCst), "the only blocking thread is held");
            assert_eq!(
                busy.running(),
                2,
                "the queued stage counts before it starts"
            );
            drop(releases);
            first.await.unwrap().unwrap();
            second.await.unwrap().unwrap();
            assert!(started.load(SeqCst));
            assert_eq!(busy.running(), 0);
        });
    }
    /// Holds a blocking stage while it runs. A failing test cannot leave it
    /// held: a blocking task that never ends would stop the runtime from
    /// shutting down.
    #[derive(Clone, Default)]
    pub struct Gate {
        entered: std::sync::Arc<std::sync::atomic::AtomicBool>,
        released: std::sync::Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
    }
    impl Gate {
        /// Called by the stage: waits until the test releases it, at most a
        /// minute.
        pub fn hold(&self) {
            self.entered
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let (released, opened) = &*self.released;
            let guard = released.lock().unwrap();
            let limit = std::time::Duration::from_secs(60);
            let _ = opened.wait_timeout_while(guard, limit, |released| !*released);
        }
        fn release(&self) {
            let (released, opened) = &*self.released;
            *released.lock().unwrap() = true;
            opened.notify_all();
        }
    }
    /// Releases the gate however the test ends.
    struct Releases<'a>(&'a Gate);
    impl Drop for Releases<'_> {
        fn drop(&mut self) {
            self.0.release();
        }
    }
    /// Drives `run` until `gate`'s stage has started and drops it while the
    /// stage runs. The stage must stay counted until it ends, then nothing.
    pub async fn drop_while_held<T>(run: impl Future<Output = T>, gate: &Gate, busy: &Busy) {
        let releases = Releases(gate);
        let mut run = Box::pin(run);
        let entered = async {
            while !gate.entered.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
        };
        tokio::select! {
            _ = &mut run => panic!("the run ended before the held stage started"),
            () = entered => {}
        }
        assert_eq!(busy.running(), 1, "the held stage is counted");
        drop(run);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(busy.running(), 1, "still counted after its run was dropped");
        drop(releases);
        for _ in 0..500 {
            if busy.running() == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(busy.running(), 0, "not counted once it has ended");
    }
    #[tokio::test]
    async fn a_dropped_run_counts_its_check_until_the_check_ends() {
        let (doc, mesh, report) = open_box();
        let (busy, gate) = (Busy::default(), Gate::default());
        let held = gate.clone();
        let script = Script::new(vec![json(&complete(&report))]);
        let run = plan_with(
            &script,
            doc,
            mesh,
            &busy,
            |_| {},
            analysis::report,
            move |_: &Document, _: &Mesh, _: &PartReport, _: &Proposal| {
                held.hold();
                let nothing = CheckReport {
                    passed: false,
                    problems: vec![],
                    operations: vec![],
                    coverage: None,
                    total_minutes: 0.,
                    lathe_minutes: 0.,
                    manual_minutes: 0.,
                    cnc_minutes: 0.,
                    move_minutes: 0.,
                    zeros: vec![],
                    stocks: vec![],
                };
                (nothing, None, None)
            },
        );
        drop_while_held(run, &gate, &busy).await;
    }
    #[tokio::test]
    async fn a_dropped_run_counts_its_analysis_until_the_analysis_ends() {
        let (doc, mesh, _) = open_box();
        let (busy, gate) = (Busy::default(), Gate::default());
        let held = gate.clone();
        let script = Script::new(vec![]);
        let run = plan_with(
            &script,
            doc,
            mesh,
            &busy,
            |_| {},
            move |_: &Document, _: &Mesh| -> Result<PartReport, String> {
                held.hold();
                Err("held".into())
            },
            plan_check::evaluate_full,
        );
        drop_while_held(run, &gate, &busy).await;
        assert!(
            script.seen.lock().unwrap().is_empty(),
            "no request was made"
        );
    }
    #[test]
    fn clip_stops_reading_at_the_limit_with_the_same_result() {
        // Short text: the first `limit` characters of the whole cleaned line.
        for text in [
            "",
            " ",
            "a b",
            "  a \n\t b  ",
            "a\u{0}b\u{1b}[2J\u{202e}c \u{200b}d\u{feff}  e\u{e0041}",
            "x \u{7} \u{202e} y\u{2028}z",
            "é ö  ü",
        ] {
            let whole = proposal::clip(text, usize::MAX);
            for limit in 0..whole.chars().count() + 3 {
                let first: String = whole.chars().take(limit).collect();
                assert_eq!(proposal::clip(text, limit), first, "{text:?} {limit}");
            }
        }
        assert_eq!(proposal::clip("a b", 2), "a ");
        assert_eq!(proposal::clip(" \u{202e} a \n b ", 60), "a b");
        // A 5 MB text costs only the characters kept.
        let huge = "word ".repeat(1_000_000);
        let start = std::time::Instant::now();
        for _ in 0..50 {
            assert_eq!(proposal::clip(&huge, 11), "word word w");
        }
        let took = start.elapsed();
        assert!(took < std::time::Duration::from_secs(1), "{took:?}");
    }
    #[test]
    fn the_planner_future_can_run_on_any_thread() {
        fn assert_send<T: Send>(_: &T) {}
        let (doc, mesh, _) = block();
        let script = Script::new(vec![]);
        assert_send(&plan(&script, doc, mesh, &Busy::default(), |_| {}));
    }
    #[test]
    fn the_final_view_shows_the_stock_after_every_operation() {
        use crate::stock_surface::Detail;
        let (doc, mesh, report) = block();
        let job = proposal::build(&doc, &mesh, &report, &good(&report)).unwrap();
        // Meshed at Draft to keep the test fast; the app's view uses Standard,
        // through the same function.
        let view = final_view_at(&job, &mesh, Detail::Draft).unwrap();
        assert_eq!(view.plan.setup, job.setups[1].id);
        assert_eq!(view.plan.operation, job.setups[1].operations[1].id);
        assert!(
            view.stock_surface
                .as_ref()
                .is_some_and(|s| !s.triangles.is_empty())
        );
        assert_eq!(view.stock_surface_detail, Detail::Draft);
        assert_eq!(view.stock_surface_seconds, view.seconds);
        assert_eq!(view.preceding, 5);
        let mut empty = job.clone();
        empty.setups[1]
            .operations
            .iter_mut()
            .for_each(|o| o.enabled = false);
        assert!(final_view(&empty, &mesh).is_none());
    }
    #[tokio::test]
    async fn api_errors_and_unmet_preconditions_fail_at_once() {
        let (doc, mesh, _) = block();
        let script = Script::new(vec![Err(ApiError::UsageLimit)]);
        let failure = plan(&script, doc.clone(), mesh.clone(), &Busy::default(), |_| {})
            .await
            .unwrap_err();
        assert_eq!(
            (failure.api.clone(), failure.requests),
            (Some(ApiError::UsageLimit), 1)
        );
        assert!(failure.message.contains("Usage limit reached"));
        let mut skewed = doc.clone();
        skewed.setups[0].p.insert("rx".into(), "30".into());
        let script = Script::new(vec![]);
        let failure = plan(&script, skewed, mesh.clone(), &Busy::default(), |_| {})
            .await
            .unwrap_err();
        assert!(failure.message.contains("multiples of 90") && failure.requests == 0);
        assert!(script.seen.lock().unwrap().is_empty());
    }
    fn look_call(id: &str, json: serde_json::Value) -> ToolCall {
        let mut args = serde_json::json!({
            "orientation": null, "angle": "top", "focus_surfaces": [], "highlight_surfaces": [],
            "show_stock": true, "show_workholding": true, "labels": true, "operation": null, "purpose": "check"
        });
        for (k, v) in json.as_object().unwrap() {
            args[k] = v.clone();
        }
        ToolCall {
            call_id: id.into(),
            name: crate::look::NAME.into(),
            arguments: args.to_string(),
        }
    }
    fn cutting_call(id: &str, tool: u32) -> ToolCall {
        ToolCall {
            call_id: id.into(),
            name: planner_tools::CUTTING_DATA.into(),
            arguments: serde_json::json!({ "tool": tool }).to_string(),
        }
    }
    fn calls(calls: Vec<ToolCall>) -> Result<Reply, ApiError> {
        Ok(Reply {
            text: String::new(),
            structured: true,
            calls,
        })
    }
    fn text(proposal: &Proposal) -> Result<Reply, ApiError> {
        Ok(Reply {
            text: serde_json::to_string(proposal).unwrap(),
            structured: true,
            calls: vec![],
        })
    }
    #[tokio::test]
    async fn the_model_gets_an_overview_and_the_pictures_it_asks_for() {
        let (doc, mesh, report) = block();
        let script = Script::calls(vec![
            calls(vec![
                look_call("a", serde_json::json!({})),
                look_call("b", serde_json::json!({"focus_surfaces": [99999]})),
            ]),
            text(&good(&report)),
            text(&good(&report)),
        ]);
        let mut events = vec![];
        let outcome = plan(&script, doc, mesh, &Busy::default(), |e| events.push(e))
            .await
            .unwrap();
        assert_eq!(outcome.requests, 2, "tool rounds are not planner requests");
        let turns = script.turns.lock().unwrap();
        let (first, tool) = &turns[0];
        assert_eq!(*tool, Some(ToolChoice::Auto));
        assert!(first[0].as_text().unwrap().1.contains("PICTURES"));
        // The instructions promise the overview the run drew and sends.
        assert!(
            first[0]
                .as_text()
                .unwrap()
                .1
                .contains("overview picture drawn by the program")
        );
        assert!(
            matches!(&first[2], Item::Message(Role::User, parts) if parts.iter().any(|p| matches!(p, Part::Image { .. })))
        );
        let (second, _) = &turns[1];
        let n = second.len();
        assert!(matches!(&second[n - 5], Item::Call(c) if c.call_id == "a"));
        assert!(
            matches!(&second[n - 4], Item::Output { call_id, output } if call_id == "a" && output.contains("up,"))
        );
        assert!(matches!(&second[n - 3], Item::Call(c) if c.call_id == "b"));
        assert!(
            matches!(&second[n - 2], Item::Output { output, .. } if output.contains("Surface 99999"))
        );
        assert!(matches!(&second[n - 1], Item::Message(Role::User, parts) if parts.len() == 2));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Overview(l) if l.png.is_some()))
        );
        let looks: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Looked(1, l) => Some(l),
                _ => None,
            })
            .collect();
        assert_eq!(looks.len(), 2);
        assert!(looks[0].png.is_some() && looks[1].png.is_none());
    }
    #[tokio::test]
    async fn pictures_are_limited_per_request_and_the_next_request_may_look_again() {
        let (doc, mesh, report) = block();
        let many = |from: usize| {
            calls(
                (from..from + 4)
                    .map(|i| look_call(&i.to_string(), serde_json::json!({})))
                    .collect(),
            )
        };
        let script = Script::calls(vec![
            many(0),
            many(4),
            text(&good(&report)),
            text(&good(&report)),
        ]);
        let mut events = vec![];
        plan(&script, doc, mesh, &Busy::default(), |e| events.push(e))
            .await
            .unwrap();
        let turns = script.turns.lock().unwrap();
        // After 4 + 2 pictures the per-request limit is reached: the other
        // tools stay open, so the tools are still offered to be called.
        assert_eq!(turns[2].1, Some(ToolChoice::Auto));
        let outputs: Vec<_> = turns[2]
            .0
            .iter()
            .filter_map(|i| match i {
                Item::Output { output, .. } => Some(output.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            outputs
                .iter()
                .filter(|o| *o == "No more pictures for this request; reply with the proposal.")
                .count(),
            2
        );
        let images = turns[2]
            .0
            .iter()
            .flat_map(|i| match i {
                Item::Message(_, p) => p.clone(),
                _ => vec![],
            })
            .filter(|p| matches!(p, Part::Image { .. }))
            .count();
        assert_eq!(
            images, 7,
            "the overview and 6 pictures, all within the 8 kept"
        );
        // The next planner request may look again.
        assert_eq!(turns[3].1, Some(ToolChoice::Auto));
    }
    #[tokio::test]
    async fn a_model_that_never_stops_looking_fails_the_request() {
        let (doc, mesh, _) = block();
        let replies = (0..MAX_TOOL_ROUNDS + 2)
            .map(|i| {
                calls(vec![look_call(
                    &i.to_string(),
                    serde_json::json!({"labels": false}),
                )])
            })
            .collect();
        let script = Script::calls(replies);
        let failure = plan(&script, doc, mesh, &Busy::default(), |_| {})
            .await
            .unwrap_err();
        assert!(
            failure.message.contains("kept calling tools"),
            "{}",
            failure.message
        );
    }
    #[tokio::test]
    async fn a_refused_tool_then_refused_pictures_are_dropped_in_order() {
        let (doc, mesh, report) = block();
        let script = Script::calls(vec![
            Err(ApiError::BadRequest("tools".into())),
            Err(ApiError::Unsupported("images".into())),
            text(&good(&report)),
            text(&good(&report)),
        ]);
        let mut events = vec![];
        plan(&script, doc, mesh, &Busy::default(), |e| events.push(e))
            .await
            .unwrap();
        let drops: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Dropped(n, d) => Some((*n, *d)),
                _ => None,
            })
            .collect();
        assert_eq!(drops, vec![(1, Dropped::Tool), (1, Dropped::Pictures)]);
        let turns = script.turns.lock().unwrap();
        assert_eq!(turns[1].1, None);
        assert_eq!(turns[3].1, None, "the run remembers");
        assert!(!turns[3].0[0].as_text().unwrap().1.contains("PICTURES"));
    }
    #[tokio::test]
    async fn refused_pictures_are_dropped_from_the_whole_conversation() {
        let (doc, mesh, report) = block();
        let script = Script::calls(vec![
            calls(vec![look_call("a", serde_json::json!({}))]),
            Err(ApiError::BadRequest("tool".into())),
            Err(ApiError::BadRequest("images".into())),
            text(&good(&report)),
            text(&good(&report)),
        ]);
        plan(&script, doc, mesh, &Busy::default(), |_| {})
            .await
            .unwrap();
        let turns = script.turns.lock().unwrap();
        let last = &turns.last().unwrap().0;
        assert!(!last.iter().any(|i| matches!(i, Item::Message(_, p) if p.iter().any(|p| matches!(p, Part::Image { .. })))));
        assert!(
            !last
                .iter()
                .any(|i| matches!(i, Item::Call(_) | Item::Output { .. })),
            "calls become text once the tool is gone"
        );
        assert!(last.iter().any(|i| {
            i.as_text()
                .is_some_and(|(_, t)| t.contains("(Picture not sent: "))
        }));
    }
    #[tokio::test]
    async fn pictures_that_cannot_be_drawn_count_against_the_limits() {
        let (doc, mesh, report) = block();
        let unknown = (0..MAX_PICTURES_PER_REQUEST)
            .map(|i| {
                look_call(
                    &format!("x{i}"),
                    serde_json::json!({"focus_surfaces": [99999]}),
                )
            })
            .collect();
        let script = Script::calls(vec![
            calls(unknown),
            calls(vec![look_call("more", serde_json::json!({}))]),
            text(&good(&report)),
            text(&good(&report)),
        ]);
        plan(&script, doc, mesh, &Busy::default(), |_| {})
            .await
            .unwrap();
        let turns = script.turns.lock().unwrap();
        assert_eq!(turns[1].1, Some(ToolChoice::Auto));
        let output = turns[2].0.iter().find_map(|i| match i {
            Item::Output { call_id, output } if call_id == "more" => Some(output.as_str()),
            _ => None,
        });
        assert_eq!(
            output,
            Some("No more pictures for this request; reply with the proposal."),
            "six failed views use up the request"
        );
        assert_eq!(turns[2].1, Some(ToolChoice::Auto));
        // The next planner request may look again.
        assert_eq!(turns[3].1, Some(ToolChoice::Auto));
    }
    #[tokio::test]
    async fn calls_past_the_limit_each_get_a_sentence_and_are_not_run() {
        let (doc, mesh, report) = block();
        let eighteen = (0..18).map(|i| cutting_call(&format!("c{i}"), 3)).collect();
        let script = Script::calls(vec![calls(eighteen), text(&good(&report))]);
        plan(&script, doc, mesh, &Busy::default(), |_| {})
            .await
            .unwrap();
        let turns = script.turns.lock().unwrap();
        let outputs: Vec<(&str, &str)> = turns[1]
            .0
            .iter()
            .filter_map(|i| match i {
                Item::Output { call_id, output } => Some((call_id.as_str(), output.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(outputs.len(), 18, "every call has an answer");
        for (i, (id, output)) in outputs.iter().enumerate() {
            assert_eq!(*id, format!("c{i}"));
            assert_eq!(output.contains("Only the first 16 calls"), i >= MAX_CALLS);
        }
    }
    #[tokio::test]
    async fn after_the_runs_last_picture_view_model_is_refused_and_the_other_tools_stay_open() {
        let (doc, mesh, report) = block();
        let script = Script::calls(vec![
            calls(vec![
                look_call("late", serde_json::json!({})),
                cutting_call("data", 3),
            ]),
            text(&good(&report)),
        ]);
        let mut sight = Sight {
            pictures: true,
            tool: true,
            overview: false,
            edit: false,
        };
        let mut state = ToolState::new(look::Scene {
            doc: Arc::new(doc),
            mesh: Arc::new(mesh),
            report: Arc::new(report.clone()),
            job: None,
        });
        state.pictures_run = MAX_PICTURES_PER_RUN;
        let mut conversation = vec![Item::text(Role::User, "Part.")];
        let mut events = vec![];
        let text = ask(
            &script,
            &mut sight,
            &mut conversation,
            &mut state,
            &Busy::default(),
            1,
            &mut |e| events.push(e),
        )
        .await
        .unwrap();
        assert_eq!(text, serde_json::to_string(&good(&report)).unwrap());
        // The tools stay offered, and may be called: a tool switched off
        // would take cutting_data with it for the rest of the run.
        let turns = script.turns.lock().unwrap();
        assert_eq!(turns.len(), 2);
        assert!(
            turns
                .iter()
                .all(|(_, tool)| *tool == Some(ToolChoice::Auto))
        );
        assert_eq!(
            conversation[1..3],
            [
                Item::Call(look_call("late", serde_json::json!({}))),
                Item::Output {
                    call_id: "late".into(),
                    output: "No more pictures in this run; reply with the proposal.".into()
                },
            ]
        );
        assert!(
            matches!(&conversation[4], Item::Output { call_id, output }
                if call_id == "data" && output.contains("\"recommended\":11970")),
            "{:?}",
            conversation[4]
        );
        assert_eq!(
            state.pictures_run, MAX_PICTURES_PER_RUN,
            "a refused call is not counted"
        );
        assert!(
            matches!(&events[..], [Event::Looked(1, a), Event::Looked(1, b)]
                if a.png.is_none() && b.png.is_none()
                    && b.description == "Cutting data for T3: 11 970 rpm, 1 260 mm/min")
        );
    }
    #[test]
    fn only_the_last_eight_images_are_resent_and_the_rest_become_text() {
        let image = |i: usize| Part::Image {
            png: Arc::new(vec![i as u8]),
            alt: format!("p{i}"),
        };
        let message =
            |range: std::ops::Range<usize>| Item::Message(Role::User, range.map(image).collect());
        let call = ToolCall {
            call_id: "c".into(),
            name: look::NAME.into(),
            arguments: "{\"angle\":\"top\"}".into(),
        };
        let conversation = vec![
            Item::text(Role::User, "Part."),
            message(0..3),
            Item::Call(call),
            Item::Output {
                call_id: "c".into(),
                output: "Top view.".into(),
            },
            message(3..6),
            message(6..10),
        ];
        // Every image part, in order, as sent.
        let images = |sent: &[Item]| -> Vec<Part> {
            [1, 4, 5]
                .iter()
                .flat_map(|&k| match &sent[k] {
                    Item::Message(Role::User, parts) => parts.clone(),
                    other => panic!("{other:?}"),
                })
                .collect()
        };
        let sent = outgoing(&conversation, true, true);
        assert_eq!(sent.len(), conversation.len());
        assert_eq!(sent[..1], conversation[..1]);
        assert_eq!(
            sent[2..4],
            conversation[2..4],
            "calls stay while the tool is on"
        );
        let parts = images(&sent);
        assert_eq!(parts.len(), 10);
        for (i, part) in parts.iter().enumerate() {
            if i < 2 {
                assert_eq!(
                    *part,
                    Part::Text(format!("(Earlier picture, not resent: p{i})"))
                );
            } else {
                assert_eq!(*part, image(i), "{i}");
            }
        }
        // Pictures off: every image becomes its description.
        let parts = images(&outgoing(&conversation, false, true));
        for (i, part) in parts.iter().enumerate() {
            assert_eq!(*part, Part::Text(format!("(Picture not sent: p{i})")));
        }
        // Tool off: the call and its output become plain text.
        let sent = outgoing(&conversation, true, false);
        assert_eq!(
            sent[2],
            Item::text(
                Role::Assistant,
                "(Asked for a picture: {\"angle\":\"top\"})"
            )
        );
        assert_eq!(
            sent[3],
            Item::text(Role::User, "(Picture tool answered: Top view.)")
        );
        assert_eq!(
            images(&sent)[2..],
            images(&outgoing(&conversation, true, true))[2..]
        );
    }
    #[test]
    fn the_pictures_section_comes_before_the_rules_only_with_pictures() {
        let plain = instructions();
        assert_eq!(plain, instructions_with(false, false, true));
        assert!(!plain.contains("PICTURES") && !plain.contains("view_model"));
        assert!(!plain.contains("TOOLS") && !plain.contains("cutting_data"));
        let limits = format!(
            "at most {MAX_PICTURES_PER_REQUEST} pictures per request (the part report, or a check report the program sends back) and {MAX_PICTURES_PER_RUN} in the run"
        );
        for (tool, overview) in [(true, true), (false, true), (true, false), (false, false)] {
            let with = instructions_with(true, tool, overview);
            let (at, rules) = (
                with.find("\nPICTURES\n").unwrap(),
                with.find("\nRULES\n").unwrap(),
            );
            assert!(at < rules);
            // The tool and its limits are explained only while it is offered.
            assert_eq!(with.contains("view_model"), tool);
            assert_eq!(with.contains(&limits), tool);
            // The overview is promised only when the conversation has it.
            assert_eq!(with.contains("overview"), overview, "{tool} {overview}");
            assert!(with.contains("stock (amber, see-through)"));
            assert!(with.contains("vise or clamps (grey)"));
            assert!(with.contains("the triad shows +X (red)"));
            assert!(with.contains("the report's numbers are right"));
            // Without the section the text is the same as a run without
            // pictures, but for the TOOLS section's line about the picture tool.
            let without = with.replacen(&with[at + 1..rules + 1], "", 1);
            assert_eq!(
                without
                    .lines()
                    .filter(|l| !l.starts_with("- `view_model`"))
                    .collect::<Vec<_>>(),
                instructions_with(false, tool, overview)
                    .lines()
                    .collect::<Vec<_>>(),
                "{tool} {overview}"
            );
        }
    }
    #[tokio::test]
    async fn once_the_tool_is_refused_the_instructions_no_longer_offer_it() {
        let (doc, mesh, report) = block();
        let script = Script::calls(vec![
            Err(ApiError::BadRequest("tools".into())),
            text(&good(&report)),
            text(&good(&report)),
        ]);
        let mut events = vec![];
        plan(&script, doc, mesh, &Busy::default(), |e| events.push(e))
            .await
            .unwrap();
        assert!(events.contains(&Event::Dropped(1, Dropped::Tool)));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::Dropped(_, Dropped::Pictures)))
        );
        let turns = script.turns.lock().unwrap();
        assert!(turns[0].0[0].as_text().unwrap().1.contains("view_model"));
        for (items, tool) in &turns[1..] {
            assert_eq!(*tool, None);
            let system = items[0].as_text().unwrap().1;
            assert!(system.contains("PICTURES"), "{system}");
            assert!(system.contains("overview picture drawn by the program"));
            assert!(!system.contains("view_model") && !system.contains("pictures per request"));
            // The overview itself is still sent, with its description.
            assert!(matches!(&items[2], Item::Message(Role::User, parts)
                if matches!(&parts[0], Part::Text(t) if t.starts_with("The program's overview picture: "))
                    && matches!(&parts[1], Part::Image { .. })));
        }
        assert!(instructions_with(true, true, true).contains("view_model"));
    }
    #[tokio::test]
    async fn without_pictures_the_tools_are_still_offered_and_nothing_else_changes() {
        let (doc, mesh, report) = block();
        let script = Script::new(vec![json(&good(&report)), json(&good(&report))]);
        plan(&script, doc, mesh, &Busy::default(), |_| {})
            .await
            .unwrap();
        let turns = script.turns.lock().unwrap();
        // Before the tools, a run without pictures offered nothing; now it
        // offers the tools that need no pictures, and not view_model.
        assert!(
            turns
                .iter()
                .all(|(_, tool)| *tool == Some(ToolChoice::Auto))
        );
        assert!(
            script
                .offered
                .lock()
                .unwrap()
                .iter()
                .all(|names| names == &PART_TOOLS)
        );
        assert_eq!(turns[0].0.len(), 2, "system and part report only");
        let system = turns[0].0[0].as_text().unwrap().1;
        assert!(!system.contains("PICTURES") && !system.contains("view_model"));
        assert!(system.contains("\nTOOLS\n") && system.contains("`cutting_data`"));
        assert_eq!(system, instructions_with(false, true, false));
    }
    #[tokio::test]
    async fn tool_calls_are_answered_with_json_and_each_gets_a_line_in_the_timeline() {
        let (doc, mesh, report) = block();
        let surface = report.orientations[0].planes[0].surfaces[0];
        let surface_call = ToolCall {
            call_id: "b".into(),
            name: planner_tools::SURFACE_INFO.into(),
            arguments: serde_json::json!({ "surface": surface }).to_string(),
        };
        let unknown = ToolCall {
            call_id: "c".into(),
            name: planner_tools::CUTTING_DATA.into(),
            arguments: serde_json::json!({ "tool": 99 }).to_string(),
        };
        let script = Script::calling(vec![
            calls(vec![cutting_call("a", 3), surface_call, unknown]),
            text(&good(&report)),
            text(&good(&report)),
        ]);
        let mut events = vec![];
        let outcome = plan(&script, doc, mesh, &Busy::default(), |e| events.push(e))
            .await
            .unwrap();
        assert_eq!(outcome.requests, 2, "tool rounds are not planner requests");
        let turns = script.turns.lock().unwrap();
        let second = &turns[1].0;
        let outputs: Vec<(&str, &str)> = second
            .iter()
            .filter_map(|item| match item {
                Item::Output { call_id, output } => Some((call_id.as_str(), output.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(outputs.len(), 3);
        let data: serde_json::Value = serde_json::from_str(outputs[0].1).unwrap();
        assert_eq!(data["rpm"]["recommended"], 11970);
        let info: serde_json::Value = serde_json::from_str(outputs[1].1).unwrap();
        assert_eq!(info["surface"], surface);
        assert_eq!(outputs[2].1, "The part report has no tool number 99.");
        // No picture goes to the model: only the three answers follow the calls.
        assert!(matches!(second.last(), Some(Item::Output { call_id, .. }) if call_id == "c"));
        // One line each under the request, with no picture.
        let looks: Vec<&Look> = events
            .iter()
            .filter_map(|e| match e {
                Event::Looked(1, look) => Some(look),
                _ => None,
            })
            .collect();
        assert_eq!(looks.len(), 3);
        assert!(
            looks
                .iter()
                .all(|l| l.png.is_none() && l.purpose.is_empty())
        );
        assert_eq!(
            looks[0].description,
            "Cutting data for T3: 11 970 rpm, 1 260 mm/min"
        );
        assert!(
            looks[1]
                .description
                .starts_with(&format!("Surface {surface}: "))
        );
        assert_eq!(
            looks[2].description,
            "The part report has no tool number 99."
        );
        // The timeline shows such a line bare, as it shows an error sentence.
        assert_eq!(crate::ui::look_line(looks[0]), looks[0].description);
    }
    #[tokio::test]
    async fn an_unknown_tool_is_answered_and_the_run_goes_on() {
        let (doc, mesh, report) = block();
        let nothing = ToolCall {
            call_id: "x".into(),
            name: "make_coffee".into(),
            arguments: "{}".into(),
        };
        let script = Script::calling(vec![
            calls(vec![nothing]),
            text(&good(&report)),
            text(&good(&report)),
        ]);
        plan(&script, doc, mesh, &Busy::default(), |_| {})
            .await
            .unwrap();
        let turns = script.turns.lock().unwrap();
        assert!(
            turns[1]
                .0
                .iter()
                .any(|i| matches!(i, Item::Output { call_id, output }
            if call_id == "x" && output == "There is no tool named make_coffee."))
        );
    }
    #[tokio::test]
    async fn sixteen_tool_rounds_are_allowed_and_a_seventeenth_fails_the_request() {
        assert_eq!(MAX_TOOL_ROUNDS, 16);
        let (doc, mesh, report) = block();
        let mut replies: Vec<_> = (0..MAX_TOOL_ROUNDS)
            .map(|i| calls(vec![cutting_call(&i.to_string(), 3)]))
            .collect();
        replies.push(text(&good(&report)));
        replies.push(text(&good(&report)));
        let script = Script::calling(replies);
        let outcome = plan(&script, doc.clone(), mesh.clone(), &Busy::default(), |_| {})
            .await
            .unwrap();
        assert_eq!(outcome.requests, 2);
        // Each of the sixteen rounds may call a tool; the send after the
        // last may not, so the model has to answer.
        let choices: Vec<_> = script.turns.lock().unwrap().iter().map(|t| t.1).collect();
        let expected: Vec<_> = (0..MAX_TOOL_ROUNDS + 2)
            .map(|n| {
                Some(if n == MAX_TOOL_ROUNDS {
                    ToolChoice::None
                } else {
                    ToolChoice::Auto
                })
            })
            .collect();
        assert_eq!(choices, expected);
        // One round too many fails, and says so.
        let replies = (0..=MAX_TOOL_ROUNDS)
            .map(|i| calls(vec![cutting_call(&i.to_string(), 3)]))
            .collect();
        let script = Script::calling(replies);
        let failure = plan(&script, doc, mesh, &Busy::default(), |_| {})
            .await
            .unwrap_err();
        assert!(
            failure.message.contains("kept calling tools"),
            "{}",
            failure.message
        );
        assert_eq!(script.turns.lock().unwrap().len(), MAX_TOOL_ROUNDS + 1);
    }
    #[tokio::test]
    async fn the_round_count_starts_again_with_each_planner_request() {
        let (doc, mesh, report) = block();
        // Sixteen rounds in the first request, sixteen in the second: the
        // count is per planner request.
        let round = |n: usize| calls(vec![cutting_call(&n.to_string(), 3)]);
        let mut replies: Vec<_> = (0..MAX_TOOL_ROUNDS).map(round).collect();
        replies.push(text(&good(&report)));
        replies.extend((0..MAX_TOOL_ROUNDS).map(round));
        replies.push(text(&good(&report)));
        let script = Script::calling(replies);
        let outcome = plan(&script, doc, mesh, &Busy::default(), |_| {})
            .await
            .unwrap();
        assert_eq!(outcome.requests, 2);
    }
    #[tokio::test]
    async fn a_refused_tool_request_drops_all_the_tools_and_their_instructions() {
        // Without pictures: the request is sent again with no tools at all.
        let (doc, mesh, report) = block();
        let script = Script::calling(vec![
            Err(ApiError::BadRequest("tools".into())),
            text(&good(&report)),
            text(&good(&report)),
        ]);
        let mut events = vec![];
        plan(&script, doc, mesh, &Busy::default(), |e| events.push(e))
            .await
            .unwrap();
        let drops: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Dropped(n, d) => Some((*n, *d)),
                _ => None,
            })
            .collect();
        assert_eq!(drops, vec![(1, Dropped::Tool)]);
        {
            let offered = script.offered.lock().unwrap();
            assert_eq!(offered[0], PART_TOOLS);
            assert!(
                offered[1..].iter().all(|names| names.is_empty()),
                "{offered:?}"
            );
            let turns = script.turns.lock().unwrap();
            assert!(turns[1..].iter().all(|(_, tool)| tool.is_none()));
            for (items, _) in &turns[1..] {
                let system = items[0].as_text().unwrap().1;
                assert!(!system.contains("TOOLS") && !system.contains("cutting_data"));
                assert_eq!(system, instructions_with(false, false, false));
            }
        }
        // With pictures: view_model goes with the others; the pictures stay.
        let (doc, mesh, report) = block();
        let script = Script::calls(vec![
            Err(ApiError::Unsupported("tools".into())),
            text(&good(&report)),
            text(&good(&report)),
        ]);
        let mut events = vec![];
        plan(&script, doc, mesh, &Busy::default(), |e| events.push(e))
            .await
            .unwrap();
        assert!(events.contains(&Event::Dropped(1, Dropped::Tool)));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::Dropped(_, Dropped::Pictures)))
        );
        let offered = script.offered.lock().unwrap();
        assert_eq!(offered[0][0], look::NAME);
        assert_eq!(offered[0][1..], PART_TOOLS);
        assert!(
            offered[1..].iter().all(|names| names.is_empty()),
            "{offered:?}"
        );
    }
    #[tokio::test]
    async fn a_second_refusal_drops_the_pictures_after_the_tools() {
        let (doc, mesh, report) = block();
        let script = Script::calls(vec![
            Err(ApiError::BadRequest("tools".into())),
            Err(ApiError::BadRequest("images".into())),
            text(&good(&report)),
            text(&good(&report)),
        ]);
        let mut events = vec![];
        plan(&script, doc, mesh, &Busy::default(), |e| events.push(e))
            .await
            .unwrap();
        let drops: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Dropped(n, d) => Some((*n, *d)),
                _ => None,
            })
            .collect();
        assert_eq!(drops, vec![(1, Dropped::Tool), (1, Dropped::Pictures)]);
    }
    #[tokio::test]
    async fn a_refusal_naming_only_the_images_drops_the_pictures_and_keeps_the_tools() {
        let (doc, mesh, report) = block();
        let script = Script::calls(vec![
            Err(ApiError::BadRequest(
                "Invalid value: 'image_url'. (parameter: input[3].content[1].image_url)".into(),
            )),
            text(&good(&report)),
            text(&good(&report)),
        ]);
        let mut events = vec![];
        plan(&script, doc, mesh, &Busy::default(), |e| events.push(e))
            .await
            .unwrap();
        let drops: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                Event::Dropped(n, d) => Some((*n, *d)),
                _ => None,
            })
            .collect();
        assert_eq!(drops, vec![(1, Dropped::Pictures)]);
        let offered = script.offered.lock().unwrap();
        assert_eq!(offered[0][0], look::NAME);
        assert_eq!(offered[0][1..], PART_TOOLS);
        assert!(
            offered[1..].iter().all(|names| names[..] == PART_TOOLS),
            "{offered:?}"
        );
        // A refusal that names both, or `Unsupported`, still drops the tools first.
        let sight = Sight {
            pictures: true,
            tool: true,
            overview: true,
            edit: false,
        };
        for error in [
            ApiError::BadRequest("tools and images".into()),
            ApiError::Unsupported("images".into()),
        ] {
            assert_eq!(
                dropped_for(&error, &sight),
                Some(Dropped::Tool),
                "{error:?}"
            );
        }
        assert_eq!(
            dropped_for(&ApiError::BadRequest("image_url".into()), &sight),
            Some(Dropped::Pictures)
        );
    }
    #[tokio::test]
    async fn a_bad_request_that_names_neither_tools_nor_pictures_ends_the_run_at_once() {
        use crate::fake_openai::{Fake, Resp};
        use crate::session::SessionStore;
        let fake = Fake::start(|_| {
            Resp::json(
                400,
                serde_json::json!({"error": {
                    "message": "Invalid 'max_output_tokens': integer below minimum value.",
                    "type": "invalid_request_error",
                    "param": "max_output_tokens",
                    "code": "integer_below_min_value",
                }}),
            )
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(crate::session::FileStore(dir.path().join("s.json")));
        let session = crate::session::sample();
        store.save(&session).unwrap();
        let client = crate::openai::Client::new(
            crate::oauth::Endpoints::local(&fake.base),
            "oaiapp_1".into(),
            store,
            session,
        );
        let backend = OpenAiBackend::new(
            std::sync::Arc::new(client),
            "gpt-6.1-sol".into(),
            std::sync::Arc::new(std::sync::Mutex::new(String::new())),
            true,
        );
        let (doc, mesh, _) = block();
        let mut events = vec![];
        let failure = plan(&backend, doc, mesh, &Busy::default(), |e| events.push(e))
            .await
            .unwrap_err();
        assert!(
            matches!(&failure.api, Some(ApiError::BadRequest(m)) if m.contains("max_output_tokens")),
            "{failure:?}"
        );
        assert_eq!(fake.seen("/v1/responses").len(), 1, "no re-sends");
        assert!(
            !events.iter().any(|e| matches!(e, Event::Dropped(..))),
            "{events:?}"
        );
    }
    #[test]
    fn only_a_bad_request_naming_tools_functions_or_pictures_is_a_refusal_of_them() {
        for named in [
            "tools",
            "Unknown parameter: 'tools[0].strict'.",
            "Invalid schema for function 'view_model'",
            "Invalid value: 'input_image'.",
            "images are not supported (parameter: input)",
            "Pictures are not supported",
        ] {
            assert!(refuses_tools_or_pictures(named), "{named}");
        }
        for unrelated in [
            "OpenAI returned HTTP 400.",
            "Invalid 'max_output_tokens': integer below minimum value.",
            "This model's maximum context length is 200000 tokens. (parameter: messages)",
            "The model `gpt-9` does not exist. (parameter: model)",
        ] {
            assert!(!refuses_tools_or_pictures(unrelated), "{unrelated}");
        }
    }
    #[tokio::test]
    async fn without_tools_or_pictures_a_refusal_is_the_requests_error() {
        let (doc, mesh, _) = block();
        let script = Script::calling(vec![
            Err(ApiError::BadRequest("tools".into())),
            Err(ApiError::BadRequest("still".into())),
        ]);
        let failure = plan(&script, doc, mesh, &Busy::default(), |_| {})
            .await
            .unwrap_err();
        assert_eq!(failure.api, Some(ApiError::BadRequest("still".into())));
    }
    #[test]
    fn the_tools_section_lists_each_offered_tool_with_its_limits() {
        // No tools offered, no section and no tool named.
        let none = instructions_with(true, false, true);
        assert!(!none.contains("\nTOOLS\n") && !none.contains("cutting_data"));
        assert!(!none.contains("surface_info") && !none.contains("check_plan"));
        // Without pictures: the part tools, not view_model.
        let text = instructions_with(false, true, false);
        let tools = section(&text, "TOOLS");
        assert!(tools.contains("`cutting_data`") && tools.contains("`surface_info`"));
        assert!(!text.contains("view_model") && !text.contains("PICTURES"));
        assert!(tools.contains(&format!(
            "a request allows at most {MAX_TOOL_ROUNDS} rounds"
        )));
        // One word for one thing: a request is what the program sends, a
        // reply what the model sends back.
        assert!(
            tools
                .contains("A request is the part report, or a check report the program sends back")
        );
        assert!(!text.contains("a reply allows"));
        // The limit of calls is the model's: per reply.
        assert!(tools.contains("runs at most 16 calls per reply"));
        assert!(tools.contains("No limit"));
        let checks = format!(
            "At most {} per request and {} in the run",
            planner_tools::MAX_CHECKS_PER_REQUEST,
            planner_tools::MAX_CHECKS_PER_RUN
        );
        for needed in [
            "`tool`",
            "feed range",
            "recommended rpm",
            "`surface`",
            "`surface_area`",
            "`mean_normal`",
            "setup 1's frame",
            "- `check_plan` (`proposal`",
            "- `stock_after` (`setup`",
            "- `check_holding` (`setup`",
            "`holder`",
            "`stocks`",
            "`box_from_part_corner`",
            "`removed`",
            "`by`",
            "`holds`",
            "`problems`",
            "latest draft",
            checks.as_str(),
        ] {
            assert!(tools.contains(needed), "{needed}\n{tools}");
        }
        // Each tool on its line, in the order they are offered.
        let at = |name: &str| tools.find(&format!("- `{name}`")).unwrap();
        assert!(at("check_plan") < at("stock_after") && at("stock_after") < at("check_holding"));
        assert!(at("check_holding") < at("cutting_data"));
        // With pictures: view_model too, with the picture limits.
        let text = instructions_with(true, true, true);
        let tools = section(&text, "TOOLS");
        assert!(tools.contains("`view_model`"));
        assert!(tools.contains(&format!(
            "At most {MAX_PICTURES_PER_REQUEST} pictures per request and {MAX_PICTURES_PER_RUN} in the run"
        )));
        // The section sits before the rules, after the speeds and feeds.
        let at = |title: &str| text.find(&format!("\n{title}\n")).unwrap();
        assert!(at("SPEEDS AND FEEDS") < at("TOOLS") && at("TOOLS") < at("RULES"));
        // The holding rules name the tools that test a draft only with tools.
        assert!(text.contains(holding_rules(true).trim_end()));
        assert!(instructions_with(true, false, true).contains(holding_rules(false).trim_end()));
        assert!(text.contains("`check_plan`") && !none.contains("`check_plan`"));
        // The check report's stocks are explained with or without tools.
        let rules = section(&instructions(), "RULES");
        assert!(
            rules.contains("`stocks` gives the stock each setup starts from"),
            "{rules}"
        );
    }
    #[test]
    fn a_null_holder_means_setup_1s_entry_or_no_modelled_holding() {
        let text = instructions();
        let holding = section(&text, "PLANNING THE HOLDING");
        for needed in [
            "whose `number` is given in the report's `workholding`",
            "absent when setup 1 models no vise or clamps",
            "null means the same holder as setup 1 when setup 1 has an entry",
            "no modelled holding when it has none",
        ] {
            assert!(holding.contains(needed), "{needed}\n{holding}");
        }
        assert!(text.contains("with its `number` in `workholding_library`"));
    }
    #[tokio::test]
    async fn the_report_names_setup_1s_library_number_in_its_workholding() {
        use crate::proposal::tests::hold;
        let (mut doc, mesh, _) = block();
        hold(&mut doc, "Vise", &[("vise_axis", "Y"), ("vise_grip", "8")]);
        let report = analysis::report(&doc, &mesh).unwrap();
        let script = Script::new(vec![json(&good(&report)), json(&good(&report))]);
        let _ = plan(&script, doc.clone(), mesh.clone(), &Busy::default(), |_| {}).await;
        let part = script.seen.lock().unwrap()[0][1].1.clone();
        let sent: serde_json::Value = serde_json::from_str(&part).unwrap();
        assert_eq!(sent["workholding"]["kind"], "Vise");
        assert_eq!(sent["workholding"]["number"], 1);
        assert_eq!(sent["workholding_library"][0]["number"], 1);
        // Without holding on setup 1 there is no `workholding` and no number.
        let (doc, mesh, report) = block();
        let script = Script::new(vec![json(&good(&report)), json(&good(&report))]);
        let _ = plan(&script, doc, mesh, &Busy::default(), |_| {}).await;
        let sent: serde_json::Value =
            serde_json::from_str(&script.seen.lock().unwrap()[0][1].1).unwrap();
        assert!(sent.get("workholding").is_none());
    }
    fn tool_call(id: &str, name: &str, arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            call_id: id.into(),
            name: name.into(),
            arguments: arguments.to_string(),
        }
    }
    #[tokio::test]
    async fn the_planners_own_check_of_an_answer_becomes_the_latest_draft_for_the_tools() {
        let (mut doc, mesh, _) = block();
        doc.adopt_holders();
        for holder in &mut doc.workholding {
            holder.p.insert("name".into(), "ZZ-SECRET-HOLDER".into());
            holder.p.insert("notes".into(), "ZZ-SECRET-NOTE".into());
        }
        doc.title = "ZZ-SECRET-TITLE".into();
        doc.setups[0]
            .p
            .insert("name".into(), "ZZ-SECRET-SETUP".into());
        let report = analysis::report(&doc, &mesh).unwrap();
        let mut bad = good(&report);
        bad.setups[0].operations[0].tool = 9;
        let no_draft = "Check a draft with check_plan first, or answer with a proposal.";
        let script = Script::calling(vec![
            // Checked by the planner: it passes, with faces still covered.
            text(&good(&report)),
            // The tools read the answer the planner checked.
            calls(vec![
                tool_call("a", "stock_after", serde_json::json!({"setup": 2})),
                tool_call(
                    "b",
                    "check_holding",
                    serde_json::json!({"setup": 2, "holder": 2}),
                ),
            ]),
            // A draft that does not build is the latest draft now.
            calls(vec![tool_call(
                "c",
                "check_plan",
                serde_json::json!({ "proposal": bad }),
            )]),
            calls(vec![tool_call(
                "d",
                "stock_after",
                serde_json::json!({"setup": 2}),
            )]),
            Ok(Reply {
                text: "Not a plan.".into(),
                structured: true,
                calls: vec![],
            }),
        ]);
        let mut events = vec![];
        let outcome = plan(&script, doc, mesh, &Busy::default(), |e| events.push(e))
            .await
            .unwrap();
        assert_eq!((outcome.requests, outcome.chosen), (2, 1));
        // The feedback on the answer carries the stock before each setup.
        let seen = script.seen.lock().unwrap();
        assert!(seen[1][3].1.contains("\"stocks\":[{\"setup\":1,"));
        let turns = script.turns.lock().unwrap();
        let outputs: Vec<(&str, &str)> = turns
            .last()
            .unwrap()
            .0
            .iter()
            .filter_map(|item| match item {
                Item::Output { call_id, output } => Some((call_id.as_str(), output.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(outputs.len(), 4);
        assert_eq!(
            outputs[0],
            (
                "a",
                serde_json::to_string(&outcome.check.stocks[1])
                    .unwrap()
                    .as_str()
            )
        );
        // Setup 2's Face runs over the whole stock top, where clamps would
        // sit: the plan check would refuse them, and so does check_holding.
        let held: serde_json::Value = serde_json::from_str(outputs[1].1).unwrap();
        assert_eq!(held["holds"], false, "{held}");
        assert!(
            held["problems"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["message"]
                    .as_str()
                    .unwrap()
                    .contains("the tool hits the clamp on")),
            "{held}"
        );
        let checked: serde_json::Value = serde_json::from_str(outputs[2].1).unwrap();
        assert_eq!(checked["passed"], false);
        assert_eq!(outputs[3], ("d", no_draft));
        // One line each under request 2.
        let lines: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                Event::Looked(2, look) => Some(look.description.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(
            lines[0],
            "Stock before setup 2: 124 × 84 × 43 mm; bottom cut 4 mm by setup 1"
        );
        assert!(
            lines[1].starts_with("Clamp 2 for setup 2: ")
                && lines[1].contains(" problem")
                && lines[1].contains(" (clamps on "),
            "{}",
            lines[1]
        );
        assert_eq!(lines[2..], ["Checked a draft: 1 problem", no_draft]);
        // No title, setup name, entry name or note in what the tools said.
        let said = format!("{outputs:?} {lines:?}");
        assert!(!said.contains("ZZ-SECRET"), "{said}");
    }
    #[test]
    fn calls_and_outputs_of_any_tool_become_plain_text_once_the_tools_are_gone() {
        let call = |id: &str, name: &str, arguments: &str| ToolCall {
            call_id: id.into(),
            name: name.into(),
            arguments: arguments.into(),
        };
        let conversation = vec![
            Item::text(Role::User, "Part."),
            Item::Call(call("a", look::NAME, "{\"angle\":\"top\"}")),
            Item::Output {
                call_id: "a".into(),
                output: "Top view.".into(),
            },
            Item::Call(call("b", "cutting_data", "{\"tool\":3}")),
            Item::Output {
                call_id: "b".into(),
                output: "{\"rpm\":1}".into(),
            },
        ];
        let sent = outgoing(&conversation, true, true);
        assert_eq!(sent, conversation, "calls stay while the tools are on");
        let sent = outgoing(&conversation, true, false);
        assert_eq!(
            sent[1],
            Item::text(
                Role::Assistant,
                "(Asked for a picture: {\"angle\":\"top\"})"
            )
        );
        assert_eq!(
            sent[2],
            Item::text(Role::User, "(Picture tool answered: Top view.)")
        );
        assert_eq!(
            sent[3],
            Item::text(Role::Assistant, "(Called cutting_data: {\"tool\":3})")
        );
        assert_eq!(
            sent[4],
            Item::text(Role::User, "(cutting_data answered: {\"rpm\":1})")
        );
    }
    /// The block with the good plan built in, its setups named "Top" and
    /// "Underside": a job a change can start from.
    fn planned() -> (Document, Mesh, PartReport) {
        let (doc, mesh, report) = block();
        let job = proposal::build(&doc, &mesh, &report, &good(&report)).unwrap();
        (job, mesh, report)
    }
    /// The reply that changes nothing: the current plan as the planner is
    /// shown it (its setups named by number), with a summary.
    fn unchanged(doc: &Document, report: &PartReport) -> Proposal {
        let mut plan = proposal::describe(&anonymized(doc), report)
            .unwrap()
            .current_plan;
        plan.summary = "Nothing to change.".into();
        plan
    }
    /// `doc` with every operation taken out: a job nothing is planned in yet.
    fn without_operations(mut doc: Document) -> Document {
        doc.setups.iter_mut().for_each(|s| s.operations.clear());
        doc
    }
    fn asking(request: &str) -> Chat {
        Chat {
            turns: vec![],
            request: request.into(),
        }
    }
    /// A run of `plan_with_chat` with the real plan check.
    async fn change(
        script: &Script,
        doc: &Document,
        mesh: &Mesh,
        chat: Chat,
    ) -> Result<Outcome, Failure> {
        plan_with_chat(
            script,
            doc.clone(),
            mesh.clone(),
            chat,
            &Busy::default(),
            |_| {},
            analysis::report,
            check_or_boom,
        )
        .await
    }
    #[test]
    fn a_run_changes_the_plan_when_the_user_asked_and_the_job_has_an_operation() {
        let (planned, _, _) = planned();
        let empty = without_operations(planned.clone());
        assert!(editing(&planned, &asking("Drill the holes first.")));
        // No request, or one with nothing in it.
        for blank in ["", "   ", "\n\t", "\u{200b}\u{7}"] {
            assert!(!editing(&planned, &asking(blank)), "{blank:?}");
        }
        assert!(!editing(&planned, &Chat::default()));
        // No operation to change: a request is what to plan.
        assert!(!editing(&empty, &asking("Drill the holes first.")));
        // An operation in a later setup is enough.
        let mut later = planned.clone();
        later.setups[0].operations.clear();
        assert!(editing(&later, &asking("Drill the holes first.")));
        later.setups[1].operations.clear();
        assert!(!editing(&later, &asking("Drill the holes first.")));
        // Earlier exchanges alone do not ask for anything.
        let turns = vec![Turn {
            request: "Before.".into(),
            answer: "Done.".into(),
        }];
        assert!(!editing(
            &planned,
            &Chat {
                turns,
                request: String::new()
            }
        ));
    }
    #[test]
    fn the_chat_sends_the_latest_exchanges_then_the_request() {
        let turns: Vec<Turn> = (0..KEPT_TURNS + 3)
            .map(|i| Turn {
                request: format!("ask {i}"),
                answer: if i == KEPT_TURNS + 2 {
                    String::new()
                } else {
                    format!("answer {i}")
                },
            })
            .collect();
        let chat = Chat {
            turns,
            request: "now".into(),
        };
        let items = chat.messages(Some("{\"current_plan\":{}}"));
        let texts: Vec<(Role, String)> = items.iter().filter_map(Item::as_text).collect();
        assert_eq!(texts.len(), 2 * KEPT_TURNS + 1);
        for (n, i) in (3..KEPT_TURNS + 3).enumerate() {
            assert_eq!(
                texts[2 * n],
                (Role::User, format!("Earlier the user asked: ask {i}"))
            );
            let answer = if i == KEPT_TURNS + 2 {
                "(no answer)".to_string()
            } else {
                format!("answer {i}")
            };
            assert_eq!(texts[2 * n + 1], (Role::Assistant, answer));
        }
        assert_eq!(
            texts.last().unwrap(),
            &(
                Role::User,
                "The job's current plan: {\"current_plan\":{}}\n\nThe user asks for this change: now"
                    .to_string()
            )
        );
        // Without a plan to change, the request is what to plan.
        let fresh = chat.messages(None);
        let last = fresh.last().and_then(Item::as_text).unwrap();
        assert_eq!(
            last,
            (
                Role::User,
                "Plan the job, and follow this request of the user where the rules allow it: now"
                    .to_string()
            )
        );
        // No request, nothing is sent; a long one is cut.
        assert!(Chat::default().messages(Some("{}")).is_empty());
        let blank = Chat {
            request: " \n ".into(),
            ..chat.clone()
        };
        assert!(blank.messages(Some("{}")).is_empty());
        let long = Chat {
            turns: vec![Turn {
                request: "q".repeat(MAX_REQUEST_CHARS + 50),
                answer: "a".repeat(MAX_REQUEST_CHARS + 50),
            }],
            request: "r".repeat(MAX_REQUEST_CHARS + 50),
        };
        for item in long.messages(None) {
            let (_, text) = item.as_text().unwrap();
            assert!(text.matches(['q', 'a', 'r']).count() >= MAX_REQUEST_CHARS);
            assert!(!text.contains(&"q".repeat(MAX_REQUEST_CHARS + 1)));
            assert!(!text.contains(&"a".repeat(MAX_REQUEST_CHARS + 1)));
            assert!(!text.contains(&"r".repeat(MAX_REQUEST_CHARS + 1)));
        }
    }
    #[tokio::test]
    async fn a_change_sends_the_current_plan_and_the_request_after_the_part_report() {
        let (doc, mesh, report) = planned();
        let script = Script::new(vec![json(&unchanged(&doc, &report))]);
        let outcome = change(&script, &doc, &mesh, asking("Drill the holes first."))
            .await
            .unwrap();
        assert_eq!((outcome.requests, outcome.chosen), (1, 1));
        let seen = script.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let roles: Vec<Role> = seen[0].iter().map(|(role, _)| *role).collect();
        assert_eq!(roles, [Role::System, Role::User, Role::User]);
        let (system, part, ask) = (&seen[0][0].1, &seen[0][1].1, &seen[0][2].1);
        // The instructions end with the section on changing a plan.
        assert!(system.ends_with(EDITING));
        assert!(system.contains("CHANGING THE CURRENT PLAN"));
        assert!(system.contains("`from`"));
        let sent: serde_json::Value = serde_json::from_str(part).unwrap();
        assert!(sent["orientations"].is_array());
        // The plan as the planner is shown it, then the request.
        let (plan, request) = ask
            .strip_prefix("The job's current plan: ")
            .unwrap()
            .split_once("\n\nThe user asks for this change: ")
            .unwrap();
        assert_eq!(request, "Drill the holes first.");
        let shown: serde_json::Value = serde_json::from_str(plan).unwrap();
        let described = proposal::describe(&anonymized(&doc), &report).unwrap();
        assert_eq!(shown, serde_json::to_value(&described).unwrap());
        assert_eq!(shown["current_plan"]["setups"][0]["from"], 1);
        assert_eq!(
            shown["current_plan"]["setups"][1]["operations"][0]["from"],
            5
        );
        assert_eq!(shown["kept_operations"], serde_json::json!([]));
        // The user's own setup names do not leave the program.
        assert!(!plan.contains("Underside") && plan.contains("Setup 2"));
        // The reply follows the schema with `from`, and so do check_plan's drafts.
        assert_eq!(*script.edits.lock().unwrap(), [true]);
        let schemas = script.check_schemas.lock().unwrap();
        assert_eq!(schemas[0], Some(proposal::schema_for(true)));
        // The job keeps its setups: their ids and, as the user named them, their names.
        let names: Vec<&str> = outcome.job.setups.iter().map(|s| s.name()).collect();
        assert_eq!(names, ["Top", "Underside"]);
        for (kept, user) in outcome.job.setups.iter().zip(&doc.setups) {
            assert_eq!(kept.id, user.id);
            for key in ["offset", "notes", "rx", "ry", "rz"] {
                assert_eq!(get(&kept.p, key), get(&user.p, key), "{key}");
            }
            let ids =
                |s: &Setup| -> Vec<String> { s.operations.iter().map(|o| o.id.clone()).collect() };
            assert_eq!(ids(kept), ids(user));
        }
        assert_eq!(outcome.job.title, doc.title);
    }
    #[tokio::test]
    async fn a_setup_the_planner_renamed_keeps_the_planners_name() {
        let (doc, mesh, report) = planned();
        let mut plan = unchanged(&doc, &report);
        plan.setups[1].name = "Flipped over".into();
        let script = Script::new(vec![json(&plan)]);
        let outcome = change(&script, &doc, &mesh, asking("Rename setup 2."))
            .await
            .unwrap();
        let names: Vec<&str> = outcome.job.setups.iter().map(|s| s.name()).collect();
        assert_eq!(names, ["Top", "Flipped over"]);
        assert_eq!(outcome.job.setups[1].id, doc.setups[1].id);
    }
    #[tokio::test]
    async fn a_change_that_passes_gets_no_request_to_machine_more() {
        let (doc, mesh, report) = planned();
        // Setup 2 left out: its planes are still covered, which a run that
        // plans from zero would ask to machine.
        let mut smaller = unchanged(&doc, &report);
        smaller.setups.truncate(1);
        let script = Script::new(vec![json(&smaller), json(&unchanged(&doc, &report))]);
        let outcome = change(&script, &doc, &mesh, asking("Skip the underside."))
            .await
            .unwrap();
        assert_eq!((outcome.requests, outcome.chosen), (1, 1));
        assert_eq!(outcome.job.setups.len(), 1);
        assert_eq!(script.seen.lock().unwrap().len(), 1);
        // Without a request the run plans from zero: the same reply gets its round.
        let mut plain = good(&report);
        plain.setups.truncate(1);
        let script = Script::new(vec![json(&plain), json(&good(&report))]);
        let outcome = change(&script, &doc, &mesh, Chat::default()).await.unwrap();
        assert_eq!(outcome.requests, 2);
        assert_eq!(*script.edits.lock().unwrap(), [false, false]);
    }
    #[tokio::test]
    async fn a_change_that_never_passes_says_it_could_not_change_the_plan() {
        let (doc, mesh, report) = planned();
        let mut wrong = unchanged(&doc, &report);
        wrong.setups[0].operations[0].tool = 9;
        let replies = vec![json(&wrong); MAX_REQUESTS];
        let script = Script::new(replies);
        let failure = change(&script, &doc, &mesh, asking("Use a bigger cutter."))
            .await
            .unwrap_err();
        assert_eq!(
            failure.message,
            "The planner could not change the plan so that it passes every check in 4 attempts."
        );
        assert_eq!(
            (failure.requests, failure.api.clone()),
            (MAX_REQUESTS, None)
        );
        assert!(
            failure
                .problems
                .iter()
                .any(|p| p.message.contains("T9 is not in the tool library")),
            "{:?}",
            failure.problems
        );
        // Each correction asks again for the whole changed plan.
        assert_eq!(*script.edits.lock().unwrap(), [true; MAX_REQUESTS]);
        // Planning from zero keeps its own wording.
        let mut wrong = good(&report);
        wrong.setups[0].operations[0].tool = 9;
        let script = Script::new(vec![json(&wrong); MAX_REQUESTS]);
        let failure = change(&script, &doc, &mesh, Chat::default())
            .await
            .unwrap_err();
        assert_eq!(
            failure.message,
            "The planner could not produce a plan that passes every check in 4 attempts."
        );
    }
    #[tokio::test]
    async fn a_request_on_a_job_without_operations_plans_from_zero_with_the_request() {
        let (doc, mesh, report) = block();
        let doc = without_operations(doc);
        assert!(!editing(&doc, &asking("Hold it in the vise.")));
        let script = Script::new(vec![json(&good(&report)), json(&good(&report))]);
        let outcome = change(&script, &doc, &mesh, asking("Hold it in the vise."))
            .await
            .unwrap();
        assert_eq!(outcome.requests, 2, "a plan from zero still gets its round");
        let seen = script.seen.lock().unwrap();
        let roles: Vec<Role> = seen[0].iter().map(|(role, _)| *role).collect();
        assert_eq!(roles, [Role::System, Role::User, Role::User]);
        assert!(!seen[0][0].1.contains("CHANGING THE CURRENT PLAN"));
        assert_eq!(
            seen[0][2].1,
            "Plan the job, and follow this request of the user where the rules allow it: Hold it in the vise."
        );
        let all: String = seen.iter().flatten().map(|(_, t)| t.as_str()).collect();
        assert!(!all.contains("current_plan") && !all.contains("kept_operations"));
        assert_eq!(*script.edits.lock().unwrap(), [false, false]);
        let schemas = script.check_schemas.lock().unwrap();
        assert!(
            schemas
                .iter()
                .all(|s| s.as_ref().is_none_or(|s| *s == proposal::schema()))
        );
    }
    #[tokio::test]
    async fn a_plan_that_cannot_be_described_fails_before_anything_is_sent() {
        let (mut doc, mesh, _) = planned();
        doc.setups[1].p.insert("rx".into(), "45".into());
        let script = Script::new(vec![]);
        let failure = change(&script, &doc, &mesh, asking("Change it."))
            .await
            .unwrap_err();
        assert_eq!(
            failure.message,
            "Setup 2 is not turned square to the part, so the planner cannot change this plan."
        );
        assert_eq!((failure.requests, failure.api), (0, None));
        assert!(script.seen.lock().unwrap().is_empty());
    }
    #[test]
    fn check_plan_in_a_change_takes_drafts_with_from() {
        for pictures in [false, true] {
            let schema = |edit: bool| {
                planner_tools::offered_with(pictures, edit)
                    .into_iter()
                    .find(|t| t.name == "check_plan")
                    .unwrap()
                    .parameters["properties"]["proposal"]
                    .clone()
            };
            assert_eq!(schema(true), proposal::schema_for(true));
            assert_eq!(schema(false), proposal::schema());
            let names = |edit: bool| -> Vec<&'static str> {
                planner_tools::offered_with(pictures, edit)
                    .iter()
                    .map(|t| t.name)
                    .collect()
            };
            assert_eq!(names(true), names(false));
            assert_eq!(
                planner_tools::offered(pictures)
                    .iter()
                    .map(|t| t.name)
                    .collect::<Vec<_>>(),
                names(false)
            );
        }
    }
    #[tokio::test]
    async fn the_openai_backend_asks_for_the_schema_with_from_when_the_plan_is_changed() {
        use crate::fake_openai::{Fake, Resp};
        use crate::session::SessionStore;
        let fake = Fake::start(|_| Resp::answer("{}")).await;
        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(crate::session::FileStore(dir.path().join("s.json")));
        let session = crate::session::sample();
        store.save(&session).unwrap();
        let client = crate::openai::Client::new(
            crate::oauth::Endpoints::local(&fake.base),
            "oaiapp_1".into(),
            store,
            session,
        );
        let stream = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let backend = OpenAiBackend::new(
            std::sync::Arc::new(client),
            "gpt-6.1-sol".into(),
            stream,
            false,
        );
        let messages = vec![
            Item::text(Role::System, "Plan."),
            Item::text(Role::User, "Part."),
        ];
        for edit in [false, true] {
            backend
                .reply(&messages, &[], ToolChoice::Auto, edit)
                .await
                .unwrap();
        }
        let sent = fake.seen("/v1/responses");
        assert_eq!(sent.len(), 2);
        assert_eq!(
            sent[0].json()["text"]["format"]["schema"],
            proposal::schema()
        );
        assert_eq!(
            sent[1].json()["text"]["format"]["schema"],
            proposal::schema_for(true)
        );
    }
    #[test]
    fn applied_joins_setup_1s_planner_note_to_its_notes_and_replaces_the_last_one() {
        let (mut job, mesh, report) = planned();
        job.setups[0].p.insert("notes".into(), "Mine".into());
        let mut proposal = good(&report);
        proposal.setups[0].workholding = "Vise on the long sides.".into();
        let (check, built) = plan_check::check(&job, &mesh, &report, &proposal);
        let outcome = |proposal: Proposal, job: Document| Outcome {
            proposal,
            check: check.clone(),
            job,
            report: report.clone(),
            requests: 1,
            chosen: 1,
            more_warnings: 0,
            more_unmachined: 0,
            note: None,
        };
        let first = outcome(proposal.clone(), built.unwrap());
        let applied_once = applied(&first);
        let p = &applied_once.setups[0].p;
        assert_eq!(get(p, "notes"), "Mine · Planner: Vise on the long sides.");
        assert_eq!(get(p, PLANNER_NOTE_KEY), "Vise on the long sides.");
        // The outcome's own job is not touched; later setups are as planned.
        assert_eq!(get(&first.job.setups[0].p, "notes"), "Mine");
        assert!(!first.job.setups[0].p.contains_key(PLANNER_NOTE_KEY));
        assert_eq!(applied_once.setups[1].p, first.job.setups[1].p);
        // A change planned from the applied job: its note replaces the last.
        let mut next = proposal.clone();
        next.setups[0].workholding = "Clamp it instead.".into();
        let second = outcome(next.clone(), applied_once.clone());
        let p = &applied(&second).setups[0].p.clone();
        assert_eq!(get(p, "notes"), "Mine · Planner: Clamp it instead.");
        assert_eq!(get(p, PLANNER_NOTE_KEY), "Clamp it instead.");
        // Without a note the last one goes and the user's notes stay.
        next.setups[0].workholding = String::new();
        let third = outcome(next, applied_once);
        let p = &applied(&third).setups[0].p.clone();
        assert_eq!(get(p, "notes"), "Mine");
        assert!(!p.contains_key(PLANNER_NOTE_KEY));
    }
    #[test]
    fn a_rotary_machine_gets_the_axis_in_its_report_and_prompt_a_3_axis_one_nothing_new() {
        let (_, _, rotary) = crate::fake_openai::rotary::report();
        let unit = rotary.rotary.as_ref().unwrap();
        assert_eq!(
            (unit.centre_height, unit.max_diameter, unit.max_length),
            (70., 80., 250.)
        );
        assert_eq!((unit.a_min, unit.a_max), (-360., 360.));
        assert_eq!((unit.grip_min, unit.grip_max), (5., 80.));
        // The geometry is the machine's assumed one until the A centreline is probed.
        assert_eq!(unit.centreline, "assumed");
        let json = serde_json::to_value(&rotary).unwrap();
        assert_eq!(json["rotary"]["centreline"], "assumed");
        let text = instructions_for(true, true, true, true);
        for words in [
            "ROTARY A AXIS",
            "`centre_height`",
            "`max_diameter`",
            "`a_min`",
            "`grip_min`",
            "`assumed`",
            "`a_angles`",
            "`chuck`",
        ] {
            assert!(text.contains(words), "{words}");
        }
        // The same text and schema without the axis as before it existed.
        let (_, _, flat) = block();
        assert!(flat.rotary.is_none());
        assert!(!serde_json::to_string(&flat).unwrap().contains("rotary"));
        assert_eq!(proposal::schema_on(&flat), proposal::schema());
        assert_eq!(
            instructions_for(true, true, true, false),
            instructions_with(true, true, true)
        );
        let plain = instructions_with(true, true, true);
        assert!(!plain.contains("ROTARY") && !plain.contains("a_angles"));
        assert!(!proposal::schema().to_string().contains("a_angles"));
        let schema = proposal::schema_on(&rotary).to_string();
        assert!(schema.contains("a_angles") && schema.contains("\"chuck\""));
    }
    #[tokio::test]
    async fn a_square_bar_is_planned_as_a_chuck_and_four_indexed_angles_after_a_stock_too_big_for_z()
     {
        use crate::fake_openai::{Fake, rotary};
        use crate::session::SessionStore;
        let (doc, mesh, _) = rotary::report();
        let fake = Fake::start(rotary::replies(rotary::too_big(), rotary::good())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(crate::session::FileStore(dir.path().join("s.json")));
        let session = crate::session::sample();
        store.save(&session).unwrap();
        let client = crate::openai::Client::new(
            crate::oauth::Endpoints::local(&fake.base),
            "oaiapp_1".into(),
            store,
            session,
        );
        let backend = OpenAiBackend::new(
            std::sync::Arc::new(client),
            "gpt-6.1-sol".into(),
            std::sync::Arc::new(std::sync::Mutex::new(String::new())),
            false,
        );
        let mut events = vec![];
        let outcome = plan(&backend, doc, mesh, &Busy::default(), |e| events.push(e))
            .await
            .unwrap();
        // The oversized stock is refused with the check's own message ...
        let Some(Event::Checked(1, StepResult::Problems { shown, .. })) =
            events.iter().find(|e| matches!(e, Event::Checked(1, _)))
        else {
            panic!("{events:?}")
        };
        assert!(
            shown.iter().any(
                |l| l.starts_with("Setup 2, operation 1: Index moves turn A")
                    && l.contains("as far as the stock reaches from it at any angle")
                    && l.contains("1.11 mm above the top of the machine's 100 mm Z")
            ),
            "{shown:?}"
        );
        // ... which the model is sent, and the second plan passes. Its four
        // sides are faced, at the angle each is up, to a square joined to the
        // bar at a shoulder (LIO-214): all that stock still covers is the
        // part's end face on the bar, which the plan lists as gripped, so the
        // planner asks once to machine it, and the same plan, answered
        // again, stands.
        assert_eq!((outcome.requests, outcome.chosen), (3, 3), "{events:?}");
        assert!(events.contains(&Event::Stage(Progress::Extending(3))));
        let sent = fake.seen("/v1/responses");
        assert_eq!(sent.len(), 3);
        let first = sent[0].json();
        assert!(first.to_string().contains("ROTARY A AXIS"));
        let schema = first["text"]["format"]["schema"].to_string();
        assert!(
            schema.contains("a_angles") && schema.contains("\"chuck\""),
            "{schema}"
        );
        let covered = sent[2].json();
        let covered = covered["input"].as_array().unwrap().last().unwrap()["content"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            covered.contains(r#""covered_planes":[{"orientation":"-X""#)
                && covered.contains(r#""covered_other":[]"#)
                && covered.matches(r#""orientation""#).count() == 1,
            "{covered}"
        );
        let again = sent[1].json().to_string();
        assert!(
            again.contains("above the top of the machine's 100 mm Z travel"),
            "{again}"
        );
        // The job is the chuck and the one indexed setup, as the setups page makes them.
        let job = &outcome.job;
        assert_eq!(job.setups.len(), 2);
        let (first, second) = (&job.setups[0], &job.setups[1]);
        assert_eq!(first.p["holding"], "Chuck");
        assert_eq!(first.p["stock_type"], "Fixed size box");
        assert_eq!(first.p["origin"], crate::indexed::ORIGINS[0]);
        assert!(crate::indexed::is_indexed(second));
        // Its one operation faces the square's four sides, each where it is
        // on top, with the tool's centre out to the face's edges.
        let face = &second.operations[0];
        assert_eq!(face.strategy, crate::model::Strategy::Face);
        assert_eq!(face.surfaces.as_ref().unwrap().ids.len(), 4);
        assert_eq!(face.p["containment"], "Tool center on boundary");
        assert_eq!(
            crate::indexed::angles(second).unwrap(),
            [0., 90., 180., 270.]
        );
        assert_eq!(second.p["offset"], first.p["offset"]);
        assert_eq!(
            outcome.report.rotary.as_ref().unwrap().centreline,
            "assumed"
        );
    }
}
