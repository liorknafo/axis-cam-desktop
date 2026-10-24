//! The Steps page: a setup on the manual mill shown screen by screen
//! (`guide`), as pictures and the large numbers its DRO should read; a
//! setup on the manual lathe as side views and dial pictures (`turning`).
mod turning;
use crate::{
    App, GuideAction, Message,
    guide::{Guide, Limits, Section, Step, Target},
    help::HelpButton,
    holding::Gripped,
    manual,
    mesh::{Bounds, Mesh},
    model::{Document, Setup, Strategy, Tool, get, num},
    ui::{BLUE, BUTTON_EDGE, HOVER_BUTTON, INK, LINE, MUTED, WARNING, bold, muted, primary},
    viewport,
};
use iced::{
    Alignment, Border, Color, Element, Font,
    Length::{self, Fill, FillPortion},
    Point, Rectangle, Size, Theme, mouse,
    widget::{Column, button, canvas, column, container, image, row, scrollable, space, text},
};
use std::cell::Cell;

const ACCENT: Color = Color::from_rgb(0.91, 0.35, 0.05);
const TOOL: Color = Color::from_rgb(0.18, 0.50, 0.93);
const NEVER: Color = Color::from_rgb(0.81, 0.13, 0.18);
const CHIP: Color = Color::from_rgb(0.90, 0.91, 0.93);
const STOCK: Color = Color::from_rgb(0.90, 0.92, 0.94);
const PART: Color = Color::from_rgb(0.69, 0.73, 0.77);
const JAW: Color = Color::from_rgb(0.80, 0.82, 0.85);
const DONE: Color = Color::from_rgb(0.50, 0.55, 0.60);
/// The pixels of the 3D pictures, twice the size they are shown at.
const PICTURE: (u32, u32) = (960, 640);
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
    boxes: Vec<Bounds>,
    /// The vise jaws or clamps, in work coordinates, from the stock's
    /// bottom up (`holding::shown`).
    jaws: Vec<Bounds>,
    /// For the Hold step, in a vise: its jaws at their full height and the
    /// parallels under the stock, in work coordinates.
    vise: Vec<Bounds>,
    parallels: Vec<Bounds>,
    /// The stock in 3D: before the setup, then after each operation.
    pictures: Vec<Option<image::Handle>>,
    /// The stock in 3D as it starts, in the vise on its parallels.
    held: Option<image::Handle>,
    /// On the lathe: what its side views draw.
    side: Option<turning::Side>,
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
    use crate::simulation::leftover_box;
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
    let mut boxes = vec![work(leftover_box(&guide.before, setup, mesh))];
    boxes.extend(
        guide
            .done
            .iter()
            .map(|d| work(leftover_box(&d.after, setup, mesh))),
    );
    let jaws: Vec<Bounds> = match &guide.steps[0] {
        Step::Hold { holding, .. } if crate::holding::modelled(setup) => {
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
                .map(|b| b.bounds)
                .collect()
        }
        _ => vec![],
    };
    let jaws: Vec<Bounds> = jaws.into_iter().map(work).collect();
    // In a vise: the jaw plates go down from the grip by their height, and
    // parallels fill what is under the stock between them.
    let (vise, parallels) = match &guide.steps[0] {
        Step::Hold {
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
            (tall, if under { bars.to_vec() } else { vec![] })
        }
        _ => (vec![], vec![]),
    };
    let frame = crate::volume::Frame::new(setup, mesh);
    let solid = |boxes: &[Bounds]| -> Vec<_> {
        (boxes.iter())
            .flat_map(|b| viewport::box_triangles(b.translated(zero), viewport::JAWS))
            .collect()
    };
    let held = solid(&jaws);
    let picture = |v: &crate::volume::Volume, box_: Bounds, held: &[_]| {
        let surface =
            crate::stock_surface::Snapshot::of(v, crate::stock_surface::Detail::Draft).build();
        let mut triangles: Vec<_> = match &surface {
            Ok(s) => viewport::stock_triangles(v, s, frame).collect(),
            Err(_) => viewport::box_triangles(box_.translated(zero), viewport::BLOCK),
        };
        triangles.extend(held.iter().copied());
        viewport::stock_picture(&triangles, PICTURE)
    };
    let mut pictures = vec![picture(&guide.before, boxes[0], &held)];
    for (d, b) in guide.done.iter().zip(&boxes[1..]) {
        pictures.push(picture(&d.after, *b, &held));
    }
    let held = if vise.is_empty() {
        pictures[0].clone()
    } else {
        let tall = solid(&[&vise[..], &parallels].concat());
        picture(&guide.before, boxes[0], &tall)
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
        side,
    })
}

