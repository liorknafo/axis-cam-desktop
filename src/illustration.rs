//! Hover-card illustrations drawn as small scenes: amber stock that the
//! cutter takes away, a cutter whose flutes turn, and for depths and
//! spacings a section or top view with the dimension marked. Each draws one
//! moment of its 5 s phase on the card's 278 × 155 canvas.
use crate::{choice_diagram::SPIN, help::Diagram, model::Strategy};
use iced::{
    Color, Point, Size,
    widget::canvas::{Frame, Path, Stroke, Text},
};
use std::f32::consts::{PI, TAU};

type V = [f32; 3];
const CYAN: Color = Color::from_rgb(0.26, 0.83, 0.92);
const GOLD: Color = Color::from_rgb(1., 0.75, 0.33);
const TEXT: Color = Color::from_rgb(0.725, 0.81, 0.875);
const DIM: Color = Color::from_rgba(0.26, 0.83, 0.92, 0.28);
const BACKGROUND: Color = Color::from_rgb(0.106, 0.173, 0.239);
const EDGE: Color = Color::from_rgba(0.85, 0.9, 0.94, 0.35);
const HOLE: Color = Color::from_rgb(0.12, 0.18, 0.24);
/// The three faces this view sees of a box: its top, its +Y face (front
/// left on screen) and its +X face (front right).
#[derive(Clone, Copy)]
struct Tone {
    top: Color,
    front: Color,
    side: Color,
}
/// Finished metal.
const METAL: Tone = Tone {
    top: Color::from_rgb(0.57, 0.67, 0.73),
    front: Color::from_rgb(0.45, 0.56, 0.64),
    side: Color::from_rgb(0.40, 0.52, 0.60),
};
/// Metal the cutter has just cut: brighter than the rest.
const FRESH: Tone = Tone {
    top: Color::from_rgb(0.72, 0.84, 0.90),
    front: Color::from_rgb(0.60, 0.72, 0.80),
    side: Color::from_rgb(0.54, 0.67, 0.76),
};
/// Material still to be removed.
const STOCK: Tone = Tone {
    top: Color::from_rgb(0.80, 0.63, 0.36),
    front: Color::from_rgb(0.66, 0.50, 0.27),
    side: Color::from_rgb(0.58, 0.44, 0.24),
};
/// The direction the view looks from: +X, +Y and up.
const VIEW: V = [1., 1., 0.77];

