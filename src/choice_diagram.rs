//! Small native illustrations driven by a specific choice, never by a placeholder.
use crate::help::Diagram;
use iced::{Color, Point, Size, widget::canvas};
const CYAN: Color = Color::from_rgb(0.26, 0.83, 0.92);
const GOLD: Color = Color::from_rgb(1., 0.75, 0.33);
/// The spindle's turn in radians per second: about 2, but a whole number of
/// turns (19) in the animations' period, so the wrapped clock does not jump.
pub const SPIN: f32 = std::f32::consts::TAU * 19. / 60.;
fn line(f: &mut canvas::Frame, a: Point, b: Point, color: Color, width: f32) {
    f.stroke(
        &canvas::Path::line(a, b),
        canvas::Stroke::default()
            .with_color(color)
            .with_width(width),
    );
}
fn label(f: &mut canvas::Frame, s: &str, x: f32, y: f32) {
    f.fill_text(canvas::Text {
        content: s.into(),
        position: Point::new(x, y),
        color: CYAN,
        size: 10.into(),
        ..Default::default()
    });
}
fn point(p: [f32; 3]) -> Point {
    Point::new(
        139. + (p[0] - p[1]) * 1.45,
        124. + (p[0] + p[1]) * 0.5 - p[2] * 1.5,
    )
}
fn poly(f: &mut canvas::Frame, p: &[[f32; 3]], color: Color) {
    let path = canvas::Path::new(|b| {
        b.move_to(point(p[0]));
        for p in &p[1..] {
            b.line_to(point(*p));
        }
        b.close();
    });
    f.fill(&path, color);
}
fn block(f: &mut canvas::Frame) {
    poly(
        f,
        &[
            [-40., -22., 4.],
            [40., -22., 4.],
            [40., -22., 24.],
            [-40., -22., 24.],
        ],
        Color::from_rgb(0.37, 0.45, 0.5),
    );
    poly(
        f,
        &[
            [40., -22., 4.],
            [40., 22., 4.],
            [40., 22., 24.],
            [40., -22., 24.],
        ],
        Color::from_rgb(0.28, 0.36, 0.42),
    );
    poly(
        f,
        &[
            [-40., -22., 24.],
            [40., -22., 24.],
            [40., 22., 24.],
            [-40., 22., 24.],
        ],
        Color::from_rgb(0.46, 0.56, 0.62),
    );
}
fn cutter(f: &mut canvas::Frame, p: [f32; 3]) {
    let top = [p[0], p[1], p[2] + 22.];
    line(f, point(p), point(top), GOLD, 9.);
    line(
        f,
        point(top),
        point([p[0], p[1], p[2] + 30.]),
        Color::from_rgb(0.68, 0.76, 0.81),
        17.,
    );
}
fn trace(f: &mut canvas::Frame, path: &[([f32; 3], bool)], phase: f32) {
    for pair in path.windows(2) {
        line(
            f,
            point(pair[0].0),
            point(pair[1].0),
            if pair[1].1 { CYAN } else { GOLD },
            1.7,
        );
    }
    let t = phase * (path.len() - 1) as f32;
    let i = (t as usize).min(path.len() - 2);
    let u = t - i as f32;
    cutter(
        f,
        std::array::from_fn(|k| path[i].0[k] + (path[i + 1].0[k] - path[i].0[k]) * u),
    );
}
pub fn draw(d: Diagram, f: &mut canvas::Frame, time: f32) -> bool {
    let phase = (time / 5.).fract();
    match d {
        Diagram::FixtureClearance => {
            // Side elevation: the gap starts at the holder's outside edge,
            // which is closer to the jaw than the narrower cutter. Only the
            // highlight and flute marks animate; the requested gap stays
            // fixed. It is the setup's note, which nothing checks, and the
            // heading and the dimension say so.
            label(f, "REQUESTED GAP · NOT CHECKED", 6., 2.);
            let pulse = 0.12 + 0.06 * (time * SPIN).sin();
            f.fill_rectangle(
                Point::new(137., 59.),
                Size::new(54., 79.),
                Color { a: pulse, ..GOLD },
            );
            f.fill_rectangle(
                Point::new(14., 127.),
                Size::new(163., 12.),
                Color::from_rgb(0.39, 0.50, 0.57),
            );
            f.fill_rectangle(
                Point::new(191., 64.),
                Size::new(68., 75.),
                Color::from_rgb(0.36, 0.45, 0.52),
            );
            f.fill_rectangle(
                Point::new(191., 64.),
                Size::new(8., 75.),
                Color::from_rgb(0.52, 0.61, 0.66),
            );
            label(f, "Fixture", 207., 91.);
            f.fill_rectangle(
                Point::new(105., 58.),
                Size::new(32., 43.),
                Color::from_rgb(0.65, 0.73, 0.78),
            );
            line(f, Point::new(111., 60.), Point::new(111., 98.), CYAN, 2.);
            f.fill_rectangle(Point::new(115., 101.), Size::new(12., 22.), GOLD);
            // Animated flute highlights remain inside the cutter silhouette.
            for i in 0..4 {
                let y = 102. + ((i as f32 * 6. + phase * 24.) % 18.);
                line(
                    f,
                    Point::new(116., y + 3.),
                    Point::new(126., y),
                    Color::from_rgb(0.62, 0.43, 0.16),
                    1.5,
                );
            }
            label(f, "Holder", 35., 75.);
            line(f, Point::new(73., 82.), Point::new(102., 82.), CYAN, 1.);
            label(f, "Cutter", 66., 107.);
            label(f, "Requested", 141., 26.);
            // Dimension extension lines terminate at the actual nearest faces.
            for (x, bottom) in [(137., 101.), (191., 64.)] {
                line(f, Point::new(x, 43.), Point::new(x, bottom), GOLD, 1.);
            }
            line(f, Point::new(137., 48.), Point::new(191., 48.), GOLD, 1.5);
            for (x, sign) in [(137., 1.), (191., -1.)] {
                for dy in [-3., 3.] {
                    line(
                        f,
                        Point::new(x, 48.),
                        Point::new(x + sign * 5., 48. + dy),
                        GOLD,
                        1.5,
                    );
                }
            }
            label(f, "Side view · illustrative dimensions", 45., 143.);
        }
        Diagram::Spindle(cw) => {
            let c = Point::new(139., 77.);
            let spin = if cw { 1. } else { -1. };
            f.fill(
                &canvas::Path::circle(c, 35.),
                Color::from_rgb(0.20, 0.30, 0.38),
            );
            f.stroke(
                &canvas::Path::circle(c, 35.),
                canvas::Stroke::default().with_color(GOLD).with_width(2.),
            );
            for i in 0..6 {
                let a = spin * time * SPIN + i as f32 * std::f32::consts::TAU / 6.;
                line(
                    f,
                    Point::new(c.x + 20. * a.cos(), c.y + 20. * a.sin()),
                    Point::new(c.x + 35. * a.cos(), c.y + 35. * a.sin()),
                    GOLD,
                    3.,
                );
            }
            let a = spin * phase * std::f32::consts::TAU;
            let end = Point::new(c.x + 49. * a.cos(), c.y + 49. * a.sin());
            let tangent = Point::new(-spin * a.sin(), spin * a.cos());
            line(
                f,
                Point::new(end.x - 15. * tangent.x, end.y - 15. * tangent.y),
                end,
                CYAN,
                2.,
            );
            for sign in [-1., 1.] {
                line(
                    f,
                    end,
                    Point::new(
                        end.x - 7. * tangent.x + sign * 4. * tangent.y,
                        end.y - 7. * tangent.y - sign * 4. * tangent.x,
                    ),
                    CYAN,
                    2.,
                );
            }
            label(
                f,
                if cw {
                    "CLOCKWISE · CW"
                } else {
                    "COUNTERCLOCKWISE · CCW"
                },
                if cw { 85. } else { 54. },
                4.,
            );
            label(f, "VIEWED FROM SPINDLE TOWARD PART", 32., 140.);
        }
        Diagram::FacingPattern(both) => {
            block(f);
            let mut path = vec![];
            for row in 0..4 {
                let y = -16. + row as f32 * 10.;
                let sign = if both && row % 2 == 1 { -1. } else { 1. };
                let start = [-sign * 35., y, 25.];
                let end = [sign * 35., y, 25.];
                if !both && row > 0 {
                    let last = path.last().unwrap();
                    let (p, _): &([f32; 3], bool) = last;
                    path.push(([p[0], p[1], 38.], false));
                    path.push(([start[0], start[1], 38.], false));
                }
                path.push((start, true));
                path.push((end, true));
            }
            trace(f, &path, phase);
            label(
                f,
                if both {
                    "ALTERNATING ROWS · MIXED CUT"
                } else {
                    "ONE CUT DIRECTION · RETRACT TO RETURN"
                },
                8.,
                4.,
            );
        }
        Diagram::FacingLink(down) => {
            block(f);
            let mut path = vec![([-34., -14., 25.], true), ([34., -14., 25.], true)];
            if down {
                path.push(([34., 14., 25.], true));
            } else {
                path.extend([
                    ([34., -14., 43.], false),
                    ([34., 14., 43.], false),
                    ([34., 14., 25.], true),
                ]);
            }
            path.push(([-34., 14., 25.], true));
            trace(f, &path, phase);
            label(
                f,
                if down {
                    "FEED LINK AT CUTTING DEPTH"
                } else {
                    "LIFT · REPOSITION · FEED DOWN"
                },
                16.,
                4.,
            );
        }
        Diagram::Entry(ramp) => {
            block(f);
            let mut path = vec![];
            if ramp {
                for i in 0..=48 {
                    let t = i as f32 / 48.;
                    let side = (t * 4.).min(3.9999);
                    let n = side as usize;
                    let u = side - n as f32;
                    let corners = [
                        [-26., -13.],
                        [26., -13.],
                        [26., 13.],
                        [-26., 13.],
                        [-26., -13.],
                    ];
                    path.push((
                        [
                            corners[n][0] + (corners[n + 1][0] - corners[n][0]) * u,
                            corners[n][1] + (corners[n + 1][1] - corners[n][1]) * u,
                            30. - t * 17.,
                        ],
                        true,
                    ));
                }
            } else {
                path = vec![
                    ([-26., -13., 35.], true),
                    ([-26., -13., 13.], true),
                    ([26., -13., 13.], true),
                ];
            }
            trace(f, &path, phase);
            label(
                f,
                if ramp {
                    "DESCEND ALONG THE CLOSED PROFILE"
                } else {
                    "VERTICAL PLUNGE · THEN CUT"
                },
                10.,
                4.,
            );
        }
        Diagram::DrillCycle(cycle) => {
            block(f);
            let mut path = vec![([0., 0., 40.], false), ([0., 0., 26.], true)];
            if cycle == 1 || cycle == 2 {
                for z in [19., 12., 5.] {
                    path.push(([0., 0., z], true));
                    path.push(([0., 0., if cycle == 1 { z + 2. } else { 38. }], false));
                    if cycle == 2 && z > 5. {
                        path.push(([0., 0., z + 1.], false));
                    }
                }
            } else {
                path.push(([0., 0., 5.], true));
                if cycle == 3 {
                    path.push(([0., 0., 5.], true));
                    path.push(([0., 0., 5.], true));
                }
            }
            path.push(([0., 0., 40.], false));
            trace(f, &path, phase);
            label(
                f,
                match cycle {
                    1 => "PECK · SHORT RETRACT",
                    2 => "PECK · FULL CLEARANCE RETRACT",
                    3 => "FEED · BOTTOM DWELL · RETRACT",
                    _ => "ONE FEED TO DEPTH · RETRACT",
                },
                14.,
                4.,
            );
            if cycle == 3 {
                label(f, "DWELL AT THE BOTTOM", 55., 139.);
            }
        }
        Diagram::Containment(side) => {
            let c = Point::new(139., 77.);
            let rx = 67.;
            let ry = 34.;
            f.fill_rectangle(
                Point::new(c.x - rx, c.y - ry),
                Size::new(rx * 2., ry * 2.),
                Color::from_rgb(0.40, 0.48, 0.52),
            );
            let corners = [
                Point::new(c.x - rx, c.y - ry),
                Point::new(c.x + rx, c.y - ry),
                Point::new(c.x + rx, c.y + ry),
                Point::new(c.x - rx, c.y + ry),
            ];
            for i in 0..4 {
                line(f, corners[i], corners[(i + 1) % 4], Color::WHITE, 1.);
            }
            let r = 12.;
            let rx = rx + side as f32 * r;
            let ry = ry + side as f32 * r;
            let corners = [
                Point::new(c.x - rx, c.y - ry),
                Point::new(c.x + rx, c.y - ry),
                Point::new(c.x + rx, c.y + ry),
                Point::new(c.x - rx, c.y + ry),
            ];
            for i in 0..4 {
                line(f, corners[i], corners[(i + 1) % 4], CYAN, 1.5);
            }
            let t = phase * 4.;
            let i = t as usize;
            let u = t - i as f32;
            let a = corners[i];
            let b = corners[(i + 1) % 4];
            let p = Point::new(a.x + (b.x - a.x) * u, a.y + (b.y - a.y) * u);
            f.stroke(
                &canvas::Path::circle(p, r),
                canvas::Stroke::default().with_color(GOLD).with_width(2.),
            );
            f.fill(&canvas::Path::circle(p, 2.), GOLD);
            label(
                f,
                match side {
                    -1 => "CUTTER EDGE STAYS INSIDE",
                    0 => "CENTER ON BOUNDARY",
                    _ => "CENTER MAY EXTEND OUTSIDE",
                },
                35.,
                4.,
            );
            label(f, "WHITE: BOUNDARY   CYAN: CENTER PATH", 20., 140.);
        }
        _ => return false,
    }
    true
}
