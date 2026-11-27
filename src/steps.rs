//! The Steps page: a setup on the manual mill as a short sheet (`guide`,
//! LIO-688), where its work zero is on the stock held, then a page an
//! operation, each feature numbered on a plan drawing and listed with its
//! place, size and depth; a setup on the manual lathe screen by screen, as
//! side views and dial pictures (`turning`).
pub mod tablet;
mod turning;
use crate::{
    App, GuideAction, Message,
    guide::{Depth, Features, Guide, Section, Step},
    help::HelpButton,
    holding::Gripped,
    mesh::{Bounds, Mesh},
    model::{Document, Setup, Strategy, Tool, get, num},
    ui::{BLUE, BUTTON_EDGE, HOVER_BUTTON, INK, LINE, MUTED, WARNING, bold, muted, primary},
    viewport,
};
use iced::advanced::{
    Layout, Widget, layout,
    widget::{Operation, Tree},
};
use iced::{
    Alignment, Border, Color, Element, Font,
    Length::{self, Fill, FillPortion},
    Point, Rectangle, Size, Theme, mouse,
    widget::{Column, button, canvas, column, container, image, row, scrollable, space, text},
};
use std::cell::Cell;
pub(crate) use turning::dial_total;

const ACCENT: Color = Color::from_rgb(0.91, 0.35, 0.05);
const TOOL: Color = Color::from_rgb(0.18, 0.50, 0.93);
const NEVER: Color = Color::from_rgb(0.81, 0.13, 0.18);
const CHIP: Color = Color::from_rgb(0.90, 0.91, 0.93);
const STOCK: Color = Color::from_rgb(0.90, 0.92, 0.94);
const PART: Color = Color::from_rgb(0.69, 0.73, 0.77);
const JAW: Color = Color::from_rgb(0.80, 0.82, 0.85);
const DONE: Color = Color::from_rgb(0.50, 0.55, 0.60);
/// A clamp seen from above, over the stock it holds down (LIO-458).
const CLAMP: Color = Color::from_rgba(0.38, 0.43, 0.49, 0.9);
/// The pixels of the 3D pictures, twice the size they are shown at.
const PICTURE: (u32, u32) = (960, 640);
/// The size of a Tool step's picture: square, as the tool stands up off
/// the stock and the panel is about as high as it is wide (LIO-379).
const TOOL_PICTURE: (u32, u32) = (900, 900);
/// The views a drawing can be: from above (X, Y), from the front (X, Z) and
/// from the right (Y, Z).
const VIEWS: [[usize; 2]; 3] = [[0, 1], [0, 2], [1, 2]];
const AXES: [&str; 3] = ["X", "Y", "Z"];

