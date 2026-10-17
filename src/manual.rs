use crate::model::{Document, Fields, Machine, Setup, get, num};

const TURNS: [&str; 3] = ["turn_x", "turn_y", "turn_z"];

pub fn is_manual(m: &Machine) -> bool {
    get(&m.p, "kind") == "Manual"
}

pub fn defaults() -> Machine {
    let mut m = Machine::new(3);
    for (k, v) in [
        ("name", "Manual Mill"),
        ("kind", "Manual"),
        ("x", "700"),
        ("y", "300"),
        ("z", "400"),
        ("speeds", "90,150,250,400,640,1000,1600"),
        ("hand_feed", "150"),
        ("hand_rapid", "1500"),
        ("turn_x", "5"),
        ("turn_y", "5"),
        ("turn_z", "2.5"),
        ("dro_comp", "Yes"),
        ("pass_time", "20"),
        ("tool_time", "3"),
        ("move_time", "10"),
    ] {
        m.p.insert(k.into(), v.into());
    }
    settle(&mut m);
    m
}

/// The `min-max` of a variable drive.
fn variable(m: &Machine) -> Option<(f32, f32)> {
    let (a, b) = get(&m.p, "speeds").split_once('-')?;
    let (a, b) = (a.trim().parse::<f32>().ok()?, b.trim().parse::<f32>().ok()?);
    (a.is_finite() && b.is_finite() && 0. < a && a < b).then_some((a, b))
}

/// The gearbox's speeds, lowest first; a variable drive gives its two ends.
/// Empty when any entry is not a positive number.
pub fn gears(m: &Machine) -> Vec<f32> {
    if let Some((a, b)) = variable(m) {
        return vec![a, b];
    }
    let parsed: Option<Vec<f32>> = get(&m.p, "speeds")
        .split(',')
        .map(|s| s.trim().parse::<f32>().ok())
        .map(|v| v.filter(|v| v.is_finite() && *v > 0.))
        .collect();
    let mut gears = parsed.unwrap_or_default();
    gears.sort_by(f32::total_cmp);
    gears
}

/// The speed the machine can run nearest to `rpm`.
pub fn gear(m: &Machine, rpm: f32) -> f32 {
    if let Some((a, b)) = variable(m) {
        return ((rpm / 10.).round() * 10.).clamp(a, b);
    }
    gears(m)
        .into_iter()
        .min_by(|a, b| (a - rpm).abs().total_cmp(&(b - rpm).abs()))
        .unwrap_or(rpm)
}

/// Whether `rpm` is a speed the machine runs: within 1 % of a gear, or
/// anywhere in the range of a variable drive.
pub fn on_gear(m: &Machine, rpm: f32) -> bool {
    match variable(m) {
        Some((a, b)) => (a..=b).contains(&rpm),
        None => (gear(m, rpm) - rpm).abs() <= rpm * 0.01,
    }
}

/// The machine's speeds as the checks list them.
pub fn speeds(m: &Machine) -> String {
    let list: Vec<&str> = get(&m.p, "speeds").split(',').map(str::trim).collect();
    list.join(", ")
}

/// Puts a new operation's `rpm` on a gear and its `feed` at most the hand feed.
pub fn snap(m: &Machine, p: &mut Fields) {
    if let Some(rpm) = num(p, "rpm") {
        p.insert("rpm".into(), gear(m, rpm).to_string());
    }
    if let Some((feed, hand)) = num(p, "feed").zip(num(&m.p, "hand_feed")) {
        p.insert("feed".into(), feed.min(hand).to_string());
    }
}

/// Handwheel turns per minute that give `feed` mm/min on an axis (0 X, 1 Y, 2 Z).
pub fn turns(m: &Machine, axis: usize, feed: f32) -> f32 {
    TURNS
        .get(axis)
        .and_then(|k| num(&m.p, k))
        .filter(|per| *per > 0.)
        .map_or(0., |per| feed / per)
}

/// Sets `rpm` to the highest gear and `feed` to the hand feed, so the rpm,
/// feed and material checks apply to a manual machine as to any other.
pub fn settle(m: &mut Machine) {
    if let Some(top) = gears(m).last() {
        m.p.insert("rpm".into(), top.to_string());
    }
    if let Some(feed) = num(&m.p, "hand_feed") {
        m.p.insert("feed".into(), feed.to_string());
    }
}