fn p(a: V) -> Point {
    Point::new(
        139. + (a[0] - a[1]) * 1.40,
        111. + (a[0] + a[1]) * 0.52 - a[2] * 1.35,
    )
}
fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn lerp(a: V, b: V, t: f32) -> V {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}
fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgb(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
    )
}
/// The phase as progress: 0 to 1 over all but the last `hold` of the
/// phase, which shows the finished result before it starts again.
fn progress(phase: f32, hold: f32) -> f32 {
    (phase / (1. - hold)).min(1.)
}
fn smooth(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    t * t * (3. - 2. * t)
}
fn line(f: &mut Frame, a: Point, b: Point, color: Color, width: f32) {
    f.stroke(
        &Path::line(a, b),
        Stroke::default().with_color(color).with_width(width),
    );
}
fn line3(f: &mut Frame, a: V, b: V, color: Color, width: f32) {
    line(f, p(a), p(b), color, width);
}
fn polyline3(f: &mut Frame, points: &[V], color: Color, width: f32) {
    if points.len() < 2 {
        return;
    }
    let path = Path::new(|b| {
        b.move_to(p(points[0]));
        for a in &points[1..] {
            b.line_to(p(*a));
        }
    });
    f.stroke(&path, Stroke::default().with_color(color).with_width(width));
}
fn dashed(f: &mut Frame, a: Point, b: Point, color: Color, width: f32) {
    let n = ((b.x - a.x).hypot(b.y - a.y) / 6.).max(1.) as usize;
    for i in (0..n).step_by(2) {
        let t0 = i as f32 / n as f32;
        let t1 = (i + 1) as f32 / n as f32;
        line(
            f,
            Point::new(a.x + (b.x - a.x) * t0, a.y + (b.y - a.y) * t0),
            Point::new(a.x + (b.x - a.x) * t1, a.y + (b.y - a.y) * t1),
            color,
            width,
        );
    }
}
fn polygon(f: &mut Frame, points: &[Point], fill: Color) {
    let path = Path::new(|b| {
        b.move_to(points[0]);
        for a in &points[1..] {
            b.line_to(*a);
        }
        b.close();
    });
    f.fill(&path, fill);
}
fn quad(f: &mut Frame, points: &[V], fill: Color) {
    let screen: Vec<Point> = points.iter().map(|a| p(*a)).collect();
    polygon(f, &screen, fill);
    let path = Path::new(|b| {
        b.move_to(screen[0]);
        for a in &screen[1..] {
            b.line_to(*a);
        }
        b.close();
    });
    f.stroke(&path, Stroke::default().with_color(EDGE).with_width(0.6));
}
/// A box from `min` to `max` as this view sees it.
fn cuboid(f: &mut Frame, min: V, max: V, t: Tone) {
    let [x, y, z] = min;
    let [xx, yy, zz] = max;
    if xx - x < 0.01 || yy - y < 0.01 || zz - z < 0.01 {
        return;
    }
    quad(
        f,
        &[[x, yy, z], [xx, yy, z], [xx, yy, zz], [x, yy, zz]],
        t.front,
    );
    quad(
        f,
        &[[xx, y, z], [xx, yy, z], [xx, yy, zz], [xx, y, zz]],
        t.side,
    );
    quad(
        f,
        &[[x, y, zz], [xx, y, zz], [xx, yy, zz], [x, yy, zz]],
        t.top,
    );
}
/// A face shaded by its normal, or nothing when it faces away.
fn lit(f: &mut Frame, points: &[V], base: Tone) {
    let u = [
        points[1][0] - points[0][0],
        points[1][1] - points[0][1],
        points[1][2] - points[0][2],
    ];
    let v = [
        points[2][0] - points[0][0],
        points[2][1] - points[0][1],
        points[2][2] - points[0][2],
    ];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
    if n[0] * VIEW[0] + n[1] * VIEW[1] + n[2] * VIEW[2] <= 0. {
        return;
    }
    // Lit from above, front left: a top, +Y and +X face get the tone's
    // own three colours and slanted faces what lies between.
    let b = (0.25 * n[0] + 0.55 * n[1] + 0.8 * n[2]) / len;
    let color = if b >= 0.55 {
        mix(base.front, base.top, ((b - 0.55) / 0.25).min(1.))
    } else {
        mix(base.side, base.front, ((b - 0.25) / 0.3).max(0.))
    };
    quad(f, points, color);
}
fn text(f: &mut Frame, s: &str, x: f32, y: f32, color: Color, size: f32) {
    f.fill_text(Text {
        content: s.into(),
        position: Point::new(x, y),
        color,
        size: size.into(),
        ..Default::default()
    });
}
/// The line at the top of the canvas.
fn heading(f: &mut Frame, s: &str) {
    text(f, s, 8., 3., GOLD, 10.);
}
/// The legend under the drawing.
fn caption(f: &mut Frame, s: &str) {
    text(f, s, 8., 143., TEXT, 9.);
}
/// A dimension between `a` and `b` with an arrowhead at each end.
fn dimension(f: &mut Frame, a: Point, b: Point, color: Color) {
    line(f, a, b, color, 1.4);
    let len = (b.x - a.x).hypot(b.y - a.y).max(1e-3);
    let (ux, uy) = ((b.x - a.x) / len, (b.y - a.y) / len);
    for (tip, s) in [(a, 1.), (b, -1.)] {
        for side in [-1., 1.] {
            line(
                f,
                tip,
                Point::new(
                    tip.x + s * 5. * ux + side * 3. * uy,
                    tip.y + s * 5. * uy - side * 3. * ux,
                ),
                color,
                1.4,
            );
        }
    }
}
/// A cutter in screen space: its tip at `tip`, its axis toward the
/// spindle along the unit `up`, `d` px wide. Gold flutes whose marks move
/// as it turns, a light shank and a grey holder; a drill ends in a point.
/// Nothing is drawn of the first `hidden` px from the tip (inside a hole).
fn cutter_at(
    f: &mut Frame,
    tip: Point,
    up: (f32, f32),
    time: f32,
    d: f32,
    drill: bool,
    hidden: f32,
) {
    let at = |s: f32, side: f32| {
        Point::new(
            tip.x + up.0 * s - up.1 * side * d / 2.,
            tip.y + up.1 * s + up.0 * side * d / 2.,
        )
    };
    // The canvas does not clip, so a cutter raised to its clearance stops
    // at the top edge rather than drawing over the card's text above.
    let top = if up.1 < 0. {
        tip.y / -up.1
    } else {
        f32::INFINITY
    };
    let band = |f: &mut Frame, s0: f32, s1: f32, width: f32, color: Color| {
        let (s0, s1) = (s0.max(hidden).min(top), s1.max(hidden).min(top));
        if s1 - s0 < 0.1 {
            return;
        }
        let k = width / d;
        polygon(f, &[at(s0, -k), at(s1, -k), at(s1, k), at(s0, k)], color);
    };
    let flutes = 17.;
    band(f, 31., 40., d + 9., Color::from_rgb(0.58, 0.67, 0.75));
    band(f, flutes, 31., d * 0.85, Color::from_rgb(0.87, 0.91, 0.93));
    let point = if drill { d * 0.45 } else { 0. };
    band(f, point, flutes, d, GOLD);
    if drill && hidden < point && point <= top {
        let h = hidden.max(0.);
        let k = h / point;
        polygon(
            f,
            &[at(h, -k), at(point, -1.), at(point, 1.), at(h, k)],
            GOLD,
        );
    }
    // Helical flute marks running up the flutes as the cutter turns.
    for i in 0..4 {
        let s = point + ((i as f32 + time * 2.) % 4.) / 4. * (flutes - point);
        if s < hidden || s + 3. > flutes.min(top) {
            continue;
        }
        line(
            f,
            at(s, -1.),
            at(s + 3., 1.),
            Color::from_rgb(0.56, 0.39, 0.16),
            1.6,
        );
    }
}
/// A vertical cutter in the 3D view with its tip at `tip`, hidden below
/// the height `hide_below` (as when it is down a hole).
fn cutter(f: &mut Frame, tip: V, time: f32, d: f32, drill: bool, hide_below: f32) {
    let hidden = (hide_below - tip[2]).max(0.) * 1.35;
    cutter_at(f, p(tip), (0., -1.), time, d, drill, hidden);
}
/// A cutter whose axis leans by `tilt` radians toward +X.
fn tilted_cutter(f: &mut Frame, tip: V, tilt: f32, time: f32, d: f32) {
    let a = p(tip);
    let b = p(add(tip, [tilt.sin() * 10., 0., tilt.cos() * 10.]));
    let len = (b.x - a.x).hypot(b.y - a.y);
    cutter_at(
        f,
        a,
        ((b.x - a.x) / len, (b.y - a.y) / len),
        time,
        d,
        false,
        0.,
    );
}
/// A recess cut into a flat top at `top`, from `x0..x1 × y0..y1` down to
/// `floor`: its two far walls and as much of its floor as shows.
fn pocket(f: &mut Frame, x0: f32, x1: f32, y0: f32, y1: f32, top: f32, floor: f32) {
    if x1 - x0 < 0.01 || y1 - y0 < 0.01 {
        return;
    }
    // A point lowered by h shows where one 1.3 h further along +X and +Y
    // would on the top.
    let s = (1.3 * (top - floor)).min(x1 - x0).min(y1 - y0);
    let walls = Color::from_rgb(0.24, 0.33, 0.41);
    quad(
        f,
        &[
            [x0, y0, top],
            [x1, y0, top],
            [x1 - s, y0, floor],
            [x0, y0, floor],
        ],
        walls,
    );
    quad(
        f,
        &[
            [x0, y0, top],
            [x0, y1, top],
            [x0, y1 - s, floor],
            [x0, y0, floor],
        ],
        Color::from_rgb(0.30, 0.40, 0.48),
    );
    quad(
        f,
        &[
            [x0, y0, floor],
            [x1 - s, y0, floor],
            [x1 - s, y1 - s, floor],
            [x0, y1 - s, floor],
        ],
        FRESH.top,
    );
}
/// A disc of radius r around `c` in the plane at height c[2].
fn disc(c: V, r: f32) -> Vec<V> {
    (0..24)
        .map(|i| {
            let a = i as f32 * TAU / 24.;
            [c[0] + r * a.cos(), c[1] + r * a.sin(), c[2]]
        })
        .collect()
}
/// The point a fraction `u` along a polyline, by length, and how many of
/// its points lie behind it.
fn along(path: &[V], u: f32) -> (V, usize) {
    let lengths: Vec<f32> = path
        .windows(2)
        .map(|w| {
            ((w[1][0] - w[0][0]).powi(2)
                + (w[1][1] - w[0][1]).powi(2)
                + (w[1][2] - w[0][2]).powi(2))
            .sqrt()
        })
        .collect();
    let total: f32 = lengths.iter().sum();
    let mut left = u.clamp(0., 1.) * total;
    for (i, l) in lengths.iter().enumerate() {
        if left <= *l && *l > 0. {
            return (lerp(path[i], path[i + 1], left / l), i + 1);
        }
        left -= l;
    }
    (*path.last().unwrap(), path.len())
}
pub fn draw(d: Diagram, f: &mut Frame, time: f32) -> bool {
    let phase = (time / 5.).fract();
    match d {
        Diagram::Operation(Strategy::Face) => facing(f, time, phase),
        Diagram::Operation(Strategy::Box) => manual_box(f, time, phase),
        Diagram::Operation(Strategy::Drill) => drilling(f, time, phase),
        Diagram::Operation(Strategy::Pocket) => pocketing(f, time, phase),
        Diagram::Operation(Strategy::Adaptive) => adaptive(f, time, phase),
        Diagram::Operation(Strategy::Contour) => contour(f, time, phase),
        Diagram::Operation(Strategy::Waterline) => waterline(f, time, phase),
        Diagram::Operation(Strategy::Indexed) => indexed(f, time, phase, false),
        Diagram::Orientation => indexed(f, time, phase, true),
        Diagram::Operation(Strategy::Turn) => turning(f, time, phase),
        Diagram::Operation(Strategy::FiveAxis) => five_axis(f, time, phase),
        Diagram::Stepdown => stepdown(f, time, phase),
        Diagram::Stepover => stepover(f, time, phase),
        Diagram::Allowance => allowance(f, time, phase),
        Diagram::Ramp => ramp(f, time, phase),
        Diagram::Offsets => offsets(f, time, phase),
        Diagram::StockOffset(axis, positive) => stock_offset(f, phase, axis, positive),
        Diagram::Entry(ramp) => entry(f, time, phase, ramp),
        Diagram::DrillCycle(cycle) => drill_cycle(f, time, phase, cycle),
        _ => return false,
    }
    true
}

/// Facing: the amber top layer goes, one row at a time, leaving a bright
/// faced top.
fn facing(f: &mut Frame, time: f32, phase: f32) {
    let u = progress(phase, 0.15) * 4.;
    let row = (u as usize).min(3);
    let k = u - row as f32;
    let x = if row % 2 == 0 {
        -48. + 96. * k
    } else {
        48. - 96. * k
    };
    cuboid(
        f,
        [-40., -24., 0.],
        [40., 24., 22.],
        Tone {
            top: FRESH.top,
            ..METAL
        },
    );
    let piece = |f: &mut Frame, i: usize| {
        let y = -24. + 12. * i as f32;
        let (a, b) = if i < row {
            return;
        } else if i > row {
            (-40., 40.)
        } else if row % 2 == 0 {
            ((x + 7.).clamp(-40., 40.), 40.)
        } else {
            (-40., (x - 7.).clamp(-40., 40.))
        };
        cuboid(f, [a, y, 22.], [b, y + 12., 26.], STOCK);
    };
    for i in 0..=row {
        piece(f, i);
    }
    cutter(f, [x, -18. + 12. * row as f32, 22.], time, 17., false, 0.);
    for i in row + 1..4 {
        piece(f, i);
    }
    heading(f, "FACING · ROW BY ROW ACROSS THE TOP");
    caption(f, "AMBER: TOP LAYER TO REMOVE   BRIGHT: FACED");
}

