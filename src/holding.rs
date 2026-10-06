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
    fn opposite(self) -> Self {
        match self {
            Side::MinusX => Side::PlusX,
            Side::PlusX => Side::MinusX,
            Side::MinusY => Side::PlusY,
            Side::PlusY => Side::MinusY,
        }
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
/// The clearance gap a setup keeps around its jaws or clamps, in mm: 2 when
/// the field is empty.
pub fn gap(setup: &Setup) -> Result<f32, String> {
    match get(&setup.p, "holding_gap").trim() {
        "" => Ok(2.),
        text => number(text)
            .filter(|g| *g >= 0.)
            .ok_or_else(|| "The clearance gap must be 0 or more.".into()),
    }
}
pub fn layout(setup: &Setup) -> Result<Layout, String> {
    let p = &setup.p;
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
                gap: gap(setup)?,
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
                gap: gap(setup)?,
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
/// The least grip a suggested vise is reduced to, in mm.
const MIN_GRIP: f32 = 3.;
/// How far the cutter's edge stays from `footprint` (a box seen from above)
/// along the stretches of the `plans`' moves whose tip is at or below `top`;
/// infinite when no move is.
fn clearance(plans: &[&Plan], footprint: &Bounds, top: f32) -> f32 {
    let mut least = f32::INFINITY;
    for plan in plans {
        for m in &plan.moves {
            let (from, to) = (m.from.map(f64::from), m.to.map(f64::from));
            let Some((t0, t1)) = within(from[2], to[2], f64::NEG_INFINITY, f64::from(top)) else {
                continue;
            };
            let tip = |t: f64| [0, 1].map(|k| from[k] + (to[k] - from[k]) * t);
            let (distance, _) = xy_closest(footprint, tip(t0), tip(t1));
            least = least.min(distance as f32 - plan.radius);
        }
    }
    least
}
/// The vise jaws' axis (true: jaws on the Y sides) and grip for the `plans`
/// of a setup whose stock is `stock`, at offset 0. Each axis tries the
/// holder's grip, then 1 mm less at a time down to 3 mm, for the first at
/// which no move comes within `gap` of the jaws (`hits`); the axis clear at
/// the larger grip wins.
/// With neither clear the grip is 3 mm and the axis whose jaws are farther
/// from the moves wins. Only the axes whose width across the jaws fits the
/// vise's opening are tried, when one does. A tie goes to the axis the stock
/// is narrower across.
pub fn suggest_vise(stock: Bounds, holder: &Holder, plans: &[&Plan], gap: f32) -> (bool, f32) {
    let start = num(&holder.p, "grip")
        .filter(|g| *g > 0.)
        .unwrap_or(MIN_GRIP);
    // The grips to try: the holder's, then 1 mm less at a time while above 3 mm, then 3 mm itself.
    let steps = ((start - MIN_GRIP).max(0.).ceil() as usize).saturating_add(1);
    let grip_at = |k: usize| {
        if k + 1 < steps {
            start - k as f32
        } else {
            start.min(MIN_GRIP)
        }
    };
    let layout = |axis_y, grip| Layout::Vise {
        holder: String::new(),
        axis_y,
        offset: 0.,
        grip,
        gap,
    };
    let jaws = |axis_y, grip| boxes(&layout(axis_y, grip), holder, stock).unwrap_or_default();
    let fits = |axis_y| opening_issue(&layout(axis_y, start), holder, stock).is_none();
    let across = |axis_y: bool| stock.dimensions()[usize::from(axis_y)];
    let mut axes = vec![false, true];
    if axes.iter().any(|a| fits(*a)) {
        axes.retain(|a| fits(*a));
    }
    axes.sort_by(|a, b| across(*a).total_cmp(&across(*b)));
    let clear = |axis_y: bool, grip: f32| {
        let placed = jaws(axis_y, grip);
        plans.iter().all(|plan| hits(plan, &placed, gap).is_empty())
    };
    let tried = |axis_y: bool| {
        // A higher jaw only adds hits, so the clear grips are the tail of the
        // list: bisect for the first of them.
        let (mut low, mut high) = (0, steps);
        while low < high {
            let mid = low + (high - low) / 2;
            if clear(axis_y, grip_at(mid)) {
                high = mid;
            } else {
                low = mid + 1;
            }
        }
        if low < steps {
            let grip = grip_at(low);
            return (axis_y, grip, true, grip);
        }
        let grip = grip_at(steps - 1);
        let nearest = jaws(axis_y, grip)
            .iter()
            .map(|j| clearance(plans, &j.bounds, j.bounds.max[2] + gap))
            .fold(f32::INFINITY, f32::min);
        (axis_y, grip, false, nearest)
    };
    axes.into_iter()
        .map(tried)
        .reduce(|best, next| {
            let order = next.2.cmp(&best.2).then(next.3.total_cmp(&best.3));
            if order.is_gt() { next } else { best }
        })
        .map_or((false, start), |(axis_y, grip, ..)| (axis_y, grip))
}
/// The clamps for the `plans` of a setup whose stock is `stock`: 4, or 2 when
/// the stock is shorter than 3 clamp widths along both axes. The candidates
/// are the middle and the quarter points of each side; a candidate's score is
/// how far the cutter's edge stays from the clamp (grown by `gap`) over the
/// moves whose tip is below the clamp's top plus `gap`, infinite for none.
/// Greedy, one clamp per side: the best candidate, the best on the opposite
/// side, then the best on the remaining sides and again the opposite one.
/// Ties keep the middle. In the order of `Side::ALL`.
pub fn suggest_clamps(
    stock: Bounds,
    holder: &Holder,
    plans: &[&Plan],
    gap: f32,
) -> Vec<(Side, f32)> {
    let [x, y, _] = stock.dimensions();
    let width = num(&holder.p, "width").unwrap_or(0.);
    let count = if x < 3. * width && y < 3. * width {
        2
    } else {
        4
    };
    let score = |side, offset| {
        let layout = Layout::Clamps {
            holder: String::new(),
            clamps: vec![(side, offset)],
            gap,
        };
        let placed = boxes(&layout, holder, stock).unwrap_or_default();
        placed.iter().fold(f32::INFINITY, |least, c| {
            let b = c.bounds;
            let grown = Bounds {
                min: [b.min[0] - gap, b.min[1] - gap, b.min[2]],
                max: [b.max[0] + gap, b.max[1] + gap, b.max[2]],
            };
            least.min(clearance(plans, &grown, b.max[2] + gap))
        })
    };
    let candidates: Vec<(Side, f32, f32)> = Side::ALL
        .into_iter()
        .flat_map(|side| {
            let length = if matches!(side, Side::MinusX | Side::PlusX) {
                y
            } else {
                x
            };
            let quarter = (length / 4. * 100.).round() / 100.;
            [0., -quarter, quarter]
                .into_iter()
                .map(move |offset| (side, offset))
        })
        .map(|(side, offset)| (side, offset, score(side, offset)))
        .collect();
    let mut chosen: Vec<(Side, f32)> = vec![];
    while chosen.len() < count {
        let allowed: Vec<Side> = match chosen.last() {
            Some(&(last, _)) if chosen.len() % 2 == 1 => vec![last.opposite()],
            _ => Side::ALL
                .into_iter()
                .filter(|side| !chosen.iter().any(|c| c.0 == *side))
                .collect(),
        };
        let best = candidates.iter().filter(|c| allowed.contains(&c.0)).fold(
            None::<&(Side, f32, f32)>,
            |best, c| match best {
                Some(b) if b.2 >= c.2 => Some(b),
                _ => Some(c),
            },
        );
        let Some(&(side, offset, _)) = best else {
            break;
        };
        chosen.push((side, offset));
    }
    chosen.sort_by_key(|c| Side::ALL.iter().position(|side| *side == c.0));
    chosen
}
/// How many of `plans` have a move that comes within `gap` of `boxes`.
pub fn clashes(plans: &[&Plan], boxes: &[HoldBox], gap: f32) -> usize {
    plans
        .iter()
        .filter(|plan| !hits(plan, boxes, gap).is_empty())
        .count()
}
/// The setup keys that hold the placement of `layout`.
pub fn placement(layout: &Layout) -> Vec<(&'static str, String)> {
    match layout {
        Layout::None => vec![],
        Layout::Vise {
            axis_y,
            offset,
            grip,
            ..
        } => vec![
            ("vise_axis", if *axis_y { "Y" } else { "X" }.into()),
            ("vise_offset", mm(*offset)),
            ("vise_grip", mm(*grip)),
        ],
        Layout::Clamps { clamps, .. } => vec![("clamps", clamps_text(clamps))],
    }
}
/// Where `layout` puts the jaws or clamps, for a status line.
fn describe(layout: &Layout) -> String {
    match layout {
        Layout::None => String::new(),
        Layout::Vise { axis_y, grip, .. } => format!(
            "vise jaws on ±{}, grip {} mm",
            if *axis_y { "Y" } else { "X" },
            mm(*grip)
        ),
        Layout::Clamps { clamps, .. } => format!(
            "{} clamps on {}",
            clamps.len(),
            clamps
                .iter()
                .map(|(side, offset)| format!("{} at {offset}", side.label()))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
/// The status line for a suggested `layout`: where it puts the jaws or
/// clamps, how many of the `considered` operations come within the gap
/// (`clashing`) and how many `left_out` could not be generated. Without
/// operations only the stock decided, and it says so.
pub fn summary(layout: &Layout, considered: usize, clashing: usize, left_out: usize) -> String {
    let verdict = match (considered, clashing, left_out) {
        (0, ..) => "No toolpaths were available to check, so only the stock was considered.".into(),
        (_, 0, 0) => "Every operation is clear of them.".into(),
        (_, 0, _) => "Every operation considered is clear of them.".into(),
        (_, 1, _) => "1 operation comes within the gap; see the warnings.".into(),
        (_, n, _) => format!("{n} operations come within the gap; see the warnings."),
    };
    let skipped = match left_out {
        0 => String::new(),
        1 => " 1 operation was not considered because it could not be generated.".into(),
        n => format!(" {n} operations were not considered because they could not be generated."),
    };
    format!("Suggested: {}. {verdict}{skipped}", describe(layout))
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
    /// Feed moves around a rectangle `x` and `y` from the stock's middle, at Z `z`.
    fn rectangle(x: f32, y: f32, z: f32) -> Vec<([f32; 3], [f32; 3])> {
        let c = [[x, y], [-x, y], [-x, -y], [x, -y]];
        (0..4)
            .map(|i| {
                let (a, b) = (c[i], c[(i + 1) % 4]);
                ([a[0], a[1], z], [b[0], b[1], z])
            })
            .collect()
    }
    fn vise_for(stock: Bounds, holder: &Holder, plans: &[&Plan]) -> (bool, f32) {
        suggest_vise(stock, holder, plans, 2.)
    }
    fn near_the_x_sides() -> Plan {
        plan(
            3.,
            &[
                ([48., -15., 1.], [48., 15., 1.]),
                ([-48., 15., 1.], [-48., -15., 1.]),
            ],
        )
    }
    fn near_the_y_sides() -> Plan {
        plan(
            3.,
            &[
                ([-30., 28., 1.], [30., 28., 1.]),
                ([30., -28., 1.], [-30., -28., 1.]),
            ],
        )
    }
    #[test]
    fn the_vise_goes_on_the_sides_the_toolpaths_leave_alone() {
        // Without moves both axes are clear: the jaws go where the stock is
        // narrower, across Y (60 against 100).
        assert_eq!(vise_for(stock(), &vise(), &[]), (true, 8.));
        // Contours at X +-48 run beside the jaws on the X sides.
        assert_eq!(
            vise_for(stock(), &vise(), &[&near_the_x_sides()]),
            (true, 8.)
        );
        // Contours near +-Y: the jaws go on the X sides, though the stock is wider across them.
        assert_eq!(
            vise_for(stock(), &vise(), &[&near_the_y_sides()]),
            (false, 8.)
        );
        // Every plan counts, not only the first.
        assert_eq!(
            vise_for(
                stock(),
                &vise(),
                &[&near_the_x_sides(), &near_the_y_sides()]
            ),
            (true, 3.)
        );
    }
    #[test]
    fn a_deep_pocket_on_every_side_lowers_the_grip_and_keeps_the_clearer_axis() {
        // A floor at Z 7.5 near all four sides: a grip of 5 (jaw top 5 + gap 2 < 7.5) is the largest that clears
        // it, on either axis, so the narrower stock decides.
        let floor = plan(3., &rectangle(47., 26., 7.5));
        assert_eq!(vise_for(stock(), &vise(), &[&floor]), (true, 5.));
        // A floor at Z 5 is reached even at grip 3 (3 + 2): the grip is 3 and the axis whose jaws are farther
        // from the moves is kept: X, 4 mm from the contour against 3 mm for Y.
        let deep = plan(3., &rectangle(46., 27., 5.));
        assert_eq!(vise_for(stock(), &vise(), &[&deep]), (false, 3.));
        // The reduction ends at 3 mm, not at 3.5 for a grip of 8.5.
        let mut fine = vise();
        fine.p.insert("grip".into(), "8.5".into());
        assert_eq!(vise_for(stock(), &fine, &[&deep]), (false, 3.));
    }
    #[test]
    fn a_tall_grip_is_reduced_to_the_largest_that_clears() {
        // A grip of 100 000 mm comes down to the 5 that clears a floor at Z 7.5.
        let floor = plan(3., &rectangle(47., 26., 7.5));
        let mut tall = vise();
        tall.p.insert("grip".into(), "100000".into());
        assert_eq!(vise_for(stock(), &tall, &[&floor]), (true, 5.));
        // A hit at every grip, however many are tried, ends at 3.
        let deep = plan(3., &rectangle(47., 26., 5.));
        assert_eq!(vise_for(stock(), &tall, &[&deep]).1, 3.);
    }
    #[test]
    fn the_vise_grip_starts_at_the_holders_and_is_never_raised() {
        let mut tall = vise();
        tall.p.insert("grip".into(), "12".into());
        assert_eq!(vise_for(stock(), &tall, &[]), (true, 12.));
        let mut shallow = vise();
        shallow.p.insert("grip".into(), "2".into());
        assert_eq!(vise_for(stock(), &shallow, &[]), (true, 2.));
        // A grip of 4 reduces to 3 and no lower.
        let mut four = vise();
        four.p.insert("grip".into(), "4".into());
        let deep = plan(3., &rectangle(47., 26., 5.));
        assert_eq!(vise_for(stock(), &four, &[&deep]).1, 3.);
    }
    #[test]
    fn the_vise_axis_must_fit_its_opening_when_one_axis_does() {
        // 140 across X does not fit a 120 mm opening, 60 across Y does: Y, though the moves
        // near +-Y hit it at every grip.
        let wide = Bounds {
            min: [-70., -30., 0.],
            max: [70., 30., 40.],
        };
        assert_eq!(vise_for(wide, &vise(), &[&near_the_y_sides()]), (true, 3.));
        // Neither fits (140 and 130): both are considered; the moves near +-Y rule out Y.
        let huge = Bounds {
            min: [-70., -65., 0.],
            max: [70., 65., 40.],
        };
        let near_y = plan(
            3.,
            &[
                ([-30., 63., 1.], [30., 63., 1.]),
                ([30., -63., 1.], [-30., -63., 1.]),
            ],
        );
        assert_eq!(vise_for(huge, &vise(), &[&near_y]), (false, 8.));
    }
    #[test]
    fn the_cutter_size_counts_in_how_far_the_moves_are() {
        // Both axes hit at every grip. The X jaws see a Ø6 contour 4 mm away, the Y jaws a Ø20 one 5 mm away:
        // the Ø20 tool's edge is the closer, so the jaws go on the X sides.
        let narrow = plan(3., &[([46., -15., 5.], [46., 15., 5.])]);
        let broad = plan(10., &[([-20., 25., 5.], [20., 25., 5.])]);
        assert_eq!(vise_for(stock(), &vise(), &[&narrow, &broad]), (false, 3.));
    }
    #[test]
    fn the_suggestion_follows_a_stock_away_from_the_origin() {
        let stock = Bounds {
            min: [900., 200., 100.],
            max: [1100., 400., 140.],
        };
        // Contours 2 mm inside the X sides, 1 mm above the stock's bottom.
        let beside = plan(
            3.,
            &[
                ([902., 280., 101.], [902., 320., 101.]),
                ([1098., 320., 101.], [1098., 280., 101.]),
            ],
        );
        // 200 mm across either axis: neither fits, both are considered, the X jaws are hit.
        assert_eq!(vise_for(stock, &vise(), &[&beside]), (true, 8.));
        let clamps = suggest_clamps(stock, &clamp(), &[&beside], 2.);
        assert_eq!(clamps.len(), 4);
        let layout = clamp_layout(&clamps);
        let placed = boxes(&layout, &clamp(), stock).unwrap();
        assert!(hits(&beside, &placed, 2.).is_empty(), "{clamps:?}");
    }
    fn sides_of(clamps: &[(Side, f32)]) -> Vec<Side> {
        clamps.iter().map(|c| c.0).collect()
    }
    #[test]
    fn clamps_leave_a_side_whose_middle_the_toolpaths_cross() {
        // A move along X at Y 0 from X -80 to -20, 1 mm above the stock's top, runs through the -X clamp's
        // middle position (X -98..-38, Y -10..10, Z 40..65).
        let lead = plan(2., &[([-80., 0., 41.], [-20., 0., 41.])]);
        let crossed = boxes(&clamp_layout(&[(Side::MinusX, 0.)]), &clamp(), stock()).unwrap();
        assert!(!hits(&lead, &crossed, 2.).is_empty());
        let chosen = suggest_clamps(stock(), &clamp(), &[&lead], 2.);
        // One clamp per side, in side order; the -X one at a quarter of that side (60 mm long).
        assert_eq!(sides_of(&chosen), Side::ALL);
        assert_eq!(chosen[0].1.abs(), 15.);
        let placed = boxes(&clamp_layout(&chosen), &clamp(), stock()).unwrap();
        assert!(hits(&lead, &placed, 2.).is_empty(), "{chosen:?}");
        // The Y sides' quarter points are a quarter of their 100 mm.
        let along_y = plan(2., &[([0., 20., 41.], [0., 70., 41.])]);
        let chosen = suggest_clamps(stock(), &clamp(), &[&along_y], 2.);
        assert_eq!(sides_of(&chosen), Side::ALL);
        assert_eq!(chosen[3].1.abs(), 25.);
        let placed = boxes(&clamp_layout(&chosen), &clamp(), stock()).unwrap();
        assert!(hits(&along_y, &placed, 2.).is_empty(), "{chosen:?}");
    }
    #[test]
    fn a_cut_low_in_the_stock_counts_for_the_clamps_above_it() {
        // The tip at Z 5 is below the clamps' top (65 + 2): the cut along Y through the middle of the stock
        // is 13 mm from the Y sides' quarter points but inside their middles' reach.
        let cut = plan(2., &[([0., -20., 5.], [0., 20., 5.])]);
        let chosen = suggest_clamps(stock(), &clamp(), &[&cut], 2.);
        assert_eq!(chosen[2].1.abs(), 25., "{chosen:?}");
        assert_eq!(chosen[3].1.abs(), 25., "{chosen:?}");
    }
    #[test]
    fn clamps_are_four_one_per_side_and_the_middles_with_no_moves_nearby() {
        let middles = [
            (Side::MinusX, 0.),
            (Side::PlusX, 0.),
            (Side::MinusY, 0.),
            (Side::PlusY, 0.),
        ];
        assert_eq!(suggest_clamps(stock(), &clamp(), &[], 2.), middles);
        // A rapid far above the clamps (its tip over their top + gap) changes nothing.
        let high = plan(3., &[([-90., 0., 100.], [90., 0., 100.])]);
        assert_eq!(suggest_clamps(stock(), &clamp(), &[&high], 2.), middles);
    }
    #[test]
    fn a_small_stock_gets_two_clamps_on_opposite_sides() {
        let square = |size: [f32; 2]| Bounds {
            min: [-size[0] / 2., -size[1] / 2., 0.],
            max: [size[0] / 2., size[1] / 2., 20.],
        };
        // Both sides shorter than 3 clamp widths (60): 2 clamps.
        assert_eq!(
            suggest_clamps(square([50., 50.]), &clamp(), &[], 2.),
            [(Side::MinusX, 0.), (Side::PlusX, 0.)]
        );
        // One side of 60 is enough for 4.
        assert_eq!(
            suggest_clamps(square([60., 50.]), &clamp(), &[], 2.).len(),
            4
        );
        assert_eq!(
            suggest_clamps(square([50., 60.]), &clamp(), &[], 2.).len(),
            4
        );
        assert_eq!(
            suggest_clamps(square([59.9, 59.9]), &clamp(), &[], 2.).len(),
            2
        );
        // The two stay on opposite sides whatever the moves do; the -X one leaves its crossed middle.
        let lead = plan(2., &[([-80., 0., 21.], [-20., 0., 21.])]);
        let chosen = suggest_clamps(square([50., 50.]), &clamp(), &[&lead], 2.);
        assert_eq!(sides_of(&chosen), [Side::MinusX, Side::PlusX]);
        assert_eq!(chosen[0].1, -12.5);
    }
    #[test]
    fn the_clashes_are_counted_per_plan() {
        let c = boxes(&clamp_layout(&[(Side::MinusX, 0.)]), &clamp(), stock()).unwrap();
        let into = plan(3., &[([-60., 0., 41.], [-30., 0., 41.])]);
        let beside = plan(3., &[([-60., 40., 41.], [-30., 40., 41.])]);
        assert_eq!(clashes(&[&into, &beside, &into], &c, 2.), 2);
        assert_eq!(clashes(&[&beside], &c, 2.), 0);
        assert_eq!(clashes(&[], &c, 2.), 0);
    }
    #[test]
    fn a_suggested_layout_is_written_and_described_in_the_setups_own_terms() {
        let jaws = Layout::Vise {
            holder: "v".into(),
            axis_y: true,
            offset: 0.,
            grip: 8.,
            gap: 2.,
        };
        assert_eq!(
            placement(&jaws),
            [
                ("vise_axis", "Y".to_string()),
                ("vise_offset", "0".to_string()),
                ("vise_grip", "8".to_string())
            ]
        );
        assert_eq!(describe(&jaws), "vise jaws on ±Y, grip 8 mm");
        let across_x = Layout::Vise {
            holder: "v".into(),
            axis_y: false,
            offset: 0.,
            grip: 3.5,
            gap: 2.,
        };
        assert_eq!(placement(&across_x)[2], ("vise_grip", "3.5".to_string()));
        assert_eq!(describe(&across_x), "vise jaws on ±X, grip 3.5 mm");
        let clamps = Layout::Clamps {
            holder: "c".into(),
            clamps: vec![(Side::MinusX, -15.), (Side::PlusX, 0.)],
            gap: 2.,
        };
        assert_eq!(placement(&clamps), [("clamps", "-X:-15;+X:0".to_string())]);
        assert_eq!(describe(&clamps), "2 clamps on -X at -15, +X at 0");
        assert!(placement(&Layout::None).is_empty());
    }
    #[test]
    fn the_summary_says_what_was_chosen_and_how_the_operations_fare() {
        let jaws = Layout::Vise {
            holder: "v".into(),
            axis_y: true,
            offset: 0.,
            grip: 8.,
            gap: 2.,
        };
        let say = |considered, clashing, left_out| summary(&jaws, considered, clashing, left_out);
        assert_eq!(
            say(6, 0, 0),
            "Suggested: vise jaws on ±Y, grip 8 mm. Every operation is clear of them."
        );
        assert_eq!(
            say(5, 0, 1),
            "Suggested: vise jaws on ±Y, grip 8 mm. Every operation considered is clear of them. 1 operation was not considered because it could not be generated."
        );
        assert_eq!(
            say(6, 1, 0),
            "Suggested: vise jaws on ±Y, grip 8 mm. 1 operation comes within the gap; see the warnings."
        );
        assert_eq!(
            say(6, 3, 2),
            "Suggested: vise jaws on ±Y, grip 8 mm. 3 operations come within the gap; see the warnings. 2 operations were not considered because they could not be generated."
        );
        // Nothing to check: only the stock decided, with or without operations left out.
        assert_eq!(
            say(0, 0, 0),
            "Suggested: vise jaws on ±Y, grip 8 mm. No toolpaths were available to check, so only the stock was considered."
        );
        assert!(say(0, 0, 4).ends_with(
            "only the stock was considered. 4 operations were not considered because they could not be generated."
        ));
    }
    #[test]
    fn the_gap_is_two_unless_the_setup_says_otherwise() {
        assert_eq!(gap(&setup(&[])), Ok(2.));
        assert_eq!(gap(&setup(&[("holding_gap", " ")])), Ok(2.));
        assert_eq!(gap(&setup(&[("holding_gap", " 3.5 ")])), Ok(3.5));
        assert_eq!(gap(&setup(&[("holding_gap", "0")])), Ok(0.));
        for bad in ["-1", "abc", "NaN"] {
            assert_eq!(
                gap(&setup(&[("holding_gap", bad)])),
                Err("The clearance gap must be 0 or more.".to_string()),
                "{bad}"
            );
        }
    }
}