/// Whether a tool or holder with these `machines` ids fits `m`. Empty fits
/// every CNC machine and no manual machine.
pub fn fits(machines: &str, m: &Machine) -> bool {
    if machines.trim().is_empty() {
        return !is_manual(m);
    }
    machines.split(',').any(|id| id.trim() == m.id)
}

/// Makes the wizard's draft `kind` ("Manual" or "CNC"). Choosing Manual fills
/// the manual defaults and keeps the name, units, travel and power entered.
pub fn set_kind(m: &mut Machine, kind: &str) {
    if kind != "Manual" {
        m.p.insert("kind".into(), "CNC".into());
    } else if !is_manual(m) {
        for (k, v) in defaults().p {
            if !matches!(k.as_str(), "name" | "units" | "x" | "y" | "z" | "power") {
                m.p.insert(k, v);
            }
        }
    }
}

/// `machines` with `id` added or taken off, as the check boxes edit it.
pub fn toggle(machines: &str, id: &str, on: bool) -> String {
    let mut ids: Vec<&str> = machines
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != id)
        .collect();
    if on {
        ids.push(id);
    }
    ids.join(",")
}

/// The changes of tool between the enabled operations of a manual setup,
/// counting the first tool.
pub fn tool_changes(setup: &Setup) -> usize {
    let mut last = None;
    setup
        .operations
        .iter()
        .filter(|o| o.enabled)
        .filter(|o| last.replace(&o.tool) != Some(&o.tool))
        .count()
}

/// The minutes the stock spends moving between machines: for each pair of
/// consecutive setups on different machines, the `move_time` of the manual
/// machine in the pair (the longer when both are), 0 when neither is manual.
pub fn move_minutes(doc: &Document) -> f32 {
    doc.setups
        .windows(2)
        .filter(|w| w[0].machine.id != w[1].machine.id)
        .map(|w| {
            w.iter()
                .map(|s| &s.machine)
                .filter(|m| is_manual(m))
                .map(|m| num(&m.p, "move_time").unwrap_or(0.))
                .fold(0., f32::max)
        })
        .sum()
}

/// The export's line for a manual setup, which has no program.
pub fn no_gcode(setup: &str) -> String {
    format!("{setup}: manual mill, no G-code — follow the steps on the MACHINE page.")
}