/// Box on a manual mill: a step cut by straight passes along X, a step
/// in Y between them, and then a layer lower.
fn manual_box(f: &mut Frame, time: f32, phase: f32) {
    let u = progress(phase, 0.12) * 8.;
    let pass = (u as usize).min(7);
    let k = u - pass as f32;
    let (layer, row) = (pass / 4, pass % 4);
    let x = if row % 2 == 0 {
        -1. + 49. * k
    } else {
        48. - 49. * k
    };
    cuboid(f, [-40., -24., 0.], [40., 24., 12.], FRESH);
    cuboid(f, [-40., -24., 12.], [-8., 24., 24.], METAL);
    let mut pieces = vec![];
    for l in 0..2 {
        for i in 0..4 {
            let q = l * 4 + i;
            let (a, b) = if q < pass {
                continue;
            } else if q > pass {
                (-8., 40.)
            } else if i % 2 == 0 {
                ((x + 7.).clamp(-8., 40.), 40.)
            } else {
                (-8., (x - 7.).clamp(-8., 40.))
            };
            let y = -24. + 12. * i as f32;
            let z = 18. - 6. * l as f32;
            pieces.push(([a, y, z], [b, y + 12., z + 6.]));
        }
    }
    let tip = [x, -18. + 12. * row as f32, 18. - 6. * layer as f32];
    // Behind-first: lower layers, then rows toward the front.
    pieces.sort_by(|a, b| (a.0[2], a.0[1]).partial_cmp(&(b.0[2], b.0[1])).unwrap());
    let mut drawn = false;
    for (min, max) in pieces {
        if !drawn && min[2] >= tip[2] && min[1] > tip[1] {
            cutter(f, tip, time, 17., false, 0.);
            drawn = true;
        }
        cuboid(f, min, max, STOCK);
    }
    if !drawn {
        cutter(f, tip, time, 17., false, 0.);
    }
    // The axis moving now: X along a pass, Z at a layer's first pass.
    axes(
        f,
        Some(if k < 0.06 && row == 0 {
            2
        } else if k < 0.06 {
            1
        } else {
            0
        }),
    );
    heading(f, "ONE AXIS AT A TIME: X, THEN Y, THEN Z");
    caption(f, "AMBER: THE BLOCK OF MATERIAL TO REMOVE");
}

/// Drilling: the drill goes down each hole in turn and comes back up,
/// leaving a hole behind.
fn drilling(f: &mut Frame, time: f32, phase: f32) {
    let holes = [[-22., -10.], [22., -10.], [22., 10.], [-22., 10.]];
    let u = progress(phase, 0.1) * 4.;
    let h = (u as usize).min(3);
    let k = u - h as f32;
    let from = if h == 0 { [-44., -10.] } else { holes[h - 1] };
    let (xy, z) = if k < 0.15 {
        let t = smooth(k / 0.15);
        (
            [
                from[0] + (holes[h][0] - from[0]) * t,
                from[1] + (holes[h][1] - from[1]) * t,
            ],
            34.,
        )
    } else if k < 0.6 {
        (holes[h], 34. - 26. * (k - 0.15) / 0.45)
    } else if k < 0.75 {
        (holes[h], 8. + 26. * (k - 0.6) / 0.15)
    } else {
        (holes[h], 34.)
    };
    cuboid(f, [-40., -24., 0.], [40., 24., 24.], METAL);
    let deepest = if k < 0.15 {
        24.
    } else if k < 0.6 {
        z
    } else {
        8.
    };
    for (i, c) in holes.iter().enumerate() {
        if i < h || (i == h && deepest < 24.) {
            let rim: Vec<Point> = disc([c[0], c[1], 24.], 4.6).iter().map(|a| p(*a)).collect();
            polygon(f, &rim, HOLE);
            let inner: Vec<Point> = disc([c[0] + 1.2, c[1] + 1.2, 24.], 3.2)
                .iter()
                .map(|a| p(*a))
                .collect();
            polygon(f, &inner, Color::from_rgb(0.07, 0.11, 0.15));
        }
    }
    cutter(f, [xy[0], xy[1], z], time, 10., true, 24.);
    heading(f, "DRILLING · DOWN EACH HOLE, THEN BACK UP");
    caption(f, "FEED IN, RETRACT, MOVE TO THE NEXT HOLE");
}

/// A rectangular spiral growing from the centre: the point a fraction
/// `t` of the way out, with the half sizes it ends at.
fn spiral(t: f32, hx: f32, hy: f32) -> V {
    let rings = 3.;
    let r = t * rings;
    let v = r.fract();
    let (ax, ay) = (hx * r / rings, hy * r / rings);
    // Round the rectangle's perimeter, starting at its -X, -Y corner.
    let perimeter = 4. * (ax + ay);
    let mut d = v * perimeter;
    let sides = [
        (2. * ax, [1., 0.]),
        (2. * ay, [0., 1.]),
        (2. * ax, [-1., 0.]),
        (2. * ay, [0., -1.]),
    ];
    let mut at = [-ax, -ay];
    for (len, dir) in sides {
        if d <= len {
            at = [at[0] + dir[0] * d, at[1] + dir[1] * d];
            break;
        }
        at = [at[0] + dir[0] * len, at[1] + dir[1] * len];
        d -= len;
    }
    [at[0], at[1], 0.]
}

/// Pocket: three layers, each cleared from the centre outward.
fn pocketing(f: &mut Frame, time: f32, phase: f32) {
    let (x0, x1, y0, y1) = (-26., 26., -14., 14.);
    let u = progress(phase, 0.1) * 3.;
    let layer = (u as usize).min(2);
    let k = u - layer as f32;
    let above = 24. - 4. * layer as f32;
    let floor = above - 4.;
    cuboid(f, [-40., -24., 0.], [40., 24., 24.], METAL);
    if layer > 0 {
        pocket(f, x0, x1, y0, y1, 24., above);
    }
    // The part of this layer cleared so far: the spiral plus the cutter's
    // radius, inside what the layer above shows of its floor.
    let s = 1.3 * (24. - above);
    let (hx, hy) = (21. * k + 5., 9. * k + 5.);
    pocket(
        f,
        (-hx).max(x0),
        hx.min(x1 - s),
        (-hy).max(y0),
        hy.min(y1 - s),
        above,
        floor,
    );
    let path: Vec<V> = (0..=90)
        .map(|i| {
            let a = spiral(i as f32 / 90. * k, 21., 9.);
            [a[0], a[1], floor]
        })
        .collect();
    polyline3(f, &path, CYAN, 1.5);
    let at = *path.last().unwrap();
    cutter(f, at, time, 11., false, 0.);
    heading(f, &format!("POCKET · LAYER {} OF 3", layer + 1));
    caption(f, "EACH LAYER CLEARED FROM THE CENTRE OUTWARD");
}

/// Adaptive clearing: small loops that advance along the pocket at its
/// full depth, so the cutter never takes a full-width bite.
fn adaptive(f: &mut Frame, time: f32, phase: f32) {
    let u = progress(phase, 0.1);
    let at = |u: f32| {
        let cx = -20. + 40. * u;
        let a = u * TAU * 7.;
        [cx + 9. * a.cos(), 9. * a.sin(), 12.]
    };
    cuboid(f, [-40., -24., 0.], [40., 24., 24.], METAL);
    let cx = -20. + 40. * u;
    pocket(f, -32., (cx + 18.).min(32.), -16., 16., 24., 12.);
    // The pocket's outline still to clear.
    polyline3(
        f,
        &[
            [-32., -16., 24.],
            [32., -16., 24.],
            [32., 16., 24.],
            [-32., 16., 24.],
            [-32., -16., 24.],
        ],
        DIM,
        1.,
    );
    let path: Vec<V> = (0..=160).map(|i| at(u * i as f32 / 160.)).collect();
    polyline3(f, &path, CYAN, 1.6);
    cutter(f, at(u), time, 12., false, 0.);
    heading(f, "ADAPTIVE · SMALL LOOPS AT FULL DEPTH");
    caption(f, "A LIGHT, STEADY BITE WHILE THE POCKET GROWS");
}