/// A guide with what its pages draw, built in the background.
#[derive(Debug)]
pub struct Shown {
    pub guide: Guide,
    /// The part's faces in work coordinates, as each of `VIEWS` sees them
    /// (only those that face it).
    part: [Vec<Vec<[f32; 2]>>; 3],
    /// The stock box before each operation, then after the last, in work
    /// coordinates (`simulation::leftover_box`).
    pub(crate) boxes: Vec<Bounds>,
    /// The vise jaws or clamps, in work coordinates, from the stock's
    /// bottom up (`holding::shown`).
    jaws: Vec<Bounds>,
    /// For the Zero page, in a vise: its jaws at their full height and the
    /// parallels under the stock, in work coordinates.
    vise: Vec<Bounds>,
    parallels: Vec<Bounds>,
    /// The stock in 3D: before the setup, then after each operation.
    pictures: Vec<Option<image::Handle>>,
    /// The stock in 3D as it starts, in the vise on its parallels.
    held: Option<image::Handle>,
    /// The work zero marked on `held`, in its pixels (LIO-688).
    zero: Vec<Dim>,
    /// On the lathe: what its side views draw.
    side: Option<turning::Side>,
}
/// A size drawn on a picture: a dimension line from `a` to `b` (picture
/// pixels), with extension lines to them from the feature's points `ext`,
/// or with `leader` an arrow onto `a` from `b`, labelled.
#[derive(Debug, Clone)]
struct Dim {
    a: Point,
    b: Point,
    ext: Option<[Point; 2]>,
    label: String,
    leader: bool,
}
impl Shown {
    /// The same pictures with the guide worked out again for `doc`
    /// (`guide::again`): after a measurement or the dial check's answer.
    pub fn again(&self, doc: &Document, mesh: &Mesh) -> Result<Shown, String> {
        Ok(Shown {
            guide: crate::guide::again(doc, mesh, &self.guide)?,
            part: self.part.clone(),
            boxes: self.boxes.clone(),
            jaws: self.jaws.clone(),
            vise: self.vise.clone(),
            parallels: self.parallels.clone(),
            pictures: self.pictures.clone(),
            held: self.held.clone(),
            zero: self.zero.clone(),
            side: self.side.clone(),
        })
    }
}
/// The normal of a planar polygon (Newell's), not normalised.
fn normal(p: &[[f32; 3]]) -> [f32; 3] {
    let mut n = [0.; 3];
    for (i, a) in p.iter().enumerate() {
        let b = p[(i + 1) % p.len()];
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    n
}
/// The guide of setup `si` (`guide::build`) with the part's outline, the
/// stock boxes, the jaws and the 3D pictures of the stock.
pub fn shown(doc: &Document, mesh: &Mesh, si: usize) -> Result<Shown, String> {
    let guide = crate::guide::build(doc, mesh, si)?;
    let setup = &doc.setups[si];
    let zero = guide.zero;
    let work = |b: Bounds| b.translated(zero.map(|v| -v));
    let placement = crate::stock::Placement::new(mesh, Some(setup));
    let scale = 1. / mesh.display_scale();
    let mut part: [Vec<Vec<[f32; 2]>>; 3] = Default::default();
    for face in mesh.faces.iter().filter(|f| f.points.len() >= 3) {
        let points: Vec<[f32; 3]> = (face.points.iter())
            .map(|p| {
                let q = placement.transform(*p);
                std::array::from_fn(|k| q[k] * scale - zero[k])
            })
            .collect();
        let n = normal(&points);
        let size = n.iter().map(|v| v.abs()).sum::<f32>().max(1e-9);
        for (i, [a, b]) in VIEWS.iter().enumerate() {
            if n[3 - a - b].abs() / size > 0.02 {
                part[i].push(points.iter().map(|p| [p[*a], p[*b]]).collect());
            }
        }
    }
    // The stock box before each operation, then after the last, and the
    // vise's jaws or the clamps, each in work coordinates.
    use crate::simulation::leftover_box;
    let mut boxes = vec![work(leftover_box(&guide.before, setup, mesh))];
    boxes.extend(
        (guide.done.iter()).map(|d| work(leftover_box(&d.after, setup, mesh))),
    );
    let jaws: Vec<Bounds> = match &guide.steps[0] {
        Step::Zero { holding, .. } if crate::holding::modelled(setup) => {
            let held = Gripped {
                stock: holding.stock.translated(zero),
                band: holding.band.translated(zero),
                bottom: holding.bottom + zero[2],
                girth: holding.girth,
            };
            (doc.holding_boxes(setup, held))
                .map(|b| crate::holding::shown(&b, held.stock))
                .unwrap_or_default()
                .into_iter()
                .map(|b| work(b.bounds))
                .collect()
        }
        _ => vec![],
    };
    // The parallels the setup says the stock stands on (LIO-537), under
    // the box it starts from.
    let modelled: Vec<Bounds> = match &guide.steps[0] {
        Step::Zero { holding, .. } => {
            crate::parallels::boxes(setup, holding.stock.translated(zero))
                .into_iter()
                .map(|b| work(b.bounds))
                .collect()
        }
        _ => vec![],
    };
    // In a vise: the jaw plates go down from the grip by their height, and
    // parallels fill what is under the stock between them: the setup's own
    // when it says where they are.
    let (vise, parallels) = match &guide.steps[0] {
        Step::Zero {
            holding,
            vise: Some(v),
            ..
        } if !jaws.is_empty() => {
            let tall: Vec<Bounds> = (jaws.iter())
                .map(|j| Bounds {
                    min: [j.min[0], j.min[1], j.max[2] - v.jaw_height],
                    max: j.max,
                })
                .collect();
            let (s, k) = (holding.stock, v.across);
            let floor = holding.bottom + v.grip - v.jaw_height;
            let w = ((s.max[k] - s.min[k]) / 4.).min(10.);
            let bars = [s.min[k] + 2., s.max[k] - 2. - w].map(|a| {
                let mut b = s;
                (b.min[k], b.max[k]) = (a, a + w);
                (b.min[2], b.max[2]) = (floor, s.min[2]);
                b
            });
            let under = floor < s.min[2] - 0.5;
            let bars = match () {
                _ if !modelled.is_empty() => modelled.clone(),
                _ if under => bars.to_vec(),
                _ => vec![],
            };
            (tall, bars)
        }
        _ => (vec![], modelled.clone()),
    };
    // The jaws or clamps and the parallels, in the pictures.
    let mut jaws = jaws;
    jaws.extend(modelled.iter().copied());
    let frame = crate::volume::Frame::new(setup, mesh);
    let solid = |boxes: &[Bounds]| -> Vec<_> {
        (boxes.iter())
            .flat_map(|b| viewport::box_triangles(b.translated(zero), viewport::JAWS))
            .collect()
    };
    let held = solid(&jaws);
    // The stock before the setup, then after each operation, lit alike
    // over the depth of every cut of the setup (LIO-370).
    let volumes: Vec<&crate::volume::Volume> = std::iter::once(&guide.before)
        .chain(guide.done.iter().map(|d| &d.after))
        .collect();
    let surfaces: Vec<_> = (volumes.iter())
        .map(|v| crate::stock_surface::Snapshot::of(v, crate::stock_surface::Detail::Draft).build())
        .collect();
    let span = (volumes.iter().zip(&surfaces))
        .filter_map(|(v, s)| Some(viewport::cut_span(v, s.as_ref().ok()?, frame)))
        .fold([f32::NEG_INFINITY, f32::INFINITY], |a, b| {
            [a[0].max(b[0]), a[1].min(b[1])]
        });
    let stock: Vec<Vec<_>> = (volumes.iter().zip(&surfaces).zip(&boxes))
        .map(|((v, surface), box_)| match surface {
            Ok(s) => viewport::stock_triangles_lit(v, s, frame, None, span).collect(),
            Err(_) => viewport::box_triangles(box_.translated(zero), viewport::BLOCK),
        })
        .collect();
    let picture = |i: usize, held: &[_]| {
        let mut triangles = stock[i].clone();
        triangles.extend(held.iter().copied());
        viewport::stock_picture(&triangles, PICTURE)
    };
    let pictures: Vec<_> = (0..stock.len()).map(|i| picture(i, &held)).collect();
    // The stock as it starts, held: in a vise, its jaws at their full
    // height and the parallels under it.
    let mut seen = stock[0].clone();
    if vise.is_empty() {
        seen.extend(held.iter().copied());
    } else {
        seen.extend(solid(&[&vise[..], &parallels].concat()));
    }
    let held = viewport::stock_picture(&seen, PICTURE);
    // The work zero marked on it, an arrow down onto it (LIO-688).
    let zero_mark = match crate::lathe::is_lathe(&setup.machine) {
        true => vec![],
        false => {
            let up = [zero[0], zero[1], zero[2] + 30.];
            match viewport::picture_points(&seen, PICTURE, &[zero, up])[..] {
                [a, b] => vec![Dim {
                    a,
                    b,
                    ext: None,
                    label: "X0 Y0 Z0".into(),
                    leader: true,
                }],
                _ => vec![],
            }
        }
    };
    let side = crate::lathe::is_lathe(&setup.machine).then(|| turning::side(doc, mesh, &guide));
    Ok(Shown {
        guide,
        part,
        boxes,
        jaws,
        vise,
        parallels,
        pictures,
        held,
        zero: zero_mark,
        side,
    })
}
/// Where a new tool touches the top: on the highest flat face of the stock
/// `triangles` (stock frame), the middle of it, or the face's triangle
/// nearest that when the face is not convex.
pub(crate) fn touch_spot(triangles: &[([[f32; 3]; 3], [u8; 4])]) -> Option<[f32; 3]> {
    let flat = |t: &[[f32; 3]; 3]| {
        let n = normal(t);
        let size = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        size > 1e-9 && n[2].abs() / size > 0.999
    };
    let top = (triangles.iter())
        .filter(|(t, _)| flat(t))
        .map(|(t, _)| t[0][2])
        .fold(f32::NEG_INFINITY, f32::max);
    let middles: Vec<[f32; 3]> = (triangles.iter())
        .filter(|(t, _)| flat(t) && t.iter().all(|p| (p[2] - top).abs() < 0.01))
        .map(|(t, _)| std::array::from_fn(|k| (t[0][k] + t[1][k] + t[2][k]) / 3.))
        .collect();
    let n = middles.len() as f32;
    let centre: [f32; 3] = std::array::from_fn(|k| middles.iter().map(|m| m[k]).sum::<f32>() / n);
    middles
        .into_iter()
        .min_by(|a, b| {
            let d = |m: &[f32; 3]| (m[0] - centre[0]).hypot(m[1] - centre[1]);
            d(a).total_cmp(&d(b))
        })
        .map(|m| [m[0], m[1], top])
}

/// A number to two decimals, a true minus, never −0.00.
fn dro(v: f32) -> String {
    let v = if v.abs() < 0.005 { 0. } else { v };
    format!("{v:.2}").replace('-', "−")
}
/// A length without trailing zeros: 25, 8.5.
fn mm(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0')
        .trim_end_matches('.')
        .replace('-', "−")
}
fn chip<'a>(s: impl Into<String>) -> Element<'a, Message> {
    container(text(s.into()).size(13))
        .padding([4, 9])
        .style(|_| container::Style {
            background: Some(CHIP.into()),
            text_color: Some(INK),
            border: Border {
                radius: 6.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}
fn warn_chip<'a>(s: impl Into<String>) -> Element<'a, Message> {
    container(text(s.into()).size(13))
        .padding([4, 9])
        .style(|_| container::Style {
            background: Some(WARNING[0].into()),
            text_color: Some(WARNING[1]),
            border: Border {
                radius: 6.into(),
                color: WARNING[1],
                width: 1.,
            },
            ..Default::default()
        })
        .into()
}
fn chips<'a>(items: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    row(items).spacing(6).wrap().vertical_spacing(6).into()
}
/// The Zero page's label over the setup's notes.
const SETUP_NOTES: &str = "Setup notes";
/// What a page that leaves the supports under the part to the user adds
/// when the setup's notes say where they go (LIO-487).
const SEE_NOTES: &str = "the setup notes on the Zero page say where the supports go";
/// A small capital label over a group.
fn label<'a>(s: impl Into<String>) -> Element<'a, Message> {
    text(s.into().to_uppercase()).size(11).color(MUTED).into()
}
/// A round mark with a number or a tick.
fn disc<'a>(s: impl Into<String>, on: bool, size: f32) -> Element<'a, Message> {
    container(
        text(s.into())
            .size(size * 0.5)
            .color(if on { Color::WHITE } else { INK }),
    )
    .width(size)
    .height(size)
    .center_x(size)
    .center_y(size)
    .style(move |_| container::Style {
        background: Some(if on { ACCENT } else { CHIP }.into()),
        border: Border {
            radius: (size / 2.).into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}
/// A box with a border, the accent's when `on`.
fn boxed<'a>(
    content: impl Into<Element<'a, Message>>,
    on: bool,
) -> container::Container<'a, Message> {
    container(content).style(move |_| container::Style {
        background: Some(Color::WHITE.into()),
        border: Border {
            color: if on { ACCENT } else { LINE },
            width: if on { 2. } else { 1. },
            radius: 8.into(),
        },
        ..Default::default()
    })
}
/// A button with no face of its own, for a readout or a tile.
fn tap<'a>(
    content: impl Into<Element<'a, Message>>,
    msg: Option<Message>,
    label: &str,
) -> Element<'a, Message> {
    let b = button(content)
        .padding(0)
        .width(Fill)
        .on_press_maybe(msg.clone())
        .style(|_, _| button::Style::default());
    match msg {
        Some(m) => HelpButton::new(b, &m, label).width(Fill).into(),
        None => b.into(),
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
enum Lit {
    Done,
    Now,
    Next,
}
fn lit(i: usize, at: usize) -> Lit {
    match i.cmp(&at) {
        std::cmp::Ordering::Less => Lit::Done,
        std::cmp::Ordering::Equal => Lit::Now,
        std::cmp::Ordering::Greater => Lit::Next,
    }
}
/// One large DRO number: its order, a word before it, the axis, the number
/// and short tags on its right (a line with a lock first when `lock`).
struct Readout {
    order: Option<u8>,
    lead: Option<String>,
    axis: String,
    value: String,
    lock: Option<String>,
    tags: Vec<String>,
    lit: Lit,
    big: bool,
}
fn readout<'a>(r: Readout, msg: Option<Message>) -> Element<'a, Message> {
    let now = r.lit == Lit::Now;
    let ink = match r.lit {
        Lit::Done => DONE,
        Lit::Now => ACCENT,
        Lit::Next => INK,
    };
    let mono = Font {
        weight: iced::font::Weight::Bold,
        ..Font::MONOSPACE
    };
    // The order and the word before the number, which wrap when the
    // number has no room beside them on one line (LIO-588).
    let mut lead = row![].spacing(10).align_y(Alignment::Center);
    let has_lead = r.order.is_some() || r.lead.is_some();
    if let Some(n) = r.order {
        let mark = if r.lit == Lit::Done {
            "✓".to_string()
        } else {
            n.to_string()
        };
        lead = lead.push(disc(mark, now, 22.));
    }
    if let Some(words) = r.lead {
        lead = lead.push(text(words).size(15).color(MUTED));
    }
    let number = row![
        text(r.axis).size(20).font(mono).color(ink),
        // A DRO number has no space to wrap at; "M8×1.25 screw" does,
        // when the page is narrower than it (`Beside`).
        text(r.value)
            .size(if r.big { 40 } else { 28 })
            .font(mono)
            .color(ink)
            .wrapping(text::Wrapping::Word)
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    let mut side: Vec<Element<'a, Message>> = vec![];
    if let Some(axes) = r.lock {
        side.push(
            row![
                crate::fault::canvas(Lock).width(10).height(12),
                text(format!("lock {axes}")).size(12).color(MUTED)
            ]
            .spacing(4)
            .align_y(Alignment::Center)
            .into(),
        );
    }
    // A long tag goes under the value, not squeezed beside it a word a
    // line (LIO-431); so does a short one with no room beside it on one
    // line, as at the tablet's narrow numbers (LIO-588).
    let said = r.tags.join(" · ");
    let mut under = vec![];
    if !said.is_empty() {
        let tag: Element<'a, Message> = text(said.clone()).size(12).color(MUTED).into();
        match said.chars().count() > 30 {
            true => under.push(tag),
            false => side.push(tag),
        }
    }
    let body = Element::new(Beside::new(
        has_lead.then(|| lead.into()),
        number,
        side,
        under,
    ));
    let face = boxed(container(body).padding([4, 12]), now).width(Fill);
    match msg {
        Some(m) => tap(face, Some(m), "Reached"),
        None => face.into(),
    }
}
/// A readout's line, its order and words then its number, with its short
/// tags on the right, each on one line, stacked and centred on the line's
/// height. Nothing is squeezed or runs past the right edge (LIO-588: at the
/// tablet's narrow numbers, "what the last pass left" was squeezed beside a
/// bore's size a few letters a line and cut off): a tag with no room beside
/// the line goes under it, full width, with the ones that always go there;
/// words with no room beside the number on one line wrap, or at a very
/// narrow width go over it; a number wider than the page wraps at its
/// spaces, "M8×1.25 screw" on a Tap's Check.
struct Beside<'a> {
    /// The words if any, the number, the tags for the right, then those
    /// for under the line.
    items: Vec<Element<'a, Message>>,
    lead: bool,
    side: usize,
}
impl<'a> Beside<'a> {
    /// Room between the words, the number and the tags, and between
    /// stacked tags.
    const GAP: f32 = 10.;
    const STACK: f32 = 2.;
    fn new(
        lead: Option<Element<'a, Message>>,
        number: impl Into<Element<'a, Message>>,
        side: Vec<Element<'a, Message>>,
        under: Vec<Element<'a, Message>>,
    ) -> Self {
        let n = side.len();
        let has_lead = lead.is_some();
        let mut items: Vec<_> = lead.into_iter().collect();
        items.push(number.into());
        items.extend(side);
        items.extend(under);
        Beside {
            items,
            lead: has_lead,
            side: n,
        }
    }
}
impl Widget<Message, Theme, iced::Renderer> for Beside<'_> {
    fn children(&self) -> Vec<Tree> {
        self.items.iter().map(Tree::new).collect()
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.items);
    }
    fn size(&self) -> Size<Length> {
        Size::new(Fill, Length::Shrink)
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let free = layout::Limits::new(Size::ZERO, Size::INFINITE);
        let max = limits.max().width;
        let within = |w: f32| layout::Limits::new(Size::ZERO, Size::new(w, f32::INFINITY));
        let (has_lead, side, count) = (self.lead, self.side, self.items.len());
        let at = usize::from(has_lead);
        let mut lay = |k: usize, l: &layout::Limits| {
            (self.items[k].as_widget_mut()).layout(&mut tree.children[k], renderer, l)
        };
        // The words and the number side by side, the words wrapped in the
        // room the number leaves them; over it when that is under a third.
        let mut number = lay(at, &free);
        if number.size().width > max {
            number = lay(at, &within(max));
        }
        let (num_w, num_h) = (number.size().width, number.size().height);
        let mut line = vec![];
        let (line_w, line_h) = if has_lead {
            let mut lead = lay(0, &free);
            let room = max - Self::GAP - num_w;
            if lead.size().width > room && room >= max / 3. {
                lead = lay(0, &within(room));
            }
            let (lead_w, lead_h) = (lead.size().width, lead.size().height);
            if lead_w <= room {
                let h = lead_h.max(num_h);
                line.push(lead.move_to(Point::new(0., (h - lead_h) / 2.)));
                let x = lead_w + Self::GAP;
                line.push(number.move_to(Point::new(x, (h - num_h) / 2.)));
                (x + num_w, h)
            } else {
                let lead = lay(0, &within(max));
                let lead_h = lead.size().height;
                let w = lead.size().width.max(num_w);
                line.push(lead);
                line.push(number.move_to(Point::new(0., lead_h + Self::STACK)));
                (w, lead_h + Self::STACK + num_h)
            }
        } else {
            line.push(number);
            (num_w, num_h)
        };
        // Each tag at its one-line width: beside the line while it has room
        // there, else under.
        let mut beside = vec![];
        let mut below = vec![];
        for k in at + 1..count {
            let one_line = lay(k, &free);
            let room = line_w + Self::GAP + one_line.size().width <= max;
            if k <= at + side && room {
                beside.push((k, one_line));
            } else {
                below.push(k);
            }
        }
        let side_w = beside
            .iter()
            .map(|(_, n)| n.size().width)
            .fold(0., f32::max);
        let side_h = beside.iter().map(|(_, n)| n.size().height).sum::<f32>()
            + Self::STACK * beside.len().saturating_sub(1) as f32;
        let width = match max.is_finite() {
            true => max,
            false => line_w + Self::GAP + side_w,
        };
        let top = line_h.max(side_h);
        let mut nodes: Vec<Option<layout::Node>> = (0..count).map(|_| None).collect();
        let line_y = (top - line_h) / 2.;
        for (k, n) in line.into_iter().enumerate() {
            nodes[k] = Some(n.translate(iced::Vector::new(0., line_y)));
        }
        let mut y = (top - side_h) / 2.;
        for (k, n) in beside {
            let x = width - n.size().width;
            let h = n.size().height;
            nodes[k] = Some(n.move_to(Point::new(x, y)));
            y += h + Self::STACK;
        }
        let mut y = top;
        for k in below {
            let n = lay(k, &within(width));
            y += Self::STACK;
            let h = n.size().height;
            nodes[k] = Some(n.move_to(Point::new(0., y)));
            y += h;
        }
        let children = nodes.into_iter().map(|n| n.expect("every item placed"));
        layout::Node::with_children(Size::new(width, y), children.collect())
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &iced::advanced::renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        for ((item, t), l) in self.items.iter().zip(&tree.children).zip(layout.children()) {
            (item.as_widget()).draw(t, renderer, theme, style, l, cursor, viewport);
        }
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.traverse(&mut |operation| {
            for ((item, t), l) in (self.items.iter_mut())
                .zip(&mut tree.children)
                .zip(layout.children())
            {
                (item.as_widget_mut()).operate(t, l, renderer, operation);
            }
        });
    }
}
/// The tiles of a section's passes or holes: done, now, next. On the
/// tablet, a tile a tap goes nowhere from is a plain label in a strip, the
/// current one lit, no box (LIO-544); one that goes to its pass is a box a
/// finger tall, which the tablet finds by its tap (LIO-525).
fn tiles<'a>(
    labels: Vec<(String, Option<Message>)>,
    at: usize,
    tablet: bool,
) -> Element<'a, Message> {
    let mut lines = column![].spacing(6);
    let mut line = row![].spacing(6);
    let count = labels.len();
    for (i, (s, msg)) in labels.into_iter().enumerate() {
        let state = lit(i, at);
        if tablet && msg.is_none() {
            let label = text(s).size(13).font(Font::MONOSPACE).color(match state {
                Lit::Done => MUTED,
                Lit::Now => ACCENT,
                Lit::Next => INK,
            });
            let under = container(space().width(Fill).height(2)).style(move |_| container::Style {
                background: (state == Lit::Now).then(|| ACCENT.into()),
                ..Default::default()
            });
            line = line.push(
                column![container(label).center_x(Fill), under]
                    .spacing(2)
                    .width(FillPortion(1)),
            );
            if (i + 1) % 3 == 0 || i + 1 == count {
                lines = lines.push(line);
                line = row![].spacing(6);
            }
            continue;
        }
        let face = container(
            text(s)
                .size(13)
                .font(Font::MONOSPACE)
                .color(if state == Lit::Now { ACCENT } else { INK }),
        )
        .center_x(Fill)
        .padding([if tablet { 14 } else { 6 }, 0])
        .style(move |_| container::Style {
            background: Some(
                if state == Lit::Done {
                    CHIP
                } else {
                    Color::WHITE
                }
                .into(),
            ),
            border: Border {
                color: if state == Lit::Now { ACCENT } else { LINE },
                width: if state == Lit::Now { 2. } else { 1. },
                radius: 6.into(),
            },
            ..Default::default()
        });
        line = line.push(container(tap(face, msg, "Pass")).width(FillPortion(1)));
        // At most three readable depth/row labels per line, even on the
        // narrow Steps pane. Keep every pass directly selectable.
        if (i + 1) % 3 == 0 || i + 1 == count {
            lines = lines.push(line);
            line = row![].spacing(6);
        }
    }
    lines.into()
}
/// Back (dimmed when there is no step before) and the step's own command,
/// large enough to tap with a glove on.
fn buttons<'a>(next: Option<(String, Message)>, back: bool) -> Element<'a, Message> {
    let back = HelpButton::new(
        button(text("◀ Back").size(16).width(Fill).center())
            .width(Fill)
            .padding([14, 12])
            .on_press_maybe(back.then_some(Message::Guide(GuideAction::Back)))
            .style(|_, status| button::Style {
                background: Some(
                    if matches!(status, button::Status::Hovered) {
                        HOVER_BUTTON
                    } else {
                        Color::WHITE
                    }
                    .into(),
                ),
                text_color: if matches!(status, button::Status::Disabled) {
                    LINE
                } else {
                    INK
                },
                border: Border {
                    color: BUTTON_EDGE,
                    width: 1.,
                    radius: 6.into(),
                },
                ..Default::default()
            }),
        &Message::Guide(GuideAction::Back),
        "◀ Back",
    );
    let mut line = row![container(back).width(FillPortion(1))].spacing(8);
    if let Some((label, msg)) = next {
        let go = button(text(label.clone()).size(16).width(Fill).center())
            .width(Fill)
            .padding([14, 12])
            .on_press(msg.clone())
            .style(|_, status| button::Style {
                background: Some(
                    if matches!(status, button::Status::Hovered) {
                        Color::from_rgb8(32, 97, 155)
                    } else {
                        BLUE
                    }
                    .into(),
                ),
                text_color: Color::WHITE,
                border: Border {
                    radius: 6.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        line = line
            .push(container(HelpButton::new(go, &msg, label).width(Fill)).width(FillPortion(2)));
    }
    line.into()
}
/// The padlock beside a readout's "lock" tag.
struct Lock;
impl canvas::Program<Message> for Lock {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &iced::Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut f = canvas::Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        let shackle = canvas::Path::new(|b| {
            b.move_to(Point::new(w * 0.25, h * 0.5));
            b.line_to(Point::new(w * 0.25, h * 0.2));
            b.line_to(Point::new(w * 0.5, h * 0.05));
            b.line_to(Point::new(w * 0.75, h * 0.2));
            b.line_to(Point::new(w * 0.75, h * 0.5));
        });
        f.stroke(&shackle, stroke(MUTED, 1.4));
        f.fill_rectangle(Point::new(0., h * 0.45), Size::new(w, h * 0.55), MUTED);
        vec![f.into_geometry()]
    }
}
fn stroke<'a>(c: Color, w: f32) -> canvas::Stroke<'a> {
    canvas::Stroke::default().with_color(c).with_width(w)
}
fn dashed<'a>(c: Color, w: f32) -> canvas::Stroke<'a> {
    canvas::Stroke {
        line_dash: canvas::LineDash {
            segments: &[6., 4.],
            offset: 0,
        },
        ..stroke(c, w)
    }
}
fn arrow(f: &mut canvas::Frame, a: Point, b: Point, c: Color, w: f32) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l = (dx * dx + dy * dy).sqrt();
    if l < 2. {
        return;
    }
    let (ux, uy) = (dx / l, dy / l);
    // A heavier line, a larger head.
    let head = (3.6 * w).max(9.).min(l);
    let base = Point::new(b.x - ux * head, b.y - uy * head);
    f.stroke(&canvas::Path::line(a, base), stroke(c, w));
    let tip = canvas::Path::new(|p| {
        p.move_to(b);
        p.line_to(Point::new(
            base.x - uy * head * 0.5,
            base.y + ux * head * 0.5,
        ));
        p.line_to(Point::new(
            base.x + uy * head * 0.5,
            base.y - ux * head * 0.5,
        ));
        p.close();
    });
    f.fill(&tip, c);
}
use iced::advanced::text::Alignment as Align;
fn write(
    f: &mut canvas::Frame,
    s: impl Into<String>,
    at: Point,
    c: Color,
    size: f32,
    align: Align,
) {
    let s = s.into();
    #[cfg(test)]
    WRITTEN.with_borrow_mut(|w| w.push((s.clone(), text_box(&s, at, size, align))));
    f.fill_text(canvas::Text {
        content: s,
        position: at,
        color: c,
        size: size.into(),
        align_x: align,
        align_y: iced::alignment::Vertical::Center,
        ..Default::default()
    });
}
/// Where `s` written `size` px high at `at`, `align`ed, lies on a canvas.
// ponytail: a letter taken as 0.6 of the size wide, not measured; widen it
// if a font's digits run wider.
fn text_box(s: &str, at: Point, size: f32, align: Align) -> Rectangle {
    let (w, h) = (s.chars().count() as f32 * size * 0.6, size * 1.2);
    let x = match align {
        Align::Right => at.x - w,
        Align::Center => at.x - w / 2.,
        _ => at.x,
    };
    Rectangle::new(Point::new(x, at.y - h / 2.), Size::new(w, h))
}
/// The labels a drawing has written, so that each next one keeps clear of
/// them (LIO-534: on a narrow picture a slot pass wrote "X 59.00" over
/// "X 81.00"), as the Check's dimensions do (`dim_tags`).
struct Labels {
    room: Size,
    taken: Vec<Rectangle>,
    /// What each label `put` said, and where.
    said: Vec<(String, Rectangle)>,
}
impl Labels {
    /// Whether `r` lies on the canvas, 2 px clear of every label before it.
    fn free(&self, r: &Rectangle) -> bool {
        let (w, h) = (self.room.width, self.room.height);
        r.x >= 0.
            && r.y >= 0.
            && r.x + r.width <= w
            && r.y + r.height <= h
            && (self.taken.iter()).all(|o| {
                r.x + r.width + 2. <= o.x
                    || o.x + o.width + 2. <= r.x
                    || r.y + r.height + 2. <= o.y
                    || o.y + o.height + 2. <= r.y
            })
    }
    /// The first of `boxes`, in order, that is free, or failing that the
    /// first free with all of them slid `a` times `step` and `b` times
    /// `across` either way, fewest moves first and along `step` first, up
    /// to 40 moves: its index and how far it slid. `None` when none is.
    // ponytail: up to 40² tries × the boxes taken when nothing is free;
    // fine for a drawing's dozen labels.
    fn slide(
        &self,
        boxes: &[Rectangle],
        step: iced::Vector,
        across: iced::Vector,
    ) -> Option<(usize, iced::Vector)> {
        (0..40)
            .flat_map(|n: i32| (0..=n).rev().map(move |a| (a, n - a)))
            .flat_map(|(a, b)| [b, -b].map(|b| step * a as f32 + across * b as f32))
            .flat_map(|d| (0..boxes.len()).map(move |i| (i, d)))
            .find(|(i, d)| self.free(&(boxes[*i] + *d)))
    }
    /// Writes `s` at the first of `tries` (a point and how the words align
    /// on it) where it is clear (`slide`), or at the first as it is; not at
    /// all when a label before it says the same within a line of one of
    /// `tries` or of where it slid to, as both sides of a slot as wide as
    /// its cutter read one X (LIO-545, LIO-546).
    fn put(
        &mut self,
        f: &mut canvas::Frame,
        s: impl Into<String>,
        tries: &[(Point, Align)],
        step: iced::Vector,
        c: Color,
        size: f32,
    ) {
        let s = s.into();
        let boxes: Vec<Rectangle> = (tries.iter())
            .map(|(at, al)| text_box(&s, *at, size, *al))
            .collect();
        // Slid along `step` and across it: a label hemmed in by lines and
        // the stock's edge goes past the nearest of them (LIO-546).
        let across = iced::Vector::new(step.y, step.x);
        let (i, d) = (self.slide(&boxes, step, across)).unwrap_or((0, iced::Vector::new(0., 0.)));
        let at = boxes[i] + d;
        let near =
            |o: &Rectangle| (boxes.iter().chain([&at])).any(|b| b.intersects(&o.expand(o.height)));
        let twin = |(t, o): &(String, Rectangle)| *t == s && near(o);
        if self.said.iter().any(twin) {
            return;
        }
        self.taken.push(at);
        self.said.push((s.clone(), at));
        write(f, s, tries[i].0 + d, c, size, tries[i].1);
    }
    /// Takes the box `r` (a tag), slid on along `step` until it is clear,
    /// and gives where.
    fn take(&mut self, r: Rectangle, step: iced::Vector) -> Rectangle {
        let (_, d) = self
            .slide(&[r], step, iced::Vector::new(0., 0.))
            .unwrap_or((0, iced::Vector::new(0., 0.)));
        self.taken.push(r + d);
        r + d
    }
}
#[cfg(test)]
thread_local! {
    /// The words `write` wrote since the drawing being painted began.
    static WRITTEN: std::cell::RefCell<Vec<(String, Rectangle)>> = Default::default();
    /// Each Steps drawing's words as painted, for the tests.
    static DRAWN: std::cell::RefCell<Vec<Vec<(String, Rectangle)>>> = Default::default();
}
/// A drawing's painting begins (`true`) or ends: the words written between
/// are its own (tests only).
#[cfg(test)]
fn drawing_words(begins: bool) {
    match begins {
        true => drop(WRITTEN.take()),
        false => DRAWN.with_borrow_mut(|d| d.push(WRITTEN.take())),
    }
}
#[cfg(not(test))]
fn drawing_words(_: bool) {}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Hatch {
    /// What this step cuts.
    Cut,
    /// What was cut before it.
    Earlier,
    /// What the passes after it cut: an outline.
    Later,
}
/// What a drawing shows over the stock, in work coordinates.
#[derive(Debug, Clone)]
enum Shape {
    Hatch(Bounds, Hatch),
    /// What a cut takes in a side view, by its outline: a centre drill's
    /// pilot and cone.
    Section(Vec<[f32; 3]>),
    /// A line not to pass, labelled with its DRO number.
    Never([f32; 3], [f32; 3], String),
    /// The depth a hole is cut to: a line not to pass, its label right of
    /// its right end, clear of the hole drawn across it (LIO-417).
    Depth([f32; 3], [f32; 3], String),
    /// A finished wall.
    Wall([f32; 3], [f32; 3]),
    /// The tool's centre at the start of the cut, its radius and the end.
    Tool([f32; 3], f32, Option<[f32; 3]>),
    /// An edge finder touching at a point, from a direction, labelled.
    Finder([f32; 3], [f32; 2], String),
    /// The round an edge finder touches: its centre and radius, labelled
    /// with its diameter.
    Round([f32; 3], f32, String),
    /// A hole or a round corner: its centre, radius and number.
    Hole([f32; 3], f32, usize),
    /// A slot from above: the centres of its round ends, its radius and
    /// its number (LIO-688).
    Slot([f32; 3], [f32; 3], f32, usize),
    /// A pattern the DRO leads to, its holes (centres, the DRO's order) and
    /// their radius, in the top view: the holes numbered and their order,
    /// hole 1 marked, the pitches or the circle with its centre and first
    /// angle.
    Pattern(Pattern, Vec<[f32; 2]>, f32, Vec<usize>),
    /// A word at a point.
    Word([f32; 3], String, Color),
    /// A solid block, such as a parallel under the stock.
    Block(Bounds),
    /// A clamp from above, over the stock, labelled at its outer end
    /// (LIO-458).
    Clamp(Bounds, String),
    /// An arrow `mm` long down onto a point, labelled at its tail.
    Down([f32; 3], f32, String),
    /// A height from `[0]` down to `[1]`, measured just right of them,
    /// labelled on its right.
    Height([f32; 3], [f32; 3], String),
}
/// The top view or a side view of a step, drawn once per step
/// (`canvas::Cache`): the stock box, the part, the jaws, the cuts, the
/// tool, the lines not to pass, the work zero and the axes.
struct Drawing<'a> {
    /// What it shows: a new key draws it again.
    key: u64,
    /// The horizontal and vertical axes, one of `VIEWS`.
    axes: [usize; 2],
    stock: Bounds,
    part: &'a [Vec<[f32; 2]>],
    jaws: &'a [Bounds],
    shapes: Vec<Shape>,
}
impl Drawing<'_> {
    fn paint(&self, f: &mut canvas::Frame) {
        drawing_words(true);
        let [a, b] = self.axes;
        let (mut lo, mut hi) = (
            [self.stock.min[a], self.stock.min[b]],
            [self.stock.max[a], self.stock.max[b]],
        );
        let mut grow = |p: [f32; 2]| {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        };
        for j in self.jaws {
            grow([j.min[a], j.min[b]]);
            grow([j.max[a], j.max[b]]);
        }
        for s in &self.shapes {
            match s {
                Shape::Tool(c, r, end) => {
                    for p in std::iter::once(c).chain(end) {
                        let up = if b == 2 { 2. * r } else { *r };
                        grow([p[a] - r, p[b] - if b == 2 { 0. } else { *r }]);
                        grow([p[a] + r, p[b] + up]);
                    }
                }
                Shape::Finder(p, _, _) => {
                    grow([p[a] - 12., p[b] - 12.]);
                    grow([p[a] + 12., p[b] + 12.]);
                }
                Shape::Down(p, mm, _) => {
                    grow([p[a], p[b]]);
                    grow([p[a], p[b] + mm]);
                }
                Shape::Block(bx) | Shape::Clamp(bx, _) => {
                    grow([bx.min[a], bx.min[b]]);
                    grow([bx.max[a], bx.max[b]]);
                }
                Shape::Section(q) => q.iter().for_each(|q| grow([q[a], q[b]])),
                _ => {}
            }
        }
        let size = f.size();
        let (left, right, top, bottom) = (34., 74., 14., 34.);
        let span = [(hi[0] - lo[0]).max(1.), (hi[1] - lo[1]).max(1.)];
        let s = ((size.width - left - right) / span[0]).min((size.height - top - bottom) / span[1]);
        let ox = left + (size.width - left - right - span[0] * s) / 2.;
        let oy = size.height - bottom - (size.height - top - bottom - span[1] * s) / 2.;
        let p = |q: [f32; 2]| Point::new(ox + (q[0] - lo[0]) * s, oy - (q[1] - lo[1]) * s);
        let p3 = |q: [f32; 3]| p([q[a], q[b]]);
        // At least 3 px across, so a thin slab still shows.
        let rect = |bx: &Bounds| {
            let (mut x0, mut x1) = (p([bx.min[a], 0.]).x, p([bx.max[a], 0.]).x);
            let (mut y0, mut y1) = (p([0., bx.max[b]]).y, p([0., bx.min[b]]).y);
            if x1 - x0 < 3. {
                let m = (x0 + x1) / 2.;
                (x0, x1) = (m - 1.5, m + 1.5);
            }
            if y1 - y0 < 3. {
                let m = (y0 + y1) / 2.;
                (y0, y1) = (m - 1.5, m + 1.5);
            }
            Rectangle::new(Point::new(x0, y0), Size::new(x1 - x0, y1 - y0))
        };
        let path = |r: Rectangle| canvas::Path::rectangle(r.position(), r.size());
        for j in self.jaws {
            let r = rect(j);
            f.fill(&path(r), JAW);
            f.stroke(&path(r), stroke(MUTED, 1.));
        }
        let stock = rect(&self.stock);
        f.fill(&path(stock), STOCK);
        for face in self.part {
            let outline = canvas::Path::new(|b| {
                b.move_to(p(face[0]));
                for q in &face[1..] {
                    b.line_to(p(*q));
                }
                b.close();
            });
            f.fill(&outline, PART);
        }
        f.stroke(&path(stock), stroke(INK, 1.3));
        // The axes' names in the bottom-left corner, written last, first
        // in the way of the labels.
        let corner = Point::new(10., size.height - 10.);
        let names = [
            (AXES[a], Point::new(40., corner.y)),
            (AXES[b], Point::new(corner.x - 4., corner.y - 34.)),
        ];
        // The work zero's mark too (LIO-545: "Z −7.00" ran over it).
        let zero = p3([0.; 3]);
        let mark = Rectangle::new(zero - iced::Vector::new(6., 6.), Size::new(12., 12.));
        let names_at = names
            .iter()
            .map(|(s, at)| text_box(s, *at, 12., Align::Left));
        // And the lines not to pass and the stock's outline, 1 px wide (a
        // label keeps 2 px clear), so that no label is written across one
        // (LIO-546: the Z −7 line ran through "X 60.00", the stock's top
        // through "Z −7.00").
        let line = |m: Point, n: Point| {
            let (w, h) = ((m.x - n.x).abs(), (m.y - n.y).abs());
            Rectangle::new(
                Point::new(m.x.min(n.x) - 0.5, m.y.min(n.y) - 0.5),
                Size::new(w + 1., h + 1.),
            )
        };
        let (tl, br) = (
            stock.position(),
            stock.position() + iced::Vector::new(stock.width, stock.height),
        );
        let corners = [tl, Point::new(br.x, tl.y), br, Point::new(tl.x, br.y), tl];
        let edges = corners
            .windows(2)
            .map(|e| ("stock".to_string(), line(e[0], e[1])));
        let lines = (self.shapes.iter())
            .filter_map(|s| match s {
                Shape::Never(m, n, s) | Shape::Depth(m, n, s) => {
                    Some((s.clone(), line(p3(*m), p3(*n))))
                }
                _ => None,
            })
            .chain(edges);
        #[cfg(test)]
        WRITTEN.with_borrow_mut(|w| w.extend(lines.clone().map(|(s, r)| (format!("line {s}"), r))));
        let taken = names_at
            .chain([mark])
            .chain(lines.map(|(_, r)| r))
            .collect();
        let mut labels = Labels {
            room: size,
            taken,
            said: vec![],
        };
        let (down, up, leftward) = (
            iced::Vector::new(0., 7.),
            iced::Vector::new(0., -7.),
            iced::Vector::new(-7., 0.),
        );
        // A side, a level, on the side away from the stock's middle first
        // (LIO-534).
        let middle = stock.center();
        for shape in &self.shapes {
            match shape {
                Shape::Hatch(bx, kind) => {
                    let r = rect(bx);
                    match kind {
                        Hatch::Later => {
                            f.stroke(&path(r), dashed(Color { a: 0.6, ..ACCENT }, 1.));
                            continue;
                        }
                        Hatch::Cut => f.fill(&path(r), Color { a: 0.14, ..ACCENT }),
                        Hatch::Earlier => {}
                    }
                    let (c, w) = match kind {
                        Hatch::Cut => (ACCENT, 1.6),
                        _ => (Color { a: 0.55, ..MUTED }, 1.),
                    };
                    let mut k = r.x + r.y;
                    while k < r.x + r.width + r.y + r.height {
                        let x0 = r.x.max(k - (r.y + r.height));
                        let x1 = (r.x + r.width).min(k - r.y);
                        if x1 > x0 {
                            f.stroke(
                                &canvas::Path::line(Point::new(x0, k - x0), Point::new(x1, k - x1)),
                                stroke(c, w),
                            );
                        }
                        k += 7.;
                    }
                    f.stroke(
                        &path(r),
                        stroke(c, if *kind == Hatch::Cut { 1.5 } else { 0.8 }),
                    );
                }
                Shape::Wall(from, to) => {
                    f.stroke(&canvas::Path::line(p3(*from), p3(*to)), stroke(INK, 2.5))
                }
                Shape::Section(q) if q.is_empty() => {}
                Shape::Section(q) => {
                    let outline = canvas::Path::new(|b| {
                        b.move_to(p3(q[0]));
                        q[1..].iter().for_each(|q| b.line_to(p3(*q)));
                        b.close();
                    });
                    f.fill(&outline, Color { a: 0.14, ..ACCENT });
                    f.stroke(&outline, stroke(ACCENT, 1.5));
                }
                Shape::Depth(from, to, s) => {
                    let (m, n) = (p3(*from), p3(*to));
                    f.stroke(&canvas::Path::line(m, n), dashed(NEVER, 1.8));
                    let at = depth_label(m, n);
                    let other = Point::new(m.x.min(n.x) - 5., at.y);
                    let tries = [(at, Align::Left), (other, Align::Right)];
                    labels.put(f, s.clone(), &tries, down, NEVER, 12.);
                }
                Shape::Never(from, to, s) => {
                    let (m, n) = (p3(*from), p3(*to));
                    f.stroke(&canvas::Path::line(m, n), dashed(NEVER, 1.8));
                    // Inside the stock: over or under a level's right end,
                    // beside a side's top.
                    if (m.x - n.x).abs() > (m.y - n.y).abs() && (m.x - n.x).abs() < 80. {
                        // A short line (a hole's floor): its label right of it.
                        let tries = [
                            (Point::new(m.x.max(n.x) + 5., m.y), Align::Left),
                            (Point::new(m.x.min(n.x) - 5., m.y), Align::Right),
                        ];
                        labels.put(f, s.clone(), &tries, down, NEVER, 12.);
                    } else if (m.x - n.x).abs() > (m.y - n.y).abs() {
                        let x = m.x.max(n.x) - 8.;
                        let mut tries = [
                            (Point::new(x, m.y - 10.), Align::Right),
                            (Point::new(x, m.y + 10.), Align::Right),
                        ];
                        if m.y > middle.y {
                            tries.reverse();
                        }
                        labels.put(f, s.clone(), &tries, leftward, NEVER, 12.);
                    } else {
                        let y = m.y.min(n.y) + 12.;
                        let mut tries = [
                            (Point::new(m.x + 5., y), Align::Left),
                            (Point::new(m.x - 5., y), Align::Right),
                        ];
                        if m.x < middle.x {
                            tries.reverse();
                        }
                        labels.put(f, s.clone(), &tries, down, NEVER, 12.);
                    }
                }
                Shape::Tool(c, r, end) => {
                    let at = p3(*c);
                    let rs = r * s;
                    let body = if b == 2 {
                        let up = (2. * r).min(30.) * s;
                        canvas::Path::rectangle(
                            Point::new(at.x - rs, at.y - up),
                            Size::new(2. * rs, up),
                        )
                    } else {
                        canvas::Path::circle(at, rs)
                    };
                    f.fill(&body, Color { a: 0.25, ..TOOL });
                    f.stroke(&body, stroke(TOOL, 1.6));
                    f.fill(&canvas::Path::circle(at, 2.), TOOL);
                    if let Some(end) = end {
                        arrow(f, at, p3(*end), TOOL, 2.);
                    }
                }
                Shape::Finder(q, from, s) => {
                    let at = p3(*q);
                    let r = 6.;
                    let c = Point::new(at.x + from[0] * r, at.y - from[1] * r);
                    let body = canvas::Path::circle(c, r);
                    f.fill(&body, Color { a: 0.25, ..TOOL });
                    f.stroke(&body, stroke(TOOL, 1.6));
                    let tail = Point::new(c.x + from[0] * 30., c.y - from[1] * 30.);
                    arrow(
                        f,
                        tail,
                        Point::new(c.x + from[0] * r, c.y - from[1] * r),
                        TOOL,
                        2.,
                    );
                    if from[0] != 0. {
                        // Outside the stock, clear of its face.
                        let at = Point::new(c.x + from[0] * 4., c.y - 16.);
                        let align = if from[0] < 0. {
                            Align::Right
                        } else {
                            Align::Left
                        };
                        labels.put(f, s.clone(), &[(at, align)], up, INK, 13.);
                    } else {
                        // Right of it, or left of it when it is well left
                        // of the stock's middle: away from what is there,
                        // a clamp it was moved clear of (LIO-458).
                        let middle = rect(&self.stock);
                        let left = c.x < middle.center_x() - middle.width / 4.;
                        let mut tries = [
                            (Point::new(c.x + 14., c.y), Align::Left),
                            (Point::new(c.x - 14., c.y), Align::Right),
                        ];
                        if left {
                            tries.reverse();
                        }
                        labels.put(f, s.clone(), &tries, down, INK, 13.);
                    }
                }
                Shape::Hole(q, r, n) => {
                    let at = p3(*q);
                    let rs = (r * s).max(4.);
                    let body = canvas::Path::circle(at, rs);
                    f.fill(&body, Color { a: 0.18, ..ACCENT });
                    f.stroke(&body, stroke(ACCENT, 2.));
                    let tries = [
                        (Point::new(at.x + rs + 5., at.y - 9.), Align::Left),
                        (Point::new(at.x - rs - 5., at.y - 9.), Align::Right),
                        (Point::new(at.x - 4., at.y - rs - 20.), Align::Left),
                        (Point::new(at.x - 4., at.y + rs + 2.), Align::Left),
                    ];
                    labels.put(f, n.to_string(), &tries, up, ACCENT, 13.);
                }
                Shape::Pattern(pattern, holes, r, numbers) => {
                    paint_pattern(f, &mut labels, p, s, pattern, holes, *r, numbers)
                }
                Shape::Word(q, s, c) => {
                    labels.put(f, s.clone(), &[(p3(*q), Align::Left)], down, *c, 13.)
                }
                Shape::Round(q, r, label) => {
                    let at = p3(*q);
                    f.stroke(&canvas::Path::circle(at, r * s), stroke(ACCENT, 2.));
                    // On its back-right, off the finders' lines.
                    let d = r * s * std::f32::consts::FRAC_1_SQRT_2;
                    let at = Point::new(at.x + d + 4., at.y - d - 14.);
                    labels.put(f, label.clone(), &[(at, Align::Left)], up, ACCENT, 13.);
                }
                Shape::Block(bx) => {
                    let r = rect(bx);
                    f.fill(&path(r), DONE);
                    f.stroke(&path(r), stroke(INK, 1.));
                }
                Shape::Clamp(bx, label) => {
                    let r = rect(bx);
                    f.fill(&path(r), CLAMP);
                    f.stroke(&path(r), stroke(INK, 1.4));
                    // Beside its end out of the stock: right of it on the
                    // drawing's bottom or top side, over it on its left or
                    // right side.
                    let (c, st) = (r.center(), rect(&self.stock));
                    let m = st.center();
                    let across = (c.y - m.y).abs() / st.height.max(1.)
                        > (c.x - m.x).abs() / st.width.max(1.);
                    let (at, align) = if across {
                        let y = if c.y < m.y { r.y } else { r.y + r.height - 14. };
                        (Point::new(r.x + r.width + 5., y), Align::Left)
                    } else if c.x < m.x {
                        (Point::new(r.x, r.y - 16.), Align::Left)
                    } else {
                        (Point::new(r.x + r.width, r.y - 16.), Align::Right)
                    };
                    labels.put(f, label.clone(), &[(at, align)], up, INK, 12.);
                }
                Shape::Down(q, mm, s) => {
                    let mut t = *q;
                    t[b] += mm;
                    let (head, tail) = (p3(*q), p3(t));
                    arrow(f, tail, head, ACCENT, 2.5);
                    let tries = [
                        (Point::new(tail.x + 8., tail.y + 6.), Align::Left),
                        (Point::new(tail.x - 8., tail.y + 6.), Align::Right),
                    ];
                    labels.put(f, s.clone(), &tries, up, ACCENT, 13.);
                }
                Shape::Slot(from, to, r, n) => {
                    let (m, e) = (p3(*from), p3(*to));
                    let rs = (r * s).max(3.);
                    let way = canvas::Path::line(m, e);
                    let round = |c: Color, w: f32| canvas::Stroke {
                        line_cap: canvas::LineCap::Round,
                        ..stroke(c, w)
                    };
                    f.stroke(&way, round(ACCENT, 2. * rs + 3.));
                    f.stroke(&way, round(Color::WHITE, 2. * rs));
                    f.stroke(&way, round(Color { a: 0.18, ..ACCENT }, 2. * rs));
                    let at = Point::new(m.x.max(e.x), m.y.min(e.y));
                    let tries = [
                        (Point::new(at.x + rs + 5., at.y - rs - 9.), Align::Left),
                        (Point::new(at.x - rs - 5., at.y - rs - 9.), Align::Right),
                    ];
                    labels.put(f, n.to_string(), &tries, up, ACCENT, 12.);
                }
                Shape::Height(top, bottom, s) => {
                    let (m, n) = (p3(*top), p3(*bottom));
                    let x = m.x + 7.;
                    for y in [m.y, n.y] {
                        let tick = canvas::Path::line(Point::new(x - 5., y), Point::new(x + 5., y));
                        f.stroke(&tick, stroke(ACCENT, 1.6));
                    }
                    let span = canvas::Path::line(Point::new(x, m.y), Point::new(x, n.y));
                    f.stroke(&span, stroke(ACCENT, 1.6));
                    let y = (m.y + n.y) / 2.;
                    let tries = [
                        (Point::new(x + 9., y), Align::Left),
                        (Point::new(x - 9., y), Align::Right),
                    ];
                    labels.put(f, s.clone(), &tries, down, ACCENT, 13.);
                }
            }
        }
        // The work zero and the axes.
        let mark = canvas::Path::circle(zero, 5.);
        f.fill(&mark, Color::WHITE);
        f.stroke(&mark, stroke(INK, 1.4));
        f.stroke(
            &canvas::Path::line(
                Point::new(zero.x - 5., zero.y),
                Point::new(zero.x + 5., zero.y),
            ),
            stroke(INK, 1.),
        );
        f.stroke(
            &canvas::Path::line(
                Point::new(zero.x, zero.y - 5.),
                Point::new(zero.x, zero.y + 5.),
            ),
            stroke(INK, 1.),
        );
        arrow(f, corner, Point::new(36., corner.y), MUTED, 1.3);
        arrow(f, corner, Point::new(corner.x, corner.y - 26.), MUTED, 1.3);
        for (s, at) in names {
            write(f, s, at, MUTED, 12., Align::Left);
        }
        drawing_words(false);
    }
}
/// Where a pattern drawing's tag of `chars` letters goes on a canvas of
/// `size`: out from `from` along `out`, clear of it, and kept whole inside
/// the canvas, pushed in from an edge it would cross (LIO-407).
fn tag_box(from: Point, out: [f32; 2], chars: usize, k: f32, size: Size) -> Rectangle {
    let (w, h) = (chars as f32 * 9. * k + 16., 26. * k);
    let reach = 8. + out[0].abs() * w / 2. + out[1].abs() * h / 2.;
    let c = Point::new(from.x + out[0] * reach, from.y - out[1] * reach);
    let x = (c.x - w / 2.).clamp(2., (size.width - w - 2.).max(2.));
    let y = (c.y - h / 2.).clamp(2., (size.height - h - 2.).max(2.));
    Rectangle::new(Point::new(x, y), Size::new(w, h))
}
/// Where a hole's depth line from `m` to `n` puts its label: right of its
/// right end, past the hole the line crosses (LIO-417).
fn depth_label(m: Point, n: Point) -> Point {
    Point::new(m.x.max(n.x) + 5., (m.y + n.y) / 2.)
}
/// The lowest Z of the part, work coordinates: a hole whose tip goes below
/// it goes through.
// ponytail: the part's lowest point, not the bottom under each hole; a part
// with a step under a blind hole reads it as blind, which is the safe side.
fn part_bottom(p: &Page<'_>) -> f32 {
    (p.shown.part[1].iter().flatten())
        .map(|q| q[1])
        .fold(f32::INFINITY, f32::min)
}
/// A Drill's warning when its tip comes out under the stock, how far
/// (LIO-417).
/// With `notes`, the setup's notes say where the supports go: said so.
fn under_stock<'a>(tip: f32, stock: &Bounds, notes: bool) -> Option<Element<'a, Message>> {
    let under = stock.min[2] - tip;
    (under > 0.005).then(|| {
        let mut said = format!(
            "comes out {} under the stock: nothing under the hole",
            mm(under)
        );
        if notes {
            said += &format!("; {SEE_NOTES}");
        }
        warn_chip(said)
    })
}
/// `Shape::Pattern` in the top view, `at` from mm to the canvas at `s` px
/// a mm: the order the DRO goes in as arrows from hole to hole (dashed onto
/// the next row), the pitches as sizes off hole 1, or the circle with its
/// centre, its diameter and the angle from +X round to hole 1; then the
/// holes numbered, hole 1 in the accent.
#[allow(clippy::too_many_arguments)]
fn paint_pattern(
    f: &mut canvas::Frame,
    labels: &mut Labels,
    at: impl Fn([f32; 2]) -> Point,
    s: f32,
    pattern: &Pattern,
    holes: &[[f32; 2]],
    r: f32,
    numbers: &[usize],
) {
    if holes.is_empty() {
        return;
    }
    let rs = (r * s).max(6.);
    // The words a little smaller on a small drawing.
    let k = (s / 4.).clamp(0.8, 1.);
    let pts: Vec<Point> = holes.iter().map(|h| at(*h)).collect();
    // `px` on from `q` along the direction `d` (mm axes: +Y is up).
    let on = |q: Point, d: [f32; 2], px: f32| Point::new(q.x + d[0] * px, q.y - d[1] * px);
    let unit = |deg: f32| {
        let t = deg.to_radians();
        [t.cos(), t.sin()]
    };
    // The screen's way `px` along `d`.
    let along = |d: [f32; 2], px: f32| iced::Vector::new(d[0] * px, -d[1] * px);
    // A label on a white tag, `from` a point out along `out`, clear of it
    // and of the labels before it.
    let tag =
        |f: &mut canvas::Frame, labels: &mut Labels, from: Point, out: [f32; 2], label: String| {
            let place = tag_box(from, out, label.chars().count(), k, f.size());
            let place = labels.take(place, along(out, 6.));
            let c = place.center();
            let r = canvas::Path::rounded_rectangle(place.position(), place.size(), 5.into());
            // A white tag hides the lines under it (tests: LIO-546).
            #[cfg(test)]
            WRITTEN.with_borrow_mut(|w| w.push(("tag".into(), place)));
            f.fill(&r, Color::WHITE);
            f.stroke(&r, stroke(ACCENT, 1.5));
            write(f, label, c, ACCENT, 16. * k, Align::Center);
        };
    // A size from `a` to `b`, `gap` px off the holes along `out`.
    let size = |f: &mut canvas::Frame,
                labels: &mut Labels,
                a: Point,
                b: Point,
                out: [f32; 2],
                label: String| {
        let gap = rs + 22.;
        let (m, n) = (on(a, out, gap), on(b, out, gap));
        for (h, e) in [(a, m), (b, n)] {
            let tick = canvas::Path::line(on(h, out, rs + 4.), on(e, out, 6.));
            f.stroke(&tick, stroke(Color { a: 0.7, ..ACCENT }, 1.2));
        }
        let mid = Point::new((m.x + n.x) / 2., (m.y + n.y) / 2.);
        arrow(f, mid, m, ACCENT, 2.);
        arrow(f, mid, n, ACCENT, 2.);
        tag(f, labels, mid, out, label);
    };
    // The order: hole to hole, a grid back and forth (LIO-405).
    for w in pts.windows(2) {
        let (dx, dy) = (w[1].x - w[0].x, w[1].y - w[0].y);
        let l = dx.hypot(dy);
        if l < 2. * rs + 10. {
            continue;
        }
        let (ux, uy) = (dx / l * (rs + 4.), dy / l * (rs + 4.));
        let a = Point::new(w[0].x + ux, w[0].y + uy);
        let b = Point::new(w[1].x - ux, w[1].y - uy);
        arrow(f, a, b, Color { a: 0.85, ..TOOL }, 2.);
    }
    let outward: Vec<[f32; 2]> = match *pattern {
        Pattern::Grid {
            columns,
            rows,
            pitch,
            angle,
            ..
        } => {
            let (ua, ub) = (unit(angle), unit(angle + 90.));
            let back = |d: [f32; 2]| [-d[0], -d[1]];
            if columns > 1 {
                size(
                    f,
                    labels,
                    pts[0],
                    pts[1],
                    back(ub),
                    format!("pitch {}", dro(pitch[0])),
                );
            }
            if rows > 1 {
                // Hole 1's neighbour in the second row, which runs back.
                let to = pts[2 * columns - 1];
                size(
                    f,
                    labels,
                    pts[0],
                    to,
                    back(ua),
                    format!("rows {}", dro(pitch[1])),
                );
            }
            if angle.abs() >= 0.005 {
                let c = pts[0];
                let reach = 3. * rs + 30.;
                let x = canvas::Path::line(c, on(c, [1., 0.], reach + 20.));
                f.stroke(&x, dashed(MUTED, 1.3));
                let sweep = canvas::Path::new(|b| {
                    b.arc(canvas::path::Arc {
                        center: c,
                        radius: reach,
                        start_angle: iced::Radians(0.),
                        end_angle: iced::Radians(-angle.to_radians()),
                    })
                });
                f.stroke(&sweep, stroke(ACCENT, 2.));
                let end = on(c, unit(angle), reach);
                arrow(f, on(end, unit(angle - 90.), 12.), end, ACCENT, 2.);
                tag(
                    f,
                    labels,
                    on(c, unit(angle / 2.), reach),
                    unit(angle / 2.),
                    degrees(angle),
                );
            }
            // Each number up and right of its hole.
            vec![unit(45.); pts.len()]
        }
        Pattern::Arc {
            centre,
            diameter,
            first,
            ..
        } => {
            let c = at(centre);
            let big = diameter / 2. * s;
            f.stroke(&canvas::Path::circle(c, big), dashed(ACCENT, 1.6));
            for d in [[1., 0.], [0., 1.]] {
                let cross = canvas::Path::line(on(c, d, -11.), on(c, d, 11.));
                f.stroke(&cross, stroke(ACCENT, 2.2));
            }
            let under = Point::new(c.x - 10., c.y + 16.);
            let down = iced::Vector::new(0., 6.);
            labels.put(f, "centre", &[(under, Align::Right)], down, ACCENT, 15. * k);
            // From +X round to hole 1, counter-clockwise, outside the circle
            // and its numbers.
            let reach = big + rs + 34. * k;
            let x = canvas::Path::line(c, on(c, [1., 0.], reach + 16.));
            f.stroke(&x, dashed(MUTED, 1.3));
            let one = canvas::Path::line(c, on(c, unit(first), big - rs));
            f.stroke(&one, stroke(ACCENT, 1.6));
            if first >= 0.005 {
                let sweep = canvas::Path::new(|b| {
                    b.arc(canvas::path::Arc {
                        center: c,
                        radius: reach,
                        start_angle: iced::Radians(0.),
                        end_angle: iced::Radians(-first.to_radians()),
                    })
                });
                f.stroke(&sweep, stroke(ACCENT, 2.2));
                let end = on(c, unit(first), reach);
                arrow(f, on(end, unit(first - 90.), 12.), end, ACCENT, 2.2);
            }
            let half = unit(first / 2.);
            tag(
                f,
                labels,
                on(c, half, reach),
                half,
                format!("first {}", degrees(first)),
            );
            // Each number outside its hole.
            (holes.iter())
                .map(|h| {
                    let d = [h[0] - centre[0], h[1] - centre[1]];
                    let l = d[0].hypot(d[1]).max(1e-6);
                    [d[0] / l, d[1] / l]
                })
                .collect()
        }
    };
    for (j, (c, out)) in pts.iter().zip(outward).enumerate() {
        let body = canvas::Path::circle(*c, rs);
        let one = j == 0;
        f.fill(
            &body,
            if one {
                Color { a: 0.3, ..ACCENT }
            } else {
                Color::WHITE
            },
        );
        f.stroke(
            &body,
            if one {
                stroke(ACCENT, 3.)
            } else {
                stroke(INK, 1.5)
            },
        );
        let n = on(*c, out, rs + (if one { 15. } else { 12. }) * k);
        let (ink, size) = if one { (ACCENT, 20.) } else { (INK, 16.) };
        let number = numbers.get(j).copied().unwrap_or(j + 1);
        let tries = [(n, Align::Center)];
        labels.put(f, number.to_string(), &tries, along(out, 4.), ink, size * k);
    }
}
impl canvas::Program<Message> for Drawing<'_> {
    type State = (canvas::Cache, Cell<u64>);
    fn draw(
        &self,
        state: &Self::State,
        renderer: &iced::Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        if state.1.get() != self.key {
            state.0.clear();
            state.1.set(self.key);
        }
        vec![state.0.draw(renderer, bounds.size(), |f| self.paint(f))]
    }
}
/// Where `dims` fall on a canvas of `size` showing a picture of `PICTURE`
/// pixels as wide and high as it fits, centred (an image's `Contain`): each
/// one's ends, its extension lines' feature ends and its label's tag. A tag
/// sits beside its line on the side away from the picture's middle, clear of
/// the line, and is pushed on out until it covers no tag before it and no
/// dimension line (LIO-379: "Y 15" and "Z 9.5" lay on each other at the
/// step's corner).
fn dim_tags(dims: &[Dim], size: Size) -> Vec<(Point, Point, Option<[Point; 2]>, Rectangle)> {
    let (w, h) = (PICTURE.0 as f32, PICTURE.1 as f32);
    let k = (size.width / w).min(size.height / h);
    let (ox, oy) = ((size.width - w * k) / 2., (size.height - h * k) / 2.);
    let at = |p: Point| Point::new(ox + p.x * k, oy + p.y * k);
    let centre = Point::new(size.width / 2., size.height / 2.);
    // Every dimension line as drawn, a short one with its arrows outside.
    let lines: Vec<(Point, Point)> = (dims.iter())
        .map(|d| {
            let (a, b) = (at(d.a), at(d.b));
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let l = dx.hypot(dy).max(1.);
            if d.leader || l >= SHORT {
                (a, b)
            } else {
                let (ux, uy) = (dx / l * OUTSIDE, dy / l * OUTSIDE);
                (
                    Point::new(a.x - ux, a.y - uy),
                    Point::new(b.x + ux, b.y + uy),
                )
            }
        })
        .collect();
    let crosses = |r: &Rectangle, (a, b): (Point, Point)| {
        let n = ((b.x - a.x).hypot(b.y - a.y) / 2.).ceil().max(1.) as usize;
        (0..=n).any(|i| {
            let t = i as f32 / n as f32;
            let q = Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
            q.x > r.x - 3.
                && q.x < r.x + r.width + 3.
                && q.y > r.y - 3.
                && q.y < r.y + r.height + 3.
        })
    };
    let mut out: Vec<(Point, Point, Option<[Point; 2]>, Rectangle)> = vec![];
    for d in dims {
        let (a, b) = (at(d.a), at(d.b));
        let (tw, th) = (d.label.chars().count() as f32 * 9.5 + 12., 26.);
        // The way out from the line, and where the tag's middle starts.
        let (n, from) = if d.leader {
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let l = dx.hypot(dy).max(1.);
            ((dx / l, dy / l), b)
        } else {
            let mid = Point::new((a.x + b.x) / 2., (a.y + b.y) / 2.);
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let l = dx.hypot(dy).max(1.);
            let mut n = (-dy / l, dx / l);
            if n.0 * (mid.x - centre.x) + n.1 * (mid.y - centre.y) < 0. {
                n = (-n.0, -n.1);
            }
            (n, mid)
        };
        // Half the tag across the line, and a gap.
        let reach = n.0.abs() * tw / 2. + n.1.abs() * th / 2. + 8.;
        let tag = |out: f32| {
            let c = Point::new(from.x + n.0 * out, from.y + n.1 * out);
            Rectangle {
                x: (c.x - tw / 2.).clamp(0., (size.width - tw).max(0.)),
                y: (c.y - th / 2.).clamp(0., (size.height - th).max(0.)),
                width: tw,
                height: th,
            }
        };
        let clear = |r: &Rectangle| {
            (out.iter()).all(|(.., o)| {
                r.x + r.width + 4. <= o.x
                    || o.x + o.width + 4. <= r.x
                    || r.y + r.height + 4. <= o.y
                    || o.y + o.height + 4. <= r.y
            }) && lines.iter().all(|l| !crosses(r, *l))
        };
        let mut placed = tag(reach);
        for step in 1..60 {
            if clear(&placed) {
                break;
            }
            placed = tag(reach + 4. * step as f32);
        }
        out.push((a, b, d.ext.map(|e| e.map(at)), placed));
    }
    out
}
/// A dimension line shorter than this (px) has its arrows outside it,
/// `OUTSIDE` long.
const SHORT: f32 = 56.;
const OUTSIDE: f32 = 22.;
/// The sizes `dims` drawn over a picture of `PICTURE` pixels (`dim_tags`):
/// each a dimension line with its two arrowheads and its extension lines
/// from the feature, or a leader, and its label on a white tag.
struct Dims<'a>(&'a [Dim]);
impl canvas::Program<Message> for Dims<'_> {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &iced::Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut f = canvas::Frame::new(renderer, bounds.size());
        let thin = Color { a: 0.9, ..ACCENT };
        // A line on a white halo, to read over the stock's faces.
        let line = |f: &mut canvas::Frame, a: Point, b: Point, w: f32| {
            f.stroke(
                &canvas::Path::line(a, b),
                stroke(
                    Color {
                        a: 0.7,
                        ..Color::WHITE
                    },
                    w + 2.,
                ),
            );
            f.stroke(&canvas::Path::line(a, b), stroke(ACCENT, w));
        };
        for (d, (a, b, ext, tag)) in self.0.iter().zip(dim_tags(self.0, bounds.size())) {
            if d.leader {
                line(&mut f, b, a, 2.);
                arrow(&mut f, b, a, ACCENT, 2.5);
            } else {
                // Extension lines from the feature, a gap off it and a
                // little past the dimension line.
                for (e, end) in ext.into_iter().flatten().zip([a, b]) {
                    let (dx, dy) = (end.x - e.x, end.y - e.y);
                    let l = dx.hypot(dy);
                    if l > 4. {
                        let (ux, uy) = (dx / l, dy / l);
                        let from = Point::new(e.x + ux * 3., e.y + uy * 3.);
                        let past = Point::new(end.x + ux * 7., end.y + uy * 7.);
                        f.stroke(&canvas::Path::line(from, past), stroke(thin, 1.4));
                    }
                }
                let (dx, dy) = (b.x - a.x, b.y - a.y);
                let l = dx.hypot(dy).max(1.);
                let (ux, uy) = (dx / l, dy / l);
                line(&mut f, a, b, 2.);
                if l >= SHORT {
                    let mid = Point::new((a.x + b.x) / 2., (a.y + b.y) / 2.);
                    arrow(&mut f, mid, a, ACCENT, 2.);
                    arrow(&mut f, mid, b, ACCENT, 2.);
                } else {
                    // Too short for its arrows: they come in from outside
                    // the extension lines, as on a drawing.
                    let (oa, ob) = (
                        Point::new(a.x - ux * OUTSIDE, a.y - uy * OUTSIDE),
                        Point::new(b.x + ux * OUTSIDE, b.y + uy * OUTSIDE),
                    );
                    line(&mut f, oa, a, 2.);
                    line(&mut f, ob, b, 2.);
                    arrow(&mut f, oa, a, ACCENT, 2.);
                    arrow(&mut f, ob, b, ACCENT, 2.);
                }
            }
            // From the tag to its line when it was pushed off it.
            let tc = Point::new(tag.x + tag.width / 2., tag.y + tag.height / 2.);
            let anchor = if d.leader {
                b
            } else {
                Point::new((a.x + b.x) / 2., (a.y + b.y) / 2.)
            };
            if (tc.x - anchor.x).hypot(tc.y - anchor.y) > tag.width.max(tag.height) {
                f.stroke(&canvas::Path::line(anchor, tc), stroke(thin, 1.));
            }
            // On a white tag, to read over the stock.
            let r = canvas::Path::rounded_rectangle(tag.position(), tag.size(), 5.into());
            f.fill(&r, Color::WHITE);
            f.stroke(&r, stroke(ACCENT, 1.5));
            write(
                &mut f,
                d.label.clone(),
                Point::new(tag.x + 6., tc.y),
                ACCENT,
                17.,
                Align::Left,
            );
        }
        vec![f.into_geometry()]
    }
}
/// The gear to set, large: `rpm` in a ring.
fn gear<'a>(rpm: f32) -> Element<'a, Message> {
    let mono = Font {
        weight: iced::font::Weight::Bold,
        ..Font::MONOSPACE
    };
    container(
        column![
            text(format!("{}", rpm.round()))
                .size(44)
                .font(mono)
                .color(ACCENT),
            text("rpm · set the gear").size(14).color(MUTED)
        ]
        .align_x(Alignment::Center)
        .spacing(2),
    )
    .padding([18, 20])
    .style(|_| container::Style {
        background: Some(Color::WHITE.into()),
        border: Border {
            color: ACCENT,
            width: 3.,
            radius: 14.into(),
        },
        ..Default::default()
    })
    .into()
}
/// The tool seen from above with the edge the DRO's compensation shows on
/// X and Y: as large as its panel allows (LIO-379: a 60 px circle), shaded,
/// its four cutting edges, and each compensated side a heavy arrow from the
/// centre to the rim with the side named past it.
struct CompPicture([i8; 2]);
impl canvas::Program<Message> for CompPicture {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &iced::Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut f = canvas::Frame::new(renderer, bounds.size());
        // Room round it for the labels.
        let r = (bounds.width.min(bounds.height) * 0.32).clamp(24., 170.);
        let c = Point::new(bounds.width / 2., bounds.height / 2.);
        let body = canvas::Path::circle(c, r);
        // Lit from the top left, as the 3D pictures are.
        f.fill(
            &body,
            canvas::gradient::Linear::new(
                Point::new(c.x - r, c.y - r),
                Point::new(c.x + r, c.y + r),
            )
            .add_stop(0., Color::from_rgb8(226, 233, 242))
            .add_stop(1., Color::from_rgb8(150, 166, 188)),
        );
        // The four cutting edges, curving out from the centre.
        for q in 0..4 {
            let a = std::f32::consts::FRAC_PI_2 * q as f32;
            let at = |t: f32, da: f32| {
                Point::new(c.x + t * r * (a + da).cos(), c.y + t * r * (a + da).sin())
            };
            let flute = canvas::Path::new(|p| {
                p.move_to(at(0.12, 0.));
                p.quadratic_curve_to(at(0.6, 0.55), at(1., 0.35));
            });
            f.stroke(
                &flute,
                stroke(Color::from_rgb8(92, 108, 132), (r / 40.).max(1.5)),
            );
        }
        f.stroke(
            &body,
            stroke(Color::from_rgb8(70, 86, 110), (r / 50.).max(2.)),
        );
        f.fill(&canvas::Path::circle(c, (r / 14.).max(3.)), INK);
        let w = (r / 22.).clamp(2.5, 7.);
        let size = (r / 5.).clamp(15., 30.);
        let [x, y] = self.0.map(f32::from);
        for (k, v) in [(0, x), (1, y)] {
            if v == 0. {
                continue;
            }
            // Screen y grows down: +Y is up.
            let (dx, dy) = if k == 0 { (v, 0.) } else { (0., -v) };
            let rim = Point::new(c.x + dx * r, c.y + dy * r);
            arrow(&mut f, c, rim, ACCENT, w);
            f.fill(&canvas::Path::circle(rim, w * 1.2), ACCENT);
            let sign = if v > 0. { "+" } else { "−" };
            let past = Point::new(rim.x + dx * size * 0.8, rim.y + dy * size * 0.9);
            let align = match (k, v > 0.) {
                (0, true) => Align::Left,
                (0, false) => Align::Right,
                _ => Align::Center,
            };
            write(
                &mut f,
                format!("{} {sign}", AXES[k]),
                past,
                ACCENT,
                size,
                align,
            );
        }
        write(
            &mut f,
            "seen from above",
            Point::new(c.x, (c.y + r + size * 2.2).min(bounds.height - 10.)),
            MUTED,
            13.,
            Align::Center,
        );
        vec![f.into_geometry()]
    }
}
/// The colour mark of a legend entry.
struct Swatch(u8);
impl canvas::Program<Message> for Swatch {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &iced::Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut f = canvas::Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        let mid = Point::new(0., h / 2.);
        match self.0 {
            0 | 1 => {
                let c = if self.0 == 0 { ACCENT } else { MUTED };
                for k in [3., 8., 13.] {
                    f.stroke(
                        &canvas::Path::line(Point::new(k - h, h), Point::new(k, 0.)),
                        stroke(c, 1.4),
                    );
                }
                f.stroke(
                    &canvas::Path::rectangle(Point::ORIGIN, bounds.size()),
                    stroke(c, 1.),
                );
            }
            2 => f.fill(
                &canvas::Path::circle(Point::new(w / 2., h / 2.), h / 2.),
                Color { a: 0.5, ..TOOL },
            ),
            3 => f.fill_rectangle(Point::ORIGIN, bounds.size(), PART),
            _ => f.stroke(
                &canvas::Path::line(mid, Point::new(w, h / 2.)),
                dashed(NEVER, 2.),
            ),
        }
        vec![f.into_geometry()]
    }
}
fn legend<'a>() -> Element<'a, Message> {
    let entry = |k: u8, s: &'static str| {
        row![
            crate::fault::canvas(Swatch(k)).width(16).height(10),
            text(s).size(12).color(MUTED)
        ]
        .spacing(5)
        .align_y(Alignment::Center)
    };
    row![
        entry(0, "cut now"),
        entry(1, "cut before"),
        entry(2, "tool"),
        entry(3, "part"),
        entry(4, "do not pass")
    ]
    .spacing(14)
    .into()
}

