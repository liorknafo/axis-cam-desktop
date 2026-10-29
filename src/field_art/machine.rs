//! Field pictures: machine. The manual machines' settings, the machine
//! profile and the material's cutting data, each drawn as a small scene.
use crate::{choice_diagram::SPIN, illustration::*};
use iced::{
    Color, Point, Size,
    widget::canvas::{Frame, Path, Stroke},
};
use std::f32::consts::{PI, TAU};

const RED: Color = Color::from_rgb(0.95, 0.38, 0.36);
const OK: Color = Color::from_rgb(0.36, 0.80, 0.55);
const SKIN: Color = Color::from_rgb(0.90, 0.72, 0.60);
const IRON: Color = Color::from_rgb(0.24, 0.31, 0.38);
const IRON_LIGHT: Color = Color::from_rgb(0.38, 0.46, 0.53);
const STEEL: Tone = Tone {
    top: Color::from_rgb(0.62, 0.68, 0.73),
    front: Color::from_rgb(0.42, 0.47, 0.52),
    side: Color::from_rgb(0.32, 0.37, 0.42),
};
const NORMAL: Color = Color::from_rgba(0.26, 0.83, 0.92, 0.55);

pub fn manual(f: &mut Frame, time: f32, phase: f32, n: u8) {
    match n {
        0 => speeds(f, time, phase),
        1 => hand_feed(f, time, phase),
        2 => hand_rapid(f, time, phase),
        3 => turn_one(f, phase),
        4 => swing(f, time, phase),
        5 => centres(f, time, phase),
        6 => cross_reads(f, time, phase),
        7 => cross_turn(f, phase),
        8 => cross_div(f, phase),
        9 => feeds(f, phase),
        10 => pitches(f, phase),
        11 => dro_comp(f, time, phase),
        12 => edge_finder(f, time, phase),
        13 => pass_time(f, phase),
        14 => tool_time(f, phase),
        15 => move_time(f, phase),
        _ => heading(f, "MANUAL"),
    }
}

pub fn machine(f: &mut Frame, time: f32, phase: f32, n: u8) {
    match n {
        0 => rpm_limit(f, time, phase),
        1 => power(f, time, phase),
        2 => feed_limit(f, time, phase),
        3 => kind(f, phase),
        4..=6 => travel(f, time, phase, n - 4),
        7 => tool_change(f, phase),
        _ => heading(f, "MACHINE"),
    }
}

pub fn cutting(f: &mut Frame, time: f32, phase: f32, n: u8) {
    match n {
        0 => surface_speed(f, time, phase),
        1 => chip_load(f, time, phase),
        _ => heading(f, "CUTTING DATA"),
    }
}

// ---------------------------------------------------------------- helpers

fn at(x: f32, y: f32) -> Point {
    Point::new(x, y)
}
/// A point at angle `a` (clockwise from straight up) round `c`.
fn pt(c: Point, r: f32, a: f32) -> Point {
    at(c.x + r * a.sin(), c.y - r * a.cos())
}
fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}
fn say(f: &mut Frame, s: &str, x: f32, y: f32, color: Color, size: f32) {
    text(f, s, x, y, color, size);
}
fn say_mid(f: &mut Frame, s: &str, cx: f32, y: f32, color: Color, size: f32) {
    let w = s.chars().count() as f32 * size * 0.58;
    text(f, s, cx - w / 2., y, color, size);
}
fn disc_at(f: &mut Frame, c: Point, r: f32, color: Color) {
    f.fill(&Path::circle(c, r), color);
}
fn ring(f: &mut Frame, c: Point, r: f32, color: Color, w: f32) {
    f.stroke(
        &Path::circle(c, r),
        Stroke::default().with_color(color).with_width(w),
    );
}
fn arc(f: &mut Frame, c: Point, r: f32, a0: f32, a1: f32, color: Color, w: f32) {
    let n = (((a1 - a0).abs() * r / 3.).ceil() as usize).max(2);
    let path = Path::new(|b| {
        b.move_to(pt(c, r, a0));
        for i in 1..=n {
            b.line_to(pt(c, r, a0 + (a1 - a0) * i as f32 / n as f32));
        }
    });
    f.stroke(&path, Stroke::default().with_color(color).with_width(w));
}
fn dashed_circle(f: &mut Frame, c: Point, r: f32, color: Color) {
    for i in 0..24 {
        let a = i as f32 * TAU / 24.;
        arc(f, c, r, a, a + 0.14, color, 1.2);
    }
}
fn outline(f: &mut Frame, x0: f32, y0: f32, x1: f32, y1: f32, color: Color, w: f32) {
    f.stroke(
        &Path::rectangle(at(x0, y0), Size::new(x1 - x0, y1 - y0)),
        Stroke::default().with_color(color).with_width(w),
    );
}
fn dashed_rect(f: &mut Frame, x0: f32, y0: f32, x1: f32, y1: f32, color: Color) {
    dashed(f, at(x0, y0), at(x1, y0), color, 1.);
    dashed(f, at(x1, y0), at(x1, y1), color, 1.);
    dashed(f, at(x1, y1), at(x0, y1), color, 1.);
    dashed(f, at(x0, y1), at(x0, y0), color, 1.);
}
fn panel(f: &mut Frame, x0: f32, y0: f32, x1: f32, y1: f32) {
    rect(f, x0, y0, x1, y1, Color::from_rgb(0.15, 0.23, 0.30));
    outline(f, x0, y0, x1, y1, DIM, 1.);
}
fn poly_line(f: &mut Frame, pts: &[Point], color: Color, w: f32) {
    if pts.len() < 2 {
        return;
    }
    let path = Path::new(|b| {
        b.move_to(pts[0]);
        for p in &pts[1..] {
            b.line_to(*p);
        }
    });
    f.stroke(&path, Stroke::default().with_color(color).with_width(w));
}
/// A single arrow from `a` to `b`.
fn arrow(f: &mut Frame, a: Point, b: Point, color: Color) {
    line(f, a, b, color, 1.6);
    let len = (b.x - a.x).hypot(b.y - a.y).max(1e-3);
    let (ux, uy) = ((b.x - a.x) / len, (b.y - a.y) / len);
    for side in [-1., 1.] {
        line(
            f,
            b,
            at(
                b.x - 6. * ux + side * 3.5 * uy,
                b.y - 6. * uy - side * 3.5 * ux,
            ),
            color,
            1.6,
        );
    }
}
/// Piecewise eased keyframes over the phase.
fn track(phase: f32, keys: &[(f32, f32)]) -> f32 {
    for w in keys.windows(2) {
        if phase <= w[1].0 {
            let t = if w[1].0 > w[0].0 {
                (phase - w[0].0) / (w[1].0 - w[0].0)
            } else {
                1.
            };
            return w[0].1 + (w[1].1 - w[0].1) * smooth(t);
        }
    }
    keys.last().map(|k| k.1).unwrap_or(0.)
}
/// A handwheel seen face on, its handle at `angle` (clockwise from up) and
/// a fixed index mark on top.
fn handwheel(f: &mut Frame, c: Point, r: f32, angle: f32) {
    disc_at(f, c, r, Color::from_rgb(0.20, 0.28, 0.35));
    ring(f, c, r, TEXT, 1.6);
    for k in 0..3 {
        let a = angle + k as f32 * PI / 3.;
        line(f, pt(c, r, a), pt(c, r, a + PI), EDGE, 1.6);
    }
    disc_at(f, c, 3., TEXT);
    disc_at(f, pt(c, r, angle), r * 0.2, GOLD);
    polygon(
        f,
        &[
            at(c.x, c.y - r - 1.),
            at(c.x - 4., c.y - r - 8.),
            at(c.x + 4., c.y - r - 8.),
        ],
        CYAN,
    );
}
/// The gauge angle of a fraction of the scale: 270 degrees, gap at the bottom.
fn ang(frac: f32) -> f32 {
    (-135. + 270. * frac).to_radians()
}
fn gauge(f: &mut Frame, c: Point, r: f32, frac: f32, zones: &[(f32, f32, Color)], needle: Color) {
    disc_at(f, c, r + 7., Color::from_rgb(0.14, 0.21, 0.28));
    arc(f, c, r, ang(0.), ang(1.), DIM, 6.);
    for (a, b, col) in zones {
        arc(f, c, r, ang(*a), ang(*b), *col, 6.);
    }
    for i in 0..=10 {
        let fr = i as f32 / 10.;
        let long = if i % 5 == 0 { 11. } else { 8. };
        line(
            f,
            pt(c, r - 4., ang(fr)),
            pt(c, r - long, ang(fr)),
            TEXT,
            1.,
        );
    }
    line(f, c, pt(c, r - 8., ang(frac.clamp(0., 1.))), needle, 2.6);
    disc_at(f, c, 4.5, needle);
}
fn bar_gauge(f: &mut Frame, x0: f32, y0: f32, x1: f32, y1: f32, frac: f32, limit: f32) {
    rect(f, x0, y0, x1, y1, alpha(CYAN, 0.14));
    let xl = x0 + (x1 - x0) * limit;
    let xv = x0 + (x1 - x0) * frac.clamp(0., 1.);
    rect(f, x0, y0, xv.min(xl), y1, CYAN);
    if frac > limit {
        rect(f, xl, y0, xv, y1, RED);
    }
    outline(f, x0, y0, x1, y1, DIM, 1.);
    line(f, at(xl, y0 - 5.), at(xl, y1 + 5.), GOLD, 2.2);
}
/// A cutter seen from above: a disc, its flutes turning with `a`.
fn top_cutter(f: &mut Frame, c: Point, r: f32, a: f32, flutes: usize) {
    disc_at(f, c, r, Color::from_rgb(0.30, 0.38, 0.45));
    ring(f, c, r, GOLD, 1.8);
    for j in 0..flutes {
        let b = a + j as f32 * TAU / flutes as f32;
        line(f, pt(c, r * 0.22, b), pt(c, r - 1., b), GOLD, 2.4);
    }
    disc_at(f, c, r * 0.2, Color::from_rgb(0.20, 0.26, 0.32));
}
/// A tool standing in a holder: shank on top, coloured flutes below.
fn tool_glyph(f: &mut Frame, x: f32, top: f32, len: f32, w: f32, color: Color) {
    rect(f, x - w / 2., top, x + w / 2., top + len * 0.4, METAL.top);
    rect(
        f,
        x - w * 0.42,
        top + len * 0.4,
        x + w * 0.42,
        top + len,
        color,
    );
    for k in 0..2 {
        let y = top + len * (0.55 + 0.2 * k as f32);
        line(
            f,
            at(x - w * 0.42, y + 3.),
            at(x + w * 0.42, y),
            Color::from_rgba(0.3, 0.2, 0.05, 0.6),
            1.2,
        );
    }
}
/// A row of time blocks. `newest` is the block being added (gold, `grow`
/// wide); the ones before it are full, the ones after are empty slots.
#[allow(clippy::too_many_arguments)]
fn block_row(
    f: &mut Frame,
    x0: f32,
    y: f32,
    w: f32,
    h: f32,
    gap: f32,
    slots: usize,
    newest: usize,
    grow: f32,
    label: &str,
) {
    for k in 0..slots {
        let x = x0 + k as f32 * (w + gap);
        rect(f, x, y, x + w, y + h, alpha(CYAN, 0.08));
        if k < newest {
            rect(f, x, y, x + w, y + h, GOLD);
            say_mid(f, label, x + w / 2., y + h / 2. - 5., BACKGROUND, 10.);
        } else if k == newest {
            rect(f, x, y, x + w * grow, y + h, GOLD);
            if grow >= 0.7 {
                say_mid(f, label, x + w / 2., y + h / 2. - 5., BACKGROUND, 10.);
            }
        }
    }
}

