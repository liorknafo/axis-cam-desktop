//! Field pictures: the lathe's chuck (seen from the front and from the
//! side) and its tools (seen from above or in section), each a small scene
//! with the named dimension drawn and made to change.
use crate::illustration::*;
use iced::{
    Color, Point,
    widget::canvas::{Frame, Path, Stroke},
};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

const BODY: Color = Color::from_rgb(0.27, 0.31, 0.36);
const BODY_LIGHT: Color = Color::from_rgb(0.46, 0.51, 0.56);
const JAW: Color = Color::from_rgb(0.52, 0.57, 0.62);
const STEEL: Color = Color::from_rgb(0.62, 0.69, 0.75);
const DARK_STEEL: Color = Color::from_rgb(0.30, 0.36, 0.42);
const INK: Color = Color::from_rgb(0.07, 0.11, 0.15);
const BAR_EDGE: Color = Color::from_rgb(0.95, 0.80, 0.50);
const CUT_EDGE: Color = Color::from_rgb(0.75, 0.83, 0.88);
/// The chuck seen from the front.
const C: Point = Point { x: 139., y: 74. };
/// Three jaws at 120°, one straight up.
const JAWS3: [f32; 3] = [
    -FRAC_PI_2,
    -FRAC_PI_2 + TAU / 3.,
    -FRAC_PI_2 + 2. * TAU / 3.,
];
const JAWS4: [f32; 4] = [0., FRAC_PI_2, PI, 3. * FRAC_PI_2];

