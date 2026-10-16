//! The machine on the Control page: a flat-shaded model of the Jingyan
//! CNC4030 that moves with the controller's machine position. Its own small
//! canvas, drawn from scratch each frame (under 200 polygons), not the Work
//! page's viewport.
use crate::Message;
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, mouse, widget::canvas};

// ASSUMED (LIO-47) from a similar 800 W CNC4030's spec sheet, NOT measured on
// the machine: measure it and correct these; nothing else here holds machine
// dimensions. World coordinates are the tool tip's machine coordinates in mm:
// machine zero is X at the left end, Y at the front end, Z at the top.
/// Travel X × Y × Z (Z −55…0).
pub const TRAVEL: [f32; 3] = [390., 280., 55.];
/// The T-slot table, X × Y; the travel is centred on it.
const TABLE: [f32; 2] = [530., 320.];
/// Table top to the underside of the gantry beam.
const BEAM_CLEARANCE: f32 = 70.;
/// Table top to the tool tip at Z top, with the nominal tool.
const TIP_HEIGHT: f32 = 60.;
/// Spindle body diameter and length.
const SPINDLE: [f32; 2] = [65., 185.];
/// Nominal tool: diameter and length below the collet nut.
const TOOL: [f32; 2] = [6., 30.];

/// The model for one frame: where the tool tip is, whether the machine
/// position is set, and the box the tool tip may travel in.
pub struct MachineView {
    pub pos: [f32; 3],
    pub known: bool,
    /// The tool-tip travel box: the profile's soft limits when X, Y and Z
    /// all have them, otherwise the assumed travel.
    pub envelope: [[f32; 3]; 2],
    /// The envelope is the assumed travel, not the profile's soft limits:
    /// drawn dashed and captioned as assumed, whether or not the position is
    /// set (#73).
    pub assumed: bool,
    /// The simulator's block, which stands for the stock (min and max).
    pub block: Option<[[f32; 3]; 2]>,
    /// The active work zero (machine mm), marked once the position is set.
    pub zero: [f32; 3],
}

/// What a caption says, which sets its colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Caption {
    /// The profile's soft-limit box.
    Travel,
    /// The assumed travel box.
    Assumed,
    /// The model does not show the machine.
    Unset,
    /// The tool tip and the work zero.
    Zero,
}

impl MachineView {
    /// The travel box from the profile's soft limits, or the assumed travel,
    /// and whether it is the assumed one.
    pub fn envelope(config: &uc100::config::MachineConfig) -> ([[f32; 3]; 2], bool) {
        let limit = |a: usize| {
            let x = config.axes.get(a)?;
            Some((x.soft_min? as f32, x.soft_max? as f32))
        };
        match (limit(0), limit(1), limit(2)) {
            (Some(x), Some(y), Some(z)) => ([[x.0, y.0, z.0], [x.1, y.1, z.1]], false),
            _ => ([[0., 0., -TRAVEL[2]], [TRAVEL[0], TRAVEL[1], 0.]], true),
        }
    }

    /// The captions at the top left, in order. The box's caption depends on
    /// whether it is the assumed travel, never on the position: an assumed
    /// box is never called where the tool tip can travel.
    pub fn captions(&self) -> Vec<(Caption, &'static str)> {
        let mut lines = vec![];
        if !self.known {
            lines.push((
                Caption::Unset,
                "Machine position not set: the model follows the step counters, not the machine",
            ));
        }
        if self.assumed {
            lines.push((
                Caption::Assumed,
                "Dashed box: assumed travel, travel not configured in the profile",
            ));
        } else if self.known {
            lines.push((Caption::Travel, "Box: where the tool tip can travel"));
        }
        if self.known {
            lines.push((
                Caption::Zero,
                "Red dot: the tool tip · green ring: the work zero",
            ));
        }
        lines
    }
}

/// One flat polygon. Layer 0 is the base and table, drawn first: with the
/// view always from above, everything else stands on them. Layer 1 is the
/// table's slots, layer 2 the rest, each sorted far to near.
struct Face {
    pts: Vec<[f32; 3]>,
    normal: [f32; 3],
    color: Color,
    layer: u8,
    /// Which part it belongs to, for the tests.
    #[cfg_attr(not(test), allow(dead_code))]
    part: &'static str,
}

