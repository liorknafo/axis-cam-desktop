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
//! drilling, Add threading and the AI planner put one in (`complete`); the
//! Steps page and the plan check say when a drill starts on a flat face
//! (`guide`). Taps, reamers, boring heads and a drill that opens up an
//! earlier one's holes follow those holes, and need none.
use crate::{
    cam, manual,
    mesh::Mesh,
    model::{CENTRE_DRILL, Document, Machine, Operation, Setup, Strategy, Tool, get, num},
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
pub const FROM_SOLID: [&str; 3] = ["Drilling", "Chip breaking", "Deep drilling"];
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
/// Whether `op` is a centre drilling: a Drill with a centre drill.
pub fn is_centre(op: &Operation, tools: &[Tool]) -> bool {
    op.strategy == Strategy::Drill
        && tool_of(op, tools).is_some_and(|t| get(&t.p, "type") == CENTRE_DRILL)
}
/// Whether Drills `a` and `b` pick the same holes.
pub fn same_holes(a: &Operation, b: &Operation) -> bool {
    let ids = |o: &Operation| o.surfaces.as_ref().map(|s| (s.mesh_fingerprint.clone(), s.ids.clone()));
    a.geometry == b.geometry
        && ids(a) == ids(b)
        && ["holes", "hole_diameter"].iter().all(|k| {
            let (x, y) = (get(&a.p, k), get(&b.p, k));
            x == y || num(&a.p, k).zip(num(&b.p, k)).is_some_and(|(x, y)| x == y)
        })
}
/// Whether operation `oi` of `setup` has its holes opened before it: an
/// enabled earlier operation centre drills it (`centres`), or drills the
/// same holes (a centre drilling, a countersink, a pilot drill).
pub fn centred(setup: &Setup, oi: usize) -> bool {
    let Some(op) = setup.operations.get(oi) else {
        return false;
    };
    setup.operations[..oi].iter().filter(|o| o.enabled).any(|o| {
        get(&o.p, CENTRES) == op.id
            || (o.strategy == Strategy::Drill && !o.drills_pin_holes() && same_holes(o, op))
    })
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
/// The narrowest hole drill `op` of `setup` drills (the widest drill it
/// takes) and the widest (its diameter); None when its holes are not known.
fn sizes(setup: &Setup, op: &Operation, mesh: &Mesh) -> Option<(f32, f32)> {
    let holes = cam::holes(setup, op, mesh).ok()?;
    let holes: Vec<_> = holes.iter().filter(|h| cam::drills(op, h)).collect();
    let narrowest = holes.iter().map(|h| h.widest_drill()).reduce(f32::min)?;
    let widest = holes.iter().map(|h| h.diameter).reduce(f32::max)?;
    Some((narrowest, widest))
}
/// The library's centre drill for holes of `sizes` (the narrowest, by the
/// widest drill it takes, and the widest diameter) on machine `m`: its pilot
/// goes down the narrowest; of those, the narrowest body that opens the
/// widest hole's cone to its size, else the widest body. Without sizes, the
/// one whose body is nearest 6.3 mm, the common A 2.5×6.3.
pub fn pick<'a>(tools: &'a [Tool], m: &Machine, sizes: Option<(f32, f32)>) -> Option<&'a Tool> {
    let fitting: Vec<(&Tool, crate::model::CentreDrill)> = (tools.iter())
        .filter(|t| t.fits(m) && t.issues().is_empty())
        .filter_map(|t| Some((t, t.centre_drill()?)))
        .filter(|(_, c)| sizes.is_none_or(|(narrowest, _)| c.pilot < narrowest))
        .collect();
    let Some((_, widest)) = sizes else {
        return (fitting.iter())
            .min_by(|a, b| (a.1.body - 6.3).abs().total_cmp(&(b.1.body - 6.3).abs()))
            .map(|(t, _)| *t);
    };
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
fn speeds(drill: &Operation, drilled: Option<f32>, body: f32, pilot: f32, m: &Machine) -> (f32, f32) {
    let rpm = num(&drill.p, "rpm").filter(|v| *v > 0.).unwrap_or(1000.);
    let feed = num(&drill.p, "feed").filter(|v| *v > 0.).unwrap_or(50.);
    let d = drilled.filter(|d| *d > 0.).unwrap_or(body);
    let speed = rpm * (d / body).min(1.);
    let per_turn = feed / rpm * (pilot / d).min(1.);
    let mut p = crate::model::Fields::new();
    p.insert("rpm".into(), speed.round().to_string());
    p.insert("feed".into(), (per_turn * speed).round().max(1.).to_string());
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
    let sizes = mesh.and_then(|mesh| sizes(setup, drill, mesh));
    let tool = pick(&doc.tools, machine, sizes);
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
        let (rpm, feed) = speeds(drill, drilled, c.body, c.pilot, machine);
        op.p.insert("rpm".into(), rpm.to_string());
        op.p.insert("feed".into(), feed.to_string());
    }
    let missing = tool.is_none().then(|| match sizes {
        Some((narrowest, _)) => format!(
            "a centre drill whose pilot is under Ø{}",
            crate::post::exact_mm(narrowest)
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
    let Some(setup) = doc.setups.get(si) else {
        return (vec![], String::new());
    };
    let mut out = vec![];
    let (mut added, mut missing) = (vec![], vec![]);
    for (oi, op) in setup.operations.iter().enumerate() {
        if !wants_centre(op, &doc.tools, &setup.machine) || centred(setup, oi) {
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
    let Some(setup) = doc.setups.get_mut(si) else {
        return;
    };
    for (oi, op) in needed.into_iter().rev() {
        setup.operations.insert(oi.min(setup.operations.len()), op);
    }
}
/// Puts the centre drillings setup `si` lacks into it (`needed`); what to
/// say of them.
pub fn complete(doc: &mut Document, si: usize, mesh: Option<&Mesh>) -> String {
    let (ops, said) = needed(doc, si, mesh);
    insert(doc, si, ops);
    said
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
