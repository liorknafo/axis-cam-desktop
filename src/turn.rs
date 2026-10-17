//! Turning passes on the manual lathe: each Turn kind, and a Drill on the
//! centre, as a plan whose moves lie in the setup's frame (X the tool's
//! distance from the spindle axis, Y 0, Z along the axis) for the time and
//! the guide, and whose rings (`simulation::Ring`) the simulation turns away.
//!
//! Lathe work coordinates: Z 0 at the free end, which is the stock top in the
//! setup's frame, where the "Stock top center" work zero puts it (`post::datum`);
//! negative toward the chuck; X = Y = 0 on the spindle axis; sizes are
//! diameters. `frame_z` and `work_z` are the one conversion.
//!
//! Every Turn starts from the material the setup has at that point: the
//! stock (a round bar, or the box an earlier setup left), less the lathe cuts
//! made about the same axis before it, in this setup and in earlier ones
//! (`rest::Coaxial`). Mill cuts are not counted, so a pass may start in air
//! after a mill setup, but never deeper than the material.
use crate::{
    analysis::{self, Index},
    cam::Geometry,
    lathe,
    mesh::{Bounds, Mesh},
    model::{Operation, Setup, Strategy, Tool, get, num},
    rest::Coaxial,
    simulation::{CutterShape, Motion, Move, POINT_RISE, Plan, Ring},
    stock,
};

/// The lathe's work zero: Z 0 at the free end, on the spindle axis.
pub const ZERO: &str = "Stock top center";
/// What feeding by hand is taken as, mm per revolution: Face, Groove, Part
/// off, Chamfer, and a drill in the tailstock.
pub const HAND_FEED: f32 = 0.05;
/// The power feed an Outside or Bore pass asks for, mm per revolution: the
/// machine's nearest is used.
pub const POWER_FEED: f32 = 0.1;
/// How fast the slides are wound back by hand between passes, mm/min.
pub const HAND_RAPID: f32 = 1000.;
/// Where a pass starts, mm in front of the material: "from Z +1".
pub const CLEAR: f32 = 1.;
/// How far the tool backs off the cut surface before it is wound back, mm.
const BACK_OFF: f32 = 0.5;
/// The step of the thin rings a chamfer and a drill point are made of, mm.
const STEP: f32 = 0.1;
/// Lengths and radii closer than this are the same, mm.
const ON: f32 = 1e-3;
/// The most moves or rings a plan may have.
const LIMIT: usize = 30_000;

/// The frame Z of the free end, lathe work Z 0: the top of `stock`.
pub fn end(stock: Bounds) -> f32 {
    stock.max[2]
}
/// The setup-frame Z of lathe work Z `z` on `stock`.
pub fn frame_z(stock: Bounds, z: f32) -> f32 {
    end(stock) + z
}
/// The lathe work Z of setup-frame Z `z` on `stock`.
pub fn work_z(stock: Bounds, z: f32) -> f32 {
    z - end(stock)
}
/// `v` to the nearest thousandth of a mm.
fn round(v: f32) -> f32 {
    (v * 1000.).round() / 1000.
}

/// The material about the setup's Z axis when an operation starts.
#[derive(Debug, Clone)]
pub struct Material {
    /// The stock's Z range in the setup frame.
    low: f32,
    high: f32,
    /// How far from the axis the stock reaches before any lathe cut.
    outer: f32,
    /// The stock box's farthest corner from the axis: every ring cut from
    /// the outside reaches out to it.
    rim: f32,
    /// The radius of the largest disc about the axis inside the stock box.
    circle: f32,
    /// The earlier lathe cuts about this axis, `[z0, z1, r0, r1]`.
    rings: Vec<[f32; 4]>,
}
/// Material between the radii `inner` and `outer` (mm from the axis) all
/// along `z` (setup frame).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Section {
    pub z: [f32; 2],
    pub inner: f32,
    pub outer: f32,
}
impl Material {
    /// What `setup` has to turn (`stock`, its resolved box): the round bar
    /// it starts as, or the box an earlier setup left, less the lathe cuts
    /// its `preview_remaining` holds about its Z axis.
    pub fn new(setup: &Setup, stock: Bounds) -> Self {
        let round = stock::is_round(setup) && setup.preview_stock.is_none();
        let coaxial = setup.preview_remaining.as_ref().map(|rest| rest.coaxial());
        Self::of(stock, round, coaxial)
    }
    /// The material of `stock` (a box, in the setup frame): a round bar when
    /// `round`, less the lathe cuts `coaxial` holds.
    pub fn of(stock: Bounds, round: bool, coaxial: Option<&Coaxial>) -> Self {
        let rim = (0..4)
            .map(|mask| {
                let x = if mask & 1 == 0 {
                    stock.min[0]
                } else {
                    stock.max[0]
                };
                let y = if mask & 2 == 0 {
                    stock.min[1]
                } else {
                    stock.max[1]
                };
                x.hypot(y)
            })
            .fold(0., f32::max);
        let mut outer = rim;
        // The largest disc about the axis inside the box: the material is
        // round (a full disc) only up to there. A bar whose axis is off the
        // spindle's reaches past it, and so does a bar a mill has flatted.
        let inside = |lo: f32, hi: f32| {
            if lo <= 0. && hi >= 0. {
                (-lo).min(hi)
            } else {
                0.
            }
        };
        let circle = inside(stock.min[0], stock.max[0]).min(inside(stock.min[1], stock.max[1]));
        if round {
            let (centre, radius) = stock::bar(stock);
            outer = outer.min(centre[0].hypot(centre[1]) + radius);
        }
        let mut rings = vec![];
        if let Some(coaxial) = coaxial {
            if let Some(bar) = coaxial.bar {
                outer = outer.min(bar);
            }
            rings = coaxial.rings.clone();
        }
        Self {
            low: stock.min[2],
            high: stock.max[2],
            outer,
            rim,
            circle,
            rings,
        }
    }
    /// The widest the material is between `a` and `b` (setup frame Z), as a
    /// diameter, and whether the section there is round (a full disc to that
    /// diameter, about the axis): the bar a chuck grips. None where no
    /// material is left.
    pub fn girth(&self, a: f32, b: f32) -> Option<(f32, bool)> {
        let outer = self
            .sections(a, b)
            .iter()
            .map(|s| s.outer)
            .reduce(f32::max)?;
        Some((2. * outer, outer <= self.circle + ON))
    }
    /// The material at `z`: (inner, outer) radius, None where none is left.
    /// From the outside in, every ring that reaches the surface without a
    /// gap takes it down to its inner radius; from the axis out, every ring
    /// that starts at the hole takes it out to its outer one.
    fn at(&self, z: f32) -> Option<(f32, f32)> {
        if z < self.low || z > self.high {
            return None;
        }
        let here: Vec<&[f32; 4]> = self
            .rings
            .iter()
            .filter(|g| g[0] <= z && z <= g[1])
            .collect();
        let mut outer = self.outer;
        loop {
            let next = here
                .iter()
                .filter(|g| g[2] < outer && g[3] >= outer - ON)
                .map(|g| g[2])
                .fold(outer, f32::min);
            if next >= outer {
                break;
            }
            outer = next;
        }
        let mut inner = 0f32;
        loop {
            let next = here
                .iter()
                .filter(|g| g[3] > inner && g[2] <= inner + ON)
                .map(|g| g[3])
                .fold(inner, f32::max);
            if next <= inner {
                break;
            }
            inner = next;
        }
        (outer - inner > ON).then_some((inner, outer))
    }
    /// The material between `a` and `b` (setup frame Z), in sections of one
    /// inner and outer radius, lowest first; none where nothing is left.
    pub fn sections(&self, a: f32, b: f32) -> Vec<Section> {
        let (a, b) = (a.max(self.low), b.min(self.high));
        if b - a <= 1e-5 {
            return vec![];
        }
        let mut levels: Vec<f32> = self
            .rings
            .iter()
            .flat_map(|g| [g[0], g[1]])
            .filter(|z| *z > a && *z < b)
            .chain([a, b])
            .collect();
        levels.sort_by(f32::total_cmp);
        levels.dedup();
        levels
            .windows(2)
            .filter(|w| w[1] - w[0] > 1e-5)
            .filter_map(|w| {
                let (inner, outer) = self.at((w[0] + w[1]) / 2.)?;
                Some(Section {
                    z: [w[0], w[1]],
                    inner,
                    outer,
                })
            })
            .collect()
    }
}

/// The diameters of the passes from `from` to `to` (either way): equal
/// steps of at most `depth` to `finish` short of `to`, then `to` (the finish
/// pass). Each to the thousandth.
pub fn steps(from: f32, to: f32, depth: f32, finish: f32) -> Result<Vec<f32>, String> {
    let sign = if to < from { -1. } else { 1. };
    let rough = to - sign * finish;
    let mut out = vec![];
    if (rough - from) * sign > ON {
        let n = ((rough - from).abs() / depth - 1e-4).ceil().max(1.);
        if n > 500. {
            return Err("Too many passes: take more off the diameter per pass.".into());
        }
        let n = n as usize;
        for i in 1..=n {
            out.push(round(if i == n {
                rough
            } else {
                from + (rough - from) * i as f32 / n as f32
            }));
        }
    }
    if (to - out.last().copied().unwrap_or(from)) * sign > ON {
        out.push(round(to));
    }
    Ok(out)
}
/// The usual number of passes for a thread of `pitch` mm: 4 × pitch,
/// rounded, plus 2 (6 for 1 mm, 8 for 1.5 mm, 10 for 2 mm), from 3 to 30.
pub fn thread_passes(pitch: f32) -> usize {
    ((4. * pitch).round() as usize + 2).clamp(3, 30)
}
/// How deep a thread of `pitch` is from its crest, mm, radially: 0.61 ×
/// pitch outside, 0.54 × pitch inside.
pub fn thread_depth(outside: bool, pitch: f32) -> f32 {
    pitch * if outside { 0.61 } else { 0.54 }
}
/// The infeed reached after each of `passes` passes to the thread `depth`
/// (radial, cumulative): `depth × √(i / passes)`, so that each pass takes
/// about the same chip, the first the deepest, and the last ends exactly on
/// `depth`.
pub fn thread_infeeds(depth: f32, passes: usize) -> Vec<f32> {
    (1..=passes)
        .map(|i| {
            if i == passes {
                depth
            } else {
                depth * (i as f32 / passes as f32).sqrt()
            }
        })
        .collect()
}
/// The pitch diameter of a 60° thread of major diameter `d`, the thread
/// the simulation leaves (the thread form is not simulated).
pub fn pitch_diameter(d: f32, pitch: f32) -> f32 {
    d - 0.6495 * pitch
}