// ---------------------------------------------------------------- icons

/// A CNC machine: an enclosure with a window on the spindle and a G-code
/// panel. `t` is the loop phase.
fn cnc_icon(f: &mut Frame, x: f32, y: f32, s: f32, t: f32) {
    let r = |f: &mut Frame, a: f32, b: f32, c: f32, d: f32, col: Color| {
        rect(f, x + a * s, y + b * s, x + c * s, y + d * s, col)
    };
    r(f, 2., 4., 76., 60., IRON);
    r(f, 8., 10., 52., 50., HOLE);
    r(f, 8., 16., 52., 19., IRON_LIGHT);
    r(f, 27., 19., 33., 28., STEEL.front);
    r(f, 29., 28., 31., 36., GOLD);
    r(f, 12., 44., 48., 48., METAL.top);
    r(f, 24., 38., 38., 44., STOCK.top);
    r(f, 56., 10., 72., 50., Color::from_rgb(0.14, 0.20, 0.26));
    for j in 0..5 {
        let yy = (j as f32 * 8. + t * 80.) % 40.;
        let w = 6. + ((j * 7) % 5) as f32 * 2.;
        r(f, 58., 11. + yy, 58. + w, 12.5 + yy, CYAN);
    }
}
/// A hand-operated mill: column, head, table, a handwheel and a DRO.
fn mill_icon(f: &mut Frame, x: f32, y: f32, s: f32, t: f32) {
    let r = |f: &mut Frame, a: f32, b: f32, c: f32, d: f32, col: Color| {
        rect(f, x + a * s, y + b * s, x + c * s, y + d * s, col)
    };
    r(f, 4., 56., 74., 62., IRON);
    r(f, 54., 6., 68., 56., IRON_LIGHT);
    r(f, 24., 6., 68., 20., IRON_LIGHT);
    r(f, 34., 20., 44., 25., STEEL.front);
    r(f, 37., 25., 41., 34., GOLD);
    r(f, 12., 44., 56., 50., METAL.top);
    r(f, 26., 38., 46., 44., STOCK.top);
    r(f, 57., 24., 67., 36., HOLE);
    r(f, 58.5, 27., 65.5, 28.5, CYAN);
    r(f, 58.5, 31., 63., 32.5, CYAN);
    let c = at(x + 6. * s, y + 47. * s);
    ring(f, c, 6.5 * s, TEXT, 1.4);
    let a = t * TAU * 2.;
    line(f, pt(c, 6.5 * s, a), pt(c, 6.5 * s, a + PI), EDGE, 1.2);
    disc_at(f, pt(c, 6.5 * s, a), 2. * s, GOLD);
}
/// A hand-operated lathe: headstock, chuck, bar, carriage, tailstock, and a
/// graduated dial.
fn lathe_icon(f: &mut Frame, x: f32, y: f32, s: f32, t: f32) {
    let r = |f: &mut Frame, a: f32, b: f32, c: f32, d: f32, col: Color| {
        rect(f, x + a * s, y + b * s, x + c * s, y + d * s, col)
    };
    r(f, 2., 54., 76., 60., IRON);
    r(f, 2., 22., 24., 54., IRON_LIGHT);
    r(f, 24., 26., 30., 50., STEEL.front);
    r(f, 30., 35., 56., 41., METAL.top);
    r(f, 58., 28., 74., 54., IRON_LIGHT);
    polygon(
        f,
        &[
            at(x + 58. * s, y + 35. * s),
            at(x + 58. * s, y + 41. * s),
            at(x + 55. * s, y + 38. * s),
        ],
        STEEL.top,
    );
    r(f, 34., 44., 52., 50., IRON_LIGHT);
    r(f, 40., 41., 48., 44., IRON);
    polygon(
        f,
        &[
            at(x + 42. * s, y + 44. * s),
            at(x + 46. * s, y + 44. * s),
            at(x + 44. * s, y + 41. * s),
        ],
        GOLD,
    );
    let c = at(x + 36. * s, y + 12. * s);
    disc_at(f, c, 9. * s, Color::from_rgb(0.14, 0.21, 0.28));
    ring(f, c, 9. * s, TEXT, 1.4);
    for i in 0..8 {
        let a = t * TAU / 8. + i as f32 * TAU / 8.;
        line(f, pt(c, 6. * s, a), pt(c, 9. * s, a), TEXT, 1.);
    }
    line(f, at(c.x, c.y - 9. * s), at(c.x, c.y - 4. * s), GOLD, 1.8);
}

// ---------------------------------------------------------------- manual

/// Speeds: a gearbox with a few fixed speeds, then a variable drive.
fn speeds(f: &mut Frame, time: f32, phase: f32) {
    let list = [90, 180, 350, 700, 1400];
    let stepped = phase < 0.75;
    let (rpm, k, streaks, sel) = if stepped {
        let i = ((phase / 0.15) as usize).min(4);
        (list[i] as f32, [1., 2., 3., 5., 8.][i], i + 1, Some(i))
    } else {
        let t = (phase - 0.75) / 0.25;
        let s = smooth(1. - (2. * t - 1.).abs());
        let rpm = 100. + 1900. * s;
        (
            rpm,
            (rpm / 250.).round().max(1.),
            1 + (s * 4.) as usize,
            None,
        )
    };
    if let Some(sel) = sel {
        for (i, v) in list.iter().enumerate() {
            let x = 12. + i as f32 * 52.;
            let on = i == sel;
            let fill = if on {
                alpha(GOLD, 0.30)
            } else {
                alpha(CYAN, 0.08)
            };
            rect(f, x, 20., x + 46., 38., fill);
            outline(
                f,
                x,
                20.,
                x + 46.,
                38.,
                if on { GOLD } else { DIM },
                if on { 1.6 } else { 1. },
            );
            say_mid(
                f,
                &v.to_string(),
                x + 23.,
                25.,
                if on { GOLD } else { TEXT },
                11.,
            );
        }
        let px = 12. + sel as f32 * 52. + 23.;
        polygon(f, &[at(px, 40.), at(px - 5., 47.), at(px + 5., 47.)], GOLD);
    } else {
        let x = 24. + 230. * ((rpm - 100.) / 1900.);
        rect(f, 24., 26., 254., 32., alpha(CYAN, 0.18));
        rect(f, 24., 26., x, 32., CYAN);
        disc_at(f, at(x, 29.), 7., GOLD);
        say(f, "100", 18., 38., TEXT, 9.);
        say(f, "2000", 236., 38., TEXT, 9.);
        say_mid(f, "VARIABLE DRIVE", 139., 40., CYAN, 9.);
    }
    // The spindle, turning faster for each higher speed.
    let c = at(66., 96.);
    disc_at(f, c, 24., Color::from_rgb(0.22, 0.29, 0.36));
    ring(f, c, 24., TEXT, 1.6);
    let a = time * SPIN * k;
    for j in 0..4 {
        let a2 = a + j as f32 * PI / 2.;
        let col = if j == 0 { GOLD } else { EDGE };
        line(f, pt(c, 6., a2), pt(c, 22., a2), col, 2.);
    }
    for j in 0..streaks {
        arc(
            f,
            c,
            28. + j as f32 * 3.2,
            a - 0.9,
            a - 0.1,
            alpha(CYAN, 0.9 - 0.1 * j as f32),
            2.,
        );
    }
    say(
        f,
        &format!("{} RPM", rpm.round() as i32),
        120.,
        66.,
        GOLD,
        22.,
    );
    say(
        f,
        if stepped {
            "SELECTED GEARBOX SPEED"
        } else {
            "ANY SPEED IN THE RANGE"
        },
        122.,
        94.,
        TEXT,
        9.,
    );
    heading(f, "SPINDLE SPEEDS THE MACHINE HAS (RPM)");
    caption(
        f,
        if stepped {
            "A MANUAL OPERATION'S RPM MUST BE ONE OF THESE"
        } else {
            "A RANGE · 100-2000 · MEANS A VARIABLE DRIVE"
        },
    );
}

