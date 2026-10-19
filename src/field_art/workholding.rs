//! Field pictures: workholding (the vise and the strap clamps) and stock.
//! Jaws and clamps are drawn in the 3D view as the boxes they are kept as;
//! the stock pictures show a translucent amber box or bar round a blue part.
use crate::illustration::*;
use iced::{
    Color, Point,
    widget::canvas::{Frame, Path, Stroke},
};
use std::f32::consts::{PI, TAU};

const RED: Color = Color::from_rgb(0.95, 0.36, 0.3);
/// The part, in blue.
const PART: Tone = Tone {
    top: Color::from_rgb(0.42, 0.70, 0.98),
    front: Color::from_rgb(0.28, 0.55, 0.88),
    side: Color::from_rgb(0.22, 0.46, 0.78),
};
const GHOST: Color = Color::from_rgba(0.725, 0.81, 0.875, 0.45);
/// The vise body and the machine table: darker than the jaws.
const BODY: Tone = Tone {
    top: Color::from_rgb(0.30, 0.38, 0.45),
    front: Color::from_rgb(0.23, 0.31, 0.38),
    side: Color::from_rgb(0.19, 0.26, 0.33),
};
const JAW: Tone = Tone {
    top: Color::from_rgb(0.66, 0.70, 0.74),
    front: Color::from_rgb(0.54, 0.58, 0.63),
    side: Color::from_rgb(0.46, 0.50, 0.55),
};
const JAW_MOVING: Tone = Tone {
    top: Color::from_rgb(0.76, 0.80, 0.83),
    front: Color::from_rgb(0.64, 0.68, 0.72),
    side: Color::from_rgb(0.56, 0.60, 0.64),
};

fn wave(phase: f32) -> f32 {
    0.5 - 0.5 * (phase * TAU).cos()
}
fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}
fn tone_a(t: Tone, a: f32) -> Tone {
    Tone {
        top: alpha(t.top, a),
        front: alpha(t.front, a),
        side: alpha(t.side, a),
    }
}
fn arrow(f: &mut Frame, a: Point, b: Point, color: Color) {
    line(f, a, b, color, 1.8);
    let len = (b.x - a.x).hypot(b.y - a.y).max(1e-3);
    let (ux, uy) = ((b.x - a.x) / len, (b.y - a.y) / len);
    for side in [-1., 1.] {
        line(
            f,
            b,
            Point::new(
                b.x - 6. * ux + side * 3.5 * uy,
                b.y - 6. * uy - side * 3.5 * ux,
            ),
            color,
            1.8,
        );
    }
}
/// A faint dashed extension line from a part to its dimension.
fn ext(f: &mut Frame, a: V, b: V, color: Color) {
    dashed(f, p(a), p(b), alpha(color, 0.65), 1.);
}
/// A dimension that drops its arrowheads when too short to hold them.
fn dim3(f: &mut Frame, a: V, b: V, color: Color) {
    let (pa, pb) = (p(a), p(b));
    if (pb.x - pa.x).hypot(pb.y - pa.y) > 11. {
        dimension(f, pa, pb, color);
    } else {
        line(f, pa, pb, color, 2.);
    }
}
/// A dashed box: the boxes a move is checked against.
fn dbox(f: &mut Frame, lo: V, hi: V, color: Color) {
    let c = |i: usize| {
        [
            if i & 1 == 0 { lo[0] } else { hi[0] },
            if i & 2 == 0 { lo[1] } else { hi[1] },
            if i & 4 == 0 { lo[2] } else { hi[2] },
        ]
    };
    for i in 0..8 {
        for bit in [1, 2, 4] {
            if i & bit == 0 {
                dashed(f, p(c(i)), p(c(i | bit)), color, 1.2);
            }
        }
    }
}
fn grown(lo: V, hi: V, g: f32) -> (V, V) {
    (
        [lo[0] - g, lo[1] - g, lo[2] - g],
        [hi[0] + g, hi[1] + g, hi[2] + g],
    )
}
/// A row of names at the top, the active one in gold.
fn tabs(f: &mut Frame, names: &[&str], active: usize) {
    let mut x = 6.;
    for (i, n) in names.iter().enumerate() {
        let color = if i == active { GOLD } else { GHOST };
        text(f, n, x, 16., color, 9.);
        x += n.chars().count() as f32 * 6.2 + 9.;
    }
}
/// A small X and Y marker at the left: X runs right-down, Y left-down, the
/// active one in gold.
fn marker(f: &mut Frame, active: usize) {
    let o = Point::new(46., 48.);
    let x = Point::new(o.x + 21., o.y + 8.);
    let y = Point::new(o.x - 21., o.y + 8.);
    line(f, o, x, if active == 0 { GOLD } else { TEXT }, 2.);
    line(f, o, y, if active == 1 { GOLD } else { TEXT }, 2.);
    text(
        f,
        "X",
        x.x + 3.,
        x.y - 5.,
        if active == 0 { GOLD } else { TEXT },
        9.,
    );
    text(
        f,
        "Y",
        y.x - 9.,
        y.y - 5.,
        if active == 1 { GOLD } else { TEXT },
        9.,
    );
}

// ---------------------------------------------------------------- the vise

/// Top of the vise body, and of the parallels (the stock's bottom).
const FLOOR: f32 = 6.;
const BASE: f32 = 9.;

/// A machine vise. `a` runs across the jaws (X for `axis` 0, Y for 1) and
/// `b` along them; the fixed jaw is on the −a side, the moving one on +a.
#[derive(Clone, Copy)]
struct Vise {
    axis: usize,
    fixed: f32,
    moving: f32,
    stock: (f32, f32),
    sb: f32,
    sh: f32,
    lift: f32,
    len: f32,
    off: f32,
    thick: f32,
    top: f32,
    stock_tone: Tone,
    stock_last: bool,
}

