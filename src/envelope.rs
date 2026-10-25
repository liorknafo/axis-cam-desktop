//! What fits in the machine (LIO-252): the clearance from the spindle nose
//! at Z top to the table, and the opening under the gantry, beside the
//! travel and the top speed, each marked ASSUMED until it is measured.
//!
//! Travel says how far the tool goes, not whether a tall part fits under it.
//! A setup's stack-up is the table, the workholding under the stock (a
//! vise's body, or the rotary unit's centre height), the stock, and over its
//! highest cut the tool's stick-out and holder up to the spindle nose, which
//! at Z top is `nose_to_table` over the table. Everything on the table
//! passes under the gantry as Y moves, so its height and width must be inside
//! `gantry_height` and `gantry_width`. `problems` refuses a setup and tool
//! whose numbers do not fit, naming them; the same refusal reaches export,
//! Run and the plan check through `rotary_check::problems`.
//!
//! A machine that gives no figure is not checked for it (an older file, or a
//! machine nobody has measured): the check only knows what it is told. The
//! shipped Jingyan CNC4030 has placeholders, ASSUMED, chosen on the cautious
//! side of the fit: a low gantry and nose height refuse more. A tool too
//! short to reach down is the other way round (a low nose height lets it
//! pass), so that check waits for a nose height marked Measured.
use crate::mesh::Mesh;
use crate::model::{Document, HOLDER_LENGTH, Machine, get, num};
use crate::simulation::Simulation;

/// The figures a machine marks as measured or ASSUMED: the machine key, the
/// key of its source, and its name in a message.
pub const FIGURES: [(&str, &str, &str); 7] = [
    ("x", "x_source", "X travel"),
    ("y", "y_source", "Y travel"),
    ("z", "z_source", "Z travel"),
    ("rpm", "rpm_source", "Maximum spindle speed"),
    (
        "nose_to_table",
        "nose_to_table_source",
        "Spindle nose to table at Z top",
    ),
    (
        "gantry_height",
        "gantry_height_source",
        "Gantry clearance height",
    ),
    (
        "gantry_width",
        "gantry_width_source",
        "Gantry clearance width",
    ),
];
/// What a figure's source says: the choices of the form.
pub const ASSUMED: &str = "ASSUMED";
pub const MEASURED: &str = "Measured";
/// The three clearances: the `p` key, the form's label, and the placeholder
/// the shipped Jingyan CNC4030 takes (`JINGYAN`). All in mm.
pub const CLEARANCES: [(&str, &str, &str); 3] = [
    (
        "nose_to_table",
        "Spindle nose to table at Z top · mm",
        "160",
    ),
    ("gantry_height", "Gantry clearance height · mm", "120"),
    ("gantry_width", "Gantry clearance width · mm", "390"),
];
/// The shipped Jingyan CNC4030's travel (from a similar machine's spec sheet)
/// and top speed (its profile's placeholder), which are marked ASSUMED while
/// they are still the shipped numbers.
const JINGYAN: [(&str, &str); 4] = [("x", "390"), ("y", "280"), ("z", "55"), ("rpm", "24000")];
/// How far above the table a vise's stock rests when the library entry gives
/// no `base_height`, mm: a 100 mm vise's bed with a low pair of parallels.
/// Clamps and no workholding take none (the stock on the table).
pub const VISE_BASE: f32 = 40.;
/// How much a figure may be over its limit before it is refused, mm.
const EPS: f32 = 0.005;

/// Where a figure of the machine comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Nothing is recorded about it.
    Unmarked,
    /// A placeholder, not measured on the machine.
    Assumed,
    Measured,
}
/// The source of the figure `key` of `m`.
pub fn source(m: &Machine, key: &str) -> Source {
    let Some((_, source, _)) = FIGURES.iter().find(|f| f.0 == key) else {
        return Source::Unmarked;
    };
    match get(&m.p, source) {
        ASSUMED => Source::Assumed,
        MEASURED => Source::Measured,
        _ => Source::Unmarked,
    }
}
/// The names of `m`'s figures that are ASSUMED and given, for the form and
/// the planner's report.
pub fn assumed(m: &Machine) -> Vec<&'static str> {
    FIGURES
        .iter()
        .filter(|(key, _, _)| {
            source(m, key) == Source::Assumed && !get(&m.p, key).trim().is_empty()
        })
        .map(|f| f.2)
        .collect()
}
/// What a message says after an ASSUMED number.
fn flag(source: Source) -> &'static str {
    match source {
        Source::Assumed => " (ASSUMED, not measured: measure it and correct the machine)",
        _ => "",
    }
}