/// A mill table under a cutter, a handwheel on its leadscrew.
fn hand_mill(f: &mut Frame, time: f32, x0: f32, slot: f32, angle: f32) {
    const CX: f32 = 130.;
    const LEN: f32 = 70.;
    rect(f, 86., 110., 272., 126., IRON);
    line(f, at(44., 118.), at(272., 118.), IRON_LIGHT, 2.);
    for i in 0..23 {
        let x = 90. + i as f32 * 8.;
        line(f, at(x, 115.), at(x + 4., 121.), alpha(TEXT, 0.35), 1.);
    }
    rect(f, x0 + 25., 110., x0 + 43., 124., METAL.side);
    rect(f, x0 - 6., 100., x0 + LEN + 6., 110., METAL.top);
    stock(f, x0, 64., x0 + LEN, 100.);
    rect(f, x0 - 1., 62., (x0 + slot).min(x0 + LEN), 76., BACKGROUND);
    side_cutter(f, CX, 76., time, 20., false);
    handwheel(f, at(44., 118.), 20., angle);
}

fn hand_feed(f: &mut Frame, time: f32, phase: f32) {
    let u = progress(phase, 0.15);
    let x0 = 150. - 58. * u;
    let slot = (140. - x0).max(0.);
    hand_mill(f, time, x0, slot, (150. - x0) * 0.12);
    panel(f, 164., 17., 270., 47.);
    say(f, "HAND FEED", 170., 20., GOLD, 9.);
    say(f, "120 MM/MIN", 170., 31., TEXT, 14.);
    arrow(f, at(214., 133.), at(190., 133.), CYAN);
    say(f, "TABLE", 218., 129., CYAN, 9.);
    heading(f, "HAND FEED · FASTEST STEADY TURN");
    caption(f, "TURN THE HANDWHEEL STEADILY: THE TABLE FEEDS");
}

fn hand_rapid(f: &mut Frame, time: f32, phase: f32) {
    let x0 = track(
        phase,
        &[
            (0., 140.),
            (0.25, 100.),
            (0.45, 190.),
            (0.65, 190.),
            (0.85, 140.),
            (1., 140.),
        ],
    );
    let slot = if phase < 0.25 {
        (140. - x0).max(0.)
    } else {
        40.
    };
    let angle = (140. - x0) * 0.12;
    hand_mill(f, time, x0, slot, angle);
    let out = (0.25..0.45).contains(&phase);
    let back = (0.65..0.85).contains(&phase);
    if out || back {
        let c = at(44., 118.);
        for j in 0..3 {
            let (a0, a1) = if out { (0.3, 1.1) } else { (-1.1, -0.3) };
            arc(
                f,
                c,
                24. + j as f32 * 3.,
                a0,
                a1,
                alpha(CYAN, 0.9 - 0.2 * j as f32),
                2.,
            );
        }
        for j in 0..3 {
            let x = 200. + j as f32 * 10.;
            let d = if out { 4. } else { -4. };
            poly_line(
                f,
                &[at(x - d, 126.), at(x + d, 132.), at(x - d, 138.)],
                CYAN,
                2.,
            );
        }
    }
    panel(f, 164., 17., 270., 47.);
    if out || back {
        say(f, "HAND RAPID", 170., 20., GOLD, 9.);
        say(f, "1500 MM/MIN", 170., 31., CYAN, 14.);
    } else if !(0.25..0.85).contains(&phase) {
        say(f, "CUTTING FEED", 170., 20., TEXT, 9.);
        say(f, "120 MM/MIN", 170., 31., alpha(TEXT, 0.8), 14.);
    } else {
        say(f, "HAND RAPID", 170., 20., GOLD, 9.);
        say(f, "CLEAR OF THE CUT", 170., 33., TEXT, 10.);
    }
    heading(f, "HAND RAPID · MOVING OFF THE CUT");
    caption(f, "CRANK THE AXIS QUICKLY AWAY BETWEEN CUTS");
}

/// Turn: one turn of the handwheel moves the table a fixed distance.
fn turn_one(f: &mut Frame, phase: f32) {
    let u = progress(phase, 0.2);
    let c = at(50., 91.);
    let x_l = 100. + 80. * u;
    rect(f, 96., 98., 258., 108., IRON);
    dashed_rect(f, 100., 84., 164., 98., DIM);
    line(f, c, at(x_l, 91.), IRON_LIGHT, 3.);
    rect(f, x_l, 84., x_l + 64., 98., METAL.top);
    rect(f, x_l + 14., 74., x_l + 50., 84., STOCK.front);
    handwheel(f, c, 30., TAU * u);
    arc(f, c, 37., 0., TAU * u, GOLD, 2.);
    if 80. * u > 10. {
        dashed(f, at(100., 84.), at(100., 64.), GOLD, 1.);
        dashed(f, at(x_l, 74.), at(x_l, 64.), GOLD, 1.);
        dimension(f, at(100., 68.), at(x_l, 68.), GOLD);
    }
    // The readouts are the same tenth of a turn and five times it in mm.
    let q = (u * 10.).round() / 10.;
    say(f, "1 TURN = 5 MM", 108., 42., GOLD, 12.);
    say(f, &format!("{q:.1} TURN"), 18., 128., TEXT, 9.);
    say(f, &format!("{:.1} MM", 5. * q), 112., 113., TEXT, 9.);
    heading(f, "TURN · TABLE TRAVEL PER HANDWHEEL TURN");
    caption(f, "ONE TURN OF THE HANDWHEEL MOVES THE AXIS 5 MM");
}

/// Swing: the circle a part can turn over the bed.
fn swing(f: &mut Frame, time: f32, phase: f32) {
    let big = phase >= 0.5;
    let c = at(110., 76.);
    let r_swing = 42.;
    rect(f, 50., 118., 170., 130., IRON);
    rect(f, 50., 118., 170., 120., IRON_LIGHT);
    let pr = if big { 54. } else { 34. };
    disc_at(f, c, pr, METAL.front);
    ring(f, c, pr, if big { RED } else { TEXT }, 1.6);
    let a = time * SPIN;
    for j in 0..3 {
        let b = a + j as f32 * TAU / 3.;
        line(f, pt(c, 5., b), pt(c, pr - 3., b), alpha(HOLE, 0.7), 1.6);
    }
    disc_at(f, c, 4., TEXT);
    dashed_circle(f, c, r_swing, CYAN);
    if big {
        rect(f, 50., 118., 170., 130., alpha(RED, 0.65));
        say(f, "TOO BIG:", 200., 98., RED, 10.);
        say(f, "HITS THE BED", 200., 110., RED, 10.);
    } else {
        say(f, "FITS OVER", 200., 98., OK, 10.);
        say(f, "THE BED", 200., 110., OK, 10.);
    }
    dashed(f, at(110., 34.), at(194., 34.), DIM, 1.);
    dashed(f, at(110., 118.), at(194., 118.), DIM, 1.);
    dimension(f, at(188., 34.), at(188., 118.), GOLD);
    say(f, "SWING", 200., 62., GOLD, 11.);
    say(f, "Ø OVER BED", 200., 75., GOLD, 11.);
    heading(f, "SWING · LARGEST Ø TURNED OVER THE BED");
    caption(f, "A BAR OR PART WIDER THAN THE SWING IS REFUSED");
}

/// Centres: the longest work between chuck and tailstock.
fn centres(f: &mut Frame, time: f32, phase: f32) {
    let u = smooth(progress(phase, 0.45));
    let tx = 150. + 88. * u;
    let tip = tx - 10.;
    rect(f, 14., 112., 264., 124., IRON);
    rect(f, 14., 52., 52., 112., IRON_LIGHT);
    rect(f, 52., 62., 62., 102., STEEL.front);
    polygon(f, &[at(62., 78.), at(62., 86.), at(72., 82.)], STEEL.top);
    dashed_rect(f, 238., 58., 260., 112., DIM);
    rect(f, tx, 58., tx + 22., 112., IRON_LIGHT);
    polygon(f, &[at(tx, 78.), at(tx, 86.), at(tip, 82.)], STEEL.top);
    rect(f, 72., 74., tip, 90., METAL.top);
    for k in 0..3 {
        let a = time * SPIN + k as f32 * TAU / 3.;
        if a.cos() > 0. {
            let y = 82. + 8. * a.sin();
            line(f, at(72., y), at(tip, y), alpha(HOLE, 0.5), 1.2);
        }
    }
    dashed(f, at(72., 78.), at(72., 40.), GOLD, 1.);
    dashed(f, at(tip, 78.), at(tip, 40.), GOLD, 1.);
    dimension(f, at(72., 44.), at(tip, 44.), GOLD);
    say_mid(
        f,
        "DISTANCE BETWEEN CENTRES",
        (72. + tip) / 2.,
        24.,
        GOLD,
        10.,
    );
    say_mid(f, "LONGEST WORK", (72. + tip) / 2., 98., TEXT, 9.);
    heading(f, "CENTRES · LONGEST WORK, CHUCK TO TAILSTOCK");
    caption(f, "THE TAILSTOCK SLIDES OUT TO ITS LAST POSITION");
}