/// Contour: the cutter follows the outside wall of a raised boss, once at
/// half depth and once at full depth, taking off the amber skin.
fn contour(f: &mut Frame, time: f32, phase: f32) {
    let u = progress(phase, 0.1) * 2.;
    let pass = (u as usize).min(1);
    let s = (u - pass as f32) * 4.;
    let side = (s as usize).min(3);
    let v = s - side as f32;
    let corners = [
        [-27., 17.],
        [27., 17.],
        [27., -17.],
        [-27., -17.],
        [-27., 17.],
    ];
    let c = [
        corners[side][0] + (corners[side + 1][0] - corners[side][0]) * v,
        corners[side][1] + (corners[side + 1][1] - corners[side][1]) * v,
    ];
    let tip = [c[0], c[1], 18. - 8. * pass as f32];
    cuboid(f, [-40., -24., 0.], [40., 24., 10.], METAL);
    let mut items: Vec<(f32, V, V, Tone)> = vec![(18., [-22., -12., 10.], [22., 12., 26.], FRESH)];
    for p in 0..2 {
        let (z0, z1) = (18. - 8. * p as f32, 26. - 8. * p as f32);
        // The four strips of skin, each shortened as the cutter passes.
        let strips: [(usize, V, V); 4] = [
            (0, [-26., 12., z0], [26., 16., z1]),
            (1, [22., -12., z0], [26., 12., z1]),
            (2, [-26., -16., z0], [26., -12., z1]),
            (3, [-26., -12., z0], [-22., 12., z1]),
        ];
        for (j, mut min, mut max) in strips {
            if p < pass || (p == pass && j < side) {
                continue;
            }
            if p == pass && j == side {
                match j {
                    0 => min[0] = (c[0] + 5.).clamp(-26., 26.),
                    1 => max[1] = (c[1] - 5.).clamp(-12., 12.),
                    2 => max[0] = (c[0] - 5.).clamp(-26., 26.),
                    _ => min[1] = (c[1] + 5.).clamp(-12., 12.),
                }
            }
            let key = (min[0] + max[0] + min[1] + max[1] + min[2] + max[2]) / 2.;
            items.push((key, min, max, STOCK));
        }
    }
    items.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let key = c[0] + c[1] + tip[2] + 14.;
    let mut drawn = false;
    for (k, min, max, tone) in items {
        if !drawn && k > key {
            cutter(f, tip, time, 13., false, 0.);
            drawn = true;
        }
        cuboid(f, min, max, tone);
    }
    if !drawn {
        cutter(f, tip, time, 13., false, 0.);
    }
    heading(
        f,
        &format!("CONTOUR · AROUND THE WALL, PASS {} OF 2", pass + 1),
    );
    caption(f, "AMBER: MATERIAL OUTSIDE THE FINISHED WALL");
}

/// Waterline: loops round a steep boss at constant heights, one level
/// lower each time, each leaving a band of finished wall.
fn waterline(f: &mut Frame, time: f32, phase: f32) {
    let levels = [26., 21., 16., 11.];
    let half = |z: f32| (30. - 18. * (z - 8.) / 22., 18. - 12. * (z - 8.) / 22.);
    let u = progress(phase, 0.1) * 4.;
    let level = (u as usize).min(3);
    let v = u - level as f32;
    // Each loop's sides: front (+Y), right (+X), then the two behind.
    let segments = |z: f32, upto: f32| {
        let (hx, hy) = half(z);
        let (hx, hy) = (hx + 3., hy + 3.);
        let c = [
            [-hx, hy, z],
            [hx, hy, z],
            [hx, -hy, z],
            [-hx, -hy, z],
            [-hx, hy, z],
        ];
        let s = upto * 4.;
        (0..4)
            .filter(|j| (*j as f32) < s)
            .map(|j| (j, c[j], lerp(c[j], c[j + 1], (s - j as f32).min(1.))))
            .collect::<Vec<_>>()
    };
    let done = |i: usize| if i < level { 1. } else { v };
    let tip = segments(levels[level], v)
        .last()
        .map(|s| s.2)
        .unwrap_or_else(|| segments(levels[level], 0.25)[0].1);
    let behind = v * 4. > 2.;
    cuboid(f, [-40., -24., 0.], [40., 24., 8.], METAL);
    for (i, z) in levels.iter().enumerate().take(level + 1) {
        for (j, a, b) in segments(*z, done(i)) {
            if j >= 2 {
                line3(f, a, b, CYAN, 1.5);
            }
        }
    }
    if behind {
        cutter(f, tip, time, 9., false, 0.);
    }
    let (bx, by) = half(8.);
    let (tx, ty) = half(30.);
    quad(
        f,
        &[[-bx, by, 8.], [bx, by, 8.], [tx, ty, 30.], [-tx, ty, 30.]],
        METAL.front,
    );
    quad(
        f,
        &[[bx, -by, 8.], [bx, by, 8.], [tx, ty, 30.], [tx, -ty, 30.]],
        METAL.side,
    );
    quad(
        f,
        &[
            [-tx, -ty, 30.],
            [tx, -ty, 30.],
            [tx, ty, 30.],
            [-tx, ty, 30.],
        ],
        METAL.top,
    );
    // A band of finished wall for each level done.
    for (i, z) in levels.iter().enumerate().take(level + 1) {
        let s = done(i) * 4.;
        let (zt, zb) = (z + 2.5, z - 2.5);
        let (ax, ay) = half(zt);
        let (cx, cy) = half(zb);
        let k = s.min(1.);
        quad(
            f,
            &[
                [-ax, ay, zt],
                [-ax + 2. * ax * k, ay, zt],
                [-cx + 2. * cx * k, cy, zb],
                [-cx, cy, zb],
            ],
            Color::from_rgb(0.80, 0.91, 0.97),
        );
        let k = (s - 1.).clamp(0., 1.);
        if k > 0. {
            quad(
                f,
                &[
                    [ax, ay, zt],
                    [ax, ay - 2. * ay * k, zt],
                    [cx, cy - 2. * cy * k, zb],
                    [cx, cy, zb],
                ],
                Color::from_rgb(0.70, 0.83, 0.91),
            );
        }
        for (j, a, b) in segments(*z, done(i)) {
            if j < 2 {
                line3(f, a, b, CYAN, 1.7);
            }
        }
    }
    if !behind {
        cutter(f, tip, time, 9., false, 0.);
    }
    heading(
        f,
        &format!("WATERLINE · LEVEL {} OF 4, STEPPING DOWN", level + 1),
    );
    caption(f, "BRIGHT: STEEP WALL FINISHED AT EACH Z LEVEL");
}

