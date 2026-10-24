//! Add the drilling before a Reaming or a Boring: the operations that open
//! its holes for it, on the same holes. A reamer's holes are drilled a
//! little under its size (`cam::boring::ream_drill`, Ø9.8 for Ø10); a
//! boring head's are drilled as wide as the library's drills go, then
//! milled out to the size the head finishes from (`cam::boring::bore_rough`)
//! by Helical milling with the widest flat end mill that reaches, which
//! leaves the rest on the diameter (its `bore_allowance`). Tools come from
//! the library, as Add threading's do; one that is missing is added
//! without it, and the status says which tool to add.
use crate::{
    cam::{
        self,
        boring::{REAM_MOST, bore_rough, ream_drill},
    },
    mesh::Mesh,
    model::{Document, HELICAL_MILLING, Machine, Operation, Strategy, Tool, get, num},
};

/// What Add the drilling adds before operation `oi` of setup `si`, and
/// what to tell the user.
pub struct Added {
    pub ops: Vec<Operation>,
    pub status: String,
}

/// The keys of a finishing operation its stages share: the holes they cut
/// and how, from what height.
const SHARED: [&str; 9] = [
    "holes",
    "hole_diameter",
    "hole_order",
    "through",
    "breakthrough",
    "coolant",
    "clearance",
    "retract",
    "top",
];

