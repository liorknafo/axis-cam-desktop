//! The machine on the Control page: a model of a gantry router shaped like
//! the Jingyan CNC4030, drawn to the machine library's travel and
//! clearances, that moves with the controller's machine position. Its own
//! canvas and its own small renderer (LIO-342): the model's triangles, with
//! a normal at each corner, go through a depth buffer at twice the frame's
//! resolution, are lit per pixel, outlined where solids meet and at creases,
//! and scaled down, as the Work page's view draws a part. The picture is
//! kept while nothing it is drawn from changes.
use crate::Message;
use iced::{Color, Point, Rectangle, Renderer, Theme, mouse, widget::canvas};
use std::cell::RefCell;

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
/// A NEMA 23 stepper motor's square, and its length.
const MOTOR: [f32; 2] = [57., 76.];
/// The Y rails' round rods: their radius, and how high their axis stands
/// over the table's top.
const ROD: [f32; 2] = [8., -4.];

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
    /// The soft limits and the machine's travel disagree.
    Mismatch,
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
    /// box is never called where the tool tip can travel. Soft limits that
    /// span other than the machine's travel are said, since the model is
    /// drawn to the travel and the box to the limits.
    pub fn captions(&self) -> Vec<(Caption, String)> {
        let mut lines = vec![];
        if !self.known {
            lines.push((
                Caption::Unset,
                "Machine position not set: the model follows the step counters, not the machine"
                    .into(),
            ));
        }
        if self.assumed {
            lines.push((
                Caption::Assumed,
                "Dashed box: assumed travel, travel not configured in the profile".into(),
            ));
        } else if self.known {
            lines.push((Caption::Travel, "Box: where the tool tip can travel".into()));
        }
        let [lo, hi] = self.envelope;
        let span: [f32; 3] = std::array::from_fn(|k| hi[k] - lo[k]);
        let travel = self.figures.travel;
        if !self.assumed && (0..3).any(|k| (span[k] - travel[k]).abs() > 0.5) {
            let size = |v: [f32; 3]| v.map(crate::envelope::mm).join(" × ");
            lines.push((
                Caption::Mismatch,
                format!(
                    "The profile's soft limits span {} mm, the machine's travel is {} mm",
                    size(span),
                    size(travel)
                ),
            ));
        }
        if self.known {
            lines.push((
                Caption::Zero,
                "Red dot: the tool tip · green ring: the work zero".into(),
            ));
        }
        lines
    }
}

/// A surface's colour (0 to 1) and how much it shines: 0 matte paint, 1
/// polished steel.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Finish {
    color: [f32; 3],
    shine: f32,
}

const fn finish(r: u8, g: u8, b: u8, shine: f32) -> Finish {
    Finish {
        color: [r as f32 / 255., g as f32 / 255., b as f32 / 255.],
        shine,
    }
}

const ALUMINIUM: Finish = finish(190, 196, 203, 0.45);
const ANODISED: Finish = finish(40, 96, 168, 0.35);
const CHROME: Finish = finish(206, 212, 218, 0.95);
const BLACK: Finish = finish(38, 40, 44, 0.3);
const RUBBER: Finish = finish(30, 31, 33, 0.05);
const TABLE: Finish = finish(168, 176, 186, 0.3);
const STEEL: Finish = finish(98, 108, 120, 0.5);
const CARBIDE: Finish = finish(214, 168, 64, 0.6);
const STOCK: Finish = finish(214, 178, 118, 0.1);

/// One triangle of the model: its corners, their normals, its finish, the
/// solid it belongs to (edges are drawn where solids meet) and its part.
struct Tri {
    pts: [[f32; 3]; 3],
    normals: [[f32; 3]; 3],
    finish: Finish,
    solid: u32,
    part: &'static str,
}

/// The model's triangles as they are built, each solid numbered.
#[derive(Default)]
struct Model {
    tris: Vec<Tri>,
    solids: u32,
}

impl Model {
    fn solid(&mut self) -> u32 {
        self.solids += 1;
        self.solids
    }

    /// The quad `p` (in order round it) with the corner normals `n`.
    fn quad(
        &mut self,
        solid: u32,
        p: [[f32; 3]; 4],
        n: [[f32; 3]; 4],
        finish: Finish,
        part: &'static str,
    ) {
        for [a, b, c] in [[0, 1, 2], [0, 2, 3]] {
            self.tris.push(Tri {
                pts: [p[a], p[b], p[c]],
                normals: [n[a], n[b], n[c]],
                finish,
                solid,
                part,
            });
        }
    }

    /// A convex polygon, flat, with the normal `n`.
    fn polygon(
        &mut self,
        solid: u32,
        p: &[[f32; 3]],
        n: [f32; 3],
        finish: Finish,
        part: &'static str,
    ) {
        for i in 1..p.len().saturating_sub(1) {
            self.tris.push(Tri {
                pts: [p[0], p[i], p[i + 1]],
                normals: [n; 3],
                finish,
                solid,
                part,
            });
        }
    }

    /// A box from `lo` to `hi` with its edges and corners rounded to radius
    /// `r`. Each face of the plain box is cut on a grid that crowds toward
    /// its edges, and each grid point is moved onto the rounded box: to the
    /// nearest point of the box shrunk by `r`, then `r` out along the way
    /// it came, which is also its normal.
    fn rounded(&mut self, lo: [f32; 3], hi: [f32; 3], r: f32, finish: Finish, part: &'static str) {
        let solid = self.solid();
        let r = (0..3)
            .map(|k| (hi[k] - lo[k]) / 2.)
            .fold(r, f32::min)
            .max(0.);
        let (inner_lo, inner_hi) = (lo.map(|v| v + r), hi.map(|v| v - r));
        // On each face half an edge's quarter circle, in two steps of 22.5°.
        let steps = |k: usize| -> Vec<f32> {
            let mut s = vec![lo[k]];
            if r > 0. {
                let t = r * (1. - (std::f32::consts::PI / 8.).tan());
                s.extend([lo[k] + t, lo[k] + r, hi[k] - r, hi[k] - t]);
            }
            s.push(hi[k]);
            s.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
            s
        };
        let place = |q: [f32; 3], face: [f32; 3]| -> ([f32; 3], [f32; 3]) {
            let c: [f32; 3] = std::array::from_fn(|k| q[k].clamp(inner_lo[k], inner_hi[k]));
            let d = sub(q, c);
            let length = dot(d, d).sqrt();
            if length < 1e-5 {
                (q, face)
            } else {
                let n = d.map(|v| v / length);
                (std::array::from_fn(|k| c[k] + r * n[k]), n)
            }
        };
        for k in 0..3 {
            let (a, b) = ((k + 1) % 3, (k + 2) % 3);
            let (sa, sb) = (steps(a), steps(b));
            for (side, at) in [(-1., lo[k]), (1., hi[k])] {
                let mut face = [0.; 3];
                face[k] = side;
                for i in 0..sa.len() - 1 {
                    for j in 0..sb.len() - 1 {
                        let corner = |u: f32, v: f32| {
                            let mut q = [0.; 3];
                            (q[k], q[a], q[b]) = (at, u, v);
                            place(q, face)
                        };
                        let c = [
                            corner(sa[i], sb[j]),
                            corner(sa[i + 1], sb[j]),
                            corner(sa[i + 1], sb[j + 1]),
                            corner(sa[i], sb[j + 1]),
                        ];
                        self.quad(solid, c.map(|c| c.0), c.map(|c| c.1), finish, part);
                    }
                }
            }
        }
    }

