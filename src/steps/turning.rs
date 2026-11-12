//! The lathe's screens on the Steps page: a side view of the bar (the chuck
//! on the left, you at the front) and every number as a picture of a dial
//! collar, as the lathe has no DRO.
use super::{
    ACCENT, CHIP, Lit, NEVER, Page, STOCK, TOOL, arrow, chip, chips, dashed, disc, frame, label,
    lit, stroke, tiles, warn_chip,
};
use crate::{
    GuideAction, Message, Target,
    guide::{Dial, Guide, LatheStep, Section, Step},
    help,
    lathe::{self, Reads, Slide},
    mesh::Mesh,
    model::{Document, num},
    simulation::Motion,
    turn,
    ui::{INK, LINE, MUTED, bold, muted, primary},
};
use iced::{
    Alignment, Color, Element, Font, Point, Rectangle, Size, Theme,
    advanced::text::Alignment as Align,
    mouse,
    widget::{Column, canvas, column, container, row, space, text, text_input},
};
use std::cell::Cell;

/// What the lathe's side views draw, in lathe work coordinates: Z 0 at the
/// free end as the setup starts, negative toward the chuck; radii in mm.
#[derive(Debug, Clone, Default)]
pub(crate) struct Side {
    /// The material before each operation the setup runs, then after the
    /// last: sections `[z0, z1, inner, outer]`.
    bars: Vec<Vec<[f32; 4]>>,
    /// Each operation's rings in the order cut, each as the pieces of the
    /// material it turns away (`[z0, z1, r0, r1]`).
    cuts: Vec<Vec<Vec<[f32; 4]>>>,
    /// The part's outline (`turn::profile`).
    part: Vec<[f32; 4]>,
    /// The chuck: how much of the bar its jaws grip, how far the bar stands
    /// out of them, and the radius they close on.
    grip: f32,
    stickout: f32,
    gripped: f32,
}
/// The side views of lathe `guide`: the material before and after each
/// operation (`turn::Material`, as `guide` reads it), what each ring of each
/// operation turns away of it, the part's outline and the chuck.
pub(super) fn side(doc: &Document, mesh: &Mesh, guide: &Guide) -> Side {
    let setup = &doc.setups[guide.setup];
    let Some(first) = guide.done.first() else {
        return Side::default();
    };
    let stock = first.plan.stock;
    let frame = crate::volume::Frame::new(setup, mesh);
    let material = |v: &crate::volume::Volume| {
        turn::Material::of(stock, false, Some(&crate::rest::coaxial(v, frame)))
    };
    let work = |z: f32| turn::work_z(stock, z);
    let sections = |m: &turn::Material| -> Vec<[f32; 4]> {
        (m.sections(stock.min[2], stock.max[2]).iter())
            .map(|s| [work(s.z[0]), work(s.z[1]), s.inner, s.outer])
            .collect()
    };
    let mut bars = vec![];
    let mut cuts = vec![];
    let mut m = material(&guide.before);
    for d in &guide.done {
        bars.push(sections(&m));
        let mut at = m.clone();
        let mut rings = vec![];
        for ring in &d.plan.rings {
            let g = [ring.z[0], ring.z[1], ring.r[0], ring.r[1]];
            let pieces = (at.sections(g[0], g[1]).iter())
                .filter_map(|s| {
                    let (a, b) = (s.inner.max(g[2]), s.outer.min(g[3]));
                    (b - a > 1e-3).then(|| [work(s.z[0]), work(s.z[1]), a, b])
                })
                .collect();
            rings.push(pieces);
            at = at.with(g);
        }
        cuts.push(rings);
        m = material(&d.after);
    }
    bars.push(sections(&m));
    let part = (turn::profile(setup, mesh).into_iter())
        .map(|s| [work(s[0]), work(s[1]), s[2], s[3]])
        .collect();
    let (mut grip, mut stickout, mut gripped) = (0., 0., 0.);
    if let Some(Step::Lathe(LatheStep::Hold {
        grip: g,
        stickout: s,
        diameter,
        ..
    })) = guide
        .steps
        .iter()
        .find(|s| matches!(s, Step::Lathe(LatheStep::Hold { .. })))
    {
        (grip, stickout, gripped) = (*g, *s, diameter / 2.);
    }
    Side {
        bars,
        cuts,
        part,
        grip,
        stickout,
        gripped,
    }
}

/// Where a section ends toward the tailstock: the free end.
fn end_of(bar: &[[f32; 4]]) -> f32 {
    bar.iter().map(|s| s[1]).fold(f32::NEG_INFINITY, f32::max)
}
/// The bar's largest radius.
fn outer_of(bar: &[[f32; 4]]) -> f32 {
    bar.iter().map(|s| s[3]).fold(0., f32::max)
}
/// The bar's outside radius at `z`, else its largest.
fn outer_at(bar: &[[f32; 4]], z: f32) -> f32 {
    (bar.iter())
        .find(|s| s[0] - 1e-3 <= z && z <= s[1] + 1e-3)
        .map_or(outer_of(bar), |s| s[3])
}
/// The extent of `pieces`: lowest and highest Z, smallest and largest radius.
fn extent(pieces: &[[f32; 4]]) -> Option<[f32; 4]> {
    let mut it = pieces.iter();
    let first = *it.next()?;
    Some(it.fold(first, |e, p| {
        [
            e[0].min(p[0]),
            e[1].max(p[1]),
            e[2].min(p[2]),
            e[3].max(p[3]),
        ]
    }))
}

