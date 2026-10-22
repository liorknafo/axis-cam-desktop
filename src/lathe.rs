//! The manual lathe: its machine profile and the conversion of a distance
//! into what a dial shows. The lathe has no DRO; every number the guide gives
//! is a dial picture (whole turns plus the reading) or a measurement.

use crate::{
    manual,
    mesh::Mesh,
    model::{Machine, Setup, Tool, get, num},
    thread::Spec,
};

/// What the guide says in place of a cross-slide number while the dial check
/// has not been answered (`cross_reads` empty).
pub const CHECK_THE_DIAL: &str = "Check the dial first";

/// The slides whose handwheel has a dial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slide {
    Cross,
    Top,
    Carriage,
    Quill,
}

impl Slide {
    /// The machine's keys for the dial's numbers per turn and divisions.
    fn keys(self) -> (&'static str, &'static str) {
        match self {
            Slide::Cross => ("cross_turn", "cross_div"),
            Slide::Top => ("top_turn", "top_div"),
            Slide::Carriage => ("carriage_turn", "carriage_div"),
            Slide::Quill => ("quill_turn", "quill_div"),
        }
    }
}

/// What the cross-slide dial's numbers mean, the dial check's answer
/// (`cross_reads`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reads {
    /// A reading of `d` takes `d` off the diameter.
    Diameter,
    /// A reading of `d` takes `d` off the radius, `2d` off the diameter.
    Radius,
}
impl Reads {
    /// The machine's `cross_reads` value.
    pub fn key(self) -> &'static str {
        match self {
            Reads::Diameter => "Diameter",
            Reads::Radius => "Radius",
        }
    }
}

/// The tool types a lathe uses besides drills and taps (held in the
/// tailstock). A die cuts an outside thread by hand, with no gearbox.
pub const TOOLS: [&str; 6] = [
    "Turning tool",
    "Boring bar",
    "Grooving tool",
    "Parting blade",
    "Threading tool",
    "Die",
];

/// Why a strategy other than Turn and Drill is dimmed on a lathe setup.
pub const ONLY_TURN: &str = "The lathe turns and drills on the centre.";
/// Why a lathe's Drill refuses Helical milling (a copied operation's).
pub const HELICAL_ON_LATHE: &str =
    "The lathe drills on the centre: Helical milling is for a mill. Choose another drilling cycle.";
/// Why Turn is dimmed, and refused, on a setup of a mill.
pub const TURN_ON_MILL: &str = "Turn runs on a lathe.";
/// What a lathe Drill without a hole selection says when it is given one.
pub const DRILL_ON_CENTRE: &str =
    "A lathe drills on the centre line: remove the hole selection and set the depth.";

/// The kinds of Turn (`turn_kind`).
pub const TURN_KINDS: [&str; 7] = [
    "Face", "Outside", "Bore", "Chamfer", "Groove", "Part off", "Thread",
];
/// The edges a Chamfer breaks (`edge`).
pub const EDGES: [&str; 3] = ["End outside", "End inside", "Shoulder"];
/// The sides a Thread is cut on (`thread_side`).
pub const THREAD_SIDES: [&str; 2] = ["Outside", "Inside"];
/// Every field a Turn kind has besides the spindle speed and the allowance:
/// the keys that change meaning with the kind.
pub const TURN_KEYS: [&str; 13] = [
    "face_off",
    "to_diameter",
    "diameter",
    "z_end",
    "depth",
    "finish",
    "edge",
    "size",
    "z",
    "width",
    "thread_side",
    "pitch",
    "passes",
];

/// The tool type a Turn of `kind` cuts with; None for an unknown kind.
pub fn tool_for(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "Face" | "Outside" | "Chamfer" => "Turning tool",
        "Bore" => "Boring bar",
        "Groove" => "Grooving tool",
        "Part off" => "Parting blade",
        "Thread" => "Threading tool",
        _ => return None,
    })
}