    /// A cylinder of radius `r` from `a` to `b` in `sides` sides, smooth or
    /// faceted (a hex nut), with flat ends.
    #[allow(clippy::too_many_arguments)]
    fn tube(
        &mut self,
        a: [f32; 3],
        b: [f32; 3],
        r: f32,
        sides: usize,
        smooth: bool,
        finish: Finish,
        part: &'static str,
    ) {
        let solid = self.solid();
        let d = sub(b, a);
        let length = dot(d, d).sqrt().max(1e-6);
        let d = d.map(|v| v / length);
        // Two directions square to the axis and to each other.
        let helper = if d[2].abs() < 0.9 {
            [0., 0., 1.]
        } else {
            [1., 0., 0.]
        };
        let u = normalized(cross(helper, d));
        let v = cross(d, u);
        let around = |t: f32| -> [f32; 3] {
            let (s, c) = t.sin_cos();
            std::array::from_fn(|k| c * u[k] + s * v[k])
        };
        let angle = |i: usize| i as f32 * std::f32::consts::TAU / sides as f32;
        let ring = |at: [f32; 3], i: usize| -> [f32; 3] {
            let o = around(angle(i));
            std::array::from_fn(|k| at[k] + r * o[k])
        };
        for i in 0..sides {
            let n = if smooth {
                [around(angle(i)), around(angle(i + 1))]
            } else {
                [around((angle(i) + angle(i + 1)) / 2.); 2]
            };
            self.quad(
                solid,
                [ring(a, i), ring(a, i + 1), ring(b, i + 1), ring(b, i)],
                [n[0], n[1], n[1], n[0]],
                finish,
                part,
            );
        }
        for (end, n) in [(a, d.map(|v| -v)), (b, d)] {
            let pts: Vec<[f32; 3]> = (0..sides).map(|i| ring(end, i)).collect();
            self.polygon(solid, &pts, n, finish, part);
        }
    }

    /// A solid between two convex end polygons of as many corners: smooth
    /// sides round the line through their middles, or flat ones.
    fn prism(&mut self, ends: [&[[f32; 3]]; 2], smooth: bool, finish: Finish, part: &'static str) {
        let solid = self.solid();
        let n = ends[0].len().min(ends[1].len());
        if n < 3 {
            return;
        }
        let middle = |p: &[[f32; 3]]| -> [f32; 3] {
            std::array::from_fn(|k| p.iter().map(|q| q[k]).sum::<f32>() / p.len() as f32)
        };
        let (c0, c1) = (middle(&ends[0][..n]), middle(&ends[1][..n]));
        let centre: [f32; 3] = std::array::from_fn(|k| (c0[k] + c1[k]) / 2.);
        let axis = normalized(sub(c1, c0));
        // A flat face's normal, turned away from the solid's middle.
        let outward = |pts: &[[f32; 3]]| -> [f32; 3] {
            let normal = normalized(cross(sub(pts[1], pts[0]), sub(pts[pts.len() - 1], pts[0])));
            let mid = middle(pts);
            if dot(normal, sub(mid, centre)) < 0. {
                normal.map(|v| -v)
            } else {
                normal
            }
        };
        let radial = |p: [f32; 3], c: [f32; 3]| {
            let d = sub(p, c);
            normalized(sub(d, axis.map(|v| v * dot(d, axis))))
        };
        for i in 0..n {
            let j = (i + 1) % n;
            let p = [ends[0][i], ends[0][j], ends[1][j], ends[1][i]];
            let normals = if smooth {
                [
                    radial(p[0], c0),
                    radial(p[1], c0),
                    radial(p[2], c1),
                    radial(p[3], c1),
                ]
            } else {
                [outward(&p); 4]
            };
            if dot(normals[0], normals[0]) > 0.5 {
                self.quad(solid, p, normals, finish, part);
            }
        }
        for end in ends {
            self.polygon(solid, &end[..n], outward(&end[..n]), finish, part);
        }
    }