impl Vise {
    fn new() -> Self {
        Vise {
            axis: 0,
            fixed: -14.,
            moving: 14.,
            stock: (-14., 14.),
            sb: 14.,
            sh: 16.,
            lift: 0.,
            len: 18.,
            off: 0.,
            thick: 8.,
            top: BASE + 10.,
            stock_tone: STOCK,
            stock_last: false,
        }
    }
    fn at(&self, a: f32, b: f32, z: f32) -> V {
        if self.axis == 0 { [a, b, z] } else { [b, a, z] }
    }
    fn bx(&self, a0: f32, a1: f32, b0: f32, b1: f32, z0: f32, z1: f32) -> (V, V) {
        (self.at(a0, b0, z0), self.at(a1, b1, z1))
    }
    fn jaw_box(&self, moving: bool) -> (V, V) {
        let (a0, a1) = if moving {
            (self.moving, self.moving + self.thick)
        } else {
            (self.fixed - self.thick, self.fixed)
        };
        self.bx(
            a0,
            a1,
            self.off - self.len,
            self.off + self.len,
            FLOOR,
            self.top,
        )
    }
    fn stock_top(&self) -> f32 {
        BASE + self.lift + self.sh
    }
    fn draw_stock(&self, f: &mut Frame) {
        let (lo, hi) = self.bx(
            self.stock.0,
            self.stock.1,
            -self.sb,
            self.sb,
            BASE + self.lift,
            self.stock_top(),
        );
        cuboid(f, lo, hi, self.stock_tone);
    }
    fn draw(&self, f: &mut Frame) {
        let (lo, hi) = self.bx(-32., 32., -24., 24., 0., FLOOR);
        cuboid(f, lo, hi, BODY);
        for (a0, a1) in [(-10., -4.), (4., 10.)] {
            let (lo, hi) = self.bx(a0, a1, -self.sb - 4., self.sb + 4., FLOOR, BASE);
            cuboid(f, lo, hi, METAL);
        }
        let (lo, hi) = self.jaw_box(false);
        cuboid(f, lo, hi, JAW);
        let (mlo, mhi) = self.jaw_box(true);
        if self.stock_last {
            cuboid(f, mlo, mhi, JAW_MOVING);
            self.draw_stock(f);
        } else {
            self.draw_stock(f);
            cuboid(f, mlo, mhi, JAW_MOVING);
        }
    }
}

/// A thin pointer from a label to what it names.
fn leader(f: &mut Frame, a: Point, b: Point) {
    line(f, a, b, alpha(TEXT, 0.7), 1.);
}

