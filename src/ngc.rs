//! Strict reader for the LinuxCNC subset that `post` writes. It replays modal
//! state so each exported program can be compared with the simulated moves.
//! Any word the postprocessor never emits is an error, not ignored.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spindle {
    Off,
    Cw,
    Ccw,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Blend {
    Unset,
    Exact,
    Tolerance(f64),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Length {
    /// No G43/G49 seen yet: whatever the controller had before this program.
    Unknown,
    /// G49
    Off,
    /// Bare G43 before any M6: the loaded tool, whose number is unknown.
    Loaded,
    /// G43 H#, or bare G43 after T# M6
    Tool(u32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    Rapid,
    Feed,
    Dwell,
}
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub metric: bool,
    pub absolute: bool,
    pub xy_plane: bool,
    pub per_minute: bool,
    pub comp_off: bool,
    pub cycle_off: bool,
    pub blend: Blend,
    pub offset: Option<String>,
    pub selected: Option<u32>,
    pub tool: Option<u32>,
    pub length: Length,
    pub touched_off: bool,
    pub spindle: Spindle,
    pub rpm: f64,
    pub mist: bool,
    pub flood: bool,
    pub feed: f64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            metric: false,
            absolute: false,
            xy_plane: false,
            per_minute: false,
            comp_off: false,
            cycle_off: false,
            blend: Blend::Unset,
            offset: None,
            selected: None,
            tool: None,
            length: Length::Unknown,
            touched_off: false,
            spindle: Spindle::Off,
            rpm: 0.,
            mist: false,
            flood: false,
            feed: 0.,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub line: usize,
    pub kind: StepKind,
    /// G53 move: `to` holds machine values and the work position becomes unknown.
    pub machine: bool,
    pub from: [Option<f64>; 3],
    pub to: [Option<f64>; 3],
    pub seconds: f64,
    pub state: State,
}
#[derive(Debug, Clone, Default)]
pub struct Replay {
    pub steps: Vec<Step>,
    pub messages: Vec<String>,
    pub ended: bool,
    pub end_state: State,
}
/// Supported G codes ×10: G0 G1 G4 G17 G21 G40 G43 G49 G53 G54–G59.3 G61 G64 G80 G90 G94.
const G_CODES: [i64; 23] = [
    0, 10, 40, 170, 210, 400, 430, 490, 530, 540, 550, 560, 570, 580, 590, 591, 592, 593, 610, 640,
    800, 900, 940,
];
const M_CODES: [i64; 9] = [0, 2, 3, 4, 5, 6, 7, 8, 9];

fn strip_comments(raw: &str, messages: &mut Vec<String>) -> Result<String, String> {
    let mut code = String::new();
    let mut rest = raw;
    while let Some(open) = rest.find('(') {
        code.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after
            .find(')')
            .ok_or_else(|| "unterminated comment".to_string())?;
        let comment = &after[..close];
        if comment.contains('(') {
            return Err("nested comment".into());
        }
        if let Some(message) = comment.strip_prefix("MSG,") {
            messages.push(message.trim().to_string());
        }
        rest = &after[close + 1..];
    }
    if rest.contains(')') {
        return Err("unmatched )".into());
    }
    code.push_str(rest);
    Ok(code)
}
fn number(text: &str) -> Option<f64> {
    let digits = text.strip_prefix(|c| c == '-' || c == '+').unwrap_or(text);
    if digits.is_empty()
        || digits.matches('.').count() > 1
        || !digits.chars().all(|c| c.is_ascii_digit() || c == '.')
        || !digits.chars().any(|c| c.is_ascii_digit())
    {
        return None;
    }
    text.parse::<f64>().ok().filter(|v| v.is_finite())
}
fn words(code: &str) -> Result<Vec<(char, f64)>, String> {
    let chars: Vec<char> = code.chars().filter(|c| *c != ' ' && *c != '\t').collect();
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        let letter = chars[i];
        if !"GMTSFHPXYZ".contains(letter) {
            return Err(format!("unsupported word {letter}"));
        }
        let mut end = i + 1;
        while end < chars.len()
            && (chars[end].is_ascii_digit() || matches!(chars[end], '.' | '-' | '+'))
        {
            end += 1;
        }
        let text: String = chars[i + 1..end].iter().collect();
        out.push((
            letter,
            number(&text).ok_or_else(|| format!("malformed number after {letter}"))?,
        ));
        i = end;
    }
    Ok(out)
}
pub fn replay(text: &str) -> Result<Replay, String> {
    let mut out = Replay::default();
    let mut state = State::default();
    let mut position: [Option<f64>; 3] = [None; 3];
    let mut motion: Option<StepKind> = None;
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let fail = |why: &str| format!("line {line}: {why}: {raw}");
        let code = strip_comments(raw, &mut out.messages).map_err(|e| fail(&e))?;
        if code.trim().is_empty() {
            continue;
        }
        if out.ended {
            return Err(fail("code after M2"));
        }
        let mut g: Vec<i64> = vec![];
        let mut m: Vec<i64> = vec![];
        let mut axes: [Option<f64>; 3] = [None; 3];
        let (mut f, mut s, mut t, mut h, mut p) = (None, None, None, None, None);
        for (letter, value) in words(&code).map_err(|e| fail(&e))? {
            let slot = match letter {
                'G' => {
                    let tenths = value * 10.;
                    if (tenths - tenths.round()).abs() > 1e-6 {
                        return Err(fail("malformed G code"));
                    }
                    g.push(tenths.round() as i64);
                    continue;
                }
                'M' => {
                    if value.fract() != 0. {
                        return Err(fail("unsupported M code"));
                    }
                    m.push(value as i64);
                    continue;
                }
                'X' => &mut axes[0],
                'Y' => &mut axes[1],
                'Z' => &mut axes[2],
                'F' => &mut f,
                'S' => &mut s,
                'T' => &mut t,
                'H' => &mut h,
                _ => &mut p,
            };
            if slot.replace(value).is_some() {
                return Err(fail("repeated word"));
            }
        }
        if let Some(code) = g.iter().find(|c| !G_CODES.contains(c)) {
            return Err(fail(&format!("unsupported G{}", *code as f64 / 10.)));
        }
        if let Some(code) = m.iter().find(|c| !M_CODES.contains(c)) {
            return Err(fail(&format!("unsupported M{code}")));
        }
        let has = |c: i64| g.contains(&c);
        let motions = g.iter().filter(|c| **c == 0 || **c == 10).count();
        if motions > 1
            || m.iter().filter(|c| (3..=5).contains(*c)).count() > 1
            || g.iter().filter(|c| (540..=593).contains(*c)).count() > 1
            || (has(430) && has(490))
            || (has(40) && has(640))
            || (has(610) && has(640))
        {
            return Err(fail("conflicting codes on one line"));
        }
        if h.is_some() && !has(430) {
            return Err(fail("H without G43"));
        }
        if p.is_some() && !has(40) && !has(640) {
            return Err(fail("P without G4 or G64"));
        }
        if let Some(v) = f {
            if v <= 0. {
                return Err(fail("F must be positive"));
            }
            state.feed = v;
        }
        if let Some(v) = s {
            if v < 0. {
                return Err(fail("S cannot be negative"));
            }
            state.rpm = v;
        }
        if let Some(v) = t {
            if v < 0. || v.fract() != 0. {
                return Err(fail("T must be a whole number"));
            }
            state.selected = Some(v as u32);
        }
        if m.contains(&6) {
            state.tool = Some(
                state
                    .selected
                    .ok_or_else(|| fail("M6 without a selected tool"))?,
            );
            state.touched_off = false;
            // LinuxCNC stops the spindle for a tool change.
            state.spindle = Spindle::Off;
        }
        if m.contains(&3) || m.contains(&4) {
            if state.rpm <= 0. {
                return Err(fail("spindle start without S"));
            }
            state.spindle = if m.contains(&3) {
                Spindle::Cw
            } else {
                Spindle::Ccw
            };
        }
        if m.contains(&5) {
            state.spindle = Spindle::Off;
        }
        if m.contains(&9) {
            if m.contains(&7) || m.contains(&8) {
                return Err(fail("M9 with M7 or M8"));
            }
            state.mist = false;
            state.flood = false;
        }
        state.mist |= m.contains(&7);
        state.flood |= m.contains(&8);
        if has(40) {
            if axes.iter().any(Option::is_some) {
                return Err(fail("G4 with axis words"));
            }
            let seconds = p.filter(|v| *v >= 0.).ok_or_else(|| fail("G4 needs P"))?;
            out.steps.push(Step {
                line,
                kind: StepKind::Dwell,
                machine: false,
                from: position,
                to: position,
                seconds,
                state: state.clone(),
            });
        }
        state.xy_plane |= has(170);
        state.metric |= has(210);
        state.comp_off |= has(400);
        state.cycle_off |= has(800);
        state.absolute |= has(900);
        state.per_minute |= has(940);
        if has(430) {
            // A bare G43 uses the loaded tool, as LinuxCNC does.
            state.length = match h {
                None => state.tool.map_or(Length::Loaded, Length::Tool),
                Some(v) if v >= 0. && v.fract() == 0. => Length::Tool(v as u32),
                Some(_) => return Err(fail("G43 needs a whole-number H")),
            };
        }
        if has(490) {
            state.length = Length::Off;
        }
        if let Some(code) = g.iter().find(|c| (540..=593).contains(*c)) {
            state.offset = Some(match code {
                591..=593 => format!("G59.{}", code - 590),
                c => format!("G{}", c / 10),
            });
        }
        if has(610) {
            state.blend = Blend::Exact;
        }
        if has(640) {
            state.blend =
                Blend::Tolerance(p.filter(|v| *v >= 0.).ok_or_else(|| fail("G64 needs P"))?);
        }
        if let Some(code) = g.iter().find(|c| **c == 0 || **c == 10) {
            motion = Some(if *code == 0 {
                StepKind::Rapid
            } else {
                StepKind::Feed
            });
        }
        let machine = has(530);
        if machine && (motions == 0 || axes.iter().all(Option::is_none)) {
            return Err(fail("G53 needs G0 or G1 and an axis word"));
        }
        if axes.iter().any(Option::is_some) {
            let kind = motion.ok_or_else(|| fail("axis words without G0 or G1"))?;
            if !(state.metric
                && state.absolute
                && state.xy_plane
                && state.per_minute
                && state.comp_off
                && state.cycle_off)
            {
                return Err(fail("motion before G17 G21 G40 G80 G90 G94"));
            }
            if kind == StepKind::Feed && state.feed <= 0. {
                return Err(fail("G1 without F"));
            }
            let from = position;
            let to = if machine {
                for k in 0..3 {
                    if axes[k].is_some() {
                        position[k] = None;
                    }
                }
                axes
            } else {
                if state.offset.is_none() {
                    return Err(fail("motion before a work offset"));
                }
                for k in 0..3 {
                    if axes[k].is_some() {
                        position[k] = axes[k];
                    }
                }
                position
            };
            out.steps.push(Step {
                line,
                kind,
                machine,
                from,
                to,
                seconds: 0.,
                state: state.clone(),
            });
        }
        state.touched_off |= m.contains(&0);
        out.ended |= m.contains(&2);
    }
    out.end_state = state;
    Ok(out)
}

