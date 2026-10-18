//! The lathe's screens on the Steps page: a side view of the bar (the chuck
//! on the left, you at the front) and every number as a picture of a dial
//! collar, as the lathe has no DRO.
use super::{
    ACCENT, CHIP, Lit, NEVER, Page, STOCK, TOOL, arrow, buttons, chip, chips, dashed, disc, frame,
    label, lit, stroke, tiles, warn_chip,
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

/// The shape of the tool drawn at its tip.
#[derive(Debug, Clone, Copy)]
enum Cutter {
    Turn,
    Bore,
    /// A grooving tool or parting blade this wide, mm.
    Blade(f32),
    /// A drill of this radius, from the tailstock.
    Drill(f32),
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
impl SideView<'_> {
    fn paint(&self, f: &mut canvas::Frame) {
        let side = self.side;
        let size = f.size();
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
        let cy = top + (size.height - top - bottom) / 2.;
        let x = |z: f32| left + (z - low) * s;
        let y = |r: f32| cy - r * s;
        let at = |q: [f32; 2]| Point::new(x(q[0]), y(q[1]));
        // At least 3 px across, so a thin cut still shows.
        let rect = |z0: f32, z1: f32, ra: f32, rb: f32| {
            let (mut x0, mut x1) = (x(z0.max(low)), x(z1));
            let (mut y0, mut y1) = (y(ra.max(rb)), y(ra.min(rb)));
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
        let both = |g: [f32; 4]| [rect(g[0], g[1], g[2], g[3]), rect(g[0], g[1], -g[2], -g[3])];
        let path = |r: Rectangle| canvas::Path::rectangle(r.position(), r.size());
        // The chuck's body, behind the jaws.
        let chuck = rect(low, back, body, -body);
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
        let g = side.gripped.max(1.);
        for r in [
            rect(back, -side.stickout, g + jaw, g),
            rect(back, -side.stickout, -g, -g - jaw),
        ] {
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
        for m in &self.marks {
            match m {
                Mark::Never(a, b, s) => {
                    let (m, n) = (at(*a), at(*b));
                    f.stroke(&canvas::Path::line(m, n), dashed(NEVER, 1.8));
                    let top = if m.y < n.y { m } else { n };
                    word(
                        f,
                        s,
                        Point::new(top.x - 5., top.y - 9.),
                        NEVER,
                        12.,
                        Align::Right,
                        bold_font(),
                    );
                }
                Mark::Tool(kind, q) => tool(f, *kind, at(*q), cy, s),
                Mark::Path(a, b) => arrow(f, at(*a), at(*b), TOOL, 2.),
                Mark::Word(q, s, c, align) => word(f, s, at(*q), *c, 13., *align, bold_font()),
                Mark::Dot(q, s) => {
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
                Mark::Span(a, b, s) => {
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
                Mark::Across(z, r, s) => {
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
/// The tool of `kind` with its tip at `tip`; `cy` is the axis, `s` px per mm.
fn tool(f: &mut canvas::Frame, kind: Cutter, tip: Point, cy: f32, s: f32) {
    let fill = Color { a: 0.3, ..TOOL };
    let draw = |f: &mut canvas::Frame, p: canvas::Path| {
        f.fill(&p, fill);
        f.stroke(&p, stroke(TOOL, 1.5));
    };
    let (x, y) = (tip.x, tip.y);
    let poly = |points: &[(f32, f32)]| {
        canvas::Path::new(|b| {
            b.move_to(Point::new(points[0].0, points[0].1));
            for p in &points[1..] {
                b.line_to(Point::new(p.0, p.1));
            }
            b.close();
        })
    };
    let bar = |x0: f32, y0: f32, w: f32, h: f32| {
        canvas::Path::rectangle(Point::new(x0, y0), Size::new(w, h))
    };
    match kind {
        Cutter::Turn => {
            draw(f, bar(x + 6., y - 40., 20., 26.));
            draw(
                f,
                poly(&[(x, y), (x + 10., y - 14.), (x + 28., y - 14.), (x + 22., y)]),
            );
        }
        Cutter::Bore => {
            draw(f, bar(x, y - 2., 90., 8.));
            draw(f, poly(&[(x, y - 2.), (x - 6., y - 8.), (x - 6., y - 2.)]));
        }
        Cutter::Blade(w) => {
            let w = (w * s).max(4.);
            draw(f, bar(x - w / 2., y - 36., w, 36.));
        }
        Cutter::Drill(r) => {
            let r = (r * s).max(3.);
            draw(f, bar(x, cy - r, 70., 2. * r));
            draw(f, poly(&[(x, cy - r), (x - r * 0.6, cy), (x, cy + r)]));
        }
        Cutter::Thread => {
            draw(f, bar(x - 7., y - 42., 14., 26.));
            draw(f, poly(&[(x, y), (x - 7., y - 16.), (x + 7., y - 16.)]));
        }
        Cutter::Chamfer(inside) => {
            // The 45° edge runs from the point toward the chuck and out
            // (in, inside a hole), drawn bold.
            let l = 18.;
            let v = if inside { 1. } else { -1. };
            draw(
                f,
                poly(&[(x, y), (x - l, y + v * l), (x + 0.5 * l, y + v * l)]),
            );
            if inside {
                draw(f, bar(x - l, y + l, 90., 8.));
            } else {
                draw(f, bar(x - l, y - l - 24., 1.5 * l, 24.));
            }
            f.stroke(
                &canvas::Path::line(Point::new(x, y), Point::new(x - l, y + v * l)),
                stroke(ACCENT, 2.5),
            );
        }
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
            let r0 = r - if big { 10. } else { 5. };
            f.stroke(
                &canvas::Path::line(point(a, r0), point(a, r)),
                stroke(ink, if big { 1.4 } else { 1. }),
            );
            if big {
                word(
                    &mut f,
                    &trimmed(i as f32 * self.turn / div as f32),
                    point(a, r - (r * 0.3).clamp(15., 21.)),
                    if grey { ink } else { MUTED },
                    if r < 60. { 10. } else { 11. },
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
                            ..stroke(Color { a: 0.85, ..ACCENT }, 6.)
                        },
                    );
                    let tip = Point::new(c.x + end.cos() * (r - 3.), c.y + end.sin() * (r - 3.));
                    f.fill(&canvas::Path::circle(tip, 5.5), ACCENT);
                }
                word(
                    &mut f,
                    &fixed(*v, places),
                    c,
                    INK,
                    (r * 0.3).clamp(14., 20.),
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
/// What to do with a dial wound out: "2 turns out", "reads 2.20" (its
/// collar counts down).
fn out_tags(d: &Dial) -> Vec<String> {
    let (t, _) = d.reading.clone().unwrap_or((0, 0.));
    let r = d.reads.unwrap_or(0.);
    vec![
        match t {
            1 => "1 turn out".into(),
            t => format!("{t} turns out"),
        },
        format!("reads {r:.2}"),
    ]
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
    /// "1 t + 2.00", "2 t out, 2.20".
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
                    format!("{t} t + {reading}")
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
        canvas(face).width(size).height(size + 10.),
        text(sub).size(12).color(MUTED).center()
    ]
    .spacing(2)
    .align_x(Alignment::Center)
    .width(if d.out { size + 100. } else { size + 60. })
    .into()
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
    order: Option<u8>,
    key: &str,
    value: Element<'a, Message>,
    tags: Vec<String>,
    state: Lit,
    msg: Option<Message>,
) -> Element<'a, Message> {
    let now = state == Lit::Now;
    let mut l = row![].spacing(10).align_y(Alignment::Center);
    if let Some(n) = order {
        let mark = if state == Lit::Done {
            "✓".to_string()
        } else {
            n.to_string()
        };
        l = l.push(disc(mark, now, 22.));
    }
    l = l
        .push(text(key.to_string()).size(13).color(MUTED).width(78))
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
    let view = |bar: &'a [[f32; 4]], marks: Vec<Mark>, height: f32| -> Element<'a, Message> {
        use std::hash::{Hash, Hasher};
        let mut h = std::hash::DefaultHasher::new();
        (p.key, p.at, p.item, "lathe").hash(&mut h);
        canvas(SideView {
            key: h.finish(),
            side,
            bar,
            marks,
        })
        .width(iced::Length::Fill)
        .height(height)
        .into()
    };
    let note = |s: String| -> Element<'a, Message> { text(s).size(12).color(MUTED).into() };
    let item = p.item;
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
                    canvas(face).width(130).height(140),
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
                view(start, marks, 380.),
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
            let right = column![
                chips(c),
                line(
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
                view(start, marks, 380.),
                right,
            )
        }
        LatheStep::Zero {
            tool,
            key,
            planned,
            measured,
        } => {
            let end = end_of(start).max(0.);
            let r = outer_at(start, end);
            let marks = vec![
                Mark::Tool(Cutter::Turn, [end, r * 0.45]),
                Mark::Word(
                    [end + 1., r * 0.15],
                    "touch the end".into(),
                    ACCENT,
                    Align::Left,
                ),
                Mark::Dot([end - (r * 0.5).clamp(4., 15.), r], "touch the Ø".into()),
            ];
            let right = column![
                chips(vec![tool_chip(p, tool)]),
                label("Touch, then zero the collars"),
                line(
                    Some(1),
                    "carriage",
                    big("0", lit(0, item)),
                    vec!["touch the end".into(), "zero the carriage dial".into()],
                    lit(0, item),
                    reached(0),
                ),
                line(
                    Some(2),
                    "measure",
                    measure_box(p, key, *measured, *planned),
                    vec!["type what the".into(), "calipers read".into()],
                    lit(1, item),
                    None,
                ),
                line(
                    Some(3),
                    "cross slide",
                    big("0", lit(2, item)),
                    vec!["touch the Ø".into(), "zero the cross dial".into()],
                    lit(2, item),
                    reached(2),
                ),
            ];
            (
                "Zero".into(),
                "touch, then zero both collars",
                view(start, marks, 380.),
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
                    vec!["swing it".into(), "zero it on the touch".into()],
                ));
            }
            if *carriage {
                marks.push(Mark::Tool(cutter_of(p, tool), [end, r * 0.45]));
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
            let right = column![
                chips(vec![tool_chip(p, tool), rpm_chip(*rpm)]),
                label("Touch, then zero"),
            ]
            .extend(rows);
            (
                "Touch".into(),
                "zero the dials for this tool",
                view(bar, marks, 380.),
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
            let right = column![
                chips(vec![tool_chip(p, tool), rpm_chip(*rpm)]),
                dials(vec![dial_box(
                    p,
                    top,
                    format!("in {} from the touch", fixed(*face_off, 2)),
                    150.
                )]),
                chips(vec![warn_chip("lock the carriage")]),
                line(
                    Some(1),
                    "top slide",
                    big(reading(p, top).total, lit(0, item)),
                    vec!["touch the end, zero".into(), "then in, face across".into()],
                    lit(0, item),
                    reached(0),
                ),
                line(
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
                view(bar, marks, 380.),
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
        } => {
            let bar = bar_of(*op);
            let end = end_of(bar);
            let rings = rings_of(*op);
            let i = index - 1;
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
                marks.push(Mark::Never(a, b, format!("{what} {}", mm(z_end))));
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
            let from = touched(g, *op, p.at).unwrap_or(2. * big_r);
            let depth = along.first().map_or(0., |a| (2. * a.0 - from).abs());
            let left = column![
                view(bar, marks, 380.),
                note(format!(
                    "{} → {} · {} long · {} off the Ø a pass · the last after you measure",
                    dia(from),
                    dia(finish),
                    mm(end - z_end),
                    fixed(depth, 2)
                ))
            ]
            .spacing(4)
            .into();
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
            let mut cross_tags = match c.at {
                Some(_) if cross.out => out_tags(cross),
                Some(_) => vec![format!("{} on the dial", c.long)],
                None => vec![c.long.clone()],
            };
            if index == of
                && let Some(m) = measured
            {
                cross_tags.push(format!("from the {} measured", dia(m)));
            }
            let mut carriage_tags = vec![];
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
                        112.
                    ),
                    dial_box(p, carriage, format!("stop at {}", at.short), 112.),
                ]),
                line(
                    Some(1),
                    "back off",
                    big("—", lit(0, item)),
                    vec!["carriage right".into(), "of the end".into()],
                    lit(0, item),
                    reached(0),
                ),
                line(
                    Some(2),
                    "cross slide",
                    big(c.total.clone(), lit(1, item)),
                    cross_tags,
                    lit(1, item),
                    reached(1),
                ),
                line(
                    Some(3),
                    "carriage",
                    big(at.short.clone(), lit(2, item)),
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
                    None,
                    "about",
                    big(dia(*expect), Lit::Next),
                    vec!["as the passes left it".into()],
                    Lit::Next,
                    None,
                ),
                line(
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
                        140.,
                    )]));
            }
            right = right.push(pass_tiles(p, *op));
            (
                "Measure".into(),
                "before the last pass",
                view(bar, marks, 380.),
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
                    150.
                )]),
                line(
                    Some(1),
                    "tailstock",
                    big("lock", lit(0, item)),
                    vec!["slide it up until".into(), "the drill touches".into()],
                    lit(0, item),
                    reached(0),
                ),
                line(
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
                view(bar, marks, 380.),
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
            let mut d = vec![];
            let mut rows = vec![];
            if let Some(c) = carriage {
                let at = reading(p, c);
                d.push(dial_box(
                    p,
                    c,
                    format!("the point {} short", mm(*size)),
                    130.,
                ));
                rows.push(line(
                    Some(1),
                    "carriage",
                    big(at.short, lit(0, item)),
                    vec![
                        format!("the point {} short", mm(*size)),
                        "of the corner, lock it".into(),
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
                130.,
            ));
            rows.push(line(
                Some(n as u8 + 1),
                "top slide",
                big(reading(p, top).total, lit(n, item)),
                vec!["45° edge on the corner".into(), "zero, then in".into()],
                lit(n, item),
                reached(n),
            ));
            let right = column![
                chips(vec![
                    tool_chip(p, tool),
                    rpm_chip(*rpm),
                    chip(edge.to_lowercase())
                ]),
                chips(vec![
                    chip("45° tool edge"),
                    chip("top slide 0°, along the bed"),
                    warn_chip("lock the carriage"),
                ]),
                dials(d),
            ]
            .extend(rows);
            (
                "Chamfer".into(),
                "a 45° tool edge, fed along the bed",
                view(bar, marks, 380.),
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
                    130.,
                ));
            }
            let mut rows = vec![];
            if let Some(c) = carriages.get(i) {
                rows.push(line(
                    Some(1),
                    "carriage",
                    big(reading(p, c).short, Lit::Now),
                    vec!["left side of the blade".into(), "then lock it".into()],
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
                        130.,
                    ));
                    rows.push(line(
                        Some(2),
                        "cross slide",
                        big(cr.total, Lit::Next),
                        vec![format!("from the touch on {}", dia(from)), cr.long],
                        Lit::Next,
                        None,
                    ));
                }
                None => rows.push(line(
                    Some(2),
                    "cross slide",
                    big("centre", Lit::Next),
                    vec!["in until the part drops".into(), "catch it".into()],
                    Lit::Next,
                    None,
                )),
            }
            let mut right = column![
                chips(vec![tool_chip(p, tool), rpm_chip(*rpm)]),
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
            (title.into(), sub, view(bar, marks, 380.), right)
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
            marks.push(Mark::Never(
                [z0, last.min(r0) - 1.5],
                [z0, last.max(r0) + 2.],
                format!("end {}", mm(z0)),
            ));
            marks.push(Mark::Word(
                [(z0 + end) / 2. + 4., r0 + 7.],
                format!("{} pitch · {} long", mm(*pitch), mm(end - z0)),
                ACCENT,
                Align::Center,
            ));
            let left = column![
                view(bar, marks, 380.),
                note(format!(
                    "gearbox {} · half-nuts stay closed: back the tool out, reverse the spindle to come back",
                    mm(*pitch)
                ))
            ]
            .spacing(4)
            .into();
            let t = reading(p, top);
            let right = column![
                chips(vec![
                    tool_chip(p, &p.setup.operations[*op].tool),
                    chip(format!("top slide {}°", mm(*angle))),
                    rpm_chip(*rpm),
                    warn_chip(format!("gearbox {}", mm(*pitch))),
                ]),
                dials(vec![dial_box(
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
                    118.
                )]),
                line(
                    Some(1),
                    "cross slide",
                    big("0", lit(0, item)),
                    vec!["back to its zero".into(), "every pass".into()],
                    lit(0, item),
                    reached(0),
                ),
                line(
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
                line(
                    Some(3),
                    "half-nuts",
                    big("close", lit(2, item)),
                    vec![
                        format!("stay closed to {}", reading(p, carriage).short),
                        "back out, reverse".into()
                    ],
                    lit(2, item),
                    reached(2),
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
            canvas(SideView {
                key: h.finish(),
                side,
                bar,
                marks: vec![],
            })
            .width(iced::Length::Fill)
            .height(380)
            .into()
        }
        None => space().into(),
    };
    let right = column![chips(c)]
        .push(space().height(iced::Length::Fill))
        .push(buttons(next));
    frame("Hand over".into(), "the part leaves the lathe", left, right)
}
