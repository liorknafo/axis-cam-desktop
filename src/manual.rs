use crate::{
    cam::Geometry,
    mesh::{Bounds, Mesh, rotation},
    model::{Document, Fields, Machine, Setup, get, num},
    surface::Selection,
};

const TURNS: [&str; 3] = ["turn_x", "turn_y", "turn_z"];

/// The manual mill: a DRO, one axis at a time, steps instead of G-code.
pub fn is_manual(m: &Machine) -> bool {
    get(&m.p, "kind") == "Manual"
}

/// A machine worked by hand: the manual mill and the manual lathe. Neither
/// has a program, a post, an export or a UC100 Run, nor a work offset of its own.
pub fn is_hand(m: &Machine) -> bool {
    is_manual(m) || crate::lathe::is_lathe(m)
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
        ("edge_finder", "10"),
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

/// The comma-separated positive numbers of the machine's setting `key`,
/// lowest first. Empty when any entry is not a positive number.
pub fn list(m: &Machine, key: &str) -> Vec<f32> {
    let parsed: Option<Vec<f32>> = get(&m.p, key)
        .split(',')
        .map(|s| s.trim().parse::<f32>().ok())
        .map(|v| v.filter(|v| v.is_finite() && *v > 0.))
        .collect();
    let mut list = parsed.unwrap_or_default();
    list.sort_by(f32::total_cmp);
    list
}

/// The gearbox's speeds, lowest first; a variable drive gives its two ends.
/// Empty when any entry is not a positive number.
pub fn gears(m: &Machine) -> Vec<f32> {
    if let Some((a, b)) = variable(m) {
        return vec![a, b];
    }
    list(m, "speeds")
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
/// feed and material checks apply to a manual machine as to any other. A
/// lathe has no hand feed; its travel is its swing and its centres.
pub fn settle(m: &mut Machine) {
    if let Some(top) = gears(m).last() {
        m.p.insert("rpm".into(), top.to_string());
    }
    if let Some(feed) = num(&m.p, "hand_feed") {
        m.p.insert("feed".into(), feed.to_string());
    }
    if crate::lathe::is_lathe(m) {
        for (travel, size) in [("x", "swing"), ("y", "swing"), ("z", "centres")] {
            if let Some(v) = num(&m.p, size).filter(|v| *v > 0.) {
                m.p.insert(travel.into(), v.to_string());
            }
        }
    }
}

/// Whether a tool or holder with these `machines` ids fits `m`. A list fits
/// the machines it names. Empty fits every CNC machine and no manual machine
/// or lathe, except for a tool or holder made for a lathe (`lathe_only`: a
/// lathe tool, a chuck), which then fits lathes and nothing else.
pub fn fits(machines: &str, lathe_only: bool, m: &Machine) -> bool {
    if machines.trim().is_empty() {
        return if lathe_only {
            crate::lathe::is_lathe(m)
        } else {
            !is_hand(m)
        };
    }
    machines.split(',').any(|id| id.trim() == m.id)
}

/// Makes the wizard's draft `kind` ("Manual", "Lathe" or "CNC"). Choosing
/// Manual or Lathe fills that machine's defaults and keeps the name, units,
/// travel and power entered.
pub fn set_kind(m: &mut Machine, kind: &str) {
    let (fresh, new) = match kind {
        "Manual" => (!is_manual(m), defaults()),
        "Lathe" => (!crate::lathe::is_lathe(m), crate::lathe::defaults()),
        _ => {
            m.p.insert("kind".into(), "CNC".into());
            return;
        }
    };
    if fresh {
        for (k, v) in new.p {
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
/// consecutive setups on different machines, the `move_time` of the hand
/// machine in the pair (the longer when both are), 0 when neither is.
pub fn move_minutes(doc: &Document) -> f32 {
    doc.setups
        .windows(2)
        .filter(|w| w[0].machine.id != w[1].machine.id)
        .map(|w| {
            w.iter()
                .map(|s| &s.machine)
                .filter(|m| is_hand(m))
                .map(|m| num(&m.p, "move_time").unwrap_or(0.))
                .fold(0., f32::max)
        })
        .sum()
}

/// The export's line for a manual mill's setup, which has no program.
pub fn no_gcode(setup: &str) -> String {
    format!("{setup}: manual mill, no G-code — follow the steps on the MACHINE page.")
}

/// `no_gcode` for a setup on the hand machine `m`: the mill's or the lathe's.
pub fn no_gcode_on(m: &Machine, setup: &str) -> String {
    if crate::lathe::is_lathe(m) {
        format!("{setup}: manual lathe, no G-code — follow the steps on the MACHINE page.")
    } else {
        no_gcode(setup)
    }
}

/// What `box_from_face` answers for anything but one flat face that faces up
/// or sideways.
pub const ONE_FACE: &str = "Select one flat face that faces up or sideways.";
const NO_STOCK: &str = "There is no stock to remove beside that face.";
/// cos 1°: how near an axis a face's normal must be.
const SQUARE: f32 = 0.999_847_7;
/// How far above a floor part material has to rise to stop the box (mm).
const RISE: f32 = 0.01;
/// Contact closer than this is not overlap (mm).
const TOUCH: f32 = 0.001;

/// `poly` cut to the half-space `p[axis] >= at` (`above`) or `p[axis] <= at`.
fn clip(poly: Vec<[f32; 3]>, axis: usize, above: bool, at: f32) -> Vec<[f32; 3]> {
    let inside = |p: &[f32; 3]| if above { p[axis] >= at } else { p[axis] <= at };
    let mut out = vec![];
    for (i, a) in poly.iter().enumerate() {
        let b = &poly[(i + 1) % poly.len()];
        if inside(a) {
            out.push(*a);
        }
        if inside(a) != inside(b) {
            let t = (at - a[axis]) / (b[axis] - a[axis]);
            out.push(std::array::from_fn(|k| a[k] + t * (b[k] - a[k])));
        }
    }
    out
}

/// The Box for the one flat face `selection` selects, in work coordinates
/// (`x_min x_max y_min y_max z_top z_bottom`), within `stock`, the box the
/// setup starts from in its frame (mm).
///
/// A selection is a list of ids of connected coplanar regions of the
/// tessellation (`surface::group`), so one face is exactly one id. Its normal
/// (outward by the mesh's winding, as everywhere) must be within 1° of the
/// frame's +Z, a floor, or of ±X or ±Y, a wall; a ceiling, a slanted face and
/// a facet of a curve are refused.
/// - A floor: from the stock top down to the face's highest point; its extent
///   grown in X and then in Y, on each side up to the stock's edge or the
///   first part material that rises above the floor over the block's width.
/// - A wall: from the stock top down to the wall's lowest point; along the
///   wall its extent; from the wall out to the stock's edge on its normal side.
///
/// The limits of a floor are worked out on the triangles, not on a sampled
/// grid, which can step past a wall that stands inside one of its cells.
pub fn box_from_face(
    setup: &Setup,
    mesh: &Mesh,
    selection: &Selection,
    stock: Bounds,
) -> Result<[f32; 6], String> {
    let &[id] = selection.ids.as_slice() else {
        return Err(ONE_FACE.into());
    };
    let surface = mesh
        .surfaces
        .get(id)
        .filter(|_| selection.matches(mesh))
        .ok_or(ONE_FACE)?;
    let angles = ["rx", "ry", "rz"].map(|k| num(&setup.p, k).unwrap_or(0.));
    let n = rotation(surface.normal, angles);
    let axis = (0..3)
        .find(|&k| n[k].abs() >= SQUARE)
        .filter(|&k| k < 2 || n[2] > 0.)
        .ok_or(ONE_FACE)?;
    let geometry = Geometry::new(setup, mesh);
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for p in geometry
        .triangles
        .iter()
        .filter(|t| t.feature == Some(id))
        .flat_map(|t| t.p)
    {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let s = stock;
    let inside = |k: usize| [lo[k].max(s.min[k]), hi[k].min(s.max[k])];
    let top = s.max[2];
    let floor = (if axis == 2 { hi[2] } else { lo[2] }).max(s.min[2]);
    // The block's x and y ranges: a wall's side is open out to the stock.
    let mut r = [inside(0), inside(1)];
    if axis < 2 {
        r[axis] = if n[axis] > 0. {
            [hi[axis], s.max[axis]]
        } else {
            [s.min[axis], lo[axis]]
        };
    }
    if floor >= top - TOUCH || r.iter().any(|v| v[0] >= v[1]) {
        return Err(NO_STOCK.into());
    }
    if axis == 2 {
        // Only a triangle that rises above the floor is material over it.
        let over: Vec<_> = geometry
            .triangles
            .iter()
            .filter(|t| t.feature != Some(id) && t.p.iter().any(|p| p[2] > floor + RISE))
            .collect();
        for k in 0..2 {
            let span = r[1 - k];
            let mut limit = [s.min[k], s.max[k]];
            for t in &over {
                // What of it rises above the floor over the block's width.
                let poly = [
                    (2, true, floor + RISE),
                    (1 - k, true, span[0] + TOUCH),
                    (1 - k, false, span[1] - TOUCH),
                ]
                .into_iter()
                .fold(t.p.to_vec(), |poly, (axis, above, at)| {
                    clip(poly, axis, above, at)
                });
                if poly.is_empty() {
                    continue;
                }
                let near = poly.iter().map(|p| p[k]).fold(f32::INFINITY, f32::min);
                let far = poly.iter().map(|p| p[k]).fold(f32::NEG_INFINITY, f32::max);
                // Material that reaches beyond an edge stops the block there.
                if far > r[k][1] + TOUCH {
                    limit[1] = limit[1].min(near.max(r[k][1]));
                }
                if near < r[k][0] - TOUCH {
                    limit[0] = limit[0].max(far.min(r[k][0]));
                }
            }
            r[k] = limit;
        }
    }
    let zero = crate::post::datum(setup, stock, mesh)?;
    // To work coordinates, to 0.001 mm.
    let work = |k: usize, v: f32| ((v - zero[k]) * 1000.).round() / 1000.;
    Ok([
        work(0, r[0][0]),
        work(0, r[0][1]),
        work(1, r[1][0]),
        work(1, r[1][1]),
        work(2, top),
        work(2, floor),
    ])
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
        assert_eq!(get(&m.p, "edge_finder"), "10");
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
        assert!(fits("", false, &cnc));
        assert!(fits("  ", false, &cnc));
        assert!(!fits("", false, &manual));
    }
    #[test]
    fn a_machines_list_fits_the_named_ids_whatever_their_kind() {
        let (cnc, manual, other) = (Machine::new(3), defaults(), Machine::new(4));
        let list = format!("{}, {}", cnc.id, manual.id);
        assert!(fits(&list, false, &cnc));
        assert!(fits(&list, false, &manual));
        assert!(!fits(&list, false, &other));
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
            "edge_finder",
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
    /// The example block turned over about X, so the pockets cut from its
    /// bottom open upward: 120 × 80 × 40 mm, the stock its box, the work zero
    /// on the top's centre.
    fn block() -> (Setup, Mesh) {
        let mesh = crate::mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/four-setup-block.3mf"),
        )
        .unwrap();
        let mut setup = Setup::new(defaults(), "G54");
        setup.p.insert("rx".into(), "180".into());
        (setup, mesh)
    }
    /// Plan Z of the block's top.
    const TOP: f32 = 43.428_57;
    /// The one face of the part whose box in the setup's frame is `[lo, hi]`.
    fn face(setup: &Setup, mesh: &Mesh, lo: [f32; 3], hi: [f32; 3]) -> Selection {
        let geometry = Geometry::new(setup, mesh);
        let near = |a: [f32; 3], b: [f32; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 0.01);
        let found: Vec<usize> = (0..mesh.surfaces.len())
            .filter(|&id| {
                let (mut l, mut h) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
                for p in geometry
                    .triangles
                    .iter()
                    .filter(|t| t.feature == Some(id))
                    .flat_map(|t| t.p)
                {
                    for k in 0..3 {
                        l[k] = l[k].min(p[k]);
                        h[k] = h[k].max(p[k]);
                    }
                }
                near(l, lo) && near(h, hi)
            })
            .collect();
        assert_eq!(found.len(), 1, "{lo:?} {hi:?}: {found:?}");
        Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: found,
        }
    }
    /// `setup` with `mm` more stock on each of `sides` (`top`, `left`...).
    fn thicker(setup: &Setup, sides: &[&str], mm: &str) -> Setup {
        let mut setup = setup.clone();
        for side in sides {
            setup.p.insert(format!("stock_{side}"), mm.into());
        }
        setup
    }
    fn stock_of(setup: &Setup, mesh: &Mesh) -> Bounds {
        crate::stock::resolve(setup, Some(mesh)).unwrap().stock
    }
    fn close(a: &[f32], b: &[f32]) {
        assert!(
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.002),
            "{a:?} != {b:?}"
        );
    }
    /// The Box `b` generated on `setup` with a Ø8 end mill.
    fn generated(setup: &Setup, mesh: &Mesh, b: [f32; 6]) -> Result<(), String> {
        let mut tool = crate::model::Tool::new("End mill 8", "Flat end mill", "8");
        for (k, v) in [("length", "40"), ("stickout", "60")] {
            tool.p.insert(k.into(), v.into());
        }
        tool.p.insert("machines".into(), setup.machine.id.clone());
        let mut op = Operation::new(Strategy::Box, tool.id.clone());
        for (k, v) in crate::model::BOX_FIELDS.iter().zip(b) {
            op.p.insert((*k).into(), v.to_string());
        }
        let plan = crate::cam::generate(setup, &op, &tool, mesh)?;
        for m in &plan.moves {
            assert!(
                (0..3).filter(|&k| m.from[k] != m.to[k]).count() <= 1,
                "{m:?}"
            );
        }
        assert!(plan.moves.iter().any(|m| m.cutting_pass));
        Ok(())
    }
    #[test]
    fn a_floor_grows_to_the_stock_edges_and_passes_the_part_check() {
        let (setup, mesh) = block();
        // The block's top, now facing up, with 10 mm of stock over it and
        // 10 mm more beside it on the right.
        let setup = thicker(&thicker(&setup, &["top"], "10"), &["right"], "10");
        let top = face(&setup, &mesh, [-59.4, -39.4, TOP], [59.4, 39.4, TOP]);
        let b = box_from_face(&setup, &mesh, &top, stock_of(&setup, &mesh)).unwrap();
        // The stock's centre is at X 5: its edges at −60 and 70 are −65 and 65.
        close(&b, &[-65., 65., -40., 40., 0., -10.]);
        generated(&setup, &mesh, b).unwrap();
        // A box a millimetre lower is in the part.
        let deeper = [b[0], b[1], b[2], b[3], b[4], b[5] - 1.];
        let error = generated(&setup, &mesh, deeper).unwrap_err();
        assert!(error.starts_with("The box cuts into the part"), "{error}");
    }
    #[test]
    fn a_floor_in_walls_keeps_its_extent_and_its_height_is_the_boxs_floor() {
        let (setup, mesh) = block();
        let stock = stock_of(&setup, &mesh);
        // The 38 × 44 pocket floor, 6 mm down, in walls all round.
        let pocket = face(&setup, &mesh, [-44., -22., TOP - 6.], [-6., 22., TOP - 6.]);
        let b = box_from_face(&setup, &mesh, &pocket, stock).unwrap();
        close(&b, &[-44., -6., -22., 22., 0., -6.]);
        // Its corners are rounded: the box's are in the part, and say so.
        let error = generated(&setup, &mesh, b).unwrap_err();
        assert!(error.starts_with("The box cuts into the part"), "{error}");
        // A floor between a wall and the stock's edge, a ledge 2 mm up on both
        // sides and a wall at its inner end: only the open end grows.
        let keyway = face(&setup, &mesh, [54., -12., TOP - 27.], [60., 12., TOP - 27.]);
        let b = box_from_face(&setup, &mesh, &keyway, stock).unwrap();
        close(&b, &[54., 60., -12., 12., 0., -27.]);
        // Under the keyway's roof the box is in the part, and says so.
        let error = generated(&setup, &mesh, b).unwrap_err();
        assert!(error.starts_with("The box cuts into the part"), "{error}");
    }
    #[test]
    fn a_wall_gives_a_step_box_open_on_its_normal_side() {
        let (setup, mesh) = block();
        let setup = thicker(&setup, &["left", "right", "front", "back"], "10");
        let stock = stock_of(&setup, &mesh);
        // The block's four walls, 10 mm of stock round them, from the top
        // down to where each wall starts (36.4 mm, 4.03 mm up from the bottom).
        for (lo, hi, want) in [
            ([60., -37., 4.03], [60., 37., 42.83], [60., 70., -37., 37.]),
            (
                [-60., -37., 4.03],
                [-60., 37., 42.83],
                [-70., -60., -37., 37.],
            ),
            ([-57., 40., 4.03], [57., 40., 42.83], [-57., 57., 40., 50.]),
            (
                [-57., -40., 4.03],
                [57., -40., 42.83],
                [-57., 57., -50., -40.],
            ),
        ] {
            let wall = face(&setup, &mesh, lo, hi);
            let b = box_from_face(&setup, &mesh, &wall, stock).unwrap();
            close(
                &b,
                &[
                    want[0],
                    want[1],
                    want[2],
                    want[3],
                    0.,
                    4.0286 - 53.4286 + 10.,
                ],
            );
            generated(&setup, &mesh, b).unwrap();
        }
        // A pocket's wall: out to the stock's edge, over the pocket and beyond.
        let setup = thicker(&setup, &["left", "right", "front", "back"], "0");
        let inner = face(&setup, &mesh, [-44., -18., TOP - 6.], [-44., 18., TOP]);
        let b = box_from_face(&setup, &mesh, &inner, stock_of(&setup, &mesh)).unwrap();
        close(&b, &[-44., 60., -18., 18., 0., -6.]);
    }
    #[test]
    fn a_wall_on_the_stock_edge_leaves_nothing_to_remove() {
        let (setup, mesh) = block();
        let stock = stock_of(&setup, &mesh);
        let wall = face(&setup, &mesh, [60., -37., 4.03], [60., 37., 42.83]);
        assert_eq!(
            box_from_face(&setup, &mesh, &wall, stock),
            Err(NO_STOCK.to_string())
        );
        // The top face is at the stock top.
        let top = face(&setup, &mesh, [-59.4, -39.4, TOP], [59.4, 39.4, TOP]);
        assert_eq!(
            box_from_face(&setup, &mesh, &top, stock),
            Err(NO_STOCK.to_string())
        );
    }
    #[test]
    fn a_slanted_curved_downward_or_several_faces_are_refused() {
        let (setup, mesh) = block();
        let stock = stock_of(&setup, &mesh);
        let ask = |ids: Vec<usize>| {
            let selection = Selection {
                mesh_fingerprint: mesh.fingerprint.clone(),
                ids,
            };
            box_from_face(&setup, &mesh, &selection, stock)
        };
        let id = |lo, hi| face(&setup, &mesh, lo, hi).ids[0];
        // A 45° chamfer of the block's edge.
        let chamfer = id([-57., 39.4, TOP - 0.6], [57., 40., TOP]);
        // The ceiling of a round hole, facing down.
        let ceiling = id([-53.4975, 22.5006, 9.4286], [-42.5, 33.4994, 9.4286]);
        // One facet of a bore: neither square to X nor to Y.
        let facet = mesh
            .surfaces
            .iter()
            .position(|s| s.normal[0].abs() > 0.2 && s.normal[1].abs() > 0.2 && s.normal[2] == 0.)
            .expect("a bore");
        let wall = id([-44., -18., TOP - 6.], [-44., 18., TOP]);
        assert!(ask(vec![wall]).is_ok());
        for ids in [
            vec![chamfer],
            vec![ceiling],
            vec![facet],
            vec![],
            vec![wall, wall + 1],
        ] {
            assert_eq!(ask(ids.clone()), Err(ONE_FACE.to_string()), "{ids:?}");
        }
        assert_eq!(ask(vec![mesh.surfaces.len()]), Err(ONE_FACE.to_string()));
        let stale = Selection {
            mesh_fingerprint: "0".repeat(64),
            ids: vec![wall],
        };
        assert_eq!(
            box_from_face(&setup, &mesh, &stale, stock),
            Err(ONE_FACE.to_string())
        );
    }
}