/// Turns of a tap or a die to a tenth, a whole number without its ".0".
fn trim_turns(turns: f32) -> String {
    if turns.fract() == 0. {
        format!("{turns:.0}")
    } else {
        format!("{turns:.1}")
    }
}
/// The shape of the tool drawn at its tip.
#[derive(Debug, Clone, Copy)]
enum Cutter {
    Turn,
    Bore,
    /// A grooving tool or parting blade this wide, mm.
    Blade(f32),
    /// A drill of this radius, from the tailstock.
    Drill(f32),
    /// A tap of this radius in a wrench, guided by the tailstock.
    Tap(f32),
    /// A die in its holder, this radius across (its outside), pushed by the
    /// tailstock quill.
    Die(f32),
    Thread,
    /// A tool with a 45° edge up to its point (down to it, inside a hole).
    Chamfer(bool),
}
/// What a side view shows over the bar, at `[z, r]` points (work Z, mm from
/// the axis; a negative radius is the half below the axis).
#[derive(Debug, Clone)]
enum Mark {
    /// What this step cuts, hatched, both halves.
    Cut([f32; 4]),
    /// What the passes before it cut, hatched light.
    Earlier([f32; 4]),
    /// What the passes after it cut: an outline.
    Later([f32; 4]),
    /// A line not to pass, labelled at its top.
    Never([f32; 2], [f32; 2], String),
    Tool(Cutter, [f32; 2]),
    /// The tool's feed, an arrow.
    Path([f32; 2], [f32; 2]),
    Word([f32; 2], String, Color, Align),
    /// A touch: a dot, labelled above.
    Dot([f32; 2], String),
    /// A length between two points, arrows at both ends, labelled above.
    Span([f32; 2], [f32; 2], String),
    /// The calipers across the diameter at `z`, of radius `r`, labelled.
    Across(f32, f32, String),
}
/// A side view of the lathe: the chuck body and its jaws, the bar as it is
/// before the step's operation with its centre line, the part's outline,
/// and the step's `marks`, drawn once per step (`canvas::Cache`).
struct SideView<'a> {
    key: u64,
    side: &'a Side,
    bar: &'a [[f32; 4]],
    marks: Vec<Mark>,
}
/// How deep the chuck's body is drawn behind its jaws, mm.
const BODY: f32 = 14.;
fn bold_font() -> Font {
    Font {
        weight: iced::font::Weight::Bold,
        ..Font::DEFAULT
    }
}
fn word(f: &mut canvas::Frame, s: &str, at: Point, c: Color, size: f32, align: Align, font: Font) {
    f.fill_text(canvas::Text {
        content: s.into(),
        position: at,
        color: c,
        size: size.into(),
        font,
        align_x: align,
        align_y: iced::alignment::Vertical::Center,
        ..Default::default()
    });
}
/// Diagonal hatching across `r`.
fn hatch(f: &mut canvas::Frame, r: Rectangle, c: Color, w: f32) {
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
        k += 6.;
    }
}
/// Where a side view of `size` puts the lathe: px per mm `s`, the work Z at
/// its left edge `low` (the chuck's back), the axis `cy`, and the chuck's.
struct Scale {
    s: f32,
    low: f32,
    left: f32,
    cy: f32,
    /// The jaws' back (work Z), how far the chuck's body and the jaws reach
    /// from the axis, and the radius the jaws close on.
    back: f32,
    body: f32,
    jaw: f32,
    gripped: f32,
}
impl Scale {
    fn x(&self, z: f32) -> f32 {
        self.left + (z - self.low) * self.s
    }
    fn y(&self, r: f32) -> f32 {
        self.cy - r * self.s
    }
    fn at(&self, q: [f32; 2]) -> Point {
        Point::new(self.x(q[0]), self.y(q[1]))
    }
    /// `[z0, z1]` × `[ra, rb]`, at least 3 px across, so a thin cut shows.
    fn rect(&self, z0: f32, z1: f32, ra: f32, rb: f32) -> Rectangle {
        let (mut x0, mut x1) = (self.x(z0.max(self.low)), self.x(z1));
        let (mut y0, mut y1) = (self.y(ra.max(rb)), self.y(ra.min(rb)));
        if x1 - x0 < 3. {
            let m = (x0 + x1) / 2.;
            (x0, x1) = (m - 1.5, m + 1.5);
        }
        if y1 - y0 < 3. {
            let m = (y0 + y1) / 2.;
            (y0, y1) = (m - 1.5, m + 1.5);
        }
        Rectangle::new(Point::new(x0, y0), Size::new(x1 - x0, y1 - y0))
    }
    /// The chuck's body, then its two jaws.
    fn chuck(&self, stickout: f32) -> [Rectangle; 3] {
        let g = self.gripped;
        [
            self.rect(self.low, self.back, self.body, -self.body),
            self.rect(self.back, -stickout, g + self.jaw, g),
            self.rect(self.back, -stickout, -g, -g - self.jaw),
        ]
    }
}
/// A label of a side view as placed: where its text is anchored and how,
/// its box, and the leader back to the point it names when it had to move.
#[derive(Debug, Clone)]
struct Label {
    at: Point,
    align: Align,
    rect: Rectangle,
    leader: Option<(Point, Point)>,
}
/// The box of `text` at `size` px anchored at `at` (its middle's height).
fn text_box(text: &str, size: f32, at: Point, align: Align) -> Rectangle {
    let (w, h) = (text.chars().count() as f32 * size * 0.6, size + 4.);
    let x = match align {
        Align::Right => at.x - w,
        Align::Center => at.x - w / 2.,
        _ => at.x,
    };
    Rectangle::new(Point::new(x, at.y - h / 2.), Size::new(w, h))
}
impl SideView<'_> {
    fn scale(&self, size: Size) -> Scale {
        let side = self.side;
        let start = side.bars.first().map_or(self.bar, |b| b.as_slice());
        let big = outer_of(start).max(side.gripped).max(1.);
        let jaw = (big * 0.45).clamp(5., 25.);
        let body = big + jaw + 3.;
        let back = -(side.stickout + side.grip);
        let low = back - BODY;
        let high = end_of(start).max(end_of(self.bar)).max(0.) + 3.;
        let (top, bottom, left, right) = (30., 26., 8., 84.);
        let s = ((size.width - left - right) / (high - low).max(1.))
            .min((size.height - top - bottom) / (2. * body));
        Scale {
            s,
            low,
            left,
            cy: top + (size.height - top - bottom) / 2.,
            back,
            body,
            jaw,
            gripped: side.gripped.max(1.),
        }
    }
    /// Each mark's label, placed (LIO-389): beside its point as the mark
    /// asks, else, when that meets the chuck, the jaws, another label or the
    /// view's edge, on the other side of the point or up or down from it
    /// until it meets none, with a leader back to the point. None for a
    /// mark without a label.
    fn labels(&self, size: Size) -> Vec<Option<Label>> {
        let k = self.scale(size);
        let mut taken: Vec<Rectangle> = k.chuck(self.side.stickout).to_vec();
        // The tool and the cut it makes, which the labels name (LIO-392).
        for m in &self.marks {
            match m {
                Mark::Tool(kind, q) => taken.push(tool_box(*kind, k.at(*q), k.cy, k.s)),
                Mark::Cut(g) => {
                    taken.push(k.rect(g[0], g[1], g[2], g[3]));
                    taken.push(k.rect(g[0], g[1], -g[2], -g[3]));
                }
                _ => {}
            }
        }
        let inside = |r: &Rectangle| {
            r.x >= 0. && r.y >= 0. && r.x + r.width <= size.width && r.y + r.height <= size.height
        };
        let meets = |a: &Rectangle, b: &Rectangle| {
            a.x < b.x + b.width
                && b.x < a.x + a.width
                && a.y < b.y + b.height
                && b.y < a.y + a.height
        };
        let mut out = vec![];
        for m in &self.marks {
            let (text, size_px, wanted, align, point) = match m {
                Mark::Never(a, b, s) => {
                    let (m, n) = (k.at(*a), k.at(*b));
                    let top = if m.y < n.y { m } else { n };
                    (
                        s,
                        12.,
                        Point::new(top.x - 5., top.y - 9.),
                        Align::Right,
                        top,
                    )
                }
                Mark::Word(q, s, _, align) => (s, 13., k.at(*q), *align, k.at(*q)),
                _ => {
                    out.push(None);
                    continue;
                }
            };
            let flip = |a: Align| match a {
                Align::Right => Align::Left,
                Align::Left => Align::Right,
                other => other,
            };
            let mirrored = Point::new(2. * point.x - wanted.x, wanted.y);
            let mut tries = vec![(wanted, align), (mirrored, flip(align))];
            for step in 1..8 {
                let dy = 16. * step as f32;
                for dy in [-dy, dy] {
                    tries.push((Point::new(wanted.x, wanted.y + dy), align));
                    tries.push((Point::new(mirrored.x, wanted.y + dy), flip(align)));
                }
            }
            // Each slid along to lie inside the view, when it is wider.
            let slid = |(at, align): (Point, Align)| {
                let r = text_box(text, size_px, at, align);
                let dx = (-r.x).max(0.) - (r.x + r.width - size.width).max(0.);
                (Point::new(at.x + dx, at.y), align)
            };
            // A few px clear of what it must not lie on (LIO-392).
            let free = |(at, align): &(Point, Align)| {
                let r = text_box(text, size_px, *at, *align);
                let clear = r.expand(3.);
                inside(&r) && !taken.iter().any(|t| meets(&clear, t))
            };
            let (at, align) = (tries.into_iter().map(slid))
                .find(free)
                .unwrap_or((wanted, align));
            let rect = text_box(text, size_px, at, align);
            taken.push(rect);
            let moved = (at.y - wanted.y).abs() > 1. || (at.x - wanted.x).abs() > 1.;
            let leader = moved.then(|| {
                let end = Point::new(
                    point.x.clamp(rect.x, rect.x + rect.width),
                    if point.y < rect.y {
                        rect.y
                    } else {
                        rect.y + rect.height
                    },
                );
                (end, point)
            });
            out.push(Some(Label {
                at,
                align,
                rect,
                leader,
            }));
        }
        out
    }
    /// A cut drawn less than 40 px across (a groove, a chamfer) drawn again,
    /// enlarged, in a corner of the view (LIO-203): the bar `bar` and the
    /// part `part` round it, its half above the axis, the spot it shows
    /// marked on the view with a leader to it.
    fn inset(&self, f: &mut canvas::Frame, k: &Scale, bar: &[[f32; 4]], part: &[[f32; 4]]) {
        let cut: Vec<[f32; 4]> = (self.marks.iter())
            .filter_map(|m| match m {
                Mark::Cut(g) => Some(*g),
                _ => None,
            })
            .collect();
        let Some(e) = extent(&cut) else {
            return;
        };
        if (e[1] - e[0]).max(e[3] - e[2]) * k.s >= 40. {
            return;
        }
        let size = f.size();
        let w = (size.width * 0.3).clamp(120., 200.);
        let h = w * 0.65;
        // The corner whose rows the bar crosses least.
        let (top, bottom) = (k.y(outer_of(bar)), k.y(-outer_of(bar)));
        let over = |y: f32| (y + h).min(bottom) - y.max(top);
        let y = [6., size.height - h - 22.]
            .into_iter()
            .min_by(|a, b| over(*a).total_cmp(&over(*b)))
            .unwrap_or(6.);
        let r = Rectangle::new(Point::new(size.width - w - 6., y), Size::new(w, h));
        // The cut with as much again round it.
        let pad = (e[1] - e[0]).max(e[3] - e[2]).max(1.) * 1.2;
        let (z0, z1, r0, r1) = (e[0] - pad, e[1] + pad, (e[2] - pad).max(0.), e[3] + pad);
        let zs = (w / (z1 - z0)).min(h / (r1 - r0));
        // In the view's own coordinates, everything cut to the inset's box
        // by hand: a frame's clip is not kept by every renderer.
        let ix = |z: f32| r.x + (w - (z1 - z0) * zs) / 2. + (z - z0) * zs;
        let iy = |q: f32| r.y + (h - (r1 - r0) * zs) / 2. + (r1 - q) * zs;
        let inside = |q: Rectangle| {
            let (x0, y0) = (q.x.max(r.x), q.y.max(r.y));
            let (x1, y1) = ((q.x + q.width).min(r.x + w), (q.y + q.height).min(r.y + h));
            (x1 > x0 && y1 > y0)
                .then(|| Rectangle::new(Point::new(x0, y0), Size::new(x1 - x0, y1 - y0)))
        };
        // `[z0, z1]` × `[ra, rb]` (above the axis), cut to the box.
        let boxed = |a: f32, b: f32, ra: f32, rb: f32| {
            inside(Rectangle::new(
                Point::new(ix(a), iy(ra.max(rb))),
                Size::new(ix(b) - ix(a), (ra - rb).abs() * zs),
            ))
        };
        let path = |q: Rectangle| canvas::Path::rectangle(q.position(), q.size());
        // A line along Z or across it, cut to the box.
        let line = |f: &mut canvas::Frame, a: Point, b: Point, pen: canvas::Stroke<'_>| {
            let (x0, x1) = (a.x.min(b.x).max(r.x), a.x.max(b.x).min(r.x + w));
            let (y0, y1) = (a.y.min(b.y).max(r.y), a.y.max(b.y).min(r.y + h));
            if x1 >= x0 && y1 >= y0 {
                f.stroke(
                    &canvas::Path::line(Point::new(x0, y0), Point::new(x1, y1)),
                    pen,
                );
            }
        };
        // Each section's edges above the axis.
        let edges = |f: &mut canvas::Frame, sections: &[[f32; 4]], pen: canvas::Stroke<'_>| {
            for g in sections {
                let (a, b) = (ix(g[0]), ix(g[1]));
                let (top, low) = (iy(g[3]), iy(g[2]));
                line(f, Point::new(a, top), Point::new(b, top), pen);
                if g[2] > 1e-3 {
                    line(f, Point::new(a, low), Point::new(b, low), pen);
                }
                line(f, Point::new(a, top), Point::new(a, low), pen);
                line(f, Point::new(b, top), Point::new(b, low), pen);
            }
        };
        // Where it is on the view, and the leader to it.
        let spot = k.rect(z0, z1, r1, r0);
        f.stroke(&path(spot), dashed(ACCENT, 1.2));
        let from = Point::new(spot.x + spot.width, spot.y + spot.height / 2.);
        let to = Point::new(r.x, r.y + r.height / 2.);
        f.stroke(
            &canvas::Path::line(from, to),
            stroke(Color { a: 0.6, ..ACCENT }, 1.),
        );
        f.fill(&path(r), Color::WHITE);
        for g in bar {
            if let Some(q) = boxed(g[0], g[1], g[3], g[2]) {
                f.fill(&path(q), STOCK);
            }
        }
        edges(f, bar, stroke(INK, 1.2));
        for m in &self.marks {
            match m {
                Mark::Earlier(g) => {
                    if let Some(q) = boxed(g[0], g[1], g[3], g[2]) {
                        hatch(f, q, Color { a: 0.5, ..MUTED }, 1.);
                    }
                }
                Mark::Cut(g) => {
                    if let Some(q) = boxed(g[0], g[1], g[3], g[2]) {
                        f.fill(&path(q), Color { a: 0.14, ..ACCENT });
                        hatch(f, q, ACCENT, 1.6);
                        f.stroke(&path(q), stroke(ACCENT, 1.4));
                    }
                }
                _ => {}
            }
        }
        edges(f, part, stroke(INK, 2.));
        word(
            f,
            &format!("×{}", (zs / k.s).round()),
            Point::new(r.x + w - 6., r.y + 10.),
            ACCENT,
            12.,
            Align::Right,
            bold_font(),
        );
        f.stroke(&path(r), stroke(ACCENT, 1.4));
    }
    fn paint(&self, f: &mut canvas::Frame) {
        let side = self.side;
        let size = f.size();
        let k = self.scale(size);
        let (low, back, cy, s) = (k.low, k.back, k.cy, k.s);
        let x = |z: f32| k.x(z);
        let y = |r: f32| k.y(r);
        let at = |q: [f32; 2]| k.at(q);
        let rect = |z0: f32, z1: f32, ra: f32, rb: f32| k.rect(z0, z1, ra, rb);
        let both = |g: [f32; 4]| [rect(g[0], g[1], g[2], g[3]), rect(g[0], g[1], -g[2], -g[3])];
        let path = |r: Rectangle| canvas::Path::rectangle(r.position(), r.size());
        let [chuck, jaw_up, jaw_down] = k.chuck(side.stickout);
        // The chuck's body, behind the jaws.
        f.fill(&path(chuck), LINE);
        f.stroke(&path(chuck), stroke(MUTED, 1.));
        // The bar, as it is before this step's operation.
        let bar: Vec<[f32; 4]> = (self.bar.iter())
            .filter(|g| g[1] > back)
            .map(|g| [g[0].max(back), g[1], g[2], g[3]])
            .collect();
        for g in &bar {
            if g[2] <= 1e-3 {
                f.fill(&path(rect(g[0], g[1], g[3], -g[3])), STOCK);
            } else {
                for r in both(*g) {
                    f.fill(&path(r), STOCK);
                }
            }
        }
        outline(f, &bar, &x, &y, stroke(INK, 1.2));
        // The cuts.
        for m in &self.marks {
            match m {
                Mark::Earlier(g) => {
                    for r in both(*g) {
                        hatch(f, r, Color { a: 0.5, ..MUTED }, 1.);
                        f.stroke(&path(r), stroke(Color { a: 0.6, ..MUTED }, 0.8));
                    }
                }
                Mark::Cut(g) => {
                    for r in both(*g) {
                        f.fill(&path(r), Color { a: 0.14, ..ACCENT });
                        hatch(f, r, ACCENT, 1.6);
                        f.stroke(&path(r), stroke(ACCENT, 1.4));
                    }
                }
                Mark::Later(g) => {
                    for r in both(*g) {
                        f.stroke(&path(r), dashed(Color { a: 0.6, ..ACCENT }, 1.));
                    }
                }
                _ => {}
            }
        }
        // The jaws over the bar.
        for r in [jaw_up, jaw_down] {
            f.fill(&path(r), Color { a: 0.55, ..MUTED });
            f.stroke(&path(r), stroke(INK, 1.));
        }
        // The centre line.
        let centre = canvas::Stroke {
            line_dash: canvas::LineDash {
                segments: &[10., 3., 2., 3.],
                offset: 0,
            },
            ..stroke(MUTED, 0.8)
        };
        f.stroke(
            &canvas::Path::line(Point::new(x(low) - 4., cy), Point::new(size.width - 4., cy)),
            centre,
        );
        // The part.
        let part: Vec<[f32; 4]> = (side.part.iter())
            .filter(|g| g[1] > low)
            .map(|g| [g[0].max(low), g[1], g[2], g[3]])
            .collect();
        outline(f, &part, &x, &y, stroke(INK, 2.));
        // A label that had to move clear of the chuck, the jaws or another
        // label, with its leader (LIO-389).
        let label = |f: &mut canvas::Frame, l: &Label, s: &str, c: Color, size: f32| {
            if let Some((a, b)) = l.leader {
                f.stroke(&canvas::Path::line(a, b), stroke(Color { a: 0.7, ..c }, 1.));
                // On a plate, as it may lie over the drawing.
                f.fill_rectangle(
                    l.rect.position(),
                    l.rect.size(),
                    Color {
                        a: 0.85,
                        ..Color::WHITE
                    },
                );
            }
            word(f, s, l.at, c, size, l.align, bold_font());
        };
        for (m, l) in self.marks.iter().zip(self.labels(size)) {
            match (m, l) {
                (Mark::Never(a, b, s), Some(l)) => {
                    f.stroke(&canvas::Path::line(at(*a), at(*b)), dashed(NEVER, 1.8));
                    label(f, &l, s, NEVER, 12.);
                }
                (Mark::Tool(kind, q), _) => tool(f, *kind, at(*q), cy, s),
                (Mark::Path(a, b), _) => arrow(f, at(*a), at(*b), TOOL, 2.),
                (Mark::Word(_, s, c, _), Some(l)) => label(f, &l, s, *c, 13.),
                (Mark::Dot(q, s), _) => {
                    let p = at(*q);
                    f.fill(&canvas::Path::circle(p, 5.), ACCENT);
                    word(
                        f,
                        s,
                        Point::new(p.x, p.y - 15.),
                        ACCENT,
                        13.,
                        Align::Center,
                        bold_font(),
                    );
                }
                (Mark::Span(a, b, s), _) => {
                    let (m, n) = (at(*a), at(*b));
                    let mid = Point::new((m.x + n.x) / 2., (m.y + n.y) / 2.);
                    arrow(f, mid, m, ACCENT, 1.8);
                    arrow(f, mid, n, ACCENT, 1.8);
                    word(
                        f,
                        s,
                        Point::new(mid.x, mid.y - 11.),
                        ACCENT,
                        13.,
                        Align::Center,
                        bold_font(),
                    );
                }
                (Mark::Across(z, r, s), _) => {
                    let (m, n) = (at([*z, *r]), at([*z, -*r]));
                    let mid = Point::new(m.x, cy);
                    arrow(f, mid, m, ACCENT, 2.);
                    arrow(f, mid, n, ACCENT, 2.);
                    word(
                        f,
                        s,
                        Point::new(m.x + 8., cy - 12.),
                        ACCENT,
                        15.,
                        Align::Left,
                        bold_font(),
                    );
                }
                _ => {}
            }
        }
        self.inset(f, &k, &bar, &part);
        word(
            f,
            "chuck ◀ · side view, you at the front · tailstock ▶",
            Point::new(8., size.height - 9.),
            MUTED,
            12.,
            Align::Left,
            Font::DEFAULT,
        );
    }
}
/// The outline of `sections` (`[z0, z1, inner, outer]`) in both halves.
fn outline(
    f: &mut canvas::Frame,
    sections: &[[f32; 4]],
    x: &dyn Fn(f32) -> f32,
    y: &dyn Fn(f32) -> f32,
    pen: canvas::Stroke<'_>,
) {
    let line = |f: &mut canvas::Frame, a: (f32, f32), b: (f32, f32)| {
        f.stroke(
            &canvas::Path::line(Point::new(a.0, a.1), Point::new(b.0, b.1)),
            pen,
        );
    };
    let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
    for (i, g) in sections.iter().enumerate() {
        let before = (i.checked_sub(1))
            .map(|j| sections[j])
            .filter(|q| near(q[1], g[0]));
        let after = sections.get(i + 1).copied().filter(|q| near(q[0], g[1]));
        for sign in [1., -1.] {
            let r = |v: f32| y(sign * v);
            line(f, (x(g[0]), r(g[3])), (x(g[1]), r(g[3])));
            if g[2] > 1e-3 {
                line(f, (x(g[0]), r(g[2])), (x(g[1]), r(g[2])));
            }
            match before {
                Some(q) => {
                    if !near(q[3], g[3]) {
                        line(f, (x(g[0]), r(q[3])), (x(g[0]), r(g[3])));
                    }
                    if !near(q[2], g[2]) {
                        line(f, (x(g[0]), r(q[2])), (x(g[0]), r(g[2])));
                    }
                }
                None => line(f, (x(g[0]), r(g[2])), (x(g[0]), r(g[3]))),
            }
            if after.is_none() {
                line(f, (x(g[1]), r(g[2])), (x(g[1]), r(g[3])));
            }
        }
    }
}
/// The outlines of the tool of `kind` with its tip at `tip`, each a closed
/// polygon; `cy` is the axis, `s` px per mm.
fn tool_shapes(kind: Cutter, tip: Point, cy: f32, s: f32) -> Vec<Vec<(f32, f32)>> {
    let (x, y) = (tip.x, tip.y);
    let bar = |x0: f32, y0: f32, w: f32, h: f32| {
        vec![(x0, y0), (x0 + w, y0), (x0 + w, y0 + h), (x0, y0 + h)]
    };
    match kind {
        Cutter::Turn => vec![
            bar(x + 6., y - 40., 20., 26.),
            vec![(x, y), (x + 10., y - 14.), (x + 28., y - 14.), (x + 22., y)],
        ],
        Cutter::Bore => vec![
            bar(x, y - 2., 90., 8.),
            vec![(x, y - 2.), (x - 6., y - 8.), (x - 6., y - 2.)],
        ],
        Cutter::Blade(w) => {
            let w = (w * s).max(4.);
            vec![bar(x - w / 2., y - 36., w, 36.)]
        }
        Cutter::Drill(r) => {
            let r = (r * s).max(3.);
            vec![
                bar(x, cy - r, 70., 2. * r),
                vec![(x, cy - r), (x - r * 0.6, cy), (x, cy + r)],
            ]
        }
        Cutter::Tap(r) => {
            // The tap with its chamfered lead, its square end in a wrench
            // across it, the tailstock's centre behind.
            let r = (r * s).max(3.);
            vec![
                bar(x, cy - r, 54., 2. * r),
                vec![(x, cy - r * 0.7), (x - r * 0.6, cy), (x, cy + r * 0.7)],
                bar(x + 54., cy - 5., 10., 10.),
                bar(x + 60., cy - 30., 8., 60.),
                bar(x + 68., cy - 3., 16., 6.),
            ]
        }
        Cutter::Die(r) => {
            // The die's holder face on, with its two handles; the quill
            // pushes the holder's back.
            let r = (r * s).max(8.);
            vec![
                bar(x, cy - r, 18., 2. * r),
                bar(x + 4., cy - r - 26., 8., 26.),
                bar(x + 4., cy + r, 8., 26.),
                bar(x + 18., cy - 7., 56., 14.),
            ]
        }
        Cutter::Thread => vec![
            bar(x - 7., y - 42., 14., 26.),
            vec![(x, y), (x - 7., y - 16.), (x + 7., y - 16.)],
        ],
        Cutter::Chamfer(inside) => {
            // The 45° edge runs from the point toward the chuck and out
            // (in, inside a hole).
            let l = 18.;
            let v = if inside { 1. } else { -1. };
            let edge = vec![(x, y), (x - l, y + v * l), (x + 0.5 * l, y + v * l)];
            if inside {
                vec![edge, bar(x - l, y + l, 90., 8.)]
            } else {
                vec![edge, bar(x - l, y - l - 24., 1.5 * l, 24.)]
            }
        }
    }
}
/// The box round the tool's drawing (`tool_shapes`), which labels keep off
/// (LIO-392).
fn tool_box(kind: Cutter, tip: Point, cy: f32, s: f32) -> Rectangle {
    let points: Vec<(f32, f32)> = tool_shapes(kind, tip, cy, s).concat();
    let (mut lo, mut hi) = (tip, tip);
    for (x, y) in points {
        (lo.x, lo.y, hi.x, hi.y) = (lo.x.min(x), lo.y.min(y), hi.x.max(x), hi.y.max(y));
    }
    Rectangle::new(lo, Size::new(hi.x - lo.x, hi.y - lo.y))
}
/// The tool of `kind` with its tip at `tip`; `cy` is the axis, `s` px per mm.
fn tool(f: &mut canvas::Frame, kind: Cutter, tip: Point, cy: f32, s: f32) {
    let fill = Color { a: 0.3, ..TOOL };
    for points in tool_shapes(kind, tip, cy, s) {
        let p = canvas::Path::new(|b| {
            b.move_to(Point::new(points[0].0, points[0].1));
            for q in &points[1..] {
                b.line_to(Point::new(q.0, q.1));
            }
            b.close();
        });
        f.fill(&p, fill);
        f.stroke(&p, stroke(TOOL, 1.5));
    }
    // A chamfering tool's edge, drawn bold.
    if let Cutter::Chamfer(inside) = kind {
        let (l, v) = (18., if inside { 1. } else { -1. });
        f.stroke(
            &canvas::Path::line(tip, Point::new(tip.x - l, tip.y + v * l)),
            stroke(ACCENT, 2.5),
        );
    }
}
impl canvas::Program<Message> for SideView<'_> {
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