/// A clearance of machine `m` in mm, with where it comes from: None when it
/// gives none or not a positive number.
fn clearance(m: &Machine, key: &str) -> Option<(f32, Source)> {
    let inch = if get(&m.p, "units") == "inch" {
        25.4
    } else {
        1.
    };
    let v = num(&m.p, key).filter(|v| *v > 0.)?;
    Some((v * inch, source(m, key)))
}

/// What is wrong with `m`'s clearances and sources, for the machine form.
pub fn issues(m: &Machine) -> Vec<String> {
    let mut e = vec![];
    for (key, label, _) in CLEARANCES {
        let given = !get(&m.p, key).trim().is_empty();
        if given && num(&m.p, key).is_none_or(|v| v <= 0.) {
            let name = label.split(" · ").next().unwrap_or(label);
            e.push(format!("{name} must be greater than zero."));
        }
    }
    for (_, source, name) in FIGURES {
        if !matches!(get(&m.p, source), "" | ASSUMED | MEASURED) {
            e.push(format!("{name}: choose {ASSUMED} or {MEASURED}."));
        }
    }
    // The rotary unit stays on the table: its chuck passes under the gantry
    // whatever the job.
    if let Some(((height, _), unit)) = clearance(m, "gantry_height").zip(m.rotary_unit()) {
        let chuck = unit.centre_height + unit.chuck_diameter / 2.;
        if chuck > height + EPS {
            e.push(format!(
                "The rotary unit's chuck stands {} mm over the table (its {} mm centre height and half its {} mm), more than the {} mm under the gantry.",
                mm(chuck),
                mm(unit.centre_height),
                mm(unit.chuck_diameter),
                mm(height)
            ));
        }
    }
    // The spindle crosses the gantry over the whole X travel.
    if let Some(((width, _), x)) = clearance(m, "gantry_width").zip(clearance(m, "x"))
        && width + EPS < x.0
    {
        e.push(format!(
            "Gantry clearance width cannot be narrower than the {} mm X travel the spindle crosses it over.",
            mm(x.0)
        ));
    }
    e
}

/// Marks the figures of `m` the shipped Jingyan CNC4030 has as ASSUMED and
/// gives it the placeholders of `CLEARANCES`: the machine library's entries
/// and a saved copy that lacks them.
pub fn jingyan(m: &mut Machine) {
    for (key, value) in JINGYAN {
        m.p.insert(key.into(), value.into());
    }
    for (key, _, value) in CLEARANCES {
        m.p.insert(key.into(), value.into());
    }
    for (_, source, _) in FIGURES {
        m.p.insert(source.into(), ASSUMED.into());
    }
}
/// Whether `m` is a copy of the shipped Jingyan CNC4030: a UC100 machine on
/// the JY5800 profile `control::owners_machine` and `control::rotary_machine`
/// write.
fn is_jingyan(m: &Machine) -> bool {
    get(&m.p, "controller") == "UC100"
        && std::path::Path::new(get(&m.p, "uc100_profile"))
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.to_lowercase().starts_with("jy5800"))
}
/// Gives a machine saved before the clearances existed what the shipped one
/// has, ASSUMED: for the figures it lacks, so that nothing the operator
/// entered or measured is touched. Whether it changed anything.
fn adopt_machine(m: &mut Machine) -> bool {
    if !is_jingyan(m) {
        return false;
    }
    let mut changed = false;
    for (key, _, value) in CLEARANCES {
        if !m.p.contains_key(key) {
            m.p.insert(key.into(), value.into());
            m.p.insert(format!("{key}_source"), ASSUMED.into());
            changed = true;
        }
    }
    // A shipped number nobody has edited stays visibly ASSUMED.
    for (key, value) in JINGYAN {
        let source = format!("{key}_source");
        if !m.p.contains_key(&source) && get(&m.p, key) == value {
            m.p.insert(source, ASSUMED.into());
            changed = true;
        }
    }
    changed
}
/// Brings the machines of a saved document, the library's and the copy each
/// setup keeps, up to the clearances (`adopt_machine`). The note to show if
/// it changed any.
pub fn adopt(doc: &mut Document) -> Option<String> {
    let mut changed = false;
    for m in &mut doc.machines {
        changed |= adopt_machine(m);
    }
    for s in &mut doc.setups {
        changed |= adopt_machine(&mut s.machine);
    }
    changed.then(|| {
        "The Jingyan CNC4030 has a spindle-nose-to-table height and a gantry clearance now, ASSUMED until you measure them (Machine library → Travel & spindle).".into()
    })
}