fn boxed(
    faces: &mut Vec<Face>,
    lo: [f32; 3],
    hi: [f32; 3],
    color: Color,
    layer: u8,
    part: &'static str,
) {
    let c = |i: usize| {
        [
            if i & 1 == 0 { lo[0] } else { hi[0] },
            if i & 2 == 0 { lo[1] } else { hi[1] },
            if i & 4 == 0 { lo[2] } else { hi[2] },
        ]
    };
    for (quad, normal) in [
        ([0, 2, 6, 4], [-1., 0., 0.]),
        ([1, 5, 7, 3], [1., 0., 0.]),
        ([0, 4, 5, 1], [0., -1., 0.]),
        ([2, 3, 7, 6], [0., 1., 0.]),
        ([0, 1, 3, 2], [0., 0., -1.]),
        ([4, 6, 7, 5], [0., 0., 1.]),
    ] {
        faces.push(Face {
            pts: quad.iter().map(|&i| c(i)).collect(),
            normal,
            color,
            layer,
            part,
        });
    }
}

/// An upright cylinder of 16 sides.
fn cylinder(
    faces: &mut Vec<Face>,
    at: [f32; 2],
    radius: f32,
    z: [f32; 2],
    color: Color,
    part: &'static str,
) {
    const N: usize = 16;
    let ring = |i: usize, z: f32| {
        let a = i as f32 * std::f32::consts::TAU / N as f32;
        [at[0] + radius * a.cos(), at[1] + radius * a.sin(), z]
    };
    for i in 0..N {
        let a = (i as f32 + 0.5) * std::f32::consts::TAU / N as f32;
        faces.push(Face {
            pts: vec![
                ring(i, z[0]),
                ring(i + 1, z[0]),
                ring(i + 1, z[1]),
                ring(i, z[1]),
            ],
            normal: [a.cos(), a.sin(), 0.],
            color,
            layer: 2,
            part,
        });
    }
    for (z, normal) in [(z[0], [0., 0., -1.]), (z[1], [0., 0., 1.])] {
        faces.push(Face {
            pts: (0..N).map(|i| ring(i, z)).collect(),
            normal,
            color,
            layer: 2,
            part,
        });
    }
}