fn pt(x: f32, y: f32) -> Point {
    Point::new(x, y)
}
fn ring(f: &mut Frame, c: Point, r: f32, color: Color, w: f32) {
    f.stroke(
        &Path::circle(c, r),
        Stroke::default().with_color(color).with_width(w),
    );
}
fn disc2(f: &mut Frame, c: Point, r: f32, fill: Color) {
    f.fill(&Path::circle(c, r), fill);
}
fn poly_fill(f: &mut Frame, pts: &[Point], fill: Color, edge: Color, w: f32) {
    let path = Path::new(|b| {
        b.move_to(pts[0]);
        for q in &pts[1..] {
            b.line_to(*q);
        }
        b.close();
    });
    f.fill(&path, fill);
    f.stroke(&path, Stroke::default().with_color(edge).with_width(w));
}
fn poly_line(f: &mut Frame, pts: &[Point], color: Color, w: f32) {
    if pts.len() < 2 {
        return;
    }
    let path = Path::new(|b| {
        b.move_to(pts[0]);
        for q in &pts[1..] {
            b.line_to(*q);
        }
    });
    f.stroke(&path, Stroke::default().with_color(color).with_width(w));
}
fn box_o(f: &mut Frame, x0: f32, y0: f32, x1: f32, y1: f32, fill: Color, edge: Color) {
    poly_fill(
        f,
        &[pt(x0, y0), pt(x1, y0), pt(x1, y1), pt(x0, y1)],
        fill,
        edge,
        1.,
    );
}
/// Points of an arc around `c`, screen angles in radians.
fn arc_pts(c: Point, r: f32, a0: f32, a1: f32) -> Vec<Point> {
    (0..=18)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / 18.;
            pt(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect()
}
/// Text centred on `cx`.
fn text_c(f: &mut Frame, s: &str, cx: f32, y: f32, color: Color, size: f32) {
    text(
        f,
        s,
        cx - s.chars().count() as f32 * size * 0.3,
        y,
        color,
        size,
    );
}
/// A line ending in an arrowhead at `b`.
fn arrow(f: &mut Frame, a: Point, b: Point, color: Color) {
    line(f, a, b, color, 1.6);
    let len = (b.x - a.x).hypot(b.y - a.y).max(1e-3);
    let (ux, uy) = ((b.x - a.x) / len, (b.y - a.y) / len);
    for side in [-1., 1.] {
        line(
            f,
            b,
            pt(
                b.x - 6. * ux + side * 3.5 * uy,
                b.y - 6. * uy - side * 3.5 * ux,
            ),
            color,
            1.6,
        );
    }
}
/// A point in a frame turned by `ang` around `c`: `u` along, `v` across.
fn rot(c: Point, ang: f32, u: f32, v: f32) -> Point {
    pt(
        c.x + u * ang.cos() - v * ang.sin(),
        c.y + u * ang.sin() + v * ang.cos(),
    )
}
/// A rise and fall used for jaws: 0 open, 1 closed, open again at the end.
fn closing(t: f32) -> f32 {
    smooth((t - 0.08) / 0.35) * (1. - smooth((t - 0.72) / 0.2))
}

// ---- The chuck from the front ----

const RED: Color = Color::from_rgb(0.95, 0.38, 0.38);
const GREEN: Color = Color::from_rgb(0.40, 0.85, 0.50);
const FAINT: Color = Color::from_rgba(0.26, 0.83, 0.92, 0.6);

/// One jaw as the front sees it: its screen angle, how far its inner face
/// is from the centre, and whether it is the one moving.
type FrontJaw = (f32, f32, bool);

/// A jaw's outline. A round one has an inner face that is an arc of the
/// bar's own circle, so it touches the bar along its surface; a flat one a
/// flat face. Neither reaches into another jaw.
fn jaw_points(a: f32, d: f32, h: f32, w: f32, round: bool) -> Vec<Point> {
    if round {
        let half = 0.38;
        let mut pts = arc_pts(C, d, a - half, a + half);
        pts.extend(arc_pts(C, d + h, a + half, a - half));
        pts
    } else {
        vec![
            rot(C, a, d, -w / 2.),
            rot(C, a, d + h, -w / 2.),
            rot(C, a, d + h, w / 2.),
            rot(C, a, d, w / 2.),
        ]
    }
}

/// The chuck from the front: its body, the slots and the jaws on them,
/// the bar between.
fn front_chuck(
    f: &mut Frame,
    r: f32,
    jaws: &[FrontJaw],
    round: bool,
    w: f32,
    h: f32,
    bar: impl Fn(&mut Frame),
) {
    disc2(f, C, r, BODY);
    ring(f, C, r, BODY_LIGHT, 1.8);
    ring(f, C, r - 5., Color::from_rgb(0.36, 0.41, 0.46), 1.);
    disc2(f, C, r * 0.5, HOLE);
    for (a, _, _) in jaws {
        line(f, C, rot(C, *a, r - 4., 0.), HOLE, 6.);
    }
    bar(f);
    for (a, d, moving) in jaws {
        let pts = jaw_points(*a, *d, h, w, round);
        if *moving {
            poly_fill(f, &pts, JAW, GOLD, 2.2);
        } else {
            poly_fill(f, &pts, JAW, BODY_LIGHT, 1.);
        }
    }
}
fn bar_disc(f: &mut Frame, r: f32) {
    disc2(f, C, r, STOCK.top);
    ring(f, C, r, BAR_EDGE, 1.2);
}
/// The bar's diameter across the bar itself, through its centre, with a
/// gold label outside the chuck.
fn dia_across(f: &mut Frame, r: f32, label: &str) {
    if r < 10. {
        // Too small for arrowheads inside: they point in from outside.
        arrow(f, pt(C.x - r - 14., C.y), pt(C.x - r, C.y), GOLD);
        arrow(f, pt(C.x + r + 14., C.y), pt(C.x + r, C.y), GOLD);
        line(f, pt(C.x - r, C.y), pt(C.x + r, C.y), INK, 1.2);
    } else {
        dimension(f, pt(C.x - r, C.y), pt(C.x + r, C.y), INK);
    }
    dashed(f, pt(C.x + r + 14., C.y), pt(C.x + 54., C.y), GOLD, 1.);
    text(f, label, C.x + 58., C.y - 5., GOLD, 9.);
}

pub fn chuck(f: &mut Frame, _time: f32, phase: f32, n: u8) {
    match n {
        0 => chuck_jaws(f, phase),
        1 => chuck_min_grip(f, phase),
        2 => chuck_max_grip(f, phase),
        3 => chuck_jaw_length(f, phase),
        4 => chuck_body(f, phase),
        5 => chuck_grip(f, phase),
        _ => chuck_stickout(f, phase),
    }
}

fn chuck_jaws(f: &mut Frame, phase: f32) {
    heading(f, "CHUCK JAWS · SELF-CENTRING OR INDEPENDENT");
    if phase < 0.5 {
        let t = phase * 2.;
        let d = 28. - 14. * closing(t);
        let jaws: Vec<FrontJaw> = JAWS3.iter().map(|a| (*a, d, false)).collect();
        front_chuck(f, 48., &jaws, true, 15., 14., |f| bar_disc(f, 14.));
        text(f, "3", 44., 50., GOLD, 40.);
        text(f, "SELF-CENTRING", 196., 62., TEXT, 9.);
        text(f, "JAWS TOGETHER", 196., 74., TEXT, 9.);
        caption(f, "3 JAWS · ROUND BAR");
    } else {
        // One jaw at a time, the moving one outlined in gold.
        let t = (phase - 0.5) * 2.;
        let jaws: Vec<FrontJaw> = JAWS4
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let x = (t - 0.2 * i as f32) / 0.2;
                let open = smooth((t - 0.88) / 0.12);
                (*a, 28. - 15. * smooth(x) * (1. - open), x > 0. && x < 1.)
            })
            .collect();
        front_chuck(f, 48., &jaws, false, 15., 14., |f| {
            poly_fill(
                f,
                &[
                    pt(C.x - 13., C.y - 13.),
                    pt(C.x + 13., C.y - 13.),
                    pt(C.x + 13., C.y + 13.),
                    pt(C.x - 13., C.y + 13.),
                ],
                STOCK.top,
                BAR_EDGE,
                1.2,
            );
        });
        text(f, "4", 44., 50., GOLD, 40.);
        text(f, "INDEPENDENT", 196., 62., TEXT, 9.);
        text(f, "ONE AT A TIME", 196., 74., TEXT, 9.);
        caption(f, "4 JAWS · SQUARE OR OFF-CENTRE");
    }
}