/// Whether a Turn of `kind` cuts with a tool of type `tool`: the type
/// `tool_for` names, and for a Thread also a die.
pub fn takes(kind: &str, tool: &str) -> bool {
    tool_for(kind) == Some(tool) || (kind == "Thread" && tool == "Die")
}
/// The thread size of a die (its `thread`), None for another tool or a size
/// off the table.
pub fn die_spec(tool: &Tool) -> Option<&'static Spec> {
    (get(&tool.p, "type") == "Die")
        .then(|| crate::thread::spec(get(&tool.p, "thread")))
        .flatten()
}

/// The settings a Turn of `kind` starts with (lathe work coordinates: Z 0 at
/// the free end, negative toward the chuck; diameters as diameters), the
/// spindle speed included. Empty for an unknown kind.
pub fn turn_defaults(kind: &str) -> &'static [(&'static str, &'static str)] {
    match kind {
        "Face" => &[("rpm", "450"), ("face_off", "1"), ("to_diameter", "0")],
        "Outside" => &[
            ("rpm", "710"),
            ("diameter", "40"),
            ("z_end", "-30"),
            ("depth", "2"),
            ("finish", "0.2"),
            ("allowance", "0"),
        ],
        "Bore" => &[
            ("rpm", "710"),
            ("diameter", "20"),
            ("z_end", "-20"),
            ("depth", "1"),
            ("finish", "0.1"),
            ("allowance", "0"),
        ],
        "Chamfer" => &[("rpm", "450"), ("edge", "End outside"), ("size", "1")],
        "Groove" => &[
            ("rpm", "280"),
            ("z", "-20"),
            ("width", "3"),
            ("diameter", "35"),
        ],
        "Part off" => &[("rpm", "180"), ("z", "-60")],
        "Thread" => &[
            ("rpm", "110"),
            ("thread_side", "Outside"),
            ("diameter", "40"),
            ("pitch", "1.5"),
            ("z_end", "-20"),
            ("passes", ""),
        ],
        _ => &[],
    }
}

/// The hands a turning tool has.
pub const HANDS: [&str; 3] = ["Right", "Left", "Neutral"];

/// Whether `kind` is a lathe tool type (`TOOLS`).
pub fn is_tool(kind: &str) -> bool {
    TOOLS.contains(&kind)
}

/// The keys a tool of type `kind` starts with, and their values: none for a
/// mill tool or a drill.
pub fn tool_defaults(kind: &str) -> &'static [(&'static str, &'static str)] {
    match kind {
        "Turning tool" => &[
            ("hand", "Right"),
            ("nose_radius", "0.4"),
            ("lead_angle", "93"),
        ],
        "Boring bar" => &[("nose_radius", "0.4"), ("min_bore", "12"), ("reach", "40")],
        "Grooving tool" => &[("width", "3"), ("reach", "15")],
        "Parting blade" => &[("width", "3"), ("reach", "25")],
        "Threading tool" => &[("thread_angle", "60"), ("pitches", "0.5-3")],
        "Die" => &[("thread", "M6×1"), ("od", "20")],
        _ => &[],
    }
}

/// The manual lathe: `kind` is `Lathe`.
pub fn is_lathe(m: &Machine) -> bool {
    get(&m.p, "kind") == "Lathe"
}

/// A manual lathe profile with the usual small bench lathe's dials, speeds,
/// feeds and threads. The cross-slide dial is not known until the dial check.
pub fn defaults() -> Machine {
    let mut m = Machine::new(3);
    for (k, v) in [
        ("name", "Manual Lathe"),
        ("kind", "Lathe"),
        ("speeds", "70,110,180,280,450,710,1120,1800"),
        ("swing", "250"),
        ("centres", "550"),
        ("cross_reads", ""),
        ("cross_turn", "4"),
        ("cross_div", "100"),
        ("top_turn", "2"),
        ("top_div", "100"),
        ("carriage_turn", "20"),
        ("carriage_div", "100"),
        ("quill_turn", "2.5"),
        ("quill_div", "50"),
        ("feeds", "0.05,0.1,0.15,0.2"),
        ("pitches", "0.5,0.75,1,1.25,1.5,1.75,2,2.5,3"),
        ("pass_time", "20"),
        ("tool_time", "3"),
        ("move_time", "10"),
    ] {
        m.p.insert(k.into(), v.into());
    }
    manual::settle(&mut m);
    m
}