/// Indexed: the part turns 90° about the rotary axis, is locked there, and
/// its new top (the amber face) is faced. A new setup turns it the same way.
fn indexed(f: &mut Frame, time: f32, phase: f32, setup: bool) {
    let turn = smooth(phase / 0.3);
    let locked = phase >= 0.3;
    let cutting = ((phase - 0.42) / 0.5).clamp(0., 1.);
    let angle = turn * PI / 2.;
    let pose = |a: V| {
        let (y, z) = (a[1], a[2] - 18.);
        [
            a[0],
            y * angle.cos() - z * angle.sin(),
            18. + y * angle.sin() + z * angle.cos(),
        ]
    };
    let hub = |f: &mut Frame, x: f32| {
        let ring: Vec<Point> = (0..24)
            .map(|i| {
                let a = i as f32 * TAU / 24.;
                p([x, 8. * a.cos(), 18. + 8. * a.sin()])
            })
            .collect();
        polygon(f, &ring, Color::from_rgb(0.33, 0.40, 0.46));
    };
    let (ax, ay) = (p([-48., 0., 18.]), p([48., 0., 18.]));
    dashed(f, ax, ay, GOLD, 1.2);
    hub(f, -32.);
    let (x0, x1, y0, y1, z0, z1) = (-28., 28., -12., 12., 0., 36.);
    let v = |x: f32, y: f32, z: f32| pose([x, y, z]);
    let faces = [
        (
            [v(x0, y0, z1), v(x1, y0, z1), v(x1, y1, z1), v(x0, y1, z1)],
            false,
        ),
        (
            [v(x0, y0, z0), v(x0, y1, z0), v(x1, y1, z0), v(x1, y0, z0)],
            false,
        ),
        (
            [v(x0, y1, z0), v(x0, y1, z1), v(x1, y1, z1), v(x1, y1, z0)],
            true,
        ),
        (
            [v(x0, y0, z0), v(x1, y0, z0), v(x1, y0, z1), v(x0, y0, z1)],
            false,
        ),
        (
            [v(x1, y0, z0), v(x1, y1, z0), v(x1, y1, z1), v(x1, y0, z1)],
            false,
        ),
        (
            [v(x0, y0, z0), v(x0, y0, z1), v(x0, y1, z1), v(x0, y1, z0)],
            false,
        ),
    ];
    for (face, marked) in faces {
        lit(f, &face, if marked { STOCK } else { METAL });
    }
    // The faced rows on the new top (z 30 once turned).
    let rows = cutting * 3.;
    let row = (rows as usize).min(2);
    let k = rows - row as f32;
    let x = if row % 2 == 0 {
        -36. + 72. * k
    } else {
        36. - 72. * k
    };
    if locked {
        for i in 0..=row {
            let y = -18. + 12. * i as f32;
            let (a, b) = if i < row {
                (x0, x1)
            } else if row % 2 == 0 {
                (x0, (x + 6.).clamp(x0, x1))
            } else {
                ((x - 6.).clamp(x0, x1), x1)
            };
            if b > a {
                quad(
                    f,
                    &[
                        [a, y, 30.],
                        [b, y, 30.],
                        [b, y + 12., 30.],
                        [a, y + 12., 30.],
                    ],
                    FRESH.top,
                );
            }
        }
    }
    hub(f, 32.);
    // The turn's arrow round the axis, and the lock once it holds.
    let c = p([40., 0., 18.]);
    if !locked {
        let arc: Vec<Point> = (0..=12)
            .map(|i| {
                let a = -PI / 2. + angle * i as f32 / 12.;
                p([40., 14. * a.cos(), 18. + 14. * a.sin()])
            })
            .collect();
        for w in arc.windows(2) {
            line(f, w[0], w[1], CYAN, 2.);
        }
        let end = arc[12];
        let before = arc[11];
        let (dx, dy) = (end.x - before.x, end.y - before.y);
        let len = dx.hypot(dy).max(1e-3);
        let (ux, uy) = (dx / len, dy / len);
        for s in [-1., 1.] {
            line(
                f,
                end,
                Point::new(end.x - 6. * ux + s * 4. * uy, end.y - 6. * uy - s * 4. * ux),
                CYAN,
                2.,
            );
        }
    } else {
        f.stroke(
            &Path::rectangle(Point::new(c.x + 8., c.y - 5.), Size::new(9., 7.)),
            Stroke::default().with_color(GOLD).with_width(1.5),
        );
        text(f, "LOCKED", c.x + 20., c.y - 8., GOLD, 9.);
    }
    let tip = if locked && cutting > 0. {
        [x, -12. + 12. * row as f32, 30.]
    } else {
        [44., -34., 30.]
    };
    cutter(f, tip, time, 13., false, 0.);
    let steps = ["1 ROTATE", "2 LOCK", "3 CUT"];
    let now = if !locked {
        0
    } else if cutting <= 0. {
        1
    } else {
        2
    };
    let mut x = 8.;
    for (i, s) in steps.iter().enumerate() {
        text(f, s, x, 3., if i == now { GOLD } else { DIM }, 10.);
        x += 58.;
    }
    caption(
        f,
        if setup {
            "PART TURNED SO THE AMBER FACE IS ON TOP"
        } else {
            "ROTARY AXIS TURNS THE PART; AMBER FACE IS CUT"
        },
    );
}

/// A cylinder along X from x0 to x1 of radius r round (y 0, z zc), turned
/// by `spin`: its sides this view sees and its +X end.
fn cylinder(f: &mut Frame, x0: f32, x1: f32, r: f32, zc: f32, spin: f32, tone: Tone) {
    let n = 28;
    let at = |x: f32, a: f32| [x, r * a.cos(), zc + r * a.sin()];
    for i in 0..n {
        let a = i as f32 * TAU / n as f32;
        let b = (i + 1) as f32 * TAU / n as f32;
        lit(f, &[at(x0, a), at(x0, b), at(x1, b), at(x1, a)], tone);
    }
    let cap: Vec<Point> = (0..n)
        .map(|i| p(at(x1, i as f32 * TAU / n as f32)))
        .collect();
    polygon(f, &cap, tone.side);
    // Marks along the surface turning with it.
    for k in 0..6 {
        let a = spin + k as f32 * TAU / 6.;
        if a.cos() + VIEW[2] * a.sin() > 0.15 {
            line3(
                f,
                at(x0, a),
                at(x1, a),
                Color::from_rgba(0.1, 0.15, 0.2, 0.45),
                1.2,
            );
        }
    }
}

/// Turning: the bar spins in the chuck and the tool feeds along it,
/// leaving a smaller diameter behind.
fn turning(f: &mut Frame, time: f32, phase: f32) {
    let u = progress(phase, 0.15);
    let x = 44. - 62. * smooth(u);
    let spin = -time * SPIN * 2.;
    let chuck = Tone {
        top: Color::from_rgb(0.42, 0.47, 0.52),
        front: Color::from_rgb(0.32, 0.37, 0.42),
        side: Color::from_rgb(0.27, 0.31, 0.36),
    };
    cylinder(f, -50., -38., 22., 18., spin, chuck);
    cylinder(f, -38., x.min(40.), 14., 18., spin, STOCK);
    if x < 40. {
        cylinder(f, x, 40., 9., 18., spin, FRESH);
    }
    // The tool from the front: a gold insert on a grey shank.
    let tx = x.min(44.);
    cuboid(
        f,
        [tx - 1., 9., 16.],
        [tx + 4., 14., 19.],
        Tone { top: GOLD, ..STOCK },
    );
    cuboid(
        f,
        [tx + 0.5, 14., 15.5],
        [tx + 5., 40., 19.5],
        Tone {
            top: Color::from_rgb(0.75, 0.80, 0.84),
            front: Color::from_rgb(0.62, 0.68, 0.73),
            side: Color::from_rgb(0.55, 0.61, 0.66),
        },
    );
    if u < 1. && x < 40. {
        for i in 0..4 {
            let a = time * 9. + i as f32 * 1.6;
            let c = p([
                tx - 2. + 2. * a.cos(),
                12. + 3. * (i as f32),
                22. + 2. * a.sin(),
            ]);
            f.stroke(
                &Path::circle(c, 1.6),
                Stroke::default().with_color(GOLD).with_width(1.),
            );
        }
    }
    heading(f, "TURNING · BAR SPINS, TOOL FEEDS ALONG");
    caption(f, "AMBER: STOCK   BRIGHT: TURNED TO A SMALLER Ø");
}