/// Cross reads: one cross-slide move, read as radius or as diameter.
fn cross_reads(f: &mut Frame, time: f32, phase: f32) {
    let radius_mode = phase < 0.5;
    let sub = (phase * 2.) % 1.;
    let u = smooth(progress(sub, 0.4));
    let c = at(88., 76.);
    let r0 = 42.;
    let r = r0 - 14. * u;
    let tip = c.x + r;
    dashed(f, at(c.x + r0, 54.), at(c.x + r0, 76.), CYAN, 1.);
    disc_at(f, c, r, METAL.front);
    ring(f, c, r, TEXT, 1.4);
    dashed_circle(f, c, r0, DIM);
    let a = time * SPIN;
    line(f, pt(c, r - 9., a), pt(c, r - 3., a), alpha(TEXT, 0.7), 1.6);
    polygon(
        f,
        &[
            at(tip, 76.),
            at(tip + 10., 67.),
            at(tip + 44., 67.),
            at(tip + 44., 85.),
            at(tip + 10., 85.),
        ],
        alpha(GOLD, 0.9),
    );
    if u > 0.1 {
        dimension(f, at(tip, 58.), at(c.x + r0, 58.), CYAN);
    }
    let col_r = if radius_mode { GOLD } else { alpha(CYAN, 0.7) };
    let col_d = if radius_mode { alpha(CYAN, 0.7) } else { GOLD };
    dimension(f, at(c.x - r, c.y), at(c.x, c.y), col_r);
    say(
        f,
        &format!("R {:.1}", 3. - u),
        c.x - r + 6.,
        c.y - 15.,
        col_r,
        9.,
    );
    dashed(f, at(c.x - r, c.y), at(c.x - r, 130.), col_d, 1.);
    dashed(f, at(c.x + r, c.y), at(c.x + r, 130.), col_d, 1.);
    dimension(f, at(c.x - r, 126.), at(c.x + r, 126.), col_d);
    say(
        f,
        &format!("Ø {:.1}", 6. - 2. * u),
        c.x + r + 6.,
        120.,
        col_d,
        9.,
    );
    let dim = |on: bool| if on { GOLD } else { alpha(TEXT, 0.55) };
    say(f, "TOOL IN 1 MM", 186., 22., CYAN, 9.);
    say(f, "RADIUS -1 MM", 186., 36., dim(radius_mode), 9.);
    say(f, "Ø -2 MM", 186., 50., dim(!radius_mode), 9.);
    let (reading, mode) = if radius_mode {
        (u, "ON RADIUS")
    } else {
        (2. * u, "ON DIAMETER")
    };
    say(f, "DIAL READS", 186., 88., TEXT, 9.);
    say(f, &format!("{reading:.1}"), 186., 100., GOLD, 20.);
    say(f, mode, 186., 126., GOLD, 10.);
    heading(f, "CROSS READS · ON THE RADIUS OR THE Ø");
}

/// Cross turn: the dial's numbers in one full turn.
fn cross_turn(f: &mut Frame, phase: f32) {
    let u = progress(phase, 0.2);
    let c = at(84., 80.);
    let r = 46.;
    // The dial turns anticlockwise while its numbers rise clockwise, as on a
    // clock face: each next number arrives at the index in turn.
    let rot = -TAU * u;
    disc_at(f, c, r, Color::from_rgb(0.20, 0.28, 0.35));
    ring(f, c, r, TEXT, 1.8);
    for i in 0..50 {
        let a = rot + i as f32 * TAU / 50.;
        let major = i % 10 == 0;
        let (len, col, w) = if major {
            (10., GOLD, 2.)
        } else {
            (5., TEXT, 1.)
        };
        line(f, pt(c, r - 1., a), pt(c, r - len, a), col, w);
    }
    for k in 0..5 {
        let a = rot + k as f32 * TAU / 5.;
        let t = pt(c, 28., a);
        if k == 0 && u >= 0.97 {
            say(f, "5/0", t.x - 10., t.y - 6., GOLD, 12.);
        } else {
            say(f, &k.to_string(), t.x - 3.5, t.y - 6., TEXT, 12.);
        }
    }
    disc_at(f, c, 4., TEXT);
    polygon(
        f,
        &[
            at(c.x, c.y - r - 2.),
            at(c.x - 5., c.y - r - 11.),
            at(c.x + 5., c.y - r - 11.),
        ],
        CYAN,
    );
    arc(f, c, r + 5., 0., rot, GOLD, 2.5);
    say(f, "AT THE INDEX", 152., 34., TEXT, 9.);
    let reading = if u >= 0.97 {
        "5 / 0".to_string()
    } else {
        format!("{:.1}", 5. * u)
    };
    say(f, &reading, 152., 48., GOLD, 30.);
    let done = if u >= 0.97 { GOLD } else { CYAN };
    say(f, "FULL TURN = 5.0", 152., 102., done, 10.);
    heading(f, "DIAL TURN · NUMBERS IN ONE FULL TURN");
    caption(f, "THE READING AT THE INDEX AFTER ONE FULL TURN");
}

/// Cross div: the divisions round the collar, counted.
fn cross_div(f: &mut Frame, phase: f32) {
    let u = progress(phase, 0.2);
    let n = (40. * u).round() as usize;
    let c = at(84., 82.);
    let r = 46.;
    disc_at(f, c, r, Color::from_rgb(0.20, 0.28, 0.35));
    ring(f, c, r, TEXT, 1.8);
    for i in 0..40 {
        let a = i as f32 * TAU / 40.;
        let lit = i < n;
        let len = if i % 5 == 0 { 14. } else { 8. };
        let (col, w) = if lit {
            (GOLD, 2.4)
        } else {
            (alpha(CYAN, 0.35), 1.2)
        };
        line(f, pt(c, r - 1., a), pt(c, r - len, a), col, w);
    }
    // The magnifier it is read through.
    ring(f, c, r + 4., EDGE, 2.5);
    line(f, pt(c, r + 4., 2.36), pt(c, r + 22., 2.36), EDGE, 5.);
    polygon(
        f,
        &[
            at(c.x, c.y - r - 2.),
            at(c.x - 5., c.y - r - 11.),
            at(c.x + 5., c.y - r - 11.),
        ],
        CYAN,
    );
    say_mid(f, "COLLAR", c.x, c.y - 5., alpha(TEXT, 0.6), 9.);
    say(f, "COUNTED", 156., 34., TEXT, 9.);
    say(f, &n.to_string(), 156., 48., GOLD, 32.);
    let col = if n == 40 { GOLD } else { CYAN };
    say(f, "OF 40 DIVISIONS", 156., 100., col, 10.);
    heading(f, "DIAL DIVISIONS · MARKS ROUND THE COLLAR");
    caption(f, "A READING IS ROUNDED TO ONE DIVISION");
}

/// Feeds: how far the carriage advances for each turn of the bar.
fn feeds(f: &mut Frame, phase: f32) {
    let first = phase < 0.5;
    let (p, name) = if first { (14., "0.1") } else { (28., "0.2") };
    let sub = (phase * 2.) % 1.;
    let u = progress(sub, 0.2);
    let travel = 4. * p * u;
    let (xs, xe) = (214., 220.);
    let xt = xs - 4. * p * u;
    let revs = travel / p;
    let spin = TAU * revs;
    rect(f, 14., 52., 52., 106., STEEL.side);
    // A half section of the bar: the tool side, down to the axis.
    rect(f, 52., 64., xe, 92., STOCK.front);
    rect(f, xt, 63., xe, 69., BACKGROUND);
    rect(f, xt, 69., xe, 92., FRESH.front);
    dashed(f, at(52., 92.), at(xe, 92.), alpha(TEXT, 0.5), 1.);
    let mark = Color::from_rgba(0.1, 0.15, 0.2, 0.45);
    for k in 0..3 {
        let a = spin + k as f32 * TAU / 3.;
        if a.cos() > 0. {
            let y = 78. + 12. * a.sin();
            line(f, at(52., y), at(xt, y.min(91.)), mark, 1.2);
        }
    }
    for k in 1..=(revs as usize) {
        let x = xs - k as f32 * p;
        line(f, at(x, 69.), at(x, 92.), alpha(HOLE, 0.8), 1.);
    }
    rect(
        f,
        xt + 3.,
        43.,
        xt + 30.,
        57.,
        Color::from_rgb(0.62, 0.68, 0.73),
    );
    polygon(
        f,
        &[
            at(xt, 69.),
            at(xt + 14., 69.),
            at(xt + 10., 57.),
            at(xt + 3., 57.),
        ],
        GOLD,
    );
    // The bar seen from its end, once round for each step.
    let c = at(30., 30.);
    ring(f, c, 11., TEXT, 1.4);
    line(f, c, pt(c, 10., spin), GOLD, 2.);
    say(f, &format!("REV {:.1}", revs), 46., 26., TEXT, 9.);
    say(f, "SECTION", 56., 95., alpha(TEXT, 0.6), 8.);
    dashed(f, at(xt, 92.), at(xt, 110.), GOLD, 1.);
    dashed(f, at(xt + p, 92.), at(xt + p, 110.), GOLD, 1.);
    dimension(f, at(xt, 106.), at(xt + p, 106.), GOLD);
    say(f, &format!("{name} MM PER REV"), 110., 112., GOLD, 11.);
    heading(f, "POWER FEED · CARRIAGE ADVANCE PER REV");
    caption(f, "THE CARRIAGE ADVANCES A SET AMOUNT PER TURN");
}