/// How far a thread's axis may be from the spindle axis, mm, to be the one
/// a lathe cuts.
const ON_AXIS: f32 = 0.2;

/// The fields of a Thread turn that cut the thread the part has on the
/// lathe axis of `setup`: `thread_side`, `diameter` (the size's major),
/// `pitch` and `z_end` (lathe work Z of the thread's end toward the chuck),
/// as text for the operation's fields. Of several, the one nearest the free
/// end (the longer when they start together). None when the part has no
/// straight thread on the axis, or none that ends below the free end.
pub fn thread_from_part(setup: &Setup, mesh: &Mesh) -> Option<Vec<(&'static str, String)>> {
    let stock = crate::stock::resolve(setup, Some(mesh)).ok()?.stock;
    let found = crate::thread::in_setup(setup, mesh)
        .into_iter()
        .filter(|t| t.axis == 2 && !t.tapered && t.centre[0].hypot(t.centre[1]) <= ON_AXIS)
        .filter(|t| crate::turn::work_z(stock, t.from) < -1e-3)
        .max_by(|a, b| {
            a.to.total_cmp(&b.to)
                .then(a.length().total_cmp(&b.length()))
        })?;
    // A size on the table is exact; one off it is as measured.
    let (diameter, pitch) = found
        .spec
        .map_or((found.major, found.pitch), |s| (s.major, s.pitch));
    let text = |v: f32| ((v * 1000.).round() / 1000.).to_string();
    Some(vec![
        (
            "thread_side",
            if found.internal { "Inside" } else { "Outside" }.into(),
        ),
        ("diameter", text(diameter)),
        ("pitch", text(pitch)),
        ("z_end", text(crate::turn::work_z(stock, found.from))),
    ])
}

/// The carriage's power feeds, mm per revolution, lowest first. Empty when
/// any entry is not a positive number.
pub fn feeds(m: &Machine) -> Vec<f32> {
    manual::list(m, "feeds")
}

/// The threads the gearbox cuts, mm, lowest first. Empty when any entry is
/// not a positive number.
pub fn pitches(m: &Machine) -> Vec<f32> {
    manual::list(m, "pitches")
}

/// Whether the cross-slide dial reads the diameter or the radius, when known.
pub fn cross_reads(m: &Machine) -> Option<&'static str> {
    match get(&m.p, "cross_reads") {
        "Diameter" => Some("Diameter"),
        "Radius" => Some("Radius"),
        _ => None,
    }
}

/// What the `slide`'s dial shows for `distance`, as (whole turns, reading).
///
/// `distance` is in the user's terms: for the cross slide the amount to take
/// off the diameter (or to feed in on it), for the others the travel in mm.
/// The cross slide converts with `cross_reads`: a reading of `d` takes `d` off
/// the diameter with `Diameter`, `2d` with `Radius`; the guide never guesses,
/// so an unknown `cross_reads` is an error. The reading is in the dial's own
/// numbers (one full turn takes it from 0 to the slide's `*_turn`), rounded to
/// the dial's division, `*_turn / *_div`.
pub fn dial(m: &Machine, slide: Slide, distance: f32) -> Result<(u32, f32), String> {
    if !distance.is_finite() || distance < 0. {
        return Err("A dial distance cannot be negative.".into());
    }
    let (turn_key, div_key) = slide.keys();
    let turn = num(&m.p, turn_key).filter(|v| *v > 0.);
    let div = num(&m.p, div_key).filter(|v| *v >= 1. && v.fract() == 0.);
    let (Some(turn), Some(div)) = (turn, div) else {
        return Err("Set the dial's numbers per turn and divisions on the machine.".into());
    };
    let amount = match slide {
        Slide::Cross => match cross_reads(m) {
            Some("Diameter") => distance as f64,
            Some(_) => distance as f64 / 2.,
            None => return Err(CHECK_THE_DIAL.into()),
        },
        _ => distance as f64,
    };
    let division = turn as f64 / div as f64;
    let divisions = (amount / division).round() as u64;
    let (turns, rest) = (divisions / div as u64, divisions % div as u64);
    Ok((turns as u32, (rest as f64 * division) as f32))
}