/// A dial collar: the index at the top, the ticks and their numbers, the
/// arc from 0 to the reading and the reading in the middle; greyed with why
/// when there is no number.
struct DialFace {
    /// The dial's numbers in one turn, and its divisions.
    turn: f32,
    div: u32,
    /// What the collar reads at the index, or why there is no number.
    reading: Result<f32, String>,
    /// Wound out: the collar counted down from 0, so the arc runs
    /// counter-clockwise from 0 to the reading.
    out: bool,
}
/// The decimals a dial of `turn` numbers a turn over `div` divisions shows.
fn places(turn: f32, div: f32) -> usize {
    let step = turn / div.max(1.);
    (0..3)
        .find(|&k| {
            let v = step * 10f32.powi(k as i32);
            (v - v.round()).abs() < 1e-3
        })
        .unwrap_or(3)
}
fn fixed(v: f32, places: usize) -> String {
    format!("{v:.places$}")
}
/// A number on a dial: 0.4, 2, 1.5.
fn trimmed(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}
impl canvas::Program<Message> for DialFace {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &iced::Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        use std::f32::consts::{FRAC_PI_2, TAU};
        let mut f = canvas::Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        let r = (w.min(h - 10.) / 2. - 2.).max(10.);
        let c = Point::new(w / 2., 10. + r);
        let grey = self.reading.is_err();
        let ink = if grey {
            Color { a: 0.45, ..MUTED }
        } else {
            INK
        };
        let face = canvas::Path::circle(c, r);
        f.fill(&face, if grey { CHIP } else { Color::WHITE });
        f.stroke(&face, stroke(ink, 1.4));
        let div = self.div.max(1);
        let every = if div >= 100 { 2 } else { 1 };
        let point = |a: f32, d: f32| Point::new(c.x + a.sin() * d, c.y - a.cos() * d);
        for i in (0..div).step_by(every) {
            let a = TAU * i as f32 / div as f32;
            let big = i % 10 == 0;
            let tick = if big { 10. } else { 5. };
            let r0 = r - tick * (r / 55.).max(1.);
            f.stroke(
                &canvas::Path::line(point(a, r0), point(a, r)),
                stroke(ink, if big { 1.4 } else { 1. }),
            );
            if big {
                word(
                    &mut f,
                    &trimmed(i as f32 * self.turn / div as f32),
                    point(a, r - (r * 0.3).clamp(15., 34.)),
                    if grey { ink } else { MUTED },
                    (r * 0.12).clamp(10., 17.),
                    Align::Center,
                    Font::MONOSPACE,
                );
            }
        }
        let places = places(self.turn, div as f32);
        match &self.reading {
            Ok(v) => {
                let turn = self.turn.max(1e-6);
                if *v > 1e-6 {
                    // The arc from 0 to the reading, the way the collar
                    // turned: clockwise winding in, counter-clockwise out.
                    let end = -FRAC_PI_2 + TAU * (v / turn).min(0.999);
                    let (a0, a1) = if self.out {
                        (
                            -FRAC_PI_2 - TAU * (1. - v / turn).clamp(0.001, 0.999),
                            -FRAC_PI_2,
                        )
                    } else {
                        (-FRAC_PI_2, end)
                    };
                    let arc = canvas::Path::new(|b| {
                        b.arc(canvas::path::Arc {
                            center: c,
                            radius: r - 3.,
                            start_angle: iced::Radians(a0),
                            end_angle: iced::Radians(a1),
                        })
                    });
                    f.stroke(
                        &arc,
                        canvas::Stroke {
                            line_cap: canvas::LineCap::Round,
                            ..stroke(Color { a: 0.85, ..ACCENT }, (r * 0.05).max(6.))
                        },
                    );
                    let tip = Point::new(c.x + end.cos() * (r - 3.), c.y + end.sin() * (r - 3.));
                    f.fill(&canvas::Path::circle(tip, (r * 0.045).max(5.5)), ACCENT);
                }
                word(
                    &mut f,
                    &fixed(*v, places),
                    c,
                    INK,
                    (r * 0.3).clamp(14., 34.),
                    Align::Center,
                    Font {
                        weight: iced::font::Weight::Bold,
                        ..Font::MONOSPACE
                    },
                );
            }
            Err(why) => {
                // Two lines across the middle, never a number.
                let words: Vec<&str> = why.split(' ').collect();
                let half = words.len().div_ceil(2);
                for (k, part) in [words[..half].join(" "), words[half..].join(" ")]
                    .iter()
                    .enumerate()
                {
                    word(
                        &mut f,
                        part,
                        Point::new(c.x, c.y - 8. + 16. * k as f32),
                        MUTED,
                        12.,
                        Align::Center,
                        bold_font(),
                    );
                }
            }
        }
        // The index the collar's numbers are read at.
        let index = canvas::Path::new(|b| {
            b.move_to(Point::new(c.x - 7., c.y - r - 9.));
            b.line_to(Point::new(c.x + 7., c.y - r - 9.));
            b.line_to(Point::new(c.x, c.y - r + 2.));
            b.close();
        });
        f.fill(&index, if grey { ink } else { INK });
        vec![f.into_geometry()]
    }
}