/// The operations that open the holes of Reaming or Boring operation `oi`
/// of setup `si` before it, or why there are none.
pub fn add(doc: &Document, si: usize, oi: usize, mesh: &Mesh) -> Result<Added, String> {
    let setup = doc.setups.get(si).ok_or("Choose a setup.")?;
    let op = setup.operations.get(oi).ok_or("Choose an operation.")?;
    if !op.finishes_holes() {
        return Err("Choose a Reaming or a Boring operation.".into());
    }
    let machine = &setup.machine;
    let tool = doc.tools.iter().find(|t| t.id == op.tool);
    // The size they finish to: the reamer's, or the Boring's one size.
    let size = if op.reaming() {
        tool.and_then(|t| num(&t.p, "diameter"))
            .filter(|d| *d > 0.)
            .ok_or("Choose the reamer first: its size says what to drill.")?
    } else {
        num(&op.p, "hole_diameter")
            .filter(|d| *d > 0.)
            .ok_or(crate::model::BORE_ONE_SIZE)?
    };
    let mut probe = op.clone();
    probe.p.insert("hole_diameter".into(), format!("{size:.2}"));
    let holes: Vec<cam::Hole> = cam::holes(setup, &probe, mesh)?
        .into_iter()
        .filter(|h| (h.diameter - size).abs() <= 0.1)
        .collect();
    if holes.is_empty() {
        return Err(format!(
            "No selected hole is Ø{size:.2}: select the holes it finishes."
        ));
    }
    let depth = holes.iter().map(|h| h.top - h.bottom).fold(0., f32::max)
        + if get(&op.p, "through") == "Yes" {
            num(&op.p, "breakthrough").unwrap_or(1.).max(0.)
        } else {
            0.
        };
    let stage = |tool: Option<&Tool>, name: String| {
        let mut o = Operation::for_machine(
            Strategy::Drill,
            tool.map(|t| t.id.clone()).unwrap_or_default(),
            machine,
        );
        o.geometry = op.geometry.clone();
        o.surfaces = op.surfaces.clone();
        for k in SHARED {
            o.p.insert(k.into(), get(&op.p, k).into());
        }
        o.p.insert("hole_diameter".into(), format!("{size:.2}"));
        o.p.insert("name".into(), name);
        o
    };
    // Under the stock is the vise or the table: nothing goes below it.
    let below = crate::stock::resolve(setup, Some(mesh)).map_or(f32::INFINITY, |r| {
        (holes.iter().map(|h| h.bottom).fold(f32::INFINITY, f32::min) - r.stock.min[2]).max(0.)
    });
    // A drill's cycle by how deep it goes: pecking out the chips past three
    // diameters. Through a hole that goes through, its 118° point goes out
    // past the exit, so the finish finds the hole full size all the way, as
    // far as the stock's bottom lets it.
    let drilled = |o: &mut Operation, d: f32| {
        if get(&o.p, "through") == "Yes" {
            let given = num(&o.p, "breakthrough").unwrap_or(1.);
            let past = given.max((0.3 * d + 0.5).min(below - 0.1));
            o.p.insert(
                "breakthrough".into(),
                format!("{:.1}", (past * 10.).ceil() / 10.),
            );
        }
        if depth > 3. * d {
            o.p.insert("cycle".into(), "Deep drilling".into());
            o.p.insert("peck".into(), format!("{:.1}", d));
        }
    };
    let label = format!("Ø{}", trim(size));
    let mut ops = vec![];
    let mut missing = vec![];
    if op.reaming() {
        let want = ream_drill(size);
        let drill = drills(&doc.tools, machine, depth)
            .filter(|(_, d)| *d >= size - REAM_MOST && *d <= size - 0.05)
            .min_by(|a, b| (a.1 - want).abs().total_cmp(&(b.1 - want).abs()));
        let mut hole = stage(drill.map(|(t, _)| t), format!("{label} · drill"));
        drilled(&mut hole, drill.map_or(want, |(_, d)| d));
        if drill.is_none() {
            missing.push(format!("a Ø{} drill", trim(want)));
        }
        ops.push(hole);
    } else {
        let rough = bore_rough(size);
        let drill = drills(&doc.tools, machine, depth)
            .filter(|(_, d)| *d <= rough)
            .max_by(|a, b| a.1.total_cmp(&b.1));
        let mut hole = stage(drill.map(|(t, _)| t), format!("{label} · drill"));
        drilled(&mut hole, drill.map_or(rough, |(_, d)| d));
        if drill.is_none() {
            missing.push(format!("a drill under Ø{}", trim(rough)));
        }
        ops.push(hole);
        // Milled out to the size the head finishes from, the rest left.
        let mill = mills(&doc.tools, machine, depth)
            .filter(|(_, d)| *d <= rough - 0.1)
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((mill, _)) = mill {
            let mut milled = stage(Some(mill), format!("{label} · rough to Ø{}", trim(rough)));
            milled.p.insert("cycle".into(), HELICAL_MILLING.into());
            milled
                .p
                .insert("bore_allowance".into(), format!("{:.2}", size - rough));
            ops.push(milled);
        } else {
            missing.push(format!(
                "a flat end mill under Ø{} with flutes over {:.1} mm to rough it (without one, the boring head roughs it from the drilled size)",
                trim(rough - 0.1),
                depth
            ));
        }
    }
    let mut status = format!(
        "Added {} before {} in {}.",
        ops.iter()
            .map(|o| o.name())
            .collect::<Vec<_>>()
            .join(", then "),
        op.name(),
        setup.name()
    );
    if !missing.is_empty() {
        status.push_str(&format!(
            " Add to the library, then choose in the operation: {}.",
            missing.join("; ")
        ));
    }
    Ok(Added { ops, status })
}
/// The library's tools on `machine` (`threading` takes them the same way).
fn tools<'a>(tools: &'a [Tool], machine: &'a Machine) -> impl Iterator<Item = &'a Tool> {
    tools
        .iter()
        .filter(move |t| t.fits(machine) && t.issues().is_empty())
}
/// The library's tools of `kind` whose flutes are longer than `depth`, with
/// their diameters.
fn reaching<'a>(
    tools: &'a [Tool],
    machine: &'a Machine,
    kind: &'a str,
    depth: f32,
) -> impl Iterator<Item = (&'a Tool, f32)> {
    self::tools(tools, machine)
        .filter(move |t| get(&t.p, "type") == kind)
        .filter(move |t| num(&t.p, "length").is_some_and(|l| l > depth))
        .filter_map(|t| Some((t, num(&t.p, "diameter")?)))
}
fn drills<'a>(
    tools: &'a [Tool],
    machine: &'a Machine,
    depth: f32,
) -> impl Iterator<Item = (&'a Tool, f32)> {
    reaching(tools, machine, "Drill", depth)
}
fn mills<'a>(
    tools: &'a [Tool],
    machine: &'a Machine,
    depth: f32,
) -> impl Iterator<Item = (&'a Tool, f32)> {
    reaching(tools, machine, "Flat end mill", depth)
}
fn trim(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cam::boring::tests::{drill, drilling, end_mill, head, job, reamer, ring},
        model::{BORE_RETRACTS, BORING, REAMING},
    };
    /// `ops` with the stages Add the drilling adds before its last, the
    /// library `tools` (each numbered); the job's results.
    fn staged(
        mesh: &Mesh,
        tools: Vec<Tool>,
        finish: (Tool, Operation),
    ) -> (Added, Vec<Result<crate::simulation::Simulation, String>>) {
        let (mut doc, _) = job(mesh, Machine::new(3), vec![finish.clone()]);
        doc.tools.extend(tools.iter().cloned());
        let added = add(&doc, 0, 0, mesh).unwrap();
        let mut ops: Vec<(Tool, Operation)> = (added.ops.iter())
            .map(|o| {
                let t = tools.iter().find(|t| t.id == o.tool).cloned();
                (t.unwrap_or_else(|| drill("1")), o.clone())
            })
            .collect();
        ops.push(finish);
        let (_, all) = job(mesh, Machine::new(3), ops);
        (added, all)
    }
    #[test]
    fn a_reamer_is_drilled_for_at_its_drill_size_first() {
        let mesh = ring(10., 40., 8.7);
        let drills: Vec<Tool> = ["8", "9.5", "9.8", "10"].map(drill).into();
        let r = reamer("10");
        let finish = (r.clone(), drilling(&mesh, &r, REAMING));
        let (added, all) = staged(&mesh, drills.clone(), finish);
        assert_eq!(added.ops.len(), 1);
        let d = &added.ops[0];
        assert_eq!(d.tool, drills[2].id, "{}", added.status);
        assert_eq!(get(&d.p, "hole_diameter"), "10.00");
        assert_eq!(get(&d.p, "cycle"), "Drilling");
        // Out past the exit as far as its point and a half millimetre.
        assert_eq!(get(&d.p, "breakthrough"), "3.5");
        assert!(
            all.iter().all(|r| r.is_ok()),
            "{:?}",
            all.iter().find_map(|r| r.as_ref().err())
        );
        // Without the drill: added without it, and the status says which.
        let (mut doc, _) = job(
            &mesh,
            Machine::new(3),
            vec![(r.clone(), drilling(&mesh, &r, REAMING))],
        );
        doc.tools.push(drill("8"));
        let added = add(&doc, 0, 0, &mesh).unwrap();
        assert!(added.ops[0].tool.is_empty());
        assert!(added.status.contains("a Ø9.8 drill"), "{}", added.status);
    }
    #[test]
    fn a_bore_is_drilled_and_milled_out_to_its_rough_size_first() {
        let mesh = ring(32., 60., 28.);
        let mut short = drill("13");
        short.p.insert("length".into(), "20".into());
        let tools = vec![
            drill("9.8"),
            drill("12"),
            short,
            end_mill("6"),
            end_mill("10"),
        ];
        let h = head(BORE_RETRACTS[0]);
        let mut bore = drilling(&mesh, &h, BORING);
        bore.p.insert("hole_diameter".into(), "32".into());
        let (added, all) = staged(&mesh, tools.clone(), (h, bore));
        assert_eq!(added.ops.len(), 2, "{}", added.status);
        // The widest drill that reaches through (29 mm, under three of its
        // diameters: no pecking), then the widest end mill.
        assert_eq!(added.ops[0].tool, tools[1].id);
        assert_eq!(added.ops[0].p["cycle"], "Drilling");
        assert_eq!(added.ops[1].tool, tools[4].id);
        assert_eq!(added.ops[1].p["cycle"], crate::model::HELICAL_MILLING);
        assert_eq!(added.ops[1].p["bore_allowance"], "0.60");
        assert!(added.status.contains("rough to Ø31.4"), "{}", added.status);
        // Drilled, roughed and bored: each programs.
        assert!(
            all.iter().all(|r| r.is_ok()),
            "{:?}",
            all.iter().find_map(|r| r.as_ref().err())
        );
        let plan = &all.last().unwrap().as_ref().unwrap().plan;
        assert_eq!(plan.stops.iter().filter(|s| s.pause.is_some()).count(), 2);
    }
    #[test]
    fn a_reamer_and_a_boring_head_need_their_sizes_and_the_cnc() {
        let mut r = reamer("10");
        assert!(r.issues().is_empty(), "{:?}", r.issues());
        r.p.insert("tolerance".into(), "".into());
        assert!(r.issues().iter().any(|e| e.contains("tolerance class")));
        let mut h = head(BORE_RETRACTS[0]);
        assert!(h.issues().is_empty(), "{:?}", h.issues());
        h.p.insert("bore_range".into(), "50-12".into());
        assert!(h.issues().iter().any(|e| e.contains("Bore range")));
        h.p.insert("bore_range".into(), "12-50".into());
        h.p.insert("bar_diameter".into(), "14".into());
        assert!(
            h.issues()
                .iter()
                .any(|e| e.contains("under the smallest bore"))
        );
        // A Boring of one size, on a CNC.
        let mesh = ring(32., 60., 28.);
        let h = head(BORE_RETRACTS[0]);
        let mut bore = drilling(&mesh, &h, BORING);
        let tools = vec![h.clone()];
        let cnc = Machine::new(3);
        assert!(
            bore.issues(&cnc, &tools, &[])
                .iter()
                .any(|e| e == crate::model::BORE_ONE_SIZE)
        );
        bore.p.insert("hole_diameter".into(), "32".into());
        assert!(
            !bore
                .issues(&cnc, &tools, &[])
                .iter()
                .any(|e| e.contains("size") || e.contains("CNC"))
        );
        let manual = crate::manual::defaults();
        assert!(
            bore.issues(&manual, &tools, &[])
                .iter()
                .any(|e| e == crate::model::FINISH_ON_HAND)
        );
        assert_eq!(bore.tool_types(&cnc), vec!["Boring head"]);
        let ream = drilling(&mesh, &r, REAMING);
        assert_eq!(ream.tool_types(&cnc), vec!["Reamer"]);
    }
}