/// A DRO number: two decimals, a true minus, never −0.00.
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
    let mut line = row![].spacing(10).align_y(Alignment::Center);
    if let Some(n) = r.order {
        let mark = if r.lit == Lit::Done {
            "✓".to_string()
        } else {
            n.to_string()
        };
        line = line.push(disc(mark, now, 22.));
    }
    if let Some(lead) = r.lead {
        line = line.push(text(lead).size(15).color(MUTED));
    }
    line = line
        .push(text(r.axis).size(20).font(mono).color(ink))
        .push(
            text(r.value)
                .size(if r.big { 40 } else { 28 })
                .font(mono)
                .color(ink)
                .wrapping(text::Wrapping::None),
        )
        .push(space().width(Fill));
    let mut tags = column![].spacing(2).align_x(Alignment::End);
    if let Some(axes) = r.lock {
        tags = tags.push(
            row![
                canvas(Lock).width(10).height(12),
                text(format!("lock {axes}")).size(12).color(MUTED)
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        );
    }
    if !r.tags.is_empty() {
        tags = tags.push(text(r.tags.join(" · ")).size(12).color(MUTED));
    }
    line = line.push(tags);
    let face = boxed(line.padding([4, 12]), now).width(Fill);
    match msg {
        Some(m) => tap(face, Some(m), "Reached"),
        None => face.into(),
    }
}
/// The tiles of a section's passes or holes: done, now, next.
fn tiles<'a>(labels: Vec<(String, Option<Message>)>, at: usize) -> Element<'a, Message> {
    let mut line = row![].spacing(6);
    for (i, (s, msg)) in labels.into_iter().enumerate() {
        let state = lit(i, at);
        let face = container(
            text(s)
                .size(13)
                .font(Font::MONOSPACE)
                .color(if state == Lit::Now { ACCENT } else { INK }),
        )
        .center_x(Fill)
        .padding([6, 0])
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
    }
    line.into()
}
/// Back and the step's own command under a screen's right column.
fn buttons<'a>(next: Option<(String, Message)>) -> Element<'a, Message> {
    let back = HelpButton::new(
        button(text("◀ Back").size(13).center())
            .width(Fill)
            .padding([8, 12])
            .on_press(Message::Guide(GuideAction::Back))
            .style(|_, status| button::Style {
                background: Some(
                    if matches!(status, button::Status::Hovered) {
                        HOVER_BUTTON
                    } else {
                        Color::WHITE
                    }
                    .into(),
                ),
                text_color: INK,
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
        line = line.push(container(primary(label, msg).width(Fill)).width(FillPortion(2)));
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
    let head = 9_f32.min(l);
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
    f.fill_text(canvas::Text {
        content: s.into(),
        position: at,
        color: c,
        size: size.into(),
        align_x: align,
        align_y: iced::alignment::Vertical::Center,
        ..Default::default()
    });
}

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
    /// A line not to pass, labelled with its DRO number.
    Never([f32; 3], [f32; 3], String),
    /// A finished wall.
    Wall([f32; 3], [f32; 3]),
    /// The tool's centre at the start of the cut, its radius and the end.
    Tool([f32; 3], f32, Option<[f32; 3]>),
    /// An edge finder touching at a point, from a direction, labelled.
    Finder([f32; 3], [f32; 2], String),
    /// A hole: its centre, radius, number and whether done, now or next.
    Hole([f32; 3], f32, usize, Lit),
    /// A word at a point.
    Word([f32; 3], String, Color),
    /// A solid block, such as a parallel under the stock.
    Block(Bounds),
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
                Shape::Block(bx) => {
                    grow([bx.min[a], bx.min[b]]);
                    grow([bx.max[a], bx.max[b]]);
                }
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
                Shape::Never(from, to, s) => {
                    let (m, n) = (p3(*from), p3(*to));
                    f.stroke(&canvas::Path::line(m, n), dashed(NEVER, 1.8));
                    // Inside the stock: above a level's right end, beside a
                    // side's top.
                    if (m.x - n.x).abs() > (m.y - n.y).abs() && (m.x - n.x).abs() < 80. {
                        // A short line (a hole's floor): its label right of it.
                        let at = Point::new(m.x.max(n.x) + 5., m.y);
                        write(f, s.clone(), at, NEVER, 12., Align::Left);
                    } else if (m.x - n.x).abs() > (m.y - n.y).abs() {
                        let at = Point::new(m.x.max(n.x) - 8., m.y - 9.);
                        write(f, s.clone(), at, NEVER, 12., Align::Right);
                    } else {
                        let at = Point::new(m.x + 5., m.y.min(n.y) + 12.);
                        write(f, s.clone(), at, NEVER, 12., Align::Left);
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
                        write(
                            f,
                            s.clone(),
                            Point::new(c.x + from[0] * 4., c.y - 16.),
                            INK,
                            13.,
                            if from[0] < 0. {
                                Align::Right
                            } else {
                                Align::Left
                            },
                        );
                    } else {
                        write(
                            f,
                            s.clone(),
                            Point::new(c.x + 14., c.y),
                            INK,
                            13.,
                            Align::Left,
                        );
                    }
                }
                Shape::Hole(q, r, n, state) => {
                    let at = p3(*q);
                    let body = canvas::Path::circle(at, (r * s).max(4.));
                    let (fill, edge, w) = match state {
                        Lit::Done => (DONE, INK, 1.),
                        Lit::Now => (Color::WHITE, ACCENT, 2.5),
                        Lit::Next => (Color::WHITE, INK, 1.),
                    };
                    if *state == Lit::Now {
                        let zero = p([0., 0.]);
                        f.stroke(
                            &canvas::Path::line(Point::new(zero.x, at.y), at),
                            dashed(ACCENT, 1.2),
                        );
                        f.stroke(
                            &canvas::Path::line(Point::new(at.x, zero.y), at),
                            dashed(ACCENT, 1.2),
                        );
                    }
                    f.fill(&body, fill);
                    f.stroke(&body, stroke(edge, w));
                    let c = if *state == Lit::Now { ACCENT } else { MUTED };
                    write(
                        f,
                        n.to_string(),
                        Point::new(at.x + r * s + 5., at.y - 9.),
                        c,
                        12.,
                        Align::Left,
                    );
                }
                Shape::Word(q, s, c) => write(f, s.clone(), p3(*q), *c, 13., Align::Left),
                Shape::Block(bx) => {
                    let r = rect(bx);
                    f.fill(&path(r), DONE);
                    f.stroke(&path(r), stroke(INK, 1.));
                }
                Shape::Down(q, mm, s) => {
                    let mut t = *q;
                    t[b] += mm;
                    let (head, tail) = (p3(*q), p3(t));
                    arrow(f, tail, head, ACCENT, 2.5);
                    write(
                        f,
                        s.clone(),
                        Point::new(tail.x + 8., tail.y + 6.),
                        ACCENT,
                        13.,
                        Align::Left,
                    );
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
                    let mid = Point::new(x + 9., (m.y + n.y) / 2.);
                    write(f, s.clone(), mid, ACCENT, 13., Align::Left);
                }
            }
        }
        // The work zero and the axes.
        let zero = p3([0.; 3]);
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
        let corner = Point::new(10., size.height - 10.);
        arrow(f, corner, Point::new(36., corner.y), MUTED, 1.3);
        arrow(f, corner, Point::new(corner.x, corner.y - 26.), MUTED, 1.3);
        write(
            f,
            AXES[a],
            Point::new(40., corner.y),
            MUTED,
            12.,
            Align::Left,
        );
        write(
            f,
            AXES[b],
            Point::new(corner.x - 4., corner.y - 34.),
            MUTED,
            12.,
            Align::Left,
        );
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
/// A tool in its holder, its diameter and the gear it runs at.
struct ToolPicture {
    diameter: f32,
    rpm: f32,
    drill: bool,
}
impl canvas::Program<Message> for ToolPicture {
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
        let (cx, w) = (bounds.width * 0.6, 60.);
        f.fill_rectangle(Point::new(cx - 44., 16.), Size::new(88., 62.), JAW);
        f.stroke(
            &canvas::Path::rectangle(Point::new(cx - 44., 16.), Size::new(88., 62.)),
            stroke(MUTED, 1.),
        );
        write(
            &mut f,
            "holder",
            Point::new(cx + 52., 46.),
            MUTED,
            13.,
            Align::Left,
        );
        let (top, h) = (78., 96.);
        let body = canvas::Path::new(|b| {
            b.move_to(Point::new(cx - w / 2., top));
            b.line_to(Point::new(cx + w / 2., top));
            b.line_to(Point::new(cx + w / 2., top + h));
            if self.drill {
                b.line_to(Point::new(cx, top + h + 18.));
            }
            b.line_to(Point::new(cx - w / 2., top + h));
            b.close();
        });
        f.fill(&body, Color { a: 0.3, ..TOOL });
        f.stroke(&body, stroke(TOOL, 1.6));
        for i in 0..4 {
            let y = top + 14. + i as f32 * 21.;
            f.stroke(
                &canvas::Path::line(Point::new(cx - w / 2., y), Point::new(cx + w / 2., y + 14.)),
                stroke(TOOL, 1.4),
            );
        }
        let y = top + h + 34.;
        arrow(
            &mut f,
            Point::new(cx, y),
            Point::new(cx - w / 2., y),
            INK,
            1.2,
        );
        arrow(
            &mut f,
            Point::new(cx, y),
            Point::new(cx + w / 2., y),
            INK,
            1.2,
        );
        f.fill_text(canvas::Text {
            content: format!("Ø {}", dro(self.diameter)),
            position: Point::new(cx, y + 18.),
            color: INK,
            size: 16.into(),
            align_x: iced::advanced::text::Alignment::Center,
            align_y: iced::alignment::Vertical::Center,
            ..Default::default()
        });
        // The gear.
        let g = Point::new(bounds.width * 0.18, 120.);
        f.stroke(&canvas::Path::circle(g, 40.), stroke(ACCENT, 2.5));
        f.fill_text(canvas::Text {
            content: format!("{}", self.rpm.round()),
            position: g,
            color: ACCENT,
            size: 22.into(),
            align_x: iced::advanced::text::Alignment::Center,
            align_y: iced::alignment::Vertical::Center,
            ..Default::default()
        });
        f.fill_text(canvas::Text {
            content: "rpm".into(),
            position: Point::new(g.x, g.y + 58.),
            color: MUTED,
            size: 13.into(),
            align_x: iced::advanced::text::Alignment::Center,
            align_y: iced::alignment::Vertical::Center,
            ..Default::default()
        });
        vec![f.into_geometry()]
    }
}
/// The tool seen from above with the edge the DRO's compensation shows on
/// X and Y.
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
        let (c, r) = (Point::new(bounds.width / 2., bounds.height / 2. + 4.), 30.);
        let body = canvas::Path::circle(c, r);
        f.fill(&body, Color { a: 0.25, ..TOOL });
        f.stroke(&body, stroke(TOOL, 1.6));
        f.fill(&canvas::Path::circle(c, 2.5), TOOL);
        let [x, y] = self.0.map(f32::from);
        if x != 0. {
            let to = Point::new(c.x + x * r, c.y);
            arrow(&mut f, c, to, ACCENT, 2.);
            let align = if x < 0. { Align::Right } else { Align::Left };
            write(
                &mut f,
                format!("X {}", if x > 0. { "+" } else { "−" }),
                Point::new(to.x + x * 8., c.y),
                ACCENT,
                15.,
                align,
            );
        }
        if y != 0. {
            let to = Point::new(c.x, c.y - y * r);
            arrow(&mut f, c, to, ACCENT, 2.);
            write(
                &mut f,
                format!("Y {}", if y > 0. { "+" } else { "−" }),
                Point::new(c.x + 6., to.y - y * 10.),
                ACCENT,
                15.,
                Align::Left,
            );
        }
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
            canvas(Swatch(k)).width(16).height(10),
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
        canvas(CompPicture(comp)).width(Fill).height(height).into(),
    ]
}

/// A new tool picks up Z: touch the top and set Z to `top`.
fn pick_up<'a>(top: f32) -> [Element<'a, Message>; 2] {
    [
        label("Pick up Z with the tool"),
        readout(
            Readout {
                order: None,
                lead: Some("touch the top → set".into()),
                axis: "Z".into(),
                value: dro(top),
                lock: None,
                tags: vec![],
                lit: Lit::Next,
                big: false,
            },
            None,
        ),
    ]
}