/// A plan being built: the moves in the setup frame, the rings and what the
/// part check asks of each.
struct Cut {
    moves: Vec<Move>,
    pos: [f32; 3],
    duration: f32,
    rings: Vec<Ring>,
    /// Rings the part must keep clear of, with the radial gap it needs.
    checks: Vec<([f32; 4], f32)>,
    passes: usize,
    pause: f32,
}
impl Cut {
    fn new(x: f32, z: f32, pause: f32) -> Self {
        Self {
            moves: vec![],
            pos: [x, 0., z],
            duration: 0.,
            rings: vec![],
            checks: vec![],
            passes: 0,
            pause,
        }
    }
    /// A move to the tool position (`x` from the axis, `z`): a feed at
    /// `rate` mm/min, else wound back by hand.
    fn go(&mut self, x: f32, z: f32, motion: Motion, rate: f32) -> Result<(), String> {
        if self.moves.len() >= LIMIT {
            return Err("The plan exceeds 30,000 moves.".into());
        }
        let to = [x, 0., z];
        let distance = (x - self.pos[0]).hypot(z - self.pos[2]);
        let rate = if motion == Motion::Rapid {
            HAND_RAPID
        } else {
            rate
        };
        if !distance.is_finite() || !rate.is_finite() || rate <= 0. {
            return Err("Nonfinite motion or invalid feed.".into());
        }
        if distance > 1e-5 {
            let end = self.duration + distance / rate * 60.;
            self.moves.push(Move {
                from: self.pos,
                to,
                motion,
                cutting_pass: motion == Motion::Feed,
                feed: rate,
                start: self.duration,
                end,
            });
            self.duration = end;
        }
        self.pos = to;
        Ok(())
    }
    fn rapid(&mut self, x: f32, z: f32) -> Result<(), String> {
        self.go(x, z, Motion::Rapid, HAND_RAPID)
    }
    /// One pass begins: `pass_time` to set the dials, where the tool is.
    fn pass(&mut self) {
        self.passes += 1;
        if self.pause > 0. {
            let end = self.duration + self.pause;
            self.moves.push(Move {
                from: self.pos,
                to: self.pos,
                motion: Motion::Dwell,
                cutting_pass: false,
                feed: 0.,
                start: self.duration,
                end,
            });
            self.duration = end;
        }
    }
    /// The ring `[z0, z1, r0, r1]` turned away when the last move ends, not
    /// checked against the part.
    fn ring_only(&mut self, g: [f32; 4]) -> Result<(), String> {
        if self.rings.len() >= LIMIT {
            return Err("The plan exceeds 30,000 rings.".into());
        }
        self.rings.push(Ring {
            at: self.moves.len().saturating_sub(1),
            z: [g[0], g[1]],
            r: [g[2], g[3]],
        });
        Ok(())
    }
    /// `ring_only`, and the part must keep `gap` mm clear of it.
    fn ring(&mut self, g: [f32; 4], gap: f32) -> Result<(), String> {
        self.checks.push((g, gap));
        self.ring_only(g)
    }
}

/// A number field of the operation.
fn field(op: &Operation, key: &str) -> Result<f32, String> {
    num(&op.p, key).ok_or_else(|| format!("Enter a finite {} value.", key.replace('_', " ")))
}
fn above_zero(op: &Operation, key: &str) -> Result<f32, String> {
    field(op, key)?;
    num(&op.p, key)
        .filter(|v| *v > 0.)
        .ok_or_else(|| format!("{} must be above 0.", key.replace('_', " ")))
}
fn at_least_zero(op: &Operation, key: &str) -> Result<f32, String> {
    field(op, key)?;
    num(&op.p, key)
        .filter(|v| *v >= 0.)
        .ok_or_else(|| format!("{} must be 0 or more.", key.replace('_', " ")))
}
/// A Z of the operation, which must be below the free end.
fn below_end(op: &Operation, key: &str) -> Result<f32, String> {
    let z = field(op, key)?;
    if z >= 0. {
        return Err(format!(
            "{} must be below 0: Z is 0 at the free end and negative toward the chuck.",
            key.replace('_', " ")
        ));
    }
    Ok(z)
}

/// The plan of a Turn, or of a Drill on a lathe's centre line, on `setup`.
pub fn generate(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    if !op.enabled {
        return Err("Enable the operation before generating it.".into());
    }
    if !lathe::is_lathe(&setup.machine) {
        return Err(lathe::TURN_ON_MILL.into());
    }
    if get(&setup.p, "origin") != ZERO {
        return Err(format!(
            "A lathe's work zero is {ZERO}: Z 0 at the free end, on the axis."
        ));
    }
    let resolved = stock::resolve(setup, Some(mesh))?;
    let stock = resolved.stock;
    let rpm = num(&op.p, "rpm")
        .filter(|v| *v > 0.)
        .ok_or("Enter a positive spindle speed.")?;
    let pause = num(&setup.machine.p, "pass_time")
        .filter(|t| *t >= 0.)
        .unwrap_or(0.);
    let kind = if op.strategy == Strategy::Drill {
        "Drill"
    } else {
        get(&op.p, "turn_kind")
    };
    if op.strategy == Strategy::Turn {
        let Some(needed) = lathe::tool_for(kind) else {
            return Err(format!("Unknown turn kind \"{kind}\"."));
        };
        if get(&tool.p, "type") != needed {
            return Err(format!(
                "Choose a {} for the {kind} turn.",
                needed.to_lowercase()
            ));
        }
    } else if get(&tool.p, "type") != "Drill" {
        return Err("A lathe drills on the centre with a drill in the tailstock.".into());
    }
    let material = Material::new(setup, stock);
    let lathe = Lathe {
        op,
        tool,
        stock,
        end: end(stock),
        rpm,
        material,
    };
    let start = [lathe.material.rim + CLEAR, lathe.end + CLEAR];
    let mut cut = Cut::new(start[0], start[1], pause);
    let radius = match kind {
        "Outside" | "Bore" => {
            lathe.along(&mut cut, kind == "Outside", power_feed(&setup.machine)?)?;
            num(&tool.p, "nose_radius").unwrap_or(0.)
        }
        "Face" => {
            lathe.face(&mut cut)?;
            num(&tool.p, "nose_radius").unwrap_or(0.)
        }
        "Chamfer" => {
            lathe.chamfer(&mut cut)?;
            num(&tool.p, "nose_radius").unwrap_or(0.)
        }
        "Groove" => lathe.groove(&mut cut)? / 2.,
        "Part off" => lathe.part_off(&mut cut)? / 2.,
        "Thread" => lathe.thread(&mut cut)? / 8.,
        _ => lathe.drill(&mut cut)?,
    };
    part_check(setup, mesh, stock, &cut.checks)?;
    if !cut.moves.iter().any(|m| m.motion == Motion::Feed) || !cut.duration.is_finite() {
        return Err("No cutting motion fits the selected geometry and settings.".into());
    }
    let reach = ["reach", "length"]
        .iter()
        .find_map(|k| num(&tool.p, k).filter(|v| *v > 0.))
        .unwrap_or(10.);
    let target = cut
        .moves
        .iter()
        .filter(|m| m.motion == Motion::Feed)
        .map(|m| m.to[2])
        .fold(f32::INFINITY, f32::min);
    Ok(Plan {
        shape: CutterShape::Revolve,
        strategy: op.strategy,
        operation: op.id.clone(),
        setup: setup.id.clone(),
        moves: cut.moves,
        stock,
        radius: radius.max(0.05),
        flute_length: reach,
        tool_length: num(&tool.p, "stickout")
            .filter(|v| *v >= reach)
            .unwrap_or(reach),
        holder_radius: radius.max(0.05),
        display_scale: resolved.display_scale,
        duration: cut.duration,
        target,
        levels: cut.passes,
        rest_top: None,
        spindle_clockwise: true,
        milling_direction: "Turning",
        rings: cut.rings,
    })
}
/// The machine's power feed nearest `POWER_FEED`, the finer of two as near
/// (to a micron per revolution).
fn power_feed(m: &crate::model::Machine) -> Result<f32, String> {
    let off = |f: f32| ((f - POWER_FEED).abs() * 1e3).round() as i64;
    lathe::feeds(m)
        .into_iter()
        .min_by(|a, b| off(*a).cmp(&off(*b)).then(a.total_cmp(b)))
        .ok_or_else(|| "Set the carriage's power feeds on the machine.".into())
}

