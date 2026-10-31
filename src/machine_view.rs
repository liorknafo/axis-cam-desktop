//! The machine on the Control page: a flat-shaded model of a gantry router
//! shaped like the Jingyan CNC4030, drawn to the machine library's travel and
//! clearances, that moves with the controller's machine position. Its own
//! small canvas, drawn from scratch each frame (a few hundred polygons), not
//! the Work page's viewport.
use crate::Message;
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, mouse, widget::canvas};

// World coordinates are the tool tip's machine coordinates in mm: machine
// zero is X at the left end, Y at the front end, Z at the top. The sizes
// below are the parts the machine library does not give (LIO-47, ASSUMED
// from a similar 800 W CNC4030); `Figures` holds the ones it does.
/// How far the T-slot table reaches past the X and the Y travel, each side.
const TABLE_OVER: [f32; 2] = [70., 20.];
/// Spindle body diameter and length, without a rotary unit (with one, its
/// machine's spindle nose, `model::NOSE_DEFAULT` assumed).
const SPINDLE: [f32; 2] = [65., 185.];
/// Nominal tool: diameter and length below the collet nut.
const TOOL: [f32; 2] = [6., 30.];
/// The collet nut's length.
const NUT: f32 = 14.;
/// A stepper motor's square, and its length.
const MOTOR: [f32; 2] = [57., 60.];

/// The machine's figures the model is drawn to, mm: the X, Y and Z travel,
/// the spindle nose over the table at Z top, and the gantry's clearance
/// height and width (`envelope::FIGURES`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Figures {
    pub travel: [f32; 3],
    pub nose_to_table: f32,
    pub gantry: [f32; 2],
}

impl Figures {
    /// The shipped Jingyan CNC4030's, ASSUMED (`envelope::jingyan`).
    pub const JINGYAN: Figures = Figures {
        travel: [390., 280., 55.],
        nose_to_table: 160.,
        gantry: [120., 390.],
    };

    /// Machine `m`'s figures, the Jingyan's for any it does not give.
    pub fn of(m: &crate::model::Machine) -> Self {
        let j = Self::JINGYAN;
        let f = |key, or| crate::envelope::clearance(m, key).map_or(or, |c| c.0);
        Figures {
            travel: [
                f("x", j.travel[0]),
                f("y", j.travel[1]),
                f("z", j.travel[2]),
            ],
            nose_to_table: f("nose_to_table", j.nose_to_table),
            gantry: [
                f("gantry_height", j.gantry[0]),
                f("gantry_width", j.gantry[1]),
            ],
        }
    }
}

/// The model for one frame: where the tool tip is, whether the machine
/// position is set, and the box the tool tip may travel in.
pub struct MachineView {
    /// The machine's sizes.
    pub figures: Figures,
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
    /// The rotary unit at its machine library position (`chuck::Rig::on_machine`),
    /// on a machine with A and its geometry; it stands on the table, whose
    /// top is then its centre height under the centreline.
    pub rotary: Option<crate::chuck::Rig>,
    /// The controller's A (degrees): the chuck and its jaws turn to it.
    pub a: f32,
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
    /// The travel box from the profile's soft limits, or the machine's
    /// `travel` (X, Y, Z) from machine zero, and whether it is the latter.
    pub fn envelope(
        config: &uc100::config::MachineConfig,
        travel: [f32; 3],
    ) -> ([[f32; 3]; 2], bool) {
        let limit = |a: usize| {
            let x = config.axes.get(a)?;
            Some((x.soft_min? as f32, x.soft_max? as f32))
        };
        match (limit(0), limit(1), limit(2)) {
            (Some(x), Some(y), Some(z)) => ([[x.0, y.0, z.0], [x.1, y.1, z.1]], false),
            _ => ([[0., 0., -travel[2]], [travel[0], travel[1], 0.]], true),
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

/// One flat polygon. Layer 0 is the base, drawn first, then layer 1, the
/// table and the rails on it: with the view always from above, everything
/// else stands on them. Layer 2 is the table's slots, layer 3 the rest,
/// each sorted far to near.
struct Face {
    pts: Vec<[f32; 3]>,
    normal: [f32; 3],
    color: Color,
    layer: u8,
    /// Which part it belongs to.
    part: &'static str,
    /// Its edges are drawn: not between a cylinder's sides or the beam's
    /// pieces, which would show as seams.
    outline: bool,
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
            outline: true,
        });
    }
}