/// `v` mm for a message: up to two decimals, without trailing zeros.
fn mm(v: f32) -> String {
    match format!("{v:.2}")
        .trim_end_matches('0')
        .trim_end_matches('.')
    {
        "-0" => "0".into(),
        text => text.into(),
    }
}

/// What stands on the table in a setup, heights above the table in mm.
#[derive(Debug, Clone, PartialEq)]
pub struct Stack {
    /// The stock's underside above the table.
    pub base: f32,
    /// Whether the height under the stock is taken, the library entry
    /// giving none.
    pub base_assumed: bool,
    /// The stock's top, and the highest thing in the setup's workholding
    /// or the stock (a clamp stands over it).
    pub stock_top: f32,
    pub top: f32,
    /// How far the stock and its workholding reach along X, mm.
    pub x: Option<[f32; 2]>,
    /// The setup's Z of the stock's underside, for a stack on a flat table:
    /// a height in the setup's frame is `base` + its Z less this. None for
    /// the stock held on the rotary unit, whose axis is not Z.
    pub z0: Option<f32>,
    /// The stack in words, for a message: what holds the stock how high up,
    /// and the stock's own height.
    pub what: String,
}
impl Stack {
    /// The tool tip's height over the table at setup Z `z`.
    fn height(&self, z: f32) -> Option<f32> {
        self.z0.map(|z0| self.base + z - z0)
    }
    /// The stack of setup `si` of `doc`, or None when it cannot be told:
    /// the stock or the unit's geometry is not known, which the other
    /// checks of the setup say.
    pub fn of(doc: &Document, mesh: &Mesh, si: usize) -> Option<Self> {
        let setup = &doc.setups[si];
        let machine = &setup.machine;
        // The stock on the rotary unit turns about its centreline over the
        // centre height; the chuck and the tailstock stand beside it.
        if crate::chuck::holds(setup) || crate::indexed::is_indexed(setup) {
            let base = doc.index_base(si).unwrap_or(si);
            let unit = doc.rotary_unit(machine)?;
            let (radius, x) = crate::rotary_check::bar_reach(doc, base, mesh)?;
            let stock_top = unit.centre_height + radius;
            // The unit's own chuck and tailstock are fixed: whether they pass
            // under the gantry is the unit's (`a_fit_diameter`), not the stock's.
            return Some(Self {
                base: unit.centre_height - radius,
                base_assumed: false,
                stock_top,
                top: stock_top,
                x: Some(x),
                z0: None,
                what: format!(
                    "the rotary centre {} mm over the table and the stock's {} mm reach from it",
                    mm(unit.centre_height),
                    mm(radius)
                ),
            });
        }
        let stock = doc.setup_stock(si, Some(mesh))?;
        let layout = crate::holding::layout(setup).ok()?;
        let holder = doc.holder(get(&setup.p, "holder"));
        let given = holder.and_then(|h| num(&h.p, "base_height").filter(|v| *v >= 0.));
        let (base, under, base_assumed) = match (&layout, given) {
            (crate::holding::Layout::None, _) => (0., "the table".to_string(), false),
            (_, Some(v)) => (
                v,
                format!("the {}", holder.map_or("workholding", |h| h.name())),
                false,
            ),
            (crate::holding::Layout::Vise { .. }, None) => (
                VISE_BASE,
                format!("the {}", holder.map_or("vise", |h| h.name())),
                true,
            ),
            _ => (0., "the table".to_string(), false),
        };
        let height = stock.dimensions()[2];
        let stock_top = base + height;
        let mut top = stock_top;
        let mut x = [stock.min[0], stock.max[0]];
        if let Some(holder) = holder {
            for b in crate::holding::boxes(&layout, holder, stock).unwrap_or_default() {
                top = top.max(base + b.bounds.max[2] - stock.min[2]);
                x = [x[0].min(b.bounds.min[0]), x[1].max(b.bounds.max[0])];
            }
        }
        let taken = if base_assumed {
            format!(
                " (taken: the workholding entry gives no \"Stock underside above the table\", and {} mm is a vise's typical)",
                mm(VISE_BASE)
            )
        } else {
            String::new()
        };
        let on = match base {
            0. => format!("on {under}"),
            b => format!("{} mm up on {under}{taken}", mm(b)),
        };
        Some(Self {
            base,
            base_assumed,
            stock_top,
            top,
            x: Some(x),
            z0: Some(stock.min[2]),
            what: format!("{on}, the stock's {} mm", mm(height)),
        })
    }
}