pub fn vise(f: &mut Frame, time: f32, phase: f32, n: u8) {
    let mut v = Vise::new();
    match n {
        0 => {
            v.len = 12. + 10. * wave(phase);
            v.draw(f);
            let x = v.moving + v.thick + 7.;
            let z = v.top;
            for s in [-1., 1.] {
                ext(
                    f,
                    v.at(v.moving + v.thick, s * v.len, z),
                    v.at(x, s * v.len, z),
                    GOLD,
                );
            }
            let a = p(v.at(x, v.len, z));
            let b = p(v.at(x, -v.len, z));
            dimension(f, a, b, GOLD);
            let t = Point::new(b.x + 12., b.y - 22.);
            text(f, "JAW WIDTH", t.x, t.y, GOLD, 9.);
            leader(f, Point::new(t.x + 3., t.y + 11.), b);
            heading(f, "JAW WIDTH · LENGTH OF EACH JAW");
            caption(f, "JAW WIDTH: EACH JAW'S LENGTH ALONG THE JAWS");
        }
        1 => {
            let jh = 6. + 9. * wave(phase);
            v.top = BASE + jh;
            v.draw(f);
            let a = v.moving + v.thick / 2.;
            let b = v.len + 8.;
            ext(f, v.at(a, v.len, BASE), v.at(a, b, BASE), GOLD);
            ext(f, v.at(a, v.len, v.top), v.at(a, b, v.top), GOLD);
            let lo = p(v.at(a, b, BASE));
            let hi = p(v.at(a, b, v.top));
            dimension(f, lo, hi, GOLD);
            let mid = (lo.y + hi.y) / 2.;
            text(f, "JAW HEIGHT", lo.x - 76., mid - 5., GOLD, 9.);
            leader(f, Point::new(lo.x - 15., mid), Point::new(lo.x - 3., mid));
            heading(f, "JAW HEIGHT · ABOVE THE PARALLELS' TOP");
            text(f, "FOR REFERENCE", 8., 16., TEXT, 9.);
            caption(f, "MODELLED JAWS REACH THE GRIP HEIGHT INSTEAD");
        }
        2 => {
            v.thick = 8. + 8. * wave(phase);
            v.draw(f);
            let b = v.len - 5.;
            let z = v.top;
            // The near jaw carries the dimension.
            dim3(
                f,
                v.at(v.moving, b, z),
                v.at(v.moving + v.thick, b, z),
                GOLD,
            );
            let mid = p(v.at(v.moving + v.thick / 2., b, z));
            let t = Point::new(mid.x + 50., mid.y - 12.);
            text(f, "JAW THICKNESS", t.x, t.y, GOLD, 9.);
            leader(f, Point::new(t.x - 2., t.y + 5.), mid);
            heading(f, "JAW THICKNESS · OUTWARD FROM THE STOCK");
            caption(f, "JAW THICKNESS: HOW THICK EACH JAW IS");
        }
        3 => {
            v.fixed = -20.;
            v.stock = (-20., 8.);
            let wide = phase >= 0.5;
            if wide {
                v.moving = 20.;
                let k = smooth(((phase - 0.5) / 0.25).min(1.));
                // It stops on top of the jaws, at their top.
                v.lift = (v.top - BASE) + 16. * (1. - k);
                v.stock = (-24., 24.);
                v.sb = 10.;
                v.stock_tone = Tone {
                    top: mix(STOCK.top, RED, 0.45),
                    front: mix(STOCK.front, RED, 0.45),
                    side: mix(STOCK.side, RED, 0.45),
                };
                v.stock_last = true;
            } else {
                v.moving = 8. + 12. * smooth((phase / 0.3).min(1.));
            }
            v.draw(f);
            let b = v.len + 8.;
            for a in [v.fixed, v.moving] {
                ext(f, v.at(a, v.len, BASE), v.at(a, b, BASE), GOLD);
            }
            let l = p(v.at(v.fixed, b, BASE));
            dimension(f, l, p(v.at(v.moving, b, BASE)), GOLD);
            text(f, "MAX OPENING", l.x - 66., l.y + 6., GOLD, 9.);
            if wide {
                let zb = BASE + v.lift;
                let sb = v.sb + 8.;
                for a in [-24., 24.] {
                    ext(f, v.at(a, v.sb, zb), v.at(a, sb, zb), RED);
                }
                let r = p(v.at(-24., sb, zb));
                dimension(f, r, p(v.at(24., sb, zb)), RED);
                text(f, "TOO WIDE", r.x - 68., r.y - 8., RED, 10.);
                caption(f, "WIDER THAN THE OPENING: A SETUP ISSUE");
            } else {
                caption(f, "MAX OPENING: THE WIDEST STOCK THE JAWS TAKE");
            }
            heading(f, "MAX OPENING · WIDEST STOCK THE JAWS TAKE");
        }
        4 => {
            v.sh = 26.;
            let g = 6. + 12. * wave(phase);
            v.top = BASE + g;
            v.draw(f);
            let zt = v.stock_top();
            // The stock above the grip is free: the grip height marked on it.
            line3(
                f,
                v.at(-14., v.sb, v.top),
                v.at(14., v.sb, v.top),
                GOLD,
                1.6,
            );
            let b = v.len + 8.;
            ext(f, v.at(-14., v.sb, BASE), v.at(-14., b, BASE), GOLD);
            ext(f, v.at(-14., v.sb, v.top), v.at(-14., b, v.top), GOLD);
            let lo = p(v.at(-14., b, BASE));
            let hi = p(v.at(-14., b, v.top));
            dimension(f, lo, hi, GOLD);
            let mid = (lo.y + hi.y) / 2.;
            text(f, "GRIP", lo.x - 38., mid - 5., GOLD, 9.);
            leader(f, Point::new(lo.x - 11., mid), Point::new(lo.x - 3., mid));
            let free = p(v.at(14., -v.sb, zt));
            text(f, "STOCK ABOVE", free.x + 10., free.y - 6., TEXT, 9.);
            text(f, "THE GRIP: FREE", free.x + 10., free.y + 5., TEXT, 9.);
            heading(f, "GRIP · HEIGHT OF STOCK THE JAWS HOLD");
            caption(f, "GRIP: STOCK BOTTOM UP TO THE JAWS' TOP");
        }
        5 => {
            let pulse = wave(phase);
            v.draw(f);
            let z = v.top + 6.;
            let l = 12. + 6. * pulse;
            // Both jaws squeeze toward the stock from outside, at the front.
            let lt = v.fixed - v.thick - 2.;
            arrow(f, p(v.at(lt - l, 14., z)), p(v.at(lt, 14., z)), GOLD);
            let n = p(v.at(lt - l, 14., z));
            text(f, "N", n.x - 10., n.y - 6., GOLD, 10.);
            let rt = v.moving + v.thick + 2.;
            arrow(f, p(v.at(rt + l, 14., z)), p(v.at(rt, 14., z)), GOLD);
            let n = p(v.at(rt + l, 14., z));
            text(f, "N", n.x + 5., n.y - 6., GOLD, 10.);
            // The cutter pushes the part sideways, along the jaws.
            let push = 6. + 12. * progress(phase, 0.1);
            let zs = v.stock_top();
            cutter(f, [0., 12., zs], time, 10., false, 0.);
            arrow(f, p([0., 6., zs]), p([0., 6. - push, zs]), RED);
            let c = p([0., 6. - push, zs]);
            let t = Point::new(c.x + 26., c.y - 28.);
            text(f, "CUT PUSH", t.x, t.y, RED, 9.);
            leader(f, Point::new(t.x + 3., t.y + 11.), c);
            // The jaws' friction holds it: on the jaw's back half.
            arrow(
                f,
                p(v.at(v.moving + 4., -4. - push, v.top)),
                p(v.at(v.moving + 4., -4., v.top)),
                CYAN,
            );
            let lbl = p(v.at(v.moving + v.thick, -v.len, v.top));
            text(f, "FRICTION", lbl.x + 8., lbl.y - 4., CYAN, 9.);
            heading(f, "FORCE · HOW HARD THE VISE CLAMPS, IN N");
            caption(f, "N: JAW FORCE   RED: CUT PUSH   CYAN: FRICTION");
        }
        6 => {
            let half = phase < 0.5;
            v.axis = if half { 0 } else { 1 };
            // The same stock, 28 × 36: the jaws close on a different side.
            let sa = if half { 14. } else { 18. };
            v.fixed = -sa;
            v.moving = sa;
            v.stock = (-sa, sa);
            v.sb = if half { 18. } else { 14. };
            v.len = 20.;
            v.draw(f);
            marker(f, v.axis);
            tabs(f, &["X SIDES", "Y SIDES"], if half { 0 } else { 1 });
            heading(f, "VISE AXIS · WHICH SIDES THE JAWS FACE");
            caption(
                f,
                if half {
                    "JAWS ON THE −X AND +X SIDES OF THE STOCK"
                } else {
                    "JAWS ON THE −Y AND +Y SIDES OF THE STOCK"
                },
            );
        }
        7 => {
            let o = 11. * (phase * TAU).sin();
            v.len = 14.;
            v.off = o;
            v.draw(f);
            let zs = v.stock_top();
            let zj = v.top;
            let edge = v.moving + v.thick;
            let x = edge + 8.;
            // Centre marks: the stock's across its top, the jaws' on both jaw tops.
            dashed(f, p(v.at(-14., 0., zs)), p(v.at(14., 0., zs)), CYAN, 3.);
            line3(f, v.at(v.moving, o, zj), v.at(edge, o, zj), GOLD, 4.);
            line3(
                f,
                v.at(v.fixed - v.thick, o, zj),
                v.at(v.fixed, o, zj),
                GOLD,
                4.,
            );
            ext(f, v.at(edge, 0., zj), v.at(x, 0., zj), CYAN);
            ext(f, v.at(edge, o, zj), v.at(x, o, zj), GOLD);
            dim3(f, v.at(x, 0., zj), v.at(x, o, zj), GOLD);
            let q = p(v.at(x, 0., zj));
            text(f, "OFFSET", 200., 74., GOLD, 9.);
            leader(f, Point::new(204., 86.), Point::new(q.x + 2., q.y - 2.));
            heading(f, "VISE OFFSET · JAWS SLIDE ALONG THEIR LENGTH");
            caption(f, "OFFSET: STOCK CENTRE TO THE JAWS' CENTRE");
        }
        8 => {
            let third = ((phase * 3.) as usize).min(2);
            tabs(f, &["VISE", "CLAMPS", "NONE"], third);
            let g = 3.;
            match third {
                0 => {
                    v.draw(f);
                    for m in [false, true] {
                        let (lo, hi) = v.jaw_box(m);
                        let (lo, hi) = grown(lo, hi, g);
                        dbox(f, lo, hi, CYAN);
                    }
                    caption(f, "TWO JAW BOXES, CHECKED WITH A GAP");
                }
                1 => {
                    let c = Clamps::new();
                    let list = [(1, 5., 0.), (3, -5., 0.)];
                    draw_clamps(f, &c, &list);
                    for &(s, o, l) in &list {
                        let (lo, hi) = c.envelope(s, o, l);
                        let (lo, hi) = grown(lo, hi, g);
                        dbox(f, lo, hi, CYAN);
                    }
                    caption(f, "ONE BOX PER CLAMP, CHECKED WITH A GAP");
                }
                _ => {
                    let c = Clamps::new();
                    draw_clamps(f, &c, &[]);
                    text(f, "NONE · NOTHING CHECKED", 8., 30., RED, 10.);
                    caption(f, "NO BOXES: NO MOVE IS CHECKED AGAINST HOLDING");
                }
            }
            heading(f, "HOLDING · WHAT THE SETUP MODELS");
        }
        _ => {
            let g = 8.;
            v.len = 14.;
            let (lo, hi) = v.jaw_box(true);
            let (mut elo, ehi) = grown(lo, hi, g);
            // The envelope hugs the jaw, from the vise body up.
            elo[2] = FLOOR;
            let outer = v.moving + v.thick + g;
            let rc = 4.;
            let t = if phase < 0.5 {
                phase / 0.5
            } else {
                (1. - phase) / 0.5
            };
            let xc = 58. + (v.moving + v.thick + rc + 1. - 58.) * smooth(t);
            let warn = xc - rc < outer;
            let color = if warn { RED } else { CYAN };
            v.draw(f);
            let tip = v.at(xc, 0., 22.);
            cutter(f, tip, time, 10., false, 0.);
            if warn {
                let q = p(tip);
                f.fill_rectangle(
                    Point::new(q.x - 5., q.y - 17.),
                    iced::Size::new(10., 17.),
                    alpha(RED, 0.65),
                );
            }
            dbox(f, elo, ehi, color);
            // The gap, between the jaw's end face and the envelope's.
            let a = v.moving + v.thick / 2.;
            let d0 = p(v.at(a, v.len, v.top));
            let d1 = p(v.at(a, v.len + g, v.top));
            dimension(f, d0, d1, GOLD);
            let t = Point::new(d0.x - 44., d0.y + 12.);
            text(f, "GAP", t.x, t.y, GOLD, 9.);
            leader(
                f,
                Point::new(t.x + 20., t.y + 4.),
                Point::new((d0.x + d1.x) / 2., (d0.y + d1.y) / 2.),
            );
            if warn {
                text(f, "WARNING", 8., 16., RED, 10.);
            }
            heading(f, "HOLDING GAP · CLEARANCE AROUND THE JAWS");
            caption(f, "CYAN BOXES: JAW + GAP   RED: A MOVE WITHIN IT");
        }
    }
}