fn chuck_min_grip(f: &mut Frame, phase: f32) {
    heading(f, "MIN GRIP · SMALLEST BAR THE JAWS CLOSE ON");
    let d = 26. - 19. * smooth(progress(phase, 0.35));
    let jaws: Vec<FrontJaw> = JAWS3.iter().map(|a| (*a, d, false)).collect();
    front_chuck(f, 48., &jaws, true, 9., 12., |f| bar_disc(f, 7.));
    dia_across(f, 7., "MIN Ø");
    caption(f, "JAWS CLOSED AS FAR AS THEY GO");
}

fn chuck_max_grip(f: &mut Frame, phase: f32) {
    heading(f, "MAX GRIP · LARGEST BAR THE JAWS CLOSE ON");
    let r = 12. + 18. * smooth(progress(phase, 0.35));
    let jaws: Vec<FrontJaw> = JAWS3.iter().map(|a| (*a, r, false)).collect();
    front_chuck(f, 48., &jaws, true, 15., 13., |f| bar_disc(f, r));
    dia_across(f, r, "MAX Ø");
    // The chuck body's own diameter, faint: the bar stays inside it.
    let y = C.y + 48. + 9.;
    for s in [-1., 1.] {
        dashed(
            f,
            pt(C.x + s * 48., C.y),
            pt(C.x + s * 48., y + 4.),
            FAINT,
            1.,
        );
    }
    dimension(f, pt(C.x - 48., y), pt(C.x + 48., y), FAINT);
    text(f, "BODY Ø", C.x + 56., y - 5., FAINT, 9.);
    caption(f, "MAX GRIP STAYS BELOW THE CHUCK BODY Ø");
}

fn chuck_body(f: &mut Frame, phase: f32) {
    heading(f, "CHUCK BODY · OUTSIDE DIAMETER OF THE CHUCK");
    let r = 42. + 8. * (TAU * phase).sin();
    let k = r / 48.;
    let jaws: Vec<FrontJaw> = JAWS3.iter().map(|a| (*a, 14. * k, false)).collect();
    front_chuck(f, r, &jaws, true, 15. * k, 14. * k, |f| {
        bar_disc(f, 14. * k)
    });
    let y = C.y + 50. + 7.;
    for s in [-1., 1.] {
        dashed(f, pt(C.x + s * r, C.y), pt(C.x + s * r, y + 4.), GOLD, 1.);
    }
    dimension(f, pt(C.x - r, y), pt(C.x + r, y), GOLD);
    text(f, "BODY Ø", C.x + r + 8., y - 5., GOLD, 9.);
    caption(f, "MEASURED ACROSS THE CHUCK'S OUTSIDE");
}

// ---- The chuck from the side ----

