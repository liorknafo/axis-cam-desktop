//! What a planner preview changes in the open job, as the lines the preview
//! page lists under "What changes against the current job".
use crate::{
    model::{Document, Fields, Operation, Setup, get},
    ui::{clip, plural},
};

/// The longest value a line shows, in characters.
const VALUE: usize = 40;

/// The lines saying what `after`, a preview job, changes in `before`, the
/// open job: removed, new and changed setups, the operations removed, added,
/// moved, changed and reordered inside the setups both keep, and how many
/// operations stay as they are. Empty when `before` has no operation, as
/// there is nothing to change then.
pub fn changes(before: &Document, after: &Document) -> Vec<String> {
    let before_ops: usize = before.setups.iter().map(|s| s.operations.len()).sum();
    if before_ops == 0 {
        return vec![];
    }
    let in_after = |id: &str| {
        after
            .setups
            .iter()
            .any(|s| s.operations.iter().any(|o| o.id == id))
    };
    if !before
        .setups
        .iter()
        .any(|s| s.operations.iter().any(|o| in_after(&o.id)))
    {
        return vec![format!(
            "Replaces the current plan: {} in {}.",
            plural(before_ops, "operation"),
            plural(before.setups.len(), "setup")
        )];
    }
    let mut lines = vec![];
    for (i, setup) in before.setups.iter().enumerate() {
        if !after.setups.iter().any(|s| s.id == setup.id) {
            lines.push(format!(
                "Removed setup {} “{}” with its {}",
                i + 1,
                clip(setup.name(), VALUE),
                plural(setup.operations.len(), "operation")
            ));
        }
    }
    let mut unchanged = 0;
    for (i, setup) in after.setups.iter().enumerate() {
        let number = i + 1;
        let Some((was, old)) = before
            .setups
            .iter()
            .enumerate()
            .find(|(_, s)| s.id == setup.id)
        else {
            lines.push(new_setup(number, setup));
            continue;
        };
        let differences = setup_differences(&old.p, &setup.p);
        if !differences.is_empty() {
            lines.push(format!(
                "Setup {number} “{}”: {}",
                clip(setup.name(), VALUE),
                differences.join("; ")
            ));
        }
        let removed: Vec<String> = old
            .operations
            .iter()
            .filter(|o| !in_after(&o.id))
            .map(operation_name)
            .collect();
        if !removed.is_empty() {
            lines.push(format!(
                "Removed from Setup {number}: {}",
                removed.join(", ")
            ));
        }
        let anywhere = |id: &str| {
            before
                .setups
                .iter()
                .enumerate()
                .find_map(|(n, s)| s.operations.iter().find(|o| o.id == id).map(|o| (n, o)))
        };
        let added: Vec<String> = setup
            .operations
            .iter()
            .filter(|o| anywhere(&o.id).is_none())
            .map(|o| format!("{} with {}", operation_name(o), tool_label(after, &o.tool)))
            .collect();
        if !added.is_empty() {
            lines.push(format!("Added in Setup {number}: {}", added.join(", ")));
        }
        for o in &setup.operations {
            let Some((from, old_op)) = anywhere(&o.id) else {
                continue;
            };
            let mut differences = operation_differences(before, old_op, after, o);
            let moved = from != was;
            if moved {
                differences.insert(0, format!("moved from Setup {}", from + 1));
            }
            if differences.is_empty() {
                unchanged += 1;
            } else {
                lines.push(format!(
                    "Setup {number} · {} “{}”: {}",
                    o.strategy.name(),
                    clip(o.name(), VALUE),
                    differences.join(", ")
                ));
            }
        }
        let order = |from: &Setup, other: &Setup| -> Vec<String> {
            let shared = |id: &str| other.operations.iter().any(|o| o.id == id);
            from.operations
                .iter()
                .filter(|o| shared(&o.id))
                .map(|o| o.id.clone())
                .collect()
        };
        if order(old, setup) != order(setup, old) {
            lines.push(format!("Setup {number}: operations reordered"));
        }
    }
    if lines.is_empty() {
        return vec!["Nothing changes: every setup and operation stays as it is.".into()];
    }
    if unchanged > 0 {
        lines.push(format!("{} unchanged.", plural(unchanged, "operation")));
    }
    lines
}

/// A setup the preview adds: its place, its name and its operations.
fn new_setup(number: usize, setup: &Setup) -> String {
    let mut line = format!(
        "New setup {number} “{}” with {}",
        clip(setup.name(), VALUE),
        plural(setup.operations.len(), "operation")
    );
    if !setup.operations.is_empty() {
        let names: Vec<&str> = setup.operations.iter().map(|o| o.strategy.name()).collect();
        line.push_str(&format!(": {}", names.join(", ")));
    }
    line
}

