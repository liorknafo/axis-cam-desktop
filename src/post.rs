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
        if get(&m.p, "controller") != "LinuxCNC" {
            e.push(format!(
                "{} uses the {} controller. Only the LinuxCNC postprocessor is implemented.",
                m.name(),
                get(&m.p, "controller")
            ));
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
        let value = |k: &str| post_setting(&m.p, k).trim().parse::<f32>().unwrap_or(0.);
        Ok(Self {
            tool_table: post_setting(&m.p, "post_tool_length") == POST_TOOL_LENGTH[0],
            machine_home: post_setting(&m.p, "post_retract") == POST_RETRACT[0],
            blend: value("post_blend"),
            dwell: value("post_dwell"),
            air: match post_setting(&m.p, "post_air") {
                "M7" => Some((true, false)),
                "M8" => Some((false, true)),
                _ => None,
            },
        })
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
pub fn coord(v: f32) -> String {
    let s = format!("{v:.3}");
    if s == "-0.000" { "0.000".into() } else { s }
}
/// Comment text limited to printable ASCII on one line, without parentheses.
pub fn comment(s: &str) -> String {
    let mapped: String = s
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
    mapped
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(200)
        .collect()
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
        if change {
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
    pub datum: [f32; 3],
    pub disabled: Vec<String>,
}
struct Writer(String);
impl Writer {
    fn line(&mut self, s: impl AsRef<str>) {
        self.0.push_str(s.as_ref());
        self.0.push('\n');
    }
    fn note(&mut self, s: &str) {
        self.line(format!("({})", comment(s)));
    }
}
pub fn emit(h: &Header, ops: &[OpState], blocks: &[Block], settings: &PostSettings) -> String {
    let (d, setup) = (h.datum, h.setup);
    let offset = get(&setup.p, "offset");
    let mut w = Writer(String::new());
    w.line("(AXIS CAM Studio - LinuxCNC 3-axis program)");
    w.note(&format!(
        "Job: {} - Setup {}: {} - {offset}",
        h.job,
        h.number,
        setup.name()
    ));
    if let Some((i, count)) = h.part {
        w.note(&format!(
            "Program {i} of {count} for this setup, one per tool"
        ));
    }
    w.note(&format!(
        "Machine: {} v{}",
        setup.machine.name(),
        setup.machine.version
    ));
    w.note(&format!(
        "Work zero: {}. Touch off X0 Y0 Z0 there.",
        get(&setup.p, "origin")
    ));
    let size = h.stock.dimensions();
    w.note(&format!(
        "Stock: {} x {} x {} mm. Part rotation X{} Y{} Z{} deg.",
        coord(size[0]),
        coord(size[1]),
        coord(size[2]),
        get(&setup.p, "rx"),
        get(&setup.p, "ry"),
        get(&setup.p, "rz")
    ));
    w.note(&format!("Fixture: {}", get(&setup.p, "fixture")));
    let mut listed = vec![];
    for op in ops {
        if !listed.contains(&op.tool) {
            listed.push(op.tool);
            w.note(&format!(
                "T{}: {} - diameter {} mm - stickout {} mm",
                op.tool,
                op.tool_name,
                coord(op.diameter),
                coord(op.stickout)
            ));
        }
    }
    for (k, op) in ops.iter().enumerate() {
        w.note(&format!(
            "Operation {}: {} - {} - T{} - S{:.0}",
            k + 1,
            op.name,
            op.strategy,
            op.tool,
            op.rpm
        ));
    }
    for name in &h.disabled {
        w.note(&format!("Suppressed: {name}"));
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
    w.line("(Not checked: fixtures, clamps and the machine body. LinuxCNC soft limits check absolute position. Preview in LinuxCNC and air-cut first.)");
    if !settings.tool_table {
        // AXIS cannot touch off while a program is paused; G49 below would cancel a Tool Touch Off.
        w.note(&format!(
            "Load T{} {} and use AXIS Touch Off, not Tool Touch Off, to set Z0 at the work zero before starting this program.",
            ops[0].tool, ops[0].tool_name
        ));
    }
    w.line("G17 G21 G40 G49 G80 G90 G94");
    w.line(if settings.blend > 0. {
        format!("G64 P{:.3}", settings.blend)
    } else {
        "G61".into()
    });
    w.line(offset);
    if settings.tool_table && !settings.machine_home {
        // The first clearance retract must position the loaded tool's tip, not the gauge line.
        w.line("G43");
    }
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
                w.note(&format!(
                    "Operation {}/{}: {} - T{}",
                    op + 1,
                    ops.len(),
                    s.name,
                    s.tool
                ));
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
                    w.line(format!(
                        "S{:.0} {}",
                        s.rpm,
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
fn op_state(
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
pub(crate) struct Prepared {
    pub file_name: String,
    pub text: String,
    pub blocks: Vec<Block>,
    pub ops: Vec<OpState>,
    pub datum: [f32; 3],
    pub settings: PostSettings,
    pub offset: String,
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
    let offset = get(&setup.p, "offset").to_string();
    let mut errors = vec![];
    let settings = PostSettings::from_machine(&setup.machine)
        .map_err(|e| errors.extend(e))
        .ok();
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
    let count = groups.len();
    let mut out = vec![];
    for (i, g) in groups.into_iter().enumerate() {
        let group_ops = &ops[g.clone()];
        let group_sims = &simulations[g];
        let plans: Vec<&Plan> = group_sims.iter().map(|s| &s.plan).collect();
        let blocks = blocks(group_ops, &plans, &settings);
        let problems =
            crate::post_check::check_setup(setup, group_ops, group_sims, &blocks, &settings, zero);
        if !problems.is_empty() {
            return Err(problems);
        }
        let part = (!settings.tool_table).then(|| (i + 1, count, group_ops[0].tool));
        let header = Header {
            job: &doc.title,
            setup,
            number: si + 1,
            part: part.map(|(i, count, _)| (i, count)),
            stock: plans[0].stock,
            datum: zero,
            disabled: setup
                .operations
                .iter()
                .filter(|o| !o.enabled)
                .map(|o| o.name().to_string())
                .collect(),
        };
        let text = emit(&header, group_ops, &blocks, &settings);
        out.push(Prepared {
            file_name: file_name(doc, si, part),
            text,
            blocks,
            ops: group_ops.to_vec(),
            datum: zero,
            settings,
            offset: offset.clone(),
        });
    }
    Ok(out)
}
pub fn post_job(doc: &Document, mesh: &Mesh) -> JobPost {
    if doc.file.is_none() {
        return JobPost::blocked(
            "Import a part before posting. The example project's paths are illustrative and cannot be machined.",
        );
    }
    if doc.setups.is_empty() {
        return JobPost::blocked("Create a machining setup.");
    }
    let mut results: Vec<Vec<(usize, Result<Simulation, String>)>> = vec![vec![]; doc.setups.len()];
    crate::simulation::sequence(doc, mesh, None, |si, oi, r| results[si].push((oi, r)));
    let setups = (0..doc.setups.len())
        .map(|si| {
            let result = prepare(doc, mesh, si, &results[si]).and_then(|parts| {
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
                        .map_err(|e| {
                            vec![format!(
                                "{}: round-trip verification failed: {e}",
                                p.file_name
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
        let (mut doc, mesh) = job();
        doc.setups[0].operations.truncate(2);
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
            datum: [0., 0., 10.],
            disabled: vec![],
        };
        (emit(&header, &ops, &blocks, &settings), blocks, settings)
    }
    const GOLDEN: &str = "(AXIS CAM Studio - LinuxCNC 3-axis program)
(Job: Golden - Setup 1: Top side - G54)
(Machine: Workshop Mill v1)
(Work zero: Stock top center. Touch off X0 Y0 Z0 there.)
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
    fn machine_settings_change_tool_length_retract_blend_and_dwell() {
        let mut m = Machine::new(3);
        m.p.insert("post_tool_length".into(), POST_TOOL_LENGTH[1].into());
        m.p.insert("post_retract".into(), POST_RETRACT[1].into());
        m.p.insert("post_blend".into(), "0".into());
        m.p.insert("post_dwell".into(), "0".into());
        let (text, _, _) = program(m);
        assert!(text.contains("(Load T1 Flat end mill - D10 and use AXIS Touch Off, not Tool Touch Off, to set Z0 at the work zero before starting this program.)\nG17 G21"), "{text}");
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
    fn clearance_retracts_use_the_loaded_tool_length() {
        let mut m = Machine::new(3);
        m.p.insert("post_retract".into(), POST_RETRACT[1].into());
        let (text, _, _) = program(m);
        assert!(
            text.contains(
                "\nG54\nG43\nG0 Z15.000\n(Operation 1/1: Face top - T1)\nT1 M6\nG43 H1\nG0 Z15.000\nS8000 M3\n"
            ),
            "{text}"
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
            );
        }
        assert!(
            programs[1]
                .text
                .contains("(Program 2 of 4 for this setup, one per tool)")
        );
        assert!(
            programs[1]
                .text
                .contains("(Load T3 Drill 4 and use AXIS Touch Off")
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
        assert!(out.join("Fixture block - 1 Top (G54).ngc").exists());
    }
}
