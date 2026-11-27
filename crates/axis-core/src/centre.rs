//! Centre drilling on the manual mill. A twist drill started on a flat
//! face walks: its chisel edge skids before its lips bite, and the hole
//! lands off its position. So every hole a manual-mill drill opens from the
//! solid is centre drilled first, with a 60° A-type centre drill
//! (`model::CENTRE_DRILL`), its cone opened to the hole's diameter (the
//! body's when the hole is wider), where the drill's lips start.
//!
//! The centre drilling is an operation of its own, a Drill with the centre
//! drill just before the drill it centres. It names that drill (`centres`)
//! and its holes follow the drill's (`follow`, `sync`). A new Drill, Add the
//! drilling, Add threading and the AI planner put one in (`needed`); the
//! Steps page and the plan check say when a drill starts on a flat face
//! (`guide`). Taps, reamers, boring heads and a drill that opens up an
//! earlier one's holes follow those holes, and need none.
use crate::{
    cam, manual,
    mesh::Mesh,
    model::{Document, Machine, Operation, Setup, Strategy, Tool, get, num},
};

/// The keys of a Drill that pick its holes, order them and say where it
/// starts from: the centre drilling's are its drill's.
pub const FOLLOWS: [&str; 6] = [
    "holes",
    "hole_diameter",
    "hole_order",
    "top",
    "clearance",
    "retract",
];
/// The cycles of a drill that drills its holes from the solid.
pub const FROM_SOLID: [&str; 4] = [
    "Drilling",
    "Chip breaking",
    "Deep drilling",
    "Counterboring",
];
/// What a manual-mill drill with no centre drilling before it is told.
pub const CENTRE_FIRST: &str = "On the manual mill a drill started on a flat face walks off its position: centre drill its holes first, so that it finds their centres.";
/// The key of a centre drilling that names the drill it centres (its id).
pub const CENTRES: &str = "centres";

