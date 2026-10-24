//! Round parts held on a 3-axis mill (LIO-249): a round bar lying along X
//! (`stock::lying`) in a V-block or in soft jaws bored along X, or one
//! standing along Z in soft jaws bored upright or in an ER collet block
//! stood in the vise. What a setup's keys place (`layout`), what is wrong
//! with the numbers (`issues`), the solids the cutter is kept out of
//! (`boxes`), the zeros on the bar's axis (`ORIGINS`), and what the contact
//! resists of a cut (`grip`), the round-contact model `forces::check` sets
//! against the cut's push.
use crate::holding::{HoldBox, Holder, Shape};
use crate::mesh::Bounds;
use crate::model::{Setup, get, num};

pub const V_BLOCK: &str = "V-block";
pub const SOFT_JAWS: &str = "Soft jaws";
pub const COLLET_BLOCK: &str = "Collet block";
/// The library kinds, which are also a setup's `holding` choices.
pub const KINDS: [&str; 3] = [V_BLOCK, SOFT_JAWS, COLLET_BLOCK];
/// How far below the bar a V-block, a soft jaw or a collet block is
/// modelled to reach, mm: down to the vise or the table.
const DEPTH: f32 = 100.;
/// How many steps the flanks of a V and the bore of a soft jaw are cut
/// into: boxes that hold the round surface, never inside it by more than
/// one step's rise.
const STEPS: usize = 4;
/// A V-block's clamp force when its library entry gives none, N: the M8
/// screw of a V-block's strap clamp tightened by hand.
pub const V_BLOCK_FORCE: f32 = 3000.;
/// How far the bar's diameter may be from the soft jaws' bore, mm: more
/// and the jaws close on its edges, not round it.
pub const BORE_FIT: f32 = 0.5;
/// What a collet's grip length must be of the collet's to grip with its
/// full force: less grips in proportion.
pub const FULL_GRIP: f32 = 2. / 3.;

