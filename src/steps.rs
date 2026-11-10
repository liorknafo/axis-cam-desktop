//! The Steps page: a setup on the manual mill shown screen by screen
//! (`guide`), as pictures and the large numbers its DRO should read; a
//! setup on the manual lathe as side views and dial pictures (`turning`).
mod turning;
use crate::{
    App, GuideAction, Message,
    guide::{Guide, Limits, Pattern, Section, Step, Target},
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
    /// By picture, the sizes drawn on it (the Check screen's), in its
    /// pixels.
    dims: Vec<Vec<Dim>>,
    /// By step, a Tool step's picture: the stock before its operation with
    /// the tool to scale on the spot to touch (`viewport::tool_triangles`).
    tools: Vec<Option<image::Handle>>,
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
            dims: self.dims.clone(),
            tools: self.tools.clone(),
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
    let held = if vise.is_empty() {
        pictures[0].clone()
    } else {
        let tall = solid(&[&vise[..], &parallels].concat());
        picture(0, &tall)
    };
    // The sizes each operation leaves, on the picture after it, placed on
    // the triangles that picture is made of.
    let jaw = solid(&jaws);
    let mut dims = vec![vec![]];
    for (j, d) in guide.done.iter().enumerate() {
        let measures = measures(doc, &guide, d, boxes[j].max[2] + zero[2]);
        let ends: Vec<[f32; 3]> = (measures.iter())
            .flat_map(|m| {
                let [ea, eb] = m.ext.unwrap_or([m.from, m.to]);
                [m.from, m.to, ea, eb]
            })
            .collect();
        let mut seen = stock[j + 1].clone();
        seen.extend(jaw.iter().copied());
        let points = viewport::picture_points(&seen, PICTURE, &ends);
        dims.push(
            (measures.iter().zip(points.chunks(4)))
                .map(|(m, p)| Dim {
                    a: p[0],
                    b: p[1],
                    ext: m.ext.map(|_| [p[2], p[3]]),
                    label: format!("{} {}", m.axis, mm(m.value)),
                    leader: m.leader,
                })
                .collect(),
        );
    }
    // A new tool on the stock before its operation, standing on the
    // highest top left (LIO-376).
    let tools = (guide.steps.iter())
        .map(|step| {
            let Step::Tool {
                op,
                top: Some(_),
                diameter,
                tool,
                ..
            } = step
            else {
                return None;
            };
            let j = guide.done.iter().position(|d| d.op == *op)?;
            let t = doc.tools.iter().find(|t| &t.id == tool);
            let length = |k: &str| t.and_then(|t| num(&t.p, k)).filter(|v| *v > 0.);
            let flutes = length("length").unwrap_or(3. * diameter);
            let stickout = length("stickout").unwrap_or(flutes + diameter);
            let point = t.is_some_and(|t| get(&t.p, "type").to_lowercase().contains("drill"));
            let centre = t.and_then(|t| t.centre_drill());
            let spot = touch_spot(&stock[j])?;
            let mut triangles = stock[j].clone();
            triangles.extend(viewport::tool_triangles(
                spot, *diameter, flutes, stickout, point, centre,
            ));
            triangles.extend(jaw.iter().copied());
            viewport::stock_picture(&triangles, TOOL_PICTURE)
        })
        .collect();
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
        dims,
        tools,
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

/// A size the Check screen gives to measure (LIO-370): `value` mm along
/// `axis` ("X", "Y", "Z" or "Ø"), said by `words` as a caliper or depth
/// gauge reads it, and drawn on the picture from `from` to `to` (stock
/// frame, mm): a dimension line, off the stock, with extension lines from
/// the feature's points `ext` (LIO-379), or with `leader` an arrow onto
/// `from`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Measure {
    pub axis: &'static str,
    pub value: f32,
    pub words: Vec<String>,
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub ext: Option<[[f32; 3]; 2]>,
    pub leader: bool,
}
/// The sizes operation `op` of the guide (its `done` entry `d`) was to
/// leave, as it leaves them, with the stock to leave in them; `top` is the
/// Z of the stock's top before it (stock frame). A Box: its width from the
/// face it is measured from, or between its walls, on each axis it has a
/// wall on, and its depth below that top; holes: how many, their
/// diameter and their depth from the mouth.
pub(crate) fn measures(
    doc: &Document,
    g: &Guide,
    d: &crate::guide::Done,
    top: f32,
) -> Vec<Measure> {
    let Some(op) = doc.setups.get(g.setup).and_then(|s| s.operations.get(d.op)) else {
        return vec![];
    };
    let (zero, s) = (g.zero, d.plan.stock);
    let mut out = vec![];
    match op.strategy {
        Strategy::Box => {
            let Ok((low, high)) = crate::cam::box_block(op, zero) else {
                return out;
            };
            let allowance = crate::cam::number(op, "allowance").unwrap_or(0.);
            let open = crate::cam::open_sides(low, high, &s);
            let (lo, hi) = crate::cam::box_cut(low, high, &s, allowance);
            let faces = [["left", "right"], ["front", "back"]];
            let mut wall = None;
            for k in 0..2 {
                let o = 1 - k;
                let from = match open[k] {
                    [true, true] => continue,
                    [true, false] => format!("from the {} face", faces[k][0]),
                    [false, true] => format!("from the {} face", faces[k][1]),
                    [false, false] => "between the walls".into(),
                };
                // Off the side of the stock the pictures see, clear of it:
                // in front (−Y) for an X width, right (+X) for a Y width.
                let mut a = [0., 0., top];
                a[o] = if o == 0 {
                    s.max[0] + OFF
                } else {
                    s.min[1] - OFF
                };
                let mut b = a;
                (a[k], b[k]) = (lo[k], hi[k]);
                // From the cut's end on that side: a wall's top, or an open
                // side's face where the floor meets it.
                let mut ext = [a, b];
                for (end, e) in ext.iter_mut().enumerate() {
                    e[o] = if o == 0 { hi[0] } else { lo[1] };
                    e[2] = if open[k][end] { lo[2] } else { top };
                }
                // The depth beside it, a further step off, so that the two
                // do not meet at the wall's corner.
                let (mut at, edge) = if open[k][0] { (b, ext[1]) } else { (a, ext[0]) };
                at[o] += if o == 0 { OFF } else { -OFF };
                wall.get_or_insert((at, edge));
                out.push(Measure {
                    axis: AXES[k],
                    value: hi[k] - lo[k],
                    words: vec!["wide".into(), from],
                    from: a,
                    to: b,
                    ext: Some(ext),
                    leader: false,
                });
            }
            // Down from where that width meets the wall, beside the wall.
            let (at, edge) = wall.unwrap_or({
                let at = [s.max[0] + OFF, s.min[1] - OFF, top];
                (at, [hi[0], lo[1], top])
            });
            let words = if open[2][0] {
                vec!["through".into()]
            } else {
                vec![
                    "deep below the top".into(),
                    format!("floor Z {}", dro(lo[2] - zero[2])),
                ]
            };
            out.push(Measure {
                axis: "Z",
                value: top - lo[2],
                words,
                from: at,
                to: [at[0], at[1], lo[2]],
                ext: Some([edge, [edge[0], edge[1], lo[2]]]),
                leader: false,
            });
        }
        // A thread is tried, not measured (LIO-413).
        Strategy::Drill if op.tapping() => {}
        Strategy::Drill => {
            let Some((holes, mouths)) = g.steps.iter().find_map(|st| match st {
                Step::Holes {
                    op: o,
                    holes,
                    mouths,
                    ..
                } if *o == d.op => Some((holes, mouths)),
                Step::Bore {
                    op: o,
                    holes,
                    mouths,
                    ..
                } if *o == d.op => Some((holes, mouths)),
                _ => None,
            }) else {
                return out;
            };
            let Some(h) = holes.first() else {
                return out;
            };
            let mouth = mouths.first().copied().unwrap_or(top - zero[2]);
            let at = [h[0] + zero[0], h[1] + zero[1], mouth + zero[2]];
            let up = [at[0], at[1], at[2] + 2. * d.plan.radius.max(6.)];
            let count = if holes.len() == 1 {
                "1 hole".to_string()
            } else {
                format!("{} holes", holes.len())
            };
            out.push(Measure {
                axis: "Ø",
                value: 2. * d.plan.radius,
                words: vec![count],
                from: at,
                to: up,
                ext: None,
                leader: true,
            });
            let deep: Vec<f32> = (holes.iter().zip(mouths)).map(|(h, m)| m - h[2]).collect();
            let (least, most) = deep
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
            let mut words = vec!["deep from the mouth".to_string()];
            if most - least > 0.005 {
                words.push(format!("{}–{} by hole", mm(least), mm(most)));
            }
            out.push(Measure {
                axis: "Z",
                value: most,
                words,
                from: at,
                to: [at[0], at[1], at[2] - deep[0]],
                ext: None,
                leader: false,
            });
        }
        _ => {}
    }
    out
}
/// How far off the stock a size is drawn, mm.
const OFF: f32 = 16.;
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
                crate::fault::canvas(Lock).width(10).height(12),
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
    /// A hole: its centre, radius, number and whether done, now or next.
    Hole([f32; 3], f32, usize, Lit),
    /// A pattern the DRO leads to, its holes (centres, the DRO's order) and
    /// their radius, in the top view: the holes numbered and their order,
    /// hole 1 marked, the pitches or the circle with its centre and first
    /// angle.
    Pattern(Pattern, Vec<[f32; 2]>, f32, Vec<usize>),
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
                    write(f, s.clone(), depth_label(m, n), NEVER, 12., Align::Left);
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
                Shape::Pattern(pattern, holes, r, numbers) => {
                    paint_pattern(f, p, s, pattern, holes, *r, numbers)
                }
                Shape::Word(q, s, c) => write(f, s.clone(), p3(*q), *c, 13., Align::Left),
                Shape::Round(q, r, label) => {
                    let at = p3(*q);
                    f.stroke(&canvas::Path::circle(at, r * s), stroke(ACCENT, 2.));
                    // On its back-right, off the finders' lines.
                    let d = r * s * std::f32::consts::FRAC_1_SQRT_2;
                    write(
                        f,
                        label.clone(),
                        Point::new(at.x + d + 4., at.y - d - 14.),
                        ACCENT,
                        13.,
                        Align::Left,
                    );
                }
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
fn under_stock<'a>(tip: f32, stock: &Bounds) -> Option<Element<'a, Message>> {
    let under = stock.min[2] - tip;
    (under > 0.005).then(|| {
        warn_chip(format!(
            "comes out {} under the stock: nothing under the hole",
            mm(under)
        ))
    })
}
/// The Check of a tapping `op`: try a screw, as a thread is not measured
/// (LIO-413).
fn screw_try<'a>(p: &Page<'a>, op: usize) -> Option<Element<'a, Message>> {
    let tool = p.tool(op)?;
    let spec = tool.tap_spec()?;
    let (tip, mouth) = p.guide().steps.iter().find_map(|s| match s {
        Step::Holes {
            op: o,
            holes,
            mouths,
            ..
        } if *o == op => Some((holes.first()?[2], *mouths.first()?)),
        _ => None,
    })?;
    let through = tip < part_bottom(p) - 0.005;
    let thread = (mouth - tip) - crate::cam::tap_lead(tool) * spec.pitch;
    let goes = if through {
        "it goes through by hand".to_string()
    } else {
        format!("it goes in {} by hand", mm(thread))
    };
    Some(readout(
        Readout {
            order: None,
            lead: Some("try".into()),
            axis: String::new(),
            value: format!("{} screw", spec.name),
            lock: None,
            tags: vec![goes],
            lit: Lit::Next,
            big: true,
        },
        None,
    ))
}
/// `Shape::Pattern` in the top view, `at` from mm to the canvas at `s` px
/// a mm: the order the DRO goes in as arrows from hole to hole (dashed onto
/// the next row), the pitches as sizes off hole 1, or the circle with its
/// centre, its diameter and the angle from +X round to hole 1; then the
/// holes numbered, hole 1 in the accent.
fn paint_pattern(
    f: &mut canvas::Frame,
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
    // A label on a white tag, `from` a point out along `out`, clear of it.
    let tag = |f: &mut canvas::Frame, from: Point, out: [f32; 2], label: String| {
        let place = tag_box(from, out, label.chars().count(), k, f.size());
        let c = place.center();
        let r = canvas::Path::rounded_rectangle(place.position(), place.size(), 5.into());
        f.fill(&r, Color::WHITE);
        f.stroke(&r, stroke(ACCENT, 1.5));
        write(f, label, c, ACCENT, 16. * k, Align::Center);
    };
    // A size from `a` to `b`, `gap` px off the holes along `out`.
    let size = |f: &mut canvas::Frame, a: Point, b: Point, out: [f32; 2], label: String| {
        let gap = rs + 22.;
        let (m, n) = (on(a, out, gap), on(b, out, gap));
        for (h, e) in [(a, m), (b, n)] {
            let tick = canvas::Path::line(on(h, out, rs + 4.), on(e, out, 6.));
            f.stroke(&tick, stroke(Color { a: 0.7, ..ACCENT }, 1.2));
        }
        let mid = Point::new((m.x + n.x) / 2., (m.y + n.y) / 2.);
        arrow(f, mid, m, ACCENT, 2.);
        arrow(f, mid, n, ACCENT, 2.);
        tag(f, mid, out, label);
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
                    pts[0],
                    pts[1],
                    back(ub),
                    format!("pitch {}", dro(pitch[0])),
                );
            }
            if rows > 1 {
                // Hole 1's neighbour in the second row, which runs back.
                let to = pts[2 * columns - 1];
                size(f, pts[0], to, back(ua), format!("rows {}", dro(pitch[1])));
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
            write(f, "centre", under, ACCENT, 15. * k, Align::Right);
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
        write(f, number.to_string(), n, ink, size * k, Align::Center);
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

/// A new tool picks up Z: touch the top and set Z to `top`; nothing for a
/// tool already in.
fn pick_up<'a>(top: Option<f32>, ringed: bool) -> Vec<Element<'a, Message>> {
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
                tags: match ringed {
                    true => vec!["where it is ringed".into()],
                    false => vec![],
                },
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
            format!("Open {name} in MANUFACTURE ▶"),
            GuideAction::Fix(Some(i), rest.to_string()),
        ),
        None => (
            "Open the setup in MANUFACTURE ▶".to_string(),
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
    let mut c = column![bold(title, 16), muted("Setups")].spacing(8);
    for (si, s) in app.doc.setups.iter().enumerate() {
        if !manual::is_hand(&s.machine) {
            continue;
        }
        let on = s.id == app.doc.active;
        let msg = Message::Guide(GuideAction::Open(si));
        // The machine under the name only when the name does not say it.
        let named = s.name().starts_with(s.machine.name());
        let b = button(
            column![bold(s.name(), 13)]
                .push((!named).then(|| muted(s.machine.name())))
                .spacing(2),
        )
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
        Step::Bore {
            index, of, holes, ..
        } if holes.len() > 1 => format!("pass {index}/{of} · hole {}/{}", item + 1, holes.len()),
        Step::Bore { index, of, .. } => format!("pass {index}/{of}"),
        _ if s.steps.len() > 1 => format!("{}/{}", at - s.steps.start + 1, s.steps.len()),
        _ => String::new(),
    }
}
/// What a section's count counts: "4 passes", "1 hole". A Boring's count
/// is its passes.
fn counted(setup: &Setup, s: &Section) -> String {
    let hole =
        s.op.and_then(|op| setup.operations.get(op))
            .is_some_and(|o| o.strategy == Strategy::Drill && !o.boring());
    match (s.count, hole) {
        (1, true) => "1 hole".into(),
        (1, false) => "1 pass".into(),
        (n, true) => format!("{n} holes"),
        (n, false) => format!("{n} passes"),
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
        // Numbered from 1 (LIO-377).
        let mark = if done {
            "✓".to_string()
        } else {
            (i + 1).to_string()
        };
        let mut face = row![disc(mark, now, 20.), text(section_name(app, g, s)).size(13)]
            .spacing(6)
            .align_y(Alignment::Center);
        let small = if now {
            progress(g, s, at, app.guide_item)
        } else if s.count > 0 && !done {
            counted(setup, s)
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
        disc((here + 1).to_string(), true, 26.),
        bold(section_name(app, g, s), 17)
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    let mut tags = vec![];
    let lathe = crate::lathe::is_lathe(&setup.machine);
    if lathe && let Some(op) = s.op {
        tags.extend(turning::header(&page, op, s));
    } else if let Some(op) = s.op
        && setup.operations.get(op).is_some()
    {
        let d = num(
            &page.tool(op).map_or(Default::default(), |t| t.p.clone()),
            "diameter",
        );
        if let Some(d) = d {
            tags.push(chip(page.tool_name(op, d)));
        }
        if s.count > 0 {
            tags.push(chip(counted(setup, s)));
        }
        tags.push(chip(format!("~{} min", s.minutes.ceil())));
    }
    header = header
        .push(row(tags).spacing(6))
        .push(space().width(Fill))
        .push(bold(progress(g, s, at, app.guide_item), 14).color(ACCENT));
    let top = row![
        // A setup named as its machine says it once (LIO-382).
        bold(
            if setup.name().starts_with(setup.machine.name()) {
                format!("{} · steps", setup.name())
            } else {
                format!("{} on the {}", setup.name(), setup.machine.name())
            },
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
        container(screen(&page)).height(Fill)
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
        Step::Holes { holes, .. } | Step::Bore { holes, .. } if item + 1 < holes.len() => {
            "Hole done ▶".into()
        }
        Step::Bore { index, of, .. } if index < of => "Pass done ▶".into(),
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
                p.held_picture(FillPortion(3)),
                p.drawing_with(view, *stock, shapes, FillPortion(2), &p.shown.vise),
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
            // The caption follows the holding (LIO-375).
            let sub = match (vise, get(&p.setup.p, "holding")) {
                (Some(_), _) => "where it goes in the vise",
                (None, "Clamps") => "where the clamps hold it",
                _ => "the stock as it starts",
            };
            ("Hold".into(), sub, left, right)
        }
        Step::Zero {
            approach,
            touch,
            finder,
            at,
            round,
            ..
        } => {
            let stock = p.shown.boxes[0];
            let sides = [["left", "right"], ["front", "back"]];
            let side = |k: usize| sides[k][usize::from(approach[k] > 0)];
            // How far under the top each face is touched: 0 at the top.
            let under = |k: usize| {
                let d: f32 = stock.max[2] - at[k][2];
                if d > 0.5 { d } else { 0. }
            };
            // The work Z 0 is on the top, or the top is at a height.
            let top = touch[2].abs() < 0.005;
            let mut shapes = vec![];
            // A round touched on X and Y: the diameter the finder meets,
            // its centre where their lines cross.
            if round[0] && round[1] {
                let centre = [at[1][0], at[0][1], at[0][2]];
                let r = (at[0][0] - centre[0]).abs();
                shapes.push(Shape::Round(centre, r, format!("Ø{} touched", mm(2. * r))));
            }
            for k in 0..2 {
                let at = at[k];
                let mut from = [0.; 2];
                from[k] = f32::from(approach[k].signum());
                let set = format!("{} {}", AXES[k], dro(touch[k]));
                shapes.push(Shape::Finder(at, from, set));
            }
            // Off the middle lines, where a round's finders come in.
            shapes.push(Shape::Word(
                [
                    (stock.min[0] + stock.max[0]) / 2. - 12.,
                    stock.min[1] + (stock.max[1] - stock.min[1]) * 0.75,
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
                } else if under(k) > 0. {
                    format!("touch {} face {} under top → set", side(k), mm(under(k)))
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
            let mut c = vec![
                chip(format!("Ø{} edge finder", mm(*finder))),
                warn_chip("DRO comp off"),
            ];
            // On a round the finder trips farthest out on its centre line,
            // where the drawing puts it; off it the DRO reads short.
            if round.iter().any(|r| *r) {
                c.push(warn_chip("round: touch on its centre line"));
            }
            let right = column![chips(c), label("Touch, then set the DRO"),].extend(rows);
            (
                "Zero".into(),
                "pick up the corner",
                p.drawing(0, stock, shapes, Fill),
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
            // Set the parallels before drilling starts (LIO-417).
            let tip = (g.steps.iter())
                .filter_map(|s| match s {
                    Step::Holes { op: o, holes, .. } if o == op => {
                        holes.iter().map(|h| h[2]).reduce(f32::min)
                    }
                    _ => None,
                })
                .reduce(f32::min);
            let under = tip.and_then(|tip| under_stock(tip, &p.before(*op)));
            let right = column![chips(
                vec![
                    chip(p.named(tool, *diameter)),
                    chip(format!("{} rpm", rpm.round()))
                ]
                .into_iter()
                .chain(under)
                .collect()
            ),]
            .extend(pick_up(*top, true))
            .extend(
                comp.map(|c| dro_tool(*diameter, c, 110.))
                    .unwrap_or_default(),
            );
            // The stock as it is with the tool to scale on the spot to
            // touch, ringed (LIO-376), filling the panel with the gear in its
            // corner (LIO-379: beside the gear it was a third as wide).
            let picture = p.shown.tools.get(p.at).cloned().flatten();
            let left = match picture {
                Some(_) => iced::widget::stack![
                    Page::image(picture, Fill),
                    container(gear(*rpm)).align_left(Fill).align_top(Fill)
                ]
                .into(),
                None => container(gear(*rpm)).center(Fill).into(),
            };
            match top {
                Some(_) => ("Tool".into(), "put it in, set the gear", left, right),
                None => ("Gear".into(), "same tool, set the gear", left, right),
            }
        }
        // A hand tap: put it in, the spindle off and free; no gear and no
        // Z pick-up (LIO-410).
        Step::TapIn { op, tool } => {
            let d = p
                .tool(*op)
                .and_then(|t| num(&t.p, "diameter"))
                .unwrap_or(0.);
            let row = |n: u8, axis: &str, value: &str, tags: Vec<String>| {
                readout(
                    Readout {
                        order: Some(n),
                        lead: None,
                        axis: axis.into(),
                        value: value.into(),
                        lock: None,
                        tags,
                        lit: lit(usize::from(n) - 1, p.item),
                        big: false,
                    },
                    Some(Message::Guide(GuideAction::Reached(usize::from(n) - 1))),
                )
            };
            let right = column![
                chips(vec![chip(p.named(tool, d)), warn_chip("spindle off")]),
                label("Put the tap in"),
                row(
                    1,
                    "spindle",
                    "off",
                    vec![
                        "neutral, or the lowest gear".into(),
                        "free to turn by hand".into()
                    ]
                ),
                row(
                    2,
                    "tap",
                    "in the chuck",
                    vec![
                        "or in a wrench, a centre in the chuck".into(),
                        "in its back end to keep it square".into()
                    ]
                ),
            ];
            let left = p.picture(p.done(*op), Fill);
            ("Tap".into(), "put it in", left, right)
        }
        Step::Comp { op, comp, diameter } => {
            let right = column![
                chips(vec![chip(p.tool(*op).map_or("", |t| t.name()).to_string())]),
                label("Enter in the DRO"),
                readout(
                    Readout {
                        order: None,
                        lead: None,
                        axis: "Ø".into(),
                        value: dro(*diameter),
                        lock: None,
                        tags: vec!["tool diameter".into()],
                        lit: Lit::Now,
                        big: false,
                    },
                    None,
                ),
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
            let left = crate::fault::canvas(CompPicture(*comp))
                .width(Fill)
                .height(Fill)
                .into();
            (
                "Comp".into(),
                "same tool: its diameter and sides",
                left,
                right,
            )
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
                    FillPortion(3)
                ),
                p.drawing(
                    1,
                    stock,
                    vec![
                        Shape::Hatch(band, Hatch::Cut),
                        never,
                        Shape::Tool([tool[0], 0., *to], r, None)
                    ],
                    FillPortion(2)
                ),
            ]
            .spacing(6)
            .into();
            let right = column![chips(vec![
                chip(p.named(id, *diameter)),
                chip(format!("{} rpm", rpm.round()))
            ]),]
            .extend(pick_up(*top, false))
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
            // The way it feeds along the wall, conventional as its plan
            // cuts (LIO-176).
            let feed = feed_along(g, p.done(*op), o);
            let along = (feed != 0.).then(|| {
                let mut end = tool;
                end[o] += feed * (stock.max[o] - stock.min[o]) * 0.35;
                end
            });
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
                        Shape::Tool(tool, r, along),
                        Shape::Word(word, format!("{} mm off", mm(*remove)), ACCENT)
                    ],
                    FillPortion(3)
                ),
                p.drawing(
                    view,
                    stock,
                    vec![
                        Shape::Hatch(slab, Hatch::Cut),
                        never,
                        Shape::Tool(tool_side, r, None)
                    ],
                    FillPortion(2)
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
            .extend(pick_up(*top, false))
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
        Step::Pattern { op, pattern, same } => pattern_screen(p, *op, pattern, *same),
        Step::Holes {
            op,
            holes,
            mouths,
            uncentred,
            pattern,
            numbers,
        } => {
            // The DRO's number of each hole, drilled in this order (LIO-406).
            let number = |n: usize| numbers.get(n).copied().unwrap_or(n + 1);
            let stock = p.before(*op);
            let i = p.item.min(holes.len().saturating_sub(1));
            let r = p
                .tool(*op)
                .and_then(|t| num(&t.p, "diameter"))
                .unwrap_or(4.)
                / 2.;
            let mut shapes = p.cut_before(*op);
            let spotting = p.tool(*op).and_then(|t| t.centre_drill());
            // A centre drill's hole is its cone at hole `n`'s mouth, where
            // the material over it starts (a pocket's floor, say).
            let cone = |n: usize, h: &[f32; 3]| {
                spotting.map_or(r, |c| {
                    let mouth = mouths.get(n).copied().unwrap_or(stock.max[2]);
                    (c.pilot / 2.
                        + (mouth - h[2] - c.pilot_length) / crate::model::CentreDrill::rise())
                    .clamp(c.pilot / 2., c.body / 2.)
                })
            };
            shapes.extend(holes.iter().enumerate().map(|(n, h)| {
                Shape::Hole([h[0], h[1], stock.max[2]], cone(n, h), number(n), lit(n, i))
            }));
            let h = holes.get(i).copied().unwrap_or([0.; 3]);
            // An axis where the hole before left it stays, locked, and is no
            // step: the steps are what moves (LIO-405). Hole 1 moves both.
            let before = i.checked_sub(1).and_then(|j| holes.get(j));
            let stays = |k: usize| before.is_some_and(|b| (b[k] - h[k]).abs() <= 0.005);
            let moving: Vec<usize> = (0..2).filter(|&k| !stays(k)).collect();
            // The depth's step, after the axes that move.
            let z = moving.len();
            // X and Y: the centre, or 0 on the DRO's hole of a pattern with
            // the centre small beside it (LIO-399).
            let xy = |k: usize| {
                if stays(k) {
                    return readout(
                        Readout {
                            order: None,
                            lead: None,
                            axis: AXES[k].into(),
                            value: "stays".into(),
                            lock: None,
                            tags: vec!["leave it locked".into()],
                            lit: Lit::Done,
                            big: false,
                        },
                        None,
                    );
                }
                let n = moving.iter().position(|&m| m == k).unwrap_or(k);
                let (value, tags) = match pattern {
                    Some(_) => (
                        dro(0.),
                        vec![
                            format!("on the DRO's hole {}", number(i)),
                            format!("{}\u{a0}{}", AXES[k], dro(h[k])),
                        ],
                    ),
                    None => (dro(h[k]), vec!["centre".into()]),
                };
                readout(
                    Readout {
                        order: Some(n as u8 + 1),
                        lead: None,
                        axis: AXES[k].into(),
                        value,
                        lock: Some(AXES[k].into()),
                        tags,
                        lit: lit(n, p.app.guide_reached),
                        big: false,
                    },
                    Some(Message::Guide(GuideAction::Reached(n))),
                )
            };
            let tapping = p.setup.operations.get(*op).is_some_and(|o| o.tapping());
            // Every hole is cut from its mouth: the stock above it may be
            // gone already (LIO-365).
            let mouth = mouths.get(i).copied().unwrap_or(stock.max[2]);
            let drilled = Bounds {
                min: [h[0] - r, h[1] - r, h[2]],
                max: [h[0] + r, h[1] + r, mouth],
            };
            // A centre drill's pilot, to its tip, under its cone, which is
            // as wide as the hole at the top or as its body when wider.
            let spot = p.tool(*op).and_then(|t| t.centre_drill()).map(|c| {
                let (x, y, (q, b)) = (h[0], h[1], (c.pilot / 2., c.body / 2.));
                let shoulder = h[2] + c.pilot_length;
                let wide = (q + (mouth - shoulder) / crate::model::CentreDrill::rise()).min(b);
                let rim = shoulder + (wide - q) * crate::model::CentreDrill::rise();
                Shape::Section(vec![
                    [x - q, y, h[2]],
                    [x + q, y, h[2]],
                    [x + q, y, shoulder],
                    [x + wide, y, rim],
                    [x + wide, y, mouth],
                    [x - wide, y, mouth],
                    [x - wide, y, rim],
                    [x - q, y, shoulder],
                ])
            });
            let side = vec![
                spot.unwrap_or(Shape::Hatch(drilled, Hatch::Cut)),
                Shape::Depth(
                    [h[0] - 3. * r, 0., h[2]],
                    [h[0] + 3. * r, 0., h[2]],
                    format!("Z {}", dro(h[2])),
                ),
            ];
            let left = column![
                p.drawing(0, stock, shapes, FillPortion(3)),
                p.drawing(1, stock, side, FillPortion(2))
            ]
            .spacing(6)
            .into();
            let rpm = p
                .setup
                .operations
                .get(*op)
                .and_then(|o| num(&o.p, "rpm"))
                .unwrap_or(0.);
            let depth = mouth - h[2];
            // Tapping: by hand, the spindle holding the tap square; the
            // depth is how many turns the tap goes in.
            let pitch = p.tool(*op).and_then(|t| t.tap_spec()).map(|s| s.pitch);
            if let (true, Some(pitch)) = (tapping, pitch) {
                // Rounded down: a turn more would go past the drilled hole.
                let turns = (depth / pitch * 10.).floor() / 10.;
                let mut said = vec![
                    chip(p.tool_name(*op, 2. * r)),
                    chip("by hand · spindle off"),
                    warn_chip("oil"),
                    // Held square as on the lathe (LIO-410).
                    chip("square: chuck, or wrench on a centre"),
                ];
                // The tap's tip comes out under the stock: a parallel there
                // stops it, or it cuts the vise.
                said.extend(under_stock(h[2], &stock));
                // The depth as a hand follows it (LIO-410).
                let through = h[2] < part_bottom(p) - 0.005;
                let how = if through {
                    "through: turn until it runs free".to_string()
                } else {
                    format!("mark the tap {} from its tip", mm(depth))
                };
                let right = column![
                    chips(said),
                    hole_label(i, holes.len(), number(i)),
                    xy(0),
                    xy(1),
                    readout(
                        Readout {
                            order: Some(z as u8 + 1),
                            lead: None,
                            axis: "turns".into(),
                            value: format!("{:.1}", turns),
                            lock: None,
                            tags: vec![how, "½ back every turn".into()],
                            lit: lit(z, p.app.guide_reached),
                            big: true
                        },
                        Some(Message::Guide(GuideAction::Reached(z)))
                    ),
                    tiles(
                        (0..holes.len())
                            .map(|n| (number(n).to_string(), None))
                            .collect(),
                        i
                    ),
                ];
                ("Tap".into(), "one hole at a time, by hand", left, right)
            } else {
                // A reamer is fed back out: pulled out at a rapid, it scores
                // the wall it has just cut.
                let o = p.setup.operations.get(*op);
                let reaming = o.is_some_and(|o| o.reaming());
                // A centre drill goes in to its pilot's tip, its cone opened
                // to the hole's size at the top, the body's when wider.
                let centre = p.tool(*op).and_then(|t| t.centre_drill());
                let feed = o.and_then(|o| num(&o.p, "feed")).unwrap_or(0.);
                let mut said = vec![
                    chip(p.tool_name(*op, 2. * r)),
                    chip(format!("{} rpm", rpm.round())),
                    chip(format!("{} mm/min", feed.round())),
                    warn_chip("centres"),
                ];
                if reaming {
                    said.push(warn_chip("feed back out"));
                }
                if uncentred.get(i).copied().unwrap_or(false) {
                    said.push(warn_chip("centre drill first"));
                }
                said.extend(under_stock(h[2], &stock));
                let cone = centre.map(|_| format!("cone Ø{} at the top", mm(2. * cone(i, &h))));
                // The planned cycle, as the hand on the quill does it.
                let cycle = o.map(|o| get(&o.p, "cycle")).unwrap_or_default();
                let peck = o.and_then(|o| num(&o.p, "peck")).filter(|p| *p > 0.);
                let pecking = match (cycle, peck) {
                    ("Chip breaking", Some(peck)) => {
                        vec![format!("peck {} mm", mm(peck)), "back off 0.5 each".into()]
                    }
                    ("Deep drilling", Some(peck)) => vec![
                        format!("peck {} mm", mm(peck)),
                        "out of the hole each".into(),
                    ],
                    _ => vec![],
                };
                if !pecking.is_empty() {
                    said.push(chip(cycle.to_string()));
                }
                let right = column![
                    chips(said),
                    hole_label(i, holes.len(), number(i)),
                    xy(0),
                    xy(1),
                    readout(
                        Readout {
                            order: Some(z as u8 + 1),
                            lead: None,
                            axis: "Z".into(),
                            value: dro(h[2]),
                            lock: None,
                            tags: [
                                if centre.is_some() {
                                    "pilot tip to"
                                } else if reaming {
                                    "ream to"
                                } else {
                                    "drill to"
                                }
                                .into(),
                                format!("{} deep", mm(depth))
                            ]
                            .into_iter()
                            .chain(cone)
                            .chain(pecking)
                            .collect(),
                            lit: lit(z, p.app.guide_reached),
                            big: false
                        },
                        Some(Message::Guide(GuideAction::Reached(z)))
                    ),
                    tiles(
                        (0..holes.len())
                            .map(|n| (number(n).to_string(), None))
                            .collect(),
                        i
                    ),
                ];
                let title = if centre.is_some() {
                    "Centre drill"
                } else if reaming {
                    "Ream"
                } else {
                    "Holes"
                };
                (title.into(), "one hole at a time", left, right)
            }
        }
        Step::Bore {
            op,
            index,
            of,
            size,
            before,
            dial,
            feed_out,
            holes,
            mouths,
        } => {
            let stock = p.before(*op);
            let i = p.item.min(holes.len().saturating_sub(1));
            let r = size / 2.;
            let mut shapes = p.cut_before(*op);
            shapes.extend(
                holes
                    .iter()
                    .enumerate()
                    .map(|(n, h)| Shape::Hole([h[0], h[1], stock.max[2]], r, n + 1, lit(n, i))),
            );
            let h = holes.get(i).copied().unwrap_or([0.; 3]);
            let mouth = mouths.get(i).copied().unwrap_or(stock.max[2]);
            // The bore as this pass leaves it, over what the last one left.
            let ring = |r: f32| Bounds {
                min: [h[0] - r, h[1] - r, h[2]],
                max: [h[0] + r, h[1] + r, mouth],
            };
            let mut side = vec![Shape::Hatch(ring(r), Hatch::Cut)];
            if let Some(b) = before {
                side.push(Shape::Hatch(ring(b / 2.), Hatch::Earlier));
            }
            side.push(Shape::Never(
                [h[0] - 2. * r, 0., h[2]],
                [h[0] + 2. * r, 0., h[2]],
                format!("Z {}", dro(h[2])),
            ));
            let left = column![
                p.drawing(0, stock, shapes, FillPortion(3)),
                p.drawing(1, stock, side, FillPortion(2))
            ]
            .spacing(6)
            .into();
            let rpm = p
                .setup
                .operations
                .get(*op)
                .and_then(|o| num(&o.p, "rpm"))
                .unwrap_or(0.);
            let first = i == 0;
            let mut said = vec![
                chip(p.tool_name(*op, *size)),
                chip(format!("{} rpm", rpm.round())),
            ];
            if first {
                said.push(warn_chip("spindle off to set the head"));
            }
            said.push(if *feed_out {
                chip("feed back out")
            } else {
                warn_chip("spindle off, then raise")
            });
            let mut order = 0u8;
            let mut next = || {
                order += 1;
                Some(order)
            };
            let mut right = column![chips(said), label(format!("Pass {index} of {of}"))];
            // Before the pass: measure what the last left, then set the
            // head out by the difference; before the first, set it.
            if first && let Some(b) = before {
                right = right.push(readout(
                    Readout {
                        order: next(),
                        lead: Some("measure".into()),
                        axis: "bore".into(),
                        value: format!("Ø{b:.2}"),
                        lock: None,
                        tags: vec!["what the last pass left".into()],
                        lit: Lit::Next,
                        big: false,
                    },
                    None,
                ));
            }
            let out = before.map(|b| size - b);
            let mut head_tags = match out {
                Some(d) if first => vec![format!("out {d:.2} on Ø")],
                Some(_) => vec!["as set".into()],
                None if first => vec!["set to".into()],
                None => vec!["as set".into()],
            };
            if let (true, Some(d)) = (first, out.filter(|_| *dial > 0.)) {
                head_tags.push(format!("{} divisions", (d / dial).round()));
            }
            right = right.push(readout(
                Readout {
                    order: if first { next() } else { None },
                    lead: None,
                    axis: "head".into(),
                    value: format!("Ø{size:.2}"),
                    lock: None,
                    tags: head_tags,
                    lit: if first { Lit::Next } else { Lit::Done },
                    big: first,
                },
                None,
            ));
            for (axis, value, lock, tags) in [
                ("X", h[0], Some("X"), vec!["centre".to_string()]),
                ("Y", h[1], Some("Y"), vec!["centre".to_string()]),
                (
                    "Z",
                    h[2],
                    None,
                    vec!["bore to".to_string(), format!("{} deep", mm(mouth - h[2]))],
                ),
            ] {
                right = right.push(readout(
                    Readout {
                        order: next(),
                        lead: None,
                        axis: axis.into(),
                        value: dro(value),
                        lock: lock.map(String::from),
                        tags,
                        lit: Lit::Next,
                        big: false,
                    },
                    None,
                ));
            }
            if holes.len() > 1 {
                right = right.push(tiles(
                    (1..=holes.len()).map(|n| (n.to_string(), None)).collect(),
                    i,
                ));
            }
            ("Bore".into(), "set the head, then each hole", left, right)
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
            // The sizes to measure, drawn on the picture and listed large
            // (LIO-370).
            let top = p.shown.boxes[j].max[2] + g.zero[2];
            let sizes = g
                .done
                .get(j)
                .map_or(vec![], |d| measures(&p.app.doc, g, d, top));
            let rows = sizes.into_iter().map(|m| {
                readout(
                    Readout {
                        order: None,
                        lead: None,
                        axis: m.axis.into(),
                        value: dro(m.value),
                        lock: None,
                        tags: m.words,
                        lit: Lit::Next,
                        big: true,
                    },
                    None,
                )
            });
            // A tapped hole is tried with a screw (LIO-413).
            let tried = o.filter(|o| o.tapping()).and_then(|_| screw_try(p, *op));
            let right = match tried {
                Some(row) => column![label("Try"), row],
                None => column![label("Measure")].extend(rows),
            }
            .push(chips(c))
            .push(label("Before"))
            .push(p.picture(j, Length::Fixed(160.)));
            let dims = p.shown.dims.get(j + 1).map_or(&[][..], Vec::as_slice);
            let left = iced::widget::stack![
                p.picture(j + 1, Fill),
                crate::fault::canvas(Dims(dims)).width(Fill).height(Fill)
            ]
            .into();
            ("Check".into(), "measure what it left", left, right)
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
            return frame(
                "Hand over".into(),
                "the stock leaves the mill",
                p.picture(last, Fill),
                column![chips(c)],
                next,
                true,
            );
        }
    };
    // The lathe's numbers take half the card at a narrow window (LIO-389).
    let split = match &g.steps[p.at] {
        Step::Lathe(_) => turning::split(p),
        Step::Pattern { .. } if p.app.window_width < 1200. => (1, 1),
        _ => (11, 9),
    };
    framed(title, sub, left, right, next, p.at > 0 || p.item > 0, split)
}
/// A step's screen: its caption, the pictures, which grow with the room
/// the page has, and the numbers, which scroll when they do not fit; Back
/// (live unless `back` is false) and `next` in one place at the card's foot
/// on every screen (LIO-369).
pub(super) fn frame<'a>(
    title: String,
    sub: &str,
    left: Element<'a, Message>,
    right: Column<'a, Message>,
    next: Option<(String, Message)>,
    back: bool,
) -> Element<'a, Message> {
    framed(title, sub, left, right, next, back, (11, 9))
}
/// `frame`, the pictures and the numbers sharing the card's width as `split`.
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
/// Which way (−1, +1; 0 when it does not) the first cut of the plan of
/// the setup's `done`th operation feeds along `axis`.
fn feed_along(g: &Guide, done: usize, axis: usize) -> f32 {
    (g.done.get(done).into_iter())
        .flat_map(|d| d.plan.moves.iter())
        .find(|m| m.cutting_pass)
        .map_or(0., |m| (m.to[axis] - m.from[axis]).signum())
}
/// The lines not to pass of operation `op` (tool radius `r`) where they are
/// inside `stock`, as `view` sees them, each labelled with what the DRO
/// reads with the tool against it (LIO-363): a line is the tool's edge on
/// its low (−1) or high (+1) side, and the DRO reads the edge on the
/// compensation side its targets read, or the centre with it off.
fn never_lines(
    g: &Guide,
    op: usize,
    stock: Bounds,
    never: Limits,
    r: f32,
    view: [usize; 2],
) -> Vec<Shape> {
    let comp: [f32; 2] = std::array::from_fn(|a| {
        (g.steps.iter())
            .filter_map(|s| match s {
                Step::Pass { op: o, targets, .. } if *o == op => Some(targets),
                _ => None,
            })
            .flatten()
            .find(|t| t.axis == a && t.edge)
            .map_or(0., |t| ((t.shown - t.centre) / r).round())
    });
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
        for (v, side) in [(lo, -1.), (hi, 1.)] {
            if v > stock.min[axis] + 0.05 && v < stock.max[axis] - 0.05 {
                let (mut a, mut b) = ([0.; 3], [0.; 3]);
                (a[axis], b[axis]) = (v, v);
                (a[other], b[other]) = (stock.min[other] - 4., stock.max[other] + 4.);
                let reads = match comp.get(axis) {
                    Some(c) => v + (c - side) * r,
                    None => v,
                };
                out.push(Shape::Never(a, b, format!("{} {}", AXES[axis], dro(reads))));
            }
        }
    }
    out
}
/// The most a pass lifts off the floor to return (`cam::LIFT`), with room
/// for rounding.
const LIFT: f32 = crate::cam::LIFT + 0.05;
/// The heading over a hole's readouts: "Hole 2 of 6", and the DRO's hole
/// to call up when that is another number (LIO-407).
fn hole_label<'a>(i: usize, of: usize, called: usize) -> Element<'a, Message> {
    match called == i + 1 {
        true => label(format!("Hole {} of {of}", i + 1)),
        false => label(format!(
            "Hole {} of {of} · call up the DRO's hole {called}",
            i + 1
        )),
    }
}
/// An angle as the DRO takes it: "30.00°".
fn degrees(v: f32) -> String {
    format!("{}°", dro(v))
}
/// The Pattern screen (LIO-399): the numbers to type into the DRO's matrix
/// or arc holes, large, two to a row, each the ways DROs ask for it, and
/// the part from above with the pattern on it.
fn pattern_screen<'a>(
    p: &Page<'a>,
    op: usize,
    pattern: &Pattern,
    same: bool,
) -> (
    String,
    &'static str,
    Element<'a, Message>,
    Column<'a, Message>,
) {
    let (holes, called): (Vec<[f32; 2]>, Vec<usize>) = match p.guide().steps.get(p.at + 1) {
        Some(Step::Holes { holes, numbers, .. }) => (
            holes.iter().map(|h| [h[0], h[1]]).collect(),
            numbers.clone(),
        ),
        _ => (vec![], vec![]),
    };
    // Out of the DRO's order round the circle, one axis a move (LIO-406).
    let out_of_order = called.iter().enumerate().any(|(j, n)| *n != j + 1);
    let one_axis = (holes.windows(2))
        .filter(|w| crate::guide::one_axis(w[0], w[1]))
        .count();
    let r = (p.tool(op).and_then(|t| num(&t.p, "diameter"))).unwrap_or(4.) / 2.;
    // Smaller at a narrow window, where the numbers get half the card.
    let size = if p.app.window_width < 1200. { 30 } else { 40 };
    let mono = Font {
        weight: iced::font::Weight::Bold,
        ..Font::MONOSPACE
    };
    // A number to type: what it is, the number, and the other way to say it.
    let typed = |what: &str, value: String, other: Option<String>| -> Element<'a, Message> {
        let c = column![
            text(what.to_string()).size(14).color(MUTED),
            text(value)
                .size(size)
                .font(mono)
                .color(INK)
                .wrapping(text::Wrapping::None),
        ]
        .push(other.map(|o| text(o).size(13).color(MUTED)));
        boxed(c.spacing(2).padding([6, 12]), false)
            .width(Fill)
            .into()
    };
    // The tool is in the header; the screen keeps its room for the numbers
    // (LIO-404).
    let said = same.then(|| {
        chips(vec![warn_chip(
            "the same pattern: start the DRO again at hole 1",
        )])
    });
    let (title, sub, kind, numbers, then) = match *pattern {
        Pattern::Grid {
            first,
            columns,
            rows,
            pitch,
            angle,
        } => {
            let span =
                |k: usize, n: usize| Some(format!("span {}", dro(pitch[k] * (n - 1) as f32)));
            let mut c = vec![
                typed("first hole X", dro(first[0]), None),
                typed("first hole Y", dro(first[1]), None),
            ];
            if columns > 1 {
                c.push(typed("pitch along a row", dro(pitch[0]), span(0, columns)));
            }
            if rows > 1 {
                c.push(typed("pitch between rows", dro(pitch[1]), span(1, rows)));
            }
            c.push(typed(
                "holes in a row × rows",
                format!("{columns} × {rows}"),
                None,
            ));
            c.push(typed(
                "rows from +X",
                degrees(angle),
                Some("counter-clockwise".into()),
            ));
            // Square to the axes, back and forth: one axis a move (LIO-405).
            let then = if angle == 0. {
                "Then the DRO leads to each hole, moving one axis at a time: move it to 0.00."
            } else {
                "Then the DRO leads to each hole: move X and Y to 0.00."
            };
            (
                "Matrix holes",
                "type the grid into the DRO",
                "the DRO's matrix holes",
                c,
                then.to_string(),
            )
        }
        Pattern::Arc {
            centre,
            diameter,
            count,
            first,
            step,
            full,
        } => {
            let last = (first + step * (count - 1) as f32).rem_euclid(360.);
            let round = if full {
                "full circle"
            } else {
                "part of a circle"
            };
            let c = vec![
                typed("centre X", dro(centre[0]), None),
                typed("centre Y", dro(centre[1]), None),
                typed(
                    "circle Ø",
                    dro(diameter),
                    Some(format!("radius {}", dro(diameter / 2.))),
                ),
                typed("holes", count.to_string(), Some(round.into())),
                typed("first hole from +X", degrees(first), None),
                // The angle between holes is the other way to say the last
                // hole's: three rows fit at 1024 x 700 (LIO-404).
                typed(
                    "last hole from +X",
                    degrees(last),
                    Some(format!("or {} between holes", degrees(step))),
                ),
            ];
            (
                "Arc holes",
                "type the arc into the DRO",
                "the DRO's arc holes",
                c,
                if out_of_order {
                    // One sentence (LIO-407).
                    format!(
                        "Call up each hole on the DRO in this order and move to 0.00: {one_axis} of the {} moves are along one axis.",
                        holes.len().saturating_sub(1)
                    )
                } else {
                    "Then the DRO leads to each hole: move X and Y to 0.00.".into()
                },
            )
        }
    };
    let mut right = column![]
        .push(said)
        .push(label(format!("Type into {kind}")));
    let mut numbers = numbers.into_iter();
    while let Some(a) = numbers.next() {
        let b = numbers.next().unwrap_or_else(|| space().width(Fill).into());
        right = right.push(row![a, b].spacing(8));
    }
    // The DRO's order may not be ours: every hole is drilled alike
    // (LIO-403, LIO-405); an arc out of its order calls each hole up.
    let after = if out_of_order {
        String::new()
    } else {
        " If the DRO takes the holes in another order, follow it: every hole is drilled alike."
            .into()
    };
    // An arc out of the DRO's order, written out (LIO-407).
    let right = right.push(out_of_order.then(|| {
        let order: Vec<String> = called.iter().map(|n| n.to_string()).collect();
        bold(format!("Order on the DRO: {}", order.join(" → ")), 16)
    }));
    let right = right.push(muted(format!("{then}{after}")).size(13));
    let stock = p.before(op);
    let left = p.drawing(
        0,
        stock,
        vec![Shape::Pattern(pattern.clone(), holes, r, called)],
        Fill,
    );
    (title.into(), sub, left, right)
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
    let lines = |view: [usize; 2]| never_lines(g, op, stock, never, r, view);
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
        p.drawing(0, stock, shapes, FillPortion(3)),
        p.drawing(side, stock, across, FillPortion(2)),
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
            if last[2].is_some_and(|v| v > t.centre && v - t.centre < LIFT) {
                // Down again after the lift off the floor (LIO-382).
                tags.push("back to the level".into());
            } else {
                // Down to the pass's level: how much deeper it cuts.
                tags.push(format!("{} deeper", mm(cut.max[2] - cut.min[2])));
                if t.feed {
                    tags.push("plunge".into());
                }
            }
        } else if t.axis == 2 && last[2].is_some_and(|v| t.centre > v && t.centre - v < LIFT) {
            // Up off the floor so the return does not rub (LIO-176, LIO-382).
            tags.push("off the floor".into());
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
    /// The plate's Ø32 bore through 28 on the manual mill (LIO-248):
    /// drilled Ø12, then bored with the head 1 mm a pass on the radius;
    /// the Steps page open on it.
    fn bored_app() -> App {
        use crate::cam::boring::tests::{drill, drilling, head, job, ring};
        let mesh = ring(32., 60., 28.);
        let manual = crate::manual::defaults();
        let on = |mut t: Tool| {
            t.p.insert("machines".into(), manual.id.clone());
            t
        };
        let (d, h) = (on(drill("12")), on(head(crate::model::BORE_RETRACTS[0])));
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
        on_page(doc, mesh)
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
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        a.test_dir = Some(dir);
        a.doc = doc;
        a.imported_mesh = Some(mesh);
        let _ = a.update(Message::Page(Page::Control));
        build(&mut a);
        a
    }
    /// The plate of `guide::tests::pattern_job` with its holes `at` on the
    /// Steps page.
    fn pattern_app(at: &[[f32; 2]], grids: bool, arcs: bool, centred: bool) -> App {
        let (doc, mesh) = crate::guide::tests::pattern_job(at, grids, arcs, centred);
        on_page(doc, mesh)
    }
    /// The Pattern steps.
    fn patterns(a: &App) -> Vec<usize> {
        (guide(a).steps.iter().enumerate())
            .filter(|(_, s)| matches!(s, Step::Pattern { .. }))
            .map(|(i, _)| i)
            .collect()
    }
    #[test]
    fn the_pattern_and_holes_screens_say_what_to_type_and_each_hole_reads_zero() {
        use crate::guide::tests::{plate_circle, plate_grid};
        let shows = |a: &App, wants: &[&str]| {
            let t = texts(a);
            for want in wants {
                assert!(t.iter().any(|s| s == want), "{want} in {t:?}");
            }
        };
        // Hole 2's readouts: an axis that moves reads 0 on the DRO's hole 2,
        // its centre beside; one that stays says so (LIO-405).
        let hole_2 = |a: &mut App, at: usize| {
            send(a, GuideAction::Go(at + 1));
            send(a, GuideAction::Next);
            let Step::Holes { holes, numbers, .. } = &guide(a).steps[at + 1] else {
                panic!()
            };
            let t = texts(a);
            for k in 0..2 {
                if (holes[1][k] - holes[0][k]).abs() < 0.005 {
                    assert_eq!(after(&t, AXES[k]), ["stays", "leave it locked"]);
                    continue;
                }
                // The DRO's number of the hole drilled second (LIO-406).
                let tag = format!(
                    "on the DRO's hole {} · {}\u{a0}{}",
                    numbers[1],
                    AXES[k],
                    dro(holes[1][k])
                );
                assert_eq!(after(&t, AXES[k])[0], "0.00", "{t:?}");
                assert!(t.contains(&tag), "{tag} in {t:?}");
            }
            assert!(t.iter().any(|s| s.starts_with("drill to · ")), "{t:?}");
        };
        let mut a = pattern_app(&plate_grid(), true, false, false);
        let at = patterns(&a)[0];
        send(&mut a, GuideAction::Go(at));
        let Step::Pattern {
            pattern: Pattern::Grid { first, .. },
            ..
        } = guide(&a).steps[at]
        else {
            panic!()
        };
        shows(
            &a,
            &[
                "Matrix holes",
                "TYPE INTO THE DRO'S MATRIX HOLES",
                &dro(first[0]),
                &dro(first[1]),
                "20.00",
                "span 60.00",
                "15.00",
                "span 30.00",
                "4 × 3",
                "holes in a row × rows",
                "0.00°",
            ],
        );
        assert!(!texts(&a).iter().any(|s| s.contains("same pattern")));
        hole_2(&mut a, at);
        // A centre drilling, then its drill: the second says it is the same.
        let mut a = pattern_app(&plate_circle(), false, true, true);
        let at = patterns(&a);
        assert_eq!(at.len(), 2);
        send(&mut a, GuideAction::Go(at[1]));
        shows(
            &a,
            &[
                "Arc holes",
                "TYPE INTO THE DRO'S ARC HOLES",
                "the same pattern: start the DRO again at hole 1",
                "60.00",
                "radius 30.00",
                "6",
                "full circle",
                "30.00°",
                "330.00°",
                "or 60.00° between holes",
            ],
        );
        hole_2(&mut a, at[1]);
    }
    /// The Holes screen of a grid, back and forth (LIO-405): each hole asks
    /// only for the axis that moves, the other "stays"; hole 1 and a bolt
    /// circle's holes move both; a plain Drill's hole on the Y of the one
    /// before keeps Y.
    #[test]
    fn a_hole_asks_only_for_the_axis_that_moves() {
        use crate::guide::tests::{plate_circle, plate_grid};
        // The numbered steps of the screen: the axis after each disc.
        let steps = |a: &App| -> Vec<(String, String)> {
            let t = texts(a);
            (0..t.len().saturating_sub(1))
                .filter(|&i| ["1", "2", "3"].contains(&t[i].as_str()))
                .filter(|&i| ["X", "Y", "Z"].contains(&t[i + 1].as_str()))
                .map(|i| (t[i].clone(), t[i + 1].clone()))
                .collect()
        };
        let pairs = |v: &[(&str, &str)]| -> Vec<(String, String)> {
            v.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        };
        let mut a = pattern_app(&plate_grid(), true, false, false);
        let holes = patterns(&a)[0] + 1;
        send(&mut a, GuideAction::Go(holes));
        assert_eq!(steps(&a), pairs(&[("1", "X"), ("2", "Y"), ("3", "Z")]));
        // Hole 2: X, then Z; Y stays. Hole 5, up onto the second row: Y.
        a.guide_item = 1;
        assert_eq!(steps(&a), pairs(&[("1", "X"), ("2", "Z")]));
        assert_eq!(after(&texts(&a), "Y"), ["stays", "leave it locked"]);
        a.guide_item = 4;
        assert_eq!(steps(&a), pairs(&[("1", "Y"), ("2", "Z")]));
        assert_eq!(after(&texts(&a), "X"), ["stays", "leave it locked"]);
        // Tapping X lights Z, the next step.
        send(&mut a, GuideAction::Reached(0));
        assert_eq!(a.guide_reached, 1);
        // A bolt circle moves both, but between two holes on one X or Y
        // (150° and 210° on this one) that one stays, as on any Drill.
        let mut a = pattern_app(&plate_circle(), false, true, false);
        let at = patterns(&a)[0] + 1;
        let Step::Holes { holes, .. } = &guide(&a).steps[at] else {
            panic!()
        };
        let holes = holes.clone();
        send(&mut a, GuideAction::Go(at));
        for item in 0..6 {
            a.guide_item = item;
            let stays = |k: usize| item > 0 && (holes[item][k] - holes[item - 1][k]).abs() < 0.005;
            let mut want = vec![];
            for k in (0..2).filter(|&k| !stays(k)) {
                want.push(((want.len() + 1).to_string(), AXES[k].to_string()));
            }
            want.push(((want.len() + 1).to_string(), "Z".to_string()));
            assert_eq!(steps(&a), want, "hole {}", item + 1);
            // Drilled 1, 3, 4, 6, 5, 2: all but the first and 6 to 5 keep
            // an axis (LIO-406).
            assert_eq!(
                stays(0) || stays(1),
                ![0, 4].contains(&item),
                "hole {}",
                item + 1
            );
        }
        // A plain Drill, no pattern: a hole on the Y of the one before keeps Y.
        let mut a = app_with("Yes", |doc| {
            for k in ["dro_matrix", "dro_arc"] {
                doc.setups[0].machine.p.insert(k.into(), "No".into());
            }
        });
        let at = (guide(&a).steps.iter())
            .position(|s| matches!(s, Step::Holes { pattern: None, .. }))
            .unwrap();
        let Step::Holes { holes, .. } = &guide(&a).steps[at] else {
            panic!()
        };
        let same_y = (1..holes.len())
            .find(|&i| (holes[i][1] - holes[i - 1][1]).abs() < 0.005)
            .expect("two holes one after the other on one Y");
        send(&mut a, GuideAction::Go(at));
        a.guide_item = same_y;
        assert_eq!(after(&texts(&a), "Y"), ["stays", "leave it locked"]);
    }
    /// LIO-406: the second hole drilled of the bolt circle is the DRO's
    /// hole 3, Y staying; the Pattern screen says how many moves keep an axis.
    #[test]
    fn a_bolt_circle_names_each_hole_by_the_dros_number() {
        use crate::guide::tests::plate_circle;
        let mut a = pattern_app(&plate_circle(), false, true, false);
        let at = patterns(&a)[0];
        send(&mut a, GuideAction::Go(at));
        let t = texts(&a);
        // The order written out, the closing line one sentence (LIO-407).
        for want in [
            "Order on the DRO: 1 → 3 → 4 → 6 → 5 → 2",
            "Call up each hole on the DRO in this order and move to 0.00: 4 of the 5 moves are along one axis.",
        ] {
            assert!(t.iter().any(|s| s == want), "{want} in {t:?}");
        }
        send(&mut a, GuideAction::Go(at + 1));
        send(&mut a, GuideAction::Next);
        let t = texts(&a);
        let heading = "HOLE 2 OF 6 · CALL UP THE DRO'S HOLE 3".to_string();
        assert!(t.contains(&heading), "{t:?}");
        assert_eq!(after(&t, "Y"), ["stays", "leave it locked"]);
        assert!(
            t.iter().any(|s| s.starts_with("on the DRO's hole 3 · X")),
            "{t:?}"
        );
    }
    /// LIO-417: a hole's depth label is right of its line, past the hole.
    #[test]
    fn a_holes_depth_label_is_clear_of_the_hole() {
        // A Ø10 hole drawn 4 px a mm round X 200: its line ±3 radii.
        let (x, r, s) = (200., 5., 4.);
        let (m, n) = (
            Point::new(x - 3. * r * s, 150.),
            Point::new(x + 3. * r * s, 150.),
        );
        let at = depth_label(m, n);
        assert!(at.x > x + r * s + 4., "{at:?}");
    }
    /// LIO-407: an arc taken round (four from 45°) has no order line and a
    /// plain heading.
    #[test]
    fn an_arc_taken_round_needs_no_order_line() {
        let four = crate::guide::tests::circle_at([0., 0.], 60., 4, 45., 90.);
        let mut a = pattern_app(&four, false, true, false);
        let at = patterns(&a)[0];
        send(&mut a, GuideAction::Go(at));
        assert!(!texts(&a).iter().any(|s| s.starts_with("Order on the DRO")));
        send(&mut a, GuideAction::Go(at + 1));
        send(&mut a, GuideAction::Next);
        let t = texts(&a);
        assert!(t.contains(&"HOLE 2 OF 4".to_string()), "{t:?}");
        assert!(!t.iter().any(|s| s.contains("CALL UP")), "{t:?}");
    }
    /// LIO-407: a pattern drawing's tag stays whole inside the drawing, at
    /// its left, bottom, right and top edges.
    #[test]
    fn a_pattern_tag_stays_inside_the_drawing() {
        let size = Size::new(400., 300.);
        for (from, out) in [
            (Point::new(5., 150.), [-1., 0.]),
            (Point::new(200., 295.), [0., -1.]),
            (Point::new(395., 150.), [1., 0.]),
            (Point::new(200., 3.), [0., 1.]),
        ] {
            let r = tag_box(from, out, "rows 56.00".len(), 1., size);
            assert!(
                r.x >= 0.
                    && r.y >= 0.
                    && r.x + r.width <= size.width
                    && r.y + r.height <= size.height,
                "{from:?} {r:?}"
            );
        }
    }
    /// LIO-404: at 1024 x 700 every number of the Arc holes screen and its
    /// closing line are above the buttons, with nothing to scroll.
    #[test]
    fn the_arc_holes_screen_fits_at_1024_by_700() {
        use crate::guide::tests::{plate_circle, plate_grid};
        let window = iced::Size::new(1024., 700.);
        for (holes, grids) in [(plate_circle(), false), (plate_grid(), true)] {
            let mut a = pattern_app(&holes, grids, !grids, false);
            (a.window_width, a.window_height) = (window.width, window.height);
            let at = patterns(&a)[0];
            send(&mut a, GuideAction::Go(at));
            let seen = crate::ui::probe::placed(a.view(), window);
            let find = |start: &str| {
                (seen.iter())
                    .find(|(t, _)| t.starts_with(start))
                    .unwrap_or_else(|| panic!("{start} in {seen:?}"))
                    .1
            };
            let back = find("◀ Back");
            // The closing line: the bolt circle's calls each hole up, as it
            // is drilled out of the DRO's order (LIO-406).
            let closing = if grids {
                "Then the DRO leads"
            } else {
                "Call up each hole"
            };
            let order = if grids {
                "first hole"
            } else {
                "Order on the DRO"
            };
            for start in [closing, "first hole", order] {
                let r = find(start);
                // Above the gap over the buttons, where the column ends.
                assert!(
                    r.y + r.height + 14. <= back.y,
                    "{start} {r:?} over {back:?}"
                );
            }
        }
    }
    /// The Pattern screen and the Holes screens of the 2nd, 5th and 1st
    /// hole drilled of the plate's grid and of its bolt circle, at
    /// 1440 × 900 and 1024 × 700, into `AXIS_RENDER_DIR`.
    #[test]
    #[ignore]
    fn render_the_hole_patterns() {
        use crate::guide::tests::{plate_circle, plate_grid};
        let dir = std::path::PathBuf::from(std::env::var("AXIS_RENDER_DIR").unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        for (name, holes, grids) in [("grid", plate_grid(), true), ("arc", plate_circle(), false)] {
            let mut a = pattern_app(&holes, grids, !grids, false);
            let step = patterns(&a)[0];
            for (w, h) in [(1440, 900), (1024, 700)] {
                (a.window_width, a.window_height) = (w as f32, h as f32);
                at(&mut a, step, 0);
                render(&a, (w, h), &dir.join(format!("{name}-pattern-{w}.png")));
                for hole in [2, 5, 1] {
                    at(&mut a, step + 1, hole - 1);
                    let file = format!("{name}-holes-{hole}-{w}.png");
                    render(&a, (w, h), &dir.join(file));
                }
            }
        }
    }
    #[test]
    fn a_bore_pass_shows_what_to_measure_the_head_setting_and_the_hole() {
        let mut a = bored_app();
        let g = guide(&a);
        let passes: Vec<usize> = (g.steps.iter().enumerate())
            .filter(|(_, s)| matches!(s, Step::Bore { .. }))
            .map(|(i, _)| i)
            .collect();
        // Ø12 to Ø31.9, at most 2 mm on the diameter a pass, then Ø32.
        assert_eq!(passes.len(), 11, "{:?}", g.steps);
        send(&mut a, GuideAction::Go(passes[0]));
        let t = texts(&a);
        assert_eq!(after(&t, "head")[0], "Ø13.99", "{t:?}");
        assert!(t.iter().any(|s| s == "set to") && !t.iter().any(|s| s == "measure"));
        // The last pass: measure what the one before left, set the head out
        // by the difference, then the hole and its depth.
        send(&mut a, GuideAction::Go(passes[10]));
        let t = texts(&a);
        for want in [
            "PASS 11 OF 11",
            "11 passes",
            "measure",
            "Ø31.90",
            "Ø32.00",
            "out 0.10 on Ø · 10 divisions",
            "spindle off to set the head",
            "spindle off, then raise",
            "bore to · 31 deep",
            "640 rpm",
        ] {
            assert!(t.iter().any(|s| s == want), "{want} in {t:?}");
        }
        assert_eq!(
            after(&t, "head"),
            ["Ø32.00", "out 0.10 on Ø · 10 divisions"]
        );
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
    /// The Holes step of operation `op`.
    fn holes_step(a: &App, op: usize) -> usize {
        (guide(a).steps.iter())
            .position(|s| matches!(s, Step::Holes { op: o, .. } if *o == op))
            .unwrap()
    }
    #[test]
    fn a_centre_drill_goes_to_its_pilot_tip_and_an_uncentred_drill_is_warned() {
        // Without a centre drilling, each hole of the drill is warned.
        let mut a = app("Yes");
        let at = holes_step(&a, 2);
        send(&mut a, GuideAction::Go(at));
        let t = texts(&a);
        assert!(t.iter().any(|s| s == "centre drill first"), "{t:?}");
        assert!(t.iter().any(|s| s == "Holes"), "{t:?}");
        // With one: its own screen, to the pilot's tip, the cone at the top
        // as wide as the body; the drill after it is not warned.
        let mut a = centred_app();
        let at = holes_step(&a, 2);
        send(&mut a, GuideAction::Go(at));
        let t = texts(&a);
        assert!(t.iter().any(|s| s == "Centre drill"), "{t:?}");
        let tags = t
            .iter()
            .find(|s| s.starts_with("pilot tip to · "))
            .expect("pilot tip to");
        assert!(tags.ends_with(" deep · cone Ø6.3 at the top"), "{tags}");
        assert!(!t.iter().any(|s| s == "centre drill first"), "{t:?}");
        let at = holes_step(&a, 3);
        send(&mut a, GuideAction::Go(at));
        let t = texts(&a);
        assert!(t.iter().any(|s| s.starts_with("drill to · ")), "{t:?}");
        assert!(!t.iter().any(|s| s == "centre drill first"), "{t:?}");
    }
    /// The centre drilling's Tool and Holes screens, the drill's after it,
    /// and the drill's with no centre drilling, at 1280 × 800, into
    /// `AXIS_RENDER_DIR`.
    #[test]
    #[ignore]
    fn render_the_centre_drilling() {
        let dir = std::path::PathBuf::from(std::env::var("AXIS_RENDER_DIR").unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let mut a = centred_app();
        let tool = (guide(&a).steps.iter())
            .position(|s| matches!(s, Step::Tool { op: 2, .. }))
            .unwrap();
        let shots = [
            ("centre-tool", tool, 0),
            ("centre-hole1", holes_step(&a, 2), 0),
            ("centre-hole3", holes_step(&a, 2), 2),
            ("drill-centred", holes_step(&a, 3), 0),
        ];
        for (name, step, item) in shots {
            at(&mut a, step, item);
            render(&a, (1280, 800), &dir.join(format!("{name}.png")));
        }
        let mut a = app("Yes");
        let step = holes_step(&a, 2);
        at(&mut a, step, 0);
        render(&a, (1280, 800), &dir.join("drill-uncentred.png"));
    }
    /// The last boring pass at 1280 × 800, in AXIS_RENDER_DIR (default
    /// /tmp/manual-steps), with the first beside it.
    #[test]
    #[ignore]
    fn render_a_bore_pass() {
        let dir = std::path::PathBuf::from(
            std::env::var("AXIS_RENDER_DIR").unwrap_or("/tmp/manual-steps".into()),
        );
        std::fs::create_dir_all(&dir).unwrap();
        let mut a = bored_app();
        let passes: Vec<usize> = (guide(&a).steps.iter().enumerate())
            .filter(|(_, s)| matches!(s, Step::Bore { .. }))
            .map(|(i, _)| i)
            .collect();
        for (name, at) in [("12-bore-first", passes[0]), ("13-bore-last", passes[10])] {
            send(&mut a, GuideAction::Go(at));
            render(&a, (1280, 800), &dir.join(format!("{name}.png")));
        }
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
        // The Holes' drill, after the 2 mm face.
        assert_eq!(tools.len(), 1);
        for i in tools {
            send(&mut a, GuideAction::Go(i));
            let t = texts(&a);
            assert!(t.iter().any(|s| s == "PICK UP Z WITH THE TOOL"), "{t:?}");
            assert_eq!(after(&t, "touch the top → set"), ["Z", "−2.00"]);
        }
        // The Step keeps the Face's mill and its Z: its first screen is
        // the DRO's diameter and sides, with no pick-up; nor does a Square
        // after the Face pick up (LIO-368).
        let comp = (guide(&a).steps.iter())
            .position(|s| matches!(s, Step::Comp { op: 1, .. }))
            .unwrap();
        send(&mut a, GuideAction::Go(comp));
        let t = texts(&a);
        assert!(t.iter().any(|s| s == "ENTER IN THE DRO"), "{t:?}");
        assert!(t.iter().any(|s| s == "25.00"), "{t:?}");
        for t in [t, texts(&squared("Yes"))] {
            assert!(!t.iter().any(|s| s == "PICK UP Z WITH THE TOOL"), "{t:?}");
        }
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
    fn the_square_step_feeds_along_its_wall_as_its_plan_cuts() {
        // Squaring the +X side: its arrow runs along Y (LIO-176), the way
        // its plan feeds, with the material on its left (conventional).
        let a = squared("Yes");
        let g = guide(&a);
        let Step::Square { op, .. } = g.steps[3] else {
            panic!()
        };
        let i = g.done.iter().position(|d| d.op == op).unwrap();
        let feed = feed_along(&g, i, 1);
        assert!(feed != 0., "no feed along Y");
        let cut = g.done[i]
            .plan
            .moves
            .iter()
            .find(|m| m.cutting_pass)
            .unwrap();
        assert_eq!(feed, (cut.to[1] - cut.from[1]).signum());
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
        // The setup carries its machine's name: said once in the title and
        // once in the list (LIO-382; "Manual Mill on the Manual Mill").
        assert!(t.iter().any(|s| s == "Manual Mill · steps"), "{t:?}");
        assert_eq!(t.iter().filter(|s| *s == "Manual Mill").count(), 1, "{t:?}");
        assert!(!t.iter().any(|s| s.contains(" on the ")), "{t:?}");
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
        // The tiles are the passes' depths, one row on each level (LIO-366).
        for tile in ["−6.75", "−11.50"] {
            assert!(t.iter().any(|s| s == tile), "{tile} in {t:?}");
        }
        // Tapping a readout lights the next.
        send(&mut a, GuideAction::Reached(0));
        assert_eq!(a.guide_item, 1);
    }
    #[test]
    fn the_check_screen_gives_the_sizes_to_measure_on_and_beside_the_picture() {
        let mut a = app("Yes");
        let g = guide(&a);
        let checks: Vec<usize> = (g.steps.iter().enumerate())
            .filter(|(_, s)| matches!(s, Step::Check { .. }))
            .map(|(i, _)| i)
            .collect();
        // The Step: 15 wide from the front face, 9.5 below the faced top.
        send(&mut a, GuideAction::Go(checks[0]));
        let t = texts(&a);
        for want in [
            "15.00",
            "wide · from the front face",
            "9.50",
            "deep below the top · floor Z −11.50",
        ] {
            assert!(t.iter().any(|s| s == want), "{want} in {t:?}");
        }
        // The Holes: 4 of Ø4, 42 deep from their mouths on the faced top.
        send(&mut a, GuideAction::Go(checks[1]));
        let t = texts(&a);
        for want in ["4.00", "4 holes", "42.00", "deep from the mouth"] {
            assert!(t.iter().any(|s| s == want), "{want} in {t:?}");
        }
        // Drawn on the pictures after them, inside the picture.
        let shown = a.guide_at().unwrap().0;
        let labels = |i: usize| -> Vec<String> {
            for d in &shown.dims[i] {
                for q in [d.a, d.b] {
                    assert!(
                        (0. ..=960.).contains(&q.x) && (0. ..=640.).contains(&q.y),
                        "{d:?}"
                    );
                }
            }
            shown.dims[i].iter().map(|d| d.label.clone()).collect()
        };
        assert_eq!(labels(2), ["Y 15", "Z 9.5"]);
        assert_eq!(labels(3), ["Ø 4", "Z 42"]);
        // The labels' tags overlap neither each other nor any dimension
        // line, in the picture's panel at 1440 × 900 and at 1024 × 700
        // (LIO-379: "Z 9.5" lay on "Y 15"); the step's sizes have extension
        // lines from the step.
        assert!(shown.dims[2].iter().all(|d| d.ext.is_some()));
        for size in [Size::new(640., 600.), Size::new(420., 470.)] {
            for i in [2, 3] {
                let placed = dim_tags(&shown.dims[i], size);
                for (n, (a, b, _, r)) in placed.iter().enumerate() {
                    for (m, (.., o)) in placed.iter().enumerate() {
                        let on = (0..=20).any(|t| {
                            let t = t as f32 / 20.;
                            let q = Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
                            q.x > o.x && q.x < o.x + o.width && q.y > o.y && q.y < o.y + o.height
                        });
                        assert!(!on, "{size:?} {i}: tag {m} on line {n}");
                        if m != n {
                            assert!(r.intersection(o).is_none(), "{size:?} {i}: {r:?} {o:?}");
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn a_new_tool_is_drawn_to_scale_on_the_highest_top() {
        // To scale: the cutting part as wide as the tool.
        for d in [4., 25.] {
            let tool = viewport::tool_triangles([0., 0., 10.], d, 20., 40., d < 10., None);
            let (lo, hi) = (tool.iter())
                .flat_map(|(t, _)| t.iter())
                // Below the shank, less the ring round the spot.
                .filter(|p| p[2] < 29.9 && (p[2] - 10.05).abs() > 0.001)
                .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p[0]), b.max(p[0])));
            close(hi - lo, d);
        }
        // A step 2 mm down along the front: the spot is on the higher top.
        let low = Bounds {
            min: [0., 0., 0.],
            max: [100., 15., 8.],
        };
        let high = Bounds {
            min: [0., 15., 0.],
            max: [100., 60., 10.],
        };
        let mut stock = viewport::box_triangles(low, viewport::BLOCK);
        stock.extend(viewport::box_triangles(high, viewport::BLOCK));
        let spot = touch_spot(&stock).unwrap();
        assert!(spot[2] == 10. && spot[1] > 15., "{spot:?}");
        // On the guide's job every new tool has its picture.
        let a = app("Yes");
        let shown = a.guide_at().unwrap().0;
        for (i, s) in shown.guide.steps.iter().enumerate() {
            let new = matches!(s, Step::Tool { top: Some(_), .. });
            assert_eq!(shown.tools[i].is_some(), new, "{i} {s:?}");
        }
    }
    #[test]
    fn back_and_next_are_in_one_place_on_every_screen() {
        let mut a = app("Yes");
        let g = guide(&a);
        let first = |want: fn(&Step) -> bool| g.steps.iter().position(want).unwrap();
        let screens = [
            0,
            first(|s| matches!(s, Step::Pass { .. })),
            first(|s| matches!(s, Step::Check { .. })),
            g.steps.len() - 1,
        ];
        let window = Size::new(1280., 800.);
        let bottom = Rectangle::new(Point::new(0., 400.), Size::new(1280., 400.));
        let mut spans = vec![];
        for at in screens {
            send(&mut a, GuideAction::Go(at));
            let presses = crate::ui::probe::presses(a.view(), window, bottom, 5.);
            // Where pressing gives what `want` matches: (x min, y min, x
            // max, y max), None when nowhere.
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
        // Next (or Go to the next setup) in one place; Back beside it, dead
        // on the first screen.
        assert!(spans[0].0.is_some() && spans[0].1.is_none(), "{spans:?}");
        for s in &spans[1..] {
            assert_eq!(s.0, spans[0].0, "{spans:?}");
            assert_eq!(s.1, spans[1].1, "{spans:?}");
        }
        assert!(spans[1].1.is_some());
        // Back from the Holes' Check lands on its last hole.
        let check = (g.steps.iter()).rposition(|s| matches!(s, Step::Check { .. }));
        send(&mut a, GuideAction::Go(check.unwrap()));
        send(&mut a, GuideAction::Back);
        assert!(matches!(g.steps[place(&a)], Step::Holes { .. }));
        assert_eq!(a.guide_item, 3);
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
        assert!(has(&a, "Open Step in MANUFACTURE ▶"), "{:?}", texts(&a));
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
        assert!(
            has(&a, "Open the setup in MANUFACTURE ▶"),
            "{:?}",
            texts(&a)
        );
        a.selected = Some(0);
        let _ = a.update(Message::Guide(GuideAction::Fix(None, String::new())));
        assert_eq!((a.page, a.selected), (Page::Work, None));
    }
    #[test]
    fn the_hold_caption_follows_the_holding_and_a_holes_readouts_light_in_turn() {
        // No holding modelled, then a vise (LIO-375).
        let a = app("Yes");
        assert!(has(&a, "the stock as it starts") && !has(&a, "where it goes in the vise"));
        let a = app_with("Yes", crate::guide::tests::in_the_vise);
        assert!(has(&a, "where it goes in the vise"));
        // Tapping X on a Holes screen lights Y and keeps the hole.
        let mut a = app("Yes");
        let holes = (guide(&a).steps.iter())
            .position(|s| matches!(s, Step::Holes { .. }))
            .unwrap();
        send(&mut a, GuideAction::Go(holes));
        send(&mut a, GuideAction::Next);
        assert_eq!((a.guide_item, a.guide_reached), (1, 0));
        send(&mut a, GuideAction::Reached(0));
        assert_eq!((a.guide_item, a.guide_reached), (1, 1));
        // Hole done: the next hole, from its first readout.
        send(&mut a, GuideAction::Next);
        assert_eq!((a.guide_item, a.guide_reached), (2, 0));
    }
    #[test]
    fn a_holes_depth_is_counted_from_its_mouth() {
        // A Box takes the top down to Z −3 over the two holes left of X 30.
        let mut a = app_with("Yes", |doc| {
            let mill = doc.tools[3].id.clone();
            let b = [-500., 30., -500., 500., 0., -3.5];
            let low = crate::guide::tests::boxed("Low", &mill, b, &[("stepdown", "5")]);
            doc.setups[0].operations[0] = low;
            doc.setups[0].operations.remove(1);
        });
        let g = guide(&a);
        let (at, holes, mouths) = (g.steps.iter().enumerate())
            .find_map(|(i, s)| match s {
                Step::Holes { holes, mouths, .. } => Some((i, holes.clone(), mouths.clone())),
                _ => None,
            })
            .unwrap();
        assert!(
            mouths.iter().any(|m| (m - mouths[0]).abs() > 1.),
            "{mouths:?}"
        );
        send(&mut a, GuideAction::Go(at));
        for (i, (h, m)) in holes.iter().zip(&mouths).enumerate() {
            a.guide_item = i;
            let deep = format!("{} deep", mm(m - h[2]));
            assert!(texts(&a).iter().any(|s| s.contains(&deep)), "{i} {deep}");
        }
    }
    #[test]
    fn a_do_not_pass_line_reads_what_the_dro_reads_with_the_tool_at_it() {
        // The labels of the Step's lines in the top view, Ø25 cutter.
        let labels = |a: App| -> Vec<String> {
            let shown = a.guide_at().unwrap().0;
            let never = (shown.guide.steps.iter())
                .find_map(|s| match s {
                    Step::Pass { op: 1, never, .. } => Some(*never),
                    _ => None,
                })
                .unwrap();
            never_lines(&shown.guide, 1, shown.boxes[1], never, 12.5, [0, 1])
                .into_iter()
                .filter_map(|s| match s {
                    Shape::Never(_, _, label) => Some(label),
                    _ => None,
                })
                .collect()
        };
        // Comp X +, closed at X 20.5 and 99.5: the low wall reads its edge
        // a diameter on (it said X 20.50, 25 mm into the wall); the high one
        // and the +Y wall read as they are, the comp on their side.
        let closed = labels(app_with("Yes", crate::guide::tests::close_the_step));
        assert_eq!(closed, ["X 45.50", "X 99.50", "Y 15.00"]);
        // Comp off: the centre, a radius off the wall.
        assert_eq!(labels(app("No")), ["Y 2.50"]);
    }
    #[test]
    fn a_pass_that_plunges_warns_of_a_centre_cutting_end_mill() {
        let mut a = app_with("Yes", crate::guide::tests::pocket_the_step);
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
            // The pocket's first row lies on its −Y wall's allowance: the
            // second goes back along it, with no room to come off a wall.
            if n % 2 == 1 {
                assert!(t.iter().any(|s| s.contains("feed, cuts")), "{t:?}");
                // Its lift off the floor and back down say so; it was a
                // bare Z, then "1 deeper" (LIO-382).
                for tag in ["off the floor", "back to the level"] {
                    assert_eq!(t.iter().filter(|s| *s == tag).count(), 1, "{tag} {t:?}");
                }
                assert!(!t.iter().any(|s| s.ends_with(" deeper")), "{t:?}");
            }
        }
        // The open-ended Step drops outside the stock, and the Step closed
        // at both ends comes in across its open front: no warning.
        for mut a in [
            app("Yes"),
            app_with("Yes", crate::guide::tests::close_the_step),
        ] {
            let first = (guide(&a).steps.iter())
                .position(|s| matches!(s, Step::Pass { .. }))
                .unwrap();
            send(&mut a, GuideAction::Go(first));
            assert!(!texts(&a).iter().any(|s| s == warning));
        }
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
    fn a_drill_step_says_the_cycle_its_pecks_and_feed() {
        // The Holes' drill as planned: its feed, and the pecks of each cycle
        // with how far the quill comes back between them.
        for (cycle, back) in [
            ("Chip breaking", Some("back off 0.5 each")),
            ("Deep drilling", Some("out of the hole each")),
            ("Drilling", None),
        ] {
            let mut a = app_with("Yes", |doc| {
                let op = (doc.setups[0].operations.iter_mut())
                    .find(|o| o.strategy == Strategy::Drill)
                    .unwrap();
                op.p.insert("cycle".into(), cycle.into());
                op.p.insert("peck".into(), "3".into());
                op.p.insert("feed".into(), "150".into());
            });
            let holes = (guide(&a).steps.iter())
                .position(|s| matches!(s, Step::Holes { .. }))
                .unwrap();
            send(&mut a, GuideAction::Go(holes));
            if let Ok(dir) = std::env::var("AXIS_RENDER_DIR") {
                let path = std::path::Path::new(&dir).join(format!("drill-{cycle}.png"));
                render(&a, (1280, 800), &path);
            }
            let t = texts(&a);
            assert!(has(&a, "150 mm/min"), "{t:?}");
            match back {
                Some(back) => {
                    let line = format!("drill to · 42 deep · peck 3 mm · {back}");
                    assert!(has(&a, &line) && has(&a, cycle), "{t:?}");
                }
                None => assert!(has(&a, "drill to · 42 deep"), "{t:?}"),
            }
        }
    }
    #[test]
    fn a_section_opens_at_its_first_unfinished_step_and_the_bar_shows_progress() {
        let mut a = app("Yes");
        let g = guide(&a);
        let names: Vec<&str> = g.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Set up", "Face", "Step", "Holes", "Hand over"]);
        send(&mut a, GuideAction::Section(2));
        assert_eq!(place(&a), g.sections[2].steps.start);
        // The Step: Tool, its two passes (one row a level, LIO-366), Check.
        assert!(has(&a, "1/4"), "{:?}", texts(&a));
        // Two passes on: the bar and the header say where.
        send(&mut a, GuideAction::Next);
        send(&mut a, GuideAction::Next);
        let t = texts(&a);
        assert_eq!(t.iter().filter(|s| *s == "pass 2/2").count(), 2, "{t:?}");
        // The sections ahead show their counts, the next setup is last.
        // Each with what it counts (LIO-377).
        assert!(t.contains(&"Holes".to_string()) && t.contains(&"4 holes".to_string()));
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
        assert!(has(&a, "It broke.") && has(&a, "Open the setup in MANUFACTURE ▶"));
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
        // A frame's redraw, which sets each button's status: without it a
        // button draws as disabled.
        let redraw = iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        ));
        let _ = ui.update(
            &[redraw],
            mouse::Cursor::Unavailable,
            &mut renderer,
            &mut iced::advanced::clipboard::Null,
            &mut vec![],
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
        // A pocket: a plunge, then feeds across.
        let mut a = app_with("Yes", crate::guide::tests::pocket_the_step);
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
    /// The manual mill's Hold, Zero and first Tool screens of the shaft
    /// turned on the lathe and then drilled by hand
    /// (`guide::tests::turned_then_drilled_by_hand`), and the Hold and
    /// Zero of `guide::tests::short_part_in_soft_jaws`, at 1280 × 800, into
    /// `AXIS_RENDER_DIR`.
    #[test]
    #[ignore]
    fn render_the_hand_drilling_after_the_lathe() {
        let dir = std::path::PathBuf::from(std::env::var("AXIS_RENDER_DIR").unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        let (job, mesh) = crate::guide::tests::turned_then_drilled_by_hand();
        let mut a = lathe_app_of(job, mesh, |d| d.active = d.setups[1].id.clone());
        let tool = (guide(&a).steps.iter())
            .position(|s| matches!(s, Step::Tool { .. }))
            .unwrap();
        for (name, step) in [("hold", 0), ("zero", 1), ("tool", tool)] {
            let id = a.doc.setups[1].id.clone();
            a.doc.guide.insert(id, step);
            render(&a, (1280, 800), &dir.join(format!("mill-{name}.png")));
        }
        // The short part in soft jaws: touched on its Ø20 above them.
        let (doc, mesh, report, plan) = crate::guide::tests::short_part_in_soft_jaws();
        let job = crate::proposal::build(&doc, &mesh, &report, &plan).unwrap();
        let mut a = lathe_app_of(job, mesh, |d| d.active = d.setups[1].id.clone());
        for (name, step) in [("hold", 0), ("zero", 1)] {
            let id = a.doc.setups[1].id.clone();
            a.doc.guide.insert(id, step);
            render(&a, (1280, 800), &dir.join(format!("short-{name}.png")));
        }
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
        shows(&a, &["Try", "the M40×1.5 nut", "spring pass"]);
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
        let keys = ["cross slide", "top slide", "spindle"];
        let mut last = 0.;
        for key in keys {
            let r = *at(key)
                .last()
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
}