// ------------------------------------------------------------ strap clamps

const HX: f32 = 18.;
const HY: f32 = 13.;
const TABLE: f32 = 5.;
const SH: f32 = 12.;

#[derive(Clone, Copy)]
struct Clamps {
    len: f32,
    w: f32,
    ch: f32,
    bite: f32,
}

impl Clamps {
    fn new() -> Self {
        Clamps {
            len: 22.,
            w: 9.,
            ch: 5.,
            bite: 8.,
        }
    }
    fn top(&self) -> f32 {
        TABLE + SH
    }
    /// A point of a clamp on `side` (0 −X, 1 +X, 2 −Y, 3 +Y): `u` outward
    /// from the stock's middle, `lat` along the side.
    fn pt(side: usize, u: f32, lat: f32, z: f32) -> V {
        let u = if side % 2 == 0 { -u } else { u };
        if side < 2 { [u, lat, z] } else { [lat, u, z] }
    }
    fn edge(side: usize) -> f32 {
        if side < 2 { HX } else { HY }
    }
    /// The bar, its stud and its nut as boxes.
    fn parts(&self, side: usize, off: f32, lift: f32) -> [(V, V); 3] {
        let e = Self::edge(side);
        let (u0, u1) = (e - self.bite, e + self.len - self.bite);
        let top = self.top() + lift;
        let mk = |ua: f32, ub: f32, l0: f32, l1: f32, z0: f32, z1: f32| {
            let a = Self::pt(side, ua, l0, z0);
            let b = Self::pt(side, ub, l1, z1);
            (
                [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])],
                [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])],
            )
        };
        [
            mk(
                u0,
                u1,
                off - self.w / 2.,
                off + self.w / 2.,
                top,
                top + self.ch,
            ),
            mk(u1 - 7., u1 - 1., off - 3., off + 3., TABLE + lift, top),
            mk(
                u1 - 8.,
                u1 - 1.,
                off - 3.5,
                off + 3.5,
                top + self.ch,
                top + self.ch + 2.,
            ),
        ]
    }
    /// One box round a whole clamp.
    fn envelope(&self, side: usize, off: f32, lift: f32) -> (V, V) {
        let [bar, stud, nut] = self.parts(side, off, lift);
        let mut lo = bar.0;
        let mut hi = bar.1;
        for b in [stud, nut] {
            for i in 0..3 {
                lo[i] = lo[i].min(b.0[i]);
                hi[i] = hi[i].max(b.1[i]);
            }
        }
        (lo, hi)
    }
}