fn side_body(f: &mut Frame, face: f32, cy: f32, h: f32) {
    box_o(
        f,
        face - 38.,
        cy - h / 2.,
        face,
        cy + h / 2.,
        BODY,
        BODY_LIGHT,
    );
    rect(f, face - 6., cy - h / 2., face, cy + h / 2., BODY_LIGHT);
}
fn side_bar(f: &mut Frame, x0: f32, x1: f32, cy: f32, r: f32) {
    if x1 <= x0 {
        return;
    }
    rect(f, x0, cy - r, x1, cy + r, STOCK.front);
    rect(f, x0, cy - r, x1, cy - r * 0.35, STOCK.top);
    rect(f, x0, cy + r * 0.45, x1, cy + r, STOCK.side);
    line(f, pt(x0, cy - r), pt(x1, cy - r), BAR_EDGE, 1.);
    line(f, pt(x0, cy + r), pt(x1, cy + r), BAR_EDGE, 1.);
    line(f, pt(x1, cy - r), pt(x1, cy + r), BAR_EDGE, 1.);
}
/// The two jaws seen from the side, pressing on the bar's surface.
fn side_jaws(f: &mut Frame, x0: f32, x1: f32, cy: f32, rb: f32, th: f32) {
    box_o(f, x0, cy - rb - th, x1, cy - rb + 1., JAW, BODY_LIGHT);
    box_o(f, x0, cy + rb - 1., x1, cy + rb + th, JAW, BODY_LIGHT);
}
/// A turning tool seen from the front, its tip at `t`.
fn side_tool(f: &mut Frame, t: Point) {
    poly_fill(
        f,
        &[
            t,
            pt(t.x + 5., t.y - 5.),
            pt(t.x - 4., t.y - 30.),
            pt(t.x - 18., t.y - 30.),
        ],
        STEEL,
        CUT_EDGE,
        1.,
    );
    poly_fill(
        f,
        &[t, pt(t.x + 4., t.y - 4.), pt(t.x - 3., t.y - 6.)],
        GOLD,
        GOLD,
        1.,
    );
}

fn chuck_jaw_length(f: &mut Frame, phase: f32) {
    heading(f, "JAW LENGTH · HOW DEEP THE JAWS REACH");
    let (face, cy, rb, th) = (60., 66., 13., 13.);
    let l = 22. + 48. * smooth(progress(phase, 0.3));
    side_body(f, face, cy, 76.);
    side_bar(f, face, 262., cy, rb);
    side_jaws(f, face, face + l, cy, rb, th);
    let y = 108.;
    for x in [face, face + l] {
        dashed(f, pt(x, cy + rb + th), pt(x, y + 3.), GOLD, 1.);
    }
    dimension(f, pt(face, y), pt(face + l, y), GOLD);
    text(f, "JAW LENGTH", face + 4., y + 5., GOLD, 9.);
    caption(f, "THE MOST BAR A SETUP CAN HOLD IN THE JAWS");
}

fn chuck_grip(f: &mut Frame, phase: f32) {
    heading(f, "GRIP · HOW MUCH OF THE BAR THE JAWS HOLD");
    let (face, cy, rb, th) = (48., 60., 22., 11.);
    let jt = face + 70.;
    let u = progress(phase, 0.1);
    let xe = jt + 10. - 62. * u;
    let third = 2. * rb / 3.;
    let grip = (jt - xe).max(0.);
    let status = if grip >= third { GREEN } else { RED };
    side_body(f, face, cy, 76.);
    side_bar(f, xe, xe + 130., cy, rb);
    side_jaws(f, face, jt, cy, rb, th);
    // The bar's diameter, drawn across the bar.
    let dx = xe + 110.;
    dimension(f, pt(dx, cy - rb), pt(dx, cy + rb), INK);
    text(f, "Ø", dx + 6., cy - 5., INK, 9.);
    // A third of it: the least that should be held.
    let ym = cy + rb + th + 8.;
    dashed(
        f,
        pt(jt - third, cy - rb),
        pt(jt - third, ym + 3.),
        GOLD,
        1.2,
    );
    dashed(f, pt(jt, cy + rb + th), pt(jt, ym + 3.), GOLD, 1.);
    dimension(f, pt(jt - third, ym), pt(jt, ym), GOLD);
    text(f, "Ø/3", jt + 6., ym - 5., GOLD, 9.);
    // How much is held, red until it passes the third.
    let y = 120.;
    if grip > 2. {
        for x in [xe, jt] {
            dashed(f, pt(x, cy + rb), pt(x, y + 3.), status, 1.);
        }
        dimension(f, pt(xe, y), pt(jt, y), status);
    }
    let s = if grip >= third {
        "GRIPPED · OK"
    } else {
        "GRIPPED · TOO SHORT"
    };
    text(f, s, jt + 8., y - 5., status, 9.);
    caption(f, "HOLD ≥ 1/3 OF Ø");
}