/// The DRO's compensation sides as words: "comp X − · Y +".
fn comp_words(comp: [i8; 2]) -> String {
    let sides: Vec<String> = (0..2)
        .filter(|&k| comp[k] != 0)
        .map(|k| format!("{} {}", AXES[k], if comp[k] > 0 { "+" } else { "−" }))
        .collect();
    format!("comp {}", sides.join(" · "))
}
/// What to set in the DRO for a tool: its diameter and the compensation
/// sides, drawn; or a warning that the readouts are centres.
fn dro_tool<'a>(diameter: f32, comp: [i8; 2], height: f32) -> Vec<Element<'a, Message>> {
    if comp == [0, 0] {
        return vec![warn_chip("DRO comp off: centres")];
    }
    vec![
        label("Enter in the DRO"),
        readout(
            Readout {
                order: None,
                lead: None,
                axis: "Ø".into(),
                value: dro(diameter),
                lock: None,
                tags: vec!["tool diameter".into(), comp_words(comp)],
                lit: Lit::Now,
                big: false,
            },
            None,
        ),
        crate::fault::canvas(CompPicture(comp))
            .width(Fill)
            .height(height)
            .into(),
    ]
}

/// A Face that makes the part's top on a bottom already at size: measure
/// before the last level and set it so the part reads `size`, then check
/// it and that it is parallel (LIO-487). Measured at the ends out of a
/// vise's jaws: along the axis its jaws are not on (`across`, the axis
/// they close on); without one, at the ends of the stock's longer side
/// (`stock`, work coordinates).
/// The three lines of a Face to size, as the page and the agent's read of
/// the step both say them (LIO-487, LIO-470).
pub(crate) fn to_size_lines(size: f32, stock: &Bounds, across: Option<usize>) -> [String; 3] {
    let d = stock.dimensions();
    let along = match across {
        Some(k) => 1 - k,
        None => usize::from(d[1] > d[0]),
    };
    let ends = ["the left and right ends", "the front and back ends"][along];
    let size = dro(size);
    [
        format!(
            "Before the last level, measure it at {ends}; if they differ, seat the part again."
        ),
        format!("For the last level, go down from the Z just cut by what it reads over {size}."),
        format!("Then check {size} at both ends: the top parallel to the bottom."),
    ]
}
fn to_size<'a>(size: f32, stock: &Bounds, across: Option<usize>) -> Vec<Element<'a, Message>> {
    let [first, second, third] = to_size_lines(size, stock, across);
    let size = dro(size);
    let line = |n: u8, s: String| -> Element<'a, Message> {
        row![
            disc(n.to_string(), false, 22.),
            text(s).size(13).color(INK).width(Fill)
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .into()
    };
    vec![
        readout(
            Readout {
                order: None,
                lead: Some("Finish to".into()),
                axis: String::new(),
                value: size.clone(),
                lock: None,
                tags: vec!["top to bottom".into()],
                lit: Lit::Next,
                big: false,
            },
            None,
        ),
        line(1, first),
        line(2, second),
        line(3, third),
    ]
}
/// Where a new tool touches the top, over its picture with the spot ringed.
const RINGED: &str = "where it is ringed";
/// Where it touches when no spot is found for the picture, which is then
/// left out (LIO-488): the words name no picture.
const UNRINGED: &str = "on solid stock, clear of holes and slots";
/// A new tool picks up Z: touch the top and set Z to `top`, `place` it
/// touches said beside it; nothing for a tool already in.
fn pick_up<'a>(top: Option<f32>, place: Option<&str>) -> Vec<Element<'a, Message>> {
    let Some(top) = top else {
        return vec![];
    };
    vec![
        label("Pick up Z with the tool"),
        readout(
            Readout {
                order: None,
                lead: Some("touch the top → set".into()),
                axis: "Z".into(),
                value: dro(top),
                lock: None,
                tags: place.map(String::from).into_iter().collect(),
                lit: Lit::Next,
                big: false,
            },
            None,
        ),
    ]
}