/// The table, the stock and the clamps `(side, offset, lift)` on it.
fn draw_clamps(f: &mut Frame, c: &Clamps, list: &[(usize, f32, f32)]) {
    cuboid(f, [-44., -22., 0.], [44., 22., TABLE], BODY);
    for &(s, o, l) in list {
        if s % 2 == 0 {
            let [_, stud, _] = c.parts(s, o, l);
            cuboid(f, stud.0, stud.1, METAL);
        }
    }
    cuboid(f, [-HX, -HY, TABLE], [HX, HY, c.top()], STOCK);
    for &(s, o, l) in list {
        if s % 2 == 1 {
            let [_, stud, _] = c.parts(s, o, l);
            cuboid(f, stud.0, stud.1, METAL);
        }
    }
    let mut order = list.to_vec();
    order.sort_by(|a, b| {
        let key = |&(s, o, l): &(usize, f32, f32)| {
            let [bar, _, _] = c.parts(s, o, l);
            bar.0[0] + bar.1[0] + bar.0[1] + bar.1[1]
        };
        key(a).total_cmp(&key(b))
    });
    for (s, o, l) in order {
        let [bar, _, nut] = c.parts(s, o, l);
        cuboid(f, bar.0, bar.1, JAW_MOVING);
        cuboid(f, nut.0, nut.1, JAW);
    }
}

