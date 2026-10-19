//! Field pictures: the fields of a manual-lathe Turn operation. All share
//! one side view: the chuck on the left, the bar running right to its free
//! end (Z 0) with the axis as a dash-dot line, and a small grey tool with a
//! gold insert. Amber is the material the cut takes off and it vanishes as
//! the tool passes; the surface it leaves is metal, or bright when fresh.
use crate::illustration::*;
use iced::{
    Color, Point,
    widget::canvas::{Frame, Path, Stroke},
};

/// Where the bar leaves the chuck.
const CX0: f32 = 42.;
/// The free end of the bar: Z 0.
const ZX: f32 = 200.;
/// The axis.
const CY: f32 = 80.;
/// The bar's radius, so its diameter is the full height 2 R.
const R: f32 = 26.;
/// Screen px per mm along Z.
const PX: f32 = 4.;
/// Where the thread ends, short of the bar's end.
const TEND: f32 = ZX - 12.;

const AMBER: Color = STOCK.front;
const GREY: Color = Color::from_rgb(0.62, 0.68, 0.73);
const CHUCK: Color = Color::from_rgb(0.24, 0.29, 0.34);
const JAW: Color = Color::from_rgb(0.40, 0.45, 0.50);
const AXIS: Color = Color::from_rgba(0.85, 0.9, 0.94, 0.6);
/// The turning tool as it leans away from its tip: down and to the right.
const TOOL_DIR: (f32, f32) = (0.45, 0.89);

pub fn turn(f: &mut Frame, _time: f32, phase: f32, n: u8) {
    match n {
        0 => turn_kind(f, phase),
        1 => face_off(f, phase),
        2 => to_diameter(f, phase),
        3 => diameter(f, phase),
        4 => z_end(f, phase),
        5 => groove_z(f, phase),
        6 => depth(f, phase),
        7 => finish(f, phase),
        8 => edge(f, phase),
        9 => size(f, phase),
        10 => width(f, phase),
        11 => thread_side(f, phase),
        12 => pitch_pic(f, phase),
        13 => passes(f, phase),
        _ => heading(f, "TURN"),
    }
}

// ---------------------------------------------------------------- helpers

