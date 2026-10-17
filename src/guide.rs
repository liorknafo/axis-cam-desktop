//! The step guide of a setup on a manual mill: what to do at the hand-fed
//! machine, screen by screen, with the numbers its DRO should read. It is
//! made from the setup's simulated plans, so every number is a move the app
//! checked. Every coordinate a step shows is a work coordinate (the stock
//! frame less the work zero); `Guide::zero` converts the kept plans.
use crate::{
    cam::{box_block, box_cut, open_sides},
    holding::Gripped,
    manual,
    mesh::{Bounds, Mesh},
    model::{Document, Machine, Strategy, get, num},
    post::At,
    simulation::{self, Motion, Move, Plan},
    volume::Volume,
};
use std::ops::Range;

/// One DRO number to move to, in the order the axes are moved.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    /// 1, 2, 3… within the pass.
    pub order: u8,
    /// 0 X, 1 Y, 2 Z.
    pub axis: usize,
    /// The tool centre at the end of the move.
    pub centre: f32,
    /// What the DRO reads there: the tool edge on X and Y with the DRO's
    /// compensation on, else the centre.
    pub shown: f32,
    /// `shown` is the tool edge.
    pub edge: bool,
    /// The axes to lock: on the cut, the two that do not move; on a move
    /// that sets an axis for the last time before the cut, that axis.
    pub lock: [bool; 3],
}
/// The lowest Z and the extreme X and Y the tool's edge reaches in an
/// operation's feeds: the numbers not to pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub x: [f32; 2],
    pub y: [f32; 2],
    pub z: f32,
}
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// The stock box the setup starts from and what the vise or clamps hold.
    Hold { stock: Bounds, holding: Gripped },
    /// The point the edge finder picks up, set to `corner` on the DRO, and
    /// the side each of X and Y is approached from (−1 the − face, +1 the +
    /// face, 0 both, for a centre).
    Zero { corner: [f32; 3], approach: [i8; 2] },
    /// Put in the tool `tool` (an id) at `rpm`; with the DRO's compensation
    /// on, enter `diameter` and set X and Y to `comp` (0 off: centres).
    Tool {
        op: usize,
        tool: String,
        rpm: f32,
        diameter: f32,
        comp: [i8; 2],
    },
    /// Same tool: only the compensation sides change.
    Comp { op: usize, comp: [i8; 2] },
    /// One pass of a Box, 1-based `index` of `of`. `cut` is what it sweeps
    /// out of the stock, `earlier` what the operation's passes before it did.
    Pass {
        op: usize,
        index: usize,
        of: usize,
        targets: Vec<Target>,
        cut: Bounds,
        earlier: Vec<Bounds>,
        never: Limits,
        turns_per_minute: f32,
    },
    /// A facing Box (open on all four sides): face the top down to `to`,
    /// taking off `remove`; the user plans the passes.
    Face {
        op: usize,
        tool: String,
        rpm: f32,
        diameter: f32,
        remove: f32,
        to: f32,
    },
    /// A squaring Box (one closed X or Y side, the wall, and a slab off it
    /// one row wide): cut the stock's `side` (−1 or +1) of `axis` to `to`
    /// (the wall as cut, with its allowance), which the DRO reads as `shown`
    /// (the edge with compensation, else the centre), taking off `remove`,
    /// down to `down_to` (the floor as cut).
    Square {
        op: usize,
        tool: String,
        rpm: f32,
        diameter: f32,
        axis: usize,
        side: i8,
        to: f32,
        shown: f32,
        remove: f32,
        down_to: f32,
    },
    /// The holes of a Drill in the order drilled: X and Y (centres) and the
    /// Z each is drilled to.
    Holes { op: usize, holes: Vec<[f32; 3]> },
    /// The stock after the operation.
    Check { op: usize },
    /// The stock leaves for setup `next` (none for the last setup).
    HandOver { next: Option<usize> },
}
/// A stretch of the steps: "Set up", one per operation, "Hand over".
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub name: String,
    pub op: Option<usize>,
    pub steps: Range<usize>,
    /// Passes or holes; 0 for the others.
    pub count: usize,
    pub minutes: f32,
}
/// What an operation of the setup did, for the pictures.
#[derive(Debug, Clone)]
pub struct Done {
    pub op: usize,
    /// In the stock frame; its `stock` is the box the setup starts from.
    pub plan: Plan,
    /// The stock after it.
    pub after: Volume,
}
#[derive(Debug, Clone)]
pub struct Guide {
    pub setup: usize,
    pub steps: Vec<Step>,
    pub sections: Vec<Section>,
    pub minutes: f32,
    /// The work zero in the stock frame: a work coordinate is a stock one less it.
    pub zero: [f32; 3],
    /// The stock before the setup's first operation.
    pub before: Volume,
    /// The enabled operations in order.
    pub done: Vec<Done>,
}
/// What a Box is on the hand-fed mill (`cam::open_sides`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Face,
    /// The closed side, the wall, is on `axis`, on its − side when `low`.
    Square {
        axis: usize,
        low: bool,
    },
    Other,
}
/// A Box with its sides `open` that cuts `lo`..`hi` (`cam::box_cut`) with a
/// tool of radius `r`: facing when all four X and Y sides are open;
/// squaring when exactly one is closed (the wall; its ends along the wall
/// are then open) and the slab off the wall is at most `r` wide, so one row
/// per level takes it, whatever its floor; otherwise passes.
fn kind(open: [[bool; 2]; 3], lo: [f32; 3], hi: [f32; 3], r: f32) -> Kind {
    let closed: Vec<(usize, bool)> = (0..2)
        .flat_map(|k| [(k, true), (k, false)])
        .filter(|&(k, low)| !open[k][usize::from(!low)])
        .collect();
    match closed[..] {
        [] => Kind::Face,
        [(axis, low)] if hi[axis] - lo[axis] <= r + crate::cam::ON => Kind::Square { axis, low },
        _ => Kind::Other,
    }
}
/// The DRO's compensation side on X and Y: toward the box's closed side,
/// + when both or neither are closed.
fn comp_sides(open: [[bool; 2]; 3]) -> [i8; 2] {
    std::array::from_fn(|k| if open[k] == [false, true] { -1 } else { 1 })
}
/// The one axis a manual move changes.
fn moved(m: &Move) -> Result<usize, String> {
    match (0..3).filter(|&k| m.from[k] != m.to[k]).collect::<Vec<_>>()[..] {
        [k] => Ok(k),
        _ => Err("A move on the manual mill changes more than one axis.".into()),
    }
}
/// The targets of `legs` (axis, stock-frame end), the last of them the cut,
/// as the DRO shows them.
fn targets(legs: &[(usize, f32)], zero: [f32; 3], comp: [i8; 2], r: f32) -> Vec<Target> {
    let last = legs.len() - 1;
    let cut = legs[last].0;
    (legs.iter().enumerate())
        .map(|(i, &(axis, at))| {
            let centre = at - zero[axis];
            let side = comp.get(axis).copied().unwrap_or(0);
            let lock = std::array::from_fn(|k| match i == last {
                true => k != axis,
                false => k == axis && k != cut && legs[i + 1..].iter().all(|l| l.0 != k),
            });
            Target {
                order: u8::try_from(i + 1).unwrap_or(u8::MAX),
                axis,
                centre,
                shown: centre + f32::from(side) * r,
                edge: side != 0,
                lock,
            }
        })
        .collect()
}
/// One Pass step per cut of a Box's `plan`.
fn passes(
    op: usize,
    plan: &Plan,
    zero: [f32; 3],
    comp: [i8; 2],
    machine: &Machine,
) -> Result<Vec<Step>, String> {
    let (r, s) = (plan.radius, plan.stock);
    let work = |b: Bounds| b.translated(zero.map(|v| -v));
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for m in plan.moves.iter().filter(|m| m.motion == Motion::Feed) {
        for p in [m.from, m.to] {
            for k in 0..3 {
                (lo[k], hi[k]) = (lo[k].min(p[k]), hi[k].max(p[k]));
            }
        }
    }
    let never = Limits {
        x: [lo[0] - r - zero[0], hi[0] + r - zero[0]],
        y: [lo[1] - r - zero[1], hi[1] + r - zero[1]],
        z: lo[2] - zero[2],
    };
    let cuts: Vec<&Move> = plan.moves.iter().filter(|m| m.cutting_pass).collect();
    let top = plan.rest_top.unwrap_or(s.max[2]);
    let (mut steps, mut earlier, mut legs) = (vec![], vec![], vec![]);
    for m in plan.moves.iter().filter(|m| m.motion != Motion::Dwell) {
        let axis = moved(m)?;
        legs.push((axis, m.to[axis]));
        if !m.cutting_pass {
            continue;
        }
        if steps.is_empty() {
            // Where the tool is before the first pass is not known: the axes
            // the plan does not move from its start are set before the plunge.
            let before = &legs[..legs.len() - 1];
            let missing: Vec<(usize, f32)> = (0..3)
                .filter(|&k| before.iter().all(|l| l.0 != k))
                .map(|k| (k, m.from[k]))
                .collect();
            let at = before
                .iter()
                .rposition(|l| l.0 == 2)
                .unwrap_or(before.len());
            legs.splice(at..at, missing);
        }
        let targets = targets(&legs, zero, comp, r);
        legs.clear();
        // From the level above (or the top of the material) down to this one.
        let above = cuts
            .iter()
            .map(|c| c.to[2])
            .filter(|z| *z > m.to[2])
            .fold(top, f32::min);
        let cut = Bounds {
            min: std::array::from_fn(|k| match k {
                2 => m.to[2],
                _ => m.from[k].min(m.to[k]) - r,
            }),
            max: std::array::from_fn(|k| match k {
                2 => above,
                _ => m.from[k].max(m.to[k]) + r,
            }),
        };
        let cut = work(Bounds {
            min: std::array::from_fn(|k| cut.min[k].max(s.min[k])),
            max: std::array::from_fn(|k| cut.max[k].min(s.max[k])),
        });
        steps.push(Step::Pass {
            op,
            index: steps.len() + 1,
            of: cuts.len(),
            targets,
            cut,
            earlier: earlier.clone(),
            never,
            turns_per_minute: manual::turns(machine, axis, m.feed),
        });
        earlier.push(cut);
    }
    Ok(steps)
}
/// The holes of a Drill's `plan` in the order drilled: centre and depth.
fn holes(plan: &Plan, zero: [f32; 3]) -> Vec<[f32; 3]> {
    let mut holes: Vec<[f32; 3]> = vec![];
    for m in plan.moves.iter().filter(|m| m.cutting_pass) {
        let at: [f32; 3] = std::array::from_fn(|k| m.to[k] - zero[k]);
        match holes.last_mut() {
            Some(h) if h[0] == at[0] && h[1] == at[1] => h[2] = h[2].min(at[2]),
            _ => holes.push(at),
        }
    }
    holes
}
/// The guide of setup `si`, a setup on a manual mill: the document's
/// operations simulated in order up to the end of it.
pub fn build(doc: &Document, mesh: &Mesh, si: usize) -> Result<Guide, String> {
    let setup = doc.setups.get(si).ok_or("Choose a setup.")?;
    let machine = &setup.machine;
    if !manual::is_manual(machine) {
        return Err("Steps are for a setup on a manual mill.".into());
    }
    let last = (setup.operations.iter())
        .rposition(|o| o.enabled)
        .ok_or("Add a Box or Drill operation to see the steps.")?;
    let (mut done, mut afters, mut before, mut error) = (vec![], vec![], None, None);
    let mut end = None;
    simulation::sequence_with(
        doc,
        mesh,
        Some((si, last)),
        |s, oi, result| match result {
            Ok(mut sim) if s == si => {
                if before.is_none() {
                    before = sim.initial_volume.clone();
                }
                if oi == last {
                    // The walk stops before playing its last operation.
                    done.push((oi, sim.plan.clone()));
                    sim.seek(1.);
                    end = sim.volume;
                } else {
                    done.push((oi, sim.plan));
                }
            }
            Err(e) if s == si && error.is_none() => error = Some(e),
            _ => {}
        },
        |s, _, volume| {
            if s == si {
                afters.push(volume.clone());
            }
        },
    );
    if let Some(error) = error {
        return Err(error);
    }
    afters.extend(end);
    let before = before.ok_or("The stock before the setup is not known.")?;
    if done.is_empty() || afters.len() != done.len() {
        return Err("The setup's operations did not all simulate.".into());
    }
    let stock = done[0].1.stock;
    let zero = crate::post::datum(setup, stock, mesh)?;
    let work = |b: Bounds| b.translated(zero.map(|v| -v));
    let dro = get(&machine.p, "dro_comp") == "Yes";
    let tool_time = num(&machine.p, "tool_time").unwrap_or(0.);

    // Set up.
    let frame = crate::volume::Frame::new(setup, mesh);
    let bands = (!before.cuts.is_empty())
        .then(|| crate::holding::Bands::new(&before, frame, simulation::part_box(setup, mesh)));
    let held = crate::holding::held_by(doc, setup, stock, bands.as_ref());
    let origin = get(&setup.p, "origin");
    let found = crate::post::references(origin)
        .zip(crate::post::touch_point(origin, stock))
        .ok_or("Choose a work zero for the setup.")?;
    let mut steps = vec![
        Step::Hold {
            stock: work(stock),
            holding: Gripped {
                stock: work(held.stock),
                band: work(held.band),
                bottom: held.bottom - zero[2],
            },
        },
        Step::Zero {
            corner: std::array::from_fn(|k| found.1[k] - zero[k]),
            approach: found.0.map(|a| match a {
                At::Min => -1,
                At::Mid => 0,
                At::Max => 1,
            }),
        },
    ];
    let mut sections = vec![Section {
        name: "Set up".into(),
        op: None,
        steps: 0..2,
        count: 0,
        minutes: 0.,
    }];

    // One section per operation.
    let mut last_tool: Option<&str> = None;
    // The tool, rpm and compensation sides the last operation left set.
    let mut set: Option<(&str, f32, Option<[i8; 2]>)> = None;
    let mut done_ops = vec![];
    for ((oi, plan), after) in done.into_iter().zip(afters) {
        let op = &setup.operations[oi];
        let tool = op.tool.as_str();
        let rpm = num(&op.p, "rpm").unwrap_or(0.);
        let (r, first) = (plan.radius, steps.len());
        let diameter = 2. * r;
        let changed = last_tool.replace(tool) != Some(tool);
        let minutes = plan.duration / 60. + if changed { tool_time } else { 0. };
        let mut count = 0;
        // The stock before this operation.
        let earlier = done_ops.last().map_or(&before, |d: &Done| &d.after);
        let mut sides = |comp: [i8; 2], steps: &mut Vec<Step>| {
            let new = (set.as_ref())
                .is_some_and(|&(t, s, c)| t == tool && s == rpm && c.is_some_and(|c| c != comp));
            steps.push(match new {
                true => Step::Comp { op: oi, comp },
                false => Step::Tool {
                    op: oi,
                    tool: tool.into(),
                    rpm,
                    diameter,
                    comp,
                },
            });
            set = Some((tool, rpm, Some(comp)));
        };
        match op.strategy {
            Strategy::Box => {
                let (low, high) = box_block(op, zero)?;
                let open = open_sides(low, high, &plan.stock);
                let allowance = crate::cam::number(op, "allowance")?;
                let (lo, hi) = box_cut(low, high, &plan.stock, allowance);
                let comp = if dro { comp_sides(open) } else { [0; 2] };
                match kind(open, lo, hi, r) {
                    Kind::Face => {
                        steps.push(Step::Face {
                            op: oi,
                            tool: tool.into(),
                            rpm,
                            diameter,
                            remove: plan.rest_top.unwrap_or(stock.max[2]) - plan.target,
                            to: plan.target - zero[2],
                        });
                        set = Some((tool, rpm, None));
                    }
                    Kind::Square { axis, low } => {
                        let side: i8 = if low { 1 } else { -1 };
                        // The cut nearest the wall: its centre, and its edge on the wall.
                        let centres = (plan.moves.iter())
                            .filter(|m| m.motion == Motion::Feed)
                            .flat_map(|m| [m.from[axis], m.to[axis]]);
                        let centre = match low {
                            true => centres.fold(f32::INFINITY, f32::min),
                            false => centres.fold(f32::NEG_INFINITY, f32::max),
                        };
                        let wall = centre - f32::from(side) * r;
                        let left = simulation::leftover_box(earlier, setup, mesh);
                        steps.push(Step::Square {
                            op: oi,
                            tool: tool.into(),
                            rpm,
                            diameter,
                            axis,
                            side,
                            to: wall - zero[axis],
                            shown: (if dro { wall } else { centre }) - zero[axis],
                            remove: if low {
                                left.max[axis] - wall
                            } else {
                                wall - left.min[axis]
                            },
                            down_to: plan.target - zero[2],
                        });
                        set = Some((tool, rpm, None));
                    }
                    Kind::Other => {
                        sides(comp, &mut steps);
                        let passes = passes(oi, &plan, zero, comp, machine)?;
                        count = passes.len();
                        steps.extend(passes);
                        steps.push(Step::Check { op: oi });
                    }
                }
            }
            Strategy::Drill => {
                sides([0; 2], &mut steps);
                let holes = holes(&plan, zero);
                count = holes.len();
                steps.push(Step::Holes { op: oi, holes });
                steps.push(Step::Check { op: oi });
            }
            other => return Err(format!("{} does not run on a manual mill.", other.name())),
        }
        sections.push(Section {
            name: op.name().into(),
            op: Some(oi),
            steps: first..steps.len(),
            count,
            minutes,
        });
        done_ops.push(Done {
            op: oi,
            plan,
            after,
        });
    }
    let n = steps.len();
    steps.push(Step::HandOver {
        next: (si + 1 < doc.setups.len()).then_some(si + 1),
    });
    sections.push(Section {
        name: "Hand over".into(),
        op: None,
        steps: n..n + 1,
        count: 0,
        minutes: 0.,
    });
    Ok(Guide {
        setup: si,
        minutes: sections.iter().map(|s| s.minutes).sum(),
        steps,
        sections,
        zero,
        before,
        done: done_ops,
    })
}
/// The step the guide was left at (`Document::guide`), within its steps.
pub fn place(doc: &Document, guide: &Guide) -> usize {
    (doc.setups.get(guide.setup))
        .and_then(|s| doc.guide.get(&s.id))
        .map_or(0, |&at| at.min(guide.steps.len().saturating_sub(1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BOX_FIELDS, Operation, Tool};

    /// A Box named `name` cutting `b` (x_min, x_max, y_min, y_max, z_top,
    /// z_bottom, work frame) with the tool `tool`.
    fn boxed(name: &str, tool: &str, b: [f32; 6], fields: &[(&str, &str)]) -> Operation {
        let mut op = Operation::new(Strategy::Box, tool.into());
        op.p.insert("name".into(), name.into());
        for (k, v) in BOX_FIELDS.iter().zip(b) {
            op.p.insert((*k).into(), v.to_string());
        }
        for (k, v) in fields {
            op.p.insert((*k).into(), (*v).into());
        }
        op
    }
    /// `post::fixture::job` with the manual mill's setup in front, the CNC
    /// setup after it. The manual setup has 20 mm of stock in front of and
    /// behind the part, its zero on the stock top's front-left corner, and
    /// three operations: Face (2 mm off the whole top), Step (open on −Y, its
    /// wall at Y 15.5 with 0.5 left, down to −12 in levels of at most 5)
    /// with a Ø25 end mill, and Holes (a Ø4 drill in the top's four Ø6.6
    /// through holes).
    fn job(dro: &str) -> (Document, Mesh) {
        let (mut doc, mesh) = crate::post::fixture::job();
        let mut machine = manual::defaults();
        machine.p.insert("dro_comp".into(), dro.into());
        let mut mill = Tool::new("End mill 25", "Flat end mill", "25");
        let mut drill = Tool::new("Drill 4", "Drill", "4");
        for t in [&mut mill, &mut drill] {
            for (k, v) in [
                ("length", "60"),
                ("stickout", "65"),
                ("machines", &machine.id),
            ] {
                t.p.insert(k.into(), v.into());
            }
        }
        let mut holes = Operation::new(Strategy::Drill, drill.id.clone());
        holes.surfaces = doc.setups[0].operations[3].surfaces.clone();
        for (k, v) in [
            ("name", "Holes"),
            ("rpm", "1600"),
            ("feed", "100"),
            ("hole_diameter", "6.6"),
        ] {
            holes.p.insert(k.into(), v.into());
        }
        let ops = vec![
            boxed(
                "Face",
                &mill.id,
                [-500., 500., -500., 500., 0., -2.],
                &[("allowance", "0")],
            ),
            boxed(
                "Step",
                &mill.id,
                [-500., 500., -500., 15.5, 0., -12.],
                &[("stepdown", "5")],
            ),
            holes,
        ];
        doc.machines.push(machine);
        doc.tools.extend([mill, drill]);
        doc.add_manual_setup().unwrap();
        let setup = &mut doc.setups[0];
        for (k, v) in [
            ("origin", "Stock top front-left"),
            ("stock_front", "20"),
            ("stock_back", "20"),
        ] {
            setup.p.insert(k.into(), v.into());
        }
        setup.operations = ops;
        (doc, mesh)
    }
    fn kinds(guide: &Guide, range: Range<usize>) -> Vec<&'static str> {
        guide.steps[range]
            .iter()
            .map(|s| match s {
                Step::Hold { .. } => "Hold",
                Step::Zero { .. } => "Zero",
                Step::Tool { .. } => "Tool",
                Step::Comp { .. } => "Comp",
                Step::Pass { .. } => "Pass",
                Step::Face { .. } => "Face",
                Step::Square { .. } => "Square",
                Step::Holes { .. } => "Holes",
                Step::Check { .. } => "Check",
                Step::HandOver { .. } => "HandOver",
            })
            .collect()
    }
    fn close(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-3, "{a} != {b}");
    }
    /// The Pass steps' targets with their pass's number.
    fn targets(guide: &Guide) -> Vec<(usize, &Target)> {
        (guide.steps.iter())
            .filter_map(|s| match s {
                Step::Pass { index, targets, .. } => Some((*index, targets)),
                _ => None,
            })
            .flat_map(|(i, t)| t.iter().map(move |t| (i, t)))
            .collect()
    }
    #[test]
    fn one_section_per_operation_between_set_up_and_hand_over() {
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        let names: Vec<&str> = guide.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Set up", "Face", "Step", "Holes", "Hand over"]);
        let ops: Vec<Option<usize>> = guide.sections.iter().map(|s| s.op).collect();
        assert_eq!(ops, [None, Some(0), Some(1), Some(2), None]);
        // The ranges cover every step once, in order.
        let mut at = 0;
        for s in &guide.sections {
            assert_eq!(s.steps.start, at);
            assert!(s.steps.end > at);
            at = s.steps.end;
        }
        assert_eq!(at, guide.steps.len());
        let of = |i: usize| kinds(&guide, guide.sections[i].steps.clone());
        assert_eq!(of(0), ["Hold", "Zero"]);
        assert_eq!(of(1), ["Face"]);
        assert_eq!(of(2), ["Tool", "Pass", "Pass", "Pass", "Pass", "Check"]);
        assert_eq!(of(3), ["Tool", "Holes", "Check"]);
        assert_eq!(of(4), ["HandOver"]);
        let counts: Vec<usize> = guide.sections.iter().map(|s| s.count).collect();
        assert_eq!(counts, [0, 0, 4, 4, 0]);
        // The minutes: the operations' plans and two tool changes of 3 minutes.
        let plans: f32 = guide.done.iter().map(|d| d.plan.duration / 60.).sum();
        close(guide.minutes, plans + 6.);
        let sum: f32 = guide.sections.iter().map(|s| s.minutes).sum();
        close(sum, guide.minutes);
        close(
            guide.sections[1].minutes,
            guide.done[0].plan.duration / 60. + 3.,
        );
        close(guide.sections[2].minutes, guide.done[1].plan.duration / 60.);
        assert_eq!(manual::tool_changes(&doc.setups[0]), 2);
        // What the pictures need: the stock before, and after each operation.
        assert_eq!(
            guide.done.iter().map(|d| d.op).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert!(guide.before.cuts.is_empty());
        let removed: Vec<f32> = guide
            .done
            .iter()
            .map(|d| d.after.removed_volume())
            .collect();
        assert!(
            removed[0] > 0. && removed.windows(2).all(|w| w[1] > w[0]),
            "{removed:?}"
        );
    }
    #[test]
    fn steps_run_hold_zero_then_each_operation_and_a_face_is_one_step() {
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(
            kinds(&guide, 0..guide.steps.len()),
            [
                "Hold", "Zero", "Face", "Tool", "Pass", "Pass", "Pass", "Pass", "Check", "Tool",
                "Holes", "Check", "HandOver"
            ]
        );
        let Step::Hold { stock, holding } = &guide.steps[0] else {
            panic!()
        };
        // In work coordinates: the zero on the stock top's front-left corner.
        assert_eq!((stock.min[0], stock.min[1], stock.max[2]), (0., 0., 0.));
        close(stock.max[1], 120.);
        assert_eq!(holding.stock, *stock);
        assert_eq!(
            guide.steps[1],
            Step::Zero {
                corner: [0.; 3],
                approach: [-1, -1]
            }
        );
        let Step::Face {
            op,
            tool,
            rpm,
            diameter,
            remove,
            to,
        } = &guide.steps[2]
        else {
            panic!("{:?}", guide.steps[2])
        };
        assert_eq!(
            (*op, tool, *rpm, *diameter),
            (0, &doc.tools[3].id, 640., 25.)
        );
        close(*to, -2.);
        close(*remove, 2.);
        assert_eq!(guide.steps[12], Step::HandOver { next: Some(1) });
        // The Step's floor is closed: not a squaring Box.
        assert!(matches!(guide.steps[3], Step::Tool { op: 1, .. }));
    }
    #[test]
    fn a_squaring_box_is_one_square_step_cut_to_the_wall() {
        let (width, height) = (124., 47.);
        // The +X side, 1 mm off as cut: down to a closed floor at −30 with
        // and without compensation, with 0.5 left on the wall and the floor,
        // and down through the stock's bottom.
        for (dro, readout, x_min, z_bottom, allowance, down_to) in [
            ("Yes", 0., 123., -30., "0", -30.),
            ("No", 12.5, 123., -30., "0", -30.),
            ("Yes", 0., 122.5, -30., "0.5", -29.5),
            ("Yes", 0., 123., -100., "0", -height),
        ] {
            let (mut doc, mesh) = job(dro);
            let mill = doc.tools[3].id.clone();
            let b = [x_min, 500., -500., 500., 0., z_bottom];
            let square = boxed("Square", &mill, b, &[("allowance", allowance)]);
            doc.setups[0].operations.insert(1, square);
            let guide = build(&doc, &mesh, 0).unwrap();
            let Step::Hold { stock, .. } = &guide.steps[0] else {
                panic!()
            };
            close(stock.max[0], width);
            close(stock.min[2], -height);
            assert_eq!(kinds(&guide, guide.sections[2].steps.clone()), ["Square"]);
            let Step::Square {
                op,
                axis,
                side,
                to,
                shown,
                remove,
                down_to: down,
                diameter,
                ..
            } = &guide.steps[3]
            else {
                panic!("{:?}", guide.steps[3])
            };
            assert_eq!((*op, *axis, *side, *diameter), (1, 0, 1, 25.));
            close(*to, 123.);
            close(*shown, 123. + readout);
            close(*remove, 1.);
            close(*down, down_to);
            // The plan's cut nearest the wall has its centre a radius off it,
            // on the floor.
            let plan = &guide.done[1].plan;
            let near = (plan.moves.iter())
                .filter(|m| m.cutting_pass)
                .map(|m| m.to[0] - guide.zero[0])
                .fold(f32::INFINITY, f32::min);
            close(near, 123. + 12.5);
            close(plan.target - guide.zero[2], down_to);
            // The 15.5 mm front step, wider than the radius, is not squaring:
            // its section starts with its tool and runs its passes.
            assert_eq!(
                kinds(&guide, guide.sections[3].steps.clone()),
                ["Tool", "Pass", "Pass", "Pass", "Pass", "Check"]
            );
        }
    }
    #[test]
    fn targets_replay_the_plans_moves_in_order() {
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        let (zero, plan) = (guide.zero, &guide.done[1].plan);
        let work = |p: [f32; 3]| -> [f32; 3] { std::array::from_fn(|k| p[k] - zero[k]) };
        let start = work(plan.moves[0].from);
        let mut moves = plan.moves.iter().filter(|m| m.motion != Motion::Dwell);
        let (mut at, mut n) = (start, 0);
        for step in &guide.steps {
            let Step::Pass {
                targets, index, of, ..
            } = step
            else {
                continue;
            };
            n += 1;
            assert_eq!((*index, *of), (n, 4));
            // The plan's moves since the last cut, this cut last.
            let mut legs = vec![];
            for m in moves.by_ref() {
                let k = moved(m).unwrap();
                legs.push((k, work(m.to)[k]));
                if m.cutting_pass {
                    break;
                }
            }
            let got: Vec<(usize, f32)> = targets.iter().map(|t| (t.axis, t.centre)).collect();
            if n == 1 {
                // The first pass also sets, before the plunge, each axis the
                // plan leaves where it starts (here Y on the stock's front).
                let extra: Vec<(usize, f32)> =
                    got.iter().copied().filter(|g| !legs.contains(g)).collect();
                assert_eq!(extra, [(1, start[1])]);
                assert_eq!(got.len(), legs.len() + 1);
                let plunge = got.iter().rposition(|g| g.0 == 2).unwrap();
                assert_eq!(got[plunge - 1], extra[0]);
                for k in 0..3 {
                    assert!(got[..got.len() - 1].iter().any(|g| g.0 == k));
                }
            } else {
                assert_eq!(got, legs);
            }
            // Applied in order from where the last pass left the tool, they
            // reach the cut's end; their numbers run 1, 2, 3…
            for (i, t) in targets.iter().enumerate() {
                assert_eq!(usize::from(t.order), i + 1);
                at[t.axis] = t.centre;
            }
            let cut = plan
                .moves
                .iter()
                .filter(|m| m.cutting_pass)
                .nth(n - 1)
                .unwrap();
            assert_eq!(at, work(cut.to));
            // The cut is along X with Y and Z locked; Y and Z are locked
            // where they are set for the last time.
            let (cut, setting) = targets.split_last().unwrap();
            assert_eq!((cut.axis, cut.lock), (0, [false, true, true]));
            for (i, t) in setting.iter().enumerate() {
                let again = setting[i + 1..].iter().any(|u| u.axis == t.axis);
                let want = t.axis != 0 && !again;
                assert_eq!(
                    t.lock,
                    std::array::from_fn(|k| k == t.axis && want),
                    "{t:?}"
                );
            }
        }
        assert_eq!(n, 4);
        assert!(moves.all(|m| !m.cutting_pass));
    }
    #[test]
    fn x_and_y_read_the_tool_edge_on_the_comp_side_with_the_dros_compensation() {
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        // Open on both X ends (+), closed on +Y (+).
        assert!(matches!(guide.steps[3], Step::Tool { comp: [1, 1], .. }));
        assert!(matches!(guide.steps[9], Step::Tool { comp: [0, 0], .. }));
        let all = targets(&guide);
        for (_, t) in &all {
            match t.axis {
                2 => assert!(!t.edge && t.shown == t.centre),
                _ => {
                    assert!(t.edge);
                    close(t.shown, t.centre + 12.5);
                }
            }
        }
        // The last row's edge stops on the wall less the stock to leave.
        let (_, y) = all.iter().rev().find(|(_, t)| t.axis == 1).unwrap();
        close(y.shown, 15.);
        close(y.centre, 2.5);
        // Never past the wall's allowance, the floor's, nor below the floor.
        let Step::Pass { never, cut, .. } = &guide.steps[7] else {
            panic!()
        };
        close(never.y[1], 15.);
        close(never.z, -11.5);
        close(cut.max[1], 15.);
        close(cut.min[2], -11.5);
        // Without it, every readout is the centre.
        let (doc, mesh) = job("No");
        let guide = build(&doc, &mesh, 0).unwrap();
        assert!(matches!(guide.steps[3], Step::Tool { comp: [0, 0], .. }));
        for (_, t) in targets(&guide) {
            assert!(!t.edge && t.shown == t.centre, "{t:?}");
        }
    }
    #[test]
    fn the_holes_are_centres_and_depths_in_work_coordinates() {
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        let Step::Holes { op, holes } = &guide.steps[10] else {
            panic!()
        };
        assert_eq!((*op, holes.len()), (2, 4));
        let plan = &guide.done[2].plan;
        for h in holes {
            let at: Vec<&Move> = (plan.moves.iter())
                .filter(|m| {
                    m.cutting_pass
                        && m.to[0] - guide.zero[0] == h[0]
                        && m.to[1] - guide.zero[1] == h[1]
                })
                .collect();
            assert!(!at.is_empty());
            let deepest = at.iter().map(|m| m.to[2]).fold(f32::INFINITY, f32::min);
            assert_eq!(h[2], deepest - guide.zero[2]);
            assert!(
                h[2] < -2. && (0. ..124.).contains(&h[0]) && (20. ..100.).contains(&h[1]),
                "{h:?}"
            );
        }
    }
    #[test]
    fn a_change_of_sides_alone_is_a_comp_step() {
        let (mut doc, mesh) = job("Yes");
        let mill = doc.tools[3].id.clone();
        // Behind the part: closed on −Y.
        let back = boxed(
            "Back step",
            &mill,
            [-500., 500., 104.5, 500., 0., -12.],
            &[("stepdown", "5")],
        );
        doc.setups[0].operations.insert(2, back);
        let guide = build(&doc, &mesh, 0).unwrap();
        let of = |i: usize| kinds(&guide, guide.sections[i].steps.clone());
        assert_eq!(guide.sections[3].name, "Back step");
        assert_eq!(of(3)[0], "Comp");
        assert_eq!(
            guide.steps[guide.sections[3].steps.start],
            Step::Comp {
                op: 2,
                comp: [1, -1]
            }
        );
        // The back step's last row reads its wall less the allowance.
        let pass = |s: &&Step| matches!(s, Step::Pass { op: 2, .. });
        let Some(Step::Pass { targets, .. }) = guide.steps.iter().rfind(pass) else {
            panic!()
        };
        let y = targets.iter().rev().find(|t| t.axis == 1).unwrap();
        close(y.shown, 105.);
        close(y.centre, 117.5);
        // A different speed with the same tool is a Tool step.
        doc.setups[0].operations[2]
            .p
            .insert("rpm".into(), "400".into());
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(kinds(&guide, guide.sections[3].steps.clone())[0], "Tool");
    }
    #[test]
    fn a_pass_shows_the_handwheel_turns_for_its_feed() {
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(manual::turns(&doc.setups[0].machine, 0, 120.), 24.);
        for step in &guide.steps {
            if let Step::Pass {
                turns_per_minute, ..
            } = step
            {
                assert_eq!(*turns_per_minute, 24.);
            }
        }
    }
    #[test]
    fn the_place_is_saved_with_the_project_and_kept_within_the_steps() {
        let (mut doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(place(&doc, &guide), 0);
        let id = doc.setups[0].id.clone();
        doc.guide.insert(id.clone(), 7);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("job.axis.json");
        crate::storage::save(&path, &doc).unwrap();
        let mut doc = crate::storage::load(&path).unwrap();
        assert_eq!(doc.guide.get(&id), Some(&7));
        assert_eq!(place(&doc, &guide), 7);
        // Without the Step the setup has 7 steps: the place is the last.
        doc.setups[0].operations.remove(1);
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(guide.steps.len(), 7);
        assert_eq!(place(&doc, &guide), 6);
        // A project saved without places loads with none.
        let mut json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        json.as_object_mut().unwrap().remove("guide");
        let old: Document = serde_json::from_value(json).unwrap();
        assert!(old.guide.is_empty());
    }
    #[test]
    fn faces_squares_and_comp_sides_follow_the_open_sides() {
        let all = [[true; 2]; 3];
        let (lo, hi, r) = ([0., 0., -10.], [124., 120., 0.], 12.5);
        assert_eq!(kind(all, lo, hi, r), Kind::Face);
        // A wall on −X with a 1 mm slab off it: squaring, floor open or closed.
        let mut wall = all;
        wall[0][0] = false;
        let slab = [123., 0., -10.];
        assert_eq!(kind(wall, slab, hi, r), Kind::Square { axis: 0, low: true });
        assert_eq!(comp_sides(wall), [-1, 1]);
        wall[2][0] = false;
        assert_eq!(kind(wall, slab, hi, r), Kind::Square { axis: 0, low: true });
        // As wide as the radius still is; wider is not.
        assert_eq!(
            kind(wall, [111.5, 0., -10.], hi, r),
            Kind::Square { axis: 0, low: true }
        );
        assert_eq!(kind(wall, [111.4, 0., -10.], hi, r), Kind::Other);
        // The front step: 15 mm off its +Y wall, wider than the radius.
        let mut step = all;
        (step[1][1], step[2][0]) = (false, false);
        let front = [124., 15., 0.];
        assert_eq!(
            (kind(step, lo, front, r), comp_sides(step)),
            (Kind::Other, [1, 1])
        );
        // Two closed sides are never squaring, however thin.
        let mut slot = all;
        slot[1] = [false; 2];
        let thin = ([0., 50., -10.], [124., 60., 0.]);
        assert_eq!(
            (kind(slot, thin.0, thin.1, r), comp_sides(slot)),
            (Kind::Other, [1, 1])
        );
    }
    #[test]
    fn only_a_manual_setup_with_an_enabled_operation_has_steps() {
        let (mut doc, mesh) = job("Yes");
        assert!(build(&doc, &mesh, 1).is_err());
        for op in &mut doc.setups[0].operations {
            op.enabled = false;
        }
        assert!(build(&doc, &mesh, 0).is_err());
    }
}
