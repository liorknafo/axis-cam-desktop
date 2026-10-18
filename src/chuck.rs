//! A chuck on the rotary A axis (LIO-187): the stock, a round bar or a box,
//! held in a 3- or 4-jaw chuck on a machine with an A table around X, the
//! tailstock at its free end or out of the way, and whether a tool comes
//! too close to them at an A angle or while A turns. The sizes come from the
//! machine's rotary unit (`model::ROTARY_GEOMETRY`); what it does not give
//! (a jaw's width and height, the chuck body's and the rotary head's
//! length, the tailstock's quill and centre) is assumed from it below,
//! rounded up. Every solid is checked as the box round it, grown by the
//! setup's clearance gap like a vise's jaws (`holding::hits_segments`), so a
//! check never finds less than the real solid would.
//!
//! The setup's frame keeps the machine's axes: X along the A axis, the
//! chuck at the stock's -X end, the A axis through the middle of the stock's
//! Y and Z. At A, every solid that turns is turned about that axis by A, +Y
//! towards +Z for a machine whose A direction is Positive (the other way for
//! Negative); the tailstock body and the rotary head do not turn.
use crate::holding::{Hit, HoldBox, Shape, hits_segments};
use crate::mesh::Bounds;
use crate::model::{RotaryUnit, Setup, get};
use std::sync::Arc;

pub const JAWS: [&str; 2] = ["3-jaw", "4-jaw"];
pub const TAILSTOCK: [&str; 3] = ["Retracted", "Supports the stock", "Off the table"];
/// A jaw's width across, over the chuck's diameter (20 mm on a 100 mm chuck).
const JAW_WIDTH: f32 = 0.2;
/// How far a jaw reaches out from the stock it grips, over the chuck's
/// diameter (35 mm on a 100 mm chuck).
const JAW_HEIGHT: f32 = 0.35;
/// The chuck body's length behind its face, over its diameter.
const BODY: f32 = 0.6;
/// The rotary head's length behind the chuck body, over the chuck's diameter.
const HEAD: f32 = 1.;
/// The tailstock quill's radius over the tailstock body's width.
const QUILL: f32 = 0.25;
/// What a chuck setup's fields start with: a 3-jaw chuck, 40 mm of stock
/// out of the jaws, the tailstock drawn back and jaw 1 straight up at A0.
pub const DEFAULTS: [(&str, &str); 4] = [
    ("chuck_jaws", "3-jaw"),
    ("chuck_stickout", "40"),
    ("chuck_tailstock", "Retracted"),
    ("chuck_jaw_angle", "0"),
];
/// Why a setup's chuck cannot be modelled on its machine.
pub const NO_UNIT: &str = "A chuck needs a machine with a rotary A axis around X and its rotary unit's sizes: choose one, or edit the machine to fill them in.";
/// The least share of the jaw length that must grip the stock.
pub const MIN_GRIP: f32 = 0.5;
/// Sides of the polygon that stands for a round solid: its corners lie
/// outside the circle, so it holds the whole circle.
const SIDES: usize = 24;
/// The angle between the turned solids a sweep is checked at, degrees. Each
/// is grown by what a point of it can move between two of them.
pub const SWEEP_STEP: f32 = 1.;