/// 5-axis: the cutter follows a curved surface and leans with it.
fn five_axis(f: &mut Frame, time: f32, phase: f32) {
    let surface = |x: f32, y: f32| [x, y, 22. + 9. * (x / 22.).cos() * (1. - (y / 60.).powi(2))];
    cuboid(f, [-40., -24., 0.], [40., 24., 14.], METAL);
    // The curved top as strips across X, then its front edge.
    for i in 0..16 {
        let xa = -40. + 5. * i as f32;
        let xb = xa + 5.;
        let ya = [-24., 24.];
        lit(
            f,
            &[
                surface(xa, ya[0]),
                surface(xb, ya[0]),
                surface(xb, ya[1]),
                surface(xa, ya[1]),
            ],
            METAL,
        );
        quad(
            f,
            &[
                [xa, 24., 14.],
                [xb, 24., 14.],
                surface(xb, 24.),
                surface(xa, 24.),
            ],
            METAL.front,
        );
    }
    let u = progress(phase, 0.1);
    let rows = 3.;
    let r = u * rows;
    let row = (r as usize).min(2);
    let k = r - row as f32;
    let y = -12. + 12. * row as f32;
    for i in 0..row {
        let y = -12. + 12. * i as f32;
        let path: Vec<V> = (0..=40)
            .map(|j| surface(-38. + 76. * j as f32 / 40., y))
            .collect();
        polyline3(f, &path, CYAN, 1.5);
    }
    let sign = if row % 2 == 0 { 1. } else { -1. };
    let x = sign * (-38. + 76. * k);
    let path: Vec<V> = (0..=40)
        .map(|j| surface(sign * (-38. + 76. * k * j as f32 / 40.), y))
        .collect();
    polyline3(f, &path, CYAN, 1.7);
    // The cutter stays square to the surface: its lean is the slope.
    let tilt = (9. / 22. * (x / 22.).sin()).atan();
    tilted_cutter(f, surface(x, y), tilt, time, 10.);
    heading(f, "5-AXIS · TOOL LEANS WITH THE SURFACE");
    caption(f, "CYAN: PASSES ALONG THE CURVED TOP");
}

// Sections and plans: a straight-on view where a depth or a spacing can be
// read off, with the stock in blue and what is cut away in the background.

const SECTION: Color = Color::from_rgb(0.45, 0.56, 0.64);
fn rect(f: &mut Frame, x0: f32, y0: f32, x1: f32, y1: f32, c: Color) {
    if x1 > x0 && y1 > y0 {
        f.fill_rectangle(Point::new(x0, y0), Size::new(x1 - x0, y1 - y0), c);
    }
}
/// A block of stock in section from x0..x1, its top at y0.
fn stock(f: &mut Frame, x0: f32, y0: f32, x1: f32, y1: f32) {
    rect(f, x0, y0, x1, y1, SECTION);
    // Hatching, as a section is drawn.
    let mut x = x0 - (y1 - y0);
    while x < x1 {
        let a = Point::new(x.max(x0), y1 - (x.max(x0) - x));
        let bx = (x + (y1 - y0)).min(x1);
        let b = Point::new(bx, y1 - (bx - x));
        line(f, a, b, Color::from_rgba(0.85, 0.9, 0.94, 0.12), 1.);
        x += 9.;
    }
    line(
        f,
        Point::new(x0, y0),
        Point::new(x1, y0),
        Color::from_rgb(0.75, 0.83, 0.88),
        1.,
    );
}
/// A cutter side on, its bottom centre at (x, y).
fn side_cutter(f: &mut Frame, x: f32, y: f32, time: f32, w: f32, drill: bool) {
    cutter_at(f, Point::new(x, y), (0., -1.), time, w, drill, 0.);
}

/// Stepdown: a pocket in section, cut one layer at a time.
fn stepdown(f: &mut Frame, time: f32, phase: f32) {
    let (left, right, top) = (74., 204., 46.);
    let depth = 18.;
    stock(f, 24., top, 254., 132.);
    let u = progress(phase, 0.12) * 4.;
    let layer = (u as usize).min(3);
    let k = u - layer as f32;
    let x = if layer % 2 == 0 {
        84. + 110. * k
    } else {
        194. - 110. * k
    };
    let above = top + depth * layer as f32;
    let floor = above + depth;
    rect(f, left, top, right, above, BACKGROUND);
    if layer % 2 == 0 {
        rect(f, left, above, x + 10., floor, BACKGROUND);
    } else {
        rect(f, x - 10., above, right, floor, BACKGROUND);
    }
    for i in 1..=4 {
        let y = top + depth * i as f32;
        dashed(f, Point::new(left, y), Point::new(right, y), DIM, 1.);
    }
    side_cutter(f, x, floor, time, 20., false);
    dimension(
        f,
        Point::new(222., top),
        Point::new(222., top + depth),
        GOLD,
    );
    text(f, "STEPDOWN", 228., top + 3., GOLD, 9.);
    heading(f, "SECTION · ONE LAYER DEEPER AT A TIME");
    caption(
        f,
        &format!("LAYER {} OF 4 · EACH LAYER ONE STEPDOWN DEEP", layer + 1),
    );
}

/// Stepover: from above, overlapping passes the spacing apart.
fn stepover(f: &mut Frame, time: f32, phase: f32) {
    let (x0, x1, y0, y1) = (76., 266., 24., 138.);
    rect(f, x0, y0, x1, y1, METAL.top);
    let rows: [f32; 5] = [40., 60., 80., 100., 120.];
    let r = 16.;
    let u = progress(phase, 0.12) * 5.;
    let row = (u as usize).min(4);
    let k = u - row as f32;
    let x = if row % 2 == 0 {
        x0 + r + (x1 - x0 - 2. * r) * k
    } else {
        x1 - r - (x1 - x0 - 2. * r) * k
    };
    let band = Color::from_rgba(0.80, 0.92, 0.98, 0.5);
    for (i, y) in rows.iter().enumerate().take(row + 1) {
        let (a, b) = if i < row {
            (x0, x1)
        } else if row % 2 == 0 {
            (x0, x + r)
        } else {
            (x - r, x1)
        };
        rect(f, a, (y - r).max(y0), b, (y + r).min(y1), band);
    }
    for y in rows {
        dashed(
            f,
            Point::new(x0, y),
            Point::new(x1, y),
            Color::from_rgba(0.1, 0.2, 0.3, 0.6),
            1.,
        );
    }
    let c = Point::new(x, rows[row]);
    f.fill(&Path::circle(c, r), Color::from_rgba(0.2, 0.3, 0.38, 0.85));
    f.stroke(
        &Path::circle(c, r),
        Stroke::default().with_color(GOLD).with_width(1.6),
    );
    for tooth in 0..4 {
        let a = time * SPIN + tooth as f32 * TAU / 4.;
        line(
            f,
            Point::new(c.x + 7. * a.cos(), c.y + 7. * a.sin()),
            Point::new(c.x + r * a.cos(), c.y + r * a.sin()),
            GOLD,
            2.,
        );
    }
    dimension(f, Point::new(64., rows[0]), Point::new(64., rows[1]), GOLD);
    for y in [rows[0], rows[1]] {
        line(f, Point::new(58., y), Point::new(x0, y), GOLD, 1.);
    }
    text(f, "STEP-", 18., rows[0] + 1., GOLD, 9.);
    text(f, "OVER", 21., rows[0] + 11., GOLD, 9.);
    dimension(
        f,
        Point::new(c.x - r, c.y - r - 4.),
        Point::new(c.x + r, c.y - r - 4.),
        CYAN,
    );
    heading(f, "TOP VIEW · SPACING BETWEEN PASSES");
    caption(f, "PASSES OVERLAP; CYAN: TOOL DIAMETER");
}

/// Allowance: roughing stops short of the finished wall and floor and
/// leaves an amber skin for a finishing pass.
fn allowance(f: &mut Frame, time: f32, phase: f32) {
    let (top, left, right, bottom) = (44., 70., 208., 116.);
    let skin = 11.;
    stock(f, 24., top, 254., 132.);
    let u = progress(phase, 0.15) * 3.;
    let layer = (u as usize).min(2);
    let k = u - layer as f32;
    let depth = (bottom - skin - top) / 3.;
    let above = top + depth * layer as f32;
    let floor = above + depth;
    let (a, b) = (left + skin, right - skin);
    let x = if layer % 2 == 0 {
        a + 10. + (b - a - 20.) * k
    } else {
        b - 10. - (b - a - 20.) * k
    };
    rect(f, a, top, b, above, BACKGROUND);
    if layer % 2 == 0 {
        rect(f, a, above, x + 10., floor, BACKGROUND);
    } else {
        rect(f, x - 10., above, b, floor, BACKGROUND);
    }
    // The skin left beside what is cut so far.
    let cut = floor.min(bottom - skin);
    let amber = STOCK.top;
    rect(f, left, top, a, cut, amber);
    rect(f, b, top, right, cut, amber);
    if layer == 2 {
        rect(f, a, bottom - skin, x + 10., bottom, amber);
    }
    let finished = Path::new(|p| {
        p.move_to(Point::new(left, top));
        p.line_to(Point::new(left, bottom));
        p.line_to(Point::new(right, bottom));
        p.line_to(Point::new(right, top));
    });
    f.stroke(&finished, Stroke::default().with_color(CYAN).with_width(1.));
    side_cutter(f, x, floor, time, 20., false);
    dimension(f, Point::new(b, 72.), Point::new(right, 72.), Color::WHITE);
    text(f, "STOCK TO LEAVE", 150., 30., GOLD, 9.);
    line(
        f,
        Point::new(210., 40.),
        Point::new(right - 3., 64.),
        GOLD,
        1.,
    );
    heading(f, "SECTION · ROUGHING STOPS SHORT");
    caption(f, "AMBER: SKIN LEFT   CYAN: FINISHED SIZE");
}