/// One lathe operation being planned.
struct Lathe<'a> {
    op: &'a Operation,
    tool: &'a Tool,
    stock: Bounds,
    /// The frame Z of work Z 0, the free end.
    end: f32,
    rpm: f32,
    material: Material,
}
impl Lathe<'_> {
    fn hand(&self) -> f32 {
        HAND_FEED * self.rpm
    }
    fn rim(&self) -> f32 {
        self.material.rim
    }
    fn work(&self, z: f32) -> f32 {
        work_z(self.stock, z)
    }
    /// The frame Z of work Z `z`, refused beyond the stock's far end.
    fn on_stock(&self, z: f32) -> Result<f32, String> {
        let f = frame_z(self.stock, z);
        if f < self.material.low - ON {
            return Err(format!(
                "Z {z:.2} is beyond the stock, which reaches Z {:.2}.",
                self.work(self.material.low)
            ));
        }
        Ok(f)
    }
    /// Outside or Bore: passes along the axis from the material at hand to
    /// the diameter plus (Outside) or less (Bore) the stock to leave, each
    /// `depth` off the diameter and the last `finish`, at power feed `feed`
    /// mm/rev; each feeds toward the chuck from 1 mm before the material to
    /// `z_end`.
    fn along(&self, cut: &mut Cut, outside: bool, feed: f32) -> Result<(), String> {
        let op = self.op;
        let d = above_zero(op, "diameter")?;
        let z_end = below_end(op, "z_end")?;
        let depth = above_zero(op, "depth")?;
        let finish = at_least_zero(op, "finish")?;
        let allowance = at_least_zero(op, "allowance")?;
        let z0 = self.on_stock(z_end)?;
        let span = self.material.sections(z0, self.end);
        if span.is_empty() {
            return Err(format!("No material is left from Z 0 to Z {z_end:.2}."));
        }
        let biggest = span.iter().map(|s| s.outer).fold(0., f32::max);
        let smallest = span.iter().map(|s| s.outer).fold(f32::INFINITY, f32::min);
        let widest_hole = span.iter().map(|s| s.inner).fold(0., f32::max);
        let narrowest_hole = span.iter().map(|s| s.inner).fold(f32::INFINITY, f32::min);
        let (from, target) = if outside {
            let (from, target) = (round(2. * biggest), d + allowance);
            if from <= target + ON {
                return Err(format!(
                    "The bar is already Ø{from:.2} or less from Z 0 to Z {z_end:.2}: nothing to turn to Ø{target:.2}."
                ));
            }
            if 2. * widest_hole >= target - ON {
                return Err(format!(
                    "Ø{target:.2} reaches the bore, Ø{:.2}, there.",
                    2. * widest_hole
                ));
            }
            (from, target)
        } else {
            if narrowest_hole <= ON {
                return Err(format!(
                    "A bore needs a hole from Z 0 to Z {z_end:.2}: drill it first."
                ));
            }
            let (from, target) = (round(2. * narrowest_hole), d - allowance);
            if target <= ON {
                return Err("The diameter less the stock to leave must be above 0.".into());
            }
            if from >= target - ON {
                return Err(format!(
                    "The hole is already Ø{from:.2} or more from Z 0 to Z {z_end:.2}: nothing to bore to Ø{target:.2}."
                ));
            }
            if 2. * smallest <= target + ON {
                return Err(format!(
                    "Ø{target:.2} breaks through the outside, Ø{:.2}, there.",
                    2. * smallest
                ));
            }
            if let Some(min) = num(&self.tool.p, "min_bore").filter(|m| from < m - ON) {
                return Err(format!(
                    "The hole is Ø{from:.2}: {} enters Ø{min} or more.",
                    self.tool.name()
                ));
            }
            (from, target)
        };
        let rate = feed * self.rpm;
        let gap = allowance / 2.;
        let (rim, end) = (self.rim(), self.end);
        cut.rapid(
            if outside {
                from / 2. + CLEAR
            } else {
                (from / 2. - CLEAR).max(0.)
            },
            end + CLEAR,
        )?;
        for d in steps(from, target, depth, finish)? {
            let r = d / 2.;
            // The pass meets the material where it first reaches past `r`.
            let top = span
                .iter()
                .filter(|s| {
                    if outside {
                        s.outer > r + ON
                    } else {
                        s.inner < r - ON
                    }
                })
                .map(|s| s.z[1])
                .fold(z0, f32::max);
            cut.rapid(r, end + CLEAR)?;
            cut.rapid(r, (top + CLEAR).min(end + CLEAR))?;
            cut.pass();
            cut.go(r, z0, Motion::Feed, rate)?;
            if outside {
                cut.ring([z0, end, r, rim], gap)?;
            } else {
                cut.ring([z0, end, 0., r], gap)?;
            }
            let back = if outside {
                r + BACK_OFF
            } else {
                (r - BACK_OFF).max(0.)
            };
            cut.rapid(back, z0)?;
            cut.rapid(back, end + CLEAR)?;
        }
        Ok(())
    }
    /// Face: one cut across the end, `face_off` off the end as it is, from
    /// outside the bar in to `to_diameter`, by hand.
    fn face(&self, cut: &mut Cut) -> Result<(), String> {
        let face_off = above_zero(self.op, "face_off")?;
        let to = at_least_zero(self.op, "to_diameter")? / 2.;
        let all = self.material.sections(self.material.low, self.end);
        let Some(at) = all
            .iter()
            .filter(|s| s.outer > to + ON)
            .map(|s| s.z[1])
            .reduce(f32::max)
        else {
            return Err(format!(
                "No material is left outside Ø{:.2} to face.",
                2. * to
            ));
        };
        let z = at - face_off;
        if z < self.material.low + ON {
            return Err(format!("Facing {face_off} mm off takes the whole stock."));
        }
        let outer = self
            .material
            .sections(z, self.end)
            .iter()
            .map(|s| s.outer)
            .fold(to, f32::max);
        cut.rapid(outer + CLEAR, self.end + CLEAR)?;
        cut.rapid(outer + CLEAR, z)?;
        cut.pass();
        cut.go(to, z, Motion::Feed, self.hand())?;
        cut.ring([z, self.end, to, self.rim()], 0.)?;
        cut.rapid(to, z + CLEAR)?;
        cut.rapid(outer + CLEAR, z + CLEAR)
    }
    /// Chamfer: a 45° chamfer of leg `size` on the end's outside or inside
    /// edge, or on the first shoulder from the end, by hand; simulated as
    /// thin rings inside the chamfer, never past it.
    fn chamfer(&self, cut: &mut Cut) -> Result<(), String> {
        let size = above_zero(self.op, "size")?;
        let edge = get(&self.op.p, "edge");
        let all = self.material.sections(self.material.low, self.end);
        let Some(top) = all.last().copied() else {
            return Err("No material is left to chamfer.".into());
        };
        let n = (size / STEP - 1e-4).ceil().max(1.) as usize;
        let s = size / n as f32;
        // The sections the chamfer's band crosses, from `z` down `size`.
        let band = |z: f32| self.material.sections(z - size, z);
        let long = |sections: &[Section]| {
            sections.iter().map(|s| s.z[1] - s.z[0]).sum::<f32>() >= size - ON
        };
        let (rim, end) = (self.rim(), self.end);
        match edge {
            "End outside" | "Shoulder" => {
                let (z, big) = if edge == "End outside" {
                    (top.z[1], top.outer)
                } else {
                    // The first step up in the outside from the end at
                    // least as high as the chamfer: smaller steps, such as
                    // an earlier chamfer's, are passed over.
                    let step = all.windows(2).rev().find(|w| {
                        (w[0].z[1] - w[1].z[0]).abs() < 1e-4 && w[0].outer - w[1].outer >= size - ON
                    });
                    let Some(w) = step else {
                        return Err(format!(
                            "No shoulder {size} mm high or more to chamfer: turn a smaller diameter first."
                        ));
                    };
                    (w[0].z[1], w[0].outer)
                };
                let sections = band(z);
                if !long(&sections) || sections.iter().any(|s| (s.outer - big).abs() > ON) {
                    return Err("The edge is shorter than the chamfer.".into());
                }
                if sections.iter().any(|s| s.inner >= big - size - ON) {
                    return Err("The chamfer is bigger than the wall.".into());
                }
                cut.rapid(big + CLEAR, end + CLEAR)?;
                cut.rapid(big - size, end + CLEAR)?;
                cut.rapid(big - size, z)?;
                cut.pass();
                cut.go(big, z - size, Motion::Feed, self.hand())?;
                for k in 1..n {
                    let low = z - size + k as f32 * s;
                    cut.ring([low, low + s, big - k as f32 * s, rim], 0.)?;
                }
                cut.rapid(big + CLEAR, z - size)?;
                cut.rapid(big + CLEAR, end + CLEAR)
            }
            "End inside" => {
                let (z, hole) = (top.z[1], top.inner);
                if hole <= ON {
                    return Err("End inside needs a hole at the end: drill it first.".into());
                }
                let sections = band(z);
                if !long(&sections) || sections.iter().any(|s| (s.inner - hole).abs() > ON) {
                    return Err("The hole is shorter than the chamfer.".into());
                }
                if sections.iter().any(|s| s.outer <= hole + size + ON) {
                    return Err("The chamfer is bigger than the wall.".into());
                }
                let back = (hole - BACK_OFF).max(0.);
                cut.rapid(back, end + CLEAR)?;
                cut.rapid(hole + size, end + CLEAR)?;
                cut.rapid(hole + size, z)?;
                cut.pass();
                cut.go(hole, z - size, Motion::Feed, self.hand())?;
                for k in 1..n {
                    let low = z - size + k as f32 * s;
                    cut.ring([low, low + s, 0., hole + k as f32 * s], 0.)?;
                }
                cut.rapid(back, z - size)?;
                cut.rapid(back, end + CLEAR)
            }
            _ => Err("edge must be End outside, End inside or Shoulder.".into()),
        }
    }
    /// The tool's width, mm.
    fn width(&self) -> Result<f32, String> {
        num(&self.tool.p, "width")
            .filter(|w| *w > 0.)
            .ok_or_else(|| format!("Enter {}'s width.", self.tool.name()))
    }
    /// Groove: plunges by hand to the bottom diameter, the tool's left side
    /// (toward the chuck) from `z` to `z + width - tool width`, as many as
    /// the groove's width needs. Returns the tool's width.
    fn groove(&self, cut: &mut Cut) -> Result<f32, String> {
        let z = field(self.op, "z")?;
        let width = above_zero(self.op, "width")?;
        let bottom = above_zero(self.op, "diameter")? / 2.;
        let w = self.width()?;
        if w > width + ON {
            return Err(format!(
                "The groove is narrower than {}'s {w} mm.",
                self.tool.name()
            ));
        }
        if z + width > ON {
            return Err(
                "z must leave the whole groove on the bar: z plus width is 0 or below.".into(),
            );
        }
        let left = self.on_stock(z)?;
        let span = self.material.sections(left, left + width);
        let outer = span.iter().map(|s| s.outer).fold(0., f32::max);
        if outer <= bottom + ON {
            return Err(format!(
                "Nothing to groove: the bar is Ø{:.2} or less there.",
                2. * outer
            ));
        }
        if span.iter().any(|s| s.inner >= bottom - ON) {
            return Err(format!("Ø{:.2} reaches the bore there.", 2. * bottom));
        }
        if let Some(reach) = num(&self.tool.p, "reach").filter(|r| outer - bottom > r + ON) {
            return Err(format!(
                "The groove is {:.2} mm deep: {} reaches {reach} mm.",
                outer - bottom,
                self.tool.name()
            ));
        }
        let n = ((width - w) / w - 1e-4).ceil().max(0.) as usize + 1;
        cut.rapid(outer + CLEAR, self.end + CLEAR)?;
        for i in 0..n {
            let at = if n == 1 {
                left
            } else {
                left + (width - w) * i as f32 / (n - 1) as f32
            };
            cut.rapid(outer + CLEAR, at)?;
            cut.pass();
            cut.go(bottom, at, Motion::Feed, self.hand())?;
            cut.ring([at, at + w, bottom, self.rim()], 0.)?;
            cut.rapid(outer + CLEAR, at)?;
        }
        cut.rapid(outer + CLEAR, self.end + CLEAR)?;
        Ok(w)
    }
    /// Part off: one plunge by hand to 0.5 mm past the centre, the blade's
    /// side toward the part at `z`. The part falls: the bar left in the
    /// chuck is not the part, so it goes from the stock too. Returns the
    /// blade's width.
    fn part_off(&self, cut: &mut Cut) -> Result<f32, String> {
        let z = below_end(self.op, "z")?;
        let w = self.width()?;
        let right = self.on_stock(z)?;
        let left = right - w;
        let span = self.material.sections(left, right);
        if span.is_empty() {
            return Err(format!("Nothing to part off at Z {z:.2}."));
        }
        let outer = span.iter().map(|s| s.outer).fold(0., f32::max);
        if let Some(reach) = num(&self.tool.p, "reach").filter(|r| outer > r + ON) {
            return Err(format!(
                "The bar is Ø{:.2} there: {} reaches {reach} mm.",
                2. * outer,
                self.tool.name()
            ));
        }
        let rim = self.rim();
        cut.rapid(outer + CLEAR, self.end + CLEAR)?;
        cut.rapid(outer + CLEAR, left)?;
        cut.pass();
        cut.go(-BACK_OFF, left, Motion::Feed, self.hand())?;
        cut.ring([left, right, 0., rim], 0.)?;
        if left > self.material.low + 1e-4 {
            cut.ring([self.material.low, left, 0., rim], 0.)?;
        }
        cut.rapid(outer + CLEAR, left)?;
        cut.rapid(outer + CLEAR, self.end + CLEAR)?;
        Ok(w)
    }
    /// Thread: passes at the pitch, each fed in on the infeed table
    /// (`thread_infeeds`) to the thread depth; simulated as a ring to the
    /// pitch diameter. The part check asks only that the part keep inside
    /// the major diameter (outside) or outside the minor (inside): the
    /// model's thread is a plain cylinder. Returns the pitch.
    fn thread(&self, cut: &mut Cut) -> Result<f32, String> {
        let op = self.op;
        let outside = match get(&op.p, "thread_side") {
            "Outside" => true,
            "Inside" => false,
            _ => return Err("thread side must be Outside or Inside.".into()),
        };
        let d = above_zero(op, "diameter")?;
        let pitch = above_zero(op, "pitch")?;
        let z_end = below_end(op, "z_end")?;
        let passes = if get(&op.p, "passes").trim().is_empty() {
            thread_passes(pitch)
        } else {
            let n = field(op, "passes")?;
            if !(1. ..=30.).contains(&n) || n.fract() != 0. {
                return Err("passes must be a whole number from 1 to 30.".into());
            }
            n as usize
        };
        let depth = thread_depth(outside, pitch);
        let minor = d - 2. * depth;
        let pitch_r = pitch_diameter(d, pitch) / 2.;
        let z0 = self.on_stock(z_end)?;
        let span = self.material.sections(z0, self.end);
        if span.is_empty() {
            return Err(format!("No material is left from Z 0 to Z {z_end:.2}."));
        }
        let (rim, end) = (self.rim(), self.end);
        if outside {
            let blank = round(2. * span.iter().map(|s| s.outer).fold(0., f32::max));
            if blank > d + 0.1 {
                return Err(format!(
                    "Turn the thread's blank to Ø{d:.2} first: it is Ø{blank:.2} there."
                ));
            }
            if span.iter().any(|s| 2. * s.inner >= minor - ON) {
                return Err("The thread reaches the bore there.".into());
            }
        } else {
            if span.iter().any(|s| s.inner <= ON) {
                return Err(format!(
                    "An inside thread needs a hole from Z 0 to Z {z_end:.2}: drill and bore it first."
                ));
            }
            let hole = round(2. * span.iter().map(|s| s.inner).fold(f32::INFINITY, f32::min));
            if hole < minor - 0.1 {
                return Err(format!(
                    "Bore the hole to Ø{minor:.2} first: it is Ø{hole:.2} there."
                ));
            }
            if span.iter().any(|s| 2. * s.outer <= d + ON) {
                return Err("The thread breaks through the outside there.".into());
            }
        }
        cut.checks.push(if outside {
            ([z0, end, d / 2., rim], 0.)
        } else {
            ([z0, end, 0., minor / 2.], 0.)
        });
        let rate = pitch * self.rpm;
        let back = if outside {
            d / 2. + BACK_OFF
        } else {
            (minor / 2. - BACK_OFF).max(0.)
        };
        cut.rapid(back, end + CLEAR)?;
        let mut made = if outside { f32::INFINITY } else { 0. };
        for infeed in thread_infeeds(depth, passes) {
            let r = if outside {
                d / 2. - infeed
            } else {
                minor / 2. + infeed
            };
            cut.rapid(r, end + CLEAR)?;
            cut.pass();
            cut.go(r, z0, Motion::Feed, rate)?;
            if outside && r.max(pitch_r) < made - 1e-5 {
                made = r.max(pitch_r);
                cut.ring_only([z0, end, made, rim])?;
            } else if !outside && r.min(pitch_r) > made + 1e-5 {
                made = r.min(pitch_r);
                cut.ring_only([z0, end, 0., made])?;
            }
            cut.rapid(back, z0)?;
            cut.rapid(back, end + CLEAR)?;
        }
        Ok(pitch)
    }
    /// Drill on the centre from the tailstock, by hand, pecking every
    /// diameter: the full diameter `depth` in from where the drill first
    /// touches, and its 118° point beyond. Simulated as a ring to the
    /// depth and thin rings inside the point; the part check takes the whole
    /// diameter down to the point's tip. Returns the drill's radius.
    fn drill(&self, cut: &mut Cut) -> Result<f32, String> {
        let r = num(&self.tool.p, "diameter")
            .filter(|d| *d > 0.)
            .ok_or("Enter a positive drill diameter.")?
            / 2.;
        let depth = above_zero(self.op, "depth")?;
        let all = self.material.sections(self.material.low, self.end);
        let Some(touch) = all
            .iter()
            .filter(|s| s.inner < r - ON)
            .map(|s| s.z[1])
            .reduce(f32::max)
        else {
            return Err(format!(
                "The hole is already Ø{:.2} or more: nothing to drill.",
                2. * r
            ));
        };
        let point = r * POINT_RISE;
        let full = touch - depth;
        let tip = full - point;
        if tip < self.material.low - ON {
            return Err(format!(
                "The drill's point reaches Z {:.2}, past the stock's end at Z {:.2}.",
                self.work(tip),
                self.work(self.material.low)
            ));
        }
        let end = self.end;
        let feed = self.hand();
        // The drill is in the tailstock, on the axis.
        cut.pos = [0., 0., end + CLEAR];
        cut.rapid(0., touch + CLEAR)?;
        cut.pass();
        let mut reached = touch;
        loop {
            let next = (reached - 2. * r).max(tip);
            if reached < touch {
                cut.rapid(0., reached + BACK_OFF)?;
            }
            cut.go(0., next, Motion::Feed, feed)?;
            reached = next;
            if reached <= tip + 1e-5 {
                break;
            }
            cut.rapid(0., touch + CLEAR)?;
        }
        cut.checks.push(([tip, end, 0., r], 0.));
        cut.ring_only([full, end, 0., r])?;
        // The point: rings no wider than the cone at their lower end.
        let n = (point / STEP - 1e-4).ceil().max(1.) as usize;
        let s = point / n as f32;
        for k in 0..n {
            let low = full - (k + 1) as f32 * s;
            let wide = (low - tip) / POINT_RISE;
            if wide > 0.01 {
                cut.ring_only([low, low + s, 0., wide])?;
            }
        }
        cut.rapid(0., end + CLEAR)?;
        Ok(r)
    }
}