/// The tool of an operation as the spindle holds it: how far its tip is
/// under the spindle nose, and the stick-out and holder length that make it
/// (`rotary_check::in_spindle`'s, apart from the nose's width), and whether
/// the holder's length is taken because the tool gives none.
fn gauge(doc: &Document, id: &str, stickout: f32) -> (f32, f32, bool) {
    let (holder, assumed) = (doc.tools.iter().find(|t| t.id == id))
        .map_or((HOLDER_LENGTH, true), crate::model::Tool::holder_length);
    (stickout + holder, holder, assumed)
}

/// What does not fit in setup `si` of `doc` on its machine's clearances,
/// from its `sims` (the generated operations): the stock, its workholding
/// and the rotary unit under the gantry, and each operation's tool under the
/// spindle nose at Z top. Each with the index of its operation in the setup
/// (None for the setup's), named by the setup and the operation. Empty for a
/// machine that gives no clearance, and for a machine worked by hand.
pub fn problems(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    sims: &[(usize, Result<Simulation, String>)],
) -> Vec<(Option<usize>, String)> {
    let setup = &doc.setups[si];
    let machine = &setup.machine;
    if crate::manual::is_hand(machine) {
        return vec![];
    }
    let (nose, height, width) = (
        clearance(machine, "nose_to_table"),
        clearance(machine, "gantry_height"),
        clearance(machine, "gantry_width"),
    );
    if nose.is_none() && height.is_none() && width.is_none() {
        return vec![];
    }
    let Some(stack) = Stack::of(doc, mesh, si) else {
        return vec![];
    };
    let name = setup.name();
    let mut out = vec![];
    if let Some((limit, from)) = height
        && stack.top > limit + EPS
    {
        out.push((
            None,
            format!(
                "{name}: the part doesn't fit under the gantry: needs {} mm, has {} mm{}. It stands {}; stand it lower, or hold it lower.",
                mm(stack.top),
                mm(limit),
                flag(from),
                stack.what,
            ),
        ));
    }
    if let Some(((limit, from), [lo, hi])) = width.zip(stack.x)
        && hi - lo > limit + EPS
    {
        out.push((
            None,
            format!(
                "{name}: the part doesn't fit under the gantry: needs {} mm across X, has {} mm{}. The stock and its jaws or clamps reach {} mm along X; turn it to put its long side along Y.",
                mm(hi - lo),
                mm(limit),
                flag(from),
                mm(hi - lo),
            ),
        ));
    }
    let Some((nose, from)) = nose else {
        return out;
    };
    let travel = crate::post_check::machine_mm(machine, "z");
    for (oi, sim) in sims {
        let (Ok(sim), Some(op)) = (sim, setup.operations.get(*oi)) else {
            continue;
        };
        let (tool, holder, holder_assumed) = gauge(doc, &op.tool, sim.plan.tool_length);
        // The highest the tip goes: the stock's top, or a move above it.
        let moves = || sim.plan.moves.iter().flat_map(|m| [m.from[2], m.to[2]]);
        let high = moves()
            .filter_map(|z| stack.height(z))
            .fold(stack.stock_top, f32::max);
        let named = format!("{name}, {}", op.name());
        let taken = if holder_assumed {
            format!(
                " The holder's {} mm length is taken: the tool gives none (enter it in the Tool library).",
                mm(holder)
            )
        } else {
            String::new()
        };
        if high + tool > nose + EPS {
            out.push((
                Some(*oi),
                format!(
                    "{named}: the tool doesn't fit under the spindle: needs {} mm, has {} mm{} from the table to the spindle nose at Z top. The tip goes up to {} mm over the table ({}), and the tool is {} mm under the nose ({} mm stick-out and {} mm holder).{taken} Use a shorter tool, or stand the stock lower.",
                    mm(high + tool),
                    mm(nose),
                    flag(from),
                    mm(high),
                    stack.what,
                    mm(tool),
                    mm(sim.plan.tool_length),
                    mm(holder),
                ),
            ));
        }
        // A tool too short to reach the cut with the spindle at the bottom
        // of its travel. A nose height that is only assumed is not trusted to
        // refuse this: being low is the safe side of the fit above, and the
        // wrong side of this.
        if from == Source::Measured && travel > 0. {
            let lowest = nose - tool - travel;
            let low = moves()
                .filter_map(|z| stack.height(z))
                .fold(f32::INFINITY, f32::min);
            if low < lowest - EPS {
                out.push((
                    Some(*oi),
                    format!(
                        "{named}: the tool can't reach down: the cut goes to {} mm over the table, and with the {} mm tool the spindle at the bottom of its {} mm Z travel puts the tip at {} mm ({} mm to the nose, less the tool and the travel).{taken} Use a longer tool, or stand the stock higher.",
                        mm(low),
                        mm(tool),
                        mm(travel),
                        mm(lowest),
                        mm(nose),
                    ),
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Setup;
    use crate::post::fixture;

    /// A CNC machine with the placeholders, and Z travel of 55 mm.
    fn machine(nose: &str, height: &str, width: &str) -> Machine {
        let mut m = Machine::new(3);
        for (k, v) in [
            ("x", "390"),
            ("y", "280"),
            ("z", "55"),
            ("nose_to_table", nose),
            ("gantry_height", height),
            ("gantry_width", width),
        ] {
            m.p.insert(k.into(), v.into());
        }
        m
    }
    fn simulated(doc: &Document, mesh: &Mesh) -> Vec<(usize, Result<Simulation, String>)> {
        crate::run::simulated_for_tests(doc, mesh, 0)
    }
    /// `fixture::job` held in the starter vise on a machine of the given
    /// clearances, only its drilling enabled.
    fn drilled(m: Machine, base: Option<&str>) -> (Document, Mesh) {
        let (mut doc, mesh) = fixture::job();
        doc.adopt_holders();
        for op in &mut doc.setups[0].operations {
            op.enabled = op.name() == "Drilling";
        }
        let vise = doc
            .workholding
            .iter_mut()
            .find(|h| h.kind() == "Vise")
            .unwrap();
        match base {
            Some(b) => vise.p.insert("base_height".into(), b.into()),
            None => vise.p.remove("base_height"),
        };
        let id = vise.id.clone();
        let p = &mut doc.setups[0].p;
        p.insert("holder".into(), id);
        p.insert("holding".into(), "Vise".into());
        p.insert("vise_axis".into(), "Y".into());
        p.insert("vise_grip".into(), "8".into());
        doc.setups[0].machine = m;
        (doc, mesh)
    }

    #[test]
    fn a_machine_without_the_figures_is_not_checked_and_a_bad_one_is_said() {
        let (doc, mesh) = drilled(machine("", "", ""), Some("25"));
        assert!(problems(&doc, &mesh, 0, &simulated(&doc, &mesh)).is_empty());
        assert!(!Machine::new(3).p.contains_key("nose_to_table"));
        assert!(issues(&machine("150", "120", "390")).is_empty());
        assert!(issues(&machine("", "", "")).is_empty());
        assert_eq!(
            issues(&machine("0", "120", "390")),
            ["Spindle nose to table at Z top must be greater than zero."]
        );
        assert_eq!(
            issues(&machine("150", "abc", "390")),
            ["Gantry clearance height must be greater than zero."]
        );
        assert_eq!(
            issues(&machine("150", "120", "300")),
            [
                "Gantry clearance width cannot be narrower than the 390 mm X travel the spindle crosses it over."
            ]
        );
        let mut m = machine("150", "120", "390");
        m.p.insert("nose_to_table_source".into(), "guess".into());
        assert_eq!(
            issues(&m),
            ["Spindle nose to table at Z top: choose ASSUMED or Measured."]
        );
    }

    #[test]
    fn the_stack_up_is_the_vise_the_stock_and_the_tool_up_to_the_nose() {
        // The fixture's stock is 98 mm along Y; the vise holds it by 8 mm.
        let (doc, mesh) = drilled(machine("1000", "1000", "1000"), Some("25"));
        let stack = Stack::of(&doc, &mesh, 0).unwrap();
        let height = doc.setup_stock(0, Some(&mesh)).unwrap().dimensions()[2];
        assert_eq!(stack.base, 25.);
        assert!(!stack.base_assumed);
        assert_eq!(stack.stock_top, 25. + height);
        assert_eq!(stack.top, stack.stock_top);
        // A frame Z is the stock's underside at the base, rising with it.
        let stock = doc.setup_stock(0, Some(&mesh)).unwrap();
        assert_eq!(stack.height(stock.min[2]), Some(25.));
        assert_eq!(stack.height(stock.max[2]), Some(25. + height));
        // Roomy: the job passes, and a library vise that gives no height
        // is taken as the typical one, saying so.
        assert!(problems(&doc, &mesh, 0, &simulated(&doc, &mesh)).is_empty());
        let (doc, mesh) = drilled(machine("1000", "1000", "1000"), None);
        let stack = Stack::of(&doc, &mesh, 0).unwrap();
        assert_eq!((stack.base, stack.base_assumed), (VISE_BASE, true));
        assert!(stack.what.contains("taken"), "{}", stack.what);
        // Clamps and nothing stand on the table.
        let mut none = doc.clone();
        none.setups[0].p.insert("holding".into(), "None".into());
        let stack = Stack::of(&none, &mesh, 0).unwrap();
        assert_eq!((stack.base, stack.base_assumed), (0., false));
        assert!(
            stack.what.starts_with("on the table, the stock's "),
            "{}",
            stack.what
        );
    }

    #[test]
    fn a_part_taller_than_the_gantry_is_refused_naming_the_numbers() {
        let (doc, mesh) = drilled(machine("1000", "50", "1000"), Some("25"));
        let stack = Stack::of(&doc, &mesh, 0).unwrap();
        let found = problems(&doc, &mesh, 0, &simulated(&doc, &mesh));
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, None);
        let top = super::mm(stack.top);
        assert!(
            found[0].1.contains(&format!(
                "the part doesn't fit under the gantry: needs {top} mm, has 50 mm."
            )),
            "{}",
            found[0].1
        );
        // The same part under a gantry with the room passes, and one marked
        // ASSUMED says so in the refusal.
        let tall = stack.top + 1.;
        let (roomy, _) = drilled(machine("1000", &tall.to_string(), "1000"), Some("25"));
        assert!(problems(&roomy, &mesh, 0, &simulated(&roomy, &mesh)).is_empty());
        let (mut doc, mesh) = drilled(machine("1000", "50", "1000"), Some("25"));
        doc.setups[0]
            .machine
            .p
            .insert("gantry_height_source".into(), ASSUMED.into());
        let found = problems(&doc, &mesh, 0, &simulated(&doc, &mesh));
        assert!(
            found[0].1.contains("has 50 mm (ASSUMED, not measured"),
            "{}",
            found[0].1
        );
    }

    #[test]
    fn a_part_wider_than_the_gantry_is_refused() {
        let (doc, mesh) = drilled(machine("1000", "1000", "10"), Some("25"));
        let found = problems(&doc, &mesh, 0, &simulated(&doc, &mesh));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].1.contains("doesn't fit under the gantry: needs ")
                && found[0].1.contains(" mm across X, has 10 mm"),
            "{}",
            found[0].1
        );
    }

    #[test]
    fn a_tool_that_reaches_the_nose_is_refused_with_its_stick_out_and_holder() {
        let (mut doc, mesh) = drilled(machine("1000", "1000", "1000"), Some("25"));
        let sims = simulated(&doc, &mesh);
        let op = &doc.setups[0].operations[sims[0].0];
        let (stickout, id) = (
            sims[0].1.as_ref().unwrap().plan.tool_length,
            op.tool.clone(),
        );
        let tool = doc.tools.iter_mut().find(|t| t.id == id).unwrap();
        tool.p.insert("holder_length".into(), "20".into());
        let stack = Stack::of(&doc, &mesh, 0).unwrap();
        let sim = sims[0].1.as_ref().unwrap();
        let high = sim
            .plan
            .moves
            .iter()
            .flat_map(|m| [m.from[2], m.to[2]])
            .filter_map(|z| stack.height(z))
            .fold(stack.stock_top, f32::max);
        let needs = high + stickout + 20.;
        // A hair of room passes; a hair short is refused with the sum.
        doc.setups[0]
            .machine
            .p
            .insert("nose_to_table".into(), (needs + 0.1).to_string());
        assert!(problems(&doc, &mesh, 0, &sims).is_empty());
        doc.setups[0]
            .machine
            .p
            .insert("nose_to_table".into(), (needs - 0.1).to_string());
        let found = problems(&doc, &mesh, 0, &sims);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, Some(sims[0].0));
        let said = &found[0].1;
        assert!(
            said.contains(&format!(
                "the tool doesn't fit under the spindle: needs {} mm, has {} mm from the table",
                super::mm(needs),
                super::mm(needs - 0.1)
            )),
            "{said}"
        );
        assert!(
            said.contains(&format!(
                "{} mm stick-out and 20 mm holder",
                super::mm(stickout)
            )),
            "{said}"
        );
        assert!(!said.contains("is taken"), "{said}");
        // A tool that gives no holder length is taken as the short default.
        doc.tools
            .iter_mut()
            .find(|t| t.id == id)
            .unwrap()
            .p
            .remove("holder_length");
        let found = problems(&doc, &mesh, 0, &sims);
        assert!(
            found.is_empty() || found[0].1.contains("The holder's 10 mm length is taken"),
            "{found:?}"
        );
    }

    #[test]
    fn a_tool_too_short_to_reach_down_is_refused_only_for_a_measured_nose_height() {
        let (mut doc, mesh) = drilled(machine("150", "1000", "1000"), Some("25"));
        // A long Z travel, so that the span of the job is inside it.
        doc.setups[0].machine.p.insert("z".into(), "100".into());
        let sims = simulated(&doc, &mesh);
        // The drilling goes down through the stock to near the vise.
        let stack = Stack::of(&doc, &mesh, 0).unwrap();
        let sim = sims[0].1.as_ref().unwrap();
        let tool = sim.plan.tool_length + HOLDER_LENGTH;
        let low = sim
            .plan
            .moves
            .iter()
            .flat_map(|m| [m.from[2], m.to[2]])
            .filter_map(|z| stack.height(z))
            .fold(f32::INFINITY, f32::min);
        let id = doc.setups[0].operations[sims[0].0].tool.clone();
        doc.tools
            .iter_mut()
            .find(|t| t.id == id)
            .unwrap()
            .p
            .remove("holder_length");
        // The tip's lowest reach is `low` + 1 mm too high.
        let nose = low + 1. + tool + 100.;
        doc.setups[0]
            .machine
            .p
            .insert("nose_to_table".into(), nose.to_string());
        let found = problems(&doc, &mesh, 0, &sims);
        assert!(found.is_empty(), "{found:?}");
        doc.setups[0]
            .machine
            .p
            .insert("nose_to_table_source".into(), ASSUMED.into());
        assert!(problems(&doc, &mesh, 0, &sims).is_empty());
        doc.setups[0]
            .machine
            .p
            .insert("nose_to_table_source".into(), MEASURED.into());
        let found = problems(&doc, &mesh, 0, &sims);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0]
                .1
                .contains("the tool can't reach down: the cut goes to"),
            "{}",
            found[0].1
        );
        assert!(found[0].1.contains("100 mm Z travel"), "{}", found[0].1);
        // Lower by 2 mm: it reaches.
        doc.setups[0]
            .machine
            .p
            .insert("nose_to_table".into(), (nose - 2.).to_string());
        assert!(problems(&doc, &mesh, 0, &sims).is_empty());
    }

    #[test]
    fn the_shipped_jingyan_is_marked_assumed_and_an_edited_figure_is_not() {
        let mut m = Machine::new(3);
        m.p.insert("controller".into(), "UC100".into());
        m.p.insert(
            "uc100_profile".into(),
            "/home/u/AXIS CAM Studio/jy5800.toml".into(),
        );
        for (k, v) in [("x", "390"), ("y", "280"), ("z", "55"), ("rpm", "24000")] {
            m.p.insert(k.into(), v.into());
        }
        let mut library = m.clone();
        // The operator measured Z already and typed it.
        library.p.insert("z".into(), "60".into());
        let mut doc = Document {
            machines: vec![library, Machine::new(3)],
            setups: vec![Setup::new(m, "G54")],
            ..Default::default()
        };
        let note = adopt(&mut doc).unwrap();
        assert!(note.contains("ASSUMED"), "{note}");
        let lib = &doc.machines[0];
        for (key, _, value) in CLEARANCES {
            assert_eq!(get(&lib.p, key), value, "{key}");
            assert_eq!(source(lib, key), Source::Assumed, "{key}");
        }
        // Still the shipped numbers: ASSUMED; the typed 60 mm is left alone.
        assert_eq!(source(lib, "x"), Source::Assumed);
        assert_eq!(source(lib, "z"), Source::Unmarked);
        assert_eq!(get(&lib.p, "z"), "60");
        assert_eq!(
            assumed(lib),
            [
                "X travel",
                "Y travel",
                "Maximum spindle speed",
                "Spindle nose to table at Z top",
                "Gantry clearance height",
                "Gantry clearance width"
            ]
        );
        // The setup's own copy of the machine, and not another machine.
        assert_eq!(source(&doc.setups[0].machine, "z"), Source::Assumed);
        assert!(!doc.machines[1].p.contains_key("nose_to_table"));
        // A second time changes nothing, and a measured figure stays.
        assert!(adopt(&mut doc).is_none());
        let mut measured = doc.machines[0].clone();
        measured.p.insert("nose_to_table".into(), "162".into());
        measured
            .p
            .insert("nose_to_table_source".into(), MEASURED.into());
        doc.machines[0] = measured;
        assert!(adopt(&mut doc).is_none());
        assert_eq!(get(&doc.machines[0].p, "nose_to_table"), "162");
        assert!(!assumed(&doc.machines[0]).contains(&"Spindle nose to table at Z top"));
    }
    #[test]
    fn the_library_machines_carry_the_assumed_placeholders_and_are_valid() {
        let dir = tempfile::tempdir().unwrap();
        for m in [
            crate::control::owners_machine(dir.path()).unwrap(),
            crate::control::rotary_machine(dir.path()).unwrap(),
        ] {
            assert!(m.issues().is_empty(), "{:?}", m.issues());
            for (key, _, value) in CLEARANCES {
                assert_eq!(get(&m.p, key), value, "{key}");
            }
            // Every figure the profile only assumed is marked, and named.
            assert_eq!(
                assumed(&m),
                FIGURES.iter().map(|f| f.2).collect::<Vec<_>>(),
                "{}",
                m.name()
            );
        }
        // The profile itself is unchanged: it still says ASSUMED.
        assert!(include_str!("../assets/jy5800.toml").contains("ASSUMED 2026-10-11, NOT MEASURED"));
    }

    #[test]
    fn a_workspace_saved_before_the_clearances_adopts_them_once_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let mut old = crate::control::owners_machine(dir.path()).unwrap();
        for (key, _, _) in CLEARANCES {
            old.p.remove(key);
        }
        for (_, source, _) in FIGURES {
            old.p.remove(source);
        }
        let mut doc = Document::default();
        doc.machines = vec![old.clone()];
        doc.setups = vec![Setup::new(old, "G54")];
        doc.active = doc.setups[0].id.clone();
        let path = dir.path().join("workspace.json");
        crate::storage::save(&path, &doc).unwrap();
        let (loaded, notes) = crate::storage::load_reporting(&path).unwrap();
        assert!(notes.iter().any(|n| n.contains("ASSUMED")), "{notes:?}");
        for m in [&loaded.machines[0], &loaded.setups[0].machine] {
            assert_eq!(get(&m.p, "gantry_height"), "120");
            assert_eq!(source(m, "nose_to_table"), Source::Assumed);
            assert_eq!(get(&m.p, "x"), "390");
        }
        // Saved again, it loads without the note.
        crate::storage::save(&path, &loaded).unwrap();
        let (_, notes) = crate::storage::load_reporting(&path).unwrap();
        assert!(!notes.iter().any(|n| n.contains("ASSUMED")), "{notes:?}");
    }
    #[test]
    fn a_bar_on_the_rotary_unit_stands_at_its_centre_height_and_reach() {
        let (mut doc, mesh) = crate::indexed::tests::posting_bar_job(20., "100", "90, 180");
        let sim = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap();
        let (tool, _, _) = gauge(
            &doc,
            &doc.setups[1].operations[0].tool,
            sim.plan.tool_length,
        );
        let set = |doc: &mut Document, nose: f32, height: f32| {
            for (k, v) in [("nose_to_table", nose), ("gantry_height", height)] {
                doc.setups[0].machine.p.insert(k.into(), v.to_string());
            }
            doc.sync_indexed();
        };
        let sims = [(0, Ok(sim))];
        // The Ø40 bar on the assumed 70 mm centre height: 90 mm to its top.
        set(&mut doc, 90. + tool + 0.1, 90.1);
        let stack = Stack::of(&doc, &mesh, 1).unwrap();
        assert_eq!((stack.stock_top, stack.top), (90., 90.));
        assert!(problems(&doc, &mesh, 1, &sims).is_empty());
        set(&mut doc, 90. + tool - 0.1, 90.1);
        let found = problems(&doc, &mesh, 1, &sims);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0]
                .1
                .contains("the tool doesn't fit under the spindle")
                && found[0]
                    .1
                    .contains("the rotary centre 70 mm over the table and the stock's 20 mm reach"),
            "{}",
            found[0].1
        );
        set(&mut doc, 1000., 89.9);
        let found = problems(&doc, &mesh, 1, &sims);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0]
                .1
                .contains("doesn't fit under the gantry: needs 90 mm, has 89.9 mm"),
            "{}",
            found[0].1
        );
    }

    #[test]
    fn export_run_and_the_plan_check_see_the_refusal() {
        let (doc, mesh) = drilled(machine("1000", "50", "1000"), Some("25"));
        let said = |messages: &[String]| {
            messages
                .iter()
                .filter(|m| m.contains("doesn't fit under the gantry"))
                .count()
        };
        let posted = crate::post::post_job(&doc, &mesh);
        assert_eq!(said(posted.setups[0].result.as_ref().unwrap_err()), 1);
        assert_eq!(said(&crate::run::ready(&doc, &mesh, 0, 0).problems), 1);
        // The same job on a gantry with the room is ready.
        let (doc, mesh) = drilled(machine("1000", "1000", "1000"), Some("25"));
        assert_eq!(said(&crate::run::ready(&doc, &mesh, 0, 0).problems), 0);
        let posted = crate::post::post_job(&doc, &mesh);
        let errors = posted.setups[0].result.clone().err().unwrap_or_default();
        assert_eq!(said(&errors), 0);
    }
}