/// An operation as a line names it: its strategy and its name in quotes.
fn operation_name(op: &Operation) -> String {
    format!("{} “{}”", op.strategy.name(), clip(op.name(), VALUE))
}

/// What differs between a kept setup's settings, in the words of the
/// setup page. Only the settings the planner writes are compared.
fn setup_differences(old: &Fields, new: &Fields) -> Vec<String> {
    let differs = |key: &str| get(old, key) != get(new, key);
    let any = |keys: &[&str]| keys.iter().any(|k| differs(k));
    let mut out = vec![];
    if differs("name") {
        out.push(format!(
            "renamed “{}” → “{}”",
            clip(get(old, "name"), VALUE),
            clip(get(new, "name"), VALUE)
        ));
    }
    if differs("origin") {
        out.push(format!(
            "work zero {} → {}",
            value(old, "origin"),
            value(new, "origin")
        ));
    }
    if differs("notes") {
        out.push("workholding note changed".into());
    }
    if any(&["holding", "holder"]) {
        out.push("holder changed".into());
    }
    if any(&["vise_axis", "vise_offset", "vise_grip", "clamps"]) {
        out.push("holding layout changed".into());
    }
    out
}

/// What differs between an operation and its changed version, as phrases:
/// the settings, then the tool, the surfaces and whether it is on.
fn operation_differences(
    before: &Document,
    old: &Operation,
    after: &Document,
    new: &Operation,
) -> Vec<String> {
    let mut out = vec![];
    if old.strategy != new.strategy {
        out.push(format!(
            "strategy {} → {}",
            old.strategy.name(),
            new.strategy.name()
        ));
    }
    if get(&old.p, "name") != get(&new.p, "name") {
        out.push(format!(
            "renamed “{}” → “{}”",
            clip(get(&old.p, "name"), VALUE),
            clip(get(&new.p, "name"), VALUE)
        ));
    }
    let keys: std::collections::BTreeSet<&String> = old.p.keys().chain(new.p.keys()).collect();
    for key in keys {
        if key == "name" || get(&old.p, key) == get(&new.p, key) {
            continue;
        }
        out.push(format!(
            "{} {} → {}",
            label(key),
            value(&old.p, key),
            value(&new.p, key)
        ));
    }
    if old.tool != new.tool {
        out.push(format!(
            "tool {} → {}",
            tool_label(before, &old.tool),
            tool_label(after, &new.tool)
        ));
    }
    let surfaces = |op: &Operation| {
        op.surfaces.as_ref().map(|s| {
            let mut ids = s.ids.clone();
            ids.sort_unstable();
            ids
        })
    };
    if surfaces(old) != surfaces(new) {
        out.push("surfaces changed".into());
    }
    if old.enabled != new.enabled {
        out.push(
            if new.enabled {
                "turned on"
            } else {
                "turned off"
            }
            .into(),
        );
    }
    out
}

/// A tool as the lines name it: "T3", or its name when it has no number.
fn tool_label(doc: &Document, id: &str) -> String {
    match doc.tools.iter().find(|t| t.id == id) {
        Some(tool) => match tool.number() {
            Some(n) => format!("T{n}"),
            None => clip(tool.name(), VALUE),
        },
        None => "(none)".into(),
    }
}

/// A setting's value, shortened; "(none)" when it is missing or empty.
fn value(p: &Fields, key: &str) -> String {
    let v = get(p, key);
    if v.trim().is_empty() {
        "(none)".into()
    } else {
        clip(v, VALUE)
    }
}