/// Ramp: in section, the cutter enters on a slope down to depth, then cuts.
fn ramp(f: &mut Frame, time: f32, phase: f32) {
    let top = 50.;
    let path = [[40., top, 0.], [176., 104., 0.], [236., 104., 0.]];
    stock(f, 20., top, 258., 132.);
    let u = progress(phase, 0.12);
    let (at, _) = along(&path, u);
    let bottom = |x: f32| {
        if x <= path[0][0] {
            top
        } else if x <= path[1][0] {
            top + (x - path[0][0]) * (104. - top) / (path[1][0] - path[0][0])
        } else {
            104.
        }
    };
    let half = 9.;
    let mut cut = vec![Point::new(path[0][0] - half, top)];
    let n = 40;
    for i in 0..=n {
        let x = path[0][0] - half + (at[0] + 2. * half - path[0][0]) * i as f32 / n as f32;
        cut.push(Point::new(x, bottom((x + half).min(at[0])).max(top)));
    }
    cut.push(Point::new(at[0] + half, top));
    if at[0] > path[0][0] {
        polygon(f, &cut, BACKGROUND);
    }
    dashed(
        f,
        Point::new(path[0][0], top),
        Point::new(path[1][0], top),
        GOLD,
        1.,
    );
    dashed(
        f,
        Point::new(path[0][0], top),
        Point::new(path[1][0], 104.),
        CYAN,
        1.,
    );
    let slope = ((104. - top) / (path[1][0] - path[0][0])).atan();
    let arc: Vec<Point> = (0..=8)
        .map(|i| {
            let a = slope * i as f32 / 8.;
            Point::new(path[0][0] + 46. * a.cos(), top + 46. * a.sin())
        })
        .collect();
    for w in arc.windows(2) {
        line(f, w[0], w[1], GOLD, 1.5);
    }
    text(f, "RAMP ANGLE", 92., 36., GOLD, 9.);
    side_cutter(f, at[0], at[1], time, 2. * half, false);
    heading(f, "SECTION · ENTERS ON A SLOPE");
    caption(f, "A SMALLER ANGLE TAKES LONGER TO REACH DEPTH");
}

/// Top and bottom offsets: in section, where the cut starts against the
/// stock top and where it stops against the selected bottom.
fn offsets(f: &mut Frame, time: f32, phase: f32) {
    let (top, bottom) = (56., 106.);
    let (left, right) = (84., 196.);
    stock(f, 20., top, 258., 134.);
    let offset = 10. * (phase * TAU).sin();
    let floor = bottom - offset;
    rect(f, left, top, right, floor, BACKGROUND);
    dashed(
        f,
        Point::new(20., bottom),
        Point::new(258., bottom),
        CYAN,
        1.3,
    );
    text(f, "SELECTED BOTTOM", 24., bottom + 3., CYAN, 9.);
    line(
        f,
        Point::new(left, floor),
        Point::new(right, floor),
        GOLD,
        2.,
    );
    let start = top - 14.;
    dashed(
        f,
        Point::new(left - 30., start),
        Point::new(right + 30., start),
        GOLD,
        1.3,
    );
    text(f, "CUT STARTS (TOP OFFSET)", 24., start - 13., GOLD, 9.);
    dimension(f, Point::new(214., start), Point::new(214., top), GOLD);
    let x = 140. + 40. * (phase * TAU * 2.).sin();
    side_cutter(f, x, floor, time, 18., false);
    heading(
        f,
        if offset >= 0. {
            "BOTTOM OFFSET +: THE FLOOR IS LEFT HIGHER"
        } else {
            "BOTTOM OFFSET −: THE CUT GOES BELOW IT"
        },
    );
    caption(f, "CYAN: GEOMETRY   AMBER: WITH THE OFFSET");
}

/// The setup's X, Y and Z at the bottom left, `bright` in gold.
fn axes(f: &mut Frame, bright: Option<usize>) {
    let o = [-36., 31., 0.];
    for (i, name) in ["X", "Y", "Z"].iter().enumerate() {
        let mut e = o;
        e[i] += 14.;
        let color = if bright == Some(i) { GOLD } else { TEXT };
        line3(f, o, e, color, if bright == Some(i) { 2. } else { 1. });
        // Each name beyond its axis' end, clear of the line.
        let q = p(e);
        let (dx, dy) = [(3., -4.), (-9., -4.), (3., -8.)][i];
        text(f, name, q.x + dx, q.y + dy, color, 9.);
    }
}
/// Stock offset: extra material on one side of the part, shown as an amber
/// slab that grows and shrinks on that side only.
fn stock_offset(f: &mut Frame, phase: f32, axis: usize, positive: bool) {
    let part_min = [-30., -18., 0.];
    let part_max = [30., 18., 22.];
    let offset = 5. + 9. * (0.5 - 0.5 * (phase * TAU).cos());
    let (mut min, mut max) = (part_min, part_max);
    if positive {
        min[axis] = part_max[axis];
        max[axis] = part_max[axis] + offset;
    } else {
        max[axis] = part_min[axis];
        min[axis] = part_min[axis] - offset;
    }
    let behind = !positive;
    if behind {
        cuboid(f, min, max, STOCK);
    }
    cuboid(f, part_min, part_max, METAL);
    if !behind {
        cuboid(f, min, max, STOCK);
    }
    axes(f, Some(axis));
    let side = match (axis, positive) {
        (0, false) => "LEFT (−X)",
        (0, true) => "RIGHT (+X)",
        (1, false) => "FRONT (−Y)",
        (1, true) => "BACK (+Y)",
        (2, false) => "BOTTOM (−Z)",
        _ => "TOP (+Z)",
    };
    heading(f, &format!("EXTRA STOCK ON THE {side} SIDE ONLY"));
    caption(f, "BLUE: PART   AMBER: EXTRA STOCK ON THAT SIDE");
}