/// Refuses `checks` (rings, each with the radial gap the part needs from
/// it) when the part comes within that gap: "The cut reaches the part at Z
/// …, Ø …." in work coordinates, the part's diameter there.
///
/// The part is read from its triangles in the setup frame. Over each ring's
/// length (less `ON` at each end, so a shoulder face on it does not count),
/// a ring cut from the outside (`r0` > 0) needs the part's farthest point
/// from the axis inside `r0` less the gap: that point is a corner of a
/// triangle clipped to the ring's length, as the distance from a line is
/// convex. A ring from the axis out (`r0` = 0) needs the part's nearest
/// point outside `r1` plus the gap: the nearest point of the clipped
/// triangles, or the axis itself where it runs through the solid.
fn part_check(
    setup: &Setup,
    mesh: &Mesh,
    stock: Bounds,
    checks: &[([f32; 4], f32)],
) -> Result<(), String> {
    if checks.is_empty() {
        return Ok(());
    }
    let geometry = Geometry::new(setup, mesh);
    let b = geometry.bounds;
    let index = Index::new(&geometry);
    let axis = [1e-4, 1e-4];
    let along = if (0..2).all(|k| b.min[k] <= axis[k] && axis[k] <= b.max[k]) {
        analysis::solid(&index, axis, b.max[2])
    } else {
        vec![]
    };
    for &(g, gap) in checks {
        let (low, high) = (g[0] + ON, g[1] - ON);
        if high <= low || high < b.min[2] || low > b.max[2] {
            continue;
        }
        // The worst point: how far inside the gap, its Z and radius.
        let mut worst: Option<(f32, f32, f32)> = None;
        let mut keep = |excess: f32, z: f32, r: f32| {
            if excess > ON && worst.is_none_or(|w| excess > w.0) {
                worst = Some((excess, z, r));
            }
        };
        let from_axis = g[2] <= 1e-6;
        if from_axis {
            let limit = g[3] + gap;
            for &(a, z) in &along {
                if z > low && a < high {
                    keep(limit, a.max(low), 0.);
                }
            }
            for t in &geometry.triangles {
                let polygon = clip(&t.p, low, high);
                if let Some((r, z)) = nearest(&polygon) {
                    keep(limit - r, z, r);
                }
            }
        } else {
            let limit = g[2] - gap;
            for t in &geometry.triangles {
                for p in clip(&t.p, low, high) {
                    let r = p[0].hypot(p[1]);
                    keep(r - limit, p[2], r);
                }
            }
        }
        if let Some((_, z, r)) = worst {
            return Err(format!(
                "The cut reaches the part at Z {:.2}, Ø{:.2}.",
                work_z(stock, z),
                2. * r
            ));
        }
    }
    Ok(())
}
/// Triangle `t` clipped to the heights `low`..`high`: a convex polygon,
/// empty when it lies outside them.
fn clip(t: &[[f32; 3]; 3], low: f32, high: f32) -> Vec<[f32; 3]> {
    let (lo, hi) = t
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), p| {
            (a.min(p[2]), b.max(p[2]))
        });
    if hi < low || lo > high {
        return vec![];
    }
    let mut polygon = t.to_vec();
    for (level, above) in [(low, true), (high, false)] {
        let side = |p: &[f32; 3]| if above { p[2] - level } else { level - p[2] };
        let mut out = vec![];
        for i in 0..polygon.len() {
            let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
            let (da, db) = (side(&a), side(&b));
            if da >= 0. {
                out.push(a);
            }
            if (da >= 0.) != (db >= 0.) {
                let t = da / (da - db);
                out.push(std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t));
            }
        }
        polygon = out;
        if polygon.is_empty() {
            break;
        }
    }
    polygon
}
/// The point of the convex `polygon` nearest the Z axis: its distance and
/// Z; None for an empty polygon.
fn nearest(polygon: &[[f32; 3]]) -> Option<(f32, f32)> {
    let first = *polygon.first()?;
    let n = polygon.len();
    // The axis runs through the polygon when the origin lies inside its
    // outline seen along Z.
    if n >= 3 {
        let cross = |a: [f32; 3], b: [f32; 3]| {
            f64::from(a[0]) * f64::from(b[1]) - f64::from(a[1]) * f64::from(b[0])
        };
        let area: f64 = (0..n)
            .map(|i| cross(polygon[i], polygon[(i + 1) % n]))
            .sum();
        if area.abs() > 1e-9 {
            let inside = (0..n).all(|i| {
                let (a, b) = (polygon[i], polygon[(i + 1) % n]);
                let edge = [b[0] - a[0], b[1] - a[1], 0.];
                cross(edge, [-a[0], -a[1], 0.]) * area.signum() >= -1e-9
            });
            if inside {
                let z = polygon.iter().map(|p| p[2]).sum::<f32>() / n as f32;
                return Some((0., z));
            }
        }
    }
    let mut best = (first[0].hypot(first[1]), first[2]);
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        let d = [b[0] - a[0], b[1] - a[1]];
        let length = d[0] * d[0] + d[1] * d[1];
        let t = if length < 1e-12 {
            0.
        } else {
            (-(a[0] * d[0] + a[1] * d[1]) / length).clamp(0., 1.)
        };
        let p: [f32; 3] = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t);
        let r = p[0].hypot(p[1]);
        if r < best.0 {
            best = (r, p[2]);
        }
    }
    Some(best)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        mesh::Mesh,
        model::Document,
        simulation::{self, Simulation},
        volume::Volume,
    };

    /// A closed tube about Z from `z0` to `z1` (mm, a solid cylinder when
    /// `inner` is 0), its faces outward.
    pub(crate) fn tube(inner: f32, outer: f32, z0: f32, z1: f32) -> Vec<[[f32; 3]; 3]> {
        const N: usize = 96;
        let at = |r: f32, i: usize, z: f32| {
            let a = std::f32::consts::TAU * (i % N) as f32 / N as f32;
            [r * a.cos(), r * a.sin(), z]
        };
        let mut t = vec![];
        for i in 0..N {
            let j = i + 1;
            let (o0b, o1b, o0t, o1t) = (
                at(outer, i, z0),
                at(outer, j, z0),
                at(outer, i, z1),
                at(outer, j, z1),
            );
            t.push([o0b, o1b, o1t]);
            t.push([o0b, o1t, o0t]);
            if inner > 0. {
                let (i0b, i1b, i0t, i1t) = (
                    at(inner, i, z0),
                    at(inner, j, z0),
                    at(inner, i, z1),
                    at(inner, j, z1),
                );
                t.push([i0b, i1t, i1b]);
                t.push([i0b, i0t, i1t]);
                t.push([o0t, o1t, i1t]);
                t.push([o0t, i1t, i0t]);
                t.push([o0b, i1b, o1b]);
                t.push([o0b, i0b, i1b]);
            } else {
                t.push([[0., 0., z1], o0t, o1t]);
                t.push([[0., 0., z0], o1b, o0b]);
            }
        }
        t
    }
    /// A lathe job: a round bar Ø`bar` × `length` in the 3-jaw, the part
    /// (`tube`s about Z, mm) with its lowest point at work Z `bottom`, and a
    /// tool of each lathe type and a Ø10 drill; no operations yet.
    pub(crate) fn job(
        bar: f32,
        length: f32,
        part: Vec<[[f32; 3]; 3]>,
        bottom: f32,
    ) -> (Document, Mesh) {
        let mesh = crate::mesh::normalize(part).unwrap();
        let mut doc = Document::sample();
        doc.sample_model = false;
        doc.machines.push(lathe::defaults());
        doc.add_lathe_setup().unwrap();
        doc.setups.truncate(1);
        doc.active = doc.setups[0].id.clone();
        let lathe_id = doc.setups[0].machine.id.clone();
        let s = &mut doc.setups[0];
        s.operations.clear();
        let lift = length + bottom - 4. / mesh.display_scale();
        for (k, v) in [
            ("stock_diameter", bar.to_string()),
            ("stock_length", length.to_string()),
            ("rx", "0".into()),
            ("ry", "0".into()),
            ("rz", "0".into()),
            ("tx", "0".into()),
            ("ty", "0".into()),
            ("tz", lift.to_string()),
        ] {
            s.p.insert(k.into(), v);
        }
        let mut drill = Tool::new("Drill Ø10", "Drill", "10");
        drill.p.insert("length".into(), "60".into());
        drill.p.insert("stickout".into(), "80".into());
        drill.p.insert("machines".into(), lathe_id);
        doc.tools = lathe::TOOLS
            .iter()
            .map(|kind| Tool::lathe(kind, kind))
            .chain([drill])
            .collect();
        (doc, mesh)
    }
    /// `job`'s tools numbered from T1 and the job given a part file, as the
    /// export-side checks (the plan check, the holding walk) need them.
    pub(crate) fn numbered(doc: &mut Document) {
        for (n, tool) in doc.tools.iter_mut().enumerate() {
            tool.p.insert("number".into(), (n + 1).to_string());
        }
        doc.file = Some(crate::model::ModelFile {
            name: "shaft.stl".into(),
            path: std::path::PathBuf::from("shaft.stl"),
            format: "stl".into(),
            bytes: 0,
        });
    }
    /// Adds a Turn of `kind` (or a Drill on the centre, "Drill") to the
    /// lathe setup with `fields`, cutting with the job's tool for it.
    pub(crate) fn add(doc: &mut Document, kind: &str, fields: &[(&str, &str)]) {
        let m = doc.setups[0].machine.clone();
        let (strategy, tool) = match kind {
            "Drill" => (Strategy::Drill, "Drill"),
            _ => (Strategy::Turn, lathe::tool_for(kind).unwrap()),
        };
        let tool = doc
            .tools
            .iter()
            .find(|t| get(&t.p, "type") == tool)
            .unwrap()
            .id
            .clone();
        let mut op = Operation::for_machine(strategy, tool, &m);
        if strategy == Strategy::Turn {
            op.set_turn_kind(kind);
        }
        for (k, v) in fields {
            op.p.insert((*k).into(), (*v).into());
        }
        doc.setups[0].operations.push(op);
    }
    /// Each operation of the job simulated in order, every setup's.
    pub(crate) fn run(doc: &Document, mesh: &Mesh) -> Vec<Result<Simulation, String>> {
        let mut out = vec![];
        simulation::sequence(doc, mesh, None, |_, _, r| out.push(r));
        out
    }
    fn plan(result: &Result<Simulation, String>) -> &Plan {
        &result.as_ref().unwrap().plan
    }
    /// The diameters of the passes along the axis: each feed along Z.
    fn diameters(plan: &Plan) -> Vec<f32> {
        plan.moves
            .iter()
            .filter(|m| m.motion == Motion::Feed && m.from[0] == m.to[0])
            .map(|m| round(2. * m.to[0]))
            .collect()
    }
    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }
    fn same(a: &[f32], b: &[f32]) {
        assert!(
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| near(*x, *y)),
            "{a:?} != {b:?}"
        );
    }
    /// Whether the voxel of `v` around `p` (its frame, mm) holds material.
    fn solid(v: &Volume, p: [f32; 3]) -> bool {
        let i: [usize; 3] =
            std::array::from_fn(|k| ((p[k] - v.bounds.min[k]) / v.cell[k]).floor() as usize);
        v.occupied[v.index(i)] != 0
    }
    /// The stock left once `sim` has run to its end.
    fn after(sim: &Simulation) -> Volume {
        let mut s = sim.clone();
        s.seek(1.);
        s.volume.unwrap()
    }
    /// A Ø30 shaft from work Z -60 to -10 in a Ø50.2 × 100 bar.
    fn shaft() -> (Document, Mesh) {
        job(50.2, 100., tube(0., 15., 0., 50.), -60.)
    }

    #[test]
    fn work_z_is_zero_at_the_free_end_which_is_the_stock_top_center_work_zero() {
        let (doc, mesh) = shaft();
        let setup = &doc.setups[0];
        assert_eq!(get(&setup.p, "origin"), ZERO);
        let stock = stock::resolve(setup, Some(&mesh)).unwrap().stock;
        // The bar sits from 0 at the chuck to 100 at its free end.
        assert!(near(stock.min[2], 0.) && near(stock.max[2], 100.));
        let datum = crate::post::datum(setup, stock, &mesh).unwrap();
        same(&datum, &[0., 0., end(stock)]);
        assert!(near(frame_z(stock, -20.), 80.));
        assert!(near(work_z(stock, 80.), -20.));
        // The part is where the fixture put it: Z -60 to -10.
        let part = simulation::part_box(setup, &mesh);
        assert!(near(work_z(stock, part.min[2]), -60.) && near(work_z(stock, part.max[2]), -10.));
        // Another work zero is refused, not converted.
        let mut other = doc.clone();
        other.setups[0]
            .p
            .insert("origin".into(), "Stock top front-left".into());
        add(&mut other, "Face", &[]);
        let result = run(&other, &mesh).remove(0);
        assert!(result.unwrap_err().contains(ZERO));
    }

    #[test]
    fn an_outside_turn_takes_equal_passes_to_the_diameter_then_the_finish_pass() {
        let (mut doc, mesh) = shaft();
        add(
            &mut doc,
            "Outside",
            &[
                ("diameter", "40"),
                ("z_end", "-30"),
                ("depth", "2"),
                ("finish", "0.2"),
                ("allowance", "0"),
                ("rpm", "710"),
            ],
        );
        let results = run(&doc, &mesh);
        let plan = plan(&results[0]);
        same(&diameters(plan), &[48.2, 46.2, 44.2, 42.2, 40.2, 40.]);
        assert_eq!(plan.levels, 6);
        assert!(plan.is_lathe());
        // Each pass feeds toward the chuck from Z +1 to the shoulder at Z -30
        // at the 0.1 mm/rev power feed: 71 mm/min at 710 rpm.
        let feeds: Vec<&Move> = plan
            .moves
            .iter()
            .filter(|m| m.motion == Motion::Feed)
            .collect();
        assert_eq!(feeds.len(), 6);
        for m in &feeds {
            assert!(near(m.from[2], 101.) && near(m.to[2], 70.), "{m:?}");
            assert!(near(m.feed, 71.) && m.from[1] == 0. && m.to[1] == 0.);
        }
        // Each pass is a ring from the shoulder to the end, from its radius out.
        assert_eq!(plan.rings.len(), 6);
        for (ring, d) in plan.rings.iter().zip([48.2, 46.2, 44.2, 42.2, 40.2, 40.]) {
            assert!(near(ring.z[0], 70.) && near(ring.z[1], 100.), "{ring:?}");
            assert!(near(ring.r[0], d / 2.) && ring.r[1] >= 25.1, "{ring:?}");
            assert_eq!(plan.moves[ring.at].motion, Motion::Feed);
        }
        // Time: the moves, and 20 s at each pass to set the dials.
        let dwells = plan.moves.iter().filter(|m| m.motion == Motion::Dwell);
        assert_eq!(dwells.clone().count(), 6);
        assert!(dwells.clone().all(|m| near(m.end - m.start, 20.)));
        let cutting: f32 = feeds.iter().map(|m| m.end - m.start).sum();
        assert!(near(cutting, 6. * 31. / 71. * 60.), "{cutting}");
        assert!(near(plan.duration, plan.moves.last().unwrap().end));
        assert!(plan.duration > cutting + 120.);
        // The simulation turns the bar to Ø40 over those 30 mm, no further.
        let v = after(results[0].as_ref().unwrap());
        assert!(!solid(&v, [21., 0., 90.]) && !solid(&v, [0., 23., 75.]));
        assert!(solid(&v, [18., 0., 90.]) && solid(&v, [23., 0., 65.]));
    }

    #[test]
    fn the_power_feed_is_the_machines_nearest_to_a_tenth_and_the_finer_of_two() {
        let mut m = lathe::defaults();
        assert!(near(power_feed(&m).unwrap(), 0.1));
        for (feeds, nearest) in [("0.04,0.08,0.12", 0.08), ("0.15,0.3", 0.15), ("0.05", 0.05)] {
            m.p.insert("feeds".into(), feeds.into());
            assert!(near(power_feed(&m).unwrap(), nearest), "{feeds}");
        }
        m.p.insert("feeds".into(), "".into());
        assert!(power_feed(&m).is_err());
    }

    #[test]
    fn steps_are_equal_and_no_larger_than_the_depth_and_the_finish_pass_comes_last() {
        same(
            &steps(50.2, 40., 2., 0.2).unwrap(),
            &[48.2, 46.2, 44.2, 42.2, 40.2, 40.],
        );
        // Bore: outward, the same way round.
        same(
            &steps(10., 20., 4., 0.5).unwrap(),
            &[13.167, 16.333, 19.5, 20.],
        );
        // Less than a finish pass left: the finish pass alone.
        same(&steps(40.1, 40., 2., 0.2).unwrap(), &[40.]);
        // No finish pass: the roughing ends on the diameter.
        same(&steps(44., 40., 2., 0.).unwrap(), &[42., 40.]);
        assert!(steps(100., 0., 0.001, 0.).is_err());
    }

    #[test]
    fn an_outside_after_an_outside_starts_from_the_turned_diameter_and_cuts_no_air() {
        let (mut doc, mesh) = shaft();
        for (d, z) in [("44", "-30"), ("40", "-30"), ("36", "-40")] {
            add(
                &mut doc,
                "Outside",
                &[
                    ("diameter", d),
                    ("z_end", z),
                    ("depth", "2"),
                    ("finish", "0.2"),
                ],
            );
        }
        let results = run(&doc, &mesh);
        same(&diameters(plan(&results[0])), &[48.2, 46.2, 44.2, 44.]);
        // From the Ø44 the first left, not from the bar.
        same(&diameters(plan(&results[1])), &[42.1, 40.2, 40.]);
        // Down to Z -40 the bar is still Ø50.2 below the shoulder at -30:
        // those passes start 1 mm before it, the ones under Ø40 at Z +1.
        let third = plan(&results[2]);
        let passes: Vec<&Move> = third
            .moves
            .iter()
            .filter(|m| m.motion == Motion::Feed)
            .collect();
        same(
            &diameters(third),
            &[48.2, 46.2, 44.2, 42.2, 40.2, 38.2, 36.2, 36.],
        );
        for m in passes {
            let start = if 2. * m.to[0] > 40. { 71. } else { 101. };
            assert!(near(m.from[2], start), "{m:?}");
            assert!(near(m.to[2], 60.));
        }
        // The same diameter again has nothing to cut.
        let (mut again, mesh) = shaft();
        for _ in 0..2 {
            add(
                &mut again,
                "Outside",
                &[("diameter", "40"), ("z_end", "-30")],
            );
        }
        let error = run(&again, &mesh).remove(1).unwrap_err();
        assert!(error.contains("already Ø40.00"), "{error}");
    }

    #[test]
    fn a_bore_after_a_drill_widens_the_hole_in_equal_passes_to_the_diameter() {
        // A tube with a Ø24 bore: the Ø20 bore keeps 2 mm clear of it.
        let (mut doc, mesh) = job(50., 100., tube(12., 20., 0., 50.), -60.);
        let bar = doc
            .tools
            .iter_mut()
            .find(|t| get(&t.p, "type") == "Boring bar")
            .unwrap();
        bar.p.insert("min_bore".into(), "8".into());
        add(&mut doc, "Drill", &[("depth", "30")]);
        add(
            &mut doc,
            "Bore",
            &[
                ("diameter", "20"),
                ("z_end", "-20"),
                ("depth", "1"),
                ("finish", "0.1"),
                ("allowance", "0"),
            ],
        );
        let results = run(&doc, &mesh);
        let bore = plan(&results[1]);
        let d = diameters(bore);
        assert_eq!(d.len(), 11, "{d:?}");
        assert!(near(d[0], 10.99) && near(d[9], 19.9) && near(d[10], 20.));
        for w in d.windows(2) {
            assert!(w[1] > w[0] && w[1] - w[0] <= 1. + 1e-3, "{d:?}");
        }
        // Rings from the axis out to each pass, Z -20 to the end.
        for (ring, d) in bore.rings.iter().zip(&d) {
            assert!(ring.r[0] == 0. && near(ring.r[1], d / 2.) && near(ring.z[0], 80.));
        }
        let v = after(results[1].as_ref().unwrap());
        assert!(!solid(&v, [9., 0., 85.]) && solid(&v, [11., 0., 85.]));
        // Without the drill there is no hole to bore.
        let mut undrilled = doc.clone();
        undrilled.setups[0].operations.remove(0);
        let error = run(&undrilled, &mesh).remove(0).unwrap_err();
        assert!(error.contains("needs a hole"), "{error}");
        // A bar too big for the Ø10 hole is refused.
        let mut big = doc.clone();
        for t in &mut big.tools {
            if get(&t.p, "type") == "Boring bar" {
                t.p.insert("min_bore".into(), "12".into());
            }
        }
        let error = run(&big, &mesh).remove(1).unwrap_err();
        assert!(
            error.contains("The hole is Ø10.00") && error.contains("Ø12"),
            "{error}"
        );
    }

    #[test]
    fn the_drill_on_the_centre_goes_its_depth_at_full_diameter_and_its_point_beyond() {
        let (mut doc, mesh) = job(50., 100., tube(12., 20., 0., 50.), -60.);
        add(&mut doc, "Drill", &[("depth", "30")]);
        let results = run(&doc, &mesh);
        let plan = plan(&results[0]);
        let tip = 70. - 5. * POINT_RISE;
        let feeds: Vec<&Move> = plan
            .moves
            .iter()
            .filter(|m| m.motion == Motion::Feed)
            .collect();
        // Pecks every diameter: to 90, 80, 70 and the tip; on the axis.
        let depths: Vec<f32> = feeds.iter().map(|m| m.to[2]).collect();
        same(&depths, &[90., 80., 70., tip]);
        assert!(plan.moves.iter().all(|m| m.from[0] == 0. && m.to[0] == 0.));
        // By hand at 0.05 mm/rev on the top gear.
        let rpm = num(&doc.setups[0].operations[0].p, "rpm").unwrap();
        assert!(feeds.iter().all(|m| near(m.feed, 0.05 * rpm)));
        assert_eq!(plan.levels, 1);
        // The full diameter to Z -30, and the point's rings inside its cone.
        let first = plan.rings[0];
        assert!(near(first.z[0], 70.) && near(first.z[1], 100.) && near(first.r[1], 5.));
        for ring in &plan.rings[1..] {
            assert!(ring.z[0] >= tip - 1e-4 && ring.z[1] <= 70. + 1e-4);
            assert!(
                ring.r[1] <= (ring.z[0] - tip) / POINT_RISE + 1e-4,
                "{ring:?}"
            );
        }
        let v = after(results[0].as_ref().unwrap());
        assert!(!solid(&v, [0.5, 0.5, 72.]) && solid(&v, [0.5, 0.5, 60.]));
        assert!(solid(&v, [7., 0., 85.]));
    }

    #[test]
    fn a_face_takes_its_cut_off_the_end_as_it_is() {
        let (mut doc, mesh) = shaft();
        add(&mut doc, "Face", &[("face_off", "1"), ("to_diameter", "0")]);
        add(&mut doc, "Face", &[("face_off", "1"), ("to_diameter", "0")]);
        let results = run(&doc, &mesh);
        let first = plan(&results[0]);
        let feeds: Vec<&Move> = first
            .moves
            .iter()
            .filter(|m| m.motion == Motion::Feed)
            .collect();
        assert_eq!(feeds.len(), 1);
        // Across the end at Z -1, from outside the bar to the centre.
        assert!(near(feeds[0].from[2], 99.) && near(feeds[0].to[2], 99.));
        assert!(feeds[0].from[0] > 25.1 && near(feeds[0].to[0], 0.));
        let rpm = num(&doc.setups[0].operations[0].p, "rpm").unwrap();
        assert!(near(feeds[0].feed, 0.05 * rpm));
        assert_eq!(first.rings.len(), 1);
        let ring = first.rings[0];
        assert!(near(ring.z[0], 99.) && near(ring.z[1], 100.) && ring.r[0] == 0.);
        // The second takes the next millimetre off the faced end.
        let ring = plan(&results[1]).rings[0];
        assert!(near(ring.z[0], 98.), "{ring:?}");
        let v = after(results[1].as_ref().unwrap());
        assert!(!solid(&v, [0.5, 0.5, 98.6]) && solid(&v, [0.5, 0.5, 97.4]));
    }

    #[test]
    fn a_groove_at_z_minus_20_is_cut_20_mm_from_the_free_end() {
        let (mut doc, mesh) = shaft();
        add(
            &mut doc,
            "Groove",
            &[("z", "-20"), ("width", "3"), ("diameter", "35")],
        );
        add(
            &mut doc,
            "Groove",
            &[("z", "-50"), ("width", "6"), ("diameter", "40")],
        );
        let results = run(&doc, &mesh);
        let one = plan(&results[0]);
        let feeds: Vec<&Move> = one
            .moves
            .iter()
            .filter(|m| m.motion == Motion::Feed)
            .collect();
        // One plunge, the tool's left side at Z -20, to Ø35.
        assert_eq!(feeds.len(), 1);
        assert!(near(feeds[0].from[2], 80.) && near(feeds[0].to[2], 80.));
        assert!(near(feeds[0].to[0], 17.5) && feeds[0].from[0] > 25.);
        let ring = one.rings[0];
        assert!(near(ring.z[0], 80.) && near(ring.z[1], 83.) && near(ring.r[0], 17.5));
        let v = after(results[0].as_ref().unwrap());
        assert!(!solid(&v, [21., 0., 81.5]) && !solid(&v, [0., -21., 81.5]));
        for kept in [[21., 0., 78.], [21., 0., 85.], [16., 0., 81.5]] {
            assert!(solid(&v, kept), "{kept:?}");
        }
        // A groove twice the tool's width takes two plunges side by side.
        let two = plan(&results[1]);
        let sides: Vec<f32> = two.rings.iter().map(|r| r.z[0]).collect();
        same(&sides, &[50., 53.]);
        assert!(two.rings.iter().all(|r| near(r.z[1] - r.z[0], 3.)));
        assert_eq!(two.levels, 2);
    }

    #[test]
    fn parting_off_goes_past_the_centre_and_the_bar_left_in_the_chuck_goes() {
        let (mut doc, mesh) = shaft();
        add(&mut doc, "Part off", &[("z", "-62")]);
        // The starter blade reaches 25 mm: not the Ø50.2 bar's centre.
        let error = run(&doc, &mesh).remove(0).unwrap_err();
        assert!(error.contains("reaches 25 mm"), "{error}");
        for t in &mut doc.tools {
            if get(&t.p, "type") == "Parting blade" {
                t.p.insert("reach".into(), "30".into());
            }
        }
        let results = run(&doc, &mesh);
        let plan = plan(&results[0]);
        let feeds: Vec<&Move> = plan
            .moves
            .iter()
            .filter(|m| m.motion == Motion::Feed)
            .collect();
        // The 3 mm blade's side toward the part at Z -62: its left at -65.
        assert_eq!(feeds.len(), 1);
        assert!(near(feeds[0].to[0], -0.5) && near(feeds[0].to[2], 35.));
        assert_eq!(plan.rings.len(), 2);
        let (cut, left) = (plan.rings[0], plan.rings[1]);
        assert!(near(cut.z[0], 35.) && near(cut.z[1], 38.) && cut.r[0] == 0.);
        assert!(near(left.z[0], 0.) && near(left.z[1], 35.) && left.r[0] == 0.);
        let v = after(results[0].as_ref().unwrap());
        assert!(!solid(&v, [5., 0., 20.]) && !solid(&v, [5., 0., 36.5]));
        assert!(solid(&v, [5., 0., 40.]));
        // Into the part: refused.
        let mut into = doc.clone();
        into.setups[0].operations[0]
            .p
            .insert("z".into(), "-50".into());
        let error = run(&into, &mesh).remove(0).unwrap_err();
        assert!(
            error.starts_with("The cut reaches the part at Z -5"),
            "{error}"
        );
    }

    #[test]
    fn a_thread_feeds_in_on_the_infeed_table_to_its_depth_at_the_pitch() {
        // M40 × 1.5 on a Ø40 bar.
        let (mut doc, mesh) = job(40., 100., tube(0., 15., 0., 50.), -60.);
        add(
            &mut doc,
            "Thread",
            &[
                ("thread_side", "Outside"),
                ("diameter", "40"),
                ("pitch", "1.5"),
                ("z_end", "-20"),
            ],
        );
        let results = run(&doc, &mesh);
        let plan = plan(&results[0]);
        let feeds: Vec<&Move> = plan
            .moves
            .iter()
            .filter(|m| m.motion == Motion::Feed)
            .collect();
        assert_eq!(feeds.len(), 8);
        assert_eq!(thread_passes(1.5), 8);
        let infeeds: Vec<f32> = feeds.iter().map(|m| 20. - m.to[0]).collect();
        assert!((infeeds[7] - 0.61 * 1.5).abs() < 0.01, "{infeeds:?}");
        assert!((infeeds[7] - 0.915).abs() < 1e-4, "{infeeds:?}");
        let mut last = f32::INFINITY;
        for (i, w) in infeeds.iter().enumerate() {
            let step = w - if i == 0 { 0. } else { infeeds[i - 1] };
            assert!(step > 0. && step < last, "{infeeds:?}");
            last = step;
        }
        // At the pitch per turn, along the thread to Z -20.
        let rpm = num(&doc.setups[0].operations[0].p, "rpm").unwrap();
        for m in &feeds {
            assert!(near(m.feed, 1.5 * rpm) && near(m.from[2], 101.) && near(m.to[2], 80.));
        }
        // The simulation leaves the pitch diameter.
        let deepest = plan
            .rings
            .iter()
            .map(|r| r.r[0])
            .fold(f32::INFINITY, f32::min);
        assert!(near(2. * deepest, pitch_diameter(40., 1.5)), "{deepest}");
        assert!(near(pitch_diameter(40., 1.5), 39.026));
        // Inside, the depth is 0.54 × pitch.
        assert!(near(thread_depth(false, 1.5), 0.81));
        assert!(near(*thread_infeeds(0.81, 7).last().unwrap(), 0.81));
        assert_eq!([0.5, 1., 2., 3.].map(thread_passes), [4, 6, 10, 14]);
        // An unturned blank is refused.
        let (mut big, mesh) = job(50., 100., tube(0., 15., 0., 50.), -60.);
        add(
            &mut big,
            "Thread",
            &[("diameter", "40"), ("pitch", "1.5"), ("z_end", "-20")],
        );
        let error = run(&big, &mesh).remove(0).unwrap_err();
        assert!(error.contains("blank to Ø40.00"), "{error}");
    }

    #[test]
    fn the_part_check_refuses_a_turn_into_the_part_and_passes_one_clear_of_it() {
        // A Ø40 shaft from Z -60 to -10.
        let part = || tube(0., 20., 0., 50.);
        let outside = |d: &str, allowance: &str| {
            let (mut doc, mesh) = job(50., 100., part(), -60.);
            add(
                &mut doc,
                "Outside",
                &[("diameter", d), ("z_end", "-30"), ("allowance", allowance)],
            );
            run(&doc, &mesh).remove(0)
        };
        let error = outside("39", "0").unwrap_err();
        assert!(
            error.starts_with("The cut reaches the part at Z -") && error.ends_with(", Ø40.00."),
            "{error}"
        );
        let z: f32 = error["The cut reaches the part at Z ".len()..]
            .split(',')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!((-30. ..=-10.).contains(&z), "{z}");
        // Half a millimetre above the part's diameter, and on it, pass.
        for d in ["40.5", "40"] {
            assert!(outside(d, "0").is_ok(), "{d}");
        }
        // The stock to leave is on the diameter: Ø40 + 0.4 passes, and
        // every ring over the part keeps half of it clear on the radius.
        for (d, allowance) in [("40", "0.4"), ("40.2", "0.4")] {
            let sim = outside(d, allowance).unwrap();
            for ring in &sim.plan.rings {
                assert!(ring.r[0] >= 20. + 0.2 - 1e-4, "{ring:?}");
            }
        }
        // A face into the part's end is refused, one short of it passes.
        let face = |off: &str| {
            let (mut doc, mesh) = job(50., 100., part(), -60.);
            add(&mut doc, "Face", &[("face_off", off)]);
            run(&doc, &mesh).remove(0)
        };
        assert!(
            face("11")
                .unwrap_err()
                .starts_with("The cut reaches the part at Z -1")
        );
        assert!(face("10").is_ok());
        // A drill into a solid part is refused at the axis.
        let (mut doc, mesh) = job(50., 100., part(), -60.);
        add(&mut doc, "Drill", &[("depth", "20")]);
        let error = run(&doc, &mesh).remove(0).unwrap_err();
        assert!(error.contains("Ø0.00"), "{error}");
    }

    #[test]
    fn rings_never_reach_into_the_part_or_its_allowance() {
        // A Ø30 shaft from Z -60 to -10: face, turn, chamfer, a groove past
        // it, part off.
        let (mut doc, mesh) = job(50., 100., tube(0., 15., 0., 50.), -60.);
        add(&mut doc, "Face", &[("face_off", "2")]);
        add(
            &mut doc,
            "Outside",
            &[("diameter", "30"), ("z_end", "-60"), ("allowance", "0.6")],
        );
        add(
            &mut doc,
            "Chamfer",
            &[("edge", "End outside"), ("size", "1")],
        );
        add(
            &mut doc,
            "Groove",
            &[("z", "-70"), ("width", "3"), ("diameter", "20")],
        );
        add(&mut doc, "Part off", &[("z", "-62")]);
        let results = run(&doc, &mesh);
        let stock = stock::resolve(&doc.setups[0], Some(&mesh)).unwrap().stock;
        let part = simulation::part_box(&doc.setups[0], &mesh);
        for (i, result) in results.iter().enumerate() {
            let plan = &result.as_ref().unwrap_or_else(|e| panic!("{i}: {e}")).plan;
            let gap = if i == 1 { 0.3 } else { 0. };
            for ring in &plan.rings {
                // Over the part's length a ring stays outside its Ø30 plus
                // the gap; elsewhere it may go to the axis.
                let over = ring.z[1] > part.min[2] + 1e-3 && ring.z[0] < part.max[2] - 1e-3;
                if over {
                    assert!(ring.r[0] >= 15. + gap - 1e-4, "{i}: {ring:?}");
                }
                assert!(ring.z[1] <= end(stock) + 1e-4);
            }
        }
        // The chamfer breaks the corner of the faced end (Z -2) on the Ø30.6
        // the Outside left, its rings inside the 45° triangle: never deeper
        // than the chamfer's line.
        let chamfer = plan(&results[2]);
        assert_eq!(chamfer.rings.len(), 9);
        for ring in &chamfer.rings {
            assert!(ring.z[1] <= 98. + 1e-4 && ring.z[0] >= 97. - 1e-4);
            assert!(ring.r[0] >= 15.3 - (ring.z[0] - 97.) - 1e-4, "{ring:?}");
        }
    }

    #[test]
    fn a_chamfer_breaks_the_end_its_hole_or_the_first_shoulder() {
        let (mut doc, mesh) = job(50., 100., tube(0., 10., 0., 30.), -60.);
        add(&mut doc, "Outside", &[("diameter", "30"), ("z_end", "-20")]);
        for edge in crate::lathe::EDGES {
            add(&mut doc, "Chamfer", &[("edge", edge), ("size", "1")]);
        }
        // End inside needs a hole: refused, then the shoulder.
        let results = run(&doc, &mesh);
        let outside = plan(&results[1]);
        let feed = outside
            .moves
            .iter()
            .find(|m| m.motion == Motion::Feed)
            .unwrap();
        // From (Ø28, the end) to (Ø30, 1 mm in): the 45° leg.
        assert!(near(feed.from[0], 14.) && near(feed.from[2], 100.));
        assert!(near(feed.to[0], 15.) && near(feed.to[2], 99.));
        assert_eq!(outside.rings.len(), 9);
        let error = results[2].as_ref().unwrap_err();
        assert!(error.contains("needs a hole"), "{error}");
        // With End inside gone, the shoulder at Z -20 from Ø50 down to Ø30.
        doc.setups[0].operations.remove(2);
        let results = run(&doc, &mesh);
        let shoulder = plan(&results[2]);
        let feed = shoulder
            .moves
            .iter()
            .find(|m| m.motion == Motion::Feed)
            .unwrap();
        assert!(near(feed.from[0], 24.) && near(feed.from[2], 80.));
        assert!(near(feed.to[0], 25.) && near(feed.to[2], 79.));
    }

    #[test]
    fn a_second_lathe_setup_turned_end_for_end_starts_from_the_first_ones_cuts() {
        // Setup 1 turns Ø40 from its end to Z -95, leaving 5 mm of the
        // Ø50.2 bar at the chuck. Setup 2 holds the bar the other way round:
        // that 5 mm is at its free end.
        let (mut doc, mesh) = shaft();
        add(&mut doc, "Outside", &[("diameter", "40"), ("z_end", "-95")]);
        let mut flipped = doc.setups[0].clone();
        flipped.id = crate::model::id();
        flipped.operations.clear();
        flipped.p.insert("ry".into(), "180".into());
        flipped.p.insert("offset".into(), "G55".into());
        doc.setups.push(flipped);
        doc.active = doc.setups[1].id.clone();
        let tool = doc.setups[0].operations[0].tool.clone();
        let m = doc.setups[1].machine.clone();
        for z in ["-4", "-50"] {
            let mut op = Operation::for_machine(Strategy::Turn, tool.clone(), &m);
            op.set_turn_kind("Groove");
            op.tool = doc
                .tools
                .iter()
                .find(|t| get(&t.p, "type") == "Grooving tool")
                .unwrap()
                .id
                .clone();
            for (k, v) in [("z", z), ("width", "3"), ("diameter", "36")] {
                op.p.insert(k.into(), v.into());
            }
            doc.setups[1].operations.push(op);
        }
        let results = run(&doc, &mesh);
        // Each plunge starts 1 mm off the material there: the bar's Ø50.2
        // near setup 2's end, the Ø40 setup 1 left further in.
        for (result, outer) in results[1..].iter().zip([25.1, 20.]) {
            let plan = plan(result);
            let feed = plan
                .moves
                .iter()
                .find(|m| m.motion == Motion::Feed)
                .unwrap();
            assert!(near(feed.from[0], outer + CLEAR), "{feed:?}");
            assert!(near(feed.to[0], 18.));
        }
    }
    #[test]
    fn a_lathe_then_mill_job_hands_the_mill_the_turned_diameter() {
        // Ø30 × 50 inside the Ø40 × 62 the lathe turns and parts off.
        let (mut doc, mesh) = shaft();
        add(&mut doc, "Outside", &[("diameter", "40"), ("z_end", "-70")]);
        add(&mut doc, "Part off", &[("z", "-62")]);
        let mut mill = crate::model::Setup::new(crate::model::Machine::new(3), "G55");
        mill.p.insert("ry".into(), "90".into());
        doc.setups.push(mill);
        let mut starts = vec![];
        simulation::sequence_walk(
            &doc,
            &mesh,
            None,
            |si, oi, r| assert!(r.is_ok(), "{si}/{oi}: {:?}", r.err()),
            |_, _, _| {},
            |si, start| starts.push((si, start)),
        );
        let (si, start) = starts[1];
        assert_eq!(si, 1);
        let mut size = start.unwrap().dimensions();
        size.sort_by(f32::total_cmp);
        assert!(
            (size[0] - 40.).abs() < 0.01
                && (size[1] - 40.).abs() < 0.01
                && (size[2] - 62.).abs() < 0.01,
            "{size:?}"
        );
    }
}