/// Pitches: a thread being cut, the pitch changing through the list.
fn pitches(f: &mut Frame, phase: f32) {
    // Drawn 28 px to the mm, a half section of the bar down to its axis.
    let list = [("0.5", 14.), ("1.0", 28.), ("1.5", 42.)];
    let i = ((phase * 3.) as usize).min(2);
    let sub = phase * 3. - i as f32;
    let (name, p) = list[i];
    let u = smooth(progress(sub, 0.3));
    let (xa, xb, cy, rr) = (30., 244., 90., 32.);
    let depth = 0.4 * p;
    let xt = xb - (xb - xa) * u;
    let groove = |x: f32| -> f32 {
        let s = ((xb - x) / p).rem_euclid(1.);
        depth * (1. - (2. * s - 1.).abs())
    };
    let mut top: Vec<(f32, f32)> = Vec::new();
    let mut j = 0;
    loop {
        let x = xb - j as f32 * p / 2.;
        if x < xt {
            break;
        }
        top.push((x, cy - rr + if j % 2 == 1 { depth } else { 0. }));
        j += 1;
    }
    let tipy = cy - rr + groove(xt);
    top.push((xt, tipy));
    top.push((xt - 0.01, cy - rr));
    top.push((xa, cy - rr));
    let mut pts: Vec<Point> = top.iter().map(|(x, y)| at(*x, *y)).collect();
    pts.push(at(xa, cy));
    pts.push(at(xb, cy));
    polygon(f, &pts, METAL.front);
    rect(f, xa, cy - rr, xt, cy, STOCK.front);
    let upper: Vec<Point> = top.iter().map(|(x, y)| at(*x, *y)).collect();
    poly_line(f, &upper, TEXT, 1.2);
    dashed(f, at(xa - 8., cy), at(xb + 8., cy), alpha(TEXT, 0.5), 1.);
    say(f, "HALF SECTION", xa, cy + 3., alpha(TEXT, 0.6), 8.);
    polygon(
        f,
        &[
            at(xt, tipy),
            at(xt - 9., tipy - 15.),
            at(xt + 9., tipy - 15.),
        ],
        GOLD,
    );
    rect(
        f,
        xt - 9.,
        tipy - 30.,
        xt + 9.,
        tipy - 15.,
        Color::from_rgb(0.62, 0.68, 0.73),
    );
    if xt < xb - p - 4. {
        dashed(f, at(xb - p, cy), at(xb - p, 118.), GOLD, 1.);
        dashed(f, at(xb, cy), at(xb, 118.), GOLD, 1.);
        dimension(f, at(xb - p, 114.), at(xb, 114.), GOLD);
    }
    say(f, &format!("PITCH {name} MM"), 90., 112., GOLD, 11.);
    say(f, "PITCHES (MM):", 8., 143., TEXT, 9.);
    for (k, (n, _)) in list.iter().enumerate() {
        let col = if k == i { GOLD } else { alpha(TEXT, 0.5) };
        say(f, n, 92. + k as f32 * 34., 142., col, 10.);
    }
    heading(f, "THREAD PITCHES THE GEARBOX CUTS");
}

/// DRO compensation: the edge or the centre of the tool is what it reads.
fn dro_comp(f: &mut Frame, time: f32, phase: f32) {
    let on = phase < 0.5;
    let (zero, edge, r) = (22., 110., 16.);
    stock(f, 58., 34., edge, 118.);
    dashed(f, at(zero, 26.), at(zero, 132.), CYAN, 1.);
    say(f, "ZERO", 6., 16., CYAN, 9.);
    let c = at(edge + r, 76.);
    top_cutter(f, c, r, time * SPIN * 2., 4);
    let px = if on { edge } else { c.x };
    if on {
        ring(f, at(edge, 76.), 4.5, CYAN, 2.);
        disc_at(f, at(edge, 76.), 1.8, CYAN);
    } else {
        line(f, at(c.x - 6., 76.), at(c.x + 6., 76.), CYAN, 2.);
        line(f, at(c.x, 70.), at(c.x, 82.), CYAN, 2.);
    }
    dashed(f, at(px, 92.), at(px, 132.), CYAN, 1.);
    dimension(f, at(zero, 128.), at(px, 128.), CYAN);
    dashed(f, at(edge, 76.), at(edge, 108.), GOLD, 1.);
    dashed(f, at(c.x, 92.), at(c.x, 108.), GOLD, 1.);
    dimension(f, at(edge, 104.), at(c.x, 104.), GOLD);
    say(f, "R = TOOL RADIUS", c.x + 7., 99., GOLD, 9.);
    panel(f, 168., 20., 274., 90.);
    say(f, "DRO", 174., 24., TEXT, 9.);
    say(
        f,
        if on { "COMP ON" } else { "COMP OFF" },
        174.,
        36.,
        GOLD,
        13.,
    );
    let val = (px - zero) / 4.;
    say(f, &format!("X {val:.1}"), 174., 54., CYAN, 18.);
    say(
        f,
        if on {
            "READS THE EDGE"
        } else {
            "READS THE CENTRE"
        },
        174.,
        78.,
        TEXT,
        9.,
    );
    heading(f, "DRO COMPENSATION · EDGE OR CENTRE");
    caption(
        f,
        if on {
            "COMP ON: THE DRO READS THE TOOL'S EDGE"
        } else {
            "COMP OFF: THE DRO READS THE TOOL CENTRE"
        },
    );
}

/// Edge finder: touch the face, the tip kicks off, the zero is one radius
/// from the spindle centre.
fn edge_finder(f: &mut Frame, time: f32, phase: f32) {
    let (face, r, cy) = (96., 24., 76.);
    stock(f, 24., 26., face, 120.);
    let approach = smooth(progress(phase, 0.5));
    let cx = 214. - (214. - (face + r)) * approach;
    let touched = phase >= 0.5;
    let kick = smooth(((phase - 0.5) / 0.06).clamp(0., 1.));
    let a = time * SPIN * 2.;
    let wobble = (3. * a.cos(), 3. * a.sin());
    let off = (wobble.0 * (1. - kick), -20. * kick + wobble.1 * (1. - kick));
    let c = at(cx, cy);
    disc_at(f, c, r, Color::from_rgb(0.45, 0.52, 0.58));
    ring(f, c, r, TEXT, 1.8);
    if touched {
        dashed_circle(f, c, 9., DIM);
    }
    let tip = at(cx + off.0, cy + off.1);
    disc_at(f, tip, 9., FRESH.top);
    ring(f, tip, 9., GOLD, 1.8);
    line(f, at(cx - 6., cy), at(cx + 6., cy), CYAN, 2.);
    line(f, at(cx, cy - 6.), at(cx, cy + 6.), CYAN, 2.);
    if !touched {
        arrow(f, at(cx + r + 34., cy), at(cx + r + 10., cy), CYAN);
        say_mid(f, "EDGE FINDER", cx.max(170.), cy - r - 14., TEXT, 9.);
    } else {
        // The radius runs along the lower half of the body, clear of the tip.
        dashed(f, at(cx, cy), at(cx, cy + 18.), GOLD, 1.);
        dimension(f, at(face, cy + 14.), at(cx, cy + 14.), GOLD);
        say(f, "R = Ø ÷ 2", cx + r + 6., cy + 8., GOLD, 10.);
        say(f, "TIP KICKS OFF", cx + r + 6., cy - 28., CYAN, 10.);
        dashed(
            f,
            at(face + 2. * r, cy + 24.),
            at(face + 2. * r, 112.),
            CYAN,
            1.,
        );
        dimension(f, at(face, 108.), at(face + 2. * r, 108.), CYAN);
        say(f, "Ø = EDGE FINDER", face + 2. * r + 8., 104., CYAN, 9.);
    }
    say_mid(f, "FACE = WORK ZERO", 60., 127., GOLD, 9.);
    heading(f, "EDGE FINDER Ø · PICKS UP THE WORK ZERO");
    caption(f, "SPINDLE CENTRE SITS ONE RADIUS FROM THE FACE");
}

/// Pass time: each pass adds a block of seconds to the total.
fn pass_time(f: &mut Frame, phase: f32) {
    let step = 0.17;
    let k = ((phase / step) as usize + 1).min(5);
    let frac = ((phase - (k - 1) as f32 * step) / step).clamp(0., 1.);
    let grow = (frac / 0.5).clamp(0., 1.);
    let total = 12 * (k - 1) + (12. * grow).round() as usize;
    let c = at(46., 58.);
    disc_at(f, c, 22., Color::from_rgb(0.20, 0.28, 0.35));
    ring(f, c, 22., TEXT, 1.8);
    rect(f, c.x - 3., c.y - 29., c.x + 3., c.y - 23., TEXT);
    for i in 0..12 {
        let a = i as f32 * TAU / 12.;
        line(f, pt(c, 19., a), pt(c, 22., a), TEXT, 1.);
    }
    let hand = TAU * frac;
    arc(f, c, 16., 0., hand, alpha(GOLD, 0.5), 5.);
    line(f, c, pt(c, 17., hand), GOLD, 2.);
    say(f, "+12 S PER PASS", 86., 30., GOLD, 12.);
    say(f, "RUNNING TOTAL", 86., 54., TEXT, 9.);
    say(f, &format!("{total} S"), 86., 66., TEXT, 22.);
    block_row(f, 24., 104., 44., 22., 3., 5, k - 1, grow, "+12");
    for i in 0..5 {
        say_mid(
            f,
            &format!("PASS {}", i + 1),
            24. + i as f32 * 47. + 22.,
            129.,
            alpha(TEXT, 0.6),
            8.,
        );
    }
    heading(f, "PASS TIME · SECONDS ADDED PER PASS");
    caption(f, "RESET, SET AND LOCK THE AXES, EVERY PASS");
}