/// What the `slide`'s collar reads at its index once the handle is wound
/// `reading` past its whole turns (the reading `dial` gives, in the dial's
/// numbers): the same number winding in; winding `out`, the collar counts
/// down from its zero, so it reads `(turn − reading) mod turn`. None when
/// the machine has no usable dial for it.
pub fn reads(m: &Machine, slide: Slide, reading: f32, out: bool) -> Option<f32> {
    if !out {
        return Some(reading);
    }
    let (turn_key, div_key) = slide.keys();
    let turn = num(&m.p, turn_key).filter(|v| *v > 0.)? as f64;
    let div = num(&m.p, div_key).filter(|v| *v >= 1. && v.fract() == 0.)? as u64;
    let division = turn / div as f64;
    let k = (reading as f64 / division).round() as u64 % div;
    Some((((div - k) % div) as f64 * division) as f32)
}
/// `distance` rounded down to the `slide`'s division (in the user's terms,
/// as `dial` takes it): the farthest a dial can be set without going past
/// it. `distance` itself when the dial is not usable.
pub fn floor(m: &Machine, slide: Slide, distance: f32) -> f32 {
    let (turn_key, div_key) = slide.keys();
    let turn = num(&m.p, turn_key).filter(|v| *v > 0.);
    let div = num(&m.p, div_key).filter(|v| *v >= 1. && v.fract() == 0.);
    let per = match (slide, cross_reads(m)) {
        (Slide::Cross, Some("Radius")) => 2.,
        _ => 1.,
    };
    match (turn, div) {
        (Some(turn), Some(div)) => {
            let step = turn as f64 / div as f64 * per;
            // A ten-thousandth of a division over, f32 noise, still counts.
            ((distance as f64 / step + 1e-4).floor() * step) as f32
        }
        _ => distance,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(m: &mut Machine, k: &str, v: &str) {
        m.p.insert(k.into(), v.into());
    }
    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn each_turn_kind_has_a_tool_type_and_defaults_that_are_its_own() {
        assert_eq!(TURN_KINDS.len(), 7);
        for kind in TURN_KINDS {
            let tool = tool_for(kind).expect(kind);
            assert!(is_tool(tool), "{kind}");
            let starts = turn_defaults(kind);
            // The spindle speed is one of the default lathe's gears.
            let rpm = starts.iter().find(|(k, _)| *k == "rpm").expect(kind).1;
            assert!(manual::on_gear(&defaults(), rpm.parse().unwrap()), "{kind}");
            for (k, _) in starts {
                assert!(
                    *k == "rpm" || *k == "allowance" || TURN_KEYS.contains(k),
                    "{kind} {k}"
                );
            }
        }
        assert_eq!(tool_for("Part off"), Some("Parting blade"));
        assert_eq!(tool_for("Knurl"), None);
        assert!(turn_defaults("Knurl").is_empty());
        // Every key of a kind is in the list the kind change clears.
        let all: std::collections::BTreeSet<_> = TURN_KINDS
            .iter()
            .flat_map(|k| turn_defaults(k))
            .map(|(k, _)| *k)
            .filter(|k| !matches!(*k, "rpm" | "allowance"))
            .collect();
        assert_eq!(all, TURN_KEYS.into_iter().collect());
    }
    #[test]
    fn a_die_is_a_lathe_tool_that_cuts_a_thread_turn_only() {
        assert!(is_tool("Die") && TOOLS.contains(&"Die"));
        assert_eq!(tool_defaults("Die"), [("thread", "M6×1"), ("od", "20")]);
        assert!(takes("Thread", "Threading tool") && takes("Thread", "Die"));
        assert!(!takes("Outside", "Die") && !takes("Thread", "Boring bar"));
        assert!(!takes("Knurl", "Die") && takes("Outside", "Turning tool"));
        let die = Tool::lathe("Die", "Die");
        assert!(die.is_lathe());
        assert_eq!(die_spec(&die).map(|s| s.name), Some("M6×1"));
        assert!(die_spec(&Tool::lathe("T", "Turning tool")).is_none());
        let mut off = die;
        off.p.insert("thread".into(), "M7×0.9".into());
        assert!(die_spec(&off).is_none());
    }
    #[test]
    fn only_the_mill_is_manual_only_the_lathe_is_a_lathe_and_both_are_hand() {
        let (mill, lathe, cnc) = (manual::defaults(), defaults(), Machine::new(3));
        assert!(manual::is_manual(&mill) && !manual::is_manual(&lathe) && !manual::is_manual(&cnc));
        assert!(!is_lathe(&mill) && is_lathe(&lathe) && !is_lathe(&cnc));
        assert!(manual::is_hand(&mill) && manual::is_hand(&lathe) && !manual::is_hand(&cnc));
    }

    #[test]
    fn the_defaults_are_valid_and_settle_the_rpm_on_the_top_gear() {
        let m = defaults();
        assert_eq!(m.issues(), Vec::<String>::new());
        assert_eq!(get(&m.p, "rpm"), "1800");
        assert!(cross_reads(&m).is_none());
        assert_eq!(feeds(&m), [0.05, 0.1, 0.15, 0.2]);
        assert_eq!(pitches(&m).len(), 9);
        assert_eq!(manual::gears(&m).len(), 8);
        // Its travel is its size.
        assert_eq!(
            [get(&m.p, "x"), get(&m.p, "y"), get(&m.p, "z")],
            ["250", "250", "550"]
        );
    }

    #[test]
    fn the_issues_of_a_lathe() {
        let bad = |k: &str, v: &str| {
            let mut m = defaults();
            with(&mut m, k, v);
            m.issues()
        };
        for (k, v) in [
            ("speeds", "fast"),
            ("speeds", ""),
            ("swing", "0"),
            ("centres", "-5"),
            ("cross_reads", "Both"),
            ("cross_turn", "0"),
            ("top_turn", ""),
            ("carriage_turn", "-1"),
            ("quill_turn", "x"),
            ("cross_div", "0"),
            ("top_div", "99.5"),
            ("carriage_div", "-100"),
            ("quill_div", ""),
            ("feeds", "0.1,x"),
            ("feeds", ""),
            ("pitches", "1,,2"),
            ("pass_time", "-1"),
        ] {
            assert!(!bad(k, v).is_empty(), "{k}={v}");
        }
        for (k, v) in [
            ("cross_reads", "Diameter"),
            ("cross_reads", "Radius"),
            ("cross_reads", ""),
            ("speeds", "100-2000"),
            ("quill_turn", "2.5"),
        ] {
            assert_eq!(bad(k, v), Vec::<String>::new(), "{k}={v}");
        }
    }

    #[test]
    fn six_off_the_diameter_is_a_turn_and_two_on_a_diameter_dial_and_three_on_a_radius_dial() {
        let mut m = defaults();
        with(&mut m, "cross_reads", "Diameter");
        let (turns, reading) = dial(&m, Slide::Cross, 6.).unwrap();
        assert_eq!(turns, 1);
        assert!(near(reading, 2.), "{reading}");
        with(&mut m, "cross_reads", "Radius");
        let (turns, reading) = dial(&m, Slide::Cross, 6.).unwrap();
        assert_eq!(turns, 0);
        assert!(near(reading, 3.), "{reading}");
    }

    #[test]
    fn an_unknown_cross_slide_dial_gives_no_number_but_the_other_slides_do() {
        let m = defaults();
        assert_eq!(dial(&m, Slide::Cross, 6.), Err(CHECK_THE_DIAL.into()));
        assert!(dial(&m, Slide::Carriage, 6.).is_ok());
        assert!(dial(&m, Slide::Top, 1.).is_ok());
        assert!(dial(&m, Slide::Quill, 1.).is_ok());
    }

    #[test]
    fn the_carriage_quill_and_top_slide_convert_with_their_own_dials() {
        let m = defaults();
        let (turns, reading) = dial(&m, Slide::Carriage, 65.).unwrap();
        assert_eq!(turns, 3);
        assert!(near(reading, 5.), "{reading}");
        // The quill: 2.5 per turn in 50 divisions of 0.05.
        let (turns, reading) = dial(&m, Slide::Quill, 30.).unwrap();
        assert_eq!(turns, 12);
        assert!(near(reading, 0.), "{reading}");
        let (turns, reading) = dial(&m, Slide::Quill, 31.3).unwrap();
        assert_eq!(turns, 12);
        assert!(near(reading, 1.3), "{reading}");
        // The top slide: 2 per turn in 100 divisions of 0.02.
        let (turns, reading) = dial(&m, Slide::Top, 0.5).unwrap();
        assert_eq!(turns, 0);
        assert!(near(reading, 0.5), "{reading}");
        // Nothing is a dial at its zero.
        assert_eq!(dial(&m, Slide::Carriage, 0.).unwrap(), (0, 0.));
    }

    #[test]
    fn a_reading_is_rounded_to_the_dials_division_and_carries_into_the_turns() {
        let mut m = defaults();
        // Carriage division 0.2: 65.05 is the 325th, 65.11 the 326th.
        for (distance, turns, reading) in [(65.05, 3, 5.), (65.11, 3, 5.2)] {
            let (t, r) = dial(&m, Slide::Carriage, distance).unwrap();
            assert_eq!(t, turns, "{distance}");
            assert!(near(r, reading), "{distance}: {r}");
        }
        // 19.95 rounds up to a whole turn, not to a reading of 20.
        let (t, r) = dial(&m, Slide::Carriage, 19.95).unwrap();
        assert_eq!((t, r), (1, 0.));
        // A coarser dial: 4 per turn in 40 divisions of 0.1.
        with(&mut m, "carriage_turn", "4");
        with(&mut m, "carriage_div", "40");
        let (t, r) = dial(&m, Slide::Carriage, 9.37).unwrap();
        assert_eq!(t, 2);
        assert!(near(r, 1.4), "{r}");
    }

    #[test]
    fn winding_out_the_collar_counts_down_from_its_zero() {
        let mut m = defaults();
        with(&mut m, "cross_reads", "Diameter");
        // 9.80 out on a 4-a-turn dial: 2 turns and 1.80 more, so the collar
        // reads 4 − 1.80 = 2.20; in, it reads 1.80.
        let (turns, reading) = dial(&m, Slide::Cross, 9.8).unwrap();
        assert_eq!(turns, 2);
        assert!(near(reading, 1.8), "{reading}");
        assert!(near(reads(&m, Slide::Cross, reading, true).unwrap(), 2.2));
        assert!(near(reads(&m, Slide::Cross, reading, false).unwrap(), 1.8));
        // Whole turns out read 0, not 4.
        let (turns, reading) = dial(&m, Slide::Cross, 8.).unwrap();
        assert_eq!(
            (turns, reads(&m, Slide::Cross, reading, true)),
            (2, Some(0.))
        );
        // The top slide: 0.64 out of 2 a turn reads 1.36.
        assert!(near(reads(&m, Slide::Top, 0.64, true).unwrap(), 1.36));
        with(&mut m, "top_div", "0");
        assert_eq!(reads(&m, Slide::Top, 0.64, true), None);
    }
    #[test]
    fn floor_never_goes_past_the_distance() {
        let mut m = defaults();
        // The top slide's 0.02s: 1.0513 is set at 1.04, never 1.06.
        assert!(near(floor(&m, Slide::Top, 1.0513), 1.04));
        assert!(near(floor(&m, Slide::Top, 1.04), 1.04));
        // A radius cross dial's division is 0.08 off the diameter.
        with(&mut m, "cross_reads", "Radius");
        assert!(near(floor(&m, Slide::Cross, 1.0), 0.96));
    }
    #[test]
    fn a_negative_or_unusable_distance_or_dial_is_refused() {
        let mut m = defaults();
        with(&mut m, "cross_reads", "Diameter");
        for slide in [Slide::Cross, Slide::Top, Slide::Carriage, Slide::Quill] {
            assert!(dial(&m, slide, -0.5).is_err());
            assert!(dial(&m, slide, f32::NAN).is_err());
            assert!(dial(&m, slide, f32::INFINITY).is_err());
        }
        with(&mut m, "carriage_turn", "0");
        assert!(dial(&m, Slide::Carriage, 5.).is_err());
        with(&mut m, "top_div", "0");
        assert!(dial(&m, Slide::Top, 5.).is_err());
    }

    /// The part's thread as a Thread turn takes it, by key.
    fn fields_of(doc: &crate::model::Document, mesh: &Mesh) -> Option<Vec<(&'static str, String)>> {
        thread_from_part(&doc.setups[0], mesh)
    }
    /// A mesh of `a` then `b`, each moved up `dz` (mm).
    fn lifted(parts: &[(Vec<[[f32; 3]; 3]>, f32)]) -> Vec<[[f32; 3]; 3]> {
        parts
            .iter()
            .flat_map(|(t, dz)| t.iter().map(move |t| t.map(|p| [p[0], p[1], p[2] + dz])))
            .collect()
    }
    fn value(fields: &[(&'static str, String)], key: &str) -> f32 {
        fields
            .iter()
            .find(|(k, _)| *k == key)
            .unwrap_or_else(|| panic!("{key} in {fields:?}"))
            .1
            .parse()
            .unwrap()
    }
    #[test]
    fn the_thread_on_the_lathe_axis_fills_the_thread_turn() {
        use crate::{
            thread::tests::modeled,
            turn::tests::{job, tube},
        };
        // A Ø30 body from Z -60 to -30 with an M16 × 2 stud on it to -10.
        let part = lifted(&[
            (tube(0., 15., 0., 30.), 0.),
            (modeled([0., 0.], 16., 2., 60., true, false, 20., 0.), 30.),
        ]);
        let (doc, mesh) = job(50., 100., part, -60.);
        let fields = fields_of(&doc, &mesh).expect("the stud is a thread on the axis");
        let keys: Vec<&str> = fields.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, ["thread_side", "diameter", "pitch", "z_end"]);
        assert_eq!(fields[0].1, "Outside");
        // The table's size, not the measured one; the end toward the chuck
        // is at the body's shoulder, Z -30 from the free end.
        assert_eq!(fields[1].1, "16");
        assert_eq!(fields[2].1, "2");
        assert!((value(&fields, "z_end") + 30.).abs() < 0.6, "{fields:?}");
        // They are what a Thread turn takes: set on one, it has no issues.
        let mut ready = doc.clone();
        crate::turn::tests::add(&mut ready, "Thread", &[]);
        let mut op = ready.setups[0].operations.pop().unwrap();
        for (k, v) in &fields {
            op.p.insert((*k).into(), v.clone());
        }
        assert_eq!(
            op.issues(&ready.setups[0].machine, &ready.tools, &[]),
            Vec::<String>::new()
        );
    }
    #[test]
    fn a_threaded_hole_on_the_axis_reads_inside_with_its_size_and_end() {
        use crate::{thread::tests::modeled, turn::tests::job};
        // M8 × 1.25 from Z -40 to -20.
        let part = modeled([0., 0.], 8., 1.25, 60., true, true, 20., 0.);
        let (doc, mesh) = job(30., 100., part, -40.);
        let fields = fields_of(&doc, &mesh).expect("the hole is a thread on the axis");
        assert_eq!(fields[0].1, "Inside");
        assert_eq!((fields[1].1.as_str(), fields[2].1.as_str()), ("8", "1.25"));
        assert!((value(&fields, "z_end") + 40.).abs() < 0.6, "{fields:?}");
    }
    #[test]
    fn a_part_with_no_thread_on_the_axis_gives_nothing() {
        use crate::{
            thread::tests::modeled,
            turn::tests::{job, tube},
        };
        // A plain shaft.
        let (doc, mesh) = job(50., 100., tube(0., 15., 0., 50.), -60.);
        assert_eq!(fields_of(&doc, &mesh), None);
        // Two threads either side of the axis, 20 mm out: not on it.
        let part = lifted(&[
            (modeled([-20., 0.], 8., 1.25, 60., true, true, 20., 0.), 0.),
            (modeled([20., 0.], 8., 1.25, 60., true, true, 20., 0.), 0.),
        ]);
        let (doc, mesh) = job(70., 100., part, -40.);
        assert_eq!(
            crate::thread::in_setup(&doc.setups[0], &mesh)
                .iter()
                .filter(|t| t.axis == 2)
                .count(),
            2
        );
        assert_eq!(fields_of(&doc, &mesh), None);
    }
}