/// Pocket entry: the stock is drawn see-through so the descent shows. A
/// ramp goes down along the outline and then round it at depth; a plunge
/// goes straight down at a corner first.
fn entry(f: &mut Frame, time: f32, phase: f32, ramp: bool) {
    let corners = [
        [-26., -13.],
        [26., -13.],
        [26., 13.],
        [-26., 13.],
        [-26., -13.],
    ];
    let round = |z: &dyn Fn(f32) -> f32| -> Vec<V> {
        (0..=48)
            .map(|i| {
                let t = i as f32 / 48.;
                let side = (t * 4.).min(3.9999);
                let n = side as usize;
                let u = side - n as f32;
                [
                    corners[n][0] + (corners[n + 1][0] - corners[n][0]) * u,
                    corners[n][1] + (corners[n + 1][1] - corners[n][1]) * u,
                    z(t),
                ]
            })
            .collect()
    };
    let mut path = if ramp {
        round(&|t| 24. - 11. * t)
    } else {
        vec![[-26., -13., 32.], [-26., -13., 13.]]
    };
    path.extend(round(&|_| 13.).into_iter().skip(1));
    let u = progress(phase, 0.1);
    let (at, n) = along(&path, u);
    let mut done = path[..n].to_vec();
    done.push(at);
    let top = 24.;
    let split = |a: V, b: V| {
        if (a[2] - top) * (b[2] - top) < 0. {
            Some(lerp(a, b, (top - a[2]) / (b[2] - a[2])))
        } else {
            None
        }
    };
    // Inside the stock first, then the stock over it, then what is above.
    for inside in [true, false] {
        if !inside {
            let glass = |c: Color| Color { a: 0.62, ..c };
            cuboid(
                f,
                [-40., -24., 0.],
                [40., 24., top],
                Tone {
                    top: glass(METAL.top),
                    front: glass(METAL.front),
                    side: glass(METAL.side),
                },
            );
        }
        for w in path.windows(2) {
            let (a, b) = (w[0], w[1]);
            let pieces = match split(a, b) {
                Some(m) => vec![(a, m), (m, b)],
                None => vec![(a, b)],
            };
            for (a, b) in pieces {
                if ((a[2] + b[2]) / 2. < top) == inside {
                    line3(f, a, b, DIM, 1.);
                }
            }
        }
        for w in done.windows(2) {
            let (a, b) = (w[0], w[1]);
            let pieces = match split(a, b) {
                Some(m) => vec![(a, m), (m, b)],
                None => vec![(a, b)],
            };
            for (a, b) in pieces {
                if ((a[2] + b[2]) / 2. < top) == inside {
                    line3(f, a, b, CYAN, 1.8);
                }
            }
        }
        cutter(f, at, time, 10., false, if inside { 0. } else { top });
    }
    heading(
        f,
        if ramp {
            "RAMP · DOWN ALONG THE OUTLINE, THEN ROUND"
        } else {
            "PLUNGE · STRAIGHT DOWN, THEN ROUND"
        },
    );
    caption(f, "STOCK SEEN THROUGH · CYAN: THE CUTTER'S PATH");
}

#[derive(Clone, Copy, PartialEq)]
enum Move {
    Rapid,
    Feed,
    Dwell,
}
/// A drill cycle: on the left a section of the hole as it is drilled, on
/// the right the drill's depth against time, where pecks, retracts and a
/// dwell show as their own shapes.
fn drill_cycle(f: &mut Frame, time: f32, phase: f32, cycle: u8) {
    use Move::*;
    let clear = 36.;
    let moves: Vec<(f32, Move)> = match cycle {
        1 => vec![
            (27., Rapid),
            (19., Feed),
            (21.5, Rapid),
            (12., Feed),
            (14.5, Rapid),
            (5., Feed),
            (clear, Rapid),
        ],
        2 => vec![
            (27., Rapid),
            (19., Feed),
            (31., Rapid),
            (20., Rapid),
            (12., Feed),
            (31., Rapid),
            (13., Rapid),
            (5., Feed),
            (clear, Rapid),
        ],
        3 => vec![(27., Rapid), (5., Feed), (5., Dwell), (clear, Rapid)],
        4 => vec![
            (27., Rapid),
            (24., Feed),
            (8., Feed),
            (8., Dwell),
            (clear, Rapid),
        ],
        _ => vec![(27., Rapid), (5., Feed), (clear, Rapid)],
    };
    // Each move's share of the time: feeds slow, rapids quick.
    let mut z = clear;
    let mut timeline = vec![(0., clear, Rapid)];
    let mut t = 0.;
    for (to, kind) in &moves {
        t += match kind {
            Rapid => (to - z).abs() / 40.,
            Feed => (to - z).abs() / 9.,
            Dwell => 0.9,
        };
        timeline.push((t, *to, *kind));
        z = *to;
    }
    let total = t;
    let now = progress(phase, 0.12) * total;
    let at = |now: f32| {
        for w in timeline.windows(2) {
            if now <= w[1].0 {
                let k = ((now - w[0].0) / (w[1].0 - w[0].0).max(1e-6)).clamp(0., 1.);
                return (w[0].1 + (w[1].1 - w[0].1) * k, w[1].2, k);
            }
        }
        (clear, Rapid, 1.)
    };
    let (z, _, _) = at(now);
    let y = |z: f32| 50. + (24. - z) * 3.3;
    let helix = cycle == 4;
    // The deepest point so far is the hole's bottom.
    let mut deepest: f32 = 24.;
    for w in timeline.windows(2) {
        if w[0].0 > now {
            break;
        }
        let k = ((now - w[0].0) / (w[1].0 - w[0].0).max(1e-6)).min(1.);
        deepest = deepest.min(w[0].1 + (w[1].1 - w[0].1) * k);
    }
    let cx = 64.;
    stock(f, 14., y(24.), 112., 132.);
    let (hw, dw) = if helix { (30., 14.) } else { (13., 12.) };
    if deepest < 24. {
        rect(
            f,
            cx - hw / 2.,
            y(24.),
            cx + hw / 2.,
            y(deepest),
            BACKGROUND,
        );
        if !helix {
            polygon(
                f,
                &[
                    Point::new(cx - hw / 2., y(deepest)),
                    Point::new(cx + hw / 2., y(deepest)),
                    Point::new(cx, y(deepest) + 5.),
                ],
                BACKGROUND,
            );
        }
    }
    // The helix: round the hole while going down, once more at the bottom.
    let angle = |now: f32| {
        if !helix {
            return 0.;
        }
        let (start, bottom) = (timeline[2].0, timeline[4].0);
        ((now - start) / (bottom - start)).clamp(0., 1.) * TAU * 4.
    };
    let offset = |now: f32| (hw - dw) / 2. * angle(now).sin();
    if helix {
        let points: Vec<Point> = (0..=120)
            .map(|i| now * i as f32 / 120.)
            .filter(|t| *t >= timeline[2].0 && *t <= timeline[4].0)
            .map(|t| Point::new(cx + offset(t), y(at(t).0)))
            .collect();
        for w in points.windows(2) {
            line(f, w[0], w[1], CYAN, 1.2);
        }
    }
    cutter_at(
        f,
        Point::new(cx + offset(now), y(z)),
        (0., -1.),
        time,
        dw,
        !helix,
        0.,
    );
    // The depth against time, on the same heights as the section.
    let (g0, g1) = (130., 268.);
    line(f, Point::new(g0, y(clear)), Point::new(g0, y(0.)), TEXT, 1.);
    line(f, Point::new(g0, y(0.)), Point::new(g1, y(0.)), TEXT, 1.);

    text(f, "DEPTH", g0 + 4., y(clear) - 1., TEXT, 8.);
    dashed(f, Point::new(14., y(24.)), Point::new(g1, y(24.)), DIM, 1.);
    text(f, "STOCK TOP", 16., y(24.) + 3., TEXT, 8.);
    let gx = |t: f32| g0 + (g1 - g0) * t / total;
    for w in timeline.windows(2) {
        if w[0].0 > now {
            break;
        }
        let end = now.min(w[1].0);
        let (ze, _, _) = at(end);
        let color = match w[1].2 {
            Feed => CYAN,
            Rapid => GOLD,
            Dwell => Color::WHITE,
        };
        line(
            f,
            Point::new(gx(w[0].0), y(w[0].1)),
            Point::new(gx(end), y(ze)),
            color,
            if w[1].2 == Dwell { 3. } else { 1.6 },
        );
        if w[1].2 == Dwell {
            text(
                f,
                if helix { "ONCE ROUND" } else { "DWELL" },
                gx(w[0].0) - 4.,
                y(w[0].1) + 4.,
                Color::WHITE,
                8.,
            );
        }
    }
    f.fill(&Path::circle(Point::new(gx(now), y(z)), 2.6), GOLD);
    text(
        f,
        "CYAN: FEED · AMBER: RAPID · TIME →",
        g0 + 2.,
        y(0.) + 2.,
        TEXT,
        8.,
    );
    caption(
        f,
        match cycle {
            1 => "PECK · SHORT RETRACT TO BREAK THE CHIP",
            2 => "PECK · FULL RETRACT OUT OF THE HOLE EACH TIME",
            3 => "FEED · DWELL AT THE BOTTOM · RETRACT",
            4 => "HELIX DOWN · ONCE ROUND AT THE BOTTOM · OUT",
            _ => "ONE FEED TO DEPTH · RETRACT",
        },
    );
}