/// The top slide seen from above, you at the front, the chuck on the left:
/// the cross slide's line (toward the axis, up), the bed's, and the top
/// slide at `from_cross`° from the cross slide (90 = along the bed), its
/// arrow the way the tool goes as its dial is wound (`out`: wound out,
/// inside a hole).
struct TopAngle {
    from_cross: f32,
    out: bool,
}
impl canvas::Program<Message> for TopAngle {
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
        // Wound out, the arrow points down, away from the axis: the
        // centre higher, room for it and its words below.
        let c = Point::new(w * 0.62, h * if self.out { 0.3 } else { 0.6 });
        let l = (h * 0.5).min(w * 0.45);
        // The bed, along the axis, and the cross slide, toward it.
        f.stroke(
            &canvas::Path::line(Point::new(6., c.y), Point::new(w - 6., c.y)),
            dashed(MUTED, 1.),
        );
        let (top, bottom) = if self.out {
            (c.y - l * 0.5, c.y + l * 0.9)
        } else {
            (c.y - l, c.y + l * 0.55)
        };
        f.stroke(
            &canvas::Path::line(Point::new(c.x, bottom), Point::new(c.x, top)),
            stroke(INK, 1.4),
        );
        word(
            &mut f,
            "cross slide",
            Point::new(c.x + 5., top + 6.),
            MUTED,
            11.,
            Align::Left,
            Font::DEFAULT,
        );
        word(
            &mut f,
            "◀ chuck",
            Point::new(6., c.y + 10.),
            MUTED,
            11.,
            Align::Left,
            Font::DEFAULT,
        );
        // The top slide, at its angle toward the chuck: in and toward the
        // chuck winding in, out and toward the chuck winding out.
        let a = self.from_cross.to_radians();
        let v = if self.out { 1. } else { -1. };
        let tip = Point::new(c.x - a.sin() * l, c.y + v * a.cos() * l);
        arrow(&mut f, c, tip, ACCENT, 3.);
        let along = self.from_cross > 89.9;
        // The angle between the cross slide and the top slide.
        if self.from_cross > 0.5 {
            let up = if self.out {
                std::f32::consts::FRAC_PI_2
            } else {
                -std::f32::consts::FRAC_PI_2
            };
            let far = up + v * a;
            let (a0, a1) = if far < up { (far, up) } else { (up, far) };
            let arc = canvas::Path::new(|b| {
                b.arc(canvas::path::Arc {
                    center: c,
                    radius: l * 0.45,
                    start_angle: iced::Radians(a0),
                    end_angle: iced::Radians(a1),
                })
            });
            f.stroke(&arc, stroke(ACCENT, 1.4));
        }
        // The number left of the top slide's arrow, the arc showing which
        // angle it is: along the bed, the quarter from the cross slide.
        let mid = Point::new(c.x - a.sin() * l * 0.55, c.y + v * a.cos() * l * 0.55);
        let at = if along {
            Point::new(c.x - l * 0.48, c.y - l * 0.42)
        } else {
            Point::new(mid.x - 8., mid.y)
        };
        let label = format!("{}°", trimmed(self.from_cross));
        word(&mut f, &label, at, ACCENT, 13., Align::Right, bold_font());
        // Which way the arrow takes the tool.
        // Which way the arrow takes the tool (along the bed: to the chuck,
        // as its word says).
        if !along {
            // Above the tip ending just right of it, clear of "cross
            // slide"; below it, under the cross slide's line.
            let (way, at, align) = if self.out {
                (
                    "away from the axis",
                    Point::new(tip.x + 10., bottom + 9.),
                    Align::Center,
                )
            } else {
                (
                    "toward the axis",
                    Point::new(tip.x + 8., tip.y - 9.),
                    Align::Right,
                )
            };
            word(&mut f, way, at, ACCENT, 10., align, Font::DEFAULT);
        }
        vec![f.into_geometry()]
    }
}
/// The top slide's angle as a small picture over its words: "29.5° from
/// the cross slide", or "along the bed (90° from the cross slide)".
fn top_angle<'a>(from_cross: f32, out: bool) -> Element<'a, Message> {
    let along = (from_cross - 90.).abs() < 1e-3;
    let words = if along {
        "along the bed\n(90° from the cross slide)".to_string()
    } else {
        format!("{}° from the cross slide", trimmed(from_cross))
    };
    column![
        crate::fault::canvas(TopAngle {
            from_cross: if along { 90. } else { from_cross },
            out,
        })
        .width(150)
        .height(110),
        text(words).size(12).color(MUTED).center()
    ]
    .spacing(2)
    .align_x(Alignment::Center)
    .width(170)
    .into()
}
/// A stop on a picture as the carriage dial measures it: from the end the
/// dial was zeroed on, "shoulder 64 from the end", the same number as the
/// dial's turns and reading, never the Z from the end the setup began with.
pub(super) fn from_end(what: &str, carriage: &Dial) -> String {
    format!("{what} {} from the end", mm(carriage.distance))
}
/// What a part off or a face to the centre asks of the tool's height.
const ON_CENTRE: &str = "on centre height";
/// What the set up asks of every tool once.
const EVERY_ON_CENTRE: &str = "set every tool on centre";
/// Whether operation `op` cuts to the centre: a part off, or a face to Ø0.
fn to_centre(p: &Page<'_>, op: usize) -> bool {
    let Some(o) = p.setup.operations.get(op) else {
        return false;
    };
    match crate::model::get(&o.p, "turn_kind") {
        "Part off" => true,
        "Face" => num(&o.p, "to_diameter").unwrap_or(0.) <= 1e-3,
        _ => false,
    }
}
/// The top slide along the bed, named, as a thread's angle is, from the
/// cross slide.
const ALONG: &str = "top slide along the bed (90° from the cross slide)";
/// The chip of the top slide's setting for a Face or a Chamfer: along the
/// bed, or set back to it from the angle a thread left it at.
fn set_back<'a>(swung: Option<f32>) -> Element<'a, Message> {
    match swung {
        Some(a) => warn_chip(format!(
            "top slide from {}° back along the bed (90° from the cross slide)",
            mm(a)
        )),
        None => chip(ALONG),
    }
}
/// A slide's name on the screens.
fn slide_name(slide: Slide) -> &'static str {
    match slide {
        Slide::Cross => "cross slide",
        Slide::Top => "top slide",
        Slide::Carriage => "carriage",
        Slide::Quill => "quill",
    }
}
/// The dial's numbers in a turn and its divisions on `machine`.
fn collar(machine: &crate::model::Machine, slide: Slide) -> (f32, f32) {
    let (t, d) = match slide {
        Slide::Cross => ("cross_turn", "cross_div"),
        Slide::Top => ("top_turn", "top_div"),
        Slide::Carriage => ("carriage_turn", "carriage_div"),
        Slide::Quill => ("quill_turn", "quill_div"),
    };
    (
        num(&machine.p, t).unwrap_or(1.),
        num(&machine.p, d).unwrap_or(100.),
    )
}
/// What a dial wound out reads, its collar counting down: the large text,
/// "reads 2.20", and the turns beside it, "2 turns out" (LIO-390).
fn out_read(p: &Page<'_>, d: &Dial) -> (String, Vec<String>) {
    let (t, _) = d.reading.clone().unwrap_or((0, 0.));
    let reads = fixed(d.reads.unwrap_or(0.), places_of(p, d.slide));
    (
        format!("reads {reads}"),
        vec![match t {
            1 => "1 turn out".into(),
            t => format!("{t} turns out"),
        }],
    )
}
/// The decimals the `slide`'s collar shows.
fn places_of(p: &Page<'_>, slide: Slide) -> usize {
    let (turn, div) = collar(&p.setup.machine, slide);
    places(turn, div)
}
/// What the page says of a dial's number.
struct Reading {
    /// The whole turns and the reading, when there is a number.
    at: Option<(u32, f32)>,
    /// The number the dial goes to from its zero, in its own numbers
    /// (turns × a turn + the reading): "6.00"; wound out, the turns and
    /// what the collar reads, "2 t out, 2.20", as it counts down.
    total: String,
    /// "1 turn + 2.00", "2 t out, 2.20".
    short: String,
    /// "1 turn + 2.00", "2 turns out, reads 2.20".
    long: String,
}
fn reading(p: &Page<'_>, d: &Dial) -> Reading {
    let (turn, div) = collar(&p.setup.machine, d.slide);
    let k = places(turn, div);
    match &d.reading {
        Ok((t, r)) if d.out => {
            let reads = fixed(d.reads.unwrap_or(0.), k);
            let turns = if *t == 1 { "turn" } else { "turns" };
            let short = format!("{t} t out, {reads}");
            Reading {
                at: Some((*t, *r)),
                total: short.clone(),
                short,
                long: format!("{t} {turns} out, reads {reads}"),
            }
        }
        Ok((t, r)) => {
            let reading = fixed(*r, k);
            let turns = match t {
                1 => "1 turn".to_string(),
                t => format!("{t} turns"),
            };
            Reading {
                at: Some((*t, *r)),
                total: fixed(*t as f32 * turn + r, k),
                short: if *t == 0 {
                    reading.clone()
                } else {
                    // One way everywhere: "3 turns + 4.0" (LIO-426).
                    format!("{turns} + {reading}")
                },
                long: if *t == 0 {
                    reading.clone()
                } else {
                    format!("{turns} + {reading}")
                },
            }
        }
        Err(e) => Reading {
            at: None,
            total: "?".into(),
            short: "?".into(),
            long: e.clone(),
        },
    }
}
/// A dial picture with its title above ("cross slide · 1 turn +") and `sub`
/// under it.
fn dial_box<'a>(p: &Page<'a>, d: &Dial, sub: String, size: f32) -> Element<'a, Message> {
    let (turn, div) = collar(&p.setup.machine, d.slide);
    let r = reading(p, d);
    let name = slide_name(d.slide);
    let title = match r.at {
        Some(_) if d.out => r.long.clone(),
        Some((0, _)) | None => name.to_string(),
        Some((1, _)) => format!("{name} · 1 turn +"),
        Some((t, _)) => format!("{name} · {t} turns +"),
    };
    let face = DialFace {
        turn,
        div: div.max(1.) as u32,
        reading: match (&d.reading, d.out) {
            (Ok(_), true) => Ok(d.reads.unwrap_or(0.)),
            (Ok((_, v)), false) => Ok(*v),
            (Err(e), _) => Err(e.clone()),
        },
        out: d.out,
    };
    // Wound out, the title is the turns and the reading; the slide is named
    // under the dial.
    let sub = if d.out {
        format!("{name} · {sub}")
    } else {
        sub
    };
    column![
        bold(title, 13),
        crate::fault::canvas(face).width(size).height(size + 10.),
        text(sub).size(12).color(MUTED).center()
    ]
    .spacing(2)
    .align_x(Alignment::Center)
    .width(if d.out { size + 40. } else { size + 24. })
    .into()
}
/// How wide a dial is drawn, px (LIO-389): as wide as `n` pictures side by
/// side leave of the right-hand column (`frame`'s 9 parts of 20 of the card
/// beside the setups list), and no taller than the window leaves above a
/// Pass screen's three readouts, so that at 1024 × 700 they are all in
/// sight; 84 to 260.
fn dial_size(p: &Page<'_>, n: f32) -> f32 {
    let (pictures, numbers) = split(p);
    let column = (p.app.window_width - 300.) * f32::from(numbers) / f32::from(pictures + numbers);
    (column / n - 30.)
        .min(p.app.window_height - 608.)
        .clamp(84., 260.)
}
/// Whether the window is narrow: the lathe's numbers then take half the card
/// (`split`) and its readouts' keys only their width (LIO-389).
pub(super) fn narrow(p: &Page<'_>) -> bool {
    p.app.window_width < 1200.
}
/// How the side view and the numbers share a lathe screen's card, as
/// `steps::frame` has it: half each at a narrow window, so that a Pass
/// screen's readouts and both dials are in sight at 1024 × 700.
pub(super) fn split(p: &Page<'_>) -> (u16, u16) {
    if narrow(p) { (1, 1) } else { (11, 9) }
}
/// The dials side by side, centred.
fn dials<'a>(items: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    container(row(items).spacing(14))
        .center_x(iced::Length::Fill)
        .into()
}
/// A big number in a readout.
fn big<'a>(s: impl Into<String>, state: Lit) -> Element<'a, Message> {
    text(s.into())
        .size(26)
        .font(Font {
            weight: iced::font::Weight::Bold,
            ..Font::MONOSPACE
        })
        .color(match state {
            Lit::Done => super::DONE,
            Lit::Now => ACCENT,
            Lit::Next => INK,
        })
        .wrapping(text::Wrapping::None)
        .into()
}
/// One line of what to do: its order, what it is, the big number (or a
/// box to type it) and short tags; tapping it lights the next.
fn line<'a>(
    narrow: bool,
    order: Option<u8>,
    key: &str,
    value: Element<'a, Message>,
    tags: Vec<String>,
    state: Lit,
    msg: Option<Message>,
) -> Element<'a, Message> {
    let now = state == Lit::Now;
    // At a narrow window the key takes only its width, so that the tags
    // keep room beside the number (LIO-389).
    let gap = if narrow { 6. } else { 10. };
    let mut l = row![].spacing(gap).align_y(Alignment::Center);
    if let Some(n) = order {
        let mark = if state == Lit::Done {
            "✓".to_string()
        } else {
            n.to_string()
        };
        l = l.push(disc(mark, now, 22.));
    }
    l = l
        .push(
            text(key.to_string())
                .size(13)
                .color(MUTED)
                .width(if narrow {
                    iced::Length::Shrink
                } else {
                    iced::Length::Fixed(78.)
                }),
        )
        .push(value)
        .push(space().width(iced::Length::Fill));
    let mut t = column![].spacing(1).align_x(Alignment::End);
    for tag in tags {
        t = t.push(text(tag).size(12).color(MUTED));
    }
    l = l.push(t);
    let face = super::boxed(l.padding([5, 10]), now).width(iced::Length::Fill);
    match msg {
        Some(m) => super::tap(face, Some(m), "Reached"),
        None => face.into(),
    }
}
/// The box to type a measured diameter in, under `key`, and the button
/// that keeps it.
fn measure_box<'a>(
    p: &Page<'a>,
    key: &str,
    measured: Option<f32>,
    expect: f32,
) -> Element<'a, Message> {
    let typed = match &p.app.guide_typed {
        Some((k, t)) if k == key => t.clone(),
        _ => measured.map_or(String::new(), |m| format!("{m:.2}")),
    };
    let value = (typed.trim().replace(',', ".").parse::<f32>().ok())
        .filter(|v| v.is_finite() && *v > 0. && *v < 10_000.);
    let keep = value.map(|v| Message::Guide(GuideAction::Measured(key.into(), v)));
    let k = key.to_string();
    let input = text_input(&format!("{expect:.2}"), &typed)
        .on_input(move |t| Message::Guide(GuideAction::Typed(k.clone(), t)))
        .on_submit_maybe(keep.clone())
        .width(110)
        .padding([5, 8])
        .size(22)
        .style(|theme, status| {
            let mut style = iced::widget::text_input::default(theme, status);
            style.background = Color::WHITE.into();
            style.value = INK;
            style.border.color = ACCENT;
            style.border.width =
                if matches!(status, iced::widget::text_input::Status::Focused { .. }) {
                    2.
                } else {
                    1.2
                };
            style
        })
        .font(Font {
            weight: iced::font::Weight::Bold,
            ..Font::MONOSPACE
        });
    let mut r = row![
        text("Ø").size(22).color(ACCENT),
        help::field(input, "measured", "Measured Ø · mm", Target::Machine)
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    match keep {
        Some(m) if value != measured => r = r.push(primary("Keep ▶", m)),
        Some(_) => r = r.push(text("✓ kept").size(12).color(MUTED)),
        None if !typed.trim().is_empty() => {
            r = r.push(text("a number in mm").size(12).color(super::NEVER))
        }
        None => {}
    }
    r.into()
}
/// How the tool with this id is drawn.
fn cutter_of(p: &Page<'_>, id: &str) -> Cutter {
    let tool = p.app.doc.tools.iter().find(|t| t.id == id);
    match tool.map_or("", |t| crate::model::get(&t.p, "type")) {
        "Boring bar" => Cutter::Bore,
        "Threading tool" => Cutter::Thread,
        "Grooving tool" | "Parting blade" => {
            Cutter::Blade(tool.and_then(|t| num(&t.p, "width")).unwrap_or(3.))
        }
        _ => Cutter::Turn,
    }
}
/// The chip of the tool with this id.
fn tool_chip<'a>(p: &Page<'a>, id: &str) -> Element<'a, Message> {
    chip(
        (p.app.doc.tools.iter())
            .find(|t| t.id == id)
            .map_or(id.to_string(), |t| t.name().to_string()),
    )
}
fn rpm_chip<'a>(rpm: f32) -> Element<'a, Message> {
    chip(format!("{} rpm", rpm.round()))
}
/// A diameter: "Ø40.00".
fn dia(d: f32) -> String {
    format!("Ø{d:.2}")
}
/// A length without trailing zeros, a true minus.
fn mm(v: f32) -> String {
    super::mm(v)
}
/// The header's chips of a lathe operation's section: its tool, how many
/// passes or plunges, the tailstock for a drill, the minutes.
pub(super) fn header<'a>(p: &Page<'a>, op: usize, s: &Section) -> Vec<Element<'a, Message>> {
    let g = p.guide();
    let mut out = vec![];
    let first = s.steps.clone().find_map(|i| match &g.steps[i] {
        Step::Lathe(l) if !matches!(l, LatheStep::Touch { .. }) => Some(l),
        _ => None,
    });
    let tool = p.setup.operations.get(op).map_or("", |o| o.tool.as_str());
    match first {
        Some(LatheStep::TailDrill { .. }) => {
            out.push(chip(format!(
                "centre drill, then {}",
                (p.app.doc.tools.iter())
                    .find(|t| t.id == tool)
                    .map_or("the drill", |t| t.name())
            )));
            out.push(chip("tailstock"));
        }
        Some(LatheStep::TailTap { .. }) => {
            out.push(tool_chip(p, tool));
            out.push(chip("tailstock"));
        }
        Some(LatheStep::DieThread { .. }) => {
            out.push(tool_chip(p, tool));
            out.push(chip("by hand"));
        }
        _ => out.push(tool_chip(p, tool)),
    }
    if s.count > 1 {
        let what = match first {
            Some(LatheStep::Plunge { .. }) => "plunges",
            _ => "passes",
        };
        out.push(chip(format!("{} {what}", s.count)));
    }
    out.push(chip(format!("~{} min", s.minutes.ceil())));
    out
}
/// The touch the cross dial of operation `op` was zeroed on: the Touch
/// step's diameter before the step `at`, in its section.
fn touched(g: &Guide, op: usize, at: usize) -> Option<f32> {
    (0..at).rev().find_map(|i| match &g.steps[i] {
        Step::Lathe(LatheStep::Touch {
            op: o,
            cross: Some(c),
            ..
        }) if *o == op => Some(c.diameter()),
        _ => None,
    })
}
/// The feeds along the axis of the plan of the `k`th operation the setup
/// runs: (radius, work Z of the pass's end).
fn alongs(g: &Guide, k: usize) -> Vec<(f32, f32)> {
    let Some(d) = g.done.get(k) else {
        return vec![];
    };
    (d.plan.moves.iter())
        .filter(|m| m.motion == Motion::Feed && (m.from[0] - m.to[0]).abs() < 1e-6)
        .map(|m| (m.to[0], turn::work_z(d.plan.stock, m.to[2])))
        .collect()
}
type Screen<'a> = (
    String,
    &'static str,
    Element<'a, Message>,
    Column<'a, Message>,
);
/// The screen of a lathe step: the side view on the left, the dials and the
/// numbers in order on the right.
pub(super) fn screen<'a>(p: &Page<'a>, step: &'a LatheStep) -> Screen<'a> {
    let g = p.guide();
    let Some(side) = p.shown.side.as_ref() else {
        return (
            step.title().into(),
            "",
            space().into(),
            column![muted("No side view of the bar.")],
        );
    };
    let start = side.bars.first().map_or(&[][..], |b| b.as_slice());
    // The side view fills the room its panel has (LIO-389).
    let view = |bar: &'a [[f32; 4]], marks: Vec<Mark>| -> Element<'a, Message> {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        (p.key, p.at, p.item, "lathe").hash(&mut h);
        crate::fault::canvas(SideView {
            key: h.finish(),
            side,
            bar,
            marks,
        })
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .into()
    };
    let note = |s: String| -> Element<'a, Message> { text(s).size(12).color(MUTED).into() };
    let item = p.item;
    let narrow = narrow(p);
    let reached = |i: usize| Some(Message::Guide(GuideAction::Reached(i)));
    let machine = &p.setup.machine;
    let k_of = |op: usize| p.done(op);
    let bar_of = |op: usize| side.bars.get(k_of(op)).map_or(start, |b| b.as_slice());
    let rings_of = |op: usize| side.cuts.get(k_of(op)).map_or(&[][..], |c| c.as_slice());
    let all = |op: usize| -> Vec<[f32; 4]> { rings_of(op).iter().flatten().copied().collect() };
    match step {
        LatheStep::DialCheck {
            divisions,
            reading: dialed,
            length,
        } => {
            let end = end_of(start).max(0.);
            let r = outer_at(start, end);
            let z = end - length;
            let marks = vec![
                Mark::Cut([z, end, r - (dialed / 2.).max(0.25), r]),
                Mark::Path([end + 2., r], [z + 1., r]),
                Mark::Tool(Cutter::Turn, [z, r]),
                Mark::Word(
                    [z - 2., r + 4.],
                    format!("{divisions} div in"),
                    ACCENT,
                    Align::Right,
                ),
            ];
            let (turn, div) = collar(machine, Slide::Cross);
            let answer = |times: f32, reads: Reads, what: &str| -> Element<'a, Message> {
                let face = DialFace {
                    turn,
                    div: div.max(1.) as u32,
                    reading: Ok(*dialed),
                    out: false,
                };
                column![
                    bold(format!("Ø went down by {}", fixed(dialed * times, 2)), 13),
                    crate::fault::canvas(face).width(130).height(140),
                    primary(
                        format!("by {}× → {what}", times as u32),
                        Message::Guide(GuideAction::CrossReads(reads))
                    ),
                ]
                .spacing(4)
                .align_x(Alignment::Center)
                .width(170)
                .into()
            };
            let right = column![
                label("Turn a light skin, then:"),
                line(
                    narrow,
                    Some(1),
                    "cross slide",
                    big(format!("{divisions} div"), lit(0, item)),
                    vec![
                        "in from the touch".into(),
                        format!("cut {} long", mm(*length))
                    ],
                    lit(0, item),
                    reached(0),
                ),
                line(
                    narrow,
                    Some(2),
                    "measure",
                    big("Ø ?", lit(1, item)),
                    vec!["with the calipers".into()],
                    lit(1, item),
                    reached(1),
                ),
                label("The Ø went down by"),
                dials(vec![
                    answer(1., Reads::Diameter, "diameter"),
                    answer(2., Reads::Radius, "radius"),
                ]),
            ];
            (
                "Check the cross-slide dial".into(),
                "once per machine: diameter or radius?",
                view(start, marks),
                right,
            )
        }
        LatheStep::Hold {
            jaws,
            grip,
            stickout,
            diameter,
            length,
            reach,
            ..
        } => {
            let g = diameter / 2.;
            let jaw = (outer_of(start).max(g) * 0.45).clamp(5., 25.);
            let marks = vec![
                Mark::Span(
                    [-stickout, g + jaw + 2.],
                    [0., g + jaw + 2.],
                    format!("{} out of the jaws", mm(*stickout)),
                ),
                Mark::Word(
                    [-stickout - grip / 2., -g - jaw - 4.],
                    format!("{} in the jaws", mm(*grip)),
                    INK,
                    Align::Center,
                ),
            ];
            let mut c = vec![
                chip(format!("{jaws}-jaw chuck")),
                chip(format!("{} × {} bar", dia(*diameter), mm(*length))),
            ];
            if *jaws == 4 {
                c.push(warn_chip("4-jaw: centre it with the dial gauge"));
            }
            // Said once in the set up: on the Zero, else here.
            let zero =
                (p.guide().steps.iter()).any(|s| matches!(s, Step::Lathe(LatheStep::Zero { .. })));
            if !zero {
                c.push(warn_chip(EVERY_ON_CENTRE));
            }
            let right = column![
                chips(c),
                line(
                    narrow,
                    None,
                    "stick out",
                    text(mm(*stickout))
                        .size(40)
                        .font(Font {
                            weight: iced::font::Weight::Bold,
                            ..Font::MONOSPACE
                        })
                        .color(ACCENT)
                        .into(),
                    vec![
                        "from the jaws to the end".into(),
                        format!("the cuts reach {}", mm(*reach)),
                    ],
                    Lit::Now,
                    None,
                ),
                line(
                    narrow,
                    None,
                    "in the jaws",
                    big(mm(*grip), Lit::Next),
                    vec!["grip".into()],
                    Lit::Next,
                    None,
                ),
            ];
            (
                "Hold".into(),
                "the bar in the chuck",
                view(start, marks),
                right,
            )
        }
        LatheStep::Zero {
            tool,
            key,
            planned,
            measured,
            cross,
            measure,
        } => {
            let end = end_of(start).max(0.);
            let r = outer_at(start, end);
            let mut marks = vec![
                Mark::Tool(cutter_of(p, tool), [end, r * 0.45]),
                Mark::Word(
                    [end + 1., r * 0.15],
                    "touch the end".into(),
                    ACCENT,
                    Align::Left,
                ),
            ];
            if *cross {
                marks.push(Mark::Dot(
                    [end - (r * 0.5).clamp(4., 15.), r],
                    "touch the Ø".into(),
                ));
            } else if *measure {
                marks.push(Mark::Across(
                    end - (r * 0.9).clamp(8., 30.),
                    r,
                    "Ø ?".into(),
                ));
            }
            let mut right = column![
                chips(vec![tool_chip(p, tool), warn_chip(EVERY_ON_CENTRE)]),
                label(match (*cross, *measure) {
                    (true, _) => "Touch, then zero the collars",
                    (false, true) => "Touch the end, measure the bar",
                    (false, false) => "Touch the end",
                }),
                line(
                    narrow,
                    Some(1),
                    "carriage",
                    big("0", lit(0, item)),
                    vec!["touch the end".into(), "zero the carriage dial".into()],
                    lit(0, item),
                    reached(0),
                ),
            ];
            // Only when an outside touch reads from it (LIO-203).
            if *measure {
                right = right.push(line(
                    narrow,
                    Some(2),
                    "measure",
                    measure_box(p, key, *measured, *planned),
                    vec!["type what the".into(), "calipers read".into()],
                    lit(1, item),
                    None,
                ));
            }
            if *cross {
                right = right.push(line(
                    narrow,
                    Some(3),
                    "cross slide",
                    big("0", lit(2, item)),
                    vec!["touch the Ø".into(), "zero the cross dial".into()],
                    lit(2, item),
                    reached(2),
                ));
            }
            (
                "Zero".into(),
                if *cross {
                    "touch, then zero both collars"
                } else {
                    "the carriage; the hole is touched later"
                },
                view(start, marks),
                right,
            )
        }
        LatheStep::Touch {
            op,
            tool,
            rpm,
            carriage,
            cross,
            top,
            ..
        } => {
            let bar = bar_of(*op);
            let end = end_of(bar);
            let r = outer_at(bar, end);
            let mut marks = vec![];
            let mut rows = vec![];
            let mut n = 0;
            let mut next = |key: &str, value: String, tags: Vec<String>| {
                let i = n;
                n += 1;
                line(
                    narrow,
                    Some(i as u8 + 1),
                    key,
                    big(value, lit(i, item)),
                    tags,
                    lit(i, item),
                    reached(i),
                )
            };
            if let Some(a) = top {
                rows.push(next(
                    "top slide",
                    format!("{}°", mm(*a)),
                    vec!["from the cross slide".into(), "zero it on the touch".into()],
                ));
            }
            if *carriage {
                // A blade touches the end with its left side, the side its
                // carriage readings are of.
                let kind = cutter_of(p, tool);
                let z = match kind {
                    Cutter::Blade(w) => end + w / 2.,
                    _ => end,
                };
                marks.push(Mark::Tool(kind, [z, r * 0.45]));
                marks.push(Mark::Word(
                    [end + 1., r * 0.15],
                    "touch the end".into(),
                    ACCENT,
                    Align::Left,
                ));
                rows.push(next(
                    "carriage",
                    "0".into(),
                    vec!["touch the end".into(), "zero the carriage dial".into()],
                ));
            }
            if let Some(c) = cross {
                let d = c.diameter();
                let z = end - (r * 0.5).clamp(3., 12.);
                let what = if c.inside { "bore" } else { "Ø" };
                marks.push(Mark::Dot([z, d / 2.], format!("touch the {what}")));
                let mut tags = vec![format!("touch {}", dia(d))];
                if c.measured.is_some() {
                    tags[0].push_str(" measured");
                }
                tags.push("zero the cross dial".into());
                rows.push(next("cross slide", "0".into(), tags));
            }
            let mut c = vec![tool_chip(p, tool), rpm_chip(*rpm)];
            if to_centre(p, *op) {
                c.push(warn_chip(ON_CENTRE));
            }
            let mut right = column![chips(c)];
            if let Some(a) = top {
                // Swing the top slide first: its angle from the cross slide.
                let out = cross.as_ref().is_some_and(|c| c.inside);
                right = right.push(dials(vec![top_angle(*a, out)]));
            }
            let right = right.push(label("Touch, then zero")).extend(rows);
            (
                "Touch".into(),
                "zero the dials for this tool",
                view(bar, marks),
                right,
            )
        }
        LatheStep::Face {
            op,
            tool,
            rpm,
            face_off,
            to_diameter,
            top,
            swung,
            ..
        } => {
            let bar = bar_of(*op);
            let pieces = all(*op);
            let e = extent(&pieces).unwrap_or([end_of(bar) - face_off, end_of(bar), 0., 1.]);
            let mut marks: Vec<Mark> = pieces.iter().map(|g| Mark::Cut(*g)).collect();
            marks.push(Mark::Path(
                [e[0], e[3] + 1.],
                [e[0], to_diameter / 2. + 0.5],
            ));
            marks.push(Mark::Tool(Cutter::Turn, [e[0], e[3] + 1.]));
            marks.push(Mark::Word(
                [e[0] - 1.5, e[3] + 2.5],
                format!("{} off", fixed(*face_off, 2)),
                ACCENT,
                Align::Right,
            ));
            let mut face_dials = vec![dial_box(
                p,
                top,
                format!("in {} from the touch", fixed(*face_off, 2)),
                dial_size(p, 2.),
            )];
            if swung.is_some() {
                face_dials.push(top_angle(90., false));
            }
            let mut c = vec![tool_chip(p, tool), rpm_chip(*rpm)];
            if *to_diameter <= 1e-3 {
                // To the centre: a tool above or below it leaves a pip.
                c.push(warn_chip(ON_CENTRE));
            }
            let right = column![
                chips(c),
                chips(vec![set_back(*swung), warn_chip("lock the carriage")]),
                dials(face_dials),
                line(
                    narrow,
                    Some(1),
                    "top slide",
                    big(reading(p, top).total, lit(0, item)),
                    vec!["touch the end, zero".into(), "then in, face across".into()],
                    lit(0, item),
                    reached(0),
                ),
                line(
                    narrow,
                    Some(2),
                    "carriage",
                    big("0", lit(1, item)),
                    vec!["on the new end".into(), "zero the carriage dial".into()],
                    lit(1, item),
                    reached(1),
                ),
            ];
            (
                "Face".into(),
                "the only screen: the tool and how much",
                view(bar, marks),
                right,
            )
        }
        LatheStep::Pass {
            op,
            index,
            of,
            bore,
            diameter,
            rpm,
            feed,
            cross,
            carriage,
            feed_off,
            to_jaws,
        } => {
            let bar = bar_of(*op);
            let end = end_of(bar);
            let rings = rings_of(*op);
            let i = index - 1;
            let first = *index == 1;
            let mut marks = vec![];
            for (j, pieces) in rings.iter().enumerate() {
                for g in pieces {
                    marks.push(match j.cmp(&i) {
                        std::cmp::Ordering::Less => Mark::Earlier(*g),
                        std::cmp::Ordering::Equal => Mark::Cut(*g),
                        std::cmp::Ordering::Greater => Mark::Later(*g),
                    });
                }
            }
            let along = alongs(g, k_of(*op));
            let r = diameter / 2.;
            let z_end = along.get(i).map_or(end - 10., |a| a.1);
            let big_r = outer_of(bar);
            let shoulder = bar.iter().map(|s| s[0]).fold(end, f32::min) + 1e-3 < z_end;
            if shoulder {
                let (a, b) = if *bore {
                    ([z_end, 0.], [z_end, r + 1.5])
                } else {
                    ([z_end, r - 1.5], [z_end, big_r + 2.])
                };
                let what = if *bore { "bottom" } else { "shoulder" };
                marks.push(Mark::Never(a, b, from_end(what, carriage)));
            }
            let finish = along.last().map_or(*diameter, |a| 2. * a.0);
            marks.push(Mark::Path([end + 2., r], [z_end + 1., r]));
            marks.push(Mark::Tool(
                if *bore { Cutter::Bore } else { Cutter::Turn },
                [end + 2., r],
            ));
            marks.push(Mark::Word(
                [(z_end + end) / 2., big_r + 4.],
                format!("→ {}", dia(finish)),
                INK,
                Align::Center,
            ));
            if first {
                // Under power feed: how far the stop leaves the tool from
                // the jaws, below the bar.
                let y = -(big_r + 3.);
                marks.push(Mark::Span([z_end - to_jaws, y], [z_end, y], String::new()));
                marks.push(Mark::Word(
                    [z_end + 1., y - 4.],
                    format!("{} to the jaws", mm(*to_jaws)),
                    NEVER,
                    Align::Left,
                ));
            }
            let from = touched(g, *op, p.at).unwrap_or(2. * big_r);
            let depth = along.first().map_or(0., |a| (2. * a.0 - from).abs());
            let mut left = column![
                view(bar, marks),
                note(format!(
                    "{} → {} · {} long · {} off the Ø a pass · the last after you measure",
                    dia(from),
                    dia(finish),
                    mm(end - z_end),
                    fixed(depth, 2)
                ))
            ]
            .spacing(4);
            if first {
                // Before the first power feed, under the picture of how
                // far the jaws are.
                left = left.push(chips(vec![warn_chip(if *bore {
                    "set the carriage stop at the bottom"
                } else {
                    "set the carriage stop at the shoulder"
                })]));
            }
            let left = left.into();
            // The measurement the last pass reads from.
            let measured = (g.steps.iter()).find_map(|s| match s {
                Step::Lathe(LatheStep::Measure {
                    op: o,
                    measured: Some(m),
                    ..
                }) if o == op => Some(*m),
                _ => None,
            });
            let way = if *bore { "out" } else { "in" };
            let c = reading(p, cross);
            let at = reading(p, carriage);
            let (cross_big, mut cross_tags) = match c.at {
                Some(_) if cross.out => out_read(p, cross),
                Some(_) => (c.total.clone(), vec![format!("{} on the dial", c.long)]),
                None => (c.total.clone(), vec![c.long.clone()]),
            };
            if index == of
                && let Some(m) = measured
            {
                cross_tags.push(format!("from the {} measured", dia(m)));
            }
            // The carriage as the cross slide: the number big, the dial's
            // turns under it, so "3 turns + 4.0" fits at 1024 (LIO-426).
            let mut carriage_tags = vec![];
            if at.at.is_some() {
                carriage_tags.push(format!("{} on the dial", at.long));
            }
            if let Some(off) = feed_off {
                carriage_tags.push(format!("◀ feed off at {}", reading(p, off).short));
                carriage_tags.push("last 1 mm by hand".into());
            }
            let mut c_chips = vec![tool_chip(p, &p.setup.operations[*op].tool), rpm_chip(*rpm)];
            if *feed > 0. {
                c_chips.push(chip(format!("power feed {}", fixed(*feed, 2))));
            }

            let right = column![
                chips(c_chips),
                dials(vec![
                    dial_box(
                        p,
                        cross,
                        if cross.out {
                            format!("out to {}", dia(*diameter))
                        } else {
                            format!("{way} to {} ({})", c.total, dia(*diameter))
                        },
                        dial_size(p, 2.)
                    ),
                    dial_box(
                        p,
                        carriage,
                        format!("stop at {}", at.short),
                        dial_size(p, 2.)
                    ),
                ]),
                line(
                    narrow,
                    Some(1),
                    "back off",
                    big("—", lit(0, item)),
                    vec!["carriage right".into(), "of the end".into()],
                    lit(0, item),
                    reached(0),
                ),
                line(
                    narrow,
                    Some(2),
                    "cross slide",
                    big(cross_big, lit(1, item)),
                    cross_tags,
                    lit(1, item),
                    reached(1),
                ),
                line(
                    narrow,
                    Some(3),
                    "carriage",
                    big(at.total.clone(), lit(2, item)),
                    carriage_tags,
                    lit(2, item),
                    reached(2),
                ),
                pass_tiles(p, *op),
            ];
            (
                format!("Pass {index} of {of}"),
                "one screen per pass",
                left,
                right,
            )
        }
        LatheStep::Measure {
            op,
            key,
            expect,
            target,
            bore,
            measured,
        } => {
            let bar = bar_of(*op);
            let end = end_of(bar);
            let rings = rings_of(*op);
            let n = rings.len();
            let mut marks = vec![];
            for (j, pieces) in rings.iter().enumerate() {
                for g in pieces {
                    marks.push(if j + 1 < n {
                        Mark::Earlier(*g)
                    } else {
                        Mark::Later(*g)
                    });
                }
            }
            let along = alongs(g, k_of(*op));
            let z_end = along.last().map_or(end - 10., |a| a.1);
            let z = (z_end + end) / 2.;
            marks.push(Mark::Across(
                z,
                expect / 2.,
                match measured {
                    Some(m) => dia(*m),
                    None => "Ø ?".into(),
                },
            ));
            let what = if *bore { "bore" } else { "Ø" };
            // The last pass, as it reads from what was typed.
            let last = (g.steps.iter().skip(p.at)).find_map(|s| match s {
                Step::Lathe(LatheStep::Pass {
                    op: o,
                    index,
                    of,
                    cross,
                    ..
                }) if o == op && index == of => Some(cross),
                _ => None,
            });
            let mut right = column![
                label(format!("Measure the {what}")),
                line(
                    narrow,
                    None,
                    "about",
                    big(dia(*expect), Lit::Next),
                    vec!["as the passes left it".into()],
                    Lit::Next,
                    None,
                ),
                line(
                    narrow,
                    Some(1),
                    "measured",
                    measure_box(p, key, *measured, *expect),
                    vec!["type what the".into(), "calipers read".into()],
                    Lit::Now,
                    None,
                ),
            ];
            if let Some(cross) = last {
                let c = reading(p, cross);
                let way = if *bore { "out" } else { "in" };
                right = right
                    .push(label("Then the last pass"))
                    .push(dials(vec![dial_box(
                        p,
                        cross,
                        if cross.out {
                            format!("out → {}", dia(*target))
                        } else {
                            format!("{way} to {} → {}", c.total, dia(*target))
                        },
                        dial_size(p, 1.),
                    )]));
            }
            right = right.push(pass_tiles(p, *op));
            (
                "Measure".into(),
                "before the last pass",
                view(bar, marks),
                right,
            )
        }
        LatheStep::TailDrill {
            op,
            tool,
            rpm,
            diameter,
            depth,
            point,
            peck,
            quill,
        } => {
            let bar = bar_of(*op);
            let pieces = all(*op);
            let e =
                extent(&pieces).unwrap_or([end_of(bar) - depth, end_of(bar), 0., diameter / 2.]);
            let mut marks: Vec<Mark> = pieces.iter().map(|g| Mark::Cut(*g)).collect();
            marks.push(Mark::Tool(Cutter::Drill(diameter / 2.), [e[1], 0.]));
            marks.push(Mark::Word(
                [e[1] - depth / 2., diameter / 2. + 5.],
                format!("{} · {} deep", dia(*diameter), mm(*depth)),
                ACCENT,
                Align::Center,
            ));
            let q = reading(p, quill);
            let right = column![
                chips(vec![
                    tool_chip(p, tool),
                    rpm_chip(*rpm),
                    warn_chip("centre drill first")
                ]),
                dials(vec![dial_box(
                    p,
                    quill,
                    format!("{} deep + {} for the point", mm(*depth), fixed(*point, 2)),
                    dial_size(p, 1.)
                )]),
                line(
                    narrow,
                    Some(1),
                    "tailstock",
                    big("lock", lit(0, item)),
                    vec!["slide it up until".into(), "the drill touches".into()],
                    lit(0, item),
                    reached(0),
                ),
                line(
                    narrow,
                    Some(2),
                    "quill",
                    big(q.total.clone(), lit(1, item)),
                    vec![
                        format!("{} on the dial", q.long),
                        format!("peck every {}", mm(*peck)),
                    ],
                    lit(1, item),
                    reached(1),
                ),
            ];
            (
                "Drill".into(),
                "the tailstock quill",
                view(bar, marks),
                right,
            )
        }
        LatheStep::TailTap {
            op,
            tool,
            tap,
            pitch,
            depth,
            lead,
            turns,
            quill,
        } => {
            let bar = bar_of(*op);
            let pieces = all(*op);
            let r = crate::thread::spec(tap).map_or(3., |s| s.major / 2.);
            let e = extent(&pieces).unwrap_or([end_of(bar) - depth, end_of(bar), 0., r]);
            let mut marks: Vec<Mark> = pieces.iter().map(|g| Mark::Cut(*g)).collect();
            marks.push(Mark::Tool(Cutter::Tap(r), [e[1], 0.]));
            marks.push(Mark::Word(
                [e[1] - depth / 2., r + 5.],
                format!("{tap} · {} deep", mm(*depth)),
                ACCENT,
                Align::Center,
            ));
            let q = reading(p, quill);
            let right = column![
                chips(vec![
                    tool_chip(p, tool),
                    warn_chip("spindle off"),
                    chip("½ back every turn"),
                    chip("oil"),
                ]),
                dials(vec![dial_box(
                    p,
                    quill,
                    format!("{} deep + {} for the lead", mm(*depth), mm(*lead)),
                    dial_size(p, 1.)
                )]),
                line(
                    narrow,
                    Some(1),
                    "tailstock",
                    big("lock", lit(0, item)),
                    vec![
                        "tap in the chuck, or a wrench".into(),
                        "on the centre; touch the hole".into(),
                    ],
                    lit(0, item),
                    reached(0),
                ),
                line(
                    narrow,
                    Some(2),
                    "quill",
                    big(q.total.clone(), lit(1, item)),
                    vec![
                        format!("{} on the dial", q.long),
                        "the way in, then out".into()
                    ],
                    lit(1, item),
                    reached(1),
                ),
                line(
                    narrow,
                    Some(3),
                    "tap",
                    big(format!("{} turns", trim_turns(*turns)), lit(2, item)),
                    vec![
                        "turn the chuck or the wrench by hand".into(),
                        format!("{} mm a turn", mm(*pitch)),
                    ],
                    lit(2, item),
                    reached(2),
                ),
            ];
            (
                "Tap".into(),
                "by hand, the spindle off",
                view(bar, marks),
                right,
            )
        }
        LatheStep::DieThread {
            op,
            tool,
            die,
            pitch,
            diameter,
            od,
            length,
            turns,
        } => {
            let bar = bar_of(*op);
            let end = end_of(bar);
            let pieces = all(*op);
            let r = outer_at(bar, end - length / 2.).max(diameter / 2.);
            let mut marks: Vec<Mark> = pieces.iter().map(|g| Mark::Cut(*g)).collect();
            marks.push(Mark::Tool(
                Cutter::Die((*od / 2.).max(r + 3.)),
                [end + 1., 0.],
            ));
            marks.push(Mark::Path([end + 1., r + 2.], [end - length, r + 2.]));
            marks.push(Mark::Never(
                [end - length, r - 1.5],
                [end - length, (r + 4.).max(outer_of(bar) + 1.5)],
                format!("thread stops {} from the end", mm(*length)),
            ));
            marks.push(Mark::Word(
                [end - 2., -(r + 4.)],
                format!("{die} · {} long", mm(*length)),
                ACCENT,
                Align::Right,
            ));
            let right = column![
                chips(vec![
                    tool_chip(p, tool),
                    warn_chip("spindle off"),
                    chip("½ back every turn"),
                    chip("oil"),
                ]),
                line(
                    narrow,
                    Some(1),
                    "die",
                    big("square", lit(0, item)),
                    vec![
                        format!("{die} die in its holder"),
                        "quill against the holder".into(),
                    ],
                    lit(0, item),
                    reached(0),
                ),
                line(
                    narrow,
                    Some(2),
                    "holder",
                    big(format!("{} turns", trim_turns(*turns)), lit(1, item)),
                    vec![
                        "turn the holder by hand".into(),
                        format!("to {} from the end", mm(*length)),
                        format!("{} mm a turn", mm(*pitch)),
                    ],
                    lit(1, item),
                    reached(1),
                ),
            ];
            (
                "Die".into(),
                "by hand, the spindle off",
                view(bar, marks),
                right,
            )
        }
        LatheStep::Chamfer {
            op,
            tool,
            rpm,
            edge,
            size,
            top,
            carriage,
            swung,
        } => {
            let bar = bar_of(*op);
            let pieces = all(*op);
            let end = end_of(bar);
            let e = extent(&pieces).unwrap_or([end - size, end, 0., outer_of(bar)]);
            let mut marks: Vec<Mark> = pieces.iter().map(|g| Mark::Cut(*g)).collect();
            let inside = edge == "End inside";
            // The tool's point at the foot of its 45° edge, where the top
            // slide leaves it: `size` from the corner.
            marks.push(Mark::Tool(
                Cutter::Chamfer(inside),
                if inside {
                    [e[1], e[2] + size]
                } else {
                    [e[1], e[3] - size]
                },
            ));
            // Left of the tool drawn at the corner.
            marks.push(Mark::Word(
                [e[0] - 8., e[3] + 5.],
                format!("{} × 45°", mm(*size)),
                ACCENT,
                Align::Right,
            ));
            // A shoulder's: how far its foot is from the jaws, which the
            // tool's edge reaches toward (LIO-203).
            // The chamfer's foot is its size short of the shoulder (`e[1]`).
            let foot = e[1] - size;
            let to_jaws = (edge == "Shoulder").then_some(foot + side.stickout);
            if let Some(to_jaws) = to_jaws {
                let y = -(outer_of(bar) + 3.);
                marks.push(Mark::Span([-side.stickout, y], [foot, y], String::new()));
                marks.push(Mark::Word(
                    [foot + 1., y - 4.],
                    format!("{} to the jaws", mm(to_jaws)),
                    NEVER,
                    Align::Left,
                ));
            }
            // Sized as three in a row: its chips take two or three rows.
            let size_of = dial_size(p, 3.);
            let mut d = vec![];
            let mut rows = vec![];
            if let Some(c) = carriage {
                let at = reading(p, c);
                d.push(dial_box(
                    p,
                    c,
                    format!("the point {} short", mm(*size)),
                    size_of,
                ));
                rows.push(line(
                    narrow,
                    Some(1),
                    "carriage",
                    // As a pass shows the carriage (LIO-437).
                    big(at.total.clone(), lit(0, item)),
                    vec![
                        format!("{} on the dial", at.long),
                        format!("the point {} short of the corner, lock it", mm(*size)),
                    ],
                    lit(0, item),
                    reached(0),
                ));
            }
            let n = rows.len();
            d.push(dial_box(
                p,
                top,
                format!("in {} toward the chuck", mm(*size)),
                size_of,
            ));
            if swung.is_some() {
                d.push(top_angle(90., false));
            }
            rows.push(line(
                narrow,
                Some(n as u8 + 1),
                "top slide",
                big(reading(p, top).total, lit(n, item)),
                vec!["45° edge on the corner".into(), "zero, then in".into()],
                lit(n, item),
                reached(n),
            ));
            let mut right = column![
                chips(vec![
                    tool_chip(p, tool),
                    rpm_chip(*rpm),
                    chip(edge.to_lowercase())
                ]),
                chips(vec![
                    chip("45° tool edge"),
                    set_back(*swung),
                    warn_chip("lock the carriage"),
                ]),
            ];
            if let Some(n) = to_jaws {
                right = right.push(chips(vec![warn_chip(format!(
                    "the edge reaches {} past the corner: {} to the jaws",
                    mm(*size),
                    mm(n)
                ))]));
            }
            let right = right.push(dials(d)).extend(rows);
            (
                "Chamfer".into(),
                "a 45° tool edge, fed along the bed",
                view(bar, marks),
                right,
            )
        }
        LatheStep::Plunge {
            op,
            tool,
            rpm,
            width,
            part_off,
            bottom,
            cross,
            carriages,
        } => {
            let bar = bar_of(*op);
            // Parting off, the bar left in the chuck goes from the
            // simulation, as the part is what the next setup holds: the
            // kerf is what the blade cuts.
            let pieces: Vec<[f32; 4]> = match part_off {
                true => rings_of(*op).first().cloned().unwrap_or_default(),
                false => all(*op),
            };
            let end = end_of(bar);
            let e = extent(&pieces).unwrap_or([end - 10., end - 10. + width, 0., outer_of(bar)]);
            let mut marks: Vec<Mark> = pieces.iter().map(|g| Mark::Cut(*g)).collect();
            let z = e[0] + width / 2.;
            let r = outer_at(bar, z);
            marks.push(Mark::Tool(Cutter::Blade(*width), [z, r + 0.5]));
            marks.push(Mark::Word(
                [e[0] - 1., r + 2.],
                format!("{} wide", mm(*width)),
                ACCENT,
                Align::Right,
            ));
            if *part_off {
                let left = side.part.iter().map(|s| s[0]).fold(f32::INFINITY, f32::min);
                if left.is_finite() {
                    marks.push(Mark::Word(
                        [(left + end) / 2., -r - 4.],
                        format!("{} long", mm(end - e[1])),
                        INK,
                        Align::Center,
                    ));
                }
            }
            let i = item.min(carriages.len().saturating_sub(1));
            let mut d = vec![];
            if let Some(c) = carriages.get(i) {
                d.push(dial_box(
                    p,
                    c,
                    format!("blade's left side at {}", reading(p, c).short),
                    dial_size(p, 2.),
                ));
            }
            let mut rows = vec![];
            if let Some(c) = carriages.get(i) {
                rows.push(line(
                    narrow,
                    Some(1),
                    "carriage",
                    // As a pass shows the carriage (LIO-437).
                    big(reading(p, c).total, Lit::Now),
                    vec![
                        format!("{} on the dial", reading(p, c).long),
                        "left side of the blade, then lock it".into(),
                    ],
                    Lit::Now,
                    None,
                ));
            }
            match cross {
                Some(c) => {
                    let cr = reading(p, c);
                    let from = touched(g, *op, p.at).unwrap_or(2. * r);
                    d.push(dial_box(
                        p,
                        c,
                        format!("in {} → {}", cr.total, dia(*bottom)),
                        dial_size(p, 2.),
                    ));
                    rows.push(line(
                        narrow,
                        Some(2),
                        "cross slide",
                        big(cr.total, Lit::Next),
                        vec![format!("from the touch on {}", dia(from)), cr.long],
                        Lit::Next,
                        None,
                    ));
                }
                None => rows.push(line(
                    narrow,
                    Some(2),
                    "cross slide",
                    big("centre", Lit::Next),
                    vec!["in until the part drops".into(), "catch it".into()],
                    Lit::Next,
                    None,
                )),
            }
            let mut c = vec![tool_chip(p, tool), rpm_chip(*rpm)];
            if *part_off {
                c.push(warn_chip(ON_CENTRE));
            }
            let mut right = column![
                chips(c),
                dials(d),
                chips(vec![warn_chip("lock the carriage"), chip("slow, oil")]),
            ]
            .extend(rows);
            if carriages.len() > 1 {
                right = right.push(tiles(
                    (carriages.iter())
                        .map(|c| (reading(p, c).short, None))
                        .collect(),
                    i,
                ));
            }
            let (title, sub) = if *part_off {
                ("Part off", "catch the part")
            } else {
                ("Groove", "one plunge")
            };
            (title.into(), sub, view(bar, marks), right)
        }
        LatheStep::ThreadPass {
            op,
            index,
            of,
            rpm,
            pitch,
            angle,
            infeed,
            top,
            carriage,
        } => {
            let bar = bar_of(*op);
            let end = end_of(bar);
            let along = alongs(g, k_of(*op));
            let i = index - 1;
            let (r, z0) = along.get(i).copied().unwrap_or((outer_of(bar), end - 10.));
            let r0 = touched(g, *op, p.at).map_or(outer_at(bar, end), |d| d / 2.);
            let before = if i == 0 { r0 } else { along[i - 1].0 };
            let last = along.last().map_or(r, |a| a.0);
            let band = |a: f32, b: f32| [z0, end, a.min(b), a.max(b)];
            let mut marks = vec![];
            if (before - r0).abs() > 1e-3 {
                marks.push(Mark::Earlier(band(before, r0)));
            }
            marks.push(Mark::Cut(band(r, before)));
            if (last - r).abs() > 1e-3 {
                marks.push(Mark::Later(band(last, r)));
            }
            marks.push(Mark::Path([end + 2., r], [z0 + 1., r]));
            marks.push(Mark::Tool(Cutter::Thread, [end + 2., r]));
            // Its label above the bar, clear of its outline.
            marks.push(Mark::Never(
                [z0, last.min(r0) - 1.5],
                [z0, (last.max(r0) + 2.).max(outer_of(bar) + 1.5)],
                from_end("thread stops", carriage),
            ));
            marks.push(Mark::Word(
                [(z0 + end) / 2. + 4., r0 + 7.],
                format!("{} pitch · {} long", mm(*pitch), mm(end - z0)),
                ACCENT,
                Align::Center,
            ));
            // The Try screen after the last pass says to try the nut
            // (LIO-426: no chip for it here).
            let left = column![
                view(bar, marks),
                note("the half-nuts stay closed through every pass".into())
            ]
            .spacing(4)
            .into();
            let t = reading(p, top);
            let right = column![
                chips(vec![
                    tool_chip(p, &p.setup.operations[*op].tool),
                    rpm_chip(*rpm),
                    chip(format!("gearbox {}", mm(*pitch))),
                    chip("half-nuts stay closed"),
                ]),
                // At a narrow window the top slide's number is in its row:
                // no dial, so the four motions are in view (LIO-415).
                dials(
                    vec![dial_box(
                        p,
                        top,
                        if top.out {
                            format!(
                                "pass {index}: out, reads {}",
                                fixed(top.reads.unwrap_or(0.), places_of(p, Slide::Top))
                            )
                        } else {
                            format!("pass {index}: in to {}", t.total)
                        },
                        dial_size(p, 2.)
                    ),]
                    .into_iter()
                    .chain([top_angle(*angle, top.out)])
                    .filter(|_| !narrow)
                    .collect()
                ),
                line(
                    narrow,
                    Some(1),
                    "cross slide",
                    big("0", lit(0, item)),
                    vec!["back to its zero".into(), "every pass".into()],
                    lit(0, item),
                    reached(0),
                ),
                line(
                    narrow,
                    Some(2),
                    "top slide",
                    big(t.total.clone(), lit(1, item)),
                    vec![
                        if top.out {
                            "out from the touch".into()
                        } else {
                            "in from the touch".into()
                        },
                        format!(
                            "{} {}",
                            fixed(*infeed, 3),
                            if top.out {
                                "away from the axis"
                            } else {
                                "toward the axis"
                            }
                        )
                    ],
                    lit(1, item),
                    reached(1),
                ),
                // The motions in order: the cut forward, then out and back
                // in the warning colour (LIO-415).
                line(
                    narrow,
                    Some(3),
                    "spindle",
                    big("forward", lit(2, item)),
                    vec![
                        format!("cut to {} from the end", mm(end - z0)),
                        format!("{} on the dial", reading(p, carriage).long),
                    ],
                    lit(2, item),
                    reached(2),
                ),
                line(
                    narrow,
                    Some(4),
                    "cross slide",
                    text("OUT")
                        .size(26)
                        .font(Font {
                            weight: iced::font::Weight::Bold,
                            ..Font::MONOSPACE
                        })
                        .color(super::WARNING[1])
                        .into(),
                    vec!["at the end, then".into(), "reverse to come back".into()],
                    lit(3, item),
                    reached(3),
                ),
                thread_tiles(p, *op),
            ];
            (
                format!("Thread pass {index} of {of}"),
                "the top slide swung, infeed on its dial",
                left,
                right,
            )
        }
        LatheStep::ThreadSetUp {
            op,
            rpm,
            pitch,
            angle,
        } => {
            let bar = bar_of(*op);
            let (end, r) = (end_of(bar), outer_of(bar));
            let left = column![
                view(bar, vec![Mark::Tool(Cutter::Thread, [end + 2., r])]),
                note("set once: the passes after it leave it all as it is".into())
            ]
            .spacing(4)
            .into();
            let step = |n: usize, key: &str, value: String, tags: Vec<String>| {
                line(
                    narrow,
                    Some(n as u8 + 1),
                    key,
                    big(value, lit(n, item)),
                    tags,
                    lit(n, item),
                    reached(n),
                )
            };
            let right = column![
                chips(vec![tool_chip(p, &p.setup.operations[*op].tool)]),
                label("Before the first pass"),
                step(0, "gearbox", mm(*pitch), vec!["mm pitch".into()]),
                step(1, "spindle", format!("{}", rpm.round()), vec!["rpm".into()]),
                step(
                    2,
                    "top slide",
                    format!("{}°", trimmed(*angle)),
                    vec!["swung from the cross slide".into()]
                ),
                step(
                    3,
                    "tool",
                    "on centre".into(),
                    vec!["square to the work".into(), "with the thread gauge".into()]
                ),
                step(
                    4,
                    "half-nuts",
                    "close".into(),
                    vec![
                        "leave them closed".into(),
                        "until the thread is done".into()
                    ]
                ),
            ];
            (
                "Set up the thread".into(),
                "once, before the first pass",
                left,
                right,
            )
        }
        LatheStep::Try { op, what, spring } => {
            let bar = bar_of(*op);
            let marks = all(*op).iter().map(|g| Mark::Cut(*g)).collect();
            let left = view(bar, marks);
            let mut right = column![
                chips(vec![tool_chip(p, &p.setup.operations[*op].tool)]),
                // "Try" said once, by the screen's name (LIO-437).
                label("By hand"),
                line(
                    narrow,
                    None,
                    "",
                    big(what.clone(), Lit::Now),
                    vec!["it goes on by hand".into()],
                    Lit::Now,
                    None,
                ),
            ];
            if *spring {
                right = right.push(line(
                    narrow,
                    None,
                    "if tight",
                    big("spring pass", Lit::Next),
                    vec![
                        "one at the same setting,".into(),
                        "then 0.02 more on the top slide at a time".into(),
                    ],
                    Lit::Next,
                    None,
                ));
            }
            ("Try".into(), "the thread", left, right)
        }
    }
}
/// The tiles of an Outside or Bore's passes, their diameters, with the
/// measure before the last, the step shown lit; each opens its step.
fn pass_tiles<'a>(p: &Page<'a>, op: usize) -> Element<'a, Message> {
    let g = p.guide();
    let mut labels = vec![];
    let mut now = 0;
    for (i, s) in g.steps.iter().enumerate() {
        let label = match s {
            Step::Lathe(LatheStep::Pass {
                op: o, diameter, ..
            }) if *o == op => format!("{diameter:.2}"),
            Step::Lathe(LatheStep::Measure { op: o, .. }) if *o == op => "measure".into(),
            _ => continue,
        };
        if i == p.at {
            now = labels.len();
        }
        labels.push((label, Some(Message::Guide(GuideAction::Go(i)))));
    }
    tiles(labels, now)
}
/// The tiles of a thread's passes: the top slide's number for each.
fn thread_tiles<'a>(p: &Page<'a>, op: usize) -> Element<'a, Message> {
    let g = p.guide();
    let mut labels = vec![];
    let mut now = 0;
    for (i, s) in g.steps.iter().enumerate() {
        if let Step::Lathe(LatheStep::ThreadPass { op: o, top, .. }) = s
            && *o == op
        {
            if i == p.at {
                now = labels.len();
            }
            labels.push((
                reading(p, top).total,
                Some(Message::Guide(GuideAction::Go(i))),
            ));
        }
    }
    tiles(labels, now)
}
/// The Hand over screen of a lathe setup: the bar as it is left, where the
/// part goes and how the next machine holds it.
pub(super) fn hand_over<'a>(p: &Page<'a>, to: Option<usize>) -> Element<'a, Message> {
    let g = p.guide();
    let side = p.shown.side.as_ref();
    let mut c = vec![];
    let next = match to.and_then(|n| Some((n, p.app.doc.setups.get(n)?))) {
        Some((n, s)) => {
            c.push(warn_chip(format!("→ {}", s.name())));
            if !lathe::is_lathe(&s.machine) {
                c.push(chip("Round part: hold it on a V-block or in soft jaws."));
            }
            Some((
                format!("Go to {} ▶", s.name()),
                Message::Guide(GuideAction::Open(n)),
            ))
        }
        None => {
            c.push(chip("the last setup"));
            None
        }
    };
    c.push(chip(format!("lathe ~{} min", g.minutes.ceil())));
    let left: Element<'a, Message> = match side.and_then(|s| Some((s, s.bars.last()?))) {
        Some((side, bar)) => {
            use std::hash::{Hash, Hasher};
            let mut h = std::hash::DefaultHasher::new();
            (p.key, p.at, "lathe-hand-over").hash(&mut h);
            crate::fault::canvas(SideView {
                key: h.finish(),
                side,
                bar,
                marks: vec![],
            })
            .width(iced::Length::Fill)
            .height(iced::Length::Fill)
            .into()
        }
        None => space().into(),
    };
    frame(
        "Hand over".into(),
        "the part leaves the lathe",
        left,
        column![chips(c)],
        next,
        true,
    )
}

