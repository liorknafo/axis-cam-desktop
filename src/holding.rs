//! Workholding: vises and clamps from the library, where they sit in a
//! setup, and whether a toolpath comes too close to them. Toolpaths are
//! never changed; moves that hit are reported.
use crate::material::NAME_LIMIT;
use crate::mesh::Bounds;
use crate::model::{Fields, Setup, fields, get, id, num};
use crate::simulation::Plan;
use serde::{Deserialize, Serialize};

pub const KINDS: [&str; 2] = ["Vise", "Clamp"];
const NOTE: &str = "Measure yours: these are typical sizes.";
/// How far below the stock bottom a vise jaw is modelled to reach.
const JAW_DEPTH: f32 = 100.;
pub const MAX_CLAMPS: usize = 8;
/// What a setup's issues say when the library entry it points at is gone.
pub const DELETED: &str = "The workholding entry was deleted; choose another.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Holder {
    pub id: String,
    pub p: Fields,
}
impl Holder {
    pub fn new(kind: &str, name: &str) -> Self {
        Self {
            id: id(),
            p: fields(&[("kind", kind), ("name", name)]),
        }
    }
    pub fn name(&self) -> &str {
        get(&self.p, "name")
    }
    pub fn kind(&self) -> &str {
        get(&self.p, "kind")
    }
    fn keys(&self) -> &'static [&'static str] {
        if self.kind() == "Vise" {
            &[
                "jaw_width",
                "jaw_height",
                "jaw_thickness",
                "max_opening",
                "grip",
            ]
        } else {
            &["length", "width", "height", "bite"]
        }
    }
    pub fn issues(&self) -> Vec<String> {
        let mut e = vec![];
        if self.name().trim().is_empty() {
            e.push("Give the workholding a name.".into());
        } else if self.name().trim().chars().count() > NAME_LIMIT {
            e.push(format!("The name can be at most {NAME_LIMIT} characters."));
        }
        if get(&self.p, "notes").chars().count() > 500 {
            e.push("Notes can be at most 500 characters.".into());
        }
        if !KINDS.contains(&self.kind()) {
            e.push("Choose Vise or Clamp.".into());
        }
        for k in self.keys() {
            if !num(&self.p, k).is_some_and(|v| v > 0.) {
                e.push(format!(
                    "{} must be a positive number.",
                    k.replace('_', " ")
                ));
            }
        }
        if self.kind() == "Clamp"
            && num(&self.p, "bite")
                .zip(num(&self.p, "length"))
                .is_some_and(|(b, l)| b >= l)
        {
            e.push("The clamp's bite must be shorter than its length.".into());
        }
        e
    }
}
pub fn starters() -> Vec<Holder> {
    let mut v = Holder::new("Vise", "100 mm machine vise");
    for (k, x) in [
        ("jaw_width", "100"),
        ("jaw_height", "30"),
        ("jaw_thickness", "20"),
        ("max_opening", "120"),
        ("grip", "8"),
    ] {
        v.p.insert(k.into(), x.into());
    }
    let mut c = Holder::new("Clamp", "Toe clamp 60 mm");
    for (k, x) in [
        ("length", "60"),
        ("width", "20"),
        ("height", "25"),
        ("bite", "12"),
    ] {
        c.p.insert(k.into(), x.into());
    }
    for h in [&mut v, &mut c] {
        h.p.insert("notes".into(), NOTE.into());
    }
    vec![v, c]
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    MinusX,
    PlusX,
    MinusY,
    PlusY,
}
impl Side {
    pub const ALL: [Side; 4] = [Side::MinusX, Side::PlusX, Side::MinusY, Side::PlusY];
    pub fn label(self) -> &'static str {
        match self {
            Side::MinusX => "-X",
            Side::PlusX => "+X",
            Side::MinusY => "-Y",
            Side::PlusY => "+Y",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        Side::ALL.into_iter().find(|side| side.label() == s.trim())
    }
}
/// A setup's modelled workholding, read from its parameters.
#[derive(Debug, Clone, PartialEq)]
pub enum Layout {
    None,
    Vise {
        holder: String,
        axis_y: bool,
        offset: f32,
        grip: f32,
        gap: f32,
    },
    Clamps {
        holder: String,
        clamps: Vec<(Side, f32)>,
        gap: f32,
    },
}
/// Encodes clamp positions as stored on the setup ("-X:0;+X:10").
pub fn clamps_text(clamps: &[(Side, f32)]) -> String {
    clamps
        .iter()
        .map(|(s, o)| format!("{}:{}", s.label(), o))
        .collect::<Vec<_>>()
        .join(";")
}
/// A number typed into a field: surrounding spaces are ignored.
fn number(text: &str) -> Option<f32> {
    text.trim().parse::<f32>().ok().filter(|v| v.is_finite())
}
pub fn layout(setup: &Setup) -> Result<Layout, String> {
    let p = &setup.p;
    let gap = || match get(p, "holding_gap").trim() {
        "" => Ok(2.),
        text => number(text)
            .filter(|g| *g >= 0.)
            .ok_or("The clearance gap must be 0 or more."),
    };
    let holder = get(p, "holder").to_string();
    match get(p, "holding") {
        "" | "None" => Ok(Layout::None),
        "Vise" => {
            let grip = number(get(p, "vise_grip"))
                .filter(|g| *g > 0.)
                .ok_or("The vise grip must be a positive number.")?;
            let offset = match get(p, "vise_offset").trim() {
                "" => 0.,
                text => number(text).ok_or("The vise offset must be a number.")?,
            };
            let axis_y = match get(p, "vise_axis").trim() {
                "" | "X" => false,
                "Y" => true,
                _ => return Err("The vise axis must be X or Y.".into()),
            };
            Ok(Layout::Vise {
                holder,
                axis_y,
                offset,
                grip,
                gap: gap()?,
            })
        }
        "Clamps" => {
            let mut clamps = vec![];
            for part in get(p, "clamps").split(';').filter(|s| !s.trim().is_empty()) {
                let (side, offset) = part
                    .split_once(':')
                    .ok_or("Write each clamp as side:offset, for example -X:0.")?;
                let side = Side::parse(side).ok_or("A clamp side must be -X, +X, -Y or +Y.")?;
                let offset = number(offset).ok_or("A clamp offset must be a number.")?;
                clamps.push((side, offset));
            }
            if clamps.is_empty() || clamps.len() > MAX_CLAMPS {
                return Err(format!("Place 1 to {MAX_CLAMPS} clamps."));
            }
            Ok(Layout::Clamps {
                holder,
                clamps,
                gap: gap()?,
            })
        }
        other => Err(format!("\"{other}\" is not a workholding choice.")),
    }
}
/// Whether `text` reads as clamp positions: `layout` of a copy of `setup`
/// holding clamps at `text`. The setup's other keys, a bad gap for one, are
/// not this text's problem.
pub fn check_clamps(setup: &Setup, text: &str) -> Result<(), String> {
    let copy = Setup {
        preview_stock: None,
        preview_remaining: None,
        id: setup.id.clone(),
        machine: setup.machine.clone(),
        p: fields(&[("holding", "Clamps"), ("clamps", text)]),
        operations: vec![],
    };
    layout(&copy).map(|_| ())
}
#[derive(Debug, Clone, PartialEq)]
pub struct HoldBox {
    pub label: String,
    pub bounds: Bounds,
}
/// Why `holder` cannot serve `layout`: it is of the other kind or its
/// dimensions are not usable. None for no workholding.
pub fn refusal(layout: &Layout, holder: &Holder) -> Option<String> {
    let want = match layout {
        Layout::None => return None,
        Layout::Vise { .. } => "Vise",
        Layout::Clamps { .. } => "Clamp",
    };
    if holder.kind() != want && KINDS.contains(&holder.kind()) {
        return Some(format!(
            "The chosen workholding entry is a {}, not a {}.",
            holder.kind().to_lowercase(),
            want.to_lowercase()
        ));
    }
    let issues = holder.issues();
    (!issues.is_empty()).then(|| {
        format!(
            "The chosen workholding entry needs fixing: {}",
            issues.join(" ")
        )
    })
}
/// `v` mm for a message: up to two decimals, without trailing zeros.
fn mm(v: f32) -> String {
    format!("{v:.2}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}
/// The vise's maximum opening is less than the stock is wide across the jaws.
/// `stock` is the setup's starting stock box in millimetres.
pub fn opening_issue(layout: &Layout, holder: &Holder, stock: Bounds) -> Option<String> {
    let Layout::Vise { axis_y, .. } = layout else {
        return None;
    };
    let across = stock.dimensions()[usize::from(*axis_y)];
    let opening = num(&holder.p, "max_opening").filter(|v| *v > 0.)?;
    (across > opening + 0.005).then(|| {
        format!(
            "The stock is {} mm across the jaws; this vise opens to {} mm.",
            mm(across),
            mm(opening)
        )
    })
}
/// The jaws or clamps of `layout`, in the frame of `stock` (the setup's
/// starting stock box). Without the clearance gap. Refuses a holder of the
/// other kind or one whose dimensions are not usable (`refusal`).
pub fn boxes(layout: &Layout, holder: &Holder, stock: Bounds) -> Result<Vec<HoldBox>, String> {
    if let Some(refused) = refusal(layout, holder) {
        return Err(refused);
    }
    let n = |k| num(&holder.p, k).unwrap_or(0.);
    let centre = [
        (stock.min[0] + stock.max[0]) / 2.,
        (stock.min[1] + stock.max[1]) / 2.,
    ];
    Ok(match layout {
        Layout::None => vec![],
        Layout::Vise {
            axis_y,
            offset,
            grip,
            ..
        } => {
            let (w, t) = (n("jaw_width"), n("jaw_thickness"));
            let (z0, z1) = (stock.min[2] - JAW_DEPTH, stock.min[2] + grip);
            let (across, along) = if *axis_y { (1, 0) } else { (0, 1) };
            [(false, "-"), (true, "+")]
                .into_iter()
                .map(|(plus, sign)| {
                    let mut min = [0.; 3];
                    let mut max = [0.; 3];
                    if plus {
                        (min[across], max[across]) = (stock.max[across], stock.max[across] + t);
                    } else {
                        (min[across], max[across]) = (stock.min[across] - t, stock.min[across]);
                    }
                    let c = centre[along] + offset;
                    (min[along], max[along]) = (c - w / 2., c + w / 2.);
                    (min[2], max[2]) = (z0, z1);
                    HoldBox {
                        label: format!("vise jaw on {sign}{}", if *axis_y { "Y" } else { "X" }),
                        bounds: Bounds { min, max },
                    }
                })
                .collect()
        }
        Layout::Clamps { clamps, .. } => {
            let (l, w, h, bite) = (n("length"), n("width"), n("height"), n("bite"));
            clamps
                .iter()
                .map(|&(side, offset)| {
                    let (across, along, plus) = match side {
                        Side::MinusX => (0, 1, false),
                        Side::PlusX => (0, 1, true),
                        Side::MinusY => (1, 0, false),
                        Side::PlusY => (1, 0, true),
                    };
                    let mut min = [0.; 3];
                    let mut max = [0.; 3];
                    if plus {
                        (min[across], max[across]) =
                            (stock.max[across] - bite, stock.max[across] + l - bite);
                    } else {
                        (min[across], max[across]) =
                            (stock.min[across] - (l - bite), stock.min[across] + bite);
                    }
                    let c = centre[along] + offset;
                    (min[along], max[along]) = (c - w / 2., c + w / 2.);
                    (min[2], max[2]) = (stock.max[2], stock.max[2] + h);
                    HoldBox {
                        label: format!("clamp on {} at {offset}", side.label()),
                        bounds: Bounds { min, max },
                    }
                })
                .collect()
        }
    })
}
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub label: String,
    /// "tool" (cutter and shank, tip to stickout) or "holder" (above the stickout).
    pub part: &'static str,
    /// The tip where the move, clipped to the heights at which the part can
    /// touch the box, first touches the box's footprint (the rectangle itself,
    /// not grown by the reach); when it never does, its closest point to it.
    pub at: [f32; 3],
}
/// The parameters in 0..=1 at which `a + (b - a) t` lies within `lo..=hi`.
fn within(a: f64, b: f64, lo: f64, hi: f64) -> Option<(f64, f64)> {
    let d = b - a;
    let (t0, t1) = if d == 0. {
        if !(lo..=hi).contains(&a) {
            return None;
        }
        (0., 1.)
    } else {
        let (x, y) = ((lo - a) / d, (hi - a) / d);
        (x.min(y).max(0.), x.max(y).min(1.))
    };
    (t0 <= t1).then_some((t0, t1))
}
/// The least XY distance between the segment `p`-`q` and the rectangle of
/// `b`, and where along the segment (0..=1) it is first reached. The
/// distance is 0 where the segment touches the rectangle, otherwise it is
/// found between an end of the segment and the rectangle or between a
/// corner of the rectangle and the segment.
fn xy_closest(b: &Bounds, p: [f64; 2], q: [f64; 2]) -> (f64, f64) {
    let (lo, hi) = (b.min.map(f64::from), b.max.map(f64::from));
    let crossing = within(p[0], q[0], lo[0], hi[0])
        .zip(within(p[1], q[1], lo[1], hi[1]))
        .map(|(x, y)| (x.0.max(y.0), x.1.min(y.1)))
        .filter(|(from, to)| from <= to);
    if let Some((from, _)) = crossing {
        return (0., from);
    }
    let outside = |c: [f64; 2]| {
        let dx = (lo[0] - c[0]).max(0.).max(c[0] - hi[0]);
        let dy = (lo[1] - c[1]).max(0.).max(c[1] - hi[1]);
        dx.hypot(dy)
    };
    let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
    let length2 = dx * dx + dy * dy;
    let corners = [
        [lo[0], lo[1]],
        [lo[0], hi[1]],
        [hi[0], lo[1]],
        [hi[0], hi[1]],
    ]
    .map(|c| {
        let s = match length2 {
            0. => 0.,
            _ => (((c[0] - p[0]) * dx + (c[1] - p[1]) * dy) / length2).clamp(0., 1.),
        };
        ((c[0] - (p[0] + dx * s)).hypot(c[1] - (p[1] + dy * s)), s)
    });
    [(outside(p), 0.), (outside(q), 1.)]
        .into_iter()
        .chain(corners)
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .unwrap_or((f64::INFINITY, 0.))
}
/// Straight `segments` (tip positions, start to end) whose tool or holder
/// comes within `gap` of a box. The tool is a vertical cylinder of the
/// cutter `radius` from the tip up by `stickout`, the holder one of
/// `holder_radius` from there upward. Exact: for each segment the stretch
/// where the part's height overlaps the box (grown by `gap`) is found, and
/// the XY distance from that stretch to the box's footprint is compared with
/// the radius plus `gap`. One hit per box and part, the first segment's.
pub fn hits_segments(
    segments: &[([f32; 3], [f32; 3])],
    radius: f32,
    stickout: f32,
    holder_radius: f32,
    boxes: &[HoldBox],
    gap: f32,
) -> Vec<Hit> {
    let parts = [
        ("tool", radius, 0., stickout),
        ("holder", holder_radius, stickout, f32::INFINITY),
    ];
    let mut out = vec![];
    for b in boxes {
        for (part, part_radius, low, high) in parts {
            let reach = f64::from(part_radius) + f64::from(gap);
            let z = (
                f64::from(b.bounds.min[2] - gap - high),
                f64::from(b.bounds.max[2] + gap - low),
            );
            let found = segments.iter().find_map(|&(from, to)| {
                let (from, to) = (from.map(f64::from), to.map(f64::from));
                let tip = |t: f64| [0, 1, 2].map(|k| from[k] + (to[k] - from[k]) * t);
                let (t0, t1) = within(from[2], to[2], z.0, z.1)?;
                let (p, q) = (tip(t0), tip(t1));
                let (distance, s) = xy_closest(&b.bounds, [p[0], p[1]], [q[0], q[1]]);
                (distance <= reach).then(|| tip(t0 + (t1 - t0) * s).map(|v| v as f32))
            });
            if let Some(at) = found {
                out.push(Hit {
                    label: b.label.clone(),
                    part,
                    at,
                });
            }
        }
    }
    out
}
/// `hits_segments` for the moves of `plan`.
pub fn hits(plan: &Plan, boxes: &[HoldBox], gap: f32) -> Vec<Hit> {
    let moves: Vec<_> = plan.moves.iter().map(|m| (m.from, m.to)).collect();
    hits_segments(
        &moves,
        plan.radius,
        plan.tool_length,
        plan.holder_radius,
        boxes,
        gap,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Machine, Setup};
    use crate::simulation::{Motion, Move, Plan};
    fn stock() -> Bounds {
        Bounds {
            min: [-50., -30., 0.],
            max: [50., 30., 40.],
        }
    }
    fn vise() -> Holder {
        starters().into_iter().find(|h| h.kind() == "Vise").unwrap()
    }
    fn clamp() -> Holder {
        starters()
            .into_iter()
            .find(|h| h.kind() == "Clamp")
            .unwrap()
    }
    fn setup(pairs: &[(&str, &str)]) -> Setup {
        let mut s = Setup::new(Machine::new(3), "G54");
        for (k, v) in pairs {
            s.p.insert((*k).into(), (*v).into());
        }
        s
    }
    /// A plan whose tool (Ø`2r`, stickout 30, holder Ø32) makes `moves`.
    fn plan(r: f32, moves: &[([f32; 3], [f32; 3])]) -> Plan {
        let mut p = crate::post::fixture::plan(moves[0].0, &[]);
        p.radius = r;
        p.flute_length = 20.;
        p.tool_length = 30.;
        p.holder_radius = 16.;
        p.moves = moves
            .iter()
            .map(|&(from, to)| Move {
                from,
                to,
                motion: Motion::Feed,
                cutting_pass: true,
                feed: 500.,
                start: 0.,
                end: 0.,
            })
            .collect();
        p
    }
    #[test]
    fn starters_are_valid() {
        let all = starters();
        assert_eq!(all.len(), 2);
        for h in &all {
            assert!(h.issues().is_empty(), "{}: {:?}", h.name(), h.issues());
            assert!(get(&h.p, "notes").contains("Measure yours"));
        }
        let mut bad = clamp();
        bad.p.insert("bite".into(), "80".into());
        assert!(bad.issues().iter().any(|e| e.contains("bite")));
        let mut negative = vise();
        negative.p.insert("jaw_width".into(), "-1".into());
        assert!(!negative.issues().is_empty());
    }
    #[test]
    fn layouts_are_read_from_the_setup() {
        assert_eq!(layout(&setup(&[])).unwrap(), Layout::None);
        assert_eq!(
            layout(&setup(&[
                ("holding", "Vise"),
                ("holder", "v1"),
                ("vise_axis", "Y"),
                ("vise_offset", "5"),
                ("vise_grip", "8"),
                ("holding_gap", "2")
            ]))
            .unwrap(),
            Layout::Vise {
                holder: "v1".into(),
                axis_y: true,
                offset: 5.,
                grip: 8.,
                gap: 2.
            }
        );
        assert_eq!(
            layout(&setup(&[
                ("holding", "Clamps"),
                ("holder", "c1"),
                ("clamps", "-X:0;+X:10")
            ]))
            .unwrap(),
            Layout::Clamps {
                holder: "c1".into(),
                clamps: vec![(Side::MinusX, 0.), (Side::PlusX, 10.)],
                gap: 2.
            }
        );
        assert!(
            layout(&setup(&[
                ("holding", "Clamps"),
                ("holder", "c1"),
                ("clamps", "-Z:0")
            ]))
            .is_err()
        );
        assert!(
            layout(&setup(&[
                ("holding", "Clamps"),
                ("holder", "c1"),
                ("clamps", "")
            ]))
            .is_err()
        );
        let nine = vec!["-X:0"; 9].join(";");
        assert!(
            layout(&setup(&[
                ("holding", "Clamps"),
                ("holder", "c1"),
                ("clamps", &nine)
            ]))
            .is_err()
        );
        assert!(
            layout(&setup(&[
                ("holding", "Vise"),
                ("holder", "v1"),
                ("vise_grip", "-1")
            ]))
            .is_err()
        );
    }
    #[test]
    fn vise_jaws_and_clamps_sit_where_the_spec_says() {
        // Vise on the ±Y sides: jaw 100 wide, 20 thick, up to grip 8 above the stock bottom.
        let jaws = boxes(
            &Layout::Vise {
                holder: String::new(),
                axis_y: true,
                offset: 5.,
                grip: 8.,
                gap: 2.,
            },
            &vise(),
            stock(),
        )
        .unwrap();
        assert_eq!(jaws.len(), 2);
        let front = jaws.iter().find(|b| b.label.contains("-Y")).unwrap();
        assert_eq!(front.bounds.max[1], -30.);
        assert_eq!(front.bounds.min[1], -50.);
        assert_eq!((front.bounds.min[0], front.bounds.max[0]), (-45., 55.));
        assert_eq!(front.bounds.max[2], 8.);
        assert!(front.bounds.min[2] <= -100.);
        // A clamp on -X at offset 10: 20 wide, bite 12 onto the stock, 60 long, 25 high.
        let c = boxes(
            &Layout::Clamps {
                holder: String::new(),
                clamps: vec![(Side::MinusX, 10.)],
                gap: 2.,
            },
            &clamp(),
            stock(),
        )
        .unwrap();
        assert_eq!(c[0].bounds.min, [-98., 0., 40.]);
        assert_eq!(c[0].bounds.max, [-38., 20., 65.]);
        assert!(c[0].label.contains("-X"));
        assert!(boxes(&Layout::None, &vise(), stock()).unwrap().is_empty());
    }
    #[test]
    fn the_check_finds_tool_and_holder_hits_and_respects_the_gap() {
        let c = boxes(
            &Layout::Clamps {
                holder: String::new(),
                clamps: vec![(Side::MinusX, 0.)],
                gap: 2.,
            },
            &clamp(),
            stock(),
        )
        .unwrap();
        // Clamp box: X -98..-38, Y -10..10, Z 40..65.
        // A Ø6 tool feeding at Z 41 across X -60..-30 at Y 0 cuts into the clamp.
        let into = hits(&plan(3., &[([-60., 0., 41.], [-30., 0., 41.])]), &c, 2.);
        assert_eq!(into.len(), 1);
        assert_eq!(into[0].part, "tool");
        assert!(into[0].label.contains("-X"));
        // Passing 4 mm beside it (tool edge 4 mm from the box) is clear with a 2 mm gap...
        assert!(hits(&plan(3., &[([-60., 17., 41.], [-30., 17., 41.])]), &c, 2.).is_empty());
        // ...and a hit with a 5 mm gap.
        assert_eq!(
            hits(&plan(3., &[([-60., 17., 41.], [-30., 17., 41.])]), &c, 5.).len(),
            1
        );
        // A rapid high above the clamp (tip at Z 100) is clear; the holder starts at the stickout.
        assert!(hits(&plan(3., &[([-60., 0., 100.], [-30., 0., 100.])]), &c, 2.).is_empty());
        // The tip at Z 70 is above the clamp (top 65 + gap 2), but a Ø32 holder... starts at Z 100: clear.
        assert!(hits(&plan(3., &[([-60., 0., 70.], [-30., 0., 70.])]), &c, 2.).is_empty());
        // With a short stickout (tip 60 → holder from Z 63, within the clamp top 65 + 2), a Ø32 holder passing at Y 30 (edge 4 from the box) is clear; at Y 20 it hits.
        let mut short = plan(3., &[([-60., 20., 60.], [-30., 20., 60.])]);
        short.tool_length = 3.;
        let h = hits(&short, &c, 2.);
        assert!(h.iter().any(|x| x.part == "holder"), "{h:?}");
        let mut beside = plan(3., &[([-60., 30., 60.], [-30., 30., 60.])]);
        beside.tool_length = 3.;
        assert!(hits(&beside, &c, 2.).is_empty());
        // One hit per box and part, not per sample.
        let long = hits(&plan(3., &[([-90., 0., 41.], [-40., 0., 41.])]), &c, 2.);
        assert_eq!(long.len(), 1);
    }
    fn vise_layout(axis_y: bool, offset: f32, gap: f32) -> Layout {
        Layout::Vise {
            holder: String::new(),
            axis_y,
            offset,
            grip: 8.,
            gap,
        }
    }
    fn clamp_layout(clamps: &[(Side, f32)]) -> Layout {
        Layout::Clamps {
            holder: String::new(),
            clamps: clamps.to_vec(),
            gap: 2.,
        }
    }
    fn near(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-3)
    }
    #[test]
    fn layouts_trim_numbers_check_the_axis_and_ignore_the_gap_without_workholding() {
        assert_eq!(
            layout(&setup(&[
                ("holding", "Vise"),
                ("holder", "v1"),
                ("vise_offset", " 5"),
                ("vise_grip", " 8 "),
                ("holding_gap", " 3 ")
            ]))
            .unwrap(),
            Layout::Vise {
                holder: "v1".into(),
                axis_y: false,
                offset: 5.,
                grip: 8.,
                gap: 3.
            }
        );
        assert_eq!(
            layout(&setup(&[
                ("holding", "Clamps"),
                ("holder", "c1"),
                ("clamps", " -X : 0 ; +Y: 4 "),
                ("holding_gap", " 4")
            ]))
            .unwrap(),
            Layout::Clamps {
                holder: "c1".into(),
                clamps: vec![(Side::MinusX, 0.), (Side::PlusY, 4.)],
                gap: 4.
            }
        );
        let axis = |a: &str| {
            layout(&setup(&[
                ("holding", "Vise"),
                ("vise_grip", "8"),
                ("vise_axis", a),
            ]))
        };
        assert!(matches!(axis(""), Ok(Layout::Vise { axis_y: false, .. })));
        assert!(matches!(axis("X"), Ok(Layout::Vise { axis_y: false, .. })));
        assert!(matches!(axis("Y"), Ok(Layout::Vise { axis_y: true, .. })));
        assert!(axis("Z").is_err());
        assert!(axis("XY").is_err());
        assert_eq!(
            layout(&setup(&[("holding_gap", "abc")])).unwrap(),
            Layout::None
        );
        assert_eq!(
            layout(&setup(&[("holding", "None"), ("holding_gap", "-3")])).unwrap(),
            Layout::None
        );
        assert!(
            layout(&setup(&[
                ("holding", "Vise"),
                ("vise_grip", "8"),
                ("holding_gap", "abc")
            ]))
            .is_err()
        );
        assert!(
            layout(&setup(&[
                ("holding", "Clamps"),
                ("clamps", "-X:0"),
                ("holding_gap", "-1")
            ]))
            .is_err()
        );
    }
    #[test]
    fn boxes_refuse_a_holder_of_the_wrong_kind_or_with_issues() {
        let jaws = vise_layout(true, 0., 2.);
        let clamps = clamp_layout(&[(Side::MinusX, 0.)]);
        assert_eq!(
            boxes(&jaws, &clamp(), stock()).unwrap_err(),
            "The chosen workholding entry is a clamp, not a vise."
        );
        assert_eq!(
            boxes(&clamps, &vise(), stock()).unwrap_err(),
            "The chosen workholding entry is a vise, not a clamp."
        );
        let mut unfinished = vise();
        unfinished.p.remove("jaw_thickness");
        assert!(
            boxes(&jaws, &unfinished, stock())
                .unwrap_err()
                .contains("jaw thickness")
        );
        assert!(boxes(&clamps, &Holder::new("Clamp", "Empty"), stock()).is_err());
        let mut odd = vise();
        odd.p.insert("kind".into(), "Table".into());
        assert!(boxes(&jaws, &odd, stock()).is_err());
        assert!(
            boxes(&Layout::None, &Holder::new("Clamp", "x"), stock())
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn boxes_follow_a_stock_away_from_the_origin() {
        let stock = Bounds {
            min: [900., 200., 100.],
            max: [1100., 400., 140.],
        };
        let of = |layout: &Layout, holder: &Holder| -> Vec<(String, [f32; 3], [f32; 3])> {
            boxes(layout, holder, stock)
                .unwrap()
                .into_iter()
                .map(|b| (b.label, b.bounds.min, b.bounds.max))
                .collect()
        };
        // Vise along X (jaws on -X and +X): centre Y 300 + offset 5, jaw 100 wide and 20 thick, Z 100 - 100 up to 100 + grip 8.
        assert_eq!(
            of(&vise_layout(false, 5., 2.), &vise()),
            [
                (
                    "vise jaw on -X".to_string(),
                    [880., 255., 0.],
                    [900., 355., 108.]
                ),
                (
                    "vise jaw on +X".to_string(),
                    [1100., 255., 0.],
                    [1120., 355., 108.]
                ),
            ]
        );
        // Vise along Y: centre X 1000 + 5.
        assert_eq!(
            of(&vise_layout(true, 5., 2.), &vise()),
            [
                (
                    "vise jaw on -Y".to_string(),
                    [955., 180., 0.],
                    [1055., 200., 108.]
                ),
                (
                    "vise jaw on +Y".to_string(),
                    [955., 400., 0.],
                    [1055., 420., 108.]
                ),
            ]
        );
        // Clamps (60 long, 20 wide, 25 high, bite 12) on all four sides at offset 0, on the stock top (Z 140).
        let all = [
            (Side::MinusX, 0.),
            (Side::PlusX, 0.),
            (Side::MinusY, 0.),
            (Side::PlusY, 0.),
        ];
        assert_eq!(
            of(&clamp_layout(&all), &clamp()),
            [
                (
                    "clamp on -X at 0".to_string(),
                    [852., 290., 140.],
                    [912., 310., 165.]
                ),
                (
                    "clamp on +X at 0".to_string(),
                    [1088., 290., 140.],
                    [1148., 310., 165.]
                ),
                (
                    "clamp on -Y at 0".to_string(),
                    [990., 152., 140.],
                    [1010., 212., 165.]
                ),
                (
                    "clamp on +Y at 0".to_string(),
                    [990., 388., 140.],
                    [1010., 448., 165.]
                ),
            ]
        );
    }
    #[test]
    fn a_move_crossing_a_box_hits_even_when_both_ends_are_clear() {
        let c = boxes(&clamp_layout(&[(Side::MinusX, 0.)]), &clamp(), stock()).unwrap();
        // X -120 and -20 are 22 and 18 mm from the clamp (X -98..-38), well outside Ø6 + gap 2.
        let across = hits(&plan(3., &[([-120., 0., 41.], [-20., 0., 41.])]), &c, 2.);
        assert_eq!(across.len(), 1);
        assert_eq!(across[0].part, "tool");
        // The hit is where the tool first comes within reach: the clamp's near face at X -98 (the path crosses the box).
        assert!(near(across[0].at, [-98., 0., 41.]), "{:?}", across[0].at);
    }
    #[test]
    fn a_long_move_through_a_thin_clamp_is_not_missed() {
        let mut thin = clamp();
        thin.p.insert("width".into(), "1".into());
        let c = boxes(&clamp_layout(&[(Side::MinusX, 0.)]), &thin, stock()).unwrap();
        // Clamp Y -0.5..0.5; a Ø0.4 tool with gap 0 reaches Y +-0.7. This 3 000 mm move along Y has 1.5 mm between the
        // old samples (-0.75 and 0.75), which both fall outside.
        let through = hits(
            &plan(0.2, &[([-60., -1500.75, 41.], [-60., 1499.25, 41.])]),
            &c,
            0.,
        );
        assert_eq!(through.len(), 1);
        assert!(
            near(through[0].at, [-60., -0.5, 41.]),
            "{:?}",
            through[0].at
        );
        // The same move 8 mm beyond the clamp's edge (X -30 against X -38) misses it.
        assert!(
            hits(
                &plan(0.2, &[([-30., -1500.75, 41.], [-30., 1499.25, 41.])]),
                &c,
                0.
            )
            .is_empty()
        );
    }
    #[test]
    fn a_diagonal_clip_of_a_jaw_corner_hits_only_within_reach() {
        let jaws = boxes(&vise_layout(true, 0., 0.), &vise(), stock()).unwrap();
        // The -Y jaw is X -50..50, Y -50..-30, Z -100..8; its corner (50, -30). A tool tip at Z 5 (Ø6, gap 0)
        // moving along the diagonal (1, -1), 60 mm long, whose nearest point to the corner is `d` mm out along
        // (1, 1) and lies half a millimetre from the nearest 1 mm sample on either side.
        let k = std::f32::consts::FRAC_1_SQRT_2;
        let diagonal = |d: f32| {
            let c = [50. + d * k, -30. + d * k];
            plan(
                3.,
                &[(
                    [c[0] - 30.5 * k, c[1] + 30.5 * k, 5.],
                    [c[0] + 29.5 * k, c[1] - 29.5 * k, 5.],
                )],
            )
        };
        let clip = hits(&diagonal(2.98), &jaws, 0.);
        assert_eq!(clip.len(), 1);
        assert_eq!(clip[0].part, "tool");
        assert!(clip[0].label.contains("-Y"));
        assert!(hits(&diagonal(3.08), &jaws, 0.).is_empty());
    }
    #[test]
    fn a_vise_too_narrow_for_the_stock_says_so_in_millimetres() {
        let wide = |w: f32| Bounds {
            min: [-w / 2., -30., 0.],
            max: [w / 2., 30., 40.],
        };
        let across_x = vise_layout(false, 0., 2.);
        assert_eq!(
            opening_issue(&across_x, &vise(), wide(140.)).unwrap(),
            "The stock is 140 mm across the jaws; this vise opens to 120 mm."
        );
        // A stock exactly as wide as the opening fits; a hair over is rounding.
        assert_eq!(opening_issue(&across_x, &vise(), wide(120.)), None);
        assert_eq!(opening_issue(&across_x, &vise(), wide(120.004)), None);
        assert!(
            opening_issue(&across_x, &vise(), wide(120.5))
                .unwrap()
                .contains("120.5 mm across")
        );
        // Jaws on the Y sides look at the stock's Y extent, not its X.
        let across_y = vise_layout(true, 0., 2.);
        assert_eq!(opening_issue(&across_y, &vise(), wide(140.)), None);
        let tall = Bounds {
            min: [-50., -65., 0.],
            max: [50., 65., 40.],
        };
        assert_eq!(
            opening_issue(&across_y, &vise(), tall).unwrap(),
            "The stock is 130 mm across the jaws; this vise opens to 120 mm."
        );
        assert_eq!(opening_issue(&across_x, &vise(), tall), None);
        // Clamps and no workholding have no opening.
        let clamps = clamp_layout(&[(Side::MinusX, 0.)]);
        assert_eq!(opening_issue(&clamps, &clamp(), wide(500.)), None);
        assert_eq!(opening_issue(&Layout::None, &vise(), wide(500.)), None);
    }
    #[test]
    fn a_holders_name_and_notes_are_bounded_like_a_materials() {
        let mut v = vise();
        v.p.insert("name".into(), "x".repeat(60));
        assert!(v.issues().is_empty());
        v.p.insert("name".into(), "x".repeat(61));
        assert_eq!(v.issues(), ["The name can be at most 60 characters."]);
        v.p.insert("name".into(), " ".into());
        assert_eq!(v.issues(), ["Give the workholding a name."]);
        v.p.insert("name".into(), "Vise".into());
        v.p.insert("notes".into(), "n".repeat(500));
        assert!(v.issues().is_empty());
        v.p.insert("notes".into(), "n".repeat(501));
        assert_eq!(v.issues(), ["Notes can be at most 500 characters."]);
    }
    #[test]
    fn the_refusal_is_what_boxes_refuses() {
        let jaws = vise_layout(false, 0., 2.);
        let clamps = clamp_layout(&[(Side::MinusX, 0.)]);
        assert_eq!(refusal(&jaws, &vise()), None);
        assert_eq!(refusal(&clamps, &clamp()), None);
        assert_eq!(refusal(&Layout::None, &Holder::new("Clamp", "x")), None);
        assert_eq!(
            refusal(&jaws, &clamp()).unwrap(),
            boxes(&jaws, &clamp(), stock()).unwrap_err()
        );
        assert_eq!(
            refusal(&clamps, &Holder::new("Clamp", "Empty")).unwrap(),
            boxes(&clamps, &Holder::new("Clamp", "Empty"), stock()).unwrap_err()
        );
    }
    #[test]
    fn typed_clamp_positions_are_read_whatever_the_setups_other_keys_say() {
        let setup = setup(&[("holding", "Vise"), ("holding_gap", "abc")]);
        assert_eq!(check_clamps(&setup, "-X:0;+X:0"), Ok(()));
        assert_eq!(check_clamps(&setup, " -X : 5 "), Ok(()));
        assert_eq!(
            check_clamps(&setup, "-X"),
            Err("Write each clamp as side:offset, for example -X:0.".to_string())
        );
        assert_eq!(
            check_clamps(&setup, "-Z:0"),
            Err("A clamp side must be -X, +X, -Y or +Y.".to_string())
        );
        assert_eq!(
            check_clamps(&setup, ""),
            Err("Place 1 to 8 clamps.".to_string())
        );
        assert!(check_clamps(&setup, &["-X:0"; 9].join(";")).is_err());
    }
    #[test]
    fn slopes_and_plunges_are_checked_exactly() {
        let c = boxes(&clamp_layout(&[(Side::MinusX, 0.)]), &clamp(), stock()).unwrap();
        // Diving across the clamp: the tool is low enough while it is over the clamp.
        let dive = hits(&plan(3., &[([-100., 0., 100.], [-20., 0., 41.])]), &c, 2.);
        assert_eq!(dive.len(), 1);
        assert_eq!(dive[0].part, "tool");
        // The same dive from higher up gets low enough only after the tool has passed the clamp.
        assert!(hits(&plan(3., &[([-100., 0., 300.], [-20., 0., 41.])]), &c, 2.).is_empty());
        // A straight plunge into the clamp is a hit at the Z where the tool tip is 2 mm above its top (65).
        let plunge = hits_segments(&[([-60., 0., 100.], [-60., 0., 41.])], 3., 30., 16., &c, 2.);
        assert_eq!(plunge.len(), 1);
        assert!(near(plunge[0].at, [-60., 0., 67.]), "{:?}", plunge[0].at);
        // The same plunge beside the clamp is clear.
        assert!(
            hits_segments(
                &[([-60., 40., 100.], [-60., 40., 0.])],
                3.,
                30.,
                16.,
                &c,
                2.
            )
            .is_empty()
        );
    }
}