fn chuck_stickout(f: &mut Frame, phase: f32) {
    heading(f, "STICKOUT · BAR LENGTH PAST THE JAWS");
    let (face, cy, rb, th) = (48., 62., 13., 13.);
    let jt = face + 30.;
    let xf = jt + 100. + 65. * smooth(progress(phase, 0.3));
    let xc = jt + 0.7 * (xf - jt);
    side_body(f, face, cy, 72.);
    side_bar(f, face, xf, cy, rb);
    side_jaws(f, face, jt, cy, rb, th);
    side_tool(f, pt(xc, cy - rb));
    // The furthest cut the stickout has to reach.
    let yc = 102.;
    dashed(f, pt(xc, cy - rb), pt(xc, yc + 3.), CYAN, 1.);
    dimension(f, pt(jt, yc), pt(xc, yc), CYAN);
    text(f, "MUST REACH", jt + 8., yc - 12., CYAN, 9.);
    // The stickout from the jaw face to the free end.
    let y = 124.;
    for x in [jt, xf] {
        dashed(f, pt(x, cy + rb), pt(x, y + 3.), GOLD, 1.);
    }
    dimension(f, pt(jt, y), pt(xf, y), GOLD);
    text(f, "STICKOUT", jt + 8., y - 12., GOLD, 9.);
    caption(f, "STICKOUT MUST REACH THE FURTHEST CUT");
}

// ---- Tools ----

pub fn tool(f: &mut Frame, _time: f32, phase: f32, n: u8) {
    match n {
        0 => tool_hand(f, phase),
        1 => tool_nose(f, phase),
        2 => tool_lead(f, phase),
        3 => tool_min_bore(f, phase),
        4 => tool_reach(f, phase),
        5 => tool_width(f, phase),
        6 => tool_thread_angle(f, phase),
        _ => tool_pitches(f, phase),
    }
}

fn tool_hand(f: &mut Frame, phase: f32) {
    heading(f, "HAND · WHICH WAY THE TOOL CUTS");
    let k = ((phase * 3.) as usize).min(2);
    let tp = (phase * 3.) % 1.;
    let u = smooth(tp / 0.85);
    let (by0, by1) = (44., 80.);
    let ny = by1 - 5.;
    // The nose, the swept stretch, which side the tool body trails on.
    let (nx, a, b, side) = match k {
        0 => {
            let x = 250. - 130. * u;
            (x, x, 250., 1.)
        }
        1 => {
            let x = 70. + 130. * u;
            (x, 70., x, -1.)
        }
        _ => {
            let at = |s: f32| 160. + 60. * (TAU * s).sin();
            let (mut lo, mut hi) = (160.0_f32, 160.0_f32);
            for i in 0..=24 {
                let x = at(tp * i as f32 / 24.);
                lo = lo.min(x);
                hi = hi.max(x);
            }
            (at(tp), lo, hi, 0.)
        }
    };
    rect(f, 38., by0, 250., by1, STOCK.top);
    if b - a > 0.5 {
        rect(f, a, by0, b, by0 + 5., BACKGROUND);
        rect(f, a, by1 - 5., b, by1, BACKGROUND);
        rect(f, a, by0 + 5., b, by1 - 5., FRESH.top);
    }
    line(f, pt(38., by0), pt(250., by0), BAR_EDGE, 1.);
    line(f, pt(250., by0), pt(250., by1), BAR_EDGE, 1.);
    // The chuck holding the bar.
    box_o(f, 8., 32., 38., 92., BODY, BODY_LIGHT);
    box_o(f, 28., 40., 38., 54., JAW, BODY_LIGHT);
    box_o(f, 28., 70., 38., 84., JAW, BODY_LIGHT);
    text(f, "CHUCK", 6., 94., TEXT, 8.);
    // The tool from above: its head, shank and gold insert.
    let (head, tip) = if side == 0. {
        (
            vec![
                pt(nx, ny),
                pt(nx + 12., ny + 10.),
                pt(nx + 12., ny + 54.),
                pt(nx - 12., ny + 54.),
                pt(nx - 12., ny + 10.),
            ],
            vec![pt(nx, ny), pt(nx + 10., ny + 9.), pt(nx - 10., ny + 9.)],
        )
    } else {
        (
            vec![
                pt(nx, ny),
                pt(nx + side * 20., ny + 12.),
                pt(nx + side * 20., ny + 54.),
                pt(nx, ny + 54.),
            ],
            vec![pt(nx, ny), pt(nx + side * 13., ny + 9.), pt(nx, ny + 14.)],
        )
    };
    poly_fill(f, &head, STEEL, CUT_EDGE, 1.);
    poly_fill(f, &tip, GOLD, GOLD, 1.);
    let (label, cap) = [
        ("RIGHT HAND", "RIGHT-HAND: CUTS TOWARD THE CHUCK"),
        ("LEFT HAND", "LEFT-HAND: CUTS AWAY FROM THE CHUCK"),
        ("NEUTRAL", "NEUTRAL: CUTS EITHER WAY"),
    ][k];
    text(f, label, 48., 20., GOLD, 11.);
    match k {
        0 => arrow(f, pt(205., 27.), pt(145., 27.), CYAN),
        1 => arrow(f, pt(145., 27.), pt(205., 27.), CYAN),
        _ => dimension(f, pt(145., 27.), pt(205., 27.), CYAN),
    }
    caption(f, cap);
}

