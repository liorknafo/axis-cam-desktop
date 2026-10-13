//! LinuxCNC postprocessor for the simulated 3-axis moves. A program is written
//! only after `post_check` accepts its motion and `ngc::verify` replays the
//! text back to the same moves.
use crate::{
    mesh::{Bounds, Mesh},
    model::*,
    simulation::{Motion, Plan, Simulation},
    stock::Placement,
};

pub const OFFSETS: [&str; 9] = [
    "G54", "G55", "G56", "G57", "G58", "G59", "G59.1", "G59.2", "G59.3",
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PostSettings {
    pub tool_table: bool,
    pub machine_home: bool,
    pub blend: f32,
    pub dwell: f32,
    /// (mist, flood) outputs used for Air; None when the machine has none.
    pub air: Option<(bool, bool)>,
}
impl PostSettings {
    pub fn from_machine(m: &Machine) -> Result<Self, Vec<String>> {
        let mut e = vec![];
        match get(&m.p, "controller") {
            "LinuxCNC" => {}
            "UC100" => e.push(format!(
                "G-code export is LinuxCNC only. {} uses the UC100: run this setup on it from the MACHINE workspace.",
                m.name()
            )),
            other => e.push(format!(
                "{} uses the {other} controller. Only the LinuxCNC postprocessor is implemented.",
                m.name()
            )),
        }
        if m.axes() != 3 || get(&m.p, "mode") != "3-axis" {
            e.push(format!(
                "{} is not a 3-axis machine. Only 3-axis output is implemented.",
                m.name()
            ));
        }
        e.extend(m.issues());
        e.extend(post_setting_issues(&m.p));
        if !e.is_empty() {
            return Err(e);
        }
        Ok(Self::read(m))
    }
    /// What running a setup on a UC100 machine uses: touch-off mode (one
    /// program per tool run, the tool changed by hand in between), retracts
    /// to the setup clearance plane, which the checks prove clear of the
    /// stock and the jaws, and the machine's spindle start dwell and Air
    /// output. The driver blends corners with the profile's own setting.
    pub fn for_uc100(m: &Machine) -> Result<Self, Vec<String>> {
        let mut e = vec![];
        if get(&m.p, "controller") != "UC100" {
            e.push(format!(
                "{} uses the {} controller, not the UC100.",
                m.name(),
                get(&m.p, "controller")
            ));
        }
        if m.axes() != 3 || get(&m.p, "mode") != "3-axis" {
            e.push(format!(
                "{} is not a 3-axis machine. Only 3-axis runs are implemented.",
                m.name()
            ));
        }
        e.extend(m.issues());
        e.extend(post_setting_issues(&m.p));
        if !e.is_empty() {
            return Err(e);
        }
        Ok(Self {
            tool_table: false,
            machine_home: false,
            blend: 0.,
            ..Self::read(m)
        })
    }
    /// What the planner checks a machine's setups with: a UC100 run's
    /// settings for a UC100 machine, LinuxCNC export's for any other.
    pub fn for_machine(m: &Machine) -> Result<Self, Vec<String>> {
        if get(&m.p, "controller") == "UC100" {
            Self::for_uc100(m)
        } else {
            Self::from_machine(m)
        }
    }
    fn read(m: &Machine) -> Self {
        let value = |k: &str| post_setting(&m.p, k).trim().parse::<f32>().unwrap_or(0.);
        Self {
            tool_table: post_setting(&m.p, "post_tool_length") == POST_TOOL_LENGTH[0],
            machine_home: post_setting(&m.p, "post_retract") == POST_RETRACT[0],
            blend: value("post_blend"),
            dwell: value("post_dwell"),
            air: match post_setting(&m.p, "post_air") {
                "M7" => Some((true, false)),
                "M8" => Some((false, true)),
                _ => None,
            },
        }
    }
}
/// Program zero in the setup's plan coordinates (mm).
pub fn datum(setup: &Setup, stock: Bounds, mesh: &Mesh) -> Result<[f32; 3], String> {
    match get(&setup.p, "origin") {
        "Stock top center" => Ok([
            (stock.min[0] + stock.max[0]) / 2.,
            (stock.min[1] + stock.max[1]) / 2.,
            stock.max[2],
        ]),
        "Stock top front-left" => Ok([stock.min[0], stock.min[1], stock.max[2]]),
        "Model origin" => Ok(Placement::new(mesh, Some(setup))
            .transform(mesh.origin)
            .map(|v| v / mesh.display_scale())),
        "Selected point (planned)" => Err("Selected-point work zeros are not implemented. Choose Stock top center, Stock top front-left or Model origin.".into()),
        other => Err(format!("Unsupported work origin \"{other}\".")),
    }
}
/// The part's box in a setup's plan coordinates (mm).
fn part_box(setup: &Setup, mesh: &Mesh) -> Bounds {
    Placement::new(mesh, Some(setup))
        .bounds
        .scaled(1. / mesh.display_scale())
}
/// What locates a work zero the operator cannot touch directly (Model
/// origin): the part's box and the stock top, in work coordinates (mm).
fn zero_place(part: Bounds, stock: Bounds, datum: [f32; 3]) -> (Bounds, f32) {
    (part.translated(datum.map(|v| -v)), stock.max[2] - datum[2])
}
/// `v` mm to 0.1 mm, without a trailing ".0".
fn short_mm(v: f32) -> String {
    let s = format!("{v:.1}");
    match s.strip_suffix(".0") {
        Some(whole) => whole.to_string(),
        None => s,
    }
}
/// Where the operator touches off a stock-based work zero (the spots
/// `stock_zero_cut` checks), said after "Touch off" in the program header
/// and in touch-off program 1's instruction. The help texts and the
/// planner's rule use the same words. None for other work zeros.
///
/// Front-left's stretch of each side, 5% to 15% of its length from the
/// corner, is rounded inwards to 0.1 mm, so that it lies in the checked
/// stretch; the -X and -Y sides are named apart when their stretches differ.
pub(crate) fn touch_off_spots(origin: &str, stock: Bounds) -> Option<String> {
    let band = short_mm(side_band(stock));
    match origin {
        "Stock top center" => Some(format!(
            "Z0 on the stock top at its centre and X0 Y0 from the four sides near their middles, between 1 mm and {band} mm below the top"
        )),
        "Stock top front-left" => {
            let stretch = |length: f32| {
                let from = (length * 0.05 * 10. - 0.001).ceil() / 10.;
                let to = (length * 0.15 * 10. + 0.001).floor() / 10.;
                format!("between {} mm and {} mm", short_mm(from), short_mm(to))
            };
            // The -X side runs along Y, the -Y side along X.
            let size = stock.dimensions();
            let (x, y) = (stretch(size[1]), stretch(size[0]));
            let sides = if x == y {
                format!("the -X and -Y sides {x}")
            } else {
                format!("the -X side {x} and the -Y side {y}")
            };
            Some(format!(
                "Z0 on the stock top within {} mm of the front-left corner, at least 1 mm in from both edges, and X0 Y0 from {sides} from the front-left corner, between 1 mm and {band} mm below the top",
                short_mm(corner_area(stock))
            ))
        }
        _ => None,
    }
}
/// What the header says of the retracts the jaws and clamps check leaves out.
const MACHINE_HOME: &str = "Retracts to machine home are assumed to clear everything.";
/// What the header says last of what was not checked.
const SOFT_LIMITS: &str =
    "LinuxCNC soft limits check absolute position. Preview in LinuxCNC and air-cut first.";
/// Said in place of the stock top when an earlier setup has cut it.
pub const STOCK_TOP_CUT: &str =
    "The stock top was cut by an earlier setup; touch off on a part face.";
/// A Model-origin setup's work zero located for the operator, from the
/// setup's first simulation: the part's box in work coordinates (mm) and the
/// stock top's Z, or None for the stock top when an earlier setup has cut it
/// (`stock_top_cut`). None for other work zeros.
pub(crate) fn model_zero(
    setup: &Setup,
    mesh: &Mesh,
    sim: &Simulation,
) -> Option<(Bounds, Option<f32>)> {
    if get(&setup.p, "origin") != "Model origin" {
        return None;
    }
    let stock = sim.plan.stock;
    let zero = datum(setup, stock, mesh).ok()?;
    let (part, top) = zero_place(part_box(setup, mesh), stock, zero);
    let cut = match (&sim.initial_volume, sim.frame) {
        (Some(volume), Some(frame)) => stock_top_cut(stock, volume, frame),
        _ => false,
    };
    Some((part, (!cut).then_some(top)))
}
pub fn coord(v: f32) -> String {
    let s = format!("{v:.3}");
    if s == "-0.000" { "0.000".into() } else { s }
}
/// Comment text limited to printable ASCII on one line, without parentheses.
///
/// LinuxCNC runs a comment that begins with MSG, LOGOPEN, PROBEOPEN and the
/// like as a command, so every comment that carries text from the user or
/// the model begins with a fixed word of the app's and a colon.
pub fn comment(s: &str) -> String {
    clean(s).chars().take(200).collect()
}
/// `text` as comment lines of at most 200 characters, broken between words
/// (a word too long for a line is split, never cut off). The first line
/// begins with `lead` and every later one with `continued`, so that no line
/// begins with the text itself.
fn comment_lines(lead: &str, continued: &str, text: &str) -> Vec<String> {
    let mut lines = vec![lead.to_string()];
    // The last line holds only its lead word.
    let mut bare = true;
    for word in clean(text).split(' ').filter(|w| !w.is_empty()) {
        let mut word = word;
        loop {
            let line = lines.last_mut().unwrap();
            let room = 200usize.saturating_sub(line.len() + usize::from(!bare));
            if word.len() <= room {
                if !bare {
                    line.push(' ');
                }
                line.push_str(word);
                bare = false;
                break;
            }
            if bare {
                // `clean` leaves ASCII only, so any byte is a character boundary.
                let (head, rest) = word.split_at(room);
                line.push_str(head);
                word = rest;
            }
            lines.push(continued.to_string());
            bare = true;
        }
    }
    lines
}
/// Printable ASCII on one line, without parentheses or repeated spaces.
fn clean(s: &str) -> String {
    let mapped: String = s
        .replace('±', "+-")
        .chars()
        .map(|c| match c {
            '(' | ')' => ' ',
            'Ø' | 'ø' => 'D',
            '·' | '–' | '—' => '-',
            '×' => 'x',
            c if c.is_ascii_graphic() || c == ' ' => c,
            c if c.is_whitespace() => ' ',
            _ => '?',
        })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}
pub fn preview(text: &str, lines: usize) -> String {
    let total = text.lines().count();
    let mut out = text.lines().take(lines).collect::<Vec<_>>().join("\n");
    if total > lines {
        out += &format!("\n(… {} more lines)", total - lines);
    }
    out
}
/// What must be active while one operation runs.
#[derive(Debug, Clone, PartialEq)]
pub struct OpState {
    pub name: String,
    pub strategy: &'static str,
    pub tool: u32,
    pub tool_id: String,
    pub tool_name: String,
    pub diameter: f32,
    pub stickout: f32,
    pub rpm: f32,
    pub clockwise: bool,
    /// (mist, flood)
    pub coolant: (bool, bool),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Rapid,
    Feed,
    Dwell,
}
/// One setup's program in plan coordinates: every motion and state change.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// M9/M5 for whatever is running.
    Stop,
    /// G53 G0 Z0, or G0 Z to the setup clearance plane.
    Retract,
    /// Begin an operation: its comment, and when `change` in tool-table mode, T# M6 then G43 H#.
    Start {
        op: usize,
        change: bool,
    },
    /// (Re)issue the spindle (S M3/M4) and coolant for an operation.
    Run {
        op: usize,
        spindle: bool,
        coolant: bool,
    },
    /// Spindle acceleration dwell from the machine settings.
    Wait {
        seconds: f32,
    },
    /// Rapid in X/Y at the retract height after a tool change.
    Approach {
        op: usize,
        xy: [f32; 2],
    },
    /// Rapid down from the retract height to the operation's first point.
    Descend {
        op: usize,
        xy: [f32; 2],
        z: f32,
    },
    Move {
        op: usize,
        kind: Kind,
        from: [f32; 3],
        to: [f32; 3],
        feed: f32,
        seconds: f32,
    },
    End,
}
pub fn blocks(ops: &[OpState], plans: &[&Plan], settings: &PostSettings) -> Vec<Block> {
    let mut out = vec![];
    for (k, (op, plan)) in ops.iter().zip(plans).enumerate() {
        let first = plan.moves[0].from;
        let previous = k.checked_sub(1);
        let change = previous.is_none_or(|p| ops[p].tool_id != op.tool_id);
        let spindle = change
            || previous.is_some_and(|p| ops[p].rpm != op.rpm || ops[p].clockwise != op.clockwise);
        let coolant = change || previous.is_some_and(|p| ops[p].coolant != op.coolant);
        // With a tool table and clearance-plane retracts, the first G0 Z needs the new tool's G43 H.
        if change && !(k == 0 && settings.tool_table && !settings.machine_home) {
            out.extend([Block::Stop, Block::Retract]);
        }
        out.push(Block::Start { op: k, change });
        if change {
            // The new tool's tip (after G43 or a manual touch-off) goes to a safe height first.
            out.push(Block::Retract);
        }
        out.push(Block::Run {
            op: k,
            spindle,
            coolant,
        });
        if spindle && settings.dwell > 0. {
            out.push(Block::Wait {
                seconds: settings.dwell,
            });
        }
        if change {
            out.push(Block::Approach {
                op: k,
                xy: [first[0], first[1]],
            });
            out.push(Block::Descend {
                op: k,
                xy: [first[0], first[1]],
                z: first[2],
            });
        } else {
            // Same tool: climb, cross and descend between the two clearance points.
            let from = plans[k - 1].moves.last().unwrap().to;
            let high = from[2].max(first[2]);
            let mut at = from;
            for to in [[from[0], from[1], high], [first[0], first[1], high], first] {
                if to != at {
                    out.push(Block::Move {
                        op: k,
                        kind: Kind::Rapid,
                        from: at,
                        to,
                        feed: 0.,
                        seconds: 0.,
                    });
                    at = to;
                }
            }
        }
        for m in &plan.moves {
            let kind = match m.motion {
                Motion::Rapid => Kind::Rapid,
                Motion::Feed => Kind::Feed,
                Motion::Dwell => Kind::Dwell,
            };
            out.push(Block::Move {
                op: k,
                kind,
                from: m.from,
                to: m.to,
                feed: if kind == Kind::Feed { m.feed } else { 0. },
                seconds: if kind == Kind::Dwell {
                    m.end - m.start
                } else {
                    0.
                },
            });
        }
    }
    out.extend([Block::Stop, Block::Retract, Block::End]);
    out
}
/// Highest tool-tip Z of the setup's motion (plan coordinates).
pub fn clearance_plane(blocks: &[Block]) -> f32 {
    blocks.iter().fold(f32::NEG_INFINITY, |z, b| match b {
        Block::Move { from, to, .. } => z.max(from[2]).max(to[2]),
        Block::Descend { z: d, .. } => z.max(*d),
        _ => z,
    })
}
/// Tool-tip extents of the setup's motion (plan coordinates).
pub fn extents(blocks: &[Block]) -> Bounds {
    let mut b = Bounds {
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
    };
    let points = blocks.iter().flat_map(|block| match block {
        Block::Move { from, to, .. } => vec![*from, *to],
        Block::Descend { xy, z, .. } => vec![[xy[0], xy[1], *z]],
        _ => vec![],
    });
    for p in points {
        for k in 0..3 {
            b.min[k] = b.min[k].min(p[k]);
            b.max[k] = b.max[k].max(p[k]);
        }
    }
    b
}
pub struct Header<'a> {
    pub job: &'a str,
    pub setup: &'a Setup,
    pub number: usize,
    /// (program, count) when touch-off mode splits a setup into one program per tool run.
    pub part: Option<(usize, usize)>,
    pub stock: Bounds,
    /// A Model-origin zero located for the operator (see `model_zero`).
    pub located: Option<(Bounds, Option<f32>)>,
    pub datum: [f32; 3],
    /// The setup's modelled workholding in a sentence (`Document::holding_text`).
    pub holding: Option<String>,
    pub disabled: Vec<String>,
}
struct Writer(String);
impl Writer {
    fn line(&mut self, s: impl AsRef<str>) {
        self.0.push_str(s.as_ref());
        self.0.push('\n');
    }
    /// A comment of the app's own words, which always fit on one line.
    fn note(&mut self, s: &str) {
        self.line(format!("({})", comment(s)));
    }
    /// `text` after `lead` as whole comment lines, never cut off: later
    /// lines begin with the lead's words and ", continued: "
    /// (`comment_lines`).
    fn wrapped(&mut self, lead: &str, text: &str) {
        let continued = format!("{}, continued: ", lead.trim_end().trim_end_matches(':'));
        for line in comment_lines(lead, &continued, text) {
            self.line(format!("({})", line.trim_end()));
        }
    }
}
pub fn emit(h: &Header, ops: &[OpState], blocks: &[Block], settings: &PostSettings) -> String {
    let (d, setup) = (h.datum, h.setup);
    let offset = get(&setup.p, "offset");
    let mut w = Writer(String::new());
    w.line("(AXIS CAM Studio - LinuxCNC 3-axis program)");
    w.wrapped(
        "Job: ",
        &format!(
            "{} - Setup {}: {} - {offset}",
            h.job,
            h.number,
            setup.name()
        ),
    );
    if let Some((i, count)) = h.part {
        w.note(&format!(
            "Program {i} of {count} for this setup, one per tool"
        ));
    }
    w.wrapped(
        "Machine: ",
        &format!("{} v{}", setup.machine.name(), setup.machine.version),
    );
    // Later touch-off programs keep program 1's X/Y; the stock top may already be cut.
    let later = h.part.is_some_and(|(i, _)| i > 1);
    // Model origin is a point in the CAD file, often inside the part or the
    // stock: it is located from the part's walls and the stock top instead.
    let origin = get(&setup.p, "origin");
    let model = origin == "Model origin";
    if later {
        w.note(&format!(
            "Work zero: {origin} in {offset}. X0 Y0 are unchanged from program 1."
        ));
    } else if let Some(spots) = touch_off_spots(origin, h.stock) {
        // Front-left's sentence runs on over a second line.
        w.wrapped(
            "Work zero: ",
            &format!("{origin} in {offset}. Touch off {spots}."),
        );
    } else {
        w.note(&format!("Work zero: {origin} in {offset}."));
    }
    // The stock top is offered only while no earlier setup has cut it.
    let top = h.located.and_then(|(_, top)| top);
    if let (true, Some((part, _))) = (model, h.located) {
        w.note(&format!(
            "Part in work coordinates: X{} to {} Y{} to {} Z{} to {} mm.{}",
            coord(part.min[0]),
            coord(part.max[0]),
            coord(part.min[1]),
            coord(part.max[1]),
            coord(part.min[2]),
            coord(part.max[2]),
            match top {
                Some(top) if !later => format!(" Stock top Z{}.", coord(top)),
                _ => String::new(),
            }
        ));
        if !later {
            w.note(if top.is_some() {
                "Touch off on a part wall or the stock top and enter its coordinate."
            } else {
                STOCK_TOP_CUT
            });
        }
    }
    let size = h.stock.dimensions();
    w.wrapped(
        "Stock: ",
        &format!(
            "{} x {} x {} mm. Part rotation X{} Y{} Z{} deg.",
            coord(size[0]),
            coord(size[1]),
            coord(size[2]),
            get(&setup.p, "rx"),
            get(&setup.p, "ry"),
            get(&setup.p, "rz")
        ),
    );
    w.wrapped("Fixture: ", get(&setup.p, "fixture"));
    if let Some(holding) = &h.holding {
        w.wrapped("Workholding: ", holding);
    }
    if !clean(get(&setup.p, "notes")).is_empty() {
        w.wrapped("Setup notes: ", get(&setup.p, "notes"));
    }
    let mut listed = vec![];
    for op in ops {
        if !listed.contains(&op.tool) {
            listed.push(op.tool);
            w.wrapped(
                &format!("T{}: ", op.tool),
                &format!(
                    "{} - diameter {} mm - stickout {} mm",
                    op.tool_name,
                    coord(op.diameter),
                    coord(op.stickout)
                ),
            );
        }
    }
    for (k, op) in ops.iter().enumerate() {
        w.wrapped(
            &format!("Operation {}: ", k + 1),
            &format!(
                "{} - {} - T{} - S{}",
                op.name,
                op.strategy,
                op.tool,
                op.rpm.round()
            ),
        );
    }
    for name in &h.disabled {
        w.wrapped("Suppressed: ", name);
    }
    let e = extents(blocks);
    w.line(format!(
        "(Extents: X{} to {} Y{} to {} Z{} to {} mm)",
        coord(e.min[0] - d[0]),
        coord(e.max[0] - d[0]),
        coord(e.min[1] - d[1]),
        coord(e.max[1] - d[1]),
        coord(e.min[2] - d[2]),
        coord(e.max[2] - d[2])
    ));
    // The program is only written once its jaws or clamps have been checked.
    if h.holding.is_some() {
        w.note("Not checked: parallels, bolts, the table, the machine body and tool-change motion. Vise jaws and clamps were checked as boxes with a clearance gap.");
        w.note(MACHINE_HOME);
        w.note(SOFT_LIMITS);
    } else {
        w.line(format!(
            "(Not checked: fixtures, clamps and the machine body. {SOFT_LIMITS})"
        ));
    }
    if settings.tool_table && !settings.machine_home {
        w.note("Start with the spindle raised clear of the part and fixtures: the first tool change happens at the current height.");
    }
    if !settings.tool_table {
        // AXIS cannot touch off while a program is paused; G49 below would cancel a Tool Touch Off.
        // Earlier programs cut away the Z0 surface, so later tools use a fixed reference.
        let (t, name) = (ops[0].tool, &ops[0].tool_name);
        if h.part.is_none_or(|(i, _)| i == 1) {
            w.wrapped(
                "Before starting: ",
                &format!(
                    "load T{t} {name}, then use AXIS Touch Off, not Tool Touch Off, {}",
                match touch_off_spots(origin, h.stock) {
                    Some(spots) => format!("to touch off {spots}."),
                    None if model && top.is_some() => {
                        "on a part wall or the stock top and enter its coordinate from the lines above.".into()
                    }
                    None if model => {
                        "on a part face and enter its coordinate from the lines above.".into()
                    }
                    None => "to set X0 Y0 Z0 at the work zero.".into(),
                }
                ),
            );
            w.note("Before removing this tool, note the Z of a fixed reference surface such as a vise jaw.");
        } else {
            w.wrapped(
                "Before starting: ",
                &format!(
                    "load T{t} {name}, then use AXIS Touch Off, not Tool Touch Off, on the fixed reference surface noted after program 1 and enter its noted Z."
                ),
            );
            w.note("Do not touch off on the stock: earlier programs have cut it.");
        }
        // A G43 left active from an earlier program would offset the touch-off.
        w.note("Run G49 in MDI before touching off, and jog the tool clear of the part before Cycle Start.");
    }
    w.line("G17 G21 G40 G49 G80 G90 G94");
    w.line(if settings.blend > 0. {
        format!("G64 P{:.3}", settings.blend)
    } else {
        "G61".into()
    });
    w.line(offset);
    let clearance = coord(clearance_plane(blocks) - d[2]);
    let mut position: [Option<String>; 3] = [None, None, None];
    let mut motion: Option<Kind> = None;
    let mut feed: Option<String> = None;
    let mut spindle_on = false;
    let mut coolant = (false, false);
    for block in blocks {
        match block {
            Block::Stop => {
                if coolant != (false, false) {
                    w.line("M9");
                    coolant = (false, false);
                }
                if spindle_on {
                    w.line("M5");
                    spindle_on = false;
                }
            }
            Block::Retract => {
                if settings.machine_home {
                    w.line("G53 G0 Z0");
                    position[2] = None;
                } else {
                    w.line(format!("G0 Z{clearance}"));
                    position[2] = Some(clearance.clone());
                }
                motion = Some(Kind::Rapid);
            }
            Block::Start { op, change } => {
                let s = &ops[*op];
                w.wrapped(
                    &format!("Operation {}/{}: ", op + 1, ops.len()),
                    &format!("{} - T{}", s.name, s.tool),
                );
                if *change && settings.tool_table {
                    w.line(format!("T{} M6", s.tool));
                    w.line(format!("G43 H{}", s.tool));
                }
            }
            Block::Run {
                op,
                spindle,
                coolant: set,
            } => {
                let s = &ops[*op];
                if *spindle {
                    // The replay expects rpm.round(); `{:.0}` would round ties to even.
                    w.line(format!(
                        "S{} {}",
                        s.rpm.round(),
                        if s.clockwise { "M3" } else { "M4" }
                    ));
                    spindle_on = true;
                }
                if *set && s.coolant != coolant {
                    if coolant != (false, false) {
                        w.line("M9");
                    }
                    if s.coolant.0 {
                        w.line("M7");
                    }
                    if s.coolant.1 {
                        w.line("M8");
                    }
                    coolant = s.coolant;
                }
            }
            Block::Wait { seconds } => w.line(format!("G4 P{seconds:.3}")),
            Block::Approach { xy, .. } => {
                let (x, y) = (coord(xy[0] - d[0]), coord(xy[1] - d[1]));
                w.line(format!("G0 X{x} Y{y}"));
                position[0] = Some(x);
                position[1] = Some(y);
                motion = Some(Kind::Rapid);
            }
            Block::Descend { z, .. } => {
                let z = coord(*z - d[2]);
                w.line(format!("G0 Z{z}"));
                position[2] = Some(z);
                motion = Some(Kind::Rapid);
            }
            Block::Move {
                kind: Kind::Dwell,
                seconds,
                ..
            } => w.line(format!("G4 P{seconds:.3}")),
            Block::Move {
                kind,
                to,
                feed: rate,
                ..
            } => {
                let mut words = String::new();
                for (k, axis) in ["X", "Y", "Z"].iter().enumerate() {
                    let v = coord(to[k] - d[k]);
                    if position[k].as_deref() != Some(v.as_str()) {
                        words += &format!(" {axis}{v}");
                        position[k] = Some(v);
                    }
                }
                if words.is_empty() {
                    continue;
                }
                let mut line = String::new();
                if motion != Some(*kind) {
                    line += if *kind == Kind::Rapid { "G0" } else { "G1" };
                    motion = Some(*kind);
                }
                line += &words;
                if *kind == Kind::Feed {
                    let f = format!("{rate:.1}");
                    if feed.as_deref() != Some(f.as_str()) {
                        line += &format!(" F{f}");
                        feed = Some(f);
                    }
                }
                w.line(line.trim_start());
            }
            Block::End => w.line("M2"),
        }
    }
    w.0
}
#[derive(Debug, Clone)]
pub struct Program {
    pub file_name: String,
    pub text: String,
    pub lines: usize,
    pub tools: Vec<String>,
    pub operations: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct SetupPost {
    pub setup: usize,
    pub name: String,
    /// One program per setup, or one per tool run in touch-off mode.
    pub result: Result<Vec<Program>, Vec<String>>,
}
#[derive(Debug, Clone, Default)]
pub struct JobPost {
    pub errors: Vec<String>,
    pub setups: Vec<SetupPost>,
}
impl JobPost {
    pub fn blocked(message: &str) -> Self {
        Self {
            errors: vec![message.into()],
            setups: vec![],
        }
    }
    pub fn ready(&self) -> bool {
        self.errors.is_empty()
            && !self.setups.is_empty()
            && self.setups.iter().all(|s| s.result.is_ok())
    }
}
pub fn file_name(doc: &Document, index: usize, part: Option<(usize, usize, u32)>) -> String {
    let clean = |s: &str| -> String {
        let s: String = s
            .chars()
            .map(|c| {
                if c.is_control() || "/\\:*?\"<>|".contains(c) {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        s.trim().chars().take(60).collect()
    };
    let setup = &doc.setups[index];
    let job = clean(&doc.title);
    let base = format!(
        "{} - {} {} ({})",
        if job.is_empty() {
            "AXIS job".into()
        } else {
            job
        },
        index + 1,
        clean(setup.name()),
        clean(get(&setup.p, "offset"))
    );
    match part {
        Some((i, count, tool)) => format!("{base} - {i} of {count} T{tool}.ngc"),
        None => format!("{base}.ngc"),
    }
}
pub(crate) fn op_state(
    doc: &Document,
    op: &Operation,
    plan: &Plan,
    settings: Option<&PostSettings>,
) -> Result<OpState, Vec<String>> {
    let Some(tool) = doc.tools.iter().find(|t| t.id == op.tool) else {
        return Err(vec![format!("{}: choose a saved tool.", op.name())]);
    };
    let mut e: Vec<String> = doc
        .tool_number_issues(tool)
        .into_iter()
        .map(|x| format!("{}: {x}", op.name()))
        .collect();
    let coolant = match get(&op.p, "coolant") {
        "Flood" => Some((false, true)),
        "Mist" => Some((true, false)),
        "Off" => Some((false, false)),
        "Air" => settings.and_then(|s| s.air),
        _ => None,
    };
    if coolant.is_none() {
        e.push(format!(
            "{}: coolant \"{}\" has no LinuxCNC output on this machine.",
            op.name(),
            get(&op.p, "coolant")
        ));
    }
    if !e.is_empty() {
        return Err(e);
    }
    Ok(OpState {
        name: op.name().into(),
        strategy: op.strategy.name(),
        tool: tool.number().unwrap(),
        tool_id: tool.id.clone(),
        tool_name: tool.name().into(),
        diameter: plan.radius * 2.,
        stickout: plan.tool_length,
        rpm: num(&op.p, "rpm").unwrap_or(0.),
        clockwise: plan.spindle_clockwise,
        coolant: coolant.unwrap(),
    })
}
/// 7 × 7 sample points across a face of stock box `s`, inset 1% from its
/// edges, on the plane at `plane` across axis `k`.
fn face_points(s: Bounds, k: usize, plane: f32) -> impl Iterator<Item = [f32; 3]> {
    let at =
        move |k: usize, n: usize| s.min[k] + (s.max[k] - s.min[k]) * (0.01 + 0.98 * n as f32 / 6.);
    let (i, j) = ((k + 1) % 3, (k + 2) % 3);
    (0..49).map(move |n| {
        let mut p = [plane; 3];
        (p[i], p[j]) = (at(i, n % 7), at(j, n / 7));
        p
    })
}
/// Whether `p` lies in box `b`. A small guard keeps float error in the frame
/// transforms on the safe side: a point that close counts as cut.
fn in_box(p: [f32; 3], b: &Bounds) -> bool {
    (0..3).all(|a| p[a] >= b.min[a] - 0.001 && p[a] <= b.max[a] + 0.001)
}
/// The stock faces a stock-based work zero needs, as (name, axis, plane
/// 0.01 mm inside the face): the top, and the sides that place X0 Y0.
fn zero_faces(s: Bounds, center: bool) -> Vec<(&'static str, usize, f32)> {
    let mut faces = vec![
        ("top face", 2, s.max[2] - 0.01),
        ("-X side", 0, s.min[0] + 0.01),
    ];
    if center {
        faces.push(("+X side", 0, s.max[0] - 0.01));
    }
    faces.push(("-Y side", 1, s.min[1] + 0.01));
    if center {
        faces.push(("+Y side", 1, s.max[1] - 0.01));
    }
    faces
}
/// 10 mm, or `fraction` of `size` when that is less, rounded down to 0.1 mm.
fn up_to_10_mm(size: f32, fraction: f32) -> f32 {
    ((size * fraction).min(10.) * 10. + 0.001).floor() / 10.
}
/// How far below the stock top an edge finder touches the sides for a
/// stock-based work zero, above the workholding (mm): 10 mm, or a quarter
/// of the stock's height when that is less.
fn side_band(s: Bounds) -> f32 {
    up_to_10_mm(s.dimensions()[2], 0.25)
}
/// The side of the stock-top area at the front-left corner where Stock top
/// front-left's Z0 is touched off (mm): 10 mm, or a quarter of the stock's
/// shorter top side when that is less.
pub(crate) fn corner_area(s: Bounds) -> f32 {
    let size = s.dimensions();
    up_to_10_mm(size[0].min(size[1]), 0.25)
}
/// Faces of `zero_faces` with no sample point left outside every earlier cut box.
fn stock_faces_cut(s: Bounds, cuts: &[Bounds], center: bool) -> Vec<&'static str> {
    zero_faces(s, center)
        .into_iter()
        .filter(|&(_, k, plane)| {
            face_points(s, k, plane).all(|p| cuts.iter().any(|b| in_box(p, b)))
        })
        .map(|f| f.0)
        .collect()
}
/// 11 × 11 sample points, edges included, over the rectangle from `lo` to
/// `hi` on the plane across axis `k` (at `lo[k]`).
fn area_points(k: usize, lo: [f32; 3], hi: [f32; 3]) -> impl Iterator<Item = [f32; 3]> {
    const N: usize = 11;
    let (i, j) = ((k + 1) % 3, (k + 2) % 3);
    let at = move |a: usize, m: usize| lo[a] + (hi[a] - lo[a]) * m as f32 / (N - 1) as f32;
    (0..N * N).map(move |n| {
        let mut p = lo;
        (p[i], p[j]) = (at(i, n % N), at(j, n / N));
        p
    })
}
/// Where the operator touches off each face of `zero_faces`, with the words
/// that name the spot.
///
/// The sides are touched with an edge finder above the workholding, in a
/// band from 1 mm to `side_band` below the stock top: for Stock top center
/// over the middle 10% of each side's length, for Stock top front-left over
/// 5% to 15% of the -X and -Y sides' length from the corner, clear of the
/// corner's edge.
///
/// The top: for Stock top center a square 10% of its shorter side at its
/// centre; for Stock top front-left a square `corner_area` on a side, 1 mm
/// in from both edges at the corner, where a Z-setter or gauge block stands.
fn touch_spots(s: Bounds, center: bool) -> Vec<(&'static str, Vec<[f32; 3]>)> {
    let size = s.dimensions();
    let band = side_band(s);
    zero_faces(s, center)
        .into_iter()
        .map(|(_, k, plane)| {
            let (mut lo, mut hi) = (s.min, s.max);
            (lo[k], hi[k]) = (plane, plane);
            if k == 2 {
                for a in 0..2 {
                    (lo[a], hi[a]) = if center {
                        let (c, half) = ((s.min[a] + s.max[a]) / 2., 0.05 * size[0].min(size[1]));
                        (c - half, c + half)
                    } else {
                        (s.min[a] + 1., s.min[a] + 1. + corner_area(s))
                    };
                }
            } else {
                // Along the side: Y for the X sides, X for the Y sides.
                let along = 1 - k;
                let (from, to) = if center { (0.45, 0.55) } else { (0.05, 0.15) };
                (lo[along], hi[along]) = (
                    s.min[along] + from * size[along],
                    s.min[along] + to * size[along],
                );
                (lo[2], hi[2]) = (s.max[2] - band, s.max[2] - band.min(1.));
            }
            let words = match (center, k) {
                (true, 2) => "at its centre",
                (true, _) => "near its centre",
                _ => "near the front-left corner",
            };
            (words, area_points(k, lo, hi).collect())
        })
        .collect()
}
/// The faces a probe touches for a stock-based work zero, in `zero_faces`'s
/// order (the top first): its name, its axis, whether it is the stock's far
/// side on that axis (+X, +Y, the top), and the box its `touch_spots` span.
pub(crate) fn probe_faces(s: Bounds, center: bool) -> Vec<(&'static str, usize, bool, Bounds)> {
    zero_faces(s, center)
        .into_iter()
        .zip(touch_spots(s, center))
        .map(|((name, k, plane), (_, points))| {
            let mut spot = Bounds {
                min: [f32::INFINITY; 3],
                max: [f32::NEG_INFINITY; 3],
            };
            for p in points {
                for a in 0..3 {
                    spot.min[a] = spot.min[a].min(p[a]);
                    spot.max[a] = spot.max[a].max(p[a]);
                }
            }
            (name, k, plane > (s.min[k] + s.max[k]) / 2., spot)
        })
        .collect()
}
/// The faces a stock-based work zero needs that earlier setups have cut
/// where the operator touches off on them: a face counts when one of its
/// `touch_spots` points lies where `rest::Removed` finds material can be
/// gone. It is named alone when the whole face is cut (`stock_faces_cut`
/// on `cut_boxes`), otherwise with the spot. `cut_boxes` reach a tool
/// radius below each cut, so they only choose the words: a side cut lower
/// down, where the workholding holds it, does not count.
fn stock_zero_cut(
    stock: Bounds,
    volume: &crate::volume::Volume,
    frame: crate::volume::Frame,
    center: bool,
) -> Vec<String> {
    let whole = stock_faces_cut(stock, &crate::rest::cut_boxes(volume, frame), center);
    let removed = crate::rest::Removed::new(volume, frame);
    zero_faces(stock, center)
        .into_iter()
        .zip(touch_spots(stock, center))
        .filter_map(|((face, ..), (spot, points))| {
            if !points.iter().any(|&p| removed.at(p)) {
                None
            } else if whole.contains(&face) {
                Some(face.to_string())
            } else {
                Some(format!("{face} {spot}"))
            }
        })
        .collect()
}
/// Whether earlier setups have cut the stock's top face, wholly or in part:
/// wholly by the stock-based zeros' test (`stock_faces_cut` on `cut_boxes`),
/// in part when one of its sample points lies where an earlier cut can have
/// removed material (`removal_boxes`, which reach no lower than the tool tip,
/// so an uncut underside 3 mm below a contour's floor stays a stock top).
fn stock_top_cut(
    stock: Bounds,
    volume: &crate::volume::Volume,
    frame: crate::volume::Frame,
) -> bool {
    let removed = crate::rest::removal_boxes(volume, frame);
    stock_faces_cut(stock, &crate::rest::cut_boxes(volume, frame), false).contains(&"top face")
        || face_points(stock, 2, stock.max[2] - 0.01).any(|p| removed.iter().any(|b| in_box(p, b)))
}
pub(crate) struct Prepared {
    pub file_name: String,
    pub text: String,
    pub blocks: Vec<Block>,
    pub ops: Vec<OpState>,
    pub datum: [f32; 3],
    pub settings: PostSettings,
    pub offset: String,
}
/// One setup's enabled operations as export walks them: their states and
/// simulations, the work zero they share and the programs they split into.
struct Walk<'a> {
    settings: PostSettings,
    zero: [f32; 3],
    stock: Bounds,
    ops: Vec<OpState>,
    /// The simulations of `ops`, in order.
    simulations: Vec<&'a Simulation>,
    /// The index in the setup of each of `ops`.
    indexes: Vec<usize>,
    /// The runs of `ops` that make one program each: touch-off mode splits
    /// the setup at every change of tool.
    groups: Vec<std::ops::Range<usize>>,
}
/// How a setup's machine turns into post settings: `PostSettings::from_machine`
/// for G-code export, `PostSettings::for_uc100` for a UC100 run.
pub(crate) type Settings = fn(&Machine) -> Result<PostSettings, Vec<String>>;
/// A setup's `Walk`, or why export cannot walk it: a setting, an operation
/// that did not generate or cannot be posted, no work zero, or stock boxes
/// that differ between its operations.
fn walk<'a>(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    sims: &'a [(usize, Result<Simulation, String>)],
    settings: Settings,
) -> Result<Walk<'a>, Vec<String>> {
    let setup = &doc.setups[si];
    let offset = get(&setup.p, "offset").to_string();
    let mut errors = vec![];
    let settings = settings(&setup.machine).map_err(|e| errors.extend(e)).ok();
    if !OFFSETS.contains(&offset.as_str()) {
        errors.push(format!(
            "Work offset \"{offset}\" is not a LinuxCNC register (G54-G59.3)."
        ));
    }
    if sims.is_empty() {
        errors.push("This setup has no enabled operations. Add one or remove the setup.".into());
    }
    let mut ops = vec![];
    let mut simulations = vec![];
    for (oi, result) in sims {
        let op = &setup.operations[*oi];
        match result {
            Err(e) => errors.push(format!("{}: {e}", op.name())),
            Ok(sim) => match op_state(doc, op, &sim.plan, settings.as_ref()) {
                Ok(state) => {
                    ops.push(state);
                    simulations.push(sim);
                }
                Err(e) => errors.extend(e),
            },
        }
    }
    let zero = simulations.first().and_then(|sim| {
        datum(setup, sim.plan.stock, mesh)
            .map_err(|e| errors.push(e))
            .ok()
    });
    let (Some(settings), Some(zero), true) = (settings, zero, errors.is_empty()) else {
        return Err(errors);
    };
    // One work zero for the whole setup, even when touch-off mode splits it into programs.
    let stock = simulations[0].plan.stock;
    if simulations.iter().any(|s| s.plan.stock != stock) {
        return Err(vec![
            "Operations in this setup report different stock boxes, so the work zero is ambiguous."
                .into(),
        ]);
    }
    // Touch-off mode: one program per run of the same tool, touched off before it starts.
    let mut groups: Vec<std::ops::Range<usize>> = vec![];
    for k in 0..ops.len() {
        match groups.last_mut() {
            Some(g) if settings.tool_table || ops[g.start].tool_id == ops[k].tool_id => {
                g.end = k + 1
            }
            _ => groups.push(k..k + 1),
        }
    }
    Ok(Walk {
        settings,
        zero,
        stock,
        ops,
        simulations,
        indexes: sims.iter().map(|(oi, _)| *oi).collect(),
        groups,
    })
}
/// A workholding problem of one setup's programs: the operation it is about,
/// an index in the setup's operations, and the message.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Held {
    pub operation: Option<usize>,
    pub message: String,
}
/// What tells the bands the vise of `setup` closes on (`holding::Bands`),
/// from `first`, the setup's first simulation, which starts from the stock
/// the earlier setups left, and the part `mesh`. None for setup 1 and
/// whenever nothing was cut before: the jaws close on the whole box then.
pub(crate) fn bands(
    setup: &Setup,
    mesh: &Mesh,
    first: &Simulation,
) -> Option<crate::holding::Bands> {
    let (volume, frame) = first.initial_volume.as_ref().zip(first.frame)?;
    (!volume.cuts.is_empty()).then(|| {
        crate::holding::Bands::new(volume, frame, crate::simulation::part_box(setup, mesh))
    })
}
/// What the vise or clamps of setup `si` hold: the stock `walk` found and,
/// for a vise, the band its jaws close on (`holding::gripped`, from what the
/// earlier setups left).
fn gripped(doc: &Document, mesh: &Mesh, si: usize, walk: &Walk) -> crate::holding::Gripped {
    let setup = &doc.setups[si];
    let bands = bands(setup, mesh, walk.simulations[0]);
    crate::holding::held_by(doc, setup, walk.stock, bands.as_ref())
}
/// The jaws or clamps of setup `si` round `held` (`gripped`), or the message
/// that says why they cannot be modelled, named by the setup.
fn holding_boxes(
    doc: &Document,
    si: usize,
    held: crate::holding::Gripped,
) -> Result<Vec<crate::holding::HoldBox>, String> {
    let setup = &doc.setups[si];
    doc.holding_boxes(setup, held)
        .map_err(|e| format!("{}: {e}", setup.name()))
}
/// Why the vise or clamps of setup `si` cannot hold `held` (`gripped`, the
/// stock `walk` found), with what earlier setups cut away from it and
/// whether they cut the part (`mesh`) free from what the jaws or clamps hold
/// (`holding::holdable`; for setup 1 only its height counts), each named by
/// the setup.
fn unholdable(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    walk: &Walk,
    held: crate::holding::Gripped,
) -> Vec<String> {
    let setup = &doc.setups[si];
    let (Ok(layout), Some(holder)) = (
        crate::holding::layout(setup),
        doc.holder(get(&setup.p, "holder")),
    ) else {
        return vec![];
    };
    let first = walk.simulations[0];
    let part = crate::simulation::part_box(setup, mesh);
    let removed = first
        .initial_volume
        .as_ref()
        .zip(first.frame)
        .map(|(volume, frame)| crate::rest::Removed::new(volume, frame).with_part(part));
    let names = |s, o| crate::holding::operation_name(doc, s, o);
    crate::holding::holdable(si, &layout, holder, held, removed.as_ref(), &names)
        .into_iter()
        .map(|message| format!("{}: {message}", setup.name()))
        .collect()
}
impl Walk<'_> {
    /// The plans of the operations of `group`, one of `groups`.
    fn plans(&self, group: &std::ops::Range<usize>) -> Vec<&Plan> {
        self.simulations[group.clone()]
            .iter()
            .map(|s| &s.plan)
            .collect()
    }
    /// What the program of `group`, whose `blocks` are emitted, does to
    /// `boxes` (`post_check::holding_problems`).
    fn held(
        &self,
        setup: &Setup,
        group: &std::ops::Range<usize>,
        blocks: &[Block],
        boxes: &[crate::holding::HoldBox],
    ) -> Vec<Held> {
        let gap = crate::holding::gap(setup).unwrap_or_default();
        crate::post_check::holding_problems(
            &self.ops[group.clone()],
            &self.plans(group),
            blocks,
            &self.settings,
            boxes,
            gap,
            self.zero,
        )
        .into_iter()
        .map(|(k, message)| Held {
            operation: k.map(|k| self.indexes[group.start + k]),
            message,
        })
        .collect()
    }
}
/// Every workholding problem `prepare` finds for setup `si` from its
/// `sequence` results, in every program (`prepare` stops at the first program
/// with any problem, and before them all at a stock the workholding cannot
/// hold), each once: the messages are those of `prepare`, or of a UC100
/// run's check for a UC100 machine (`PostSettings::for_machine`). Empty when
/// the setup cannot be walked, which `prepare` reports.
pub(crate) fn held(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    sims: &[(usize, Result<Simulation, String>)],
) -> Vec<Held> {
    held_walked(doc, mesh, si, sims).unwrap_or_default()
}
/// `held`, or None when the setup cannot be walked (its programs cannot be
/// made), so that nothing of its workholding was checked.
pub(crate) fn held_walked(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    sims: &[(usize, Result<Simulation, String>)],
) -> Option<Vec<Held>> {
    let walk = walk(doc, mesh, si, sims, PostSettings::for_machine).ok()?;
    let held = gripped(doc, mesh, si, &walk);
    let boxes = match holding_boxes(doc, si, held) {
        Ok(boxes) => boxes,
        Err(message) => {
            return Some(vec![Held {
                operation: None,
                message,
            }]);
        }
    };
    let mut found: Vec<Held> = unholdable(doc, mesh, si, &walk, held)
        .into_iter()
        .map(|message| Held {
            operation: None,
            message,
        })
        .collect();
    // Nothing to hold against: no program need be walked.
    if boxes.is_empty() {
        return Some(found);
    }
    // Every program of a touch-off setup starts the same way: what two of
    // them say alike (the clearance plane's problem) is said once.
    for group in &walk.groups {
        let blocks = blocks(&walk.ops[group.clone()], &walk.plans(group), &walk.settings);
        for problem in walk.held(&doc.setups[si], group, &blocks, &boxes) {
            if !found.contains(&problem) {
                found.push(problem);
            }
        }
    }
    Some(found)
}
/// One program of a setup that passed every check: its blocks and what they
/// were made from.
#[derive(Debug, Clone)]
pub struct Checked {
    pub blocks: Vec<Block>,
    pub ops: Vec<OpState>,
    /// The index in the setup of each of `ops`.
    pub indexes: Vec<usize>,
    pub stock: Bounds,
    pub datum: [f32; 3],
    pub settings: PostSettings,
    pub offset: String,
    /// The setup's work zero: Stock top center, Stock top front-left or Model origin.
    pub origin: String,
    /// (program, count, tool) when touch-off mode splits the setup.
    pub part: Option<(usize, usize, u32)>,
    /// A Model-origin zero located for the operator (`model_zero`).
    pub located: Option<(Bounds, Option<f32>)>,
}
/// Check and emit one setup from its `sequence` results (enabled operations only):
/// one program, or one per run of the same tool in touch-off mode.
pub(crate) fn prepare(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    sims: &[(usize, Result<Simulation, String>)],
) -> Result<Vec<Prepared>, Vec<String>> {
    let setup = &doc.setups[si];
    let programs = checked(doc, mesh, si, sims, PostSettings::from_machine)?;
    Ok(programs
        .into_iter()
        .map(|c| {
            let header = Header {
                job: &doc.title,
                setup,
                number: si + 1,
                part: c.part.map(|(i, count, _)| (i, count)),
                stock: c.stock,
                located: c.located,
                datum: c.datum,
                holding: doc.holding_text(setup),
                disabled: setup
                    .operations
                    .iter()
                    .filter(|o| !o.enabled)
                    .map(|o| o.name().to_string())
                    .collect(),
            };
            Prepared {
                file_name: file_name(doc, si, c.part),
                text: emit(&header, &c.ops, &c.blocks, &c.settings),
                blocks: c.blocks,
                ops: c.ops,
                datum: c.datum,
                settings: c.settings,
                offset: c.offset,
            }
        })
        .collect())
}
/// Check one setup from its `sequence` results (enabled operations only), as
/// export does, with the post settings `settings` makes of its machine: its
/// programs, one, or one per run of the same tool in touch-off mode.
pub(crate) fn checked(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    sims: &[(usize, Result<Simulation, String>)],
    settings: Settings,
) -> Result<Vec<Checked>, Vec<String>> {
    let setup = &doc.setups[si];
    let offset = get(&setup.p, "offset").to_string();
    let walk = walk(doc, mesh, si, sims, settings)?;
    let (settings, zero, stock) = (walk.settings, walk.zero, walk.stock);
    let (ops, simulations, groups) = (&walk.ops, &walk.simulations, &walk.groups);
    let held = gripped(doc, mesh, si, &walk);
    let boxes = holding_boxes(doc, si, held).map_err(|e| vec![e])?;
    let unholdable = unholdable(doc, mesh, si, &walk, held);
    if !unholdable.is_empty() {
        return Err(unholdable);
    }
    let origin = get(&setup.p, "origin");
    if let (true, Some(volume), Some(frame)) = (
        origin == "Stock top center" || origin == "Stock top front-left",
        &simulations[0].initial_volume,
        simulations[0].frame,
    ) {
        if !volume.frame.square_to(frame) {
            return Err(vec![format!(
                "{}: the stock was defined in an orientation that is not square to this setup, so the {origin} work zero is not on a stock face. Choose Model origin for this setup.",
                setup.name()
            )]);
        }
        let cut = stock_zero_cut(stock, volume, frame, origin == "Stock top center");
        if let Some((last, rest)) = cut.split_last() {
            let list = if rest.is_empty() {
                last.to_string()
            } else {
                format!("{} and {last}", rest.join(", "))
            };
            return Err(vec![format!(
                "{}: earlier setups have cut the stock's {list}, so the {origin} work zero is no longer on the stock. Choose Model origin for this setup.",
                setup.name()
            )]);
        }
    }
    let located = model_zero(setup, mesh, simulations[0]);
    let count = groups.len();
    let mut out = vec![];
    for (i, g) in groups.iter().enumerate() {
        let group_ops = &ops[g.clone()];
        let group_sims = &simulations[g.clone()];
        let plans = walk.plans(g);
        let blocks = blocks(group_ops, &plans, &settings);
        let mut problems =
            crate::post_check::check_setup(setup, group_ops, group_sims, &blocks, &settings, zero);
        problems.extend(
            walk.held(setup, g, &blocks, &boxes)
                .into_iter()
                .map(|h| h.message),
        );
        if !problems.is_empty() {
            return Err(problems);
        }
        out.push(Checked {
            blocks,
            ops: group_ops.to_vec(),
            indexes: walk.indexes[g.clone()].to_vec(),
            stock: plans[0].stock,
            datum: zero,
            settings,
            offset: offset.clone(),
            origin: origin.to_string(),
            part: (!settings.tool_table).then(|| (i + 1, count, group_ops[0].tool)),
            located,
        });
    }
    Ok(out)
}
/// Why a job cannot be posted at all, before anything is simulated.
fn unpostable(doc: &Document) -> Option<JobPost> {
    if doc.file.is_none() {
        return Some(JobPost::blocked(
            "Import a part before posting. The example project's paths are illustrative and cannot be machined.",
        ));
    }
    if doc.setups.is_empty() {
        return Some(JobPost::blocked("Create a machining setup."));
    }
    None
}
pub fn post_job(doc: &Document, mesh: &Mesh) -> JobPost {
    if let Some(blocked) = unpostable(doc) {
        return blocked;
    }
    let mut results: Vec<Vec<(usize, Result<Simulation, String>)>> = vec![vec![]; doc.setups.len()];
    crate::simulation::sequence(doc, mesh, None, |si, oi, r| results[si].push((oi, r)));
    post_simulated(doc, mesh, &results)
}
/// `post_job` for simulations that have already been run, one list per setup.
pub(crate) fn post_simulated(
    doc: &Document,
    mesh: &Mesh,
    results: &[Vec<(usize, Result<Simulation, String>)>],
) -> JobPost {
    if let Some(blocked) = unpostable(doc) {
        return blocked;
    }
    let setups = (0..doc.setups.len())
        .map(|si| {
            let sims = results.get(si).map_or(&[][..], Vec::as_slice);
            let result = prepare(doc, mesh, si, sims).and_then(|parts| {
                parts
                    .into_iter()
                    .map(|p| {
                        crate::ngc::verify(
                            &p.text,
                            &p.blocks,
                            &p.ops,
                            p.datum,
                            &p.settings,
                            &p.offset,
                        )
                        // Named by number: the file name carries the project title.
                        .map_err(|e| {
                            vec![format!(
                                "Setup {}: round-trip verification failed: {e}",
                                si + 1
                            )]
                        })?;
                        let mut tools: Vec<String> = vec![];
                        for op in &p.ops {
                            let tool = format!("T{} {}", op.tool, op.tool_name);
                            if !tools.contains(&tool) {
                                tools.push(tool);
                            }
                        }
                        Ok(Program {
                            file_name: p.file_name,
                            lines: p.text.lines().count(),
                            operations: p.ops.iter().map(|o| o.name.clone()).collect(),
                            tools,
                            text: p.text,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            });
            SetupPost {
                setup: si,
                name: doc.setups[si].name().to_string(),
                result,
            }
        })
        .collect();
    JobPost {
        errors: vec![],
        setups,
    }
}
/// Write every program, or nothing when any setup failed.
pub fn write_job(
    job: &JobPost,
    folder: &std::path::Path,
) -> Result<(std::path::PathBuf, usize), String> {
    if !job.ready() {
        let reason = job
            .errors
            .first()
            .cloned()
            .or_else(|| {
                job.setups.iter().find_map(|s| {
                    s.result
                        .as_ref()
                        .err()
                        .and_then(|e| e.first().map(|x| format!("{}: {x}", s.name)))
                })
            })
            .unwrap_or_else(|| "No programs were produced.".into());
        return Err(format!("Nothing was written. {reason}"));
    }
    let mut count = 0;
    for program in job.setups.iter().flat_map(|s| s.result.as_ref().unwrap()) {
        crate::storage::write_atomic(&folder.join(&program.file_name), program.text.as_bytes())?;
        count += 1;
    }
    Ok((folder.to_path_buf(), count))
}

#[cfg(test)]
pub(crate) mod fixture {
    use super::OpState;
    use crate::{
        mesh::{Bounds, Mesh},
        model::*,
        simulation::{CutterShape, Motion, Move, Plan},
        stock::Placement,
        surface::Selection,
    };
    /// A Facing-shaped plan on a 20 × 20 × 10 mm stock, starting at `start`.
    /// For dwells the third tuple value is seconds, otherwise mm/min.
    pub fn plan(start: [f32; 3], moves: &[(Motion, [f32; 3], f32)]) -> Plan {
        let mut plan = Plan {
            shape: CutterShape::Flat,
            strategy: Strategy::Face,
            operation: "op".into(),
            setup: "setup".into(),
            moves: vec![],
            stock: Bounds {
                min: [-10., -10., 0.],
                max: [10., 10., 10.],
            },
            radius: 5.,
            flute_length: 25.,
            tool_length: 35.,
            holder_radius: 16.,
            display_scale: 1.,
            duration: 0.,
            target: 9.,
            levels: 1,
            rest_top: None,
            spindle_clockwise: true,
            milling_direction: "Climb",
        };
        let mut at = start;
        for (motion, to, rate) in moves {
            let seconds = if *motion == Motion::Dwell {
                *rate
            } else {
                (0..3).map(|k| (to[k] - at[k]).powi(2)).sum::<f32>().sqrt() / rate * 60.
            };
            plan.moves.push(Move {
                from: at,
                to: *to,
                motion: *motion,
                cutting_pass: *motion == Motion::Feed,
                feed: if *motion == Motion::Dwell { 0. } else { *rate },
                start: plan.duration,
                end: plan.duration + seconds,
            });
            plan.duration += seconds;
            at = *to;
        }
        plan
    }
    pub fn facing_op() -> OpState {
        OpState {
            name: "Face top".into(),
            strategy: "Facing",
            tool: 1,
            tool_id: "t1".into(),
            tool_name: "Flat end mill · Ø10".into(),
            diameter: 10.,
            stickout: 35.,
            rpm: 8000.,
            clockwise: true,
            coolant: (false, true),
        }
    }
    /// All six implemented strategies on the example block, in one G54 setup.
    pub fn job() -> (Document, Mesh) {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = manifest.join("examples/four-setup-block.3mf");
        let mesh = crate::mesh::load(&path).unwrap();
        let mut doc = Document::default();
        doc.title = "Fixture block".into();
        let mut tools = vec![
            Tool::new("Flat 10", "Flat end mill", "10"),
            Tool::new("Ball 6", "Ball end mill", "6"),
            Tool::new("Drill 4", "Drill", "4"),
        ];
        for (n, t) in tools.iter_mut().enumerate() {
            t.p.insert("length".into(), "60".into());
            t.p.insert("stickout".into(), "65".into());
            t.p.insert("number".into(), (n + 1).to_string());
        }
        let mut setup = Setup::new(Machine::new(3), "G54");
        setup.p.insert("name".into(), "Top".into());
        for (k, v) in [
            ("stock_top", "4"),
            ("stock_bottom", "3"),
            ("stock_left", "2"),
            ("stock_right", "2"),
            ("stock_front", "2"),
            ("stock_back", "2"),
        ] {
            setup.p.insert(k.into(), v.into());
        }
        let placement = Placement::new(&mesh, Some(&setup));
        let top_z = placement.bounds.max[2];
        let flats: Vec<(usize, f32, f32)> = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| s.normal[2].abs() > 0.999999)
            .map(|(i, s)| (i, placement.transform(s.center)[2], s.area))
            .collect();
        let tops: Vec<usize> = flats
            .iter()
            .filter(|f| (f.1 - top_z).abs() < 0.01)
            .map(|f| f.0)
            .collect();
        let top = flats
            .iter()
            .filter(|f| (f.1 - top_z).abs() < 0.01)
            .max_by(|a, b| a.2.total_cmp(&b.2))
            .unwrap()
            .0;
        let floor = flats
            .iter()
            .filter(|f| f.1 < top_z - 0.1 && f.1 > placement.bounds.min[2] + 0.1)
            .max_by(|a, b| a.2.total_cmp(&b.2))
            .unwrap()
            .0;
        let selection = |ids: Vec<usize>| {
            Some(Selection {
                mesh_fingerprint: mesh.fingerprint.clone(),
                ids,
            })
        };
        let (flat, ball, drill) = (
            tools[0].id.clone(),
            tools[1].id.clone(),
            tools[2].id.clone(),
        );
        let mut face = Operation::new(Strategy::Face, flat.clone());
        face.p.insert("boundary".into(), "Stock silhouette".into());
        face.p
            .insert("containment".into(), "Tool outside boundary".into());
        face.p.insert("allowance".into(), "0".into());
        face.surfaces = selection(vec![top]);
        let mut pocket = Operation::new(Strategy::Pocket, flat.clone());
        pocket.surfaces = selection(vec![floor]);
        let contour = Operation::new(Strategy::Contour, flat.clone());
        let mut drilling = Operation::new(Strategy::Drill, drill);
        drilling.surfaces = selection(tops);
        drilling.p.insert("cycle".into(), "Deep drilling".into());
        let mut parallel = Operation::new(Strategy::Parallel, ball);
        parallel.p.insert("surface_stepover".into(), "2".into());
        parallel.p.insert("tolerance".into(), "0.1".into());
        let waterline = Operation::new(Strategy::Waterline, flat);
        setup.operations = vec![face, pocket, contour, drilling, parallel, waterline];
        for op in &mut setup.operations {
            op.geometry.clear();
        }
        doc.machines = vec![setup.machine.clone()];
        doc.tools = tools;
        doc.active = setup.id.clone();
        doc.setups = vec![setup];
        doc.file = Some(ModelFile {
            name: "four-setup-block.3mf".into(),
            path: path.to_string_lossy().into_owned(),
            format: "3mf".into(),
            bytes: 0,
        });
        doc.sync_stock_sources();
        (doc, mesh)
    }
    /// Facing and Pocket on G54, then the part flipped about X and faced on G55.
    pub fn flipped_job() -> (Document, Mesh) {
        flipped_after(2)
    }
    /// `job`'s first `keep` operations on G54, then the part flipped about
    /// X and faced on G55.
    pub fn flipped_after(keep: usize) -> (Document, Mesh) {
        let (mut doc, mesh) = job();
        doc.setups[0].operations.truncate(keep);
        let mut flipped = Setup::new(Machine::new(3), "G55");
        flipped.p.insert("name".into(), "Flipped".into());
        flipped.p.insert("rx".into(), "180".into());
        let placement = Placement::new(&mesh, Some(&flipped));
        let top = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                (placement.transform(s.center)[2] - placement.bounds.max[2]).abs() < 0.01
            })
            .max_by(|(_, a), (_, b)| a.area.total_cmp(&b.area))
            .unwrap()
            .0;
        let mut face = doc.setups[0].operations[0].clone();
        face.id = id();
        face.surfaces = Some(Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: vec![top],
        });
        flipped.operations.push(face);
        doc.setups.push(flipped);
        doc.sync_stock_sources();
        (doc, mesh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::Motion;
    fn golden_plan() -> Plan {
        fixture::plan(
            [-15., -15., 25.],
            &[
                (Motion::Rapid, [-15., -15., 11.], 6000.),
                (Motion::Feed, [-15., -15., 9.], 250.),
                (Motion::Feed, [15., -15., 9.], 1000.),
                (Motion::Rapid, [15., -15., 25.], 6000.),
            ],
        )
    }
    fn program(machine: Machine) -> (String, Vec<Block>, PostSettings) {
        let settings = PostSettings::from_machine(&machine).unwrap();
        let mut setup = Setup::new(machine, "G54");
        setup.p.insert("name".into(), "Top side".into());
        let plan = golden_plan();
        let ops = vec![fixture::facing_op()];
        let blocks = blocks(&ops, &[&plan], &settings);
        let header = Header {
            job: "Golden",
            setup: &setup,
            number: 1,
            part: None,
            stock: plan.stock,
            located: None,
            datum: [0., 0., 10.],
            holding: None,
            disabled: vec![],
        };
        (emit(&header, &ops, &blocks, &settings), blocks, settings)
    }
    const GOLDEN: &str = "(AXIS CAM Studio - LinuxCNC 3-axis program)
(Job: Golden - Setup 1: Top side - G54)
(Machine: Workshop Mill v1)
(Work zero: Stock top center in G54. Touch off Z0 on the stock top at its centre and X0 Y0 from the four sides near their middles, between 1 mm and 2.5 mm below the top.)
(Stock: 20.000 x 20.000 x 10.000 mm. Part rotation X0 Y0 Z0 deg.)
(Fixture: Low profile clamps)
(T1: Flat end mill - D10 - diameter 10.000 mm - stickout 35.000 mm)
(Operation 1: Face top - Facing - T1 - S8000)
(Extents: X-15.000 to 15.000 Y-15.000 to -15.000 Z-1.000 to 15.000 mm)
(Not checked: fixtures, clamps and the machine body. LinuxCNC soft limits check absolute position. Preview in LinuxCNC and air-cut first.)
G17 G21 G40 G49 G80 G90 G94
G64 P0.010
G54
G53 G0 Z0
(Operation 1/1: Face top - T1)
T1 M6
G43 H1
G53 G0 Z0
S8000 M3
M8
G4 P2.000
G0 X-15.000 Y-15.000
G0 Z15.000
Z1.000
G1 Z-1.000 F250.0
X15.000 F1000.0
G0 Z15.000
M9
M5
G53 G0 Z0
M2
";
    #[test]
    fn golden_facing_program() {
        assert_eq!(program(Machine::new(3)).0, GOLDEN);
    }
    #[test]
    fn a_stock_based_work_zero_says_where_to_touch_off() {
        // The golden program's 20 x 20 x 10 mm stock: the sides are touched
        // within 2.5 mm (a quarter of its height) below the top, front-left's
        // from 1 to 3 mm (5% to 15% of each 20 mm side) from the corner, and
        // the front-left corner's area is 5 mm (a quarter of its shorter
        // side).
        let mut machine = Machine::new(3);
        machine
            .p
            .insert("post_tool_length".into(), POST_TOOL_LENGTH[1].into());
        let settings = PostSettings::from_machine(&machine).unwrap();
        let plan = golden_plan();
        let ops = vec![fixture::facing_op()];
        let blocks = blocks(&ops, &[&plan], &settings);
        let mut setup = Setup::new(machine, "G59.3");
        for (origin, spots) in [
            (
                "Stock top center",
                "Z0 on the stock top at its centre and X0 Y0 from the four sides near their middles, between 1 mm and 2.5 mm below the top",
            ),
            (
                "Stock top front-left",
                "Z0 on the stock top within 5 mm of the front-left corner, at least 1 mm in from both edges, and X0 Y0 from the -X and -Y sides between 1 mm and 3 mm from the front-left corner, between 1 mm and 2.5 mm below the top",
            ),
        ] {
            setup.p.insert("origin".into(), origin.into());
            let text = |part: (usize, usize)| {
                emit(
                    &Header {
                        job: "Spots",
                        setup: &setup,
                        number: 1,
                        part: Some(part),
                        stock: plan.stock,
                        located: None,
                        datum: [0., 0., 10.],
                        holding: None,
                        disabled: vec![],
                    },
                    &ops,
                    &blocks,
                    &settings,
                )
            };
            let first = text((1, 2));
            // The header line and program 1's instruction, read back across
            // their lines; no line is cut off.
            assert_eq!(
                comment_parts(&first, "Work zero: ").join(" "),
                format!("{origin} in G59.3. Touch off {spots}."),
                "{first}"
            );
            assert!(first.lines().all(|l| l.len() <= 202), "{first}");
            assert_eq!(
                comment_parts(&first, "Before starting: ").join(" "),
                format!(
                    "load T1 Flat end mill - D10, then use AXIS Touch Off, not Tool Touch Off, to touch off {spots}."
                ),
                "{first}"
            );
            // Later programs keep program 1's X0 Y0 and touch off on the
            // noted reference.
            let later = text((2, 2));
            assert!(
                later.contains(&format!(
                    "(Work zero: {origin} in G59.3. X0 Y0 are unchanged from program 1.)\n"
                )),
                "{later}"
            );
            assert!(!later.contains(spots), "{later}");
        }
        // 10 mm at most: a 124 x 84 x 47 mm stock. Its -X side is 84 mm
        // long and its -Y side 124 mm, so each gets its own stretch.
        let stock = Bounds {
            min: [-62., -42., -47.],
            max: [62., 42., 0.],
        };
        assert_eq!(
            touch_off_spots("Stock top front-left", stock).unwrap(),
            "Z0 on the stock top within 10 mm of the front-left corner, at least 1 mm in from both edges, and X0 Y0 from the -X side between 4.2 mm and 12.6 mm and the -Y side between 6.2 mm and 18.6 mm from the front-left corner, between 1 mm and 10 mm below the top"
        );
        // Rounded to 0.1 mm inside the checked area: a 13 mm tall stock is
        // touched within 3.2 mm, and its 14.2 mm -X side from 0.71 to 2.13 mm
        // from the corner is said as 0.8 to 2.1 mm.
        let low = Bounds {
            min: [0., 0., 0.],
            max: [30., 14.2, 13.],
        };
        assert_eq!((side_band(low), corner_area(low)), (3.2, 3.5));
        assert_eq!(
            touch_off_spots("Stock top front-left", low).unwrap(),
            "Z0 on the stock top within 3.5 mm of the front-left corner, at least 1 mm in from both edges, and X0 Y0 from the -X side between 0.8 mm and 2.1 mm and the -Y side between 1.5 mm and 4.5 mm from the front-left corner, between 1 mm and 3.2 mm below the top"
        );
        assert_eq!(touch_off_spots("Model origin", stock), None);
    }
    #[test]
    fn machine_settings_change_tool_length_retract_blend_and_dwell() {
        let mut m = Machine::new(3);
        m.p.insert("post_tool_length".into(), POST_TOOL_LENGTH[1].into());
        m.p.insert("post_retract".into(), POST_RETRACT[1].into());
        m.p.insert("post_blend".into(), "0".into());
        m.p.insert("post_dwell".into(), "0".into());
        let (text, _, _) = program(m);
        assert!(text.contains("(Before removing this tool, note the Z of a fixed reference surface such as a vise jaw.)\n(Run G49 in MDI before touching off, and jog the tool clear of the part before Cycle Start.)\nG17 G21"), "{text}");
        assert_eq!(
            comment_parts(&text, "Before starting: ").join(" "),
            "load T1 Flat end mill - D10, then use AXIS Touch Off, not Tool Touch Off, to touch off Z0 on the stock top at its centre and X0 Y0 from the four sides near their middles, between 1 mm and 2.5 mm below the top.",
            "{text}"
        );
        assert!(
            text.contains("(Operation 1/1: Face top - T1)\nG0 Z15.000\nS8000 M3\n"),
            "{text}"
        );
        assert!(!text.contains("M6") && !text.contains("\nM0\n") && !text.contains("G43"));
        assert!(!text.contains("G53") && !text.contains("G4 P"));
        assert!(text.contains("\nG61\n"));
        assert_eq!(text.matches("G0 Z15.000").count(), 5, "{text}");
    }
    #[test]
    fn clearance_mode_changes_the_tool_before_the_first_retract() {
        let mut m = Machine::new(3);
        m.p.insert("post_retract".into(), POST_RETRACT[1].into());
        let (text, blocks, settings) = program(m);
        assert!(
            text.contains(
                "\nG54\n(Operation 1/1: Face top - T1)\nT1 M6\nG43 H1\nG0 Z15.000\nS8000 M3\n"
            ),
            "{text}"
        );
        assert!(text.contains("(Start with the spindle raised clear of the part and fixtures: the first tool change happens at the current height.)\nG17 G21"), "{text}");
        assert!(!text.contains("\nG43\n"), "{text}");
        let early = [vec![Block::Retract], blocks].concat();
        let text = text.replacen("G54\n", "G54\nG0 Z15.000\n", 1);
        let ops = [fixture::facing_op()];
        let result = crate::ngc::verify(&text, &early, &ops, [0., 0., 10.], &settings, "G54");
        assert!(
            result
                .unwrap_err()
                .contains("no retract is expected before the first tool change")
        );
    }
    #[test]
    fn datum_follows_the_setup_origin() {
        let mesh =
            crate::mesh::normalize(vec![[[10., 20., 30.], [20., 20., 30.], [10., 40., 35.]]])
                .unwrap();
        let stock = Bounds {
            min: [-7., -9., 1.],
            max: [5., 11., 21.],
        };
        let mut setup = Setup::new(Machine::new(3), "G54");
        assert_eq!(datum(&setup, stock, &mesh).unwrap(), [-1., 1., 21.]);
        setup
            .p
            .insert("origin".into(), "Stock top front-left".into());
        assert_eq!(datum(&setup, stock, &mesh).unwrap(), [-7., -9., 21.]);
        setup.p.insert("origin".into(), "Model origin".into());
        let sources = [[10., 20., 30.], [20., 20., 30.], [10., 40., 35.]];
        for rx in ["0", "180"] {
            setup.p.insert("rx".into(), rx.into());
            let zero = datum(&setup, stock, &mesh).unwrap();
            let placement = Placement::new(&mesh, Some(&setup));
            for p in &mesh.faces[0].points {
                let v = placement.transform(*p).map(|v| v / mesh.display_scale());
                let rel: [f32; 3] = std::array::from_fn(|k| v[k] - zero[k]);
                let sign = if rx == "0" {
                    [1., 1., 1.]
                } else {
                    [1., -1., -1.]
                };
                assert!(
                    sources
                        .iter()
                        .any(|q| (0..3).all(|k| (rel[k] - sign[k] * q[k]).abs() < 1e-3)),
                    "{rx}: {rel:?}"
                );
            }
        }
        setup
            .p
            .insert("origin".into(), "Selected point (planned)".into());
        assert!(datum(&setup, stock, &mesh).is_err());
    }
    #[test]
    fn comments_cannot_escape_or_carry_non_ascii() {
        assert_eq!(comment("bad)\nG0 X1\n(Ø·×é"), "bad G0 X1 D-x?");
    }
    #[test]
    fn controller_and_axes_are_refused_before_any_output() {
        let mut m = Machine::new(3);
        m.p.insert("controller".into(), "GRBL".into());
        assert!(PostSettings::from_machine(&m).unwrap_err()[0].contains("Only the LinuxCNC"));
        assert!(
            PostSettings::from_machine(&Machine::new(4))
                .unwrap_err()
                .iter()
                .any(|e| e.contains("3-axis"))
        );
    }
    #[test]
    fn tools_change_only_between_different_tools() {
        let (doc, mesh) = fixture::job();
        let mut plans = vec![];
        crate::simulation::sequence(&doc, &mesh, None, |_, _, r| plans.push(r.unwrap().plan));
        let tool_of = |i: usize| doc.setups[0].operations[i].tool.clone();
        let ops: Vec<OpState> = (0..plans.len())
            .map(|i| OpState {
                tool_id: tool_of(i),
                ..fixture::facing_op()
            })
            .collect();
        let settings = PostSettings::from_machine(&Machine::new(3)).unwrap();
        let refs: Vec<&Plan> = plans.iter().collect();
        let blocks = blocks(&ops, &refs, &settings);
        let changes: Vec<bool> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Start { change, .. } => Some(*change),
                _ => None,
            })
            .collect();
        assert_eq!(changes, vec![true, false, false, true, true, true]);
        for pair in blocks.windows(2) {
            if let [Block::Move { to, .. }, Block::Move { from, .. }] = pair {
                assert_eq!(to, from);
            }
        }
    }
    fn errors(job: &JobPost) -> Vec<String> {
        job.errors
            .iter()
            .cloned()
            .chain(
                job.setups
                    .iter()
                    .filter_map(|s| s.result.as_ref().err())
                    .flatten()
                    .cloned(),
            )
            .collect()
    }
    fn prepared(doc: &Document, mesh: &Mesh, si: usize) -> Prepared {
        let mut results: Vec<Vec<(usize, Result<crate::simulation::Simulation, String>)>> =
            vec![vec![]; doc.setups.len()];
        crate::simulation::sequence(doc, mesh, None, |s, o, r| results[s].push((o, r)));
        prepare(doc, mesh, si, &results[si]).unwrap().remove(0)
    }
    #[test]
    fn all_six_strategies_post_and_replay_exactly() {
        let (doc, mesh) = fixture::job();
        let job = post_job(&doc, &mesh);
        assert!(job.ready(), "{:?}", errors(&job));
        let programs = job.setups[0].result.as_ref().unwrap();
        assert_eq!(programs.len(), 1);
        let program = &programs[0];
        assert_eq!(program.file_name, "Fixture block - 1 Top (G54).ngc");
        for needle in ["T1 M6", "T3 M6", "T2 M6", "\nG54\n", "G43 H3", "\nM2\n"] {
            assert!(program.text.contains(needle), "{needle}");
        }
        assert_eq!(program.operations.len(), 6);
        assert!(program.lines > 1000);
    }
    #[test]
    fn a_fractional_spindle_speed_posts_and_verifies() {
        let (mut doc, mesh) = fixture::job();
        doc.setups[0].operations[0]
            .p
            .insert("rpm".into(), "8000.5".into());
        let job = post_job(&doc, &mesh);
        assert!(job.ready(), "{:?}", errors(&job));
        let program = &job.setups[0].result.as_ref().unwrap()[0];
        assert!(program.text.contains("\nS8001 M3\n"));
        assert!(program.text.contains("- T1 - S8001)"));
    }
    #[test]
    fn corrupted_programs_fail_verification() {
        let (doc, mesh) = fixture::job();
        let p = prepared(&doc, &mesh, 0);
        let check = |text: &str| {
            crate::ngc::verify(text, &p.blocks, &p.ops, p.datum, &p.settings, &p.offset)
        };
        assert_eq!(check(&p.text), Ok(()));
        let lines: Vec<&str> = p.text.lines().collect();
        let first_feed = lines.iter().position(|l| l.starts_with("G1 ")).unwrap();
        let with = |i: usize, replacement: &str| {
            let mut l = lines.clone();
            l[i] = replacement;
            l.join("\n") + "\n"
        };
        assert!(check(&with(first_feed, "G1 Z-50.000 F250.0")).is_err());
        assert!(
            check(&with(
                first_feed,
                lines[first_feed].split(" F").next().unwrap()
            ))
            .is_err()
        );
        assert!(check(&p.text.replace("T3 M6", "T4 M6")).is_err());
        assert!(check(&p.text.replacen("G54\n", "G54\nG2 X0 Y0\n", 1)).is_err());
        let cut = lines
            .iter()
            .rposition(|l| l.starts_with('X') || l.starts_with('Y'))
            .unwrap();
        let mut dropped = lines.clone();
        dropped.remove(cut);
        assert!(check(&(dropped.join("\n") + "\n")).is_err());
    }
    #[test]
    fn golden_programs_verify_in_every_mode() {
        for (key, value) in [
            ("post_tool_length", POST_TOOL_LENGTH[0]),
            ("post_tool_length", POST_TOOL_LENGTH[1]),
            ("post_retract", POST_RETRACT[1]),
            ("post_blend", "0"),
            ("post_dwell", "0"),
        ] {
            let mut m = Machine::new(3);
            m.p.insert(key.into(), value.into());
            let (text, blocks, settings) = program(m);
            let ops = vec![fixture::facing_op()];
            assert_eq!(
                crate::ngc::verify(&text, &blocks, &ops, [0., 0., 10.], &settings, "G54"),
                Ok(()),
                "{key}"
            );
        }
    }
    #[test]
    fn a_flipped_setup_gets_its_own_program_and_offset() {
        let (doc, mesh) = fixture::flipped_job();
        let job = post_job(&doc, &mesh);
        assert!(job.ready(), "{:?}", errors(&job));
        let names: Vec<_> = job
            .setups
            .iter()
            .map(|s| s.result.as_ref().unwrap()[0].file_name.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                "Fixture block - 1 Top (G54).ngc",
                "Fixture block - 2 Flipped (G55).ngc"
            ]
        );
        let second = &job.setups[1].result.as_ref().unwrap()[0].text;
        assert!(second.contains("\nG55\n") && !second.contains("\nG54\n"));
        assert!(second.contains("Part rotation X180"));
    }
    #[test]
    fn stock_datums_sit_on_a_whole_faced_top_but_not_on_a_partly_cut_one() {
        let (mut doc, mesh) = fixture::job();
        let ops = doc.setups[0].operations.clone();
        doc.setups[0].operations.truncate(1);
        let mut turned = Setup::new(Machine::new(3), "G55");
        turned.p.insert("name".into(), "Turned".into());
        turned.p.insert("rz".into(), "180".into());
        for op in &ops[..2] {
            let mut op = op.clone();
            op.id = id();
            turned.operations.push(op);
        }
        doc.setups.push(turned);
        doc.sync_stock_sources();
        // Setup 1 faces the whole top 4 mm deep: setup 2 starts from the
        // stock it left, whose top is the faced surface, so a stock-based
        // zero is on the stock.
        for origin in ["Stock top center", "Stock top front-left"] {
            doc.setups[1].p.insert("origin".into(), origin.into());
            let job = post_job(&doc, &mesh);
            assert!(job.ready(), "{origin}: {:?}", errors(&job));
            let text = &setup_texts(&job, 1)[0];
            assert!(
                text.contains("(Stock: 124.000 x 84.000 x 43.000 mm."),
                "{text}"
            );
        }
        // Faced only inside the part's top, the stock keeps a rim at full
        // height: the top face is cut where a stock-based zero is touched.
        let face = &mut doc.setups[0].operations[0];
        face.p.insert("boundary".into(), "Selected geometry".into());
        face.p
            .insert("containment".into(), "Tool inside boundary".into());
        for origin in ["Stock top center", "Stock top front-left"] {
            doc.setups[1].p.insert("origin".into(), origin.into());
            let job = post_job(&doc, &mesh);
            assert!(!job.ready());
            assert!(
                errors(&job)
                    .iter()
                    .any(|e| e.contains("top face") && e.contains("Model origin")),
                "{:?}",
                errors(&job)
            );
        }
        doc.setups[1]
            .p
            .insert("origin".into(), "Model origin".into());
        let job = post_job(&doc, &mesh);
        assert!(job.ready(), "{:?}", errors(&job));
    }
    #[test]
    fn each_coolant_choice_writes_its_output_and_m9_at_the_end() {
        let (mut doc, mesh) = fixture::job();
        doc.setups[0].operations.truncate(1);
        let mut codes = |coolant: &str, air: &str| -> Vec<String> {
            doc.setups[0].operations[0]
                .p
                .insert("coolant".into(), coolant.into());
            doc.setups[0]
                .machine
                .p
                .insert("post_air".into(), air.into());
            let job = post_job(&doc, &mesh);
            assert!(job.ready(), "{coolant} {air}: {:?}", errors(&job));
            let text = &job.setups[0].result.as_ref().unwrap()[0].text;
            text.lines()
                .filter(|l| ["M7", "M8", "M9"].contains(l))
                .map(String::from)
                .collect()
        };
        // Flood is M8, Mist M7, Air the machine's Air output, Off nothing;
        // what was switched on is switched off with M9 at the end.
        assert_eq!(codes("Flood", "M7"), ["M8", "M9"]);
        assert_eq!(codes("Mist", "M8"), ["M7", "M9"]);
        assert_eq!(codes("Air", "M7"), ["M7", "M9"]);
        assert_eq!(codes("Air", "M8"), ["M8", "M9"]);
        assert_eq!(codes("Off", "M7"), Vec::<String>::new());
    }
    #[test]
    fn a_stock_datum_is_refused_when_an_earlier_setup_cut_the_side_it_needs() {
        // Setup 1 cuts only an outside Contour on stock with a margin on one
        // side alone: the Contour removes that side face and leaves the top
        // and the other sides. Setup 2 has the same orientation.
        let job_with_margin = |side: &str| {
            let (mut doc, mesh) = fixture::job();
            let ops = doc.setups[0].operations.clone();
            let setup = &mut doc.setups[0];
            for key in ["stock_left", "stock_right", "stock_front", "stock_back"] {
                setup.p.insert(key.into(), "0".into());
            }
            setup.p.insert(side.into(), "2".into());
            setup.operations.retain(|o| o.strategy == Strategy::Contour);
            let mut later = Setup::new(Machine::new(3), "G55");
            later.p.insert("name".into(), "Later".into());
            let mut face = ops[0].clone();
            face.id = id();
            later.operations.push(face);
            doc.setups.push(later);
            doc.sync_stock_sources();
            (doc, mesh)
        };
        let setup_2 = |doc: &mut Document, mesh: &Mesh, origin: &str| {
            doc.setups[1].p.insert("origin".into(), origin.into());
            let job = post_job(doc, mesh);
            job.setups[1]
                .result
                .as_ref()
                .err()
                .cloned()
                .unwrap_or_default()
        };
        for (side, face, needed_by_front_left) in [
            ("stock_right", "+X side", false),
            ("stock_back", "+Y side", false),
            ("stock_left", "-X side", true),
            ("stock_front", "-Y side", true),
        ] {
            let (mut doc, mesh) = job_with_margin(side);
            let refused = setup_2(&mut doc, &mesh, "Stock top center");
            assert_eq!(
                refused,
                vec![format!(
                    "Later: earlier setups have cut the stock's {face}, so the Stock top center work zero is no longer on the stock. Choose Model origin for this setup."
                )],
                "{side}"
            );
            let front_left = setup_2(&mut doc, &mesh, "Stock top front-left");
            if needed_by_front_left {
                // The Contour also took the margin at the top's corner.
                assert_eq!(
                    front_left,
                    vec![format!(
                        "Later: earlier setups have cut the stock's top face near the front-left corner and {face}, so the Stock top front-left work zero is no longer on the stock. Choose Model origin for this setup."
                    )],
                    "{side}"
                );
            } else {
                assert!(front_left.is_empty(), "{side}: {front_left:?}");
            }
            assert!(
                setup_2(&mut doc, &mesh, "Model origin").is_empty(),
                "{side}"
            );
        }
    }
    /// The reviewer's job: setup 1 faces only inside the part's top face
    /// (a new Face's default boundary, "Selected geometry" with "Tool inside
    /// boundary"), which leaves a rim of stock at full height around a top
    /// 4 mm lower. Setup 2, "Later" on G55, has the same orientation, a
    /// Contour and work zero `origin`.
    fn inside_face_job(origin: &str) -> (Document, Mesh) {
        let (mut doc, mesh) = fixture::job();
        let ops = doc.setups[0].operations.clone();
        doc.setups[0].operations.truncate(1);
        let face = &mut doc.setups[0].operations[0];
        face.p.insert("boundary".into(), "Selected geometry".into());
        face.p
            .insert("containment".into(), "Tool inside boundary".into());
        let mut later = Setup::new(Machine::new(3), "G55");
        later.p.insert("name".into(), "Later".into());
        later.p.insert("origin".into(), origin.into());
        let mut contour = ops[2].clone();
        contour.id = id();
        later.operations.push(contour);
        doc.setups.push(later);
        doc.sync_stock_sources();
        (doc, mesh)
    }
    /// Each setup's simulations, as `post_job` runs them.
    fn simulated(doc: &Document, mesh: &Mesh) -> Vec<Vec<(usize, Result<Simulation, String>)>> {
        let mut results: Vec<Vec<(usize, Result<Simulation, String>)>> =
            vec![vec![]; doc.setups.len()];
        crate::simulation::sequence(doc, mesh, None, |s, o, r| results[s].push((o, r)));
        results
    }
    #[test]
    fn a_stock_datum_is_refused_when_an_earlier_setup_cut_its_touch_off_spot() {
        // The whole top face is not cut: its rim is still there. But the
        // stock-top centre, where Stock top center puts Z0, is 4 mm lower.
        let (doc, mesh) = inside_face_job("Stock top center");
        let results = simulated(&doc, &mesh);
        let first = results[1][0].1.as_ref().unwrap();
        let (volume, frame) = (first.initial_volume.as_ref().unwrap(), first.frame.unwrap());
        let stock = first.plan.stock;
        assert!(stock_faces_cut(stock, &crate::rest::cut_boxes(volume, frame), true).is_empty());
        let job = post_simulated(&doc, &mesh, &results);
        assert_eq!(
            job.setups[1].result.as_ref().err().cloned().unwrap_or_default(),
            vec![
                "Later: earlier setups have cut the stock's top face at its centre, so the Stock top center work zero is no longer on the stock. Choose Model origin for this setup.".to_string()
            ]
        );
    }
    #[test]
    fn stock_top_front_left_needs_an_uncut_corner_area() {
        // The reviewer's job leaves only a rim about 2.6 mm wide at the
        // corner: a Z-setter or gauge block placed a few millimetres in from
        // the corner would stand on the faced top, 4 mm lower.
        let (doc, mesh) = inside_face_job("Stock top front-left");
        let job = post_job(&doc, &mesh);
        assert_eq!(
            job.setups[1].result.as_ref().err().cloned().unwrap_or_default(),
            vec![
                "Later: earlier setups have cut the stock's top face near the front-left corner, so the Stock top front-left work zero is no longer on the stock. Choose Model origin for this setup.".to_string()
            ]
        );
        // With 12 mm of stock beyond the part on -X and -Y, the 10 mm corner
        // area lies wholly in the uncut margin.
        let (mut doc, mesh) = inside_face_job("Stock top front-left");
        for key in ["stock_left", "stock_front"] {
            doc.setups[0].p.insert(key.into(), "12".into());
        }
        doc.sync_stock_sources();
        let job = post_job(&doc, &mesh);
        assert!(job.ready(), "{:?}", errors(&job));
        // Stock top center still finds the top's centre cut.
        doc.setups[1]
            .p
            .insert("origin".into(), "Stock top center".into());
        assert!(
            errors(&post_job(&doc, &mesh))
                .iter()
                .any(|e| e.contains("top face at its centre")),
        );
    }
    /// The reviewer's two-sided job: setup 1 faces, pockets, contours down
    /// to `bottom` mm above the part's underside and drills; setup 2,
    /// "Underside" on G55, is flipped about X, faces the underside and has
    /// work zero `origin`. The stock has 2 mm side margins.
    fn two_sided_job(bottom: &str, origin: &str) -> (Document, Mesh) {
        let (mut doc, mesh) = fixture::flipped_after(4);
        let strategies: Vec<Strategy> = doc.setups[0]
            .operations
            .iter()
            .map(|o| o.strategy)
            .collect();
        assert_eq!(
            strategies,
            [
                Strategy::Face,
                Strategy::Pocket,
                Strategy::Contour,
                Strategy::Drill
            ]
        );
        doc.setups[0].operations[2]
            .p
            .insert("bottom".into(), bottom.into());
        let underside = &mut doc.setups[1];
        underside.p.insert("name".into(), "Underside".into());
        underside.p.insert("origin".into(), origin.into());
        doc.sync_stock_sources();
        (doc, mesh)
    }
    /// The reviewer's probe: `fixture::flipped_after(keep)`, whose setup 1
    /// contours the part's walls down to its underside and leaves the 3 mm
    /// of stock under it whole, with setup 2 held in the starter vise (opens
    /// to 120 mm, grip 8) on ±`axis`.
    fn flipped_in_vise(keep: usize, axis: &str) -> (Document, Mesh) {
        let (mut doc, mesh) = fixture::flipped_after(keep);
        doc.adopt_holders();
        let id = doc
            .workholding
            .iter()
            .find(|h| h.kind() == "Vise")
            .unwrap()
            .id
            .clone();
        for (k, v) in [
            ("holding", "Vise"),
            ("holder", id.as_str()),
            ("vise_axis", axis),
            ("vise_offset", "0"),
            ("vise_grip", "8"),
        ] {
            doc.setups[1].p.insert(k.into(), v.into());
        }
        (doc, mesh)
    }
    #[test]
    fn a_flipped_setup_in_a_vise_closes_on_the_walls_setup_1_contoured() {
        let near = |a: f32, lo: f32, hi: f32| a > lo && a < hi;
        for keep in [3, 4] {
            // The flipped stock keeps the 124 x 84 mm of the stock left under
            // the part, but the jaws' 8 mm band is what setup 1's Contour
            // went through: the walls, 120 x 80 mm and the 0.3 mm allowance
            // the Contour leaves on them (and the 0.01 mm the band keeps
            // short of a cut face), at any grip up to the part's height.
            let (mut doc, mesh) = flipped_in_vise(keep, "Y");
            let first = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap();
            let stock = first.plan.stock;
            let [x, y, _] = stock.dimensions();
            assert!(
                near(x, 123.99, 124.01) && near(y, 83.99, 84.01),
                "{stock:?}"
            );
            let bands = bands(&doc.setups[1], &mesh, &first).unwrap();
            let vise = doc.holder(get(&doc.setups[1].p, "holder")).unwrap();
            for (grip, axis_y) in [(8., true), (30., true), (8., false)] {
                let layout = crate::holding::Layout::Vise {
                    holder: vise.id.clone(),
                    axis_y,
                    offset: 0.,
                    grip,
                    gap: 2.,
                };
                let [x, y, z] = bands.at(stock, &layout, vise).dimensions();
                // Across the jaws, and along them too: the walls.
                assert!(
                    near(x, 120.69, 120.73) && near(y, 80.69, 80.73),
                    "{keep} {grip} {axis_y}: {x} {y}"
                );
                assert_eq!(z, stock.dimensions()[2]);
            }
            // On ±Y the vise holds: with a work zero on the model, both
            // setups export and the plan check passes but for the drill's
            // speed, which the fixture leaves low.
            doc.setups[1]
                .p
                .insert("origin".into(), "Model origin".into());
            let job = post_job(&doc, &mesh);
            assert!(job.ready(), "{keep}: {:?}", errors(&job));
            crate::proposal::tests::aluminium(&mut doc);
            let report = crate::analysis::report(&doc, &mesh).unwrap();
            let check = crate::plan_check::check_built(&doc, &mesh, &report);
            assert!(
                check.problems.iter().all(|p| p.stage == "cutting data"),
                "{keep}: {:?}",
                check.problems
            );
            // On ±X the walls are wider than the vise opens: refused for that
            // alone.
            let (mut doc, mesh) = flipped_in_vise(keep, "X");
            doc.setups[1]
                .p
                .insert("origin".into(), "Model origin".into());
            assert_eq!(
                errors(&post_job(&doc, &mesh)),
                ["Flipped: The stock is 120.71 mm across the jaws; this vise opens to 120 mm."]
            );
        }
    }
    #[test]
    fn side_touch_off_spots_lie_just_below_the_stock_top() {
        // Setup 1's Contour stops 10 or 18 mm above the part's underside, so
        // only the lower band of the flipped setup's sides is cut, in the
        // jaws. An edge finder touches the sides just below the top, where
        // they are intact, so both stock-based zeros export.
        for bottom in ["10", "18"] {
            for (origin, spots) in [
                (
                    "Stock top center",
                    "Z0 on the stock top at its centre and X0 Y0 from the four sides near their middles",
                ),
                (
                    "Stock top front-left",
                    "Z0 on the stock top within 10 mm of the front-left corner, at least 1 mm in from both edges, and X0 Y0 from the -X side between 4.2 mm and 12.6 mm and the -Y side between 6.2 mm and 18.6 mm from the front-left corner",
                ),
            ] {
                let (doc, mesh) = two_sided_job(bottom, origin);
                let job = post_job(&doc, &mesh);
                assert!(job.ready(), "{bottom} {origin}: {:?}", errors(&job));
                // The 124 x 84 x 47 mm stock: between 1 mm and 10 mm below the top.
                let text = &setup_texts(&job, 1)[0];
                assert_eq!(
                    comment_parts(text, "Work zero: ").join(" "),
                    format!(
                        "{origin} in G55. Touch off {spots}, between 1 mm and 10 mm below the top."
                    ),
                    "{text}"
                );
            }
        }
        // Stock with a 2 mm margin on +X only, whose Contour (bottom 30)
        // cuts the top 14 mm of the stock: in a same-orientation setup the
        // +X side is cut where an edge finder touches it, although its
        // lower part is intact.
        let (mut doc, mesh) = fixture::job();
        let ops = doc.setups[0].operations.clone();
        let setup = &mut doc.setups[0];
        for key in ["stock_left", "stock_front", "stock_back"] {
            setup.p.insert(key.into(), "0".into());
        }
        setup.operations.retain(|o| o.strategy == Strategy::Contour);
        setup.operations[0].p.insert("bottom".into(), "30".into());
        let mut later = Setup::new(Machine::new(3), "G55");
        later.p.insert("name".into(), "Later".into());
        let mut face = ops[0].clone();
        face.id = id();
        later.operations.push(face);
        doc.setups.push(later);
        doc.sync_stock_sources();
        let results = simulated(&doc, &mesh);
        let first = results[1][0].1.as_ref().unwrap();
        let (volume, frame) = (first.initial_volume.as_ref().unwrap(), first.frame.unwrap());
        assert!(
            stock_faces_cut(
                first.plan.stock,
                &crate::rest::cut_boxes(volume, frame),
                true
            )
            .is_empty()
        );
        assert_eq!(
            post_simulated(&doc, &mesh, &results).setups[1]
                .result
                .as_ref()
                .err()
                .cloned()
                .unwrap_or_default(),
            vec![
                "Later: earlier setups have cut the stock's +X side near its centre, so the Stock top center work zero is no longer on the stock. Choose Model origin for this setup.".to_string()
            ]
        );
        // Stock top front-left does not use the +X side.
        doc.setups[1]
            .p
            .insert("origin".into(), "Stock top front-left".into());
        let job = post_job(&doc, &mesh);
        assert!(job.ready(), "{:?}", errors(&job));
    }
    /// The reviewer's small cube job: examples/preview-cube.obj (30 x 20 x
    /// 10) on 34 x 24 x 14 stock (2 mm side margins, 1 mm on top, 3 mm
    /// below). Setup 1 faces and contours with the flat 10 mm down to
    /// `bottom` mm above the part's underside; setup 2, "Underside" on G55,
    /// is flipped about X, faces and has work zero `origin`.
    fn cube_job(bottom: &str, origin: &str) -> (Document, Mesh) {
        let (mut doc, _) = fixture::job();
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/preview-cube.obj");
        let mesh = crate::mesh::load(&path).unwrap();
        // The largest surface at the top of the part as `setup` places it.
        let top = |setup: &Setup| {
            let placement = Placement::new(&mesh, Some(setup));
            let top = mesh
                .surfaces
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    (placement.transform(s.center)[2] - placement.bounds.max[2]).abs() < 0.01
                })
                .max_by(|(_, a), (_, b)| a.area.total_cmp(&b.area))
                .unwrap()
                .0;
            Some(crate::surface::Selection {
                mesh_fingerprint: mesh.fingerprint.clone(),
                ids: vec![top],
            })
        };
        let setup = &mut doc.setups[0];
        setup.p.insert("stock_top".into(), "1".into());
        setup.p.insert("stock_bottom".into(), "3".into());
        setup
            .operations
            .retain(|o| matches!(o.strategy, Strategy::Face | Strategy::Contour));
        setup.operations[0].surfaces = top(setup);
        setup.operations[1].p.insert("bottom".into(), bottom.into());
        let mut underside = Setup::new(Machine::new(3), "G55");
        underside.p.insert("name".into(), "Underside".into());
        underside.p.insert("rx".into(), "180".into());
        underside.p.insert("origin".into(), origin.into());
        let mut face = doc.setups[0].operations[0].clone();
        face.id = id();
        face.surfaces = top(&underside);
        underside.operations.push(face);
        doc.setups.push(underside);
        doc.file = Some(ModelFile {
            name: "preview-cube.obj".into(),
            path: path.to_string_lossy().into_owned(),
            format: "obj".into(),
            bytes: 0,
        });
        doc.sync_stock_sources();
        (doc, mesh)
    }
    #[test]
    fn a_side_cut_only_below_the_touch_off_band_keeps_a_stock_zero() {
        // An earlier setup's cutter boxes reach a tool radius below the
        // cut, so with a large cutter or a short stock they cover a whole
        // side although the cut itself stops below where an edge finder
        // touches it. Only the touch-off spots decide.
        let origins = ["Stock top center", "Stock top front-left"];
        // The reviewer's two-sided job with setup 1's Contour on a flat
        // 25 mm cutter, 8 mm above the part's underside: setup 2's first
        // side cut is 11 mm below its top, below the 10 mm band.
        for origin in origins {
            let (mut doc, mesh) = two_sided_job("8", origin);
            let mut big = Tool::new("Flat 25", "Flat end mill", "25");
            big.p.insert("length".into(), "60".into());
            big.p.insert("stickout".into(), "65".into());
            big.p.insert("number".into(), "4".into());
            doc.setups[0].operations[2].tool = big.id.clone();
            doc.tools.push(big);
            let job = post_job(&doc, &mesh);
            assert!(job.ready(), "D25 {origin}: {:?}", errors(&job));
        }
        // The small cube job: the Contour stops 5 mm below setup 2's top,
        // below its 3.2 mm band (a quarter of the 13 mm stock setup 1 left
        // after facing 1 mm off its top, now setup 2's bottom).
        for origin in origins {
            let (doc, mesh) = cube_job("2", origin);
            let job = post_job(&doc, &mesh);
            assert!(job.ready(), "cube {origin}: {:?}", errors(&job));
            let text = &setup_texts(&job, 1)[0];
            assert!(
                text.contains("(Stock: 34.000 x 24.000 x 13.000 mm."),
                "{text}"
            );
            assert!(
                comment_parts(text, "Work zero: ")
                    .join(" ")
                    .ends_with(" between 1 mm and 3.2 mm below the top."),
                "{text}"
            );
        }
    }
    #[test]
    fn touch_off_spots_follow_the_work_zero() {
        use crate::{simulation::CutterShape, volume::Volume};
        let (doc, mesh) = fixture::job();
        let frame = crate::volume::Frame::new(&doc.setups[0], &mesh);
        let stock = Bounds {
            min: [-50., -30., 0.],
            max: [50., 30., 30.],
        };
        // What Stock top center and Stock top front-left find cut after
        // the flat 6 mm cuts `path`, each (from, to) with a 5 mm flute.
        let after = |path: &[([f32; 3], [f32; 3])]| {
            let mut volume = Volume::new(stock, frame);
            for &(a, b) in path {
                volume.cut(a, b, 3., 5., CutterShape::Flat, frame, (0, 0));
            }
            [true, false].map(|center| stock_zero_cut(stock, &volume, frame, center))
        };
        let none = Vec::<String>::new();
        let one = |face: &str| vec![face.to_string()];
        // The sides are touched in a band from 1 mm to 7.5 mm (a quarter of
        // the 30 mm height) below the top: Z22.5 to Z29.
        assert_eq!(side_band(stock), 7.5);
        assert_eq!(corner_area(stock), 10.);
        // A 0.5 mm deep cut at the front-left corner of the top, above the
        // side band.
        assert_eq!(
            after(&[([-47., -27., 29.5], [-46., -27., 29.5])]),
            [none.clone(), one("top face near the front-left corner")]
        );
        // One 5 to 11 mm in from both edges, inside the corner's 10 mm area.
        assert_eq!(
            after(&[([-42., -22., 29.5], [-41., -22., 29.5])]),
            [none.clone(), one("top face near the front-left corner")]
        );
        // A cut into the +X side near its centre, from Z24 up: in the band.
        assert_eq!(
            after(&[([52., -1., 24.], [52., 1., 24.])]),
            [one("+X side near its centre"), none.clone()]
        );
        // The same cut from Z17 to Z22 lies below the band, where the
        // workholding holds the stock: no zero touches it there.
        assert_eq!(
            after(&[([52., -1., 17.], [52., 1., 17.])]),
            [none.clone(), none.clone()]
        );
        // A cut into the -Y side in the band, a little way from the corner.
        assert_eq!(
            after(&[([-44., -32., 24.], [-43., -32., 24.])]),
            [none.clone(), one("-Y side near the front-left corner")]
        );
        // A short cut in the middle of the top.
        assert_eq!(
            after(&[([-1., 0., 28.], [1., 0., 28.])]),
            [one("top face at its centre"), none.clone()]
        );
        // The whole top faced 0.5 mm deep: the face is named as before, with
        // no spot.
        let rows: Vec<_> = (0..17)
            .map(|k| {
                let y = -32. + 4. * k as f32;
                ([-55., y, 29.5], [55., y, 29.5])
            })
            .collect();
        assert_eq!(after(&rows), [one("top face"), one("top face")]);
        // A contour arc around the corner, just outside the stock, leaves
        // the corner intact although its boxes reach into it.
        let arc: Vec<_> = (0..10)
            .map(|k| {
                let at = |k: f32| {
                    let t = std::f32::consts::PI * (1. + k / 20.);
                    [-50. + 3.05 * t.cos(), -30. + 3.05 * t.sin(), -5.]
                };
                (at(k as f32), at(k as f32 + 1.))
            })
            .collect();
        let mut volume = Volume::new(stock, frame);
        for &(a, b) in &arc {
            volume.cut(a, b, 3., 40., CutterShape::Flat, frame, (0, 0));
        }
        let corner = [stock.min[0] + 1., stock.min[1] + 0.6, stock.max[2] - 0.01];
        assert!(
            crate::rest::removal_boxes(&volume, frame)
                .iter()
                .any(|b| in_box(corner, b))
        );
        assert_eq!(after(&arc), [none.clone(), none]);
    }
    #[test]
    fn a_contour_beside_stock_without_side_margin_exports() {
        let (mut doc, mesh) = fixture::job();
        let setup = &mut doc.setups[0];
        // Facing 8 mm takes the approach below the stock top; with no front margin
        // the contour's start column lies wholly in front of the stock box.
        setup.p.insert("stock_front".into(), "0".into());
        setup.p.insert("stock_top".into(), "8".into());
        setup
            .operations
            .retain(|o| matches!(o.strategy, Strategy::Face | Strategy::Contour));
        assert_eq!(setup.operations.len(), 2);
        doc.sync_stock_sources();
        let job = post_job(&doc, &mesh);
        assert!(job.ready(), "{:?}", errors(&job));
    }
    #[test]
    fn stock_datums_are_refused_when_the_stock_is_not_square_to_the_setup() {
        let (mut doc, mesh) = fixture::job();
        let ops = doc.setups[0].operations.clone();
        doc.setups[0].operations.truncate(1);
        let mut turned = Setup::new(Machine::new(3), "G55");
        turned.p.insert("name".into(), "Turned".into());
        turned.p.insert("rz".into(), "30".into());
        for op in &ops[..2] {
            let mut op = op.clone();
            op.id = id();
            turned.operations.push(op);
        }
        doc.setups.push(turned);
        doc.sync_stock_sources();
        for origin in ["Stock top center", "Stock top front-left"] {
            doc.setups[1].p.insert("origin".into(), origin.into());
            let job = post_job(&doc, &mesh);
            assert!(!job.ready());
            assert!(
                errors(&job)
                    .iter()
                    .any(|e| e.contains("not square to this setup") && e.contains("Model origin")),
                "{:?}",
                errors(&job)
            );
        }
        doc.setups[1]
            .p
            .insert("origin".into(), "Model origin".into());
        let job = post_job(&doc, &mesh);
        assert!(
            !errors(&job).iter().any(|e| e.contains("not square")),
            "{:?}",
            errors(&job)
        );
    }
    #[test]
    fn touch_off_mode_writes_one_program_per_tool_run() {
        let (mut doc, mesh) = fixture::job();
        doc.setups[0]
            .machine
            .p
            .insert("post_tool_length".into(), POST_TOOL_LENGTH[1].into());
        let job = post_job(&doc, &mesh);
        assert!(job.ready(), "{:?}", errors(&job));
        let programs = job.setups[0].result.as_ref().unwrap();
        let names: Vec<_> = programs.iter().map(|p| p.file_name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Fixture block - 1 Top (G54) - 1 of 4 T1.ngc",
                "Fixture block - 1 Top (G54) - 2 of 4 T3.ngc",
                "Fixture block - 1 Top (G54) - 3 of 4 T2.ngc",
                "Fixture block - 1 Top (G54) - 4 of 4 T1.ngc",
            ]
        );
        assert_eq!(programs[0].operations.len(), 3);
        for p in programs {
            assert!(
                !p.text.contains("M6")
                    && !p.text.contains("G43")
                    && p.text.contains("use AXIS Touch Off")
                    && p.text.contains("Run G49 in MDI before touching off")
            );
        }
        assert!(
            programs[1]
                .text
                .contains("(Program 2 of 4 for this setup, one per tool)")
        );
        assert!(
            programs[0]
                .text
                .contains("note the Z of a fixed reference surface")
        );
        assert!(
            programs[1]
                .text
                .contains("(Before starting: load T3 Drill 4, then use AXIS Touch Off")
        );
        assert!(programs[1].text.contains("Do not touch off on the stock"));
        assert!(
            programs[1]
                .text
                .contains("X0 Y0 are unchanged from program 1")
        );
    }
    #[test]
    fn unsafe_or_unsupported_jobs_are_refused_with_reasons() {
        let refused = |edit: &dyn Fn(&mut Document), needle: &str| {
            let (mut doc, mesh) = fixture::job();
            edit(&mut doc);
            let job = post_job(&doc, &mesh);
            assert!(!job.ready());
            assert!(
                errors(&job).iter().any(|e| e.contains(needle)),
                "{needle}: {:?}",
                errors(&job)
            );
        };
        refused(
            &|d| {
                d.setups[0]
                    .machine
                    .p
                    .insert("controller".into(), "GRBL".into());
            },
            "Only the LinuxCNC",
        );
        refused(&|d| d.setups[0].machine = Machine::new(4), "3-axis");
        refused(
            &|d| {
                d.setups[0].machine.p.insert("x".into(), "50".into());
            },
            "travel",
        );
        refused(
            &|d| {
                d.tools[1].p.insert("number".into(), "1".into());
            },
            "also used",
        );
        refused(
            &|d| {
                d.setups[0]
                    .p
                    .insert("origin".into(), "Selected point (planned)".into());
            },
            "Selected-point",
        );
        refused(
            &|d| {
                d.setups[0].p.insert("offset".into(), "G92".into());
            },
            "G54-G59.3",
        );
        refused(
            &|d| {
                d.setups[0]
                    .machine
                    .p
                    .insert("post_air".into(), "Not available".into());
                d.setups[0].operations[0]
                    .p
                    .insert("coolant".into(), "Air".into());
            },
            "coolant",
        );
        refused(
            &|d| d.setups[0].operations[1].surfaces = None,
            "Cannot inherit",
        );
        refused(
            &|d| {
                d.setups[0]
                    .operations
                    .iter_mut()
                    .for_each(|o| o.enabled = false)
            },
            "no enabled operations",
        );
        let sample = post_job(&Document::sample(), &crate::mesh::sample());
        assert!(!sample.ready() && sample.errors[0].contains("Import a part"));
    }
    #[test]
    fn write_job_writes_every_program_or_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (doc, mesh) = fixture::flipped_job();
        let (folder, count) = write_job(&post_job(&doc, &mesh), dir.path()).unwrap();
        assert_eq!((folder.as_path(), count), (dir.path(), 2));
        let written =
            std::fs::read_to_string(dir.path().join("Fixture block - 2 Flipped (G55).ngc"))
                .unwrap();
        assert!(written.ends_with("M2\n"));
        let empty = tempfile::tempdir().unwrap();
        let (mut bad, mesh) = fixture::flipped_job();
        bad.setups[1]
            .p
            .insert("origin".into(), "Selected point (planned)".into());
        assert!(
            write_job(&post_job(&bad, &mesh), empty.path())
                .unwrap_err()
                .contains("Nothing was written")
        );
        assert_eq!(std::fs::read_dir(empty.path()).unwrap().count(), 0);
    }
    #[test]
    fn different_stock_boxes_are_refused_across_touch_off_programs() {
        let (mut doc, mesh) = fixture::job();
        doc.setups[0]
            .machine
            .p
            .insert("post_tool_length".into(), POST_TOOL_LENGTH[1].into());
        let mut results: Vec<Vec<(usize, Result<crate::simulation::Simulation, String>)>> =
            vec![vec![]; doc.setups.len()];
        crate::simulation::sequence(&doc, &mesh, None, |s, o, r| results[s].push((o, r)));
        assert!(prepare(&doc, &mesh, 0, &results[0]).is_ok());
        // Drilling is alone in program 2 of 4, so only a setup-wide check can see this.
        results[0][3].1.as_mut().unwrap().plan.stock.min[0] -= 1.;
        let problems = prepare(&doc, &mesh, 0, &results[0]).err().unwrap();
        assert!(
            problems.iter().any(|p| p.contains("different stock boxes")),
            "{problems:?}"
        );
    }
    #[test]
    fn verify_checks_length_offset_at_retracts_and_spindle_at_dwells() {
        let run = |m: Machine, edit: &dyn Fn(&str) -> String| {
            let (text, blocks, settings) = program(m);
            let ops = vec![fixture::facing_op()];
            assert_ne!(edit(&text), text);
            (
                crate::ngc::verify(&text, &blocks, &ops, [0., 0., 10.], &settings, "G54"),
                crate::ngc::verify(&edit(&text), &blocks, &ops, [0., 0., 10.], &settings, "G54"),
            )
        };
        let clearance = || {
            let mut m = Machine::new(3);
            m.p.insert("post_retract".into(), POST_RETRACT[1].into());
            m
        };
        let touch_off = || {
            let mut m = Machine::new(3);
            m.p.insert("post_tool_length".into(), POST_TOOL_LENGTH[1].into());
            m
        };
        for (machine, edit) in [
            (
                clearance(),
                &(|t: &str| t.replace("G43 H1\nG0 Z15.000\n", "G0 Z15.000\nG43 H1\n"))
                    as &dyn Fn(&str) -> String,
            ),
            (clearance(), &|t: &str| {
                t.replacen("G54\n", "G54\nG0 Z15.000\n", 1)
            }),
            (Machine::new(3), &|t: &str| {
                t.replace("S8000 M3\nM8\nG4 P2.000\n", "G4 P2.000\nS8000 M3\nM8\n")
            }),
            (touch_off(), &|t: &str| {
                t.replacen("G40 G49 G80", "G40 G80", 1)
            }),
            (touch_off(), &|t: &str| t.replacen("G54\n", "G54\nG43\n", 1)),
        ] {
            let (clean, edited) = run(machine, edit);
            assert_eq!(clean, Ok(()));
            assert!(edited.is_err());
        }
    }
    #[test]
    fn a_blend_that_prints_as_g64_p0_fails_verification() {
        let (text, blocks, mut settings) = program(Machine::new(3));
        settings.blend = 0.0004;
        let text = text.replace("G64 P0.010", &format!("G64 P{:.3}", settings.blend));
        assert!(text.contains("\nG64 P0.000\n"));
        let ops = vec![fixture::facing_op()];
        let result = crate::ngc::verify(&text, &blocks, &ops, [0., 0., 10.], &settings, "G54");
        assert!(result.unwrap_err().contains("blending"));
    }
    #[test]
    fn headless_export_matches_the_ui_path() {
        let dir = tempfile::tempdir().unwrap();
        let (doc, _) = fixture::job();
        let project = dir.path().join("job.axis.json");
        crate::storage::save(&project, &doc).unwrap();
        let out = dir.path().join("out");
        let message = crate::export_gcode(
            Some(&project.to_string_lossy().into_owned()),
            Some(&out.to_string_lossy().into_owned()),
        )
        .unwrap();
        assert!(message.starts_with("Wrote 1 LinuxCNC program"));
        assert!(!message.contains("LinuxCNC tool table"), "{message}");
        assert!(!message.contains("material"), "{message}");
        assert!(out.join("Fixture block - 1 Top (G54).ngc").exists());
    }
    #[test]
    fn headless_export_announces_tools_numbered_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let (mut doc, _) = fixture::job();
        for t in &mut doc.tools {
            t.p.remove("number");
        }
        doc.material = "Brass C360".into();
        let project = dir.path().join("job.axis.json");
        crate::storage::save(&project, &doc).unwrap();
        let out = dir.path().join("out");
        let message = crate::export_gcode(
            Some(&project.to_string_lossy().into_owned()),
            Some(&out.to_string_lossy().into_owned()),
        )
        .unwrap();
        assert!(message.starts_with("Wrote 1 LinuxCNC program"), "{message}");
        // The notes of the load follow the outcome, in the order of the load.
        let notes = format!(
            "{} The project material \"Brass C360\" was linked to the library.",
            crate::storage::NUMBERED_TOOLS
        );
        assert!(message.ends_with(&format!("\n{notes}")), "{message}");
        // A refused export carries the note too.
        doc.setups[0].p.insert("offset".into(), "G99".into());
        crate::storage::save(&project, &doc).unwrap();
        let error = crate::export_gcode(
            Some(&project.to_string_lossy().into_owned()),
            Some(&out.to_string_lossy().into_owned()),
        )
        .unwrap_err();
        assert!(error.starts_with("Nothing was written."), "{error}");
        assert!(error.ends_with(&format!("\n{notes}")), "{error}");
    }
    #[test]
    fn posting_simulations_already_run_matches_post_job() {
        let (doc, mesh) = fixture::job();
        let direct = post_job(&doc, &mesh);
        let mut results: Vec<Vec<(usize, Result<Simulation, String>)>> =
            vec![vec![]; doc.setups.len()];
        crate::simulation::sequence(&doc, &mesh, None, |si, oi, r| results[si].push((oi, r)));
        let split = post_simulated(&doc, &mesh, &results);
        let texts = |job: &JobPost| -> Vec<Result<Vec<String>, Vec<String>>> {
            job.setups
                .iter()
                .map(|s| {
                    s.result
                        .as_ref()
                        .map(|p| p.iter().map(|p| p.text.clone()).collect())
                        .map_err(Clone::clone)
                })
                .collect()
        };
        assert!(direct.ready());
        assert_eq!(split.ready(), direct.ready());
        assert_eq!(split.errors, direct.errors);
        assert_eq!(texts(&split), texts(&direct));
        assert!(texts(&direct)[0].as_ref().unwrap()[0].contains("G54"));
    }
    /// The good two-setup plan, built and posted; setup 2 uses Model origin.
    fn good_plan_job() -> (Document, Mesh) {
        use crate::proposal::tests::{block, good};
        let (doc, mesh, report) = block();
        let job = crate::proposal::build(&doc, &mesh, &report, &good(&report)).unwrap();
        (job, mesh)
    }
    fn setup_texts(job: &JobPost, si: usize) -> Vec<String> {
        match &job.setups[si].result {
            Ok(programs) => programs.iter().map(|p| p.text.clone()).collect(),
            Err(e) => panic!("{e:?}"),
        }
    }
    /// The text of the comment that begins with `lead`, such as "Work
    /// zero: ", then of each "<lead word>, continued: " line after it,
    /// without the leads and parentheses: joined with spaces, the sentence
    /// the post wrote. Empty when no comment begins with `lead`.
    fn comment_parts<'a>(text: &'a str, lead: &str) -> Vec<&'a str> {
        let continued = format!("({}, continued: ", lead.trim_end_matches(": "));
        text.lines()
            .skip_while(|l| !l.starts_with(&format!("({lead}")))
            .enumerate()
            .take_while(|(k, l)| *k == 0 || l.starts_with(&continued))
            .map(|(k, l)| {
                let l = l.strip_suffix(')').unwrap();
                if k == 0 {
                    &l[lead.len() + 1..]
                } else {
                    &l[continued.len()..]
                }
            })
            .collect()
    }
    #[test]
    fn planned_setups_carry_setup_1s_fixture_and_their_workholding_note() {
        use crate::proposal::tests::{block, good};
        let (mut doc, mesh, report) = block();
        doc.setups[0].p.insert("fixture".into(), "Vise".into());
        let mut job = crate::proposal::build(&doc, &mesh, &report, &good(&report)).unwrap();
        assert_eq!(get(&job.setups[1].p, "fixture"), "Vise");
        let posted = post_job(&job, &mesh);
        let text = &setup_texts(&posted, 1)[0];
        assert!(
            text.contains(
                "(Fixture: Vise)\n(Setup notes: Flip about X; clamp on the machined walls.)\n"
            ),
            "{text}"
        );
        // Setup 1 has no notes and no modelled workholding: neither line.
        let first = &setup_texts(&posted, 0)[0];
        assert!(!first.contains("Workholding") && !first.contains("Setup notes"));
        // Long notes are cleaned and broken between words, never cut off.
        let words = "Clamp (soft jaws)\non the walls; keep ".repeat(12) + "the end.";
        job.setups[1].p.insert("notes".into(), words);
        let text = &setup_texts(&post_job(&job, &mesh), 1)[0];
        let lines: Vec<&str> = text
            .lines()
            .skip_while(|l| !l.starts_with("(Setup notes: "))
            .take_while(|l| !l.starts_with("(T1:"))
            .collect();
        assert!(lines.len() > 2, "{text}");
        assert!(lines.iter().all(|l| l.len() <= 202), "{lines:?}");
        assert!(
            lines.last().unwrap().ends_with("keep the end.)"),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .all(|l| l[1..l.len() - 1].chars().all(|c| c != '(' && c != ')'))
        );
        assert!(lines[0].starts_with("(Setup notes: Clamp soft jaws on the walls; keep Clamp"));
        // Every later line names what it continues, so that none begins with
        // the note's own text.
        assert!(
            lines[1..]
                .iter()
                .all(|l| l.starts_with("(Setup notes, continued: ")),
            "{lines:?}"
        );
        // A word longer than a line is split over lines, not cut off.
        let long = "x".repeat(450);
        job.setups[1]
            .p
            .insert("notes".into(), format!("Use {long} here."));
        let text = &setup_texts(&post_job(&job, &mesh), 1)[0];
        let joined: String = text
            .lines()
            .filter_map(|l| {
                l.strip_prefix("(Setup notes: ")
                    .or_else(|| l.strip_prefix("(Setup notes, continued: "))
            })
            .map(|l| l.strip_suffix(')').unwrap().replace(' ', ""))
            .collect();
        assert_eq!(joined, format!("Use{long}here."));
        assert!(text.lines().all(|l| l.len() <= 202), "{text}");
    }
    /// The fixture job reduced to its Pocket (its path lies within X -39 to
    /// 23 and Y -15 to 15 of the stock's centre), with the starter library.
    fn pocket_job() -> (Document, Mesh) {
        let (mut doc, mesh) = fixture::job();
        doc.adopt_holders();
        doc.setups[0].operations.retain(|o| o.name() == "Pocket");
        (doc, mesh)
    }
    /// Setup 1 held by the starter vise or clamp, with `pairs` on the setup.
    fn hold(doc: &mut Document, kind: &str, pairs: &[(&str, &str)]) {
        let id = doc
            .workholding
            .iter()
            .find(|h| h.kind() == kind)
            .unwrap()
            .id
            .clone();
        let p = &mut doc.setups[0].p;
        p.insert("holder".into(), id);
        p.insert(
            "holding".into(),
            if kind == "Vise" { "Vise" } else { "Clamps" }.into(),
        );
        for (k, v) in pairs {
            p.insert((*k).into(), (*v).into());
        }
    }
    #[test]
    fn workholding_over_the_toolpath_refuses_the_setup_and_clear_workholding_exports() {
        use crate::holding;
        let (mut doc, mesh) = pocket_job();
        // No workholding: not checked, and an entry that is gone does not matter.
        assert!(post_job(&doc, &mesh).ready());
        doc.setups[0].p.insert("holder".into(), "gone".into());
        assert!(post_job(&doc, &mesh).ready());
        // A toe clamp on -X, 12 mm onto the stock, is clear of the pocket.
        hold(&mut doc, "Clamp", &[("clamps", "-X:0")]);
        assert!(post_job(&doc, &mesh).ready());
        // Reaching 40 mm onto the stock it is in the way: the setup is refused.
        let id = get(&doc.setups[0].p, "holder").to_string();
        doc.workholding
            .iter_mut()
            .find(|h| h.id == id)
            .unwrap()
            .p
            .insert("bite".into(), "40".into());
        let posted = post_job(&doc, &mesh);
        assert!(!posted.ready());
        let mut plans = vec![];
        crate::simulation::sequence(&doc, &mesh, None, |_, _, r| plans.push(r.unwrap().plan));
        let boxes = doc.holding_boxes(&doc.setups[0], plans[0].stock).unwrap();
        let hit = &holding::hits(&plans[0], &boxes, 2.)[0];
        let datum = datum(&doc.setups[0], plans[0].stock, &mesh).unwrap();
        let expected = holding::message("Pocket", hit, 2., datum);
        assert_eq!(errors(&posted), [expected.as_str()]);
        // The clamp's inner face is X-22 in work coordinates; the rapids pass
        // it 15 mm above the stock top.
        assert!(
            expected.starts_with("Pocket: the tool hits the clamp on -X (offset 0) at X-22.0 Y")
                && expected.ends_with(" Z15.0 (within the 2 mm gap)."),
            "{expected}"
        );
        // Moved along the side, clear of the pocket, it exports again.
        doc.setups[0].p.insert("clamps".into(), "-X:42".into());
        assert!(post_job(&doc, &mesh).ready());
    }
    #[test]
    fn workholding_that_cannot_be_modelled_refuses_the_setup_with_the_editors_line() {
        use crate::holding::DELETED;
        let (mut doc, mesh) = pocket_job();
        let refused = |doc: &Document| {
            let posted = post_job(doc, &mesh);
            assert!(!posted.ready());
            errors(&posted)
        };
        // A deleted entry, none chosen, a clamp text that does not read.
        hold(&mut doc, "Clamp", &[("clamps", "-X:0"), ("holder", "gone")]);
        assert_eq!(refused(&doc), [format!("Top: {DELETED}")]);
        assert_eq!(doc.holding_issues(0, Some(&mesh))[0], DELETED);
        doc.setups[0].p.insert("holder".into(), "".into());
        assert_eq!(
            refused(&doc),
            ["Top: Choose a clamp from the library.".to_string()]
        );
        hold(&mut doc, "Clamp", &[("clamps", "-X")]);
        assert_eq!(
            refused(&doc),
            ["Top: Write each clamp as side:offset, for example -X:0.".to_string()]
        );
        // The vise opens to 120 mm: the stock is 124 mm across X, 84 across Y.
        hold(&mut doc, "Vise", &[("vise_axis", "X"), ("vise_grip", "8")]);
        assert_eq!(
            refused(&doc),
            ["Top: The stock is 124 mm across the jaws; this vise opens to 120 mm.".to_string()]
        );
        assert_eq!(
            doc.holding_issues(0, Some(&mesh)),
            ["The stock is 124 mm across the jaws; this vise opens to 120 mm."]
        );
        doc.setups[0].p.insert("vise_axis".into(), "Y".into());
        assert!(post_job(&doc, &mesh).ready());
    }
    #[test]
    fn a_later_setup_not_square_to_setup_1_is_refused_for_its_workholding_at_export() {
        use crate::holding::NOT_SQUARE;
        let (mut doc, mesh) = pocket_job();
        hold(&mut doc, "Clamp", &[("clamps", "-X:0")]);
        let mut later = doc.setups[0].duplicate("G55");
        later.p.insert("name".into(), "Turned".into());
        later.p.insert("origin".into(), "Model origin".into());
        doc.setups.push(later);
        // A quarter or half turn keeps the jaws and clamps on the stock's faces.
        for rz in ["90", "180", "270"] {
            doc.setups[1].p.insert("rz".into(), rz.into());
            let posted = post_job(&doc, &mesh);
            assert!(posted.ready(), "rz {rz}: {:?}", errors(&posted));
        }
        // Another angle: the real vise or clamps would turn with the stock.
        doc.setups[1].p.insert("rz".into(), "30".into());
        let posted = post_job(&doc, &mesh);
        assert!(!posted.ready());
        assert_eq!(errors(&posted), [format!("Turned: {NOT_SQUARE}")]);
        assert_eq!(doc.holding_issues(1, Some(&mesh)), [NOT_SQUARE]);
        // Choosing None for that setup exports it again.
        doc.setups[1].p.insert("holding".into(), "None".into());
        assert!(post_job(&doc, &mesh).ready());
    }
    #[test]
    fn headless_export_refuses_a_clamp_hit_with_the_export_message() {
        use crate::holding;
        let dir = tempfile::tempdir().unwrap();
        let (mut doc, mesh) = pocket_job();
        // Reaching 40 mm onto the stock the clamp is in the pocket's way.
        hold(&mut doc, "Clamp", &[("clamps", "-X:0")]);
        let id = get(&doc.setups[0].p, "holder").to_string();
        doc.workholding
            .iter_mut()
            .find(|h| h.id == id)
            .unwrap()
            .p
            .insert("bite".into(), "40".into());
        let mut plans = vec![];
        crate::simulation::sequence(&doc, &mesh, None, |_, _, r| plans.push(r.unwrap().plan));
        let boxes = doc.holding_boxes(&doc.setups[0], plans[0].stock).unwrap();
        let hit = &holding::hits(&plans[0], &boxes, 2.)[0];
        let datum = datum(&doc.setups[0], plans[0].stock, &mesh).unwrap();
        let expected = holding::message("Pocket", hit, 2., datum);
        let project = dir.path().join("job.axis.json");
        crate::storage::save(&project, &doc).unwrap();
        let out = dir.path().join("out");
        let error = crate::export_gcode(
            Some(&project.to_string_lossy().into_owned()),
            Some(&out.to_string_lossy().into_owned()),
        )
        .unwrap_err();
        assert_eq!(error, format!("Nothing was written. Top: {expected}"));
        assert!(!out.exists(), "nothing was written");
        // With the toe clamp's own bite it is clear of the pocket: it exports.
        doc.workholding
            .iter_mut()
            .find(|h| h.id == id)
            .unwrap()
            .p
            .insert("bite".into(), "12".into());
        crate::storage::save(&project, &doc).unwrap();
        let message = crate::export_gcode(
            Some(&project.to_string_lossy().into_owned()),
            Some(&out.to_string_lossy().into_owned()),
        )
        .unwrap();
        assert!(message.starts_with("Wrote 1 LinuxCNC program"), "{message}");
    }
    /// The `sequence` results of every setup of `doc`.
    fn results(doc: &Document, mesh: &Mesh) -> Vec<Vec<(usize, Result<Simulation, String>)>> {
        let mut results = vec![vec![]; doc.setups.len()];
        crate::simulation::sequence(doc, mesh, None, |si, oi, r| results[si].push((oi, r)));
        results
    }
    #[test]
    fn held_finds_the_problems_prepare_refuses_a_setup_with_and_the_operations_they_name() {
        use crate::holding::DELETED;
        let (mut doc, mesh) = fixture::job();
        doc.adopt_holders();
        // Only the Pocket is enabled; it is the second operation of the setup.
        for (k, op) in doc.setups[0].operations.iter_mut().enumerate() {
            op.enabled = k == 1;
        }
        assert!(held(&doc, &mesh, 0, &results(&doc, &mesh)[0]).is_empty());
        // A clamp 40 mm onto the stock is in the pocket's way.
        hold(&mut doc, "Clamp", &[("clamps", "-X:0")]);
        let id = get(&doc.setups[0].p, "holder").to_string();
        let entry = doc.workholding.iter_mut().find(|h| h.id == id).unwrap();
        entry.p.insert("bite".into(), "40".into());
        let found = held(&doc, &mesh, 0, &results(&doc, &mesh)[0]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].operation, Some(1));
        assert!(
            found[0]
                .message
                .starts_with("Pocket: the tool hits the clamp on -X (offset 0) at X-22.0")
        );
        // These are the messages export refuses the setup with.
        assert_eq!(errors(&post_job(&doc, &mesh)), [found[0].message.as_str()]);
        // A setup that cannot be modelled: the one line, for no operation.
        doc.setups[0].p.insert("holder".into(), "gone".into());
        let found = held(&doc, &mesh, 0, &results(&doc, &mesh)[0]);
        assert_eq!(
            found,
            [Held {
                operation: None,
                message: format!("Top: {DELETED}")
            }]
        );
        assert_eq!(errors(&post_job(&doc, &mesh)), [found[0].message.as_str()]);
        // A setup export cannot walk has nothing to hold against.
        assert!(held(&doc, &mesh, 0, &[]).is_empty());
    }
    /// `fixture::job`'s Facing, Pocket, Contour and Drilling in setup 1, held
    /// by nothing, then setup "Again" the same way up, drilling again, held
    /// by the starter vise on ±Y at grip 8.
    fn vise_after_the_contour() -> (Document, Mesh) {
        let (mut doc, mesh) = fixture::job();
        doc.setups[0].operations.truncate(4);
        held_again(&mut doc, 3);
        (doc, mesh)
    }
    /// `doc` with setup "Again" added, the same way up as setup 1, repeating
    /// its operation `op`, held by the starter vise on ±Y at grip 8.
    fn held_again(doc: &mut Document, op: usize) {
        doc.adopt_holders();
        let mut again = Setup::new(Machine::new(3), "G55");
        again.p.insert("name".into(), "Again".into());
        again.p.insert("origin".into(), "Model origin".into());
        let mut repeated = doc.setups[0].operations[op].clone();
        repeated.id = id();
        again.operations.push(repeated);
        let vise = doc
            .workholding
            .iter()
            .find(|h| h.kind() == "Vise")
            .unwrap()
            .id
            .clone();
        for (k, v) in [
            ("holding", "Vise"),
            ("holder", vise.as_str()),
            ("vise_axis", "Y"),
            ("vise_grip", "8"),
            ("holding_gap", "2"),
        ] {
            again.p.insert(k.into(), v.into());
        }
        doc.setups.push(again);
        doc.sync_stock_sources();
    }
    #[test]
    fn a_vise_gripping_where_an_earlier_contour_went_down_is_refused_naming_the_contour() {
        let (mut doc, mesh) = vise_after_the_contour();
        // The Contour went from the stock top down to the part's bottom, 3 mm
        // above the stock's: of the 8 mm the jaws grip, 5 are air.
        let said = "Again: Setup 2 cannot be held in the vise: setup 1, operation 3 removed the material the jaws grip on ±Y from 0 to 8 mm above the stock's bottom. Leave at least 8 mm there (for example with `bottom_offset`) and finish it in a later setup, or choose another holder. Setup 1, operation 3 is Contour \"Contour\".";
        let sims = results(&doc, &mesh);
        assert_eq!(
            held(&doc, &mesh, 1, &sims[1]),
            [Held {
                operation: None,
                message: said.into()
            }]
        );
        assert_eq!(
            prepare(&doc, &mesh, 1, &sims[1]).err(),
            Some(vec![said.into()])
        );
        assert_eq!(errors(&post_job(&doc, &mesh)), [said]);
        // Stopped 6 mm higher it leaves the band whole: the job exports.
        doc.setups[0].operations[2]
            .p
            .insert("bottom".into(), "6".into());
        let sims = results(&doc, &mesh);
        assert!(held(&doc, &mesh, 1, &sims[1]).is_empty());
        let posted = post_job(&doc, &mesh);
        assert!(posted.ready(), "{:?}", errors(&posted));
    }
    #[test]
    fn a_vise_on_a_frame_an_earlier_contour_cut_free_from_the_part_is_refused() {
        let (mut doc, mesh) = fixture::job();
        // Facing, Pocket and the Contour, with 15 mm of stock round the part
        // and none under it: the Contour, one tool wide and down to the
        // part's bottom, leaves a frame cut free from the part.
        doc.setups[0].operations.truncate(3);
        for (k, v) in [
            ("stock_left", "15"),
            ("stock_right", "15"),
            ("stock_front", "15"),
            ("stock_back", "15"),
            ("stock_bottom", "0"),
        ] {
            doc.setups[0].p.insert(k.into(), v.into());
        }
        held_again(&mut doc, 1);
        let said = "Again: Setup 2 cannot be held in the vise: setup 1, operation 3 cut the part free from the material the jaws grip on ±Y. Leave material joining them (for example a `bottom_offset` above the stock's bottom, or a skin under the part) and cut it free in the last setup. Setup 1, operation 3 is Contour \"Contour\".";
        let sims = results(&doc, &mesh);
        assert_eq!(
            held(&doc, &mesh, 1, &sims[1]),
            [Held {
                operation: None,
                message: said.into()
            }]
        );
        assert_eq!(errors(&post_job(&doc, &mesh)), [said]);
        // 10 mm of stock under the part joins the frame to it, and the jaws
        // grip 8 mm of that: the job exports.
        doc.setups[0].p.insert("stock_bottom".into(), "10".into());
        let sims = results(&doc, &mesh);
        assert!(held(&doc, &mesh, 1, &sims[1]).is_empty());
        let posted = post_job(&doc, &mesh);
        assert!(posted.ready(), "{:?}", errors(&posted));
    }
    #[test]
    fn a_touch_off_setup_reports_the_clearance_plane_once_not_once_per_program() {
        let (mut doc, mesh) = fixture::job();
        doc.adopt_holders();
        // The Pocket and the Drill use two tools: two programs in touch-off
        // mode, each starting at the clearance plane (Z15) under the clamps.
        for op in &mut doc.setups[0].operations {
            op.enabled = ["Pocket", "Drilling"].contains(&op.name());
        }
        hold(&mut doc, "Clamp", &[("clamps", "-X:0;+X:0")]);
        let machine = &mut doc.setups[0].machine.p;
        machine.insert("post_retract".into(), POST_RETRACT[1].into());
        machine.insert("post_tool_length".into(), POST_TOOL_LENGTH[1].into());
        let sims = results(&doc, &mesh);
        let programs = walk(&doc, &mesh, 0, &sims[0], PostSettings::from_machine)
            .unwrap()
            .groups
            .len();
        assert_eq!(programs, 2);
        let found = held(&doc, &mesh, 0, &sims[0]);
        let plane: Vec<_> = found
            .iter()
            .filter(|h| h.message.starts_with("The setup clearance plane (Z15.0)"))
            .collect();
        assert_eq!(plane.len(), 1, "{found:?}");
        assert_eq!(plane[0].operation, None);
        // Nothing is said twice, and the order is the programs' order.
        for (i, h) in found.iter().enumerate() {
            assert!(!found[..i].contains(h), "{h:?}");
        }
        // Where the programs differ their problems differ, and all stay, in
        // the programs' order: the Pocket's plane at Z20, the Drill's at Z15.
        let pocket = doc.setups[0]
            .operations
            .iter_mut()
            .find(|o| o.name() == "Pocket")
            .unwrap();
        pocket.p.insert("clearance".into(), "20".into());
        let sims = results(&doc, &mesh);
        let found = held(&doc, &mesh, 0, &sims[0]);
        let planes: Vec<_> = found.iter().filter(|h| h.operation.is_none()).collect();
        assert_eq!(planes.len(), 2, "{found:?}");
        assert!(
            planes[0]
                .message
                .starts_with("The setup clearance plane (Z20.0)")
        );
        assert!(
            planes[1]
                .message
                .starts_with("The setup clearance plane (Z15.0)")
        );
    }
    #[test]
    fn at_the_clearance_plane_clamps_above_the_plane_refuse_the_setup() {
        let (mut doc, mesh) = pocket_job();
        // Toe clamps on -X and +X, clear of the pocket's path (Z0-25 in work
        // coordinates); the program's first rapid starts from wherever the
        // tool was after the tool change, at the clearance plane (Z15).
        hold(&mut doc, "Clamp", &[("clamps", "-X:0;+X:0")]);
        assert!(post_job(&doc, &mesh).ready());
        doc.setups[0]
            .machine
            .p
            .insert("post_retract".into(), POST_RETRACT[1].into());
        let refused = |plane: &str, limit: &str| {
            format!(
                "The setup clearance plane (Z{plane}) is not above the clamp on -X (offset 0) and its 2 mm gap (Z{limit}). Raise the operations' clearance above {limit} mm over the stock top (Z{limit}), or retract to machine home."
            )
        };
        let posted = post_job(&doc, &mesh);
        assert!(!posted.ready());
        assert_eq!(errors(&posted), [refused("15.0", "27.0")]);
        // Touch-off mode: every program starts the same way.
        let mut touch = doc.clone();
        touch.setups[0]
            .machine
            .p
            .insert("post_tool_length".into(), POST_TOOL_LENGTH[1].into());
        assert_eq!(errors(&post_job(&touch, &mesh)), [refused("15.0", "27.0")]);
        // A plane under the clamp's top and gap is refused, one above it exports.
        doc.setups[0].operations[0]
            .p
            .insert("clearance".into(), "26.5".into());
        assert_eq!(errors(&post_job(&doc, &mesh)), [refused("26.5", "27.0")]);
        doc.setups[0].operations[0]
            .p
            .insert("clearance".into(), "30".into());
        let posted = post_job(&doc, &mesh);
        assert!(posted.ready(), "{:?}", errors(&posted));
        // Retracting to the machine's Z home needs no plane.
        doc.setups[0]
            .machine
            .p
            .insert("post_retract".into(), POST_RETRACT[0].into());
        doc.setups[0].operations[0]
            .p
            .insert("clearance".into(), "15".into());
        assert!(post_job(&doc, &mesh).ready());
    }
    #[test]
    fn the_header_and_the_setup_sheet_say_what_holds_the_stock() {
        let (mut doc, mesh) = pocket_job();
        let header = |doc: &Document| setup_texts(&post_job(doc, &mesh), 0).remove(0);
        // Without workholding neither names any.
        assert!(!header(&doc).contains("Workholding"));
        assert!(!doc.setup_sheet(Some(&mesh)).contains("Workholding"));
        hold(&mut doc, "Vise", &[("vise_axis", "Y"), ("vise_grip", "8")]);
        let text = header(&doc);
        assert!(
            text.contains("(Fixture: Low profile clamps)\n(Workholding: 100 mm machine vise on +-Y, grip 8 mm, offset 0 mm, gap 2 mm.)\n(T1: "),
            "{text}"
        );
        assert!(
            doc.setup_sheet(Some(&mesh)).contains("Fixture: Low profile clamps\n\nWorkholding: 100 mm machine vise on ±Y, grip 8 mm, offset 0 mm, gap 2 mm.\n\nRotation:")
        );
        // Four clamps, and the setup's own notes after them under their own lead word. A library name
        // with active-comment words and brackets is written as text.
        hold(
            &mut doc,
            "Clamp",
            &[("clamps", "-X:0;+X:0;-Y:0;+Y:0"), ("notes", "Soft jaws")],
        );
        let id = get(&doc.setups[0].p, "holder").to_string();
        doc.workholding
            .iter_mut()
            .find(|h| h.id == id)
            .unwrap()
            .p
            .insert("name".into(), "Toe (MSG,hello) clamp Ø60".into());
        let text = header(&doc);
        assert!(
            text.contains("(Fixture: Low profile clamps)\n(Workholding: 4 x Toe MSG,hello clamp D60 at -X 0, +X 0, -Y 0, +Y 0, gap 2 mm.)\n(Setup notes: Soft jaws)\n(T1: "),
            "{text}"
        );
        assert_comments_begin_with_app_words(&text);
        assert!(
            doc.setup_sheet(Some(&mesh)).contains("Workholding: 4 × Toe (MSG,hello) clamp Ø60 at -X 0, +X 0, -Y 0, +Y 0, gap 2 mm.\n\nRotation:")
        );
        // The comment text keeps ± readable.
        assert_eq!(comment("on ±Y"), "on +-Y");
    }
    #[test]
    fn the_header_says_what_was_checked_of_the_workholding() {
        let (mut doc, mesh) = pocket_job();
        let header = |doc: &Document| setup_texts(&post_job(doc, &mesh), 0).remove(0);
        let old = "(Not checked: fixtures, clamps and the machine body. LinuxCNC soft limits check absolute position. Preview in LinuxCNC and air-cut first.)\n";
        let held = "(Not checked: parallels, bolts, the table, the machine body and tool-change motion. Vise jaws and clamps were checked as boxes with a clearance gap.)\n(Retracts to machine home are assumed to clear everything.)\n(LinuxCNC soft limits check absolute position. Preview in LinuxCNC and air-cut first.)\n";
        // Without modelled workholding the old note stands, whatever the
        // setup's fixture description says.
        let text = header(&doc);
        assert!(text.contains(old) && !text.contains(held), "{text}");
        doc.setups[0].p.insert("fixture".into(), "Vise".into());
        assert!(header(&doc).contains(old));
        // A vise or clamps: the jaws and clamps were checked, the rest was not.
        for (kind, pairs) in [
            ("Vise", vec![("vise_axis", "Y"), ("vise_grip", "8")]),
            ("Clamp", vec![("clamps", "-X:0;+X:0")]),
        ] {
            hold(&mut doc, kind, &pairs);
            let text = header(&doc);
            assert!(text.contains(held) && !text.contains(old), "{kind}: {text}");
            assert_comments_begin_with_app_words(&text);
            // Each comment line is short, as the others are: none comes near
            // LinuxCNC's limit of 255 characters.
            assert!(text.lines().all(|l| l.len() <= 202), "{kind}: {text}");
        }
        // Back to None: the old note again.
        doc.setups[0].p.insert("holding".into(), "None".into());
        assert!(header(&doc).contains(old));
    }
    #[test]
    fn a_model_origin_header_locates_the_zero_from_the_part_and_the_stock_top() {
        let (job, mesh) = good_plan_job();
        assert_eq!(get(&job.setups[1].p, "origin"), "Model origin");
        let text = &setup_texts(&post_job(&job, &mesh), 1)[0];
        // The part is 120 x 80 x 40 mm with its file origin at the centre of
        // its underside, which faces up in setup 2 under 3 mm of stock.
        assert!(
            text.contains(
                "(Work zero: Model origin in G55.)\n(Part in work coordinates: X-60.000 to 60.000 Y-40.000 to 40.000 Z-40.000 to 0.000 mm. Stock top Z3.000.)\n(Touch off on a part wall or the stock top and enter its coordinate.)\n"
            ),
            "{text}"
        );
        assert!(!text.contains("Touch off X0 Y0 Z0"), "{text}");
        // Setup 1's stock-based zero says where to touch off.
        let first = &setup_texts(&post_job(&job, &mesh), 0)[0];
        assert!(
            first.contains("(Work zero: Stock top center in G54. Touch off Z0 on the stock top at its centre and X0 Y0 from the four sides near their middles, between 1 mm and 10 mm below the top.)\n"),
            "{first}"
        );
        assert!(!first.contains("Part in work coordinates"));
        // A file origin 300 mm from the part puts the part's box far from X0.
        let mut far = mesh.clone();
        far.origin[0] += 300. * far.display_scale();
        let text = &setup_texts(&post_job(&job, &far), 1)[0];
        assert!(
            text.contains("(Part in work coordinates: X-360.000 to -240.000 Y-40.000 to 40.000 Z-40.000 to 0.000 mm. Stock top Z3.000.)"),
            "{text}"
        );
        // Touch-off mode: program 1 says where to touch off.
        let mut touch = job.clone();
        touch.setups[1]
            .machine
            .p
            .insert("post_tool_length".into(), POST_TOOL_LENGTH[1].into());
        let text = &setup_texts(&post_job(&touch, &mesh), 1)[0];
        assert!(
            text.contains("(Before starting: load T1 Flat 10, then use AXIS Touch Off, not Tool Touch Off, on a part wall or the stock top and enter its coordinate from the lines above.)"),
            "{text}"
        );
        assert!(!text.contains("set X0 Y0 Z0 at the work zero"), "{text}");
    }
    /// How every comment the post writes begins; `#` stands for a number the
    /// app writes. A comment that carries text from the user or the model
    /// begins with a fixed word and a colon; the others are fixed sentences.
    const COMMENT_STARTS: [&str; 37] = [
        "AXIS CAM Studio - LinuxCNC 3-axis program",
        "Job: ",
        "Job, continued: ",
        "Program # of # for this setup, one per tool",
        "Machine: ",
        "Machine, continued: ",
        "Work zero: ",
        "Work zero, continued: ",
        "Part in work coordinates: ",
        "Touch off on a part wall or the stock top and enter its coordinate.",
        STOCK_TOP_CUT,
        "Stock: ",
        "Stock, continued: ",
        "Fixture: ",
        "Fixture, continued: ",
        "Workholding: ",
        "Workholding, continued: ",
        "Setup notes: ",
        "Setup notes, continued: ",
        "T#: ",
        "T#, continued: ",
        "Operation #: ",
        "Operation #, continued: ",
        "Operation #/#: ",
        "Operation #/#, continued: ",
        "Suppressed: ",
        "Suppressed, continued: ",
        "Extents: ",
        "Not checked: ",
        MACHINE_HOME,
        SOFT_LIMITS,
        "Start with the spindle raised clear of the part and fixtures: ",
        "Before starting: ",
        "Before starting, continued: ",
        "Before removing this tool, note the Z of a fixed reference surface such as a vise jaw.",
        "Do not touch off on the stock: earlier programs have cut it.",
        "Run G49 in MDI before touching off, and jog the tool clear of the part before Cycle Start.",
    ];
    fn starts_with_template(text: &str, template: &str) -> bool {
        let mut rest = text;
        for c in template.chars() {
            if c == '#' {
                let digits =
                    rest.len() - rest.trim_start_matches(|d: char| d.is_ascii_digit()).len();
                if digits == 0 {
                    return false;
                }
                rest = &rest[digits..];
            } else if let Some(r) = rest.strip_prefix(c) {
                rest = r;
            } else {
                return false;
            }
        }
        true
    }
    /// Every comment is a whole line, and its text, after leading spaces
    /// (which LinuxCNC skips), begins with one of `COMMENT_STARTS`. Returns
    /// how many comments use each start.
    fn assert_comments_begin_with_app_words(text: &str) -> [usize; COMMENT_STARTS.len()] {
        let mut used = [0; COMMENT_STARTS.len()];
        for line in text.lines().filter(|l| l.contains(['(', ')'])) {
            let inner = line
                .strip_prefix('(')
                .and_then(|l| l.strip_suffix(')'))
                .filter(|l| !l.contains(['(', ')']))
                .unwrap_or_else(|| panic!("not one whole comment: {line}"));
            let inner = inner.trim_start();
            let start = COMMENT_STARTS
                .iter()
                .position(|t| starts_with_template(inner, t))
                .unwrap_or_else(|| panic!("a comment begins with other text: {line}"));
            used[start] += 1;
        }
        used
    }
    /// `n` characters of words, then `inserted` and more words.
    fn text_with(n: usize, inserted: &str) -> String {
        let pad: String = "ab cde f ".chars().cycle().take(n).collect();
        format!("{pad}{inserted} tail words")
    }
    const ACTIVE: [&str; 3] = ["LOGOPEN,/tmp/x.log", "MSG,hello", "PROBEOPEN,/tmp/p.txt"];
    #[test]
    fn no_comment_begins_with_text_from_the_user_or_the_model() {
        // The golden plan, emitted with user text that holds LinuxCNC's
        // active-comment words at every offset from 0 to 400 characters (the
        // notes and workholding lines break near 187 and 363), and at offsets around
        // those breaks in every other header the post writes: touch-off
        // program 1 and a later one, clearance retracts, with and without
        // Model origin, with and without the stock top.
        let plan = golden_plan();
        let mut used = [0; COMMENT_STARTS.len()];
        let every: Vec<usize> = (0..=400).collect();
        let around: Vec<usize> = [
            0, 1, 2, 185, 186, 187, 188, 189, 200, 361, 362, 363, 364, 365,
        ]
        .into();
        for (key, value) in [
            ("post_tool_length", POST_TOOL_LENGTH[0]),
            ("post_tool_length", POST_TOOL_LENGTH[1]),
            ("post_retract", POST_RETRACT[1]),
        ] {
            let mut machine = Machine::new(3);
            machine.p.insert(key.into(), value.into());
            let settings = PostSettings::from_machine(&machine).unwrap();
            for word in ACTIVE {
                for &n in if value == POST_TOOL_LENGTH[0] {
                    &every
                } else {
                    &around
                } {
                    let user = text_with(n, word);
                    let mut setup = Setup::new(machine.clone(), "G54");
                    for field in ["name", "notes", "fixture", "rx"] {
                        setup.p.insert(field.into(), user.clone());
                    }
                    setup.machine.p.insert("name".into(), user.clone());
                    let ops = vec![OpState {
                        name: user.clone(),
                        tool_name: user.clone(),
                        ..fixture::facing_op()
                    }];
                    let blocks = blocks(&ops, &[&plan], &settings);
                    let headers = [
                        (None, "Stock top center", Some(3.)),
                        (Some((1, 2)), "Model origin", Some(3.)),
                        (Some((2, 2)), "Model origin", None),
                        (None, "Model origin", None),
                        (Some((1, 2)), "Stock top front-left", Some(3.)),
                    ];
                    let sweep = value == POST_TOOL_LENGTH[0] && !around.contains(&n);
                    for &(part, origin, top) in &headers[..if sweep { 1 } else { headers.len() }] {
                        setup.p.insert("origin".into(), origin.into());
                        let header = Header {
                            job: &user,
                            setup: &setup,
                            number: 1,
                            part,
                            stock: plan.stock,
                            located: Some((plan.stock, top)),
                            datum: [0., 0., 10.],
                            holding: Some(user.clone()),
                            disabled: vec![user.clone()],
                        };
                        let text = emit(&header, &ops, &blocks, &settings);
                        let counts = assert_comments_begin_with_app_words(&text);
                        (0..COMMENT_STARTS.len()).for_each(|k| used[k] += counts[k]);
                        assert_eq!(
                            crate::ngc::verify(
                                &text,
                                &blocks,
                                &ops,
                                [0., 0., 10.],
                                &settings,
                                "G54"
                            ),
                            Ok(()),
                            "{text}"
                        );
                    }
                }
            }
        }
        // Each kind of comment was written and checked.
        for (k, start) in COMMENT_STARTS.iter().enumerate() {
            assert!(used[k] > 0, "never written: {start}");
        }
        // A 200-character tool name: the touch-off instruction and the tool
        // line run on over more lines, and nothing is cut off.
        let name: String = "Flat end mill 10 mm "
            .chars()
            .cycle()
            .take(199)
            .chain(['x'])
            .collect();
        assert_eq!(name.len(), 200);
        let mut machine = Machine::new(3);
        machine
            .p
            .insert("post_tool_length".into(), POST_TOOL_LENGTH[1].into());
        let settings = PostSettings::from_machine(&machine).unwrap();
        let setup = Setup::new(machine, "G54");
        let ops = vec![OpState {
            tool_name: name.clone(),
            ..fixture::facing_op()
        }];
        let blocks = blocks(&ops, &[&plan], &settings);
        // The text of the comments that begin with `lead`, then go on after
        // "<lead word>, continued: ", read back as one sentence.
        let whole = |text: &str, lead: &str| -> String {
            let parts = comment_parts(text, lead);
            assert!(parts.len() > 1, "{lead} fits on one line:\n{text}");
            parts.join(" ")
        };
        for (part, instruction) in [
            (
                (1, 2),
                "then use AXIS Touch Off, not Tool Touch Off, to touch off Z0 on the stock top at its centre and X0 Y0 from the four sides near their middles, between 1 mm and 2.5 mm below the top.",
            ),
            (
                (2, 2),
                "then use AXIS Touch Off, not Tool Touch Off, on the fixed reference surface noted after program 1 and enter its noted Z.",
            ),
        ] {
            let header = Header {
                job: "Long names",
                setup: &setup,
                number: 1,
                part: Some(part),
                stock: plan.stock,
                located: None,
                datum: [0., 0., 10.],
                holding: None,
                disabled: vec![],
            };
            let text = emit(&header, &ops, &blocks, &settings);
            assert_comments_begin_with_app_words(&text);
            assert!(text.lines().all(|l| l.len() <= 202), "{text}");
            assert_eq!(
                whole(&text, "Before starting: "),
                clean(&format!("load T1 {name}, {instruction}")),
                "{text}"
            );
            assert_eq!(
                whole(&text, "T1: "),
                clean(&format!("{name} - diameter 10.000 mm - stickout 35.000 mm")),
                "{text}"
            );
        }
        // A whole job: the good plan with the three words in the title, setup
        // 2's name, notes and fixture, an operation's name, a tool's name and
        // the machine's name, posted and verified in both tool modes. The
        // text changes no move, so each mode is simulated once.
        let (mut job, mesh) = good_plan_job();
        for mode in POST_TOOL_LENGTH {
            job.setups[1]
                .machine
                .p
                .insert("post_tool_length".into(), mode.into());
            let mut results: Vec<Vec<(usize, Result<Simulation, String>)>> =
                vec![vec![]; job.setups.len()];
            crate::simulation::sequence(&job, &mesh, None, |s, o, r| results[s].push((o, r)));
            for n in [0, 160] {
                let user = text_with(n, &ACTIVE.join(" "));
                let mut named = job.clone();
                named.title = user.clone();
                named.tools[0].p.insert("name".into(), user.clone());
                let setup = &mut named.setups[1];
                for field in ["name", "notes", "fixture"] {
                    setup.p.insert(field.into(), user.clone());
                }
                setup.operations[0].p.insert("name".into(), user.clone());
                setup.machine.p.insert("name".into(), user.clone());
                let posted = post_simulated(&named, &mesh, &results);
                assert!(posted.ready(), "{:?}", errors(&posted));
                for text in setup_texts(&posted, 1) {
                    assert!(ACTIVE.iter().all(|w| text.contains(w)), "{text}");
                    assert_comments_begin_with_app_words(&text);
                }
            }
        }
    }
    #[test]
    fn a_model_origin_header_gives_no_stock_top_an_earlier_setup_has_cut() {
        use crate::proposal::tests::{block, side_up};
        const CUT: &str =
            "(The stock top was cut by an earlier setup; touch off on a part face.)\n";
        // Setup 1's Contour removes the 2 mm side margins, so a side-up later
        // setup has no stock top left: the highest material is the part wall.
        let (doc, mesh, report) = block();
        let job = crate::proposal::build(&doc, &mesh, &report, &side_up(&report, "-Y")).unwrap();
        let text = &setup_texts(&post_job(&job, &mesh), 1)[0];
        assert!(
            text.contains(&format!(
                "(Work zero: Model origin in G55.)\n(Part in work coordinates: X-60.000 to 60.000 Y0.000 to 40.000 Z-40.000 to 40.000 mm.)\n{CUT}"
            )),
            "{text}"
        );
        assert!(
            !text.contains("Stock top") && !text.contains("or the stock top"),
            "{text}"
        );
        // Touch-off mode: program 1 names a part face only.
        let mut touch = job.clone();
        touch.setups[1]
            .machine
            .p
            .insert("post_tool_length".into(), POST_TOOL_LENGTH[1].into());
        let text = &setup_texts(&post_job(&touch, &mesh), 1)[0];
        assert!(
            text.contains("then use AXIS Touch Off, not Tool Touch Off, on a part face and enter its coordinate from the lines above.)"),
            "{text}"
        );
        assert!(
            !text.contains("Stock top") && !text.contains("or the stock top"),
            "{text}"
        );
        // A user's job: setup 1 faces the whole stock top, and setup 2 has
        // the same orientation.
        let (mut doc, mesh) = fixture::job();
        let ops = doc.setups[0].operations.clone();
        doc.setups[0].operations.truncate(1);
        let mut later = Setup::new(Machine::new(3), "G55");
        later.p.insert("name".into(), "Later".into());
        later.p.insert("origin".into(), "Model origin".into());
        let mut contour = ops[2].clone();
        contour.id = id();
        later.operations.push(contour);
        doc.setups.push(later);
        doc.sync_stock_sources();
        let text = &setup_texts(&post_job(&doc, &mesh), 1)[0];
        assert!(
            text.contains(&format!("Z0.000 to 40.000 mm.)\n{CUT}")),
            "{text}"
        );
        assert!(!text.contains("Stock top"), "{text}");
        // Setup 1 faced only inside the part's top face and left a rim of
        // stock at full height: the top is cut only in part, which only the
        // removal test sees.
        let (doc, mesh) = inside_face_job("Model origin");
        let results = simulated(&doc, &mesh);
        let first = results[1][0].1.as_ref().unwrap();
        let (volume, frame) = (first.initial_volume.as_ref().unwrap(), first.frame.unwrap());
        let whole = stock_faces_cut(
            first.plan.stock,
            &crate::rest::cut_boxes(volume, frame),
            false,
        );
        assert!(!whole.contains(&"top face"), "{whole:?}");
        let text = &setup_texts(&post_simulated(&doc, &mesh, &results), 1)[0];
        assert!(text.contains(&format!("mm.)\n{CUT}")), "{text}");
        assert!(!text.contains("Stock top"), "{text}");
        assert!(stock_top_cut(first.plan.stock, volume, frame));
        // The good plan's flipped setup: setup 1 left the stock's underside,
        // now its top, in place.
        let (job, mesh) = good_plan_job();
        let text = &setup_texts(&post_job(&job, &mesh), 1)[0];
        assert!(text.contains(" Stock top Z3.000.)\n"), "{text}");
        assert!(!text.contains(CUT), "{text}");
    }
}