/// What the page needs at hand for a step.
struct Page<'a> {
    doc: &'a Document,
    shown: &'a Shown,
    /// The fingerprint of the guide, for the drawings' keys.
    key: u64,
    at: usize,
    item: usize,
    /// What is typed in a lathe step's measure box (`App::guide_typed`).
    typed: Option<&'a (String, String)>,
    /// The window's width and height, which some screens fit to.
    window: (f32, f32),
    setup: &'a Setup,
    /// Drawn for the shop tablet (`tablet::steps`), which types a lathe
    /// measurement in its own field: no text box in the picture (LIO-543).
    tablet: bool,
}
impl<'a> Page<'a> {
    fn guide(&self) -> &'a Guide {
        &self.shown.guide
    }
    /// The setup's notes, where it says what holds and supports the part
    /// (LIO-487): shown on the Zero page, which pages that leave the
    /// supports under the part to the user point back to. Empty when none.
    fn notes(&self) -> &'a str {
        get(&self.setup.p, "notes").trim()
    }
    /// The position of operation `op` among those the setup ran.
    fn done(&self, op: usize) -> usize {
        (self.guide().done.iter())
            .position(|d| d.op == op)
            .unwrap_or(0)
    }
    /// The stock box before operation `op`.
    fn before(&self, op: usize) -> Bounds {
        self.shown.boxes[self.done(op)]
    }
    fn tool(&self, op: usize) -> Option<&'a Tool> {
        let id = &self.setup.operations.get(op)?.tool;
        self.doc.tools.iter().find(|t| &t.id == id)
    }
    /// The chip of an operation's tool.
    fn tool_name(&self, op: usize, diameter: f32) -> String {
        let id = self
            .setup
            .operations
            .get(op)
            .map_or("", |o| o.tool.as_str());
        self.named(id, diameter)
    }
    /// The chip of the tool with this id.
    fn named(&self, id: &str, diameter: f32) -> String {
        match self.doc.tools.iter().find(|t| t.id == id) {
            Some(t) => match t.number() {
                Some(n) if !crate::model::names_number(n, t.name()) => {
                    format!("Ø{} {} · T{n}", mm(diameter), t.name())
                }
                Some(_) => format!("Ø{} {}", mm(diameter), t.name()),
                None => format!("Ø{} {}", mm(diameter), t.name()),
            },
            None => format!("Ø{}", mm(diameter)),
        }
    }
    fn drawing(
        &self,
        view: usize,
        stock: Bounds,
        shapes: Vec<Shape>,
        height: impl Into<Length>,
    ) -> Element<'a, Message> {
        self.drawing_with(view, stock, shapes, height, &self.shown.jaws)
    }
    /// A drawing with these `jaws`.
    fn drawing_with(
        &self,
        view: usize,
        stock: Bounds,
        shapes: Vec<Shape>,
        height: impl Into<Length>,
        jaws: &'a [Bounds],
    ) -> Element<'a, Message> {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        (self.key, self.at, self.item, view).hash(&mut h);
        crate::fault::canvas(Drawing {
            key: h.finish(),
            axes: VIEWS[view],
            stock,
            part: &self.shown.part[view],
            jaws,
            shapes,
        })
        .width(Fill)
        .height(height)
        .into()
    }
    /// The stock in 3D before the setup (0) or after its operation `i`,
    /// as wide as the room and `height` high.
    fn picture(&self, i: usize, height: impl Into<Length>) -> Element<'a, Message> {
        Self::image(self.shown.pictures.get(i).cloned().flatten(), height.into())
    }
    /// The stock as it starts, in its vise on its parallels.
    fn held_picture(&self, height: impl Into<Length>) -> Element<'a, Message> {
        Self::image(self.shown.held.clone(), height.into())
    }
    fn image(picture: Option<image::Handle>, h: Length) -> Element<'a, Message> {
        let w = Fill;
        match picture {
            Some(p) => image(p).width(w).height(h).into(),
            None => container(muted("No picture of the stock."))
                .width(w)
                .height(h)
                .center_x(w)
                .center_y(h)
                .into(),
        }
    }
}