/// What the page needs at hand for a step.
struct Page<'a> {
    app: &'a App,
    shown: &'a Shown,
    /// The fingerprint of the guide, for the drawings' keys.
    key: u64,
    at: usize,
    item: usize,
    setup: &'a Setup,
}
impl<'a> Page<'a> {
    fn guide(&self) -> &'a Guide {
        &self.shown.guide
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
        self.app.doc.tools.iter().find(|t| &t.id == id)
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
        match self.app.doc.tools.iter().find(|t| t.id == id) {
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
        height: f32,
    ) -> Element<'a, Message> {
        self.drawing_with(view, stock, shapes, height, &self.shown.jaws)
    }
    /// A drawing with these `jaws`.
    fn drawing_with(
        &self,
        view: usize,
        stock: Bounds,
        shapes: Vec<Shape>,
        height: f32,
        jaws: &'a [Bounds],
    ) -> Element<'a, Message> {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        (self.key, self.at, self.item, view).hash(&mut h);
        canvas(Drawing {
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
    fn picture(&self, i: usize, size: (f32, f32)) -> Element<'a, Message> {
        Self::image(self.shown.pictures.get(i).cloned().flatten(), size)
    }
    /// The stock as it starts, in its vise on its parallels.
    fn held_picture(&self, size: (f32, f32)) -> Element<'a, Message> {
        Self::image(self.shown.held.clone(), size)
    }
    fn image(picture: Option<image::Handle>, (w, h): (f32, f32)) -> Element<'a, Message> {
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
    /// The cuts of the passes of the operations before `op`.
    fn cut_before(&self, op: usize) -> Vec<Shape> {
        let first = self.done(op);
        (self.guide().steps.iter())
            .filter_map(|s| match s {
                Step::Pass { op: o, cut, .. } if self.done(*o) < first => {
                    Some(Shape::Hatch(*cut, Hatch::Earlier))
                }
                _ => None,
            })
            .collect()
    }
}