/// The machine with its tool tip at `pos` (machine X, Y, Z).
fn faces(pos: [f32; 3], known: bool) -> Vec<Face> {
    let rgb = |r, g, b| Color::from_rgb8(r, g, b);
    // The moving parts turn amber until the machine position is set.
    let moving = |c: Color| {
        let amber = rgb(214, 150, 60);
        match known {
            true => c,
            false => Color::from_rgb(
                (c.r + amber.r) / 2.,
                (c.g + amber.g) / 2.,
                (c.b + amber.b) / 2.,
            ),
        }
    };
    let [x, y, z] = pos;
    let table_top = -TIP_HEIGHT;
    let tx = [(TRAVEL[0] - TABLE[0]) / 2., (TRAVEL[0] + TABLE[0]) / 2.];
    let ty = [(TRAVEL[1] - TABLE[1]) / 2., (TRAVEL[1] + TABLE[1]) / 2.];
    let r = SPINDLE[0] / 2.;
    let nose = z + TOOL[1] + 14.;
    // The gantry: spindle in front (toward the operator, −Y), then the Z
    // plate, the X carriage and the beam behind it.
    let beam = [table_top + BEAM_CLEARANCE, table_top + BEAM_CLEARANCE + 80.];
    let beam_y = [y + r + 30., y + r + 90.];
    let side = [tx[0] - 40., tx[1] + 40.];
    // The base and rails reach behind the beam at the back of Y.
    let back = TRAVEL[1] + r + 110.;
    let mut f = Vec::new();
    boxed(
        &mut f,
        [side[0], ty[0] - 30., table_top - 80.],
        [side[1], back, table_top - 20.],
        rgb(72, 80, 90),
        0,
        "base",
    );
    for x0 in [side[0], tx[1] + 15.] {
        boxed(
            &mut f,
            [x0, ty[0] - 30., table_top - 20.],
            [x0 + 25., back, table_top],
            rgb(96, 104, 114),
            0,
            "rails",
        );
    }
    boxed(
        &mut f,
        [tx[0], ty[0], table_top - 20.],
        [tx[1], ty[1], table_top],
        rgb(184, 191, 199),
        0,
        "table",
    );
    // T-slots along Y.
    for i in 1..6 {
        let sx = tx[0] + TABLE[0] * i as f32 / 6.;
        f.push(Face {
            pts: vec![
                [sx - 4., ty[0], table_top],
                [sx + 4., ty[0], table_top],
                [sx + 4., ty[1], table_top],
                [sx - 4., ty[1], table_top],
            ],
            normal: [0., 0., 1.],
            color: rgb(120, 128, 138),
            layer: 1,
            part: "table",
        });
    }
    let gantry = moving(rgb(64, 110, 160));
    for x0 in [side[0], side[1] - 25.] {
        boxed(
            &mut f,
            [x0, beam_y[0] - 10., table_top],
            [x0 + 25., beam_y[1] + 10., beam[1]],
            gantry,
            2,
            "gantry",
        );
    }
    // The beam in short pieces, so that far-to-near sorting of faces holds
    // next to the carriage.
    let pieces = 8;
    let w = (side[1] - side[0] - 50.) / pieces as f32;
    for i in 0..pieces {
        let x0 = side[0] + 25. + w * i as f32;
        boxed(
            &mut f,
            [x0, beam_y[0], beam[0]],
            [x0 + w, beam_y[1], beam[1]],
            gantry,
            2,
            "beam",
        );
    }
    boxed(
        &mut f,
        [x - 65., y + r + 15., beam[0] - 25.],
        [x + 65., y + r + 30., beam[1] + 10.],
        moving(rgb(88, 98, 110)),
        2,
        "carriage",
    );
    boxed(
        &mut f,
        [x - 50., y + r, nose + 25.],
        [x + 50., y + r + 15., nose + SPINDLE[1] + 15.],
        moving(rgb(110, 122, 136)),
        2,
        "z plate",
    );
    cylinder(
        &mut f,
        [x, y],
        r,
        [nose, nose + SPINDLE[1]],
        moving(rgb(206, 210, 216)),
        "spindle",
    );
    cylinder(
        &mut f,
        [x, y],
        14.,
        [nose - 14., nose],
        moving(rgb(150, 154, 160)),
        "nut",
    );
    cylinder(
        &mut f,
        [x, y],
        TOOL[0] / 2.,
        [z, z + TOOL[1]],
        moving(rgb(226, 178, 72)),
        "tool",
    );
    f
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// An orthographic view from `yaw` round Z (0 = from the front) and `pitch`
/// above the table.
struct View {
    right: [f32; 3],
    up: [f32; 3],
    /// From the scene toward the eye.
    toward: [f32; 3],
}

impl View {
    fn new(yaw: f32, pitch: f32) -> Self {
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        View {
            right: [cy, sy, 0.],
            up: [-sp * sy, sp * cy, cp],
            toward: [cp * sy, -cp * cy, sp],
        }
    }
    /// Screen coordinates before scaling (y down).
    fn flat(&self, p: [f32; 3]) -> [f32; 2] {
        [dot(p, self.right), -dot(p, self.up)]
    }
}

/// The faces that face the eye, shaded, back to front.
fn painted(faces: &[Face], view: &View) -> Vec<(Vec<[f32; 2]>, Color)> {
    let light = {
        let l = [-0.35, -0.55, 0.76];
        let n = dot(l, l).sqrt();
        [l[0] / n, l[1] / n, l[2] / n]
    };
    let mut seen: Vec<(u8, f32, &Face)> = faces
        .iter()
        .filter(|f| dot(f.normal, view.toward) > 1e-4)
        .map(|f| {
            let depth =
                f.pts.iter().map(|&p| dot(p, view.toward)).sum::<f32>() / f.pts.len() as f32;
            (f.layer, depth, f)
        })
        .collect();
    seen.sort_by(|a, b| (a.0, a.1).partial_cmp(&(b.0, b.1)).unwrap());
    seen.into_iter()
        .map(|(_, _, f)| {
            let shade = 0.55 + 0.45 * dot(f.normal, light).max(0.);
            let color = Color {
                r: f.color.r * shade,
                g: f.color.g * shade,
                b: f.color.b * shade,
                a: f.color.a,
            };
            (f.pts.iter().map(|&p| view.flat(p)).collect(), color)
        })
        .collect()
}

/// The view's drag state: orbit with the left button.
pub struct Orbit {
    yaw: f32,
    pitch: f32,
    drag: Option<Point>,
}

impl Default for Orbit {
    fn default() -> Self {
        Orbit {
            yaw: 0.35,
            pitch: 0.5,
            drag: None,
        }
    }
}

impl canvas::Program<Message> for MachineView {
    type State = Orbit;

    fn update(
        &self,
        state: &mut Orbit,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                state.drag = Some(cursor.position_in(bounds)?);
                Some(canvas::Action::capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let last = state.drag?;
                let p = Point::new(position.x - bounds.x, position.y - bounds.y);
                state.yaw -= (p.x - last.x) * 0.01;
                // Always from above, so the table can be drawn first.
                state.pitch = (state.pitch + (p.y - last.y) * 0.01).clamp(0.05, 1.5);
                state.drag = Some(p);
                Some(canvas::Action::request_redraw().and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.drag.take().map(|_| canvas::Action::capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &Orbit,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            Color::from_rgb8(239, 243, 247),
        );
        let view = View::new(state.yaw, state.pitch);
        // Fit a fixed box round the whole machine, so the scale holds still
        // while it moves.
        let (lo, hi) = (
            [-120., -60., -TIP_HEIGHT - 80.],
            [TRAVEL[0] + 120., TRAVEL[1] + 160., 260.],
        );
        let corners: Vec<[f32; 2]> = (0..8)
            .map(|i| {
                view.flat([
                    if i & 1 == 0 { lo[0] } else { hi[0] },
                    if i & 2 == 0 { lo[1] } else { hi[1] },
                    if i & 4 == 0 { lo[2] } else { hi[2] },
                ])
            })
            .collect();
        let min = |k: usize| corners.iter().map(|c| c[k]).fold(f32::MAX, f32::min);
        let max = |k: usize| corners.iter().map(|c| c[k]).fold(f32::MIN, f32::max);
        let size = Size::new(max(0) - min(0), max(1) - min(1));
        let scale = (bounds.width / size.width).min(bounds.height / size.height) * 0.97;
        let origin = Point::new(
            (bounds.width - size.width * scale) / 2. - min(0) * scale,
            (bounds.height - size.height * scale) / 2. - min(1) * scale,
        );
        let screen = |p: [f32; 2]| Point::new(origin.x + p[0] * scale, origin.y + p[1] * scale);
        let mut f = faces(self.pos, self.known);
        if let Some([lo, hi]) = self.block {
            boxed(&mut f, lo, hi, Color::from_rgb8(214, 184, 132), 2, "stock");
        }
        for (pts, color) in painted(&f, &view) {
            let path = canvas::Path::new(|b| {
                b.move_to(screen(pts[0]));
                for &p in &pts[1..] {
                    b.line_to(screen(p));
                }
                b.close();
            });
            frame.fill(&path, color);
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(Color {
                        a: 0.25 * color.a,
                        ..Color::BLACK
                    })
                    .with_width(0.5),
            );
        }
        // The tool tip's travel, and the tool tip.
        let amber = Color::from_rgb8(196, 120, 20);
        let envelope = if self.assumed {
            Color { a: 0.75, ..amber }
        } else if self.known {
            Color::from_rgba8(33, 107, 181, 0.8)
        } else {
            amber
        };
        let dash = [6., 4.];
        let [elo, ehi] = self.envelope;
        let corner = |i: usize| {
            view.flat([
                if i & 1 == 0 { elo[0] } else { ehi[0] },
                if i & 2 == 0 { elo[1] } else { ehi[1] },
                if i & 4 == 0 { elo[2] } else { ehi[2] },
            ])
        };
        for (a, b) in [
            (0, 1),
            (2, 3),
            (4, 5),
            (6, 7),
            (0, 2),
            (1, 3),
            (4, 6),
            (5, 7),
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7),
        ] {
            frame.stroke(
                &canvas::Path::line(screen(corner(a)), screen(corner(b))),
                canvas::Stroke {
                    line_dash: if self.assumed {
                        canvas::LineDash {
                            segments: &dash,
                            offset: 0,
                        }
                    } else {
                        canvas::LineDash::default()
                    },
                    ..canvas::Stroke::default()
                        .with_color(envelope)
                        .with_width(1.2)
                },
            );
        }
        let green = Color::from_rgb8(39, 130, 93);
        if self.known {
            frame.stroke(
                &canvas::Path::circle(screen(view.flat(self.zero)), 7.),
                canvas::Stroke::default().with_color(green).with_width(2.),
            );
        }
        frame.fill(
            &canvas::Path::circle(screen(view.flat(self.pos)), 4.),
            Color::from_rgb8(214, 40, 40),
        );
        for (i, (kind, content)) in self.captions().into_iter().enumerate() {
            frame.fill_text(canvas::Text {
                content: content.into(),
                position: Point::new(10., 8. + 16. * i as f32),
                color: match kind {
                    Caption::Travel => envelope,
                    Caption::Assumed | Caption::Unset => amber,
                    Caption::Zero => green,
                },
                size: 12.into(),
                ..Default::default()
            });
        }
        frame.fill_text(canvas::Text {
            content: "CNC4030, assumed dimensions · drag to turn".into(),
            position: Point::new(10., bounds.height - 20.),
            color: Color::from_rgb8(110, 128, 145),
            size: 11.into(),
            ..Default::default()
        });
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part<'a>(f: &'a [Face], name: &str) -> impl Iterator<Item = [f32; 3]> + 'a {
        let name = name.to_string();
        f.iter()
            .filter(move |f| f.part == name)
            .flat_map(|f| f.pts.clone())
    }

    fn lowest(f: &[Face], name: &str, k: usize) -> f32 {
        part(f, name).map(|p| p[k]).fold(f32::MAX, f32::min)
    }

    #[test]
    fn the_model_follows_the_machine_position() {
        let home = faces([0., 0., 0.], true);
        let moved = faces([120., 80., -30.], true);
        // The tool tip is the machine position.
        assert_eq!(lowest(&moved, "tool", 2), -30.);
        let tip: Vec<[f32; 3]> = part(&moved, "tool").filter(|p| p[2] == -30.).collect();
        let centre = |k: usize| tip.iter().map(|p| p[k]).sum::<f32>() / tip.len() as f32;
        assert!((centre(0) - 120.).abs() < 1e-3 && (centre(1) - 80.).abs() < 1e-3);
        // The gantry beam moves with Y only; the table never moves.
        assert_eq!(lowest(&moved, "beam", 1) - lowest(&home, "beam", 1), 80.);
        assert_eq!(lowest(&moved, "beam", 0), lowest(&home, "beam", 0));
        assert_eq!(
            lowest(&moved, "carriage", 0) - lowest(&home, "carriage", 0),
            120.
        );
        let table = |f: &[Face]| part(f, "table").collect::<Vec<_>>();
        assert_eq!(table(&home), table(&moved));
        // At the bottom of Z the tool stays above the table.
        assert!(lowest(&faces([0., 0., -TRAVEL[2]], true), "tool", 2) > -TIP_HEIGHT);
        // Until the position is set, the moving parts change colour.
        let unset = faces([0., 0., 0.], false);
        let colour = |f: &[Face], name: &str| f.iter().find(|f| f.part == name).unwrap().color;
        assert_ne!(colour(&unset, "spindle"), colour(&home, "spindle"));
        assert_eq!(colour(&unset, "table"), colour(&home, "table"));
    }

    #[test]
    fn the_painter_culls_back_faces_and_draws_far_to_near() {
        let (near, far) = (Color::from_rgb8(255, 0, 0), Color::from_rgb8(0, 0, 255));
        let mut f = Vec::new();
        // Seen from the front, the box at the back is farther away.
        boxed(&mut f, [0., 100., 0.], [10., 110., 10.], far, 2, "far");
        boxed(&mut f, [0., -110., 0.], [10., -100., 10.], near, 2, "near");
        let o = Orbit::default();
        let drawn = painted(&f, &View::new(o.yaw, o.pitch));
        // Three faces of each box face the eye: front, top and one side.
        assert_eq!(drawn.len(), 6);
        let blue = |c: Color| c.b > c.r;
        assert!(drawn[..3].iter().all(|(_, c)| blue(*c)));
        assert!(drawn[3..].iter().all(|(_, c)| !blue(*c)));
        // The table layer comes first even when it is nearer.
        boxed(
            &mut f,
            [0., -300., -20.],
            [10., -290., -10.],
            Color::WHITE,
            0,
            "table",
        );
        let drawn = painted(&f, &View::new(o.yaw, o.pitch));
        assert_eq!(drawn[0].1.g, drawn[0].1.r);
        assert!(drawn[..3].iter().all(|(_, c)| c.r == c.g && c.g == c.b));
    }
}