/// A setting's name in the words of the operation page.
fn label(key: &str) -> &str {
    match key {
        "rpm" => "spindle speed",
        "stepover" => "stepover %",
        "allowance" => "radial allowance",
        "axial_allowance" => "axial allowance",
        "bottom" => "bottom offset",
        "finish_passes" => "finish passes",
        "ramp_angle" => "ramp angle",
        "surface_stepover" => "surface stepover",
        "hole_diameter" => "hole diameter",
        "cycle" => "drill cycle",
        "pocket_entry" => "entry",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Machine, Strategy, Tool};
    /// A job with tools T1 and T2 (a drill, no number) and the given setups.
    fn job(setups: Vec<Setup>) -> Document {
        let mut one = Tool::new("Flat 10", "Flat end mill", "10");
        one.p.insert("number".into(), "1".into());
        let mut three = Tool::new("Flat 6", "Flat end mill", "6");
        three.p.insert("number".into(), "3".into());
        let drill = Tool::new("Drill 4", "Drill", "4");
        Document {
            tools: vec![one, three, drill],
            setups,
            ..Default::default()
        }
    }
    fn setup(name: &str, ops: Vec<Operation>) -> Setup {
        let mut s = Setup::new(Machine::new(3), "G54");
        s.p.insert("name".into(), name.into());
        s.operations = ops;
        s
    }
    /// An operation with the tool at `tool` of `doc`.
    fn op(doc: &Document, strategy: Strategy, name: &str, tool: usize) -> Operation {
        let mut o = Operation::new(strategy, doc.tools[tool].id.clone());
        o.p.insert("name".into(), name.into());
        o
    }
    /// Two setups: "Top" with Face, Pocket, Contour; "Flip" with Face and Drill.
    fn base() -> Document {
        let mut doc = job(vec![]);
        let top = setup(
            "Top",
            vec![
                op(&doc, Strategy::Face, "Face", 0),
                op(&doc, Strategy::Pocket, "Pocket 1", 0),
                op(&doc, Strategy::Contour, "Outline", 0),
            ],
        );
        let flip = setup(
            "Flip",
            vec![
                op(&doc, Strategy::Face, "Face 2", 0),
                op(&doc, Strategy::Drill, "Holes", 2),
            ],
        );
        doc.setups = vec![top, flip];
        doc
    }
    #[test]
    fn a_job_without_operations_has_no_changes() {
        let after = base();
        assert!(changes(&job(vec![]), &after).is_empty());
        assert!(changes(&job(vec![setup("Empty", vec![])]), &after).is_empty());
    }
    #[test]
    fn a_plan_with_none_of_the_operations_replaces_the_plan() {
        let before = base();
        let mut after = base();
        after.tools = before.tools.clone();
        assert_eq!(
            changes(&before, &after),
            ["Replaces the current plan: 5 operations in 2 setups."]
        );
        let mut one = job(vec![]);
        one.setups = vec![setup("Top", vec![op(&one, Strategy::Face, "Face", 0)])];
        assert_eq!(
            changes(&one, &after),
            ["Replaces the current plan: 1 operation in 1 setup."]
        );
    }
    #[test]
    fn the_same_job_changes_nothing() {
        let before = base();
        assert_eq!(
            changes(&before, &before.clone()),
            ["Nothing changes: every setup and operation stays as it is."]
        );
    }
    #[test]
    fn a_removed_setup_is_named_with_its_operations_not_listed() {
        let before = base();
        let mut after = before.clone();
        after.setups.remove(1);
        assert_eq!(
            changes(&before, &after),
            [
                "Removed setup 2 “Flip” with its 2 operations".to_string(),
                "3 operations unchanged.".to_string()
            ]
        );
    }
    #[test]
    fn a_new_setup_lists_its_strategies() {
        let before = base();
        let mut after = before.clone();
        let extra = setup(
            "Back",
            vec![
                op(&after, Strategy::Face, "Face 3", 0),
                op(&after, Strategy::Pocket, "Pocket 3", 1),
            ],
        );
        after.setups.push(extra);
        let lines = changes(&before, &after);
        assert_eq!(
            lines,
            [
                "New setup 3 “Back” with 2 operations: Facing, Pocket".to_string(),
                "5 operations unchanged.".to_string()
            ]
        );
        after.setups[2].operations.truncate(1);
        assert_eq!(
            changes(&before, &after)[0],
            "New setup 3 “Back” with 1 operation: Facing"
        );
    }
    #[test]
    fn a_kept_setup_lists_what_differs_in_its_settings() {
        let mut before = base();
        before.setups[1]
            .p
            .insert("origin".into(), "Model origin".into());
        let mut after = before.clone();
        let p = &mut after.setups[1].p;
        p.insert("origin".into(), "Stock top center".into());
        p.insert("notes".into(), "Clamp on the walls".into());
        p.insert("holder".into(), "vise-1".into());
        p.insert("vise_grip".into(), "8".into());
        assert_eq!(
            changes(&before, &after)[0],
            "Setup 2 “Flip”: work zero Model origin → Stock top center; workholding note changed; holder changed; holding layout changed"
        );
        // Other keys of the setup are not compared.
        let mut after = before.clone();
        after.setups[0].p.insert("stock_x".into(), "999".into());
        after.setups[0].p.insert("offset".into(), "G55".into());
        assert_eq!(
            changes(&before, &after),
            ["Nothing changes: every setup and operation stays as it is."]
        );
    }
    #[test]
    fn a_renamed_setup_shows_both_names() {
        let before = base();
        let mut after = before.clone();
        after.setups[1].p.insert("name".into(), "Underside".into());
        assert_eq!(
            changes(&before, &after)[0],
            "Setup 2 “Underside”: renamed “Flip” → “Underside”"
        );
    }
    #[test]
    fn removed_and_added_operations_are_listed_by_setup() {
        let before = base();
        let mut after = before.clone();
        after.setups[0].operations.remove(2);
        let added = op(&after, Strategy::Drill, "Holes 2", 2);
        after.setups[0].operations.push(added);
        let lines = changes(&before, &after);
        assert_eq!(
            lines,
            [
                "Removed from Setup 1: Contour “Outline”".to_string(),
                "Added in Setup 1: Drilling “Holes 2” with Drill 4".to_string(),
                "4 operations unchanged.".to_string()
            ]
        );
    }
    #[test]
    fn a_changed_operation_lists_each_difference_in_friendly_words() {
        let before = base();
        let mut after = before.clone();
        let pocket = &mut after.setups[0].operations[1];
        pocket.p.insert("stepdown".into(), "1".into());
        pocket.p.insert("rpm".into(), "9000".into());
        pocket.p.insert("pocket_entry".into(), "Plunge".into());
        pocket.p.insert("zeta".into(), "3".into());
        pocket.tool = after.tools[1].id.clone();
        pocket.surfaces = Some(crate::surface::Selection {
            mesh_fingerprint: "m".into(),
            ids: vec![1, 2],
        });
        pocket.enabled = false;
        pocket.p.insert("name".into(), "Pocket A".into());
        assert_eq!(
            changes(&before, &after),
            [
                "Setup 1 · Pocket “Pocket A”: renamed “Pocket 1” → “Pocket A”, entry Profile ramp → Plunge, spindle speed 8000 → 9000, stepdown 2 → 1, zeta (none) → 3, tool T1 → T3, surfaces changed, turned off".to_string(),
                "4 operations unchanged.".to_string()
            ]
        );
    }
    #[test]
    fn a_tool_without_a_number_is_shown_by_name_and_long_values_are_clipped() {
        let before = base();
        let mut after = before.clone();
        let pocket = &mut after.setups[0].operations[1];
        pocket.tool = after.tools[2].id.clone();
        pocket.p.insert("boundary".into(), "x".repeat(100));
        let lines = changes(&before, &after);
        assert!(lines[0].contains("tool T1 → Drill 4"), "{lines:?}");
        assert!(
            lines[0].contains(&format!("boundary Selected geometry → {}…", "x".repeat(40))),
            "{lines:?}"
        );
    }
    #[test]
    fn selections_compare_by_surface_not_by_order() {
        let mut before = base();
        before.setups[0].operations[1].surfaces = Some(crate::surface::Selection {
            mesh_fingerprint: "m".into(),
            ids: vec![2, 1],
        });
        let mut after = before.clone();
        after.setups[0].operations[1].surfaces = Some(crate::surface::Selection {
            mesh_fingerprint: "m".into(),
            ids: vec![1, 2],
        });
        assert_eq!(
            changes(&before, &after),
            ["Nothing changes: every setup and operation stays as it is."]
        );
    }
    #[test]
    fn reordered_operations_are_reported_for_the_setup() {
        let before = base();
        let mut after = before.clone();
        after.setups[0].operations.swap(0, 2);
        assert_eq!(
            changes(&before, &after),
            [
                "Setup 1: operations reordered".to_string(),
                "5 operations unchanged.".to_string()
            ]
        );
        // Removing an operation is not a reorder.
        let mut after = before.clone();
        after.setups[0].operations.remove(0);
        assert!(
            !changes(&before, &after)
                .iter()
                .any(|l| l.contains("reordered"))
        );
    }
    #[test]
    fn an_operation_moved_to_another_setup_is_reported() {
        let before = base();
        let mut after = before.clone();
        let moved = after.setups[0].operations.remove(2);
        after.setups[1].operations.push(moved);
        let lines = changes(&before, &after);
        assert_eq!(
            lines,
            [
                "Setup 2 · Contour “Outline”: moved from Setup 1".to_string(),
                "4 operations unchanged.".to_string()
            ]
        );
    }
    #[test]
    fn changes_come_in_the_order_removed_setups_then_each_setup() {
        let mut before = base();
        let third = setup("Back", vec![op(&before, Strategy::Face, "Face 3", 0)]);
        before.setups.push(third);
        let mut after = before.clone();
        after.setups.remove(2);
        after.setups[0].p.insert("name".into(), "Front".into());
        after.setups[0].operations.remove(1);
        after.setups[1].operations[1]
            .p
            .insert("feed".into(), "500".into());
        let lines = changes(&before, &after);
        assert_eq!(
            lines,
            [
                "Removed setup 3 “Back” with its 1 operation".to_string(),
                "Setup 1 “Front”: renamed “Top” → “Front”".to_string(),
                "Removed from Setup 1: Pocket “Pocket 1”".to_string(),
                "Setup 2 · Drilling “Holes”: feed 280 → 500".to_string(),
                "3 operations unchanged.".to_string()
            ]
        );
    }
}
