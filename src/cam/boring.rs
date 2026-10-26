//! Reaming and Boring (`model::REAMING`, `model::BORING`): a hole an earlier
//! operation drilled or roughed, finished to size on a CNC or the manual
//! mill (whose guide shows each pass, `guide::Step::Bore`). A reamer is fed
//! in and fed back out. A boring head is set to a size and fed down each
//! hole, then leaves it with the spindle stopped or fed out, as the head
//! says: a pass at each size up to the hole's, the program pausing before
//! each pass for the head to be measured and set out (`simulation::Stop`).
//! Both read how far the hole is already open from the stock the operations
//! before them left (`rest::RestStock::bored`), and refuse one that is not.
use super::{Builder, Hole, bored, breakthrough, drill_holes, next_hole, number};
use crate::{
    mesh::Mesh,
    model::{BORE_RETRACTS, Operation, Setup, Tool, get, num},
    simulation::{CutterShape, Motion, Plan, Stop},
};

/// How much a reamer takes off the diameter of the hole drilled for it, mm:
/// 0.1 up to Ø5, 0.2 up to Ø20 and 0.3 over (a Ø10 reamer's hole is drilled
/// Ø9.8).
pub fn ream_allowance(diameter: f32) -> f32 {
    if diameter <= 5. {
        0.1
    } else if diameter <= 20. {
        0.2
    } else {
        0.3
    }
}
/// The drill a Ø`diameter` reamer's hole is drilled with (`ream_allowance`).
pub fn ream_drill(diameter: f32) -> f32 {
    diameter - ream_allowance(diameter)
}
/// The most a reamer takes off the diameter, mm: more loads it, and it
/// cuts the hole oversize.
pub const REAM_MOST: f32 = 0.5;
/// What a hole is roughed to before a boring head finishes it, on the
/// diameter: 0.6 mm less, two passes at the default cuts.
pub const BORE_ALLOWANCE: f32 = 0.6;
/// The size a Ø`diameter` bore is roughed to before the boring head.
pub fn bore_rough(diameter: f32) -> f32 {
    diameter - BORE_ALLOWANCE
}
/// The sizes a boring head is set to, mm, boring a hole open to `from` out
/// to `to`: passes that take at most `cut` off the radius up to `to` less
/// the `finish` cut, each a whole number of the head's `dial` divisions,
/// then `to`.
pub fn bore_passes(from: f32, to: f32, cut: f32, finish: f32, dial: f32) -> Vec<f32> {
    let rough = to - finish;
    let mut sizes: Vec<f32> = vec![];
    if rough > from + 1e-3 && cut > 0. {
        let n = ((rough - from) / (2. * cut) - 1e-4).ceil().max(1.) as usize;
        for i in 1..=n {
            let size = from + (rough - from) * i as f32 / n as f32;
            let size = if dial > 0. {
                (size / dial + 1e-3).floor() * dial
            } else {
                size
            };
            if size > from + 1e-3
                && size < to - 1e-3
                && sizes.last().is_none_or(|l| size > l + 1e-3)
            {
                sizes.push(size);
            }
        }
    }
    sizes.push(to);
    sizes
}
/// How wide the hole at `centre` is open from `top` down to `bottom` before
/// this operation, mm, up to `most`: 0 when it is not open at all. The
/// finishing cycles read it down to the hole's own bottom: past a through
/// hole's exit a drill leaves its point, where nothing is to be finished.
fn opened(setup: &Setup, b: &Builder, centre: [f32; 2], top: f32, bottom: f32, most: f32) -> f32 {
    let open = |r: f32| bored(setup, b, centre, r, top, bottom);
    if !open(0.) {
        return 0.;
    }
    if open(most / 2.) {
        return most;
    }
    let (mut lo, mut hi) = (0., most / 2.);
    // To a thousandth of a millimetre on the radius.
    while hi - lo > 0.001 {
        let mid = (lo + hi) / 2.;
        if open(mid) { lo = mid } else { hi = mid }
    }
    2. * lo
}
/// How far under its size `opened` may read a hole, mm on the diameter,
/// with room to spare: it reads to a thousandth on the radius, from below.
const OPEN_READ: f32 = 0.005;
/// An operation's value of `key`, or `default` when it is blank (saved
/// before the field was).
fn value_or(op: &Operation, key: &str, default: f32) -> Result<f32, String> {
    if get(&op.p, key).trim().is_empty() {
        Ok(default)
    } else {
        number(op, key)
    }
}
/// The dwell at the bottom of each hole, 0–600 seconds.
fn dwell(op: &Operation) -> Result<f32, String> {
    let dwell = value_or(op, "dwell", 0.)?;
    if !(0. ..=600.).contains(&dwell) {
        return Err("Dwell must be 0–600 seconds.".into());
    }
    Ok(dwell)
}
/// Where a finishing cycle cuts hole `h`: from where the material over it
/// starts down to its bottom, past its exit by Breakthrough when it goes
/// through; refused when it is blind and Through is on.
fn depths(
    setup: &Setup,
    b: &Builder,
    op: &Operation,
    h: &Hole,
    radius: f32,
    (through, breakthrough): (bool, f32),
) -> Result<(f32, f32), String> {
    if through && !h.through {
        return Err(format!(
            "The Ø{:.2} hole at X{:.2} Y{:.2} is blind: the part goes on under its bottom, so it cannot be finished through. Set Through to No, or use the diameter filter to finish only the holes that go through.",
            h.diameter, h.center[0], h.center[1]
        ));
    }
    let lowest = h.bottom - breakthrough;
    let target = lowest + number(op, "bottom")?;
    if target >= h.top
        || target < lowest - 0.001
        || target < b.plan.stock.min[2] - breakthrough - 0.001
    {
        return Err("The bottom offset must leave the tool in the hole, and cannot take it below the hole's bottom: only Through does, below a hole that goes through the part.".into());
    }
    let top = b.plan.stock.max[2] + number(op, "top")?.max(0.);
    let cutting_top = crate::rest::start_over(
        setup,
        &crate::rest::Reach::disc(h.center, radius),
        target,
        top,
        number(op, "top")?,
    );
    Ok((cutting_top, target))
}
/// A mill's: a CNC's, or the manual mill's, whose moves go one axis at a
/// time (`Builder::push`).
fn on_mill(setup: &Setup) -> Result<(), String> {
    if crate::lathe::is_lathe(&setup.machine) {
        return Err(crate::model::FINISH_ON_LATHE.into());
    }
    Ok(())
}
/// Reaming (`REAMING`): each hole of the reamer's size, drilled a little
/// under it first (`ream_allowance`), reamed from where its material starts
/// down to its bottom (past its exit by Breakthrough when it goes through),
/// a dwell there, and fed back out the same way: a reamer pulled out at a
/// rapid scores the wall it has just cut.
pub(super) fn ream(
    setup: &Setup,
    op: &Operation,
    tool: &Tool,
    mesh: &Mesh,
) -> Result<Plan, String> {
    let mut b = Builder::new(setup, op, tool, mesh)?;
    if get(&tool.p, "type") != "Reamer" {
        return Err("Choose a reamer for Reaming.".into());
    }
    on_mill(setup)?;
    let mut holes = drill_holes(setup, op, mesh)?;
    let ends = breakthrough(op)?;
    let dwell = dwell(op)?;
    let size = 2. * b.plan.radius;
    b.plan.shape = CutterShape::Flat;
    b.plan.milling_direction = crate::forces::REAMING_DIRECTION;
    let count = holes.len();
    crate::progress::expect(count);
    while !holes.is_empty() {
        crate::progress::report(count - holes.len())?;
        let h = next_hole(&mut holes, b.pos, op);
        let at = format!(
            "Ø{:.2} hole at X{:.2} Y{:.2}",
            h.diameter, h.center[0], h.center[1]
        );
        // As for a drill, the bore may be read a sagitta inside its size.
        if (size - h.diameter).abs() > 0.05 + 2. * h.sagitta {
            return Err(format!(
                "The {at} is not this Ø{size:.2} reamer's size. Choose a Ø{:.2} reamer, or a diameter filter.",
                h.diameter
            ));
        }
        let (cutting_top, target) = depths(setup, &b, op, &h, size / 2., ends)?;
        if cutting_top - target >= b.plan.flute_length {
            return Err(format!(
                "The {at} is reamed {:.1} mm deep, as deep as the reamer's {} mm flutes or deeper. Choose a reamer with longer flutes.",
                cutting_top - target,
                get(&tool.p, "length")
            ));
        }
        let open = opened(setup, &b, h.center, h.top, target.max(h.bottom), size);
        if open < size - REAM_MOST {
            return Err(format!(
                "Drill the {at} first: Ø{:.1}, {:.1} deep. It is open to Ø{open:.2}, and a reamer takes {}–{} off the diameter.",
                ream_drill(size),
                h.top - target,
                ream_allowance(size),
                REAM_MOST
            ));
        }
        let lower = (cutting_top < b.plan.stock.max[2] - 0.0001).then_some(cutting_top);
        b.plan.rest_top = if b.plan.levels == 0 {
            lower
        } else {
            b.plan.rest_top.zip(lower).map(|(a, b)| a.max(b))
        };
        b.start_at([h.center[0], h.center[1], cutting_top], Some(cutting_top))?;
        let rate = b.feed;
        b.push([h.center[0], h.center[1], target], Motion::Feed, true, rate)?;
        b.dwell(dwell);
        b.push(
            [h.center[0], h.center[1], cutting_top],
            Motion::Feed,
            false,
            rate,
        )?;
        b.plan.target = b.plan.target.min(target);
        b.plan.levels += 1;
        b.retract()?;
    }
    b.finish()
}
/// What the operator is told before boring pass `k` of `sizes`: to set the
/// head to the first size, and before each later one to measure the bore
/// the last left and set the head out to the next, by how much and how
/// many of its `dial` divisions.
pub fn adjust_message(sizes: &[f32], k: usize, dial: f32) -> String {
    let size = sizes[k];
    if k == 0 {
        return format!(
            "Boring head: set it to Ø{size:.2}, pass 1 of {}, then resume.",
            sizes.len()
        );
    }
    let (before, out) = (sizes[k - 1], size - sizes[k - 1]);
    let divisions = if dial > 0. {
        format!(", {} divisions of {dial},", (out / dial).round())
    } else {
        String::new()
    };
    format!(
        "Boring head: measure the bore, Ø{before:.2} expected. Set the head out {out:.2} on the diameter{divisions} to Ø{size:.2}, pass {} of {}, then resume.",
        k + 1,
        sizes.len()
    )
}
/// Boring (`BORING`): the holes of one size (its Hole diameter), each open
/// at least to the head's smallest bore from an earlier operation, bored
/// out to their size in passes (`bore_passes`) with a boring head that sets
/// to it and whose bar reaches their depth below its body. Each pass bores
/// every hole: from where its material starts down to its bottom (past its
/// exit by Breakthrough when it goes through), a dwell there, then out with
/// the spindle stopped or fed out as the head says. Before each pass the
/// program stops the spindle at the clearance height and pauses for the
/// head to be set (`adjust_message`). Its sweep is the bore's: a spinning
/// bar sweeps the circle its edge is set to, up to its reach, and the head
/// above it.
pub(super) fn bore(
    setup: &Setup,
    op: &Operation,
    tool: &Tool,
    mesh: &Mesh,
) -> Result<Plan, String> {
    let mut b = Builder::new(setup, op, tool, mesh)?;
    if get(&tool.p, "type") != "Boring head" {
        return Err("Choose a boring head for Boring.".into());
    }
    on_mill(setup)?;
    let (low, high) = tool
        .bore_range()
        .ok_or("Enter the boring head's bore range, like 12-50.")?;
    let dial = num(&tool.p, "dial")
        .filter(|d| *d > 0.)
        .ok_or("Enter the boring head's dial graduation.")?;
    let retract = get(&tool.p, "bore_retract");
    if !BORE_RETRACTS.contains(&retract) {
        return Err("Choose how the boring head leaves the bore.".into());
    }
    let feed_out = retract == BORE_RETRACTS[1];
    if !num(&op.p, "hole_diameter").is_some_and(|d| d > 0.) {
        return Err(crate::model::BORE_ONE_SIZE.into());
    }
    let cut = value_or(op, "bore_cut", 0.3)?;
    let finish = value_or(op, "finish_cut", 0.1)?;
    if cut <= 0. || finish < 0. {
        return Err("Bore cut must be above zero and finish cut zero or more.".into());
    }
    let mut holes = drill_holes(setup, op, mesh)?;
    let ends = breakthrough(op)?;
    let dwell = dwell(op)?;
    // The holes' size, as the model has it.
    let size =
        (holes.iter().map(|h| h.diameter).sum::<f32>() / holes.len() as f32 * 100.).round() / 100.;
    if size < low - 1e-3 || size > high + 1e-3 {
        return Err(format!(
            "The Ø{size:.2} holes are outside the boring head's Ø{low}–{high} range. Choose a head that sets to Ø{size:.2}."
        ));
    }
    let reach = b.plan.flute_length;
    let body = b.plan.radius;
    b.plan.radius = size / 2.;
    b.plan.shape = CutterShape::Flat;
    b.plan.neck = Some([size / 2., reach, body]);
    b.plan.milling_direction = crate::forces::BORING_DIRECTION;
    // Each hole, where it is cut and how far it is open, in drilling order.
    let mut ordered = vec![];
    let mut pos = b.pos;
    while !holes.is_empty() {
        let h = next_hole(&mut holes, pos, op);
        pos = [h.center[0], h.center[1], pos[2]];
        let at = format!(
            "Ø{:.2} hole at X{:.2} Y{:.2}",
            h.diameter, h.center[0], h.center[1]
        );
        let (cutting_top, target) = depths(setup, &b, op, &h, size / 2., ends)?;
        // The head's body stays clear above the hole's rim and the material
        // round it.
        let mouth = cutting_top.max(h.top);
        if target + reach < mouth + 0.5 {
            return Err(format!(
                "The {at} is bored {:.1} mm deep: the boring head's bar reaches {reach} mm below its body, which would hit the part. Choose a longer bar, or a head with one.",
                mouth - target
            ));
        }
        let open = opened(setup, &b, h.center, h.top, target.max(h.bottom), size);
        // `opened` reads up to 0.002 under on the diameter: a hole drilled
        // at the head's smallest bore is open to it.
        if open < low - OPEN_READ {
            return Err(format!(
                "Rough the {at} first: it is open to Ø{open:.2}, and the boring head's smallest bore is Ø{low}. Drill it, or mill it to Ø{:.1} (Helical milling), before boring.",
                bore_rough(size)
            ));
        }
        ordered.push((h, cutting_top, target, open));
    }
    let from = ordered.iter().map(|o| o.3).fold(f32::INFINITY, f32::min);
    let sizes = bore_passes(from.max(low), size, cut, finish, dial);
    let rate = b.feed;
    crate::progress::expect(sizes.len() * ordered.len());
    for k in 0..sizes.len() {
        // Set at the clearance height, the spindle stopped.
        b.retract()?;
        b.plan.stops.push(Stop {
            at: b.plan.moves.len(),
            pause: Some(adjust_message(&sizes, k, dial)),
            size: Some(sizes[k]),
        });
        for (i, (h, cutting_top, target, _)) in ordered.iter().enumerate() {
            crate::progress::report(k * ordered.len() + i)?;
            let (c, top, bottom) = (h.center, *cutting_top, *target);
            let lower = (top < b.plan.stock.max[2] - 0.0001).then_some(top);
            b.plan.rest_top = if b.plan.levels == 0 {
                lower
            } else {
                b.plan.rest_top.zip(lower).map(|(a, b)| a.max(b))
            };
            b.start_at([c[0], c[1], top], Some(top))?;
            b.push([c[0], c[1], bottom], Motion::Feed, true, rate)?;
            b.dwell(dwell);
            if feed_out {
                b.push([c[0], c[1], top], Motion::Feed, false, rate)?;
            } else {
                b.plan.stops.push(Stop {
                    at: b.plan.moves.len(),
                    pause: None,
                    size: None,
                });
            }
            b.plan.target = b.plan.target.min(bottom);
            b.plan.levels += 1;
            b.retract()?;
        }
    }
    b.finish()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        model::{BORE_ONE_SIZE, BORING, Document, Machine, REAMING, Strategy},
        simulation::Simulation,
        surface::Selection,
    };

    /// A Ø`outer` ring `height` tall with a Ø`bore` hole straight through
    /// it, 64 sides.
    pub(crate) fn ring(bore: f32, outer: f32, height: f32) -> Mesh {
        let (r, o) = (bore / 2., outer / 2.);
        let section = [(o, height), (r, height), (r, 0.), (o, 0.), (o, height)];
        crate::mesh::normalize(crate::cam::tests::revolved(&section)).unwrap()
    }
    pub(crate) fn drill(d: &str) -> Tool {
        let mut t = Tool::new(&format!("Drill {d}"), "Drill", d);
        t.p.insert("length".into(), "40".into());
        t.p.insert("stickout".into(), "45".into());
        t
    }
    pub(crate) fn end_mill(d: &str) -> Tool {
        let mut t = Tool::new(&format!("End mill {d}"), "Flat end mill", d);
        t.p.insert("length".into(), "35".into());
        t.p.insert("stickout".into(), "45".into());
        t
    }
    pub(crate) fn reamer(d: &str) -> Tool {
        let mut t = Tool::new(&format!("Reamer {d} H7"), "Reamer", d);
        t.set_type("Reamer");
        for (k, v) in [("flutes", "6"), ("length", "30"), ("stickout", "50")] {
            t.p.insert(k.into(), v.into());
        }
        t
    }
    /// A Ø50 boring head that bores Ø12–50 with a Ø10 bar reaching 40 mm
    /// below its body, leaving each bore as `retract` says.
    pub(crate) fn head(retract: &str) -> Tool {
        let mut t = Tool::new("Boring head", "Boring head", "50");
        t.set_type("Boring head");
        for (k, v) in [
            ("flutes", "1"),
            ("length", "40"),
            ("stickout", "90"),
            ("bore_retract", retract),
        ] {
            t.p.insert(k.into(), v.into());
        }
        t
    }
    /// The top face's holes of `mesh`, as a Drill picks them.
    fn select_top(op: &mut Operation, mesh: &Mesh) {
        let flat = |s: &&crate::surface::Surface| s.normal[2] > 0.999999;
        let top = (mesh.surfaces.iter())
            .filter(flat)
            .map(|s| s.center[2])
            .fold(f32::NEG_INFINITY, f32::max);
        let ids = (mesh.surfaces.iter().enumerate())
            .filter(|(_, s)| flat(s) && (s.center[2] - top).abs() < 0.001)
            .map(|(i, _)| i)
            .collect();
        op.geometry.clear();
        op.surfaces = Some(Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids,
        });
    }
    /// A Drill of `mesh`'s top holes, through, with `tool` and `cycle`.
    pub(crate) fn drilling(mesh: &Mesh, tool: &Tool, cycle: &str) -> Operation {
        let mut op = Operation::new(Strategy::Drill, tool.id.clone());
        select_top(&mut op, mesh);
        op.p.insert("cycle".into(), cycle.into());
        op.p.insert("through".into(), "Yes".into());
        op.p.insert("breakthrough".into(), "1".into());
        op
    }
    /// One setup on `machine` cutting `mesh` with `ops` (each with its
    /// tool, numbered in order), and each operation's simulation.
    pub(crate) fn job(
        mesh: &Mesh,
        machine: Machine,
        ops: Vec<(Tool, Operation)>,
    ) -> (Document, Vec<Result<Simulation, String>>) {
        let mut setup = Setup::new(machine.clone(), "G54");
        for k in [
            "stock_top",
            "stock_left",
            "stock_right",
            "stock_front",
            "stock_back",
        ] {
            setup.p.insert(k.into(), "2".into());
        }
        // Room under a through hole for a drill's point.
        setup.p.insert("stock_bottom".into(), "5".into());
        let mut doc = Document::default();
        for (n, (mut tool, op)) in ops.into_iter().enumerate() {
            tool.p.insert("number".into(), (n + 1).to_string());
            doc.tools.push(tool);
            setup.operations.push(op);
        }
        doc.machines = vec![machine];
        doc.active = setup.id.clone();
        doc.setups = vec![setup];
        let mut all = vec![];
        crate::simulation::sequence(&doc, mesh, None, |_, _, r| all.push(r));
        (doc, all)
    }
    /// The hub's bush hole: a Ø10 hole through 8.7, drilled Ø`drilled`
    /// first (none for None), then reamed with `tool`.
    fn reamed(drilled: Option<&str>, tool: Tool) -> Result<Simulation, String> {
        let mesh = ring(10., 40., 8.7);
        let mut ops = vec![];
        if let Some(d) = drilled {
            let t = drill(d);
            let mut op = drilling(&mesh, &t, "Drilling");
            op.p.insert("breakthrough".into(), "3.5".into());
            ops.push((t, op));
        }
        let op = drilling(&mesh, &tool, REAMING);
        ops.push((tool, op));
        let (_, mut all) = job(&mesh, Machine::new(3), ops);
        let last = all.pop().unwrap();
        for r in &all {
            assert!(r.is_ok(), "{:?}", r.as_ref().err());
        }
        last
    }
    /// The plate's bearing bore: Ø32 through 28, drilled Ø12, milled out to
    /// Ø31.4 with a Ø10 end mill when `roughed`, then bored with `tool`,
    /// its operation edited by `edit`.
    pub(crate) fn bored(
        roughed: bool,
        tool: Tool,
        edit: impl Fn(&mut Operation),
    ) -> (Document, Mesh, Vec<Result<Simulation, String>>) {
        let mesh = ring(32., 60., 28.);
        let t = drill("12");
        let mut op = drilling(&mesh, &t, "Deep drilling");
        op.p.insert("peck".into(), "12".into());
        op.p.insert("breakthrough".into(), "4.5".into());
        let mut ops = vec![(t, op)];
        if roughed {
            let t = end_mill("10");
            let mut op = drilling(&mesh, &t, crate::model::HELICAL_MILLING);
            op.p.insert("bore_allowance".into(), "0.6".into());
            op.p.insert("rpm".into(), "8000".into());
            op.p.insert("feed".into(), "800".into());
            ops.push((t, op));
        }
        let mut op = drilling(&mesh, &tool, BORING);
        op.p.insert("hole_diameter".into(), "32".into());
        edit(&mut op);
        ops.push((tool, op));
        let (doc, all) = job(&mesh, Machine::new(3), ops);
        (doc, mesh, all)
    }

    #[test]
    fn a_reamer_is_paired_with_a_drill_under_it_and_a_bore_with_a_rough_size() {
        assert!((ream_drill(10.) - 9.8).abs() < 1e-5);
        assert!((ream_drill(4.) - 3.9).abs() < 1e-5);
        assert!((ream_drill(25.) - 24.7).abs() < 1e-5);
        assert!((bore_rough(32.) - 31.4).abs() < 1e-5);
        // Roughed to Ø31.4: one pass to 0.1 under, then the size.
        let sizes = bore_passes(31.4, 32., 0.3, 0.1, 0.01);
        assert_eq!(sizes.len(), 2, "{sizes:?}");
        assert!(
            (sizes[0] - 31.9).abs() < 1e-4 && sizes[1] == 32.,
            "{sizes:?}"
        );
        // From a Ø12 drilling: no pass takes more than 0.5 off the radius,
        // each on the dial, and the last is the size.
        let sizes = bore_passes(12., 32., 0.5, 0.1, 0.02);
        assert_eq!(*sizes.last().unwrap(), 32.);
        let mut before = 12.;
        for s in &sizes[..sizes.len() - 1] {
            assert!(*s > before && *s - before <= 1. + 1e-4, "{sizes:?}");
            assert!(((s / 0.02).round() * 0.02 - s).abs() < 1e-4, "{sizes:?}");
            before = *s;
        }
        // Already at the size: one pass, a spring pass.
        assert_eq!(bore_passes(32., 32., 0.3, 0.1, 0.01), vec![32.]);
    }
    #[test]
    fn a_reamer_feeds_in_dwells_and_feeds_back_out() {
        let sim = reamed(Some("9.8"), reamer("10")).unwrap();
        let plan = &sim.plan;
        assert!((plan.radius - 5.).abs() < 1e-5 && plan.stops.is_empty());
        let dwell = (plan.moves.iter())
            .position(|m| m.motion == Motion::Dwell)
            .expect("a dwell at the bottom");
        let (down, out) = (&plan.moves[dwell - 1], &plan.moves[dwell + 1]);
        // Down to a millimetre past the exit (the part's bottom, 5 mm over
        // the stock's), and back up at the same feed.
        assert_eq!(down.motion, Motion::Feed);
        assert!(down.cutting_pass && (down.to[2] - (plan.stock.min[2] + 4.)).abs() < 0.01);
        assert!((plan.moves[dwell].end - plan.moves[dwell].start - 0.5).abs() < 1e-4);
        assert_eq!(out.motion, Motion::Feed);
        assert!(!out.cutting_pass && out.to[2] > down.from[2] - 0.01);
        assert_eq!(out.feed, down.feed);
        assert_eq!(plan.milling_direction, crate::forces::REAMING_DIRECTION);
    }
    #[test]
    fn a_reamer_needs_its_hole_drilled_under_it_its_size_and_flutes_as_deep() {
        let err = reamed(None, reamer("10")).unwrap_err();
        assert!(err.contains("Drill the") && err.contains("Ø9.8"), "{err}");
        // More than half a millimetre under: too much for the reamer.
        let err = reamed(Some("9"), reamer("10")).unwrap_err();
        assert!(
            err.contains("Drill the") && err.contains("open to Ø9.0"),
            "{err}"
        );
        let err = reamed(Some("9.8"), reamer("12")).unwrap_err();
        assert!(err.contains("not this Ø12.00 reamer"), "{err}");
        let mut short = reamer("10");
        short.p.insert("length".into(), "8".into());
        let err = reamed(Some("9.8"), short).unwrap_err();
        assert!(err.contains("flutes"), "{err}");
    }
    #[test]
    fn boring_bores_in_passes_pausing_to_set_the_head_and_stops_the_spindle_before_the_retract() {
        let (_, _, all) = bored(true, head(BORE_RETRACTS[0]), |_| {});
        let sim = all.last().unwrap().as_ref().unwrap();
        let plan = &sim.plan;
        // The bore's sweep, and the head's body above its bar's reach.
        assert!((plan.radius - 16.).abs() < 1e-5);
        assert_eq!(plan.neck, Some([16., 40., 25.]));
        let pauses: Vec<&Stop> = plan.stops.iter().filter(|s| s.pause.is_some()).collect();
        assert_eq!(pauses.len(), 2, "{:?}", plan.stops);
        let said = |s: &Stop| s.pause.clone().unwrap();
        assert!(
            said(pauses[0]).contains("Ø31.90, pass 1 of 2"),
            "{}",
            said(pauses[0])
        );
        let second = said(pauses[1]);
        assert!(
            second.contains("Ø31.90 expected")
                && second.contains("out 0.10")
                && second.contains("10 divisions of 0.01")
                && second.contains("Ø32.00, pass 2 of 2"),
            "{second}"
        );
        // Each pass: fed down past the exit, a dwell, the spindle stopped,
        // then a rapid out; nothing feeds while it is stopped.
        let stopped: Vec<&Stop> = plan.stops.iter().filter(|s| s.pause.is_none()).collect();
        assert_eq!(stopped.len(), 2);
        for s in stopped {
            assert_eq!(plan.moves[s.at - 1].motion, Motion::Dwell);
            assert_eq!(plan.moves[s.at - 2].motion, Motion::Feed);
            let out = &plan.moves[s.at];
            assert!(out.motion == Motion::Rapid && out.to[2] > out.from[2]);
            let restart = plan.moves[s.at..]
                .iter()
                .position(|m| m.motion == Motion::Feed)
                .map(|i| s.at + i);
            let next_stop = plan.stops.iter().map(|o| o.at).find(|at| *at > s.at);
            assert!(restart.is_none_or(|r| next_stop.is_none_or(|n| n <= r)));
        }
        // Feeding out instead: no stop but the pauses, a feed up after
        // each dwell.
        let (_, _, all) = bored(true, head(BORE_RETRACTS[1]), |_| {});
        let plan = &all.last().unwrap().as_ref().unwrap().plan;
        assert!(plan.stops.iter().all(|s| s.pause.is_some()));
        for (i, m) in plan.moves.iter().enumerate() {
            if m.motion == Motion::Dwell {
                let out = &plan.moves[i + 1];
                assert!(out.motion == Motion::Feed && out.to[2] > out.from[2]);
            }
        }
    }
    #[test]
    fn boring_from_the_drilled_size_takes_more_passes_with_the_saved_defaults() {
        // An operation saved before the cut fields: the defaults, 0.3 on the
        // radius a pass from the Ø12 drilling, the last 0.1.
        let (_, _, all) = bored(false, head(BORE_RETRACTS[0]), |op| {
            op.p.remove("bore_cut");
            op.p.remove("finish_cut");
        });
        let plan = &all.last().unwrap().as_ref().unwrap().plan;
        let pauses = plan.stops.iter().filter(|s| s.pause.is_some()).count();
        assert_eq!(pauses, 1 + ((31.9f32 - 12.) / 0.6).ceil() as usize);
        // The Ø12 drilling reads a hair under Ø12, and a head whose
        // smallest bore is a hair over still bores it: the real plate's Ø12
        // drilling read Ø11.998 and was refused.
        let mut fine = head(BORE_RETRACTS[0]);
        fine.p.insert("bore_range".into(), "12.002-50".into());
        let (_, _, all) = bored(false, fine, |_| {});
        let plan = &all.last().unwrap().as_ref().unwrap().plan;
        let first = plan.stops[0].size.unwrap();
        assert!(first > 12.002, "{first}");
    }
    #[test]
    fn boring_refuses_a_head_out_of_range_or_short_a_hole_not_roughed_and_two_sizes() {
        let err = |roughed: bool, tool: Tool, edit: &dyn Fn(&mut Operation)| {
            let (_, _, all) = bored(roughed, tool, edit);
            all.last().unwrap().as_ref().unwrap_err().clone()
        };
        let mut small = head(BORE_RETRACTS[0]);
        small.p.insert("bore_range".into(), "12-30".into());
        assert!(err(true, small, &|_| {}).contains("outside the boring head's Ø12–30 range"));
        let mut short = head(BORE_RETRACTS[0]);
        short.p.insert("length".into(), "25".into());
        let e = err(true, short, &|_| {});
        assert!(e.contains("reaches 25 mm below its body"), "{e}");
        // Drilled Ø12 only, with a head that bores from Ø20 up.
        let mut big = head(BORE_RETRACTS[0]);
        big.p.insert("bore_range".into(), "20-50".into());
        let e = err(false, big, &|_| {});
        assert!(e.contains("Rough the") && e.contains("Ø31.4"), "{e}");
        let e = err(true, head(BORE_RETRACTS[0]), &|op| {
            op.p.insert("hole_diameter".into(), "0".into());
        });
        assert_eq!(e, BORE_ONE_SIZE);
    }
    /// `doc` as a project of an imported part, which the post requires.
    pub(crate) fn imported(mut doc: Document) -> Document {
        doc.file = Some(crate::model::ModelFile {
            name: "ring.3mf".into(),
            path: "ring.3mf".into(),
            format: "3mf".into(),
            bytes: 0,
            thickness: None,
        });
        doc
    }
    /// `doc` imported (`imported`), its setup on a UC100 with the driver's
    /// template profile, written in `dir`: a setup a run checks.
    pub(crate) fn on_uc100(doc: Document, dir: &std::path::Path) -> Document {
        let mut doc = imported(doc);
        let profile = dir.join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let m = &mut doc.setups[0].machine;
        m.p.insert("controller".into(), "UC100".into());
        m.p.insert("uc100_profile".into(), profile.display().to_string());
        doc.machines = vec![m.clone()];
        doc
    }
    #[test]
    fn the_bore_posts_spindle_stops_and_pauses_and_the_run_lowers_them_in_order() {
        let (doc, mesh, all) = bored(true, head(BORE_RETRACTS[0]), |_| {});
        assert!(
            all.iter().all(|r| r.is_ok()),
            "{:?}",
            all.iter().find_map(|r| r.as_ref().err())
        );
        let job = crate::post::post_job(&imported(doc.clone()), &mesh);
        let programs = job.setups[0].result.as_ref().unwrap();
        let text = &programs[0].text;
        // Set to the first size with the spindle stopped, then started.
        let first = "(MSG, Boring head: set it to D31.90, pass 1 of 2, then resume.)\nM0\nS";
        assert!(text.contains(first), "{text}");
        assert!(
            text.contains("(MSG, Boring head: measure the bore, D31.90 expected."),
            "{text}"
        );
        assert_eq!(text.matches("\nM0\n").count(), 2, "{text}");
        // After each pass's dwell the spindle stops before the rapid out.
        let lines: Vec<&str> = text.lines().collect();
        let dwells: Vec<usize> = (0..lines.len())
            .filter(|&i| lines[i] == "G4 P0.500")
            .collect();
        assert_eq!(dwells.len(), 2, "{text}");
        for i in dwells {
            let stop = (i + 1..lines.len()).find(|&k| lines[k] == "M5").unwrap();
            assert!(
                lines[i + 1..stop].iter().all(|l| l.starts_with('M')),
                "{:?}",
                &lines[i..stop + 2]
            );
            assert!(
                lines[stop + 1].starts_with("G0 Z"),
                "{:?}",
                &lines[i..stop + 2]
            );
        }
        // The UC100 run: the same, as the driver's actions.
        use uc100::gcode::{Action as Do, Spindle};
        let dir = tempfile::tempdir().unwrap();
        let checked = crate::run::programs(&on_uc100(doc, dir.path()), &mesh, 0).unwrap();
        let p = checked
            .iter()
            .find(|p| {
                p.blocks
                    .iter()
                    .any(|b| matches!(b, crate::post::Block::Pause { .. }))
            })
            .unwrap();
        let actions = crate::run::lower(p, 0, [100., 100., -100., 0., 0., 0.], [0.; 6]).unwrap();
        let mut on = false;
        let mut pauses = vec![];
        for l in &actions {
            match &l.action {
                Do::Spindle { state, .. } => on = *state != Spindle::Off,
                Do::Pause { reason } => {
                    assert!(!on, "paused with the spindle on: {reason}");
                    pauses.push(reason.clone());
                }
                Do::Move { feed: Some(_), .. } => assert!(on, "a feed with the spindle off"),
                _ => {}
            }
        }
        assert_eq!(pauses.len(), 2);
        assert!(pauses[1].contains("set the head out") || pauses[1].contains("Set the head out"));
    }
    #[test]
    fn a_reaming_posts_a_feed_out_after_its_dwell() {
        let mesh = ring(10., 40., 8.7);
        let (t, r) = (drill("9.8"), reamer("10"));
        let mut d = drilling(&mesh, &t, "Drilling");
        d.p.insert("breakthrough".into(), "3.5".into());
        let ream = drilling(&mesh, &r, REAMING);
        let (doc, all) = job(&mesh, Machine::new(3), vec![(t, d), (r, ream)]);
        assert!(
            all.iter().all(|r| r.is_ok()),
            "{:?}",
            all.iter().find_map(|r| r.as_ref().err())
        );
        let job = crate::post::post_job(&imported(doc), &mesh);
        let text = &job.setups[0].result.as_ref().unwrap()[0].text;
        let lines: Vec<&str> = text.lines().collect();
        let i = lines.iter().position(|l| *l == "G4 P0.500").unwrap();
        // Fed up (G1 is modal), not a rapid, and no spindle stop.
        assert!(
            lines[i + 1].starts_with('Z') || lines[i + 1].starts_with("G1"),
            "{:?}",
            &lines[i..i + 3]
        );
        assert!(!text.contains("M0\n"), "{text}");
    }
}