/// The page.
pub fn view(app: &App) -> Element<'_, Message> {
    let main: Element<_> = match app.current_guide() {
        Some(Ok(shown)) => guide_page(app, shown),
        Some(Err(e)) => notice(vec![e.clone(), "Fix the setup in MANUFACTURE.".into()]),
        None if app.guide_key().is_none() => notice(vec!["Load the part to see the steps.".into()]),
        None => notice(vec!["Simulating the setup…".into()]),
    };
    row![setups(app), container(main).width(Fill).height(Fill)]
        .spacing(12)
        .padding(12)
        .height(Fill)
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
    let mut c = column![bold(title, 16), muted("Setups")].spacing(8);
    for (si, s) in app.doc.setups.iter().enumerate() {
        if !manual::is_hand(&s.machine) {
            continue;
        }
        let on = s.id == app.doc.active;
        let msg = Message::Guide(GuideAction::Open(si));
        let b = button(column![bold(s.name(), 13), muted(s.machine.name())].spacing(2))
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
        c = c.push(HelpButton::new(b, &msg, s.name()).width(Fill));
    }
    let active = app.doc.index();
    let cnc = (app.doc.setups.iter().enumerate())
        .filter(|(_, s)| !manual::is_hand(&s.machine))
        .min_by_key(|(si, _)| (*si < active, *si));
    if let Some((si, s)) = cnc {
        c = c.push(space().height(8)).push(muted("CNC")).push(
            crate::ui::secondary(
                format!("{} ▶", s.name()),
                Message::Guide(GuideAction::Open(si)),
            )
            .width(Fill),
        );
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
    container(c)
        .padding(14)
        .width(210)
        .height(Fill)
        .style(card)
        .into()
}
/// The section a step is in.
fn section_of(g: &Guide, at: usize) -> usize {
    (g.sections.iter())
        .position(|s| s.steps.contains(&at))
        .unwrap_or(0)
}
/// Where the place is in its section: "pass 2/4", "hole 1/4", "2/6".
fn progress(g: &Guide, s: &Section, at: usize, item: usize) -> String {
    use crate::guide::LatheStep as L;
    match &g.steps[at] {
        Step::Pass { index, of, .. }
        | Step::Lathe(L::Pass { index, of, .. } | L::ThreadPass { index, of, .. }) => {
            format!("pass {index}/{of}")
        }
        Step::Holes { holes, .. } => format!("hole {}/{}", item + 1, holes.len()),
        _ if s.steps.len() > 1 => format!("{}/{}", at - s.steps.start + 1, s.steps.len()),
        _ => String::new(),
    }
}
fn section_name(app: &App, g: &Guide, s: &Section) -> String {
    let lathe = (app.doc.setups.get(g.setup)).is_some_and(|s| crate::lathe::is_lathe(&s.machine));
    match (s.op, &g.steps[s.steps.start]) {
        (None, Step::HandOver { next: Some(n) }) => {
            format!("→ {}", app.doc.setups.get(*n).map_or("", |s| s.name()))
        }
        // On the lathe every operation turns: "Outside", not "Turn · Outside".
        (Some(_), _) if lathe => (s.name.strip_prefix("Turn · ").unwrap_or(&s.name)).to_string(),
        _ => s.name.clone(),
    }
}
fn guide_page<'a>(app: &'a App, shown: &'a Shown) -> Element<'a, Message> {
    let g = &shown.guide;
    let at = crate::guide::place(&app.doc, g);
    let here = section_of(g, at);
    let setup = &app.doc.setups[g.setup];
    let page = Page {
        app,
        shown,
        key: app.guide.as_ref().map_or(0, |(k, _)| *k),
        at,
        item: app.guide_item,
        setup,
    };
    // The bar of sections.
    let mut bar = row![].spacing(6);
    for (i, s) in g.sections.iter().enumerate() {
        let done = s.steps.end <= at;
        let now = i == here;
        let mark = if done {
            "✓".to_string()
        } else {
            i.to_string()
        };
        let mut face = row![disc(mark, now, 20.), text(section_name(app, g, s)).size(13)]
            .spacing(6)
            .align_y(Alignment::Center);
        let small = if now {
            progress(g, s, at, app.guide_item)
        } else if s.count > 0 && !done {
            s.count.to_string()
        } else {
            String::new()
        };
        if !small.is_empty() {
            face = face.push(text(small).size(12).color(if now { ACCENT } else { MUTED }));
        }
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
        bar = bar.push(HelpButton::new(b, &msg, s.name.clone()));
    }
    // The header of the section the place is in.
    let s = &g.sections[here];
    let mut header = row![
        disc(here.to_string(), true, 26.),
        bold(section_name(app, g, s), 17)
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    let mut tags = vec![];
    let lathe = crate::lathe::is_lathe(&setup.machine);
    if lathe && let Some(op) = s.op {
        tags.extend(turning::header(&page, op, s));
    } else if let Some(op) = s.op
        && let Some(o) = setup.operations.get(op)
    {
        let d = num(
            &page.tool(op).map_or(Default::default(), |t| t.p.clone()),
            "diameter",
        );
        if let Some(d) = d {
            tags.push(chip(page.tool_name(op, d)));
        }
        if s.count > 0 {
            let what = if o.strategy == Strategy::Drill {
                "hole"
            } else {
                "pass"
            };
            let what = match (s.count, what) {
                (1, w) => w.to_string(),
                (_, "pass") => "passes".into(),
                (_, w) => format!("{w}s"),
            };
            tags.push(chip(format!("{} {what}", s.count)));
        }
        tags.push(chip(format!("~{} min", s.minutes.ceil())));
    }
    header = header
        .push(row(tags).spacing(6))
        .push(space().width(Fill))
        .push(bold(progress(g, s, at, app.guide_item), 14).color(ACCENT));
    let top = row![
        bold(
            format!("{} on the {}", setup.name(), setup.machine.name()),
            16
        ),
        muted(format!(
            "~{} min · tap a section to jump to it",
            g.minutes.ceil()
        ))
        .size(13)
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    column![
        top,
        bar.wrap().vertical_spacing(6),
        header,
        scrollable(screen(&page)).height(Fill)
    ]
    .spacing(12)
    .into()
}
/// The command that ends a step: "Pass done ▶", "Section done ▶" on the
/// last step of an operation's section, else "Next ▶".
fn next_label(g: &Guide, at: usize, item: usize) -> String {
    let s = &g.sections[section_of(g, at)];
    match &g.steps[at] {
        Step::Pass { .. }
        | Step::Lathe(crate::guide::LatheStep::Pass { .. })
        | Step::Lathe(crate::guide::LatheStep::ThreadPass { .. }) => "Pass done ▶".into(),
        Step::Holes { holes, .. } if item + 1 < holes.len() => "Hole done ▶".into(),
        _ if s.op.is_some() && at + 1 == s.steps.end => "Section done ▶".into(),
        _ => "Next ▶".into(),
    }
}
fn screen<'a>(p: &Page<'a>) -> Element<'a, Message> {
    let g = p.guide();
    let next = Some((
        next_label(g, p.at, p.item),
        Message::Guide(GuideAction::Next),
    ));
    let (title, sub, left, right): (String, &str, Element<_>, Column<_>) = match &g.steps[p.at] {
        Step::Hold {
            stock,
            holding,
            vise,
        } => {
            let d = stock.dimensions();
            let mut c = vec![chip(p.app.doc.holding_label(p.setup)), chip("top face up")];
            // Across the jaws, seen from the front or the right: the stock
            // on its parallels, the jaws at their height and the grip.
            let view = match vise {
                Some(v) if v.across == 1 => 2,
                _ => 1,
            };
            let (k, top) = (VIEWS[view][0], stock.max[2]);
            let height = stock.max[2] - stock.min[2];
            let mut shapes: Vec<Shape> =
                p.shown.parallels.iter().map(|b| Shape::Block(*b)).collect();
            let mut mid = [0.; 3];
            mid[k] = (stock.min[k] + stock.max[k]) / 2.;
            mid[2] = top;
            shapes.push(Shape::Down(
                mid,
                (height * 0.4).max(8.),
                "THIS FACE UP".into(),
            ));
            if let Some(v) = vise {
                let jaw = holding.bottom + v.grip;
                let (mut a, mut b) = ([0.; 3], [0.; 3]);
                // Outside the + jaw.
                let out = stock.max[k] + v.jaw_thickness;
                (a[k], b[k]) = (out, out);
                (a[2], b[2]) = (jaw, holding.bottom);
                shapes.push(Shape::Height(a, b, format!("{} in the jaws", mm(v.grip))));
                c.push(chip(format!("jaws on the {} sides", AXES[v.across])));
                c.push(chip(format!("{} in the jaws", mm(v.grip))));
                let under = holding.bottom + v.grip - v.jaw_height;
                if !p.shown.parallels.is_empty() {
                    c.push(chip(format!(
                        "on parallels {} high",
                        mm(stock.min[2] - under)
                    )));
                }
            }
            let left = column![
                p.held_picture((420., 250.)),
                p.drawing_with(view, *stock, shapes, 200., &p.shown.vise),
            ]
            .spacing(6)
            .align_x(Alignment::Center)
            .into();
            let right = column![
                label("Stock as it starts"),
                chips(c),
                readout(
                    Readout {
                        order: None,
                        lead: None,
                        axis: String::new(),
                        value: format!("{} × {} × {}", mm(d[0]), mm(d[1]), mm(d[2])),
                        lock: None,
                        tags: vec!["X × Y × Z".into()],
                        lit: Lit::Next,
                        big: false
                    },
                    None
                ),
            ];
            ("Hold".into(), "where it goes in the vise", left, right)
        }
        Step::Zero {
            approach,
            touch,
            finder,
            ..
        } => {
            let stock = p.shown.boxes[0];
            let sides = [["left", "right"], ["front", "back"]];
            let side = |k: usize| sides[k][usize::from(approach[k] > 0)];
            // The work Z 0 is on the top, or the top is at a height.
            let top = touch[2].abs() < 0.005;
            let mut shapes = vec![];
            for k in 0..2 {
                let mut at = [0., 0., stock.max[2]];
                let o = 1 - k;
                at[o] = (stock.min[o] + stock.max[o]) / 2.;
                at[k] = if approach[k] > 0 {
                    stock.max[k]
                } else {
                    stock.min[k]
                };
                let mut from = [0.; 2];
                from[k] = f32::from(approach[k].signum());
                let set = format!("{} {}", AXES[k], dro(touch[k]));
                shapes.push(Shape::Finder(at, from, set));
            }
            shapes.push(Shape::Word(
                [
                    (stock.min[0] + stock.max[0]) / 2. - 12.,
                    (stock.min[1] + stock.max[1]) / 2.,
                    0.,
                ],
                if top {
                    "Z 0 = top".into()
                } else {
                    format!("top = Z {}", dro(touch[2]))
                },
                INK,
            ));
            let rows = (0..3).map(|k| {
                let what = if k == 2 {
                    "touch the top → set".to_string()
                } else {
                    format!("touch the {} face → set", side(k))
                };
                readout(
                    Readout {
                        order: Some(k as u8 + 1),
                        lead: Some(what),
                        axis: AXES[k].into(),
                        value: dro(touch[k]),
                        lock: None,
                        tags: vec![],
                        lit: lit(k, p.item),
                        big: false,
                    },
                    Some(Message::Guide(GuideAction::Reached(k))),
                )
            });
            let right = column![
                chips(vec![
                    chip(format!("Ø{} edge finder", mm(*finder))),
                    warn_chip("DRO comp off"),
                ]),
                label("Touch, then set the DRO"),
            ]
            .extend(rows);
            (
                "Zero".into(),
                "pick up the corner",
                p.drawing(0, stock, shapes, 380.),
                right,
            )
        }
        Step::Tool {
            op,
            tool,
            rpm,
            diameter,
            comp,
            top,
        } => {
            let drill = p
                .setup
                .operations
                .get(*op)
                .is_some_and(|o| o.strategy == Strategy::Drill);
            let right = column![chips(vec![
                chip(p.named(tool, *diameter)),
                chip(format!("{} rpm", rpm.round()))
            ]),]
            .extend(pick_up(*top))
            .extend(dro_tool(*diameter, *comp, 110.));
            let left = canvas(ToolPicture {
                diameter: *diameter,
                rpm: *rpm,
                drill,
            })
            .width(Fill)
            .height(300)
            .into();
            ("Tool".into(), "put it in, set the gear", left, right)
        }
        Step::Comp { op, comp } => {
            let right = column![
                chips(vec![chip(p.tool(*op).map_or("", |t| t.name()).to_string())]),
                label("Set the DRO comp"),
            ]
            .extend((0..2).map(|k| {
                let side = match comp[k] {
                    1 => "+",
                    -1 => "−",
                    _ => "off",
                };
                readout(
                    Readout {
                        order: None,
                        lead: Some("comp".into()),
                        axis: AXES[k].into(),
                        value: side.into(),
                        lock: None,
                        tags: vec!["edge side".into()],
                        lit: Lit::Next,
                        big: false,
                    },
                    None,
                )
            }));
            let left = canvas(CompPicture(*comp)).width(Fill).height(220).into();
            ("Comp".into(), "same tool, other sides", left, right)
        }
        Step::Face {
            op,
            tool: id,
            rpm,
            diameter,
            top,
            remove,
            to,
        } => {
            let stock = p.before(*op);
            let mut band = stock;
            band.min[2] = *to;
            let r = diameter / 2.;
            let tool = [stock.min[0] + r * 0.6, stock.max[1] - r * 0.6, *to];
            let never = Shape::Never(
                [stock.min[0] - 4., 0., *to],
                [stock.max[0] + 4., 0., *to],
                format!("Z {}", dro(*to)),
            );
            let left = column![
                p.drawing(
                    0,
                    stock,
                    vec![Shape::Hatch(band, Hatch::Cut), Shape::Tool(tool, r, None)],
                    250.
                ),
                p.drawing(
                    1,
                    stock,
                    vec![
                        Shape::Hatch(band, Hatch::Cut),
                        never,
                        Shape::Tool([tool[0], 0., *to], r, None)
                    ],
                    160.
                ),
            ]
            .spacing(6)
            .into();
            let right = column![chips(vec![
                chip(p.named(id, *diameter)),
                chip(format!("{} rpm", rpm.round()))
            ]),]
            .extend(pick_up(*top))
            .push(label("Face down to"))
            .push(readout(
                Readout {
                    order: None,
                    lead: Some("Face to".into()),
                    axis: "Z".into(),
                    value: dro(*to),
                    lock: None,
                    tags: vec![format!("{} mm off", dro(*remove))],
                    lit: Lit::Now,
                    big: true,
                },
                None,
            ))
            .push(muted("Your passes, your stepdown.").size(13));
            (
                "Face".into(),
                "the only screen: tool and depth",
                left,
                right,
            )
        }
        Step::Square {
            op,
            tool: id,
            rpm,
            diameter,
            comp,
            top,
            axis,
            side,
            to,
            shown,
            remove,
            down_to,
        } => {
            let (k, sd) = (*axis, f32::from(*side));
            let stock = p.before(*op);
            let r = diameter / 2.;
            let mut slab = stock;
            (slab.min[k], slab.max[k]) = if sd > 0. {
                (*to, stock.max[k])
            } else {
                (stock.min[k], *to)
            };
            slab.min[2] = *down_to;
            let o = 1 - k;
            let mut a = [0.; 3];
            let mut b = [0.; 3];
            (a[k], b[k], a[o], b[o]) = (*to, *to, stock.min[o], stock.max[o]);
            let mut tool = [0., 0., *down_to];
            tool[k] = to + sd * r;
            tool[o] = (stock.min[o] + stock.max[o]) / 2.;
            let mut word = tool;
            word[k] = to + sd * (r * 2. + 2.);
            let view = if k == 0 { 1 } else { 2 };
            let never = Shape::Never(
                {
                    let mut q = [0.; 3];
                    q[k] = stock.min[k] - 4.;
                    q[2] = *down_to;
                    q
                },
                {
                    let mut q = [0.; 3];
                    q[k] = stock.max[k] + 4.;
                    q[2] = *down_to;
                    q
                },
                format!("Z {}", dro(*down_to)),
            );
            let mut tool_side = [0.; 3];
            tool_side[k] = tool[k];
            tool_side[2] = *down_to;
            let left = column![
                p.drawing(
                    0,
                    stock,
                    vec![
                        Shape::Hatch(slab, Hatch::Cut),
                        Shape::Wall(a, b),
                        Shape::Tool(tool, r, None),
                        Shape::Word(word, format!("{} mm off", mm(*remove)), ACCENT)
                    ],
                    260.
                ),
                p.drawing(
                    view,
                    stock,
                    vec![
                        Shape::Hatch(slab, Hatch::Cut),
                        never,
                        Shape::Tool(tool_side, r, None)
                    ],
                    150.
                ),
            ]
            .spacing(6)
            .into();
            let names = [["left", "right"], ["front", "back"]];
            let edge = get(&p.setup.machine.p, "dro_comp") == "Yes";
            let right = column![chips(vec![
                chip(p.named(id, *diameter)),
                chip(format!("{} rpm", rpm.round())),
                chip(format!("down to Z {}", dro(*down_to)))
            ]),]
            .extend(pick_up(*top))
            .extend(dro_tool(*diameter, *comp, 70.))
            .push(label("Cut the side to"))
            .push(readout(
                Readout {
                    order: None,
                    lead: None,
                    axis: AXES[k].into(),
                    value: dro(*shown),
                    lock: None,
                    tags: vec![
                        format!("{} side to", names[k][usize::from(sd > 0.)]),
                        format!(
                            "{} mm off · {}",
                            dro(*remove),
                            if edge { "edge" } else { "centre" }
                        ),
                    ],
                    lit: Lit::Now,
                    big: true,
                },
                None,
            ));
            (
                "Square".into(),
                "the only screen: one side to a number",
                left,
                right,
            )
        }
        step @ Step::Pass { .. } => pass(p, step),
        Step::Holes { op, holes, mouths } => {
            let stock = p.before(*op);
            let i = p.item.min(holes.len().saturating_sub(1));
            let r = p
                .tool(*op)
                .and_then(|t| num(&t.p, "diameter"))
                .unwrap_or(4.)
                / 2.;
            let mut shapes = p.cut_before(*op);
            shapes.extend(
                holes
                    .iter()
                    .enumerate()
                    .map(|(n, h)| Shape::Hole([h[0], h[1], stock.max[2]], r, n + 1, lit(n, i))),
            );
            let h = holes.get(i).copied().unwrap_or([0.; 3]);
            let tapping = p.setup.operations.get(*op).is_some_and(|o| o.tapping());
            // A tap cuts from the hole's mouth: the stock above it may be
            // gone already.
            let mouth = if tapping {
                mouths.get(i).copied().unwrap_or(stock.max[2])
            } else {
                stock.max[2]
            };
            let drilled = Bounds {
                min: [h[0] - r, h[1] - r, h[2]],
                max: [h[0] + r, h[1] + r, mouth],
            };
            let side = vec![
                Shape::Hatch(drilled, Hatch::Cut),
                Shape::Never(
                    [h[0] - 3. * r, 0., h[2]],
                    [h[0] + 3. * r, 0., h[2]],
                    format!("Z {}", dro(h[2])),
                ),
            ];
            let left = column![
                p.drawing(0, stock, shapes, 260.),
                p.drawing(1, stock, side, 150.)
            ]
            .spacing(6)
            .into();
            let rpm = p
                .setup
                .operations
                .get(*op)
                .and_then(|o| num(&o.p, "rpm"))
                .unwrap_or(0.);
            let depth = stock.max[2] - h[2];
            // Tapping: by hand, the spindle holding the tap square; the
            // depth is how many turns the tap goes in.
            let pitch = p.tool(*op).and_then(|t| t.tap_spec()).map(|s| s.pitch);
            if let (true, Some(pitch)) = (tapping, pitch) {
                let depth = mouth - h[2];
                // Rounded down: a turn more would go past the drilled hole.
                let turns = (depth / pitch * 10.).floor() / 10.;
                let mut said = vec![
                    chip(p.tool_name(*op, 2. * r)),
                    chip("by hand · spindle off"),
                    warn_chip("oil"),
                ];
                // The tap's tip comes out under the stock: a parallel there
                // stops it, or it cuts the vise.
                if h[2] < stock.min[2] {
                    said.push(warn_chip("nothing under the hole"));
                }
                let right = column![
                    chips(said),
                    label(format!("Hole {} of {}", i + 1, holes.len())),
                    readout(
                        Readout {
                            order: Some(1),
                            lead: None,
                            axis: "X".into(),
                            value: dro(h[0]),
                            lock: Some("X".into()),
                            tags: vec!["centre".into()],
                            lit: Lit::Next,
                            big: false
                        },
                        None
                    ),
                    readout(
                        Readout {
                            order: Some(2),
                            lead: None,
                            axis: "Y".into(),
                            value: dro(h[1]),
                            lock: Some("Y".into()),
                            tags: vec!["centre".into()],
                            lit: Lit::Next,
                            big: false
                        },
                        None
                    ),
                    readout(
                        Readout {
                            order: Some(3),
                            lead: None,
                            axis: "turns".into(),
                            value: format!("{:.1}", turns),
                            lock: None,
                            tags: vec![format!("{} deep", mm(depth)), "½ back every turn".into(),],
                            lit: Lit::Next,
                            big: true
                        },
                        None
                    ),
                    tiles(
                        (1..=holes.len()).map(|n| (n.to_string(), None)).collect(),
                        i
                    ),
                ];
                ("Tap".into(), "one hole at a time, by hand", left, right)
            } else {
                let right = column![
                    chips(vec![
                        chip(p.tool_name(*op, 2. * r)),
                        chip(format!("{} rpm", rpm.round())),
                        warn_chip("centres")
                    ]),
                    label(format!("Hole {} of {}", i + 1, holes.len())),
                    readout(
                        Readout {
                            order: Some(1),
                            lead: None,
                            axis: "X".into(),
                            value: dro(h[0]),
                            lock: Some("X".into()),
                            tags: vec!["centre".into()],
                            lit: Lit::Next,
                            big: false
                        },
                        None
                    ),
                    readout(
                        Readout {
                            order: Some(2),
                            lead: None,
                            axis: "Y".into(),
                            value: dro(h[1]),
                            lock: Some("Y".into()),
                            tags: vec!["centre".into()],
                            lit: Lit::Next,
                            big: false
                        },
                        None
                    ),
                    readout(
                        Readout {
                            order: Some(3),
                            lead: None,
                            axis: "Z".into(),
                            value: dro(h[2]),
                            lock: None,
                            tags: vec!["drill to".into(), format!("{} deep", mm(depth))],
                            lit: Lit::Next,
                            big: false
                        },
                        None
                    ),
                    tiles(
                        (1..=holes.len()).map(|n| (n.to_string(), None)).collect(),
                        i
                    ),
                ];
                ("Holes".into(), "one hole at a time", left, right)
            }
        }
        Step::Check { op } => {
            let j = p.done(*op);
            let o = p.setup.operations.get(*op);
            let mut c = vec![chip(format!("{} done", o.map_or("", |o| o.name())))];
            if let Some(a) = o
                .filter(|o| o.strategy == Strategy::Box)
                .and_then(|o| num(&o.p, "allowance"))
                .filter(|a| *a > 0.)
            {
                c.push(chip(format!("{} mm left on the walls and floor", mm(a))));
            }
            let right = column![label("Before"), p.picture(j, (240., 160.)), chips(c)];
            (
                "Check".into(),
                "the stock after it",
                p.picture(j + 1, (480., 320.)),
                right,
            )
        }
        Step::Lathe(step) => turning::screen(p, step),
        Step::HandOver { next: to } if p.shown.side.is_some() => {
            return turning::hand_over(p, *to);
        }
        Step::HandOver { next: to } => {
            let last = p.shown.pictures.len() - 1;
            let mut c = vec![chip(format!("manual ~{} min", g.minutes.ceil()))];
            let next = match to.and_then(|n| Some((n, p.app.doc.setups.get(n)?))) {
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
            let right = column![chips(c)];
            return frame(
                "Hand over".into(),
                "the stock leaves the mill",
                p.picture(last, (480., 320.)),
                right.push(space().height(Fill)).push(buttons(next)),
            );
        }
    };
    frame(
        title,
        sub,
        left,
        right.push(space().height(Fill)).push(buttons(next)),
    )
}
/// A step's screen: its caption, the pictures and the numbers.
fn frame<'a>(
    title: String,
    sub: &str,
    left: Element<'a, Message>,
    right: Column<'a, Message>,
) -> Element<'a, Message> {
    let body = row![
        container(left).width(FillPortion(11)),
        container(right.spacing(10).height(Length::Shrink)).width(FillPortion(9))
    ]
    .spacing(18);
    column![
        row![bold(title, 15), muted(sub.to_string()).size(13)]
            .spacing(8)
            .align_y(Alignment::Center),
        container(body).padding(16).width(Fill).style(card)
    ]
    .spacing(6)
    .into()
}
/// Where the tool is before and after the cut of pass `index` of `op`: the
/// targets of its passes replayed.
fn ends(g: &Guide, op: usize, index: usize) -> ([f32; 3], [f32; 3]) {
    let mut at = [0.; 3];
    let mut start = at;
    for s in &g.steps {
        if let Step::Pass {
            op: o,
            index: i,
            targets,
            ..
        } = s
            && *o == op
            && *i <= index
        {
            for (n, t) in targets.iter().enumerate() {
                if *i == index && n + 1 == targets.len() {
                    start = at;
                }
                at[t.axis] = t.centre;
            }
        }
    }
    (start, at)
}
/// The screen of a Pass `step`.
fn pass<'a>(
    p: &Page<'a>,
    step: &'a Step,
) -> (
    String,
    &'static str,
    Element<'a, Message>,
    Column<'a, Message>,
) {
    let Step::Pass {
        op,
        index,
        of,
        targets,
        cut,
        earlier,
        never,
        turns_per_minute,
    } = step
    else {
        unreachable!()
    };
    let (op, index, of, cut, never, turns): (usize, usize, usize, Bounds, Limits, f32) =
        (*op, *index, *of, *cut, *never, *turns_per_minute);
    let targets: &[Target] = targets;
    let g = p.guide();
    let stock = p.before(op);
    let (start, end) = ends(g, op, index);
    let r = g.done.get(p.done(op)).map_or(0., |d| d.plan.radius);
    let k = targets.last().map_or(0, |t| t.axis);
    // The lines not to pass where they are inside the stock.
    let lines = |view: [usize; 2]| {
        let mut out = vec![];
        for (axis, (lo, hi)) in [
            (0, (never.x[0], never.x[1])),
            (1, (never.y[0], never.y[1])),
            (2, (never.z, f32::INFINITY)),
        ] {
            if !view.contains(&axis) {
                continue;
            }
            let other = view[usize::from(view[0] == axis)];
            for v in [lo, hi] {
                if v > stock.min[axis] + 0.05 && v < stock.max[axis] - 0.05 {
                    let (mut a, mut b) = ([0.; 3], [0.; 3]);
                    (a[axis], b[axis]) = (v, v);
                    (a[other], b[other]) = (stock.min[other] - 4., stock.max[other] + 4.);
                    out.push(Shape::Never(a, b, format!("{} {}", AXES[axis], dro(v))));
                }
            }
        }
        out
    };
    let later: Vec<Shape> = (g.steps.iter())
        .filter_map(|s| match s {
            Step::Pass {
                op: o,
                index: i,
                cut,
                ..
            } if *o == op && *i > index => Some(Shape::Hatch(*cut, Hatch::Later)),
            _ => None,
        })
        .collect();
    let mut shapes = p.cut_before(op);
    shapes.extend(earlier.iter().map(|b| Shape::Hatch(*b, Hatch::Earlier)));
    shapes.push(Shape::Hatch(cut, Hatch::Cut));
    let side = if k == 1 { 1 } else { 2 };
    let mut across = shapes.clone();
    across.extend(later);
    across.extend(lines(VIEWS[side]));
    across.push(Shape::Tool(start, r, Some(end)));
    shapes.extend(lines(VIEWS[0]));
    shapes.push(Shape::Tool(start, r, Some(end)));
    let left = column![
        p.drawing(0, stock, shapes, 250.),
        p.drawing(side, stock, across, 160.),
        legend()
    ]
    .spacing(6)
    .into();
    // The numbers, in order.
    // Where the tool is before the first number: where the last pass left it.
    let mut last: [Option<f32>; 3] = match index {
        1 => [None; 3],
        _ => ends(g, op, index - 1).1.map(Some),
    };
    let mut rows = vec![];
    let n = targets.len();
    for (i, t) in targets.iter().enumerate() {
        let locked: Vec<&str> = (0..3).filter(|a| t.lock[*a]).map(|a| AXES[a]).collect();
        let mut tags = vec![];
        let mut value = dro(t.shown);
        if i + 1 == n {
            let arrows = [["◀", "▶"], ["▼", "▲"], ["▼", "▲"]];
            value = format!(
                "{} {value}",
                arrows[t.axis][usize::from(end[t.axis] > start[t.axis])]
            );
            tags.push("feed".into());
        } else if t.axis == 2 && (t.centre - cut.min[2]).abs() < 0.005 {
            // Down to the pass's level: how much deeper it cuts.
            tags.push(format!("{} deeper", mm(cut.max[2] - cut.min[2])));
            if t.feed {
                tags.push("plunge".into());
            }
        } else if t.axis < 2 && t.feed {
            tags.push("feed, cuts".into());
        } else if t.axis < 2
            && let Some(v) = last[t.axis]
            && (v - t.centre).abs() < 1.
        {
            tags.push("off the wall".into());
        }
        if t.edge {
            let arrows = [["←", "→"], ["↓", "↑"], ["", ""]];
            tags.push(format!(
                "edge {}",
                arrows[t.axis][usize::from(t.shown > t.centre)]
            ));
            tags.push(format!("centre {}", dro(t.centre)));
        } else if t.axis < 2 {
            tags.push("centre".into());
        }
        last[t.axis] = Some(t.centre);
        rows.push(readout(
            Readout {
                order: Some(t.order),
                lead: None,
                axis: AXES[t.axis].into(),
                value,
                lock: (!locked.is_empty()).then(|| locked.join(" ")),
                tags,
                lit: lit(i, p.item),
                big: false,
            },
            Some(Message::Guide(GuideAction::Reached(i))),
        ));
    }
    let o = p.setup.operations.get(op);
    let rpm = o.and_then(|o| num(&o.p, "rpm")).unwrap_or(0.);
    let feed = o.and_then(|o| num(&o.p, "feed")).unwrap_or(0.);
    let mut c = vec![
        chip(p.tool_name(op, 2. * r)),
        chip(format!("{} rpm", rpm.round())),
        chip(format!("{} mm/min", feed.round())),
    ];
    if targets.iter().any(|t| t.axis == 2 && t.feed) {
        c.push(warn_chip("plunges: centre-cutting end mill"));
    }
    if turns > 0. {
        let period = 60. / turns;
        let since = p
            .app
            .guide_clock
            .1
            .saturating_duration_since(p.app.guide_clock.0);
        let on = since.as_secs_f32() % period < period.min(1.) * 0.3;
        let dot = container(space())
            .width(10)
            .height(10)
            .style(move |_| container::Style {
                background: Some(
                    Color {
                        a: if on { 1. } else { 0.18 },
                        ..ACCENT
                    }
                    .into(),
                ),
                border: Border {
                    radius: 5.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        c.push(
            container(
                row![dot, text(format!("1 turn every {} s", mm(period))).size(13)]
                    .spacing(6)
                    .align_y(Alignment::Center),
            )
            .padding([4, 9])
            .style(|_| container::Style {
                background: Some(CHIP.into()),
                border: Border {
                    radius: 6.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into(),
        );
    }
    // The tiles of the operation's passes: their floors when each pass is
    // a level of its own, else their numbers.
    let passes: Vec<(usize, Bounds)> = (g.steps.iter().enumerate())
        .filter_map(|(i, s)| match s {
            Step::Pass { op: o, cut, .. } if *o == op => Some((i, *cut)),
            _ => None,
        })
        .collect();
    // Each tile is its pass's depth (work Z), and its row on that level
    // when a level has more than one.
    let level = |c: &Bounds| (c.min[2] * 100.).round() as i64;
    let labels = (passes.iter().enumerate())
        .map(|(n, (i, c))| {
            let on: Vec<usize> = (0..passes.len())
                .filter(|&m| level(&passes[m].1) == level(c))
                .collect();
            let s = match on.iter().position(|&m| m == n) {
                Some(row) if on.len() > 1 => format!("{} · {}", dro(c.min[2]), row + 1),
                _ => dro(c.min[2]),
            };
            (s, Some(Message::Guide(GuideAction::Go(*i))))
        })
        .collect();
    let right = column![
        chips(c),
        label(format!("Move in this order · pass {index} of {of}"))
    ]
    .extend(rows)
    .push(tiles(labels, index - 1));
    (
        format!("Pass {index} of {of}"),
        "one screen per pass",
        left,
        right,
    )
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
    /// Builds the active setup's guide in line, as the background run does.
    pub(crate) fn build(a: &mut App) {
        let key = a.guide_key().unwrap();
        let built = shown(&a.doc, a.part_mesh().unwrap(), a.doc.index()).map(Arc::new);
        a.guide = Some((key, built));
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
    /// The two texts after `lead`: the axis and the number.
    fn after(t: &[String], lead: &str) -> Vec<String> {
        let i = (t.iter().position(|s| s == lead)).unwrap_or_else(|| panic!("{lead} in {t:?}"));
        t[i + 1..i + 3].to_vec()
    }
    #[test]
    fn the_face_step_shows_the_tool_the_gear_and_the_depth_and_no_passes() {
        let mut a = app("Yes");
        let face = (guide(&a).steps.iter())
            .position(|s| matches!(s, Step::Face { .. }))
            .unwrap();
        send(&mut a, GuideAction::Go(face));
        assert_eq!(place(&a), face);
        let t = texts(&a);
        for want in [
            "Face to",
            "Z",
            "−2.00",
            "2.00 mm off",
            "640 rpm",
            "Ø25 End mill 25",
            "Section done ▶",
        ] {
            assert!(t.iter().any(|s| s == want), "{want} in {t:?}");
        }
        assert!(
            !t.iter()
                .any(|s| s.starts_with("Move in this order") || s == "Pass done ▶"),
            "{t:?}"
        );
        // The face's tool picks up Z on the top first.
        assert_eq!(after(&t, "touch the top → set"), ["Z", "0.00"]);
    }
    #[test]
    fn a_new_tool_picks_up_z_on_the_top_the_operations_before_left() {
        let mut a = app("Yes");
        let g = guide(&a);
        let tools: Vec<usize> = (g.steps.iter().enumerate())
            .filter(|(_, s)| matches!(s, Step::Tool { .. }))
            .map(|(i, _)| i)
            .collect();
        // The Step's mill and the Holes' drill, after the 2 mm face.
        assert_eq!(tools.len(), 2);
        for i in tools {
            send(&mut a, GuideAction::Go(i));
            let t = texts(&a);
            assert!(t.iter().any(|s| s == "PICK UP Z WITH THE TOOL"), "{t:?}");
            assert_eq!(after(&t, "touch the top → set"), ["Z", "−2.00"]);
        }
        let t = texts(&squared("Yes"));
        assert_eq!(after(&t, "touch the top → set"), ["Z", "−2.00"]);
    }
    /// The job with the stock's +X side squared 1 mm first (step 3).
    fn squared(dro: &str) -> App {
        let mut a = app_with(dro, |doc| {
            let mill = doc.tools[3].id.clone();
            let b = [123., 500., -500., 500., 0., -30.];
            let square =
                crate::guide::tests::boxed("Square right side", &mill, b, &[("allowance", "0")]);
            doc.setups[0].operations.insert(1, square);
        });
        assert!(matches!(guide(&a).steps[3], Step::Square { .. }));
        send(&mut a, GuideAction::Go(3));
        a
    }
    #[test]
    fn the_hold_step_draws_the_vise_its_grip_and_parallels() {
        let a = app_with("Yes", crate::guide::tests::in_the_vise);
        let shown = a.guide_at().unwrap().0;
        let Step::Hold {
            stock,
            holding,
            vise: Some(v),
        } = shown.guide.steps[0]
        else {
            panic!()
        };
        let t = texts(&a);
        let grip = format!("{} in the jaws", mm(v.grip));
        for want in ["jaws on the Y sides", grip.as_str(), "top face up"] {
            assert!(t.iter().any(|s| s == want), "{want} in {t:?}");
        }
        // The jaws reach from the grip down by their height, the parallels
        // from there up to the stock's bottom, between the jaws.
        assert_eq!(shown.vise.len(), 2);
        for j in &shown.vise {
            close(j.max[2], holding.bottom + v.grip);
            close(j.max[2] - j.min[2], v.jaw_height);
        }
        assert_eq!(shown.parallels.len(), 2);
        for b in &shown.parallels {
            close(b.max[2], stock.min[2]);
            close(b.min[2], holding.bottom + v.grip - v.jaw_height);
            assert!(b.min[1] > stock.min[1] && b.max[1] < stock.max[1], "{b:?}");
        }
        let high = format!(
            "on parallels {} high",
            mm(stock.min[2] - shown.parallels[0].min[2])
        );
        assert!(t.contains(&high), "{high} in {t:?}");
        // Without a vise: no jaws, no parallels.
        let a = app("Yes");
        let shown = a.guide_at().unwrap().0;
        assert!(shown.vise.is_empty() && shown.parallels.is_empty());
    }
    fn close(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-3, "{a} != {b}");
    }
    #[test]
    fn the_square_step_shows_the_dro_diameter_and_comp_side() {
        let t = texts(&squared("Yes"));
        for want in [
            "ENTER IN THE DRO",
            "Ø",
            "25.00",
            "comp X −",
            "123.00",
            "right side to",
        ] {
            assert!(t.iter().any(|s| s.contains(want)), "{want} in {t:?}");
        }
        // Without compensation: the centre, and no diameter to enter.
        let t = texts(&squared("No"));
        assert!(t.iter().any(|s| s == "DRO comp off: centres"), "{t:?}");
        assert!(t.iter().any(|s| s == "135.50"), "{t:?}");
        assert!(!t.iter().any(|s| s.contains("comp X")), "{t:?}");
    }
    #[test]
    fn the_zero_step_sets_each_touch_less_the_edge_finders_radius() {
        let mut a = app("Yes");
        send(&mut a, GuideAction::Go(1));
        let t = texts(&a);
        for want in [
            "Ø10 edge finder",
            "DRO comp off",
            "touch the left face → set",
            "touch the front face → set",
            "touch the top → set",
        ] {
            assert!(t.iter().any(|s| s == want), "{want} in {t:?}");
        }
        // Each number follows its touch: X −5, Y −5, Z 0.
        assert_eq!(after(&t, "touch the left face → set"), ["X", "−5.00"]);
        assert_eq!(after(&t, "touch the front face → set"), ["Y", "−5.00"]);
        assert_eq!(after(&t, "touch the top → set"), ["Z", "0.00"]);
    }
    #[test]
    fn a_pass_shows_its_number_and_its_readouts_in_order() {
        let mut a = app("Yes");
        let g = guide(&a);
        let first = (g.steps.iter())
            .position(|s| matches!(s, Step::Pass { .. }))
            .unwrap();
        send(&mut a, GuideAction::Go(first));
        let Step::Pass { targets, of, .. } = &g.steps[first] else {
            panic!()
        };
        let t = texts(&a);
        assert!(t.contains(&format!("Pass 1 of {of}")), "{t:?}");
        assert!(t.contains(&format!("pass 1/{of}")), "{t:?}");
        // Every readout's number, the cut's with its direction, in order.
        let shown: Vec<usize> = (targets.iter())
            .map(|g| {
                let n = dro(g.shown);
                (t.iter())
                    .position(|s| s == &n || s.ends_with(&format!(" {n}")))
                    .unwrap_or_else(|| panic!("{n} in {t:?}"))
            })
            .collect();
        assert!(shown.windows(2).all(|w| w[0] < w[1]), "{shown:?}");
        assert!(t.iter().any(|s| s == "Pass done ▶"));
        assert!(t.iter().any(|s| s.starts_with("1 turn every")), "{t:?}");
        // The feed runs toward +X (conventional: the step's wall on the left).
        let feed = dro(targets.last().unwrap().shown);
        assert!(t.contains(&format!("▶ {feed}")), "{t:?}");
        // The edge readouts carry their centre.
        let edge = targets.iter().find(|g| g.edge).unwrap();
        assert!(
            t.iter()
                .any(|s| s.contains(&format!("centre {}", dro(edge.centre)))),
            "{t:?}"
        );
        // Down to the level: how much deeper it cuts, from the faced top at
        // −2 to −6.75; not how far from the retract height.
        assert_eq!(
            t.iter().filter(|s| s.starts_with("4.75 deeper")).count(),
            1,
            "{t:?}"
        );
        assert!(!t.iter().any(|s| s.contains(" down")), "{t:?}");
        // The tiles are the passes' depths, two rows on each level.
        for tile in ["−6.75 · 1", "−6.75 · 2", "−11.50 · 1", "−11.50 · 2"] {
            assert!(t.iter().any(|s| s == tile), "{tile} in {t:?}");
        }
        // Tapping a readout lights the next.
        send(&mut a, GuideAction::Reached(0));
        assert_eq!(a.guide_item, 1);
    }
    #[test]
    fn a_pass_that_plunges_warns_of_a_centre_cutting_end_mill() {
        let mut a = app_with("Yes", crate::guide::tests::close_the_step);
        let g = guide(&a);
        let passes: Vec<usize> = (g.steps.iter().enumerate())
            .filter(|(_, s)| matches!(s, Step::Pass { op: 1, .. }))
            .map(|(i, _)| i)
            .collect();
        let warning = "plunges: centre-cutting end mill";
        for (n, i) in passes.into_iter().enumerate() {
            send(&mut a, GuideAction::Go(i));
            let t = texts(&a);
            assert_eq!(t.iter().any(|s| s == warning), n % 2 == 0, "{n} {t:?}");
            if n % 2 == 1 {
                assert!(t.iter().any(|s| s.contains("feed, cuts")), "{t:?}");
                assert!(t.iter().any(|s| s.contains("off the wall")), "{t:?}");
            }
        }
        // The open-ended Step drops outside the stock: no warning.
        let mut a = app("Yes");
        let first = (guide(&a).steps.iter())
            .position(|s| matches!(s, Step::Pass { .. }))
            .unwrap();
        send(&mut a, GuideAction::Go(first));
        assert!(!texts(&a).iter().any(|s| s == warning));
    }
    #[test]
    fn next_and_back_move_the_place_and_it_is_saved_without_an_undo_step() {
        let mut a = app("Yes");
        assert_eq!(place(&a), 0);
        assert!(has(&a, "Hold"));
        let edits = a.project_dirty;
        send(&mut a, GuideAction::Next);
        send(&mut a, GuideAction::Next);
        assert_eq!(place(&a), 2);
        send(&mut a, GuideAction::Back);
        assert_eq!(place(&a), 1);
        assert!(has(&a, "Zero"));
        assert!(a.dirty && a.undo.is_none());
        assert_eq!(a.project_dirty, edits);
        // At the Holes, Next and Back go hole by hole first.
        let g = guide(&a);
        let holes = (g.steps.iter())
            .position(|s| matches!(s, Step::Holes { .. }))
            .unwrap();
        send(&mut a, GuideAction::Go(holes));
        send(&mut a, GuideAction::Next);
        assert_eq!((place(&a), a.guide_item), (holes, 1));
        assert!(has(&a, "HOLE 2 OF 4"));
        send(&mut a, GuideAction::Back);
        send(&mut a, GuideAction::Back);
        assert_eq!((place(&a), a.guide_item), (holes - 1, 0));
        // The place goes with the project.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("job.axis.json");
        crate::storage::save(&path, &a.doc).unwrap();
        let doc = crate::storage::load(&path).unwrap();
        assert_eq!(doc.guide.get(&doc.setups[0].id), Some(&(holes - 1)));
    }
    #[test]
    fn a_section_opens_at_its_first_unfinished_step_and_the_bar_shows_progress() {
        let mut a = app("Yes");
        let g = guide(&a);
        let names: Vec<&str> = g.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Set up", "Face", "Step", "Holes", "Hand over"]);
        send(&mut a, GuideAction::Section(2));
        assert_eq!(place(&a), g.sections[2].steps.start);
        assert!(has(&a, "1/6"), "{:?}", texts(&a));
        // Two passes on: the bar and the header say where.
        send(&mut a, GuideAction::Next);
        send(&mut a, GuideAction::Next);
        let t = texts(&a);
        assert_eq!(t.iter().filter(|s| *s == "pass 2/4").count(), 2, "{t:?}");
        // The sections ahead show their counts, the next setup is last.
        assert!(t.contains(&"Holes".to_string()) && t.contains(&"4".to_string()));
        assert!(
            t.contains(&format!("→ {}", a.doc.setups[1].name())),
            "{t:?}"
        );
        // Tapping the section the place is in keeps it; another starts it.
        let at = place(&a);
        send(&mut a, GuideAction::Section(2));
        assert_eq!(place(&a), at);
        send(&mut a, GuideAction::Section(1));
        assert_eq!(place(&a), g.sections[1].steps.start);
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
        assert!(has(&a, "It broke.") && has(&a, "Fix the setup in MANUFACTURE."));
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
    fn render(a: &App, (w, h): (u32, u32), path: &std::path::Path) {
        use iced::{
            Pixels,
            advanced::renderer::{self, Headless},
            futures::FutureExt,
        };
        use iced_runtime::user_interface::{Cache, UserInterface};
        let mut renderer =
            <iced::Renderer as Headless>::new(Font::DEFAULT, Pixels(16.), Some("tiny-skia"))
                .now_or_never()
                .flatten()
                .unwrap();
        let mut ui = UserInterface::build(
            a.view(),
            Size::new(w as f32, h as f32),
            Cache::default(),
            &mut renderer,
        );
        ui.draw(
            &mut renderer,
            &Theme::Light,
            &renderer::Style {
                text_color: Color::BLACK,
            },
            mouse::Cursor::Unavailable,
        );
        let rgba = renderer.screenshot(Size::new(w, h), 1., Color::WHITE);
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
    /// The Steps page at 1280 × 800, one PNG per kind of step, in
    /// /tmp/manual-steps.
    #[test]
    #[ignore]
    fn render_the_steps() {
        let dir = std::path::Path::new("/tmp/manual-steps");
        std::fs::create_dir_all(dir).unwrap();
        let mut a = app("Yes");
        let steps = a.guide_at().unwrap().0.guide.steps.clone();
        let first = |name: &str| {
            (steps.iter())
                .position(|s| format!("{s:?}").starts_with(name))
                .unwrap()
        };
        let shots = [
            ("01-hold", first("Hold"), 0),
            ("02-zero", first("Zero"), 1),
            ("03-face", first("Face"), 0),
            ("05-tool", first("Tool"), 0),
            ("06-pass", first("Pass") + 1, 1),
            ("07-holes", first("Holes"), 1),
            ("08-check", first("Check"), 0),
            ("09-handover", first("HandOver"), 0),
        ];
        for (name, step, item) in shots {
            at(&mut a, step, item);
            render(&a, (1280, 800), &dir.join(format!("{name}.png")));
        }
        let a = squared("Yes");
        render(&a, (1280, 800), &dir.join("04-square.png"));
        // In a vise on the manual mill: the jaws in the pictures.
        let mut a = app_with("Yes", crate::guide::tests::in_the_vise);
        let shown = a.guide_at().unwrap().0;
        let Step::Hold { stock, .. } = shown.guide.steps[0] else {
            panic!()
        };
        let held = Gripped::whole(stock.translated(shown.guide.zero));
        let found = a.doc.holding_boxes(&a.doc.setups[0], held);
        assert!(
            !shown.jaws.is_empty(),
            "no jaws: {found:?} {:?}",
            a.doc.setups[0].p
        );
        at(&mut a, 0, 0);
        render(&a, (1280, 800), &dir.join("10-hold-vise.png"));
        at(&mut a, 5, 2);
        render(&a, (1280, 800), &dir.join("11-pass-vise.png"));
        // Behind the part, closed on −Y: the same tool, other sides.
        let mut a = app_with("Yes", |doc| {
            let mill = doc.tools[3].id.clone();
            let b = [-500., 500., 104.5, 500., 0., -12.];
            let back = crate::guide::tests::boxed("Back step", &mill, b, &[("stepdown", "5")]);
            doc.setups[0].operations.insert(2, back);
        });
        let comp = (a.guide_at().unwrap().0.guide.steps.iter())
            .position(|s| matches!(s, Step::Comp { .. }))
            .unwrap();
        at(&mut a, comp, 0);
        render(&a, (1280, 800), &dir.join("12-comp.png"));
        // A step closed at both X ends: a plunge, then feeds across.
        let mut a = app_with("Yes", crate::guide::tests::close_the_step);
        let first = (a.guide_at().unwrap().0.guide.steps.iter())
            .position(|s| matches!(s, Step::Pass { .. }))
            .unwrap();
        at(&mut a, first, 3);
        render(&a, (1280, 800), &dir.join("13-pass-plunge.png"));
        at(&mut a, first + 1, 2);
        render(&a, (1280, 800), &dir.join("14-pass-across.png"));
    }

    // ---- The lathe ----

    /// The lathe guide's sample job (`guide::tests::sample`: a sleeve in a
    /// Ø50.2 bar, faced, turned to Ø40, drilled, grooved, threaded and
    /// parted off) with its cross dial reading `reads` ("" = not known),
    /// `edit` made to it, on MACHINE with its guide built.
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
        });
        doc.active = doc.setups[0].id.clone();
        edit(&mut doc);
        a.doc = doc;
        a.imported_mesh = Some(mesh);
        let _ = a.update(Message::Page(Page::Control));
        build(&mut a);
        a
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
                "stop at 3 t + 4.0",
                "6.00",
                "1 turn + 2.00 on the dial",
                "3 t + 4.0",
                "◀ feed off at 3 t + 3.0",
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
                "blade's left side at 2 t + 5.0",
                "cross slide · 1 turn +",
                "in 4.00 → Ø36.00",
                "from the touch on Ø40.00",
                "lock the carriage",
            ],
        );
        let thread = nth(&a, 2, |l| matches!(l, L::ThreadPass { .. }));
        send(&mut a, GuideAction::Go(thread - 3));
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
                "half-nuts",
                "close",
                "back to its zero",
                "8 passes",
            ],
        );
        assert!(!has(&a, "try the nut, then a spring pass"));
        let last = nth(&a, 7, |l| matches!(l, L::ThreadPass { .. }));
        send(&mut a, GuideAction::Go(last));
        shows(
            &a,
            &[
                "Thread pass 8 of 8",
                "pass 8: in to 1.04",
                "0.905 toward the axis",
                "try the nut, then a spring pass",
            ],
        );
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
                "2 t out, 2.20",
                "2 turns out",
                "reads 2.20",
            ],
        );
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
    #[test]
    fn a_stop_on_the_pictures_is_the_number_on_the_carriage_dial() {
        // Faced 1 mm: the shoulder at Z −65 is 64 from the new end, where
        // the carriage dial was zeroed, and the dial stops at 3 t + 4.0 = 64.
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
        // The thread to Z −40 stops 39 from the new end: 1 t + 19.0.
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
        shows(&a, &["stop at 3 t + 4.0"]);
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
                doc.add_lathe_setup().unwrap();
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
        // Only inside tools: the Zero touches the end and measures the bar.
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
}