fn pt(x: f32, y: f32) -> Point {
    Point::new(x, y)
}
fn mid(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
/// The screen x of Z `z` mm (negative toward the chuck).
fn zx(z: f32) -> f32 {
    ZX + z * PX
}
/// Which of `n` equal parts of the loop this is, and how far through it,
/// with `hold` of each part kept at its end.
fn part(phase: f32, n: usize, hold: f32) -> (usize, f32) {
    let x = phase * n as f32;
    let i = (x as usize).min(n - 1);
    (i, progress(x - i as f32, hold))
}
/// How far a plunging blade is in, and how far it has backed out again,
/// over `u` of one cut.
fn plunge(u: f32) -> (f32, f32) {
    (
        smooth((u / 0.5).min(1.)),
        smooth(((u - 0.7) / 0.3).clamp(0., 1.)),
    )
}

fn tw(s: &str, size: f32) -> f32 {
    s.chars().count() as f32 * size * 0.6
}
fn label_c(f: &mut Frame, s: &str, cx: f32, y: f32, color: Color, size: f32) {
    text(f, s, cx - tw(s, size) / 2., y, color, size);
}
fn label_r(f: &mut Frame, s: &str, rx: f32, y: f32, color: Color, size: f32) {
    text(f, s, rx - tw(s, size), y, color, size);
}
/// A row of names under the drawing with the active one in gold.
fn tag_w(s: &str, size: f32) -> f32 {
    s.chars()
        .map(|c| if c == ' ' { 0.3 } else { 0.64 })
        .sum::<f32>()
        * size
}
fn tags(f: &mut Frame, names: &[&str], active: usize, size: f32, gap: f32) {
    let mut x = 8.;
    for (i, s) in names.iter().enumerate() {
        let color = if i == active {
            GOLD
        } else {
            Color { a: 0.5, ..TEXT }
        };
        text(f, s, x, 143., color, size);
        x += tag_w(s, size) + gap;
    }
}

fn outline(f: &mut Frame, pts: &[Point], color: Color) {
    let path = Path::new(|b| {
        b.move_to(pts[0]);
        for a in &pts[1..] {
            b.line_to(*a);
        }
        b.close();
    });
    f.stroke(&path, Stroke::default().with_color(color).with_width(0.8));
}
fn shape(f: &mut Frame, pts: &[Point], fill: Color) {
    polygon(f, pts, fill);
    outline(f, pts, EDGE);
}
fn shape_rect(f: &mut Frame, x0: f32, y0: f32, x1: f32, y1: f32, fill: Color) {
    shape(f, &[pt(x0, y0), pt(x1, y0), pt(x1, y1), pt(x0, y1)], fill);
}
fn dot(f: &mut Frame, c: Point, r: f32, color: Color) {
    f.fill(&Path::circle(c, r), color);
}

/// Bands of the bar from radius `r0` out to `r1`, above and below the axis.
fn band(f: &mut Frame, x0: f32, x1: f32, r0: f32, r1: f32, c: Color) {
    rect(f, x0, CY - r1, x1, CY - r0, c);
    rect(f, x0, CY + r0, x1, CY + r1, c);
}

/// A thin faint dashed extension line.
fn ext(f: &mut Frame, a: Point, b: Point, color: Color) {
    dashed(f, a, b, Color { a: 0.55, ..color }, 1.);
}
fn arrow(f: &mut Frame, tail: Point, tip: Point, color: Color) {
    line(f, tail, tip, color, 1.4);
    let len = (tip.x - tail.x).hypot(tip.y - tail.y).max(1e-3);
    let (ux, uy) = ((tip.x - tail.x) / len, (tip.y - tail.y) / len);
    for side in [-1., 1.] {
        line(
            f,
            tip,
            pt(
                tip.x - 5. * ux + side * 3. * uy,
                tip.y - 5. * uy - side * 3. * ux,
            ),
            color,
            1.4,
        );
    }
}
/// A dimension between `a` and `b`; too short for arrowheads between its
/// ends, it points in at them from outside.
fn span(f: &mut Frame, a: Point, b: Point, color: Color) {
    let len = (b.x - a.x).hypot(b.y - a.y);
    if len >= 13. {
        dimension(f, a, b, color);
    } else {
        span_out(f, a, b, color);
    }
}
/// A dimension whose arrows point in at `a` and `b` from outside.
fn span_out(f: &mut Frame, a: Point, b: Point, color: Color) {
    let len = (b.x - a.x).hypot(b.y - a.y);
    let (ux, uy) = ((b.x - a.x) / len.max(1e-3), (b.y - a.y) / len.max(1e-3));
    arrow(f, pt(a.x - ux * 9., a.y - uy * 9.), a, color);
    arrow(f, pt(b.x + ux * 9., b.y + uy * 9.), b, color);
}

/// The chuck: a dark block with two jaws gripping the bar.
fn chuck(f: &mut Frame) {
    rect(f, 6., CY - R - 16., 30., CY + R + 16., CHUCK);
    rect(f, 30., CY - R - 10., CX0, CY - R, JAW);
    rect(f, 30., CY + R, CX0, CY + R + 10., JAW);
    label_c(f, "CHUCK", 18., CY - 4., TEXT, 7.);
}
/// The axis as a dash-dot line.
fn axis(f: &mut Frame) {
    let end = ZX + 6.;
    let mut x = CX0;
    while x < end {
        line(f, pt(x, CY), pt((x + 12.).min(end), CY), AXIS, 1.);
        x += 15.;
        if x < end {
            line(f, pt(x, CY), pt(x + 1.5, CY), AXIS, 1.);
        }
        x += 4.5;
    }
}
/// The Z 0 mark at the free end.
fn z_zero(f: &mut Frame) {
    dashed(
        f,
        pt(ZX, 36.),
        pt(ZX, CY + R + 6.),
        Color { a: 0.7, ..TEXT },
        1.,
    );
    label_c(f, "Z 0", ZX, 25., TEXT, 9.);
}

/// A turning tool: a gold insert at `tip`, a grey shank leaning away along
/// `dir`, `w` px wide.
fn tool(f: &mut Frame, tip: Point, dir: (f32, f32), w: f32) {
    let len = dir.0.hypot(dir.1);
    let (dx, dy) = (dir.0 / len, dir.1 / len);
    let (nx, ny) = (-dy, dx);
    let at = |s: f32, k: f32| {
        pt(
            tip.x + dx * s + nx * k * w / 2.,
            tip.y + dy * s + ny * k * w / 2.,
        )
    };
    shape(
        f,
        &[at(7., -1.), at(30., -1.), at(30., 1.), at(7., 1.)],
        GREY,
    );
    shape(f, &[tip, at(9., -1.), at(9., 1.)], GOLD);
}
/// A grooving or parting blade `w` px wide, its gold tip at `y`, the body
/// going down.
fn blade(f: &mut Frame, xc: f32, y: f32, w: f32, len: f32) {
    shape_rect(f, xc - w / 2., y + 4., xc + w / 2., y + len, GREY);
    shape_rect(f, xc - w / 2., y, xc + w / 2., y + 4., GOLD);
}
/// A threading tool: a gold V insert whose apex is `tip`, widening `half`
/// each side over `h`, on a grey shank below.
fn vtool(f: &mut Frame, tip: Point, half: f32, h: f32) {
    let base = tip.y + h;
    let end = (base + 16.).min(128.);
    shape(
        f,
        &[
            pt(tip.x - half - 2., base - 1.),
            pt(tip.x + half + 2., base - 1.),
            pt(tip.x + half + 2., end),
            pt(tip.x - half - 2., end),
        ],
        GREY,
    );
    shape(
        f,
        &[tip, pt(tip.x - half, base), pt(tip.x + half, base)],
        GOLD,
    );
}

// ------------------------------------------------------------ side scenes

/// The end being faced: the bar ends `amt` in from Z 0, the slice still to
/// come off is amber out to the tool's radius `rt`.
fn faced(f: &mut Frame, amt: f32, rt: f32, stub: f32) {
    let x = ZX - amt;
    chuck(f);
    rect(f, CX0, CY - R, x, CY + R, METAL.front);
    band(f, x, ZX, stub, rt, AMBER);
    rect(f, x, CY - stub, ZX, CY + stub, METAL.front);
    line(f, pt(x, CY - R), pt(x, CY - rt), FRESH.top, 2.);
    line(f, pt(x, CY + R), pt(x, CY + rt), FRESH.top, 2.);
    axis(f);
}

/// The diameter turned down in steps: `radii[0]` is the bar and each next
/// one a pass further in, ending at the finished radius, from `xe` to the
/// free end. `pass` is the band being cut and `xt` where the tool is.
fn turned(f: &mut Frame, radii: &[f32], colors: &[Color], xe: f32, pass: usize, xt: f32) {
    let n = radii.len() - 1;
    let last = radii[n];
    chuck(f);
    rect(f, CX0, CY - R, xe, CY + R, METAL.front);
    rect(f, xe, CY - last, ZX, CY + last, METAL.front);
    for k in pass..n {
        let x1 = if k == pass { xt.max(xe) } else { ZX };
        band(f, xe, x1, radii[k + 1], radii[k], colors[k]);
    }
    let r = radii[pass + 1];
    for sgn in [-1., 1.] {
        line(
            f,
            pt(xt, CY + sgn * r),
            pt(ZX, CY + sgn * r),
            FRESH.top,
            1.4,
        );
    }
    axis(f);
}

/// A bore opened from radius `r0` to `rb`, `xb` deep, the tool at `xt`.
fn bored(f: &mut Frame, r0: f32, rb: f32, xb: f32, xt: f32) {
    chuck(f);
    rect(f, CX0, CY - R, ZX, CY + R, METAL.front);
    rect(f, xb, CY - r0, ZX, CY + r0, HOLE);
    band(f, xb, xt, r0, rb, AMBER);
    rect(f, xt, CY - rb, ZX, CY + rb, HOLE);
    for sgn in [-1., 1.] {
        line(
            f,
            pt(xt, CY + sgn * rb),
            pt(ZX, CY + sgn * rb),
            FRESH.top,
            1.8,
        );
    }
    axis(f);
}

/// A groove from `x0` to `x1` down to radius `rg`. Each region
/// `(a, b, level)` of it still has amber from `rg` up to `level`.
fn grooved(f: &mut Frame, x0: f32, x1: f32, rg: f32, regions: &[(f32, f32, f32)]) {
    chuck(f);
    rect(f, CX0, CY - R, x0, CY + R, METAL.front);
    rect(f, x1, CY - R, ZX, CY + R, METAL.front);
    rect(f, x0, CY - rg, x1, CY + rg, METAL.front);
    for &(a, b, level) in regions {
        band(f, a, b, rg, level, AMBER);
        if level <= rg + 0.01 {
            for sgn in [-1., 1.] {
                line(
                    f,
                    pt(a, CY + sgn * rg),
                    pt(b, CY + sgn * rg),
                    FRESH.top,
                    1.8,
                );
            }
        }
    }
    let (first, last) = (regions[0], regions[regions.len() - 1]);
    for sgn in [-1., 1.] {
        line(
            f,
            pt(x0, CY + sgn * R),
            pt(x0, CY + sgn * first.2),
            FRESH.top,
            1.8,
        );
        line(
            f,
            pt(x1, CY + sgn * R),
            pt(x1, CY + sgn * last.2),
            FRESH.top,
            1.8,
        );
    }
    axis(f);
}

/// A parting cut at `xp`, `bw` wide, down to radius `rt`; the piece on the
/// free side has moved `shift` further out.
fn parted(f: &mut Frame, xp: f32, bw: f32, rt: f32, shift: f32) {
    chuck(f);
    rect(f, CX0, CY - R, xp, CY + R, METAL.front);
    rect(f, xp + bw + shift, CY - R, ZX + shift, CY + R, METAL.front);
    rect(f, xp, CY - rt, xp + bw, CY + rt, AMBER);
    for sgn in [-1., 1.] {
        line(
            f,
            pt(xp, CY + sgn * R),
            pt(xp, CY + sgn * rt),
            FRESH.top,
            1.8,
        );
        line(
            f,
            pt(xp + bw + shift, CY + sgn * R),
            pt(xp + bw + shift, CY + sgn * rt),
            FRESH.top,
            1.8,
        );
    }
    axis(f);
}

/// A 45 degree cut across a corner. `a` and `b` are where it meets the two
/// faces and the tool has got `t` of the way from `a` to `b`: the part of
/// the corner behind it is gone, the rest is still amber.
fn chamfered(f: &mut Frame, corner: Point, a: Point, b: Point, t: f32) {
    let m = pt(mid(a.x, b.x, t), mid(a.y, b.y, t));
    if t > 0.01 {
        polygon(f, &[a, m, corner], BACKGROUND);
    }
    polygon(f, &[m, b, corner], AMBER);
    line(f, a, m, FRESH.top, 2.);
}

/// The outside corner of the end chamfered `c` px; returns the tool's tip.
fn end_chamfer(f: &mut Frame, c: f32, t: f32) -> Point {
    chuck(f);
    rect(f, CX0, CY - R, ZX, CY + R, METAL.front);
    let mut tip = pt(ZX, CY);
    for sgn in [-1., 1.] {
        let corner = pt(ZX, CY + sgn * R);
        let a = pt(ZX - c, CY + sgn * R);
        let b = pt(ZX, CY + sgn * (R - c));
        chamfered(f, corner, a, b, t);
        tip = pt(mid(a.x, b.x, t), mid(a.y, b.y, t));
    }
    axis(f);
    tip
}
/// The inside corner of a bore of radius `rb` chamfered; returns the tip.
fn bore_chamfer(f: &mut Frame, rb: f32, c: f32, t: f32) -> Point {
    chuck(f);
    rect(f, CX0, CY - R, ZX, CY + R, METAL.front);
    rect(f, ZX - 80., CY - rb, ZX, CY + rb, HOLE);
    let mut tip = pt(ZX, CY);
    for sgn in [-1., 1.] {
        let corner = pt(ZX, CY + sgn * rb);
        let a = pt(ZX - c, CY + sgn * rb);
        let b = pt(ZX, CY + sgn * (rb + c));
        chamfered(f, corner, a, b, t);
        tip = pt(mid(a.x, b.x, t), mid(a.y, b.y, t));
    }
    axis(f);
    tip
}
/// The outer corner of a shoulder at `xs` chamfered; returns the tip.
fn shoulder_chamfer(f: &mut Frame, xs: f32, rs: f32, c: f32, t: f32) -> Point {
    chuck(f);
    rect(f, CX0, CY - R, xs, CY + R, METAL.front);
    rect(f, xs, CY - rs, ZX, CY + rs, METAL.front);
    let mut tip = pt(xs, CY);
    for sgn in [-1., 1.] {
        let corner = pt(xs, CY + sgn * R);
        let a = pt(xs - c, CY + sgn * R);
        let b = pt(xs, CY + sgn * (R - c));
        chamfered(f, corner, a, b, t);
        tip = pt(mid(a.x, b.x, t), mid(a.y, b.y, t));
    }
    axis(f);
    tip
}

// ---------------------------------------------------------------- threads

/// How deep a sharp V thread is at `x`: 0 on a crest, `h` at a root.
/// Roots lie at ZX, ZX - pitch, ...
fn vdepth(x: f32, pitch: f32, h: f32) -> f32 {
    let t = ((TEND - x) / pitch).rem_euclid(1.);
    h * if t < 0.5 { 1. - 2. * t } else { 2. * t - 1. }
}
fn thread_h(pitch: f32) -> f32 {
    0.8 * pitch
}
/// Where the thread begins (toward the chuck): a whole number of pitches.
fn thread_start(pitch: f32) -> f32 {
    TEND - (60. / pitch).round() * pitch
}
/// `x0`, every crest and root between, and `x1`.
fn breaks(x0: f32, x1: f32, pitch: f32) -> Vec<f32> {
    let half = pitch / 2.;
    let mut k = (((TEND - x1) / half).ceil() as i32).max(0);
    let mut mids = vec![];
    while TEND - k as f32 * half > x0 + 0.01 {
        let x = TEND - k as f32 * half;
        if x < x1 - 0.01 {
            mids.push(x);
        }
        k += 1;
    }
    mids.reverse();
    let mut xs = vec![x0];
    xs.extend(mids);
    xs.push(x1);
    xs
}
/// The area between the plain radius and the thread's radius `rz(x)` over
/// `x0..x1`, above and below the axis.
fn region(
    f: &mut Frame,
    x0: f32,
    x1: f32,
    pitch: f32,
    plain: f32,
    rz: &dyn Fn(f32) -> f32,
    color: Color,
) {
    if x1 - x0 < 0.5 {
        return;
    }
    let xs = breaks(x0, x1, pitch);
    for upper in [true, false] {
        let y = |r: f32| if upper { CY - r } else { CY + r };
        let mut pts = vec![pt(x0, y(plain))];
        pts.extend(xs.iter().map(|&x| pt(x, y(rz(x)))));
        pts.push(pt(x1, y(plain)));
        polygon(f, &pts, color);
    }
}
/// The thread's profile as a line over `x0..x1`.
fn zigzag(f: &mut Frame, x0: f32, x1: f32, pitch: f32, rz: &dyn Fn(f32) -> f32, color: Color) {
    if x1 - x0 < 0.5 {
        return;
    }
    let xs = breaks(x0, x1, pitch);
    for sgn in [-1., 1.] {
        for w in xs.windows(2) {
            line(
                f,
                pt(w[0], CY + sgn * rz(w[0])),
                pt(w[1], CY + sgn * rz(w[1])),
                color,
                1.8,
            );
        }
    }
}
/// A thread on the outside of the bar with the tool at `xt`; returns the
/// tool's tip.
fn thread_out(f: &mut Frame, pitch: f32, xt: f32) -> Point {
    let h = thread_h(pitch);
    let xe = thread_start(pitch);
    chuck(f);
    rect(f, CX0, CY - R, xe, CY + R, METAL.front);
    rect(f, TEND, CY - R, ZX, CY + R, METAL.front);
    rect(f, xe, CY - (R - h), TEND, CY + (R - h), METAL.front);
    let rz = |x: f32| R - vdepth(x, pitch, h);
    region(f, xe, TEND, pitch, R - h, &rz, METAL.front);
    region(f, xe, xt, pitch, R, &rz, AMBER);
    zigzag(f, xt.max(xe), TEND, pitch, &rz, FRESH.top);
    axis(f);
    pt(xt, CY + rz(xt))
}
/// A thread inside a bore (in section) with the tool at `xt`; returns the
/// tool's tip, on the upper wall.
fn thread_in(f: &mut Frame, pitch: f32, xt: f32) -> Point {
    let h = thread_h(pitch);
    let (rb, xb) = (14., ZX - 84.);
    let xe = thread_start(pitch);
    chuck(f);
    rect(f, CX0, CY - R, ZX, CY + R, METAL.front);
    rect(f, xb, CY - rb, ZX, CY + rb, HOLE);
    let rz = |x: f32| rb + vdepth(x, pitch, h);
    region(f, xe, xt, pitch, rb, &rz, AMBER);
    region(f, xt, TEND, pitch, rb, &rz, HOLE);
    zigzag(f, xt.max(xe), TEND, pitch, &rz, FRESH.top);
    axis(f);
    pt(xt, CY - rz(xt))
}

// ------------------------------------------------------------------ fields

const KINDS: [&str; 7] = [
    "FACE", "OUTSIDE", "BORE", "CHAMFER", "GROOVE", "PART OFF", "THREAD",
];

/// kind: seven cuts, each a few tenths of a second long.
fn turn_kind(f: &mut Frame, phase: f32) {
    let (k, u) = part(phase, 7, 0.2);
    let s = smooth(u);
    match k {
        0 => {
            let rt = R * (1. - s);
            faced(f, 12., rt, 0.);
            z_zero(f);
            tool(f, pt(ZX - 12., CY + rt), (0.8, 0.6), 9.);
        }
        1 => {
            let xe = zx(-24.);
            let xt = ZX - (ZX - xe) * s;
            turned(f, &[R, 14.], &[AMBER], xe, 0, xt);
            z_zero(f);
            tool(f, pt(xt, CY + 14.), TOOL_DIR, 9.);
        }
        2 => {
            let xb = zx(-20.);
            let xt = ZX - (ZX - xb) * s;
            bored(f, 8., 16., xb, xt);
            z_zero(f);
            tool(f, pt(xt, CY - 16.), (0.8, 0.6), 9.);
        }
        3 => {
            let tip = end_chamfer(f, 14., s);
            z_zero(f);
            tool(f, tip, (0.7, 0.7), 9.);
        }
        4 => {
            let (x0, gw, rg) = (zx(-14.), 14., 14.);
            let (pl, re) = plunge(u);
            let level = mid(R, rg, pl);
            grooved(f, x0, x0 + gw, rg, &[(x0, x0 + gw, level)]);
            z_zero(f);
            blade(f, x0 + gw / 2., CY + level + 1.5 + 16. * re, gw, 16.);
        }
        5 => {
            let (xp, bw) = (zx(-16.), 6.);
            let cut = smooth((u / 0.65).min(1.));
            let shift = smooth(((u - 0.7) / 0.3).clamp(0., 1.)) * 14.;
            let rt = R * (1. - cut);
            parted(f, xp, bw, rt, shift);
            z_zero(f);
            blade(f, xp + bw / 2., CY + rt, bw, 22.);
        }
        _ => {
            let pitch = 8.;
            let xt = TEND - (TEND - thread_start(pitch)) * s;
            let tip = thread_out(f, pitch, xt);
            z_zero(f);
            vtool(f, tip, pitch / 2., thread_h(pitch));
        }
    }
    let names = [
        "FACE THE END",
        "OUTSIDE DIAMETER",
        "BORE",
        "CHAMFER AN EDGE",
        "GROOVE",
        "PART OFF",
        "THREAD",
    ];
    heading(f, &format!("TURN KIND · {}", names[k]));
    tags(f, &KINDS, k, 7.5, 9.);
}

/// face_off: how much comes off the end.
fn face_off(f: &mut Frame, phase: f32) {
    let (i, u) = part(phase, 2, 0.3);
    let amt = [10., 20.][i];
    let rt = R * (1. - smooth(u));
    faced(f, amt, rt, 0.);
    z_zero(f);
    let y = 46.;
    ext(f, pt(ZX - amt, y), pt(ZX - amt, CY - R), GOLD);
    span(f, pt(ZX - amt, y), pt(ZX, y), GOLD);
    let lx = ZX - amt - if amt < 18. { 14. } else { 8. };
    label_r(
        f,
        &format!("FACE OFF {} MM", amt / PX),
        lx,
        y - 4.5,
        GOLD,
        9.,
    );
    tool(f, pt(ZX - amt, CY + rt), (0.8, 0.6), 9.);
    heading(f, "FACE OFF · HOW MUCH COMES OFF THE END");
    caption(f, "AMBER: SLICE FACED OFF · BRIGHT: THE NEW FACE");
}

/// to_diameter: where the facing stroke stops.
fn to_diameter(f: &mut Frame, phase: f32) {
    let (i, u) = part(phase, 2, 0.3);
    let stub = [12., 0.][i];
    let amt = 12.;
    let rt = stub + (R - stub) * (1. - smooth(u));
    faced(f, amt, rt, stub);
    z_zero(f);
    let x = ZX + 14.;
    if stub > 0. {
        for sgn in [-1., 1.] {
            ext(
                f,
                pt(ZX + 4., CY + sgn * stub),
                pt(x + 4., CY + sgn * stub),
                GOLD,
            );
        }
        span(f, pt(x, CY - stub), pt(x, CY + stub), GOLD);
        text(
            f,
            &format!("Ø {}", stub * 2. / PX),
            x + 8.,
            CY - 6.,
            GOLD,
            11.,
        );
        text(f, "STUB", ZX + 8., 50., TEXT, 8.);
        line(
            f,
            pt(ZX + 10., 60.),
            pt(ZX - 2., CY - stub + 1.),
            Color { a: 0.8, ..TEXT },
            1.,
        );
    } else {
        arrow(f, pt(x, CY - 12.), pt(x, CY - 1.), GOLD);
        arrow(f, pt(x, CY + 12.), pt(x, CY + 1.), GOLD);
        text(f, "Ø 0", x + 8., CY - 13., GOLD, 11.);
        text(f, "TO CENTRE", x + 8., CY + 2., TEXT, 8.);
    }
    tool(f, pt(ZX - amt, CY + rt), (0.8, 0.6), 9.);
    heading(f, "TO Ø · WHERE THE FACING CUT STOPS");
    caption(f, "Ø > 0 LEAVES A STUB · Ø 0 FACES TO CENTRE");
}

/// diameter: the finished diameter across the whole bar.
fn diameter(f: &mut Frame, phase: f32) {
    let (i, u) = part(phase, 2, 0.45);
    let (r0, rd) = [(R, 16.), (16., 10.)][i];
    let xe = zx(-24.);
    let xt = ZX - (ZX - xe) * smooth(u);
    turned(f, &[r0, rd], &[AMBER], xe, 0, xt);
    z_zero(f);
    // Only once the end is turned to this diameter is it dimensioned.
    if u >= 1. {
        let x = ZX + 14.;
        for sgn in [-1., 1.] {
            ext(
                f,
                pt(ZX + 4., CY + sgn * rd),
                pt(x + 4., CY + sgn * rd),
                GOLD,
            );
        }
        span(f, pt(x, CY - rd), pt(x, CY + rd), GOLD);
        text(
            f,
            &format!("Ø {} MM", rd * 2. / PX),
            x + 8.,
            CY - 13.,
            GOLD,
            10.,
        );
        text(f, "NOT RADIUS", x + 8., CY + 2., TEXT, 8.);
    }
    tool(f, pt(xt, CY + rd), TOOL_DIR, 9.);
    heading(f, "DIAMETER · THE FINISHED Ø, NOT RADIUS");
    caption(f, "FULL HEIGHT ACROSS THE AXIS · AMBER: TAKEN OFF");
}

/// z_end: where the cut stops along the bar.
fn z_end(f: &mut Frame, phase: f32) {
    let (i, u) = part(phase, 2, 0.3);
    let z = [-18., -28.][i];
    let xe = zx(z);
    let rd = 14.;
    let xt = ZX - (ZX - xe) * smooth(u);
    turned(f, &[R, rd], &[AMBER], xe, 0, xt);
    z_zero(f);
    for sgn in [-1., 1.] {
        line(f, pt(xe, CY + sgn * R), pt(xe, CY + sgn * rd), GOLD, 2.2);
    }
    let y = 46.;
    ext(f, pt(xe, y), pt(xe, CY - R), GOLD);
    ext(f, pt(ZX, y), pt(ZX, CY - R), GOLD);
    span(f, pt(xe, y), pt(ZX, y), GOLD);
    label_r(f, &format!("Z {z}"), xe - 8., y - 4.5, GOLD, 9.);
    tool(f, pt(xt, CY + rd), TOOL_DIR, 9.);
    heading(f, "Z END · WHERE THE CUT STOPS ALONG THE BAR");
    caption(f, "TURNS FROM Z 0 TOWARD THE CHUCK, STOPS AT Z END");
}

/// z: where a groove's chuck-side wall sits.
fn groove_z(f: &mut Frame, phase: f32) {
    let (i, l) = part(phase, 2, 0.0);
    let z = [-12., -28.][i];
    let (x0, gw, rg) = (zx(z), 16., 14.);
    let (pl, re) = plunge(l / 0.85);
    let level = mid(R, rg, pl);
    grooved(f, x0, x0 + gw, rg, &[(x0, x0 + gw, level)]);
    z_zero(f);
    for sgn in [-1., 1.] {
        line(f, pt(x0, CY + sgn * R), pt(x0, CY + sgn * rg), GOLD, 2.2);
    }
    let y = 46.;
    ext(f, pt(x0, y), pt(x0, CY - R), GOLD);
    ext(f, pt(ZX, y), pt(ZX, CY - R), GOLD);
    span(f, pt(x0, y), pt(ZX, y), GOLD);
    label_r(f, &format!("Z {z}"), x0 - 8., y - 4.5, GOLD, 9.);
    blade(f, x0 + gw / 2., CY + level + 1.5 + 16. * re, gw, 16.);
    text(f, "SECTION · SAME GROOVE ALL ROUND", 8., 17., TEXT, 7.5);
    heading(f, "Z · WHERE THE GROOVE SITS ALONG THE BAR");
    caption(f, "GOLD WALL: THE GROOVE SIDE TOWARD THE CHUCK");
}

/// The shared look of the pictures about passes: the bar turned down in
/// bands, a ladder of pass numbers and one pass's band dimensioned at the
/// free end. Returns the pass being cut.
fn stepped(
    f: &mut Frame,
    phase: f32,
    radii: &[f32],
    colors: &[Color],
    marks: &[&str],
    dim: (usize, &str),
) -> usize {
    let n = radii.len() - 1;
    let xe = zx(-22.);
    let x = progress(phase, 0.12) * n as f32;
    let k = (x as usize).min(n - 1);
    let l = x - k as f32;
    let cp = smooth((l / 0.75).min(1.));
    let xt = ZX - (ZX - xe) * cp;
    let lift = smooth(((l - 0.85) / 0.15).clamp(0., 1.)) * 8.;
    turned(f, radii, colors, xe, k, xt);
    z_zero(f);
    for (j, m) in marks.iter().enumerate() {
        let ym = CY - (radii[j] + radii[j + 1]) / 2.;
        let color = if j == k {
            GOLD
        } else if j < k {
            TEXT
        } else {
            Color { a: 0.55, ..TEXT }
        };
        let ly = CY - 34. + 11. * j as f32;
        line(
            f,
            pt(ZX + 2., ym),
            pt(ZX + 9., ly),
            Color { a: 0.6, ..color },
            1.,
        );
        text(f, m, ZX + 11., ly - 5.5, color, 10.);
    }
    let (b, name) = dim;
    let dx = ZX + 30.;
    for sgn in [-1., 1.] {
        for r in [radii[b], radii[b + 1]] {
            ext(
                f,
                pt(ZX + 18., CY + sgn * r),
                pt(dx + 4., CY + sgn * r),
                GOLD,
            );
        }
        span(
            f,
            pt(dx, CY + sgn * radii[b]),
            pt(dx, CY + sgn * radii[b + 1]),
            GOLD,
        );
    }
    text(f, name, dx + 8., CY - 11., GOLD, 9.);
    text(f, "ON Ø", dx + 8., CY + 1., TEXT, 9.);
    tool(f, pt(xt, CY + radii[k + 1] + lift), TOOL_DIR, 9.);
    k
}

/// depth: equal passes step in.
fn depth(f: &mut Frame, phase: f32) {
    let k = stepped(
        f,
        phase,
        &[R, 19., 12., 5.],
        &[AMBER, AMBER, AMBER],
        &["1", "2", "3"],
        (0, "DEPTH"),
    );
    heading(f, "DEPTH · HOW MUCH ONE PASS TAKES OFF Ø");
    caption(
        f,
        &format!("PASS {} OF 3 · SAME DEPTH OFF THE Ø EACH TIME", k + 1),
    );
}

/// finish: roughing passes, then one thin final pass.
fn finish(f: &mut Frame, phase: f32) {
    stepped(
        f,
        phase,
        &[R, 19., 12., 7.],
        &[AMBER, AMBER, GOLD],
        &["1", "2", "F"],
        (2, "FINISH"),
    );
    heading(f, "FINISH · WHAT THE LAST PASS TAKES OFF");
    caption(f, "AMBER: ROUGHING · GOLD: FINISH · BRIGHT: CLEAN");
}

/// edge: which edge gets the 45 degree chamfer.
fn edge(f: &mut Frame, phase: f32) {
    let (i, u) = part(phase, 3, 0.3);
    let t = smooth(u);
    let c = 12.;
    let xs = ZX - 70.;
    let (tip, dir) = match i {
        0 => (end_chamfer(f, c, t), (0.7, 0.7)),
        1 => (bore_chamfer(f, 14., c, t), (0.7, -0.7)),
        _ => (shoulder_chamfer(f, xs, 16., c, t), (0.7, 0.7)),
    };
    z_zero(f);
    // A leader from the upper chamfer to its angle.
    let (a, b, out) = match i {
        0 => (pt(ZX - c, CY - R), pt(ZX, CY - R + c), (1., -1.)),
        1 => (pt(ZX - c, CY - 14.), pt(ZX, CY - 14. - c), (1., 1.)),
        _ => (pt(xs - c, CY - R), pt(xs, CY - R + c), (1., -1.)),
    };
    let m = pt((a.x + b.x) / 2., (a.y + b.y) / 2.);
    let e = pt(m.x + out.0 * 10., m.y + out.1 * 10.);
    line(f, m, e, GOLD, 1.2);
    let ty = if out.1 < 0. { e.y - 10. } else { e.y - 4. };
    text(f, "45°", e.x + 3., ty, GOLD, 9.);
    tool(f, tip, dir, 9.);
    heading(f, "EDGE · WHICH EDGE GETS THE 45° CHAMFER");
    tags(
        f,
        &["END CORNER (OUTSIDE)", "BORE CORNER (INSIDE)", "SHOULDER"],
        i,
        7.5,
        9.,
    );
}

/// size: the leg of the chamfer, growing.
fn size(f: &mut Frame, phase: f32) {
    // A magnified view of the end's upper corner, 8 px per mm.
    let c = (2. + 3. * smooth(progress(phase, 0.2))) * 8.;
    let (fx, top) = (190., 54.);
    rect(f, 20., top, fx, 106., METAL.front);
    for k in 0..22 {
        let x = 20. + 8. * k as f32;
        let (y0, y1) = if k % 2 == 0 {
            (106., 109.)
        } else {
            (109., 106.)
        };
        line(f, pt(x, y0), pt(x + 8., y1), EDGE, 1.2);
    }
    let corner = pt(fx, top);
    let a = pt(fx - c, top);
    let b = pt(fx, top + c);
    polygon(f, &[a, b, corner], AMBER);
    line(f, a, b, FRESH.top, 2.4);
    // The leg along the bar and the leg across the end, both at this corner.
    let y = 38.;
    ext(f, pt(a.x, y), pt(a.x, top), GOLD);
    ext(f, pt(fx, y), pt(fx, top), GOLD);
    span(f, pt(a.x, y), pt(fx, y), GOLD);
    label_r(
        f,
        &format!("SIZE {:.1} MM", c / 8.),
        a.x - 8.,
        y - 4.5,
        GOLD,
        9.,
    );
    let x = fx + 14.;
    ext(f, pt(fx, top), pt(x + 4., top), GOLD);
    ext(f, pt(fx, b.y), pt(x + 4., b.y), GOLD);
    span(f, pt(x, top), pt(x, b.y), GOLD);
    text(f, "= SIZE", x + 8., top + c / 2. - 4.5, GOLD, 9.);
    tool(
        f,
        pt(mid(a.x, b.x, 0.5), mid(a.y, b.y, 0.5)),
        (0.7, -0.7),
        9.,
    );
    text(f, "MAGNIFIED: END CORNER OF THE BAR", 20., 118., TEXT, 8.);
    heading(f, "SIZE · LENGTH OF THE 45° CHAMFER'S LEG");
    caption(f, "AMBER: CORNER REMOVED · BRIGHT: THE CHAMFER");
}

/// width: the blade plunges, steps over and plunges again.
fn width(f: &mut Frame, phase: f32) {
    let (rg, bw, shift) = (14., 18., 12.);
    let xw = zx(-20.);
    let wide = shift + bw;
    let out = R + 8.;
    let (xa, xb) = (xw + bw / 2., xw + shift + bw / 2.);
    let p = phase;
    let (xc, r) = if p < 0.26 {
        (xa, mid(R, rg, smooth(p / 0.26)))
    } else if p < 0.34 {
        (xa, mid(rg, out, smooth((p - 0.26) / 0.08)))
    } else if p < 0.42 {
        (mid(xa, xb, smooth((p - 0.34) / 0.08)), out)
    } else if p < 0.68 {
        (xb, mid(out, rg, smooth((p - 0.42) / 0.26)))
    } else if p < 0.78 {
        (xb, rg)
    } else if p < 0.88 {
        (xb, mid(rg, out, smooth((p - 0.78) / 0.10)))
    } else {
        (xb, out)
    };
    let level_a = if p < 0.26 { r } else { rg };
    let level_b = if p < 0.42 {
        R
    } else if p < 0.68 {
        r.min(R)
    } else {
        rg
    };
    grooved(
        f,
        xw,
        xw + wide,
        rg,
        &[(xw, xw + bw, level_a), (xw + bw, xw + wide, level_b)],
    );
    // The groove's width, and the tool's.
    let y = 46.;
    ext(f, pt(xw, y), pt(xw, CY - R), GOLD);
    ext(f, pt(xw + wide, y), pt(xw + wide, CY - R), GOLD);
    span(f, pt(xw, y), pt(xw + wide, y), GOLD);
    text(f, "WIDTH", xw + wide + 8., y - 4.5, GOLD, 9.);
    let y2 = 33.;
    ext(f, pt(xw + bw, y2), pt(xw + bw, y), CYAN);
    ext(f, pt(xw, y2), pt(xw, y), CYAN);
    span(f, pt(xw, y2), pt(xw + bw, y2), CYAN);
    label_r(f, "TOOL", xw - 14., y2 - 4.5, CYAN, 9.);
    blade(f, xc, CY + r + 1.5, bw, 16.);
    text(f, "SECTION · SAME GROOVE ALL ROUND", 8., 17., TEXT, 7.5);
    heading(f, "WIDTH · AT LEAST AS WIDE AS THE TOOL");
    caption(f, "A SECOND PLUNGE WIDENS THE GROOVE PAST THE TOOL");
}

/// thread_side: outside the bar, then inside a bore.
fn thread_side(f: &mut Frame, phase: f32) {
    let (i, u) = part(phase, 2, 0.25);
    let pitch = 8.;
    let xt = TEND - (TEND - thread_start(pitch)) * smooth(u);
    let tip = if i == 0 {
        thread_out(f, pitch, xt)
    } else {
        thread_in(f, pitch, xt)
    };
    z_zero(f);
    vtool(f, tip, pitch / 2., thread_h(pitch));
    let (big, note) = [("OUTSIDE", "ON THE BAR"), ("INSIDE", "IN A BORE (SECTION)")][i];
    text(f, big, 8., 19., GOLD, 12.);
    text(f, note, 12. + tw(big, 12.), 22., TEXT, 8.);
    heading(f, "THREAD SIDE · ON THE BAR OR IN A BORE");
    tags(f, &["OUTSIDE", "INSIDE"], i, 8., 9.);
}

/// pitch: the distance between crests, fine then coarse.
fn pitch_pic(f: &mut Frame, phase: f32) {
    let (i, u) = part(phase, 2, 0.25);
    let pitch = [8., 16.][i];
    let xt = TEND - (TEND - thread_start(pitch)) * smooth(u);
    let tip = thread_out(f, pitch, xt);
    let (xa, xb) = (TEND - 1.5 * pitch, TEND - 0.5 * pitch);
    let y = 38.;
    ext(f, pt(xa, y), pt(xa, CY - R), GOLD);
    ext(f, pt(xb, y), pt(xb, CY - R), GOLD);
    span_out(f, pt(xa, y), pt(xb, y), GOLD);
    dot(f, pt(xa, CY - R), 2.2, GOLD);
    dot(f, pt(xb, CY - R), 2.2, GOLD);
    label_r(
        f,
        &format!("PITCH {} MM", pitch / PX),
        xa - 14.,
        y - 4.5,
        GOLD,
        9.,
    );
    vtool(f, tip, pitch / 2., thread_h(pitch));
    heading(f, "PITCH · DISTANCE BETWEEN THREAD CRESTS");
    caption(f, "TOOL LEAVES A ZIGZAG · AMBER: GROOVES CUT AWAY");
}

/// passes: a magnified thread groove cut in several passes, each lighter.
fn passes(f: &mut Frame, phase: f32) {
    const N: usize = 4;
    let (cx, ys, dmax, t30) = (139., 106., 64., 0.577);
    let d = |k: usize| dmax * (k as f32 / N as f32).sqrt();
    let x = progress(phase, 0.1) * N as f32;
    let k = (x as usize).min(N - 1);
    let l = x - k as f32;
    let cp = smooth((l / 0.8).min(1.));
    let dn = mid(d(k), d(k + 1), cp);
    let lift = if k == N - 1 {
        smooth(((l - 0.85) / 0.15).clamp(0., 1.)) * 40.
    } else {
        0.
    };
    let tri = |depth: f32| {
        [
            pt(cx - depth * t30, ys),
            pt(cx + depth * t30, ys),
            pt(cx, ys - depth),
        ]
    };
    rect(f, 8., 32., 270., ys, METAL.front);
    // Every pass's band, the deepest drawn first.
    for j in (1..=N).rev() {
        let c = if j % 2 == 0 { AMBER } else { STOCK.top };
        polygon(f, &tri(d(j)), c);
    }
    if dn > 0.5 {
        polygon(f, &tri(dn), BACKGROUND);
    }
    let flanks = |f: &mut Frame, depth: f32, color: Color, w: f32| {
        let t = tri(depth);
        line(f, t[0], t[2], color, w);
        line(f, t[1], t[2], color, w);
    };
    for j in 1..=N {
        let color = if j <= k {
            Color { a: 0.7, ..TEXT }
        } else {
            Color::from_rgba(0.1, 0.15, 0.2, 0.7)
        };
        flanks(f, d(j), color, 1.);
    }
    if dn > 0.5 {
        flanks(f, dn, FRESH.top, 2.);
    }
    line(f, pt(8., ys), pt(270., ys), EDGE, 1.);
    // Each band's number, led in from the left to where the band lies.
    for j in 1..=N {
        let h = (d(j - 1) + d(j)) / 2.;
        let hw = (d(j) - h) * t30;
        let ly = [ys - 14., ys - 33., ys - 50., ys - 66.][j - 1];
        let color = if j == k + 1 {
            GOLD
        } else if j <= k {
            TEXT
        } else {
            Color { a: 0.6, ..TEXT }
        };
        line(
            f,
            pt(cx - 62., ly),
            pt(cx - hw, ys - h),
            Color { a: 0.55, ..color },
            1.,
        );
        text(f, &j.to_string(), cx - 72., ly - 5.5, color, 11.);
    }
    // The current pass's infeed beside the apex.
    let (y1, y2) = (ys - d(k), ys - d(k + 1));
    let dx = cx + 26.;
    ext(f, pt(cx, y1), pt(dx + 4., y1), GOLD);
    ext(f, pt(cx, y2), pt(dx + 4., y2), GOLD);
    span(f, pt(dx, y1), pt(dx, y2), GOLD);
    text(f, "INFEED", dx + 8., (y1 + y2) / 2. - 4.5, GOLD, 9.);
    vtool(f, pt(cx, ys - dn + lift), 18. * t30, 18.);
    text(f, &format!("PASS {} OF {N}", k + 1), 8., 17., GOLD, 12.);
    heading(f, "PASSES · EACH PASS CUTS LESS THAN THE LAST");
    caption(f, "MAGNIFIED · A BAND PER PASS, EACH THINNER");
}
