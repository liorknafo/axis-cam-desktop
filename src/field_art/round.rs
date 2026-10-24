//! Field pictures: a round bar's V-block, soft jaws and collet block
//! (`round_hold`), each drawn as the section across the bar that shows how
//! it holds: the V's two line contacts and the clamp, the jaws' arc round
//! the bore, the collet's grip down the bar.
use crate::illustration::*;
use iced::{
    Color, Point,
    widget::canvas::{Frame, Path, Stroke},
};
use std::f32::consts::{FRAC_PI_4, PI};

const RED: Color = Color::from_rgb(0.95, 0.36, 0.3);
/// The holding's steel.
const BLOCK: Color = Color::from_rgb(0.54, 0.58, 0.63);
const BLOCK_EDGE: Color = Color::from_rgb(0.76, 0.80, 0.83);
/// The vise body and the table under the holding.
const BODY: Color = Color::from_rgb(0.23, 0.31, 0.38);
/// The bar, in blue as the part.
const BAR: Color = Color::from_rgb(0.28, 0.55, 0.88);
const BAR_EDGE: Color = Color::from_rgb(0.42, 0.70, 0.98);

/// A filled outline with an edge.
fn shape(f: &mut Frame, points: &[Point], fill: Color, edge: Color) {
    polygon(f, points, fill);
    f.stroke(
        &Path::new(|b| {
            b.move_to(points[0]);
            for q in &points[1..] {
                b.line_to(*q);
            }
            b.close();
        }),
        Stroke::default().with_color(edge).with_width(1.),
    );
}
/// The bar's section: a disc at `c` of radius `r`, with its axis marked.
fn bar(f: &mut Frame, c: Point, r: f32) {
    f.fill(&Path::circle(c, r), BAR);
    f.stroke(
        &Path::circle(c, r),
        Stroke::default().with_color(BAR_EDGE).with_width(1.2),
    );
    line(
        f,
        Point::new(c.x - 4., c.y),
        Point::new(c.x + 4., c.y),
        TEXT,
        1.,
    );
    line(
        f,
        Point::new(c.x, c.y - 4.),
        Point::new(c.x, c.y + 4.),
        TEXT,
        1.,
    );
}
/// The arc of radius `r` round `c` from angle `from` to `to` (radians,
/// from +X, counterclockwise on screen as y grows down: 90° is down).
fn arc(f: &mut Frame, c: Point, r: f32, from: f32, to: f32, color: Color, width: f32) {
    let n = 24;
    let points: Vec<Point> = (0..=n)
        .map(|i| {
            let t = from + (to - from) * i as f32 / n as f32;
            Point::new(c.x + r * t.cos(), c.y + r * t.sin())
        })
        .collect();
    for w in points.windows(2) {
        line(f, w[0], w[1], color, width);
    }
}