/// ER collet sizes: (size, smallest and largest Ø it closes on, the
/// collet's length, the nut's Ø and height, mm, and its grip, N: all its
/// segments together, the nut tightened with its spanner, on the low side).
pub const ER: [(u32, f32, f32, f32, f32, f32, f32); 6] = [
    (11, 0.5, 7., 18., 16., 11.5, 2000.),
    (16, 0.5, 10., 27.5, 22., 17.5, 3500.),
    (20, 1., 13., 31.5, 25., 19., 5000.),
    (25, 1., 16., 34., 42., 20., 7000.),
    (32, 2., 20., 40., 50., 22.5, 10000.),
    (40, 3., 26., 46., 63., 25.5, 14000.),
];
/// The ER row of `holder`'s size, None when it is not one of `ER`'s.
fn er(holder: &Holder) -> Option<(u32, f32, f32, f32, f32, f32, f32)> {
    let size = num(&holder.p, "er")?;
    ER.iter().copied().find(|row| row.0 as f32 == size)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    VBlock,
    SoftJaws,
    Collet,
}
impl Kind {
    /// The kind a library entry or a setup's `holding` names.
    pub fn of(name: &str) -> Option<Self> {
        match name {
            V_BLOCK => Some(Kind::VBlock),
            SOFT_JAWS => Some(Kind::SoftJaws),
            COLLET_BLOCK => Some(Kind::Collet),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Kind::VBlock => V_BLOCK,
            Kind::SoftJaws => SOFT_JAWS,
            Kind::Collet => COLLET_BLOCK,
        }
    }
    /// As a sentence says it.
    pub fn words(self) -> &'static str {
        match self {
            Kind::VBlock => "V-block",
            Kind::SoftJaws => "soft jaws",
            Kind::Collet => "collet",
        }
    }
}
/// The numbers that size a library entry of `kind`.
pub fn keys(kind: &str) -> Option<&'static [&'static str]> {
    Some(match Kind::of(kind)? {
        Kind::VBlock => &[
            "angle",
            "width",
            "height",
            "length",
            "v_width",
            "clamp_width",
            "clamp_height",
        ],
        Kind::SoftJaws => &[
            "jaw_width",
            "jaw_height",
            "jaw_thickness",
            "max_opening",
            "bore",
        ],
        Kind::Collet => &["er", "block", "height"],
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
/// What is wrong with a library entry of a round kind beyond its numbers
/// being positive (`Holder::issues`): a V's angle, a V wider than its block
/// or deeper than it, a bore that leaves no jaw behind it, an ER size there
/// is no collet of.
pub fn holder_issues(holder: &Holder) -> Vec<String> {
    let n = |k| num(&holder.p, k).filter(|v| *v > 0.);
    let mut e = vec![];
    match Kind::of(holder.kind()) {
        Some(Kind::VBlock) => {
            if n("angle").is_some_and(|a| !(30. ..=150.).contains(&a)) {
                e.push("The V's angle must be between 30° and 150°.".into());
            }
            if n("v_width").zip(n("width")).is_some_and(|(v, w)| v >= w) {
                e.push("The V must be narrower than the block.".into());
            }
            if let (Some(a), Some(v), Some(h)) = (n("angle"), n("v_width"), n("height"))
                && v / 2. / (a / 2.).to_radians().tan() >= h
            {
                e.push("The V must be shallower than the block is tall.".into());
            }
        }
        Some(Kind::SoftJaws) => {
            if let (Some(bore), Some(t)) = (n("bore"), n("jaw_thickness"))
                && bore / 2. > t - 2.
            {
                e.push(format!(
                    "The bore must leave 2 mm of each jaw behind it: at most Ø{} mm in {} mm jaws.",
                    mm((2. * (t - 2.)).max(0.)),
                    mm(t)
                ));
            }
            if n("bore")
                .zip(n("jaw_width"))
                .is_some_and(|(bore, w)| bore >= w)
            {
                e.push("The bore must be narrower than the jaws are wide.".into());
            }
        }
        Some(Kind::Collet) if n("er").is_some() && er(holder).is_none() => {
            e.push("The ER size must be 11, 16, 20, 25, 32 or 40.".into());
        }
        _ => {}
    }
    e
}
/// The library's starter V-block, soft jaws and ER32 collet block.
pub fn starters() -> Vec<Holder> {
    let entry = |kind, name, values: &[(&str, &str)]| {
        let mut h = Holder::new(kind, name);
        for (k, v) in values {
            h.p.insert((*k).into(), (*v).into());
        }
        h
    };
    let clamp = V_BLOCK_FORCE.to_string();
    let vise = crate::forces::VISE_FORCE.to_string();
    vec![
        entry(
            V_BLOCK,
            "90° V-block 50 mm with clamp",
            &[
                ("angle", "90"),
                ("width", "50"),
                ("height", "40"),
                ("length", "40"),
                ("v_width", "24"),
                ("clamp_width", "12"),
                ("clamp_height", "8"),
                ("force", &clamp),
            ],
        ),
        entry(
            SOFT_JAWS,
            "Soft jaws bored Ø12",
            &[
                ("jaw_width", "100"),
                ("jaw_height", "30"),
                ("jaw_thickness", "25"),
                ("max_opening", "120"),
                ("bore", "12"),
                ("force", &vise),
            ],
        ),
        entry(
            COLLET_BLOCK,
            "ER32 collet block",
            &[("er", "32"), ("block", "60"), ("height", "50")],
        ),
    ]
}

/// A round holding as a setup places it.
#[derive(Debug, Clone, PartialEq)]
pub struct Round {
    pub holder: String,
    pub kind: Kind,
    /// Whether the bar lies along X; else it stands along Z. A V-block's
    /// lies, a collet's stands, soft jaws' as they are bored (`jaw_bore`).
    pub lying: bool,
    /// Where the V-block or the jaws sit from the bar's middle, mm: along
    /// the bar when it lies, along the jaws (X) when it stands. A collet is
    /// on the bar's axis.
    pub offset: f32,
    /// Soft jaws: how high their top is above the bar's bottom, mm. A
    /// collet: how deep the bar stands in it. Unused for a V-block.
    pub grip: f32,
    /// A collet: the diameter it closes on, when not the bar's own (a hub
    /// turned on it), mm.
    pub diameter: Option<f32>,
    pub gap: f32,
}
/// A number typed into a field: surrounding spaces are ignored.
fn number(text: &str) -> Option<f32> {
    text.trim().parse::<f32>().ok().filter(|v| v.is_finite())
}
/// The round holding of kind `kind` that `setup`'s keys place: `round_offset`
/// (0 when empty), `round_grip` for soft jaws and a collet, `jaw_bore` ("X"
/// or "Z"; empty: as setup's own round bar lies) and `collet_diameter`
/// (empty: the bar's).
pub fn layout(setup: &Setup, kind: Kind, holder: String, gap: f32) -> Result<Round, String> {
    let p = &setup.p;
    let offset = match get(p, "round_offset").trim() {
        "" => 0.,
        text => number(text).ok_or("The holding's offset must be a number.")?,
    };
    let grip = match kind {
        Kind::VBlock => 0.,
        _ => number(get(p, "round_grip"))
            .filter(|g| *g > 0.)
            .ok_or(match kind {
                Kind::Collet => "How deep the bar stands in the collet must be a positive number.",
                _ => "The soft jaws' grip must be a positive number.",
            })?,
    };
    let lying = match kind {
        Kind::VBlock => true,
        Kind::Collet => false,
        Kind::SoftJaws => match get(p, "jaw_bore").trim() {
            "" => crate::stock::round_bar(setup),
            "X" => true,
            "Z" => false,
            _ => return Err("The soft jaws' bore must run along X or Z.".into()),
        },
    };
    let diameter = match (kind, get(p, "collet_diameter").trim()) {
        (Kind::Collet, text) if !text.is_empty() => Some(number(text).filter(|d| *d > 0.).ok_or(
            "The diameter the collet grips must be a positive number, or empty for the bar's.",
        )?),
        _ => None,
    };
    Ok(Round {
        holder,
        kind,
        lying,
        offset,
        grip,
        diameter,
        gap,
    })
}
/// The setup keys that hold the placement of `round`.
pub fn placement(round: &Round) -> Vec<(&'static str, String)> {
    let mut keys = vec![("round_offset", mm(round.offset))];
    if round.kind != Kind::VBlock {
        keys.push(("round_grip", mm(round.grip)));
    }
    if round.kind == Kind::SoftJaws {
        keys.push(("jaw_bore", if round.lying { "X" } else { "Z" }.into()));
    }
    if let Some(d) = round.diameter {
        keys.push(("collet_diameter", mm(d)));
    }
    keys
}
/// The bar a box holds: lying along X or standing along Z, in the middle of
/// the box, as wide as its narrower side across.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub lying: bool,
    /// Its axis across: Y and Z when it lies, X and Y when it stands.
    pub axis: [f32; 2],
    pub radius: f32,
    /// Its ends along its axis.
    pub ends: [f32; 2],
    /// How far its box's two sides across differ, mm: 0 for a round bar's.
    pub skew: f32,
}
impl Bar {
    pub fn of(lying: bool, stock: Bounds) -> Self {
        let (a, b, along) = if lying { (1, 2, 0) } else { (0, 1, 2) };
        let d = stock.dimensions();
        Self {
            lying,
            axis: [
                (stock.min[a] + stock.max[a]) / 2.,
                (stock.min[b] + stock.max[b]) / 2.,
            ],
            radius: d[a].min(d[b]) / 2.,
            ends: [stock.min[along], stock.max[along]],
            skew: (d[a] - d[b]).abs(),
        }
    }
    pub fn diameter(self) -> f32 {
        2. * self.radius
    }
    fn length(self) -> f32 {
        self.ends[1] - self.ends[0]
    }
    fn middle(self) -> f32 {
        (self.ends[0] + self.ends[1]) / 2.
    }
    /// The bar's lowest point, mm: its side when it lies, its end when it
    /// stands.
    pub fn bottom(self) -> f32 {
        if self.lying {
            self.axis[1] - self.radius
        } else {
            self.ends[0]
        }
    }
}
/// A V-block's V, from its entry: its half angle (radians), its width at
/// the top and the block's width, height and length.
struct Vee {
    half: f32,
    v: f32,
    width: f32,
    height: f32,
    length: f32,
}
impl Vee {
    fn of(holder: &Holder) -> Self {
        let n = |k| num(&holder.p, k).unwrap_or(0.);
        Self {
            half: (n("angle") / 2.).to_radians(),
            v: n("v_width"),
            width: n("width"),
            height: n("height"),
            length: n("length"),
        }
    }
    /// For a bar of `radius` on axis height `z`: the V's apex, where the
    /// bar touches the flanks (half across, height) and the block's top.
    fn seat(&self, radius: f32, z: f32) -> (f32, [f32; 2], f32) {
        let apex = z - radius / self.half.sin();
        let touch = [radius * self.half.cos(), z - radius * self.half.sin()];
        (apex, touch, apex + self.v / 2. / self.half.tan())
    }
    /// The largest bar diameter the V seats: its flanks' contact lines at
    /// least 0.5 mm inside the V's edges. A smaller bar seats lower, the
    /// clamp's screw reaching down to it.
    fn takes(&self) -> f32 {
        2. * (self.v / 2. - 0.5) / self.half.cos()
    }
}
/// What is wrong with `round` holding `stock`, the box the setup starts
/// from (mm, its frame), with `holder`'s sizes: the stock not a bar the way
/// it holds one; a bar too big for the V, or a V-block off the bar; soft
/// jaws bored to another diameter, their top not over the bar's axis or
/// not below its top, or off the bar; a collet that does not close on the
/// diameter it grips, a grip deeper than the collet, or nothing above it.
pub fn issues(round: &Round, holder: &Holder, stock: Bounds) -> Vec<String> {
    let bar = Bar::of(round.lying, stock);
    let d = bar.diameter();
    let mut e = vec![];
    if bar.skew > 0.01 {
        e.push(if round.lying {
            format!(
                "The {} holds a round bar lying along X, and this setup's stock is {} × {} mm across it. Make setup 1's stock a Round bar, lying along X, or turn this setup so that the bar lies along X.",
                round.kind.words(),
                mm(stock.dimensions()[1]),
                mm(stock.dimensions()[2])
            )
        } else {
            format!(
                "The {} holds a round bar standing along Z, and this setup's stock is {} × {} mm across it. Make setup 1's stock a Round bar standing along Z, or turn this setup so that the bar stands.",
                round.kind.words(),
                mm(stock.dimensions()[0]),
                mm(stock.dimensions()[1])
            )
        });
        return e;
    }
    let n = |k| num(&holder.p, k).unwrap_or(0.);
    match round.kind {
        Kind::VBlock => {
            let most = Vee::of(holder).takes();
            if d > most + 0.005 {
                e.push(format!(
                    "The Ø{} mm bar is too big for {}: it would sit on the edges of its {} mm V, which seats bars up to Ø{} mm. Choose a larger V-block.",
                    mm(d),
                    holder.name().trim(),
                    mm(n("v_width")),
                    mm(most)
                ));
            }
            if round.offset.abs() >= bar.length() / 2. {
                e.push(format!(
                    "The V-block's offset of {} mm puts it beyond the bar's end. Keep it under {} mm either way.",
                    mm(round.offset),
                    mm(bar.length() / 2.)
                ));
            }
        }
        Kind::SoftJaws => {
            let bore = n("bore");
            if (bore - d).abs() > BORE_FIT + 0.005 {
                e.push(format!(
                    "{} are bored to Ø{} mm and the bar is Ø{} mm: bore them to the bar's diameter, within {} mm.",
                    holder.name().trim(),
                    mm(bore),
                    mm(d),
                    mm(BORE_FIT)
                ));
            }
            if round.lying {
                if round.grip <= bar.radius {
                    e.push(format!(
                        "The soft jaws' top, {} mm above the bar's bottom, is not above its axis: they could not hold it down. Grip more than {} mm, half its diameter.",
                        mm(round.grip),
                        mm(bar.radius)
                    ));
                } else if d - round.grip < 0.5 - 0.005 {
                    e.push(format!(
                        "The soft jaws grip {} mm of the Ø{} mm bar, which leaves less than 0.5 mm of it above them.",
                        mm(round.grip),
                        mm(d)
                    ));
                }
            } else if bar.length() - round.grip < 1. - 0.005 {
                e.push(format!(
                    "The stock is {} mm tall; the soft jaws grip {} mm of it, which leaves less than 1 mm above them.",
                    mm(bar.length()),
                    mm(round.grip)
                ));
            }
            let across = if round.lying { bar.length() } else { d };
            let reach = across / 2. + n("jaw_width") / 2.;
            if round.offset.abs() >= reach {
                e.push(format!(
                    "The soft jaws' offset of {} mm puts them beside the bar, not on it. Keep it under {} mm either way.",
                    mm(round.offset),
                    mm(reach)
                ));
            }
        }
        Kind::Collet => {
            let Some((size, least, most, length, ..)) = er(holder) else {
                return e;
            };
            let gripped = round.diameter.unwrap_or(d);
            if gripped > d + 0.005 {
                e.push(format!(
                    "The collet grips Ø{} mm, wider than the Ø{} mm bar.",
                    mm(gripped),
                    mm(d)
                ));
            } else if gripped > most + 0.005 || gripped < least - 0.005 {
                e.push(format!(
                    "An ER{size} collet closes on Ø{} to Ø{} mm, and it would grip Ø{} mm here. Choose a collet block that takes it{}.",
                    mm(least),
                    mm(most),
                    mm(gripped),
                    if gripped > most { ", a larger one" } else { "" }
                ));
            }
            if round.grip > length + 0.005 {
                e.push(format!(
                    "The bar stands {} mm deep in the collet, which is {} mm long.",
                    mm(round.grip),
                    mm(length)
                ));
            }
            if bar.length() - round.grip < 1. - 0.005 {
                e.push(format!(
                    "The stock is {} mm tall; the collet grips {} mm of it, which leaves less than 1 mm above it.",
                    mm(bar.length()),
                    mm(round.grip)
                ));
            }
        }
    }
    e
}
/// A solid from its corners, drawn whole (`Shape::Whole`): it stands as
/// it is below the bar.
fn hold_box(label: String, min: [f32; 3], max: [f32; 3]) -> HoldBox {
    HoldBox {
        label,
        bounds: Bounds { min, max },
        shape: Shape::Whole,
    }
}
/// A soft jaw's solid, modelled down to the vise (`DEPTH`), which the 3D
/// view draws from the stock's bottom up as it draws a vise's jaws.
fn jaw_box(label: String, min: [f32; 3], max: [f32; 3]) -> HoldBox {
    HoldBox {
        shape: Shape::Box,
        ..hold_box(label, min, max)
    }
}
/// The solids of `round` round `stock` (the box the setup starts from, mm,
/// its frame), without the clearance gap. A V-block: the block below the V
/// and under the bar, each flank as `STEPS` steps that hold it, the
/// shoulders beside the V up to the block's top, and the clamp's strap over
/// the bar (or over the block, its screw down to a bar seated lower) with
/// its legs down the block's sides. Soft jaws: each jaw beside the bar up
/// to its top, its bore as `STEPS` steps reaching under a lying bar, or the
/// step a standing one stands on. A collet block: the
/// collet's nut round the bar below the grip, and the block under it.
pub fn boxes(round: &Round, holder: &Holder, stock: Bounds) -> Vec<HoldBox> {
    let bar = Bar::of(round.lying, stock);
    let n = |k| num(&holder.p, k).unwrap_or(0.);
    let r = bar.radius;
    let mut out = vec![];
    match round.kind {
        Kind::VBlock => {
            let vee = Vee::of(holder);
            let [y, z] = bar.axis;
            let (apex, touch, top) = vee.seat(r, z);
            let bottom = top - vee.height;
            let centre = bar.middle() + round.offset;
            let x = [centre - vee.length / 2., centre + vee.length / 2.];
            // The clamp's strap crosses the V-block's middle, on the bar's
            // top or on the block's, whichever is higher.
            let strap = [
                centre - n("clamp_width") / 2.,
                centre + n("clamp_width") / 2.,
            ];
            let over = (z + r).max(top);
            out.push(hold_box(
                "V-block under the bar".into(),
                [x[0], y - touch[0], bottom],
                [x[1], y + touch[0], apex],
            ));
            for (sign, side) in [(-1., "-Y"), (1., "+Y")] {
                let at = |across: f32| y + sign * across;
                let span = |a: f32, b: f32| (at(a).min(at(b)), at(a).max(at(b)));
                for i in 0..STEPS {
                    let from = touch[0] + (vee.v / 2. - touch[0]) * i as f32 / STEPS as f32;
                    let to = touch[0] + (vee.v / 2. - touch[0]) * (i + 1) as f32 / STEPS as f32;
                    let (y0, y1) = span(from, to);
                    out.push(hold_box(
                        format!("V-block flank on {side}"),
                        [x[0], y0, bottom],
                        [x[1], y1, apex + to / vee.half.tan()],
                    ));
                }
                let (y0, y1) = span(vee.v / 2., vee.width / 2.);
                out.push(hold_box(
                    format!("V-block on {side}"),
                    [x[0], y0, bottom],
                    [x[1], y1, top],
                ));
                let leg = n("clamp_height");
                let (y0, y1) = span(vee.width / 2., vee.width / 2. + leg);
                out.push(hold_box(
                    format!("V-block clamp leg on {side}"),
                    [strap[0], y0, bottom],
                    [strap[1], y1, over + leg],
                ));
            }
            let reach = vee.width / 2. + n("clamp_height");
            out.push(hold_box(
                "V-block clamp".into(),
                [strap[0], y - reach, over],
                [strap[1], y + reach, over + n("clamp_height")],
            ));
            // A bar seated below the block's top: the clamp's screw comes
            // down from the strap to its top.
            if over > z + r {
                let half = n("clamp_width") / 2.;
                out.push(hold_box(
                    "V-block clamp screw".into(),
                    [strap[0], y - half, z + r],
                    [strap[1], y + half, over],
                ));
            }
        }
        Kind::SoftJaws => {
            let (t, w) = (n("jaw_thickness"), n("jaw_width"));
            let below = bar.bottom() - DEPTH;
            let top = bar.bottom() + round.grip;
            if round.lying {
                let [y, z] = bar.axis;
                let centre = bar.middle() + round.offset;
                let x = [centre - w / 2., centre + w / 2.];
                // The bore's half width at height `h`: none outside the bar.
                let half = |h: f32| (r * r - (h - z) * (h - z)).max(0.).sqrt();
                for (sign, side) in [(-1., "-Y"), (1., "+Y")] {
                    let span = |a: f32, b: f32| {
                        let (p, q) = (y + sign * a, y + sign * b);
                        (p.min(q), p.max(q))
                    };
                    for i in 0..STEPS {
                        let h0 = bar.bottom() + round.grip * i as f32 / STEPS as f32;
                        let h1 = bar.bottom() + round.grip * (i + 1) as f32 / STEPS as f32;
                        // The narrowest the bore is over the step, at one
                        // of its ends: the jaw reaches in that far.
                        let inner = half(h0).min(half(h1));
                        let (y0, y1) = span(inner, t);
                        // The first step reaches under the bar, where
                        // the bore's bottom is.
                        out.push(jaw_box(
                            format!("soft jaw on {side}"),
                            [x[0], y0, if i == 0 { below } else { h0 }],
                            [x[1], y1, h1],
                        ));
                    }
                }
            } else {
                let [cx, cy] = bar.axis;
                let centre = cx + round.offset;
                let x = [centre - w / 2., centre + w / 2.];
                // The bore across X at `u`: the jaw's face is that far from
                // the bar's axis in Y; the jaws meet beyond it.
                let half = |u: f32| (r * r - (u - cx) * (u - cx)).max(0.).sqrt();
                let slices = 2 * STEPS;
                for (sign, side) in [(-1., "-Y"), (1., "+Y")] {
                    for i in 0..slices {
                        let u0 = x[0] + w * i as f32 / slices as f32;
                        let u1 = x[0] + w * (i + 1) as f32 / slices as f32;
                        let inner = half(u0).min(half(u1));
                        let (p, q) = (cy + sign * inner, cy + sign * t);
                        out.push(jaw_box(
                            format!("soft jaw on {side}"),
                            [u0, p.min(q), below],
                            [u1, p.max(q), top],
                        ));
                    }
                }
                // What the bar stands on between the jaws: the checks keep
                // a drill through it out; the view has nothing to show.
                out.push(HoldBox {
                    shape: Shape::Hidden,
                    ..hold_box(
                        "soft-jaw step under the stock".into(),
                        [x[0], cy - r, below],
                        [x[1], cy + r, bar.bottom()],
                    )
                });
            }
        }
        Kind::Collet => {
            let Some((_, _, _, _, nut, nut_height, _)) = er(holder) else {
                return out;
            };
            let [cx, cy] = bar.axis;
            let front = bar.bottom() + round.grip;
            let (half, block) = (nut / 2., n("block") / 2.);
            out.push(hold_box(
                "collet nut".into(),
                [cx - half, cy - half, front - nut_height],
                [cx + half, cy + half, front],
            ));
            out.push(hold_box(
                "collet block".into(),
                [cx - block, cy - block, front - nut_height - n("height")],
                [cx + block, cy + block, front - nut_height],
            ));
        }
    }
    out
}
/// `round` and its library entry in a sentence, for the setup sheet and
/// the program header.
pub fn sentence(round: &Round, holder: &Holder) -> String {
    let name = holder.name().trim();
    let gap = mm(round.gap);
    match round.kind {
        Kind::VBlock => format!(
            "{name}, the bar lying along X in its V and clamped on top, offset {} mm along it, gap {gap} mm.",
            mm(round.offset)
        ),
        Kind::SoftJaws if round.lying => format!(
            "{name}, bored along X round the bar lying in them, their top {} mm above its bottom, offset {} mm along it, gap {gap} mm.",
            mm(round.grip),
            mm(round.offset)
        ),
        Kind::SoftJaws => format!(
            "{name}, bored upright round the standing bar, grip {} mm, offset {} mm, gap {gap} mm.",
            mm(round.grip),
            mm(round.offset)
        ),
        Kind::Collet => format!(
            "{name} stood in the vise, the bar {} mm deep in the collet{}, gap {gap} mm.",
            mm(round.grip),
            round
                .diameter
                .map(|d| format!(" on Ø{}", mm(d)))
                .unwrap_or_default()
        ),
    }
}
/// What a round holding resists of a cut: the round-contact model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grip {
    /// The contacts' friction together, N, at the full clamping force:
    /// against a slide along the bar, a turn about it, and sideways.
    pub friction: f32,
    /// A V-block's clamp force, N, against a lift; what a push lifts off it
    /// takes its share of the friction. None for jaws and a collet, whose
    /// friction holds the bar against a lift too.
    pub down: Option<f32>,
    /// A standing bar's grip against a turn out of it: what it resists
    /// with, N·mm, and the height of its top, mm.
    pub couple: Option<(f32, f32)>,
}
/// The friction of soft-jaw arc contact over each jaw's closing force: the
/// arc from `from` to `to` (radians from the jaws' closing direction) under
/// an even pressure presses on the bar (to - from) ÷ (sin to - sin from)
/// times as hard as the jaw closes, π/2 for a half bore.
pub fn arc(from: f32, to: f32) -> f32 {
    let lift = to.sin() - from.sin();
    if lift <= 0. { 1. } else { (to - from) / lift }
}
/// What `round` with `holder`'s sizes and force resists on `stock`, the box
/// the setup starts from (`Grip`). A V-block: the clamp's force F presses the
/// bar into the V's two line contacts, each F ÷ (2 sin(angle/2)), and the
/// strap on top, all with friction. Soft jaws: each jaw's arc contact (`arc`)
/// with the vise's force, from the bar's bottom up to the jaws' top when it
/// lies, round a half bore when it stands, the stand against a turn as a
/// vise's (the force times the grip, or the friction over half the bore when
/// less). A collet: its grip all round, in proportion to how deep the bar
/// stands in it up to `FULL_GRIP` of its length, and a quarter of it times
/// that depth against a turn out of it.
pub fn grip(round: &Round, holder: &Holder, stock: Bounds) -> Grip {
    use crate::forces::{FRICTION, VISE_FORCE};
    let force = |typical: f32| {
        num(&holder.p, "force")
            .filter(|f| *f > 0.)
            .unwrap_or(typical)
    };
    let bar = Bar::of(round.lying, stock);
    let r = bar.radius;
    match round.kind {
        Kind::VBlock => {
            let f = force(V_BLOCK_FORCE);
            let half = Vee::of(holder).half;
            Grip {
                friction: FRICTION * f * (1. + 1. / half.sin()),
                down: Some(f),
                couple: None,
            }
        }
        Kind::SoftJaws => {
            let f = force(VISE_FORCE);
            let half_pi = std::f32::consts::FRAC_PI_2;
            let to = if round.lying {
                ((round.grip - r) / r).clamp(-1., 1.).asin()
            } else {
                half_pi
            };
            Grip {
                friction: 2. * FRICTION * f * arc(-half_pi, to),
                down: None,
                couple: (!round.lying)
                    .then(|| (f * round.grip.min(FRICTION * r), bar.bottom() + round.grip)),
            }
        }
        Kind::Collet => {
            let (length, typical) = er(holder).map_or((1., 0.), |row| (row.3, row.6));
            let f = force(typical) * (round.grip / (FULL_GRIP * length)).min(1.);
            Grip {
                friction: FRICTION * f,
                down: None,
                couple: Some((f * round.grip / 4., bar.bottom() + round.grip)),
            }
        }
    }
}
/// The work zeros on a lying bar's axis, on a machine without a rotary A
/// axis: Y0 Z0 on the axis, X0 at the bar's −X end or midway along it.
/// The top of the bar is the box's "Stock top center" and the like.
pub const ORIGINS: [&str; 2] = ["Bar axis at left end", "Bar axis center"];
/// The work zero of `origin` on `stock` (`ORIGINS`), the box `setup` starts
/// from: None for another origin, and refused unless setup's stock is a bar
/// lying along X.
pub fn datum(setup: &Setup, origin: &str, stock: Bounds) -> Option<Result<[f32; 3], String>> {
    let at = ORIGINS.iter().position(|o| *o == origin)?;
    if setup.machine.has_a() {
        return Some(Err(format!(
            "{origin} is for a round bar lying along X on a machine without a rotary A axis: on this machine choose A axis at stock end."
        )));
    }
    let bar = Bar::of(true, stock);
    if bar.skew > 0.01 {
        return Some(Err(format!(
            "{origin} needs a round bar lying along X: make setup 1's stock a Round bar, lying along X, or choose a zero on the stock box."
        )));
    }
    let x = if at == 0 { bar.ends[0] } else { bar.middle() };
    Some(Ok([x, bar.axis[0], bar.axis[1]]))
}
/// What a stock-based zero's touch-off says of a bar lying along X on a
/// machine without A (`setup`'s, its box `stock`): the box's ±Y sides are
/// the bar's only at its axis height, so they are touched there. None for
/// any other stock.
pub fn side_note(setup: &Setup, stock: Bounds) -> Option<String> {
    let bar = Bar::of(true, stock);
    (crate::stock::round_bar(setup) && !setup.machine.has_a() && bar.skew <= 0.01).then(|| {
        format!(
            "The bar lies along X: touch its -Y and +Y sides at its axis height, {} mm below its top, where they are widest.",
            mm(bar.radius)
        )
    })
}
/// How the operator finds a work zero of `ORIGINS`, for the program header
/// and the Work zero step. None for another work zero.
pub fn zero_words(origin: &str) -> Option<&'static str> {
    match ORIGINS.iter().position(|o| *o == origin)? {
        0 => Some(
            "Touch off the top of the bar and set Z0 one radius below it; Y0 midway between the bar's two sides, touched at its axis height; X0 on its left (-X) end face.",
        ),
        _ => Some(
            "Touch off the top of the bar and set Z0 one radius below it; Y0 midway between the bar's two sides, touched at its axis height; X0 midway between its end faces.",
        ),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::forces::{FRICTION, Push, Way, check};
    use crate::holding::{Gripped, Layout, hits_segments};
    use crate::model::{Document, Machine};
    use std::f32::consts::{FRAC_PI_2, PI};
    /// A Ø`d` × `l` bar lying along X, as a 3-axis "Round bar" lying along X
    /// resolves: centred on X and Y, standing on Z0.
    fn lying(d: f32, l: f32) -> Bounds {
        Bounds {
            min: [-l / 2., -d / 2., 0.],
            max: [l / 2., d / 2., d],
        }
    }
    /// A Ø`d` × `l` bar standing along Z on Z0.
    fn standing(d: f32, l: f32) -> Bounds {
        Bounds {
            min: [-d / 2., -d / 2., 0.],
            max: [d / 2., d / 2., l],
        }
    }
    fn starter(kind: &str) -> Holder {
        starters().into_iter().find(|h| h.kind() == kind).unwrap()
    }
    fn round(kind: Kind, lying: bool, offset: f32, grip: f32) -> Round {
        Round {
            holder: "h".into(),
            kind,
            lying,
            offset,
            grip,
            diameter: None,
            gap: 2.,
        }
    }
    fn close(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-3 * (1. + b.abs()), "{a} != {b}");
    }
    fn find<'a>(boxes: &'a [HoldBox], label: &str) -> Vec<&'a HoldBox> {
        boxes.iter().filter(|b| b.label == label).collect()
    }
    #[test]
    fn the_starters_are_whole_entries_of_their_kinds() {
        for h in starters() {
            assert!(h.issues().is_empty(), "{}: {:?}", h.name(), h.issues());
            assert!(KINDS.contains(&h.kind()));
            assert_eq!(Some(h.keys()), keys(h.kind()));
        }
        assert!(
            crate::holding::refusal(
                &Layout::Round(round(Kind::VBlock, true, 0., 0.)),
                &starter(SOFT_JAWS)
            )
            .is_some_and(|r| r.contains("is a soft jaws, not a v-block"))
        );
    }
    #[test]
    fn an_entry_is_refused_for_a_v_it_cannot_have_a_bore_without_a_jaw_or_an_unknown_er_size() {
        let with = |kind: &str, k: &str, v: &str| {
            let mut h = starter(kind);
            h.p.insert(k.into(), v.into());
            h.issues()
        };
        assert_eq!(
            with(V_BLOCK, "angle", "160"),
            ["The V's angle must be between 30° and 150°."]
        );
        assert_eq!(
            with(V_BLOCK, "v_width", "60"),
            ["The V must be narrower than the block."]
        );
        // A 90° V 24 wide is 12 deep: a 10 mm block is shallower.
        assert_eq!(
            with(V_BLOCK, "height", "10"),
            ["The V must be shallower than the block is tall."]
        );
        assert_eq!(
            with(SOFT_JAWS, "bore", "50"),
            ["The bore must leave 2 mm of each jaw behind it: at most Ø46 mm in 25 mm jaws."]
        );
        assert_eq!(
            with(COLLET_BLOCK, "er", "30"),
            ["The ER size must be 11, 16, 20, 25, 32 or 40."]
        );
        assert_eq!(
            with(COLLET_BLOCK, "block", "0"),
            ["block must be a positive number."]
        );
    }
    #[test]
    fn a_setup_places_its_round_holding_and_the_placement_reads_back() {
        let mut s = Setup::new(Machine::new(3), "G54");
        for (k, v) in [
            ("holding", COLLET_BLOCK),
            ("holder", "h"),
            ("round_grip", "15"),
            ("collet_diameter", "18.6"),
        ] {
            s.p.insert(k.into(), v.into());
        }
        let Ok(Layout::Round(read)) = crate::holding::layout(&s) else {
            panic!("{:?}", crate::holding::layout(&s));
        };
        assert_eq!(read.kind, Kind::Collet);
        assert!(!read.lying);
        assert_eq!((read.grip, read.diameter, read.gap), (15., Some(18.6), 2.));
        let mut again = s.clone();
        crate::holding::place(&mut again, &Layout::Round(read.clone()));
        assert_eq!(crate::holding::layout(&again), Ok(Layout::Round(read)));
        // Soft jaws follow the stock: along X when the bar lies.
        s.p.insert("holding".into(), SOFT_JAWS.into());
        s.p.insert("stock_type".into(), "Round bar".into());
        s.p.insert("stock_axis".into(), "X".into());
        let Ok(Layout::Round(jaws)) = crate::holding::layout(&s) else {
            panic!()
        };
        assert!(jaws.lying);
        s.p.insert("round_grip".into(), "".into());
        assert_eq!(
            crate::holding::layout(&s),
            Err("The soft jaws' grip must be a positive number.".into())
        );
    }
    #[test]
    fn a_v_block_seats_the_bar_on_two_flank_lines_and_clamps_it_on_top() {
        let v = starter(V_BLOCK);
        let stock = lying(12., 78.);
        let r = round(Kind::VBlock, true, 10., 0.);
        assert!(issues(&r, &v, stock).is_empty());
        let boxes = boxes(&r, &v, stock);
        // Axis at Z6, r 6, 90°: the apex 6√2 below it, the block's top 12
        // above the apex, its bottom 40 below that.
        let apex = 6. - 6. * 2f32.sqrt();
        let under = find(&boxes, "V-block under the bar")[0].bounds;
        close(under.max[2], apex);
        close(under.max[1], 6. * (PI / 4.).cos());
        close(under.min[2], apex + 12. - 40.);
        // 40 long, centred 10 along the bar.
        close(under.min[0], -10.);
        close(under.max[0], 30.);
        let shoulder = find(&boxes, "V-block on +Y")[0].bounds;
        close(shoulder.max[2], apex + 12.);
        close(shoulder.min[1], 12.);
        close(shoulder.max[1], 25.);
        // Each flank as steps that rise to the V's edge, never above it.
        let flank = find(&boxes, "V-block flank on -Y");
        assert_eq!(flank.len(), STEPS);
        close(flank[STEPS - 1].bounds.max[2], apex + 12.);
        assert!(
            flank
                .iter()
                .all(|b| b.bounds.max[1] <= -6. * (PI / 4.).cos() + 1e-4)
        );
        // The strap lies on the bar's top over the V-block's middle.
        let strap = find(&boxes, "V-block clamp")[0].bounds;
        close(strap.min[2], 12.);
        close(strap.max[2], 20.);
        close((strap.min[0] + strap.max[0]) / 2., 10.);
        assert!(find(&boxes, "V-block clamp screw").is_empty());
        assert!(boxes.iter().all(|b| b.shape == Shape::Whole));
        // A Ø8 bar seats below the block's top: the strap lies on the block
        // and its screw comes down to the bar.
        let low = super::boxes(&r, &v, lying(8., 78.));
        let apex = 4. - 4. * 2f32.sqrt();
        close(find(&low, "V-block clamp")[0].bounds.min[2], apex + 12.);
        let screw = find(&low, "V-block clamp screw")[0].bounds;
        close(screw.min[2], 8.);
        close(screw.max[2], apex + 12.);
    }
    #[test]
    fn a_v_block_too_small_for_the_bar_is_refused() {
        let v = starter(V_BLOCK);
        let r = round(Kind::VBlock, true, 0., 0.);
        // The 24 mm V seats up to Ø32.53 at 90°: the contact lines 0.5 mm
        // inside its edges.
        let most = Vee::of(&v).takes();
        close(most, 23. * 2f32.sqrt());
        assert_eq!(
            issues(&r, &v, lying(88., 60.)),
            [
                "The Ø88 mm bar is too big for 90° V-block 50 mm with clamp: it would sit on the edges of its 24 mm V, which seats bars up to Ø32.53 mm. Choose a larger V-block."
            ]
        );
        assert!(issues(&r, &v, lying(most - 0.1, 60.)).is_empty());
        assert!(issues(&r, &v, lying(4., 60.)).is_empty());
        assert!(
            issues(&round(Kind::VBlock, true, 40., 0.), &v, lying(12., 78.))[0]
                .contains("beyond the bar's end")
        );
        // A box is no bar.
        let mut boxy = lying(12., 78.);
        boxy.max[2] = 20.;
        assert!(issues(&r, &v, boxy)[0].contains("holds a round bar lying along X"));
    }
    #[test]
    fn soft_jaws_must_be_bored_to_the_bar_and_close_over_its_axis() {
        let mut jaws = starter(SOFT_JAWS);
        let stock = lying(12., 78.);
        assert!(issues(&round(Kind::SoftJaws, true, 0., 9.), &jaws, stock).is_empty());
        assert_eq!(
            issues(&round(Kind::SoftJaws, true, 0., 5.), &jaws, stock),
            [
                "The soft jaws' top, 5 mm above the bar's bottom, is not above its axis: they could not hold it down. Grip more than 6 mm, half its diameter."
            ]
        );
        jaws.p.insert("bore".into(), "24".into());
        assert_eq!(
            issues(&round(Kind::SoftJaws, true, 0., 9.), &jaws, stock),
            [
                "Soft jaws bored Ø12 are bored to Ø24 mm and the bar is Ø12 mm: bore them to the bar's diameter, within 0.5 mm."
            ]
        );
        // The Oldham disc, Ø24 × 9, standing in jaws bored to it.
        let disc = standing(24., 9.);
        assert!(issues(&round(Kind::SoftJaws, false, 0., 5.), &jaws, disc).is_empty());
        assert!(
            issues(&round(Kind::SoftJaws, false, 0., 8.5), &jaws, disc)[0]
                .contains("leaves less than 1 mm above them")
        );
    }
    #[test]
    fn soft_jaws_reach_round_the_bore_up_to_their_top() {
        let jaws = starter(SOFT_JAWS);
        let boxes = boxes(&round(Kind::SoftJaws, true, 0., 9.), &jaws, lying(12., 78.));
        let side = find(&boxes, "soft jaw on +Y");
        assert_eq!(side.len(), STEPS);
        // Up to 9 above the bar's bottom, the jaw's back 25 from the axis.
        close(side.iter().map(|b| b.bounds.max[2]).fold(0., f32::max), 9.);
        assert!(side.iter().all(|b| (b.bounds.max[1] - 25.).abs() < 1e-4));
        // The top step, 6.75 to 9 high, reaches in to the bore at 9: 6 above
        // the axis... 3 above it, √(36 - 9) from the axis.
        let top = side.iter().find(|b| b.bounds.max[2] > 8.9).unwrap();
        close(top.bounds.min[1], 27f32.sqrt());
        // Standing: the jaws close on Y round the disc, up to the grip.
        let disc = boxes_of(
            &round(Kind::SoftJaws, false, 0., 5.),
            &jaws,
            standing(24., 9.),
        );
        assert!(
            find(&disc, "soft jaw on -Y")
                .iter()
                .all(|b| (b.bounds.max[2] - 5.).abs() < 1e-4 && b.bounds.max[1] <= 0.)
        );
        assert_eq!(
            find(&disc, "soft-jaw step under the stock")[0].shape,
            Shape::Hidden
        );
    }
    fn boxes_of(r: &Round, h: &Holder, stock: Bounds) -> Vec<HoldBox> {
        boxes(r, h, stock)
    }
    #[test]
    fn a_collet_must_close_on_what_it_grips() {
        let mut block = starter(COLLET_BLOCK);
        // The clutch rotor by its Ø18.6 hub, 15 deep in an ER32.
        let rotor = standing(76., 30.);
        let mut r = round(Kind::Collet, false, 0., 15.);
        r.diameter = Some(18.6);
        assert!(issues(&r, &block, rotor).is_empty());
        block.p.insert("er".into(), "25".into());
        assert_eq!(
            issues(&r, &block, rotor),
            [
                "An ER25 collet closes on Ø1 to Ø16 mm, and it would grip Ø18.6 mm here. Choose a collet block that takes it, a larger one."
            ]
        );
        block.p.insert("er".into(), "32".into());
        r.grip = 45.;
        assert!(issues(&r, &block, standing(20., 60.))[0].contains("40 mm long"));
        r.diameter = Some(30.);
        r.grip = 15.;
        assert!(issues(&r, &block, standing(20., 60.))[0].contains("wider than the Ø20 mm bar"));
        // The nut's top is the grip above the bar's bottom; the block under it.
        r.diameter = None;
        let solid = boxes(&r, &block, standing(20., 60.));
        let nut = find(&solid, "collet nut")[0].bounds;
        close(nut.max[2], 15.);
        close(nut.max[0] - nut.min[0], 50.);
        let body = find(&solid, "collet block")[0].bounds;
        close(body.max[2], 15. - 22.5);
        close(body.max[2] - body.min[2], 50.);
    }
    #[test]
    fn the_cutter_is_kept_out_of_the_v_block_and_its_clamp() {
        let v = starter(V_BLOCK);
        let stock = lying(12., 78.);
        let boxes = boxes(&round(Kind::VBlock, true, 10., 0.), &v, stock);
        let cut = |from: [f32; 3], to: [f32; 3]| {
            hits_segments(&[(from, to)], 5., 25., 15., &boxes, 2.)
                .into_iter()
                .map(|h| h.label)
                .collect::<Vec<_>>()
        };
        // A 0.6 flat over the bar's -X end, 22 long: clear of the strap at
        // X10 and the block's 40 mm from X-10.
        assert!(cut([-39., -8., 11.4], [-25., 8., 11.4]).is_empty());
        // The same flat run along the bar to X0 meets the strap.
        assert!(cut([-39., 0., 11.4], [5., 0., 11.4]).contains(&"V-block clamp".to_string()));
        // A plunge beside the bar into the V's shoulder.
        let plunge = cut([10., 22., 20.], [10., 22., 0.]);
        assert!(
            plunge.iter().any(|l| l.starts_with("V-block")),
            "{plunge:?}"
        );
        // A drill through the bar beyond the block's end is clear.
        assert!(cut([-30., 0., 20.], [-30., 0., -1.]).is_empty());
    }
    #[test]
    fn the_round_contact_model_holds_by_the_v_lines_the_jaws_arc_and_the_collet_grip() {
        close(arc(-FRAC_PI_2, FRAC_PI_2), FRAC_PI_2);
        close(arc(-FRAC_PI_2, 0.), FRAC_PI_2);
        close(arc(-0.01, 0.01), 1.);
        // A 90° V: the clamp's 3000 N on two lines, each 3000 / (2 sin 45°),
        // and the strap's own: 0.2 × 3000 × (1 + √2).
        let v = starter(V_BLOCK);
        let g = grip(&round(Kind::VBlock, true, 0., 0.), &v, lying(12., 78.));
        close(g.friction, FRICTION * 3000. * (1. + 2f32.sqrt()));
        assert_eq!(g.down, Some(3000.));
        // Soft jaws round a lying bar up to 9 of its 12: each jaw's arc from
        // the bottom to 30° above the axis.
        let jaws = starter(SOFT_JAWS);
        let g = grip(&round(Kind::SoftJaws, true, 0., 9.), &jaws, lying(12., 78.));
        let up = (0.5f32).asin();
        close(
            g.friction,
            2. * FRICTION * 5000. * (FRAC_PI_2 + up) / (1. + up.sin()),
        );
        assert!(g.down.is_none() && g.couple.is_none());
        // An ER32 grips 10 kN over 2/3 of its 40 mm; half as deep, half as hard.
        let block = starter(COLLET_BLOCK);
        let full = grip(
            &round(Kind::Collet, false, 0., 30.),
            &block,
            standing(20., 60.),
        );
        close(full.friction, FRICTION * 10000.);
        let half = grip(
            &round(Kind::Collet, false, 0., 40. / 3.),
            &block,
            standing(20., 60.),
        );
        close(half.friction, FRICTION * 5000.);
        let (couple, top) = half.couple.unwrap();
        close(couple, 5000. * 40. / 3. / 4.);
        close(top, 40. / 3.);
    }
    #[test]
    fn the_force_check_holds_a_firm_grip_and_refuses_a_light_one() {
        let v = starter(V_BLOCK);
        let stock = lying(12., 78.);
        let layout = Layout::Round(round(Kind::VBlock, true, 10., 0.));
        let push = [Push {
            side: 200.,
            up: 60.,
            z: 11.,
        }];
        let m = check(&layout, &v, Gripped::whole(stock), None, &push).unwrap();
        assert!(m.factor >= crate::forces::SAFETY, "{m:?}");
        assert_eq!(m.round, Some(Kind::VBlock));
        // A V-block clamp at 300 N: 0.2 × (300 - 60) × (1 + √2) = 116 N
        // against 200 N sideways.
        let mut light = v.clone();
        light.p.insert("force".into(), "300".into());
        let m = check(&layout, &light, Gripped::whole(stock), None, &push).unwrap();
        assert_eq!(m.way, Way::Slip);
        close(m.factor, FRICTION * 240. * (1. + 2f32.sqrt()) / 200.);
        let said = crate::forces::issue("Flat", &m).unwrap();
        assert!(said.starts_with("Flat: the cut could move the bar in the V-block: about 200 N against 116 N of friction from the V's two line contacts and the clamp (×0.5, under the ×2 wanted)."), "{said}");
        // A push up past the clamp lifts the bar out.
        let lift = [Push {
            side: 0.,
            up: 400.,
            z: 11.,
        }];
        let m = check(&layout, &light, Gripped::whole(stock), None, &lift).unwrap();
        assert_eq!(m.way, Way::Lift);
        assert!(
            crate::forces::issue("Flat", &m)
                .unwrap()
                .contains("lift the bar out of the V-block")
        );
        // A collet barely gripping a tall rotor tips it.
        let block = starter(COLLET_BLOCK);
        let mut held = round(Kind::Collet, false, 0., 4.);
        held.diameter = Some(18.6);
        let tall = [Push {
            side: 150.,
            up: 50.,
            z: 30.,
        }];
        let m = check(
            &Layout::Round(held),
            &block,
            Gripped::whole(standing(76., 30.)),
            None,
            &tall,
        )
        .unwrap();
        assert!(m.factor < crate::forces::SAFETY, "{m:?}");
        assert_eq!(m.way, Way::Tip(None));
        assert!(
            crate::forces::issue("Teeth", &m)
                .unwrap()
                .contains("tip the bar out of the collet")
        );
    }
    #[test]
    fn a_lying_bar_zeroes_on_its_axis_or_its_top() {
        let mut s = Setup::new(Machine::new(3), "G54");
        s.p.insert("stock_type".into(), "Round bar".into());
        s.p.insert("stock_axis".into(), "X".into());
        let bar = lying(12., 78.);
        assert_eq!(datum(&s, "Bar axis center", bar), Some(Ok([0., 0., 6.])));
        assert_eq!(
            datum(&s, "Bar axis at left end", bar),
            Some(Ok([-39., 0., 6.]))
        );
        assert_eq!(datum(&s, "Stock top center", bar), None);
        let mesh =
            crate::mesh::normalize(vec![[[0., 0., 0.], [20., 0., 0.], [0., 20., 10.]]]).unwrap();
        s.p.insert("origin".into(), "Stock top center".into());
        assert_eq!(crate::post::datum(&s, bar, &mesh), Ok([0., 0., 12.]));
        s.p.insert("origin".into(), "Bar axis center".into());
        assert_eq!(crate::post::datum(&s, bar, &mesh), Ok([0., 0., 6.]));
        assert!(
            zero_words("Bar axis center")
                .unwrap()
                .contains("one radius below")
        );
        // A zero on the box touches the bar's sides at its axis height.
        assert_eq!(
            side_note(&s, bar).as_deref(),
            Some(
                "The bar lies along X: touch its -Y and +Y sides at its axis height, 6 mm below its top, where they are widest."
            )
        );
        assert_eq!(side_note(&Setup::new(Machine::new(3), "G54"), bar), None);
        // Not on a box, nor on a machine with A.
        let mut boxy = bar;
        boxy.max[2] = 20.;
        assert!(datum(&s, "Bar axis center", boxy).unwrap().is_err());
        let mut rotary = s.clone();
        rotary.machine = Machine::new(4);
        rotary
            .machine
            .p
            .insert("arrangement".into(), "A table around X".into());
        assert!(datum(&rotary, "Bar axis center", bar).unwrap().is_err());
    }
    #[test]
    fn a_3_axis_round_bar_lies_along_x_only_when_set_and_old_projects_still_stand() {
        let mut s = Setup::new(Machine::new(3), "G54");
        s.p.insert("stock_type".into(), "Round bar".into());
        s.p.insert("stock_diameter".into(), "12".into());
        s.p.insert("stock_length".into(), "78".into());
        // A project saved before LIO-249 has no stock_axis: the bar stands.
        assert!(crate::stock::is_round(&s) && !crate::stock::round_bar(&s));
        let up = crate::stock::resolve(&s, None).unwrap().stock;
        assert_eq!(up, standing(12., 78.));
        s.p.insert("stock_axis".into(), "X".into());
        assert!(!crate::stock::is_round(&s) && crate::stock::round_bar(&s));
        assert_eq!(
            crate::stock::resolve(&s, None).unwrap().stock,
            lying(12., 78.)
        );
        assert_eq!(
            crate::stock::round(&s, lying(12., 78.)).map(|r| (r.axis, r.radius)),
            Some(([0., 6.], 6.))
        );
    }
    #[test]
    fn round_holdings_and_entries_round_trip_and_a_vise_on_a_lying_bar_is_refused() {
        let mut doc = Document::sample();
        doc.adopt_holders();
        doc.workholding.extend(starters());
        let v = starter_in(&doc, V_BLOCK);
        let s = doc.setup_mut();
        for (k, val) in [
            ("stock_type", "Round bar"),
            ("stock_axis", "X"),
            ("stock_diameter", "12"),
            ("stock_length", "78"),
            ("holding", V_BLOCK),
            ("holder", &v),
            ("round_offset", "10"),
        ] {
            s.p.insert(k.into(), val.into());
        }
        let si = doc.index();
        doc.settle_holding(si);
        assert_eq!(doc.holding_issues(si, None), Vec::<String>::new());
        assert!(
            doc.holding_text(doc.setup())
                .unwrap()
                .contains("lying along X in its V")
        );
        let saved = serde_json::to_string(&doc).unwrap();
        let back: Document = serde_json::from_str(&saved).unwrap();
        assert_eq!(back.workholding, doc.workholding);
        assert_eq!(back.setups[si].p, doc.setups[si].p);
        // A vise on the lying bar, or a V-block on a box, is refused.
        let vise = doc
            .workholding
            .iter()
            .find(|h| h.kind() == "Vise")
            .unwrap()
            .id
            .clone();
        let mut on_vise = doc.clone();
        on_vise
            .setup_mut()
            .p
            .insert("holding".into(), "Vise".into());
        on_vise.setup_mut().p.insert("holder".into(), vise);
        assert!(on_vise.holding_issues(si, None)[0].contains("Choose V-block or Soft jaws"));
        let mut on_box = doc.clone();
        on_box
            .setup_mut()
            .p
            .insert("stock_type".into(), "Fixed size box".into());
        assert_eq!(
            on_box.holding_issues(si, None),
            ["A V-block holds a round bar: make setup 1's stock a Round bar, lying along X."]
        );
        // Soft jaws settle at three quarters of the bar's diameter.
        let jaws = starter_in(&doc, SOFT_JAWS);
        let mut on_jaws = doc.clone();
        on_jaws
            .setup_mut()
            .p
            .insert("holding".into(), SOFT_JAWS.into());
        on_jaws.setup_mut().p.insert("holder".into(), jaws);
        on_jaws.settle_holding(si);
        assert_eq!(get(&on_jaws.setup().p, "round_grip"), "9");
        assert!(on_jaws.holding_issues(si, None).is_empty());
    }
    fn starter_in(doc: &Document, kind: &str) -> String {
        doc.workholding
            .iter()
            .find(|h| h.kind() == kind)
            .unwrap()
            .id
            .clone()
    }
}