#[cfg(test)]
mod tests {
    use crate::{
        App, Message, Page,
        guide::{LatheStep as L, Step},
        mesh::Mesh,
        model::Document,
    };

    /// The Steps page of `doc`'s lathe setup, its guide built.
    fn app_of(mut doc: Document, mesh: Mesh) -> App {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        a.test_dir = Some(dir);
        doc.file = Some(crate::model::ModelFile {
            name: "part.stl".into(),
            path: std::path::PathBuf::from("part.stl"),
            format: "stl".into(),
            bytes: 1,
            thickness: None,
        });
        doc.active = doc.setups[0].id.clone();
        a.doc = doc;
        a.imported_mesh = Some(mesh);
        let _ = a.update(Message::Page(Page::Control));
        crate::steps::tests::build(&mut a);
        a
    }
    /// The page at the first step `pick` is true of, the readout `item`
    /// lit: the texts it shows.
    fn shown(a: &mut App, item: usize, pick: impl Fn(&L) -> bool) -> Vec<String> {
        let at = (a.guide_at().unwrap().0.guide.steps.iter())
            .position(|s| matches!(s, Step::Lathe(l) if pick(l)))
            .unwrap();
        let id = a.doc.setups[0].id.clone();
        a.doc.guide.insert(id, at);
        a.guide_item = item;
        crate::ui::probe::texts(a.view(), iced::Size::new(1280., 800.))
    }
    fn has(texts: &[String], wants: &[&str]) {
        for want in wants {
            assert!(texts.iter().any(|t| t == want), "{want:?} in {texts:?}");
        }
    }