/// A solid as the 3D view draws it: two rings of as many points, one at
/// each end, joined side by side. `smooth` for a round one, whose sides are
/// drawn without their edges.
#[derive(Debug, Clone, PartialEq)]
pub struct Prism {
    pub ends: [Vec<[f32; 3]>; 2],
    pub smooth: bool,
}
impl Prism {
    /// The box round every point.
    pub fn bounds(&self) -> Bounds {
        let mut b = Bounds {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        };
        for p in self.ends.iter().flatten() {
            for (k, v) in p.iter().enumerate() {
                b.min[k] = b.min[k].min(*v);
                b.max[k] = b.max[k].max(*v);
            }
        }
        b
    }
}
/// Where the tailstock is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tail {
    /// Not on the table: nothing is checked.
    Off,
    /// Its centre's tip at X (the setup's frame), clear of the stock.
    Retracted(f32),
    /// Its centre in the stock's free end, at X.
    Supports(f32),
}
/// A setup's chuck round its stock, in mm in the setup's frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Rig {
    pub unit: RotaryUnit,
    pub jaws: usize,
    /// The stock the setup starts from: the box round it.
    pub stock: Bounds,
    /// Whether the stock is a round bar (else a box).
    pub round: bool,
    /// The A axis: Y and Z.
    pub axis: [f32; 2],
    /// The chuck's face and the jaws' front, X.
    pub face: f32,
    pub front: f32,
    /// How far the jaws close in from the axis (the bar's radius, or half
    /// of the box's thinner side), and the stock's farthest reach from it.
    pub inner: f32,
    pub outer: f32,
    /// Jaw 1 from straight up at A0, degrees.
    pub phase: f32,
    /// +1 for a machine whose A turns +Y towards +Z, -1 the other way.
    pub sign: f32,
    pub tail: Tail,
    /// The table's top under the rotary unit, Z.
    pub table: f32,
    /// How far the stock sticks out of the jaws, mm.
    pub stickout: f32,
}
/// Whether the setup holds its stock in the chuck.
pub fn holds(setup: &Setup) -> bool {
    get(&setup.p, "holding") == "Chuck"
}
/// A number typed into a field: surrounding spaces are ignored.
fn number(text: &str) -> Option<f32> {
    text.trim().parse::<f32>().ok().filter(|v| v.is_finite())
}
/// The chuck of `setup` round `stock`, the box it starts from (mm, the
/// setup's frame), a round bar along X when `round`, or why it cannot be
/// modelled.
pub fn rig(setup: &Setup, stock: Bounds, round: bool) -> Result<Rig, String> {
    let p = &setup.p;
    let unit = setup.machine.rotary_unit().ok_or(NO_UNIT)?;
    let jaws = match get(p, "chuck_jaws").trim() {
        "" | "3-jaw" => 3,
        "4-jaw" => 4,
        _ => return Err("Choose a 3-jaw or a 4-jaw chuck.".into()),
    };
    let stickout = number(get(p, "chuck_stickout"))
        .filter(|s| *s > 0.)
        .ok_or("The stick-out from the jaws must be a positive number.")?;
    let phase = match get(p, "chuck_jaw_angle").trim() {
        "" => 0.,
        text => number(text).ok_or("The angle of jaw 1 must be a number.")?,
    };
    let [dx, dy, dz] = stock.dimensions();
    let axis = [
        (stock.min[1] + stock.max[1]) / 2.,
        (stock.min[2] + stock.max[2]) / 2.,
    ];
    let front = stock.max[0] - stickout;
    let face = front - unit.jaw_length;
    if stickout >= dx {
        return Err(format!(
            "The stick-out ({} mm) must be shorter than the stock ({} mm long).",
            mm(stickout),
            mm(dx)
        ));
    }
    // From the chuck's face to the machine's: X, Y and Z.
    let shift = [
        unit.chuck_x - face,
        unit.centre_y - axis[0],
        unit.centre_z - axis[1],
    ];
    let retracted = unit.tail_x - shift[0];
    let tail = match get(p, "chuck_tailstock").trim() {
        "" | "Retracted" => Tail::Retracted(retracted),
        "Supports the stock" => Tail::Supports(stock.max[0]),
        "Off the table" => Tail::Off,
        _ => return Err("Choose where the tailstock is.".into()),
    };
    let sign = if get(&setup.machine.p, "direction") == "Negative" {
        -1.
    } else {
        1.
    };
    Ok(Rig {
        unit,
        jaws,
        stock,
        round,
        axis,
        face,
        front,
        inner: dy.min(dz) / 2.,
        outer: if round {
            dy.min(dz) / 2.
        } else {
            dy.hypot(dz) / 2.
        },
        phase,
        sign,
        tail,
        table: axis[1] - unit.centre_height,
        stickout,
    })
}
/// `v` in mm for a message.
fn mm(v: f32) -> String {
    match format!("{v:.1}")
        .trim_end_matches('0')
        .trim_end_matches('.')
    {
        "-0" => "0".into(),
        text => text.into(),
    }
}
impl Rig {
    /// The rotary unit `unit` alone, in machine coordinates (mm, as the tool
    /// tip's: the chuck's face at its chuck X, the centreline at its Y and
    /// Z), its 3 jaws closed halfway through their grip range and the
    /// tailstock drawn back: the Control page's machine view draws its
    /// `solids` at the controller's A. `sign` as `indexed::sign`.
    pub fn on_machine(unit: RotaryUnit, sign: f32) -> Self {
        let r = (unit.grip_min + unit.grip_max) / 4.;
        let (face, axis) = (unit.chuck_x, [unit.centre_y, unit.centre_z]);
        let front = face + unit.jaw_length;
        Self {
            unit,
            jaws: 3,
            stock: Bounds {
                min: [face, axis[0] - r, axis[1] - r],
                max: [front, axis[0] + r, axis[1] + r],
            },
            round: true,
            axis,
            face,
            front,
            inner: r,
            outer: r,
            phase: 0.,
            sign,
            tail: Tail::Retracted(unit.tail_x),
            table: axis[1] - unit.centre_height,
            stickout: 0.,
        }
    }
    /// How long the jaws grip the stock, mm.
    pub fn grip(&self) -> f32 {
        self.unit.jaw_length.min(self.front - self.stock.min[0])
    }
    /// The radius of the chuck body.
    fn radius(&self) -> f32 {
        self.unit.chuck_diameter / 2.
    }
    /// How far the jaws reach from the axis.
    fn jaw_reach(&self) -> f32 {
        self.outer + JAW_HEIGHT * self.unit.chuck_diameter
    }
    /// The tailstock quill's radius and its centre's cone length (60°).
    fn quill(&self) -> (f32, f32) {
        let r = QUILL * self.unit.tail_width;
        (r, r * 3f32.sqrt())
    }
    /// The tip of the tailstock's centre with the quill drawn back, X.
    fn retracted(&self) -> f32 {
        self.unit.tail_x - (self.unit.chuck_x - self.face)
    }
    /// What is wrong with holding the stock this way: the stock against the
    /// jaws' grip range, the chuck's kind, how long the jaws grip it, what
    /// fits on the rotary unit, and the tailstock against the stock's end.
    pub fn issues(&self) -> Vec<String> {
        let u = &self.unit;
        let mut e = vec![];
        let (across, widest) = (2. * self.inner, 2. * self.outer);
        if self.round {
            if !(u.grip_min..=u.grip_max).contains(&across) {
                e.push(format!(
                    "The bar is Ø{} mm; the jaws grip from Ø{} to Ø{} mm.",
                    mm(across),
                    mm(u.grip_min),
                    mm(u.grip_max)
                ));
            }
        } else {
            if self.jaws == 3 {
                e.push("A 3-jaw chuck centres round stock: hold a box in a 4-jaw chuck, or use a round bar.".into());
            }
            if across < u.grip_min || widest > u.grip_max {
                e.push(format!(
                    "The box is {} mm across its thinner side and {} mm across its corners; the jaws grip from Ø{} to Ø{} mm.",
                    mm(across),
                    mm(widest),
                    mm(u.grip_min),
                    mm(u.grip_max)
                ));
            }
            if self.jaws == 4 && (self.phase / 90.).fract().abs() > 1e-4 {
                e.push("The jaws of a 4-jaw chuck must close square on the box's sides: set jaw 1 at 0, 90, 180 or 270°.".into());
            }
        }
        if self.grip() < MIN_GRIP * u.jaw_length {
            e.push(format!(
                "The jaws grip only {} mm of the stock; at least {} mm (half their {} mm length) is wanted. Shorten the stick-out or use longer stock.",
                mm(self.grip().max(0.)),
                mm(MIN_GRIP * u.jaw_length),
                mm(u.jaw_length)
            ));
        }
        if widest > u.fit_diameter {
            e.push(format!(
                "The stock is {} mm across; the rotary unit takes at most Ø{} mm under the gantry.",
                mm(widest),
                mm(u.fit_diameter)
            ));
        }
        let reach = self.stock.max[0] - self.face;
        if reach > u.fit_length {
            e.push(format!(
                "The stock reaches {} mm from the chuck's face; at most {} mm fits between the chuck and the tailstock.",
                mm(reach),
                mm(u.fit_length)
            ));
        }
        let end = self.stock.max[0];
        match self.tail {
            Tail::Off => {}
            Tail::Retracted(tip) if end >= tip => e.push(format!(
                "The stock's end reaches the drawn-back tailstock centre: it ends {} mm past it. Shorten the stock or the stick-out, or take the tailstock off the table.",
                mm(end - tip)
            )),
            Tail::Supports(_) if end > self.retracted() => e.push(format!(
                "The stock's end is {} mm past the drawn-back tailstock centre: move the tailstock along the table (the machine's tailstock X).",
                mm(end - self.retracted())
            )),
            Tail::Supports(_) if end < self.retracted() - u.tail_travel => e.push(format!(
                "The tailstock centre reaches {} mm out ({} mm of travel); the stock's end is {} mm from it. Move the tailstock along the table, or use longer stock.",
                mm(u.tail_travel),
                mm(u.tail_travel),
                mm(self.retracted() - end)
            )),
            _ => {}
        }
        e
    }
    /// The point at X `x`, `r` out from the axis at `t` degrees from
    /// straight up (turned the way +A turns, +Z towards -Y), and `w` along
    /// the tangent there.
    fn at(&self, x: f32, t: f32, r: f32, w: f32) -> [f32; 3] {
        let (s, c) = t.to_radians().sin_cos();
        // Radial (-sin, cos) and tangent (cos, sin) in Y, Z.
        [
            x,
            self.axis[0] - r * s + w * c,
            self.axis[1] + r * c + w * s,
        ]
    }
    /// A round solid about the axis from `x0` to `x1`: a polygon whose sides
    /// touch the circle of `r`, so its corners lie outside it.
    fn cylinder(&self, x0: f32, x1: f32, r: f32) -> Prism {
        let r = r / (std::f32::consts::PI / SIDES as f32).cos();
        let ring = |x| {
            (0..SIDES)
                .map(|i| self.at(x, i as f32 * 360. / SIDES as f32, r, 0.))
                .collect()
        };
        Prism {
            ends: [ring(x0), ring(x1)],
            smooth: true,
        }
    }
    /// A box along X, `y` and `z` its spans.
    fn block(x: [f32; 2], y: [f32; 2], z: [f32; 2]) -> Prism {
        let ring = |x| {
            vec![
                [x, y[0], z[0]],
                [x, y[1], z[0]],
                [x, y[1], z[1]],
                [x, y[0], z[1]],
            ]
        };
        Prism {
            ends: [ring(x[0]), ring(x[1])],
            smooth: false,
        }
    }
    /// The solids at A `a` (degrees): (label, solid, how far its farthest
    /// point is from the axis when it turns with A, None when it does not).
    pub fn solids(&self, a: f32) -> Vec<(String, Prism, Option<f32>)> {
        let u = &self.unit;
        let turn = self.sign * a;
        let r = self.radius();
        let mut out = vec![];
        let body = self.face - BODY * u.chuck_diameter;
        out.push((
            "chuck body".into(),
            self.cylinder(body, self.face, r),
            Some(r),
        ));
        let (r0, r1, half) = (
            self.inner,
            self.jaw_reach(),
            JAW_WIDTH * u.chuck_diameter / 2.,
        );
        for k in 0..self.jaws {
            let t = self.phase + turn + k as f32 * 360. / self.jaws as f32;
            let ring = |x| {
                vec![
                    self.at(x, t, r0, -half),
                    self.at(x, t, r1, -half),
                    self.at(x, t, r1, half),
                    self.at(x, t, r0, half),
                ]
            };
            out.push((
                format!("chuck jaw {}", k + 1),
                Prism {
                    ends: [ring(self.face), ring(self.front)],
                    smooth: false,
                },
                Some(r1.hypot(half)),
            ));
        }
        let (y, z) = (self.axis[0], self.axis[1]);
        out.push((
            "rotary head".into(),
            Self::block(
                [body - HEAD * u.chuck_diameter, body],
                [y - r, y + r],
                [self.table, z + r],
            ),
            None,
        ));
        let tip = match self.tail {
            Tail::Off => return out,
            Tail::Retracted(tip) | Tail::Supports(tip) => tip,
        };
        let (rq, cone) = self.quill();
        // The 60° centre as three steps, each as wide as its widest end.
        for i in 0..3 {
            let (x0, x1) = (tip + cone * i as f32 / 3., tip + cone * (i + 1) as f32 / 3.);
            let prism = self.cylinder(x0, x1, rq * (i + 1) as f32 / 3.);
            out.push(("tailstock centre".into(), prism, Some(rq)));
        }
        let front = self.retracted() + cone;
        if front > tip + cone {
            out.push((
                "tailstock quill".into(),
                self.cylinder(tip + cone, front, rq),
                Some(rq),
            ));
        }
        out.push((
            "tailstock body".into(),
            Self::block(
                [front, front + u.tail_length],
                [y - u.tail_width / 2., y + u.tail_width / 2.],
                [self.table, self.table + u.tail_height],
            ),
            None,
        ));
        out
    }
    /// The stock gripped in the jaws, which no tool may come near: from the
    /// chuck's face (or the stock's end, if that is nearer) to the jaws'
    /// front, as far out as the stock reaches from the axis.
    fn gripped(&self) -> HoldBox {
        let (r, [y, z]) = (self.outer, self.axis);
        HoldBox {
            label: "stock gripped in the chuck jaws".into(),
            bounds: Bounds {
                min: [self.face.max(self.stock.min[0]), y - r, z - r],
                max: [self.front, y + r, z + r],
            },
            shape: Shape::Hidden,
        }
    }
    /// The chuck, its jaws, the rotary head and the tailstock at A `a`
    /// (degrees) as the boxes round them, without the clearance gap, and
    /// the stock gripped in the jaws (`gripped`), not drawn.
    pub fn boxes(&self, a: f32) -> Vec<HoldBox> {
        let mut out: Vec<HoldBox> = self
            .solids(a)
            .into_iter()
            .map(|(label, prism, _)| HoldBox {
                label,
                bounds: prism.bounds(),
                shape: Shape::Prism(Arc::new(prism)),
            })
            .collect();
        out.push(self.gripped());
        out
    }
    /// What A sweeps from `from` to `to` (degrees), as boxes without the
    /// clearance gap: each turning solid, and the stock, at every
    /// `SWEEP_STEP` between them and at both ends, grown in Y and Z by what
    /// a point of it can move to the nearest of them (2 r sin(step / 4) at
    /// its farthest `r` from the axis), so that together they hold all it
    /// passes through; the solids that do not turn once. Not drawn.
    pub fn swept(&self, from: f32, to: f32) -> Vec<HoldBox> {
        let span = (to - from).abs().min(360.);
        let steps = (span / SWEEP_STEP).ceil().max(1.) as usize;
        let step = span / steps as f32;
        let dir = if to >= from { 1. } else { -1. };
        let grow = |r: f32| 2. * r * (step / 4.).to_radians().sin();
        let mut out = vec![];
        for i in 0..=steps {
            let a = from + dir * step * i as f32;
            for (label, prism, reach) in self.solids(a) {
                let Some(r) = reach else {
                    if i == 0 {
                        out.push(hidden(label, prism.bounds(), 0.));
                    }
                    continue;
                };
                out.push(hidden(label, prism.bounds(), grow(r)));
            }
            let stock = self.stock_solid(a).bounds();
            out.push(hidden("stock".into(), stock, grow(self.outer)));
        }
        out
    }
    /// The stock at A `a`: a round bar, or the box turned about the axis.
    fn stock_solid(&self, a: f32) -> Prism {
        let (x0, x1) = (self.stock.min[0], self.stock.max[0]);
        if self.round {
            return self.cylinder(x0, x1, self.outer);
        }
        let [_, dy, dz] = self.stock.dimensions();
        // Each corner's angle and radius at A0.
        let corners = [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)].map(|(sy, sz)| {
            let (y, z) = (sy * dy / 2., sz * dz / 2.);
            ((-y).atan2(z).to_degrees(), y.hypot(z))
        });
        let ring = |x| {
            corners
                .iter()
                .map(|&(t, r)| self.at(x, t + self.sign * a, r, 0.))
                .collect()
        };
        Prism {
            ends: [ring(x0), ring(x1)],
            smooth: false,
        }
    }
    /// Straight tool moves (tip positions, the setup's frame) whose tool or
    /// holder comes within `gap` of the chuck, its jaws, the rotary head,
    /// the tailstock or the stock gripped in the jaws, at A `a` degrees.
    pub fn hits_at(
        &self,
        a: f32,
        segments: &[([f32; 3], [f32; 3])],
        tool: (f32, f32, f32),
        gap: f32,
    ) -> Vec<Hit> {
        hits_segments(segments, tool.0, tool.1, tool.2, &self.boxes(a), gap)
    }
    /// `hits_at` for an index move from A `from` to A `to`: the tool, at
    /// any point of `segments` while A turns, against all that the turn
    /// sweeps (`swept`), the stock included.
    pub fn hits_turning(
        &self,
        from: f32,
        to: f32,
        segments: &[([f32; 3], [f32; 3])],
        tool: (f32, f32, f32),
        gap: f32,
    ) -> Vec<Hit> {
        hits_segments(segments, tool.0, tool.1, tool.2, &self.swept(from, to), gap)
    }
    /// How long the jaws still grip the stock after earlier cuts (`removed`,
    /// None when nothing was cut before): the jaws' contact sampled at 12
    /// places along the grip and 5 across each jaw, just inside the stock;
    /// a place counts when every jaw still touches material there.
    pub fn grip_left(&self, removed: Option<&crate::rest::Removed>) -> f32 {
        let grip = self.grip();
        let Some(removed) = removed else {
            return grip;
        };
        let half = JAW_WIDTH * self.unit.chuck_diameter / 2.;
        let r = self.inner - crate::holding::INSIDE;
        let half = half.min(r * 0.9);
        let n = 12;
        let held = (0..n)
            .filter(|&i| {
                let x = self.front - grip * (i as f32 + 0.5) / n as f32;
                (0..self.jaws).all(|k| {
                    let t = self.phase + k as f32 * 360. / self.jaws as f32;
                    (0..5).any(|j| {
                        let w = half * (j as f32 / 2. - 1.);
                        !removed.at(self.at(x, t, r, w))
                    })
                })
            })
            .count();
        grip * held as f32 / n as f32
    }
    /// What earlier cuts leave the jaws to grip, refused under `MIN_GRIP`.
    pub fn grip_issue(&self, removed: Option<&crate::rest::Removed>) -> Option<String> {
        let left = self.grip_left(removed);
        let least = MIN_GRIP * self.unit.jaw_length;
        (left < least && left < self.grip()).then(|| format!(
            "Earlier cuts leave the chuck's jaws {} mm of the stock to grip; at least {} mm is wanted. Grip where the stock is whole.",
            mm(left),
            mm(least)
        ))
    }
    /// The chuck in a sentence, for the setup sheet and the program header.
    pub fn sentence(&self, gap: f32) -> String {
        format!(
            "{} chuck on A, Ø{} mm, gripping {} mm of the {}, {} mm out of the jaws, tailstock {}, gap {} mm.",
            if self.jaws == 4 { "4-jaw" } else { "3-jaw" },
            mm(self.unit.chuck_diameter),
            mm(self.grip()),
            if self.round {
                format!("Ø{} mm bar", mm(2. * self.inner))
            } else {
                "box".into()
            },
            mm(self.stickout),
            match self.tail {
                Tail::Off => "off the table",
                Tail::Retracted(_) => "drawn back",
                Tail::Supports(_) => "supporting the end",
            },
            mm(gap)
        )
    }
}
/// A box grown by `by` in Y and Z, not drawn.
fn hidden(label: String, mut b: Bounds, by: f32) -> HoldBox {
    for k in 1..3 {
        b.min[k] -= by;
        b.max[k] += by;
    }
    HoldBox {
        label,
        bounds: b,
        shape: Shape::Hidden,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Machine;
    /// A Ø30 × 120 mm bar along X, 40 mm out of the jaws, in a 3-jaw chuck
    /// on the rotary machine's assumed unit.
    fn setup(extra: &[(&str, &str)]) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let m = crate::control::rotary_machine(dir.path()).unwrap();
        let mut s = Setup::new(m, "G54");
        for (k, v) in [
            ("stock_type", "Round bar"),
            ("stock_diameter", "30"),
            ("stock_length", "120"),
            ("holding", "Chuck"),
            ("chuck_jaws", "3-jaw"),
            ("chuck_stickout", "40"),
            ("chuck_tailstock", "Off the table"),
        ]
        .iter()
        .chain(extra)
        {
            s.p.insert((*k).into(), (*v).into());
        }
        s
    }
    fn stock(s: &Setup) -> Bounds {
        crate::stock::resolve(s, None).unwrap().stock
    }
    fn rig_of(s: &Setup) -> Rig {
        rig(s, stock(s), crate::stock::round_bar(s)).unwrap()
    }
    /// A Ø6 tool with 20 mm stickout in a Ø20 holder.
    const TOOL: (f32, f32, f32) = (3., 20., 10.);
    fn plunge(x: f32, y: f32, z: f32) -> Vec<([f32; 3], [f32; 3])> {
        vec![([x, y, z + 50.], [x, y, z])]
    }
    #[test]
    fn the_bar_sits_on_the_axis_with_the_chuck_at_its_minus_x_end() {
        let r = rig_of(&setup(&[]));
        assert_eq!(r.axis, [0., 15.]);
        assert_eq!(r.front, 60. - 40.);
        assert_eq!(r.face, 20. - 25.);
        assert_eq!(r.grip(), 25.);
        assert!(r.issues().is_empty(), "{:?}", r.issues());
        // The table is the centre height below the axis.
        assert_eq!(r.table, 15. - 70.);
    }
    #[test]
    fn jaws_turn_with_a_and_the_tailstock_body_does_not() {
        let r = rig_of(&setup(&[("chuck_tailstock", "Retracted")]));
        let jaw = |a: f32| {
            r.solids(a)
                .into_iter()
                .find(|s| s.0 == "chuck jaw 1")
                .unwrap()
                .1
                .bounds()
        };
        let body = |a: f32| {
            r.solids(a)
                .into_iter()
                .find(|s| s.0 == "tailstock body")
                .unwrap()
                .1
                .bounds()
        };
        let reach = 15. + 35.;
        // Jaw 1 up at A0, towards -Y at A90 (+Y turns towards +Z), down at
        // A180, towards +Y at A270.
        assert!((jaw(0.).max[2] - (15. + reach)).abs() < 1e-3);
        assert!((jaw(90.).min[1] - -reach).abs() < 1e-3);
        assert!((jaw(180.).min[2] - (15. - reach)).abs() < 1e-3);
        assert!((jaw(270.).max[1] - reach).abs() < 1e-3);
        // At an odd angle it lies between, its box round its turned corners.
        let odd = jaw(37.);
        let (s, c) = 37f32.to_radians().sin_cos();
        assert!(odd.max[2] >= 15. + reach * c - 1e-3);
        assert!(odd.min[1] <= -reach * s + 1e-3);
        for a in [0., 90., 180., 270., 37.] {
            assert_eq!(body(a), body(0.));
        }
        let centre = |a: f32| {
            r.solids(a)
                .into_iter()
                .find(|s| s.0 == "tailstock centre")
                .unwrap()
                .1
                .bounds()
        };
        assert_eq!(centre(37.).min[0], centre(0.).min[0]);
        // A Negative machine turns them the other way.
        let mut neg = setup(&[]);
        neg.machine.p.insert("direction".into(), "Negative".into());
        let r = rig_of(&neg);
        let jaw = r
            .solids(90.)
            .into_iter()
            .find(|s| s.0 == "chuck jaw 1")
            .unwrap();
        assert!((jaw.1.bounds().max[1] - reach).abs() < 1e-3);
    }
    #[test]
    fn a_tool_at_a_jaw_is_refused_at_its_angle_on_both_sides_of_the_gap() {
        let r = rig_of(&setup(&[]));
        let gap = 2.;
        // Jaw 1's top at A0 is 15 + 50 = Z65, over X -5 to 20: a tool
        // coming down on it beyond the jaws' front by its radius plus the
        // gap is clear, a hundredth less is not.
        let clear_x = 20. + 3. + gap + 0.01;
        assert!(
            r.hits_at(0., &plunge(clear_x, 0., 66.), TOOL, gap)
                .is_empty()
        );
        let hit = r.hits_at(0., &plunge(clear_x - 0.02, 0., 66.), TOOL, gap);
        assert!(hit.iter().any(|h| h.label == "chuck jaw 1"), "{hit:?}");
        // Over the jaws: a tip 0.01 mm above the jaw's top and the gap is
        // clear at A0 only when nothing else is under it.
        let over = |a: f32, z: f32| r.hits_at(a, &plunge(10., 0., z), TOOL, gap);
        assert!(over(0., 65. + gap + 0.01).is_empty());
        assert!(!over(0., 65. + gap - 0.01).is_empty());
        // At A60 jaw 1 and 2 stand off the top: a tip low over the bar
        // between them still hits the stock gripped in the jaws.
        let low = over(60., 15. + 15. + gap - 0.01);
        assert!(low.iter().all(|h| h.label != "chuck jaw 1"));
        assert!(
            low.iter()
                .any(|h| h.label == "stock gripped in the chuck jaws")
        );
        // At every angle checked: clear well above the jaws (their corner
        // reaches 51 mm from the axis), refused on the bar in the jaws.
        for a in [0., 90., 180., 270., 37.] {
            assert!(over(a, 15. + 51. + gap + 0.01).is_empty(), "A{a}");
            let near = r.hits_at(a, &plunge(10., 0., 15. + 15.), TOOL, gap);
            assert!(!near.is_empty(), "A{a}");
        }
    }
    #[test]
    fn cuts_into_the_gripped_length_are_refused_and_beyond_it_are_not() {
        let r = rig_of(&setup(&[]));
        let gap = 2.;
        // A Ø6 tool 40 mm out of its holder, whose holder stays above the
        // jaws: a cut along the bar's top at Z27 (3 mm deep), coming no
        // nearer the jaws' front than the tool's radius and the gap.
        let long = (3., 40., 10.);
        let cut = |x0: f32| vec![([x0, 0., 27.], [60., 0., 27.])];
        let clear = r.hits_at(180., &cut(20. + 3. + gap + 0.01), long, gap);
        assert!(clear.is_empty(), "{clear:?}");
        let into = r.hits_at(180., &cut(20. + 3. + gap - 0.01), long, gap);
        assert!(
            into.iter()
                .any(|h| h.label == "stock gripped in the chuck jaws")
        );
    }
    #[test]
    fn an_index_move_checks_every_angle_between_its_ends() {
        let r = rig_of(&setup(&[]));
        let gap = 2.;
        // The tool waits at Z60 over X10: jaw 1 points up at A0 (top Z65),
        // so the tool is in it there; at A60 and A180 no jaw points up.
        let wait = plunge(10., 0., 60.)[..]
            .iter()
            .map(|s| (s.1, s.1))
            .collect::<Vec<_>>();
        assert!(
            r.hits_at(60., &wait, TOOL, gap)
                .iter()
                .all(|h| !h.label.starts_with("chuck jaw"))
        );
        assert!(
            r.hits_at(180., &wait, TOOL, gap)
                .iter()
                .all(|h| !h.label.starts_with("chuck jaw"))
        );
        // Turning from A60 to A180 brings jaw 3 straight up at A120:
        // checking only the ends would miss it.
        let swept = r.hits_turning(60., 180., &wait, TOOL, gap);
        assert!(
            swept.iter().any(|h| h.label.starts_with("chuck jaw")),
            "{swept:?}"
        );
        // Backwards too, and over a whole turn.
        assert!(!r.hits_turning(180., 60., &wait, TOOL, gap).is_empty());
        assert!(!r.hits_turning(0., 720., &wait, TOOL, gap).is_empty());
        // High above everything the turn sweeps, nothing is hit; just
        // under what the jaws sweep (their corner's reach) it is.
        let reach = 50f32.hypot(10.);
        let at = |z: f32| vec![([10., 0., z], [10., 0., z])];
        assert!(
            r.hits_turning(60., 180., &at(15. + reach + gap + 0.5), TOOL, gap)
                .is_empty()
        );
        assert!(
            !r.hits_turning(60., 180., &at(15. + reach + gap - 0.01), TOOL, gap)
                .is_empty()
        );
    }
    #[test]
    fn a_box_turning_in_a_4_jaw_chuck_sweeps_its_corners() {
        let mut s = setup(&[("stock_type", "Fixed size box"), ("chuck_jaws", "4-jaw")]);
        for (k, v) in [("stock_x", "120"), ("stock_y", "30"), ("stock_z", "30")] {
            s.p.insert(k.into(), v.into());
        }
        let r = rig_of(&s);
        assert!(r.issues().is_empty(), "{:?}", r.issues());
        // Over the free end, 5 mm over the box's top: clear while it stays
        // at A0, hit when it turns past 45°, where its corner reaches 21.2
        // mm from the axis.
        let at = vec![([50., 0., 32. + 3.], [50., 0., 32. + 3.])];
        assert!(r.hits_turning(0., 0., &at, TOOL, 0.5).is_empty());
        assert!(!r.hits_turning(0., 90., &at, TOOL, 0.5).is_empty());
        // A box in a 3-jaw chuck, or 4 jaws not square on it, is refused.
        let mut three = s.clone();
        three.p.insert("chuck_jaws".into(), "3-jaw".into());
        assert!(rig_of(&three).issues()[0].contains("3-jaw"));
        s.p.insert("chuck_jaw_angle".into(), "30".into());
        assert!(rig_of(&s).issues().iter().any(|e| e.contains("square")));
    }
    #[test]
    fn grip_range_grip_length_and_the_tailstock_are_refused_when_wrong() {
        let issues = |extra: &[(&str, &str)]| rig_of(&setup(extra)).issues();
        assert!(issues(&[("stock_diameter", "90")])[0].contains("grip from"));
        assert!(issues(&[("stock_diameter", "4")])[0].contains("grip from"));
        // 120 mm long, 110 out: 10 mm in the 25 mm jaws.
        assert!(issues(&[("chuck_stickout", "110")])[0].contains("grip only 10 mm"));
        // The assumed tailstock's centre draws back to 330 - 20 = 310 mm
        // from the chuck's face, with 30 mm of travel. The bar reaches 65
        // mm from the face: clear of it drawn back, too short to support.
        let at = |tail: &str| issues(&[("chuck_tailstock", tail)]);
        assert!(at("Retracted").is_empty());
        assert!(at("Supports the stock")[0].contains("travel"));
        let s = setup(&[("stock_length", "300"), ("chuck_stickout", "275")]);
        let r = rig_of(&s);
        assert_eq!(r.stock.max[0] - r.face, 300.);
        assert!(r.issues().iter().any(|e| e.contains("fits between")));
        // With the tailstock 250 mm from the face, a bar reaching 240 mm is
        // within the travel; one reaching 252 mm is past it drawn back.
        let ok = rig_of(&supported("240", "215"));
        assert!(ok.issues().is_empty(), "{:?}", ok.issues());
        let long = rig_of(&supported("252", "227")).issues();
        assert!(
            long.iter().any(|e| e.contains("past the drawn-back")),
            "{long:?}"
        );
        assert!(
            rig(
                &setup(&[("chuck_stickout", "130")]),
                stock(&setup(&[])),
                true
            )
            .is_err()
        );
        // Without a rotary unit there is no chuck to model.
        let mut flat = setup(&[]);
        flat.machine = Machine::new(3);
        assert!(
            rig(&flat, stock(&flat), true)
                .unwrap_err()
                .contains("rotary")
        );
    }
    #[test]
    fn the_chucks_grip_holds_the_turn_and_the_slide_and_the_free_end_bends() {
        use crate::forces::{Give, Push, SAFETY, chuck_check, chuck_issues};
        let r = rig_of(&setup(&[]));
        // 300 N sideways and 100 N up on the bar's top (Z30, 15 mm over the
        // axis): 300 × 15 + 100 × 15 = 6000 N·mm about A against the jaws'
        // 0.2 × 6000 N × 15 mm = 18000 N·mm, ×3; along A 1200 N against
        // 300 N, ×4.
        let push = Push {
            side: 300.,
            up: 100.,
            z: 30.,
        };
        let m = chuck_check(&r, None, &[push]).unwrap();
        assert_eq!(m.give, Give::Turn);
        assert!((m.factor - 3.).abs() < 1e-3 && m.factor >= SAFETY, "{m:?}");
        // The 40 mm out of the jaws bends as a cantilever of Ø30 in the
        // stiffness of "Other" (aluminium's): 316 N × 40³ / (3 × 69000 ×
        // π 30⁴/64).
        let i = std::f32::consts::PI * 30f32.powi(4) / 64.;
        let bend = 300f32.hypot(100.) * 40f32.powi(3) / (3. * 69_000. * i);
        assert!((m.bend - bend).abs() < 1e-6, "{} {bend}", m.bend);
        assert!(chuck_issues("Face", &m).is_empty());
        // Seven times the push turns it.
        let hard = Push {
            side: 2100.,
            ..push
        };
        let m = chuck_check(&r, None, &[push, hard]).unwrap();
        assert!(m.factor < SAFETY);
        assert!(chuck_issues("Face", &m)[0].contains("turn the stock in the chuck"));
        // 150 mm out of the jaws, the end bends 0.13 mm: refused, unless the
        // tailstock holds it (3/102 of that).
        let long = rig_of(&setup(&[
            ("stock_length", "200"),
            ("chuck_stickout", "150"),
        ]));
        let m = chuck_check(&long, None, &[push]).unwrap();
        assert!(m.bend > crate::forces::DEFLECTION, "{}", m.bend);
        let said = chuck_issues("Face", &m);
        assert!(
            said.iter()
                .any(|e| e.contains("bend") && e.contains("tailstock")),
            "{said:?}"
        );
        let held = Rig {
            tail: Tail::Supports(long.stock.max[0]),
            ..long.clone()
        };
        let m2 = chuck_check(&held, None, &[push]).unwrap();
        assert!((m2.bend - m.bend * 3. / 102.).abs() < 1e-6);
        assert!(chuck_issues("Face", &m2).is_empty());
        // Nothing pushes: nothing to say.
        assert!(chuck_check(&r, None, &[]).is_none());
    }
    #[test]
    fn the_setup_names_the_chuck_and_lists_its_problems() {
        let mut doc = crate::model::Document {
            setups: vec![setup(&[
                ("chuck_tailstock", "Retracted"),
                ("holding_gap", "2"),
            ])],
            ..Default::default()
        };
        let s = &doc.setups[0];
        assert_eq!(doc.holding_label(s), "3-jaw chuck on A");
        assert_eq!(
            doc.holding_text(s).unwrap(),
            "3-jaw chuck on A, Ø100 mm, gripping 25 mm of the Ø30 mm bar, 40 mm out of the jaws, tailstock drawn back, gap 2 mm."
        );
        assert!(
            doc.setup_sheet(None)
                .contains("Workholding: 3-jaw chuck on A, Ø100 mm")
        );
        assert!(doc.holding_issues(0, None).is_empty());
        // The drawn jaws and bodies are what is checked; the gripped stock
        // is checked and not drawn.
        let boxes = doc.holding_boxes(s, stock(s)).unwrap();
        assert_eq!(boxes, rig_of(s).boxes(0.));
        let shown = crate::holding::shown(&boxes, stock(s));
        assert_eq!(shown.len(), boxes.len() - 1);
        assert!(
            shown
                .iter()
                .all(|b| b.label != "stock gripped in the chuck jaws")
        );
        // Too little in the jaws, a bad gap.
        doc.setups[0]
            .p
            .insert("chuck_stickout".into(), "110".into());
        doc.setups[0].p.insert("holding_gap".into(), "-1".into());
        let issues = doc.holding_issues(0, None);
        assert_eq!(issues.len(), 2, "{issues:?}");
        assert!(issues[1].contains("grip only 10 mm"));
    }
    /// A bar `length` long, `stickout` out of the jaws, supported by the
    /// tailstock, which stands 250 mm from the chuck's face drawn back.
    fn supported(length: &str, stickout: &str) -> Setup {
        let mut s = setup(&[
            ("stock_length", length),
            ("chuck_stickout", stickout),
            ("chuck_tailstock", "Supports the stock"),
        ]);
        s.machine.p.insert("a_tail_x".into(), "270".into());
        s
    }
    #[test]
    fn the_supporting_tailstock_centre_is_in_the_bars_end_and_its_body_stays_put() {
        let r = rig_of(&supported("240", "215"));
        let centre: Vec<_> = r
            .solids(0.)
            .into_iter()
            .filter(|s| s.0 == "tailstock centre")
            .collect();
        assert_eq!(centre.len(), 3);
        let tip = centre[0].1.bounds().min[0];
        assert!((tip - r.stock.max[0]).abs() < 1e-3);
        // A tool coming down just past the bar's end beside the centre's
        // point is refused within its radius and the gap, clear beyond.
        let (rq, gap) = (QUILL * 60., 2.);
        let r_first = rq / 3. / (std::f32::consts::PI / 24.).cos();
        let face = |y: f32| vec![([r.stock.max[0] + 1., y, 60.], [r.stock.max[0] + 1., y, 15.])];
        assert!(
            !r.hits_at(0., &face(r_first + 3. + gap - 0.01), TOOL, gap)
                .is_empty()
        );
        assert!(
            r.hits_at(0., &face(r_first + 3. + gap + 0.01), TOOL, gap)
                .is_empty()
        );
    }
}
