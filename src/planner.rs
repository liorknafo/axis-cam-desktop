//! The one-button planner: asks the model for a proposal, checks it with the
//! real generators and export checks, and asks for repairs. Nothing from the
//! model is used until it has passed every check.
use crate::{
    analysis::{self, PartReport},
    mesh::Mesh,
    model::Document,
    openai::{ApiError, Role},
    plan_check::{self, CheckReport},
    proposal::{self, Problem, Proposal},
};
use std::future::Future;

/// Model requests per run: one proposal and up to three corrections.
pub const MAX_REQUESTS: usize = 4;

/// Anything that can answer a conversation with text: the OpenAI client, or a
/// script in tests.
pub trait Backend {
    fn reply(
        &self,
        messages: &[(Role, String)],
    ) -> impl Future<Output = Result<String, ApiError>> + Send;
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
/// What the model is told before it sees the part. Every limit the builder
/// and the generators enforce is stated, so a correct plan passes first time.
pub fn instructions() -> String {
    let (per_setup, total) = (proposal::MAX_SETUP_OPERATIONS, proposal::MAX_OPERATIONS);
    let samples = format!(
        "{},{:03}",
        crate::cam::LIMIT / 1000,
        crate::cam::LIMIT % 1000
    );
    format!("You plan CNC milling jobs for AXIS CAM Studio, a 3-axis CAM program. You receive a JSON part report and reply with one JSON proposal that matches the given schema. The program then generates every operation, simulates the stock, runs its export safety checks and tells you what failed, so you can correct the proposal.

THE PART REPORT
- Lengths are millimetres. `tools` lists the only tools you may use, by `number`, with their `diameter`, `flutes`, flute length and stickout. `machine` gives the maximum rpm and feed. `material` is the project's material with its cutting data (see SPEEDS AND FEEDS).
- `orientations` lists six ways to clamp the part, named by the part axis that points up. The first entry is setup 1, where the stock is defined; your first setup must use it. Each orientation may be used by one setup only, so a plan has at most 6 setups. Add another setup only for features that cannot be reached otherwise.
- In each orientation, `planes` are the horizontal faces visible from above, with their `surfaces` IDs. `kind` is `top` for the highest face and `floor` for lower faces. `depth_below_top` is measured from the model's top in that orientation. `stock_margin` is the untouched stock beyond the model: top, bottom, -X, +X, -Y, +Y. Faces that an earlier setup machined have no stock left on them.
- A floor's `max_cutter_diameter` is the largest flat cutter that fits inside it. A plane's `holes` give the largest drill that fits (`diameter`), the opening diameter (`opening_diameter`, larger at a counterbore), the depth and whether the hole goes through.
- `freeform_area` is the visible area that is neither horizontal nor vertical; only Parallel finishing follows it.
- `truncated` true means the orientation has more planes or holes than are listed; only listed surfaces can be used.

STRATEGIES (the only six)
- Face: flattens the top. `surfaces` = the top plane's IDs. Tool: flat end mill or face mill. `stepdown` at most the flute length, `stepover_percent` 1-100. Facing needs stock above the face: use it only where the orientation's `stock_margin[0]` is greater than `bottom_offset`. `bottom_offset` above 0 leaves that much stock on the face. `facing_area` `stock` (or null) faces the whole stock top, `model` only the model's outline. Face makes levels × passes strokes, at most 2,000: levels = (`stock_margin[0]` − `bottom_offset`) ÷ `stepdown`, rounded up; passes = (the faced width in Y + the tool diameter) ÷ the stepover in mm, rounded up, plus 1. The faced width is the stock's (the model's Y size + `stock_margin[4]` + `stock_margin[5]`), or the model's Y size with `facing_area` `model`. For example, 4 mm of stock above the face of an 84 mm wide stock and a Ø10 mill at `stepover_percent` 5 need a `stepdown` of 0.4 mm or more.
- Pocket: clears from the stock top down to one floor. `surfaces` = all IDs of that floor plane. Tool: a flat end mill no wider than the floor's `max_cutter_diameter`. `stepover_percent` at most 50, `stepdown` at most the flute length. The floor's depth plus the stock above the top must be less than the tool's stickout. `radial_allowance` and `axial_allowance` leave stock. `pocket_entry` is `Profile ramp` or `Plunge`; `ramp_angle` null or 0.1-15 degrees, also with `Plunge`. `finish_passes` a whole number 0-20. Its moves are about levels × the points of all its offset rings, and three settings multiply them: a smaller `stepover_percent` adds rings, each finish pass repeats the outermost ring at every level, and a smaller `ramp_angle` adds ramp moves on every ring and level (see the move limit under RULES).
- Contour, `contour_mode` `outside`: one path around the whole model outline, from the stock top down to the model's bottom plus `bottom_offset`. `surfaces` empty. Flat end mill, `stepdown` at most the flute length, `finish_passes` a whole number 0-20. The cut depth from the stock top must be less than the stickout; for a tall part use `bottom_offset` to stop higher and finish from the opposite side. It removes one tool width around the outline, not all surrounding stock. Its moves are about levels × the outline's points (see the move limit under RULES).
- Contour, `contour_mode` `inside_floor`: the walls of one floor. `surfaces` = that floor's IDs. The tool and depth rules of Pocket apply: a flat end mill no wider than the floor's `max_cutter_diameter`, depth within the stickout, `stepdown` at most the flute length, `finish_passes` a whole number 0-20; `pocket_entry` and `ramp_angle` work as for Pocket, but its stepover is fixed at 40 %, so `stepover_percent` changes nothing. Its moves are about levels × the points of its rings, as for Pocket: each finish pass repeats the outermost ring at every level, and a smaller `ramp_angle` adds ramp moves on every ring and level (see the move limit under RULES).
- Drill: drills the holes of one plane. `surfaces` = that plane's IDs. Tool: a drill no wider than the hole's `diameter`. `hole_diameter` limits it to holes whose `diameter` equals it (not `opening_diameter`); null drills every hole of the plane. The drill's depth is measured from the stock top: `stock_margin[0]` + the plane's `depth_below_top` + the hole's `depth` + `breakthrough` must be less than the drill's flute length. `drill_cycle` is `Drilling`, `Chip breaking`, `Deep drilling` (both need `peck`) or `Counterboring`. `peck` must be at least the drilling depth from the stock top ÷ 1,000: at most 1,000 pecks per hole.
- Drill, through holes: `through` may be true only in setup 1, and only when every hole the operation selects is a through hole in the report; use `hole_diameter` to select only those. Setups after the first rest on the face where a drill would exit, so drill through holes in setup 1, or set `through` false and list the exit in `unmachined`. With `through` true, `breakthrough` extends the drill below the hole; null means 1 mm. It is at most half the drill diameter plus 1 mm and never more than the stock below the model (`stock_margin[1]`).
- Parallel: surface-following finishing of the whole model from above. `surfaces` empty. Ball or flat end mill. `surface_stepover` at most the tool radius, `tolerance` 0.01-1. The pass count times the samples per pass must stay under {samples}: (the model's Y size ÷ `surface_stepover`) × (the model's X size ÷ step), where step = min(`surface_stepover`, √(8 × tool radius × `tolerance`), 1 mm) and must be at least 0.05 mm. On a 120 × 80 mm part with a Ø6 ball end mill this means a `surface_stepover` of about 0.6 mm or more with `tolerance` 0.02.
- Waterline: constant-height passes around the model, always from the model's top down to its bottom plus `bottom_offset`. `surfaces` empty. Flat end mill, `stepdown` at most the flute length. That height must be less than the tool's stickout. It cuts that height ÷ `stepdown` layers, at most 300, but its moves (about layers × the outline's points) usually reach the move limit first: on a part of about 120 × 80 mm keep `stepdown` at or above about 0.5 mm (see RULES).

SPEEDS AND FEEDS
- `material` in the report holds the cutting data for carbide tools: `vc_min_m_per_min` and `vc_max_m_per_min` give the surface speed range in m/min, and `chip_load_mm` the chip load per tooth in mm in five bands by tool diameter. A band holds the diameters up to its `up_to_mm` (and above the previous band's), with its `min` and `max`; the last band, with `up_to_mm` null, holds every larger diameter.
- The program computes, for every operation, the surface speed Vc = π × D × rpm ÷ 1000 (m/min) and the chip load fz = feed ÷ (rpm × flutes) (mm per tooth), from the tool's diameter D and `flutes` in `tools` and the operation's `rpm` and `feed`. Vc must lie between `vc_min_m_per_min` and `vc_max_m_per_min`, and fz between the `min` and `max` of the band of the tool's diameter. Choose a Vc in range and set rpm = Vc × 1000 ÷ (π × D), then choose an fz in the band and set feed = fz × rpm × flutes. For example, a 3-flute Ø10 mm tool at 8000 rpm and 1000 mm/min runs at Vc = 251 m/min with fz = 0.042 mm per tooth.
- Drills are checked like end mills: a Drill's `feed` is divided by rpm × flutes in the same way and must give a chip load inside the band of the drill's diameter, so choose drill feeds inside those bands. A small drill needs a high rpm to reach the surface speed.
- When even the machine's maximum rpm (`machine.max_rpm`) gives a tool less than the minimum surface speed, running at that maximum rpm counts as in range. When the minimum chip load needs more than the machine's maximum feed (`machine.max_feed`), running at that maximum feed counts as in range. A value above the material's maximum is never excused, and rounding gets 1% of slack on each limit.
- Plunge and ramp feeds are not checked. Plans with a spindle speed or feed outside the material's cutting data are refused: each such operation comes back as a problem with the stage `cutting data`, together with the generation, export, tool-axis and gouge problems.

RULES
- Use only tool numbers from `tools` and surface IDs from the chosen orientation's `planes`. Never invent either.
- Every setup needs at least one operation. At most {per_setup} operations per setup and {total} in total. Spindle speed is at least 100 rpm and feed at least 1 mm/min, both within the machine's maximums.
- Every number must be finite.
- Every operation is limited to {samples} moves (Parallel to {samples} samples, as above), and a finer stepdown or stepover multiplies them: Waterline makes about layers × the outline's points, Pocket and an inside-floor Contour about levels × the points of their rings (more with each finish pass and a smaller `ramp_angle`), an outside Contour about levels × the outline's points. On a part of about 120 × 80 mm, with Face first, keep `stepdown` at or above about 0.5 mm for Pocket, Contour and Waterline; a deeper cut needs a larger one. The 0.5 mm guide assumes `stepover_percent` 40, 1 finish pass and a `ramp_angle` of 1° or more; curved walls and round islands add hundreds of moves per level. With more finish passes, a smaller stepover or curved walls, use a larger `stepdown`, and keep `ramp_angle` at 1° or more.
- The program refuses any move whose tool centre line passes through the part, and any move below the bottom of the stock. In setups after the first, nothing may go below the part's underside.
- `bottom_offset`, `radial_allowance` and `axial_allowance` cannot be negative. Only a through drill's breakthrough in setup 1 goes below the model, into the stock beneath it.
- Order operations in a setup: Face, then Pocket, then Contour, then Drill, then finishing.
- No tool tip may go deeper below the stock top than the tool's stickout.
- Choose each operation's spindle speed and feed from the material's cutting data (see SPEEDS AND FEEDS), within the machine's limits.
- Set the fields a strategy does not use to null. A null field takes the program's default: `stepdown` 2, `stepover_percent` 40, `surface_stepover` 1, `tolerance` 0.01, `peck` 3, `finish_passes` 1, `ramp_angle` 2, `breakthrough` 1, allowances and `bottom_offset` 0.
- For later setups choose the work zero: `Model origin` is accepted; the program tells the operator where it lies relative to the part, and to the stock top while no earlier setup has cut it. Prefer a stock-based zero when the stock it is touched off on is untouched. `Stock top center`: touch off Z0 on the stock top at its centre and X0 Y0 from the four sides near their middles, between 1 mm and 10 mm below the top. `Stock top front-left`: touch off Z0 on the stock top within 10 mm of the front-left corner, at least 1 mm in from both edges, and X0 Y0 from the -X and -Y sides between 5% and 15% of each side's length from the front-left corner, between 1 mm and 10 mm below the top. On stock under 40 mm tall the sides are touched within a quarter of its height below the top, and under 40 mm across the corner area is a quarter of the shorter top side. A stock-based zero is refused when earlier setups have cut the stock at one of those spots.
- Give each setup a short `workholding` note. Clamps are not modelled, so say what must be kept clear.
- List every feature you cannot machine with these tools and strategies in `unmachined`, with the reason. Do not leave it out silently, and do not add an operation that cannot work.
- Put risks the operator should know about in `warnings`.
- In the check report, `covered_other` lists the remaining surface by `facing` (+X, -X, +Y, -Y, top, bottom), named in setup 1's frame, which can differ from the orientation names when setup 1 is rotated.
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
fn name(text: &str) -> String {
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
fn tidy(mut p: Proposal) -> (Proposal, usize, usize) {
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
fn json_object(text: &str) -> &str {
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
/// Runs the planner. `progress` is called as each stage starts and with each
/// check's result. The job and mesh are taken by value so the returned
/// future can move between threads.
pub async fn plan(
    backend: &(impl Backend + Sync),
    doc: Document,
    mesh: Mesh,
    busy: &Busy,
    progress: impl FnMut(Event) + Send,
) -> Result<Outcome, Failure> {
    plan_with(
        backend,
        doc,
        mesh,
        busy,
        progress,
        analysis::report,
        plan_check::check,
    )
    .await
}
/// `plan` with its two blocking stages given: the part report (`analyze`)
/// and the plan check (`check`). Tests use it to hold a stage while it runs,
/// or to make a check fail to run.
async fn plan_with<A, C>(
    backend: &(impl Backend + Sync),
    doc: Document,
    mesh: Mesh,
    busy: &Busy,
    mut progress: impl FnMut(Event) + Send,
    analyze: A,
    check: C,
) -> Result<Outcome, Failure>
where
    A: FnOnce(&Document, &Mesh) -> Result<PartReport, String> + Send + 'static,
    C: Fn(&Document, &Mesh, &PartReport, &Proposal) -> (CheckReport, Option<Document>)
        + Clone
        + Send
        + 'static,
{
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
    let part = serde_json::to_string(&report).map_err(|e| fail(e.to_string(), vec![], None, 0))?;
    let mut messages = vec![(Role::System, instructions()), (Role::User, part)];
    let mut best: Option<Outcome> = None;
    let mut problems = vec![];
    let mut asked_for_coverage = false;
    let mut made = 0;
    let mut note = None;
    let mut stage: fn(usize) -> Progress = Progress::Asking;
    for request in 1..=MAX_REQUESTS {
        progress(Event::Stage(stage(request)));
        made = request;
        let text = match backend.reply(&messages).await {
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
        let parsed = proposal::parse(json_object(&text));
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
                messages.push((Role::Assistant, text));
                messages.push((Role::User, format!("{error} Reply with the full proposal as one JSON object that matches the schema.")));
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
        let (check, job) = match checked {
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
        progress(Event::Checked(request, StepResult::of(&check)));
        let next = feedback(&check);
        if let (true, Some(mut job)) = (check.passed, job) {
            // The job is the user's again: Apply keeps setup 1's real name.
            job.title = real.title.clone();
            if let (Some(first), Some(user)) = (job.setups.first_mut(), real.setups.first()) {
                first.p.insert("name".into(), user.name().into());
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
            // One round to machine what is still covered; then the best plan stands.
            if !covered || asked_for_coverage {
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
        messages.push((Role::Assistant, text));
        messages.push((Role::User, next));
    }
    match best {
        Some(outcome) => Ok(Outcome {
            requests: made,
            note,
            ..outcome
        }),
        None => Err(fail(
            format!(
                "The planner could not produce a plan that passes every check in {MAX_REQUESTS} attempts."
            ),
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
    /// Characters of the current reply streamed so far, for the page.
    received: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
impl OpenAiBackend {
    pub fn new(
        client: std::sync::Arc<crate::openai::Client>,
        model: String,
        received: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> Self {
        Self {
            client,
            model,
            plain: false.into(),
            received,
        }
    }
}
impl Backend for OpenAiBackend {
    async fn reply(&self, messages: &[(Role, String)]) -> Result<String, ApiError> {
        use std::sync::atomic::Ordering;
        let request = crate::openai::Request {
            model: self.model.clone(),
            messages: messages.to_vec(),
            schema_name: "machining_plan",
            schema: proposal::schema(),
        };
        if self.plain.load(Ordering::Relaxed) {
            let reply = self.client.respond_plain_counting(&request, &self.received);
            return reply.await.map(|r| r.text);
        }
        let reply = self
            .client
            .respond_counting(&request, &self.received)
            .await?;
        if !reply.structured {
            self.plain.store(true, Ordering::Relaxed);
        }
        Ok(reply.text)
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
    let mut sim = crate::simulation::prepare(job, mesh, si, oi).ok()?;
    sim.seek(1.);
    let volume = sim.volume.as_ref()?;
    let surface = crate::stock_surface::Snapshot {
        bounds: volume.bounds,
        frame: volume.frame,
        cuts: volume.cuts.clone(),
        detail,
    }
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
    use crate::proposal::tests::{block, good};
    use std::sync::Mutex;
    /// Replies in order and records every conversation it was sent.
    pub struct Script {
        replies: Mutex<std::collections::VecDeque<Result<String, ApiError>>>,
        pub seen: Mutex<Vec<Vec<(Role, String)>>>,
    }
    impl Script {
        pub fn new(replies: Vec<Result<String, ApiError>>) -> Self {
            Self {
                replies: Mutex::new(replies.into()),
                seen: Mutex::new(vec![]),
            }
        }
    }
    impl Backend for Script {
        async fn reply(&self, messages: &[(Role, String)]) -> Result<String, ApiError> {
            self.seen.lock().unwrap().push(messages.to_vec());
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Err(ApiError::Other("script exhausted".into())))
        }
    }
    fn json(proposal: &Proposal) -> Result<String, ApiError> {
        Ok(serde_json::to_string(proposal).unwrap())
    }
    /// The stages of a run's events, in order.
    fn stages(events: &[Event]) -> Vec<Progress> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Stage(stage) => Some(*stage),
                Event::Checked(..) => None,
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
        let path = doc.file.as_ref().unwrap().path.clone();
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
            file.path.as_str(),
            ".ngc",
            &mesh.fingerprint,
        ] {
            assert!(!all.contains(private), "{private}");
        }
    }
    #[test]
    fn the_instructions_state_the_limits_the_program_enforces() {
        let text = instructions();
        let limit = crate::cam::LIMIT;
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
            "measured from the stock top".into(),
            "`through` may be true only in setup 1".into(),
            "null means 1 mm".into(),
            "half the drill diameter plus 1 mm".into(),
            "`stock_margin[1]`".into(),
            format!("{},{:03}", limit / 1000, limit % 1000),
            "√(8 × tool radius × `tolerance`)".into(),
            "at least 0.05 mm".into(),
            "layers, at most 300".into(),
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
    fn the_parallel_example_in_the_instructions_holds_on_the_fixture() {
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
        assert!(instructions().contains("On a 120 × 80 mm part with a Ø6 ball end mill"));
        let size = &report.orientations[0].model_size;
        assert_eq!((size[0].round(), size[1].round()), (120., 80.));
        assert!(generate(0.6).is_ok());
        assert!(generate(0.5).unwrap_err().contains("30,000"));
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
    fn the_move_limits_in_the_instructions_hold_on_the_fixture() {
        let fixture = block();
        let size = &fixture.2.orientations[0].model_size;
        assert_eq!((size[0].round(), size[1].round()), (120., 80.));
        let text = instructions();
        assert!(text.contains("Every operation is limited to 30,000 moves"));
        assert!(text.contains(
            "On a part of about 120 × 80 mm, with Face first, keep `stepdown` at or above about 0.5 mm for Pocket, Contour and Waterline"
        ));
        // Pocket, outside Contour and Waterline at 0.5 mm pass in the order the
        // instructions give; finer stepdowns that meet every other rule fail.
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
        for (strategy, passes, fails) in [
            ("Pocket", 0.5, 0.45),
            ("Contour", 0.35, 0.3),
            ("Waterline", 0.45, 0.4),
        ] {
            assert_eq!(stepdown(strategy, 0.5), Ok(()), "{strategy} at 0.5 mm");
            assert_eq!(stepdown(strategy, passes), Ok(()), "{strategy} {passes}");
            let error = stepdown(strategy, fails).unwrap_err();
            assert!(
                error.contains("30,000 moves"),
                "{strategy} {fails}: {error}"
            );
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
                l.contains("a smaller `ramp_angle` adds ramp moves on every ring and level"),
                "{l}"
            );
        }
        assert!(pocket.contains("a smaller `stepover_percent` adds rings"));
        assert!(
            inside.contains("its stepover is fixed at 40 %, so `stepover_percent` changes nothing")
        );
        assert!(text.contains(
            "The 0.5 mm guide assumes `stepover_percent` 40, 1 finish pass and a `ramp_angle` of 1° or more; curved walls and round islands add hundreds of moves per level"
        ));
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
        for inside_floor in [false, true] {
            let name = if inside_floor {
                "inside floor"
            } else {
                "Pocket"
            };
            assert_eq!(at_half(inside_floor, &|_| {}), Ok(()), "{name}");
            assert_eq!(
                at_half(inside_floor, &|o| o.finish_passes = Some(1)),
                Ok(()),
                "{name}, 1 finish pass"
            );
            assert_eq!(
                at_half(inside_floor, &|o| o.ramp_angle = Some(1.)),
                Ok(()),
                "{name}, ramp 1°"
            );
            for (what, edit) in [
                (
                    "2 finish passes",
                    &(|o: &mut proposal::ProposedOp| o.finish_passes = Some(2))
                        as &dyn Fn(&mut proposal::ProposedOp),
                ),
                ("ramp 0.3°", &|o| o.ramp_angle = Some(0.3)),
            ] {
                let Err(error) = at_half(inside_floor, edit) else {
                    panic!("{name}, {what} passed");
                };
                assert!(error.contains("30,000 moves"), "{name}, {what}: {error}");
            }
        }
        // A smaller stepover adds Pocket rings; an inside-floor Contour keeps 40 %.
        let error = at_half(false, &|o| o.stepover_percent = Some(30.)).unwrap_err();
        assert!(error.contains("30,000 moves"), "{error}");
        assert_eq!(
            at_half(true, &|o| o.stepover_percent = Some(10.)),
            Ok(()),
            "stepover changes nothing for an inside-floor Contour"
        );
    }
    #[test]
    fn the_facing_peck_and_setup_limits_in_the_instructions_hold_on_the_fixture() {
        let fixture = block();
        let report = &fixture.2;
        let text = instructions();
        // Face: levels × passes strokes, at most 2,000.
        assert!(text.contains("Face makes levels × passes strokes, at most 2,000"));
        assert!(text.contains(
            "4 mm of stock above the face of an 84 mm wide stock and a Ø10 mill at `stepover_percent` 5 need a `stepdown` of 0.4 mm or more"
        ));
        let o = &report.orientations[0];
        let width = o.model_size[1] + o.stock_margin[4] + o.stock_margin[5];
        assert_eq!((o.stock_margin[0], width.round()), (4., 84.));
        let strokes = |stepdown: f32| {
            let levels = (o.stock_margin[0] / stepdown).ceil();
            let passes = ((width + 10.) / 0.5).ceil() + 1.;
            levels * passes
        };
        for (stepdown, ok) in [(0.4, true), (0.38, false)] {
            assert_eq!(strokes(stepdown) <= 2000., ok, "the formula at {stepdown}");
            let result = generated_in_setup_1(&fixture, |p| {
                let face = &mut p.setups[0].operations[0];
                face.stepdown = Some(stepdown);
                face.stepover_percent = Some(5.);
                0
            });
            assert_eq!(result.is_ok(), ok, "Face at {stepdown}: {result:?}");
            if !ok {
                assert!(result.unwrap_err().contains("2,000 facing strokes"));
            }
        }
        // Drill: `peck` at least the drilling depth from the stock top ÷ 1,000.
        assert!(
            text.contains("`peck` must be at least the drilling depth from the stock top ÷ 1,000")
        );
        let top = o.planes.iter().find(|p| p.kind == "top").unwrap();
        let deepest = top.holes.iter().map(|h| h.depth).fold(0., f32::max);
        let depth = o.stock_margin[0] + top.depth_below_top + deepest;
        assert_eq!(depth.round(), 44.);
        for (factor, ok) in [(1.02, true), (0.98, false)] {
            // The drill runs first, so it starts at the stock top.
            let result = generated_in_setup_1(&fixture, |p| {
                let ops = &mut p.setups[0].operations;
                let mut drill = ops.remove(3);
                drill.peck = Some(depth / 1000. * factor);
                ops.insert(0, drill);
                0
            });
            assert_eq!(result.is_ok(), ok, "peck × {factor}: {result:?}");
            if !ok {
                assert!(result.unwrap_err().contains("1,000 steps per hole"));
            }
        }
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
    fn the_face_formula_and_the_waterline_layer_limit_hold_exactly() {
        let fixture = block();
        let o = &fixture.2.orientations[0];
        // Face strokes as the instructions state them, for a faced width in
        // Y, a Ø10 mill and a stepover in percent.
        let strokes = |width: f32, percent: f32, stepdown: f32, plus_one: f32| {
            let levels = (o.stock_margin[0] / stepdown).ceil();
            let passes = ((width + 10.) / (10. * percent / 100.)).ceil() + plus_one;
            levels * passes
        };
        let face = |area: &'static str, percent: f32, stepdown: f32| {
            generated_in_setup_1(&fixture, |p| {
                let face = &mut p.setups[0].operations[0];
                face.stepdown = Some(stepdown);
                face.stepover_percent = Some(percent);
                face.facing_area = Some(area.into());
                0
            })
        };
        let stock = o.model_size[1] + o.stock_margin[4] + o.stock_margin[5];
        let model = o.model_size[1];
        assert_eq!((stock.round(), model.round()), (84., 80.));
        for (area, width, percent, stepdown, ok) in [
            // The "+1" decides this pair: 26 levels × 76 rows is 1,976, but
            // 26 × (76 + 1) is 2,002.
            ("stock", stock, 12.5, 0.155, false),
            ("stock", stock, 12.5, 0.165, true),
            // `facing_area` `model` faces the model's Y size, not the stock's.
            ("model", model, 5., 0.38, true),
            ("model", model, 5., 0.36, false),
            ("stock", stock, 5., 0.38, false),
        ] {
            let label = format!("Face {area} {percent}% at {stepdown}");
            assert_eq!(
                strokes(width, percent, stepdown, 1.) <= 2000.,
                ok,
                "{label}"
            );
            let result = face(area, percent, stepdown);
            assert_eq!(result.is_ok(), ok, "{label}: {result:?}");
            if !ok {
                assert!(result.unwrap_err().contains("2,000 facing strokes"));
            }
        }
        assert!(
            strokes(stock, 12.5, 0.155, 0.) <= 2000.,
            "without the +1 the formula would allow 0.155"
        );
        // Waterline: 300 layers generate and 301 are refused, on a 20 mm high
        // box whose outline is short enough that the move limit does not
        // bind first.
        let (doc, mesh, report) = open_box();
        let waterline = |stepdown: f32| {
            let mut proposal = complete(&report);
            let mut op = proposal::tests::op("Waterline", 1, vec![]);
            op.stepdown = Some(stepdown);
            proposal.setups[0].operations.push(op);
            let job = proposal::build(&doc, &mesh, &report, &proposal).unwrap();
            crate::simulation::prepare(&job, &mesh, 0, 2)
                .map(|s| (s.plan.levels, s.plan.moves.len()))
        };
        assert_eq!(report.orientations[0].model_size[2], 20.);
        let (levels, moves) = waterline(20. / 299.8).unwrap();
        assert_eq!(levels, 300);
        // About 59 moves a layer: 301 layers would still be far below 30,000.
        assert!(moves < crate::cam::LIMIT * 2 / 3, "{moves} moves");
        let error = waterline(20. / 300.5).unwrap_err();
        assert!(error.contains("exceeds 300 layers"), "{error}");
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
                    zeros: vec![],
                };
                (report, None)
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
                    zeros: vec![],
                };
                (report, None)
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
    ) -> (CheckReport, Option<Document>) {
        assert_ne!(proposal.summary, "boom", "the check fails to run");
        plan_check::check(doc, mesh, report, proposal)
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
            setups: vec![proposal::ProposedSetup {
                orientation: report.orientations[0].up.into(),
                name: "Top".into(),
                origin: "Stock top center".into(),
                workholding: "Vise.".into(),
                operations: vec![
                    proposal::tests::op("Face", 1, top),
                    proposal::tests::op("Contour", 1, vec![]),
                ],
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
        let received = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let backend = OpenAiBackend::new(
            std::sync::Arc::new(client),
            "gpt-6.1-sol".into(),
            received.clone(),
        );
        let messages = vec![
            (Role::System, "Plan.".to_string()),
            (Role::User, "Part.".into()),
            (Role::Assistant, "{}".into()),
            (Role::User, "Fix it.".into()),
        ];
        let count = || received.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(backend.reply(&messages).await.unwrap(), "{}");
        assert_eq!(count(), 2, "the structured refusal streamed nothing");
        received.store(40, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(backend.reply(&messages).await.unwrap(), "{}");
        assert_eq!(count(), 2, "the plain request starts the count again");
        let sent = fake.seen("/v1/responses");
        assert_eq!(
            sent.len(),
            3,
            "the second reply skips the structured attempt"
        );
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
                    zeros: vec![],
                };
                (nothing, None)
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
            plan_check::check,
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
}