/// How the Steps page was entered (LIO-449), said over its screen until
/// the place moves.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// At the place setup `setup` (an id) was left. `ahead`: the section of
    /// the operation picked in planning, which comes later, not skipped to.
    Resumed { setup: String, ahead: Option<usize> },
    /// At the start of the operation picked in planning; the place before
    /// was step `from`, at its hole or readout `item`.
    Picked {
        setup: String,
        from: usize,
        item: usize,
    },
}
/// The line over the screen that says how the page was entered, with the
/// way to the other place (LIO-449).
fn entry_line<'a>(app: &'a App, g: &Guide, at: usize) -> Option<Element<'a, Message>> {
    let setup = app.doc.setups.get(g.setup)?;
    let entry = app.steps_entry.as_ref()?;
    let place = |at: usize| {
        let s = &g.sections[section_of(g, at)];
        let p = progress(g, s, at);
        let name = section_name(&app.doc, g, s);
        if p.is_empty() {
            name
        } else {
            format!("{name} · {p}")
        }
    };
    let (said, go) = match entry {
        Entry::Resumed {
            setup: id,
            ahead: Some(i),
        } if *id == setup.id && *i < g.sections.len() => {
            let picked = section_name(&app.doc, g, &g.sections[*i]);
            let now = if at == 0 && app.guide_item == 0 {
                "These steps start at the beginning".to_string()
            } else {
                format!(
                    "Resuming where these steps were left: {}",
                    place(at)
                )
            };
            (
                format!("{now}. {picked}, picked in planning, comes later."),
                (
                    format!("Skip to {picked} ▶"),
                    GuideAction::Go(g.sections[*i].steps.start),
                ),
            )
        }
        Entry::Resumed { setup: id, .. } if *id == setup.id => {
            let s = &g.sections[section_of(g, at)];
            (
                format!(
                    "Resuming where these steps were left: {}.",
                    place(at)
                ),
                (
                    format!("◀ Start {} again", section_name(&app.doc, g, s)),
                    GuideAction::Go(s.steps.start),
                ),
            )
        }
        Entry::Picked {
            setup: id,
            from,
            item,
        } if *id == setup.id && *from < g.steps.len() => (
            format!(
                "Opened at the start of the operation picked in planning. The steps had got to {}.",
                place(*from)
            ),
            ("Back to there ▶".to_string(), GuideAction::Resume),
        ),
        _ => return None,
    };
    let line = row![
        text(said).size(13).color(INK).width(Fill),
        crate::ui::secondary(go.0, Message::Guide(go.1))
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    Some(
        container(line)
            .padding([6, 12])
            .width(Fill)
            .style(|_| container::Style {
                background: Some(CHIP.into()),
                border: Border {
                    color: ACCENT,
                    width: 1.,
                    radius: 8.into(),
                },
                text_color: Some(INK),
                ..Default::default()
            })
            .into(),
    )
}
/// The page.
pub fn view(app: &App) -> Element<'_, Message> {
    let main: Element<_> = match app.current_guide() {
        Some(Ok(shown)) => guide_page(app, shown),
        Some(Err(e)) => problem(app, e),
        None if app.guide_key().is_none() => notice(vec!["Load the part to see the steps.".into()]),
        None => notice(vec!["Simulating the setup…".into()]),
    };
    row![setups(app), container(main).width(Fill).height(Fill)]
        .spacing(12)
        .padding(12)
        .height(Fill)
        .into()
}
/// Why the active setup's steps cannot be shown, `e`, with a button to fix
/// it in MANUFACTURE: the operation it names (`guide::simulate` puts its
/// name first) at the tab the problem is on, else the setup (LIO-374).
fn problem<'a>(app: &'a App, e: &str) -> Element<'a, Message> {
    let named = (app.doc.active_setup().into_iter())
        .flat_map(|s| s.operations.iter().enumerate())
        .find_map(|(i, o)| Some((i, o.name(), e.strip_prefix(&format!("{}: ", o.name()))?)));
    let (label, action) = match named {
        Some((i, name, rest)) => (
            format!("Open {name} in planning ▶"),
            GuideAction::Fix(Some(i), rest.to_string()),
        ),
        None => (
            "Open the setup in planning ▶".to_string(),
            GuideAction::Fix(None, e.to_string()),
        ),
    };
    let open = primary(label, Message::Guide(action));
    column![notice(vec![e.to_string()]), open]
        .spacing(12)
        .into()
}
fn notice<'a>(lines: Vec<String>) -> Element<'a, Message> {
    let mut c = column![].spacing(8);
    for (i, l) in lines.into_iter().enumerate() {
        c = c.push(if i == 0 {
            bold(l, 16)
        } else {
            muted(l).size(13)
        });
    }
    container(c).padding(24).width(Fill).style(card).into()
}
fn card(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::WHITE.into()),
        border: Border {
            color: LINE,
            width: 1.,
            radius: 8.into(),
        },
        text_color: Some(INK),
        ..Default::default()
    }
}
/// The left column: the setups on the hand machines (the manual mill, the
/// lathe), and the way to the CNC.
fn setups(app: &App) -> Element<'_, Message> {
    let title = match app.doc.active_setup() {
        Some(s) if crate::lathe::is_lathe(&s.machine) => "Manual lathe",
        _ => "Manual mill",
    };
    // The one way back, top left (LIO-429).
    // None on the shop tablet: planning stays on the computer.
    let back = (!crate::SHOP)
        .then(|| crate::ui::secondary(crate::ui::BACK, Message::SelectSetup(app.doc.index())));
    let mut c = column![back, bold(title, 16), muted("Setups")].spacing(8);
    // The one setup picker, as the Run page's: the job's setups in its
    // order, each with how it runs (LIO-431).
    for (si, s) in app.doc.setups.iter().enumerate() {
        let on = s.id == app.doc.active;
        let msg = Message::Guide(GuideAction::Open(si));
        let label = crate::ui::job_setup_label(si, s);
        let b = button(column![bold(label.clone(), 13)].spacing(2))
            .width(Fill)
            .padding([8, 10])
            .on_press(msg.clone())
            .style(move |_, status| button::Style {
                background: Some(
                    if on {
                        crate::ui::BG
                    } else if matches!(status, button::Status::Hovered) {
                        HOVER_BUTTON
                    } else {
                        Color::WHITE
                    }
                    .into(),
                ),
                text_color: INK,
                border: Border {
                    color: if on { BLUE } else { LINE },
                    width: 1.,
                    radius: 6.into(),
                },
                ..Default::default()
            });
        c = c.push(HelpButton::new(b, &msg, &label).width(Fill));
    }
    // The CNC machines, for the Control page without a CNC setup.
    let cnc_machines: Vec<_> = (app.doc.machines.iter())
        .filter(|m| crate::model::get(&m.p, "controller") == "UC100")
        .collect();
    if !cnc_machines.is_empty() {
        c = c.push(space().height(8)).push(muted("CNC machines"));
        for m in cnc_machines {
            c = c.push(
                crate::ui::secondary(
                    format!("{} ▶", m.name()),
                    Message::ControlMachine(m.id.clone()),
                )
                .width(Fill),
            );
        }
    }
    // The setup's operations, as the Run page lists them, a press going to
    // that section; the row of chips over the screen is gone (LIO-431).
    // Only the list scrolls: Back and the setups stay in view.
    if let Some(Ok(shown)) = app.current_guide()
        && shown.guide.setup == app.doc.index()
    {
        c = c.push(space().height(8)).push(muted("Operations"));
        // Room at the right for the scrollbar.
        c = c.push(
            scrollable(container(operations(app, shown)).padding(iced::Padding::ZERO.right(12)))
                .id(OPERATIONS_LIST)
                .spacing(4)
                .height(Fill),
        );
    }
    container(c)
        .padding(14)
        .width(210)
        .height(Fill)
        .style(card)
        .into()
}
/// Section `i`'s number among the setup's operations, from 1; None for one
/// that is no operation (Set up, the hand-over).
fn op_number(g: &Guide, i: usize) -> Option<usize> {
    g.sections.get(i)?.op?;
    Some(g.sections[..=i].iter().filter(|s| s.op.is_some()).count())
}
/// The section a step is in.
pub(crate) fn section_of(g: &Guide, at: usize) -> usize {
    (g.sections.iter())
        .position(|s| s.steps.contains(&at))
        .unwrap_or(0)
}
/// Where the place is in its section: "pass 2/4", "2/6".
fn progress(g: &Guide, s: &Section, at: usize) -> String {
    use crate::guide::LatheStep as L;
    match &g.steps[at] {
        Step::Lathe(L::Pass { index, of, .. } | L::ThreadPass { index, of, .. }) => {
            format!("pass {index}/{of}")
        }
        _ if s.steps.len() > 1 => format!("{}/{}", at - s.steps.start + 1, s.steps.len()),
        _ => String::new(),
    }
}
/// What a section's count counts: on the mill its features (LIO-688),
/// "4 holes", "1 box", "4 corners"; on the lathe "4 passes", or a
/// drilling's "1 hole".
fn counted(setup: &Setup, s: &Section) -> String {
    let op = s.op.and_then(|op| setup.operations.get(op));
    let lathe = crate::lathe::is_lathe(&setup.machine);
    let (one, many) = match op.map(|o| o.strategy) {
        Some(Strategy::Drill) if !lathe || !op.is_some_and(|o| o.boring()) => ("hole", "holes"),
        Some(Strategy::Box) if !lathe => ("box", "boxes"),
        Some(Strategy::Slot) if !lathe => ("slot", "slots"),
        Some(Strategy::Corner) if !lathe => ("corner", "corners"),
        _ => ("pass", "passes"),
    };
    match s.count {
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    }
}
fn section_name(doc: &Document, g: &Guide, s: &Section) -> String {
    let lathe = (doc.setups.get(g.setup)).is_some_and(|s| crate::lathe::is_lathe(&s.machine));
    match (s.op, &g.steps[s.steps.start]) {
        (None, Step::HandOver { next: Some(n) }) => {
            format!("→ {}", doc.setups.get(*n).map_or("", |s| s.name()))
        }
        // On the lathe every operation turns: "Outside", not "Turn · Outside".
        (Some(_), _) if lathe => (s.name.strip_prefix("Turn · ").unwrap_or(&s.name)).to_string(),
        _ => s.name.clone(),
    }
}
/// The Steps page's line over a setup's steps. A setup named as its machine
/// says it once (LIO-382).
fn setup_title(setup: &Setup) -> String {
    if setup.name().starts_with(setup.machine.name()) {
        format!("{} · steps", setup.name())
    } else {
        format!("{} on the {}", setup.name(), setup.machine.name())
    }
}
/// The header of section `here`: its number, its name, and its tool, count
/// and time.
fn head<'a>(p: &Page<'a>, here: usize) -> iced::widget::Row<'a, Message> {
    let (g, setup) = (p.guide(), p.setup);
    let s = &g.sections[here];
    let header = row![
        disc(
            op_number(g, here).map_or("•".to_string(), |k| k.to_string()),
            true,
            26.
        ),
        bold(section_name(p.doc, g, s), 17)
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    let mut tags = vec![];
    let lathe = crate::lathe::is_lathe(&setup.machine);
    if lathe && let Some(op) = s.op {
        tags.extend(turning::header(p, op, s));
    } else if let Some(op) = s.op
        && setup.operations.get(op).is_some()
    {
        let d = num(
            &p.tool(op).map_or(Default::default(), |t| t.p.clone()),
            "diameter",
        );
        if let Some(d) = d {
            tags.push(chip(p.tool_name(op, d)));
        }
        if s.count > 0 {
            tags.push(chip(counted(setup, s)));
        }
        tags.push(chip(format!("~{} min", s.minutes.ceil())));
    }
    header.push(row(tags).spacing(6))
}
fn guide_page<'a>(app: &'a App, shown: &'a Shown) -> Element<'a, Message> {
    let g = &shown.guide;
    let at = crate::guide::place(&app.doc, g);
    let here = section_of(g, at);
    let setup = &app.doc.setups[g.setup];
    let page = Page {
        doc: &app.doc,
        shown,
        key: app.guide.as_ref().map_or(0, |(k, _)| *k),
        at,
        item: app.guide_item,
        typed: app.guide_typed.as_ref(),
        window: (app.window_width, app.window_height),
        setup,
        tablet: false,
    };
    let header = head(&page, here)
        .push(space().width(Fill))
        .push(bold(place(&app.doc, g, here, at), 14).color(ACCENT));
    let top = row![
        bold(setup_title(setup), 16),
        muted(format!("~{} min", g.minutes.ceil())).size(13)
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    let mut page_column = column![top, header].spacing(12);
    if let Some(line) = entry_line(app, g, at) {
        page_column = page_column.push(line);
    }
    page_column
        .push(container(screen(&page)).height(Fill))
        .into()
}
/// The left panel's scrollable: scrolled so that the place's section is in
/// view whenever the place moves (`App::snap_steps_list`, LIO-431).
pub(crate) const OPERATIONS_LIST: &str = "steps-operations";
/// The setup's sections as a list, each its state mark, number, name, what
/// is left of it and its time; a press goes to it (LIO-431).
fn operations<'a>(app: &'a App, shown: &'a Shown) -> Element<'a, Message> {
    let g = &shown.guide;
    let at = crate::guide::place(&app.doc, g);
    let here = section_of(g, at);
    let setup = &app.doc.setups[g.setup];
    let mut list = Column::new().spacing(4);
    for (i, s) in g.sections.iter().enumerate() {
        let done = s.steps.end <= at;
        let now = i == here;
        // Numbered from 1 (LIO-377).
        // Numbered as the Run page numbers operations; Set up and the hand
        // over carry a mark (LIO-431).
        let mark = if done {
            "✓".to_string()
        } else {
            op_number(g, i).map_or("•".to_string(), |k| k.to_string())
        };
        let small = if now {
            progress(g, s, at)
        } else if s.count > 0 && !done {
            counted(setup, s)
        } else {
            String::new()
        };
        // The name, what is left of it under it, as the Run page's lines.
        // A name is never cut (LIO-426): a section done wraps as the
        // current one does, a size smaller (LIO-431).
        let name = text(section_name(&app.doc, g, s)).size(if done && !now { 12 } else { 13 });
        let mut words = column![name].spacing(1);
        if !small.is_empty() {
            words = words.push(text(small).size(12).color(if now { ACCENT } else { MUTED }));
        }
        let face = row![disc(mark, now, 20.), words]
            .spacing(6)
            .align_y(Alignment::Center);
        let ink = if done && !now { DONE } else { INK };
        let msg = Message::Guide(GuideAction::Section(i));
        let b = button(face)
            .padding([5, 10])
            .on_press(msg.clone())
            .style(move |_, status| button::Style {
                background: Some(
                    if matches!(status, button::Status::Hovered) {
                        HOVER_BUTTON
                    } else {
                        Color::WHITE
                    }
                    .into(),
                ),
                text_color: ink,
                border: Border {
                    color: if now { ACCENT } else { LINE },
                    width: if now { 2. } else { 1. },
                    radius: 8.into(),
                },
                ..Default::default()
            });
        list = list.push(HelpButton::new(b.width(Fill), &msg, s.name.clone()).width(Fill));
    }
    list.into()
}
/// Where the job is, in the Run page's words (LIO-431): "Setup 2 of 3 ·
/// operation 2 of 4", then the hole or pass of the section.
fn place(doc: &Document, g: &Guide, here: usize, at: usize) -> String {
    let s = &g.sections[here];
    let mut said = format!("Setup {} of {}", g.setup + 1, doc.setups.len());
    let ops: Vec<usize> = (g.sections.iter().enumerate())
        .filter(|(_, x)| x.op.is_some())
        .map(|(i, _)| i)
        .collect();
    if let Some(k) = ops.iter().position(|i| *i == here) {
        said += &format!(" · operation {} of {}", k + 1, ops.len());
    }
    let p = progress(g, s, at);
    if !p.is_empty() {
        said += &format!(" · {p}");
    }
    said
}
/// The command that ends a step: "Pass done ▶", "Section done ▶" on the
/// last step of an operation's section, else "Next ▶".
fn next_label(g: &Guide, at: usize) -> String {
    let s = &g.sections[section_of(g, at)];
    match &g.steps[at] {
        Step::Lathe(crate::guide::LatheStep::Pass { .. })
        | Step::Lathe(crate::guide::LatheStep::ThreadPass { .. }) => "Pass done ▶".into(),
        _ if s.op.is_some() && at + 1 == s.steps.end => "Section done ▶".into(),
        _ => "Next ▶".into(),
    }
}
/// A step's screen, in the parts `framed` lays out: its caption, the
/// pictures, the numbers, the step's own command, whether Back is live, and
/// how the pictures and the numbers share the card. The shop tablet shows
/// them in its own frame (`tablet_steps`).
pub(crate) struct Parts<'a> {
    pub title: String,
    pub sub: &'static str,
    pub left: Element<'a, Message>,
    pub right: Column<'a, Message>,
    pub next: Option<(String, Message)>,
    pub back: bool,
    pub split: (u16, u16),
}
fn screen<'a>(p: &Page<'a>) -> Element<'a, Message> {
    let s = parts(p);
    framed(s.title, s.sub, s.left, s.right, s.next, s.back, s.split)
}
fn parts<'a>(p: &Page<'a>) -> Parts<'a> {
    let g = p.guide();
    let next = Some((
        next_label(g, p.at),
        Message::Guide(GuideAction::Next),
    ));
    let (title, sub, left, right): (String, &str, Element<_>, Column<_>) = match &g.steps[p.at] {
        Step::Zero {
            stock,
            holding,
            vise,
            clamps,
        } => zero_screen(p, stock, holding, vise.as_ref(), clamps),
        Step::Sheet {
            op,
            tool,
            rpm,
            features,
        } => sheet_screen(p, *op, tool, *rpm, features),
        Step::Lathe(step) => turning::screen(p, step),
        Step::HandOver { next: to } if p.shown.side.is_some() => {
            return turning::hand_over(p, *to);
        }
        Step::HandOver { next: to } => {
            let last = p.shown.pictures.len() - 1;
            let mut c = vec![chip(format!("manual ~{} min", g.minutes.ceil()))];
            let next = match to.and_then(|n| Some((n, p.doc.setups.get(n)?))) {
                Some((n, s)) => {
                    c.insert(0, warn_chip(format!("→ {}", s.name())));
                    Some((
                        format!("Go to {} ▶", s.name()),
                        Message::Guide(GuideAction::Open(n)),
                    ))
                }
                None => {
                    c.insert(0, chip("the last setup"));
                    None
                }
            };
            return Parts {
                title: "Hand over".into(),
                sub: "the stock leaves the mill",
                left: p.picture(last, Fill),
                right: column![chips(c)],
                next,
                back: true,
                split: (11, 9),
            };
        }
    };
    // The lathe's numbers take half the card at a narrow window (LIO-389).
    let split = match &g.steps[p.at] {
        Step::Lathe(_) => turning::split(p),
        // A table of numbers wants the room (LIO-688).
        Step::Sheet { .. } if p.window.0 < 1200. => (1, 1),
        _ => (11, 9),
    };
    Parts {
        title,
        sub,
        left,
        right,
        next,
        back: p.at > 0 || p.item > 0,
        split,
    }
}
/// A step's screen: its caption, the pictures, which grow with the room
/// the page has, and the numbers, which scroll when they do not fit; Back
/// (live unless `back` is false) and `next` in one place at the card's foot
/// on every screen (LIO-369); the pictures and the numbers sharing the
/// card's width as `split`.
fn framed<'a>(
    title: String,
    sub: &str,
    left: Element<'a, Message>,
    right: Column<'a, Message>,
    next: Option<(String, Message)>,
    back: bool,
    (pictures, numbers): (u16, u16),
) -> Element<'a, Message> {
    let body = row![
        container(left).width(FillPortion(pictures)).height(Fill),
        // Clear of the scrollbar, as the other pages are (LIO-382).
        container(
            scrollable(
                (right.spacing(10)).padding(iced::Padding::ZERO.right(crate::ui::SCROLLBAR_GUTTER)),
            )
            .height(Fill)
        )
        .width(FillPortion(numbers))
        .height(Fill)
    ]
    .spacing(18)
    .height(Fill);
    let foot = row![
        space().width(FillPortion(pictures)),
        container(buttons(next, back)).width(FillPortion(numbers))
    ]
    .spacing(18);
    column![
        row![bold(title, 15), muted(sub.to_string()).size(13)]
            .spacing(8)
            .align_y(Alignment::Center),
        container(column![body, foot].spacing(14))
            .padding(16)
            .width(Fill)
            .height(Fill)
            .style(card)
    ]
    .spacing(6)
    .height(Fill)
    .into()
}
/// Where the work zero is on axis `k` (0 X, 1 Y, 2 Z) of the stock box `s`
/// in work coordinates, where the zero is 0 (LIO-688): on a face, midway
/// between two, or how far from one.
fn zero_place(s: &Bounds, k: usize) -> String {
    let on = |v: f32| v.abs() < 0.005;
    let (lo, hi) = (s.min[k], s.max[k]);
    if k == 2 {
        return match () {
            _ if on(hi) => "the top of the stock".into(),
            _ if on(lo) => "the bottom of the stock".into(),
            _ if hi > 0. => format!("{} below the top of the stock", mm(hi)),
            _ => format!("{} above the top of the stock", mm(-hi)),
        };
    }
    let [first, last] = [["left", "right"], ["front", "back"]][k];
    let [on_way, back_way] = [["right of", "left of"], ["behind", "in front of"]][k];
    match () {
        _ if on(lo) => format!("the {first} face"),
        _ if on(hi) => format!("the {last} face"),
        _ if on(lo + hi) => format!("the middle, halfway between the {first} and {last} faces"),
        _ if lo < 0. => format!("{} {on_way} the {first} face", mm(-lo)),
        _ => format!("{} {back_way} the {first} face", mm(lo)),
    }
}
/// The zero page (LIO-688): where X0, Y0 and Z0 are on the stock, which is
/// shown held, the zero marked on it; how it is held and the setup's notes.
fn zero_screen<'a>(
    p: &Page<'a>,
    stock: &'a Bounds,
    holding: &'a Gripped,
    vise: Option<&'a crate::guide::Vise>,
    clamps: &'a [(String, Bounds)],
) -> (String, &'static str, Element<'a, Message>, Column<'a, Message>) {
    let d = stock.dimensions();
    let narrow = turning::narrow(p);
    let mut c = vec![chip(p.doc.holding_label(p.setup)), chip("top face up")];
    // Across the jaws, seen from the front or the right: the stock on its
    // parallels, the jaws at their height and the grip. Without a vise,
    // across the parallels the setup stands it on.
    let parallels = crate::parallels::of(p.setup).ok().flatten();
    let view = match (vise, &parallels) {
        (Some(v), _) if v.across == 1 => 2,
        (None, Some(par)) if par.across() == 1 => 2,
        _ => 1,
    };
    let (k, top) = (VIEWS[view][0], stock.max[2]);
    let mut side: Vec<Shape> = p.shown.parallels.iter().map(|b| Shape::Block(*b)).collect();
    side.push(Shape::Word([0.; 3], "Z0".into(), ACCENT));
    if let Some(v) = vise {
        let jaw = holding.bottom + v.grip;
        let (mut a, mut b) = ([0.; 3], [0.; 3]);
        // Outside the + jaw.
        let out = stock.max[k] + v.jaw_thickness;
        (a[k], b[k]) = (out, out);
        (a[2], b[2]) = (jaw, holding.bottom);
        side.push(Shape::Height(a, b, format!("{} in the jaws", mm(v.grip))));
        c.push(chip(format!("jaws on the {} sides", AXES[v.across])));
        c.push(chip(format!("{} in the jaws", mm(v.grip))));
        let under = holding.bottom + v.grip - v.jaw_height;
        if !p.shown.parallels.is_empty() && parallels.is_none() {
            c.push(chip(format!(
                "on parallels {} high",
                mm(stock.min[2] - under)
            )));
        }
    }
    // The parallels the setup says it stands on (LIO-537): how many, how
    // high, which way and where, and their height drawn.
    if let Some(par) = &parallels {
        c.push(chip(format!(
            "on {} parallel{} {} high",
            par.at.len(),
            if par.at.len() == 1 { "" } else { "s" },
            mm(par.height)
        )));
        c.push(chip(format!(
            "running {} at {}",
            if par.along_y {
                "front to back"
            } else {
                "left to right"
            },
            (par.at.iter())
                .map(|a| par.place(*a))
                .collect::<Vec<_>>()
                .join(", ")
        )));
        if vise.is_none() {
            let (mut a, mut b) = ([0.; 3], [0.; 3]);
            let out = stock.max[k] + 1.;
            (a[k], b[k]) = (out, out);
            (a[2], b[2]) = (stock.min[2], stock.min[2] - par.height);
            side.push(Shape::Height(a, b, format!("{} parallels", mm(par.height))));
        }
    }
    // From above: the clamps over the stock and X0 Y0.
    let mut plan: Vec<Shape> = (clamps.iter())
        .map(|(_, b)| Shape::Clamp(*b, "clamp".into()))
        .collect();
    plan.push(Shape::Word([0., 0., top], "X0 Y0".into(), ACCENT));
    let drawings = row![
        p.drawing(0, *stock, plan, Fill),
        p.drawing_with(view, *stock, side, Fill, &p.shown.vise),
    ]
    .spacing(6);
    let held = iced::widget::stack![
        p.held_picture(Fill),
        crate::fault::canvas(Dims(&p.shown.zero)).width(Fill).height(Fill)
    ];
    let left = column![
        container(held).height(FillPortion(3)),
        container(drawings).height(FillPortion(2)),
    ]
    .spacing(6)
    .align_x(Alignment::Center)
    .into();
    let mono = Font {
        weight: iced::font::Weight::Bold,
        ..Font::MONOSPACE
    };
    let mut right = column![label("Work zero")];
    for k in 0..3 {
        right = right.push(
            boxed(
                row![
                    text(format!("{}0", AXES[k]))
                        .size(22)
                        .font(mono)
                        .color(ACCENT)
                        .width(44),
                    text(zero_place(stock, k)).size(15).color(INK).width(Fill),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
                false,
            )
            .padding([6, 12])
            .width(Fill),
        );
    }
    right = right.push(label("Stock as it starts")).push(chips(c));
    // The parallels in a sentence: where they go from the stock's centre.
    if let Some(said) = crate::parallels::sentence(p.setup) {
        right = right.push(text(said).size(14).color(INK));
    }
    right = right.push(readout(
        Readout {
            order: None,
            // The key before the size, one text that does not wrap at
            // 1024 × 700 (LIO-435).
            lead: Some("X × Y × Z".into()),
            axis: String::new(),
            // Narrow, the size closes up (LIO-487).
            value: [d[0], d[1], d[2]]
                .map(mm)
                .join(if narrow { "×" } else { " × " }),
            lock: None,
            tags: vec![],
            lit: Lit::Next,
            big: false,
        },
        None,
    ));
    // How the setup holds and supports the part, in its own words: a stop
    // and the room under holes are not modelled (LIO-487).
    let notes = p.notes();
    if !notes.is_empty() {
        right = right.push(label(SETUP_NOTES)).push(
            boxed(text(notes).size(14).color(INK), false)
                .padding([8, 12])
                .width(Fill),
        );
    }
    ("Zero".into(), "where X0 Y0 Z0 are on the stock", left, right)
}
/// A table of numbers: its head, then its rows, the columns sharing the
/// width as `widths` (LIO-688).
fn table<'a>(head: &[&str], rows: Vec<Vec<String>>, widths: &[u16]) -> Element<'a, Message> {
    let mono = Font {
        weight: iced::font::Weight::Bold,
        ..Font::MONOSPACE
    };
    let line = |cells: Vec<Element<'a, Message>>| row(cells).spacing(8).align_y(Alignment::Center);
    let mut c = column![line(
        (head.iter().zip(widths))
            .map(|(h, w)| {
                text(h.to_string())
                    .size(12)
                    .color(MUTED)
                    .width(FillPortion(*w))
                    .into()
            })
            .collect()
    )]
    .spacing(6);
    for r in rows {
        c = c.push(line(
            (r.into_iter().zip(widths))
                .map(|(s, w)| {
                    text(s)
                        .size(14)
                        .font(mono)
                        .color(INK)
                        .width(FillPortion(*w))
                        .into()
                })
                .collect(),
        ));
    }
    boxed(container(c).padding([8, 12]), false)
        .width(Fill)
        .into()
}
/// A feature's sizes, a name and its numbers a line (LIO-688).
fn listed<'a>(lines: Vec<(&str, String)>) -> Element<'a, Message> {
    table(&["", ""], lines.into_iter().map(|(k, v)| vec![k.into(), v]).collect(), &[2, 7])
}
/// "−70.00 to 70.00".
fn span(a: f32, b: f32) -> String {
    format!("{} to {}", dro(a), dro(b))
}
/// One operation's page (LIO-688): its tool and spindle speed, and what it
/// cuts, each feature numbered as the plan picture numbers it, with its
/// place, size and depth.
fn sheet_screen<'a>(
    p: &Page<'a>,
    op: usize,
    tool: &str,
    rpm: Option<f32>,
    features: &'a Features,
) -> (String, &'static str, Element<'a, Message>, Column<'a, Message>) {
    let o = p.setup.operations.get(op);
    let d = p
        .tool(op)
        .and_then(|t| num(&t.p, "diameter"))
        .unwrap_or(0.);
    let mut c = vec![
        chip(p.named(tool, d)),
        match rpm {
            Some(r) => chip(format!("{} rpm", r.round())),
            // A tap is turned by hand (LIO-410).
            None => warn_chip("by hand, spindle off"),
        },
    ];
    // A twist drill started on a flat face walks off (LIO-536), as the
    // plan check says too.
    if matches!(features, Features::Holes { holes, .. } if holes.iter().any(|h| h.uncentred)) {
        c.push(warn_chip("centre drill first"));
    }
    // A tip that comes out under the stock has nothing under the hole
    // (LIO-417).
    if let Features::Holes { holes, .. } = features {
        let tip = (holes.iter())
            .map(|h| match h.depth {
                Depth::Through { tip } | Depth::Blind { tip, .. } => tip,
            })
            .reduce(f32::min);
        c.extend(tip.and_then(|tip| under_stock(tip, &p.before(op), !p.notes().is_empty())));
    }
    let stock = p.before(op);
    let top = stock.max[2];
    let (shapes, body, sub): (Vec<Shape>, Vec<Element<'a, Message>>, &'static str) = match features
    {
        Features::Holes {
            diameter,
            thread,
            holes,
        } => {
            let size = thread.clone().unwrap_or_else(|| format!("Ø{}", mm(*diameter)));
            let shapes = (holes.iter().enumerate())
                .map(|(i, h)| Shape::Hole([h.at[0], h.at[1], top], diameter / 2., i + 1))
                .collect();
            let tip = |h: &crate::guide::Hole| match h.depth {
                Depth::Through { tip } | Depth::Blind { tip, .. } => tip,
            };
            let same = |v: &dyn Fn(&crate::guide::Hole) -> f32| {
                (holes.iter()).all(|h| (v(h) - holes.first().map_or(0., v)).abs() < 0.005)
            };
            // Each hole's mouth when they start at different heights.
            let mouths = !same(&|h| h.mouth);
            let mut head = vec!["#", "X", "Y", if thread.is_some() { "Thread" } else { "Ø" }];
            let mut widths = vec![2, 5, 5, 5];
            if mouths {
                head.push("From Z");
                widths.push(5);
            }
            head.push("Depth");
            widths.push(5);
            let rows = (holes.iter().enumerate())
                .map(|(i, h)| {
                    let mut r = vec![(i + 1).to_string(), dro(h.at[0]), dro(h.at[1]), size.clone()];
                    if mouths {
                        r.push(dro(h.mouth));
                    }
                    r.push(match h.depth {
                        Depth::Through { .. } => "through".into(),
                        Depth::Blind { deep, .. } => dro(deep),
                    });
                    r
                })
                .collect();
            // What the depth is measured to, from where, and where the
            // tool's tip goes.
            let kind = match o {
                Some(o) if o.tapping() => "Depth: from the top of the hole to the full thread",
                Some(o) if o.boring() => "Depth: from the top of the hole to its floor",
                _ if p.tool(op).is_some_and(|t| {
                    ["Chamfer mill", crate::model::CENTRE_DRILL].contains(&get(&t.p, "type"))
                }) =>
                {
                    "Depth: from the top of the hole to the tool's tip"
                }
                _ => "Depth: from the top of the hole to the full Ø, not the point",
            };
            let mut said = vec![kind.to_string()];
            if let Some(h) = holes.first().filter(|_| !mouths) {
                said.push(format!("each hole's top at Z {}", dro(h.mouth)));
            }
            if let Some(h) = holes.first().filter(|_| same(&tip)) {
                said.push(format!("the tip goes to Z {}", dro(tip(h))));
            }
            let body = vec![
                table(&head, rows, &widths),
                muted(said.join("; ") + ".").size(13).into(),
            ];
            let sub = match o {
                Some(o) if o.tapping() => "where each hole is tapped",
                Some(o) if o.boring() => "where each hole is bored, to its finished size",
                _ => "where each hole goes",
            };
            (shapes, body, sub)
        }
        Features::Box { lo, hi, through } => {
            let cut = Bounds { min: *lo, max: *hi };
            let middle = [(lo[0] + hi[0]) / 2., (lo[1] + hi[1]) / 2., hi[2]];
            let shapes = vec![
                Shape::Hatch(cut, Hatch::Cut),
                Shape::Word(middle, "1".into(), ACCENT),
            ];
            let depth = if *through {
                format!("through, from Z {}", dro(hi[2]))
            } else {
                format!("{} · Z {} down to {}", dro(hi[2] - lo[2]), dro(hi[2]), dro(lo[2]))
            };
            let body = vec![listed(vec![
                ("#", "1".into()),
                ("X", span(lo[0], hi[0])),
                ("Y", span(lo[1], hi[1])),
                ("Size", format!("{} × {}", dro(hi[0] - lo[0]), dro(hi[1] - lo[1]))),
                ("Depth", depth),
            ])];
            (shapes, body, "where the box is cut")
        }
        Features::Slot {
            a,
            b,
            width,
            top: from,
            floor,
            through,
        } => {
            let shapes = vec![Shape::Slot([a[0], a[1], top], [b[0], b[1], top], width / 2., 1)];
            let depth = if *through {
                format!("through, from Z {}", dro(*from))
            } else {
                format!("{} · Z {} down to {}", dro(from - floor), dro(*from), dro(*floor))
            };
            let end = |e: &[f32; 2]| format!("X {}  Y {}", dro(e[0]), dro(e[1]));
            let body = vec![listed(vec![
                ("#", "1".into()),
                ("End 1", end(a)),
                ("End 2", end(b)),
                ("Width", dro(*width)),
                ("Depth", depth),
            ])];
            (shapes, body, "where the slot is cut")
        }
        Features::Corners(corners) => {
            let shapes = (corners.iter().enumerate())
                .map(|(i, c)| {
                    Shape::Hole([c.centre[0], c.centre[1], top], c.radius, i + 1)
                })
                .collect();
            let rows = (corners.iter().enumerate())
                .map(|(i, c)| {
                    vec![
                        (i + 1).to_string(),
                        c.place.to_string(),
                        dro(c.centre[0]),
                        dro(c.centre[1]),
                        format!("R{}", mm(c.radius)),
                    ]
                })
                .collect();
            let mut body = vec![table(&["#", "Corner", "X", "Y", "R"], rows, &[2, 7, 5, 5, 4])];
            let levels: Vec<String> = (corners.iter())
                .map(|c| format!("Z {} down to {}", dro(c.top), dro(c.bottom)))
                .collect();
            let said = match levels.first() {
                Some(first) if levels.iter().all(|l| l == first) => {
                    format!("X and Y are each round's centre; every corner {first}.")
                }
                _ => format!(
                    "X and Y are each round's centre; by corner, {}.",
                    levels.join(", ")
                ),
            };
            body.push(muted(said).size(13).into());
            (shapes, body, "where each corner is rounded")
        }
    };
    let right = column![chips(c)].extend(body);
    let name = o.map_or(String::new(), |o| o.name().to_string());
    (name, sub, p.drawing(0, stock, shapes, Fill), right)
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::Page;
    use std::sync::Arc;

    /// The guide's job (`guide::tests::job`) with `dro_comp` set to `dro`,
    /// as imported, `edit` made to it, on MACHINE with the manual setup
    /// active and its guide built.
    pub(crate) fn app_with(dro: &str, edit: impl FnOnce(&mut Document)) -> App {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        a.test_dir = Some(dir);
        let (mut doc, mesh) = crate::guide::tests::job(dro);
        doc.file = Some(crate::model::ModelFile {
            name: "four-setup-block.3mf".into(),
            path: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("examples/four-setup-block.3mf"),
            format: "3mf".into(),
            bytes: 1,
            thickness: None,
        });
        doc.active = doc.setups[0].id.clone();
        edit(&mut doc);
        a.doc = doc;
        a.imported_mesh = Some(mesh);
        let _ = a.update(Message::Page(Page::Control));
        build(&mut a);
        a
    }
    pub(crate) fn app(dro: &str) -> App {
        app_with(dro, |_| {})
    }
    /// A number as the DRO readouts show it.
    pub(crate) fn dro_of(v: f32) -> String {
        dro(v)
    }
    /// Builds the active setup's guide in line, as the background run does.
    pub(crate) fn build(a: &mut App) {
        let key = a.guide_key().unwrap();
        let built = shown(&a.doc, a.part_mesh().unwrap(), a.doc.index()).map(Arc::new);
        a.guide = Some((key, built));
        // As `GuideAction::Built` does: the page entered meanwhile.
        a.apply_steps_entry();
    }
    fn texts(a: &App) -> Vec<String> {
        crate::ui::probe::texts(a.view(), iced::Size::new(1280., 800.))
    }
    fn has(a: &App, s: &str) -> bool {
        texts(a).iter().any(|t| t == s)
    }
    fn place(a: &App) -> usize {
        a.guide_at().unwrap().1
    }
    fn guide(a: &App) -> Guide {
        a.guide_at().unwrap().0.guide.clone()
    }
    fn send(a: &mut App, action: GuideAction) {
        let _ = a.update(Message::Guide(action));
    }
    /// The plate's Ø32 bore through 28 on the manual mill (LIO-248):
    /// drilled Ø12, then bored with the head 1 mm a pass on the radius;
    /// the Steps page open on it.
    fn bored_app() -> App {
        let (doc, mesh) = bored_job(|_| {});
        on_page(doc, mesh)
    }
    /// `bored_app`'s job, the head edited by `edit`.
    pub(crate) fn bored_job(edit: impl FnOnce(&mut Tool)) -> (Document, Mesh) {
        use crate::cam::boring::tests::{drill, drilling, head, job, ring};
        let mesh = ring(32., 60., 28.);
        let manual = crate::manual::defaults();
        let on = |mut t: Tool| {
            t.p.insert("machines".into(), manual.id.clone());
            t
        };
        let (d, mut h) = (on(drill("12")), on(head(crate::model::BORE_RETRACTS[0])));
        edit(&mut h);
        let mut hole = drilling(&mesh, &d, "Drilling");
        hole.p.insert("breakthrough".into(), "4.5".into());
        let mut bore = drilling(&mesh, &h, crate::model::BORING);
        for (k, v) in [("hole_diameter", "32"), ("bore_cut", "1")] {
            bore.p.insert(k.into(), v.into());
        }
        for op in [&mut hole, &mut bore] {
            op.p.insert("rpm".into(), "640".into());
            op.p.insert("feed".into(), "60".into());
        }
        let (doc, _) = job(&mesh, manual, vec![(d, hole), (h, bore)]);
        (doc, mesh)
    }
    /// `doc` and its part `mesh` on the Steps page, its guide built.
    fn on_page(mut doc: Document, mesh: Mesh) -> App {
        doc.file = Some(crate::model::ModelFile {
            name: "ring.3mf".into(),
            path: "ring.3mf".into(),
            format: "3mf".into(),
            bytes: 1,
            thickness: None,
        });
        app_of(doc, mesh)
    }
    /// `doc`, as it is, and its part `mesh` on the Steps page, its guide
    /// built.
    pub(crate) fn app_of(doc: Document, mesh: Mesh) -> App {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        a.test_dir = Some(dir);
        a.doc = doc;
        a.imported_mesh = Some(mesh);
        let _ = a.update(Message::Page(Page::Control));
        build(&mut a);
        a
    }
    /// The plate of `guide::tests::holes_job` with its holes `at`, centre
    /// drilled first when `centred`, on the Steps page.
    fn holes_app(at: &[[f32; 2]], centred: bool) -> App {
        let (doc, mesh) = crate::guide::tests::holes_job(at, centred);
        on_page(doc, mesh)
    }
    /// The page of operation `op`: its step and what it lists.
    fn sheet(a: &App, op: usize) -> (usize, Features) {
        (guide(a).steps.iter().enumerate())
            .find_map(|(i, s)| match s {
                Step::Sheet {
                    op: o, features, ..
                } if *o == op => Some((i, features.clone())),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no page for operation {op}"))
    }
    /// The page's texts at step `at`.
    fn page_at(a: &mut App, at: usize) -> Vec<String> {
        send(a, GuideAction::Go(at));
        texts(a)
    }
    /// Whether `row` is in `t`, its cells one after another.
    fn has_row(t: &[String], row: &[&str]) -> bool {
        t.windows(row.len()).any(|w| w.iter().zip(row).all(|(a, b)| a == b))
    }
    /// LIO-688: a manual mill setup's Steps are a page for the work zero,
    /// a page an operation, and the hand-over: no tool, pass, hole, pattern
    /// or check pages. Next and Back go page by page, the place saved with
    /// the project and no undo step.
    #[test]
    fn a_manual_mill_setup_is_its_zero_then_a_page_an_operation() {
        let mut a = app("Yes");
        let g = guide(&a);
        let kinds: Vec<String> = (g.steps.iter())
            .map(|s| {
                let said = format!("{s:?}");
                said.split([' ', '(']).next().unwrap().to_string()
            })
            .collect();
        assert_eq!(kinds, ["Zero", "Sheet", "Sheet", "Sheet", "HandOver"]);
        let names: Vec<&str> = g.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Set up", "Face", "Step", "Holes", "Hand over"]);
        assert!(g.sections.iter().all(|s| s.steps.len() == 1), "{g:?}");
        // The list says what each operation cuts.
        let t = texts(&a);
        assert_eq!(t.iter().filter(|s| *s == "1 box").count(), 2, "{t:?}");
        assert!(t.iter().any(|s| s == "4 holes"), "{t:?}");
        // An operation's page heads with its name, tool and speed.
        let t = page_at(&mut a, 3);
        for want in ["Holes", "Ø4 Drill 4", "1600 rpm", "Section done ▶"] {
            assert!(t.iter().any(|s| s == want), "{want} in {t:?}");
        }
        // Nothing of the old pages on any.
        for at in 0..g.steps.len() {
            let t = page_at(&mut a, at);
            for gone in [
                "PICK UP Z WITH THE TOOL",
                "ENTER IN THE DRO",
                "TOUCH, THEN SET THE DRO",
                "MEASURE",
                "Hole done ▶",
                "Pass done ▶",
            ] {
                assert!(!t.iter().any(|s| s == gone), "{gone} at {at}: {t:?}");
            }
            assert!(!t.iter().any(|s| s.starts_with("lock ")), "{at}: {t:?}");
        }
        send(&mut a, GuideAction::Go(0));
        let edits = a.project_dirty;
        send(&mut a, GuideAction::Next);
        send(&mut a, GuideAction::Next);
        assert_eq!(place(&a), 2);
        send(&mut a, GuideAction::Back);
        assert_eq!(place(&a), 1);
        assert!(a.dirty && a.undo.is_none());
        assert_eq!(a.project_dirty, edits);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("job.axis.json");
        crate::storage::save(&path, &a.doc).unwrap();
        let doc = crate::storage::load(&path).unwrap();
        assert_eq!(doc.guide.get(&doc.setups[0].id), Some(&1));
    }
    /// LIO-688: the zero page says where X0, Y0 and Z0 are on the stock,
    /// marks the zero on the picture of it held, and says how it is held;
    /// not how to pick it up.
    #[test]
    fn the_zero_page_says_where_the_zero_is_and_marks_it_on_the_held_stock() {
        // The job's zero is on the stock top's front-left corner.
        let a = app("Yes");
        assert!(matches!(guide(&a).steps[0], Step::Zero { .. }));
        let t = texts(&a);
        for want in [
            "WORK ZERO",
            "X0",
            "the left face",
            "Y0",
            "the front face",
            "Z0",
            "the top of the stock",
            "X × Y × Z",
        ] {
            assert!(t.iter().any(|s| s == want), "{want} in {t:?}");
        }
        let shown = a.guide_at().unwrap().0;
        assert_eq!(shown.zero.len(), 1);
        assert_eq!(shown.zero[0].label, "X0 Y0 Z0");
        let z = shown.zero[0].a;
        let (w, h) = (PICTURE.0 as f32, PICTURE.1 as f32);
        assert!(z.x > 0. && z.y > 0. && z.x < w && z.y < h, "{z:?}");
        for gone in ["TOUCH, THEN SET THE DRO", "DRO comp off"] {
            assert!(!t.iter().any(|s| s == gone), "{gone} in {t:?}");
        }
        assert!(!t.iter().any(|s| s.contains("edge finder")), "{t:?}");
        // Its centre on the top, in a vise: halfway between the faces; the
        // vise, its grip and the parallels.
        let a = app_with("Yes", |d| {
            crate::guide::tests::in_the_vise(d);
            (d.setups[0].p).insert("origin".into(), "Stock top center".into());
        });
        let shown = a.guide_at().unwrap().0;
        let Step::Zero {
            stock,
            holding,
            vise: Some(v),
            ..
        } = shown.guide.steps[0].clone()
        else {
            panic!("{:?}", shown.guide.steps[0])
        };
        close(stock.min[0], -stock.max[0]);
        close(stock.min[1], -stock.max[1]);
        close(stock.max[2], 0.);
        let t = texts(&a);
        let grip = format!("{} in the jaws", mm(v.grip));
        for want in [
            "the middle, halfway between the left and right faces",
            "the middle, halfway between the front and back faces",
            "the top of the stock",
            "jaws on the Y sides",
            grip.as_str(),
            "top face up",
        ] {
            assert!(t.iter().any(|s| s == want), "{want} in {t:?}");
        }
        assert_eq!(shown.vise.len(), 2);
        for j in &shown.vise {
            close(j.max[2], holding.bottom + v.grip);
            close(j.max[2] - j.min[2], v.jaw_height);
        }
        assert_eq!(shown.parallels.len(), 2);
        let high = format!(
            "on parallels {} high",
            mm(stock.min[2] - shown.parallels[0].min[2])
        );
        assert!(t.contains(&high), "{high} in {t:?}");
        // A zero off the stock's faces: how far from them.
        let off = Bounds {
            min: [-12.3, 5., -20.],
            max: [87.7, 45., -2.],
        };
        assert_eq!(zero_place(&off, 0), "12.3 right of the left face");
        assert_eq!(zero_place(&off, 1), "5 in front of the front face");
        assert_eq!(zero_place(&off, 2), "2 above the top of the stock");
    }
    /// LIO-688: a Box's page gives where it is on X and Y from the zero,
    /// its size, and how deep it goes from the top it meets.
    #[test]
    fn a_box_page_gives_its_place_size_and_depth() {
        let mut a = app("Yes");
        let s = a.guide_at().unwrap().0.boxes[0];
        // The Face: the whole top, 2 off.
        let (at, face) = sheet(&a, 0);
        let Features::Box {
            lo,
            hi,
            through: false,
        } = face
        else {
            panic!("{face:?}")
        };
        for k in 0..2 {
            close(lo[k], s.min[k]);
            close(hi[k], s.max[k]);
        }
        close(hi[2], 0.);
        close(lo[2], -2.);
        let t = page_at(&mut a, at);
        let (w, d) = (s.max[0] - s.min[0], s.max[1] - s.min[1]);
        for want in [
            format!("{} to {}", dro(s.min[0]), dro(s.max[0])),
            format!("{} × {}", dro(w), dro(d)),
            "2.00 · Z 0.00 down to −2.00".into(),
        ] {
            assert!(t.contains(&want), "{want} in {t:?}");
        }
        assert!(has_row(&t, &["#", "1"]), "{t:?}");
        // The Step: open on −Y, its wall at Y 15.5 with 0.5 left, down to
        // −12 with 0.5 left, from the Face's −2.
        let (at, step) = sheet(&a, 1);
        let Features::Box {
            lo,
            hi,
            through: false,
        } = step
        else {
            panic!("{step:?}")
        };
        close(lo[1], s.min[1]);
        close(hi[1], 15.);
        close(hi[2], -2.);
        close(lo[2], -11.5);
        let t = page_at(&mut a, at);
        for want in [
            format!("{} to 15.00", dro(s.min[1])),
            "9.50 · Z −2.00 down to −11.50".into(),
        ] {
            assert!(t.contains(&want), "{want} in {t:?}");
        }
    }
    /// LIO-688: a Drill's page lists its holes, numbered as drawn, each
    /// with X, Y, Ø and depth; one started on a flat face is warned, as
    /// its plan check says, and not after a centre drilling, whose page
    /// gives the depth to its tip.
    #[test]
    fn a_drill_page_lists_each_hole_with_its_place_size_and_depth() {
        let mut a = app("Yes");
        let (at, f) = sheet(&a, 2);
        let Features::Holes {
            diameter,
            thread: None,
            holes,
        } = f
        else {
            panic!("{f:?}")
        };
        close(diameter, 4.);
        assert_eq!(holes.len(), 4);
        let t = page_at(&mut a, at);
        assert!(has_row(&t, &["#", "X", "Y", "Ø", "Depth"]), "{t:?}");
        for (i, h) in holes.iter().enumerate() {
            let depth = match h.depth {
                Depth::Through { .. } => "through".to_string(),
                Depth::Blind { deep, .. } => dro(deep),
            };
            let n = (i + 1).to_string();
            let row = [n.as_str(), &dro(h.at[0]), &dro(h.at[1]), "Ø4", &depth];
            assert!(has_row(&t, &row), "{row:?} in {t:?}");
        }
        assert!(t.iter().any(|s| s == "centre drill first"), "{t:?}");
        let mut a = centred_app();
        let (at, f) = sheet(&a, 2);
        let Features::Holes { holes, .. } = f else {
            panic!("{f:?}")
        };
        // The centre drill's pilot tip, under its cone at the top.
        for h in &holes {
            let Depth::Blind { deep, tip } = h.depth else {
                panic!("{h:?}")
            };
            close(deep, h.mouth - tip);
        }
        let t = page_at(&mut a, at);
        assert!(
            t.iter()
                .any(|s| s.starts_with("Depth: from the top of the hole to the tool's tip")),
            "{t:?}"
        );
        assert!(!t.iter().any(|s| s == "centre drill first"), "{t:?}");
        let (at, _) = sheet(&a, 3);
        assert!(!page_at(&mut a, at).iter().any(|s| s == "centre drill first"));
    }
    /// The hole of `drilled_job` (LIO-450, LIO-688).
    fn drilled_hole(
        mesh: &Mesh,
        size: &str,
        through: bool,
        bottom: &str,
        air: &str,
    ) -> crate::guide::Hole {
        let (_, g, _) = drilled(mesh, size, through, bottom, air);
        match &g.steps[1] {
            Step::Sheet {
                features: Features::Holes { holes, .. },
                ..
            } => holes[0].clone(),
            s => panic!("{s:?}"),
        }
    }
    /// A hole's depth as its Check measured it (LIO-450): through when it
    /// breaks out, from the stock's top whether the cut starts there or
    /// over it (Top); blind, from its mouth to its full Ø, the drill's
    /// point left out, or to its tip when the cut never reaches it.
    #[test]
    fn a_holes_depth_is_through_or_to_its_full_diameter_from_its_mouth() {
        let mesh = crate::cam::boring::tests::ring(6.8, 30., 18.5);
        let mut mouth = None;
        for air in ["0", "1.5"] {
            let h = drilled_hole(&mesh, "6.8", true, "0", air);
            assert!(matches!(h.depth, Depth::Through { .. }), "{air}: {h:?}");
            close(h.mouth, *mouth.get_or_insert(h.mouth));
        }
        // Ø8 8 deep, its drill-point floor 2.4 lower.
        let point = 4. * crate::simulation::POINT_RISE;
        let section = [
            (0., -4.),
            (12., -4.),
            (12., 10.),
            (4., 10.),
            (4., 2.),
            (0., 2. - point),
        ];
        let mesh = crate::mesh::normalize(crate::cam::tests::revolved(&section)).unwrap();
        let h = drilled_hole(&mesh, "8", false, "0", "0");
        let Depth::Blind { deep, tip } = h.depth else {
            panic!("{h:?}")
        };
        close(deep, 8.);
        close(h.mouth - tip, 8. + point);
        // Stopped 9.4 over the floor, 1 into the stock: to its tip.
        let h = drilled_hole(&mesh, "8", false, "9.4", "0");
        let Depth::Blind { deep, tip } = h.depth else {
            panic!("{h:?}")
        };
        close(deep, 1.);
        close(h.mouth - tip, 1.);
    }
    /// LIO-688: a Boring's page gives each hole's finished size and depth:
    /// no passes, head settings, dial divisions or measuring.
    #[test]
    fn a_boring_page_gives_the_finished_size_only() {
        let mut a = bored_app();
        let kinds: Vec<bool> = (guide(&a).steps.iter())
            .map(|s| matches!(s, Step::Sheet { .. }))
            .collect();
        assert_eq!(kinds, [false, true, true, false]);
        let (at, bore) = sheet(&a, 1);
        let Features::Holes {
            diameter,
            thread: None,
            holes,
        } = bore
        else {
            panic!("{bore:?}")
        };
        close(diameter, 32.);
        assert_eq!(holes.len(), 1);
        let h = &holes[0];
        let depth = match h.depth {
            Depth::Through { .. } => "through".to_string(),
            Depth::Blind { deep, .. } => {
                assert!(deep > 27., "{h:?}");
                dro(deep)
            }
        };
        let t = page_at(&mut a, at);
        let row = ["1", &dro(h.at[0]), &dro(h.at[1]), "Ø32", depth.as_str()];
        assert!(has_row(&t, &row), "{row:?} in {t:?}");
        for want in ["640 rpm", "1 hole"] {
            assert!(t.iter().any(|s| s == want), "{want} in {t:?}");
        }
        for gone in ["divisions", "measure", "head", "pass "] {
            assert!(!t.iter().any(|s| s.contains(gone)), "{gone} in {t:?}");
        }
    }
    /// LIO-688: a Slot's page gives its two ends, its width and its depth
    /// from the top it starts in, or through.
    #[test]
    fn a_slot_page_gives_its_ends_width_and_depth() {
        for through in [false, true] {
            let (doc, mesh) = slot_job(through, if through { "-8" } else { "-5" });
            let mut a = on_page(doc, mesh);
            let (at, f) = sheet(&a, 0);
            let Features::Slot {
                a: one,
                b: two,
                width,
                top,
                floor,
                through: said,
            } = f
            else {
                panic!("{f:?}")
            };
            assert_eq!(said, through);
            close(width, 10.);
            close(top, 0.);
            close(floor, if through { -8. } else { -5. });
            for (e, x) in [(one, -2.5), (two, 2.5)] {
                close(e[0], x);
                close(e[1], 0.);
            }
            let t = page_at(&mut a, at);
            let depth = match through {
                true => "through, from Z 0.00".to_string(),
                false => "5.00 · Z 0.00 down to −5.00".into(),
            };
            for row in [
                ["End 1", "X −2.50  Y 0.00"],
                ["End 2", "X 2.50  Y 0.00"],
                ["Width", "10.00"],
                ["Depth", depth.as_str()],
            ] {
                assert!(has_row(&t, &row), "{row:?} in {t:?}");
            }
        }
    }
    /// LIO-688: a Round corners' page lists each corner, where it is on
    /// the part, its round's centre and its radius.
    #[test]
    fn a_round_corners_page_lists_each_corner_and_its_radius() {
        let (doc, mesh) = crate::corner::tests::plate_job("Y");
        let setup = &doc.setups[0];
        let corners = crate::corner::picked(setup, &mesh, &setup.operations[0]).unwrap();
        let mut a = on_page(doc, mesh);
        let zero = guide(&a).zero;
        let (at, f) = sheet(&a, 0);
        let Features::Corners(c) = f else {
            panic!("{f:?}")
        };
        assert_eq!(c.len(), corners.len());
        for (r, k) in c.iter().zip(&corners) {
            assert_eq!(r.place, k.place());
            close(r.radius, k.r);
            close(r.centre[0], k.centre[0] - zero[0]);
            close(r.centre[1], k.centre[1] - zero[1]);
        }
        let t = page_at(&mut a, at);
        for (i, r) in c.iter().enumerate() {
            let n = (i + 1).to_string();
            let radius = format!("R{}", mm(r.radius));
            let row = [
                n.as_str(),
                r.place,
                &dro(r.centre[0]),
                &dro(r.centre[1]),
                radius.as_str(),
            ];
            assert!(has_row(&t, &row), "{row:?} in {t:?}");
        }
        let count = format!("{} corners", c.len());
        assert!(t.contains(&count), "{count} in {t:?}");
    }
    /// The agent's read of a step says what the page says (LIO-470,
    /// LIO-688): the zero page the stock and the setup's notes, an
    /// operation's page each feature.
    #[test]
    fn the_agents_read_of_the_steps_says_what_the_page_says() {
        let note =
            "Face up on two 15 mm parallels at X-18 and X+18, clear of the holes. Add a stop.";
        let a = app_with("Yes", |d| {
            d.setups[0].p.insert("notes".into(), format!("  {note}\n"));
        });
        let g = guide(&a);
        let read = crate::mcp::reads::step_detail(&a.doc, 0, &g.steps[0]);
        assert_eq!(read["kind"], "Zero");
        let text = read["text"].as_str().unwrap();
        assert!(text.ends_with(&format!("Setup notes: {note}")), "{read}");
        let (at, f) = sheet(&a, 2);
        let Features::Holes { holes, .. } = f else {
            panic!("{f:?}")
        };
        let read = crate::mcp::reads::step_detail(&a.doc, 0, &g.steps[at]);
        assert_eq!(read["kind"], "Sheet");
        assert!(
            read["text"]
                .as_str()
                .unwrap()
                .ends_with("at 1600 rpm: 4 hole(s), Ø4."),
            "{read}"
        );
        let listed = read["features"].as_array().unwrap();
        assert_eq!(listed.len(), holes.len());
        for (v, h) in listed.iter().zip(&holes) {
            close(v["x"].as_f64().unwrap() as f32, h.at[0]);
            close(v["y"].as_f64().unwrap() as f32, h.at[1]);
        }
        let b = app("Yes");
        let g = guide(&b);
        let read = crate::mcp::reads::step_detail(&b.doc, 0, &g.steps[0]);
        assert!(
            !read["text"].as_str().unwrap().contains("Setup notes"),
            "{read}"
        );
    }
    /// Back and Next in one place on every page, Back dead on the first.
    #[test]
    fn back_and_next_are_in_one_place_on_every_screen() {
        let mut a = app("Yes");
        let g = guide(&a);
        let window = Size::new(1280., 800.);
        let bottom = Rectangle::new(Point::new(0., 400.), Size::new(1280., 400.));
        let mut spans = vec![];
        for at in 0..g.steps.len() {
            send(&mut a, GuideAction::Go(at));
            let presses = crate::ui::probe::presses(a.view(), window, bottom, 5.);
            let span = |want: &dyn Fn(&str) -> bool| {
                let points: Vec<Point> = (presses.iter())
                    .filter(|(_, m)| want(&format!("{m:?}")))
                    .map(|(p, _)| *p)
                    .collect();
                (!points.is_empty()).then(|| {
                    points.iter().fold([f32::MAX, f32::MAX, 0., 0.], |b, p| {
                        [b[0].min(p.x), b[1].min(p.y), b[2].max(p.x), b[3].max(p.y)]
                    })
                })
            };
            let next = span(&|m| m == "Guide(Next)" || m == "Guide(Open(1))");
            spans.push((next, span(&|m| m == "Guide(Back)")));
        }
        assert!(spans[0].0.is_some() && spans[0].1.is_none(), "{spans:?}");
        assert!(spans[1].1.is_some());
        for s in &spans[1..] {
            assert_eq!(s.0, spans[0].0, "{spans:?}");
            assert_eq!(s.1, spans[1].1, "{spans:?}");
        }
    }
    /// The job's pages at 1280 × 800 and 1024 × 700 into `AXIS_RENDER_DIR`,
    /// with the vise, a bore, a slot and corners (LIO-688).
    #[test]
    #[ignore]
    fn render_the_mill_pages() {
        let dir = std::path::PathBuf::from(std::env::var("AXIS_RENDER_DIR").unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let (plate, mesh) = crate::corner::tests::plate_job("Y");
        let (slot, slot_mesh) = slot_job(false, "-5");
        let apps = [
            ("job", app("Yes")),
            ("vise", app_with("Yes", crate::guide::tests::in_the_vise)),
            ("bore", bored_app()),
            ("slot", on_page(slot, slot_mesh)),
            ("corners", on_page(plate, mesh)),
            ("grid", holes_app(&crate::guide::tests::plate_grid(), true)),
        ];
        for (name, mut a) in apps {
            for at in 0..guide(&a).steps.len() {
                for (w, h) in [(1280, 800), (1024, 700)] {
                    (a.window_width, a.window_height) = (w as f32, h as f32);
                    send(&mut a, GuideAction::Go(at));
                    render(&a, (w, h), &dir.join(format!("{name}-{at}-{w}.png")));
                }
            }
        }
    }
    #[test]
    fn layout_suggestion_reports_inherited_contact_as_well_as_clearance() {
        for stepped in [false, true] {
            let (mut doc, mesh) = crate::guide::tests::squared_job();
            if stepped {
                for (k, v) in [("y_min", "-5"), ("y_max", "5")] {
                    doc.setups[0].operations[0].p.insert(k.into(), v.into());
                }
            }
            let mut generated = vec![];
            let mut before = None;
            let mut bands = None;
            crate::simulation::sequence(&doc, &mesh, Some((2, 0)), |si, oi, result| {
                if si == 2 {
                    let sim = result.unwrap();
                    bands = crate::post::bands(&doc.setups[2], &mesh, &sim);
                    before = sim.initial_volume.clone().zip(sim.frame);
                    generated.push((oi, Some(sim.plan)));
                }
            });
            let mut a = app("Yes");
            doc.file = a.doc.file.clone();
            a.doc = doc;
            a.imported_mesh = Some(mesh);
            a.place_layout(2, &generated, bands.as_ref(), before.as_ref());
            assert_eq!(
                a.status.contains("cannot be held in the vise"),
                stepped,
                "{}",
                a.status
            );
        }
    }
    #[test]
    fn forty_five_passes_have_readable_separate_tiles() {
        let labels = (0..45)
            .map(|n| {
                (
                    format!("−1.50 · {}", n + 1),
                    Some(Message::Guide(GuideAction::Go(n))),
                )
            })
            .collect();
        let placed = crate::ui::probe::placed(tiles(labels, 44, false), Size::new(320., 2000.));
        assert_eq!(placed.len(), 45);
        for (i, (_, b)) in placed.iter().enumerate() {
            assert!(
                b.height <= 20. && b.x >= 0. && b.x + b.width <= 320.,
                "{b:?}"
            );
            for (_, other) in &placed[..i] {
                assert!(b.intersection(other).is_none(), "{b:?} overlaps {other:?}");
            }
        }
        let labels = (0..45)
            .map(|n| {
                (
                    format!("−1.50 · {}", n + 1),
                    Some(Message::Guide(GuideAction::Go(n))),
                )
            })
            .collect();
        let last = placed.last().unwrap().1;
        let presses =
            crate::ui::probe::presses(tiles(labels, 44, false), Size::new(320., 2000.), last, 15.);
        assert!(
            presses
                .iter()
                .any(|(_, m)| matches!(m, Message::Guide(GuideAction::Go(44))))
        );
    }
    /// LIO-563: `bored_app`'s Ø50 head, its bar reaching 40 mm below the
    /// body, bores the Ø32 hole by a toe clamp on +Y whose nose reaches 12
    /// mm onto the 64 mm stock, to Y20. The body comes down to 12 mm over
    /// the stock's top, over the clamp: the Steps refuse the setup, naming
    /// the operation, the clamp and the room it needs, and open the bore.
    /// With a bar reaching 60 mm the body stays over the clamp and its gap,
    /// and the pages are back, though the plan travels from the stock's
    /// corner at Z15, through the clamp on -Y: the pages lift the head
    /// clear before X and Y move.
    #[test]
    fn the_steps_refuse_a_boring_head_whose_body_hits_a_clamp() {
        let clamped = |reach: &str| {
            let (mut doc, mesh) = bored_job(|h| {
                h.p.insert("length".into(), reach.into());
                h.p.insert("stickout".into(), "120".into());
            });
            doc.workholding = crate::holding::starters();
            let manual = doc.setups[0].machine.id.clone();
            let clamp = (doc.workholding.iter_mut())
                .find(|h| h.kind() == "Clamp")
                .unwrap();
            clamp.p.insert("machines".into(), manual);
            let clamp = clamp.id.clone();
            let p = &mut doc.setups[0].p;
            p.insert("holding".into(), "Clamps".into());
            p.insert("holder".into(), clamp);
            p.insert("clamps".into(), "+Y:0;-Y:20".into());
            (doc, mesh)
        };
        let (doc, mesh) = clamped("40");
        let bore = doc.setups[0].operations[1].name().to_string();
        let why = crate::guide::build(&doc, &mesh, 0).unwrap_err();
        assert!(
            why.starts_with(&format!(
                "{bore}: the tool body hits the clamp on +Y (offset 0) at X0.0 Y0.0 Z"
            )) && why.ends_with(
                "(within the 2 mm gap). It needs 7.0 mm more room across to keep the gap. Hold the part so that the clamp on +Y (offset 0) stands clear of the boring head, or bore with a bar that reaches farther below its body."
            ),
            "{why}"
        );
        // The page says it, with the button that opens the bore.
        let a = on_page(doc, mesh);
        let t = texts(&a);
        assert!(t.contains(&why), "{t:?}");
        assert!(
            t.iter().any(|s| *s == format!("Open {bore} in planning ▶")),
            "{t:?}"
        );
        let (doc, mesh) = clamped("60");
        let g = crate::guide::build(&doc, &mesh, 0).unwrap();
        assert!(g.steps.iter().any(|s| matches!(s, Step::Sheet { op: 1, .. })));
    }
    /// `app` with an A2.5×6.3 centre drilling before its Holes, as Add
    /// would put it there.
    fn centred_app() -> App {
        app_with("Yes", |doc| {
            let mut c =
                crate::model::Tool::new("Centre drill A2.5×6.3", crate::model::CENTRE_DRILL, "6.3");
            c.set_type(crate::model::CENTRE_DRILL);
            c.p.insert("stickout".into(), "40".into());
            doc.tools.push(c);
            let (ops, _) = crate::centre::needed(doc, 0, None);
            crate::centre::insert(doc, 0, ops);
        })
    }
    /// The app counts a drill as centred as the Steps read the stock
    /// when their guide is built (LIO-536): a milling operation between
    /// the centre drilling and the drill that leaves the spots (the Step
    /// again) does not undo it. Without the guide it may have taken them
    /// away: the drill is not counted, so the editor warns and offers a
    /// centre drilling again (MCP `add_centring` reads the same).
    #[test]
    fn the_app_counts_a_centring_the_stock_still_has() {
        let a = centred_app();
        assert!(a.centred(3));
        let mut a = app_with("Yes", |doc| {
            let mut c =
                crate::model::Tool::new("Centre drill A2.5×6.3", crate::model::CENTRE_DRILL, "6.3");
            c.set_type(crate::model::CENTRE_DRILL);
            c.p.insert("stickout".into(), "40".into());
            doc.tools.push(c);
            let (ops, _) = crate::centre::needed(doc, 0, None);
            crate::centre::insert(doc, 0, ops);
            let mut again = doc.setups[0].operations[1].clone();
            again.id = "step-again".into();
            doc.setups[0].operations.insert(3, again);
        });
        if let Some(Err(why)) = a.current_guide() {
            panic!("{why}");
        }
        let uncentred: Vec<bool> = (guide(&a).steps.iter())
            .find_map(|s| match s {
                Step::Sheet {
                    op: 4,
                    features: Features::Holes { holes, .. },
                    ..
                } => Some(holes.iter().map(|h| h.uncentred).collect()),
                _ => None,
            })
            .unwrap();
        assert!(uncentred.iter().all(|u| !*u), "{uncentred:?}");
        assert!(a.centred(4));
        assert!(!crate::centre::centred(a.doc.setup(), 4, a.part_mesh()));
        a.guide = None;
        assert!(!a.centred(4));
        a.page = crate::Page::Work;
        let _ = a.update(Message::SelectOp(4));
        let _ = a.update(Message::Tab("Tool".into()));
        let t = texts(&a);
        assert!(t.iter().any(|s| s == "Centre drill first"), "{t:?}");
        assert!(
            t.iter().any(|s| s == "+ Add a centre drilling before it"),
            "{t:?}"
        );
    }
    /// The Zero page shows the setup's notes, which say what supports the
    /// part and where; none without (LIO-487). A slot's page, which leaves
    /// what is under the part to the user, points back to them.
    #[test]
    fn the_hold_page_shows_the_setups_notes() {
        let note =
            "Face up on two 15 mm parallels at X-18 and X+18, clear of the holes. Add a stop.";
        let a = app_with("Yes", |d| {
            d.setups[0].p.insert("notes".into(), format!("  {note}\n"));
        });
        assert!(matches!(guide(&a).steps[place(&a)], Step::Zero { .. }));
        let t = texts(&a);
        let i = (t.iter().position(|s| s == "SETUP NOTES")).expect("the notes' label");
        assert_eq!(t[i + 1], note);
        // The label and the notes fit at 1024 × 700, above the buttons.
        let mut a = a;
        (a.window_width, a.window_height) = (1024., 700.);
        let placed = crate::ui::probe::placed(a.view(), Size::new(1024., 700.));
        let at = |s: &str| (placed.iter()).find(|(t, _)| t == s).map(|(_, r)| *r);
        let (notes, next) = (at(note).expect("the notes"), at("Next ▶").expect("Next"));
        assert!(notes.y + notes.height < next.y, "{notes:?} {next:?}");
        let a = app("Yes");
        assert!(!texts(&a).iter().any(|s| s == "SETUP NOTES"));
    }
    fn close(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-3, "{a} != {b}");
    }
    #[test]
    fn a_setup_that_cannot_be_shown_names_its_operation_and_opens_it() {
        // The Step's speed off the gears: the page names the Step, and its
        // button opens it in MANUFACTURE on its Tool tab.
        let mut a = app_with("Yes", |doc| {
            (doc.setups[0].operations[1].p).insert("rpm".into(), "700".into());
        });
        let gears = "Spindle speed must be one of the machine's gears:";
        assert!(
            texts(&a)
                .iter()
                .any(|s| s.starts_with(&format!("Step: {gears}"))),
            "{:?}",
            texts(&a)
        );
        // "planning", not MANUFACTURE (LIO-429).
        assert!(has(&a, "Open Step in planning ▶"), "{:?}", texts(&a));
        let Some(Err(e)) = a.current_guide().cloned() else {
            panic!()
        };
        let _ = a.update(Message::Guide(GuideAction::Fix(
            Some(1),
            e.strip_prefix("Step: ").unwrap().into(),
        )));
        assert_eq!(
            (a.page, a.selected, a.tab.as_str()),
            (Page::Work, Some(1), "Tool")
        );
        // No operation: the button opens the setup on the Work page.
        let mut a = app_with("Yes", |doc| doc.setups[0].operations.clear());
        assert!(has(&a, "Open the setup in planning ▶"), "{:?}", texts(&a));
        a.selected = Some(0);
        let _ = a.update(Message::Guide(GuideAction::Fix(None, String::new())));
        assert_eq!((a.page, a.selected), (Page::Work, None));
    }
    #[test]
    fn no_two_words_of_a_steps_drawing_lie_on_each_other() {
        let dir = tempfile::tempdir().unwrap();
        // A slot close up: a Ø12 mill in a 16.5 wide counterbore.
        let slot = {
            let mesh = crate::mesh::normalize(crate::slot::tests::counterbore(
                (60., 50., 20.),
                21.5,
                16.5,
                2.,
                false,
            ))
            .unwrap();
            let fields = [
                ("slot_width", "16.5"),
                ("slot_length", "21.5"),
                ("z_top", "0"),
                ("z_bottom", "-2"),
                ("through", "No"),
                ("stepdown", "1"),
            ];
            let mut doc = crate::slot::tests::job(&mesh, "12", &fields);
            doc.active = doc.setups[0].id.clone();
            on_page(doc, mesh)
        };
        let (plate, mesh) = crate::corner::tests::plate_job("Y");
        let apps = [
            ("block", app("Yes")),
            ("vise", app_with("Yes", crate::guide::tests::in_the_vise)),
            ("corners", on_page(plate, mesh)),
            ("slot", slot),
            ("bore", bored_app()),
            (
                "grid",
                holes_app(&crate::guide::tests::plate_grid(), false),
            ),
            (
                "circle",
                holes_app(&crate::guide::tests::plate_circle(), true),
            ),
        ];
        // The screens with labelled drawings: the zero page and each
        // operation's (LIO-688).
        const KINDS: [&str; 2] = ["Zero", "Sheet"];
        // No two words of a drawing on each other, nor one said twice side
        // by side (LIO-545), nor one across a line not to pass
        // or the stock's outline, but on a white tag (LIO-546).
        let clear = |drawn: Vec<Vec<(String, Rectangle)>>, what: &str| {
            for words in drawn {
                let (tags, words): (Vec<_>, Vec<_>) =
                    words.into_iter().partition(|(s, _)| s == "tag");
                let tagged = |r: &Rectangle| tags.iter().any(|(_, t)| t.contains(r.center()));
                for (j, (s, r)) in words.iter().enumerate() {
                    for (t, o) in &words[..j] {
                        let line = |s: &str| s.strip_prefix("line ").map(str::to_string);
                        match (line(s), line(t)) {
                            (Some(_), Some(_)) => continue,
                            (Some(_), None) if tagged(o) => continue,
                            (None, Some(_)) if tagged(r) => continue,
                            _ => {}
                        }
                        assert!(!r.intersects(o), "{what}: {s:?} {r:?} on {t:?} {o:?}");
                        let gap = (o.x - r.x - r.width)
                            .max(r.x - o.x - o.width)
                            .max(o.y - r.y - r.height)
                            .max(r.y - o.y - o.height);
                        let twice = s == t && gap <= r.height;
                        assert!(!twice, "{what}: {s:?} {r:?} twice, {o:?}");
                    }
                }
            }
        };
        let mut seen = std::collections::BTreeSet::new();
        for (name, mut a) in apps {
            if let Some((_, Err(e))) = &a.guide {
                panic!("{name}: {e}");
            }
            // Every screen as the shop tablet gets it: its pictures drawn
            // ~410 × 330, smaller still (LIO-477).
            DRAWN.take();
            let _ = tablet::steps(&a.doc, Ok(a.part_mesh().unwrap()));
            clear(DRAWN.take(), &format!("{name} on the tablet"));
            let steps = guide(&a).steps;
            for (i, step) in steps.iter().enumerate() {
                let kind = format!("{step:?}")
                    .split([' ', '('])
                    .next()
                    .unwrap()
                    .to_string();
                if !KINDS.contains(&kind.as_str()) {
                    continue;
                }
                send(&mut a, GuideAction::Go(i));
                // The narrow window class draws the pictures ~440 px wide,
                // where they crowd (a debug render is slow: one width).
                let (w, h) = (1100, 800);
                (a.window_width, a.window_height) = (w as f32, h as f32);
                DRAWN.take();
                render(&a, (w, h), &dir.path().join("steps.png"));
                let drawn = DRAWN.take();
                if !drawn.is_empty() {
                    seen.insert(kind.clone());
                }
                clear(drawn, &format!("{name} step {i} ({kind}) at {w} px"));
            }
        }
        for kind in KINDS {
            assert!(seen.contains(kind), "{kind} not drawn: {seen:?}");
        }
    }
    #[test]
    fn the_page_shows_without_a_controller_and_says_when_it_is_not_ready() {
        let mut a = app("Yes");
        assert!(a.control.is_none() && a.steps_page());
        assert!(has(&a, "Set up") && has(&a, "Manual mill"));
        // Still being built, or failed.
        let key = a.guide_key().unwrap();
        let built = a.guide.take();
        assert!(has(&a, "Simulating the setup…"));
        let _ = a.update(Message::Guide(GuideAction::Built(
            key ^ 1,
            Err("old".into()),
        )));
        assert!(a.guide.is_none());
        let _ = a.update(Message::Guide(GuideAction::Built(
            key,
            Err("It broke.".into()),
        )));
        assert!(has(&a, "It broke.") && has(&a, "Open the setup in planning ▶"));
        a.guide = built;
        // The setup card in MANUFACTURE opens it.
        a.page = Page::Work;
        assert!(has(&a, "Steps ▶"));
        send(&mut a, GuideAction::Open(0));
        assert!(a.page == Page::Control && a.steps_page());
        // A CNC setup still shows the CNC's MACHINE page.
        send(&mut a, GuideAction::Open(1));
        assert_eq!(a.doc.active, a.doc.setups[1].id);
        assert!(!a.steps_page());
        assert!(has(&a, "Not connected."));
        assert!(!has(&a, "Set up"));
    }
    #[test]
    fn a_cnc_machine_on_the_steps_page_shows_its_control_page_and_back() {
        let mut a = app_with("Yes", |doc| {
            let mut m = crate::model::Machine::new(3);
            m.p.insert("name".into(), "Router".into());
            m.p.insert("controller".into(), "UC100".into());
            doc.machines.push(m);
        });
        let id = a.doc.machines.last().unwrap().id.clone();
        assert!(a.steps_page() && has(&a, "CNC machines") && has(&a, "Router ▶"));
        let _ = a.update(Message::ControlMachine(id.clone()));
        assert!(!a.steps_page());
        assert_eq!(a.control_target().map(|m| &m.id), Some(&id));
        assert!(has(&a, "Not connected.") && has(&a, "Manual mill steps"));
        let _ = a.update(Message::ShowGuide);
        assert!(a.steps_page());
        // Opening the setup's steps again shows them.
        let _ = a.update(Message::ControlMachine(id));
        assert!(!a.steps_page());
        send(&mut a, GuideAction::Open(0));
        assert!(a.steps_page());
    }
    #[test]
    fn a_cnc_machine_on_the_lathe_steps_page_shows_its_control_page_and_back() {
        let mut a = lathe_app("Diameter", |doc| {
            let mut m = crate::model::Machine::new(3);
            m.p.insert("name".into(), "Router".into());
            m.p.insert("controller".into(), "UC100".into());
            doc.machines.push(m);
        });
        let id = a.doc.machines.last().unwrap().id.clone();
        assert!(a.steps_page() && has(&a, "Router ▶"));
        let _ = a.update(Message::ControlMachine(id));
        assert!(!a.steps_page());
        assert!(has(&a, "Not connected.") && has(&a, "Manual lathe steps"));
        let _ = a.update(Message::ShowGuide);
        assert!(a.steps_page());
    }
    /// Draws the window's view on tiny-skia into a PNG.
    pub(crate) fn render(a: &App, (w, h): (u32, u32), path: &std::path::Path) {
        let size = Size::new(w as f32, h as f32);
        let rgba = tablet::render(a.view(), size, 1., mouse::Cursor::Unavailable, true);
        let file = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
        let mut encoder = png::Encoder::new(file, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&rgba)
            .unwrap();
    }
    /// Puts the page at step `at`, its readout or hole `item`.
    fn at(a: &mut App, at: usize, item: usize) {
        let id = a.doc.setups[0].id.clone();
        a.doc.guide.insert(id, at);
        a.guide_item = item;
    }
    /// A Slot (LIO-445) on the 40 × 30 × 8 slotted plate in the vise, cut
    /// blind to `bottom` or through (LIO-449).
    pub(crate) fn slot_job(through: bool, bottom: &str) -> (Document, crate::mesh::Mesh) {
        let (mut doc, _) = crate::corner::tests::plate_job("Y");
        let mesh = crate::mesh::normalize(crate::cam::arcs::tests::slot(5.)).unwrap();
        let tool = &mut doc.tools[0];
        tool.p.insert("center_cutting".into(), "Yes".into());
        let mut op = crate::model::Operation::new(Strategy::Slot, tool.id.clone());
        for (k, v) in [
            ("name", "Slot 15x10"),
            ("slot_x", "0"),
            ("slot_y", "0"),
            ("slot_axis", "X"),
            ("slot_length", "15"),
            ("slot_width", "10"),
            ("z_top", "0"),
            ("z_bottom", bottom),
            ("through", if through { "Yes" } else { "No" }),
            ("breakthrough", if through { "0.5" } else { "0" }),
            ("axial_allowance", "0"),
            ("allowance", "0"),
        ] {
            op.p.insert(k.into(), v.into());
        }
        let s = &mut doc.setups[0];
        s.p.insert("vise_grip".into(), "4".into());
        s.operations = vec![op];
        (doc, mesh)
    }
    /// A through Slot over parallels (LIO-537): its Steps are given while
    /// the parallels stand clear of its passes, and refused, naming the
    /// parallel, when one stands under the slot.
    #[test]
    fn a_through_slot_is_checked_against_the_parallels_under_it() {
        use crate::parallels::{AT, HEIGHT, LEFT_TO_RIGHT, RUN, WIDTH};
        let (mut doc, mesh) = slot_job(true, "-8");
        let mut on = |at: &str, height: &str| {
            let p = &mut doc.setups[0].p;
            for (k, v) in [
                (RUN, LEFT_TO_RIGHT),
                (AT, at),
                (HEIGHT, height),
                (WIDTH, "4"),
            ] {
                p.insert(k.into(), v.into());
            }
            shown(&doc, &mesh, 0).map(|_| ())
        };
        // The slot is Y -5..5; parallels Y 10..14 and -14..-10 are 5 mm
        // clear of its walls, past the 2 mm gap.
        assert_eq!(on("-12; 12", "10"), Ok(()));
        let refused = on("0; 12", "10").unwrap_err();
        assert!(refused.contains("the parallel at Y0"), "{refused}");
        // Its 0.5 mm breakthrough goes 0.3 mm into the table under 0.2 mm
        // parallels.
        assert_eq!(
            on("-12; 12", "0.2"),
            Err("Slot 15x10: the tool goes 0.30 mm past the 0.2 mm parallels the part stands on, into the table or vise under them. Reduce the depth or the breakthrough.".into())
        );
    }
    /// The Check page's depth of a drilled hole on the manual mill
    /// (LIO-450): `mesh` drilled with a Ø`size` drill, through with 2.5 of
    /// breakthrough or blind, from a stock whose top is the part's.
    fn drilled(
        mesh: &Mesh,
        size: &str,
        through: bool,
        bottom: &str,
        air: &str,
    ) -> (Document, Guide, f32) {
        let doc = drilled_job(mesh, size, through, bottom, air);
        let shown = shown(&doc, mesh, 0).unwrap();
        let g = &shown.guide;
        let top = shown.boxes[0].max[2] + g.zero[2];
        (doc, g.clone(), top)
    }
    /// `drilled`'s job, its Steps not built.
    pub(crate) fn drilled_job(
        mesh: &Mesh,
        size: &str,
        through: bool,
        bottom: &str,
        air: &str,
    ) -> Document {
        use crate::cam::boring::tests::{drill, drilling, job};
        let manual = crate::manual::defaults();
        let mut tool = drill(size);
        tool.p.insert("machines".into(), manual.id.clone());
        let mut op = drilling(mesh, &tool, "Drilling");
        for (k, v) in [
            ("through", if through { "Yes" } else { "No" }),
            ("breakthrough", "2.5"),
            ("bottom", bottom),
            ("top", air),
            ("rpm", "640"),
            ("feed", "60"),
        ] {
            op.p.insert(k.into(), v.into());
        }
        let (mut doc, _) = job(mesh, manual, vec![(tool, op)]);
        doc.setups[0].p.insert("stock_top".into(), "0".into());
        doc
    }
    /// A hand Drill through a part on parallels (LIO-537): its Steps are
    /// given while the drill breaks through over the open space between
    /// them, and refused when a parallel stands under the hole or the
    /// breakthrough goes past their height into the table.
    #[test]
    fn a_through_drill_is_checked_against_the_parallels_and_the_table_under_them() {
        use crate::parallels::{AT, FRONT_TO_BACK, HEIGHT, RUN, WIDTH};
        // Ø6.8 through the 18.5 mm ring, 2.5 mm of breakthrough.
        let mesh = crate::cam::boring::tests::ring(6.8, 30., 18.5);
        let mut doc = drilled_job(&mesh, "6.8", true, "0", "0");
        // The part's underside is the stock's: the breakthrough leaves it.
        doc.setups[0].p.insert("stock_bottom".into(), "0".into());
        let mut on = |at: &str, height: &str| {
            let p = &mut doc.setups[0].p;
            for (k, v) in [
                (RUN, FRONT_TO_BACK),
                (AT, at),
                (HEIGHT, height),
                (WIDTH, "4"),
            ] {
                p.insert(k.into(), v.into());
            }
            shown(&doc, &mesh, 0).map(|_| ())
        };
        // The hole is at X0; parallels X 9..13 and -13..-9 are 5.6 mm clear
        // of the drill's Ø6.8, past the 2 mm gap.
        assert_eq!(on("-11; 11", "10"), Ok(()));
        let into = on("0; 11", "10").unwrap_err();
        assert!(
            into.contains("hits the parallel at X0")
                && into.ends_with("Stand the part so that the parallels are clear of its cuts."),
            "{into}"
        );
        // Parallels 2 mm high: the breakthrough goes 0.5 mm into the table.
        let past = on("-11; 11", "2").unwrap_err();
        assert!(
            past.ends_with(": the tool goes 0.50 mm past the 2 mm parallels the part stands on, into the table or vise under them. Reduce the depth or the breakthrough."),
            "{past}"
        );
        // Parallels chosen but with no height (or a bad one) stand nothing
        // up: no Steps, and what is wrong said.
        for bad in ["", "-2", "NaN"] {
            let unread = on("-11; 11", bad).unwrap_err();
            assert!(
                unread.ends_with("Give the parallels' height in mm: how far they raise the part."),
                "{bad:?}: {unread}"
            );
        }
        // Parallels off the stock hold nothing up either, whatever holds
        // the stock (here none).
        let off = on("-100; 100", "10").unwrap_err();
        assert!(
            off.contains("The parallel at X-100 is not under the stock, which spans X-"),
            "{off}"
        );
    }
    /// Steps entered from planning (LIO-449): at the start of the operation
    /// picked there, saying where the steps had got to, one press from it;
    /// with none picked, where they were left, saying so; the line goes
    /// once the place moves.
    #[test]
    fn steps_open_at_the_operation_picked_in_planning_or_say_they_resume() {
        let mut a = app("Yes");
        let g = guide(&a);
        let id = a.doc.setups[0].id.clone();
        let si = a.doc.index();
        let last = g.steps.len() - 1;
        // The Step, the second operation: a page of its own (LIO-688).
        let k = (g.sections.iter())
            .position(|s| s.op == Some(1))
            .unwrap();
        let (picked, start) = (g.sections[k].op.unwrap(), g.sections[k].steps.start);
        // Picked, the place at the hand-over: the operation's start.
        a.doc.guide.insert(id.clone(), last);
        a.page = Page::Work;
        a.selected = Some(picked);
        let _ = a.update(Message::Guide(GuideAction::Open(si)));
        assert!(a.steps_page());
        assert_eq!(place(&a), start);
        assert!(matches!(&a.steps_entry, Some(Entry::Picked { from, .. }) if *from == last));
        let t = texts(&a);
        assert!(
            t.iter().any(|s| s.starts_with(
                "Opened at the start of the operation picked in planning. The steps had got to"
            )),
            "{t:?}"
        );
        assert!(has(&a, "Back to there ▶"));
        send(&mut a, GuideAction::Resume);
        assert_eq!(place(&a), last);
        assert_eq!(a.steps_entry, None);
        // Entered by MACHINE's page at the picked operation's page: kept,
        // nothing to say.
        a.doc.guide.insert(id.clone(), start);
        a.page = Page::Work;
        let _ = a.update(Message::Page(Page::Control));
        assert_eq!((place(&a), a.steps_entry.clone()), (start, None));
        // Picked, but the steps are not there yet (a fresh job): kept at
        // the start, none skipped unasked; the picked one a press away.
        a.doc.guide.insert(id.clone(), 0);
        a.page = Page::Work;
        a.selected = Some(picked);
        let _ = a.update(Message::Page(Page::Control));
        assert_eq!(place(&a), 0);
        assert_eq!(
            a.steps_entry,
            Some(Entry::Resumed {
                setup: id.clone(),
                ahead: Some(k)
            })
        );
        let t = texts(&a);
        let name = section_name(&a.doc, &g, &g.sections[k]);
        assert!(
            t.iter().any(|s| *s
                == format!(
                    "These steps start at the beginning. {name}, picked in planning, comes later."
                )),
            "{t:?}"
        );
        assert!(has(&a, &format!("Skip to {name} ▶")), "{t:?}");
        send(&mut a, GuideAction::Go(start));
        assert_eq!((place(&a), a.steps_entry.clone()), (start, None));
        // Nothing picked, at the very start: nothing to say.
        a.doc.guide.insert(id.clone(), 0);
        a.page = Page::Work;
        a.selected = None;
        let _ = a.update(Message::Page(Page::Control));
        assert_eq!(a.steps_entry, None);
        // Nothing picked, part way: resumed, said; its button starts that
        // operation again, and the next step drops the line.
        a.doc.guide.insert(id.clone(), start);
        a.page = Page::Work;
        let _ = a.update(Message::Page(Page::Control));
        assert_eq!(place(&a), start);
        assert!(matches!(&a.steps_entry, Some(Entry::Resumed { .. })));
        let t = texts(&a);
        assert!(
            t.iter()
                .any(|s| s.starts_with("Resuming where these steps were left:")),
            "{t:?}"
        );
        let again = (t.iter()).find(|s| s.starts_with("◀ Start ") && s.ends_with(" again"));
        assert!(again.is_some(), "{t:?}");
        send(&mut a, GuideAction::Next);
        assert_eq!(a.steps_entry, None);
        assert!(!texts(&a).iter().any(|s| s.starts_with("Resuming")));
    }

    // ---- The lathe ----

    /// The lathe guide's sample job (`guide::tests::sample`: a sleeve in a
    /// Ø50.2 bar, faced, turned to Ø40, drilled, grooved, threaded and
    /// parted off) with its cross dial reading `reads` ("" = not known),
    /// `edit` made to it, on MACHINE with its guide built.
    /// LIO-437: a lathe Thread's Passes tab prints the very readings its
    /// pass screens give, a halfway one (0.91, pass 6) included.
    #[test]
    fn the_passes_tab_gives_the_thread_pass_screens_own_readings() {
        use crate::guide::LatheStep as L;
        let mut a = lathe_app("Diameter", |_| {});
        let g = guide(&a);
        let passes: Vec<(usize, usize)> = (g.steps.iter().enumerate())
            .filter_map(|(i, s)| match s {
                Step::Lathe(L::ThreadPass { op, .. }) => Some((i, *op)),
                _ => None,
            })
            .collect();
        assert!(!passes.is_empty());
        let mut screens = vec![];
        for (at, _) in &passes {
            send(&mut a, GuideAction::Go(*at));
            screens.push(texts(&a));
        }
        a.page = crate::Page::Work;
        let _ = a.update(Message::SelectOp(passes[0].1));
        let _ = a.update(Message::Tab("Passes".into()));
        let note = (texts(&a).into_iter())
            .find(|t| t.contains(" passes, the top slide at "))
            .unwrap();
        let readings: Vec<String> = (note.split(" in to: ").nth(1).unwrap())
            .trim_end_matches('.')
            .split(", ")
            .map(str::to_string)
            .collect();
        assert_eq!(readings.len(), passes.len(), "{note}");
        for (k, reading) in readings.iter().enumerate() {
            assert!(
                screens[k].iter().any(|t| t == reading),
                "pass {}: {reading} in {:?}",
                k + 1,
                screens[k]
            );
        }
    }
    pub(crate) fn lathe_app(reads: &str, edit: impl FnOnce(&mut Document)) -> App {
        let (doc, mesh) = crate::guide::tests::sample(reads);
        lathe_app_of(doc, mesh, edit)
    }
    /// A sleeve with a Ø21 bore: a Ø10 drill, then a bore to Ø20, on a
    /// lathe whose cross dial reads the diameter.
    pub(crate) fn bore_app(edit: impl FnOnce(&mut Document)) -> App {
        use crate::turn::tests::{add, job, tube};
        let (mut doc, mesh) = job(50.2, 100., tube(10.5, 17.5, 0., 61.), -62.);
        let m = &mut doc.setups[0].machine;
        m.p.insert("cross_reads".into(), "Diameter".into());
        for t in &mut doc.tools {
            if get(&t.p, "type") == "Boring bar" {
                t.p.insert("min_bore".into(), "8".into());
            }
        }
        add(&mut doc, "Drill", &[("depth", "30")]);
        add(
            &mut doc,
            "Bore",
            &[
                ("diameter", "20"),
                ("z_end", "-20"),
                ("depth", "2"),
                ("finish", "0.2"),
                ("allowance", "0"),
            ],
        );
        lathe_app_of(doc, mesh, edit)
    }
    fn lathe_app_of(mut doc: Document, mesh: Mesh, edit: impl FnOnce(&mut Document)) -> App {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        a.test_dir = Some(dir);
        doc.file = Some(crate::model::ModelFile {
            name: "sleeve.stl".into(),
            path: std::path::PathBuf::from("sleeve.stl"),
            format: "stl".into(),
            bytes: 1,
            thickness: None,
        });
        doc.active = doc.setups[0].id.clone();
        edit(&mut doc);
        a.doc = doc;
        a.imported_mesh = Some(mesh);
        let _ = a.update(Message::Page(Page::Control));
        build(&mut a);
        a
    }
    /// Every Steps screen of the lathe setup the planner puts in front of the
    /// shaft's CNC setup (`proposal::tests::turned`), at 1280 × 800, into
    /// `AXIS_RENDER_DIR`.
    #[test]
    #[ignore]
    fn render_the_planned_lathe_steps() {
        use crate::proposal::tests::{turned, with_lathe};
        let dir = std::path::PathBuf::from(std::env::var("AXIS_RENDER_DIR").unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let (doc, mesh, report) = with_lathe();
        let job = crate::proposal::build(&doc, &mesh, &report, &turned(&report)).unwrap();
        let mut a = lathe_app_of(job, mesh, |d| {
            let m = &mut d.setups[0].machine;
            m.p.insert("cross_reads".into(), "Diameter".into());
        });
        for i in 0..guide(&a).steps.len() {
            at(&mut a, i, 0);
            render(&a, (1280, 800), &dir.join(format!("{i:02}.png")));
        }
    }
    /// The manual mill's Zero and drilling pages of the shaft turned on the
    /// lathe and then drilled by hand
    /// (`guide::tests::turned_then_drilled_by_hand`), and the Zero of
    /// `guide::tests::short_part_in_soft_jaws`, at 1280 × 800, into
    /// `AXIS_RENDER_DIR`.
    #[test]
    #[ignore]
    fn render_the_hand_drilling_after_the_lathe() {
        let dir = std::path::PathBuf::from(std::env::var("AXIS_RENDER_DIR").unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let (job, mesh) = crate::guide::tests::turned_then_drilled_by_hand();
        let mut a = lathe_app_of(job, mesh, |d| d.active = d.setups[1].id.clone());
        let drill = (guide(&a).steps.iter())
            .position(|s| matches!(s, Step::Sheet { .. }))
            .unwrap();
        for (name, step) in [("zero", 0), ("drill", drill)] {
            let id = a.doc.setups[1].id.clone();
            a.doc.guide.insert(id, step);
            render(&a, (1280, 800), &dir.join(format!("mill-{name}.png")));
        }
        // Its zero on the top's centre.
        (a.doc.setups[1].p).insert("origin".into(), "Stock top center".into());
        build(&mut a);
        let id = a.doc.setups[1].id.clone();
        a.doc.guide.insert(id, 0);
        render(&a, (1280, 800), &dir.join("mill-zero-centre.png"));
        // The short part in soft jaws.
        let (doc, mesh, report, plan) = crate::guide::tests::short_part_in_soft_jaws();
        let job = crate::proposal::build(&doc, &mesh, &report, &plan).unwrap();
        let mut a = lathe_app_of(job, mesh, |d| d.active = d.setups[1].id.clone());
        let id = a.doc.setups[1].id.clone();
        a.doc.guide.insert(id, 0);
        render(&a, (1280, 800), &dir.join("short-zero.png"));
    }
    use crate::guide::LatheStep as L;
    /// The index of the `n`th step (from 0) that `pick` is true of.
    fn nth(a: &App, n: usize, pick: impl Fn(&L) -> bool) -> usize {
        (guide(a).steps.iter().enumerate())
            .filter(|(_, s)| matches!(s, Step::Lathe(l) if pick(l)))
            .nth(n)
            .unwrap_or_else(|| panic!("no step {n} in {:?}", guide(a).steps))
            .0
    }
    fn shows(a: &App, wants: &[&str]) {
        let t = texts(a);
        for want in wants {
            assert!(t.iter().any(|s| s == want), "{want:?} in {t:?}");
        }
    }
    fn measure_key(a: &App) -> String {
        let s = &a.doc.setups[0];
        crate::guide::measure_key(&s.id, &s.operations[1].id)
    }
    #[test]
    fn each_lathe_screen_shows_its_dials_and_short_labels() {
        let mut a = lathe_app("Diameter", |_| {});
        assert!(a.steps_page() && a.control.is_none());
        let shown = a.guide_at().unwrap().0;
        assert!(shown.side.is_some());
        // Hold, Zero, the Face.
        shows(
            &a,
            &[
                "Manual lathe",
                "Hold",
                "3-jaw chuck",
                "Ø50.20 × 100 bar",
                "stick out",
            ],
        );
        let Step::Lathe(L::Hold { stickout, grip, .. }) = guide(&a).steps[0] else {
            panic!()
        };
        shows(&a, &[&mm(stickout), "in the jaws", &mm(grip)]);
        send(&mut a, GuideAction::Next);
        shows(
            &a,
            &[
                "Zero",
                "touch the end",
                "zero the carriage dial",
                "zero the cross dial",
                "Ø",
            ],
        );
        let face = nth(&a, 0, |l| matches!(l, L::Face { .. }));
        send(&mut a, GuideAction::Go(face));
        shows(
            &a,
            &[
                "Face",
                "top slide",
                "in 1.00 from the touch",
                "top slide along the bed (90° from the cross slide)",
                "lock the carriage",
                "on the new end",
                "Section done ▶",
            ],
        );
        // The Outside: the touch on the bar, then a pass with two dials.
        send(&mut a, GuideAction::Go(face + 1));
        shows(&a, &["Touch", "touch Ø50.20", "zero the cross dial"]);
        let first = nth(&a, 0, |l| matches!(l, L::Pass { .. }));
        send(&mut a, GuideAction::Go(first));
        shows(
            &a,
            &["Pass 1 of 6", "set the carriage stop at the shoulder"],
        );
        let third = nth(&a, 2, |l| matches!(l, L::Pass { .. }));
        send(&mut a, GuideAction::Go(third));
        assert!(!has(&a, "set the carriage stop at the shoulder"));
        shows(
            &a,
            &[
                "Pass 3 of 6",
                "pass 3/6",
                "power feed 0.10",
                "cross slide · 1 turn +",
                "in to 6.00 (Ø44.20)",
                "carriage · 3 turns +",
                // One way everywhere: "3 turns + 4.0", was "3 t + 4.0" (LIO-426).
                "stop at 3 turns + 4.0",
                "6.00",
                "1 turn + 2.00 on the dial",
                // The carriage as the cross slide: its number, its turns under it.
                "3 turns + 4.0 on the dial",
                "◀ feed off at 3 turns + 3.0",
                "last 1 mm by hand",
                "Pass done ▶",
            ],
        );
        // The tiles are the passes' diameters, the measure before the last.
        let t = texts(&a);
        let tiles: Vec<&str> = [
            "48.20", "46.20", "44.20", "42.20", "40.20", "measure", "40.00",
        ]
        .into_iter()
        .collect();
        let at: Vec<usize> = (tiles.iter())
            .map(|w| {
                t.iter()
                    .rposition(|s| s == w)
                    .unwrap_or_else(|| panic!("{w} in {t:?}"))
            })
            .collect();
        assert!(at.windows(2).all(|w| w[0] < w[1]), "{at:?} {t:?}");
        // The drill on the quill, the groove, a thread pass, the part off.
        let drill = nth(&a, 0, |l| matches!(l, L::TailDrill { .. }));
        send(&mut a, GuideAction::Go(drill));
        shows(
            &a,
            &[
                "Drill",
                "centre drill first",
                "quill · 13 turns +",
                "30 deep + 3.00 for the point",
                "lock",
                "33.00",
                "13 turns + 0.50 on the dial",
                "peck every 10",
                "tailstock",
            ],
        );
        let groove = nth(&a, 0, |l| {
            matches!(
                l,
                L::Plunge {
                    part_off: false,
                    ..
                }
            )
        });
        send(&mut a, GuideAction::Go(groove));
        shows(
            &a,
            &[
                "Groove",
                "carriage · 2 turns +",
                "blade's left side at 2 turns + 5.0",
                "cross slide · 1 turn +",
                "in 4.00 → Ø36.00",
                "from the touch on Ø40.00",
                "lock the carriage",
            ],
        );
        let thread = nth(&a, 2, |l| matches!(l, L::ThreadPass { .. }));
        // The set-up step sits between the Touch and pass 1 (LIO-415).
        send(&mut a, GuideAction::Go(thread - 4));
        shows(
            &a,
            &[
                "Touch",
                "top slide",
                "29.5°",
                "from the cross slide",
                "29.5° from the cross slide",
                "zero it on the touch",
            ],
        );
        send(&mut a, GuideAction::Go(thread));
        shows(
            &a,
            &[
                "Thread pass 3 of 8",
                "29.5° from the cross slide",
                "gearbox 1.5",
                "pass 3: in to 0.64",
                "0.64",
                // The motions in order, the half-nuts named once (LIO-415).
                "spindle",
                "forward",
                "OUT",
                "half-nuts stay closed",
                "back to its zero",
                "8 passes",
            ],
        );
        assert!(!has(&a, "close"));
        let last = nth(&a, 7, |l| matches!(l, L::ThreadPass { .. }));
        send(&mut a, GuideAction::Go(last));
        shows(
            &a,
            &[
                "Thread pass 8 of 8",
                "pass 8: in to 1.04",
                "0.905 toward the axis",
            ],
        );
        // Then tried with the nut, its own step (LIO-413).
        send(&mut a, GuideAction::Go(last + 1));
        // The screen named Try, the nut by its name (LIO-437).
        shows(&a, &["Try", "M40×1.5 nut", "spring pass"]);
        let part = nth(&a, 0, |l| matches!(l, L::Plunge { part_off: true, .. }));
        send(&mut a, GuideAction::Go(part));
        shows(
            &a,
            &[
                "Part off",
                "catch the part",
                "centre",
                "in until the part drops",
            ],
        );
        // The carriage as a pass shows it: the distance, its turns under it
        // (LIO-437).
        assert!(
            texts(&a).iter().any(|t| t.ends_with(" on the dial")),
            "{:?}",
            texts(&a)
        );
        send(&mut a, GuideAction::Next);
        let minutes = format!("lathe ~{} min", guide(&a).minutes.ceil());
        shows(
            &a,
            &[
                "Hand over",
                "the part leaves the lathe",
                "the last setup",
                &minutes,
            ],
        );
        // No dial check while the dial is known.
        assert!(!has(&a, "Check the cross-slide dial"));
    }
    #[test]
    fn a_bore_pass_shows_what_the_collar_reads_wound_out() {
        let mut a = bore_app(|_| {});
        let fifth = nth(&a, 4, |l| matches!(l, L::Pass { .. }));
        send(&mut a, GuideAction::Go(fifth));
        shows(
            &a,
            &[
                "Pass 5 of 6",
                "2 turns out, reads 2.20",
                "cross slide · out to Ø19.80",
                "2 turns out",
                "reads 2.20",
            ],
        );
        // The large text is what the collar reads, said once (LIO-390).
        assert!(!has(&a, "2 t out, 2.20"));
        assert!(!has(&a, "out to 9.80 (Ø19.80)"));
        assert!(!has(&a, "set the carriage stop at the bottom"));
        // The first pass: the carriage stop before the power feed.
        let first = nth(&a, 0, |l| matches!(l, L::Pass { .. }));
        send(&mut a, GuideAction::Go(first));
        shows(&a, &["set the carriage stop at the bottom"]);
        let measure = nth(&a, 0, |l| matches!(l, L::Measure { .. }));
        send(&mut a, GuideAction::Go(measure));
        shows(
            &a,
            &["2 turns out, reads 2.00", "cross slide · out → Ø20.00"],
        );
    }
    #[test]
    fn with_only_inside_tools_the_zero_does_not_touch_the_bars_outside() {
        let mut a = bore_app(|_| {});
        let zero = nth(&a, 0, |l| matches!(l, L::Zero { .. }));
        send(&mut a, GuideAction::Go(zero));
        shows(
            &a,
            &[
                "Zero",
                "Boring bar",
                "touch the end",
                "zero the carriage dial",
            ],
        );
        assert!(!has(&a, "zero the cross dial"));
        // With a turning tool, it touches the Ø too.
        let mut a = lathe_app("Diameter", |_| {});
        send(&mut a, GuideAction::Next);
        shows(&a, &["zero the cross dial"]);
    }
    #[test]
    fn after_a_thread_a_chamfer_shows_the_top_slide_set_back() {
        let mut a = lathe_app("Diameter", |doc| {
            doc.setups[0].operations.truncate(5);
            crate::turn::tests::add(doc, "Chamfer", &[("edge", "End outside"), ("size", "0.5")]);
        });
        let chamfer = nth(&a, 0, |l| matches!(l, L::Chamfer { .. }));
        send(&mut a, GuideAction::Go(chamfer));
        shows(
            &a,
            &[
                "45° tool edge",
                "top slide from 29.5° back along the bed (90° from the cross slide)",
            ],
        );
    }
    #[test]
    fn a_part_off_and_a_face_to_the_centre_ask_for_the_tool_on_centre() {
        let mut a = lathe_app("Diameter", |_| {});
        // Once in the set up, on the Zero.
        assert!(!has(&a, "set every tool on centre"));
        send(&mut a, GuideAction::Next);
        shows(&a, &["Zero", "set every tool on centre"]);
        let face = nth(&a, 0, |l| matches!(l, L::Face { .. }));
        send(&mut a, GuideAction::Go(face));
        shows(&a, &["on centre height"]);
        // The outside does not cut to the centre.
        send(&mut a, GuideAction::Go(face + 1));
        assert!(!has(&a, "on centre height"));
        let part = nth(&a, 0, |l| matches!(l, L::Plunge { part_off: true, .. }));
        send(&mut a, GuideAction::Go(part - 1));
        shows(&a, &["Touch", "on centre height"]);
        send(&mut a, GuideAction::Go(part));
        shows(&a, &["Part off", "on centre height"]);
        // A face that stops short of the centre does not ask.
        let mut a = lathe_app("Diameter", |doc| {
            doc.setups[0].operations[0]
                .p
                .insert("to_diameter".into(), "20".into());
        });
        let face = nth(&a, 0, |l| matches!(l, L::Face { .. }));
        send(&mut a, GuideAction::Go(face));
        assert!(!has(&a, "on centre height"));
    }
    /// LIO-415, LIO-413: a single-point thread is set up once before its
    /// first pass; each pass is the motions in order, in view at 1024 x 700;
    /// the section names the thread; it ends tried with the nut. A die's
    /// thread ends tried with the nut too.
    #[test]
    fn a_thread_is_set_up_once_cut_in_motions_and_tried_with_the_nut() {
        let mut a = lathe_app("Diameter", |_| {});
        let first = nth(&a, 0, |l| matches!(l, L::ThreadPass { .. }));
        send(&mut a, GuideAction::Go(first - 1));
        shows(
            &a,
            &[
                "Set up the thread",
                "gearbox",
                "1.5",
                "half-nuts",
                "close",
                "29.5°",
            ],
        );
        assert!(
            texts(&a).iter().any(|t| t == "Thread M40×1.5"),
            "{:?}",
            texts(&a)
        );
        let window = iced::Size::new(1024., 700.);
        (a.window_width, a.window_height) = (window.width, window.height);
        send(&mut a, GuideAction::Go(first + 1));
        if let Ok(dir) = std::env::var("AXIS_RENDER_DIR") {
            let dir = std::path::Path::new(&dir);
            render(&a, (1024, 700), &dir.join("thread-pass-1024.png"));
            let mut w = lathe_app("Diameter", |_| {});
            (w.window_width, w.window_height) = (1440., 900.);
            for (name, at) in [("thread-setup", first - 1), ("thread-pass", first + 1)] {
                send(&mut w, GuideAction::Go(at));
                render(&w, (1440, 900), &dir.join(format!("{name}-1440.png")));
            }
            send(&mut w, GuideAction::Go(first + 8));
            render(&w, (1440, 900), &dir.join("thread-try-1440.png"));
        }
        let t = texts(&a);
        assert!(
            t.iter()
                .any(|s| s.ends_with(" on the dial") && s.contains(" turn")),
            "{t:?}"
        );
        assert!(!t.iter().any(|s| s.contains(" t + ")), "{t:?}");
        // Every numbered step in order, above the column's foot.
        let seen = crate::ui::probe::placed(a.view(), window);
        let at = |label: &str| {
            (seen.iter())
                .filter(|(t, _)| t == label)
                .map(|(_, r)| *r)
                .collect::<Vec<_>>()
        };
        let back = at("◀ Back")[0];
        // The numbered rows' keys (the dial's title above also says "top
        // slide"): in order down the column.
        let keys = ["cross slide", "spindle"];
        let mut last = 0.;
        for key in keys {
            let r = *at(key)
                .first()
                .unwrap_or_else(|| panic!("{key} in {seen:?}"));
            assert!(r.y + r.height + 14. <= back.y, "{key} {r:?} over {back:?}");
            assert!(r.y >= last, "{key} in order");
            last = r.y;
        }
        let out = at("OUT")[0];
        assert!(
            out.y + out.height + 14. <= back.y && out.y > last,
            "{out:?}"
        );
        // A die's thread is tried with the nut.
        let (doc, mesh) = crate::guide::tests::die_sample();
        let g = crate::guide::build(&doc, &mesh, 0).unwrap();
        assert!(
            g.steps.iter().any(|s| matches!(s, Step::Lathe(L::Try { what, spring: false, .. }) if what.ends_with(" nut"))),
            "{:?}",
            g.steps
        );
    }
    #[test]
    fn a_stop_on_the_pictures_is_the_number_on_the_carriage_dial() {
        // Faced 1 mm: the shoulder at Z −65 is 64 from the new end, where
        // the carriage dial was zeroed, and the dial stops at 3 turns + 4.0 = 64.
        let a = lathe_app("Diameter", |_| {});
        let g = guide(&a);
        let (carriage, thread) = (g.steps.iter())
            .filter_map(|s| match s {
                Step::Lathe(L::Pass { carriage, .. }) => Some((Some(carriage), None)),
                Step::Lathe(L::ThreadPass { carriage, .. }) => Some((None, Some(carriage))),
                _ => None,
            })
            .fold((None, None), |acc, x| (acc.0.or(x.0), acc.1.or(x.1)));
        let carriage = carriage.unwrap();
        assert_eq!(carriage.reading, Ok((3, 4.)));
        assert_eq!(
            turning::from_end("shoulder", carriage),
            "shoulder 64 from the end"
        );
        // The thread to Z −40 stops 39 from the new end: 1 turn + 19.0.
        let thread = thread.unwrap();
        assert_eq!(
            turning::from_end("thread stops", thread),
            "thread stops 39 from the end"
        );
    }
    #[test]
    fn until_the_dial_check_is_answered_the_cross_slide_shows_no_number() {
        let mut a = lathe_app("", |_| {});
        shows(
            &a,
            &[
                "Check the cross-slide dial",
                "TURN A LIGHT SKIN, THEN:",
                "10 div",
                "Ø ?",
                "Ø went down by 0.40",
                "Ø went down by 0.80",
                "by 1× → diameter",
                "by 2× → radius",
            ],
        );
        let third = nth(&a, 2, |l| matches!(l, L::Pass { .. }));
        send(&mut a, GuideAction::Go(third));
        let t = texts(&a);
        assert!(t.iter().any(|s| s == lathe::CHECK_THE_DIAL), "{t:?}");
        assert!(
            !t.iter()
                .any(|s| s.contains("6.00") || s.contains("1 turn +")),
            "{t:?}"
        );
        // The carriage is known all the same.
        shows(&a, &["stop at 3 turns + 4.0"]);
    }
    use crate::lathe;
    #[test]
    fn the_dial_check_answer_is_saved_to_the_lathe_and_its_setups_and_the_numbers_show() {
        for (reads, key, reading) in [
            (
                lathe::Reads::Diameter,
                "Diameter",
                "1 turn + 2.00 on the dial",
            ),
            (lathe::Reads::Radius, "Radius", "3.00 on the dial"),
        ] {
            let mut a = lathe_app("", |doc| {
                doc.add_lathe_setup(None).unwrap();
                assert!(doc.setups.len() == 2);
            });
            let id = a.doc.setups[0].machine.id.clone();
            assert_eq!(a.doc.setups[1].machine.id, id);
            let edits = a.project_dirty;
            send(&mut a, GuideAction::CrossReads(reads));
            let library = a.doc.machines.iter().find(|m| m.id == id).unwrap();
            assert_eq!(get(&library.p, "cross_reads"), key);
            for s in &a.doc.setups {
                assert_eq!(get(&s.machine.p, "cross_reads"), key);
            }
            assert!(a.project_dirty && a.dirty, "{edits}");
            // The guide is worked out again at once: no dial check, and the
            // place is on the Hold.
            let (shown, at) = a.guide_at().expect("the guide, built again");
            assert!(
                !shown
                    .guide
                    .steps
                    .iter()
                    .any(|s| matches!(s, Step::Lathe(L::DialCheck { .. })))
            );
            assert_eq!(at, 0);
            assert!(has(&a, "Hold"));
            let third = nth(&a, 2, |l| matches!(l, L::Pass { .. }));
            send(&mut a, GuideAction::Go(third));
            shows(&a, &[reading]);
            assert!(!has(&a, lathe::CHECK_THE_DIAL));
        }
    }
    #[test]
    fn the_measured_diameter_sets_the_finish_pass_reading() {
        let mut a = lathe_app("Diameter", |_| {});
        let measure = nth(&a, 0, |l| matches!(l, L::Measure { .. }));
        send(&mut a, GuideAction::Go(measure));
        shows(
            &a,
            &[
                "Measure",
                "MEASURE THE Ø",
                "Ø40.20",
                "THEN THE LAST PASS",
                "in to 10.20 → Ø40.00",
                "cross slide · 2 turns +",
            ],
        );
        let key = measure_key(&a);
        let (edits, at) = (a.project_dirty, place(&a));
        send(&mut a, GuideAction::Typed(key.clone(), "40.32".into()));
        assert_eq!(a.guide_typed, Some((key.clone(), "40.32".into())));
        assert!(has(&a, "Keep ▶"));
        // Not a number: nothing is kept.
        send(&mut a, GuideAction::Measured(key.clone(), f32::NAN));
        assert!(a.doc.measured.is_empty());
        send(&mut a, GuideAction::Measured(key.clone(), -1.));
        assert!(a.doc.measured.is_empty());
        send(&mut a, GuideAction::Measured(key.clone(), 40.32));
        assert_eq!(a.doc.measured.get(&key), Some(&40.32));
        // A measurement, as the place: saved, no edit of the job.
        assert!(a.dirty && a.undo.is_none());
        assert_eq!(a.project_dirty, edits);
        assert!(a.guide_typed.is_none());
        assert_eq!(place(&a), at);
        shows(
            &a,
            &["in to 10.32 → Ø40.00", "Measured Ø40.32 kept.", "✓ kept"],
        );
        send(&mut a, GuideAction::Next);
        shows(
            &a,
            &[
                "Pass 6 of 6",
                "in to 10.32 (Ø40.00)",
                "2 turns + 2.32 on the dial",
                "from the Ø40.32 measured",
            ],
        );
        // The passes before it do not change.
        let fifth = nth(&a, 4, |l| matches!(l, L::Pass { .. }));
        send(&mut a, GuideAction::Go(fifth));
        shows(&a, &["in to 10.00 (Ø40.20)"]);
        // The key of the guide changed with it: a guide built for the job
        // as it was is not shown.
        let built = a.guide.clone();
        a.doc.measured.insert(key.clone(), 40.4);
        assert!(a.current_guide().is_none());
        a.doc.measured.insert(key, 40.32);
        a.guide = built;
        assert!(a.current_guide().is_some());
    }
    /// Every lathe screen at 1280 × 800, one PNG per kind, in
    /// /tmp/lathe-steps.
    #[test]
    #[ignore]
    fn render_the_lathe_steps() {
        let dir = std::path::Path::new("/tmp/lathe-steps");
        std::fs::create_dir_all(dir).unwrap();
        let shot = |a: &mut App, name: &str, step: usize, item: usize| {
            at(a, step, item);
            render(a, (1280, 800), &dir.join(format!("{name}.png")));
        };
        let mut a = lathe_app("", |_| {});
        shot(&mut a, "01-dial-check", 0, 1);
        let third = nth(&a, 2, |l| matches!(l, L::Pass { .. }));
        shot(&mut a, "02-pass-check-the-dial-first", third, 1);
        let mut a = lathe_app("Diameter", |_| {});
        shot(&mut a, "03-hold", 0, 0);
        shot(&mut a, "04-zero", 1, 1);
        let face = nth(&a, 0, |l| matches!(l, L::Face { .. }));
        shot(&mut a, "05-face", face, 0);
        shot(&mut a, "06-touch", face + 1, 0);
        let first = nth(&a, 0, |l| matches!(l, L::Pass { .. }));
        shot(&mut a, "07-pass-1", first, 0);
        let third = nth(&a, 2, |l| matches!(l, L::Pass { .. }));
        shot(&mut a, "08-pass-3", third, 1);
        let measure = nth(&a, 0, |l| matches!(l, L::Measure { .. }));
        shot(&mut a, "09-measure", measure, 0);
        let key = measure_key(&a);
        send(&mut a, GuideAction::Measured(key, 40.32));
        shot(&mut a, "10-measured", measure, 0);
        shot(&mut a, "11-finish-pass-measured", measure + 1, 1);
        let drill = nth(&a, 0, |l| matches!(l, L::TailDrill { .. }));
        shot(&mut a, "12-tail-drill", drill, 1);
        let groove = nth(&a, 0, |l| {
            matches!(
                l,
                L::Plunge {
                    part_off: false,
                    ..
                }
            )
        });
        shot(&mut a, "13-groove", groove, 0);
        let thread = nth(&a, 0, |l| matches!(l, L::ThreadPass { .. }));
        shot(&mut a, "14-thread-touch", thread - 1, 0);
        shot(&mut a, "15-thread-pass-3", thread + 2, 1);
        shot(&mut a, "15a-thread-pass-8", thread + 7, 1);
        let part = nth(&a, 0, |l| matches!(l, L::Plunge { part_off: true, .. }));
        shot(&mut a, "16a-part-off-touch", part - 1, 0);
        shot(&mut a, "16-part-off", part, 0);
        shot(&mut a, "17-hand-over", part + 1, 0);
        // Chamfers: a shoulder one on the carriage, an end one.
        let mut a = lathe_app("Diameter", |doc| {
            doc.setups[0].operations.truncate(2);
            crate::turn::tests::add(doc, "Chamfer", &[("edge", "Shoulder"), ("size", "1")]);
            crate::turn::tests::add(doc, "Chamfer", &[("edge", "End outside"), ("size", "1")]);
        });
        let chamfer = nth(&a, 0, |l| matches!(l, L::Chamfer { .. }));
        shot(&mut a, "18-chamfer-shoulder", chamfer, 0);
        shot(&mut a, "19-chamfer-end", chamfer + 1, 0);
        // After the thread, a chamfer and a face set the top slide back.
        let mut a = lathe_app("Diameter", |doc| {
            doc.setups[0].operations.truncate(5);
            crate::turn::tests::add(doc, "Chamfer", &[("edge", "End outside"), ("size", "0.5")]);
        });
        let chamfer = nth(&a, 0, |l| matches!(l, L::Chamfer { .. }));
        shot(&mut a, "23-chamfer-after-thread", chamfer, 0);
        let mut a = lathe_app("Diameter", |doc| {
            doc.setups[0].operations.truncate(5);
            crate::turn::tests::add(doc, "Face", &[("face_off", "0.5"), ("to_diameter", "36")]);
        });
        let face = nth(&a, 1, |l| matches!(l, L::Face { .. }));
        shot(&mut a, "24-face-after-thread", face, 0);
        // A bore: the cross slide winds out, its collar counting down.
        let mut a = bore_app(|_| {});
        let fifth = nth(&a, 4, |l| matches!(l, L::Pass { .. }));
        shot(&mut a, "20-bore-pass", fifth, 1);
        shot(&mut a, "20a-bore-pass-1", fifth - 4, 0);
        let measure = nth(&a, 0, |l| matches!(l, L::Measure { .. }));
        shot(&mut a, "22-bore-measure", measure, 0);
        // Only inside tools: the Zero touches the end only (LIO-203).
        let zero = nth(&a, 0, |l| matches!(l, L::Zero { .. }));
        shot(&mut a, "21-zero-bore-first", zero, 1);
        // An inside thread: the top slide wound out, away from the axis.
        let mut a = bore_app(|doc| {
            doc.setups[0].operations[1]
                .p
                .insert("diameter".into(), "20.4".into());
            crate::turn::tests::add(
                doc,
                "Thread",
                &[
                    ("thread_side", "Inside"),
                    ("diameter", "22"),
                    ("pitch", "1.5"),
                    ("z_end", "-15"),
                ],
            );
        });
        let thread = nth(&a, 0, |l| matches!(l, L::ThreadPass { .. }));
        shot(&mut a, "25-inside-thread-pass-2", thread + 1, 1);
    }
    mod manual_tests {
        use crate::manual::SAFE_ABOVE;
        use crate::model::num;

        #[test]
        fn a_drill_goes_from_hole_to_hole_over_the_clamps() {
            // The manual job's Holes (`guide::tests::job`), held by 25 high toe
            // clamps on its front and back (LIO-538): between holes the tip
            // goes up 5 over their tops, not at the 15 over the stock its
            // hidden clearance says, and nothing hits them.
            let holes_plan = |clamps: Option<&str>| {
                let (mut doc, mesh) = crate::guide::tests::job("No");
                if let Some(clamps) = clamps {
                    crate::guide::tests::on_clamps(&mut doc, clamps, "20");
                }
                let mut plans = vec![];
                crate::simulation::sequence(&doc, &mesh, None, |_, oi, r| {
                    plans.push((oi, r.unwrap().plan))
                });
                let plan = plans.into_iter().find(|(oi, _)| *oi == 2).unwrap().1;
                (doc, mesh, plan)
            };
            // Its tip over every move across.
            let across = |plan: &crate::simulation::Plan| {
                (plan.moves.iter())
                    .filter(|m| m.from[0] != m.to[0] || m.from[1] != m.to[1])
                    .map(|m| m.from[2].min(m.to[2]))
                    .fold(f32::INFINITY, f32::min)
            };
            let (doc, _, plan) = holes_plan(None);
            assert_eq!(num(&doc.setups[0].operations[2].p, "clearance"), Some(15.));
            assert!((across(&plan) - plan.stock.max[2] - 15.).abs() < 1e-3);
            let (doc, mesh, plan) = holes_plan(Some("-Y:0;+Y:0"));
            let boxes = doc
                .holding_boxes(&doc.setups[0], crate::holding::Gripped::whole(plan.stock))
                .unwrap();
            let tops = boxes
                .iter()
                .map(|b| b.bounds.max[2])
                .fold(f32::MIN, f32::max);
            assert!((tops - plan.stock.max[2] - 25.).abs() < 1e-3);
            assert!(
                (across(&plan) - plan.stock.max[2] - 25. - SAFE_ABOVE).abs() < 1e-3,
                "{}",
                across(&plan)
            );
            let gap = crate::holding::gap(&doc.setups[0]).unwrap();
            assert!(crate::holding::hits(&plan, &boxes, gap).is_empty());
            // The operation as saved keeps its own clearance.
            assert_eq!(num(&doc.setups[0].operations[2].p, "clearance"), Some(15.));
            // A clearance of its own above them is kept.
            let mut doc = doc;
            doc.setups[0].operations[2]
                .p
                .insert("clearance".into(), "40".into());
            let mut plans = vec![];
            crate::simulation::sequence(&doc, &mesh, None, |_, oi, r| {
                plans.push((oi, r.unwrap().plan))
            });
            let plan = &plans.iter().find(|(oi, _)| *oi == 2).unwrap().1;
            assert!((across(plan) - plan.stock.max[2] - 40.).abs() < 1e-3);
        }
    }

    mod slot_tests {
        use crate::cam::Geometry;
        use crate::model::{Strategy, num};
        use crate::simulation;
        use crate::slot::tests::adapter_counterbore;
        use crate::slot::*;
        use crate::surface::Selection;

        /// A Slot cut with a narrower cutter has one page (LIO-688): the
        /// slot as cut, its ends' centres 21.5 − 16.5 apart, its width and
        /// its depth.
        #[test]
        fn a_narrower_cutters_slot_page_gives_the_slot_as_cut() {
            let (doc, mesh) = adapter_counterbore(false);
            let shown = crate::steps::shown(&doc, &mesh, 0).unwrap();
            let crate::guide::Features::Slot {
                a,
                b,
                width,
                top,
                floor,
                through,
            } = crate::guide::tests::features(&shown.guide, 0)
            else {
                panic!("{:?}", shown.guide.steps)
            };
            assert!(!through);
            assert!((width - 16.5).abs() < 1e-3, "{width}");
            assert!(((b[0] - a[0]).hypot(b[1] - a[1]) - 5.).abs() < 1e-3, "{a:?} {b:?}");
            assert!((top - floor - 8.9).abs() < 1e-3, "{top} {floor}");
        }

        /// The user's own adapter project, a private copy in `AXIS_ADAPTER_COPY`
        /// (adapter-plate.axis.json and the STEP), never the original: every
        /// operation of every setup plans and simulates, the Slots among them
        /// (LIO-455: setup 4's two upper counterbores, far from its origin, were
        /// refused), and every setup's Steps build. Each Slot plans and simulates
        /// in seconds, and each is picked again by its round end in seconds
        /// (LIO-481: setup 4's one counterbore took ten minutes, a pick as long).
        /// Run it on a release build.
        #[test]
        #[ignore = "requires a private copy of the user's adapter project (AXIS_ADAPTER_COPY)"]
        fn the_users_adapter_project_plans_every_slot() {
            let dir = std::path::PathBuf::from(
                std::env::var_os("AXIS_ADAPTER_COPY")
                    .expect("set AXIS_ADAPTER_COPY to the private copy"),
            );
            let mut doc = crate::storage::load(&dir.join("adapter-plate.axis.json")).unwrap();
            doc.file.as_mut().unwrap().path = dir.join("adapter-plate.step");
            let mesh = crate::mesh::load_file(doc.file.as_ref().unwrap()).unwrap();
            let mut failures = vec![];
            let mut slots = vec![];
            let mut since = std::time::Instant::now();
            simulation::sequence_with(
                &doc,
                &mesh,
                None,
                |si, oi, r| {
                    let op = &doc.setups[si].operations[oi];
                    match r {
                        Ok(_) if op.strategy == Strategy::Slot => {
                            slots.push((op.name(), since.elapsed().as_secs_f32()));
                        }
                        Ok(_) => {}
                        Err(e) => failures.push(format!("{}/{} {}: {e}", si + 1, oi, op.name())),
                    }
                    since = std::time::Instant::now();
                },
                |_, _, _| {},
            );
            assert!(failures.is_empty(), "{failures:#?}");
            assert_eq!(slots.len(), 8);
            eprintln!("{slots:#?}");
            assert!(slots.iter().all(|(_, s)| *s < 10.), "{slots:#?}");
            // Each Slot of setup 4 picked again by its far round end: the fields it
            // reads back are the operation's, each pick in seconds (the whole
            // part's arcs, along its threads' relieved roots, took minutes).
            let si = 3;
            let first = doc.setups[si]
                .operations
                .iter()
                .find(|o| o.strategy == Strategy::Slot)
                .unwrap();
            let here = pick_setup(&doc, &mesh, si, &first.id).unwrap();
            let stock = crate::stock::resolve(&doc.setups[si], Some(&mesh))
                .unwrap()
                .stock;
            let zero = crate::post::datum(&here, stock, &mesh).unwrap();
            let g = Geometry::new(&here, &mesh);
            let mut picks = vec![];
            for o in doc.setups[si]
                .operations
                .iter()
                .filter(|o| o.strategy == Strategy::Slot)
            {
                let slot = Slot::read(o, zero).unwrap();
                let axis = slot.axis();
                let half = slot.width / 2.;
                let tip = (g.triangles.iter())
                    .find(|t| {
                        t.p.iter().all(|p| {
                            let d = [p[0] - slot.b[0], p[1] - slot.b[1]];
                            (d[0].hypot(d[1]) - half).abs() < 0.05
                                && d[axis] > 0.9 * half
                                && p[2] > slot.bottom - 0.01
                                && p[2] < slot.top + 0.01
                        })
                    })
                    .and_then(|t| t.feature)
                    .unwrap_or_else(|| panic!("{}: no wall at its far end", o.name()));
                let pick = Selection {
                    mesh_fingerprint: mesh.fingerprint.clone(),
                    ids: vec![tip],
                };
                let took = std::time::Instant::now();
                let fields = from_face(&here, &mesh, &pick, stock)
                    .unwrap_or_else(|e| panic!("{}: {e}", o.name()));
                let took = took.elapsed().as_secs_f32();
                for k in ["slot_x", "slot_y", "slot_length", "slot_width"] {
                    let (got, want) = (num(&fields, k).unwrap(), num(&o.p, k).unwrap());
                    assert!(
                        (got - want).abs() < 0.01,
                        "{}: {k} {got} for {want}",
                        o.name()
                    );
                }
                picks.push((o.name(), took));
            }
            eprintln!("{picks:#?}");
            assert!(picks.iter().all(|(_, s)| *s < 10.), "{picks:#?}");
            for si in 0..doc.setups.len() {
                if let Err(e) = crate::steps::shown(&doc, &mesh, si) {
                    panic!("setup {}: {e}", si + 1);
                }
            }
        }
    }
}