/// Why a strategy is dimmed on a setup of the manual mill.
pub const ONE_AXIS: &str = "The big mill cuts one axis at a time.";
/// Why Box is dimmed on a setup of a CNC machine.
pub const BOX_ON_CNC: &str = "Box runs on a manual mill.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Operation, Strategy, fields};
    fn with(m: &mut Machine, k: &str, v: &str) {
        m.p.insert(k.into(), v.into());
    }
    #[test]
    fn only_a_machine_of_kind_manual_is_manual() {
        let mut m = Machine::new(3);
        assert!(!is_manual(&m));
        with(&mut m, "kind", "CNC");
        assert!(!is_manual(&m));
        with(&mut m, "kind", "Manual");
        assert!(is_manual(&m));
    }
    #[test]
    fn the_defaults_are_a_valid_manual_machine() {
        let m = defaults();
        assert!(is_manual(&m));
        assert_eq!(m.issues(), Vec::<String>::new());
        assert_eq!(get(&m.p, "speeds"), "90,150,250,400,640,1000,1600");
        assert_eq!(get(&m.p, "hand_feed"), "150");
        assert_eq!(get(&m.p, "hand_rapid"), "1500");
        assert_eq!(get(&m.p, "turn_z"), "2.5");
        assert_eq!(get(&m.p, "dro_comp"), "Yes");
        assert_eq!(get(&m.p, "pass_time"), "20");
        assert_eq!(get(&m.p, "tool_time"), "3");
        assert_eq!(get(&m.p, "move_time"), "10");
        assert_eq!(get(&m.p, "rpm"), "1600");
        assert_eq!(get(&m.p, "feed"), "150");
    }
    #[test]
    fn gears_are_parsed_from_a_list_or_a_range() {
        let mut m = defaults();
        with(&mut m, "speeds", "250, 90 ,150");
        assert_eq!(gears(&m), vec![90., 150., 250.]);
        with(&mut m, "speeds", "90, x");
        assert!(gears(&m).is_empty());
        with(&mut m, "speeds", "100-2000");
        assert_eq!(gears(&m), vec![100., 2000.]);
    }
    #[test]
    fn a_variable_drive_clamps_and_rounds_to_ten() {
        let mut m = defaults();
        with(&mut m, "speeds", "100-2000");
        assert_eq!(gear(&m, 50.), 100.);
        assert_eq!(gear(&m, 5000.), 2000.);
        assert_eq!(gear(&m, 1234.), 1230.);
    }
    #[test]
    fn fixed_gears_pick_the_nearest() {
        let m = defaults();
        assert_eq!(gear(&m, 600.), 640.);
        assert_eq!(gear(&m, 100.), 90.);
        assert_eq!(gear(&m, 9000.), 1600.);
    }
    #[test]
    fn settle_takes_the_top_gear_and_the_hand_feed() {
        let mut m = defaults();
        with(&mut m, "speeds", "100, 400,2500");
        with(&mut m, "hand_feed", "120");
        settle(&mut m);
        assert_eq!(get(&m.p, "rpm"), "2500");
        assert_eq!(get(&m.p, "feed"), "120");
        with(&mut m, "speeds", "100-1800");
        settle(&mut m);
        assert_eq!(get(&m.p, "rpm"), "1800");
    }
    #[test]
    fn a_speed_is_on_a_gear_within_one_percent_or_in_the_range_of_a_variable_drive() {
        let mut m = defaults();
        assert!(on_gear(&m, 640.));
        assert!(on_gear(&m, 644.));
        assert!(!on_gear(&m, 600.));
        assert_eq!(speeds(&m), "90, 150, 250, 400, 640, 1000, 1600");
        with(&mut m, "speeds", "100-2000");
        assert!(on_gear(&m, 1234.));
        assert!(!on_gear(&m, 2100.));
        assert_eq!(speeds(&m), "100-2000");
    }
    #[test]
    fn a_new_operations_speed_and_feed_are_snapped_to_the_machine() {
        let m = defaults();
        let mut p = fields(&[("rpm", "2400"), ("feed", "280")]);
        snap(&m, &mut p);
        assert_eq!((get(&p, "rpm"), get(&p, "feed")), ("1600", "150"));
        let mut p = fields(&[("rpm", "600"), ("feed", "120")]);
        snap(&m, &mut p);
        assert_eq!((get(&p, "rpm"), get(&p, "feed")), ("640", "120"));
    }
    #[test]
    fn handwheel_turns_follow_the_axis_pitch() {
        let m = defaults();
        assert_eq!(turns(&m, 0, 150.), 30.);
        assert_eq!(turns(&m, 2, 150.), 60.);
        assert_eq!(turns(&m, 3, 150.), 0.);
    }
    #[test]
    fn an_empty_machines_list_fits_cnc_machines_only() {
        let (cnc, manual) = (Machine::new(3), defaults());
        assert!(fits("", &cnc));
        assert!(fits("  ", &cnc));
        assert!(!fits("", &manual));
    }
    #[test]
    fn a_machines_list_fits_the_named_ids_whatever_their_kind() {
        let (cnc, manual, other) = (Machine::new(3), defaults(), Machine::new(4));
        let list = format!("{}, {}", cnc.id, manual.id);
        assert!(fits(&list, &cnc));
        assert!(fits(&list, &manual));
        assert!(!fits(&list, &other));
    }
    #[test]
    fn choosing_manual_fills_the_defaults_and_keeps_what_was_entered() {
        let mut m = Machine::new(3);
        for (k, v) in [
            ("name", "Old"),
            ("x", "900"),
            ("power", "1500"),
            ("units", "inch"),
        ] {
            with(&mut m, k, v);
        }
        set_kind(&mut m, "Manual");
        assert!(is_manual(&m));
        assert_eq!(get(&m.p, "name"), "Old");
        assert_eq!(get(&m.p, "x"), "900");
        assert_eq!(get(&m.p, "power"), "1500");
        assert_eq!(get(&m.p, "units"), "inch");
        assert_eq!(get(&m.p, "speeds"), "90,150,250,400,640,1000,1600");
        assert_eq!(get(&m.p, "rpm"), "1600");
        with(&mut m, "hand_feed", "100");
        set_kind(&mut m, "Manual");
        assert_eq!(get(&m.p, "hand_feed"), "100");
        set_kind(&mut m, "CNC");
        assert!(!is_manual(&m));
    }
    #[test]
    fn the_check_boxes_add_and_take_off_one_id() {
        assert_eq!(toggle("", "a", true), "a");
        assert_eq!(toggle("a", "b", true), "a,b");
        assert_eq!(toggle("a, b", "a", true), "b,a");
        assert_eq!(toggle("a,b", "a", false), "b");
        assert_eq!(toggle("a", "a", false), "");
        assert_eq!(toggle("a", "b", false), "a");
    }
    #[test]
    fn a_manual_machine_needs_its_gears_hand_speeds_and_times() {
        let m = defaults();
        for k in [
            "speeds",
            "hand_feed",
            "hand_rapid",
            "turn_x",
            "turn_y",
            "turn_z",
        ] {
            let mut bad = m.clone();
            with(&mut bad, k, if k == "speeds" { "" } else { "0" });
            assert_eq!(bad.issues().len(), 1, "{k}");
        }
        for k in ["pass_time", "tool_time", "move_time"] {
            let mut bad = m.clone();
            with(&mut bad, k, "-1");
            assert_eq!(bad.issues().len(), 1, "{k}");
            with(&mut bad, k, "0");
            assert!(bad.issues().is_empty(), "{k}");
        }
        let mut bad = m.clone();
        with(&mut bad, "speeds", "100-50");
        assert_eq!(bad.issues().len(), 1);
    }
    #[test]
    fn a_manual_machine_needs_no_post_or_probe_fields() {
        let mut m = defaults();
        for k in [
            "post_tool_length",
            "post_retract",
            "probe_feed",
            "controller",
        ] {
            m.p.remove(k);
        }
        assert_eq!(m.issues(), Vec::<String>::new());
        with(&mut m, "controller", "UC100");
        assert_eq!(m.issues(), Vec::<String>::new());
    }
    #[test]
    fn a_cnc_machine_still_reads_its_own_rules() {
        let mut m = Machine::new(3);
        assert!(m.issues().is_empty());
        with(&mut m, "controller", "UC100");
        assert_eq!(m.issues().len(), 1);
    }
    fn setup_of(m: &Machine, tools: &[(&str, bool)]) -> Setup {
        let mut setup = Setup::new(m.clone(), "G54");
        setup.operations = tools
            .iter()
            .map(|(tool, on)| {
                let mut op = Operation::new(Strategy::Box, (*tool).into());
                op.enabled = *on;
                op
            })
            .collect();
        setup
    }
    #[test]
    fn tool_changes_count_the_first_tool_and_each_change_of_enabled_operations() {
        let m = defaults();
        let count = |tools: &[(&str, bool)]| tool_changes(&setup_of(&m, tools));
        assert_eq!(count(&[]), 0);
        assert_eq!(count(&[("a", true)]), 1);
        assert_eq!(count(&[("a", true), ("a", true)]), 1);
        assert_eq!(count(&[("a", true), ("b", true), ("a", true)]), 3);
        // A disabled operation neither brings a tool nor splits a run.
        assert_eq!(count(&[("a", true), ("b", false), ("a", true)]), 1);
        assert_eq!(count(&[("a", false), ("b", true)]), 1);
    }
    #[test]
    fn the_stock_moves_between_machines_cost_the_manual_mills_move_time() {
        let (big, cnc) = (defaults(), Machine::new(3));
        let doc = |machines: &[&Machine]| Document {
            setups: machines.iter().map(|m| setup_of(m, &[])).collect(),
            ..Default::default()
        };
        // Manual, CNC, manual, CNC: the stock moves three times.
        assert_eq!(move_minutes(&doc(&[&big, &cnc, &big, &cnc])), 30.);
        assert_eq!(move_minutes(&doc(&[&big, &big, &cnc, &cnc])), 10.);
        assert_eq!(move_minutes(&doc(&[&cnc, &cnc])), 0.);
        assert_eq!(move_minutes(&doc(&[&big])), 0.);
        assert_eq!(move_minutes(&doc(&[])), 0.);
        // Two CNC machines: no manual machine in the pair.
        assert_eq!(move_minutes(&doc(&[&cnc, &Machine::new(3)])), 0.);
        let mut slow = big.clone();
        slow.id = "slow".into();
        slow.p.insert("move_time".into(), "25".into());
        assert_eq!(move_minutes(&doc(&[&slow, &cnc, &big])), 25. + 10.);
        // Between two manual machines: the longer.
        assert_eq!(move_minutes(&doc(&[&slow, &big])), 25.);
    }
}
