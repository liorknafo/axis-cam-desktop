//! Field pictures: probing, the post and linking.
use crate::choice_diagram::SPIN;
use crate::illustration::*;
use iced::{
    Color, Point,
    widget::canvas::{Frame, Path, Stroke},
};
use std::f32::consts::{PI, TAU};

fn pt(x: f32, y: f32) -> Point {
    Point::new(x, y)
}
fn fl(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
/// 0 to 1 while the phase runs from `a` to `b`.
fn win(phase: f32, a: f32, b: f32) -> f32 {
    ((phase - a) / (b - a)).clamp(0., 1.)
}
fn fade(c: Color, a: f32) -> Color {
    Color { a, ..c }
}
/// Which of `n` equal parts of the phase this is, and how far into it.
fn parts(phase: f32, n: usize) -> (usize, f32) {
    let u = phase * n as f32;
    let i = (u as usize).min(n - 1);
    (i, (u - i as f32).clamp(0., 1.))
}
fn label(f: &mut Frame, s: &str, x: f32, y: f32, c: Color) {
    text(f, s, x, y, c, 9.);
}
fn mode(f: &mut Frame, s: &str, x: f32, y: f32, c: Color) {
    text(f, s, x, y, c, 10.);
}
fn dot(f: &mut Frame, c: Point, r: f32, color: Color) {
    f.fill(&Path::circle(c, r), color);
}
fn ring(f: &mut Frame, c: Point, r: f32, color: Color, w: f32) {
    f.stroke(
        &Path::circle(c, r),
        Stroke::default().with_color(color).with_width(w),
    );
}
fn arrow(f: &mut Frame, a: Point, b: Point, color: Color) {
    line(f, a, b, color, 1.4);
    let len = (b.x - a.x).hypot(b.y - a.y).max(1e-3);
    let (ux, uy) = ((b.x - a.x) / len, (b.y - a.y) / len);
    for side in [-1., 1.] {
        line(
            f,
            b,
            pt(
                b.x - 5. * ux + side * 3. * uy,
                b.y - 5. * uy - side * 3. * ux,
            ),
            color,
            1.4,
        );
    }
}
/// The first fraction `u` of a polyline, drawn; returns where it ends.
fn trace(f: &mut Frame, pts: &[Point], u: f32, color: Color, w: f32) -> Point {
    let total: f32 = pts
        .windows(2)
        .map(|s| (s[1].x - s[0].x).hypot(s[1].y - s[0].y))
        .sum();
    let mut left = u.clamp(0., 1.) * total;
    let mut head = pts[0];
    for s in pts.windows(2) {
        let l = (s[1].x - s[0].x).hypot(s[1].y - s[0].y);
        if l <= 0. {
            continue;
        }
        let k = (left / l).clamp(0., 1.);
        head = pt(fl(s[0].x, s[1].x, k), fl(s[0].y, s[1].y, k));
        if k > 0. {
            line(f, s[0], head, color, w);
        }
        left -= l;
        if left <= 0. {
            break;
        }
    }
    head
}
fn chev(f: &mut Frame, at: Point, dir: f32, color: Color) {
    line(f, pt(at.x - dir * 4., at.y - 3.), at, color, 1.4);
    line(f, pt(at.x - dir * 4., at.y + 3.), at, color, 1.4);
}
/// A cutter seen from above.
fn top_cutter(f: &mut Frame, c: Point, r: f32, time: f32) {
    dot(f, c, r, Color::from_rgba(0.2, 0.3, 0.38, 0.85));
    ring(f, c, r, GOLD, 1.6);
    for tooth in 0..4 {
        let a = time * SPIN + tooth as f32 * TAU / 4.;
        line(
            f,
            pt(c.x + r * 0.4 * a.cos(), c.y + r * 0.4 * a.sin()),
            pt(c.x + r * a.cos(), c.y + r * a.sin()),
            GOLD,
            2.,
        );
    }
}
/// A probe side on, its ball's lowest point at `tip_y`. Past the contact
/// the body goes on down by `over` while the stylus slides up inside it.
fn probe_at(f: &mut Frame, x: f32, tip_y: f32, over: f32) {
    let r = 4.;
    let body_bottom = (tip_y - 2. * r - 18. + over).max(20.);
    rect(
        f,
        x - 1.5,
        body_bottom,
        x + 1.5,
        tip_y - r,
        Color::from_rgb(0.87, 0.91, 0.93),
    );
    rect(
        f,
        x - 7.,
        (body_bottom - 26.).max(20.),
        x + 7.,
        body_bottom,
        Color::from_rgb(0.58, 0.67, 0.75),
    );
    dot(f, pt(x, tip_y - r), r, GOLD);
}
fn plate(f: &mut Frame, x0: f32, y0: f32, x1: f32, y1: f32) {
    rect(f, x0, y0, x1, y1, METAL.front);
    line(
        f,
        pt(x0, y0),
        pt(x1, y0),
        Color::from_rgb(0.75, 0.83, 0.88),
        1.,
    );
}

pub fn probe(f: &mut Frame, time: f32, phase: f32, n: u8) {
    match n {
        0 => probe_plate(f, time, phase),
        1 => probe_pins(f, phase),
        2 => probe_pin_diameter(f, phase),
        3 => probe_pin_height(f, phase),
        4 => probe_tip(f, phase),
        5 => probe_feed(f, phase),
        6 => probe_distance(f, phase),
        7 => probe_retract(f, phase),
        _ => heading(f, "?"),
    }
}

/// Touch plate: the tool touches a plate lying on the stock; Z 0 is the
/// surface under it.
fn probe_plate(f: &mut Frame, time: f32, phase: f32) {
    let top = 96.;
    stock(f, 24., top, 254., 134.);
    let (x0, x1, y0, cx) = (74., 140., 82., 107.);
    plate(f, x0, y0, x1, top);
    let touch = 0.36;
    let k = smooth(win(phase, 0.05, touch)) * (1. - smooth(win(phase, 0.64, 0.82)));
    let tip = fl(62., y0, k);
    // Thickness.
    line(f, pt(x1, y0), pt(162., y0), GOLD, 1.);
    line(f, pt(x1, top), pt(162., top), GOLD, 1.);
    dimension(f, pt(156., y0), pt(156., top), GOLD);
    label(f, "PLATE THICKNESS", 166., 80., GOLD);
    if phase > touch + 0.04 {
        let a = smooth(win(phase, touch + 0.04, touch + 0.12));
        dashed(f, pt(24., top), pt(254., top), fade(GOLD, a), 1.4);
        text(f, "Z 0", 28., top - 12., fade(GOLD, a), 10.);
    }
    side_cutter(f, cx, tip, time, 14., false);
    if phase > touch && phase < 0.64 {
        // Contact flicks beside the tip, and what the touch sets.
        for s in [-1., 1.] {
            for (dy, dx) in [(-2., 8.), (-6., 12.), (-10., 8.)] {
                line(
                    f,
                    pt(cx + s * 11., y0 + dy / 2.),
                    pt(cx + s * (11. + dx), y0 + dy),
                    CYAN,
                    1.4,
                );
            }
        }
        label(f, "Z = PLATE THICKNESS", 122., 46., CYAN);
        line(f, pt(130., 57.), pt(124., y0 - 1.), CYAN, 1.);
    }
    heading(f, "TOUCH PLATE THICKNESS · SETS Z 0");
    caption(f, "Z 0 IS THE SURFACE UNDER THE PLATE");
}

/// Pins: a plate with pin 1 as the reference and pin 2 along X.
fn probe_pins(f: &mut Frame, phase: f32) {
    let (p1, p2) = (pt(80., 98.), pt(200., 98.));
    rect(f, 40., 30., 238., 132., METAL.front);
    let drop = smooth(win(phase, 0.42, 0.74));
    let h = 1. - drop;
    for p in [p1, p2] {
        dot(f, p, 4.5, if p == p1 { GOLD } else { CYAN });
    }
    if phase >= 0.4 {
        let a = win(phase, 0.4, 0.46);
        let o = -10. * h;
        // The shadow shrinks onto the part as it lands.
        let sh = 9. * h + 1.;
        rect(
            f,
            56. + sh,
            90. + sh,
            224. + sh,
            106. + sh,
            Color::from_rgba(0., 0., 0., 0.28 * a),
        );
        rect(
            f,
            56. + o,
            90. + o,
            224. + o,
            106. + o,
            fade(METAL.top, 0.2 + 0.8 * a),
        );
        for p in [p1, p2] {
            let c = pt(p.x + o, p.y + o);
            dot(f, c, 7., HOLE);
            ring(f, c, 7., EDGE, 1.);
            // The pin shows in its hole once the part is over it.
            let pa = 1. - smooth(h * 3.);
            dot(f, p, 4.5, fade(if p == p1 { GOLD } else { CYAN }, pa));
        }
    }
    label(f, "PIN 1 · REFERENCE", 44., 46., GOLD);
    line(f, pt(80., 57.), pt(80., 90.), GOLD, 1.);
    label(f, "PIN 2 · TURN", 162., 46., CYAN);
    line(f, pt(200., 57.), pt(200., 90.), CYAN, 1.);
    dimension(f, pt(80., 76.), pt(200., 76.), CYAN);
    label(f, "PIN SPACING", 112., 64., CYAN);
    heading(f, "FIXTURE PLATE PINS · IN THE PLATE'S X Y");
    caption(f, "A PART WITH TWO HOLES DROPS OVER THEM");
}

/// Pin diameter: the part's hole is the pin's diameter to 0.2 over; a
/// wider one lets the part shift.
fn probe_pin_diameter(f: &mut Frame, phase: f32) {
    let (half, q) = parts(phase, 2);
    plate(f, 24., 112., 254., 134.);
    let (cx, d) = (139., 30.);
    let (gap, shift) = if half == 0 {
        (2., 0.)
    } else {
        let s = if q > 0.2 {
            8. * ((q - 0.2) / 0.8 * TAU * 1.5).sin()
        } else {
            0.
        };
        (fl(2., 9., smooth(win(q, 0., 0.15))), s)
    };
    let (hl, hr) = (cx - d / 2. - gap, cx + d / 2. + gap);
    stock(f, 40. + shift, 66., hl + shift, 112.);
    stock(f, hr + shift, 66., 238. + shift, 112.);
    rect(f, cx - d / 2., 84., cx + d / 2., 112., METAL.top);
    for x in [cx - d / 2., cx + d / 2.] {
        line(f, pt(x, 84.), pt(x, 112.), EDGE, 1.);
        dashed(f, pt(x, 84.), pt(x, 54.), GOLD, 1.);
    }
    line(f, pt(cx - d / 2., 84.), pt(cx + d / 2., 84.), EDGE, 1.);
    dimension(f, pt(cx - d / 2., 58.), pt(cx + d / 2., 58.), GOLD);
    mode(f, "PIN Ø", cx + d / 2. + 8., 52., GOLD);
    if half == 0 {
        mode(f, "HOLE = Ø + 0.2 MM · SNUG", 24., 28., CYAN);
        // The gap is drawn larger than life.
        label(f, "+0.2 GAP", 208., 50., CYAN);
        line(f, pt(214., 61.), pt(cx + d / 2. + gap / 2., 100.), CYAN, 1.);
        caption(f, "SNUG: THE HOLE IS THE PIN Ø TO 0.2 MM OVER");
    } else {
        mode(f, "WIDER HOLE · PART IS FREE", 24., 28., GOLD);
        arrow(f, pt(104., 46.), pt(84., 46.), GOLD);
        arrow(f, pt(104., 46.), pt(124., 46.), GOLD);
        label(f, "MORE THAN +0.2", 130., 41., GOLD);
        caption(f, "WIDER: THE PART IS FREE TO MOVE");
    }
    heading(f, "PIN Ø · THE HOLE FITS TO 0.2 MM OVER");
}

/// Pin height: how far the pins stand above the plate; the part's holes
/// must be at least as deep.
fn probe_pin_height(f: &mut Frame, phase: f32) {
    let (half, q) = parts(phase, 2);
    let (h, depth) = if half == 0 { (26., 34.) } else { (44., 34.) };
    let land = if h <= depth { 112. } else { 112. - (h - depth) };
    let yb = land - 26. * (1. - smooth(win(q, 0.08, 0.5)));
    let top = yb - 44.;
    plate(f, 24., 112., 254., 134.);
    stock(f, 56., top, 125., yb);
    stock(f, 153., top, 196., yb);
    stock(f, 125., top, 153., top + 44. - depth);
    rect(f, 131., 112. - h, 147., 112., METAL.top);
    line(f, pt(131., 112. - h), pt(147., 112. - h), EDGE, 1.);
    // The pin's height.
    dashed(f, pt(147., 112. - h), pt(222., 112. - h), GOLD, 1.);
    dimension(f, pt(222., 112. - h), pt(222., 112.), GOLD);
    label(f, "PIN", 230., 112. - h / 2. - 10., GOLD);
    label(f, "HEIGHT", 230., 112. - h / 2. + 1., GOLD);
    // The hole's depth.
    let ceil = top + 44. - depth;
    line(f, pt(114., ceil), pt(125., ceil), CYAN, 1.);
    dimension(f, pt(114., yb), pt(114., ceil), CYAN);
    label(f, "HOLE", 64., (yb + ceil) / 2. - 10., CYAN);
    label(f, "DEPTH", 64., (yb + ceil) / 2. + 1., CYAN);
    if half == 0 {
        mode(f, "OK · HOLE IS DEEP ENOUGH", 8., 15., CYAN);
        caption(f, "HOLE DEPTH AT LEAST PIN HEIGHT: PART SITS FLAT");
    } else {
        mode(f, "TOO SHALLOW", 8., 15., GOLD);
        caption(f, "SHALLOWER HOLE: PART STANDS ON THE PIN");
    }
    heading(f, "PIN HEIGHT · HOW FAR THE PINS STAND UP");
}

/// Probe tip: the ball's diameter; the stock's edge is half of it from the
/// ball's centre.
fn probe_tip(f: &mut Frame, phase: f32) {
    let (edge, y, r) = (176., 88., 18.);
    rect(f, edge, 30., 254., 128., STOCK.top);
    line(f, pt(edge, 30.), pt(edge, 128.), EDGE, 1.2);
    text(f, "STOCK", 208., 40., Color::from_rgb(0.3, 0.2, 0.05), 9.);
    let cx = fl(74., edge - r, smooth(win(phase, 0.05, 0.45)));
    let c = pt(cx, y);
    dot(f, c, r, METAL.top);
    ring(f, c, r, CYAN, 1.5);
    dot(f, c, 2., TEXT);
    // The ball's diameter, on its free side.
    for yy in [y - r, y + r] {
        dashed(f, pt(cx - r, yy), pt(cx - r - 12., yy), GOLD, 1.);
    }
    dimension(f, pt(cx - r - 8., y - r), pt(cx - r - 8., y + r), GOLD);
    mode(f, "TIP Ø", cx - r - 44., y - 5., GOLD);
    if phase >= 0.45 {
        let a = smooth(win(phase, 0.45, 0.55));
        dashed(f, c, pt(cx, y + r + 14.), fade(CYAN, a), 1.);
        dimension(f, pt(cx, y + r + 10.), pt(edge, y + r + 10.), fade(CYAN, a));
        text(f, "Ø/2", cx - 24., y + r + 6., fade(CYAN, a), 10.);
        // The touch, held to the end.
        let pulse = 0.5 + 0.5 * ((phase - 0.45) * TAU * 3.).sin();
        dot(f, pt(edge, y), 3., GOLD);
        ring(
            f,
            pt(edge, y),
            7. + 4. * pulse,
            fade(GOLD, 0.9 - 0.5 * pulse),
            1.5,
        );
    }
    heading(f, "PROBE TIP Ø · EDGE IS Ø/2 FROM CENTRE");
    caption(f, "BALL TOUCHES THE SIDE, CENTRE Ø/2 FROM THE EDGE");
}

/// Probe feed: Z feeds down slowly, overshoots a little past the contact
/// and stops; the contact point is what is used.
fn probe_feed(f: &mut Frame, phase: f32) {
    let (top, hit, x) = (104., 94., 140.);
    stock(f, 24., top, 254., 134.);
    plate(f, 90., hit, 190., top);
    let down = win(phase, 0.04, 0.56);
    let tip = fl(58., hit, down);
    let over = 14. * smooth(win(phase, 0.56, 0.68));
    // The body's lower end at the moment of contact.
    let contact_body = hit - 8. - 18.;
    if phase > 0.56 {
        let a = smooth(win(phase, 0.56, 0.68));
        dashed(f, pt(24., hit), pt(254., hit), fade(CYAN, a), 1.4);
        label(f, "CONTACT = USED", 28., hit - 12., fade(CYAN, a));
        dashed(
            f,
            pt(x + 8., contact_body),
            pt(x + 22., contact_body),
            fade(GOLD, a),
            1.,
        );
        dimension(
            f,
            pt(x + 20., contact_body),
            pt(x + 20., contact_body + over),
            fade(GOLD, a),
        );
    }
    probe_at(f, x, tip, over);
    label(f, "F 50 MM/MIN", 156., 34., CYAN);
    if phase < 0.56 {
        arrow(f, pt(118., 40.), pt(118., 66.), CYAN);
    }
    if phase > 0.68 {
        label(f, "STOPS A LITTLE PAST", 164., 50., GOLD);
        line(
            f,
            pt(176., 60.),
            pt(x + 22., contact_body + over / 2.),
            GOLD,
            1.,
        );
    }
    heading(f, "PROBE FEED · SPEED OF Z DOWN TO THE PLATE");
    caption(f, "SLOW IS ACCURATE · THE CONTACT POINT IS USED");
}

/// Probe distance: how far down it goes at most; with no contact by then
/// it stops and nothing is set.
fn probe_distance(f: &mut Frame, phase: f32) {
    let (half, q) = parts(phase, 2);
    let (start, limit, top, x) = (58., 100., 106., 130.);
    stock(f, 24., top, 254., 134.);
    let tip = if half == 0 {
        let k = smooth(win(q, 0.05, 0.4)) * (1. - smooth(win(q, 0.75, 1.)));
        plate(f, 96., 92., 164., top);
        if q > 0.4 && q < 0.9 {
            let t = win(q, 0.4, 0.6);
            ring(f, pt(x, 92.), fl(3., 11., t), fade(CYAN, 1. - t), 1.5);
            mode(f, "TOUCHES · Z SET", 24., 70., CYAN);
        }
        fl(start, 92., k)
    } else {
        let k = smooth(win(q, 0.05, 0.55)) * (1. - smooth(win(q, 0.8, 1.)));
        label(f, "NO PLATE", 168., 108., TEXT);
        if q > 0.55 && q < 0.95 {
            mode(f, "NOTHING SET", 24., 70., GOLD);
        }
        fl(start, limit, k)
    };
    dashed(f, pt(60., limit), pt(218., limit), GOLD, 1.2);
    dashed(f, pt(120., start), pt(218., start), DIM, 1.);
    dimension(f, pt(214., start), pt(214., limit), GOLD);
    label(f, "MAX", 222., 64., GOLD);
    label(f, "DISTANCE", 222., 75., GOLD);
    probe_at(f, x, tip, 0.);
    heading(f, "PROBE DISTANCE · HOW FAR DOWN IT GOES");
    if half == 0 {
        caption(f, "PLATE WITHIN THE DISTANCE: IT TOUCHES, Z IS SET");
    } else {
        caption(f, "NO PLATE: IT STOPS AT THE LIMIT, NOTHING SET");
    }
}

/// Probe retract: after the contact the tool lifts so the plate can come
/// away.
fn probe_retract(f: &mut Frame, phase: f32) {
    let (top, hit, x) = (104., 92., 128.);
    stock(f, 24., top, 254., 134.);
    let slide = 80. * smooth(win(phase, 0.58, 0.84));
    plate(f, 100. + slide, hit, 170. + slide, top);
    let down = smooth(win(phase, 0.04, 0.28));
    let lift = smooth(win(phase, 0.38, 0.52));
    let tip = fl(56., hit, down) - 18. * lift;
    if phase > 0.28 && phase < 0.42 {
        let t = win(phase, 0.28, 0.42);
        ring(f, pt(x, hit), fl(3., 11., t), fade(CYAN, 1. - t), 1.5);
    }
    if lift > 0.5 {
        dashed(f, pt(x, hit), pt(160., hit), GOLD, 1.);
        dimension(f, pt(152., hit - 18. * lift), pt(152., hit), GOLD);
        label(f, "RETRACT", 160., 76., GOLD);
    }
    if slide > 1. {
        label(f, "PLATE COMES AWAY", 176., 58., TEXT);
    }
    probe_at(f, x, tip, 0.);
    heading(f, "PROBE RETRACT · LIFT AFTER THE CONTACT");
    caption(f, "THE TOOL LIFTS SO THE PLATE CAN COME AWAY");
}

pub fn post(f: &mut Frame, time: f32, phase: f32, n: u8) {
    match n {
        0 => post_tool_length(f, time, phase),
        1 => post_retract(f, time, phase),
        2 => post_blend(f, time, phase),
        3 => post_dwell(f, phase),
        4 => post_air(f, time, phase),
        _ => heading(f, "?"),
    }
}

/// Tool length: a tool table with G43 H# or one program per tool.
fn post_tool_length(f: &mut Frame, time: f32, phase: f32) {
    let (half, q) = parts(phase, 2);
    let grey = Color::from_rgb(0.58, 0.67, 0.75);
    if half == 0 {
        let yg = 34.;
        dashed(f, pt(24., yg), pt(254., yg), CYAN, 1.2);
        label(f, "GAUGE LINE", 216., 20., CYAN);
        mode(f, "TOOL TABLE · G43 H#", 8., 15., GOLD);
        for (i, (x, l)) in [(60., 40.), (130., 58.), (200., 76.)]
            .into_iter()
            .enumerate()
        {
            let tip = yg + l;
            rect(f, x - 9., 28., x + 9., yg, grey);
            rect(
                f,
                x - 4.,
                yg,
                x + 4.,
                tip - 40.,
                Color::from_rgb(0.87, 0.91, 0.93),
            );
            side_cutter(f, x, tip, time, 14., false);
            let r = smooth(win(q, 0.05 + 0.22 * i as f32, 0.25 + 0.22 * i as f32));
            if r > 0.02 {
                dimension(f, pt(x + 16., yg), pt(x + 16., yg + l * r), CYAN);
            }
            if r >= 1. {
                dashed(f, pt(x + 7., tip), pt(x + 20., tip), DIM, 1.);
                label(f, &format!("H{}", i + 1), x + 21., yg + l / 2. - 4., GOLD);
            }
        }
        caption(f, "EACH TOOL'S LENGTH FROM ONE GAUGE LINE");
    } else {
        let top = 112.;
        stock(f, 24., top, 254., 134.);
        mode(f, "ONE PROGRAM PER TOOL · TOUCH OFF", 8., 15., GOLD);
        label(f, "PROGRAM T1", 24., 60., TEXT);
        let k = smooth(win(q, 0.08, 0.55));
        side_cutter(f, 139., fl(72., top, k), time, 14., false);
        if q > 0.55 {
            let a = smooth(win(q, 0.55, 0.7));
            dashed(f, pt(24., top), pt(254., top), fade(GOLD, a), 1.4);
            text(f, "Z 0", 28., top - 12., fade(GOLD, a), 10.);
            label(f, "TOUCH OFF", 156., 94., fade(CYAN, a));
            let t = win(q, 0.55, 0.75);
            ring(f, pt(139., top), fl(3., 11., t), fade(CYAN, 1. - t), 1.5);
        }
        caption(f, "THE TOOL IS TOUCHED OFF ON THE STOCK TOP");
    }
    heading(f, "TOOL LENGTH · TABLE OFFSETS OR TOUCH OFF");
}

/// Retract: where the tool goes before a tool change and at the end.
fn post_retract(f: &mut Frame, time: f32, phase: f32) {
    let (half, q) = parts(phase, 2);
    let top = 104.;
    let cx = 200.;
    stock(f, 24., top, 254., 134.);
    rect(f, cx - 12., top, cx + 12., 116., BACKGROUND);
    let (ym, yc) = (62., 82.);
    let on = |i: usize| if i == half { 1. } else { 0.3 };
    dashed(f, pt(24., ym), pt(254., ym), fade(CYAN, on(0)), 1.4);
    dashed(f, pt(24., yc), pt(254., yc), fade(GOLD, on(1)), 1.4);
    label(f, "G53 MACHINE Z0", 24., ym - 12., fade(CYAN, on(0)));
    label(f, "CLEARANCE HEIGHT", 24., yc - 12., fade(GOLD, on(1)));
    let target = if half == 0 { ym } else { yc };
    let k = smooth(win(q, 0.1, 0.6));
    if k > 0.35 {
        arrow(
            f,
            pt(cx + 28., 100.),
            pt(cx + 28., fl(100., target + 6., k)),
            fade(TEXT, 0.7),
        );
    }
    side_cutter(f, cx, fl(116., target, k), time, 14., false);
    if half == 0 {
        mode(f, "OPTION 1 · G53 MACHINE Z0", 8., 15., CYAN);
        caption(f, "G53 MACHINE Z0 = THE TOP OF TRAVEL");
    } else {
        mode(f, "OPTION 2 · CLEARANCE HEIGHT", 8., 15., GOLD);
        caption(f, "THE SETUP'S HIGHEST CLEARANCE HEIGHT");
    }
    heading(f, "RETRACT BEFORE A TOOL CHANGE · AT THE END");
}

/// Blend tolerance: G64 P rounds a corner by up to P, G61 stops exactly.
fn post_blend(f: &mut Frame, time: f32, phase: f32) {
    let (half, q) = parts(phase, 2);
    let (s, c, e) = (pt(30., 100.), pt(150., 100.), pt(150., 24.));
    dashed(f, s, c, EDGE, 1.2);
    dashed(f, c, e, EDGE, 1.2);
    let (p0, p2) = (pt(84., 100.), pt(150., 34.));
    let mid = pt(
        0.25 * p0.x + 0.5 * c.x + 0.25 * p2.x,
        0.25 * p0.y + 0.5 * c.y + 0.25 * p2.y,
    );
    let (pts, u) = if half == 0 {
        let mut v = vec![s, p0];
        for i in 1..=10 {
            let t = i as f32 / 10.;
            let (a, b, d) = ((1. - t) * (1. - t), 2. * (1. - t) * t, t * t);
            v.push(pt(
                a * p0.x + b * c.x + d * p2.x,
                a * p0.y + b * c.y + d * p2.y,
            ));
        }
        v.push(e);
        (v, win(q, 0.05, 0.9))
    } else {
        let k = 120. / 196.;
        let u = if q < 0.35 {
            k * win(q, 0.05, 0.35)
        } else if q < 0.6 {
            k
        } else {
            k + (1. - k) * win(q, 0.6, 0.9)
        };
        (vec![s, c, e], u)
    };
    let head = trace(f, &pts, u, CYAN, 2.);
    top_cutter(f, head, 9., time);
    if half == 0 {
        if u > 0.55 {
            dot(f, c, 2.5, TEXT);
            dimension(f, c, mid, GOLD);
            text(f, "P", 156., 104., GOLD, 11.);
        }
        text(f, "G64 P", 8., 15., CYAN, 11.);
        label(f, "ROUNDS BY UP TO P", 8., 29., TEXT);
    } else {
        if q > 0.35 && q < 0.62 {
            let t = win(q, 0.35, 0.62);
            ring(f, c, fl(10., 18., t), fade(GOLD, 1. - t), 1.5);
            label(f, "STOPS AT CORNER", 176., 100., GOLD);
        }
        text(f, "G61 EXACT", 8., 15., GOLD, 11.);
        label(f, "STOPS AT EVERY CORNER", 8., 29., TEXT);
    }
    heading(f, "BLEND TOLERANCE · G64 P OR G61 EXACT");
    caption(f, "DASHED: PROGRAMMED PATH · CYAN: CUTTER PATH");
}

/// Dwell: the spindle spins up, a timer fills, then the cutter moves.
fn post_dwell(f: &mut Frame, phase: f32) {
    let top = 110.;
    stock(f, 24., top, 254., 134.);
    let spin = smooth(win(phase, 0., 0.45));
    let moved = smooth(win(phase, 0.62, 0.95));
    let x = fl(56., 196., moved);
    let tip = 100.;
    // How far the flutes have turned: faster as the spindle comes up.
    let turned = 20.
        * if phase < 0.45 {
            0.5 * phase * phase / 0.45
        } else {
            0.225 + (phase - 0.45)
        };
    side_cutter(f, x, tip, turned, 16., false);
    for side in [-1., 1.] {
        for i in 0..3 {
            let y = 66. + 11. * i as f32;
            line(
                f,
                pt(x + side * 13., y),
                pt(x + side * (13. + 7. * spin), y),
                fade(CYAN, spin),
                1.5,
            );
        }
    }
    // The timer.
    let fill = win(phase, 0.03, 0.5);
    label(f, "DWELL 2 S", 120., 21., GOLD);
    rect(f, 120., 34., 250., 44., HOLE);
    rect(f, 120., 34., 120. + 130. * fill, 44., CYAN);
    for (a, b) in [
        (pt(120., 34.), pt(250., 34.)),
        (pt(250., 34.), pt(250., 44.)),
        (pt(250., 44.), pt(120., 44.)),
        (pt(120., 44.), pt(120., 34.)),
    ] {
        line(f, a, b, EDGE, 1.);
    }
    if phase < 0.52 {
        label(f, "SPINNING UP · WAITING", 120., 50., TEXT);
    } else {
        label(f, "FIRST MOVE", 120., 50., CYAN);
    }
    if moved > 0. && moved < 1. {
        arrow(f, pt(x + 16., 90.), pt(x + 38., 90.), CYAN);
    }
    heading(f, "SPINDLE DWELL · WAIT BEFORE THE FIRST MOVE");
    caption(f, "NO AT-SPEED FEEDBACK: WAIT FOR THE SPINDLE");
}

/// Air output: the nozzle blows air at the cutter from M7 or from M8.
fn post_air(f: &mut Frame, time: f32, phase: f32) {
    let (half, _) = parts(phase, 2);
    let top = 104.;
    stock(f, 24., top, 254., 134.);
    rect(f, 196., top, 216., 112., BACKGROUND);
    side_cutter(f, 206., 112., time, 16., false);
    let (s, color) = if half == 0 {
        ("M7 (MIST OUTPUT)", CYAN)
    } else {
        ("M8 (FLOOD OUTPUT)", GOLD)
    };
    // The outputs and the wire to the nozzle.
    rect(f, 10., 44., 58., 100., HOLE);
    for (a, b) in [
        (pt(10., 44.), pt(58., 44.)),
        (pt(58., 44.), pt(58., 100.)),
        (pt(58., 100.), pt(10., 100.)),
        (pt(10., 100.), pt(10., 44.)),
    ] {
        line(f, a, b, EDGE, 1.);
    }
    let ts = [pt(58., 68.), pt(58., 86.)];
    for i in [1 - half, half] {
        let t = ts[i];
        let (c, w) = if i == half { (color, 2.2) } else { (DIM, 1.2) };
        let wire = [t, pt(80., t.y), pt(80., 90.), pt(100., 90.)];
        for s in wire.windows(2) {
            line(f, s[0], s[1], c, w);
        }
        dot(f, t, 3., c);
        label(
            f,
            if i == 0 { "M7" } else { "M8" },
            18.,
            t.y - 4.,
            if i == half { color } else { TEXT },
        );
    }
    label(f, "OUTPUTS", 12., 47., TEXT);
    // The nozzle at the end of the lit wire, and its air.
    rect(f, 100., 86., 124., 94., METAL.top);
    polygon(
        f,
        &[pt(124., 85.), pt(136., 88.), pt(136., 92.), pt(124., 95.)],
        METAL.top,
    );
    line(f, pt(100., 86.), pt(124., 86.), color, 1.4);
    line(f, pt(100., 94.), pt(124., 94.), color, 1.4);
    label(f, "AIR BLAST", 100., 72., color);
    line(f, pt(116., 83.), pt(116., 85.), color, 1.2);
    for i in 0..10 {
        let t = (i as f32 / 10. + phase * 4.).fract();
        let x = 140. + t * 50.;
        let y = 90. + ((i * 7) % 5) as f32 * 4. * t - 8. * t;
        line(f, pt(x, y), pt(x + 7., y), fade(TEXT, 0.9 - 0.4 * t), 1.6);
    }
    mode(f, s, 100., 28., color);
    heading(f, "AIR OUTPUT · WHICH OUTPUT BLOWS THE AIR");
    caption(f, "AIR USES WHICHEVER OUTPUT IS WIRED TO THE BLAST");
}

pub fn linking(f: &mut Frame, time: f32, phase: f32, n: u8) {
    match n {
        0 => coolant(f, time, phase),
        1 => collision(f, time, phase),
        2 => hole_order(f, phase),
        3 => keep_down(f, time, phase),
        4 => facing_depth_order(f, phase),
        5 => facing_link(f, time, phase),
        6 => compensation(f, time, phase),
        7 => tolerance(f, phase),
        _ => heading(f, "?"),
    }
}

/// Coolant: flood, mist, air and off, each with its nozzle effect.
fn coolant(f: &mut Frame, time: f32, phase: f32) {
    let (which, q) = parts(phase, 4);
    stock(f, 24., 88., 254., 116.);
    rect(f, 160., 88., 180., 100., BACKGROUND);
    side_cutter(f, 170., 100., time, 16., false);
    let (n, t) = (pt(90., 58.), pt(162., 96.));
    let dir = ((t.x - n.x) / 81.4, (t.y - n.y) / 81.4);
    let perp = (-dir.1, dir.0);
    let at = |u: f32, off: f32, arc: f32| {
        pt(
            fl(n.x, t.x, u) + perp.0 * off,
            fl(n.y, t.y, u) + perp.1 * off - arc * (u * PI).sin(),
        )
    };
    let water = Color::from_rgba(0.40, 0.70, 1., 0.8);
    match which {
        0 => {
            let pts: Vec<Point> = (0..=16).map(|i| at(i as f32 / 16., 0., 10.)).collect();
            for s in pts.windows(2) {
                line(f, s[0], s[1], fade(water, 0.55), 4.);
            }
            for i in 0..6 {
                let u = (i as f32 / 6. + phase * 12.).fract();
                dot(f, at(u, 0., 10.), 2.2, water);
            }
            for k in 0..3 {
                let a = k as f32 * 1.1;
                line(
                    f,
                    t,
                    pt(t.x - 5. * a.cos() - 2., t.y - 5. * a.sin() - 2.),
                    water,
                    1.2,
                );
            }
        }
        1 => {
            // A wide cone of fine droplets.
            polygon(
                f,
                &[n, at(1., -24., 0.), at(1., 24., 0.)],
                Color::from_rgba(0.40, 0.70, 1., 0.12),
            );
            for i in 0..40 {
                let u = (i as f32 / 40. + phase * 10.).fract();
                let off = ((i * 37) % 23) as f32 / 11. - 1.;
                dot(f, at(u, off * u * 24., 0.), 1.5, fade(water, 0.9 - 0.4 * u));
            }
        }
        2 => {
            // Straight, fast white streaks, no liquid.
            for lane in [-5., 0., 5.] {
                for k in 0..3 {
                    let u0 = ((k as f32 / 3. + phase * 6. + lane * 0.07).fract()) * 0.85;
                    line(
                        f,
                        at(u0, lane, 0.),
                        at(u0 + 0.14, lane, 0.),
                        Color::from_rgba(1., 1., 1., 0.9),
                        2.,
                    );
                }
            }
        }
        _ => {}
    }
    let tone = if which == 3 {
        fade(METAL.top, 0.45)
    } else {
        METAL.top
    };
    line(f, pt(56., 38.), n, tone, 7.);
    dot(f, n, 3.5, tone);
    let name = [
        "FLOOD · M8",
        "MIST · M7",
        "AIR · THE AIR OUTPUT",
        "OFF · NOTHING WRITTEN",
    ];
    mode(f, name[which], 100., 22., GOLD);
    if which == 3 && q < 0.6 {
        mode(f, "M9", 112., 66., GOLD);
    }
    for (i, s) in ["FLOOD M8", "MIST M7", "AIR", "OFF"]
        .into_iter()
        .enumerate()
    {
        let x = 8. + i as f32 * 67.;
        let on = i == which;
        let c = if on { GOLD } else { fade(TEXT, 0.5) };
        rect(f, x, 122., x + 62., 135., HOLE);
        for (a, b) in [
            (pt(x, 122.), pt(x + 62., 122.)),
            (pt(x + 62., 122.), pt(x + 62., 135.)),
            (pt(x + 62., 135.), pt(x, 135.)),
            (pt(x, 135.), pt(x, 122.)),
        ] {
            line(f, a, b, c, if on { 1.6 } else { 1. });
        }
        label(f, s, x + 31. - s.chars().count() as f32 * 2.7, 125., c);
    }
    heading(f, "COOLANT · M8 FLOOD · M7 MIST · AIR · OFF");
    caption(f, "M9 STOPS THE COOLANT WHEN IT CHANGES");
}

/// Collision: retract and relink over an obstacle, or tilt around it.
fn collision(f: &mut Frame, time: f32, phase: f32) {
    let (half, q) = parts(phase, 2);
    stock(f, 24., 108., 254., 134.);
    rect(f, 40., 108., 100., 116., BACKGROUND);
    rect(f, 180., 108., 236., 116., BACKGROUND);
    rect(f, 124., 80., 156., 108., METAL.top);
    text(f, "CLAMP", 128., 91., BACKGROUND, 8.);
    let path: Vec<V> = if half == 0 {
        vec![
            [100., 116., 0.],
            [100., 62., 0.],
            [180., 62., 0.],
            [180., 116., 0.],
        ]
    } else {
        vec![
            [100., 116., 0.],
            [104., 96., 0.],
            [106., 76., 0.],
            [118., 62., 0.],
            [140., 58., 0.],
            [162., 62., 0.],
            [174., 76., 0.],
            [176., 96., 0.],
            [180., 116., 0.],
        ]
    };
    let pts: Vec<Point> = path.iter().map(|v| pt(v[0], v[1])).collect();
    for s in pts.windows(2) {
        dashed(f, s[0], s[1], DIM, 1.);
    }
    let u = smooth(win(q, 0.08, 0.86));
    let (pos, _) = along(&path, u);
    trace(f, &pts, u, if half == 0 { GOLD } else { CYAN }, 1.8);
    let tilt = if half == 0 {
        0.
    } else {
        -30f32.to_radians() * (PI * u).cos() * (PI * u).sin().max(0.).sqrt()
    };
    cutter_at(
        f,
        pt(pos[0], pos[1]),
        (tilt.sin(), -tilt.cos()),
        time,
        12.,
        false,
        0.,
    );
    if half == 0 {
        mode(f, "RETRACT ·", 208., 30., GOLD);
        mode(f, "RELINK", 208., 42., GOLD);
    } else {
        mode(f, "TILT", 208., 30., CYAN);
        label(f, "WITHIN LIMITS", 208., 43., CYAN);
    }
    heading(f, "COLLISION · HOW TO GET PAST AN OBSTRUCTION");
    caption(f, "PREFERENCE ONLY · NO COLLISION SOLVER YET");
}

const HOLES: [(f32, f32); 8] = [
    (30., 40.),
    (48., 118.),
    (86., 34.),
    (108., 120.),
    (150., 46.),
    (168., 124.),
    (204., 40.),
    (228., 112.),
];

/// Hole order: nearest hole next, or sorted by X then Y.
fn hole_order(f: &mut Frame, phase: f32) {
    let (half, q) = parts(phase, 2);
    let holes: Vec<(f32, f32)> = HOLES.iter().map(|&(x, y)| (x, y + 9.)).collect();
    let order: Vec<usize> = if half == 0 {
        let mut left: Vec<usize> = (1..holes.len()).collect();
        let mut order = vec![0];
        while !left.is_empty() {
            let (px, py) = holes[*order.last().unwrap()];
            let d2 = |i: usize| (holes[i].0 - px).powi(2) + (holes[i].1 - py).powi(2);
            let k = (0..left.len())
                .min_by(|&a, &b| d2(left[a]).total_cmp(&d2(left[b])))
                .unwrap();
            order.push(left.remove(k));
        }
        order
    } else {
        let mut o: Vec<usize> = (0..holes.len()).collect();
        o.sort_by(|&a, &b| {
            holes[a]
                .0
                .total_cmp(&holes[b].0)
                .then(holes[a].1.total_cmp(&holes[b].1))
        });
        o
    };
    rect(f, 18., 38., 260., 141., fade(METAL.front, 0.3));
    let pts: Vec<Point> = order.iter().map(|&i| pt(holes[i].0, holes[i].1)).collect();
    let mut cum = vec![0.];
    for s in pts.windows(2) {
        let l = (s[1].x - s[0].x).hypot(s[1].y - s[0].y);
        cum.push(cum.last().unwrap() + l);
    }
    let total = *cum.last().unwrap();
    let u = win(q, 0.04, 0.8);
    let head = trace(f, &pts, u, CYAN, 2.);
    for (k, c) in pts.iter().enumerate() {
        let lit = u * total >= cum[k] - 0.01;
        dot(f, *c, 6.5, if lit { GOLD } else { HOLE });
        ring(f, *c, 6.5, if lit { GOLD } else { fade(TEXT, 0.6) }, 1.);
        if lit {
            text(
                f,
                &format!("{}", k + 1),
                c.x - 2.7,
                c.y - 4.5,
                BACKGROUND,
                9.,
            );
        }
    }
    ring(f, head, 10., CYAN, 1.2);
    if half == 0 {
        mode(f, "OPTIMIZE TRAVEL", 8., 14., GOLD);
        label(f, "NEAREST HOLE NEXT", 8., 26., TEXT);
    } else {
        mode(f, "BY X THEN Y", 8., 14., GOLD);
        label(f, "SORTED: X, THEN Y", 8., 26., TEXT);
    }
    heading(f, "HOLE ORDER · THE ROUTE BETWEEN HOLES");
    caption(f, "CYAN: RAPID ROUTE · HOLES LIGHT UP IN ORDER");
}

/// Keep down: the link stays near cutting depth, or retracts to clearance.
fn keep_down(f: &mut Frame, time: f32, phase: f32) {
    let (half, q) = parts(phase, 2);
    let top = 108.;
    let cut = 118.;
    let ylink = if half == 0 { 102. } else { 62. };
    stock(f, 24., top, 254., 134.);
    let path: Vec<V> = [
        (50., cut),
        (110., cut),
        (110., ylink),
        (160., ylink),
        (160., cut),
        (220., cut),
    ]
    .iter()
    .map(|&(x, y)| [x, y, 0.])
    .collect();
    let pts: Vec<Point> = path.iter().map(|v| pt(v[0], v[1])).collect();
    let u = win(q, 0.06, 0.94);
    let (pos, _) = along(&path, u);
    rect(f, 44., top, pos[0].min(110.) + 6., cut, BACKGROUND);
    if pos[0] >= 160. {
        let depth = (pos[1] - top).clamp(0., cut - top);
        rect(f, 154., top, pos[0] + 6., top + depth, BACKGROUND);
    }
    dashed(f, pt(24., 62.), pt(254., 62.), fade(GOLD, 0.35), 1.);
    label(f, "CLEARANCE", 24., 50., fade(GOLD, 0.7));
    let lens = [60., cut - ylink, 50., cut - ylink, 60.];
    let total: f32 = lens.iter().sum();
    let mut start = 0.;
    for (i, l) in lens.iter().enumerate() {
        let k = ((u * total - start) / l).clamp(0., 1.);
        let link = (1..4).contains(&i);
        trace(
            f,
            &pts[i..=i + 1],
            k,
            if link { GOLD } else { CYAN },
            if link { 2.8 } else { 2. },
        );
        start += l;
    }
    side_cutter(f, pos[0], pos[1], time, 12., false);
    if half == 0 {
        mode(f, "KEEP DOWN: ON", 176., 17., CYAN);
        label(f, "LINK STAYS LOW", 176., 30., TEXT);
    } else {
        mode(f, "KEEP DOWN: OFF", 176., 17., GOLD);
        label(f, "UP TO CLEARANCE", 176., 30., TEXT);
    }
    heading(f, "KEEP DOWN · LINKS STAY NEAR CUTTING DEPTH");
    caption(f, "CYAN: CUTS · GOLD: THE LINK BETWEEN THEM");
}

/// A facing's rows as a serpentine through the stock.
fn serpentine(forward: bool, dy: f32, dx: f32) -> Vec<Point> {
    let (xl, xr) = (96. + dx, 236. - dx);
    let mut v = vec![];
    for (i, y) in [46., 66., 86., 106.].into_iter().enumerate() {
        let (a, b) = if i % 2 == 1 { (xr, xl) } else { (xl, xr) };
        v.push(pt(a, y + dy));
        v.push(pt(b, y + dy));
    }
    if !forward {
        v.reverse();
    }
    v
}
/// Direction chevrons on a route's rows, up to how far it has gone.
fn chevrons(f: &mut Frame, pts: &[Point], u: f32, color: Color) {
    let total: f32 = pts
        .windows(2)
        .map(|s| (s[1].x - s[0].x).hypot(s[1].y - s[0].y))
        .sum();
    let mut cum = 0.;
    for s in pts.windows(2) {
        let l = (s[1].x - s[0].x).hypot(s[1].y - s[0].y);
        if (s[1].y - s[0].y).abs() < 0.1 {
            for k in [0.35, 0.7] {
                if u * total >= cum + k * l {
                    chev(
                        f,
                        pt(fl(s[0].x, s[1].x, k), s[0].y),
                        (s[1].x - s[0].x).signum(),
                        color,
                    );
                }
            }
        }
        cum += l;
    }
}

/// Facing depth order: the next depth continues from where the last ended
/// with its rows reversed, or returns to the first start.
fn facing_depth_order(f: &mut Frame, phase: f32) {
    let (half, q) = parts(phase, 2);
    rect(f, 84., 34., 248., 118., fade(STOCK.top, 0.5));
    let d1 = serpentine(true, -4., 0.);
    let d2 = serpentine(half == 1, 4., 4.);
    let u1 = win(q, 0.02, 0.42);
    let u2 = win(q, 0.58, 0.97);
    trace(f, &d1, u1, GOLD, 1.8);
    chevrons(f, &d1, u1, GOLD);
    let (s1, e1) = (d1[0], *d1.last().unwrap());
    dot(f, s1, 3.5, GOLD);
    ring(f, e1, 4., GOLD, 1.4);
    if half == 0 {
        // Continues: feeds down at the last X Y.
        if q > 0.42 && q < 0.58 {
            let t = win(q, 0.42, 0.58);
            ring(f, e1, fl(4., 12., t), fade(CYAN, 1. - t), 1.5);
            label(f, "FEEDS DOWN HERE", 8., 118., CYAN);
        }
        label(f, "START 1", 8., 40., GOLD);
        label(f, "END 1", 8., 98., GOLD);
        label(f, "START 2", 8., 108., CYAN);
    } else {
        // Returns: a rapid back to the original start.
        if q > 0.42 {
            let a = smooth(win(q, 0.42, 0.52));
            let (x, rapid) = (90., fade(GOLD, a));
            dashed(f, e1, pt(x, e1.y), rapid, 1.4);
            dashed(f, pt(x, e1.y), pt(x, s1.y), rapid, 1.4);
            dashed(f, pt(x, s1.y), s1, rapid, 1.4);
            if q < 0.58 {
                label(f, "RETURNS TO START", 8., 118., GOLD);
            }
        }
        label(f, "START 1", 8., 36., GOLD);
        label(f, "START 2", 8., 47., CYAN);
        label(f, "END 1", 8., 100., GOLD);
    }
    if q >= 0.58 {
        let s2 = d2[0];
        ring(f, pt(s2.x, s2.y), 6., CYAN, 1.4);
    }
    trace(f, &d2, u2, CYAN, 2.4);
    chevrons(f, &d2, u2, CYAN);
    if half == 0 {
        mode(f, "CONTINUE FROM PREVIOUS END", 8., 15., GOLD);
    } else {
        mode(f, "RETURN TO FIRST START", 8., 15., GOLD);
    }
    heading(f, "FACING DEPTHS · WHERE THE NEXT STARTS");
    caption(f, "GOLD: FIRST DEPTH · CYAN: THE NEXT DEPTH");
}

/// A route over the part whose cut passes are cyan and links gold.
fn route3(f: &mut Frame, nodes: &[V], links: &[bool], u: f32) -> V {
    let len =
        |a: V, b: V| ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
    let total: f32 = nodes.windows(2).map(|w| len(w[0], w[1])).sum();
    let mut left = u.clamp(0., 1.) * total;
    let mut head = nodes[0];
    for (i, w) in nodes.windows(2).enumerate() {
        let l = len(w[0], w[1]);
        let k = (left / l).clamp(0., 1.);
        head = lerp(w[0], w[1], k);
        if k > 0. {
            line3(
                f,
                w[0],
                head,
                if links[i] { GOLD } else { CYAN },
                if links[i] { 2.8 } else { 2. },
            );
        }
        left -= l;
        if left <= 0. {
            break;
        }
    }
    head
}

/// Facing link: feed sideways between rows, or lift to clearance.
fn facing_link(f: &mut Frame, time: f32, phase: f32) {
    let (half, q) = parts(phase, 2);
    cuboid(f, [-34., -18., 0.], [34., 18., 10.], STOCK);
    let (z, zc) = (10., 24.);
    let (nodes, links): (Vec<V>, Vec<bool>) = if half == 0 {
        (
            vec![
                [-24., -9., z],
                [24., -9., z],
                [24., 0., z],
                [-24., 0., z],
                [-24., 9., z],
                [24., 9., z],
            ],
            vec![false, true, false, true, false],
        )
    } else {
        (
            vec![
                [-24., -9., z],
                [24., -9., z],
                [24., -9., zc],
                [24., 0., zc],
                [24., 0., z],
                [-24., 0., z],
                [-24., 0., zc],
                [-24., 9., zc],
                [-24., 9., z],
                [24., 9., z],
            ],
            vec![false, true, true, true, false, true, true, true, false],
        )
    };
    polyline3(f, &nodes, DIM, 1.);
    let head = route3(f, &nodes, &links, win(q, 0.05, 0.93));
    cutter(f, head, time, 9., false, 0.);
    if half == 0 {
        mode(f, "OPTION 1", 176., 16., CYAN);
        mode(f, "KEEP TOOL DOWN", 176., 27., CYAN);
        caption(f, "FEEDS SIDEWAYS TO THE NEXT ROW AT CUTTING DEPTH");
    } else {
        mode(f, "OPTION 2", 176., 16., GOLD);
        mode(f, "RETRACT EACH PASS", 176., 27., GOLD);
        caption(f, "LIFTS TO CLEARANCE AND DROPS ONTO THE NEXT ROW");
    }
    heading(f, "FACING LINK · BETWEEN ADJACENT ROWS");
}

/// Compensation: the path is offset in CAM, or the control offsets it.
fn compensation(f: &mut Frame, time: f32, phase: f32) {
    let (half, q) = parts(phase, 2);
    let part = [
        pt(60., 56.),
        pt(140., 56.),
        pt(140., 86.),
        pt(106., 86.),
        pt(106., 112.),
        pt(60., 112.),
    ];
    let offset = [
        pt(48., 44.),
        pt(152., 44.),
        pt(152., 98.),
        pt(118., 98.),
        pt(118., 124.),
        pt(48., 124.),
    ];
    let closed = |v: &[Point]| {
        let mut c = v.to_vec();
        c.push(v[0]);
        c
    };
    rect(f, 34., 32., 164., 134., fade(STOCK.top, 0.55));
    polygon(f, &part, METAL.top);
    let (part, offset) = (closed(&part), closed(&offset));
    if half == 0 {
        for s in part.windows(2) {
            line(f, s[0], s[1], GOLD, 1.8);
        }
        for s in offset.windows(2) {
            line(f, s[0], s[1], CYAN, 1.8);
        }
    } else {
        for s in part.windows(2) {
            line(f, s[0], s[1], GOLD, 3.);
            line(f, s[0], s[1], CYAN, 1.2);
        }
        for s in offset.windows(2) {
            dashed(f, s[0], s[1], TEXT, 1.2);
        }
    }
    let head = trace(f, &offset, win(q, 0.04, 0.94), fade(GOLD, 0.0), 1.);
    top_cutter(f, head, 12., time);
    dot(f, head, 2., Color::WHITE);
    // The tool radius: the gap between the outline and the cutter centre.
    dimension(f, pt(140., 70.), pt(152., 70.), Color::WHITE);
    text(f, "TOOL RADIUS", 176., 64., Color::WHITE, 9.);
    line(f, pt(176., 70.), pt(153., 70.), Color::WHITE, 1.);
    if half == 0 {
        mode(f, "IN CAM", 8., 15., GOLD);
        label(f, "PROGRAM HAS", 176., 100., TEXT);
        label(f, "THE OFFSET PATH", 176., 111., TEXT);
    } else {
        mode(f, "AT THE CONTROL", 8., 15., GOLD);
        label(f, "PROGRAM HAS", 176., 100., TEXT);
        label(f, "THE OUTLINE · G41", 176., 111., TEXT);
    }
    heading(f, "TOOL RADIUS OFFSET · IN CAM OR AT CONTROL");
    caption(f, "GOLD: OUTLINE · CYAN: PROGRAM PATH");
}

/// Tolerance: segments stay within a band around the computed curve; a
/// smaller band needs more of them.
fn tolerance(f: &mut Frame, phase: f32) {
    let (half, q) = parts(phase, 2);
    // Same scale in both: 10 px to the mm.
    let (tol, segs, mm) = if half == 0 {
        (4., 4, "0.4")
    } else {
        (1., 8, "0.1")
    };
    let (r, cy) = (110., 180.);
    let at = |deg: f32, rr: f32| {
        let a = deg.to_radians();
        pt(139. + rr * a.cos(), cy - rr * a.sin())
    };
    let samples: Vec<f32> = (0..=40).map(|i| 30. + 3. * i as f32).collect();
    let outer: Vec<Point> = samples.iter().map(|&d| at(d, r + tol)).collect();
    let inner: Vec<Point> = samples.iter().map(|&d| at(d, r - tol)).collect();
    let band = Path::new(|b| {
        b.move_to(outer[0]);
        for p in &outer[1..] {
            b.line_to(*p);
        }
        for p in inner.iter().rev() {
            b.line_to(*p);
        }
        b.close();
    });
    f.fill(&band, fade(CYAN, 0.18));
    for edge in [&outer, &inner] {
        for s in edge.windows(2) {
            line(f, s[0], s[1], fade(CYAN, 0.5), 1.);
        }
    }
    let curve: Vec<Point> = samples.iter().map(|&d| at(d, r)).collect();
    for s in curve.windows(2) {
        line(f, s[0], s[1], TEXT, 1.4);
    }
    // The segments, with a vertex on the curve between each.
    let verts: Vec<Point> = (0..=segs)
        .map(|k| at(30. + 120. * k as f32 / segs as f32, r))
        .collect();
    let u = win(q, 0.1, 0.8);
    trace(f, &verts, u, CYAN, 2.2);
    for (k, v) in verts.iter().enumerate() {
        if u * segs as f32 >= k as f32 - 0.01 {
            dot(f, *v, 2.8, CYAN);
        }
    }
    // The tolerance across the band, away from the peak.
    arrow(f, at(55., r + tol + 12.), at(55., r + tol), GOLD);
    arrow(f, at(55., r - tol - 12.), at(55., r - tol), GOLD);
    text(f, &format!("+/-{mm} MM"), 208., 60., GOLD, 9.);
    mode(f, &format!("{segs} SEGMENTS"), 100., 100., GOLD);
    label(f, "MAGNIFIED", 8., 30., fade(TEXT, 0.7));
    if half == 0 {
        mode(f, "LARGER TOLERANCE · FEWER SEGMENTS", 8., 15., GOLD);
    } else {
        mode(f, "SMALLER TOLERANCE · MORE SEGMENTS", 8., 15., GOLD);
    }
    heading(f, "TOLERANCE · HOW FAR SEGMENTS MAY STRAY");
    caption(f, "WHITE: CURVE · CYAN: SEGMENTS · BAND: +/- TOL");
}
