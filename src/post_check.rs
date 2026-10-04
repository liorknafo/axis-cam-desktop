//! Checks that must pass before G-code is written. Positions are plan
//! coordinates (setup frame, mm). The tool is modelled as the simulation
//! models it: a cylinder of cutter diameter up to its stickout, then the holder.
use crate::{
    mesh::Bounds,
    model::{Machine, Setup, get, num},
    post::{Block, Kind, OpState, PostSettings, clearance_plane, extents},
    rest::RestStock,
    simulation::Simulation,
};
use std::collections::{HashMap, HashSet};

const EPS: f32 = 0.0005;
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
    seen: HashSet<(Option<usize>, &'static str)>,
    list: Vec<String>,
}
impl Problems {
    /// Keep the first message for each operation and rule.
    fn add(&mut self, op: Option<usize>, rule: &'static str, message: String) {
        if self.seen.insert((op, rule)) {
            self.list.push(message);
        }
    }
}
fn key(x: f32, y: f32) -> (i64, i64) {
    ((x * 1000.).round() as i64, (y * 1000.).round() as i64)
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
    if !settings.machine_home && clearance_plane(blocks) < top - EPS {
        problems.add(None, "retract", "The setup clearance plane is below the stock top, so tool changes would retract into material. Raise the operations' clearance heights.".into());
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
    // Lowest tip Z reached at each XY by the loaded tool since its tool change.
    let mut visited: HashMap<(i64, i64), f32> = HashMap::new();
    for block in blocks {
        let (op, from, to, rapid) = match block {
            Block::Start { change: true, .. } => {
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
                ..
            } => {
                if *kind == Kind::Feed && !(*feed > 0. && *feed <= max_feed + 0.001) {
                    problems.add(Some(*op), "feed", format!("{}: F{feed:.1} is outside the machine's 0-{max_feed:.1} mm/min feed range.", ops[*op].name));
                }
                (*op, *from, *to, *kind == Kind::Rapid)
            }
            _ => continue,
        };
        let o = &ops[op];
        let low = from[2].min(to[2]);
        if low + o.stickout < top - EPS {
            problems.add(Some(op), "holder", format!("{}: the holder would drop below the stock top at {} (stickout {:.3} mm). Use a longer stickout or a shallower cut.", o.name, at([to[0], to[1], low]), o.stickout));
        }
        if rapid && low < top - EPS {
            let vertical = (from[0] - to[0]).abs() <= EPS && (from[1] - to[1]).abs() <= EPS;
            let revisited = visited
                .get(&key(to[0], to[1]))
                .is_some_and(|z| *z <= low + EPS);
            let proven = || {
                rests[op].as_ref().is_some_and(|r| {
                    let radius = o.diameter / 2.;
                    let column = Bounds {
                        min: [to[0] - radius, to[1] - radius, low],
                        max: [to[0] + radius, to[1] + radius, top],
                    };
                    r.beside(column) || r.top(column, low - 0.001, top) <= low + EPS
                })
            };
            if !vertical {
                problems.add(
                    Some(op),
                    "rapid",
                    format!(
                        "{}: a rapid moves sideways below the stock top at {}.",
                        o.name,
                        at(to)
                    ),
                );
            } else if !revisited && !proven() {
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
        let blocks = blocks(&[op.clone()], &[&plan], &settings);
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
            let blocks = blocks(&[op.clone()], &[&plan], &settings);
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
        let blocks = blocks(&[op.clone()], &[&plan], &settings);
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
}