/// The tool of `op`, from `tools`.
fn tool_of<'a>(op: &Operation, tools: &'a [Tool]) -> Option<&'a Tool> {
    tools.iter().find(|t| t.id == op.tool)
}
/// Whether `op` drills its holes from the solid with a twist drill on the
/// manual mill `m`: what wants a centre drilling before it. Not the pin
/// holes, which the pins' setting places.
pub fn wants_centre(op: &Operation, tools: &[Tool], m: &Machine) -> bool {
    manual::is_manual(m)
        && op.strategy == Strategy::Drill
        && !op.drills_pin_holes()
        && FROM_SOLID.contains(&get(&op.p, "cycle"))
        && tool_of(op, tools).is_some_and(|t| get(&t.p, "type") == "Drill")
}
/// Whether Drills `a` and `b` pick the same holes.
pub fn same_holes(a: &Operation, b: &Operation) -> bool {
    let ids = |o: &Operation| {
        o.surfaces
            .as_ref()
            .map(|s| (s.mesh_fingerprint.clone(), s.ids.clone()))
    };
    a.geometry == b.geometry
        && ids(a) == ids(b)
        && ["holes", "hole_diameter"].iter().all(|k| {
            let (x, y) = (get(&a.p, k), get(&b.p, k));
            x == y || num(&a.p, k).zip(num(&b.p, k)).is_some_and(|(x, y)| x == y)
        })
}
/// Within this, mm, a hole of an earlier drilling and one of a drill are
/// at the same centre (`centred`): far less than any drill's walk, more
/// than two arcs of one bore fitted from its facets differ by.
pub const SAME_CENTRE: f32 = 0.05;
/// What `centred` reads a Drill's holes with: those it picks
/// (`cam::holes`), None when they are not known.
pub type HolesOf<'a> = dyn FnMut(&Operation) -> Option<Vec<cam::Hole>> + 'a;
/// Whether operation `oi` of `setup` has its holes opened before it: an
/// enabled earlier operation centre drills it (`centres`), or drills the
/// same holes from the solid (a centre drilling, a countersink, a pilot
/// drill), or, with the part `mesh`, drills from the solid at the centre
/// of every hole it drills (`at_centres`): a centre drilling picked on a
/// counterbore's arcs centres the drill picked on the slot's under them
/// (LIO-536). Not one a milling operation follows before the drill: a
/// facing, a pocket or a slot may take away the face it spotted (the
/// guide's `opened`, which reads the stock, says for sure).
pub fn centred(setup: &Setup, oi: usize, mesh: Option<&Mesh>) -> bool {
    let mut kept = std::collections::HashMap::new();
    centred_by(setup, oi, &mut |o: &Operation| {
        let mesh = mesh?;
        (kept.entry(o.id.clone()))
            .or_insert_with(|| cam::holes(setup, o, mesh).ok())
            .clone()
    })
}
/// `centred`, the holes read by `holes`.
pub fn centred_by(setup: &Setup, oi: usize, holes: &mut HolesOf<'_>) -> bool {
    let Some(op) = setup.operations.get(oi) else {
        return false;
    };
    // Drills from the solid: not a tap, a reamer or a boring head, which
    // follow a hole already there.
    let opens = |o: &Operation| {
        o.strategy == Strategy::Drill
            && !o.drills_pin_holes()
            && FROM_SOLID.contains(&get(&o.p, "cycle"))
    };
    // The enabled operations since the last milling one before the drill:
    // a spot made before that may be gone.
    let since = (setup.operations[..oi].iter())
        .rposition(|o| o.enabled && o.strategy != Strategy::Drill)
        .map_or(0, |k| k + 1);
    let earlier: Vec<&Operation> = (setup.operations[since..oi].iter())
        .filter(|o| o.enabled)
        .collect();
    if (earlier.iter()).any(|o| get(&o.p, CENTRES) == op.id || (opens(o) && same_holes(o, op))) {
        return true;
    }
    let candidates: Vec<&&Operation> = earlier.iter().filter(|o| opens(o)).collect();
    if candidates.is_empty() {
        return false;
    }
    let Some(drilled) = holes(op) else {
        return false;
    };
    candidates
        .into_iter()
        .any(|o| holes(o).is_some_and(|by| at_centres(op, &drilled, o, &by)))
}
/// Whether every hole of `holes` that drill `op` drills (`cam::drills`)
/// has a hole of `by` that operation `o` drills at its centre, at the same
/// A angle (`SAME_CENTRE`); false when it drills none.
pub fn at_centres(op: &Operation, holes: &[cam::Hole], o: &Operation, by: &[cam::Hole]) -> bool {
    let same = |a: &cam::Hole, b: &cam::Hole| {
        a.a.unwrap_or(0.) == b.a.unwrap_or(0.)
            && (a.center[0] - b.center[0]).hypot(a.center[1] - b.center[1]) <= SAME_CENTRE
    };
    let mut drilled = holes.iter().filter(|h| cam::drills(op, h)).peekable();
    drilled.peek().is_some() && drilled.all(|h| by.iter().any(|c| cam::drills(o, c) && same(c, h)))
}
/// Takes out of `doc` each centre drilling of a drill that no longer
/// drills from the solid (`FROM_SOLID`): one turned into a boring head's,
/// a tap's or a reamer's, which follow the holes drilled before them and
/// need none. The setup and the index of each taken out, in the order
/// taken out, each as it was when taken out (the ones before it gone).
pub fn drop_needless(doc: &mut Document) -> Vec<(usize, usize)> {
    let mut out = vec![];
    for (si, setup) in doc.setups.iter_mut().enumerate() {
        let needless = |o: &Operation, ops: &[Operation]| {
            centres(o).is_some_and(|id| {
                (ops.iter()).any(|d| d.id == id && !FROM_SOLID.contains(&get(&d.p, "cycle")))
            })
        };
        let mut i = 0;
        while i < setup.operations.len() {
            if needless(&setup.operations[i], &setup.operations) {
                setup.operations.remove(i);
                out.push((si, i));
            } else {
                i += 1;
            }
        }
    }
    out
}
/// Gives centre drilling `centre` the holes of `drill`, the one it centres.
pub fn follow(centre: &mut Operation, drill: &Operation) {
    centre.geometry = drill.geometry.clone();
    centre.surfaces = drill.surfaces.clone();
    for k in FOLLOWS {
        match drill.p.get(k) {
            Some(v) => centre.p.insert(k.into(), v.clone()),
            None => centre.p.remove(k),
        };
    }
}
/// Gives every centre drilling of `doc` the holes of the drill it centres
/// (`follow`); whether any changed.
pub fn sync(doc: &mut Document) -> bool {
    let mut changed = false;
    for setup in &mut doc.setups {
        for i in 0..setup.operations.len() {
            let of = get(&setup.operations[i].p, CENTRES).to_string();
            if of.is_empty() {
                continue;
            }
            let Some(drill) = setup.operations.iter().find(|o| o.id == of).cloned() else {
                continue;
            };
            let centre = &mut setup.operations[i];
            let before = (
                centre.geometry.clone(),
                centre.surfaces.as_ref().map(|s| s.ids.clone()),
                FOLLOWS.map(|k| centre.p.get(k).cloned()),
            );
            follow(centre, &drill);
            let after = (
                centre.geometry.clone(),
                centre.surfaces.as_ref().map(|s| s.ids.clone()),
                FOLLOWS.map(|k| centre.p.get(k).cloned()),
            );
            changed |= before != after;
        }
    }
    changed
}
/// A hole a centre drill starts: the widest its pilot may be (its Ø, or the
/// widest drill it takes when narrower), its Ø, and how deep it is from its
/// top to its bottom.
#[derive(Debug, Clone, Copy)]
pub struct Spot {
    pub room: f32,
    pub diameter: f32,
    pub depth: f32,
}
impl Spot {
    /// Whether centre drill `c` starts it as `cam` cuts it: its pilot
    /// narrower, and the pilot under the cone opened to its Ø (or the
    /// body's) no deeper than it.
    fn takes(&self, c: &crate::model::CentreDrill) -> bool {
        c.pilot < self.room && c.cone_depth(self.diameter) + c.pilot_length <= self.depth + 1e-3
    }
}
/// The holes drill `op` of `setup` drills (`Spot`); None when they are not
/// known.
fn spots(setup: &Setup, op: &Operation, mesh: &Mesh) -> Option<Vec<Spot>> {
    let holes = cam::holes(setup, op, mesh).ok()?;
    let spots: Vec<Spot> = (holes.iter().filter(|h| cam::drills(op, h)))
        .map(|h| Spot {
            room: h.widest_drill().min(h.diameter),
            diameter: h.diameter,
            depth: h.top - h.bottom,
        })
        .collect();
    (!spots.is_empty()).then_some(spots)
}
/// The library's centre drill for the holes `spots` on machine `m`: one
/// that starts every one of them (`Spot::takes`); of those, the narrowest
/// body that opens the widest hole's cone to its size, else the widest body.
/// Without holes, the one whose body is nearest 6.3 mm, the common A 2.5×6.3.
pub fn pick<'a>(tools: &'a [Tool], m: &Machine, spots: Option<&[Spot]>) -> Option<&'a Tool> {
    let fitting: Vec<(&Tool, crate::model::CentreDrill)> = (tools.iter())
        .filter(|t| t.fits(m) && t.issues().is_empty())
        .filter_map(|t| Some((t, t.centre_drill()?)))
        .filter(|(_, c)| spots.is_none_or(|s| s.iter().all(|h| h.takes(c))))
        .collect();
    let Some(spots) = spots else {
        return (fitting.iter())
            .min_by(|a, b| (a.1.body - 6.3).abs().total_cmp(&(b.1.body - 6.3).abs()))
            .map(|(t, _)| *t);
    };
    let widest = spots.iter().map(|h| h.diameter).fold(0., f32::max);
    (fitting.iter().filter(|(_, c)| c.body >= widest))
        .min_by(|a, b| a.1.body.total_cmp(&b.1.body))
        .or_else(|| fitting.iter().max_by(|a, b| a.1.body.total_cmp(&b.1.body)))
        .map(|(t, _)| *t)
}
/// What a centre drilling added before a drill is, and what the library
/// lacks for it.
pub struct Centring {
    pub op: Operation,
    /// The centre drill to add to the library, when it has none that fits.
    pub missing: Option<String>,
}
/// The speed and feed a centre drill of body Ø`body` and pilot Ø`pilot`
/// runs at after drill `drill` of Ø`drilled` (its speed and feed): its
/// body's edge at the drill's surface speed, its pilot at the drill's feed
/// per turn, less for a pilot narrower than the drill; on machine `m`'s
/// gears and within its hand feed.
fn speeds(
    drill: &Operation,
    drilled: Option<f32>,
    body: f32,
    pilot: f32,
    m: &Machine,
) -> (f32, f32) {
    let rpm = num(&drill.p, "rpm").filter(|v| *v > 0.).unwrap_or(1000.);
    let feed = num(&drill.p, "feed").filter(|v| *v > 0.).unwrap_or(50.);
    let d = drilled.filter(|d| *d > 0.).unwrap_or(body);
    let speed = rpm * (d / body).min(1.);
    let per_turn = feed / rpm * (pilot / d).min(1.);
    let mut p = crate::model::Fields::new();
    p.insert("rpm".into(), speed.round().to_string());
    p.insert(
        "feed".into(),
        (per_turn * speed).round().max(1.).to_string(),
    );
    manual::snap(m, &mut p);
    let rpm = num(&p, "rpm").unwrap_or(speed);
    // The feed per turn kept on the gear it went to.
    let feed = (per_turn * rpm).round().max(1.);
    let feed = num(&m.p, "hand_feed").map_or(feed, |hand| feed.min(hand));
    (rpm, feed)
}
/// A centre drilling of the holes of `drill`, an operation of `setup`,
/// with the library's centre drill for them (`pick`; their sizes from
/// `mesh`), to go just before it.
pub fn centring(doc: &Document, setup: &Setup, drill: &Operation, mesh: Option<&Mesh>) -> Centring {
    let machine = &setup.machine;
    let spots = mesh.and_then(|mesh| spots(setup, drill, mesh));
    let tool = pick(&doc.tools, machine, spots.as_deref());
    let mut op = Operation::for_machine(
        Strategy::Drill,
        tool.map(|t| t.id.clone()).unwrap_or_default(),
        machine,
    );
    follow(&mut op, drill);
    op.p.insert(CENTRES.into(), drill.id.clone());
    op.p.insert("name".into(), format!("Centre drill · {}", drill.name()));
    op.p.insert("cycle".into(), "Drilling".into());
    op.p.insert("through".into(), "No".into());
    op.p.insert("coolant".into(), get(&drill.p, "coolant").into());
    op.enabled = drill.enabled;
    if let Some(c) = tool.and_then(|t| t.centre_drill()) {
        let drilled = tool_of(drill, &doc.tools).and_then(|t| num(&t.p, "diameter"));
        // The material's data for a cutter of its body and flutes, as the
        // plan check holds it to; else from the drill's.
        let (rpm, feed) = (doc.selected_material())
            .and_then(|m| crate::material::recommend_for(m, tool?, &op, machine).ok())
            .unwrap_or_else(|| speeds(drill, drilled, c.body, c.pilot, machine));
        op.p.insert("rpm".into(), rpm.to_string());
        op.p.insert("feed".into(), feed.to_string());
    }
    let missing = tool.is_none().then(|| match spots.as_deref() {
        Some(s) => format!(
            "a centre drill whose pilot is under Ø{} and whose pilot and cone go no deeper than {} mm",
            crate::post::exact_mm(s.iter().map(|h| h.room).fold(f32::INFINITY, f32::min)),
            crate::post::exact_mm(s.iter().map(|h| h.depth).fold(f32::INFINITY, f32::min))
        ),
        None => "a centre drill".into(),
    });
    Centring { op, missing }
}
/// The centre drillings setup `si` lacks: one before every operation that
/// wants one (`wants_centre`) and has none (`centred`), each with the index
/// of the operation it goes before; and what to say of them
/// (`said`, empty when none).
pub fn needed(doc: &Document, si: usize, mesh: Option<&Mesh>) -> (Vec<(usize, Operation)>, String) {
    needed_for(doc, si, mesh, |_| true)
}
/// `needed`, for the operations `of` says only: the drills an Add just put
/// in, not the setup's others, which their own centring or none was left
/// to.
pub fn needed_for(
    doc: &Document,
    si: usize,
    mesh: Option<&Mesh>,
    of: impl Fn(&Operation) -> bool,
) -> (Vec<(usize, Operation)>, String) {
    let Some(setup) = doc.setups.get(si) else {
        return (vec![], String::new());
    };
    // Each Drill's holes worked out once.
    let mut kept = std::collections::HashMap::new();
    let mut holes = |o: &Operation| {
        let mesh = mesh?;
        (kept.entry(o.id.clone()))
            .or_insert_with(|| cam::holes(setup, o, mesh).ok())
            .clone()
    };
    let mut out = vec![];
    let (mut added, mut missing) = (vec![], vec![]);
    for (oi, op) in setup.operations.iter().enumerate() {
        if !of(op)
            || !wants_centre(op, &doc.tools, &setup.machine)
            || centred_by(setup, oi, &mut holes)
        {
            continue;
        }
        let c = centring(doc, setup, op, mesh);
        added.push(c.op.name().to_string());
        if let Some(what) = c.missing {
            missing.push((c.op.name().to_string(), what));
        }
        out.push((oi, c.op));
    }
    (out, said(&added, &missing))
}
/// Puts `needed`'s centre drillings into setup `si`, each before its drill.
pub fn insert(doc: &mut Document, si: usize, needed: Vec<(usize, Operation)>) {
    if si >= doc.setups.len() {
        return;
    }
    for (oi, op) in needed.into_iter().rev() {
        // It picks what its drill picks, so a reimport finds them again.
        if let Some(drill) = centres(&op) {
            doc.copy_picks(drill, &op.id);
        }
        let ops = &mut doc.setups[si].operations;
        ops.insert(oi.min(ops.len()), op);
    }
}
/// "Added Centre drill · Holes before Holes." and what to add to the
/// library, for `added` (names) and `missing` (name, tool).
pub fn said(added: &[String], missing: &[(String, String)]) -> String {
    if added.is_empty() {
        return String::new();
    }
    let mut s = format!(
        "Added {}: a drill started on a flat face walks off its position.",
        added.join(", ")
    );
    for (name, what) in missing {
        s.push_str(&format!(
            " Add {what} to the library, then choose it in {name}."
        ));
    }
    s
}
/// The id of the drill operation `op` centre drills, when it is one.
pub fn centres(op: &Operation) -> Option<&str> {
    Some(get(&op.p, CENTRES)).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guide::{build, tests::job};
    use crate::model::{CENTRE_DRILL, CENTRE_ON_CNC};

    /// A centre drill of pilot Ø`pilot` × `reach` under a Ø`body` body,
    /// fitting every manual mill.
    fn centre_drill(pilot: &str, reach: &str, body: &str) -> Tool {
        let mut t = Tool::new(&format!("Centre drill A{pilot}×{body}"), CENTRE_DRILL, body);
        t.set_type(CENTRE_DRILL);
        for (k, v) in [
            ("pilot", pilot),
            ("pilot_length", reach),
            ("length", "12"),
            ("stickout", "40"),
        ] {
            t.p.insert(k.into(), v.into());
        }
        t
    }
    /// The manual job's Holes (`guide::tests::job`) with a centre drilling
    /// before it, by `tool`: the doc and the centre drilling's index.
    fn centred_job(tool: Tool) -> (Document, Mesh, usize) {
        let (mut doc, mesh) = job("Yes");
        doc.tools.push(tool);
        let (ops, said) = needed(&doc, 0, Some(&mesh));
        assert_eq!(ops.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [2]);
        assert!(said.starts_with("Added Centre drill · Holes"), "{said}");
        insert(&mut doc, 0, ops);
        (doc, mesh, 2)
    }
    fn plan(doc: &Document, mesh: &Mesh, oi: usize) -> Result<crate::simulation::Plan, String> {
        let setup = &doc.setups[0];
        let op = &setup.operations[oi];
        let tool = doc.tools.iter().find(|t| t.id == op.tool).unwrap();
        cam::generate(setup, op, tool, mesh)
    }

    #[test]
    fn the_cone_opens_to_the_hole_capped_by_the_body() {
        for (body, cone) in [("8", 6.6_f32), ("6.3", 6.3)] {
            let (doc, mesh, ci) = centred_job(centre_drill("2.5", "3.1", body));
            let setup = &doc.setups[0];
            let centre = &setup.operations[ci];
            assert_eq!(centres(centre), Some(setup.operations[3].id.as_str()));
            let holes = cam::holes(setup, centre, &mesh).unwrap();
            let plan = plan(&doc, &mesh, ci).unwrap();
            assert_eq!(plan.milling_direction, "Centre drilling · axial feed");
            // The holder and flutes are measured from the pilot's top, the
            // plan's point: 40 and 12 from the tip, less the 3.1 pilot.
            assert!(
                (plan.tool_length - 36.9).abs() < 1e-4,
                "{}",
                plan.tool_length
            );
            assert!(
                (plan.flute_length - 8.9).abs() < 1e-4,
                "{}",
                plan.flute_length
            );
            // The pilot's top goes down until the cone is the hole's size,
            // or the body's, at the hole's top.
            let depth = (cone - 2.5) / 2. * 3f32.sqrt();
            let mut n = 0;
            for h in holes.iter().filter(|h| cam::drills(centre, h)) {
                let low = (plan.moves.iter())
                    .filter(|m| {
                        m.cutting_pass
                            && (m.to[0] - h.center[0]).hypot(m.to[1] - h.center[1]) < 1e-3
                    })
                    .map(|m| m.to[2])
                    .fold(f32::INFINITY, f32::min);
                assert!(
                    (h.top - low - depth).abs() < 0.01,
                    "{body}: {} {low} {depth}",
                    h.top
                );
                n += 1;
            }
            assert_eq!(n, 4);
        }
    }
    #[test]
    fn a_pilot_too_wide_a_hole_too_shallow_and_a_cnc_are_refused() {
        let (mut doc, mesh, ci) = centred_job(centre_drill("2.5", "3.1", "6.3"));
        let tool = doc.setups[0].operations[ci].tool.clone();
        let set = |doc: &mut Document, k: &str, v: &str| {
            let t = doc.tools.iter_mut().find(|t| t.id == tool).unwrap();
            t.p.insert(k.into(), v.into());
        };
        set(&mut doc, "pilot", "6.6");
        set(&mut doc, "diameter", "10");
        let why = plan(&doc, &mesh, ci).unwrap_err();
        assert!(why.contains("pilot is as wide"), "{why}");
        set(&mut doc, "pilot", "2.5");
        set(&mut doc, "diameter", "6.3");
        set(&mut doc, "pilot_length", "40");
        set(&mut doc, "length", "50");
        set(&mut doc, "stickout", "60");
        let why = plan(&doc, &mesh, ci).unwrap_err();
        assert!(why.contains("too shallow for the centre drill"), "{why}");
        set(&mut doc, "pilot_length", "3.1");
        assert!(plan(&doc, &mesh, ci).is_ok());
        // Flutes 6.5 long reach 3.4 over the pilot: the 3.29 of the cone
        // alone, not the stock standing over the holes as well. Then the
        // cone opens at that stock's top, where the drill after it starts
        // (LIO-536): a counterbore's floor not bored yet.
        set(&mut doc, "length", "6.5");
        let p = plan(&doc, &mesh, ci).unwrap();
        let setup = &doc.setups[0];
        let holes = cam::holes(setup, &setup.operations[ci], &mesh).unwrap();
        let cone = 3.8 / 2. * 3f32.sqrt();
        let mut n = 0;
        for h in holes
            .iter()
            .filter(|h| cam::drills(&setup.operations[ci], h))
        {
            let low = (p.moves.iter())
                .filter(|m| {
                    m.cutting_pass && (m.to[0] - h.center[0]).hypot(m.to[1] - h.center[1]) < 1e-3
                })
                .map(|m| m.to[2])
                .fold(f32::INFINITY, f32::min);
            assert!(p.stock.max[2] > h.top + 1., "{} {}", p.stock.max[2], h.top);
            assert!((p.stock.max[2] - cone - low).abs() < 0.01, "{low}");
            n += 1;
        }
        assert_eq!(n, 4);
        // Flutes shorter than the cone alone: refused, and why.
        set(&mut doc, "length", "6.3");
        let why = plan(&doc, &mesh, ci).unwrap_err();
        assert!(
            why.contains("The centre drill's flutes reach 3.20 mm"),
            "{why}"
        );
        set(&mut doc, "length", "50");
        // With the clearance and the retract 1 mm over the stock, the
        // pilot's tip, 3.1 under the plan's point, moves across clear of it.
        for k in ["clearance", "retract"] {
            doc.setups[0].operations[ci].p.insert(k.into(), "1".into());
        }
        let p = plan(&doc, &mesh, ci).unwrap();
        let across = (p.moves.iter()).filter(|m| m.from[0] != m.to[0] || m.from[1] != m.to[1]);
        for m in across {
            let tip = m.from[2].min(m.to[2]) - 3.1;
            assert!(
                tip >= p.stock.max[2] + 0.999,
                "{m:?} over {}",
                p.stock.max[2]
            );
        }
        // On the CNC it is refused, and with no machine checked it fits
        // none.
        let manual = doc.setups[0].machine.clone();
        let cnc = Machine::new(3);
        doc.setups[0].machine = cnc.clone();
        assert_eq!(plan(&doc, &mesh, ci).unwrap_err(), CENTRE_ON_CNC);
        let t = doc.tools.iter().find(|t| t.id == tool).unwrap();
        assert!(!t.fits(&cnc) && t.fits(&manual));
        let op = &doc.setups[0].operations[ci];
        let issues = op.issues(&cnc, &doc.tools, &[]);
        assert!(issues.iter().any(|i| i == CENTRE_ON_CNC), "{issues:?}");
    }
    #[test]
    fn pick_takes_a_pilot_under_the_narrowest_and_the_smallest_body_over_the_widest() {
        let m = manual::defaults();
        let tools = [
            centre_drill("1.6", "2", "4"),
            centre_drill("2.5", "3.1", "6.3"),
            centre_drill("3.15", "3.9", "8"),
        ];
        // Holes from Ø`narrowest` to Ø`widest`, 50 deep.
        let deep = |narrowest: f32, widest: f32| {
            [narrowest, widest].map(|d| Spot {
                room: d,
                diameter: d,
                depth: 50.,
            })
        };
        let name = |s: Option<(f32, f32)>| {
            let spots = s.map(|(a, b)| deep(a, b));
            pick(&tools, &m, spots.as_ref().map(|s| &s[..])).map(|t| t.name().to_string())
        };
        assert_eq!(name(None).as_deref(), Some("Centre drill A2.5×6.3"));
        // A Ø6 hole 5 deep: the A2.5's pilot and cone go 6.13 down it, the
        // A3.15's 6.37; the A1.6's 4.08, its cone at its Ø4 body.
        let shallow = [Spot {
            room: 6.,
            diameter: 6.,
            depth: 5.,
        }];
        let picked = pick(&tools, &m, Some(&shallow)).map(|t| t.name());
        assert_eq!(picked, Some("Centre drill A1.6×4"));
        // Ø5 to Ø7 holes: the A2.5 and the A3.15 go down a Ø5; only the
        // A3.15's body reaches Ø7.
        assert_eq!(
            name(Some((5., 7.))).as_deref(),
            Some("Centre drill A3.15×8")
        );
        assert_eq!(
            name(Some((5., 6.))).as_deref(),
            Some("Centre drill A2.5×6.3")
        );
        // A Ø2 hole: only the A1.6 goes down it, though too small for Ø7;
        // with a Ø3 the A2.5 does too, and it is the wider.
        assert_eq!(name(Some((2., 7.))).as_deref(), Some("Centre drill A1.6×4"));
        assert_eq!(
            name(Some((3., 7.))).as_deref(),
            Some("Centre drill A2.5×6.3")
        );
        assert_eq!(name(Some((1.5, 7.))), None);
        assert!(pick(&tools, &Machine::new(3), None).is_none());
    }
    #[test]
    fn a_centre_drillings_holes_follow_its_drill() {
        let (mut doc, _, ci) = centred_job(centre_drill("2.5", "3.1", "6.3"));
        let ops = &mut doc.setups[0].operations;
        ops[3].p.insert("hole_diameter".into(), "5".into());
        ops[3].p.insert("hole_order".into(), "Shortest path".into());
        assert!(sync(&mut doc));
        let ops = &doc.setups[0].operations;
        assert_eq!(get(&ops[ci].p, "hole_diameter"), "5");
        assert_eq!(get(&ops[ci].p, "hole_order"), "Shortest path");
        assert!(same_holes(&ops[ci], &ops[3]));
        assert!(!sync(&mut doc));
        // The drill is centred: nothing more is needed.
        assert!(centred(&doc.setups[0], 3, None));
        assert!(needed(&doc, 0, None).0.is_empty());
        // A copied setup's centre drilling centres the copied drill.
        let copy = doc.setups[0].duplicate("G55");
        let ops = &copy.operations;
        assert_ne!(ops[3].id, doc.setups[0].operations[3].id);
        assert_eq!(centres(&ops[ci]), Some(ops[3].id.as_str()));
    }
    #[test]
    fn a_drilling_at_the_same_centres_centres_a_drill_whatever_it_picks() {
        // A centre drilling of its own before the Holes, unlinked and
        // picking none of what they pick (LIO-536): a counterbore's arcs
        // over a slot's, say.
        let (mut doc, mesh) = job("Yes");
        let tool = centre_drill("2.5", "3.1", "6.3");
        let machine = doc.setups[0].machine.clone();
        let mut spot = Operation::for_machine(Strategy::Drill, tool.id.clone(), &machine);
        spot.p.insert("cycle".into(), "Drilling".into());
        doc.tools.push(tool);
        doc.setups[0].operations.insert(2, spot);
        let setup = &doc.setups[0];
        let (spot, drill) = (&setup.operations[2], &setup.operations[3]);
        assert!(!same_holes(spot, drill));
        // The four its Ø6.6 filter drills of the face's five.
        let real: Vec<_> = (cam::holes(setup, drill, &mesh).unwrap().into_iter())
            .filter(|h| cam::drills(drill, h))
            .collect();
        assert_eq!(real.len(), 4);
        // The spot's holes, each moved `by` in X; the drill's as they are.
        let moved = |by: f32, keep: usize| {
            let (real, drill) = (real.clone(), drill.id.clone());
            move |o: &Operation| {
                Some(match o.id == drill {
                    true => real.clone(),
                    false => (real.iter().take(keep).cloned())
                        .map(|mut h| {
                            h.center[0] += by;
                            h
                        })
                        .collect(),
                })
            }
        };
        assert!(centred_by(setup, 3, &mut moved(0., 4)));
        assert!(centred_by(setup, 3, &mut moved(0.04, 4)));
        // Off the centres, or one hole short: not centred.
        assert!(!centred_by(setup, 3, &mut moved(0.2, 4)));
        assert!(!centred_by(setup, 3, &mut moved(0., 3)));
        // Holes not known: by its picks only.
        assert!(!centred(setup, 3, None));
        // A spot that is off, or a tap or a boring head before it, does
        // not open the holes.
        for (k, v) in [
            ("enabled", ""),
            ("cycle", "Tapping"),
            ("cycle", "Boring"),
            ("cycle", "Reaming"),
        ] {
            let mut doc = doc.clone();
            let spot = &mut doc.setups[0].operations[2];
            match k {
                "enabled" => spot.enabled = false,
                _ => {
                    spot.p.insert(k.into(), v.into());
                }
            }
            assert!(!centred_by(&doc.setups[0], 3, &mut moved(0., 4)), "{k} {v}");
        }
        // Picking what the drill picks: so too, only from the solid.
        let mut same = doc.clone();
        let drill = same.setups[0].operations[3].clone();
        follow(&mut same.setups[0].operations[2], &drill);
        assert!(same_holes(&same.setups[0].operations[2], &drill));
        assert!(centred(&same.setups[0], 3, None));
        for cycle in ["Tapping", "Boring", "Reaming"] {
            let mut doc = same.clone();
            (doc.setups[0].operations[2].p).insert("cycle".into(), cycle.into());
            assert!(!centred(&doc.setups[0], 3, None), "{cycle}");
        }
        // A milling operation between them may take the spots away (a
        // facing 12 deeper): not counted, by its picks or its centres. A
        // drilling between them does not.
        for (strategy, counted) in [(Strategy::Box, false), (Strategy::Drill, true)] {
            let mut doc = same.clone();
            let tool = doc.setups[0].operations[0].tool.clone();
            let mut between = crate::guide::tests::boxed(
                "Face again",
                &tool,
                [-500., 500., -500., 500., -2., -14.],
                &[],
            );
            between.strategy = strategy;
            doc.setups[0].operations.insert(3, between);
            assert_eq!(centred(&doc.setups[0], 4, None), counted, "{strategy:?}");
            assert_eq!(
                centred_by(&doc.setups[0], 4, &mut moved(0., 4)),
                counted,
                "{strategy:?}"
            );
            doc.setups[0].operations[3].enabled = false;
            assert!(centred(&doc.setups[0], 4, None), "{strategy:?} off");
        }
    }
    #[test]
    fn a_drill_turned_into_a_bore_loses_its_centring_and_an_add_centres_only_its_own() {
        let (mut doc, _, ci) = centred_job(centre_drill("2.5", "3.1", "6.3"));
        // Drilling still: the centring stays.
        assert!(drop_needless(&mut doc).is_empty());
        doc.setups[0].operations[ci + 1]
            .p
            .insert("cycle".into(), "Boring".into());
        assert_eq!(drop_needless(&mut doc), [(0, ci)]);
        let ops = &doc.setups[0].operations;
        assert!(ops.iter().all(|o| centres(o).is_none()));
        assert_eq!(ops[ci].name(), "Holes");
        assert!(drop_needless(&mut doc).is_empty());
        // An Add asks for the drills it put in: the setup's others are
        // left as they are.
        let (doc, _) = job("Yes");
        let holes = doc.setups[0].operations[2].id.clone();
        assert!(needed_for(&doc, 0, None, |_| false).0.is_empty());
        let (ops, _) = needed_for(&doc, 0, None, |o| o.id == holes);
        assert_eq!(ops.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [2]);
    }
    /// The holes (centres and the Z the tip goes to) and the uncentred
    /// flags on the page of operation `oi` in `guide`.
    fn holes_of(guide: &crate::guide::Guide, oi: usize) -> (Vec<[f32; 3]>, Vec<bool>) {
        (crate::guide::tests::holes_of_op(guide, oi).iter())
            .map(|h| {
                let (crate::guide::Depth::Through { tip } | crate::guide::Depth::Blind { tip, .. }) =
                    h.depth;
                ([h.at[0], h.at[1], tip], h.uncentred)
            })
            .unzip()
    }
    #[test]
    fn the_steps_set_a_centre_drill_to_its_pilot_tip_and_find_the_holes_centred() {
        // Without a centre drilling every hole starts on the flat face.
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(holes_of(&guide, 2).1, [true; 4]);
        let (doc, mesh, ci) = centred_job(centre_drill("2.5", "3.1", "6.3"));
        let guide = build(&doc, &mesh, 0).unwrap();
        let (at, uncentred) = holes_of(&guide, ci);
        assert_eq!(uncentred, [false; 4]);
        // The DRO's Z is the pilot's tip, 3.1 under the plan's point.
        let plan = &guide.done.iter().find(|d| d.op == ci).unwrap().plan;
        let z = guide.zero;
        for h in &at {
            let low = (plan.moves.iter())
                .filter(|m| {
                    m.cutting_pass
                        && (m.to[0] - z[0] - h[0]).abs() < 1e-3
                        && (m.to[1] - z[1] - h[1]).abs() < 1e-3
                })
                .map(|m| m.to[2] - z[2])
                .fold(f32::INFINITY, f32::min);
            assert!((low - 3.1 - h[2]).abs() < 1e-3, "{low} {}", h[2]);
        }
        // The drill after it finds every hole centred.
        assert_eq!(holes_of(&guide, ci + 1).1, [false; 4]);
    }
    #[test]
    fn the_plan_check_reports_a_manual_drill_on_a_flat_face() {
        let (mut doc, mesh) = job("Yes");
        crate::proposal::tests::aluminium(&mut doc);
        let report = crate::analysis::report(&doc, &mesh).unwrap();
        let flagged = |doc: &Document| {
            (crate::plan_check::check_built(doc, &mesh, &report)
                .problems
                .into_iter())
            .filter(|p| p.stage == "drilling")
            .map(|p| (p.setup, p.operation, p.message))
            .collect::<Vec<_>>()
        };
        let said = flagged(&doc);
        assert_eq!(said.len(), 1, "{said:?}");
        assert_eq!((said[0].0, said[0].1), (1, Some(3)));
        assert!(
            said[0]
                .2
                .starts_with("4 of its holes start on a flat face."),
            "{}",
            said[0].2
        );
        let (mut doc, _, _) = centred_job(centre_drill("2.5", "3.1", "6.3"));
        crate::proposal::tests::aluminium(&mut doc);
        assert_eq!(flagged(&doc), []);
    }

    /// The Ø8 ring's through hole drilled on the manual mill, centre
    /// drilled first by an A2.5×6.3 (a 3.1 mm pilot): the job and its mesh.
    fn ring_job() -> (Document, Mesh) {
        use crate::cam::boring::tests::{drill, drilling, job, ring};
        let mesh = ring(8., 40., 10.);
        let mill = manual::defaults();
        let mut d = drill("8");
        d.p.insert("machines".into(), mill.id.clone());
        let mut op = drilling(&mesh, &d, "Drilling");
        op.p.insert("rpm".into(), "640".into());
        op.p.insert("feed".into(), "64".into());
        let (mut doc, _) = job(&mesh, mill, vec![(d, op)]);
        let mut c = centre_drill("2.5", "3.1", "6.3");
        c.p.insert("number".into(), "2".into());
        doc.tools.push(c);
        let (ops, said) = needed(&doc, 0, Some(&mesh));
        assert_eq!(ops.len(), 1, "{said}");
        insert(&mut doc, 0, ops);
        (doc, mesh)
    }
    fn simulated(
        doc: &Document,
        mesh: &Mesh,
    ) -> Vec<Result<crate::simulation::Simulation, String>> {
        let mut all = vec![];
        crate::simulation::sequence(doc, mesh, None, |_, _, r| all.push(r));
        all
    }
    #[test]
    fn a_clamp_only_its_pilot_reaches_is_hit() {
        let (doc, mesh, ci) = centred_job(centre_drill("2.5", "3.1", "6.3"));
        let mut plan = plan(&doc, &mesh, ci).unwrap();
        assert_eq!(plan.below, 3.1);
        let across = (plan.moves.iter())
            .find(|m| {
                m.motion == crate::simulation::Motion::Rapid
                    && (m.from[0] - m.to[0]).hypot(m.from[1] - m.to[1]) > 10.
            })
            .expect("a rapid across from hole to hole");
        let mid = [
            (across.from[0] + across.to[0]) / 2.,
            (across.from[1] + across.to[1]) / 2.,
        ];
        let z = across.from[2].min(across.to[2]);
        // A clamp under that rapid, its top 1.5 mm under the cone's point:
        // the pilot's tip, 3.1 mm under it, crosses the clamp.
        let clamp = crate::holding::HoldBox {
            label: "Clamp".into(),
            bounds: crate::mesh::Bounds {
                min: [mid[0] - 1., mid[1] - 1., z - 20.],
                max: [mid[0] + 1., mid[1] + 1., z - 1.5],
            },
            shape: Default::default(),
        };
        let hits = crate::holding::hits(&plan, std::slice::from_ref(&clamp), 0.5);
        assert_eq!(hits.len(), 1, "{hits:?}");
        // The parts the setup's holding walk takes the tool to be.
        let tool = doc.setups[0].operations[ci].tool.clone();
        let (parts, _) = crate::post::tool_parts(&doc, &doc.setups[0], &tool, &plan);
        let moves: Vec<_> = plan.moves.iter().map(|m| (m.from, m.to)).collect();
        let walked = crate::holding::hits_parts(&moves, &parts, std::slice::from_ref(&clamp), 0.5);
        assert_eq!(walked.len(), 1, "{walked:?}");
        // A tool that ends at its point passes over it.
        plan.below = 0.;
        assert!(crate::holding::hits(&plan, &[clamp], 0.5).is_empty());
    }
    #[test]
    fn the_manual_mills_travel_is_counted_at_its_pilots_tip() {
        let (mut doc, mesh) = ring_job();
        let all = simulated(&doc, &mesh);
        let plans: Vec<_> = (all.iter())
            .map(|r| r.as_ref().unwrap().plan.clone())
            .collect();
        assert_eq!((plans[0].below, plans[1].below), (3.1, 0.));
        // The centre drill's clearance is raised by its pilot, so its point
        // stands 3.1 mm over the drill's highest; its tip does not.
        let span = |below: f32| {
            let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
            for (p, b) in plans.iter().zip([below, 0.]) {
                for z in p.moves.iter().flat_map(|m| [m.from[2], m.to[2]]) {
                    lo = lo.min(z - b);
                    hi = hi.max(z - b);
                }
            }
            hi - lo
        };
        let tips = span(3.1);
        assert!((span(0.) - tips - 3.1).abs() < 1e-3, "{} {tips}", span(0.));
        let refused = |doc: &mut Document, travel: f32| {
            (doc.setups[0].machine.p).insert("z".into(), travel.to_string());
            simulated(doc, &mesh)
                .into_iter()
                .filter_map(Result::err)
                .collect::<Vec<_>>()
        };
        assert_eq!(refused(&mut doc, tips + 0.5), Vec::<String>::new());
        let why = refused(&mut doc, tips - 0.5);
        assert!(
            why.iter()
                .any(|e| e.contains(&format!("span {} mm", crate::envelope::mm(tips)))),
            "{why:?}"
        );
    }
}