fn tool_nose(f: &mut Frame, phase: f32) {
    heading(f, "NOSE RADIUS · ROUNDED TIP OF THE INSERT");
    let r = 3. + 24. * smooth(progress(phase, 0.25));
    let (xw, ys, l) = (196., 104., 62.);
    // The work: the bar's surface, the shoulder and the corner left round.
    stock(f, 20., ys, xw, 128.);
    stock(f, xw, 56., 262., 128.);
    let corner = pt(xw - r, ys - r);
    let arc = arc_pts(corner, r, 0., FRAC_PI_2);
    let mut fillet = arc.clone();
    fillet.push(pt(xw, ys));
    poly_fill(f, &fillet, SECTION, SECTION, 0.5);
    line(f, pt(xw, 56.), pt(xw, ys - r), CUT_EDGE, 1.);
    // The insert itself: its corner is rounded to the radius.
    let mut insert = vec![pt(xw, ys - l)];
    insert.extend(arc.iter().copied());
    insert.push(pt(xw - l, ys));
    insert.push(pt(xw - l, ys - l));
    poly_fill(f, &insert, DARK_STEEL, CUT_EDGE, 1.2);
    ring(f, pt(xw - 48., ys - 50.), 5., BACKGROUND, 1.);
    // The square corner it would have without the radius, faint.
    dashed(f, pt(xw, ys - r), pt(xw, ys), FAINT, 1.);
    dashed(f, pt(xw - r, ys), pt(xw, ys), FAINT, 1.);
    // The radius in gold: the rounded edge, and a line from its centre.
    poly_line(f, &arc, GOLD, 2.2);
    line(
        f,
        corner,
        pt(corner.x + r * 0.707, corner.y + r * 0.707),
        GOLD,
        1.4,
    );
    disc2(f, corner, 1.8, GOLD);
    text(f, "R", corner.x - 15., corner.y - 8., GOLD, 11.);
    text(f, "R = NOSE RADIUS", 12., 46., GOLD, 9.);
    caption(f, "BIGGER R = ROUNDER CORNER, SMOOTHER FINISH");
}

fn tool_lead(f: &mut Frame, phase: f32) {
    heading(f, "LEAD ANGLE · CUTTING EDGE VS THE BAR AXIS");
    let theta = (60. + 33. * smooth(progress(phase, 0.3))).to_radians();
    let n = pt(170., 72.);
    // The bar in section, a chuck on the left; the labelled axis line
    // runs along the bar and is the line the angle is measured from.
    rect(f, 40., 34., 268., 72., SECTION);
    box_o(f, 8., 26., 40., 82., BODY, BODY_LIGHT);
    dashed(f, pt(44., 72.), pt(266., 72.), CYAN, 1.6);
    text(f, "BAR AXIS", 52., 58., INK, 9.);
    // The tool, turned about its nose so the edge leans by the angle.
    let e = (-theta.cos(), theta.sin());
    let s = (theta.sin(), theta.cos());
    let at = |a: f32, b: f32| pt(n.x + a * e.0 + b * s.0, n.y + a * e.1 + b * s.1);
    poly_fill(
        f,
        &[n, at(0., 0.), at(52., 0.), at(52., 34.), at(14., 34.)],
        STEEL,
        CUT_EDGE,
        1.,
    );
    line(f, n, at(40., 0.), GOLD, 3.);
    // The angle from the axis line (toward the chuck) to the edge.
    poly_line(f, &arc_pts(n, 26., PI - theta, PI), CYAN, 1.8);
    let mid = PI - theta / 2.;
    let label = format!("LEAD ANGLE {:.0}°", theta.to_degrees());
    let width = label.chars().count() as f32 * 5.4;
    text(
        f,
        &label,
        n.x + 30. * mid.cos() - width,
        n.y + 30. * mid.sin() - 2.,
        GOLD,
        9.,
    );
    caption(f, "GOLD: CUTTING EDGE   93° IS TYPICAL");
}