use crate::post::{Block, Kind, OpState, PostSettings};
const TOL: f64 = 0.0006;
fn take<'a>(steps: &'a [Step], i: &mut usize, what: &str) -> Result<&'a Step, String> {
    let s = steps
        .get(*i)
        .ok_or_else(|| format!("the program ends before {what}"))?;
    *i += 1;
    Ok(s)
}
fn near(a: Option<f64>, b: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() <= TOL)
}
/// State that must be active whenever an operation moves.
/// The spindle runs in the operation's direction at its rounded speed.
fn spinning(st: &State, op: &OpState) -> bool {
    let spindle = if op.clockwise {
        Spindle::Cw
    } else {
        Spindle::Ccw
    };
    st.spindle == spindle && (st.rpm - f64::from(op.rpm.round())).abs() <= 0.5
}
/// The tool and length offset are the operation's (tool table), or none at all (touch-off).
fn tool_ok(st: &State, op: &OpState, settings: &PostSettings) -> bool {
    if settings.tool_table {
        st.tool == Some(op.tool) && st.length == Length::Tool(op.tool)
    } else {
        st.tool.is_none() && st.length == Length::Off
    }
}
fn active(s: &Step, op: &OpState, settings: &PostSettings, offset: &str) -> Result<(), String> {
    let st = &s.state;
    let blend = match st.blend {
        Blend::Exact => settings.blend == 0.,
        // G64 P0 would blend without limit.
        Blend::Tolerance(p) => {
            settings.blend > 0. && p >= 0.0005 && (p - f64::from(settings.blend)).abs() <= TOL
        }
        Blend::Unset => false,
    };
    let problem = if settings.tool_table && st.tool != Some(op.tool) {
        format!("T{} is not loaded", op.tool)
    } else if settings.tool_table && st.length != Length::Tool(op.tool) {
        format!("G43 H{} is not active", op.tool)
    } else if !settings.tool_table && !tool_ok(st, op, settings) {
        "a touch-off program must not change tools or use a length offset".into()
    } else if !spinning(st, op) {
        format!(
            "the spindle is not running {} at S{}",
            if op.clockwise { "M3" } else { "M4" },
            op.rpm.round()
        )
    } else if (st.mist, st.flood) != op.coolant {
        "the coolant state is wrong".into()
    } else if st.offset.as_deref() != Some(offset) {
        format!("{offset} is not active")
    } else if !blend {
        "the path blending mode is wrong".into()
    } else {
        return Ok(());
    };
    Err(format!("line {}: {problem} for {}", s.line, op.name))
}
/// Replay `text` and require exactly the motion in `blocks` (plan coordinates).
pub fn verify(
    text: &str,
    blocks: &[Block],
    ops: &[OpState],
    datum: [f32; 3],
    settings: &PostSettings,
    offset: &str,
) -> Result<(), String> {
    let r = replay(text)?;
    if !r.ended {
        return Err("the program has no M2 end".into());
    }
    let d = datum.map(f64::from);
    let work = |p: [f32; 3]| -> [f64; 3] { std::array::from_fn(|k| f64::from(p[k]) - d[k]) };
    let clearance = f64::from(crate::post::clearance_plane(blocks)) - d[2];
    let steps = &r.steps;
    let mut i = 0;
    let mut at: [Option<f64>; 3] = [None; 3];
    // The operation of the most recent Start: its tool is the one loaded.
    let mut current: Option<usize> = None;
    for block in blocks {
        match block {
            Block::Stop | Block::Run { .. } | Block::End => {}
            Block::Start { op, .. } => current = Some(*op),
            Block::Retract => {
                let s = take(steps, &mut i, "a safe retract")?;
                let ok = s.kind == StepKind::Rapid
                    && if settings.machine_home {
                        s.machine && s.to == [None, None, Some(0.)]
                    } else {
                        !s.machine && near(s.to[2], clearance) && s.to[..2] == s.from[..2]
                    };
                if !ok {
                    return Err(format!("line {}: expected the safe retract", s.line));
                }
                // A G53 retract ignores the length offset; a clearance retract moves the tip.
                let st = &s.state;
                let length = if !settings.tool_table {
                    st.tool.is_none() && st.length == Length::Off
                } else if settings.machine_home {
                    true
                } else {
                    match current {
                        // The first tool change and its G43 H come before any clearance retract.
                        None => {
                            return Err(format!(
                                "line {}: no retract is expected before the first tool change",
                                s.line
                            ));
                        }
                        Some(c) => tool_ok(st, &ops[c], settings),
                    }
                };
                if !length {
                    return Err(format!(
                        "line {}: the safe retract runs with the wrong tool length offset",
                        s.line
                    ));
                }
                if s.machine {
                    at[2] = None;
                } else {
                    at = s.to;
                }
            }
            Block::Wait { seconds } => {
                let s = take(steps, &mut i, "the spindle dwell")?;
                if s.kind != StepKind::Dwell || (s.seconds - f64::from(*seconds)).abs() > TOL {
                    return Err(format!(
                        "line {}: expected a {seconds:.3} s spindle dwell",
                        s.line
                    ));
                }
                if !current.is_some_and(|c| spinning(&s.state, &ops[c])) {
                    return Err(format!(
                        "line {}: the spindle dwell runs before the spindle starts",
                        s.line
                    ));
                }
            }
            Block::Approach { op, xy } => {
                let s = take(steps, &mut i, "the approach")?;
                let w = work([xy[0], xy[1], 0.]);
                if s.machine
                    || s.kind != StepKind::Rapid
                    || !near(s.to[0], w[0])
                    || !near(s.to[1], w[1])
                    || s.to[2] != s.from[2]
                {
                    return Err(format!(
                        "line {}: expected the rapid approach in X/Y",
                        s.line
                    ));
                }
                active(s, &ops[*op], settings, offset)?;
                at = s.to;
            }
            Block::Descend { op, xy, z } => {
                let s = take(steps, &mut i, "the descent")?;
                let w = work([xy[0], xy[1], *z]);
                if s.machine
                    || s.kind != StepKind::Rapid
                    || (0..3).any(|k| !near(s.to[k], w[k]))
                    || s.to[..2] != s.from[..2]
                {
                    return Err(format!(
                        "line {}: expected the rapid descent to the first point",
                        s.line
                    ));
                }
                active(s, &ops[*op], settings, offset)?;
                at = s.to;
            }
            Block::Move {
                op,
                kind: Kind::Dwell,
                seconds,
                ..
            } => {
                let s = take(steps, &mut i, "a dwell")?;
                if s.kind != StepKind::Dwell || (s.seconds - f64::from(*seconds)).abs() > TOL {
                    return Err(format!("line {}: expected a {seconds:.3} s dwell", s.line));
                }
                active(s, &ops[*op], settings, offset)?;
            }
            Block::Move {
                op, kind, to, feed, ..
            } => {
                let w = work(*to);
                let expected = if *kind == Kind::Rapid {
                    StepKind::Rapid
                } else {
                    StepKind::Feed
                };
                let found = steps.get(i).filter(|s| {
                    !s.machine && s.kind == expected && (0..3).all(|k| near(s.to[k], w[k]))
                });
                if let Some(s) = found {
                    i += 1;
                    if expected == StepKind::Feed && (s.state.feed - f64::from(*feed)).abs() > 0.051
                    {
                        return Err(format!(
                            "line {}: F{:.1} does not match the simulated {feed:.1} mm/min",
                            s.line, s.state.feed
                        ));
                    }
                    active(s, &ops[*op], settings, offset)?;
                    at = s.to;
                } else if !(0..3).all(|k| near(at[k], w[k])) {
                    // Only a move shorter than the output resolution may be absent.
                    let line = steps.get(i).map_or(0, |s| s.line);
                    return Err(format!(
                        "line {line}: expected a {kind:?} move to X{:.3} Y{:.3} Z{:.3}",
                        w[0], w[1], w[2]
                    ));
                }
            }
        }
    }
    if let Some(s) = steps.get(i) {
        return Err(format!("line {}: unexpected extra motion", s.line));
    }
    if r.end_state.spindle != Spindle::Off || r.end_state.mist || r.end_state.flood {
        return Err("the spindle or coolant is still on at M2".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use StepKind::*;
    const SAMPLE: &str = "(header)\nG17 G21 G40 G49 G80 G90 G94\nG64 P0.010\nG54\nG53 G0 Z0\nT3 M6\nG43 H3\nS8000 M3\nG4 P2.000\nM8\nG0 X1.000 Y2.000\nZ5.000\nG1 Z-1.000 F250.0\nX4.000 F1000.0\nG0 Z5.000\nM9\nM5\nG53 G0 Z0\nM2\n";
    #[test]
    fn replays_motion_and_modal_state() {
        let r = replay(SAMPLE).unwrap();
        assert!(r.ended);
        let kinds: Vec<_> = r.steps.iter().map(|s| (s.kind, s.machine)).collect();
        assert_eq!(
            kinds,
            vec![
                (Rapid, true),
                (Dwell, false),
                (Rapid, false),
                (Rapid, false),
                (Feed, false),
                (Feed, false),
                (Rapid, false),
                (Rapid, true)
            ]
        );
        assert_eq!(r.steps[0].to, [None, None, Some(0.)]);
        assert_eq!(r.steps[2].to, [Some(1.), Some(2.), None]);
        let cut = &r.steps[5];
        assert_eq!(cut.from, [Some(1.), Some(2.), Some(-1.)]);
        assert_eq!(cut.to, [Some(4.), Some(2.), Some(-1.)]);
        assert_eq!(cut.state.feed, 1000.);
        assert_eq!(
            (cut.state.tool, cut.state.length),
            (Some(3), Length::Tool(3))
        );
        assert_eq!(
            (cut.state.spindle, cut.state.rpm, cut.state.flood),
            (Spindle::Cw, 8000., true)
        );
        assert_eq!(cut.state.offset.as_deref(), Some("G54"));
        assert_eq!(cut.state.blend, Blend::Tolerance(0.01));
        assert_eq!(r.steps[1].seconds, 2.);
        assert_eq!(
            (r.end_state.spindle, r.end_state.flood),
            (Spindle::Off, false)
        );
    }
    #[test]
    fn touch_off_mode_is_visible_in_state() {
        let text = "G17 G21 G40 G49 G80 G90 G94\nG61\nG59.1\nT2 M6\n(MSG, Touch off Z for T2)\nM0\nS1000 M4\nG0 X0 Y0 Z10\nG1 Z0 F100\nM5\nM2\n";
        let r = replay(text).unwrap();
        let cut = r.steps.last().unwrap();
        assert!(cut.state.touched_off);
        assert_eq!(cut.state.length, Length::Off);
        assert_eq!(cut.state.spindle, Spindle::Ccw);
        assert_eq!(cut.state.blend, Blend::Exact);
        assert_eq!(cut.state.offset.as_deref(), Some("G59.1"));
        assert_eq!(r.messages, vec!["Touch off Z for T2".to_string()]);
    }
    #[test]
    fn bare_g43_uses_the_loaded_tool() {
        let r = replay("G17 G21 G40 G49 G80 G90 G94\nG54\nT2 M6\nG43\nG0 Z5\nM2\n").unwrap();
        assert_eq!(r.steps[0].state.length, Length::Tool(2));
    }
    #[test]
    fn length_state_distinguishes_unknown_off_and_loaded_and_m6_stops_the_spindle() {
        let pre = "G17 G21 G40 G80 G90 G94\nG54\n";
        let r = replay(&format!(
            "{pre}G0 Z5\nG49\nG0 Z6\nG43\nG0 Z7\nS100 M3\nT1 M6\nG43 H1\nG0 Z8\nM2\n"
        ))
        .unwrap();
        let lengths: Vec<_> = r.steps.iter().map(|s| s.state.length).collect();
        assert_eq!(
            lengths,
            vec![
                Length::Unknown,
                Length::Off,
                Length::Loaded,
                Length::Tool(1)
            ]
        );
        assert_eq!(r.steps[3].state.spindle, Spindle::Off);
    }
    #[test]
    fn rejects_anything_the_post_does_not_emit() {
        let preamble = "G17 G21 G40 G49 G80 G90 G94\nG54\n";
        for bad in [
            "G2 X1 Y1",
            "G1 X1",
            "N10 G0 X1",
            "G0 X1.2.3",
            "G0 X1 (open",
            "G0 X1 X2",
            "X1",
            "M6",
            "G43 H1.5",
            "M98 P1",
            "G0 X1 ; note",
            "g0 x1",
            "G53 Z0",
            "G21.04",
            "G59.15",
            "G0.5 X1",
        ] {
            assert!(replay(&format!("{preamble}{bad}\nM2\n")).is_err(), "{bad}");
        }
        assert!(replay("G0 X1\nM2\n").is_err());
        assert!(replay(&format!("{preamble}M2\nG0 X1\n")).is_err());
        assert!(replay("G17 G21 G40 G49 G80 G90 G94\nG0 X1\nM2\n").is_err());
    }
}