/// Tool time: each tool change adds minutes to the setup.
fn tool_time(f: &mut Frame, phase: f32) {
    let t0 = 0.08;
    let step = (0.96 - t0) / 4.;
    let (idx, frac) = if phase < t0 {
        (0, -1.)
    } else {
        let i = (((phase - t0) / step) as usize).min(3);
        (i, ((phase - t0 - i as f32 * step) / step).clamp(0., 1.))
    };
    let slide = smooth((frac / 0.3).clamp(0., 1.));
    let grow = ((frac - 0.45) / 0.35).clamp(0., 1.);
    let total = 4 * idx + (4. * grow).round() as usize;
    let colors = [GOLD, CYAN, TEXT, OK];
    rect(f, 34., 26., 58., 38., STEEL.front);
    if frac >= 0. {
        tool_glyph(f, 46. - 28. * (1. - slide), 38., 38., 12., colors[idx]);
    }
    if frac >= 0.3 {
        // The zero picked up again with the new tool.
        line(f, at(36., 80.), at(56., 80.), CYAN, 1.6);
        line(f, at(46., 72.), at(46., 88.), CYAN, 1.6);
        ring(f, at(46., 80.), 7., CYAN, 1.2);
    }
    say(f, "+4 MIN PER TOOL CHANGE", 86., 28., GOLD, 11.);
    say(f, "SETUP TOTAL", 86., 52., TEXT, 9.);
    say(f, &format!("{total} MIN"), 86., 64., TEXT, 22.);
    block_row(f, 24., 104., 56., 22., 3., 4, idx, grow, "+4 MIN");
    for i in 0..4 {
        say_mid(
            f,
            &format!("TOOL {}", i + 1),
            24. + i as f32 * 59. + 28.,
            129.,
            alpha(TEXT, 0.6),
            8.,
        );
    }
    heading(f, "TOOL TIME · MINUTES PER TOOL CHANGE");
    caption(f, "CHANGE THE TOOL, THEN PICK UP THE ZERO AGAIN");
}

/// Move time: the stock goes between two machines and minutes are added.
fn move_time(f: &mut Frame, phase: f32) {
    let s = track(
        phase,
        &[(0., 0.), (0.3, 1.), (0.5, 1.), (0.8, 0.), (1., 0.)],
    );
    let moves = usize::from(phase >= 0.3) + usize::from(phase >= 0.8);
    mill_icon(f, 12., 28., 0.8, phase);
    lathe_icon(f, 202., 28., 0.8, phase);
    say_mid(f, "MILL", 44., 80., TEXT, 9.);
    say_mid(f, "LATHE", 234., 80., TEXT, 9.);
    dashed(f, at(88., 94.), at(190., 94.), DIM, 1.5);
    arrow(f, at(180., 94.), at(192., 94.), DIM);
    arrow(f, at(98., 94.), at(86., 94.), DIM);
    let x = 92. + 86. * s;
    rect(f, x - 8., 87., x + 8., 99., STOCK.front);
    outline(f, x - 8., 87., x + 8., 99., GOLD, 1.2);
    say_mid(f, "TOTAL", 139., 24., TEXT, 9.);
    say_mid(f, &format!("{} MIN", 15 * moves), 139., 36., GOLD, 16.);
    if (0.3..0.5).contains(&phase) || phase >= 0.8 {
        say_mid(f, "+15 MIN", 139., 62., GOLD, 11.);
    }
    let newest = moves.saturating_sub(1);
    let grow = if moves == 0 {
        0.
    } else {
        let start = if moves == 1 { 0.3 } else { 0.8 };
        ((phase - start) / 0.1).clamp(0., 1.)
    };
    for k in 0..2 {
        let bx = 24. + k as f32 * 110.;
        rect(f, bx, 108., bx + 106., 128., alpha(CYAN, 0.08));
        if moves > 0 && k < newest {
            rect(f, bx, 108., bx + 106., 128., GOLD);
            say_mid(f, "+15 MIN", bx + 53., 113., BACKGROUND, 10.);
        } else if moves > 0 && k == newest {
            rect(f, bx, 108., bx + 106. * grow, 128., GOLD);
            if grow >= 0.5 {
                say_mid(f, "+15 MIN", bx + 53., 113., BACKGROUND, 10.);
            }
        }
    }
    heading(f, "MOVE TIME · MINUTES BETWEEN MACHINES");
    caption(f, "EACH MOVE OF THE STOCK ADDS THESE MINUTES");
}

// --------------------------------------------------------------- machine

/// Rpm: a spindle dial with the limit; an operation asking more is flagged.
fn rpm_limit(f: &mut Frame, time: f32, phase: f32) {
    let c = at(100., 88.);
    let limit = 0.75;
    let frac = track(
        phase,
        &[
            (0., 0.),
            (0.3, 0.55),
            (0.45, 0.55),
            (0.65, 0.92),
            (0.85, 0.92),
            (1., 0.),
        ],
    );
    let over = frac > limit;
    gauge(
        f,
        c,
        52.,
        frac,
        &[(0., limit, NORMAL), (limit, 1., RED)],
        if over { RED } else { TEXT },
    );
    let k = 1. + (frac * 8.).floor();
    let sp = at(100., 114.);
    disc_at(f, sp, 9., Color::from_rgb(0.22, 0.29, 0.36));
    ring(f, sp, 9., TEXT, 1.2);
    for j in 0..2 {
        let a = time * SPIN * k + j as f32 * PI;
        line(
            f,
            pt(sp, 2., a),
            pt(sp, 8., a),
            if j == 0 { GOLD } else { EDGE },
            1.8,
        );
    }
    let m = pt(c, 66., ang(limit));
    say(f, "MAX", m.x - 2., m.y - 8., GOLD, 10.);
    say(f, "0", 52., 128., TEXT, 9.);
    say(f, "MAX 9000 RPM", 196., 24., GOLD, 11.);
    say(f, "OPERATION ASKS", 196., 54., TEXT, 9.);
    say(
        f,
        &format!("{} RPM", ((frac * 12000.) / 100.).round() as i32 * 100),
        196.,
        66.,
        if over { RED } else { TEXT },
        14.,
    );
    if over {
        say(f, "ABOVE THE MAX:", 196., 94., RED, 10.);
        say(f, "FLAGGED", 196., 106., RED, 10.);
    } else {
        say(f, "WITHIN", 196., 94., OK, 10.);
        say(f, "THE LIMIT", 196., 106., OK, 10.);
    }
    heading(f, "MAX RPM · THE SPINDLE'S SPEED LIMIT");
    caption(f, "AN OPERATION ASKING FOR MORE IS FLAGGED");
}

/// Power: the spindle's rating limits the cutting force it can drive.
fn power(f: &mut Frame, time: f32, phase: f32) {
    let c = at(66., 88.);
    let limit = 0.667;
    let frac = track(phase, &[(0., 0.05), (0.5, limit), (0.6, limit), (1., 0.05)]);
    gauge(
        f,
        c,
        40.,
        frac,
        &[(0., limit, NORMAL), (limit, 1., RED)],
        TEXT,
    );
    say_mid(
        f,
        &format!("{} W", (frac * 1200.).round() as i32),
        66.,
        106.,
        GOLD,
        12.,
    );
    let m = pt(c, 52., ang(limit));
    say(f, "RATED", m.x - 2., m.y - 10., GOLD, 9.);
    // The cutter works along the stock and the force it meets.
    let cx = 250. - 50. * phase;
    stock(f, 130., 100., 270., 128.);
    rect(f, cx - 10., 99., 270., 110., BACKGROUND);
    side_cutter(f, cx, 110., time, 20., false);
    let len = 8. + 52. * (frac / limit);
    arrow(f, at(cx - 12., 84.), at(cx - 12. - len, 84.), GOLD);
    say(f, "CUTTING FORCE", 130., 52., GOLD, 10.);
    if frac >= limit - 0.01 {
        say(f, "MAX = RATED", 130., 64., RED, 10.);
    }
    heading(f, "SPINDLE POWER · LIMITS THE CUTTING FORCE");
    caption(f, "THE HOLDING CHECK USES THIS CUTTING FORCE");
}