pub fn clamp(f: &mut Frame, _time: f32, phase: f32, n: u8) {
    let mut c = Clamps::new();
    let side = 1;
    // Dimensions on the back side of the bar (toward −Y), in the open.
    let back = |c: &Clamps| -(c.w / 2. + 6.);
    match n {
        0 => {
            c.len = 16. + 14. * wave(phase);
            draw_clamps(f, &c, &[(side, 0., 0.)]);
            let (u0, u1) = (HX - c.bite, HX + c.len - c.bite);
            let z = c.top() + c.ch;
            let lat = back(&c);
            for u in [u0, u1] {
                ext(
                    f,
                    Clamps::pt(side, u, -c.w / 2., z),
                    Clamps::pt(side, u, lat, z),
                    GOLD,
                );
            }
            let a = p(Clamps::pt(side, u0, lat, z));
            let b = p(Clamps::pt(side, u1, lat, z));
            dimension(f, a, b, GOLD);
            let t = Point::new(a.x + 6., a.y - 26.);
            text(f, "CLAMP LENGTH", t.x, t.y, GOLD, 9.);
            leader(
                f,
                Point::new(t.x + 30., t.y + 11.),
                Point::new((a.x + b.x) / 2., (a.y + b.y) / 2.),
            );
            heading(f, "CLAMP LENGTH · FROM ITS TIP OUTWARD");
            caption(f, "CLAMP LENGTH: TIP TO END, BITE INCLUDED");
        }
        1 => {
            c.w = 9. + 9. * wave(phase);
            draw_clamps(f, &c, &[(side, 0., 0.)]);
            let u1 = HX + c.len - c.bite;
            let z = c.top() + c.ch;
            let u = u1 + 6.;
            for s in [-1., 1.] {
                ext(
                    f,
                    Clamps::pt(side, u1, s * c.w / 2., z),
                    Clamps::pt(side, u, s * c.w / 2., z),
                    GOLD,
                );
            }
            let a = p(Clamps::pt(side, u, c.w / 2., z));
            let b = p(Clamps::pt(side, u, -c.w / 2., z));
            dimension(f, a, b, GOLD);
            let t = Point::new(b.x + 10., b.y - 24.);
            text(f, "CLAMP WIDTH", t.x, t.y, GOLD, 9.);
            leader(f, Point::new(t.x + 3., t.y + 11.), b);
            heading(f, "CLAMP WIDTH · ALONG THE STOCK'S SIDE");
            caption(f, "CLAMP WIDTH: ACROSS THE BAR, ALONG THE SIDE");
        }
        2 => {
            c.ch = 9. + 5. * wave(phase);
            draw_clamps(f, &c, &[(side, 0., 0.)]);
            let u1 = HX + c.len - c.bite;
            let u = u1 + 7.;
            let zt = c.top();
            // A reference line at the stock's top, carried out past the bar's end.
            dashed(
                f,
                p(Clamps::pt(side, u1, 0., zt)),
                p(Clamps::pt(side, u + 4., 0., zt)),
                TEXT,
                1.6,
            );
            ext(
                f,
                Clamps::pt(side, u1, 0., zt + c.ch),
                Clamps::pt(side, u, 0., zt + c.ch),
                CYAN,
            );
            let a = p(Clamps::pt(side, u, 0., zt));
            let b = p(Clamps::pt(side, u, 0., zt + c.ch));
            dimension(f, a, b, CYAN);
            text(f, "CLAMP HEIGHT", b.x + 8., b.y - 6., CYAN, 9.);
            text(f, "STOCK TOP", a.x + 8., a.y + 1., TEXT, 9.);
            heading(f, "CLAMP HEIGHT · ABOVE THE STOCK'S TOP");
            caption(f, "CLAMP HEIGHT: STOCK TOP TO TOP OF THE CLAMP");
        }
        3 => {
            c.len = 20.;
            c.bite = 4. + 12. * wave(phase);
            draw_clamps(f, &c, &[(side, 0., 0.)]);
            let (u0, u1) = (HX - c.bite, HX + c.len - c.bite);
            let zt = c.top();
            // The bite, on the stock's top in front of the bar: tip to edge.
            let fl = c.w / 2. + 4.;
            ext(
                f,
                Clamps::pt(side, u0, c.w / 2., zt),
                Clamps::pt(side, u0, fl, zt),
                CYAN,
            );
            ext(
                f,
                Clamps::pt(side, HX, 0., zt),
                Clamps::pt(side, HX, fl, zt),
                CYAN,
            );
            let a = p(Clamps::pt(side, u0, fl, zt));
            let b = p(Clamps::pt(side, HX, fl, zt));
            dimension(f, a, b, CYAN);
            text(f, "BITE", b.x - 30., b.y + 8., CYAN, 9.);
            // The clamp's whole length, along the bar's back side.
            let z = zt + c.ch;
            let lat = back(&c);
            for u in [u0, u1] {
                ext(
                    f,
                    Clamps::pt(side, u, -c.w / 2., z),
                    Clamps::pt(side, u, lat, z),
                    GOLD,
                );
            }
            let l = p(Clamps::pt(side, u0, lat, z));
            let r = p(Clamps::pt(side, u1, lat, z));
            dimension(f, l, r, GOLD);
            let t = Point::new(l.x + 6., l.y - 26.);
            text(f, "CLAMP LENGTH", t.x, t.y, GOLD, 9.);
            leader(
                f,
                Point::new(t.x + 30., t.y + 11.),
                Point::new((l.x + r.x) / 2., (l.y + r.y) / 2.),
            );
            heading(f, "CLAMP BITE · REACH ONTO THE STOCK");
            caption(f, "BITE < CLAMP LENGTH");
        }
        _ => {
            c.len = 16.;
            c.bite = 7.;
            let sides = [
                (0usize, 0., 0.02),
                (3, 5., 0.14),
                (2, -5., 0.26),
                (1, 0., 0.38),
            ];
            let slide = phase >= 0.5;
            let off = if slide {
                8. * ((phase - 0.5) * 2. * TAU).sin()
            } else {
                0.
            };
            let mut list = vec![];
            for (s, o, t0) in sides {
                if phase < t0 {
                    continue;
                }
                let o = if s == 1 { off } else { o };
                let lift = 14. * (1. - smooth((phase - t0) / 0.08));
                list.push((s, o, lift));
            }
            draw_clamps(f, &c, &list);
            for &(s, o, l) in &list {
                let name = ["−X", "+X", "−Y", "+Y"][s];
                let e = Clamps::edge(s) + c.len - c.bite + 10.;
                let q = p(Clamps::pt(s, e, o, c.top() + l + 4.));
                let (dx, dy) = [(-18., -8.), (4., -2.), (2., -10.), (-20., -2.)][s];
                text(f, name, q.x + dx, q.y + dy, GOLD, 10.);
            }
            if slide {
                // The +X clamp slides along its side: offset from the middle.
                let zt = c.top();
                let u = HX - c.bite - 3.;
                dashed(
                    f,
                    p(Clamps::pt(1, u, -HY, zt)),
                    p(Clamps::pt(1, u, HY, zt)),
                    CYAN,
                    1.2,
                );
                let a = p(Clamps::pt(1, u, 0., zt));
                let b = p(Clamps::pt(1, u, off, zt));
                if (b.x - a.x).hypot(b.y - a.y) > 11. {
                    dimension(f, a, b, GOLD);
                } else {
                    line(f, a, b, GOLD, 2.);
                }
                let q = p(Clamps::pt(1, u, -HY, zt));
                text(f, "OFFSET", q.x + 6., q.y - 10., GOLD, 9.);
            }
            heading(f, "CLAMPS · A SIDE AND AN OFFSET EACH");
            caption(f, "ONE CLAMP PER ROW: SIDE, THEN OFFSET FROM ITS MIDDLE");
        }
    }
}

// ------------------------------------------------------------------- stock