fn work_walls(f: &mut Frame, x0: f32, x1: f32, top: f32, bot: f32, y0: f32, y1: f32) {
    // Stock above and below a bore running from `y0` to `y1`.
    stock(f, x0, top, x1, y0);
    stock(f, x0, y1, x1, bot);
    line(f, pt(x0, y0), pt(x1, y0), CUT_EDGE, 1.);
}
fn boring_bar(f: &mut Frame, x_tip: f32, yc: f32) {
    box_o(f, x_tip, yc - 12., 268., yc + 12., STEEL, CUT_EDGE);
    rect(
        f,
        x_tip + 1.,
        yc - 11.,
        267.,
        yc - 4.,
        Color::from_rgb(0.74, 0.8, 0.85),
    );
    poly_fill(
        f,
        &[
            pt(x_tip, yc - 12.),
            pt(x_tip, yc - 16.),
            pt(x_tip + 10., yc - 16.),
            pt(x_tip + 12., yc - 12.),
        ],
        GOLD,
        GOLD,
        1.,
    );
}

fn tool_min_bore(f: &mut Frame, phase: f32) {
    heading(f, "MIN BORE · SMALLEST HOLE THE BAR GOES INTO");
    let yc = 76.;
    let d = 76. - 42. * smooth(progress(phase, 0.25));
    let (y0, y1) = (yc - d / 2., yc + d / 2.);
    work_walls(f, 80., 236., 24., 128., y0, y1);
    line(f, pt(80., y1), pt(236., y1), CUT_EDGE, 1.);
    boring_bar(f, 140., yc);
    for y in [y0, y1] {
        dashed(f, pt(80., y), pt(58., y), GOLD, 1.);
    }
    dimension(f, pt(62., y0), pt(62., y1), GOLD);
    text(f, "MIN BORE", 4., yc - 4., GOLD, 9.);
    caption(f, "THE BAR HAS TO FIT: SMALLER AND IT JAMS");
}

fn tool_reach(f: &mut Frame, phase: f32) {
    heading(f, "REACH · HOW FAR THE TOOL GOES INTO THE WORK");
    let yc = 70.;
    let (face, floor) = (206., 92.);
    let x_tip = 214. - 116. * smooth(progress(phase, 0.3));
    // A blind bore: stock with the hole cut out of it.
    stock(f, 70., 22., face, 116.);
    rect(f, floor, yc - 22., face, yc + 22., BACKGROUND);
    for y in [yc - 22., yc + 22.] {
        line(f, pt(floor, y), pt(face, y), CUT_EDGE, 1.);
    }
    line(f, pt(floor, yc - 22.), pt(floor, yc + 22.), CUT_EDGE, 1.);
    boring_bar(f, x_tip, yc);
    let y = 128.;
    if face - x_tip > 3. {
        for x in [face, x_tip] {
            dashed(f, pt(x, 116.), pt(x, y + 3.), GOLD, 1.);
        }
        dimension(f, pt(x_tip, y), pt(face, y), GOLD);
        text(f, "REACH", face + 6., y - 5., GOLD, 9.);
    }
    caption(f, "BORING DEPTH, OR A GROOVING OR PARTING PLUNGE");
}

fn tool_width(f: &mut Frame, phase: f32) {
    heading(f, "WIDTH · THE BLADE'S CUTTING EDGE WIDTH");
    let (top, w) = (64., 24.);
    let plunge = smooth(phase / 0.3);
    let side = smooth((phase - 0.45) / 0.25);
    let retract = smooth((phase - 0.85) / 0.13);
    let depth = 38. * plunge;
    let tip = 54. + depth - 38. * retract;
    let bx = 112. + 22. * side;
    stock(f, 30., top, 250., 124.);
    let (gx0, gx1) = (112., bx + w);
    if depth > 0.5 {
        rect(f, gx0, top, gx1, top + depth, BACKGROUND);
        for x in [gx0, gx1] {
            line(f, pt(x, top), pt(x, top + depth), CUT_EDGE, 1.);
        }
        line(f, pt(gx0, top + depth), pt(gx1, top + depth), CUT_EDGE, 1.);
    }
    // The blade, held from above.
    box_o(f, bx, 20., bx + w, tip, DARK_STEEL, CUT_EDGE);
    line(f, pt(bx, tip), pt(bx + w, tip), GOLD, 2.8);
    dimension(f, pt(bx, 38.), pt(bx + w, 38.), GOLD);
    text(f, "WIDTH", bx + w + 6., 33., GOLD, 9.);
    if depth > 2. {
        let y = 134.;
        for x in [gx0, gx1] {
            dashed(f, pt(x, top + depth), pt(x, y + 3.), CYAN, 1.);
        }
        dimension(f, pt(gx0, y), pt(gx1, y), CYAN);
        text(f, "GROOVE", gx1 + 8., y - 5., CYAN, 9.);
    }
    caption(f, "GROOVE IS AT LEAST AS WIDE AS THE BLADE");
}