    /// A NEMA stepper motor along axis `k` (0, 1, 2), its mounting face at
    /// `at` (the middle of the face), its body toward `dir` (±1): silver end
    /// caps either side of the black stator, and the shaft out of its face.
    fn stepper(
        &mut self,
        at: [f32; 3],
        k: usize,
        dir: f32,
        finish_tint: impl Fn(Finish) -> Finish,
    ) {
        let half = MOTOR[0] / 2.;
        let span = |from: f32, to: f32, half: f32| {
            let mut lo = at.map(|v| v - half);
            let mut hi = at.map(|v| v + half);
            (lo[k], hi[k]) = (at[k] + dir * from, at[k] + dir * to);
            if lo[k] > hi[k] {
                std::mem::swap(&mut lo[k], &mut hi[k]);
            }
            (lo, hi)
        };
        for (from, to, half, finish) in [
            (0., 10., half, ALUMINIUM),
            (10., MOTOR[1] - 10., half - 1., BLACK),
            (MOTOR[1] - 10., MOTOR[1], half, ALUMINIUM),
        ] {
            let (lo, hi) = span(from, to, half);
            self.rounded(lo, hi, 4., finish_tint(finish), "motor");
        }
        let mut tip = at;
        tip[k] -= dir * 18.;
        self.tube(at, tip, 4., 12, true, finish_tint(CHROME), "motor");
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalized(a: [f32; 3]) -> [f32; 3] {
    let length = dot(a, a).sqrt();
    if length < 1e-9 {
        [0.; 3]
    } else {
        a.map(|v| v / length)
    }
}

/// The table's top in machine Z: the nominal tool's tip at Z top stands the
/// spindle nose's height, less the tool and its nut, over it; with the
/// rotary unit, the unit's centre height under its centreline.
fn table_top(fig: &Figures, rotary: Option<&crate::chuck::Rig>) -> f32 {
    rotary.map_or(TOOL[1] + NUT - fig.nose_to_table, |r| r.table)
}

/// The Jingyan with its tool tip at `pos` (machine X, Y, Z).
#[cfg(test)]
fn plain(pos: [f32; 3], known: bool) -> Model {
    machine(&Figures::JINGYAN, pos, known, None, 0.)
}

/// The machine of figures `fig` with its tool tip at `pos`, and the rotary
/// unit `rotary` with its chuck at A `a` (degrees). The gantry runs in Y on
/// round rails beside the T-slot table, its beam the gantry clearance over
/// the table (over the unit's chuck, with one), its side plates the
/// clearance width apart but never over the table. On the beam's front the
/// X carriage runs on two rods and holds the Z axis, its motor on top and
/// the spindle's clamp sliding down its front.
fn machine(
    fig: &Figures,
    pos: [f32; 3],
    known: bool,
    rotary: Option<&crate::chuck::Rig>,
    a: f32,
) -> Model {
    // The moving parts take an amber tinge until the machine position is
    // set; the caption says why.
    let moving = |f: Finish| match known {
        true => f,
        false => Finish {
            color: std::array::from_fn(|k| f.color[k] * 0.7 + [0.84, 0.59, 0.24][k] * 0.3),
            ..f
        },
    };
    let [x, y, z] = pos;
    let travel = fig.travel;
    let tt = table_top(fig, rotary);
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
    let mut m = Model::default();

    // The side plates stand on the rails beside the table, the clearance
    // width apart when that is wider.
    let half = fig.gantry[1].max(tx[1] - tx[0] + 60.) / 2.;
    let open = [travel[0] / 2. - half, travel[0] / 2. + half];
    let plates = [[open[0] - 20., open[0]], [open[1], open[1] + 20.]];
    let rails = plates.map(|[a, b]| (a + b) / 2.);
    // From the spindle's axis back: its clamp, the Z slide, the X
    // carriage, its bearing blocks, the X rods and the beam.
    let carriage_y = [y + r + 18., y + r + 30.];
    let rods_y = y + r + 50.;
    let beam_y = [y + r + 58., y + r + 118.];
    let beam = [tt + clearance, tt + clearance + 80.];
    let back = travel[1] + r + 150.;
    let front = ty[0] - 30.;
    let rod_z = tt + ROD[1];

    // The base: an aluminium frame of two long extrusions under the rails
    // and cross members under the table, on rubber feet.
    let frame_z = [tt - 62., tt - 22.];
    for x0 in rails {
        m.rounded(
            [x0 - 25., front, frame_z[0]],
            [x0 + 25., back, frame_z[1]],
            3.,
            ALUMINIUM,
            "base",
        );
        for y0 in [front + 25., back - 25.] {
            m.tube(
                [x0, y0, frame_z[0]],
                [x0, y0, frame_z[0] - 14.],
                14.,
                20,
                true,
                RUBBER,
                "base",
            );
        }
    }
    let cross_x = [rails[0] + 25., rails[1] - 25.];
    for y0 in [front, ty[0] + (ty[1] - ty[0]) / 2. - 20., ty[1] - 40.] {
        m.rounded(
            [cross_x[0], y0, frame_z[0] + 4.],
            [cross_x[1], y0 + 40., frame_z[1] + 2.],
            3.,
            ALUMINIUM,
            "base",
        );
    }
    // The T-slot table: slats on a floor, the slots along Y about every
    // 85 mm.
    m.rounded(
        [tx[0], ty[0], tt - 20.],
        [tx[1], ty[1], tt - 12.],
        1.5,
        TABLE,
        "table",
    );
    let slots = ((tx[1] - tx[0]) / 85.).round().max(2.) as usize;
    let pitch = (tx[1] - tx[0]) / slots as f32;
    for i in 0..slots {
        let (x0, x1) = (tx[0] + pitch * i as f32, tx[0] + pitch * (i + 1) as f32);
        let (x0, x1) = (
            if i == 0 { x0 } else { x0 + 4. },
            if i == slots - 1 { x1 } else { x1 - 4. },
        );
        m.rounded([x0, ty[0], tt - 12.], [x1, ty[1], tt], 1.5, TABLE, "table");
    }
    // The Y rails: chrome rods on their supports along each long
    // extrusion, the Y motors at their back ends.
    for x0 in rails {
        m.rounded(
            [x0 - 12., front, frame_z[1]],
            [x0 + 12., back, rod_z - 2.],
            2.,
            ALUMINIUM,
            "rails",
        );
        m.tube(
            [x0, front, rod_z],
            [x0, back, rod_z],
            ROD[0],
            20,
            true,
            CHROME,
            "rails",
        );
        m.stepper([x0, back, rod_z], 1, 1., |f| f);
    }

    // The gantry, moving in Y.
    let blue = moving(ANODISED);
    let blocks_z = [rod_z - 4., rod_z + ROD[0] + 14.];
    for (i, [x0, x1]) in plates.into_iter().enumerate() {
        // Bearing blocks on the rod, under the plate.
        for y0 in [beam_y[0] - 50., beam_y[1] - 10.] {
            m.rounded(
                [rails[i] - 22., y0, blocks_z[0]],
                [rails[i] + 22., y0 + 40., blocks_z[1]],
                3.,
                moving(ALUMINIUM),
                "gantry blocks",
            );
        }
        // The side plate, wider at its foot.
        let (bottom, top) = (blocks_z[1], beam[1]);
        let outline = [
            [beam_y[0] - 60., bottom],
            [beam_y[1] + 30., bottom],
            [beam_y[1] + 5., top],
            [beam_y[0], top],
        ];
        let end = |x: f32| -> Vec<[f32; 3]> { outline.iter().map(|&[v, w]| [x, v, w]).collect() };
        m.prism(
            [end(x0).as_slice(), end(x1).as_slice()],
            false,
            blue,
            "gantry",
        );
    }
    m.rounded(
        [plates[0][0], beam_y[0], beam[0]],
        [plates[1][1], beam_y[1], beam[1]],
        5.,
        moving(ALUMINIUM),
        "beam",
    );
    // The X rods and the ball screw between them, on the beam's front.
    let rods_z = [beam[0] + 18., beam[1] - 18.];
    for z0 in rods_z {
        m.tube(
            [open[0], rods_y, z0],
            [open[1], rods_y, z0],
            ROD[0],
            20,
            true,
            moving(CHROME),
            "x rods",
        );
    }
    let screw_z = (rods_z[0] + rods_z[1]) / 2.;
    m.tube(
        [open[0], rods_y, screw_z],
        [open[1], rods_y, screw_z],
        6.,
        12,
        true,
        moving(STEEL),
        "x rods",
    );
    m.stepper([plates[1][1], rods_y, screw_z], 0, 1., moving);

    // The X carriage, the Z axis on it, its motor on top. The Z slide
    // reaches from just over the nose to most of the spindle's height; the
    // column it slides on spans it from Z top to the bottom of the Z
    // travel.
    let zero_nose = TOOL[1] + NUT;
    let slide = [20., spindle[1] * 0.9];
    let column = [
        (beam[0] - 25.).min(zero_nose - travel[2] + slide[0]),
        zero_nose + slide[1] + 15.,
    ];
    let carriage_top = column[1];
    m.rounded(
        [x - 45., rods_y - 8., rods_z[0] - 22.],
        [x + 45., rods_y + 8., rods_z[1] + 22.],
        3.,
        moving(ALUMINIUM),
        "carriage",
    );
    m.rounded(
        [x - 62., carriage_y[0], column[0]],
        [x + 62., rods_y - 8., carriage_top],
        3.,
        moving(ALUMINIUM),
        "carriage",
    );
    m.stepper(
        [x, (carriage_y[0] + rods_y) / 2. - 6., carriage_top],
        2,
        1.,
        moving,
    );
    // The Z slide moves with Z and holds the spindle's clamp.
    m.rounded(
        [x - 50., y + r + 6., nose + slide[0]],
        [x + 50., carriage_y[0], nose + slide[1]],
        3.,
        moving(ALUMINIUM),
        "z plate",
    );
    m.rounded(
        [x - r - 12., y - r - 12., nose + spindle[1] * 0.4],
        [x + r + 12., y + r + 6., nose + spindle[1] * 0.75],
        4.,
        moving(ALUMINIUM),
        "clamp",
    );
    // The spindle: its nose, body, black end cap and plug, the collet nut
    // and the tool.
    let at = |z0: f32| [x, y, z0];
    m.tube(
        at(nose),
        at(nose + 8.),
        r * 0.62,
        32,
        true,
        moving(STEEL),
        "spindle",
    );
    m.tube(
        at(nose + 8.),
        at(nose + spindle[1] - 22.),
        r,
        40,
        true,
        moving(CHROME),
        "spindle",
    );
    m.tube(
        at(nose + spindle[1] - 22.),
        at(nose + spindle[1]),
        r,
        40,
        true,
        moving(BLACK),
        "spindle",
    );
    m.tube(
        at(nose + spindle[1]),
        at(nose + spindle[1] + 16.),
        11.,
        20,
        true,
        moving(BLACK),
        "spindle",
    );
    m.tube(
        at(nose - NUT),
        at(nose),
        14.,
        6,
        false,
        moving(CHROME),
        "nut",
    );
    m.tube(
        at(z + 12.),
        at(z + TOOL[1]),
        TOOL[0] / 2.,
        16,
        true,
        moving(CHROME),
        "tool",
    );
    m.tube(
        at(z),
        at(z + 12.),
        TOOL[0] / 2.,
        16,
        true,
        moving(CARBIDE),
        "tool",
    );

    if let Some(rig) = rotary {
        rotary_solids(&mut m, rig, a);
    }
    m
}

/// The rotary unit's solids at A `a` (`chuck::Rig::solids`): the chuck
/// and its jaws turn with A, the head and the tailstock stand still.
fn rotary_solids(m: &mut Model, rig: &crate::chuck::Rig, a: f32) {
    for (label, prism, _) in rig.solids(a) {
        let (part, finish) = match label.as_str() {
            "chuck body" => ("chuck", CHROME),
            l if l.starts_with("chuck jaw") => ("jaw", STEEL),
            "rotary head" => ("rotary head", ANODISED),
            _ => ("tailstock", ALUMINIUM),
        };
        let [a, b] = &prism.ends;
        m.prism([a.as_slice(), b.as_slice()], prism.smooth, finish, part);
    }
}

/// The simulator's block, standing on a vise when it is over the table:
/// the vise's body under it, a jaw either side in Y and the handle in
/// front.
fn block(m: &mut Model, [lo, hi]: [[f32; 3]; 2], table: f32) {
    m.rounded(lo, hi, 0.8, STOCK, "stock");
    if lo[2] < table + 6. {
        return;
    }
    let grip = ((hi[2] - lo[2]) / 2.).min(10.);
    m.rounded(
        [lo[0] - 15., lo[1] - 45., table],
        [hi[0] + 15., hi[1] + 45., lo[2] - 2.],
        3.,
        STEEL,
        "vise",
    );
    for (y0, y1) in [(lo[1] - 20., lo[1]), (hi[1], hi[1] + 20.)] {
        m.rounded(
            [lo[0] - 15., y0, lo[2] - 2.],
            [hi[0] + 15., y1, lo[2] + grip],
            2.,
            STEEL,
            "vise",
        );
    }
    let mid = [(lo[0] + hi[0]) / 2., lo[1] - 45., (table + lo[2]) / 2.];
    m.tube(
        mid,
        [mid[0], mid[1] - 40., mid[2]],
        6.,
        16,
        true,
        CHROME,
        "vise",
    );
    m.tube(
        [mid[0] - 50., mid[1] - 40., mid[2]],
        [mid[0] + 50., mid[1] - 40., mid[2]],
        5.,
        16,
        true,
        CHROME,
        "vise",
    );
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

/// The light from above the operator's left shoulder, and a fainter one
/// from the right.
const KEY: [f32; 3] = [-0.42, -0.6, 0.68];
const FILL: [f32; 3] = [0.75, 0.25, 0.35];

/// `finish` lit where its surface's normal is `n` and the eye is toward
/// `eye`: a little light from everywhere (more from above), the two lights,
/// and a white highlight that is sharper and brighter the more it shines.
fn lit(finish: Finish, n: [f32; 3], eye: [f32; 3]) -> [f32; 3] {
    let n = normalized(n);
    let key = normalized(KEY);
    let ambient = 0.26 + 0.14 * (0.5 + 0.5 * n[2]);
    let diffuse = 0.58 * dot(n, key).max(0.) + 0.18 * dot(n, normalized(FILL)).max(0.);
    let matte = 1. - 0.3 * finish.shine;
    let half = normalized(std::array::from_fn(|k| key[k] + eye[k]));
    let spot = dot(n, half).max(0.).powf(8. + 72. * finish.shine) * (0.12 + 0.55 * finish.shine);
    // Polished metal mirrors the bright room above it.
    let sky = 0.18 * finish.shine * (0.5 + 0.5 * n[2]);
    finish
        .color
        .map(|c| (c * (ambient + diffuse) * matte + c * sky + spot).min(1.))
}

/// A line of the scene: from, to, its colour, whether it is dashed.
type Line = ([f32; 3], [f32; 3], [u8; 3], bool);

/// A triangle facing the eye, projected: its corners in samples with their
/// depth, its area on the frame (twice, signed), the rows and columns it
/// covers, and which of the model's it is.
struct Flat {
    v: [(f32, f32, f32); 3],
    area: f32,
    rows: (u32, u32),
    cols: (u32, u32),
    index: u32,
}

/// The model's `tris` and the `lines` drawn into a `w` × `h` frame, its
/// points placed by `place` (frame pixels and depth, larger nearer), at
/// `ss` × `ss` samples a pixel: RGBA, alpha 0 where nothing is. The rows
/// are shared out among the cores, each drawing every triangle that
/// reaches its band.
fn render(
    tris: &[Tri],
    lines: &[Line],
    view: &View,
    (w, h): (u32, u32),
    ss: u32,
    place: impl Fn([f32; 3]) -> (Point, f32),
) -> Vec<u8> {
    let (sw, sh) = (w * ss, h * ss);
    let cells = (sw * sh) as usize;
    let k = ss as f32;
    // The model's solids are closed, so a triangle turned away from the
    // eye is behind another of its own.
    let flats: Vec<Flat> = (tris.iter().enumerate())
        .filter_map(|(i, t)| {
            let facing = t.normals.iter().map(|n| dot(*n, view.toward)).sum::<f32>() / 3.;
            if facing < -0.35 {
                return None;
            }
            let v = t.pts.map(|p| {
                let (q, z) = place(p);
                (q.x * k, q.y * k, z)
            });
            let area =
                (v[1].0 - v[0].0) * (v[2].1 - v[0].1) - (v[2].0 - v[0].0) * (v[1].1 - v[0].1);
            if area.abs() < 1e-6 {
                return None;
            }
            let range = |c: fn(&(f32, f32, f32)) -> f32, max: u32| {
                let lo = v.iter().map(c).fold(f32::INFINITY, f32::min);
                let hi = v.iter().map(c).fold(f32::NEG_INFINITY, f32::max);
                (
                    lo.floor().clamp(0., max as f32) as u32,
                    hi.ceil().clamp(0., max as f32) as u32,
                )
            };
            let (rows, cols) = (range(|p| p.1, sh), range(|p| p.0, sw));
            (rows.0 < rows.1 && cols.0 < cols.1).then_some(Flat {
                v,
                area,
                rows,
                cols,
                index: i as u32,
            })
        })
        .collect();
    let mut depth = vec![f32::NEG_INFINITY; cells];
    let mut normal = vec![[0f32; 3]; cells];
    let mut owner = vec![u32::MAX; cells];
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()) as u32;
    let band = sh.div_ceil(threads).max(1);
    let size = (band * sw) as usize;
    std::thread::scope(|scope| {
        let bands = (depth.chunks_mut(size))
            .zip(normal.chunks_mut(size))
            .zip(owner.chunks_mut(size));
        for (b, ((depth, normal), owner)) in bands.enumerate() {
            let top = b as u32 * band;
            let bottom = top + depth.len() as u32 / sw;
            let flats = &flats;
            scope.spawn(move || {
                for f in flats.iter().filter(|f| f.rows.1 > top && f.rows.0 < bottom) {
                    let t = &tris[f.index as usize];
                    let v = f.v;
                    // Each corner's weight is linear across the frame: its
                    // value at a row's first sample and its step a column.
                    let weight = |a: usize, b: usize, qx: f32, qy: f32| {
                        ((v[b].0 - v[a].0) * (qy - v[a].1) - (qx - v[a].0) * (v[b].1 - v[a].1))
                            / f.area
                    };
                    let step = [
                        -(v[2].1 - v[1].1) / f.area,
                        -(v[0].1 - v[2].1) / f.area,
                        -(v[1].1 - v[0].1) / f.area,
                    ];
                    for py in f.rows.0.max(top)..f.rows.1.min(bottom) {
                        let (qx, qy) = (f.cols.0 as f32 + 0.5, py as f32 + 0.5);
                        let mut wts = [
                            weight(1, 2, qx, qy),
                            weight(2, 0, qx, qy),
                            weight(0, 1, qx, qy),
                        ];
                        let row = ((py - top) * sw) as usize;
                        for px in f.cols.0..f.cols.1 {
                            if wts[0] >= -1e-5 && wts[1] >= -1e-5 && wts[2] >= -1e-5 {
                                let z = wts[0] * v[0].2 + wts[1] * v[1].2 + wts[2] * v[2].2;
                                let cell = row + px as usize;
                                if z > depth[cell] {
                                    depth[cell] = z;
                                    owner[cell] = f.index;
                                    normal[cell] = std::array::from_fn(|c| {
                                        wts[0] * t.normals[0][c]
                                            + wts[1] * t.normals[1][c]
                                            + wts[2] * t.normals[2][c]
                                    });
                                }
                            }
                            for c in 0..3 {
                                wts[c] += step[c];
                            }
                        }
                    }
                }
            });
        }
    });
    // Lit, then darkened along the edges: where a solid meets another or
    // the empty frame, and where its surface turns sharply.
    let mut rgba = vec![0u8; cells * 4];
    let edge_at = |a: usize, b: usize| -> bool {
        match (owner[a], owner[b]) {
            (u32::MAX, u32::MAX) => false,
            (u32::MAX, _) | (_, u32::MAX) => true,
            (i, j) => {
                tris[i as usize].solid != tris[j as usize].solid
                    || dot(normalized(normal[a]), normalized(normal[b])) < 0.75
            }
        }
    };
    std::thread::scope(|scope| {
        for (b, pixels) in rgba.chunks_mut(size * 4).enumerate() {
            let first = b * size;
            let (owner, normal, edge_at) = (&owner, &normal, &edge_at);
            scope.spawn(move || {
                for (i, px) in pixels.chunks_mut(4).enumerate() {
                    let cell = first + i;
                    let o = owner[cell];
                    if o == u32::MAX {
                        continue;
                    }
                    let (x, y) = (cell as u32 % sw, cell as u32 / sw);
                    let edge = (x + 1 < sw && edge_at(cell, cell + 1))
                        || (x > 0 && edge_at(cell, cell - 1))
                        || (y + 1 < sh && edge_at(cell, cell + sw as usize))
                        || (y > 0 && edge_at(cell, cell - sw as usize));
                    let mut c = lit(tris[o as usize].finish, normal[cell], view.toward);
                    if edge {
                        c = c.map(|v| v * 0.45);
                    }
                    px.copy_from_slice(&[
                        (c[0] * 255.) as u8,
                        (c[1] * 255.) as u8,
                        (c[2] * 255.) as u8,
                        255,
                    ]);
                }
            });
        }
    });
    // The lines, solid where nothing is in front of them, faint and dashed
    // where something is.
    let tolerance = 0.8;
    for &(a, b, color, dashed) in lines {
        let ((pa, za), (pb, zb)) = (place(a), place(b));
        let (pa, pb) = ((pa.x * k, pa.y * k), (pb.x * k, pb.y * k));
        let length = ((pb.0 - pa.0).powi(2) + (pb.1 - pa.1).powi(2)).sqrt();
        let steps = (length.ceil() as usize).clamp(1, 20000);
        let radius = 0.75 * k;
        for s in 0..=steps {
            let t = s as f32 / steps as f32;
            let along = t * length / k;
            let (qx, qy) = (pa.0 + (pb.0 - pa.0) * t, pa.1 + (pb.1 - pa.1) * t);
            let z = za + (zb - za) * t;
            for py in (qy - radius).floor().max(0.) as u32
                ..((qy + radius).ceil().clamp(0., sh as f32) as u32)
            {
                for px in (qx - radius).floor().max(0.) as u32
                    ..((qx + radius).ceil().clamp(0., sw as f32) as u32)
                {
                    let cell = (py * sw + px) as usize;
                    let hidden = z + tolerance < depth[cell];
                    let gap = (dashed || hidden) && along % 10. > 6.;
                    if gap {
                        continue;
                    }
                    let alpha = if hidden { 0.3 } else { 0.9 };
                    blend(&mut rgba[cell * 4..cell * 4 + 4], color, alpha);
                }
            }
        }
    }
    // Down to the frame: each pixel the mean of its samples, weighted by
    // how much of each is drawn.
    let mut out = vec![0u8; (w * h * 4) as usize];
    for py in 0..h {
        for px in 0..w {
            let mut sum = [0f32; 4];
            for dy in 0..ss {
                for dx in 0..ss {
                    let cell = (((py * ss + dy) * sw + px * ss + dx) * 4) as usize;
                    let a = rgba[cell + 3] as f32;
                    for c in 0..3 {
                        sum[c] += rgba[cell + c] as f32 * a;
                    }
                    sum[3] += a;
                }
            }
            let i = ((py * w + px) * 4) as usize;
            if sum[3] > 0. {
                for c in 0..3 {
                    out[i + c] = (sum[c] / sum[3]) as u8;
                }
                out[i + 3] = (sum[3] / (ss * ss) as f32) as u8;
            }
        }
    }
    out
}

/// `color` laid over the pixel `px` at `alpha`.
fn blend(px: &mut [u8], color: [u8; 3], alpha: f32) {
    let under = px[3] as f32 / 255.;
    let out = alpha + under * (1. - alpha);
    if out <= 0. {
        return;
    }
    for c in 0..3 {
        px[c] = ((color[c] as f32 * alpha + px[c] as f32 * under * (1. - alpha)) / out) as u8;
    }
    px[3] = (out * 255.) as u8;
}

/// The view's drag state, orbit with the left button, and the picture last
/// drawn with what it was drawn from.
pub struct Orbit {
    yaw: f32,
    pitch: f32,
    drag: Option<Point>,
    picture: RefCell<Option<(String, Picture)>>,
}

/// The model drawn, the scale and origin it was drawn at, and the box round
/// its base, for the marks and the shadow drawn with it.
struct Picture {
    frame: crate::viewport::RasterFrame,
    scale: f32,
    origin: Point,
    foot: ([f32; 3], [f32; 3]),
}

impl Default for Orbit {
    fn default() -> Self {
        Orbit {
            yaw: 0.35,
            pitch: 0.5,
            drag: None,
            picture: RefCell::new(None),
        }
    }
}

/// The colours of the box: the profile's, the assumed one and the one shown
/// before the position is set.
const AMBER: Color = Color::from_rgb(196. / 255., 120. / 255., 20. / 255.);

impl MachineView {
    fn box_color(&self) -> Color {
        if self.assumed {
            Color { a: 0.75, ..AMBER }
        } else if self.known {
            Color::from_rgba8(33, 107, 181, 0.85)
        } else {
            AMBER
        }
    }

    /// The model and the frame round it: the scale and where the origin
    /// goes, so the machine fits the frame at both ends of its X and Y
    /// travel and the scale holds still while it moves.
    fn placed(&self, view: &View, size: iced::Size) -> (Model, f32, Point) {
        let rotary = self.rotary.as_ref();
        let [tx, ty, _] = self.figures.travel;
        let ends = [[0., 0., 0.], [tx, ty, 0.]];
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        let mut grow = |p: [f32; 3]| {
            let q = view.flat(p);
            lo = [lo[0].min(q[0]), lo[1].min(q[1])];
            hi = [hi[0].max(q[0]), hi[1].max(q[1])];
        };
        for end in ends {
            let at = machine(&self.figures, end, true, rotary, 0.);
            at.tris.iter().flat_map(|t| t.pts).for_each(&mut grow);
        }
        let [elo, ehi] = self.envelope;
        for i in 0..8 {
            grow(std::array::from_fn(|k| {
                if i >> k & 1 == 0 { elo[k] } else { ehi[k] }
            }));
        }
        let (width, height) = (hi[0] - lo[0], hi[1] - lo[1]);
        let scale = (size.width / width).min((size.height - 40.) / height) * 0.95;
        let origin = Point::new(
            (size.width - width * scale) / 2. - lo[0] * scale,
            (size.height - height * scale) / 2. - lo[1] * scale + 6.,
        );
        let mut model = machine(&self.figures, self.pos, self.known, rotary, self.a);
        if let Some(b) = self.block {
            block(&mut model, b, table_top(&self.figures, rotary));
        }
        (model, scale, origin)
    }

    /// The edges of the travel box, dashed when it is the assumed one.
    fn box_lines(&self) -> Vec<Line> {
        let c = self.box_color();
        let rgb = [c.r, c.g, c.b].map(|v| (v * 255.) as u8);
        let [lo, hi] = self.envelope;
        let corner = |i: usize| -> [f32; 3] {
            std::array::from_fn(|k| if i >> k & 1 == 0 { lo[k] } else { hi[k] })
        };
        let mut lines = vec![];
        for i in 0..8usize {
            for k in 0..3 {
                if i >> k & 1 == 0 {
                    lines.push((corner(i), corner(i | 1 << k), rgb, self.assumed));
                }
            }
        }
        lines
    }

    /// What the picture is drawn from, to keep it while that is the same.
    fn key(&self, state: &Orbit, bounds: Rectangle) -> String {
        format!(
            "{:?}|{:?}|{}|{:?}|{}|{:?}|{:?}|{:?}|{}|{}|{}|{:?}",
            self.figures,
            self.pos,
            self.known,
            self.envelope,
            self.assumed,
            self.block,
            self.zero,
            self.rotary,
            self.a,
            state.yaw,
            state.pitch,
            bounds.size()
        )
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
        let size = bounds.size();
        let mut back = canvas::Frame::new(renderer, size);
        // A light room, darker toward the floor.
        back.fill_rectangle(
            Point::ORIGIN,
            size,
            canvas::gradient::Linear::new(Point::ORIGIN, Point::new(0., size.height))
                .add_stop(0., Color::from_rgb8(244, 247, 250))
                .add_stop(1., Color::from_rgb8(212, 220, 228)),
        );
        let view = View::new(state.yaw, state.pitch);
        // The model, drawn again only when what it shows has changed.
        let key = self.key(state, bounds);
        let mut cached = state.picture.borrow_mut();
        if cached.as_ref().is_none_or(|(k, _)| *k != key) {
            let (model, scale, origin) = self.placed(&view, size);
            // ponytail: one core, about 4 million samples a frame at most;
            // split the rows over cores if a large window drags.
            let ratio = (1_000_000. / (size.width * size.height).max(1.))
                .sqrt()
                .clamp(0.5, 1.5);
            let (w, h) = (
                (size.width * ratio).ceil().max(1.) as u32,
                (size.height * ratio).ceil().max(1.) as u32,
            );
            let place = |p: [f32; 3]| {
                let q = view.flat(p);
                let at = |o: f32, v: f32| (o + v * scale) * ratio;
                (
                    Point::new(at(origin.x, q[0]), at(origin.y, q[1])),
                    dot(p, view.toward),
                )
            };
            let rgba = render(&model.tris, &self.box_lines(), &view, (w, h), 2, place);
            let foot = (model.tris.iter())
                .filter(|t| t.part == "base")
                .flat_map(|t| t.pts)
                .fold(([f32::MAX; 3], [f32::MIN; 3]), |(lo, hi), p| {
                    (
                        std::array::from_fn(|k| lo[k].min(p[k])),
                        std::array::from_fn(|k| hi[k].max(p[k])),
                    )
                });
            let frame = crate::viewport::RasterFrame::from_rgba(w, h, &rgba);
            *cached = Some((
                key,
                Picture {
                    frame,
                    scale,
                    origin,
                    foot,
                },
            ));
        }
        let shown = &cached.as_ref().unwrap().1;
        let (scale, origin, foot) = (shown.scale, shown.origin, shown.foot);
        let screen = |p: [f32; 3]| {
            let q = view.flat(p);
            Point::new(origin.x + q[0] * scale, origin.y + q[1] * scale)
        };
        // A soft shadow round the base's foot.
        for grow in [90., 75., 60., 48., 36., 26., 16., 8.] {
            let (lo, hi) = foot;
            let corners = [
                [lo[0] - grow, lo[1] - grow, lo[2]],
                [hi[0] + grow, lo[1] - grow, lo[2]],
                [hi[0] + grow, hi[1] + grow, lo[2]],
                [lo[0] - grow, hi[1] + grow, lo[2]],
            ];
            let path = canvas::Path::new(|b| {
                b.move_to(screen(corners[0]));
                for c in &corners[1..] {
                    b.line_to(screen(*c));
                }
                b.close();
            });
            back.fill(&path, Color::from_rgba8(60, 76, 92, 0.035));
        }
        let mut picture = canvas::Frame::new(renderer, size);
        shown.frame.draw(&mut picture, size);
        // The work zero and the tool tip over it, and the captions.
        let mut marks = canvas::Frame::new(renderer, size);
        let green = Color::from_rgb8(39, 130, 93);
        if self.known {
            marks.stroke(
                &canvas::Path::circle(screen(self.zero), 7.),
                canvas::Stroke::default().with_color(green).with_width(2.),
            );
        }
        marks.fill(
            &canvas::Path::circle(screen(self.pos), 4.),
            Color::from_rgb8(214, 40, 40),
        );
        // The captions are cut to the view's width (about 6.2 px a character
        // at 12 px), so that a narrow view does not run them past its edge.
        let fits = |px: f32| ((bounds.width - 20.) / px).max(1.) as usize;
        for (i, (kind, content)) in self.captions().into_iter().enumerate() {
            marks.fill_text(canvas::Text {
                content: crate::ui::ellipsize(&content, fits(6.2)),
                position: Point::new(10., 8. + 16. * i as f32),
                color: match kind {
                    Caption::Travel => self.box_color(),
                    Caption::Assumed | Caption::Unset | Caption::Mismatch => AMBER,
                    Caption::Zero => green,
                },
                size: 12.into(),
                ..Default::default()
            });
        }
        let [x, y, z] = self.figures.travel.map(crate::envelope::mm);
        let travel = format!("Travel {x} × {y} × {z} mm from the machine library");
        let foot = match &self.rotary {
            Some(_) => format!(
                "{travel} · rotary unit as the library places it, A {}° · drag to turn",
                crate::indexed::degrees(self.a)
            ),
            None => format!("{travel} · drag to turn"),
        };
        marks.fill_text(canvas::Text {
            content: crate::ui::ellipsize(&foot, fits(5.7)),
            position: Point::new(10., bounds.height - 20.),
            color: Color::from_rgb8(110, 128, 145),
            size: 11.into(),
            ..Default::default()
        });
        vec![
            back.into_geometry(),
            picture.into_geometry(),
            marks.into_geometry(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part<'a>(m: &'a Model, name: &str) -> impl Iterator<Item = [f32; 3]> + 'a {
        let name = name.to_string();
        m.tris
            .iter()
            .filter(move |t| t.part == name)
            .flat_map(|t| t.pts)
    }

    fn lowest(m: &Model, name: &str, k: usize) -> f32 {
        part(m, name).map(|p| p[k]).fold(f32::MAX, f32::min)
    }

    fn highest(m: &Model, name: &str, k: usize) -> f32 {
        part(m, name).map(|p| p[k]).fold(f32::MIN, f32::max)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn the_model_follows_the_machine_position() {
        let home = plain([0., 0., 0.], true);
        let moved = plain([120., 80., -30.], true);
        // The tool tip is the machine position.
        assert!(close(lowest(&moved, "tool", 2), -30.));
        let tip: Vec<[f32; 3]> = part(&moved, "tool").filter(|p| close(p[2], -30.)).collect();
        let centre = |k: usize| {
            let (lo, hi) =
                (tip.iter()).fold((f32::MAX, f32::MIN), |(l, h), p| (l.min(p[k]), h.max(p[k])));
            (lo + hi) / 2.
        };
        assert!(close(centre(0), 120.) && close(centre(1), 80.));
        // The gantry beam moves with Y only; the table never moves.
        assert!(close(
            lowest(&moved, "beam", 1) - lowest(&home, "beam", 1),
            80.
        ));
        assert!(close(lowest(&moved, "beam", 0), lowest(&home, "beam", 0)));
        assert!(close(
            lowest(&moved, "carriage", 0) - lowest(&home, "carriage", 0),
            120.
        ));
        let table = |m: &Model| part(m, "table").collect::<Vec<_>>();
        assert_eq!(table(&home), table(&moved));
        // At the bottom of Z the tool stays above the table.
        let j = Figures::JINGYAN;
        let bottom = plain([0., 0., -j.travel[2]], true);
        assert!(lowest(&bottom, "tool", 2) > table_top(&j, None));
        // Until the position is set, the moving parts change colour.
        let unset = plain([0., 0., 0.], false);
        let finish = |m: &Model, name: &str| m.tris.iter().find(|t| t.part == name).unwrap().finish;
        assert_ne!(finish(&unset, "spindle"), finish(&home, "spindle"));
        assert_eq!(finish(&unset, "table"), finish(&home, "table"));
    }

    #[test]
    fn the_rotary_unit_stands_where_the_library_puts_it_and_its_chuck_turns_with_a() {
        let mut m = crate::model::Machine::new(4);
        m.fill_rotary_defaults();
        let unit = m.rotary_unit().unwrap();
        let rig = crate::chuck::Rig::on_machine(unit, 1.);
        let at = |a: f32| machine(&Figures::JINGYAN, [100., 50., 0.], true, Some(&rig), a);
        let zero = at(0.);
        // The chuck's face at the chuck X, round the centreline at its Y
        // and Z; the table's top its centre height under it.
        let r = unit.chuck_diameter / 2.;
        assert!((highest(&zero, "chuck", 0) - unit.chuck_x).abs() < 1e-3);
        assert!((highest(&zero, "chuck", 2) - (unit.centre_z + r)).abs() < 3.);
        assert!((lowest(&zero, "chuck", 1) - (unit.centre_y - r)).abs() < 3.);
        let table = unit.centre_z - unit.centre_height;
        assert!(close(highest(&zero, "table", 2), table));
        assert!(close(lowest(&zero, "rotary head", 2), table));
        // The jaws stand out of the chuck's face and turn with A about the
        // centreline, +Y up to +Z for positive A.
        assert!(lowest(&zero, "jaw", 0) >= unit.chuck_x - 1e-3);
        let quarter = at(90.);
        let jaws = |m: &Model| part(m, "jaw").collect::<Vec<_>>();
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
        let solid = |m: &Model, name: &str| part(m, name).collect::<Vec<_>>();
        for name in ["rotary head", "tailstock"] {
            assert!(!solid(&zero, name).is_empty());
            assert_eq!(solid(&zero, name), solid(&quarter, name));
        }
        assert!(lowest(&at(0.), "tool", 2) > table);
        assert!(lowest(&zero, "beam", 2) > unit.centre_z + r);
        // Without a rotary unit the machine is the one it was.
        let plain = plain([100., 50., 0.], true);
        assert!(part(&plain, "jaw").next().is_none());
        assert!(close(
            highest(&plain, "table", 2),
            table_top(&Figures::JINGYAN, None)
        ));
    }

    #[test]
    fn the_model_is_drawn_to_the_machines_travel_and_clearances() {
        let j = Figures::JINGYAN;
        let at = |fig: &Figures| machine(fig, [0., 0., 0.], true, None, 0.);
        let shipped = at(&j);
        // The table reaches past the X and Y travel by the same margins.
        let wide = Figures {
            travel: [600., 400., 55.],
            ..j
        };
        let grown = at(&wide);
        let span = |m: &Model, k: usize| highest(m, "table", k) - lowest(m, "table", k);
        assert!(close(span(&grown, 0) - span(&shipped, 0), 210.));
        assert!(close(span(&grown, 1) - span(&shipped, 1), 120.));
        // The base and rails reach behind the gantry at the far end of Y.
        assert!(close(
            highest(&grown, "base", 1) - highest(&shipped, "base", 1),
            120.
        ));
        // The spindle nose stands its height over the table at Z top, and
        // the beam the gantry clearance.
        let nose = lowest(&shipped, "nut", 2) + NUT;
        assert!(close(nose - highest(&shipped, "table", 2), j.nose_to_table));
        let beam = |m: &Model| lowest(m, "beam", 2) - highest(m, "table", 2);
        assert!(close(beam(&shipped), j.gantry[0]));
        let tall = at(&Figures {
            nose_to_table: 220.,
            gantry: [180., j.gantry[1]],
            ..j
        });
        assert!(close(
            highest(&tall, "table", 2),
            highest(&shipped, "table", 2) - 60.
        ));
        assert!(close(beam(&tall), 180.));
        // The Z column reaches down to the bottom of a longer Z travel.
        let deep = at(&Figures {
            travel: [j.travel[0], j.travel[1], 200.],
            ..j
        });
        let column = |m: &Model| lowest(m, "carriage", 2);
        assert!(close(column(&deep), TOOL[1] + NUT - 200. + 20.));
        assert!(column(&deep) < column(&shipped) - 100.);
        // The side plates stand beside the table, the clearance width apart
        // when that is wider.
        let inside = |m: &Model| {
            let mid = j.travel[0] / 2.;
            let xs: Vec<f32> = part(m, "gantry").map(|p| p[0]).collect();
            let left = xs
                .iter()
                .copied()
                .filter(|&v| v < mid)
                .fold(f32::MIN, f32::max);
            let right = xs
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
        assert!(close(right - left, 800.));
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

    /// How long a frame takes: building the model, fitting it, drawing it
    /// at 900 × 640 (run with --release --ignored --nocapture).
    #[test]
    #[ignore]
    fn time_a_frame() {
        let v = MachineView {
            figures: Figures::JINGYAN,
            pos: [150., 120., -10.],
            known: true,
            envelope: [[0., 0., -55.], [390., 280., 0.]],
            assumed: false,
            block: Some([[100., 80., -50.], [220., 170., -30.]]),
            zero: [100., 80., -30.],
            rotary: None,
            a: 0.,
        };
        let view = View::new(0.35, 0.5);
        let size = iced::Size::new(900., 640.);
        let start = std::time::Instant::now();
        let built = plain(v.pos, true);
        let build = start.elapsed();
        let start = std::time::Instant::now();
        let (model, scale, origin) = v.placed(&view, size);
        let fit = start.elapsed();
        let place = |p: [f32; 3]| {
            let q = view.flat(p);
            (
                Point::new(origin.x + q[0] * scale, origin.y + q[1] * scale),
                dot(p, view.toward),
            )
        };
        let start = std::time::Instant::now();
        let _ = render(&model.tris, &v.box_lines(), &view, (900, 640), 2, place);
        let draw = start.elapsed();
        eprintln!(
            "{} triangles: build {build:?}, fit and build {fit:?}, draw {draw:?}",
            built.tris.len()
        );
    }

    #[test]
    fn a_rounded_box_is_closed_and_its_normals_point_out() {
        let mut m = Model::default();
        m.rounded([0., 0., 0.], [40., 20., 10.], 3., STEEL, "box");
        for t in &m.tris {
            for (p, n) in t.pts.iter().zip(t.normals) {
                // On the box, never outside it, and the normal of length
                // one pointing away from its middle.
                assert!((0..3).all(|k| p[k] >= -1e-3 && p[k] <= [40., 20., 10.][k] + 1e-3));
                assert!((dot(n, n) - 1.).abs() < 1e-3);
                assert!(dot(n, sub(*p, [20., 10., 5.])) > 0.);
            }
        }
        // The corners are cut round: no point is at the box's corner.
        assert!(m.tris.iter().flat_map(|t| t.pts).all(|p| dot(p, p) > 0.5));
    }

    #[test]
    fn the_renderer_draws_the_nearest_surface_lit_and_outlined() {
        let mut m = Model::default();
        // Seen from the front, the red box is in front of the blue one.
        m.rounded(
            [-20., -40., -20.],
            [20., -30., 20.],
            0.,
            finish(220, 30, 30, 0.),
            "near",
        );
        m.rounded(
            [-40., 0., -40.],
            [40., 10., 40.],
            0.,
            finish(30, 30, 220, 0.),
            "far",
        );
        let view = View::new(0., 0.);
        let place = |p: [f32; 3]| {
            let q = view.flat(p);
            (Point::new(50. + q[0], 50. + q[1]), dot(p, view.toward))
        };
        let px = render(&m.tris, &[], &view, (100, 100), 2, place);
        let at =
            |x: u32, y: u32| &px[((y * 100 + x) * 4) as usize..((y * 100 + x) * 4 + 4) as usize];
        // The middle shows the near box, the side of the far one the far
        // box, and the corner nothing.
        assert!(at(50, 50)[0] > at(50, 50)[2]);
        assert!(at(80, 50)[2] > at(80, 50)[0]);
        assert_eq!(at(2, 2)[3], 0);
        // The near box's outline is darker than its middle.
        let bright = |p: &[u8]| p[0] as u32 + p[1] as u32 + p[2] as u32;
        assert!(bright(at(30, 50)) < bright(at(50, 50)));
        // A line behind the near box is fainter than where it shows.
        let line = ([-35., -20., 0.], [35., -20., 0.], [0, 200, 0], false);
        let px = render(&m.tris, &[line], &view, (100, 100), 2, place);
        let at = |x: u32, y: u32| px[((y * 100 + x) * 4 + 1) as usize];
        assert!(at(25, 50) > at(50, 50));
    }
}
