//! Checks that must pass before G-code is written. Positions are plan
//! coordinates (setup frame, mm). The tool is modelled as the simulation
//! models it: a cylinder of cutter diameter up to its stickout, then the holder.
use crate::{
    holding::HoldBox,
    mesh::Bounds,
    model::{Machine, Setup, get, num},
    post::{Block, Kind, OpState, PostSettings, clearance_plane, extents},
    rest::RestStock,
    simulation::{Plan, Simulation},
};
use std::collections::{HashMap, HashSet};

const EPS: f32 = 0.0005;
/// How many times the profile's path blending tolerance (G64 P) the tool
/// keeps over the stock top where it moves sideways: the clearance plane,
/// every sideways rapid and the link between two operations of one tool
/// (issue #161). At the corner where a retract turns into a traverse the
/// tangent blend leaves the vertical up to P/(√2 − 1) ≈ 2.41 P below the
/// traverse height, so a traverse closer than that over the top can dip
/// into the pocket rim the tool has just left.
pub const BLEND_HEIGHT: f32 = 2.5;
/// How far over the stock top sideways motion keeps, in mm: `BLEND_HEIGHT`
/// times the blend tolerance of `settings`.
pub fn blend_margin(settings: &PostSettings) -> f32 {
    BLEND_HEIGHT * settings.blend
}
/// Machine-profile travel or feed in mm (or mm/min).
pub fn machine_mm(m: &Machine, key: &str) -> f32 {
    let v = num(&m.p, key).unwrap_or(0.);
    if get(&m.p, "units") == "inch" {
        v * 25.4
    } else {
        v
    }
}
#[derive(Default)]
struct Problems {
    seen: HashSet<(Option<usize>, &'static str, String)>,
    list: Vec<(Option<usize>, String)>,
}
impl Problems {
    /// Keep the first message for each operation and rule.
    fn add(&mut self, op: Option<usize>, rule: &'static str, message: String) {
        self.add_about(op, rule, "", message);
    }
    /// Keep the first message for each operation, rule and `subject` (the box a hit names).
    fn add_about(&mut self, op: Option<usize>, rule: &'static str, subject: &str, message: String) {
        if self.seen.insert((op, rule, subject.to_string())) {
            self.list.push((op, message));
        }
    }
    fn messages(self) -> Vec<String> {
        self.list.into_iter().map(|(_, message)| message).collect()
    }
}
fn key(x: f32, y: f32) -> (i64, i64) {
    ((x * 1000.).round() as i64, (y * 1000.).round() as i64)
}
/// Each operation and A angle `blocks` move at: the A the last
/// `Block::Index` turned to, or a move's own when it differs.
fn angles(blocks: &[Block]) -> Vec<(usize, f32)> {
    let mut a_now = None;
    let mut out = vec![];
    for block in blocks {
        let (op, a) = match block {
            Block::Index { op, a } => (*op, Some(*a)),
            Block::Move { op, a, .. } => (*op, a.or(a_now)),
            Block::Descend { op, .. } | Block::Approach { op, .. } => (*op, a_now),
            _ => continue,
        };
        a_now = a;
        out.extend(a.map(|a| (op, a)));
    }
    out
}
pub fn check_setup(
    setup: &Setup,
    ops: &[OpState],
    sims: &[&Simulation],
    blocks: &[Block],
    settings: &PostSettings,
    datum: [f32; 3],
) -> Vec<String> {
    let mut problems = Problems::default();
    let machine = &setup.machine;
    if !settings.tool_table {
        let mut tools: Vec<&str> = vec![];
        for op in ops {
            if !tools.contains(&op.tool_id.as_str()) {
                tools.push(&op.tool_id);
            }
        }
        if tools.len() > 1 {
            problems.add(
                None,
                "touch-off",
                "A touch-off program must use a single tool; export splits touch-off jobs into one program per tool.".into(),
            );
        }
    }
    let stock = sims[0].plan.stock;
    if sims.iter().any(|s| s.plan.stock != stock) {
        problems.add(
            None,
            "stock",
            "Operations in this setup report different stock boxes, so the work zero is ambiguous."
                .into(),
        );
    }
    let top = stock.max[2];
    let max_rpm = num(&machine.p, "rpm").unwrap_or(0.);
    let max_feed = machine_mm(machine, "feed");
    for (k, op) in ops.iter().enumerate() {
        if !(op.rpm > 0. && op.rpm <= max_rpm) {
            problems.add(
                Some(k),
                "rpm",
                format!(
                    "{}: S{} is outside the machine's 1-{max_rpm:.0} rpm range.",
                    op.name,
                    op.rpm.round()
                ),
            );
        }
    }
    let span = extents(blocks);
    for (k, (axis, rule)) in [("X", "travel X"), ("Y", "travel Y"), ("Z", "travel Z")]
        .into_iter()
        .enumerate()
    {
        let travel = machine_mm(machine, &axis.to_lowercase());
        let size = span.max[k] - span.min[k];
        if size > travel + 0.001 {
            problems.add(None, rule, format!("The program spans {size:.3} mm in {axis}, more than the machine's {axis} travel of {travel:.3} mm."));
        }
    }
    // Sideways motion keeps the blend margin over the top: at a corner the
    // blend can dip below the programmed height (`BLEND_HEIGHT`).
    let margin = blend_margin(settings);
    let clear = top + margin;
    let over = |z: f32| format!("{:.3} mm over the stock top", z - top);
    let blend = || {
        format!(
            "less than {BLEND_HEIGHT} times the path blending tolerance (G64 P{:.3}, {margin:.3} mm)",
            settings.blend
        )
    };
    let plane = clearance_plane(blocks);
    if !settings.machine_home && plane < top - EPS {
        problems.add(None, "retract", "The setup clearance plane is below the stock top, so tool changes would retract into material. Raise the operations' clearance heights.".into());
    } else if !settings.machine_home && plane < clear - EPS {
        problems.add(None, "retract", format!("The setup clearance plane is {}: {}, so the blend of a rapid there could dip into material. Raise the operations' clearance heights.", over(plane), blend()));
    }
    let rests: Vec<Option<RestStock>> = sims
        .iter()
        .map(|s| {
            s.initial_volume
                .as_ref()
                .zip(s.frame)
                .map(|(v, f)| RestStock::new(v, f))
        })
        .collect();
    let at = |p: [f32; 3]| {
        format!(
            "X{:.3} Y{:.3} Z{:.3}",
            p[0] - datum[0],
            p[1] - datum[1],
            p[2] - datum[2]
        )
    };
    // An indexed operation's moves at each A angle: the stock as its
    // simulation starts, seen in that angle's frame (`Simulation::frame_at`),
    // and the top of the box round it, which no layer earlier cuts removed
    // whole lies above (`RestStock::left`, the part not kept). A 3-axis
    // move (no A) keeps the setup's.
    let mut turned: HashMap<(usize, u32), (Option<RestStock>, f32)> = HashMap::new();
    let unbounded = Bounds {
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
    };
    for (op, a) in angles(blocks) {
        turned.entry((op, a.to_bits())).or_insert_with(|| {
            let sim = sims[op];
            let rest = (sim.initial_volume.as_ref())
                .zip(sim.frame_at(Some(a)))
                .map(|(v, f)| RestStock::new(v, f));
            let high = rest.as_ref().map_or(top, |r| r.left(unbounded).max[2]);
            (rest, high)
        });
    }
    let at_angle = |op: usize, a: Option<f32>| -> (Option<&RestStock>, f32) {
        match a.and_then(|a| turned.get(&(op, a.to_bits()))) {
            Some((rest, high)) => (rest.as_ref(), *high),
            None => (rests[op].as_ref(), top),
        }
    };
    // The A the program has turned to (`Block::Index`); None until it turns.
    let mut a_now: Option<f32> = None;
    // Lowest tip Z reached at each XY by the loaded tool since its tool
    // change, at the A it is at: a turn of A moves the stock under it.
    let mut visited: HashMap<(i64, i64), f32> = HashMap::new();
    for block in blocks {
        let (op, from, to, rapid) = match block {
            Block::Start { change: true, .. } => {
                visited.clear();
                continue;
            }
            Block::Index { a, .. } => {
                a_now = Some(*a);
                visited.clear();
                continue;
            }
            Block::Descend { op, xy, z } => {
                (*op, [xy[0], xy[1], f32::INFINITY], [xy[0], xy[1], *z], true)
            }
            Block::Move {
                op,
                kind,
                from,
                to,
                feed,
                a,
                ..
            } => {
                if *kind == Kind::Feed && !(*feed > 0. && *feed <= max_feed + 0.001) {
                    problems.add(Some(*op), "feed", format!("{}: F{feed:.1} is outside the machine's 0-{max_feed:.1} mm/min feed range.", ops[*op].name));
                }
                // Only indexed A is posted: A turns alone (G0 A), never
                // during a move. Continuous A is not posted yet.
                let same = match (*a, a_now) {
                    (Some(a), Some(b)) => (a - b).abs() < 1e-4,
                    (a, b) => a.is_none() && b.is_none(),
                };
                if !same {
                    let said = |a: Option<f32>| {
                        a.map_or("no A".to_string(), |a| {
                            format!("A {}°", crate::indexed::degrees(a))
                        })
                    };
                    problems.add(
                        Some(*op),
                        "continuous A",
                        format!(
                            "{}: A turns from {} to {} during a {} move to {}. Only indexed A is posted, A turning alone with X, Y and Z still; continuous A moves are not posted yet.",
                            ops[*op].name,
                            said(a_now),
                            said(*a),
                            if *kind == Kind::Feed { "feed" } else { "rapid" },
                            at(*to)
                        ),
                    );
                    a_now = *a;
                }
                (*op, *from, *to, *kind == Kind::Rapid)
            }
            _ => continue,
        };
        let (rest, top) = at_angle(op, a_now);
        let o = &ops[op];
        let low = from[2].min(to[2]);
        if low + o.stickout < top - EPS {
            problems.add(Some(op), "holder", format!("{}: the holder would drop below the stock top at {} (stickout {:.3} mm). Use a longer stickout or a shallower cut.", o.name, at([to[0], to[1], low]), o.stickout));
        }
        let vertical = (from[0] - to[0]).abs() <= EPS && (from[1] - to[1]).abs() <= EPS;
        if rapid && !vertical && low < clear - EPS {
            problems.add(
                Some(op),
                "rapid",
                if low < top - EPS {
                    format!(
                        "{}: a rapid moves sideways below the stock top at {}.",
                        o.name,
                        at(to)
                    )
                } else {
                    format!(
                        "{}: a rapid moves sideways at {}, {}: {}, so its blend could dip into material.",
                        o.name,
                        at(to),
                        over(low),
                        blend()
                    )
                },
            );
        } else if rapid && vertical && low < top - EPS {
            let revisited = visited
                .get(&key(to[0], to[1]))
                .is_some_and(|z| *z <= low + EPS);
            let proven = || {
                rest.is_some_and(|r| {
                    // The controller may leave the path by the blend
                    // tolerance at the corners: the column is that much wider.
                    let radius = o.diameter / 2. + settings.blend;
                    let column = Bounds {
                        min: [to[0] - radius, to[1] - radius, low],
                        max: [to[0] + radius, to[1] + radius, top],
                    };
                    r.beside(column) || r.top(column, low - 0.001, top) <= low + EPS
                })
            };
            if !revisited && !proven() {
                problems.add(
                    Some(op),
                    "rapid",
                    format!(
                        "{}: a rapid descends to {} through material that is not proven removed.",
                        o.name,
                        at([to[0], to[1], low])
                    ),
                );
            }
        }
        for p in [from, to] {
            if p[2].is_finite() {
                let z = visited.entry(key(p[0], p[1])).or_insert(p[2]);
                *z = z.min(p[2]);
            }
        }
    }
    problems.messages()
}
/// How far above the highest box a retract to the machine's Z home is checked
/// to, in mm: the home height is not known, and it is above everything.
const HOME_CLEARANCE: f32 = 10_000.;
/// What the program's motion does to the jaws or clamps in `boxes` (each grown
/// by `gap` and by the blend tolerance of `settings`, since the controller may
/// leave the programmed path by that much at every corner; the messages say
/// the setup's gap): one problem per operation and part of its tool
/// (`hits_segments` with the operation's own tool). Every stretch the tool
/// tip travels counts, in plan coordinates: the moves and the links between
/// operations, the
/// retracts (straight up from where the tip is to the retract height, the
/// clearance plane or, for the machine's Z home, far above every box), the
/// approach after a tool change (across at the retract height from where the
/// tip was; at the program's start only the point) and the descent. The
/// retracts after the last operation belong to it. At the clearance plane the
/// program's first rapid starts wherever the tool was, so the plane itself
/// has to be above the highest box and its gap: one more problem, for the
/// program. Messages are in work coordinates (`datum`), each with the index in
/// `ops` of the operation it is about (None for the plane's). The tests'
/// way in; the walk of a setup takes its tools' parts (`holding_problems_of`).
#[cfg(test)]
pub fn holding_problems(
    ops: &[OpState],
    plans: &[&Plan],
    blocks: &[Block],
    settings: &PostSettings,
    boxes: &[HoldBox],
    gap: f32,
    datum: [f32; 3],
) -> Vec<(Option<usize>, String)> {
    let tools: Vec<_> = plans
        .iter()
        .map(|p| {
            let parts = crate::holding::parts(p.radius, p.tool_length, p.holder_radius);
            (parts.to_vec(), String::new())
        })
        .collect();
    holding_problems_of(ops, plans, blocks, settings, boxes, (gap, datum), &tools)
}
/// `holding_problems` with each operation's tool made of its own parts and
/// a sentence ending the message of a hit by any part but the cutter (what
/// of it is assumed): a chuck's setup takes the spindle nose over the holder
/// (`rotary_check::in_spindle`, LIO-204). `place` is the gap and the datum.
pub fn holding_problems_of(
    ops: &[OpState],
    plans: &[&Plan],
    blocks: &[Block],
    settings: &PostSettings,
    boxes: &[HoldBox],
    place: (f32, [f32; 3]),
    tools: &[(Vec<crate::holding::Part>, String)],
) -> Vec<(Option<usize>, String)> {
    let (gap, datum) = place;
    // The highest box, the first of equals.
    let Some(highest) = boxes.iter().fold(None::<&HoldBox>, |best, b| {
        best.filter(|x| x.bounds.max[2] >= b.bounds.max[2])
            .or(Some(b))
    }) else {
        return vec![];
    };
    let top = highest.bounds.max[2];
    let plane = clearance_plane(blocks);
    let mut problems = Problems::default();
    // The path may leave the programmed one by the blend: the boxes are kept
    // that much farther than the gap.
    let reach = gap + settings.blend;
    // The program's first rapid starts from wherever the tool was, at the
    // clearance plane: it has to clear every box and its gap (a tip exactly
    // at the top and gap touches, as in the hits).
    if !settings.machine_home && plane <= top + reach {
        let stock_top = plans.first().map_or(datum[2], |plan| plan.stock.max[2]);
        let message = crate::holding::plane_message(
            &highest.label,
            plane,
            gap,
            settings.blend,
            top,
            datum[2],
            stock_top,
        );
        problems.add(None, "plane", message);
    }
    let high = if settings.machine_home {
        (top + reach + HOME_CLEARANCE).max(plane)
    } else {
        plane
    };
    let mut segments: Vec<Vec<([f32; 3], [f32; 3])>> = vec![vec![]; ops.len()];
    let (mut tip, mut op): (Option<[f32; 3]>, usize) = (None, 0);
    for block in blocks {
        match block {
            Block::Start { op: k, .. } | Block::Run { op: k, .. } => op = *k,
            Block::Move {
                op: k, from, to, ..
            } => {
                op = *k;
                segments[op].push((*from, *to));
                tip = Some(*to);
            }
            Block::Retract => {
                if let Some(at) = tip {
                    let to = [at[0], at[1], high];
                    segments[op].push((at, to));
                    tip = Some(to);
                }
            }
            Block::Approach { op: k, xy } => {
                op = *k;
                let to = [xy[0], xy[1], high];
                let from = tip.map_or(to, |at| [at[0], at[1], high]);
                segments[op].push((from, to));
                tip = Some(to);
            }
            Block::Descend { op: k, xy, z } => {
                op = *k;
                let to = [xy[0], xy[1], *z];
                segments[op].push(([xy[0], xy[1], high], to));
                tip = Some(to);
            }
            _ => {}
        }
    }
    for (k, path) in segments.iter().enumerate() {
        let (parts, assumed) = &tools[k];
        let hits = crate::holding::hits_parts(path, parts, boxes, reach);
        for hit in hits {
            let mut message = crate::holding::message(&ops[k].name, &hit, gap, datum);
            if hit.part != "tool" {
                message.push_str(assumed);
            }
            problems.add_about(Some(k), hit.part, &hit.label, message);
        }
    }
    problems.list
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        post::{OpState, PostSettings, blocks, fixture},
        simulation::{Motion, Move, Plan},
    };
    fn check(machine: Machine, op: OpState, moves: &[(Motion, [f32; 3], f32)]) -> Vec<String> {
        let plan = fixture::plan([-15., -15., 25.], moves);
        let settings = PostSettings::from_machine(&machine).unwrap();
        let blocks = blocks(std::slice::from_ref(&op), &[&plan], &settings);
        let setup = Setup::new(machine, "G54");
        let sim = Simulation::new(plan);
        check_setup(&setup, &[op], &[&sim], &blocks, &settings, [0., 0., 10.])
    }
    const FACING: [(Motion, [f32; 3], f32); 4] = [
        (Motion::Rapid, [-15., -15., 11.], 6000.),
        (Motion::Feed, [-15., -15., 9.], 250.),
        (Motion::Feed, [15., -15., 9.], 1000.),
        (Motion::Rapid, [15., -15., 25.], 6000.),
    ];
    #[test]
    fn a_normal_facing_pass_is_accepted() {
        assert_eq!(
            check(Machine::new(3), fixture::facing_op(), &FACING),
            Vec::<String>::new()
        );
    }
    #[test]
    fn retracts_and_peck_re_entry_at_the_same_xy_are_accepted() {
        let pecks = [
            (Motion::Rapid, [-15., -15., 11.], 6000.),
            (Motion::Feed, [-15., -15., 4.], 100.),
            (Motion::Rapid, [-15., -15., 11.], 6000.),
            (Motion::Rapid, [-15., -15., 4.1], 6000.),
            (Motion::Feed, [-15., -15., 2.], 100.),
            (Motion::Rapid, [-15., -15., 25.], 6000.),
        ];
        let problems = check(Machine::new(3), fixture::facing_op(), &pecks);
        assert!(problems.is_empty(), "{problems:?}");
    }
    #[test]
    fn rapids_into_unproven_material_or_sideways_below_the_top_are_refused() {
        let down = [
            (Motion::Rapid, [-15., -15., 5.], 6000.),
            (Motion::Feed, [-15., -15., 4.], 100.),
            (Motion::Rapid, [-15., -15., 25.], 6000.),
        ];
        let problems = check(Machine::new(3), fixture::facing_op(), &down);
        assert!(
            problems.iter().any(|p| p.contains("not proven removed")),
            "{problems:?}"
        );
        let sideways = [
            (Motion::Rapid, [-15., -15., 11.], 6000.),
            (Motion::Feed, [-15., -15., 5.], 100.),
            (Motion::Rapid, [0., -15., 5.], 6000.),
            (Motion::Rapid, [0., -15., 25.], 6000.),
        ];
        let problems = check(Machine::new(3), fixture::facing_op(), &sideways);
        assert!(
            problems.iter().any(|p| p.contains("sideways")),
            "{problems:?}"
        );
    }
    #[test]
    fn a_sideways_rapid_within_the_blend_margin_over_the_top_is_refused() {
        // A Facing whose rapids cross `z`, 0.5 mm over the stock top (Z10).
        // Under G64 P the blend at a corner can leave the vertical 2.41 P
        // below the crossing height, so a crossing keeps 2.5 P over the
        // top: clear with no blend and with the default 0.01 mm (0.025 mm),
        // refused with 1 mm (2.5 mm) both at the clearance plane and as a
        // sideways rapid, and clear again 2.5 mm over the top (#161).
        let crossing = |z: f32| {
            fixture::plan(
                [-15., -15., z],
                &[
                    (Motion::Rapid, [-15., -15., 10.], 6000.),
                    (Motion::Feed, [-15., -15., 9.], 250.),
                    (Motion::Feed, [15., -15., 9.], 1000.),
                    (Motion::Rapid, [15., -15., z], 6000.),
                    (Motion::Rapid, [15., 15., z], 6000.),
                ],
            )
        };
        let problems = |machine: Machine, plan: Plan| -> Vec<String> {
            let settings = PostSettings::from_machine(&machine).unwrap();
            let op = fixture::facing_op();
            let blocks = blocks(std::slice::from_ref(&op), &[&plan], &settings);
            let setup = Setup::new(machine, "G54");
            let sim = Simulation::new(plan);
            check_setup(&setup, &[op], &[&sim], &blocks, &settings, [0., 0., 10.])
        };
        let plane = |blend: &str| {
            let mut machine = blended(blend);
            machine
                .p
                .insert("post_retract".into(), crate::model::POST_RETRACT[1].into());
            machine
        };
        assert_eq!(problems(plane("0"), crossing(10.5)), Vec::<String>::new());
        assert_eq!(
            problems(plane("0.01"), crossing(10.5)),
            Vec::<String>::new()
        );
        assert_eq!(
            problems(plane("1"), crossing(10.5)),
            [
                "The setup clearance plane is 0.500 mm over the stock top: less than 2.5 times the path blending tolerance (G64 P1.000, 2.500 mm), so the blend of a rapid there could dip into material. Raise the operations' clearance heights.",
                "Face top: a rapid moves sideways at X15.000 Y15.000 Z0.500, 0.500 mm over the stock top: less than 2.5 times the path blending tolerance (G64 P1.000, 2.500 mm), so its blend could dip into material."
            ]
        );
        assert_eq!(problems(plane("1"), crossing(12.5)), Vec::<String>::new());
        // Retracting to machine home, only the crossing is named.
        assert_eq!(
            problems(blended("1"), crossing(10.5)),
            [
                "Face top: a rapid moves sideways at X15.000 Y15.000 Z0.500, 0.500 mm over the stock top: less than 2.5 times the path blending tolerance (G64 P1.000, 2.500 mm), so its blend could dip into material."
            ]
        );
        // A crossing below the top keeps its own line (from a clearance
        // point 15 mm over the top, so that nothing else is refused first).
        let below = fixture::plan(
            [-15., -15., 25.],
            &[
                (Motion::Rapid, [-15., -15., 10.], 6000.),
                (Motion::Feed, [-15., -15., 9.], 250.),
                (Motion::Feed, [15., -15., 9.], 1000.),
                (Motion::Rapid, [15., -15., 9.5], 6000.),
                (Motion::Rapid, [15., 15., 9.5], 6000.),
                (Motion::Rapid, [15., 15., 25.], 6000.),
            ],
        );
        assert_eq!(
            problems(blended("1"), below),
            ["Face top: a rapid moves sideways below the stock top at X15.000 Y15.000 Z-0.500."]
        );
    }
    #[test]
    fn holder_feed_rpm_and_travel_limits_are_enforced() {
        let short = OpState {
            stickout: 0.5,
            ..fixture::facing_op()
        };
        assert!(
            check(Machine::new(3), short, &FACING)
                .iter()
                .any(|p| p.contains("holder"))
        );
        let mut slow = Machine::new(3);
        slow.p.insert("feed".into(), "500".into());
        assert!(
            check(slow, fixture::facing_op(), &FACING)
                .iter()
                .any(|p| p.contains("F1000.0"))
        );
        let fast = OpState {
            rpm: 20000.,
            ..fixture::facing_op()
        };
        assert!(
            check(Machine::new(3), fast, &FACING)
                .iter()
                .any(|p| p.contains("S20000"))
        );
        let mut small = Machine::new(3);
        small.p.insert("x".into(), "10".into());
        assert!(
            check(small, fixture::facing_op(), &FACING)
                .iter()
                .any(|p| p.contains("X travel"))
        );
        let mut inch = Machine::new(3);
        inch.p.insert("units".into(), "inch".into());
        inch.p.insert("x".into(), "1".into());
        assert!(
            check(inch, fixture::facing_op(), &FACING)
                .iter()
                .any(|p| p.contains("25.400 mm"))
        );
    }
    #[test]
    fn zero_or_invalid_feeds_are_refused() {
        for bad in [0., f32::NAN] {
            let moves = [
                (Motion::Rapid, [-15., -15., 11.], 6000.),
                (Motion::Feed, [-15., -15., 9.], 250.),
                (Motion::Feed, [15., -15., 9.], 1000.),
                (Motion::Rapid, [15., -15., 25.], 6000.),
            ];
            let mut plan = fixture::plan([-15., -15., 25.], &moves);
            plan.moves[2].feed = bad;
            let settings = PostSettings::from_machine(&Machine::new(3)).unwrap();
            let op = fixture::facing_op();
            let blocks = blocks(std::slice::from_ref(&op), &[&plan], &settings);
            let setup = Setup::new(Machine::new(3), "G54");
            let sim = Simulation::new(plan);
            let problems = check_setup(&setup, &[op], &[&sim], &blocks, &settings, [0., 0., 10.]);
            assert!(
                problems.iter().any(|p| p.contains("feed range")),
                "{bad}: {problems:?}"
            );
        }
    }
    #[test]
    fn touch_off_programs_must_use_one_tool() {
        let mut m = Machine::new(3);
        m.p.insert(
            "post_tool_length".into(),
            crate::model::POST_TOOL_LENGTH[1].into(),
        );
        let settings = PostSettings::from_machine(&m).unwrap();
        let a = fixture::plan([-15., -15., 25.], &FACING);
        let b = fixture::plan([15., -15., 25.], &FACING);
        let ops = [
            fixture::facing_op(),
            OpState {
                tool_id: "t2".into(),
                tool: 2,
                ..fixture::facing_op()
            },
        ];
        let blocks = blocks(&ops, &[&a, &b], &settings);
        let setup = Setup::new(m, "G54");
        let (sa, sb) = (Simulation::new(a.clone()), Simulation::new(b.clone()));
        let problems = check_setup(&setup, &ops, &[&sa, &sb], &blocks, &settings, [0., 0., 10.]);
        assert!(
            problems.iter().any(|p| p.contains("single tool")),
            "{problems:?}"
        );
    }
    #[test]
    fn rest_stock_proof_accepts_rapids_into_faced_material_only() {
        let (doc, mesh) = fixture::job();
        let mut sims = vec![];
        crate::simulation::sequence(&doc, &mesh, None, |_, _, r| sims.push(r.unwrap()));
        let setup = &doc.setups[0];
        let settings = PostSettings::from_machine(&setup.machine).unwrap();
        let ops: Vec<OpState> = sims
            .iter()
            .zip(&setup.operations)
            .map(|(s, op)| crate::post::op_state(&doc, op, &s.plan, Some(&settings)).unwrap())
            .collect();
        let zero = crate::post::datum(setup, sims[0].plan.stock, &mesh).unwrap();
        let top = sims[0].plan.stock.max[2];
        // Parallel (T2) follows a tool change, so only the rest-stock proof can accept a rapid.
        let problems = |depth: f32, x: f32, y: f32| {
            let mut sims = sims.clone();
            let plan = &mut sims[4].plan;
            let p0 = plan.moves[0].to;
            let hi = [x + zero[0], y + zero[1], p0[2]];
            let lo = [hi[0], hi[1], top - depth];
            let rapid = |from, to| Move {
                from,
                to,
                motion: Motion::Rapid,
                cutting_pass: false,
                feed: 0.,
                start: 0.,
                end: 0.,
                a: None,
            };
            plan.moves.splice(
                1..1,
                [rapid(p0, hi), rapid(hi, lo), rapid(lo, hi), rapid(hi, p0)],
            );
            let plans: Vec<&Plan> = sims.iter().map(|s| &s.plan).collect();
            let blocks = blocks(&ops, &plans, &settings);
            let refs: Vec<&Simulation> = sims.iter().collect();
            check_setup(setup, &ops, &refs, &blocks, &settings, zero)
        };
        let refused = |p: &[String]| p.iter().any(|p| p.contains("not proven removed"));
        // Facing removed 4 mm; the stock box is X-62 to 62, Y-42 to 42 from the work zero.
        assert_eq!(problems(4., -55., -35.), Vec::<String>::new());
        assert!(refused(&problems(4.2, -55., -35.)));
        // A column that overlaps the stock is proven only as deep as the facing went there.
        assert_eq!(problems(3., -61.5, -41.5), Vec::<String>::new());
        assert!(refused(&problems(4.2, -61.5, -41.5)));
        // Wholly beside the stock box there was never any stock.
        assert_eq!(problems(6., -66., -46.), Vec::<String>::new());
    }
    #[test]
    fn visited_positions_reset_at_a_tool_change() {
        let a = fixture::plan(
            [-15., -15., 25.],
            &[
                (Motion::Rapid, [-15., -15., 11.], 6000.),
                (Motion::Feed, [-15., -15., 4.], 100.),
                (Motion::Rapid, [-15., -15., 25.], 6000.),
            ],
        );
        let b = fixture::plan(
            [-15., -15., 25.],
            &[
                (Motion::Rapid, [-15., -15., 4.1], 6000.),
                (Motion::Feed, [-15., -15., 2.], 100.),
                (Motion::Rapid, [-15., -15., 25.], 6000.),
            ],
        );
        let settings = PostSettings::from_machine(&Machine::new(3)).unwrap();
        let setup = Setup::new(Machine::new(3), "G54");
        let (sa, sb) = (Simulation::new(a.clone()), Simulation::new(b.clone()));
        let problems = |second: OpState| {
            let ops = [fixture::facing_op(), second];
            let blocks = blocks(&ops, &[&a, &b], &settings);
            check_setup(&setup, &ops, &[&sa, &sb], &blocks, &settings, [0., 0., 10.])
        };
        assert_eq!(problems(fixture::facing_op()), Vec::<String>::new());
        let other = OpState {
            tool_id: "t2".into(),
            tool: 2,
            ..fixture::facing_op()
        };
        let problems = problems(other);
        assert!(
            problems.iter().any(|p| p.contains("not proven removed")),
            "{problems:?}"
        );
    }
    #[test]
    fn descend_below_the_stock_top_is_refused() {
        let plan = fixture::plan(
            [-15., -15., 5.],
            &[
                (Motion::Feed, [-15., -15., 4.], 100.),
                (Motion::Rapid, [-15., -15., 25.], 6000.),
            ],
        );
        let settings = PostSettings::from_machine(&Machine::new(3)).unwrap();
        let op = fixture::facing_op();
        let blocks = blocks(std::slice::from_ref(&op), &[&plan], &settings);
        assert!(blocks.contains(&Block::Descend {
            op: 0,
            xy: [-15., -15.],
            z: 5.
        }));
        let setup = Setup::new(Machine::new(3), "G54");
        let sim = Simulation::new(plan);
        let problems = check_setup(&setup, &[op], &[&sim], &blocks, &settings, [0., 0., 10.]);
        assert!(
            problems.iter().any(|p| p.contains("not proven removed")),
            "{problems:?}"
        );
    }
    fn clamp_box(label: &str, min: [f32; 3], max: [f32; 3]) -> HoldBox {
        HoldBox {
            shape: Default::default(),
            label: label.into(),
            bounds: Bounds { min, max },
        }
    }
    /// The workholding problems of `ops` on `boxes` (2 mm gap, work zero at Z10).
    fn walk(machine: Machine, ops: &[OpState], plans: &[&Plan], boxes: &[HoldBox]) -> Vec<String> {
        let settings = PostSettings::from_machine(&machine).unwrap();
        let blocks = blocks(ops, plans, &settings);
        holding_problems(ops, plans, &blocks, &settings, boxes, 2., [0., 0., 10.])
            .into_iter()
            .map(|(_, message)| message)
            .collect()
    }
    fn clearance() -> Machine {
        let mut machine = Machine::new(3);
        machine
            .p
            .insert("post_retract".into(), crate::model::POST_RETRACT[1].into());
        machine
    }
    fn named(name: &str) -> OpState {
        OpState {
            name: name.into(),
            ..fixture::facing_op()
        }
    }
    /// A clamp on +X: footprint X30-50 Y-10-10, Z10-35.
    fn low_clamp() -> [HoldBox; 1] {
        [clamp_box(
            "clamp on +X (offset 0)",
            [30., -10., 10.],
            [50., 10., 35.],
        )]
    }
    #[test]
    fn a_fixture_pin_under_a_toolpath_is_hit_as_a_clamp_is() {
        // Ø4 pins 6 mm tall from the stock's bottom at Z0, pin 1 under the
        // tool's plunge.
        let on = crate::post::OnPins {
            pin1: [0., 0.],
            places: vec![[0., 0.], [80., 0.]],
            diameter: 4.,
            height: 6.,
        };
        let stock = Bounds {
            min: [-10., -10., 0.],
            max: [10., 10., 10.],
        };
        let boxes = crate::holding::pin_boxes(Some(&on), stock);
        let plunge = |z: f32| {
            fixture::plan(
                [-15., -15., 25.],
                &[
                    (Motion::Rapid, [0., 0., 25.], 6000.),
                    (Motion::Feed, [0., 0., z], 250.),
                    (Motion::Rapid, [0., 0., 25.], 6000.),
                ],
            )
        };
        let hits = walk(
            Machine::new(3),
            &[fixture::facing_op()],
            &[&plunge(5.)],
            &boxes,
        );
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(
            hits[0].starts_with("Face top: the tool hits the fixture pin 1 at X0.0 Y0.0"),
            "{hits:?}"
        );
        // Above the pin's top and the 2 mm gap the plunge is clear.
        assert!(
            walk(
                Machine::new(3),
                &[fixture::facing_op()],
                &[&plunge(8.5)],
                &boxes
            )
            .is_empty()
        );
    }
    #[test]
    fn a_feed_through_a_clamp_is_one_problem_for_the_tool_in_work_coordinates() {
        // The tool is Ø10 (reach 7 with the 2 mm gap), the holder Ø32 above 35 mm.
        let plan = fixture::plan(
            [-15., -15., 25.],
            &[
                (Motion::Rapid, [0., 0., 25.], 6000.),
                (Motion::Feed, [0., 0., 20.], 250.),
                (Motion::Feed, [40., 0., 20.], 1000.),
                (Motion::Feed, [45., 0., 20.], 1000.),
                (Motion::Rapid, [45., 0., 25.], 6000.),
            ],
        );
        assert_eq!(
            walk(
                Machine::new(3),
                &[fixture::facing_op()],
                &[&plan],
                &low_clamp()
            ),
            [
                "Face top: the tool hits the clamp on +X (offset 0) at X30.0 Y0.0 Z10.0 (within the 2 mm gap)."
            ]
        );
        assert!(walk(Machine::new(3), &[fixture::facing_op()], &[&plan], &[]).is_empty());
    }
    /// A clamp over the stock, Z40-60, on +X (footprint X30-50 Y-10-10), and
    /// plans whose tool (thin holder, reach 5) works low beside it.
    fn overhang() -> ([HoldBox; 1], impl Fn([f32; 3], [f32; 3]) -> Plan) {
        let boxes = [clamp_box(
            "clamp on +X (offset 0)",
            [30., -10., 40.],
            [50., 10., 60.],
        )];
        let low = |start: [f32; 3], to: [f32; 3]| {
            let mut plan = fixture::plan(start, &[(Motion::Feed, to, 1000.)]);
            plan.holder_radius = 3.;
            plan
        };
        (boxes, low)
    }
    #[test]
    fn the_final_retract_is_checked_even_when_every_move_is_clear() {
        use crate::holding;
        let (boxes, low) = overhang();
        let (a, b) = (
            low([-15., 0., 0.], [-10., 0., 0.]),
            low([-10., 0., 0.], [24., 0., 0.]),
        );
        assert!(holding::hits(&a, &boxes, 2.).is_empty());
        assert!(holding::hits(&b, &boxes, 2.).is_empty());
        // The retract after the last operation belongs to the last one.
        assert_eq!(
            walk(
                Machine::new(3),
                &[named("First"), named("Second")],
                &[&a, &b],
                &boxes
            ),
            [
                "Second: the tool hits the clamp on +X (offset 0) at X24.0 Y0.0 Z-7.0 (within the 2 mm gap)."
            ]
        );
        // At the clearance plane the retract goes only as high as the highest
        // move, 0 here: nothing is passed, but a plane that low is refused.
        assert_eq!(
            walk(
                clearance(),
                &[named("First"), named("Second")],
                &[&a, &b],
                &boxes
            ),
            [
                "The setup clearance plane (Z-10.0) is not above the clamp on +X (offset 0) and its 2 mm gap (Z52.0). Raise the operations' clearance above 52.0 mm over the stock top (Z52.0), or retract to machine home."
            ]
        );
    }
    #[test]
    fn the_descent_is_checked_even_when_every_move_is_clear() {
        use crate::holding;
        let (boxes, low) = overhang();
        // The program starts 6 mm from the clamp's edge, at Z0, and leaves.
        let plan = low([24., 0., 0.], [-10., 0., 0.]);
        assert!(holding::hits(&plan, &boxes, 2.).is_empty());
        // Coming down from the retract height the tool passes the clamp from
        // its top (62 with the gap, 52 above the work zero).
        assert_eq!(
            walk(Machine::new(3), &[fixture::facing_op()], &[&plan], &boxes),
            [
                "Face top: the tool hits the clamp on +X (offset 0) at X24.0 Y0.0 Z52.0 (within the 2 mm gap)."
            ]
        );
    }
    #[test]
    fn the_retract_before_a_tool_change_is_the_old_tools() {
        let (boxes, low) = overhang();
        let a = low([-15., 0., 0.], [24., 0., 0.]);
        let mut b = low([100., 0., 0.], [110., 0., 0.]);
        b.radius = 1.;
        let other = OpState {
            tool: 2,
            tool_id: "t2".into(),
            ..named("Cut B")
        };
        // Cut A's Ø10 tool is 6 mm from the clamp and rises past it; Cut B's
        // Ø2 tool, there afterwards, would not touch it.
        assert_eq!(
            walk(Machine::new(3), &[named("Cut A"), other], &[&a, &b], &boxes),
            [
                "Cut A: the tool hits the clamp on +X (offset 0) at X24.0 Y0.0 Z-7.0 (within the 2 mm gap)."
            ]
        );
    }
    #[test]
    fn a_tool_change_approach_sweeps_the_retract_height() {
        let boxes = low_clamp();
        let at_x = |x: f32| fixture::plan([x, 0., 20.], &[(Motion::Feed, [x, 0., 18.], 500.)]);
        let (a, b) = (at_x(0.), at_x(80.));
        let other = OpState {
            tool: 2,
            tool_id: "t2".into(),
            ..named("Cut B")
        };
        let ops = [named("Cut A"), other];
        // Neither operation reaches the clamp, but the rapid across at the
        // clearance plane (20, the highest tip, below the clamp's 37) goes
        // through it at Z20; the plane itself is refused too.
        assert_eq!(
            walk(clearance(), &ops, &[&a, &b], &boxes),
            [
                "The setup clearance plane (Z10.0) is not above the clamp on +X (offset 0) and its 2 mm gap (Z27.0). Raise the operations' clearance above 27.0 mm over the stock top (Z27.0), or retract to machine home.",
                "Cut B: the tool hits the clamp on +X (offset 0) at X30.0 Y0.0 Z10.0 (within the 2 mm gap)."
            ]
        );
        // Retracting to the machine's Z home goes over everything.
        assert!(walk(Machine::new(3), &ops, &[&a, &b], &boxes).is_empty());
    }
    #[test]
    fn a_link_between_operations_of_one_tool_is_checked_in_either_retract_mode() {
        let boxes = low_clamp();
        let at_x = |x: f32| fixture::plan([x, 0., 20.], &[(Motion::Feed, [x, 0., 18.], 500.)]);
        let (a, b) = (at_x(0.), at_x(80.));
        let ops = [named("Cut A"), named("Cut B")];
        let hit = "Cut B: the tool hits the clamp on +X (offset 0) at X30.0 Y0.0 Z10.0 (within the 2 mm gap).";
        assert_eq!(walk(Machine::new(3), &ops, &[&a, &b], &boxes), [hit]);
        // At the clearance plane the plane is refused as well (20, below 37).
        assert_eq!(
            walk(clearance(), &ops, &[&a, &b], &boxes),
            [
                "The setup clearance plane (Z10.0) is not above the clamp on +X (offset 0) and its 2 mm gap (Z27.0). Raise the operations' clearance above 27.0 mm over the stock top (Z27.0), or retract to machine home.",
                hit
            ]
        );
    }
    #[test]
    fn the_first_approach_is_a_point_not_a_sweep_from_the_origin() {
        // A clamp at the origin, the work far from it.
        let boxes = [clamp_box(
            "clamp on -X (offset 0)",
            [-10., -10., 10.],
            [10., 10., 35.],
        )];
        let plan = fixture::plan([50., 50., 20.], &[(Motion::Feed, [60., 50., 20.], 500.)]);
        let one = [fixture::facing_op()];
        assert_eq!(
            walk(Machine::new(3), &one, &[&plan], &boxes),
            Vec::<String>::new()
        );
        // At the clearance plane (20, below the clamp's 37) only the plane is
        // refused: no sweep from the origin through the clamp is invented.
        assert_eq!(
            walk(clearance(), &one, &[&plan], &boxes),
            [
                "The setup clearance plane (Z10.0) is not above the clamp on -X (offset 0) and its 2 mm gap (Z27.0). Raise the operations' clearance above 27.0 mm over the stock top (Z27.0), or retract to machine home."
            ]
        );
    }
    #[test]
    fn at_the_clearance_plane_it_must_be_above_every_box_and_its_gap() {
        // The first rapid of a program starts wherever the tool was: the
        // plane has to clear the highest box and its gap (37 for the clamp,
        // 52 for the tall one), even when nothing is near.
        let tall = clamp_box(
            "clamp on -X (offset 0)",
            [-50., -10., 10.],
            [-30., 10., 50.],
        );
        let [low] = low_clamp();
        let plan_at = |z: f32| fixture::plan([0., 0., z], &[(Motion::Feed, [5., 0., z], 500.)]);
        let one = [fixture::facing_op()];
        let refused = |z: f32, boxes: &[HoldBox]| walk(clearance(), &one, &[&plan_at(z)], boxes);
        // A plane exactly at the top and the gap touches it: refused.
        assert_eq!(
            refused(37., std::slice::from_ref(&low)),
            [
                "The setup clearance plane (Z27.0) is not above the clamp on +X (offset 0) and its 2 mm gap (Z27.0). Raise the operations' clearance above 27.0 mm over the stock top (Z27.0), or retract to machine home."
            ]
        );
        assert!(refused(37.5, std::slice::from_ref(&low)).is_empty());
        // The highest box is named, and the plane has to clear it.
        let both = [low.clone(), tall];
        assert_eq!(
            refused(45., &both),
            [
                "The setup clearance plane (Z35.0) is not above the clamp on -X (offset 0) and its 2 mm gap (Z42.0). Raise the operations' clearance above 42.0 mm over the stock top (Z42.0), or retract to machine home."
            ]
        );
        assert!(refused(52.5, &both).is_empty());
        // Equal tops name the first box.
        let twin = clamp_box(
            "clamp on -X (offset 0)",
            [-50., -10., 10.],
            [-30., 10., 35.],
        );
        assert!(
            refused(30., &[low.clone(), twin])
                .first()
                .is_some_and(|m| m.contains("the clamp on +X"))
        );
        // Machine home goes over everything; so does no workholding.
        let settings = |machine: Machine| walk(machine, &one, &[&plan_at(20.)], &both);
        assert!(settings(Machine::new(3)).is_empty());
        assert!(refused(20., &[]).is_empty());
        // Every program of a touch-off job starts from an unknown position too.
        let mut touch = clearance();
        touch.p.insert(
            "post_tool_length".into(),
            crate::model::POST_TOOL_LENGTH[1].into(),
        );
        for tool in ["t1", "t2"] {
            let op = OpState {
                tool_id: tool.into(),
                ..fixture::facing_op()
            };
            let found = walk(
                touch.clone(),
                &[op],
                &[&plan_at(20.)],
                std::slice::from_ref(&low),
            );
            assert_eq!(found.len(), 1, "{found:?}");
            assert!(found[0].starts_with("The setup clearance plane (Z10.0) is not above"));
        }
    }
    #[test]
    fn one_problem_per_operation_part_and_box() {
        // Two tall clamps; the holder (reach 18) comes within the gap of the
        // first before the tool (reach 7) does, and both come within the gap
        // of the second: a problem for each box, not one for each part.
        let boxes = [
            clamp_box("clamp on +X (offset 0)", [30., -10., 10.], [50., 10., 100.]),
            clamp_box(
                "clamp on -X (offset 0)",
                [-50., -10., 10.],
                [-30., 10., 100.],
            ),
        ];
        let plan = fixture::plan(
            [0., 0., 20.],
            &[
                (Motion::Feed, [14., 0., 20.], 1000.),
                (Motion::Feed, [24., 0., 20.], 1000.),
                (Motion::Feed, [-24., 0., 20.], 1000.),
            ],
        );
        assert_eq!(
            walk(Machine::new(3), &[fixture::facing_op()], &[&plan], &boxes),
            [
                "Face top: the tool hits the clamp on +X (offset 0) at X24.0 Y0.0 Z10.0 (within the 2 mm gap).",
                "Face top: the holder hits the clamp on +X (offset 0) at X14.0 Y0.0 Z10.0 (within the 2 mm gap).",
                "Face top: the tool hits the clamp on -X (offset 0) at X-24.0 Y0.0 Z10.0 (within the 2 mm gap).",
                "Face top: the holder hits the clamp on -X (offset 0) at X-24.0 Y0.0 Z10.0 (within the 2 mm gap).",
            ]
        );
    }
    #[test]
    fn each_operation_is_checked_with_its_own_tool() {
        let boxes = low_clamp();
        let path = [
            (Motion::Feed, [24., 0., 20.], 1000.),
            (Motion::Feed, [0., 0., 20.], 1000.),
        ];
        let wide = fixture::plan([0., 0., 20.], &path);
        let mut narrow = fixture::plan([0., 0., 20.], &path);
        narrow.radius = 1.;
        assert_eq!(
            walk(
                Machine::new(3),
                &[named("Wide"), named("Narrow")],
                &[&wide, &narrow],
                &boxes
            ),
            [
                "Wide: the tool hits the clamp on +X (offset 0) at X24.0 Y0.0 Z10.0 (within the 2 mm gap)."
            ]
        );
    }
    /// `Machine::new(3)` with the path blending tolerance `blend`.
    fn blended(blend: &str) -> Machine {
        let mut machine = Machine::new(3);
        machine.p.insert("post_blend".into(), blend.into());
        machine
    }
    #[test]
    fn the_profiles_blend_is_added_to_the_gap() {
        // A Ø10 tool whose edge passes 0.5 mm from the clamp's inner face
        // (X30): its centre at X24.5, within the clamp's heights. The setup
        // keeps no gap, so the reach is the radius, 5.
        let plan = fixture::plan(
            [24.5, 0., 25.],
            &[
                (Motion::Feed, [24.5, 0., 20.], 250.),
                (Motion::Feed, [24.5, 5., 20.], 1000.),
                (Motion::Rapid, [24.5, 5., 25.], 6000.),
            ],
        );
        let problems = |machine: Machine| -> Vec<String> {
            let settings = PostSettings::from_machine(&machine).unwrap();
            let op = fixture::facing_op();
            let blocks = blocks(std::slice::from_ref(&op), &[&plan], &settings);
            holding_problems(
                &[op],
                &[&plan],
                &blocks,
                &settings,
                &low_clamp(),
                0.,
                [0., 0., 10.],
            )
            .into_iter()
            .map(|(_, message)| message)
            .collect()
        };
        assert_eq!(problems(blended("0")), Vec::<String>::new());
        assert_eq!(problems(Machine::new(3)), Vec::<String>::new());
        // With G64 P1 the controller may leave the programmed path by up to
        // 1 mm at every corner: the tool may come 0.5 mm closer than proven.
        let found = problems(blended("1"));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0]
                .starts_with("Face top: the tool hits the clamp on +X (offset 0) at X24.5 Y0.0")
                && found[0].ends_with("(within the 0 mm gap)."),
            "{found:?}"
        );
        // At the clearance plane the blend counts too: a plane exactly at
        // the clamp's top passes with no gap and no blend, not with a blend.
        let [clamp] = low_clamp();
        let plane_at = |z: f32| fixture::plan([0., 0., z], &[(Motion::Feed, [5., 0., z], 500.)]);
        let plane = |blend: &str, z: f32| -> Vec<String> {
            let mut machine = blended(blend);
            machine
                .p
                .insert("post_retract".into(), crate::model::POST_RETRACT[1].into());
            let settings = PostSettings::from_machine(&machine).unwrap();
            let op = fixture::facing_op();
            let plan = plane_at(z);
            let blocks = blocks(std::slice::from_ref(&op), &[&plan], &settings);
            holding_problems(
                &[op],
                &[&plan],
                &blocks,
                &settings,
                std::slice::from_ref(&clamp),
                0.,
                [0., 0., 10.],
            )
            .into_iter()
            .map(|(_, message)| message)
            .collect()
        };
        assert_eq!(plane("0", 35.5), Vec::<String>::new());
        assert_eq!(
            plane("1", 35.5),
            [
                "The setup clearance plane (Z25.5) is not above the clamp on +X (offset 0) and its 0 mm gap (Z26.0). Raise the operations' clearance above 26.0 mm over the stock top (Z26.0), or retract to machine home."
            ]
        );
        assert_eq!(plane("1", 36.5), Vec::<String>::new());
    }
    #[test]
    fn the_profiles_blend_widens_the_rapid_column() {
        let (doc, mesh) = fixture::job();
        let mut sims = vec![];
        crate::simulation::sequence(&doc, &mesh, None, |_, _, r| sims.push(r.unwrap()));
        let top = sims[0].plan.stock.max[2];
        // In the Pocket (T1, Ø10, after the 4 mm Facing with the same tool)
        // a rapid 6 mm deep, 2 mm below the facing, whose column touches the
        // stock box (X-62 from the work zero) at its -X side: wholly beside
        // it, where there never was stock. Nothing of the facing was that
        // deep, so only the rest-stock proof can accept it. The crossing to
        // it runs 3 mm over the top, above the blend margin (#161), so that
        // only the column decides.
        let problems = |blend: &str| -> Vec<String> {
            let mut setup = doc.setups[0].clone();
            setup.machine.p.insert("post_blend".into(), blend.into());
            let settings = PostSettings::from_machine(&setup.machine).unwrap();
            let ops: Vec<OpState> = sims
                .iter()
                .zip(&setup.operations)
                .map(|(s, op)| crate::post::op_state(&doc, op, &s.plan, Some(&settings)).unwrap())
                .collect();
            let zero = crate::post::datum(&setup, sims[0].plan.stock, &mesh).unwrap();
            let mut sims = sims.clone();
            let plan = &mut sims[1].plan;
            assert_eq!(plan.radius, 5.);
            let p0 = plan.moves[0].to;
            let up = [p0[0], p0[1], top + 3.];
            let hi = [-67. + zero[0], -35. + zero[1], top + 3.];
            let lo = [hi[0], hi[1], top - 6.];
            let rapid = |from, to| Move {
                from,
                to,
                motion: Motion::Rapid,
                cutting_pass: false,
                feed: 0.,
                start: 0.,
                end: 0.,
                a: None,
            };
            plan.moves.splice(
                1..1,
                [
                    rapid(p0, up),
                    rapid(up, hi),
                    rapid(hi, lo),
                    rapid(lo, hi),
                    rapid(hi, up),
                    rapid(up, p0),
                ],
            );
            let plans: Vec<&Plan> = sims.iter().map(|s| &s.plan).collect();
            let blocks = blocks(&ops, &plans, &settings);
            let refs: Vec<&Simulation> = sims.iter().collect();
            check_setup(&setup, &ops, &refs, &blocks, &settings, zero)
        };
        assert_eq!(problems("0"), Vec::<String>::new());
        // With G64 P1 the column is 1 mm wider: it overlaps the stock, where
        // only the 4 mm of the facing is proven removed.
        let found = problems("1");
        assert!(
            found.iter().any(|p| p.contains("not proven removed")),
            "{found:?}"
        );
    }
}
