//! Workholding: vises and clamps from the library, where they sit in a
//! setup, and whether a toolpath comes too close to them. Toolpaths are
//! never changed; moves that hit are reported.
use crate::material::NAME_LIMIT;
use crate::mesh::{Bounds, Mesh};
use crate::model::{Document, Fields, Setup, fields, get, id, num};
use crate::simulation::Plan;
use serde::{Deserialize, Serialize};

pub const KINDS: [&str; 2] = ["Vise", "Clamp"];
/// A setup's modelled workholding: none, a vise or clamps from the library,
/// or the chuck on the machine's rotary A axis (`chuck`).
pub const CHOICES: [&str; 4] = ["None", "Vise", "Clamps", "Chuck"];
const NOTE: &str = "Measure yours: these are typical sizes.";
/// How far below the stock bottom a vise jaw is modelled to reach.
const JAW_DEPTH: f32 = 100.;
pub const MAX_CLAMPS: usize = 8;
/// What a setup's issues say when the library entry it points at is gone.
pub const DELETED: &str = "The workholding entry was deleted; choose another.";
/// What a later setup's issues say when its frame is turned from setup 1's by
/// other than a multiple of 90 degrees: the stock carried into it is no box
/// the jaws or clamps could sit on.
pub const NOT_SQUARE: &str = "This setup is not square to setup 1, so its vise or clamps cannot be modelled. Choose None, or square the setup.";

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
    /// The numbers that size the entry, by kind.
    pub fn keys(&self) -> &'static [&'static str] {
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
        // Optional: empty takes the typical force (`forces::VISE_FORCE`, `forces::CLAMP_FORCE`).
        if !get(&self.p, "force").trim().is_empty()
            && !num(&self.p, "force").is_some_and(|v| v > 0.)
        {
            e.push(
                "The clamping force must be a positive number, or empty for a typical one.".into(),
            );
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
    let vise_force = crate::forces::VISE_FORCE.to_string();
    let clamp_force = crate::forces::CLAMP_FORCE.to_string();
    let mut v = Holder::new("Vise", "100 mm machine vise");
    for (k, x) in [
        ("jaw_width", "100"),
        ("jaw_height", "30"),
        ("jaw_thickness", "20"),
        ("max_opening", "120"),
        ("grip", "8"),
        ("force", &vise_force),
    ] {
        v.p.insert(k.into(), x.into());
    }
    let mut c = Holder::new("Clamp", "Toe clamp 60 mm");
    for (k, x) in [
        ("length", "60"),
        ("width", "20"),
        ("height", "25"),
        ("bite", "12"),
        ("force", &clamp_force),
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
/// The clamps of a clamp positions text as the editor shows them, one
/// [side, offset] row each, whether or not they read: what was typed stays
/// in its row.
pub fn clamp_rows(text: &str) -> Vec<[String; 2]> {
    text.split(';')
        .filter(|s| !s.trim().is_empty())
        .map(|part| {
            let (side, offset) = part.split_once(':').unwrap_or((part, ""));
            [side.trim().to_string(), offset.trim().to_string()]
        })
        .collect()
}
/// The clamp positions text of editor rows (`clamp_rows`). The separators
/// typed into an offset are dropped, so an edit never adds or splits a clamp.
pub fn rows_text(rows: &[[String; 2]]) -> String {
    rows.iter()
        .map(|[side, offset]| format!("{side}:{}", offset.replace([';', ':'], "")))
        .collect::<Vec<_>>()
        .join(";")
}
/// A setup's fixture note: what it records as holding the stock while it
/// models no vise or clamps. Never checked. None once a vise or clamps are
/// modelled, which then say what holds the stock wherever the setup is
/// described, and when the note is empty or None.
pub fn fixture_note(setup: &Setup) -> Option<&str> {
    let p = &setup.p;
    Some(get(p, "fixture"))
        .filter(|note| matches!(get(p, "holding"), "" | "None") && !matches!(*note, "" | "None"))
}
/// The setup's requested fixture clearance in mm, a note for the operator
/// that nothing checks; the setup sheet and the program header print it,
/// "not set" when the field is blank or not a number.
pub fn requested_clearance(setup: &Setup) -> Option<f32> {
    number(get(&setup.p, "clearance"))
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
        // The chuck is modelled by `chuck`, apart from the library's.
        "" | "None" | "Chuck" => Ok(Layout::None),
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
        turn: None,
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
    /// What the checks keep the tool out of.
    pub bounds: Bounds,
    /// What the 3D view draws for it.
    pub shape: Shape,
}
/// How a holding box is drawn.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Shape {
    /// As its box.
    #[default]
    Box,
    /// As a turned solid (a chuck's), which its box holds.
    Prism(std::sync::Arc<crate::chuck::Prism>),
    /// Not at all: a keep-out the checks alone use.
    Hidden,
}
/// What a setup's workholding holds, in mm in the setup's frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gripped {
    /// The box the setup starts from: clamps bite on its top, and it must
    /// stand above a vise's jaws.
    pub stock: Bounds,
    /// What a vise's jaws close on: `stock` with its ±X and ±Y sides moved
    /// in to the material from its bottom up to the grip, past what earlier
    /// cuts provably removed there (`Bands::at`); `stock` itself for setup
    /// 1, without earlier cuts, for clamps, and wherever the earlier cuts
    /// are not known. Its heights are `stock`'s.
    pub band: Bounds,
    /// The highest the bottom of the material a vise's jaws close on can
    /// lie, which the grip is measured up from: `stock`'s bottom, raised
    /// past the layer earlier cuts of any cutter shape can have removed from
    /// it between the jaws, which the box keeps when they do not prove it
    /// removed whole (`Bands::bottom`). `stock`'s bottom for setup 1,
    /// without earlier cuts, for clamps, and wherever the earlier cuts are
    /// not known.
    pub bottom: f32,
}
/// A box whose jaws close on the whole of it (`Gripped::whole`): setup 1's,
/// or one whose earlier cuts are not known.
impl From<Bounds> for Gripped {
    fn from(stock: Bounds) -> Self {
        Self::whole(stock)
    }
}
impl Gripped {
    /// `stock` whose jaws close on its whole box, from its bottom.
    pub fn whole(stock: Bounds) -> Self {
        Self {
            stock,
            band: stock,
            bottom: stock.min[2],
        }
    }
    /// The box a layout's jaws or clamps sit against: the band for a vise,
    /// the stock for clamps.
    fn on(&self, layout: &Layout) -> Bounds {
        match layout {
            Layout::Vise { .. } => self.band,
            _ => self.stock,
        }
    }
}
/// What tells the band a later setup's vise closes on, at any grip: the
/// cuts the earlier setups made, as `rest::RestStock` takes them for
/// evidence, and the part's bounds, which no band reaches into (the setup's
/// frame, mm).
pub struct Bands {
    rest: crate::rest::RestStock,
    part: Bounds,
}
impl Bands {
    /// The bands of a setup in `frame` whose stock the earlier setups left
    /// as `volume`, the part's bounds being `part`.
    pub fn new(volume: &crate::volume::Volume, frame: crate::volume::Frame, part: Bounds) -> Self {
        Self {
            rest: crate::rest::RestStock::new(volume, frame),
            part,
        }
    }
    /// What a vise placed as `layout`, with jaws as wide as `holder`'s,
    /// closes on in `stock` (`rest::RestStock::band`: the band at its grip,
    /// its sides across the jaws over their reach). `stock` for clamps or no
    /// workholding.
    pub fn at(&self, stock: Bounds, layout: &Layout, holder: &Holder) -> Bounds {
        let Layout::Vise {
            axis_y,
            offset,
            grip,
            ..
        } = layout
        else {
            return stock;
        };
        let width = num(&holder.p, "jaw_width").unwrap_or(0.);
        self.rest.band(
            self.part,
            stock,
            *grip,
            usize::from(*axis_y),
            (*offset, width),
        )
    }
    /// The highest the bottom of what a vise placed as `layout`, with jaws
    /// as wide as `holder`'s, closes on (`band`, from `at`) can lie: the
    /// band's bottom, raised past the layer earlier cuts of any cutter shape
    /// can have removed from it between the jaws, over their reach along
    /// them (`rest::RestStock::bottom`). The band's bottom for clamps or no
    /// workholding, and for jaws beside the band, which `position_issue`
    /// refuses.
    pub fn bottom(&self, band: Bounds, layout: &Layout, holder: &Holder) -> f32 {
        let Layout::Vise { axis_y, offset, .. } = layout else {
            return band.min[2];
        };
        let along = usize::from(!*axis_y);
        let width = num(&holder.p, "jaw_width").unwrap_or(0.);
        let middle = (band.min[along] + band.max[along]) / 2. + offset;
        let mut footprint = band;
        footprint.min[along] = band.min[along].max(middle - width / 2.);
        footprint.max[along] = band.max[along].min(middle + width / 2.);
        if footprint.min[along] >= footprint.max[along] {
            return band.min[2];
        }
        self.rest.bottom(footprint)
    }
}
/// What `layout`, with `holder`'s jaws, holds of `stock`: for a vise, the
/// band it closes on and the highest its bottom can lie as `bands` tells
/// them (`Bands::at`, `Bands::bottom`; the whole box, from its bottom, when
/// they are not known), otherwise the box.
pub fn gripped(layout: &Layout, holder: &Holder, stock: Bounds, bands: Option<&Bands>) -> Gripped {
    match (layout, bands) {
        (Layout::Vise { .. }, Some(bands)) => {
            let band = bands.at(stock, layout, holder);
            Gripped {
                stock,
                band,
                bottom: bands.bottom(band, layout, holder),
            }
        }
        _ => Gripped::whole(stock),
    }
}
/// What `setup`'s vise or clamps, with their entry in `doc`'s library, hold
/// of `stock` (`gripped`): the whole box when its layout does not read or
/// the entry is not there, which `Document::holding_issues` says.
pub fn held_by(doc: &Document, setup: &Setup, stock: Bounds, bands: Option<&Bands>) -> Gripped {
    match (layout(setup), doc.holder(get(&setup.p, "holder"))) {
        (Ok(layout), Some(holder)) => gripped(&layout, holder, stock, bands),
        _ => Gripped::whole(stock),
    }
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
    match format!("{v:.2}")
        .trim_end_matches('0')
        .trim_end_matches('.')
    {
        "-0" => "0".into(),
        text => text.into(),
    }
}
/// The vise's maximum opening is less than the material it closes on is
/// wide across the jaws: the band of `held`, the stock the setup starts from.
pub fn opening_issue(layout: &Layout, holder: &Holder, held: impl Into<Gripped>) -> Option<String> {
    let Layout::Vise { axis_y, .. } = layout else {
        return None;
    };
    let across = held.into().band.dimensions()[usize::from(*axis_y)];
    let opening = num(&holder.p, "max_opening").filter(|v| *v > 0.)?;
    (across > opening + 0.005).then(|| {
        format!(
            "The stock is {} mm across the jaws; this vise opens to {} mm.",
            mm(across),
            mm(opening)
        )
    })
}
/// Where `layout` puts its jaws or clamps against `held`, the stock the setup
/// starts from: the jaws overlap the band they close on along their axis,
/// and each clamp's centre is within its side of the stock. The first that
/// is not.
pub fn position_issue(
    layout: &Layout,
    holder: &Holder,
    held: impl Into<Gripped>,
) -> Option<String> {
    let length = held.into().on(layout).dimensions();
    match layout {
        Layout::None => None,
        Layout::Vise { axis_y, offset, .. } => {
            let along = length[usize::from(!*axis_y)] / 2. + num(&holder.p, "jaw_width")? / 2.;
            (offset.abs() >= along).then(|| {
                format!(
                    "The vise offset of {} mm puts the jaws beside the stock, not on it. Keep it under {} mm either way.",
                    mm(*offset),
                    mm(along)
                )
            })
        }
        Layout::Clamps { clamps, .. } => clamps.iter().find_map(|&(side, offset)| {
            let side_length = match side {
                Side::MinusX | Side::PlusX => length[1],
                Side::MinusY | Side::PlusY => length[0],
            };
            (offset.abs() > side_length / 2. + 0.005).then(|| {
                format!(
                    "The clamp on {} is at offset {} mm, beyond its {} mm side. Keep the offset within ±{} mm.",
                    side.label(),
                    mm(offset),
                    mm(side_length),
                    mm(side_length / 2.)
                )
            })
        }),
    }
}
/// The jaws or clamps of `layout`, in the frame of `held` (the stock the
/// setup starts from): the jaws on the faces of the band they close on,
/// centred on it along their axis, from far below the stock's bottom up to
/// the grip above the highest the material's bottom can lie
/// (`Gripped::bottom`: the real jaws are never higher); the clamps on the
/// stock's sides and top. Without the clearance gap. Refuses a holder of
/// the other kind or one whose dimensions are not usable (`refusal`).
pub fn boxes(
    layout: &Layout,
    holder: &Holder,
    held: impl Into<Gripped>,
) -> Result<Vec<HoldBox>, String> {
    if let Some(refused) = refusal(layout, holder) {
        return Err(refused);
    }
    let held = held.into();
    let stock = held.on(layout);
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
            let (z0, z1) = (stock.min[2] - JAW_DEPTH, held.bottom + grip);
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
                        shape: Default::default(),
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
                        shape: Default::default(),
                        label: format!("clamp on {} (offset {})", side.label(), mm(offset)),
                        bounds: Bounds { min, max },
                    }
                })
                .collect()
        }
    })
}
/// The fixture pins a setup sits on (`post::on_pins`), as holding boxes
/// round `stock`, the box it starts from: a pin-sized box per pin from the
/// stock's bottom up to the pin's height, so that the 3D view draws them
/// and a move into one is refused as a jaw's is. They stand in holes in the
/// part; the vise or clamps still hold it down. None without pins.
pub fn pin_boxes(on: Option<&crate::post::OnPins>, stock: Bounds) -> Vec<HoldBox> {
    let Some(on) = on else {
        return vec![];
    };
    let r = on.diameter / 2.;
    (on.places.iter().enumerate())
        .map(|(i, c)| HoldBox {
            shape: Default::default(),
            label: format!("fixture pin {}", i + 1),
            bounds: Bounds {
                min: [c[0] - r, c[1] - r, stock.min[2]],
                max: [c[0] + r, c[1] + r, stock.min[2] + on.height],
            },
        })
        .collect()
}
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub label: String,
    /// "tool" (cutter and shank, tip to stickout) or "holder" (above the
    /// stickout); on the rotary, also "spindle nose" (`chuck::InSpindle`).
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
/// A part of a tool that can touch workholding: (name, radius, lowest and
/// highest height above the tip), a vertical cylinder.
pub type Part = (&'static str, f32, f32, f32);
/// The parts of a tool that can touch workholding. The tool is a vertical
/// cylinder of the cutter `radius` from the tip up by `stickout`, the holder
/// one of `holder_radius` from there upward.
pub fn parts(radius: f32, stickout: f32, holder_radius: f32) -> [Part; 2] {
    [
        ("tool", radius, 0., stickout),
        ("holder", holder_radius, stickout, f32::INFINITY),
    ]
}
/// For a tip moving straight from `from` to `to`, a part of a tool occupying
/// heights `low..=high` above the tip and the box `b` grown by `gap`: the
/// stretch of the move where the part's heights overlap the box, as the least
/// XY distance from it to the box's footprint and the tip where that is first
/// reached. None when the heights never overlap.
fn closest(
    b: &Bounds,
    from: [f64; 3],
    to: [f64; 3],
    low: f32,
    high: f32,
    gap: f32,
) -> Option<(f64, [f64; 3])> {
    let z = (
        f64::from(b.min[2] - gap - high),
        f64::from(b.max[2] + gap - low),
    );
    let tip = |t: f64| [0, 1, 2].map(|k| from[k] + (to[k] - from[k]) * t);
    let (t0, t1) = within(from[2], to[2], z.0, z.1)?;
    let (p, q) = (tip(t0), tip(t1));
    let (distance, s) = xy_closest(b, [p[0], p[1]], [q[0], q[1]]);
    Some((distance, tip(t0 + (t1 - t0) * s)))
}
/// Straight `segments` (tip positions, start to end) whose tool or holder
/// comes within `gap` of a box. Exact: for each segment the stretch where the
/// part's height overlaps the box (grown by `gap`) is found, and the XY
/// distance from that stretch to the box's footprint is compared with the
/// radius plus `gap`. One hit per box and part, the first segment's.
pub fn hits_segments(
    segments: &[([f32; 3], [f32; 3])],
    radius: f32,
    stickout: f32,
    holder_radius: f32,
    boxes: &[HoldBox],
    gap: f32,
) -> Vec<Hit> {
    hits_parts(
        segments,
        &parts(radius, stickout, holder_radius),
        boxes,
        gap,
    )
}
/// `hits_segments` for a tool made of `tool`'s parts.
pub fn hits_parts(
    segments: &[([f32; 3], [f32; 3])],
    tool: &[Part],
    boxes: &[HoldBox],
    gap: f32,
) -> Vec<Hit> {
    let mut out = vec![];
    for b in boxes {
        for &(part, part_radius, low, high) in tool {
            let reach = f64::from(part_radius) + f64::from(gap);
            let found = segments.iter().find_map(|&(from, to)| {
                let (from, to) = (from.map(f64::from), to.map(f64::from));
                let (distance, at) = closest(&b.bounds, from, to, low, high, gap)?;
                (distance <= reach).then(|| at.map(|v| v as f32))
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
/// By how much the moves of `plan` keep clear of the box `b`: for each move
/// and each part of the tool, the distance `hits_segments` finds (XY, from
/// the stretch whose heights overlap the box grown by `gap`, to the box's
/// footprint) less the part's radius and `gap`; the least of them. Positive
/// exactly when there is no hit; infinite when no move reaches the box's
/// heights.
fn margin(plan: &Plan, b: &Bounds, gap: f32) -> f32 {
    let mut least = f64::INFINITY;
    for (_, radius, low, high) in parts(plan.radius, plan.tool_length, plan.holder_radius) {
        let reach = f64::from(radius) + f64::from(gap);
        for m in &plan.moves {
            let (from, to) = (m.from.map(f64::from), m.to.map(f64::from));
            if let Some((distance, _)) = closest(b, from, to, low, high, gap) {
                least = least.min(distance - reach);
            }
        }
    }
    least as f32
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
/// A hit as a problem line: the operation, the part of the tool, the box and
/// where, in work coordinates (`datum` is the work zero in the setup's frame).
pub fn message(operation: &str, hit: &Hit, gap: f32, datum: [f32; 3]) -> String {
    let at = |k: usize| tenth(hit.at[k] - datum[k]);
    format!(
        "{operation}: the {} hits the {} at X{} Y{} Z{} (within the {} mm gap).",
        hit.part,
        hit.label,
        at(0),
        at(1),
        at(2),
        mm(gap)
    )
}
/// `layout` and its library entry in a sentence, for the setup sheet and the
/// program header: what holds the stock, where and with what gap. Empty for
/// no workholding.
pub fn sentence(layout: &Layout, holder: &Holder) -> String {
    let name = holder.name().trim();
    match layout {
        Layout::None => String::new(),
        Layout::Vise {
            axis_y,
            offset,
            grip,
            gap,
            ..
        } => format!(
            "{name} on ±{}, grip {} mm, offset {} mm, gap {} mm.",
            if *axis_y { "Y" } else { "X" },
            mm(*grip),
            mm(*offset),
            mm(*gap)
        ),
        Layout::Clamps { clamps, gap, .. } => format!(
            "{} × {name} at {}, gap {} mm.",
            clamps.len(),
            clamps
                .iter()
                .map(|(side, offset)| format!("{} {}", side.label(), mm(*offset)))
                .collect::<Vec<_>>()
                .join(", "),
            mm(*gap)
        ),
    }
}
/// `v` in work coordinates for a message: to 0.1 mm, never `-0.0`.
fn tenth(v: f32) -> String {
    match format!("{v:.1}").as_str() {
        "-0.0" => "0.0".to_string(),
        text => text.to_string(),
    }
}
/// Why a clearance plane that is not above a box and its gap is refused: the
/// first rapid of a program starts wherever the tool was, at that height.
/// `top` is the box's top, the plane, `datum_z` and `stock_top` are in the
/// setup's frame; the heights are said in work coordinates, and the least
/// clearance as the operation's field takes it, above the stock top. The
/// machine's `blend` tolerance is counted in the heights, not named: the
/// gap help says it is added.
pub fn plane_message(
    label: &str,
    plane: f32,
    gap: f32,
    blend: f32,
    top: f32,
    datum_z: f32,
    stock_top: f32,
) -> String {
    let reach = gap + blend;
    let limit = tenth(top + reach - datum_z);
    format!(
        "The setup clearance plane (Z{}) is not above the {label} and its {} mm gap (Z{limit}). Raise the operations' clearance above {} mm over the stock top (Z{limit}), or retract to machine home.",
        tenth(plane - datum_z),
        mm(gap),
        tenth(top + reach - stock_top)
    )
}
/// The boxes as the 3D view draws them: nothing lower than the bottom of
/// `stock`. The jaws are modelled far below it for the check, which keeps
/// them whole. A chuck's turned solids are drawn whole, and what the checks
/// alone use (`Shape::Hidden`) not at all.
pub fn shown(boxes: &[HoldBox], stock: Bounds) -> Vec<HoldBox> {
    boxes
        .iter()
        .filter(|b| b.shape != Shape::Hidden)
        .map(|b| {
            let mut bounds = b.bounds;
            if b.shape == Shape::Box {
                bounds.min[2] = bounds.min[2].max(stock.min[2]);
            }
            HoldBox {
                shape: b.shape.clone(),
                label: b.label.clone(),
                bounds,
            }
        })
        .collect()
}
/// The least grip a suggested vise is reduced to, in mm.
const MIN_GRIP: f32 = 3.;
/// The most margin that counts when ranking clamp spots, in mm: spots clear
/// by this much are equally good.
const CLEAR_ENOUGH: f32 = 10.;
/// The vise jaws' axis (true: jaws on the Y sides) and grip for the `plans`
/// of a setup whose stock is `stock`, at offset 0, the jaws closing on the
/// band at each grip that `bands` tells (`gripped`; the whole box when the
/// earlier cuts are not known). Each axis tries the holder's grip (at most
/// the stock's height less 1 mm, so that something stands above the jaws,
/// but not cut below 3 mm for that), then 1 mm less at a time down to 3 mm
/// (all in hundredths of a millimetre, as stored), for the first at which no
/// move comes within `gap` of the jaws (`hits`); the axis clear at the
/// larger grip wins. With neither clear the grip is 3 mm and the axis whose
/// jaws keep the larger margin (`margin`: the cutter and the holder both
/// counted) wins. Only the axes whose band at the first grip tried fits the
/// vise's opening across the jaws are tried, when one does. A tie goes to
/// the axis that band is narrower across.
pub fn suggest_vise(
    stock: Bounds,
    bands: Option<&Bands>,
    holder: &Holder,
    plans: &[&Plan],
    gap: f32,
) -> (bool, f32) {
    // Grips in hundredths of a millimetre, as the setup stores them: the holder's, then 1 mm less at a
    // time while above 3 mm, then 3 mm itself. (The 0.05 forgives the f32 of a decimal such as 3.29.)
    let holder_grip = num(&holder.p, "grip").filter(|g| *g > 0.);
    // Something stands above the jaws: at most the stock's height less 1 mm,
    // but a holder's grip is not cut below the least grip for that.
    let below_top = ((f64::from(stock.dimensions()[2]) - 1.) * 100. + 0.05)
        .floor()
        .max(f64::from(MIN_GRIP) * 100.);
    let hundredths = (f64::from(holder_grip.unwrap_or(MIN_GRIP)) * 100. + 0.05)
        .floor()
        .min(below_top)
        .max(1.);
    let start = (hundredths / 100.) as f32;
    let steps = (((hundredths - 300.) / 100.).max(0.).ceil() as usize).saturating_add(1);
    let grip_at = |k: usize| {
        let hundredths = if k + 1 < steps {
            hundredths - 100. * k as f64
        } else {
            hundredths.min(300.)
        };
        (hundredths / 100.) as f32
    };
    let layout = |axis_y, grip| Layout::Vise {
        holder: String::new(),
        axis_y,
        offset: 0.,
        grip,
        gap,
    };
    // The band of each axis and grip, worked out once.
    let found: std::cell::RefCell<Vec<((bool, f32), Gripped)>> = Default::default();
    let held = |axis_y: bool, grip: f32| {
        let key = (axis_y, grip);
        if let Some(&(_, held)) = found.borrow().iter().find(|(k, _)| *k == key) {
            return held;
        }
        let held = gripped(&layout(axis_y, grip), holder, stock, bands);
        found.borrow_mut().push((key, held));
        held
    };
    let jaws =
        |axis_y, grip| boxes(&layout(axis_y, grip), holder, held(axis_y, grip)).unwrap_or_default();
    let fits =
        |axis_y| opening_issue(&layout(axis_y, start), holder, held(axis_y, start)).is_none();
    let across = |axis_y: bool| held(axis_y, start).band.dimensions()[usize::from(axis_y)];
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
            .flat_map(|j| plans.iter().map(|plan| margin(plan, &j.bounds, gap)))
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
/// The clamps for the `plans` of a setup whose stock is `stock`: one on each
/// side, or on two opposite sides when the stock is shorter than 3 clamp
/// widths along both axes. The candidates are the middle and the quarter
/// points of each side; a candidate's score is its `margin` over the moves of
/// all plans (the cutter and the holder both counted), at most
/// `CLEAR_ENOUGH`, so that spots clear by that much tie and the middle wins.
/// Each side takes its best candidate; for two clamps the pair of opposite
/// sides (-X and +X, then -Y and +Y) whose worse clamp scores better is kept,
/// the first on a tie. In the order of `Side::ALL`.
pub fn suggest_clamps(
    stock: Bounds,
    holder: &Holder,
    plans: &[&Plan],
    gap: f32,
) -> Vec<(Side, f32)> {
    let [x, y, _] = stock.dimensions();
    let width = num(&holder.p, "width").unwrap_or(0.);
    let score = |side, offset| {
        let layout = Layout::Clamps {
            holder: String::new(),
            clamps: vec![(side, offset)],
            gap,
        };
        let placed = boxes(&layout, holder, Gripped::whole(stock)).unwrap_or_default();
        placed
            .iter()
            .flat_map(|c| plans.iter().map(|plan| margin(plan, &c.bounds, gap)))
            .fold(CLEAR_ENOUGH, f32::min)
    };
    let best = Side::ALL.map(|side| {
        let length = if matches!(side, Side::MinusX | Side::PlusX) {
            y
        } else {
            x
        };
        let quarter = (length / 4. * 100.).round() / 100.;
        [0., -quarter, quarter]
            .into_iter()
            .map(|offset| (side, offset, score(side, offset)))
            .fold(None::<(Side, f32, f32)>, |best, c| match best {
                Some(b) if b.2 >= c.2 => Some(b),
                _ => Some(c),
            })
            .unwrap_or((side, 0., CLEAR_ENOUGH))
    });
    let chosen = if x < 3. * width && y < 3. * width {
        let worst =
            |pair: &[(Side, f32, f32)]| pair.iter().map(|c| c.2).fold(f32::INFINITY, f32::min);
        let (x_sides, y_sides) = best.split_at(2);
        if worst(y_sides) > worst(x_sides) {
            y_sides
        } else {
            x_sides
        }
    } else {
        &best[..]
    };
    chosen.iter().map(|c| (c.0, c.1)).collect()
}
/// What suggesting a layout for a setup starts from: the kind of workholding
/// it has, the library entry it points at, its clearance gap and how far
/// the moves are kept from the jaws or clamps.
pub struct Request {
    pub vise: bool,
    /// The setup's `holder` key: the entry's id.
    pub id: String,
    pub holder: Holder,
    /// The setup's clearance gap, which the layout stores.
    pub gap: f32,
    /// How far the suggestion keeps the moves from the jaws or clamps: the
    /// gap and the machine profile's path blending tolerance, as the export
    /// checks them (`post_check::holding_problems`), so that what it says is
    /// clear is not refused there.
    pub reach: f32,
}
/// The `Request` for `setup`, whose library entry is `holder` (the one its
/// `holder` key names, when the library has it), or why no layout can be
/// suggested, as the status line says it.
pub fn request(setup: &Setup, holder: Option<&Holder>) -> Result<Request, String> {
    let (kind, vise) = match get(&setup.p, "holding") {
        "Vise" => ("vise", true),
        "Clamps" => ("clamp", false),
        _ => return Err("Choose a vise or clamps for this setup first.".into()),
    };
    let gap = gap(setup)?;
    let id = get(&setup.p, "holder").to_string();
    let Some(holder) = holder else {
        return Err(if id.is_empty() {
            format!("Choose a {kind} from the library first.")
        } else {
            DELETED.into()
        });
    };
    let probe = if vise {
        Layout::Vise {
            holder: id.clone(),
            axis_y: false,
            offset: 0.,
            grip: 1.,
            gap,
        }
    } else {
        Layout::Clamps {
            holder: id.clone(),
            clamps: vec![],
            gap,
        }
    };
    if let Some(refused) = refusal(&probe, holder) {
        return Err(refused);
    }
    Ok(Request {
        vise,
        id,
        holder: holder.clone(),
        gap,
        reach: gap + crate::post::blend(&setup.machine),
    })
}
impl Request {
    /// The layout for setup `si` of `doc` whose operations make `plans`
    /// (`suggest_vise`, `suggest_clamps`), with the stock it starts from: the
    /// first plan's, else the setup's own (`Document::setup_stock`, which needs
    /// the part `mesh` for a later setup). A vise closes on the bands `bands`
    /// tells (None: the whole box). None when that stock is not known.
    pub fn suggest(
        &self,
        doc: &Document,
        mesh: Option<&Mesh>,
        si: usize,
        plans: &[&Plan],
        bands: Option<&Bands>,
    ) -> Option<(Bounds, Layout)> {
        self.suggest_on(plans, || doc.setup_stock(si, mesh), bands)
    }
    /// `suggest`, the stock being the first plan's, else the one `otherwise`
    /// tells (the box the setup starts from as far as the caller knows it).
    /// The jaws or clamps are placed clear of the moves by `reach`, and the
    /// layout stores the setup's `gap`.
    pub fn suggest_on(
        &self,
        plans: &[&Plan],
        otherwise: impl FnOnce() -> Option<Bounds>,
        bands: Option<&Bands>,
    ) -> Option<(Bounds, Layout)> {
        let stock = plans.first().map(|plan| plan.stock).or_else(otherwise)?;
        let (holder, gap, reach) = (self.id.clone(), self.gap, self.reach);
        let layout = if self.vise {
            let (axis_y, grip) = suggest_vise(stock, bands, &self.holder, plans, reach);
            Layout::Vise {
                holder,
                axis_y,
                offset: 0.,
                grip,
                gap,
            }
        } else {
            Layout::Clamps {
                holder,
                clamps: suggest_clamps(stock, &self.holder, plans, reach),
                gap,
            }
        };
        Some((stock, layout))
    }
}
/// One setup's generated operations (a plan each, None for one that did not
/// generate) as a suggestion takes them: the plans there are, and how many
/// operations are left out.
pub fn gathered<'a>(
    operations: impl IntoIterator<Item = Option<&'a Plan>>,
) -> (Vec<&'a Plan>, usize) {
    let (mut plans, mut failed) = (vec![], 0);
    for plan in operations {
        match plan {
            Some(plan) => plans.push(plan),
            None => failed += 1,
        }
    }
    (plans, failed)
}
/// Whether `setup` models a vise or clamps: the jaws or clamps are checked
/// as boxes, which the texts about what is checked say only of such setups.
pub fn modelled(setup: &Setup) -> bool {
    matches!(get(&setup.p, "holding"), "Vise" | "Clamps")
}
/// How many of a job's setups model their workholding (`modelled`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    /// None of them, or there are no setups.
    No,
    Mixed,
    Every,
}
pub fn held<'a>(setups: impl IntoIterator<Item = &'a Setup>) -> Held {
    held_of(setups.into_iter().map(modelled))
}
/// `held` of setups that are, or are not, modelled as `flags` says.
pub fn held_of(flags: impl IntoIterator<Item = bool>) -> Held {
    let (mut any, mut all) = (false, true);
    for flag in flags {
        any |= flag;
        all &= flag;
    }
    match (any, all) {
        (false, _) => Held::No,
        (true, true) => Held::Every,
        _ => Held::Mixed,
    }
}
/// Whether a setup that holds its stock with a vise or clamps says where:
/// a vise has its axis (the grip alone is no placement: choosing a vise
/// fills it in from the library entry), clamps their positions. True for no
/// workholding.
pub fn placed(setup: &Setup) -> bool {
    let key = match get(&setup.p, "holding") {
        "Vise" => "vise_axis",
        "Clamps" => "clamps",
        _ => return true,
    };
    !get(&setup.p, key).trim().is_empty()
}
/// Writes the placement of `layout` on `setup`; whether anything changed.
pub fn place(setup: &mut Setup, layout: &Layout) -> bool {
    let placement = placement(layout);
    let changed = placement
        .iter()
        .any(|(key, value)| get(&setup.p, key) != value);
    for (key, value) in placement {
        setup.p.insert(key.into(), value);
    }
    changed
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
                .map(|(side, offset)| format!("{} at {}", side.label(), mm(*offset)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
/// The status line for a suggested `layout`: where it puts the jaws or
/// clamps, how many of the `considered` operations come within the gap
/// (`clashing`) and how many `left_out` could not be generated. Without
/// operations only the stock decided, and it says so. An `opening` issue, a
/// stock too wide for the vise on either axis, is added.
pub fn summary(
    layout: &Layout,
    considered: usize,
    clashing: usize,
    left_out: usize,
    opening: Option<&str>,
) -> String {
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
    let opening = opening.map(|issue| format!(" {issue}")).unwrap_or_default();
    format!(
        "Suggested: {}. {verdict}{opening}{skipped}",
        describe(layout)
    )
}
/// Why a vise with `layout` grips too much of `stock` (the setup's starting
/// box, mm): it is not taller than the grip by 1 mm, so nothing stands
/// above the jaws. None for clamps or no workholding.
pub fn height_issue(layout: &Layout, stock: Bounds) -> Option<String> {
    let Layout::Vise { grip, .. } = layout else {
        return None;
    };
    let height = stock.dimensions()[2];
    (height - grip < 1. - 0.005).then(|| {
        format!(
            "The stock is {} mm tall; the vise grips {} mm of it, which leaves less than 1 mm above the jaws.",
            mm(height),
            mm(*grip)
        )
    })
}
/// How the holding checks name operation `operation` of setup `setup` of
/// `doc` (both from 0, as `stock_surface::Cut::source` stores them): its
/// strategy and its name, "Contour \"Outside\"". Empty when `doc` has no
/// such operation.
pub fn operation_name(doc: &Document, setup: u16, operation: u16) -> String {
    let (s, o) = (usize::from(setup), usize::from(operation));
    doc.setups
        .get(s)
        .and_then(|x| x.operations.get(o))
        .map(|op| format!("{} \"{}\"", op.strategy.name(), op.name().trim()))
        .unwrap_or_default()
}
/// How far inside the stock box the holding checks look for material, mm:
/// just inside the face a jaw, the table or a clamp touches.
pub(crate) const INSIDE: f32 = 0.05;
/// `n` values spread evenly over `lo..=hi`, the ends inset by 1 % of the span.
fn spread(lo: f32, hi: f32, n: usize) -> Vec<f32> {
    let inset = (hi - lo) * 0.01;
    let (a, b) = (lo + inset, hi - inset);
    (0..n)
        .map(|i| match n {
            1 => (a + b) / 2.,
            _ => a + (b - a) * i as f32 / (n - 1) as f32,
        })
        .collect()
}
/// The points of a rectangle on a face of the stock: `n` × `m` points
/// spread over `first` and `second` (ranges on the axes `axes`), the third
/// coordinate `at`. None when either range is empty.
pub(crate) fn grid(
    axes: [usize; 3],
    first: (f32, f32),
    second: (f32, f32),
    at: f32,
    (n, m): (usize, usize),
) -> Option<Vec<[f32; 3]>> {
    if first.0 >= first.1 || second.0 >= second.1 {
        return None;
    }
    let mut points = vec![];
    for u in spread(first.0, first.1, n) {
        for v in spread(second.0, second.1, m) {
            let mut p = [0.; 3];
            (p[axes[0]], p[axes[1]], p[axes[2]]) = (u, v, at);
            points.push(p);
        }
    }
    Some(points)
}
/// `a..b` clipped to `lo..hi`.
pub(crate) fn clip((a, b): (f32, f32), lo: f32, hi: f32) -> (f32, f32) {
    (a.max(lo), b.min(hi))
}
/// The `points` without material: outside `stock` or removed by an earlier
/// cut. Empty when at least `percent` % of them have material.
fn missing(
    points: &[[f32; 3]],
    stock: Bounds,
    removed: &crate::rest::Removed,
    percent: usize,
) -> Vec<[f32; 3]> {
    let inside = |p: &[f32; 3]| (0..3).all(|k| p[k] >= stock.min[k] && p[k] <= stock.max[k]);
    let without: Vec<[f32; 3]> = points
        .iter()
        .filter(|p| !inside(p) || removed.at(**p))
        .copied()
        .collect();
    let with = points.len() - without.len();
    if with * 100 >= points.len() * percent {
        vec![]
    } else {
        without
    }
}
/// The operations whose cuts removed the material at the `failed` samples,
/// with how many each removed: the most first, equal counts by number.
fn blamed(failed: &[[f32; 3]], removed: &crate::rest::Removed) -> Vec<((u16, u16), usize)> {
    let mut counts: Vec<((u16, u16), usize)> = vec![];
    for source in failed.iter().filter_map(|p| removed.by(*p)) {
        match counts.iter_mut().find(|(s, _)| *s == source) {
            Some((_, n)) => *n += 1,
            None => counts.push((source, 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    counts
}
/// What to do about the jaws' band, `band` mm above the stock's bottom, that
/// earlier cuts removed in part (the `failed` samples), by where the band
/// lies in the frame of the setup that cut most of it. At that setup's
/// bottom, material can be left with its `bottom_offset` and finished later.
/// At its top, or on its side (when this setup is turned over or onto its
/// side), the band cannot be cut in the setup that grips it: that setup
/// leaves the sides whole there, or cuts them through the whole band so the
/// jaws close on what stands inside. The bottom when it cannot be told.
fn band_advice(failed: &[[f32; 3]], removed: &crate::rest::Removed, band: &str) -> String {
    let source = blamed(failed, removed).first().map(|(source, _)| *source);
    match (removed.up_in(source), source) {
        (Some(up), Some((earlier, _))) if up < 0.5 => format!(
            "Leave at least {band} mm of those sides uncut in setup {}, or cut them through all {band} mm there, or choose another holder.",
            earlier + 1
        ),
        _ => format!(
            "Leave at least {band} mm there (for example with `bottom_offset`) and finish it in a later setup, or choose another holder."
        ),
    }
}
/// The earlier operations that removed the material at the `failed`
/// samples: those whose cuts removed at least a quarter of them, the most
/// first (the most alone when none does). Said twice: by number, "setup 1,
/// operation 3 and setup 1, operation 2", for the sentence; and by name
/// (`names`), " Setup 1, operation 3 is Contour \"Outside\"; …", to end the
/// message, where a long name clipped from it costs no advice.
fn cause(
    failed: &[[f32; 3]],
    removed: &crate::rest::Removed,
    names: &dyn Fn(u16, u16) -> String,
) -> (String, String) {
    let counts = blamed(failed, removed);
    let mut named: Vec<(u16, u16)> = counts
        .iter()
        .filter(|(_, n)| n * 4 >= failed.len())
        .map(|(source, _)| *source)
        .collect();
    if named.is_empty() {
        named.extend(counts.first().map(|(source, _)| *source));
    }
    let numbered = |(s, o): (u16, u16)| format!("setup {}, operation {}", s + 1, o + 1);
    let who = match named.split_last() {
        None => "earlier setups".into(),
        Some((last, [])) => numbered(*last),
        Some((last, rest)) => format!(
            "{} and {}",
            rest.iter()
                .map(|s| numbered(*s))
                .collect::<Vec<_>>()
                .join(", "),
            numbered(*last)
        ),
    };
    let said: Vec<String> = named
        .iter()
        .filter_map(|&source| {
            let name = names(source.0, source.1);
            (!name.trim().is_empty()).then(|| format!("{} is {name}", numbered(source)))
        })
        .collect();
    // A sentence of its own: its first "setup" capitalised.
    let late = match said.join("; ") {
        text if text.is_empty() => text,
        text => format!(" S{}.", &text[1..]),
    };
    (who, late)
}
/// Whether the material at a jaw's or clamp's contact `points` was cut free
/// from the part: none of them is joined to it (`Removed::joined`) and some
/// hold material cut free. If so, the points between them and the part where
/// earlier cuts removed the stock, found walking from each in 192 steps of
/// `step` mm along `axis`, toward the part, across the stock: what cut it free.
fn freed(
    points: &[[f32; 3]],
    removed: &crate::rest::Removed,
    axis: usize,
    step: f32,
) -> Option<Vec<[f32; 3]>> {
    let states: Vec<Option<bool>> = points.iter().map(|p| removed.joined(*p)).collect();
    if states.contains(&Some(true)) || !states.contains(&Some(false)) {
        return None;
    }
    let mut between = vec![];
    for (p, state) in points.iter().zip(&states) {
        if *state != Some(false) {
            continue;
        }
        let mut q = *p;
        for _ in 0..192 {
            q[axis] += step;
            match removed.joined(q) {
                Some(true) => break,
                Some(false) => {}
                None => between.push(q),
            }
        }
    }
    Some(between)
}
/// The advice of a message that says the part was cut free.
const JOIN: &str = "Leave material joining them (for example a `bottom_offset` above the stock's bottom, or a skin under the part) and cut it free in the last setup.";
/// Why `layout` with `holder` cannot hold `held` (the stock setup `setup`,
/// from 0, starts from, and the band a vise closes on, in mm) given the
/// material earlier setups removed (`removed`, None for setup 1): stock not
/// taller than the grip by 1 mm, too little material at a jaw, under the
/// band, or under a clamp's bite, or (when `removed` knows the part:
/// `Removed::with_part`) a jaw's or clamp's material cut free from the
/// part. Each message names the operations whose cuts removed most of the
/// failed samples, by number in its sentence and, after its advice, by name
/// (`names(setup, operation)` gives "Contour \"Outside\"").
///
/// Material is looked for 0.05 mm inside the faces, at points spread evenly
/// with the ends inset by 1 % of the span: on each jaw's contact face, the
/// band's face, across the jaw's width (on the band) from where the stock
/// stands (`Gripped::bottom`, the box's bottom raised past the layer the
/// earlier cuts can have taken off it) up to the grip above that, at most
/// the stock's top, 12 × 4, at least 60 % of them; on the bottom the stock
/// stands on, under the band, 8 × 8, at least 50 %; under each clamp's
/// bite on the stock's top, 6 along its width × 3 across the bite, at least
/// 60 %. A point has material when it is inside the band above where the
/// stock stands (the stock for clamps) and no earlier cut removed it: what
/// lies below where it stands is no material, not something an earlier
/// operation is blamed for. A jaw or clamp with material enough is cut
/// free when none of its points' voxels is joined to the part (`freed`).
pub fn holdable(
    setup: usize,
    layout: &Layout,
    holder: &Holder,
    held: impl Into<Gripped>,
    removed: Option<&crate::rest::Removed>,
    names: &dyn Fn(u16, u16) -> String,
) -> Vec<String> {
    let held = held.into();
    let placed = match boxes(layout, holder, held) {
        Ok(placed) => placed,
        Err(refused) => return vec![refused],
    };
    let stock = held.stock;
    let mut found: Vec<String> = height_issue(layout, stock).into_iter().collect();
    let Some(removed) = removed else {
        return found;
    };
    let n = setup + 1;
    match layout {
        Layout::None => {}
        Layout::Vise { axis_y, grip, .. } => {
            let (across, along) = if *axis_y { (1, 0) } else { (0, 1) };
            let axis = if *axis_y { "Y" } else { "X" };
            // The jaws close on the band, from far below the stock up to the
            // grip above where it stands: nothing below that is material.
            let mut band = held.band;
            band.min[2] = held.bottom;
            let height = (held.bottom, (held.bottom + grip).min(stock.max[2]));
            if height.1 <= height.0 {
                found.push(format!(
                    "Setup {n} cannot be held in the vise: earlier setups cut away the material between the jaws up to the stock's top, so nothing stands on the table for them to grip. Choose another holder."
                ));
                return found;
            }
            let (mut bare, mut failed) = (vec![], vec![]);
            let (mut loose, mut between) = (vec![], vec![]);
            for jaw in &placed {
                let b = jaw.bounds;
                let plus = b.min[across] >= band.max[across];
                let face = if plus {
                    band.max[across] - INSIDE
                } else {
                    band.min[across] + INSIDE
                };
                let width = clip(
                    (b.min[along], b.max[along]),
                    band.min[along],
                    band.max[along],
                );
                let Some(points) = grid([along, 2, across], width, height, face, (12, 4)) else {
                    continue;
                };
                let sign = if plus { "+" } else { "-" };
                let lost = missing(&points, band, removed, 60);
                if !lost.is_empty() {
                    bare.push(sign);
                    failed.extend(lost);
                    continue;
                }
                // Toward the part: inward from the jaw.
                let step = (band.max[across] - band.min[across]) / 192.;
                let step = if plus { -step } else { step };
                if let Some(cut) = freed(&points, removed, across, step) {
                    loose.push(sign);
                    between.extend(cut);
                }
            }
            let sides = |signs: &[&str]| {
                let sign = if signs.len() == 2 { "±" } else { signs[0] };
                format!("{sign}{axis}")
            };
            if !bare.is_empty() {
                let band = mm(height.1 - height.0);
                let (who, late) = cause(&failed, removed, names);
                let advice = band_advice(&failed, removed, &band);
                found.push(format!(
                    "Setup {n} cannot be held in the vise: {who} removed the material the jaws grip on {} from 0 to {band} mm above the stock's bottom. {advice}{late}",
                    sides(&bare),
                ));
            }
            if !loose.is_empty() {
                let (who, late) = cause(&between, removed, names);
                found.push(format!(
                    "Setup {n} cannot be held in the vise: {who} cut the part free from the material the jaws grip on {}. {JOIN}{late}",
                    sides(&loose),
                ));
            }
            // What rests on the table: the bottom of the material between
            // the jaws, where the stock stands.
            let bottom = grid(
                [0, 1, 2],
                (band.min[0], band.max[0]),
                (band.min[1], band.max[1]),
                held.bottom + INSIDE,
                (8, 8),
            );
            let lost = missing(&bottom.unwrap_or_default(), band, removed, 50);
            if !lost.is_empty() {
                let (who, late) = cause(&lost, removed, names);
                found.push(format!(
                    "Setup {n} cannot be held in the vise: {who} removed most of the stock's bottom it would rest on. Leave at least half of it and cut the rest in a later setup, or choose another holder.{late}"
                ));
            }
        }
        Layout::Clamps { clamps, .. } => {
            let bite = num(&holder.p, "bite").unwrap_or(0.);
            for (&(side, _), placed) in clamps.iter().zip(&placed) {
                let (across, along, plus) = match side {
                    Side::MinusX => (0, 1, false),
                    Side::PlusX => (0, 1, true),
                    Side::MinusY => (1, 0, false),
                    Side::PlusY => (1, 0, true),
                };
                let b = placed.bounds;
                let width = clip(
                    (b.min[along], b.max[along]),
                    stock.min[along],
                    stock.max[along],
                );
                let reach = clip(
                    (b.min[across], b.max[across]),
                    stock.min[across],
                    stock.max[across],
                );
                let points = grid(
                    [along, across, 2],
                    width,
                    reach,
                    stock.max[2] - INSIDE,
                    (6, 3),
                );
                let points = points.unwrap_or_default();
                let lost = missing(&points, stock, removed, 60);
                let label = &placed.label;
                if !lost.is_empty() {
                    let (who, late) = cause(&lost, removed, names);
                    found.push(format!(
                        "Setup {n} cannot be held by the {label}: {who} removed the stock under its bite. Leave the top there, {} mm in from the edge, and finish it in a later setup, move the clamp, or choose another holder.{late}",
                        mm(bite)
                    ));
                    continue;
                }
                // Toward the part: inward from the clamp's side.
                let step = (stock.max[across] - stock.min[across]) / 192.;
                let step = if plus { -step } else { step };
                if let Some(cut) = freed(&points, removed, across, step) {
                    let (who, late) = cause(&cut, removed, names);
                    found.push(format!(
                        "Setup {n} cannot be held by the {label}: {who} cut the part free from the stock under its bite. {JOIN}{late}"
                    ));
                }
            }
        }
    }
    found
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Machine, Setup};
    use crate::plan_check::PartGrid;
    use crate::rest::{Removed, RestStock};
    use crate::simulation::{CutterShape, Motion, Move, Plan};
    use crate::volume::{Frame, Volume};
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
                a: None,
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
        let nine = ["-X:0"; 9].join(";");
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
                    "clamp on -X (offset 0)".to_string(),
                    [852., 290., 140.],
                    [912., 310., 165.]
                ),
                (
                    "clamp on +X (offset 0)".to_string(),
                    [1088., 290., 140.],
                    [1148., 310., 165.]
                ),
                (
                    "clamp on -Y (offset 0)".to_string(),
                    [990., 152., 140.],
                    [1010., 212., 165.]
                ),
                (
                    "clamp on +Y (offset 0)".to_string(),
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
        suggest_vise(stock, None, holder, plans, 2.)
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
        // The two stay on opposite sides: the X pair, hit by a lead along X through the -X middle, is passed
        // over for the Y pair.
        let lead = plan(2., &[([-80., 0., 21.], [-20., 0., 21.])]);
        let chosen = suggest_clamps(square([50., 50.]), &clamp(), &[&lead], 2.);
        assert_eq!(sides_of(&chosen), [Side::MinusY, Side::PlusY]);
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
    fn clamps_count_the_holder_as_the_exact_check_does() {
        // Ø6 tool, stickout 30, holder Ø32, gap 2. A short move at Z 20 beside the -X clamp's middle and two
        // moves at Z 39 along the +-Y sides: the holder, from Z 50, reaches the clamps' Z 40..65.
        let moves = plan(
            3.,
            &[
                ([-20.2, 0., 20.], [-10., 0., 20.]),
                ([-26., 20., 39.], [-26., 30., 39.]),
                ([-26., -20., 39.], [-26., -30., 39.]),
            ],
        );
        let middle = boxes(&clamp_layout(&[(Side::MinusX, 0.)]), &clamp(), stock()).unwrap();
        let held = hits(&moves, &middle, 2.);
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].part, "holder");
        let chosen = suggest_clamps(stock(), &clamp(), &[&moves], 2.);
        assert_eq!(sides_of(&chosen), Side::ALL);
        assert_eq!(chosen[0].1.abs(), 15., "{chosen:?}");
        let placed = boxes(&clamp_layout(&chosen), &clamp(), stock()).unwrap();
        assert!(hits(&moves, &placed, 2.).is_empty(), "{chosen:?}");
    }
    #[test]
    fn two_clamps_take_the_pair_of_sides_that_is_clearer() {
        let small = Bounds {
            min: [-25., -25., 0.],
            max: [25., 25., 20.],
        };
        // A contour 2 mm inside the +X side: both X clamps are hit, the Y clamps are not.
        let contour = plan(3., &[([23., -10., 19.], [23., 10., 19.])]);
        let chosen = suggest_clamps(small, &clamp(), &[&contour], 2.);
        assert_eq!(sides_of(&chosen), [Side::MinusY, Side::PlusY]);
        let placed = boxes(&clamp_layout(&chosen), &clamp(), small).unwrap();
        assert!(hits(&contour, &placed, 2.).is_empty(), "{chosen:?}");
        let x_pair = boxes(
            &clamp_layout(&[(Side::MinusX, 0.), (Side::PlusX, 0.)]),
            &clamp(),
            small,
        )
        .unwrap();
        assert!(!hits(&contour, &x_pair, 2.).is_empty());
        // The same contour near the Y side leaves the X pair.
        let along = plan(3., &[([-10., 23., 19.], [10., 23., 19.])]);
        let chosen = suggest_clamps(small, &clamp(), &[&along], 2.);
        assert_eq!(sides_of(&chosen), [Side::MinusX, Side::PlusX]);
    }
    #[test]
    fn the_vise_fallback_counts_the_holder_too() {
        // Every grip is hit on both axes. A short tool (stickout 3) near the Y jaws has its Ø32 holder reach
        // them (10 mm away, 16 + 2 needed); the Ø6 cutter, 3 mm from the X jaws, is 2 short of the 5 it needs.
        // The holder's miss is the larger, so the X sides are kept, though the stock is narrower across Y.
        let mut short = plan(
            3.,
            &[
                ([-5., 20., 1.], [5., 20., 1.]),
                ([5., -20., 1.], [-5., -20., 1.]),
            ],
        );
        short.tool_length = 3.;
        let cutter = plan(
            3.,
            &[
                ([47., -10., 1.], [47., 10., 1.]),
                ([-47., 10., 1.], [-47., -10., 1.]),
            ],
        );
        assert_eq!(vise_for(stock(), &vise(), &[&short, &cutter]), (false, 3.));
    }
    #[test]
    fn a_spot_clear_by_ten_millimetres_is_as_good_as_any_other() {
        // A short move in the middle of the stock, 1 mm above its top, is 29 mm from the -X clamp's middle
        // and a hair more from its quarter points: all of them clear by more than 10, so the middles stay.
        let middle = plan(2., &[([-5., 0., 40.], [5., 0., 40.])]);
        assert_eq!(
            suggest_clamps(stock(), &clamp(), &[&middle], 2.),
            [
                (Side::MinusX, 0.),
                (Side::PlusX, 0.),
                (Side::MinusY, 0.),
                (Side::PlusY, 0.)
            ]
        );
    }
    #[test]
    fn the_grip_is_a_hundredth_of_a_millimetre_the_setup_can_store() {
        // A grip of 8.456 is tried as 8.45, 7.45, ... The floor at Z 7.457 is cleared by 5.45 (+ gap 2 = 7.45),
        // not by 6.45; stored as "5.45" it is the grip that was checked.
        let mut fine = vise();
        fine.p.insert("grip".into(), "8.456".into());
        let floor = plan(3., &rectangle(47., 26., 7.457));
        assert_eq!(vise_for(stock(), &fine, &[&floor]), (true, 5.45));
        assert_eq!(
            placement(&Layout::Vise {
                holder: String::new(),
                axis_y: true,
                offset: 0.,
                grip: 5.45,
                gap: 2.
            })[2],
            ("vise_grip", "5.45".to_string())
        );
        // 8.1 stays 8.1: the f32 of 8.1 is a hair over, and 0.29 a hair under, their hundredths.
        fine.p.insert("grip".into(), "8.1".into());
        assert_eq!(vise_for(stock(), &fine, &[]), (true, 8.1));
        fine.p.insert("grip".into(), "3.29".into());
        assert_eq!(vise_for(stock(), &fine, &[]), (true, 3.29));
    }
    #[test]
    fn a_positive_margin_is_exactly_no_hit() {
        let mut short = plan(3., &[([-60., 20., 60.], [-30., 20., 60.])]);
        short.tool_length = 3.;
        let plans = [
            plan(3., &[([-60., 0., 41.], [-30., 0., 41.])]),
            plan(3., &[([-60., 17., 41.], [-30., 17., 41.])]),
            plan(3., &[([-60., 0., 100.], [-30., 0., 100.])]),
            plan(3., &[([-100., 0., 100.], [-20., 0., 41.])]),
            plan(0.2, &[([-60., -1500.75, 41.], [-60., 1499.25, 41.])]),
            plan(3., &[([-120., 0., 41.], [-20., 0., 41.])]),
            short,
        ];
        let sets = [
            boxes(&clamp_layout(&[(Side::MinusX, 0.)]), &clamp(), stock()).unwrap(),
            boxes(&vise_layout(true, 0., 2.), &vise(), stock()).unwrap(),
        ];
        for plan in &plans {
            for set in &sets {
                for gap in [0., 2., 5.] {
                    for b in set {
                        let clear = hits(plan, std::slice::from_ref(b), gap).is_empty();
                        let m = margin(plan, &b.bounds, gap);
                        assert_eq!(m > 0., clear, "{} gap {gap}: margin {m}", b.label);
                    }
                }
            }
        }
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
        let say =
            |considered, clashing, left_out| summary(&jaws, considered, clashing, left_out, None);
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
        // A stock too wide for the opening is said after how the operations fare.
        let wide = "The stock is 130 mm across the jaws; this vise opens to 120 mm.";
        assert_eq!(
            summary(&jaws, 6, 0, 0, Some(wide)),
            format!(
                "Suggested: vise jaws on ±Y, grip 8 mm. Every operation is clear of them. {wide}"
            )
        );
        assert_eq!(
            summary(&jaws, 5, 2, 1, Some(wide)),
            format!(
                "Suggested: vise jaws on ±Y, grip 8 mm. 2 operations come within the gap; see the warnings. {wide} 1 operation was not considered because it could not be generated."
            )
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
    #[test]
    fn a_hit_reads_as_an_export_problem_in_work_coordinates() {
        let hit = Hit {
            label: "vise jaw on +Y".into(),
            part: "tool",
            at: [12.04, 51., 13.],
        };
        assert_eq!(
            message("Contour", &hit, 2., [0., 10., 10.]),
            "Contour: the tool hits the vise jaw on +Y at X12.0 Y41.0 Z3.0 (within the 2 mm gap)."
        );
        // The holder, a gap with decimals, and a hair below zero written as 0.0.
        let hit = Hit {
            label: "clamp on -X (offset 0)".into(),
            part: "holder",
            at: [-5., 0.04, 20.],
        };
        assert_eq!(
            message("Pocket 2", &hit, 1.5, [-5., 0.06, 20.]),
            "Pocket 2: the holder hits the clamp on -X (offset 0) at X0.0 Y0.0 Z0.0 (within the 1.5 mm gap)."
        );
    }
    #[test]
    fn the_sentence_names_the_entry_and_where_it_sits() {
        let jaws = |axis_y, offset, grip, gap| Layout::Vise {
            holder: "v".into(),
            axis_y,
            offset,
            grip,
            gap,
        };
        assert_eq!(
            sentence(&jaws(true, 0., 8., 2.), &vise()),
            "100 mm machine vise on ±Y, grip 8 mm, offset 0 mm, gap 2 mm."
        );
        assert_eq!(
            sentence(&jaws(false, -12.5, 3.25, 0.5), &vise()),
            "100 mm machine vise on ±X, grip 3.25 mm, offset -12.5 mm, gap 0.5 mm."
        );
        // A shift that rounds to nothing is not written as -0.
        assert_eq!(
            sentence(&jaws(false, -0.001, 8., 2.), &vise()),
            "100 mm machine vise on ±X, grip 8 mm, offset 0 mm, gap 2 mm."
        );
        let clamps = |clamps: &[(Side, f32)]| Layout::Clamps {
            holder: "c".into(),
            clamps: clamps.to_vec(),
            gap: 2.,
        };
        assert_eq!(
            sentence(
                &clamps(&[
                    (Side::MinusX, 0.),
                    (Side::PlusX, 0.),
                    (Side::MinusY, 0.),
                    (Side::PlusY, 0.)
                ]),
                &clamp()
            ),
            "4 × Toe clamp 60 mm at -X 0, +X 0, -Y 0, +Y 0, gap 2 mm."
        );
        assert_eq!(
            sentence(&clamps(&[(Side::PlusY, 10.5)]), &clamp()),
            "1 × Toe clamp 60 mm at +Y 10.5, gap 2 mm."
        );
        assert_eq!(sentence(&Layout::None, &clamp()), "");
    }
    #[test]
    fn the_view_draws_jaws_down_to_the_stock_bottom_only() {
        let stock = stock();
        let jaws = boxes(&vise_layout(true, 0., 2.), &vise(), stock).unwrap();
        assert_eq!(jaws[0].bounds.min[2], stock.min[2] - 100.);
        let drawn = shown(&jaws, stock);
        assert_eq!(drawn.len(), 2);
        for (whole, cut) in jaws.iter().zip(&drawn) {
            assert_eq!(cut.label, whole.label);
            // Only the depth below the stock is left out.
            assert_eq!(cut.bounds.min[2], stock.min[2]);
            assert_eq!(cut.bounds.max, whole.bounds.max);
            assert_eq!(cut.bounds.min[..2], whole.bounds.min[..2]);
        }
        // Clamps sit on the stock top and are drawn whole.
        let clamps = boxes(
            &clamp_layout(&[(Side::MinusX, 0.), (Side::PlusY, 10.)]),
            &clamp(),
            stock,
        )
        .unwrap();
        assert_eq!(shown(&clamps, stock), clamps);
        // The check keeps the full depth: the same boxes still hit far below.
        let low = plan(3., &[([-40., 40., -50.], [40., 40., -50.])]);
        assert!(!hits(&low, &jaws, 2.).is_empty());
    }
    #[test]
    fn only_a_vise_or_clamps_count_as_modelled_workholding() {
        let with = |holding: &str| setup(&[("holding", holding)]);
        for (holding, modelled_now) in [
            ("Vise", true),
            ("Clamps", true),
            ("None", false),
            ("", false),
            ("Chuck", false),
        ] {
            assert_eq!(modelled(&with(holding)), modelled_now, "{holding}");
        }
        assert!(!modelled(&setup(&[])));
        let (vise, none) = (with("Vise"), with("None"));
        assert_eq!(held([]), Held::No);
        assert_eq!(held([&none, &none]), Held::No);
        assert_eq!(held([&vise]), Held::Every);
        assert_eq!(held([&vise, &with("Clamps")]), Held::Every);
        assert_eq!(held([&vise, &none]), Held::Mixed);
        assert_eq!(held([&none, &vise]), Held::Mixed);
    }
    #[test]
    fn a_setup_is_placed_by_the_key_the_first_vise_or_clamps_writes() {
        let with = |pairs: &[(&str, &str)]| placed(&setup(pairs));
        // No workholding says nothing to place.
        assert!(with(&[]));
        assert!(with(&[("holding", "None")]));
        // The grip alone is not a placement: choosing a vise fills it in
        // from the library entry before the jaws are laid out.
        let grip = [("holding", "Vise"), ("vise_grip", "8")];
        assert!(!with(&grip));
        assert!(!with(&[("holding", "Vise")]));
        assert!(with(&[("holding", "Vise"), ("vise_axis", "Y")]));
        // What `place` writes counts, for a vise and for clamps.
        for layout in [
            Layout::Vise {
                holder: "v".into(),
                axis_y: false,
                offset: 0.,
                grip: 8.,
                gap: 2.,
            },
            Layout::Clamps {
                holder: "c".into(),
                clamps: vec![(Side::MinusX, 0.)],
                gap: 2.,
            },
        ] {
            let mut s = setup(&[(
                "holding",
                if matches!(layout, Layout::Vise { .. }) {
                    "Vise"
                } else {
                    "Clamps"
                },
            )]);
            assert!(!placed(&s));
            assert!(place(&mut s, &layout));
            assert!(placed(&s));
        }
        assert!(!with(&[("holding", "Clamps")]));
        assert!(!with(&[("holding", "Clamps"), ("clamps", "  ")]));
        assert!(with(&[("holding", "Clamps"), ("clamps", "-X:0")]));
    }
    #[test]
    fn a_suggestion_says_its_offsets_like_the_other_texts() {
        let clamps = Layout::Clamps {
            holder: "c".into(),
            clamps: vec![
                (Side::MinusY, 1.234),
                (Side::PlusY, -0.),
                (Side::PlusX, 12.5),
            ],
            gap: 2.,
        };
        assert_eq!(
            describe(&clamps),
            "3 clamps on -Y at 1.23, +Y at 0, +X at 12.5"
        );
        assert!(
            summary(&clamps, 0, 0, 0, None)
                .starts_with("Suggested: 3 clamps on -Y at 1.23, +Y at 0, +X at 12.5.")
        );
    }
    #[test]
    fn a_plane_not_above_the_box_and_its_gap_reads_in_work_coordinates() {
        assert_eq!(
            plane_message("clamp on -X (offset 0)", 62.5, 2., 0., 72.5, 47.5, 47.5),
            "The setup clearance plane (Z15.0) is not above the clamp on -X (offset 0) and its 2 mm gap (Z27.0). Raise the operations' clearance above 27.0 mm over the stock top (Z27.0), or retract to machine home."
        );
        assert_eq!(
            plane_message("vise jaw on +Y", 0., 0.5, 0., 8.43, 0., 0.),
            "The setup clearance plane (Z0.0) is not above the vise jaw on +Y and its 0.5 mm gap (Z8.9). Raise the operations' clearance above 8.9 mm over the stock top (Z8.9), or retract to machine home."
        );
        // The work zero is not always the stock top: 3 mm below it, the
        // height is 3 mm less than the Z the clearance plane has to pass.
        assert_eq!(
            plane_message("clamp on +X (offset 0)", 15., 2., 0., 30., 3., 3. + 7.),
            "The setup clearance plane (Z12.0) is not above the clamp on +X (offset 0) and its 2 mm gap (Z29.0). Raise the operations' clearance above 22.0 mm over the stock top (Z29.0), or retract to machine home."
        );
    }
    #[test]
    fn gathered_and_suggest_take_a_setups_plans_and_fall_back_to_its_stock() {
        let a = plan(5., &[([0., 0., 5.], [10., 0., 5.])]);
        let b = plan(5., &[([0., 0., 5.], [0., 10., 5.])]);
        let (plans, failed) = gathered([Some(&a), None, Some(&b), None, None]);
        assert_eq!((plans.len(), failed), (2, 3));
        assert!(std::ptr::eq(plans[0], &a) && std::ptr::eq(plans[1], &b));
        assert_eq!(gathered([]).0.len() + gathered([]).1, 0);
        let (mut doc, mesh) = crate::post::fixture::job();
        doc.adopt_holders();
        let id = doc
            .workholding
            .iter()
            .find(|h| h.kind() == "Clamp")
            .unwrap()
            .id
            .clone();
        doc.setups[0].p.insert("holding".into(), "Clamps".into());
        doc.setups[0].p.insert("holder".into(), id.clone());
        let request = request(&doc.setups[0], doc.holder(&id)).unwrap();
        // The first plan's stock when there are plans, else the setup's own.
        let (stock, layout) = request.suggest(&doc, Some(&mesh), 0, &[&a], None).unwrap();
        assert_eq!(stock, a.stock);
        assert!(matches!(layout, Layout::Clamps { .. }));
        let (own, _) = request.suggest(&doc, Some(&mesh), 0, &[], None).unwrap();
        assert_eq!(Some(own), doc.setup_stock(0, Some(&mesh)));
        assert_ne!(own, a.stock);
        // Neither: this document has no second setup.
        assert!(request.suggest(&doc, None, 1, &[], None).is_none());
    }
    #[test]
    fn the_suggestion_keeps_the_moves_clear_by_the_blend_the_export_adds_to_the_gap() {
        // A Ø6 cutter 10.5 mm up along all four sides: 2.5 mm above jaws
        // gripping 8 mm, clear of the 2 mm gap, within it and a 1 mm blend.
        let mut skim = plan(3., &rectangle(47., 26., 10.5));
        skim.stock = stock();
        assert_eq!(
            suggest_vise(stock(), None, &vise(), &[&skim], 2.),
            (true, 8.)
        );
        assert_eq!(
            suggest_vise(stock(), None, &vise(), &[&skim], 3.),
            (true, 7.)
        );
        // A setup on a machine profile with G64 P1: the request keeps the
        // moves clear by the gap and the blend, 3 mm, and the layout stores
        // the gap.
        let mut blended = setup(&[("holding", "Vise"), ("holder", "v")]);
        blended.machine.p.insert("post_blend".into(), "1".into());
        let asked = request(&blended, Some(&vise())).unwrap();
        assert_eq!((asked.gap, asked.reach), (2., 3.));
        let (_, layout) = asked.suggest_on(&[&skim], || None, None).unwrap();
        assert_eq!(
            layout,
            Layout::Vise {
                holder: "v".into(),
                axis_y: true,
                offset: 0.,
                grip: 7.,
                gap: 2.,
            }
        );
        // What the status says is clear, the export (the gap and the blend)
        // finds clear too.
        let jaws = boxes(&layout, &vise(), stock()).unwrap();
        assert_eq!(clashes(&[&skim], &jaws, asked.reach), 0);
        assert!(hits(&skim, &jaws, asked.reach).is_empty());
        assert_eq!(
            summary(&layout, 1, clashes(&[&skim], &jaws, asked.reach), 0, None),
            "Suggested: vise jaws on ±Y, grip 7 mm. Every operation is clear of them."
        );
        // Gripping 8 mm, as the gap alone suggested, the export refuses the pass.
        let eight = boxes(&vise_gripping(8.), &vise(), stock()).unwrap();
        assert_eq!(clashes(&[&skim], &eight, asked.gap), 0);
        assert_eq!(clashes(&[&skim], &eight, asked.reach), 1);
        // Clamps are placed by the same reach, and store the gap too.
        let mut blended = setup(&[("holding", "Clamps"), ("holder", "c")]);
        blended.machine.p.insert("post_blend".into(), "1".into());
        let asked = request(&blended, Some(&clamp())).unwrap();
        assert_eq!((asked.gap, asked.reach), (2., 3.));
        let (_, layout) = asked.suggest_on(&[], || Some(stock()), None).unwrap();
        assert!(matches!(layout, Layout::Clamps { gap, .. } if gap == 2.));
        // The default profile's blend, 0.01 mm, counts too.
        let asked = request(
            &setup(&[("holding", "Vise"), ("holder", "v")]),
            Some(&vise()),
        )
        .unwrap();
        assert!((asked.reach - 2.01).abs() < 0.0001);
    }
    /// `stock()` uncut, in the frame of a setup 1 in which it sits as given,
    /// and that frame turned upside down (rx 180).
    fn untouched() -> (Volume, Frame, Frame) {
        let mesh =
            crate::mesh::normalize(vec![[[0., 0., 0.], [20., 0., 0.], [0., 20., 10.]]]).unwrap();
        let frame = Frame::new(&setup(&[]), &mesh);
        let flipped = Frame::new(&setup(&[("rx", "180")]), &mesh);
        (Volume::new(stock(), frame), frame, flipped)
    }
    /// Flat Ø10 passes along X from `x[0]` to `x[1]`, one at each of `ys`
    /// (the tool's centre) and each tip height of `tips`, with `flute` mm of
    /// flute, made by operation `source`.
    fn slot(
        v: &mut Volume,
        f: Frame,
        x: [f32; 2],
        ys: &[f32],
        tips: &[f32],
        flute: f32,
        source: (u16, u16),
    ) {
        for &z in tips {
            for &y in ys {
                v.cut(
                    [x[0], y, z],
                    [x[1], y, z],
                    5.,
                    flute,
                    CutterShape::Flat,
                    f,
                    source,
                );
            }
        }
    }
    /// The earlier operations the tests' cuts come from, named as the export
    /// names them.
    fn names(setup: u16, operation: u16) -> String {
        match (setup, operation) {
            (0, 0) => "Facing \"Rim\"",
            (0, 1) => "Pocket \"Underside\"",
            (0, 2) => "Contour \"Outside\"",
            (0, 3) => "Contour \"Low\"",
            _ => "",
        }
        .into()
    }
    /// The part inside `stock()` (3 mm of stock under it): the leftover box
    /// never reaches into it.
    const PART: Bounds = Bounds {
        min: [-40., -20., 3.],
        max: [40., 20., 37.],
    };
    /// The starter vise on ±Y at `grip`.
    fn vise_gripping(grip: f32) -> Layout {
        Layout::Vise {
            holder: String::new(),
            axis_y: true,
            offset: 0.,
            grip,
            gap: 2.,
        }
    }
    #[test]
    fn jaws_gripping_where_an_outside_contour_went_down_cannot_hold() {
        let (mut v, f, _) = untouched();
        // An outside contour down to 3 mm above the stock's bottom, its passes
        // over the ±Y faces: the 3 mm left under it keeps the stock's width.
        let tips = [3., 8., 13., 18., 23., 28., 33., 38.];
        slot(&mut v, f, [-60., 60.], &[-30., 30.], &tips, 5., (0, 2));
        let left = RestStock::new(&v, f).left(PART);
        assert_eq!(left, stock());
        // The jaws' band keeps that width too: its lower 3 mm are whole.
        let held = gripped(
            &vise_gripping(8.),
            &vise(),
            left,
            Some(&Bands::new(&v, f, PART)),
        );
        assert_eq!(held, Gripped::whole(left));
        let removed = Removed::new(&v, f);
        assert_eq!(
            holdable(1, &vise_gripping(8.), &vise(), held, Some(&removed), &names),
            [
                "Setup 2 cannot be held in the vise: setup 1, operation 3 removed the material the jaws grip on ±Y from 0 to 8 mm above the stock's bottom. Leave at least 8 mm there (for example with `bottom_offset`) and finish it in a later setup, or choose another holder. Setup 1, operation 3 is Contour \"Outside\"."
            ]
        );
        // Gripping only the 3 mm it left, the vise holds.
        assert!(holdable(1, &vise_gripping(3.), &vise(), left, Some(&removed), &names).is_empty());
        // Jaws on ±X grip where it did not go.
        let across_x = Layout::Vise {
            holder: String::new(),
            axis_y: false,
            offset: 0.,
            grip: 8.,
            gap: 2.,
        };
        assert!(holdable(1, &across_x, &vise(), left, Some(&removed), &names).is_empty());
        // Stopped 9 mm above the bottom (a bottom offset of 6 mm), it leaves
        // the jaws' band whole: the vise holds.
        let (mut v, f, _) = untouched();
        let tips = [9., 14., 19., 24., 29., 34., 39.];
        slot(&mut v, f, [-60., 60.], &[-30., 30.], &tips, 5., (0, 2));
        let left = RestStock::new(&v, f).left(PART);
        let removed = Removed::new(&v, f);
        assert!(holdable(1, &vise_gripping(8.), &vise(), left, Some(&removed), &names).is_empty());
    }
    #[test]
    fn a_turned_over_vise_closes_on_the_walls_setup_1s_outside_contour_finished() {
        // The same outside contour of setup 1, down to 3 mm above its bottom,
        // its passes' flutes overlapping as a real cutter's do: it takes 5 mm
        // off each Y side.
        let tips = [3., 8., 13., 18., 23., 28., 33., 38.];
        let (mut v, f, flipped) = untouched();
        slot(&mut v, f, [-60., 60.], &[-30., 30.], &tips, 10., (0, 2));
        // Turned over, setup 2's stock keeps its width, for the 3 mm the
        // contour left at what is now its top...
        let part = f.transfer_bounds(PART, flipped);
        let left = RestStock::new(&v, flipped).left(part);
        assert_eq!(left, v.transformed_bounds(flipped));
        // ...but the jaws grip what was setup 1's top, which the contour cut
        // through: they close on its walls, 50 mm apart (and the 0.01 mm the
        // band keeps short of a cut face), from the stock's bottom up.
        let bands = Bands::new(&v, flipped, part);
        let layout = vise_gripping(8.);
        let held = gripped(&layout, &vise(), left, Some(&bands));
        assert_eq!(held.stock, left);
        assert!(
            (held.band.dimensions()[1] - 50.02).abs() < 0.001
                && held.band.dimensions()[0] == left.dimensions()[0]
                && (held.band.min[2], held.band.max[2]) == (left.min[2], left.max[2]),
            "{held:?}"
        );
        let jaws = boxes(&layout, &vise(), held).unwrap();
        let minus = jaws.iter().find(|j| j.label.contains("-Y")).unwrap();
        let plus = jaws.iter().find(|j| j.label.contains("+Y")).unwrap();
        assert_eq!(
            (minus.bounds.max[1], plus.bounds.min[1]),
            (held.band.min[1], held.band.max[1])
        );
        assert!(jaws.iter().all(|j| j.bounds.max[2] == left.min[2] + 8.));
        // There is material at both jaws and under them: it holds.
        let removed = Removed::new(&v, flipped).with_part(&PartGrid::boxed(&v, PART));
        assert!(holdable(1, &layout, &vise(), held, Some(&removed), &names).is_empty());
        // A vise opening to 55 mm takes the walls, not the whole box.
        let mut narrow = vise();
        narrow.p.insert("max_opening".into(), "55".into());
        assert_eq!(opening_issue(&layout, &narrow, held), None);
        assert_eq!(
            opening_issue(&layout, &narrow, left).as_deref(),
            Some("The stock is 60 mm across the jaws; this vise opens to 55 mm.")
        );
        // Gripping 38 mm, down into the 3 mm the contour left whole (37 mm
        // above the bottom), the band is the box's width again.
        assert_eq!(
            gripped(&vise_gripping(38.), &vise(), left, Some(&bands)).band,
            left
        );
        // The suggestion puts that vise's jaws on the walls, which it opens to.
        let (axis_y, grip) = suggest_vise(left, Some(&bands), &narrow, &[], 2.);
        assert_eq!((axis_y, grip), (true, 8.));
        let suggested = vise_gripping(grip);
        let on = gripped(&suggested, &narrow, left, Some(&bands));
        assert_eq!(opening_issue(&suggested, &narrow, on), None);
    }
    #[test]
    fn jaws_reach_above_the_layer_a_ball_facing_took_off_the_bottom_a_turned_over_setup_rests_on() {
        // Setup 1 faces the whole top with a Ø10 ball end mill, 2 mm deep
        // (tips at 38 on the 40 mm stock), a pass every 4 mm: the box keeps
        // the layer, since only flat constant-height sweeps prove one gone.
        let (mut v, f, flipped) = untouched();
        for i in 0..16 {
            let y = -30. + 4. * i as f32;
            v.cut(
                [-60., y, 38.],
                [60., y, 38.],
                5.,
                10.,
                CutterShape::Ball,
                f,
                (0, 0),
            );
        }
        let part = f.transfer_bounds(PART, flipped);
        let left = RestStock::new(&v, flipped).left(part);
        assert_eq!(left, v.transformed_bounds(flipped));
        // Turned over, the stock rests on the faced surface, 2 mm above the
        // box's bottom, and the grip is measured from there: the jaws reach
        // 8 mm above it, 10 mm above the box's bottom.
        let bands = Bands::new(&v, flipped, part);
        let layout = vise_gripping(8.);
        let held = gripped(&layout, &vise(), left, Some(&bands));
        assert_eq!(held.band, left);
        assert!((held.bottom - (left.min[2] + 2.)).abs() < 0.001, "{held:?}");
        let jaws = boxes(&layout, &vise(), held).unwrap();
        assert!(
            jaws.iter()
                .all(|j| (j.bounds.max[2] - (left.min[2] + 10.)).abs() < 0.001),
            "{jaws:?}"
        );
        // The jaws have material from the faced surface up, and the stock
        // rests on it: the setup is holdable, and nothing blames the facing.
        let removed = Removed::new(&v, flipped).with_part(&PartGrid::boxed(&v, PART));
        assert_eq!(
            holdable(1, &layout, &vise(), held, Some(&removed), &names),
            Vec::<String>::new()
        );
        // Gripping 3 mm, the least suggested, the same.
        let least = vise_gripping(3.);
        let on_least = gripped(&least, &vise(), left, Some(&bands));
        assert!(holdable(1, &least, &vise(), on_least, Some(&removed), &names).is_empty());
        // Cut through everywhere between the jaws, nothing stands on the table.
        let gone = Gripped {
            bottom: left.max[2],
            ..held
        };
        assert_eq!(
            holdable(1, &layout, &vise(), gone, Some(&removed), &names),
            [
                "Setup 2 cannot be held in the vise: earlier setups cut away the material between the jaws up to the stock's top, so nothing stands on the table for them to grip. Choose another holder."
            ]
        );
        // A Ø10 cutter along the +Y jaw's face, over the stock's edge, 11 mm
        // above the box's bottom: within the 2 mm gap of the real jaws.
        let x = (left.min[0] + left.max[0]) / 2.;
        let (y, z) = (left.max[1] - 3., left.min[2] + 11.);
        let move_along = [([x - 20., y, z], [x + 20., y, z])];
        let hits = hits_segments(&move_along, 5., 30., 10., &jaws, 2.);
        assert_eq!(
            hits.iter()
                .map(|h| (h.label.as_str(), h.part))
                .collect::<Vec<_>>(),
            [("vise jaw on +Y", "tool")]
        );
        // Without the earlier cuts the jaws reach the grip above the box's
        // bottom, and so they do when setup 1 faced only the middle of the
        // top (passes from x = -20 to 20): the stock stands on its uncut
        // ends, and the same move is clear.
        let whole = boxes(&layout, &vise(), left).unwrap();
        assert!(whole.iter().all(|j| j.bounds.max[2] == left.min[2] + 8.));
        let (mut v, f, flipped) = untouched();
        for i in 0..16 {
            let y = -30. + 4. * i as f32;
            v.cut(
                [-20., y, 38.],
                [20., y, 38.],
                5.,
                10.,
                CutterShape::Ball,
                f,
                (0, 0),
            );
        }
        let bands = Bands::new(&v, flipped, part);
        let held = gripped(&layout, &vise(), left, Some(&bands));
        assert_eq!(held, Gripped::whole(left));
        let jaws = boxes(&layout, &vise(), held).unwrap();
        assert_eq!(jaws, whole);
        assert!(hits_segments(&move_along, 5., 30., 10., &jaws, 2.).is_empty());
    }
    #[test]
    fn the_advice_for_the_jaws_band_depends_on_where_it_lies_in_the_earlier_setup() {
        let tips = [3., 8., 13., 18., 23., 28., 33., 38.];
        let (mut v, f, flipped) = untouched();
        slot(&mut v, f, [-60., 60.], &[-30., 30.], &tips, 5., (0, 2));
        let on_side = Frame::new(
            &setup(&[("rx", "90")]),
            &crate::mesh::normalize(vec![[[0., 0., 0.], [20., 0., 0.], [0., 20., 10.]]]).unwrap(),
        );
        // Which way up the later setup is, seen from setup 1.
        assert_eq!(Removed::new(&v, f).up_in(Some((0, 2))), Some(1.));
        assert_eq!(Removed::new(&v, flipped).up_in(Some((0, 2))), Some(-1.));
        assert!(Removed::new(&v, on_side).up_in(None).unwrap().abs() < 1e-5);
        assert_eq!(Removed::new(&v, f).up_in(Some((0, 5))), None);
        let (empty, _, _) = untouched();
        assert_eq!(Removed::new(&empty, f).up_in(None), None);
        // Setup 1's contour goes only 5 mm down from its top. Turned over,
        // setup 2's jaws grip 8 mm of what was that top: the lower 5 mm of
        // their band is cut on ±Y, the upper 3 mm is not, so they close on
        // the box's width, half on air. The bottom offset (which keeps setup
        // 1's bottom) does not help, and setup 2 cannot cut its jaws' band:
        // setup 1 leaves those sides whole there, or cuts them through it.
        let (mut shallow, _, _) = untouched();
        slot(
            &mut shallow,
            f,
            [-60., 60.],
            &[-30., 30.],
            &[35., 38.],
            5.,
            (0, 2),
        );
        let part = f.transfer_bounds(PART, flipped);
        let left = RestStock::new(&shallow, flipped).left(part);
        assert_eq!(left, shallow.transformed_bounds(flipped));
        let held = gripped(
            &vise_gripping(8.),
            &vise(),
            left,
            Some(&Bands::new(&shallow, flipped, part)),
        );
        assert_eq!(held.band, left);
        let removed = Removed::new(&shallow, flipped);
        let found = holdable(1, &vise_gripping(8.), &vise(), held, Some(&removed), &names);
        assert_eq!(
            found,
            [
                "Setup 2 cannot be held in the vise: setup 1, operation 3 removed the material the jaws grip on ±Y from 0 to 8 mm above the stock's bottom. Leave at least 8 mm of those sides uncut in setup 1, or cut them through all 8 mm there, or choose another holder. Setup 1, operation 3 is Contour \"Outside\"."
            ]
        );
        assert!(!found[0].contains("bottom_offset") && !found[0].contains("this setup"));
        // Cut through those 8 mm (11 mm down), the jaws close on the walls.
        slot(
            &mut shallow,
            f,
            [-60., 60.],
            &[-30., 30.],
            &[29., 32.],
            5.,
            (0, 2),
        );
        let bands = Bands::new(&shallow, flipped, part);
        let held = gripped(&vise_gripping(8.), &vise(), left, Some(&bands));
        assert!(held.band.dimensions()[1] < 50.1, "{held:?}");
        let removed = Removed::new(&shallow, flipped);
        assert!(holdable(1, &vise_gripping(8.), &vise(), held, Some(&removed), &names).is_empty());
        // The full-depth cut seen from a setup facing the same way keeps the
        // old advice: the band is at setup 1's bottom.
        let left = RestStock::new(&v, f).left(PART);
        let removed = Removed::new(&v, f);
        let found = holdable(1, &vise_gripping(8.), &vise(), left, Some(&removed), &names);
        assert!(
            found[0].contains("(for example with `bottom_offset`)"),
            "{found:?}"
        );
        assert!(!found[0].contains("cut those sides"));
    }
    #[test]
    fn the_stock_must_stand_a_millimetre_above_the_jaws() {
        let short = Bounds {
            min: [-50., -30., 0.],
            max: [50., 30., 9.],
        };
        let said = "The stock is 9 mm tall; the vise grips 8.5 mm of it, which leaves less than 1 mm above the jaws.";
        assert_eq!(
            height_issue(&vise_gripping(8.5), short).as_deref(),
            Some(said)
        );
        assert_eq!(
            height_issue(&vise_gripping(12.), short).as_deref(),
            Some(
                "The stock is 9 mm tall; the vise grips 12 mm of it, which leaves less than 1 mm above the jaws."
            )
        );
        // Exactly 1 mm above the jaws is enough; clamps hold from above.
        assert_eq!(height_issue(&vise_gripping(8.), short), None);
        assert_eq!(
            height_issue(&clamp_layout(&[(Side::MinusX, 0.)]), short),
            None
        );
        assert_eq!(height_issue(&Layout::None, short), None);
        // Setup 1 has no earlier cuts: the height is all that is checked.
        assert_eq!(
            holdable(0, &vise_gripping(8.5), &vise(), short, None, &names),
            [said]
        );
        assert!(holdable(0, &vise_gripping(8.), &vise(), short, None, &names).is_empty());
        let clamps = clamp_layout(&[(Side::MinusX, 0.), (Side::PlusX, 0.)]);
        assert!(holdable(0, &clamps, &clamp(), short, None, &names).is_empty());
        // A later setup on uncut stock: the height alone again.
        let (v, f, _) = untouched();
        let removed = Removed::new(&v, f);
        assert_eq!(
            holdable(
                1,
                &vise_gripping(8.5),
                &vise(),
                short,
                Some(&removed),
                &names
            ),
            [said]
        );
        // An entry of the other kind cannot hold at all.
        assert_eq!(
            holdable(0, &vise_gripping(8.), &clamp(), stock(), None, &names),
            ["The chosen workholding entry is a clamp, not a vise."]
        );
    }
    #[test]
    fn a_vise_needs_most_of_the_stocks_bottom_to_rest_on() {
        let (mut v, f, flipped) = untouched();
        // Setup 1 pockets 4 mm into the top over 90 of its 100 mm in X and 50
        // of its 60 mm in Y; setup 2 is flipped onto that face.
        let ys = [-20., -12., -4., 4., 12., 20.];
        slot(&mut v, f, [-40., 40.], &ys, &[36.], 10., (0, 1));
        let part = f.transfer_bounds(PART, flipped);
        let left = RestStock::new(&v, flipped).left(part);
        assert_eq!(left, v.transformed_bounds(flipped));
        let removed = Removed::new(&v, flipped);
        assert_eq!(
            holdable(1, &vise_gripping(8.), &vise(), left, Some(&removed), &names),
            [
                "Setup 2 cannot be held in the vise: setup 1, operation 2 removed most of the stock's bottom it would rest on. Leave at least half of it and cut the rest in a later setup, or choose another holder. Setup 1, operation 2 is Pocket \"Underside\"."
            ]
        );
        // A pocket over the middle 18 mm leaves enough to rest on.
        let (mut v, f, flipped) = untouched();
        slot(&mut v, f, [-40., 40.], &[-4., 4.], &[36.], 10., (0, 1));
        let left = RestStock::new(&v, flipped).left(part);
        let removed = Removed::new(&v, flipped);
        assert!(holdable(1, &vise_gripping(8.), &vise(), left, Some(&removed), &names).is_empty());
    }
    #[test]
    fn each_clamp_needs_the_stock_under_its_bite() {
        let (mut v, f, _) = untouched();
        // Setup 1 faces 2 mm off a rim along the ±X edges, 11 mm wide.
        for x in [-50., -44., 44., 50.] {
            v.cut(
                [x, -40., 38.],
                [x, 40., 38.],
                5.,
                10.,
                CutterShape::Flat,
                f,
                (0, 0),
            );
        }
        let left = RestStock::new(&v, f).left(PART);
        assert_eq!(left, stock());
        let removed = Removed::new(&v, f);
        // The clamps on ±X bite 12 mm onto the stock, over the rim; the one
        // on -Y bites where nothing was cut.
        let clamps = clamp_layout(&[(Side::MinusX, 0.), (Side::PlusX, 0.), (Side::MinusY, 0.)]);
        let said = |side: &str| {
            format!(
                "Setup 2 cannot be held by the clamp on {side} (offset 0): setup 1, operation 1 removed the stock under its bite. Leave the top there, 12 mm in from the edge, and finish it in a later setup, move the clamp, or choose another holder. Setup 1, operation 1 is Facing \"Rim\"."
            )
        };
        assert_eq!(
            holdable(1, &clamps, &clamp(), left, Some(&removed), &names),
            [said("-X"), said("+X")]
        );
        // Moved along the side past the rim's end, the clamp on -X holds.
        let (mut v, f, _) = untouched();
        v.cut(
            [-50., -40., 38.],
            [-50., 0., 38.],
            11.,
            10.,
            CutterShape::Flat,
            f,
            (0, 0),
        );
        let removed = Removed::new(&v, f);
        let moved = clamp_layout(&[(Side::MinusX, -15.), (Side::MinusX, 18.)]);
        assert_eq!(
            holdable(1, &moved, &clamp(), stock(), Some(&removed), &names),
            [said("-X").replace("(offset 0)", "(offset -15)")]
        );
    }
    #[test]
    fn the_cause_names_the_operations_behind_a_quarter_of_the_failed_samples_most_first() {
        // The jaws on ±Y sample 0.08, 2.69, 5.31 and 7.92 mm above the bottom.
        let (mut v, f, _) = untouched();
        // Operation 4 takes the top two rows on -Y, the jaw's whole width;
        // operation 3 the row below them on a third of it: 4 of the 28
        // failed samples, too few to name.
        slot(&mut v, f, [-60., 60.], &[-30.], &[5.], 40., (0, 3));
        slot(&mut v, f, [-60., -20.], &[-30.], &[2.], 2., (0, 2));
        let removed = Removed::new(&v, f);
        let found = holdable(
            1,
            &vise_gripping(8.),
            &vise(),
            stock(),
            Some(&removed),
            &names,
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].starts_with(
                "Setup 2 cannot be held in the vise: setup 1, operation 4 removed the material the jaws grip on -Y from 0 to 8 mm"
            ) && found[0].ends_with(" holder. Setup 1, operation 4 is Contour \"Low\"."),
            "{found:?}"
        );
        // Along the whole width it is 12 of 36: named after the larger.
        slot(&mut v, f, [-20., 60.], &[-30.], &[2.], 2., (0, 2));
        let removed = Removed::new(&v, f);
        let found = holdable(
            1,
            &vise_gripping(8.),
            &vise(),
            stock(),
            Some(&removed),
            &names,
        );
        assert!(
            found[0].starts_with(
                "Setup 2 cannot be held in the vise: setup 1, operation 4 and setup 1, operation 3 removed the material the jaws grip on -Y from 0 to 8 mm"
            ) && found[0].ends_with(
                " holder. Setup 1, operation 4 is Contour \"Low\"; setup 1, operation 3 is Contour \"Outside\"."
            ),
            "{found:?}"
        );
    }
    #[test]
    fn earlier_operations_are_named_by_their_setup_number_and_strategy() {
        let (doc, _) = crate::post::fixture::job();
        assert_eq!(operation_name(&doc, 0, 2), "Contour \"Contour\"");
        assert_eq!(operation_name(&doc, 0, 0), "Facing \"Facing\"");
        // One the document does not have has no name.
        assert_eq!(operation_name(&doc, 2, 0), "");
    }
    /// Flat Ø10 passes, 5 mm of flute each, along the rectangle of corners
    /// (±`half[0]`, ±`half[1]`) at each tip height of `tips`: an outside
    /// contour by setup 1's operation 3.
    fn around(v: &mut Volume, f: Frame, half: [f32; 2], tips: &[f32]) {
        let [x, y] = half;
        outline(v, f, &[[-x, -y], [x, -y], [x, y], [-x, y]], tips);
    }
    /// Flat Ø10 passes, 5 mm of flute each, round the closed polygon of
    /// `corners` at each tip height of `tips`: an outside contour by setup
    /// 1's operation 3.
    fn outline(v: &mut Volume, f: Frame, corners: &[[f32; 2]], tips: &[f32]) {
        for &z in tips {
            for k in 0..corners.len() {
                let (a, b) = (corners[k], corners[(k + 1) % corners.len()]);
                v.cut(
                    [a[0], a[1], z],
                    [b[0], b[1], z],
                    5.,
                    5.,
                    CutterShape::Flat,
                    f,
                    (0, 2),
                );
            }
        }
    }
    /// The part of `a_part_cut_free_from_what_the_jaws_or_clamps_hold_cannot_be_held`
    /// with its +X+Y corner cut away: the box less x > 0, y > 0, an L of
    /// 20 triangles facing out.
    fn l_part() -> Vec<[[f32; 3]; 3]> {
        // Counter-clockwise seen from above.
        let rim = [
            [-30., -15.],
            [30., -15.],
            [30., 0.],
            [0., 0.],
            [0., 15.],
            [-30., 15.],
        ];
        let (lo, hi) = (0., 37.);
        let at = |p: [f32; 2], z: f32| [p[0], p[1], z];
        let mut tris = vec![];
        for k in 0..rim.len() {
            let (a, b) = (rim[k], rim[(k + 1) % rim.len()]);
            tris.push([at(a, lo), at(b, lo), at(b, hi)]);
            tris.push([at(a, lo), at(b, hi), at(a, hi)]);
        }
        // The caps, as the two rectangles the L is made of.
        for [a, b, c, d] in [
            [[-30., -15.], [0., -15.], [0., 15.], [-30., 15.]],
            [[0., -15.], [30., -15.], [30., 0.], [0., 0.]],
        ] {
            tris.push([at(a, hi), at(b, hi), at(c, hi)]);
            tris.push([at(a, hi), at(c, hi), at(d, hi)]);
            tris.push([at(a, lo), at(c, lo), at(b, lo)]);
            tris.push([at(a, lo), at(d, lo), at(c, lo)]);
        }
        tris
    }
    #[test]
    fn a_part_cut_free_from_what_the_jaws_or_clamps_hold_cannot_be_held() {
        // A part on the stock's bottom, 20 mm inside its ±X faces and 15
        // inside its ±Y faces, and an outside contour 10 mm wide round it,
        // down past the bottom: it leaves a frame cut free from the part.
        let part = Bounds {
            min: [-30., -15., 0.],
            max: [30., 15., 37.],
        };
        let (mut v, f, _) = untouched();
        around(
            &mut v,
            f,
            [35., 20.],
            &[-1., 4., 9., 14., 19., 24., 29., 34., 39.],
        );
        let left = RestStock::new(&v, f).left(part);
        assert_eq!(left, stock());
        let removed = Removed::new(&v, f).with_part(&PartGrid::boxed(&v, part));
        let advice = "Leave material joining them (for example a `bottom_offset` above the stock's bottom, or a skin under the part) and cut it free in the last setup. Setup 1, operation 3 is Contour \"Outside\".";
        // The jaws grip the frame: material, but not the part's.
        assert_eq!(
            holdable(1, &vise_gripping(8.), &vise(), left, Some(&removed), &names),
            [format!(
                "Setup 2 cannot be held in the vise: setup 1, operation 3 cut the part free from the material the jaws grip on ±Y. {advice}"
            )]
        );
        // So do the clamps on ±X: their bites are on the frame and the cut.
        let clamps = clamp_layout(&[(Side::MinusX, 0.), (Side::PlusX, 0.)]);
        let said = |side: &str| {
            format!(
                "Setup 2 cannot be held by the clamp on {side} (offset 0): setup 1, operation 3 cut the part free from the stock under its bite. {advice}"
            )
        };
        assert_eq!(
            holdable(1, &clamps, &clamp(), left, Some(&removed), &names),
            [said("-X"), said("+X")]
        );
        // Not knowing the part, only the material is checked: it is there.
        let unknown = Removed::new(&v, f);
        assert!(holdable(1, &vise_gripping(8.), &vise(), left, Some(&unknown), &names).is_empty());
        // Stopped 3 mm above the bottom, the contour leaves a skin joining
        // the frame to the part: both hold.
        let (mut v, f, _) = untouched();
        around(
            &mut v,
            f,
            [35., 20.],
            &[3., 8., 13., 18., 23., 28., 33., 38.],
        );
        let removed = Removed::new(&v, f).with_part(&PartGrid::boxed(&v, part));
        assert!(holdable(1, &vise_gripping(8.), &vise(), left, Some(&removed), &names).is_empty());
        assert!(holdable(1, &clamps, &clamp(), left, Some(&removed), &names).is_empty());
    }
    #[test]
    fn a_part_cut_free_along_its_own_outline_cannot_be_held_whatever_its_box_keeps() {
        // The part above with its +X+Y corner cut away (`l_part`), and an
        // outside contour 10 mm wide round its outline, down past the
        // bottom: it leaves a frame cut free from the part, joined to the
        // block the corner left (x 10 to 40, y 10 to 20), which lies within
        // the part's box but outside the part.
        let path = [
            [-35., -20.],
            [35., -20.],
            [35., 5.],
            [5., 5.],
            [5., 20.],
            [-35., 20.],
        ];
        let deep = [-1., 4., 9., 14., 19., 24., 29., 34., 39.];
        let (mut v, f, _) = untouched();
        outline(&mut v, f, &path, &deep);
        let removed = Removed::new(&v, f).with_part(&PartGrid::of(&v, l_part()));
        let advice = "Leave material joining them (for example a `bottom_offset` above the stock's bottom, or a skin under the part) and cut it free in the last setup. Setup 1, operation 3 is Contour \"Outside\".";
        assert_eq!(
            holdable(
                1,
                &vise_gripping(8.),
                &vise(),
                stock(),
                Some(&removed),
                &names
            ),
            [format!(
                "Setup 2 cannot be held in the vise: setup 1, operation 3 cut the part free from the material the jaws grip on ±Y. {advice}"
            )]
        );
        // The corner's block is stock, not the part's material.
        assert_eq!(removed.joined([-20., 10., 20.]), Some(true));
        assert_eq!(removed.joined([20., 15., 20.]), Some(false));
        let clamps = clamp_layout(&[(Side::MinusX, 0.), (Side::PlusX, 0.)]);
        let said = |side: &str| {
            format!(
                "Setup 2 cannot be held by the clamp on {side} (offset 0): setup 1, operation 3 cut the part free from the stock under its bite. {advice}"
            )
        };
        assert_eq!(
            holdable(1, &clamps, &clamp(), stock(), Some(&removed), &names),
            [said("-X"), said("+X")]
        );
        // Stopped 3 mm above the bottom, the contour leaves a skin joining
        // the frame to the part: both hold.
        let (mut v, f, _) = untouched();
        outline(&mut v, f, &path, &[3., 8., 13., 18., 23., 28., 33., 38.]);
        let removed = Removed::new(&v, f).with_part(&PartGrid::of(&v, l_part()));
        assert_eq!(removed.joined([20., 15., 20.]), Some(true));
        assert!(
            holdable(
                1,
                &vise_gripping(8.),
                &vise(),
                stock(),
                Some(&removed),
                &names
            )
            .is_empty()
        );
        assert!(holdable(1, &clamps, &clamp(), stock(), Some(&removed), &names).is_empty());
    }
    #[test]
    fn the_suggested_grip_leaves_a_millimetre_of_a_thin_stock_above_the_jaws() {
        let tall = |z: f32| Bounds {
            min: [-50., -30., 0.],
            max: [50., 30., z],
        };
        for (z, grip) in [(40., 8.), (9., 8.), (6., 5.), (6.25, 5.25), (4.5, 3.5)] {
            let (axis_y, suggested) = suggest_vise(tall(z), None, &vise(), &[], 2.);
            assert_eq!(suggested, grip, "{z} mm tall");
            let layout = Layout::Vise {
                holder: String::new(),
                axis_y,
                offset: 0.,
                grip: suggested,
                gap: 2.,
            };
            assert_eq!(height_issue(&layout, tall(z)), None, "{z} mm tall");
        }
        // Never below the least grip a suggestion makes, however thin.
        assert_eq!(suggest_vise(tall(3.5), None, &vise(), &[], 2.).1, 3.);
        assert_eq!(suggest_vise(tall(1.), None, &vise(), &[], 2.).1, 3.);
        // A holder gripping less than that keeps its own grip.
        let mut low = vise();
        low.p.insert("grip".into(), "2".into());
        assert_eq!(suggest_vise(tall(1.), None, &low, &[], 2.).1, 2.);
    }
}