/// Feed: the table speeds up to the machine's limit; faster is flagged.
fn feed_limit(f: &mut Frame, _time: f32, phase: f32) {
    let limit = 0.667;
    let keys = [
        (0., 0.1),
        (0.4, 0.55),
        (0.55, 0.55),
        (0.75, 0.93),
        (0.9, 0.93),
        (1., 0.1),
    ];
    // The bar and the words share one rounded value.
    let value = ((track(phase, &keys) * 3000.) / 50.).round() as i32 * 50;
    let frac = value as f32 / 3000.;
    let over = value > 2000;
    let mut pos = 0.;
    let steps = 60;
    for i in 0..steps {
        pos += track(phase * (i as f32 + 0.5) / steps as f32, &keys) * phase * 6.;
    }
    bar_gauge(f, 20., 36., 258., 46., frac, limit);
    say(
        f,
        "MAX 2000 MM/MIN",
        20. + 238. * limit - 94.,
        20.,
        GOLD,
        10.,
    );
    say(
        f,
        &format!("{value} MM/MIN"),
        20.,
        52.,
        if over { RED } else { TEXT },
        12.,
    );
    if over {
        say(f, "ABOVE MAX: FLAGGED", 162., 66., RED, 10.);
    } else {
        say(f, "WITHIN THE MAX", 162., 66., OK, 10.);
    }
    // The table with parts on it, sliding left.
    rect(f, 14., 100., 264., 112., IRON_LIGHT);
    rect(f, 14., 112., 264., 124., IRON);
    rect(f, 14., 88., 264., 100., STOCK.front);
    for i in 0..10 {
        let x = 14. + ((i as f32 * 30. - pos) % 300. + 300.) % 300.;
        if x < 264. {
            line(f, at(x, 88.), at(x, 100.), alpha(HOLE, 0.8), 1.4);
        }
    }
    side_cutter(f, 139., 88., 0., 20., false);
    let n = 1 + (frac * 5.) as usize;
    for j in 0..n {
        let x = 200. + j as f32 * 12.;
        poly_line(
            f,
            &[at(x + 4., 101.), at(x, 106.), at(x + 4., 111.)],
            CYAN,
            2.,
        );
    }
    heading(f, "MAX FEED · THE MACHINE'S TOP SPEED");
    caption(f, "AN OPERATION ABOVE THE MAX IS FLAGGED");
}

/// Kind: a CNC, a manual mill and a lathe, one after the other.
fn kind(f: &mut Frame, phase: f32) {
    let i = ((phase * 3.) as usize).min(2);
    let names = ["CYCLE CNC", "MANUAL MILL", "LATHE"];
    let notes = [
        "RUNS G-CODE",
        "HAND-OPERATED · DRO",
        "HAND-OPERATED · DIALS",
    ];
    for (j, name) in names.into_iter().enumerate() {
        let x = 8. + j as f32 * 90.;
        match j {
            0 => cnc_icon(f, x, 28., 1., phase),
            1 => mill_icon(f, x, 28., 1., phase),
            _ => lathe_icon(f, x, 28., 1., phase),
        }
        if j == i {
            outline(f, x - 4., 22., x + 82., 94., GOLD, 1.6);
        } else {
            rect(f, x - 4., 22., x + 82., 94., alpha(BACKGROUND, 0.65));
        }
        say_mid(
            f,
            name,
            x + 39.,
            98.,
            if j == i { GOLD } else { alpha(TEXT, 0.6) },
            9.,
        );
    }
    say_mid(f, notes[i], 139., 118., TEXT, 12.);
    heading(f, "MACHINE KIND · WHAT KIND OF MACHINE");
}

/// Travel: the table or spindle goes from one end of the axis to the other.
fn travel(f: &mut Frame, time: f32, phase: f32, axis: u8) {
    let s = smooth(1. - (2. * phase - 1.).abs());
    let a = time * SPIN * 2.;
    match axis {
        0 => {
            // Front view: the table slides under the spindle, never into the
            // column.
            rect(f, 20., 112., 258., 124., IRON);
            rect(f, 214., 22., 242., 112., IRON_LIGHT);
            rect(f, 112., 24., 214., 44., IRON_LIGHT);
            rect(f, 129., 44., 149., 52., STEEL.front);
            rect(f, 136., 52., 142., 74., METAL.top);
            rect(f, 136., 74., 142., 90., GOLD);
            let (lo, hi, w) = (24., 134., 70.);
            dashed_rect(f, lo, 102., lo + w, 112., DIM);
            dashed_rect(f, hi, 102., hi + w, 112., DIM);
            let tx = lo + (hi - lo) * s;
            rect(f, tx, 102., tx + w, 112., METAL.top);
            rect(f, tx + 16., 90., tx + 54., 102., STOCK.front);
            dashed(f, at(lo, 112.), at(lo, 134.), GOLD, 1.);
            dashed(f, at(hi, 112.), at(hi, 134.), GOLD, 1.);
            dimension(f, at(lo, 130.), at(hi, 130.), GOLD);
            say(f, "X TRAVEL", 158., 124., GOLD, 10.);
            dimension(f, at(20., 40.), at(56., 40.), CYAN);
            say(f, "X", 62., 34., CYAN, 12.);
            heading(f, "X TRAVEL · TABLE STROKE, END TO END");
            caption(f, "THE TABLE SLIDES FROM ONE END TO THE OTHER");
        }
        1 => {
            // Plan view: the table slides toward and away from the column,
            // and the spindle stays clear of the workpiece.
            rect(f, 40., 30., 200., 138., IRON);
            rect(f, 40., 30., 200., 48., IRON_LIGHT);
            say_mid(f, "COLUMN", 120., 34., TEXT, 8.);
            let (lo, hi) = (50., 106.);
            dashed_rect(f, 60., lo, 180., lo + 30., DIM);
            dashed_rect(f, 60., hi, 180., hi + 30., DIM);
            let ty = lo + (hi - lo) * s;
            rect(f, 60., ty, 180., ty + 30., METAL.top);
            rect(f, 68., ty + 6., 116., ty + 24., STOCK.top);
            top_cutter(f, at(152., 78.), 10., a, 4);
            dashed(f, at(180., lo), at(226., lo), GOLD, 1.);
            dashed(f, at(180., hi), at(226., hi), GOLD, 1.);
            dimension(f, at(222., lo), at(222., hi), GOLD);
            say(f, "Y", 232., 70., GOLD, 12.);
            say(f, "TRAVEL", 232., 84., GOLD, 9.);
            // The arrow follows the table: toward the operator, then back.
            if phase < 0.5 {
                arrow(f, at(50., 66.), at(50., 96.), CYAN);
            } else {
                arrow(f, at(50., 96.), at(50., 66.), CYAN);
            }
            dimension(f, at(18., 110.), at(18., 70.), CYAN);
            say(f, "Y", 14., 56., CYAN, 12.);
            heading(f, "Y TRAVEL · TABLE STROKE, TOWARD AND AWAY");
            caption(f, "THE TABLE SLIDES TOWARD AND AWAY FROM THE COLUMN");
        }
        _ => {
            // Front view: the spindle head rides up and down the column.
            rect(f, 20., 120., 258., 130., IRON);
            rect(f, 56., 112., 156., 120., METAL.top);
            rect(f, 74., 104., 128., 112., STOCK.front);
            let (hi, lo) = (20., 56.);
            let hy = hi + (lo - hi) * s;
            dashed(f, at(60., hi + 21.), at(228., hi + 21.), DIM, 1.);
            dashed(f, at(60., lo + 21.), at(228., lo + 21.), DIM, 1.);
            rect(f, 172., 18., 200., 120., IRON_LIGHT);
            rect(f, 92., hy, 172., hy + 16., IRON_LIGHT);
            rect(f, 93., hy + 16., 109., hy + 21., STEEL.front);
            rect(f, 98., hy + 21., 104., hy + 29., METAL.top);
            rect(f, 98., hy + 29., 104., hy + 41., GOLD);
            dashed(f, at(200., hi + 21.), at(224., hi + 21.), GOLD, 1.);
            dashed(f, at(200., lo + 21.), at(224., lo + 21.), GOLD, 1.);
            dimension(f, at(222., hi + 21.), at(222., lo + 21.), GOLD);
            say(f, "Z TRAVEL", 230., 62., GOLD, 9.);
            dimension(f, at(24., 90.), at(24., 50.), CYAN);
            say(f, "Z", 30., 62., CYAN, 12.);
            heading(f, "Z TRAVEL · SPINDLE STROKE, UP AND DOWN");
            caption(f, "THE SPINDLE HEAD RIDES THE COLUMN END TO END");
        }
    }
}