pub fn round(f: &mut Frame, _time: f32, phase: f32, n: u8) {
    match n {
        0 => v_block(f, phase),
        1 => soft_jaws(f, phase),
        _ => collet(f, phase),
    }
}
/// A 90° V-block from the bar's end: the bar grows until it sits on the
/// V's edges, which is refused.
fn v_block(f: &mut Frame, phase: f32) {
    heading(f, "V-BLOCK · THE BAR LIES IN THE V, CLAMPED ON TOP");
    let (cx, top, bottom) = (139., 98., 134.);
    let (half_w, half_v) = (70., 26.);
    // The V's apex, 45° flanks: as deep as half its width.
    let apex = top + half_v;
    shape(
        f,
        &[
            Point::new(cx - half_w, bottom),
            Point::new(cx - half_w, top),
            Point::new(cx - half_v, top),
            Point::new(cx, apex),
            Point::new(cx + half_v, top),
            Point::new(cx + half_w, top),
            Point::new(cx + half_w, bottom),
        ],
        BLOCK,
        BLOCK_EDGE,
    );
    rect(
        f,
        cx - half_w - 10.,
        bottom,
        cx + half_w + 10.,
        bottom + 6.,
        BODY,
    );
    // The bar grows over the middle of the phase, until it rests on the
    // V's edges instead of its flanks.
    let grow = smooth(((phase - 0.3) / 0.45).clamp(0., 1.));
    let r = 14. + 21. * grow;
    let s = FRAC_PI_4.sin();
    let (c, touch, drop) = if r * s <= half_v {
        (Point::new(cx, apex - r / s), r * s, r * s)
    } else {
        let rise = (r * r - half_v * half_v).sqrt();
        (Point::new(cx, top - rise), half_v, rise)
    };
    let too_big = r * s > half_v - 2.;
    bar(f, c, r);
    // The two line contacts, each pressed along the flank's normal.
    for side in [-1., 1.] {
        let q = Point::new(c.x + side * touch, c.y + drop);
        let color = if too_big { RED } else { GOLD };
        f.fill(&Path::circle(q, 3.), color);
        line(f, Point::new(q.x + side * 9., q.y + 9.), q, color, 1.4);
    }
    // The strap over the bar, its legs down the block's sides.
    let strap = (c.y - r).min(top) - 8.;
    let color = BLOCK_EDGE;
    rect(
        f,
        cx - half_w - 6.,
        strap,
        cx + half_w + 6.,
        strap + 6.,
        BLOCK,
    );
    for x in [cx - half_w - 6., cx + half_w] {
        rect(f, x, strap, x + 6., bottom, BLOCK);
    }
    if c.y - r > strap + 6. {
        rect(f, cx - 3., strap + 6., cx + 3., c.y - r, color);
    }
    let arrow = Point::new(cx, strap - 2.);
    line(f, Point::new(cx, strap - 14.), arrow, GOLD, 1.6);
    line(f, Point::new(cx - 4., arrow.y - 5.), arrow, GOLD, 1.6);
    line(f, Point::new(cx + 4., arrow.y - 5.), arrow, GOLD, 1.6);
    text(f, "CLAMP", cx + 8., strap - 13., GOLD, 9.);
    if too_big {
        caption(f, "RED: TOO BIG, IT RESTS ON THE V'S EDGES: REFUSED");
    } else {
        caption(f, "GOLD: THE V'S TWO LINE CONTACTS AND THE CLAMP");
    }
}
/// Soft jaws bored to the bar, from its end: the jaws close and each
/// presses round its arc, from the bar's bottom up past its axis.
fn soft_jaws(f: &mut Frame, phase: f32) {
    heading(f, "SOFT JAWS · BORED TO THE BAR, CLOSED ROUND IT");
    let (cx, cy, r) = (139., 82., 26.);
    let bottom = cy + r + 20.;
    // The jaws close over the first third of the phase.
    let open = 14. * (1. - smooth((phase / 0.33).clamp(0., 1.)));
    // Their top: 40° above the axis.
    let lift = 40f32.to_radians();
    let top = cy - r * lift.sin();
    for side in [-1., 1.] {
        let face = cx + side * open;
        let back = cx + side * (r + 46. + open);
        let mut outline = vec![Point::new(face, bottom), Point::new(back, bottom)];
        outline.push(Point::new(back, top));
        // Along the top to the bore, round the bore down to the bottom.
        let n = 16;
        for i in 0..=n {
            let t = -lift + (PI / 2. + lift) * i as f32 / n as f32;
            outline.push(Point::new(
                cx + side * (open + r * t.cos()),
                cy + r * t.sin(),
            ));
        }
        shape(f, &outline, BLOCK, BLOCK_EDGE);
    }
    rect(f, cx - r - 60., bottom, cx + r + 60., bottom + 6., BODY);
    bar(f, Point::new(cx, cy), r);
    if open < 0.5 {
        // Each jaw's arc contact, from the bar's bottom up to the jaw's top.
        for (from, to) in [(PI / 2., PI + lift), (-lift, PI / 2.)] {
            arc(f, Point::new(cx, cy), r + 1.5, from, to, GOLD, 2.2);
        }
        dashed(
            f,
            Point::new(cx - r - 50., cy),
            Point::new(cx + r + 50., cy),
            TEXT,
            1.,
        );
        text(f, "AXIS", cx + r + 26., cy - 12., TEXT, 9.);
        text(f, "JAW TOPS ABOVE THE AXIS", 8., 18., GOLD, 9.);
    }
    caption(f, "GOLD: THE ARC EACH JAW PRESSES ROUND THE BAR");
}
/// A collet block stood in the vise, from the side: the bar stands in the
/// collet, the depth it grips marked.
fn collet(f: &mut Frame, phase: f32) {
    heading(f, "COLLET BLOCK · AN ER COLLET STOOD IN THE VISE");
    let (cx, table) = (139., 134.);
    let (block_top, nut_top) = (98., 82.);
    // The vise jaws on the block's sides, the block, the nut on top.
    rect(f, cx - 88., table - 30., cx - 50., table, BODY);
    rect(f, cx + 50., table - 30., cx + 88., table, BODY);
    rect(f, cx - 50., block_top, cx + 50., table, BLOCK);
    shape(
        f,
        &[
            Point::new(cx - 34., block_top),
            Point::new(cx - 34., nut_top + 4.),
            Point::new(cx - 30., nut_top),
            Point::new(cx + 30., nut_top),
            Point::new(cx + 34., nut_top + 4.),
            Point::new(cx + 34., block_top),
        ],
        BLOCK_EDGE,
        BLOCK,
    );
    // The bar slides down into the collet over the first part of the phase.
    let drop = 10. * (1. - smooth((phase / 0.4).clamp(0., 1.)));
    let (grip, half) = (34., 12.);
    let bar_bottom = nut_top + grip - drop;
    rect(f, cx - half, 30. - drop, cx + half, bar_bottom, BAR);
    rect(
        f,
        cx - half - 20.,
        30. - drop,
        cx + half + 20.,
        40. - drop,
        BAR,
    );
    // The collet's jaws round the bar inside the nut and the block.
    for side in [-1., 1.] {
        let x = cx + side * (half + 3.);
        line(
            f,
            Point::new(x, nut_top),
            Point::new(x, nut_top + 40.),
            if drop < 0.5 { GOLD } else { TEXT },
            2.,
        );
    }
    if drop < 0.5 {
        dimension(
            f,
            Point::new(cx + 62., nut_top),
            Point::new(cx + 62., bar_bottom),
            GOLD,
        );
        text(f, "IN THE COLLET", cx + 68., nut_top + 10., GOLD, 9.);
    }
    caption(f, "GOLD: HOW DEEP THE BAR STANDS IN THE COLLET");
}