/// A standing round bar of radius `r` and height `h` with its base centre
/// at `c`, translucent so what is inside shows.
fn vbar(f: &mut Frame, c: V, r: f32, h: f32, a: f32) {
    let base = p(c);
    let (rx, ry) = (r * 1.98, r * 0.735);
    let top = Point::new(base.x, base.y - h * 1.35);
    let n = 28;
    let mut body = vec![];
    for i in 0..=n {
        let t = PI * i as f32 / n as f32;
        body.push(Point::new(base.x + rx * t.cos(), base.y + ry * t.sin()));
    }
    for i in (0..=n).rev() {
        let t = PI * i as f32 / n as f32;
        body.push(Point::new(top.x + rx * t.cos(), top.y + ry * t.sin()));
    }
    polygon(f, &body, alpha(STOCK.front, a));
    let ring: Vec<Point> = (0..n)
        .map(|i| {
            let t = TAU * i as f32 / n as f32;
            Point::new(top.x + rx * t.cos(), top.y + ry * t.sin())
        })
        .collect();
    polygon(f, &ring, alpha(STOCK.top, a));
    let edge = Stroke::default().with_color(EDGE).with_width(0.8);
    f.stroke(
        &Path::new(|b| {
            b.move_to(ring[0]);
            for q in &ring[1..] {
                b.line_to(*q);
            }
            b.close();
        }),
        edge,
    );
    f.stroke(
        &Path::new(|b| {
            b.move_to(body[0]);
            for q in &body[1..=n] {
                b.line_to(*q);
            }
            b.line_to(Point::new(top.x - rx, top.y));
            b.move_to(body[0]);
            b.line_to(Point::new(top.x + rx, top.y));
        }),
        edge,
    );
}

/// The fixed-size stock box centred on X and Y, its bottom on Z 0, round
/// a part standing on Z0; `dims` are the axes (0 X, 1 Y, 2 Z) whose size is
/// marked, in `letters` named by a letter.
fn fixed_box(f: &mut Frame, sx: f32, sy: f32, sz: f32, dims: &[usize], letters: bool) {
    let (hx, hy) = (sx / 2., sy / 2.);
    cuboid(f, [-12., -8., 0.], [12., 8., 16.], PART);
    cuboid(f, [-hx, -hy, 0.], [hx, hy, sz], tone_a(STOCK, 0.3));
    // The Z0 mark touches the box's bottom edge.
    let corner = p([-hx, hy, 0.]);
    line(f, Point::new(corner.x - 16., corner.y), corner, CYAN, 1.6);
    f.fill(&Path::circle(corner, 2.2), CYAN);
    text(f, "Z0", corner.x - 36., corner.y - 5., CYAN, 9.);
    for &d in dims {
        let (a, b, name) = match d {
            0 => ([-hx, -hy - 6., sz], [hx, -hy - 6., sz], "X"),
            1 => ([hx + 6., hy, sz], [hx + 6., -hy, sz], "Y"),
            _ => ([hx + 6., -hy, 0.], [hx + 6., -hy, sz], "Z"),
        };
        let (pa, pb) = (p(a), p(b));
        dimension(f, pa, pb, GOLD);
        if letters {
            let m = Point::new((pa.x + pb.x) / 2., (pa.y + pb.y) / 2.);
            let (dx, dy) = [(-4., -16.), (10., 0.), (8., -5.)][d];
            text(f, name, m.x + dx, m.y + dy, GOLD, 10.);
        }
    }
}