/// An upright cylinder of 24 sides.
fn cylinder(
    faces: &mut Vec<Face>,
    at: [f32; 2],
    radius: f32,
    z: [f32; 2],
    color: Color,
    part: &'static str,
) {
    const N: usize = 24;
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
            layer: 3,
            part,
            outline: false,
        });
    }
    for (z, normal) in [(z[0], [0., 0., -1.]), (z[1], [0., 0., 1.])] {
        faces.push(Face {
            pts: (0..N).map(|i| ring(i, z)).collect(),
            normal,
            color,
            layer: 3,
            part,
            outline: true,
        });
    }
}

/// The Jingyan with its tool tip at `pos` (machine X, Y, Z).
#[cfg(test)]
fn faces(pos: [f32; 3], known: bool) -> Vec<Face> {
    machine(&Figures::JINGYAN, pos, known, None, 0.)
}

/// The table's top in machine Z: the nominal tool's tip at Z top stands the
/// spindle nose's height, less the tool and its nut, over it; with the
/// rotary unit, the unit's centre height under its centreline.
fn table_top(fig: &Figures, rotary: Option<&crate::chuck::Rig>) -> f32 {
    rotary.map_or(TOOL[1] + NUT - fig.nose_to_table, |r| r.table)
}

/// The machine of figures `fig` with its tool tip at `pos`, and the rotary
/// unit `rotary` with its chuck at A `a` (degrees). The gantry runs in Y on
/// rails beside the table, its beam the gantry clearance over the table
/// (over the unit's chuck, with one), its side plates the clearance width
/// apart but never on the table.
fn machine(
    fig: &Figures,
    pos: [f32; 3],
    known: bool,
    rotary: Option<&crate::chuck::Rig>,
    a: f32,
) -> Vec<Face> {
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
    let travel = fig.travel;
    let table_top = table_top(fig, rotary);
    let clearance = rotary.map_or(fig.gantry[0], |r| {
        fig.gantry[0].max(r.unit.centre_height + r.unit.chuck_diameter / 2. + 10.)
    });
    let tx = [-TABLE_OVER[0], travel[0] + TABLE_OVER[0]];
    let ty = [-TABLE_OVER[1], travel[1] + TABLE_OVER[1]];
    // On a machine with A, the spindle its rotary checks take (LIO-204).
    let spindle = rotary.map_or(SPINDLE, |rig| {
        [rig.unit.nose_diameter, rig.unit.nose_length]
    });
    let r = spindle[0] / 2.;
    let nose = z + TOOL[1] + NUT;
    // The gantry: spindle in front (toward the operator, −Y), then the Z
    // plate, the X carriage and the beam behind it.
    let beam = [table_top + clearance, table_top + clearance + 80.];
    let beam_y = [y + r + 30., y + r + 90.];
    let half = fig.gantry[1].max(tx[1] - tx[0] + 30.) / 2.;
    let open = [travel[0] / 2. - half, travel[0] / 2. + half];
    let plates = [[open[0] - 20., open[0]], [open[1], open[1] + 20.]];
    // The base and rails reach behind the gantry at the back of Y.
    let back = travel[1] + r + 130.;
    let mut f = Vec::new();
    boxed(
        &mut f,
        [open[0] - 40., ty[0] - 30., table_top - 60.],
        [open[1] + 40., back, table_top - 20.],
        rgb(62, 68, 76),
        0,
        "base",
    );
    for [x0, x1] in plates {
        boxed(
            &mut f,
            [x0 - 5., ty[0] - 30., table_top - 20.],
            [x1 + 5., back, table_top],
            rgb(150, 158, 168),
            1,
            "rails",
        );
        // The Y motor at the rail's back end, under its top.
        boxed(
            &mut f,
            [(x0 + x1 - MOTOR[0]) / 2., back, table_top - MOTOR[0]],
            [(x0 + x1 + MOTOR[0]) / 2., back + MOTOR[1], table_top],
            rgb(44, 47, 52),
            0,
            "motor",
        );
    }
    boxed(
        &mut f,
        [tx[0], ty[0], table_top - 20.],
        [tx[1], ty[1], table_top],
        rgb(184, 191, 199),
        1,
        "table",
    );
    // T-slots along Y, about every 85 mm.
    let slots = ((tx[1] - tx[0]) / 85.).round().max(2.) as usize;
    for i in 1..slots {
        let sx = tx[0] + (tx[1] - tx[0]) * i as f32 / slots as f32;
        f.push(Face {
            pts: vec![
                [sx - 4., ty[0], table_top],
                [sx + 4., ty[0], table_top],
                [sx + 4., ty[1], table_top],
                [sx - 4., ty[1], table_top],
            ],
            normal: [0., 0., 1.],
            color: rgb(120, 128, 138),
            layer: 2,
            part: "table",
            outline: true,
        });
    }
    let gantry = moving(rgb(52, 104, 158));
    for [x0, x1] in plates {
        boxed(
            &mut f,
            [x0, beam_y[0] - 30., table_top],
            [x1, beam_y[1] + 30., beam[1]],
            gantry,
            3,
            "gantry",
        );
    }
    // The beam in short pieces, so that far-to-near sorting of faces holds
    // next to the carriage.
    let pieces = 8;
    let w = (plates[1][1] - plates[0][0]) / pieces as f32;
    for i in 0..pieces {
        let x0 = plates[0][0] + w * i as f32;
        boxed(
            &mut f,
            [x0, beam_y[0], beam[0]],
            [x0 + w, beam_y[1], beam[1]],
            gantry,
            3,
            "beam",
        );
    }
    for face in f.iter_mut().filter(|f| f.part == "beam") {
        face.outline = false;
    }
    // The X motor at the beam's right end.
    let (beam_mid, beam_mid_y) = ((beam[0] + beam[1]) / 2., (beam_y[0] + beam_y[1]) / 2.);
    boxed(
        &mut f,
        [
            plates[1][1],
            beam_mid_y - MOTOR[0] / 2.,
            beam_mid - MOTOR[0] / 2.,
        ],
        [
            plates[1][1] + MOTOR[1],
            beam_mid_y + MOTOR[0] / 2.,
            beam_mid + MOTOR[0] / 2.,
        ],
        moving(rgb(44, 47, 52)),
        3,
        "motor",
    );
    boxed(
        &mut f,
        [x - 65., y + r + 15., beam[0] - 25.],
        [x + 65., y + r + 30., beam[1] + 10.],
        moving(rgb(88, 98, 110)),
        3,
        "carriage",
    );
    let plate_top = nose + spindle[1] + 15.;
    boxed(
        &mut f,
        [x - 50., y + r, nose + 25.],
        [x + 50., y + r + 15., plate_top],
        moving(rgb(110, 122, 136)),
        3,
        "z plate",
    );
    // The Z motor on its bracket over the spindle.
    boxed(
        &mut f,
        [x - MOTOR[0] / 2., y + r + 15. - MOTOR[0], plate_top],
        [x + MOTOR[0] / 2., y + r + 15., plate_top + MOTOR[1]],
        moving(rgb(44, 47, 52)),
        3,
        "motor",
    );
    // The spindle, held to the Z plate by its clamp; the clamp hides the
    // part of it inside.
    let clamp = [nose + spindle[1] * 0.45, nose + spindle[1] * 0.8];
    boxed(
        &mut f,
        [x - r - 10., y - r - 10., clamp[0]],
        [x + r + 10., y + r, clamp[1]],
        moving(rgb(70, 78, 88)),
        3,
        "clamp",
    );
    for span in [[nose, clamp[0]], [clamp[1], nose + spindle[1]]] {
        cylinder(
            &mut f,
            [x, y],
            r,
            span,
            moving(rgb(214, 218, 224)),
            "spindle",
        );
    }
    cylinder(
        &mut f,
        [x, y],
        14.,
        [nose - NUT, nose],
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
    if let Some(rig) = rotary {
        rotary_faces(&mut f, rig, a);
    }
    f
}

/// The rotary unit's solids at A `a` (`chuck::Rig::solids`): the chuck
/// and its jaws turn with A, the head and the tailstock stand still.
fn rotary_faces(f: &mut Vec<Face>, rig: &crate::chuck::Rig, a: f32) {
    let rgb = |r, g, b| Color::from_rgb8(r, g, b);
    for (label, prism, _) in rig.solids(a) {
        let (part, color) = match label.as_str() {
            "chuck body" => ("chuck", rgb(176, 182, 190)),
            l if l.starts_with("chuck jaw") => ("jaw", rgb(92, 100, 110)),
            "rotary head" => ("rotary head", rgb(64, 110, 160)),
            _ => ("tailstock", rgb(120, 132, 146)),
        };
        let [a, b] = &prism.ends;
        let n = a.len();
        let all = a.iter().chain(b);
        let centre = all.clone().fold([0.; 3], |c, p| {
            std::array::from_fn(|k| c[k] + p[k] / (2 * n) as f32)
        });
        // Each face's normal, turned away from the solid's middle.
        let mut push = |pts: Vec<[f32; 3]>| {
            let (u, v) = (sub(pts[1], pts[0]), sub(pts[pts.len() - 1], pts[0]));
            let mut normal = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let length = dot(normal, normal).sqrt();
            if length < 1e-9 {
                return;
            }
            let mid: [f32; 3] =
                std::array::from_fn(|k| pts.iter().map(|p| p[k]).sum::<f32>() / pts.len() as f32);
            let out = if dot(normal, sub(mid, centre)) < 0. {
                -1.
            } else {
                1.
            };
            normal = normal.map(|v| v * out / length);
            f.push(Face {
                pts,
                normal,
                color,
                layer: 3,
                part,
                outline: true,
            });
        };
        for i in 0..n {
            let j = (i + 1) % n;
            push(vec![a[i], a[j], b[j], b[i]]);
        }
        push(a.clone());
        push(b.clone());
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
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

/// The faces that face the eye, shaded, back to front, and whether each
/// has its edges drawn.
fn painted(faces: &[Face], view: &View) -> Vec<(Vec<[f32; 2]>, Color, bool)> {
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
            (
                f.pts.iter().map(|&p| view.flat(p)).collect(),
                color,
                f.outline,
            )
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
        // Fit the box round the whole machine at both ends of its X and Y
        // travel, so the scale holds still while it moves.
        let rotary = self.rotary.as_ref();
        let [tx, ty, _] = self.figures.travel;
        let ends = [[0., 0., 0.], [tx, ty, 0.]];
        let all = ends
            .iter()
            .flat_map(|&p| machine(&self.figures, p, true, rotary, 0.));
        let (lo, hi) = all.flat_map(|f| f.pts).chain(self.envelope).fold(
            ([f32::MAX; 3], [f32::MIN; 3]),
            |(lo, hi), p| {
                (
                    std::array::from_fn(|k| lo[k].min(p[k])),
                    std::array::from_fn(|k| hi[k].max(p[k])),
                )
            },
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
        let mut f = machine(&self.figures, self.pos, self.known, rotary, self.a);
        if let Some([lo, hi]) = self.block {
            boxed(&mut f, lo, hi, Color::from_rgb8(214, 184, 132), 3, "stock");
            // A block over the table stands on a vise's body, not in the air.
            let table = table_top(&self.figures, rotary);
            if rotary.is_none() && lo[2] > table + 1. {
                boxed(
                    &mut f,
                    [lo[0], lo[1] - 15., table],
                    [hi[0], hi[1] + 15., lo[2]],
                    Color::from_rgb8(96, 108, 122),
                    3,
                    "vise",
                );
            }
        }
        for (pts, color, outline) in painted(&f, &view) {
            let path = canvas::Path::new(|b| {
                b.move_to(screen(pts[0]));
                for &p in &pts[1..] {
                    b.line_to(screen(p));
                }
                b.close();
            });
            frame.fill(&path, color);
            // Without its edges, a face is stroked in its own colour, which
            // closes the anti-aliased seams to its neighbours.
            let edge = match outline {
                true => Color {
                    a: 0.25 * color.a,
                    ..Color::BLACK
                },
                false => color,
            };
            frame.stroke(
                &path,
                canvas::Stroke::default().with_color(edge).with_width(0.5),
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
        // The captions are cut to the view's width (about 6.2 px a character
        // at 12 px), so that a narrow view does not run them past its edge.
        let fits = |px: f32| ((bounds.width - 20.) / px).max(1.) as usize;
        for (i, (kind, content)) in self.captions().into_iter().enumerate() {
            frame.fill_text(canvas::Text {
                content: crate::ui::ellipsize(content, fits(6.2)),
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
        let [x, y, z] = self.figures.travel.map(crate::envelope::mm);
        let size = format!("Travel {x} × {y} × {z} mm from the machine library");
        let foot = match &self.rotary {
            Some(_) => format!(
                "{size} · rotary unit as the library places it, A {}° · drag to turn",
                crate::indexed::degrees(self.a)
            ),
            None => format!("{size} · drag to turn"),
        };
        frame.fill_text(canvas::Text {
            content: crate::ui::ellipsize(&foot, fits(5.7)),
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
        let j = Figures::JINGYAN;
        let bottom = faces([0., 0., -j.travel[2]], true);
        assert!(lowest(&bottom, "tool", 2) > table_top(&j, None));
        // Until the position is set, the moving parts change colour.
        let unset = faces([0., 0., 0.], false);
        let colour = |f: &[Face], name: &str| f.iter().find(|f| f.part == name).unwrap().color;
        assert_ne!(colour(&unset, "spindle"), colour(&home, "spindle"));
        assert_eq!(colour(&unset, "table"), colour(&home, "table"));
    }

    #[test]
    fn the_rotary_unit_stands_where_the_library_puts_it_and_its_chuck_turns_with_a() {
        let mut m = crate::model::Machine::new(4);
        m.fill_rotary_defaults();
        let unit = m.rotary_unit().unwrap();
        let rig = crate::chuck::Rig::on_machine(unit, 1.);
        let at = |a: f32| machine(&Figures::JINGYAN, [100., 50., 0.], true, Some(&rig), a);
        let zero = at(0.);
        let highest =
            |f: &[Face], name: &str, k: usize| part(f, name).map(|p| p[k]).fold(f32::MIN, f32::max);
        // The chuck's face at the chuck X, round the centreline at its Y
        // and Z; the table's top its centre height under it.
        let r = unit.chuck_diameter / 2.;
        assert!((highest(&zero, "chuck", 0) - unit.chuck_x).abs() < 1e-3);
        assert!((highest(&zero, "chuck", 2) - (unit.centre_z + r)).abs() < 3.);
        assert!((lowest(&zero, "chuck", 1) - (unit.centre_y - r)).abs() < 3.);
        let table = unit.centre_z - unit.centre_height;
        assert_eq!(highest(&zero, "table", 2), table);
        assert_eq!(lowest(&zero, "rotary head", 2), table);
        // The jaws stand out of the chuck's face and turn with A about the
        // centreline, +Y up to +Z for positive A.
        assert!(lowest(&zero, "jaw", 0) >= unit.chuck_x - 1e-3);
        let quarter = at(90.);
        let jaws = |f: &[Face]| part(f, "jaw").collect::<Vec<_>>();
        let axis = [unit.centre_y, unit.centre_z];
        assert!(!jaws(&zero).is_empty());
        for (p, q) in jaws(&zero).into_iter().zip(jaws(&quarter)) {
            let want = crate::indexed::turn_point(p, axis, 90.);
            assert!(
                (0..3).all(|k| (want[k] - q[k]).abs() < 1e-3),
                "{want:?} {q:?}"
            );
        }
        // The head and the tailstock stand still; the tool stays above the
        // table and the beam above the chuck.
        let solid = |f: &[Face], name: &str| part(f, name).collect::<Vec<_>>();
        for name in ["rotary head", "tailstock"] {
            assert!(!solid(&zero, name).is_empty());
            assert_eq!(solid(&zero, name), solid(&quarter, name));
        }
        assert!(lowest(&at(0.), "tool", 2) > table);
        assert!(lowest(&zero, "beam", 2) > unit.centre_z + r);
        // Without a rotary unit the machine is the one it was.
        let plain = faces([100., 50., 0.], true);
        assert!(part(&plain, "jaw").next().is_none());
        assert_eq!(
            highest(&plain, "table", 2),
            table_top(&Figures::JINGYAN, None)
        );
    }

    #[test]
    fn the_model_is_drawn_to_the_machines_travel_and_clearances() {
        let highest =
            |f: &[Face], name: &str, k: usize| part(f, name).map(|p| p[k]).fold(f32::MIN, f32::max);
        let j = Figures::JINGYAN;
        let at = |fig: &Figures| machine(fig, [0., 0., 0.], true, None, 0.);
        let shipped = at(&j);
        // The table reaches past the X and Y travel by the same margins.
        let wide = Figures {
            travel: [600., 400., 55.],
            ..j
        };
        let grown = at(&wide);
        let span = |f: &[Face], k: usize| highest(f, "table", k) - lowest(f, "table", k);
        assert_eq!(span(&grown, 0) - span(&shipped, 0), 210.);
        assert_eq!(span(&grown, 1) - span(&shipped, 1), 120.);
        // The base and rails reach behind the gantry at the far end of Y.
        assert_eq!(
            highest(&grown, "rails", 1) - highest(&shipped, "rails", 1),
            120.
        );
        // The spindle nose stands its height over the table at Z top, and
        // the beam the gantry clearance.
        let nose = lowest(&shipped, "nut", 2) + NUT;
        assert_eq!(nose - highest(&shipped, "table", 2), j.nose_to_table);
        assert_eq!(
            lowest(&shipped, "beam", 2) - highest(&shipped, "table", 2),
            j.gantry[0]
        );
        let tall = at(&Figures {
            nose_to_table: 220.,
            gantry: [180., j.gantry[1]],
            ..j
        });
        assert_eq!(
            highest(&tall, "table", 2),
            highest(&shipped, "table", 2) - 60.
        );
        assert_eq!(lowest(&tall, "beam", 2) - highest(&tall, "table", 2), 180.);
        // The side plates stand beside the table, the clearance width apart
        // when that is wider.
        let inside = |f: &[Face]| {
            let plates: Vec<f32> = part(f, "gantry").map(|p| p[0]).collect();
            let mid = j.travel[0] / 2.;
            let left = plates
                .iter()
                .copied()
                .filter(|&v| v < mid)
                .fold(f32::MIN, f32::max);
            let right = plates
                .iter()
                .copied()
                .filter(|&v| v > mid)
                .fold(f32::MAX, f32::min);
            (left, right)
        };
        let (left, right) = inside(&shipped);
        assert!(left < lowest(&shipped, "table", 0) && right > highest(&shipped, "table", 0));
        let (left, right) = inside(&at(&Figures {
            gantry: [j.gantry[0], 800.],
            ..j
        }));
        assert_eq!(right - left, 800.);
        // The figures come from the machine library, in inches too, and
        // the Jingyan's stand in for those it does not give.
        let mut m = crate::model::Machine::new(3);
        for (k, v) in [
            ("x", "500"),
            ("y", "350"),
            ("z", "90"),
            ("gantry_height", "150"),
        ] {
            m.p.insert(k.into(), v.into());
        }
        m.p.insert("nose_to_table".into(), "".into());
        m.p.insert("gantry_width".into(), "".into());
        let fig = Figures::of(&m);
        assert_eq!(fig.travel, [500., 350., 90.]);
        assert_eq!(fig.gantry, [150., j.gantry[1]]);
        assert_eq!(fig.nose_to_table, j.nose_to_table);
        m.p.insert("units".into(), "inch".into());
        assert_eq!(
            Figures::of(&m).travel,
            [500. * 25.4, 350. * 25.4, 90. * 25.4]
        );
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
        assert!(drawn[..3].iter().all(|(_, c, _)| blue(*c)));
        assert!(drawn[3..].iter().all(|(_, c, _)| !blue(*c)));
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
        assert!(drawn[..3].iter().all(|(_, c, _)| c.r == c.g && c.g == c.b));
    }
}