/// Tool change: a hand swaps the tool, then a carousel does it.
fn tool_change(f: &mut Frame, phase: f32) {
    let auto = phase >= 0.5;
    let s = (phase * 2.) % 1.;
    let dim = alpha(TEXT, 0.5);
    say_mid(f, "MANUAL", 70., 16., if auto { dim } else { GOLD }, 10.);
    say_mid(
        f,
        "AUTOMATIC",
        200.,
        16.,
        if auto { GOLD } else { dim },
        10.,
    );
    let (ux, uw) = if auto { (200., 28.) } else { (70., 20.) };
    line(f, at(ux - uw, 29.), at(ux + uw, 29.), GOLD, 1.5);
    rect(f, 40., 34., 100., 54., IRON_LIGHT);
    say_mid(f, "SPINDLE", 70., 40., TEXT, 8.);
    rect(f, 62., 54., 78., 61., STEEL.front);
    let b_col = CYAN;
    if !auto {
        rect(f, 150., 121., 262., 126., IRON);
        // The hand: reach the spindle tool, set it on the bench, fetch the
        // other one, carry it high over the bench and fit it.
        let keys_x = [
            (0., 150.),
            (0.1, 78.),
            (0.16, 78.),
            (0.26, 130.),
            (0.34, 190.),
            (0.40, 190.),
            (0.46, 190.),
            (0.52, 190.),
            (0.60, 244.),
            (0.64, 244.),
            (0.70, 244.),
            (0.78, 244.),
            (0.90, 78.),
            (0.96, 78.),
            (1., 78.),
        ];
        let keys_y = [
            (0., 58.),
            (0.1, 73.),
            (0.16, 73.),
            (0.26, 70.),
            (0.34, 70.),
            (0.40, 103.),
            (0.46, 103.),
            (0.52, 70.),
            (0.60, 70.),
            (0.64, 103.),
            (0.70, 103.),
            (0.78, 66.),
            (0.90, 66.),
            (0.96, 73.),
            (1., 73.),
        ];
        let gx = track(s, &keys_x);
        let gy = track(s, &keys_y);
        // Tool A: in the spindle, carried to its place on the bench.
        if s < 0.16 {
            tool_glyph(f, 70., 61., 30., 8., GOLD);
        } else if s < 0.40 {
            tool_glyph(f, gx - 8., gy - 12., 30., 8., GOLD);
        } else {
            tool_glyph(f, 182., 91., 30., 8., GOLD);
        }
        // Tool B: on the bench, then carried to the spindle.
        if s < 0.70 {
            tool_glyph(f, 236., 91., 30., 8., b_col);
        } else if s < 0.96 {
            tool_glyph(f, gx - 8., gy - 12., 30., 8., b_col);
        } else {
            tool_glyph(f, 70., 61., 30., 8., b_col);
        }
        line(f, at(gx + 6., gy + 2.), at(gx + 26., gy + 30.), SKIN, 11.);
        disc_at(f, at(gx + 6., gy), 7., SKIN);
        line(f, at(gx + 2., gy - 5.), at(gx - 4., gy - 5.), SKIN, 3.);
        line(f, at(gx + 2., gy + 5.), at(gx - 4., gy + 5.), SKIN, 3.);
        say(f, "A HAND SWAPS", 8., 100., GOLD, 10.);
        say(f, "THE TOOL", 8., 112., GOLD, 10.);
        caption(f, "MANUAL: THE OPERATOR SWAPS THE TOOL BY HAND");
    } else {
        let cc = at(200., 92.);
        let rot = -PI / 4. * smooth(((s - 0.3) / 0.3).clamp(0., 1.));
        disc_at(f, cc, 40., IRON);
        ring(f, cc, 40., IRON_LIGHT, 2.);
        for i in 0..8 {
            let a = -PI / 2. + i as f32 * PI / 4. + rot;
            let pc = pt(cc, 28., a);
            disc_at(f, pc, 7., HOLE);
            let held = match i {
                0 => (s >= 0.3).then_some(GOLD),
                1 => (s < 0.6).then_some(b_col),
                _ => Some(if i % 2 == 0 { FRESH.top } else { METAL.top }),
            };
            if let Some(col) = held {
                rect(f, pc.x - 2., pc.y - 5., pc.x + 2., pc.y + 5., col);
            }
        }
        let p_left = pt(cc, 28., -PI / 2.);
        let (sx, sy) = (70., 73.);
        if s < 0.1 {
            tool_glyph(f, sx, 61., 30., 8., GOLD);
        } else if s < 0.3 {
            let t = smooth((s - 0.1) / 0.2);
            let x = sx + (p_left.x - sx) * t;
            let y = sy + (p_left.y - sy) * t;
            tool_glyph(f, x, y - 12., 30., 8., GOLD);
        }
        if (0.6..0.8).contains(&s) {
            let t = smooth((s - 0.6) / 0.2);
            let x = p_left.x + (sx - p_left.x) * t;
            let y = p_left.y + (sy - p_left.y) * t;
            tool_glyph(f, x, y - 12., 30., 8., b_col);
        } else if s >= 0.8 {
            tool_glyph(f, sx, 61., 30., 8., b_col);
        }
        say(f, "THE CAROUSEL TURNS,", 8., 112., GOLD, 10.);
        say(f, "THE TOOL SWAPS", 8., 124., GOLD, 10.);
        caption(f, "AUTOMATIC TOOL CHANGER: THE CAROUSEL SWAPS IT");
    }
    heading(f, "TOOL CHANGE · MANUAL OR AUTOMATIC");
}

// --------------------------------------------------------------- cutting

/// Surface speed: a point on the cutter's rim, and the recommended band.
fn surface_speed(f: &mut Frame, time: f32, phase: f32) {
    let c = at(66., 76.);
    let r = 32.;
    top_cutter(f, c, r, time * SPIN * 2., 4);
    let a = time * SPIN * 2. + PI / 4.;
    arc(f, c, r + 3., a - 1.2, a, CYAN, 2.5);
    let dot = pt(c, r, a);
    disc_at(f, dot, 3.5, CYAN);
    let tail = at(dot.x + 18. * a.cos(), dot.y + 18. * a.sin());
    arrow(f, dot, tail, CYAN);
    let (tx, ty) = (a.cos(), a.sin());
    // The label sits past the arrowhead, and above the dimension lines.
    say(
        f,
        "V",
        tail.x + 11. * tx - 3.,
        (tail.y + 11. * ty - 5.).min(104.),
        CYAN,
        10.,
    );
    dashed(f, at(c.x - r, 112.), at(c.x - r, 124.), GOLD, 1.);
    dashed(f, at(c.x + r, 112.), at(c.x + r, 124.), GOLD, 1.);
    dimension(f, at(c.x - r, 120.), at(c.x + r, 120.), GOLD);
    say_mid(f, "D", c.x, 123., GOLD, 10.);
    let g = at(204., 82.);
    let (lo, hi) = (0.35, 0.7);
    let frac = track(
        phase,
        &[
            (0., 0.15),
            (0.2, 0.15),
            (0.33, 0.52),
            (0.55, 0.52),
            (0.68, 0.88),
            (0.88, 0.88),
            (1., 0.15),
        ],
    );
    let state = if frac < lo {
        ("TOO SLOW: FLAGGED", RED)
    } else if frac > hi {
        ("TOO FAST: FLAGGED", RED)
    } else {
        ("IN THE BAND: OK", OK)
    };
    gauge(
        f,
        g,
        44.,
        frac,
        &[
            (0., lo, alpha(RED, 0.5)),
            (lo, hi, OK),
            (hi, 1., alpha(RED, 0.5)),
        ],
        state.1,
    );
    let m0 = pt(g, 56., ang(lo));
    let m1 = pt(g, 56., ang(hi));
    say(f, "MIN", m0.x - 14., m0.y - 6., GOLD, 10.);
    say(f, "MAX", m1.x - 2., m1.y - 6., GOLD, 10.);
    say_mid(f, "M/MIN", g.x, g.y + 26., TEXT, 9.);
    say_mid(
        f,
        &format!("{}", (frac * 300.).round() as i32),
        g.x,
        g.y + 8.,
        state.1,
        14.,
    );
    say_mid(f, state.0, g.x, 128., state.1, 9.);
    heading(f, "SURFACE SPEED · π × D × RPM ÷ 1000 (M/MIN)");
    caption(f, "BAND = RECOMMENDED MIN TO MAX · OUTSIDE FLAGGED");
}

/// Chip load: the chip each tooth takes, and the recommended band.
fn chip_load(f: &mut Frame, time: f32, phase: f32) {
    let (c, r) = (at(100., 72.), 28.);
    let (lo, hi) = (6., 11.);
    let v = track(
        phase,
        &[
            (0., 3.5),
            (0.2, 3.5),
            (0.35, 8.5),
            (0.6, 8.5),
            (0.75, 15.5),
            (0.92, 15.5),
            (1., 3.5),
        ],
    );
    // The chip drawn exaggerated so even the thinnest shows.
    let d = 6. + 1.6 * v;
    rect(f, 14., 30., 206., 108., STOCK.top);
    rect(f, 14., c.y - r, c.x, c.y + r, BACKGROUND);
    // The cutter a tooth later: what shows beyond the cutter is the chip.
    disc_at(f, at(c.x + d, c.y), r, GOLD);
    top_cutter(f, c, r, time * SPIN * 2., 4);
    // The chip thickness, measured below the stock on the card's own colour.
    dashed(f, at(c.x, c.y + r), at(c.x, 120.), HOLE, 1.);
    dashed(f, at(c.x + d, c.y + r), at(c.x + d, 120.), HOLE, 1.);
    dashed(f, at(c.x, 108.), at(c.x, 124.), GOLD, 1.);
    dashed(f, at(c.x + d, 108.), at(c.x + d, 124.), GOLD, 1.);
    arrow(f, at(c.x - 14., 120.), at(c.x, 120.), GOLD);
    arrow(f, at(c.x + d + 14., 120.), at(c.x + d, 120.), GOLD);
    say(f, "FZ", c.x + d + 18., 115., GOLD, 10.);
    // The feed: the cutter advances through the slot it has cut.
    arrow(f, at(26., c.y), at(60., c.y), CYAN);
    say(f, "FEED", 26., c.y - 16., CYAN, 9.);
    // The scale: the recommended band and where this chip sits in it.
    let (y0, y1) = (34., 108.);
    let ys = |val: f32| y1 - (y1 - y0) * val / 18.;
    rect(f, 226., y0, 238., y1, alpha(RED, 0.25));
    rect(f, 226., ys(hi), 238., ys(lo), OK);
    outline(f, 226., y0, 238., y1, DIM, 1.);
    let py = ys(v);
    let bad = v < lo || v > hi;
    polygon(
        f,
        &[at(224., py), at(214., py - 5.), at(214., py + 5.)],
        if bad { RED } else { GOLD },
    );
    say(f, "MAX", 242., ys(hi) - 5., GOLD, 8.);
    say(f, "MIN", 242., ys(lo) - 5., GOLD, 8.);
    let (label, col) = if v < lo {
        ("TOO THIN", RED)
    } else if v > hi {
        ("TOO THICK", RED)
    } else {
        ("IN BAND", OK)
    };
    say(f, label, 214., 113., col, 9.);
    heading(f, "CHIP LOAD = FEED ÷ (RPM × FLUTES)");
    caption(f, "GOLD: THE CHIP ONE TOOTH TAKES · BAND: MIN-MAX");
}