pub fn stock(f: &mut Frame, _time: f32, phase: f32, n: u8) {
    match n {
        0 => {
            let third = ((phase * 3.) as usize).min(2);
            tabs(
                f,
                &["RELATIVE SIZE BOX", "FIXED SIZE BOX", "ROUND BAR"],
                third,
            );
            let w = wave(phase * 3. - third as f32);
            match third {
                0 => {
                    let o = 8. + 6. * w;
                    cuboid(f, [-14., -9., 6.], [14., 9., 22.], PART);
                    cuboid(
                        f,
                        [-14. - o, -9. - o, 6. - o],
                        [14. + o, 9. + o, 22. + o],
                        tone_a(STOCK, 0.3),
                    );
                    // Short offset arrows: on the top, to the right, upward.
                    let zt = 22. + o;
                    dim3(f, [14., 0., zt], [14. + o, 0., zt], GOLD);
                    dim3(f, [0., 9., zt], [0., 9. + o, zt], GOLD);
                    dim3(
                        f,
                        [14. + o + 2., 9. + o + 2., 22.],
                        [14. + o + 2., 9. + o + 2., 22. + o],
                        GOLD,
                    );
                    text(f, "PART + AN OFFSET ON EACH OF 6 SIDES", 8., 30., TEXT, 9.);
                    caption(f, "BLUE: PART   AMBER BOX: PART + OFFSETS");
                }
                1 => {
                    fixed_box(
                        f,
                        44. + 8. * w,
                        30. + 6. * w,
                        26. + 6. * w,
                        &[0, 1, 2],
                        true,
                    );
                    text(f, "X, Y AND Z SIZES, BOTTOM ON Z0", 8., 30., TEXT, 9.);
                    caption(f, "BLUE: PART   AMBER BOX: THE SIZES GIVEN");
                }
                _ => {
                    cuboid(f, [-12., -8., 0.], [12., 8., 16.], PART);
                    vbar(f, [0., 0., 0.], 18. + 3. * w, 28., 0.3);
                    text(
                        f,
                        "A BAR ALONG Z ON X0 Y0: FOR THE LATHE",
                        8.,
                        30.,
                        TEXT,
                        9.,
                    );
                    caption(f, "BLUE: PART   AMBER BAR: ROUND BAR AROUND IT");
                }
            }
            heading(f, "STOCK TYPE · THE SHAPE OF THE MATERIAL");
        }
        1 => {
            let r = 12. + 10. * wave(phase);
            vbar(f, [0., 0., 0.], r, 40., 0.7);
            let top = Point::new(139., 111. - 40. * 1.35);
            let rx = r * 1.98;
            dimension(
                f,
                Point::new(top.x - rx, top.y),
                Point::new(top.x + rx, top.y),
                GOLD,
            );
            text(f, "DIAMETER", 105., top.y - r * 0.735 - 16., GOLD, 10.);
            line(
                f,
                Point::new(top.x, top.y),
                Point::new(top.x, 111.),
                DIM,
                1.,
            );
            text(f, "X0 Y0", 122., 132., TEXT, 9.);
            heading(f, "STOCK DIAMETER · ROUND BAR");
            caption(f, "DIAMETER: ACROSS THE BAR, WHICH STANDS ON Z");
        }
        2 => {
            let h = 22. + 26. * wave(phase);
            let r = 14.;
            vbar(f, [0., 0., 0.], r, h, 0.7);
            let rx = r * 1.98;
            let x = 139. + rx + 12.;
            let top = 111. - h * 1.35;
            for y in [111., top] {
                dashed(f, Point::new(139. + rx, y), Point::new(x + 3., y), GOLD, 1.);
            }
            dimension(f, Point::new(x, 111.), Point::new(x, top), GOLD);
            text(f, "LENGTH", x + 7., (111. + top) / 2. - 5., GOLD, 10.);
            // Z0: the bar starts here and grows up.
            line(
                f,
                Point::new(139. - rx - 18., 111.),
                Point::new(139. - rx, 111.),
                CYAN,
                1.6,
            );
            f.fill(&Path::circle(Point::new(139. - rx, 111.), 2.2), CYAN);
            text(f, "Z0", 139. - rx - 38., 106., CYAN, 10.);
            heading(f, "STOCK LENGTH · BAR LENGTH ALONG Z");
            caption(f, "LENGTH: FROM Z0 UP");
        }
        3 => {
            let (cx, cy) = (139., 80.);
            let (hw, hh) = (28_f32, 17_f32);
            let corner = (hw * hw + hh * hh).sqrt();
            let extra = 4. + 11. * wave(phase);
            let r = corner + extra;
            let circle = Path::circle(Point::new(cx, cy), r);
            f.fill(&circle, alpha(STOCK.top, 0.35));
            f.stroke(
                &circle,
                Stroke::default().with_color(STOCK.top).with_width(1.5),
            );
            rect(f, cx - hw, cy - hh, cx + hw, cy + hh, PART.top);
            let (ux, uy) = (hw / corner, -hh / corner);
            let tip = Point::new(cx + ux * corner, cy + uy * corner);
            line(f, Point::new(cx, cy), tip, CYAN, 2.);
            line(f, tip, Point::new(cx + ux * r, cy + uy * r), GOLD, 3.);
            f.fill(&Path::circle(Point::new(cx, cy), 2.5), TEXT);
            text(f, "Z", cx - 12., cy + 2., TEXT, 9.);
            text(f, "PART", cx - hw + 4., cy + hh - 13., BACKGROUND, 9.);
            let mid = Point::new(
                cx + ux * (corner + extra / 2.),
                cy + uy * (corner + extra / 2.),
            );
            let t = Point::new(cx + ux * r + 12., cy + uy * r - 22.);
            text(f, "EXTRA", t.x, t.y, GOLD, 10.);
            leader(f, Point::new(t.x + 4., t.y + 12.), mid);
            heading(f, "STOCK RADIAL · EXTRA ROUND THE PART");
            caption(f, "RADIUS = CYAN FURTHEST POINT + EXTRA");
            text(f, "SEEN FROM ABOVE", 8., 16., GHOST, 9.);
        }
        _ => {
            let axis = (n - 4) as usize;
            let w = wave(phase);
            let (mut sx, mut sy, mut sz) = (52., 32., 26.);
            let name = ["STOCK X SIZE", "STOCK Y SIZE", "STOCK Z SIZE"][axis];
            match axis {
                0 => sx = 44. + 32. * w,
                1 => {
                    sx = 40.;
                    sy = 28. + 16. * w;
                }
                _ => sz = 24. + 20. * w,
            }
            fixed_box(f, sx, sy, sz, &[axis], false);
            let (hx, hy) = (sx / 2., sy / 2.);
            let (a, b) = match axis {
                0 => (p([-hx, -hy - 6., sz]), p([hx, -hy - 6., sz])),
                1 => (p([hx + 6., hy, sz]), p([hx + 6., -hy, sz])),
                _ => (p([hx + 6., -hy, 0.]), p([hx + 6., -hy, sz])),
            };
            let mid = Point::new((a.x + b.x) / 2., (a.y + b.y) / 2.);
            // The label sits in clear space, with a short leader.
            match axis {
                0 => {
                    text(f, name, mid.x - 38., mid.y - 32., GOLD, 10.);
                    leader(
                        f,
                        Point::new(mid.x, mid.y - 20.),
                        Point::new(mid.x, mid.y - 3.),
                    );
                }
                1 => {
                    text(f, "STOCK", b.x + 8., b.y - 16., GOLD, 10.);
                    text(f, "Y SIZE", b.x + 8., b.y - 5., GOLD, 10.);
                }
                _ => text(f, name, a.x + 8., mid.y - 5., GOLD, 10.),
            }
            heading(
                f,
                &format!(
                    "{name} · THE BOX STRETCHES ALONG {} ONLY",
                    ["X", "Y", "Z"][axis]
                ),
            );
            caption(f, "CENTRED ON X AND Y, ITS BOTTOM ON Z0");
        }
    }
}