    /// LIO-389: at 1024 × 700 a Pass screen's third readout, the carriage's
    /// stop, lies in the visible part of its column; the side view's red
    /// labels keep off the chuck and the jaws at both window sizes.
    #[test]
    fn a_pass_screen_shows_its_carriage_stop_at_1024_and_labels_keep_off_the_jaws() {
        let mut a = crate::steps::tests::lathe_app("Diameter", |_| {});
        let window = iced::Size::new(1024., 700.);
        let _ = a.update(Message::Resized(window));
        let _ = shown(&mut a, 0, |l| matches!(l, L::Pass { .. }));
        let texts = crate::ui::probe::placed(a.view(), window);
        let (_, stop) = texts.iter().find(|(t, _)| t == "carriage").unwrap();
        let column = (crate::ui::probe::scrollables(a.view(), window).into_iter())
            .map(|(_, b)| b)
            .filter(|b| b.x <= stop.x && stop.x <= b.x + b.width && b.y <= stop.y)
            .min_by(|p, q| (p.width * p.height).total_cmp(&(q.width * q.height)))
            .unwrap();
        assert!(
            stop.y + stop.height <= column.y + column.height,
            "{stop:?} below {column:?}"
        );
        // The shoulder 6 mm from the jaws and the thread's end, as the
        // Outside's and the Thread's passes draw them.
        let side = a.guide_at().unwrap().0.side.clone().unwrap();
        let bar = side.bars[1].clone();
        let view = super::SideView {
            key: 0,
            side: &side,
            bar: &bar,
            marks: vec![
                super::Mark::Never(
                    [-64., 18.5],
                    [-64., 27.1],
                    "shoulder 64 from the end".into(),
                ),
                super::Mark::Never(
                    [-39., 18.5],
                    [-39., 22.],
                    "thread stops 39 from the end".into(),
                ),
            ],
        };
        let meets = |a: &iced::Rectangle, b: &iced::Rectangle| {
            a.x < b.x + b.width
                && b.x < a.x + a.width
                && a.y < b.y + b.height
                && b.y < a.y + a.height
        };
        // The side view's room at 1440 × 900 and at 1024 × 700.
        for size in [iced::Size::new(630., 560.), iced::Size::new(390., 330.)] {
            let chuck = view.scale(size).chuck(side.stickout);
            let labels: Vec<_> = view.labels(size).into_iter().flatten().collect();
            assert_eq!(labels.len(), 2);
            for l in &labels {
                assert!(
                    !chuck.iter().any(|c| meets(&l.rect, c)),
                    "{size:?}: {l:?} on {chuck:?}"
                );
            }
            assert!(!meets(&labels[0].rect, &labels[1].rect), "{labels:?}");
        }
        // A Thread pass's two labels keep off each other, the threading
        // tool and the band it cuts, at both sizes (LIO-392), as the pass
        // draws them: 39 mm of 1.5 pitch on the outside.
        let (end, r0) = (super::end_of(&bar), super::outer_of(&bar));
        let (z0, r) = (end - 39., r0 - 0.3);
        let view = super::SideView {
            marks: vec![
                super::Mark::Cut([z0, end, r, r0]),
                super::Mark::Path([end + 2., r], [z0 + 1., r]),
                super::Mark::Tool(super::Cutter::Thread, [end + 2., r]),
                super::Mark::Never(
                    [z0, r - 1.5],
                    [z0, r0 + 2.],
                    "thread stops 39 from the end".into(),
                ),
                super::Mark::Word(
                    [(z0 + end) / 2. + 4., r0 + 7.],
                    "1.5 pitch · 39 long".into(),
                    super::ACCENT,
                    super::Align::Center,
                ),
            ],
            ..view
        };
        for size in [iced::Size::new(630., 560.), iced::Size::new(390., 330.)] {
            let k = view.scale(size);
            let mut kept = k.chuck(side.stickout).to_vec();
            kept.push(super::tool_box(
                super::Cutter::Thread,
                k.at([end + 2., r]),
                k.cy,
                k.s,
            ));
            kept.push(k.rect(z0, end, r, r0));
            let labels: Vec<_> = view.labels(size).into_iter().flatten().collect();
            assert_eq!(labels.len(), 2);
            for l in &labels {
                assert!(
                    !kept.iter().any(|c| meets(&l.rect, c)),
                    "{size:?}: {l:?} on {kept:?}"
                );
            }
            assert!(!meets(&labels[0].rect, &labels[1].rect), "{labels:?}");
        }
    }
    /// LIO-203: a shoulder chamfer 5 mm from the jaws says how far the tool's
    /// edge reaches and how far that leaves it from the jaws; a Zero with
    /// only inside tools asks for no measurement of the bar's outside.
    #[test]
    fn a_shoulder_chamfer_says_its_reach_to_the_jaws_and_a_bore_only_zero_measures_nothing() {
        let mut a = crate::steps::tests::lathe_app("Diameter", |doc| {
            doc.setups[0].operations.truncate(2);
            crate::turn::tests::add(doc, "Chamfer", &[("edge", "Shoulder"), ("size", "1")]);
        });
        let t = shown(&mut a, 0, |l| matches!(l, L::Chamfer { .. }));
        has(&t, &["the edge reaches 1 past the corner: 4 to the jaws"]);
        let mut a = crate::steps::tests::bore_app(|_| {});
        let t = shown(&mut a, 0, |l| matches!(l, L::Zero { .. }));
        has(&t, &["TOUCH THE END", "zero the carriage dial"]);
        assert!(
            !t.iter()
                .any(|s| s == "measure" || s.contains("measure the bar")),
            "{t:?}"
        );
    }
    #[test]
    fn the_tap_screen_shows_the_quill_the_turns_and_how_to_turn_it() {
        let (mut doc, mesh, tap) = crate::turn::tests::m12_job();
        doc.setups[0]
            .machine
            .p
            .insert("cross_reads".into(), "Diameter".into());
        crate::turn::tests::add(&mut doc, "Drill", &[("depth", "16")]);
        crate::turn::tests::add_tap(&mut doc, &tap, "10");
        let mut a = app_of(doc, mesh);
        let Some(Step::Lathe(L::TailTap { lead, turns, .. })) = a
            .guide_at()
            .unwrap()
            .0
            .guide
            .steps
            .iter()
            .find(|s| matches!(s, Step::Lathe(L::TailTap { .. })))
            .cloned()
        else {
            panic!()
        };
        let t = shown(&mut a, 1, |l| matches!(l, L::TailTap { .. }));
        has(
            &t,
            &[
                "Tap",
                "tailstock",
                "spindle off",
                "½ back every turn",
                "oil",
                &format!("10 deep + {lead:.2} for the lead"),
                &format!("{turns} turns"),
                "turn the chuck or the wrench by hand",
                "1.75 mm a turn",
            ],
        );
        // The quill's dial and its total are read off the Drill's way.
        assert!(t.iter().any(|s| s.starts_with("quill · ")), "{t:?}");
        assert!(t.iter().any(|s| s.ends_with(" on the dial")), "{t:?}");
        // The Drill before it is the drill's screen, not a tap's.
        let t = shown(&mut a, 1, |l| matches!(l, L::TailDrill { .. }));
        assert!(!t.iter().any(|s| s == "oil"), "{t:?}");
    }
    /// Renders the lathe's Tap and Die screens to `$AXIS_RENDER_DIR`
    /// (default /tmp/threading). Not a check: run it with --ignored.
    #[test]
    #[ignore]
    fn render_the_tap_and_die_screens() {
        let out = std::path::PathBuf::from(
            std::env::var("AXIS_RENDER_DIR").unwrap_or("/tmp/threading".into()),
        );
        std::fs::create_dir_all(&out).unwrap();
        let size = iced::Size::new(1280., 800.);
        let (mut doc, mesh, tap) = crate::turn::tests::m12_job();
        doc.setups[0]
            .machine
            .p
            .insert("cross_reads".into(), "Diameter".into());
        crate::turn::tests::add(&mut doc, "Drill", &[("depth", "16")]);
        crate::turn::tests::add_tap(&mut doc, &tap, "10");
        let mut a = app_of(doc, mesh);
        shown(&mut a, 1, |l| matches!(l, L::TailTap { .. }));
        crate::smoke::tests::render_png(&a, size, &out.join("10-lathe-tap.png"));
        let (doc, mesh) = crate::guide::tests::die_sample();
        let mut a = app_of(doc, mesh);
        shown(&mut a, 1, |l| matches!(l, L::DieThread { .. }));
        crate::smoke::tests::render_png(&a, size, &out.join("11-lathe-die.png"));
    }
    #[test]
    fn the_die_screen_is_one_screen_of_turns_not_thread_passes() {
        let (doc, mesh) = crate::guide::tests::die_sample();
        let mut a = app_of(doc, mesh);
        let t = shown(&mut a, 1, |l| matches!(l, L::DieThread { .. }));
        has(
            &t,
            &[
                "Die",
                "by hand",
                "spindle off",
                "½ back every turn",
                "oil",
                "square",
                "quill against the holder",
                "10 turns",
                "turn the holder by hand",
                "to 20 from the end",
            ],
        );
        assert!(!t.iter().any(|s| s.starts_with("Thread pass")), "{t:?}");
    }
}