fn tool_thread_angle(f: &mut Frame, phase: f32) {
    heading(f, "THREAD ANGLE · INCLUDED ANGLE OF THE FORM");
    // 60° for a while, over to 55° and back.
    let m = smooth((phase - 0.42) / 0.1) - smooth((phase - 0.92) / 0.08);
    let alpha = (60. - 5. * m).to_radians();
    let half = alpha / 2.;
    let (cx, ys, hd, p) = (118., 74., 30., 40.);
    // The thread cut in the bar: grooves a pitch apart.
    let hw = hd * half.tan();
    let mut profile = vec![pt(8., ys)];
    for k in -2..=2 {
        let x = cx + k as f32 * p;
        profile.extend([pt(x - hw, ys), pt(x, ys + hd), pt(x + hw, ys)]);
    }
    profile.push(pt(268., ys));
    let mut body = profile.clone();
    body.extend([pt(268., 124.), pt(8., 124.)]);
    poly_fill(f, &body, SECTION, SECTION, 0.5);
    // The threading insert sitting in the middle groove.
    let apex = pt(cx, ys + hd);
    let top = 50.;
    let wt = (apex.y - top) * half.tan();
    poly_fill(
        f,
        &[
            apex,
            pt(cx - wt, top),
            pt(cx - wt, 24.),
            pt(cx + wt, 24.),
            pt(cx + wt, top),
        ],
        DARK_STEEL,
        CUT_EDGE,
        1.2,
    );
    // The groove walls show along the insert.
    poly_line(f, &profile, CUT_EDGE, 1.6);
    poly_line(
        f,
        &arc_pts(apex, 22., -FRAC_PI_2 - half, -FRAC_PI_2 + half),
        GOLD,
        2.6,
    );
    let (deg, name) = if m < 0.5 {
        ("60°", ["METRIC", "AND UNIFIED"])
    } else {
        ("55°", ["WHITWORTH", ""])
    };
    text(f, deg, cx + 6., apex.y - 50., GOLD, 10.);
    text(f, deg, 206., 24., GOLD, 20.);
    text(f, name[0], 206., 44., TEXT, 9.);
    text(f, name[1], 206., 55., TEXT, 9.);
    caption(f, "60° METRIC AND UNIFIED · 55° WHITWORTH");
}

fn tool_pitches(f: &mut Frame, phase: f32) {
    heading(f, "PITCH RANGE · FINEST AND COARSEST CUT");
    // Fine (0.5 mm), over to coarse (3 mm) and back.
    let m = smooth((phase - 0.38) / 0.14) - smooth((phase - 0.88) / 0.12);
    let p = 14. * 6.0_f32.powf(m);
    let (ys, h) = (80., 0.54 * p);
    let crest = |k: f32| 139. + (k - 0.5) * p;
    let mut profile = Vec::new();
    for k in -30..30 {
        let c = crest(k as f32);
        for (x, y) in [
            (c - p / 16., ys),
            (c + p / 16., ys),
            (c + p * 0.375, ys + h),
            (c + p * 0.625, ys + h),
        ] {
            if (8. ..=270.).contains(&x) {
                profile.push(pt(x, y));
            }
        }
    }
    let mut body = vec![pt(profile[0].x, 124.)];
    body.extend(profile.iter().copied());
    body.push(pt(profile[profile.len() - 1].x, 124.));
    poly_fill(f, &body, SECTION, SECTION, 0.5);
    poly_line(f, &profile, CUT_EDGE, 1.4);
    // The pitch from one crest to the next.
    let y = ys - 10.;
    for k in [0., 1.] {
        dashed(f, pt(crest(k), ys - 2.), pt(crest(k), y - 4.), GOLD, 1.);
    }
    dimension(f, pt(crest(0.), y), pt(crest(1.), y), GOLD);
    text_c(f, "PITCH", 139., y - 16., GOLD, 9.);
    let fine = m < 0.5;
    text(
        f,
        "FINEST 0.5 MM",
        10.,
        22.,
        if fine { GOLD } else { DIM },
        10.,
    );
    text(
        f,
        "COARSEST 3 MM",
        150.,
        22.,
        if fine { DIM } else { GOLD },
        10.,
    );
    caption(f, "TOOL RANGE: 0.5–3 MM PITCH");
}
