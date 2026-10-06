//! Workholding: vises and clamps from the library, where they sit in a
//! setup, and whether a toolpath comes too close to them. Toolpaths are
//! never changed; moves that hit are reported.
use crate::mesh::Bounds;
use crate::model::{Fields, Setup, fields, get, id, num};
use crate::simulation::Plan;
use serde::{Deserialize, Serialize};

pub const KINDS: [&str; 2] = ["Vise", "Clamp"];
const NOTE: &str = "Measure yours: these are typical sizes.";
/// How far below the stock bottom a vise jaw is modelled to reach.
const JAW_DEPTH: f32 = 100.;
pub const MAX_CLAMPS: usize = 8;

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
pub fn layout(setup: &Setup) -> Result<Layout, String> {
    let p = &setup.p;
    let gap = match get(p, "holding_gap").trim() {
        "" => 2.,
        _ => num(p, "holding_gap")
            .filter(|g| *g >= 0.)
            .ok_or("The clearance gap must be 0 or more.")?,
    };
    let holder = get(p, "holder").to_string();
    match get(p, "holding") {
        "" | "None" => Ok(Layout::None),
        "Vise" => {
            let grip = num(p, "vise_grip")
                .filter(|g| *g > 0.)
                .ok_or("The vise grip must be a positive number.")?;
            let offset = match get(p, "vise_offset").trim() {
                "" => 0.,
                _ => num(p, "vise_offset").ok_or("The vise offset must be a number.")?,
            };
            Ok(Layout::Vise {
                holder,
                axis_y: get(p, "vise_axis") == "Y",
                offset,
                grip,
                gap,
            })
        }
        "Clamps" => {
            let mut clamps = vec![];
            for part in get(p, "clamps").split(';').filter(|s| !s.trim().is_empty()) {
                let (side, offset) = part
                    .split_once(':')
                    .ok_or("Write each clamp as side:offset, for example -X:0.")?;
                let side = Side::parse(side).ok_or("A clamp side must be -X, +X, -Y or +Y.")?;
                let offset = offset
                    .trim()
                    .parse::<f32>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .ok_or("A clamp offset must be a number.")?;
                clamps.push((side, offset));
            }
            if clamps.is_empty() || clamps.len() > MAX_CLAMPS {
                return Err(format!("Place 1 to {MAX_CLAMPS} clamps."));
            }
            Ok(Layout::Clamps {
                holder,
                clamps,
                gap,
            })
        }
        other => Err(format!("\"{other}\" is not a workholding choice.")),
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct HoldBox {
    pub label: String,
    pub bounds: Bounds,
}
/// The jaws or clamps of `layout`, in the frame of `stock` (the setup's
/// starting stock box). Without the clearance gap.
pub fn boxes(layout: &Layout, holder: &Holder, stock: Bounds) -> Vec<HoldBox> {
    let n = |k| num(&holder.p, k).unwrap_or(0.);
    let centre = [
        (stock.min[0] + stock.max[0]) / 2.,
        (stock.min[1] + stock.max[1]) / 2.,
    ];
    match layout {
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
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub label: String,
    /// "tool" (cutter and shank, tip to stickout) or "holder" (above the stickout).
    pub part: &'static str,
    pub at: [f32; 3],
}
/// Distance in XY from `p` to the rectangle of `b`.
fn xy_distance(b: &Bounds, p: [f32; 3]) -> f32 {
    let dx = (b.min[0] - p[0]).max(0.).max(p[0] - b.max[0]);
    let dy = (b.min[1] - p[1]).max(0.).max(p[1] - b.max[1]);
    dx.hypot(dy)
}
/// Moves of `plan` whose tool or holder comes within `gap` of a box. The
/// tool is a vertical cylinder of the cutter radius from the tip to the tip
/// plus the stickout, the holder one of the holder radius from there
/// upward. Moves are sampled every 1 mm (at least both ends, at most 2 000
/// samples); one hit per box and part.
pub fn hits(plan: &Plan, boxes: &[HoldBox], gap: f32) -> Vec<Hit> {
    let mut out: Vec<Hit> = vec![];
    for m in &plan.moves {
        let length = (0..3)
            .map(|k| (m.to[k] - m.from[k]).powi(2))
            .sum::<f32>()
            .sqrt();
        let steps = (length.ceil() as usize).clamp(1, 2000);
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let tip: [f32; 3] = std::array::from_fn(|k| m.from[k] + (m.to[k] - m.from[k]) * t);
            for b in boxes {
                let parts = [
                    ("tool", plan.radius, tip[2], tip[2] + plan.tool_length),
                    (
                        "holder",
                        plan.holder_radius,
                        tip[2] + plan.tool_length,
                        f32::INFINITY,
                    ),
                ];
                for (part, radius, low, high) in parts {
                    let overlaps = low <= b.bounds.max[2] + gap && high >= b.bounds.min[2] - gap;
                    if overlaps
                        && xy_distance(&b.bounds, tip) <= radius + gap
                        && !out.iter().any(|h| h.label == b.label && h.part == part)
                    {
                        out.push(Hit {
                            label: b.label.clone(),
                            part,
                            at: tip,
                        });
                    }
                }
            }
        }
    }
    out
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
        );
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
        );
        assert_eq!(c[0].bounds.min, [-98., 0., 40.]);
        assert_eq!(c[0].bounds.max, [-38., 20., 65.]);
        assert!(c[0].label.contains("-X"));
        assert!(boxes(&Layout::None, &vise(), stock()).is_empty());
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
        );
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
}
