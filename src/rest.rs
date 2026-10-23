//! Conservative start-height planning from earlier continuous cutter sweeps.
//! A cap is skipped only when its whole volume is covered by known cuts,
//! or, over the area a cutter reaches (`Reach`), all of it there is.
//! Coarse display voxels and samples never establish that material is absent.
use crate::{
    mesh::Bounds,
    model::Setup,
    plan_check::PartGrid,
    simulation::CutterShape,
    stock_surface::Cut,
    volume::{Frame, Volume},
};
use clipper2::{FillRule, Milli, difference, intersect};

/// The units of `covered` work `left` and `band` spend on each side.
const BUDGET: usize = 200_000;
/// How far short of a capsule's face, mm, `band` takes a candidate side:
/// a slab whose face lies this far inside the removed material is proven
/// with far fewer, larger boxes than one a hair inside it, and the jaws
/// close at most this much outside a cut wall.
const BAND_SLACK: f32 = 0.01;
/// How far inside its cylinder a sweep's outline (`cleared`) may run, mm:
/// its chords' sagitta.
const CHORD: f32 = 0.01;
/// How clear a point is of the earlier sweeps and of the area's edge, mm,
/// when it shows material left there (`RestStock::uncut`): far past
/// Clipper's rounding to thousandths, so its clipping leaves that too.
const CLEAR: f64 = 0.01;
#[derive(Debug, Clone)]
struct Capsule {
    cut: Cut,
    bounds: Bounds,
}
#[derive(Debug, Clone)]
pub struct RestStock {
    bounds: Bounds,
    reference: Frame,
    current: Frame,
    /// The box the material left lies in, in `current`: the stock's box,
    /// less what lathe cuts turned away (`turned`).
    extent: Bounds,
    /// The stock's whole box in `current`.
    whole: Bounds,
    cuts: Vec<Capsule>,
    /// Where every earlier cut, of any cutter shape, can have removed
    /// material (`removal_boxes`, in `current`): what `bottom` reads.
    removal: Vec<Bounds>,
    /// The earlier lathe cuts about `current`'s Z axis (`coaxial`).
    coaxial: Coaxial,
    /// Every earlier cut, of any shape, in its own frame (`bored`).
    all: std::sync::Arc<Vec<Cut>>,
}
fn corners(b: Bounds) -> [[f32; 3]; 8] {
    std::array::from_fn(|mask| {
        std::array::from_fn(|k| {
            if mask & (1 << k) == 0 {
                b.min[k]
            } else {
                b.max[k]
            }
        })
    })
}
fn transfer(p: [f32; 3], from: Frame, to: Frame) -> [f32; 3] {
    if from == to { p } else { from.transfer(p, to) }
}
fn transformed(b: Bounds, from: Frame, to: Frame) -> Bounds {
    let mut out = Bounds {
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
    };
    for p in corners(b).map(|p| transfer(p, from, to)) {
        for (k, &v) in p.iter().enumerate() {
            out.min[k] = out.min[k].min(v);
            out.max[k] = out.max[k].max(v);
        }
    }
    out
}
/// The box, in `current`, the material left in `volume` lies in once its
/// lathe cuts (`Revolve`) are made: None without one. In each frame those
/// cuts were made in, the stock is taken as a bar round that frame's Z axis
/// out to its box's farthest corner (or to the round stock's cylinder, when
/// that shares the axis); at each height the rings reaching in from there
/// without a gap take that radius down to their inner one, and a section
/// they take to the axis is gone. The bar left, as wide as the widest
/// section left and as long as the sections left reach, bounds the
/// material: each frame's box, seen in `current`, narrows the one before.
/// Other cuts are not counted, so the box never claims more than was cut.
fn turned(volume: &Volume, current: Frame) -> Option<Bounds> {
    const TOLERANCE: f32 = 1e-4;
    let mut frames: Vec<Frame> = vec![];
    for c in volume.cuts.iter() {
        if c.shape == CutterShape::Revolve && !frames.contains(&c.frame) {
            frames.push(c.frame);
        }
    }
    if frames.is_empty() {
        return None;
    }
    let mut out = volume.transformed_bounds(current);
    for frame in frames {
        let seen = transformed(out, current, frame);
        // (z0, z1, r0, r1) of each ring made in this frame.
        let rings: Vec<[f32; 4]> = volume
            .cuts
            .iter()
            .filter(|c| c.shape == CutterShape::Revolve && c.frame == frame)
            .map(|c| [c.a[0], c.b[0], c.a[1], c.b[1]])
            .collect();
        let mut outer = corners(seen)
            .iter()
            .map(|p| p[0].hypot(p[1]))
            .fold(0., f32::max);
        if volume.round {
            let (centre, radius) = crate::stock::bar(volume.bounds);
            let axis = volume.frame.transfer_vector([0., 0., 1.], frame);
            if axis[0].abs() < 1e-4 && axis[1].abs() < 1e-4 {
                let c = volume.frame.transfer([centre[0], centre[1], 0.], frame);
                outer = outer.min(c[0].hypot(c[1]) + radius);
            }
        }
        let mut levels: Vec<f32> = rings
            .iter()
            .flat_map(|r| [r[0], r[1]])
            .filter(|z| *z > seen.min[2] && *z < seen.max[2])
            .chain([seen.min[2], seen.max[2]])
            .collect();
        levels.sort_by(f32::total_cmp);
        levels.dedup();
        let (mut low, mut high, mut widest) = (f32::INFINITY, f32::NEG_INFINITY, 0f32);
        for w in levels.windows(2) {
            let mut r = outer;
            loop {
                let inner = rings
                    .iter()
                    .filter(|g| {
                        g[0] <= w[0] + TOLERANCE
                            && g[1] >= w[1] - TOLERANCE
                            && g[2] < r
                            && g[3] >= r - TOLERANCE
                    })
                    .map(|g| g[2])
                    .fold(r, f32::min);
                if inner >= r {
                    break;
                }
                r = inner;
            }
            if r > TOLERANCE {
                (low, high, widest) = (low.min(w[0]), high.max(w[1]), widest.max(r));
            }
        }
        // Every section turned to the axis: nothing proven, keep the box.
        if low > high {
            continue;
        }
        let left = Bounds {
            min: [(-widest).max(seen.min[0]), (-widest).max(seen.min[1]), low],
            max: [widest.min(seen.max[0]), widest.min(seen.max[1]), high],
        };
        let back = transformed(left, frame, current);
        for k in 0..3 {
            out.min[k] = out.min[k].max(back.min[k]);
            out.max[k] = out.max[k].min(back.max[k]);
        }
    }
    Some(out)
}
/// The lathe's view of the stock in `volume` about `current`'s Z axis
/// (X = Y = 0), what a Turn plans its passes from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Coaxial {
    /// How far from the axis the round bar reaches, when the stock started
    /// as a round bar whose axis is parallel to it: its radius, plus the
    /// offset of its axis when that is not on it.
    pub bar: Option<f32>,
    /// Every lathe cut (`Revolve`) made about this same axis, in any setup
    /// whose Z axis lies on it either way round, as `[z0, z1, r0, r1]` in
    /// `current` (mm, `z0 <= z1`, `r0 <= r1`). Lathe cuts about another
    /// axis, and every other cut, are left out: the passes may start in air
    /// after them, but never deeper than the material.
    pub rings: Vec<[f32; 4]>,
}
/// `Coaxial` of `volume` about `current`'s Z axis.
pub fn coaxial(volume: &Volume, current: Frame) -> Coaxial {
    const TOLERANCE: f32 = 1e-3;
    let on_axis = |frame: Frame, p: [f32; 3]| {
        let axis = frame.transfer_vector([0., 0., 1.], current);
        let at = frame.transfer(p, current);
        (axis[0].abs() < 1e-4 && axis[1].abs() < 1e-4).then_some(at)
    };
    let bar = volume
        .round
        .then(|| crate::stock::bar(volume.bounds))
        .and_then(|(centre, radius)| {
            on_axis(volume.frame, [centre[0], centre[1], 0.]).map(|c| c[0].hypot(c[1]) + radius)
        });
    let rings = volume
        .cuts
        .iter()
        .filter(|c| c.shape == CutterShape::Revolve)
        .filter_map(|c| {
            let low = on_axis(c.frame, [0., 0., c.a[0]])?;
            let high = on_axis(c.frame, [0., 0., c.b[0]])?;
            (low[0].hypot(low[1]) < TOLERANCE).then_some([
                low[2].min(high[2]),
                low[2].max(high[2]),
                c.a[1],
                c.b[1],
            ])
        })
        .collect();
    Coaxial { bar, rings }
}
/// Box, in `current`, around each earlier cut of any cutter shape: material can
/// only have been removed inside these boxes.
pub fn cut_boxes(volume: &Volume, current: Frame) -> Vec<Bounds> {
    volume.cuts.iter().map(|c| cut_box(c, current)).collect()
}
/// `cut_boxes`' box of one cut: a lathe's ring's own box.
pub fn cut_box(c: &Cut, current: Frame) -> Bounds {
    let b = c.ring().unwrap_or_else(|| Bounds {
        min: std::array::from_fn(|k| c.a[k].min(c.b[k]) - c.radius),
        max: std::array::from_fn(|k| {
            c.a[k].max(c.b[k]) + c.radius + if k == 2 { c.flute } else { 0. }
        }),
    });
    transformed(b, c.frame, current)
}
/// Box, in `current`, around the material each earlier cut can have
/// removed: within the tool radius of its path, from its lowest tip up to
/// its flute top, as `Volume::cut` carves. `cut_boxes` also reaches a tool
/// radius below the tip.
pub fn removal_boxes(volume: &Volume, current: Frame) -> Vec<Bounds> {
    volume
        .cuts
        .iter()
        .map(|c| removal_box(c, current))
        .collect()
}
/// `removal_boxes`' box of one cut: a lathe's ring's own box.
pub fn removal_box(c: &Cut, current: Frame) -> Bounds {
    let b = c.ring().unwrap_or(Bounds {
        min: [
            c.a[0].min(c.b[0]) - c.radius,
            c.a[1].min(c.b[1]) - c.radius,
            c.a[2].min(c.b[2]),
        ],
        max: [
            c.a[0].max(c.b[0]) + c.radius,
            c.a[1].max(c.b[1]) + c.radius,
            c.a[2].max(c.b[2]) + c.flute,
        ],
    });
    transformed(b, c.frame, current)
}
/// Where earlier cuts can have removed material, for single points: within
/// the tool radius of each cut's path, from its tip up to its flute top,
/// tested with `Volume::cut`'s own `swept`. Tighter than `removal_boxes`
/// around arcs and diagonal moves, whose boxes reach up to a tool radius
/// past the path.
pub struct Removed<'a> {
    cuts: &'a [Cut],
    current: Frame,
    /// `removal_boxes`, one per cut: a quick test before the exact one.
    boxes: Vec<Bounds>,
    /// The stock earlier setups left, whose voxels `joined` reads.
    volume: &'a Volume,
    /// Per voxel of `volume`: whether it holds material joined to the part
    /// (`with_part`). None until the part is given, and without earlier cuts.
    joined: Option<Vec<bool>>,
}
impl<'a> Removed<'a> {
    pub fn new(volume: &'a Volume, current: Frame) -> Self {
        Self {
            cuts: &volume.cuts[..],
            current,
            boxes: removal_boxes(volume, current),
            volume,
            joined: None,
        }
    }
    /// `self` knowing the part, `part` being it on the stock's grid (the
    /// voxels whose centres lie inside the model, `PartGrid`): which of the
    /// stock's voxels still hold material joined to it (`joined`). A flood
    /// fill through the occupied voxels (6-connectivity) from the part's
    /// own, each voxel visited once (at most 96³). Nothing is filled without
    /// earlier cuts, the stock then being one piece, nor when no material is
    /// left inside the part.
    pub fn with_part(mut self, part: &PartGrid) -> Self {
        let v = self.volume;
        if v.cuts.is_empty() {
            return self;
        }
        let mut joined = vec![false; v.occupied.len()];
        let mut stack = vec![];
        for z in 0..v.n[2] {
            for y in 0..v.n[1] {
                for x in 0..v.n[0] {
                    let i = v.index([x, y, z]);
                    if v.occupied[i] != 0 && part.inside(i) {
                        joined[i] = true;
                        stack.push([x, y, z]);
                    }
                }
            }
        }
        if stack.is_empty() {
            return self;
        }
        while let Some(p) = stack.pop() {
            for k in 0..3 {
                for next in [p[k].checked_sub(1), Some(p[k] + 1)] {
                    let Some(next) = next.filter(|c| *c < v.n[k]) else {
                        continue;
                    };
                    let mut q = p;
                    q[k] = next;
                    let i = v.index(q);
                    if v.occupied[i] != 0 && !joined[i] {
                        joined[i] = true;
                        stack.push(q);
                    }
                }
            }
        }
        self.joined = Some(joined);
        self
    }
    /// Whether the stock's voxel at `p` (current frame) holds material joined
    /// to the part (`with_part`): Some(true) when it does, Some(false) for
    /// material cut free from it, None for a voxel cut away, a point outside
    /// the stock, or the part not given.
    pub fn joined(&self, p: [f32; 3]) -> Option<bool> {
        let joined = self.joined.as_ref()?;
        let v = self.volume;
        let q = transfer(p, self.current, v.frame);
        let mut cell = [0; 3];
        for k in 0..3 {
            let t = ((q[k] - v.bounds.min[k]) / v.cell[k]).floor();
            if !(t >= 0. && t < v.n[k] as f32) {
                return None;
            }
            cell[k] = t as usize;
        }
        let i = v.index(cell);
        (v.occupied[i] != 0).then(|| joined[i])
    }
    /// Whether an earlier cut can have removed material at `p`, in the
    /// current frame. A point within 0.001 mm of a cut counts as removed,
    /// which keeps float error in the frame transforms on the safe side.
    pub fn at(&self, p: [f32; 3]) -> bool {
        self.by(p).is_some()
    }
    /// How far the current frame's up (+Z) points up in the frame of the
    /// setup that made `source`'s cuts (the first cut of any operation when
    /// None): 1 when the two setups face the same way, -1 when the current one
    /// is turned over, about 0 when it is turned on its side. None without
    /// cuts, or when no cut has that source.
    pub fn up_in(&self, source: Option<(u16, u16)>) -> Option<f32> {
        let cut = match source {
            Some(source) => self.cuts.iter().find(|c| c.source == source)?,
            None => self.cuts.first()?,
        };
        let origin = transfer([0.; 3], self.current, cut.frame);
        let up = transfer([0., 0., 1.], self.current, cut.frame);
        Some(up[2] - origin[2])
    }
    /// The operation whose cut removed material at `p` (`at`), as its
    /// `Cut::source`: the first such cut's, in the order they were made.
    pub fn by(&self, p: [f32; 3]) -> Option<(u16, u16)> {
        self.cuts
            .iter()
            .zip(&self.boxes)
            .find(|(cut, b)| removes(cut, b, self.current, p))
            .map(|(cut, _)| cut.source)
    }
}
/// Whether `cut`, whose `removal_box` is `b`, can have removed material at
/// `p` (current frame), as `Removed::at` tests it.
fn removes(cut: &Cut, b: &Bounds, current: Frame, p: [f32; 3]) -> bool {
    const GUARD: f32 = 0.001;
    (0..3).all(|k| p[k] >= b.min[k] - 2. * GUARD && p[k] <= b.max[k] + 2. * GUARD) && {
        let q = transfer(p, current, cut.frame);
        if cut.shape == CutterShape::Revolve {
            // The ring grown by the guard on every side.
            return crate::volume::swept(
                q,
                [cut.a[0] - GUARD, cut.a[1] - GUARD, 0.],
                [cut.b[0] + GUARD, cut.b[1] + GUARD, 0.],
                0.,
                0.,
                cut.shape,
            );
        }
        // Raised by the guard against a flute top raised twice as far.
        crate::volume::swept(
            [q[0], q[1], q[2] + GUARD],
            cut.a,
            cut.b,
            cut.radius + GUARD,
            cut.flute + 2. * GUARD,
            cut.shape,
        )
    }
}
/// `Removed::by` over some of a volume's cuts, in the order they were made:
/// the operation whose cut first removed material at a point.
pub struct FirstCut<'a> {
    cuts: Vec<&'a Cut>,
    current: Frame,
    boxes: Vec<Bounds>,
}
impl<'a> FirstCut<'a> {
    pub fn new(cuts: Vec<&'a Cut>, current: Frame) -> Self {
        let boxes = cuts.iter().map(|c| removal_box(c, current)).collect();
        Self {
            cuts,
            current,
            boxes,
        }
    }
    pub fn by(&self, p: [f32; 3]) -> Option<(u16, u16)> {
        self.cuts
            .iter()
            .zip(&self.boxes)
            .find(|(cut, b)| removes(cut, b, self.current, p))
            .map(|(cut, _)| cut.source)
    }
}
/// Whether some column of the rectangle `lo..hi` (XY, mm) lies in none of
/// the rectangles `boxes`: Some(true) when one does, Some(false) when the
/// boxes cover the whole rectangle, None when `budget` runs out first. A
/// rectangle inside one box is covered; one whose centre lies in no box is
/// not; any other is split in two, along the axis it most exceeds a typical
/// box on (the median of the overlapping boxes' spans), down to 0.001 mm,
/// below which a sliver is taken as covered. Each box looked at costs a unit
/// of `budget`.
fn uncovered(
    lo: [f32; 2],
    hi: [f32; 2],
    boxes: &[([f32; 2], [f32; 2])],
    budget: &mut usize,
) -> Option<bool> {
    let mut overlapping: Vec<([f32; 2], [f32; 2])> = vec![];
    for &(a, b) in boxes {
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        if (0..2).all(|k| a[k] < hi[k] && b[k] > lo[k]) {
            overlapping.push((a, b));
        }
    }
    if overlapping.is_empty() {
        return Some(true);
    }
    if overlapping
        .iter()
        .any(|(a, b)| (0..2).all(|k| a[k] <= lo[k] && b[k] >= hi[k]))
    {
        return Some(false);
    }
    let centre = [(lo[0] + hi[0]) / 2., (lo[1] + hi[1]) / 2.];
    if !overlapping
        .iter()
        .any(|(a, b)| (0..2).all(|k| a[k] <= centre[k] && centre[k] <= b[k]))
    {
        return Some(true);
    }
    let span = |k: usize| {
        let mut spans: Vec<f32> = overlapping.iter().map(|(a, b)| b[k] - a[k]).collect();
        let mid = spans.len() / 2;
        *spans.select_nth_unstable_by(mid, f32::total_cmp).1
    };
    let ratio = |k: usize| (hi[k] - lo[k]) / span(k).max(0.01);
    let k = if ratio(0) >= ratio(1) { 0 } else { 1 };
    if hi[k] - lo[k] < 0.001 {
        return Some(false);
    }
    let (mut first_hi, mut second_lo) = (hi, lo);
    first_hi[k] = centre[k];
    second_lo[k] = centre[k];
    match uncovered(lo, first_hi, &overlapping, budget) {
        Some(false) => uncovered(second_lo, hi, &overlapping, budget),
        other => other,
    }
}
impl RestStock {
    pub fn new(volume: &Volume, current: Frame) -> Self {
        let cuts = volume
            .cuts
            .iter()
            // Constant-height flat sweeps are convex capsules with exact caps.
            // Other cutter shapes and ramps remain in the simulated stock but
            // are not used as evidence to omit cutting layers.
            .filter(|c| c.shape == CutterShape::Flat && c.a[2] == c.b[2])
            .map(|cut| {
                let b = Bounds {
                    min: [
                        cut.a[0].min(cut.b[0]) - cut.radius,
                        cut.a[1].min(cut.b[1]) - cut.radius,
                        cut.a[2],
                    ],
                    max: [
                        cut.a[0].max(cut.b[0]) + cut.radius,
                        cut.a[1].max(cut.b[1]) + cut.radius,
                        cut.a[2] + cut.flute,
                    ],
                };
                Capsule {
                    cut: cut.clone(),
                    bounds: transformed(b, cut.frame, current),
                }
            })
            .collect();
        let whole = volume.transformed_bounds(current);
        Self {
            bounds: volume.bounds,
            reference: volume.frame,
            current,
            extent: turned(volume, current).unwrap_or(whole),
            whole,
            cuts,
            removal: removal_boxes(volume, current),
            coaxial: coaxial(volume, current),
            all: volume.cuts.clone(),
        }
    }
    /// Whether earlier cuts have emptied the upright cylinder of `radius`
    /// round `centre` from `bottom` up to `top` (current frame, mm): a hole
    /// drilled or milled there, that a tool as wide can go down. Its axis
    /// and a ring of points round it, every quarter millimetre up, must
    /// each be inside some cut's sweep or outside the stock.
    pub fn bored(&self, centre: [f32; 2], radius: f32, top: f32, bottom: f32) -> bool {
        let near = Bounds {
            min: [centre[0] - radius, centre[1] - radius, bottom],
            max: [centre[0] + radius, centre[1] + radius, top],
        };
        let overlaps =
            |b: &Bounds| (0..3).all(|k| b.min[k] <= near.max[k] && b.max[k] >= near.min[k]);
        let cuts: Vec<&Cut> = self
            .all
            .iter()
            .filter(|c| overlaps(&removal_box(c, self.current)))
            .collect();
        let rings = 8;
        let levels = ((top - bottom) / 0.25).ceil().max(1.) as usize;
        (0..=levels).all(|l| {
            let z = bottom + (top - bottom) * l as f32 / levels as f32;
            (0..=rings).all(|k| {
                let p = if k == rings {
                    [centre[0], centre[1], z]
                } else {
                    let a = k as f32 * std::f32::consts::TAU / rings as f32;
                    [
                        centre[0] + radius * a.cos(),
                        centre[1] + radius * a.sin(),
                        z,
                    ]
                };
                !(0..3).all(|i| p[i] > self.whole.min[i] && p[i] < self.whole.max[i])
                    || cuts.iter().any(|c| {
                        crate::volume::swept(
                            transfer(p, self.current, c.frame),
                            c.a,
                            c.b,
                            c.radius,
                            c.flute,
                            c.shape,
                        )
                    })
            })
        })
    }
    /// What earlier lathe cuts left about the current frame's Z axis.
    pub fn coaxial(&self) -> &Coaxial {
        &self.coaxial
    }
    /// The box the stock left lies in, in the current frame, mm: outside it
    /// no stock is left (the stock's box, less what lathe cuts turned away).
    pub fn extent(&self) -> Bounds {
        self.extent
    }
    /// Outlines in XY (current frame, mm) of the earlier constant-height flat
    /// sweeps that reach `near` and whose cylinders span every height from
    /// `low` to `high`: between those heights the material inside each is
    /// gone. A sweep counts in any setup's frame where its tool axis is
    /// vertical, pointing up or down. Each outline is a capsule with its
    /// vertices on the cylinder, its chords at most `CHORD` inside it, so the
    /// outlines never claim more than was cut.
    pub fn cleared(&self, near: Bounds, low: f32, high: f32) -> Vec<Vec<(f64, f64)>> {
        (self.sweeps(near, low, high, |_| true).into_iter())
            .map(|(a, e, r)| outline(a, e, r))
            .collect()
    }
    /// The sweeps `cleared` outlines, of only those whose boxes `keep`
    /// takes: each its ends and radius in the current frame.
    fn sweeps(
        &self,
        near: Bounds,
        low: f32,
        high: f32,
        keep: impl Fn(Bounds) -> bool,
    ) -> Vec<([f32; 3], [f32; 3], f32)> {
        let mut out = vec![];
        for capsule in &self.cuts {
            let b = capsule.bounds;
            if (0..2).any(|k| b.max[k] < near.min[k] || b.min[k] > near.max[k])
                || b.min[2] > low + 0.001
                || b.max[2] < high - 0.001
                || !keep(b)
            {
                continue;
            }
            let cut = &capsule.cut;
            let a = transfer(cut.a, cut.frame, self.current);
            let e = transfer(cut.b, cut.frame, self.current);
            let up = transfer(
                [cut.a[0], cut.a[1], cut.a[2] + cut.flute],
                cut.frame,
                self.current,
            );
            // Only a vertical axis sweeps a capsule in this frame's XY.
            if (up[0] - a[0]).abs() > 0.001
                || (up[1] - a[1]).abs() > 0.001
                || (e[2] - a[2]).abs() > 0.001
            {
                continue;
            }
            if cut.radius > CHORD {
                out.push((a, e, cut.radius));
            }
        }
        out
    }
    /// Whether some point of `reach`, within the stock's box, lies clear of
    /// all of `sweeps` (as `RestStock::sweeps` gives them): material left
    /// there, which the clipping in `swept` would find too. Tried along the
    /// reach's edges, where a larger tool leaves the corners, then over a
    /// grid.
    fn uncut(&self, reach: &Reach, sweeps: &[([f32; 3], [f32; 3], f32)]) -> bool {
        let e = self.extent;
        let (lo, hi) = (reach.bounds.min, reach.bounds.max);
        let size = [(hi[0] - lo[0]) as f64, (hi[1] - lo[1]) as f64];
        // The sweeps by cell of a 32 × 32 grid over the reach's box.
        const N: usize = 32;
        let cell = [size[0] / N as f64, size[1] / N as f64];
        let at =
            |v: f64, k: usize| (((v - lo[k] as f64) / cell[k]).floor().max(0.) as usize).min(N - 1);
        let mut cells = vec![vec![]; N * N];
        for (i, (a, b, r)) in sweeps.iter().enumerate() {
            let r = *r as f64 + CLEAR;
            let span = |k: usize| {
                (
                    at(a[k].min(b[k]) as f64 - r, k),
                    at(a[k].max(b[k]) as f64 + r, k),
                )
            };
            let ((x0, x1), (y0, y1)) = (span(0), span(1));
            for y in y0..=y1 {
                for x in x0..=x1 {
                    cells[y * N + x].push(i);
                }
            }
        }
        let clear = |p: (f64, f64)| {
            (0..2).all(|k| {
                let v = if k == 0 { p.0 } else { p.1 };
                v >= e.min[k] as f64 + CLEAR && v <= e.max[k] as f64 - CLEAR
            }) && cells[at(p.1, 1) * N + at(p.0, 0)].iter().all(|&i| {
                let (a, b, r) = sweeps[i];
                let d = [b[0] as f64 - a[0] as f64, b[1] as f64 - a[1] as f64];
                let v = [p.0 - a[0] as f64, p.1 - a[1] as f64];
                let length = d[0] * d[0] + d[1] * d[1];
                let t = if length == 0. {
                    0.
                } else {
                    ((v[0] * d[0] + v[1] * d[1]) / length).clamp(0., 1.)
                };
                (v[0] - t * d[0]).hypot(v[1] - t * d[1]) > r as f64 + CLEAR
            }) && reach.holds_by(p, CLEAR)
        };
        let along = reach.edges.iter().flat_map(|&[a, b]| {
            let d = (b.0 - a.0, b.1 - a.1);
            let length = d.0.hypot(d.1);
            let (u, n) = if length > 0. {
                ((d.0 / length, d.1 / length), (-d.1 / length, d.0 / length))
            } else {
                ((0., 0.), (0., 0.))
            };
            let end = (length / 2.).min(0.05);
            [end, length / 2., length - end]
                .into_iter()
                .flat_map(move |s| {
                    [1., -1.].map(|side| {
                        (
                            a.0 + u.0 * s + side * 2. * CLEAR * n.0,
                            a.1 + u.1 * s + side * 2. * CLEAR * n.1,
                        )
                    })
                })
        });
        let step = (size[0] * size[1] / 4096.).sqrt().max(0.25);
        let (nx, ny) = ((size[0] / step) as usize + 1, (size[1] / step) as usize + 1);
        let grid = (0..ny).flat_map(|y| {
            (0..nx).map(move |x| {
                (
                    lo[0] as f64 + (x as f64 + 0.5) * step,
                    lo[1] as f64 + (y as f64 + 0.5) * step,
                )
            })
        });
        along.chain(grid).any(clear)
    }
    /// Whether `region` lies wholly beside the stock box, where no stock is left.
    pub fn beside(&self, region: Bounds) -> bool {
        (0..2).any(|k| region.max[k] <= self.extent.min[k] || region.min[k] >= self.extent.max[k])
    }
    /// Upper bound on remaining material within an operation's XY envelope.
    /// Uncertain coverage, thin remnants, or exhausted work budgets keep the
    /// higher starting height. Clearance/retract planes are independent.
    pub fn top(&self, region: Bounds, target: f32, original: f32) -> f32 {
        self.cap(region, None, target, original)
    }
    /// `top` over only the area the cutter reaches: material beside it, as
    /// in the corners of its box, does not hold the start up.
    pub fn top_over(&self, reach: &Reach, target: f32, original: f32) -> f32 {
        self.cap(reach.bounds, Some(reach), target, original)
    }
    /// `top` within `region`, and within `reach` where there is one.
    fn cap(&self, mut region: Bounds, reach: Option<&Reach>, target: f32, original: f32) -> f32 {
        if self.cuts.is_empty()
            || original <= target
            || !original.is_finite()
            || !target.is_finite()
        {
            return original;
        }
        for k in 0..2 {
            region.min[k] = region.min[k].max(self.extent.min[k]);
            region.max[k] = region.max[k].min(self.extent.max[k]);
            if !region.min[k].is_finite()
                || !region.max[k].is_finite()
                || region.min[k] >= region.max[k]
            {
                return original;
            }
        }
        region.max[2] = self.extent.max[2].min(original);
        let mut candidates: Vec<f32> = self
            .cuts
            .iter()
            .flat_map(|c| [c.bounds.min[2], c.bounds.max[2]])
            .filter(|z| z.is_finite() && *z > target && *z < original)
            .collect();
        candidates.extend([target, original]);
        candidates.sort_by(f32::total_cmp);
        candidates.dedup();
        // Only cuts that reach the region can cover any of it: the others
        // would spend the work budget of a job with many cuts for nothing.
        let ids: Vec<_> = (0..self.cuts.len())
            .filter(|&i| {
                let c = self.cuts[i].bounds;
                (0..2).all(|k| c.max[k] >= region.min[k] && c.min[k] <= region.max[k])
                    && c.max[2] >= target
                    && c.min[2] <= region.max[2]
            })
            .collect();
        let (mut lo, mut hi) = (0, candidates.len() - 1);
        let mut budget = BUDGET;
        while lo < hi && budget > 0 {
            let mid = lo + (hi - lo) / 2;
            region.min[2] = candidates[mid];
            if self.covered(region, &ids, reach, 0, &mut budget) {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        // Below that, with a reach, the same in the plane: the reach less
        // the outlines of the sweeps through every height of the cap. It
        // sees a reach that passes a hair from material beside it, as a
        // contour does round the part, which boxes cannot follow.
        if let Some(reach) = reach {
            let (mut lo, mut budget) = (0, BUDGET);
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                match self.swept(reach, candidates[mid], region.max[2], &mut budget) {
                    Some(true) => hi = mid,
                    Some(false) => lo = mid + 1,
                    // Out of work: the start stays as proven so far.
                    None => break,
                }
            }
        }
        // hi always identifies a proven cap (or the original height).
        candidates[hi]
    }
    /// Whether the outlines of the earlier sweeps through every height from
    /// `low` to `high` (`cleared`) cover all of `reach` within the stock's
    /// box. Their chords lie inside the sweeps, so a reach whose edge
    /// follows a sweep's exactly is not covered. Each outline costs a unit
    /// of `budget`; None when there are more than it has left.
    fn swept(&self, reach: &Reach, low: f32, high: f32, budget: &mut usize) -> Option<bool> {
        // A sweep whose box lies wholly off the reach, as a pocket's inside
        // a contour round the part does, takes none of it: left out, it
        // spares Clipper the crossings of thousands of outlines.
        let sweeps = self.sweeps(reach.bounds, low, high, |b| !reach.misses(b));
        *budget = budget.checked_sub(sweeps.len())?;
        // A point the sweeps missed answers without clipping them all, as
        // round a smaller tool's zone, whose corners a larger one left.
        if sweeps.is_empty() || self.uncut(reach, &sweeps) {
            return Some(false);
        }
        let gone: Vec<_> = (sweeps.into_iter())
            .map(|(a, e, r)| outline(a, e, r))
            .collect();
        let e = self.extent;
        let inside = vec![vec![
            (e.min[0] as f64, e.min[1] as f64),
            (e.max[0] as f64, e.min[1] as f64),
            (e.max[0] as f64, e.max[1] as f64),
            (e.min[0] as f64, e.max[1] as f64),
        ]];
        let area = intersect::<Milli>(reach.loops.clone(), inside, FillRule::NonZero);
        let left = area.and_then(|a| difference::<Milli>(a, gone, FillRule::NonZero));
        Some(left.is_ok_and(|l| !l.contains_points()))
    }
    /// The stock box a later setup starts from, in the current frame (mm):
    /// the original block's box, narrowed to what lathe cuts left (`turned`),
    /// with each of its six sides moved in past
    /// every layer earlier cuts provably removed whole (the same evidence and
    /// proof as `top`). Sides are trimmed in a fixed order (top, bottom, -X,
    /// +X, -Y, +Y), each against the box trimmed so far. Without proof a side
    /// stays where it was, and when the work budget runs out it stops at the
    /// deepest layer proven so far; it never moves into `part`, the part's
    /// bounds in the current frame.
    pub fn left(&self, part: Bounds) -> Bounds {
        self.trimmed(part, BUDGET)
    }
    /// `left` with `budget` units of `covered` work for each side.
    fn trimmed(&self, part: Bounds, budget: usize) -> Bounds {
        const SIX: [(usize, bool); 6] = [
            (2, true),
            (2, false),
            (0, false),
            (0, true),
            (1, false),
            (1, true),
        ];
        // Lathe cuts never take the box into the part (a cut that did is
        // refused): it reaches out to it, within the stock's box.
        let start = Bounds {
            min: std::array::from_fn(|k| {
                self.extent.min[k].min(part.min[k]).max(self.whole.min[k])
            }),
            max: std::array::from_fn(|k| {
                self.extent.max[k].max(part.max[k]).min(self.whole.max[k])
            }),
        };
        self.trim(start, &SIX, part, budget, None)
    }
    /// What a vise's jaws close on in a later setup that starts from `stock`
    /// (current frame, mm) and grips `grip` mm of it, the jaws closing along
    /// axis `across` (0 for X, 1 for Y): `stock` with its sides moved in past
    /// every layer of the band from its bottom up to the grip that earlier
    /// cuts provably removed whole, the evidence and proof of `left` with
    /// each slab reaching only over the band's heights. First the two sides
    /// along the jaws, each slab as long as the box; then the two the jaws
    /// close on, each slab only as long as the jaws' reach along them (`jaws`:
    /// the jaws' offset from the middle of that box along them and their
    /// width), where they can touch: material left beyond the jaws' ends,
    /// such as the corners a small cutter cannot reach, does not hold them
    /// open. Never into `part`; its heights are `stock`'s. A side stays where
    /// nothing is proven, so material the proof cannot see keeps the band as
    /// wide as it was.
    pub fn band(
        &self,
        part: Bounds,
        stock: Bounds,
        grip: f32,
        across: usize,
        jaws: (f32, f32),
    ) -> Bounds {
        self.band_within(part, stock, grip, across, jaws, BUDGET)
    }
    /// `band` with `budget` units of `covered` work for each side.
    fn band_within(
        &self,
        part: Bounds,
        stock: Bounds,
        grip: f32,
        across: usize,
        (offset, width): (f32, f32),
        budget: usize,
    ) -> Bounds {
        let along = 1 - across;
        let top = (stock.min[2] + grip).min(stock.max[2]);
        // Nothing gripped, or a grip that is not a number.
        if top.is_nan() || top <= stock.min[2] {
            return stock;
        }
        let mut slab = stock;
        slab.max[2] = top;
        let sides = |k| [(k, false), (k, true)];
        let mut band = self.trim(slab, &sides(along), part, budget, Some(BAND_SLACK));
        let middle = (band.min[along] + band.max[along]) / 2. + offset;
        let mut reach = band;
        reach.min[along] = band.min[along].max(middle - width / 2.);
        reach.max[along] = band.max[along].min(middle + width / 2.);
        if reach.min[along] < reach.max[along] {
            let closed = self.trim(reach, &sides(across), part, budget, Some(BAND_SLACK));
            (band.min[across], band.max[across]) = (closed.min[across], closed.max[across]);
        }
        (band.min[2], band.max[2]) = (stock.min[2], stock.max[2]);
        band
    }
    /// The highest the bottom of the material under `footprint` (current
    /// frame, mm; its heights the stock's) can lie, which a vise's grip is
    /// measured up from when the stock stands on that material: the
    /// footprint's bottom, raised past the layer earlier cuts of any cutter
    /// shape can have removed from it. The box a later setup starts from
    /// (`left`) keeps a layer that was cut but not provably removed whole, a
    /// ball-cut or ramped facing for one, so the material's bottom can lie
    /// above the box's; it never lies below it. Each column of the footprint
    /// can have lost its material from the bottom up to where the cuts'
    /// boxes (`removal_boxes`) stacked on it without a break stop, and the
    /// stock stands on the lowest material left: the least of those heights
    /// over the footprint. Found exactly, as the lowest of the heights a
    /// stack can stop at (the bottom, then the boxes' tops) at which some
    /// column lies in no box reaching past it, each height proven by
    /// splitting the footprint, with `BUDGET` units of work in all; when they
    /// run out, the least over the columns at 8 × 8 fixed points instead,
    /// which is never below the true least. Never above the footprint's top.
    pub fn bottom(&self, footprint: Bounds) -> f32 {
        self.bottom_within(footprint, BUDGET)
    }
    /// `bottom` with `budget` units of work for the proof.
    fn bottom_within(&self, footprint: Bounds, budget: usize) -> f32 {
        const GUARD: f32 = 0.001;
        let (floor, ceiling) = (footprint.min[2], footprint.max[2]);
        if !floor.is_finite() || !ceiling.is_finite() || floor >= ceiling {
            return floor;
        }
        // The boxes over the footprint that reach more than a guard above its
        // bottom: `left` keeps a face a hair under a proven cut's.
        let boxes: Vec<Bounds> = self
            .removal
            .iter()
            .filter(|b| {
                (0..2).all(|k| b.min[k] < footprint.max[k] && b.max[k] > footprint.min[k])
                    && b.max[2] > floor + GUARD
                    && b.min[2] <= ceiling
            })
            .copied()
            .collect();
        if boxes.is_empty() {
            return floor;
        }
        let xy = |b: &Bounds| ([b.min[0], b.min[1]], [b.max[0], b.max[1]]);
        let (lo, hi) = xy(&footprint);
        // A column's stack of boxes passes `level` in a box reaching past it
        // from no more than a guard above it, which bridges the float error
        // between stacked boxes.
        let through = |level: f32| -> Vec<([f32; 2], [f32; 2])> {
            boxes
                .iter()
                .filter(|b| b.min[2] <= level + GUARD && b.max[2] > level)
                .map(xy)
                .collect()
        };
        // The heights a stack can stop at, lowest first: the bottom, then the
        // boxes' tops.
        let mut levels: Vec<f32> = boxes
            .iter()
            .map(|b| b.max[2])
            .filter(|z| *z > floor && *z < ceiling)
            .collect();
        levels.sort_by(f32::total_cmp);
        levels.dedup();
        levels.insert(0, floor);
        let mut budget = budget;
        let mut proven = true;
        for &level in &levels {
            match uncovered(lo, hi, &through(level), &mut budget) {
                Some(true) => return level,
                Some(false) => {}
                None => {
                    proven = false;
                    break;
                }
            }
        }
        if proven {
            // Every column's stack passes every height below the top.
            return ceiling;
        }
        let mut least = ceiling;
        for i in 0..8 {
            for j in 0..8 {
                let x = lo[0] + (hi[0] - lo[0]) * (i as f32 + 0.5) / 8.;
                let y = lo[1] + (hi[1] - lo[1]) * (j as f32 + 0.5) / 8.;
                let mut z = floor;
                while z < ceiling {
                    let next = boxes
                        .iter()
                        .filter(|b| {
                            b.min[0] <= x
                                && x <= b.max[0]
                                && b.min[1] <= y
                                && y <= b.max[1]
                                && b.min[2] <= z + GUARD
                                && b.max[2] > z
                        })
                        .map(|b| b.max[2])
                        .reduce(f32::max);
                    match next {
                        Some(top) => z = top,
                        None => break,
                    }
                }
                least = least.min(z);
            }
        }
        least.min(ceiling)
    }
    /// `start` (current frame, mm) with each of `sides` (axis, and whether
    /// its high side), in order, moved in past every layer earlier cuts
    /// provably removed whole, each slab tested against the box trimmed so
    /// far: `budget` units of `covered` work for each side. Never into
    /// `part`. The candidate depths are the capsules' faces and the part's;
    /// with a `slack`, each capsule face is taken that many mm short of it,
    /// toward the side, instead of on it and a hair short of it.
    fn trim(
        &self,
        start: Bounds,
        sides: &[(usize, bool)],
        part: Bounds,
        budget: usize,
        slack: Option<f32>,
    ) -> Bounds {
        let mut b = start;
        if self.cuts.is_empty() {
            return b;
        }
        // Across frames `contains` refuses a point on a capsule's face by a
        // guard of 16 ulps of its largest coordinate there, so each face also
        // offers a candidate this much short of it, toward the side.
        let mut frames = vec![self.current];
        for c in &self.cuts {
            if !frames.contains(&c.cut.frame) {
                frames.push(c.cut.frame);
            }
        }
        let reach = frames
            .iter()
            .map(|&f| transformed(self.extent, self.current, f))
            .flat_map(|e| e.min.into_iter().chain(e.max))
            .map(f32::abs)
            .fold(1., f32::max);
        let margin = reach * f32::EPSILON * 64.;
        // The evidence: each earlier operation's capsules alone, smallest
        // first, then all of them together. A layer is usually removed whole
        // by one operation (a facing, a contour), whose proof costs far less
        // than one that wades through every other operation's long flutes.
        let mut groups: Vec<((u16, u16), Vec<usize>)> = vec![];
        for (id, c) in self.cuts.iter().enumerate() {
            match groups.iter_mut().find(|(s, _)| *s == c.cut.source) {
                Some((_, ids)) => ids.push(id),
                None => groups.push((c.cut.source, vec![id])),
            }
        }
        let mut groups: Vec<Vec<usize>> = groups.into_iter().map(|(_, ids)| ids).collect();
        groups.sort_by_key(Vec::len);
        if groups.len() > 1 {
            groups.push((0..self.cuts.len()).collect());
        }
        for &(k, high) in sides {
            let side = if high { b.max[k] } else { b.min[k] };
            let toward = if high { margin } else { -margin };
            // How far a candidate face lies inside the box from this side.
            let depth = |v: f32| if high { side - v } else { v - side };
            // The side never moves into the part: no slab past its face is
            // worth proving, and the face itself is a candidate.
            let face = if high { part.max[k] } else { part.min[k] };
            let short = |v: f32| match slack {
                Some(s) => [v + if high { s } else { -s }; 2],
                None => [v, v + toward],
            };
            let mut candidates: Vec<f32> = self
                .cuts
                .iter()
                .flat_map(|c| [c.bounds.min[k], c.bounds.max[k]])
                .flat_map(short)
                .chain([face])
                .filter(|v| {
                    v.is_finite() && *v > b.min[k] && *v < b.max[k] && depth(*v) <= depth(face)
                })
                .collect();
            // Deepest first; the side itself last, never tested.
            candidates.sort_by(|x, y| depth(*y).total_cmp(&depth(*x)));
            candidates.dedup();
            candidates.push(side);
            let slab = |v: f32| {
                let mut s = b;
                if high {
                    s.min[k] = v;
                } else {
                    s.max[k] = v;
                }
                s
            };
            // Each group searches the candidates deeper than the deepest
            // proven so far; the budget is shared by the side's groups.
            let mut hi = candidates.len() - 1;
            let mut budget = budget;
            for ids in &groups {
                // Slabs nest: when the thinnest is not covered, no deeper one is.
                if hi == 0
                    || budget == 0
                    || !self.covered(slab(candidates[hi - 1]), ids, None, 0, &mut budget)
                {
                    continue;
                }
                hi -= 1;
                let mut lo = 0;
                while lo < hi && budget > 0 {
                    let mid = lo + (hi - lo) / 2;
                    if self.covered(slab(candidates[mid]), ids, None, 0, &mut budget) {
                        hi = mid;
                    } else {
                        lo = mid + 1;
                    }
                }
            }
            // hi always identifies a proven slab (or the side itself); a
            // thinner slab, stopping at the part, is proven with it.
            let v = candidates[hi];
            if high {
                b.max[k] = v.max(part.max[k]).min(b.max[k]);
            } else {
                b.min[k] = v.min(part.min[k]).max(b.min[k]);
            }
        }
        b
    }
    /// Whether earlier cuts provably removed all the material in `b`, or,
    /// with a `reach`, all of it within the reach: a box wholly beside the
    /// reach holds nothing that counts.
    fn covered(
        &self,
        b: Bounds,
        ids: &[usize],
        reach: Option<&Reach>,
        depth: u8,
        budget: &mut usize,
    ) -> bool {
        if b.min[2] > b.max[2] {
            return true;
        }
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        if reach.is_some_and(|r| r.misses(b)) {
            return true;
        }
        let points = corners(b);
        let reference = points.map(|p| transfer(p, self.current, self.reference));
        let guard = if self.current == self.reference {
            0.
        } else {
            reference
                .iter()
                .flatten()
                .copied()
                .map(f32::abs)
                .fold(1., f32::max)
                * f32::EPSILON
                * 16.
        };
        // A separating stock-box plane proves the entire box is outside stock.
        if (0..3).any(|k| {
            reference.iter().all(|p| p[k] < self.bounds.min[k] - guard)
                || reference.iter().all(|p| p[k] > self.bounds.max[k] + guard)
        }) {
            return true;
        }
        let mut overlap = Vec::new();
        let mut spans: [Vec<f32>; 3] = Default::default();
        for &id in ids {
            if *budget == 0 {
                return false;
            }
            *budget -= 1;
            let c = &self.cuts[id];
            if (0..3).any(|k| b.min[k] > c.bounds.max[k] || b.max[k] < c.bounds.min[k]) {
                continue;
            }
            overlap.push(id);
            for (k, spans) in spans.iter_mut().enumerate() {
                spans.push(c.bounds.max[k] - c.bounds.min[k]);
            }
            if (0..3).all(|k| b.min[k] >= c.bounds.min[k] && b.max[k] <= c.bounds.max[k])
                && points.iter().all(|&p| self.contains(c, p))
            {
                return true;
            }
        }
        if overlap.is_empty() || depth >= 24 {
            return false;
        }
        // A box whose centre lies inside the stock (and the reach) but in no
        // capsule cannot be proven: the part of it round its centre would
        // have to lie inside one. Said before splitting it, so that a slab reaching into
        // material is given up at the first such box, not after proving all
        // the rest of it.
        let centre: [f32; 3] = std::array::from_fn(|k| (b.min[k] + b.max[k]) / 2.);
        let at = transfer(centre, self.current, self.reference);
        let in_stock = (0..3)
            .all(|k| at[k] > self.bounds.min[k] + guard && at[k] < self.bounds.max[k] - guard);
        if in_stock
            && reach.is_none_or(|r| r.holds(centre))
            && !overlap
                .iter()
                .any(|&id| self.contains(&self.cuts[id], centre))
        {
            return false;
        }
        // Split where the query exceeds a typical sweep's extent: the median
        // of the overlapping sweeps', so that a few long strokes do not hide
        // the short ones round a corner. A long facing stroke usually needs
        // subdivision across its rows, not along X; a slab no taller than
        // the flutes, none along Z.
        let span: [f32; 3] = std::array::from_fn(|k| {
            let mid = spans[k].len() / 2;
            *spans[k].select_nth_unstable_by(mid, f32::total_cmp).1
        });
        let k = (0..3)
            .max_by(|&a, &b_axis| {
                ((b.max[a] - b.min[a]) / span[a].max(0.01))
                    .total_cmp(&((b.max[b_axis] - b.min[b_axis]) / span[b_axis].max(0.01)))
            })
            .unwrap();
        if b.max[k] - b.min[k] < 0.001 {
            return false;
        }
        let mid = b.min[k] + (b.max[k] - b.min[k]) / 2.;
        if mid <= b.min[k] || mid >= b.max[k] {
            return false;
        }
        let (mut left, mut right) = (b, b);
        left.max[k] = mid;
        right.min[k] = mid;
        self.covered(left, &overlap, reach, depth + 1, budget)
            && self.covered(right, &overlap, reach, depth + 1, budget)
    }
    fn contains(&self, capsule: &Capsule, p: [f32; 3]) -> bool {
        let c = &capsule.cut;
        let q = transfer(p, self.current, c.frame);
        // Double precision comparisons, no outward membership tolerance. A
        // guard on transformed coordinates makes uncertain boundaries remain.
        let guard = if self.current == c.frame {
            0.
        } else {
            q.into_iter().map(f32::abs).fold(1., f32::max) as f64 * f32::EPSILON as f64 * 16.
        };
        if q[2] as f64 - guard < c.a[2] as f64
            || q[2] as f64 + guard > c.a[2] as f64 + c.flute as f64
        {
            return false;
        }
        let d = [c.b[0] as f64 - c.a[0] as f64, c.b[1] as f64 - c.a[1] as f64];
        let v = [q[0] as f64 - c.a[0] as f64, q[1] as f64 - c.a[1] as f64];
        let length = d[0] * d[0] + d[1] * d[1];
        let t = if length == 0. {
            0.
        } else {
            ((v[0] * d[0] + v[1] * d[1]) / length).clamp(0., 1.)
        };
        let r = c.radius as f64 - guard;
        r > 0. && (v[0] - t * d[0]).powi(2) + (v[1] - t * d[1]).powi(2) <= r * r
    }
}

pub fn start(setup: &Setup, region: Bounds, target: f32, original: f32, offset: f32) -> f32 {
    setup.preview_remaining.as_ref().map_or(original, |rest| {
        (rest.top(region, target, original) + offset.max(0.))
            .min(original)
            .max(target)
    })
}
/// `start` with clearance proven over only the area the cutter reaches
/// (`Reach`), not over its box: stock left beside that area, as in the
/// corners of an irregular pocket's box, does not keep the start up.
pub fn start_over(setup: &Setup, reach: &Reach, target: f32, original: f32, offset: f32) -> f32 {
    setup.preview_remaining.as_ref().map_or(original, |rest| {
        (rest.top_over(reach, target, original) + offset.max(0.))
            .min(original)
            .max(target)
    })
}

/// A sweep's outline in XY from `a` to `e` of radius `r`: a capsule with
/// its vertices on the cylinder, its chords at most `CHORD` inside it.
fn outline(a: [f32; 3], e: [f32; 3], r: f32) -> Vec<(f64, f64)> {
    let steps = (std::f32::consts::PI / (1. - CHORD / r).acos())
        .ceil()
        .max(8.) as usize;
    let heading = (e[1] - a[1]).atan2(e[0] - a[0]);
    let mut outline = Vec::with_capacity(2 * steps + 2);
    for (centre, from) in [
        (e, heading - std::f32::consts::FRAC_PI_2),
        (a, heading + std::f32::consts::FRAC_PI_2),
    ] {
        for i in 0..=steps {
            let angle = from + std::f32::consts::PI * i as f32 / steps as f32;
            outline.push((
                (centre[0] + r * angle.cos()) as f64,
                (centre[1] + r * angle.sin()) as f64,
            ));
        }
    }
    outline
}

/// The area in XY (current frame, mm) a cutter reaches over all its moves:
/// closed loops, filled by the nonzero rule as Clipper's are. A cutting
/// start found over it (`start_over`) sees only the material the cutter
/// can meet.
#[derive(Debug, Clone)]
pub struct Reach {
    loops: Vec<Vec<(f64, f64)>>,
    edges: Vec<[(f64, f64); 2]>,
    /// The loops' box; its heights unbounded.
    bounds: Bounds,
    /// The edges by band of Y, `band` mm each from `bounds.min[1]`, each
    /// edge in every band its Y range meets.
    bands: Vec<Vec<usize>>,
    band: f64,
}
impl Reach {
    pub fn new(loops: &[Vec<(f64, f64)>]) -> Self {
        let edges: Vec<[(f64, f64); 2]> = loops
            .iter()
            .filter(|l| l.len() > 2)
            .flat_map(|l| {
                l.iter()
                    .zip(l.iter().cycle().skip(1))
                    .map(|(a, b)| [*a, *b])
            })
            .collect();
        let mut bounds = Bounds {
            min: [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY],
            max: [f32::NEG_INFINITY, f32::NEG_INFINITY, f32::INFINITY],
        };
        // A micrometre out, past any rounding to single precision.
        for p in edges.iter().map(|e| e[0]) {
            for (k, v) in [p.0, p.1].into_iter().enumerate() {
                bounds.min[k] = bounds.min[k].min(v as f32 - 0.001);
                bounds.max[k] = bounds.max[k].max(v as f32 + 0.001);
            }
        }
        // About eight edges a band.
        let count = (edges.len() / 8).clamp(1, 1024);
        let band = ((bounds.max[1] - bounds.min[1]) as f64 / count as f64).max(1e-6);
        let mut reach = Self {
            loops: loops.iter().filter(|l| l.len() > 2).cloned().collect(),
            edges,
            bounds,
            bands: vec![vec![]; count],
            band,
        };
        for (i, e) in reach.edges.iter().enumerate() {
            let (lo, hi) = (e[0].1.min(e[1].1), e[0].1.max(e[1].1));
            for b in reach.at(lo)..=reach.at(hi) {
                reach.bands[b].push(i);
            }
        }
        reach
    }
    /// A disc of `radius` about `centre`, within a polygon round it.
    pub fn disc(centre: [f32; 2], radius: f32) -> Self {
        const SIDES: usize = 32;
        // Out to the polygon's corners, so its sides are on the circle or
        // beyond it.
        let r = radius as f64 / (std::f64::consts::PI / SIDES as f64).cos();
        Self::new(&[(0..SIDES)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / SIDES as f64;
                (
                    centre[0] as f64 + r * a.cos(),
                    centre[1] as f64 + r * a.sin(),
                )
            })
            .collect()])
    }
    /// The band holding height `y`.
    fn at(&self, y: f64) -> usize {
        (((y - self.bounds.min[1] as f64) / self.band)
            .floor()
            .max(0.) as usize)
            .min(self.bands.len() - 1)
    }
    /// Whether `p` lies in the area.
    fn holds(&self, p: [f32; 3]) -> bool {
        let (x, y) = (p[0] as f64, p[1] as f64);
        if self.edges.is_empty() {
            return false;
        }
        let mut winding = 0;
        for e in self.bands[self.at(y)].iter().map(|&i| self.edges[i]) {
            let [a, b] = e;
            let left = (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0);
            if a.1 <= y && y < b.1 && left > 0. {
                winding += 1;
            } else if b.1 <= y && y < a.1 && left < 0. {
                winding -= 1;
            }
        }
        winding != 0
    }
    /// Whether `p` lies in the area, more than `d` from its every edge.
    fn holds_by(&self, p: (f64, f64), d: f64) -> bool {
        let near = |e: &[(f64, f64); 2]| {
            let [a, b] = *e;
            let v = (b.0 - a.0, b.1 - a.1);
            let w = (p.0 - a.0, p.1 - a.1);
            let length = v.0 * v.0 + v.1 * v.1;
            let t = if length == 0. {
                0.
            } else {
                ((w.0 * v.0 + w.1 * v.1) / length).clamp(0., 1.)
            };
            (w.0 - t * v.0).hypot(w.1 - t * v.1) <= d
        };
        !self.edges.is_empty()
            && !(self.at(p.1 - d)..=self.at(p.1 + d))
                .any(|band| self.bands[band].iter().any(|&i| near(&self.edges[i])))
            && self.holds([p.0 as f32, p.1 as f32, 0.])
    }
    /// Whether `b`'s XY lies wholly outside the area: no edge meets it and
    /// its centre is outside.
    fn misses(&self, b: Bounds) -> bool {
        if (0..2).any(|k| b.max[k] < self.bounds.min[k] || b.min[k] > self.bounds.max[k]) {
            return true;
        }
        let (x0, x1) = (b.min[0] as f64, b.max[0] as f64);
        let (y0, y1) = (b.min[1] as f64, b.max[1] as f64);
        for band in self.at(y0)..=self.at(y1) {
            for &[a, e] in self.bands[band].iter().map(|&i| &self.edges[i]) {
                if a.0.max(e.0) < x0 || a.0.min(e.0) > x1 || a.1.max(e.1) < y0 || a.1.min(e.1) > y1
                {
                    continue;
                }
                // Within the box's span, an edge whose line has corners on
                // both sides of it, or on it, meets the box.
                let side = |x: f64, y: f64| (e.0 - a.0) * (y - a.1) - (e.1 - a.1) * (x - a.0);
                let sides = [side(x0, y0), side(x1, y0), side(x1, y1), side(x0, y1)];
                if !(sides.iter().all(|s| *s > 0.) || sides.iter().all(|s| *s < 0.)) {
                    return false;
                }
            }
        }
        !self.holds(std::array::from_fn(|k| (b.min[k] + b.max[k]) / 2.))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mesh, model::Machine, plan_check::PartGrid};
    fn fixture() -> (Volume, Frame, Bounds, Setup, crate::mesh::Mesh) {
        let mesh = mesh::normalize(vec![[[0., 0., 0.], [20., 0., 0.], [0., 20., 10.]]]).unwrap();
        let setup = Setup::new(Machine::new(3), "G54");
        let frame = Frame::new(&setup, &mesh);
        let bounds = Bounds {
            min: [-10., -10., 0.],
            max: [10., 10., 12.],
        };
        (Volume::new(bounds, frame), frame, bounds, setup, mesh)
    }
    fn face(v: &mut Volume, f: Frame, z: f32, ys: &[f32], flute: f32) {
        for &y in ys {
            v.cut(
                [-15., y, z],
                [15., y, z],
                2.5,
                flute,
                CutterShape::Flat,
                f,
                (0, 0),
            );
        }
    }
    #[test]
    fn a_mill_setup_turned_square_to_a_lathe_starts_from_the_turned_bar() {
        // A 10 mm cube for the part, well inside the Ø40 the bar is turned to.
        let mesh = mesh::normalize(vec![[[-5., -5., 0.], [5., 5., 10.], [5., -5., 0.]]]).unwrap();
        let mut lathe = Setup::new(Machine::new(3), "G54");
        for (k, v) in [
            ("stock_type", "Round bar"),
            ("stock_diameter", "50"),
            ("stock_length", "100"),
        ] {
            lathe.p.insert(k.into(), v.into());
        }
        let stock = crate::stock::resolve(&lathe, Some(&mesh)).unwrap().stock;
        let a = Frame::new(&lathe, &mesh);
        // The mill holds the bar on its side: the lathe's Z is its X.
        let mut mill = Setup::new(Machine::new(3), "G55");
        mill.p.insert("ry".into(), "90".into());
        let b = Frame::new(&mill, &mesh);
        let close = |x: Bounds, y: Bounds| {
            for k in 0..3 {
                assert!(
                    (x.min[k] - y.min[k]).abs() < 0.001 && (x.max[k] - y.max[k]).abs() < 0.001,
                    "{x:?} != {y:?}"
                );
            }
        };
        let in_mill = |b0: Bounds| a.transfer_bounds(b0, b);
        let mut v = Volume::bar(stock, a);
        let whole = crate::simulation::leftover_box(&v, &mill, &mesh);
        close(whole, in_mill(stock));
        let d = whole.dimensions();
        assert!(
            (d[0] - 100.).abs() < 0.001 && (d[1] - 50.).abs() < 0.001,
            "{d:?}"
        );
        // Turned over 60 mm from the free end only: the chuck end keeps the
        // bar's diameter, so the box does not narrow.
        let mut part_way = v.clone();
        part_way.revolve(40., 101., 20., 25., a, (0, 0));
        close(
            crate::simulation::leftover_box(&part_way, &mill, &mesh),
            whole,
        );
        // Turned to Ø40 over the whole length in two passes, and faced 2 mm.
        v.revolve(-1., 101., 22.5, 25., a, (0, 0));
        v.revolve(-1., 101., 20., 22.5, a, (0, 1));
        v.revolve(98., 101., 0., 20., a, (0, 2));
        let turned = Bounds {
            min: [-20., -20., 0.],
            max: [20., 20., 98.],
        };
        close(
            crate::simulation::leftover_box(&v, &mill, &mesh),
            in_mill(turned),
        );
        // The mill sees the rings point by point, through `Frame::transfer`.
        let removed = Removed::new(&v, b);
        for (p, cut) in [
            ([21., 0., 50.], true),
            ([0., -24., 3.], true),
            ([5., 5., 99.], true),
            ([19., 0., 50.], false),
            ([0., 0., 97.], false),
        ] {
            assert_eq!(removed.at(a.transfer(p, b)), cut, "{p:?}");
        }
        assert_eq!(removed.by(a.transfer([0., 0., 99.], b)), Some((0, 2)));
        // The rings are not capsule evidence: nothing is cleared, no start
        // height is lowered by them.
        let rest = RestStock::new(&v, b);
        assert!(rest.cuts.is_empty());
        let everywhere = Bounds {
            min: [-200.; 3],
            max: [200.; 3],
        };
        assert!(rest.cleared(everywhere, -100., 100.).is_empty());
        assert_eq!(rest.top(everywhere, -50., 60.), 60.);
        // The removal boxes are the rings' boxes, seen in the mill's frame.
        assert_eq!(removal_boxes(&v, b).len(), 3);
        close(
            cut_boxes(&v, b)[2],
            in_mill(Bounds {
                min: [-20., -20., 98.],
                max: [20., 20., 101.],
            }),
        );
    }
    #[test]
    fn cleared_outlines_the_flat_sweeps_spanning_the_heights_in_either_frame() {
        let (mut v, f, _, s, mesh) = fixture();
        // Tip at 2, flute 6: the sweep spans 2 to 8.
        v.cut(
            [-5., 0., 2.],
            [5., 0., 2.],
            2.,
            6.,
            CutterShape::Flat,
            f,
            (0, 0),
        );
        let rest = RestStock::new(&v, f);
        let near = Bounds {
            min: [-20., -20., 0.],
            max: [20., 20., 20.],
        };
        let outline = &rest.cleared(near, 3., 7.)[0];
        // Every vertex on the capsule, so its chords lie inside it.
        for &(x, y) in outline {
            let d = (x.abs() - 5.).max(0.).hypot(y);
            assert!((d - 2.).abs() < 0.001, "{x} {y}");
        }
        assert!(rest.cleared(near, 1., 7.).is_empty());
        assert!(rest.cleared(near, 3., 9.).is_empty());
        let far = Bounds {
            min: [10., 10., 0.],
            max: [20., 20., 20.],
        };
        assert!(rest.cleared(far, 3., 7.).is_empty());
        // Seen from a setup turned over the sweep spans its mirror heights.
        let mut turned = s.clone();
        turned.p.insert("rx".into(), "180".into());
        let g = Frame::new(&turned, &mesh);
        let from_below = RestStock::new(&v, g);
        let z = [2., 8.].map(|h| f.transfer([0., 0., h], g)[2]);
        let (low, high) = (z[0].min(z[1]), z[0].max(z[1]));
        assert_eq!(from_below.cleared(near, low + 0.5, high - 0.5).len(), 1);
        assert!(from_below.cleared(near, low - 0.5, high).is_empty());
    }
    #[test]
    fn removal_boxes_start_at_the_tool_tip() {
        let (mut v, f, _, _, _) = fixture();
        face(&mut v, f, 8., &[-6.], 3.);
        let removed = removal_boxes(&v, f)[0];
        assert_eq!(
            (removed.min, removed.max),
            ([-17.5, -8.5, 8.], [17.5, -3.5, 11.])
        );
        // cut_boxes reaches a tool radius lower.
        assert_eq!(cut_boxes(&v, f)[0].min[2], 5.5);
    }
    #[test]
    fn removed_follows_the_tool_path_from_its_tip_to_its_flute_top() {
        let (mut v, f, _, mut s, mesh) = fixture();
        v.cut(
            [-5., -5., 8.],
            [5., 5., 8.],
            1.,
            3.,
            CutterShape::Flat,
            f,
            (0, 0),
        );
        let removed = Removed::new(&v, f);
        // Inside the cut's box, but 5.7 mm from its diagonal path.
        assert!(in_any(&removal_boxes(&v, f), [4., -4., 9.]));
        assert!(!removed.at([4., -4., 9.]));
        assert!(removed.at([0.5, 0., 9.]));
        // From the tip up to the flute top, with a 0.001 mm guard.
        assert!(!removed.at([0., 0., 7.99]));
        assert!(removed.at([0., 0., 7.9995]));
        assert!(removed.at([0., 0., 11.0005]));
        assert!(!removed.at([0., 0., 11.01]));
        assert!(removed.at([1.4, 0., 9.]) && !removed.at([1.42, 0., 9.]));
        // The same material seen from a flipped setup.
        s.p.insert("rx".into(), "180".into());
        let flipped = Frame::new(&s, &mesh);
        let removed = Removed::new(&v, flipped);
        assert!(removed.at(f.transfer([0.5, 0., 9.], flipped)));
        assert!(!removed.at(f.transfer([4., -4., 9.], flipped)));
    }
    #[test]
    fn removed_says_which_operation_cut_a_point_first() {
        let (mut v, f, _, mut s, mesh) = fixture();
        // Setup 1's operation 3 cuts along y = -6, then setup 2's operation 1
        // along x = 0, crossing it.
        v.cut(
            [-15., -6., 8.],
            [15., -6., 8.],
            2.,
            3.,
            CutterShape::Flat,
            f,
            (0, 2),
        );
        v.cut(
            [0., -15., 8.],
            [0., 15., 8.],
            2.,
            3.,
            CutterShape::Flat,
            f,
            (1, 0),
        );
        let removed = Removed::new(&v, f);
        assert_eq!(removed.by([8., -6., 9.]), Some((0, 2)));
        assert_eq!(removed.by([0., 6., 9.]), Some((1, 0)));
        // Where both cut, the first.
        assert_eq!(removed.by([0., -6., 9.]), Some((0, 2)));
        assert_eq!(removed.by([8., 6., 9.]), None);
        assert!(!removed.at([8., 6., 9.]) && removed.at([0., 6., 9.]));
        // The same seen from a flipped setup.
        s.p.insert("rx".into(), "180".into());
        let flipped = Frame::new(&s, &mesh);
        let removed = Removed::new(&v, flipped);
        assert_eq!(removed.by(f.transfer([0., 6., 9.], flipped)), Some((1, 0)));
        assert_eq!(removed.by(f.transfer([8., 6., 9.], flipped)), None);
    }
    /// Flat Ø4 passes round the square of `half` mm about the middle of the
    /// fixture's stock, tool centres on it, one at each tip height of `tips`
    /// with 3 mm of flute, made by setup 1's operation 3.
    fn ring(v: &mut Volume, f: Frame, half: f32, tips: &[f32]) {
        let corners = [[-half, -half], [half, -half], [half, half], [-half, half]];
        for &z in tips {
            for k in 0..4 {
                let (a, b) = (corners[k], corners[(k + 1) % 4]);
                v.cut(
                    [a[0], a[1], z],
                    [b[0], b[1], z],
                    2.,
                    3.,
                    CutterShape::Flat,
                    f,
                    (0, 2),
                );
            }
        }
    }
    #[test]
    fn the_material_joined_to_the_part_is_found_through_the_stock_left() {
        // The part is the middle 8 mm square, its walls 2 mm inside a ring
        // cut down past the stock's bottom: the stock outside is cut free.
        let part = Bounds {
            min: [-4., -4., 0.],
            max: [4., 4., 12.],
        };
        let (mut v, f, _, mut s, mesh) = fixture();
        let grid = PartGrid::boxed(&v, part);
        // Without earlier cuts the stock is one piece: nothing is said.
        assert_eq!(
            Removed::new(&v, f).with_part(&grid).joined([9., 0., 6.]),
            None
        );
        ring(&mut v, f, 6., &[-1., 2., 5., 8., 11.]);
        let removed = Removed::new(&v, f);
        // Not knowing the part, nothing is said either.
        assert_eq!(removed.joined([0., 0., 6.]), None);
        let removed = removed.with_part(&grid);
        assert_eq!(removed.joined([0., 0., 6.]), Some(true));
        assert_eq!(removed.joined([9.5, 0., 0.5]), Some(false));
        assert_eq!(removed.joined([-9.5, 9.5, 11.5]), Some(false));
        // In the ring, and outside the stock.
        assert_eq!(removed.joined([6., 0., 6.]), None);
        assert_eq!(removed.joined([0., 0., 13.]), None);
        // The same from a flipped setup.
        s.p.insert("rx".into(), "180".into());
        let flipped = Frame::new(&s, &mesh);
        let turned = Removed::new(&v, flipped).with_part(&grid);
        assert_eq!(turned.joined(f.transfer([0., 0., 6.], flipped)), Some(true));
        assert_eq!(
            turned.joined(f.transfer([9.5, 0., 0.5], flipped)),
            Some(false)
        );
        // Stopped 2 mm above the bottom, the ring leaves a skin joining them.
        let (mut v, f, _, _, _) = fixture();
        ring(&mut v, f, 6., &[2., 5., 8., 11.]);
        let removed = Removed::new(&v, f).with_part(&PartGrid::boxed(&v, part));
        assert_eq!(removed.joined([9.5, 0., 6.]), Some(true));
        assert_eq!(removed.joined([6., 0., 6.]), None);
        // A part where no stock is left cannot be told apart from the rest.
        let gone = Bounds {
            min: [5.8, -1., 6.],
            max: [6.2, 1., 7.],
        };
        let gone = PartGrid::boxed(&v, gone);
        assert_eq!(
            Removed::new(&v, f).with_part(&gone).joined([9.5, 0., 6.]),
            None
        );
    }
    fn in_any(boxes: &[Bounds], p: [f32; 3]) -> bool {
        boxes
            .iter()
            .any(|b| (0..3).all(|k| p[k] >= b.min[k] && p[k] <= b.max[k]))
    }
    #[test]
    fn overlapping_cylinders_prove_full_cap_without_voxel_evidence() {
        let (mut v, f, b, _, _) = fixture();
        face(&mut v, f, 8., &[-10., -6., -2., 2., 6., 10.], 10.);
        v.occupied.fill(1); // deliberately wrong volume estimate cannot affect planning
        assert_eq!(RestStock::new(&v, f).top(b, 2., 12.), 8.);
    }
    #[test]
    fn partial_facing_leaves_high_stock_and_respects_local_region() {
        let (mut v, f, b, _, _) = fixture();
        face(&mut v, f, 8., &[-10., -6.], 10.);
        v.occupied.fill(0);
        let rest = RestStock::new(&v, f);
        assert_eq!(rest.top(b, 2., 12.), 12.);
        assert_eq!(
            rest.top(
                Bounds {
                    min: [-8., -9., 0.],
                    max: [8., -5., 12.]
                },
                2.,
                12.
            ),
            8.
        );
    }
    #[test]
    fn a_reach_holds_its_area_less_its_holes_and_misses_only_boxes_wholly_beside_it() {
        // A 20 mm square with a 6 mm square hole, as Clipper gives it: the
        // outline anticlockwise, the hole clockwise.
        let reach = Reach::new(&[
            vec![(-10., -10.), (10., -10.), (10., 10.), (-10., 10.)],
            vec![(-3., -3.), (-3., 3.), (3., 3.), (3., -3.)],
        ]);
        let square = |x: f32, y: f32, half: f32| Bounds {
            min: [x - half, y - half, 0.],
            max: [x + half, y + half, 1.],
        };
        assert!(reach.holds([5., 5., 0.]) && reach.holds([-9.9, 0., 0.]));
        assert!(!reach.holds([0., 0., 0.]) && !reach.holds([11., 0., 0.]));
        // In the hole, beyond the outline: missed.
        assert!(reach.misses(square(0., 0., 2.9)));
        assert!(reach.misses(square(12., 12., 1.9)));
        assert!(reach.misses(square(0., -15., 4.)));
        // Inside it, over an edge, round the hole: met.
        assert!(!reach.misses(square(6., 6., 1.)));
        assert!(!reach.misses(square(0., 0., 3.)));
        assert!(!reach.misses(square(10.5, 0., 1.)));
        assert!(!reach.misses(square(0., 0., 30.)));
        // A disc reaches out to its radius, not to its box's corners.
        let disc = Reach::disc([1., 2.], 3.);
        assert!(disc.holds([1., 4.99, 0.]) && disc.holds([3.1, 4.1, 0.]));
        assert!(disc.misses(square(1. + 2.7, 2. + 2.7, 0.3)));
    }
    #[test]
    fn a_reach_within_a_diagonal_cut_starts_under_it_though_its_box_corners_are_full_height() {
        let (mut v, f, _, _, _) = fixture();
        v.cut(
            [-15., -15., 8.],
            [15., 15., 8.],
            2.5,
            10.,
            CutterShape::Flat,
            f,
            (0, 0),
        );
        let rest = RestStock::new(&v, f);
        // A band along the cut, 0.7 mm either side of its centre line.
        let band = Reach::new(&[vec![(-8., -9.), (9., 8.), (8., 9.), (-9., -8.)]]);
        assert_eq!(rest.top(band.bounds, 2., 12.), 12.);
        assert_eq!(rest.top_over(&band, 2., 12.), 8.);
        // The plane's proof sees it too, and gives up without the work.
        assert_eq!(rest.swept(&band, 8., 12., &mut 10), Some(true));
        assert_eq!(rest.swept(&band, 8., 12., &mut 0), None);
        // One that reaches past the cut's side meets the stock beside it.
        let wide = Reach::new(&[vec![(-8., -9.), (9., 8.), (8., 9.), (-9., -4.)]]);
        assert_eq!(rest.top_over(&wide, 2., 12.), 12.);
    }
    #[test]
    fn a_point_the_sweeps_miss_shows_what_clipping_would_find_left() {
        let square = |r: f64| Reach::new(&[vec![(-r, -r), (r, -r), (r, r), (-r, r)]]);
        // What clipping the outlines from the reach leaves.
        let left = |rest: &RestStock, reach: &Reach| {
            let e = rest.extent;
            let inside = vec![vec![
                (e.min[0] as f64, e.min[1] as f64),
                (e.max[0] as f64, e.min[1] as f64),
                (e.max[0] as f64, e.max[1] as f64),
                (e.min[0] as f64, e.max[1] as f64),
            ]];
            let area = intersect::<Milli>(reach.loops.clone(), inside, FillRule::NonZero);
            let gone = rest.cleared(reach.bounds, 8., 12.);
            area.and_then(|a| difference::<Milli>(a, gone, FillRule::NonZero))
                .unwrap()
                .contains_points()
        };
        // Rows of a larger tool over a square: they reach its corners out
        // to 3.27 mm from the centre, no further.
        let (mut v, f, _, _, _) = fixture();
        for (i, y) in [-1.5, 0., 1.5].into_iter().enumerate() {
            v.cut(
                [-1.5, y, 8.],
                [1.5, y, 8.],
                2.5,
                10.,
                CutterShape::Flat,
                f,
                (0, i as u16),
            );
        }
        let rest = RestStock::new(&v, f);
        for (r, uncut) in [(2., false), (3., false), (3.5, true)] {
            let reach = square(r);
            let sweeps = rest.sweeps(reach.bounds, 8., 12., |_| true);
            assert_eq!(rest.uncut(&reach, &sweeps), uncut, "{r}");
            assert_eq!(left(&rest, &reach), uncut, "{r}");
            assert_eq!(rest.swept(&reach, 8., 12., &mut 3), Some(!uncut), "{r}");
        }
        // Four sweeps round a spot 0.6 mm square in the middle that none
        // reach: the grid finds it, with every edge of the reach cut.
        let (mut v, f, _, _, _) = fixture();
        for (i, (a, b)) in [
            ([-2., -1.5], [2., -1.5]),
            ([-2., 1.5], [2., 1.5]),
            ([-1.5, -2.], [-1.5, 2.]),
            ([1.5, -2.], [1.5, 2.]),
        ]
        .into_iter()
        .enumerate()
        {
            v.cut(
                [a[0], a[1], 8.],
                [b[0], b[1], 8.],
                1.2,
                10.,
                CutterShape::Flat,
                f,
                (0, i as u16),
            );
        }
        let rest = RestStock::new(&v, f);
        let reach = square(1.6);
        let sweeps = rest.sweeps(reach.bounds, 8., 12., |_| true);
        assert!(rest.uncut(&reach, &sweeps) && left(&rest, &reach));
        assert_eq!(rest.swept(&reach, 8., 12., &mut 4), Some(false));
    }
    #[test]
    fn sweeps_off_a_reach_cost_its_plane_proof_no_work() {
        let (mut v, f, _, _, _) = fixture();
        // A ring 6 to 9 mm out, cleared by four sweeps along its sides…
        for (a, b) in [
            ([-7.5, -7.5], [7.5, -7.5]),
            ([7.5, -7.5], [7.5, 7.5]),
            ([7.5, 7.5], [-7.5, 7.5]),
            ([-7.5, 7.5], [-7.5, -7.5]),
        ] {
            v.cut(
                [a[0], a[1], 8.],
                [b[0], b[1], 8.],
                2.5,
                10.,
                CutterShape::Flat,
                f,
                (0, 0),
            );
        }
        // …round a pocket of a hundred sweeps in its hole.
        for i in 0..100 {
            let y = -2. + 4. * i as f32 / 99.;
            v.cut(
                [-2., y, 8.],
                [2., y, 8.],
                1.,
                10.,
                CutterShape::Flat,
                f,
                (0, 1 + i),
            );
        }
        let rest = RestStock::new(&v, f);
        let square = |r: f64| vec![(-r, -r), (r, -r), (r, r), (-r, r)];
        let mut hole = square(6.);
        hole.reverse();
        let ring = Reach::new(&[square(9.), hole]);
        // Only the four along the ring are worked through.
        assert_eq!(rest.swept(&ring, 8., 12., &mut 4), Some(true));
        assert_eq!(rest.swept(&ring, 8., 12., &mut 3), None);
        assert_eq!(rest.top_over(&ring, 2., 12.), 8.);
        // Over the hole the pocket's sweeps count, and do not cover it.
        assert_eq!(
            rest.swept(&Reach::new(&[square(5.)]), 8., 12., &mut 200),
            Some(false)
        );
    }
    #[test]
    fn finite_flutes_and_thin_uncut_strips_are_not_ignored() {
        let (mut v, f, b, _, _) = fixture();
        face(&mut v, f, 8., &[-10., -6., -2., 2., 6., 10.], 1.);
        assert_eq!(RestStock::new(&v, f).top(b, 2., 12.), 12.);
        let (mut v, f, b, _, _) = fixture();
        face(&mut v, f, 8., &[-10., -5., 0.001, 5., 10.], 10.);
        assert_eq!(RestStock::new(&v, f).top(b, 2., 12.), 12.);
    }
    #[test]
    fn allowance_stays_and_flipped_stock_does_not_reuse_the_old_top() {
        let (mut v, f, b, mut s, mesh) = fixture();
        face(&mut v, f, 8.6, &[-10., -6., -2., 2., 6., 10.], 10.);
        assert_eq!(RestStock::new(&v, f).top(b, 2., 12.), 8.6);
        s.p.insert("rx".into(), "180".into());
        let flipped = Frame::new(&s, &mesh);
        let b = v.transformed_bounds(flipped);
        assert_eq!(
            RestStock::new(&v, flipped).top(b, b.min[2] + 2., b.max[2]),
            b.max[2]
        );
    }
    #[test]
    fn ramps_ball_cuts_and_tiny_budgets_keep_uncertain_material() {
        let (mut v, f, b, _, _) = fixture();
        v.cut(
            [-15., 0., 8.],
            [15., 0., 8.],
            30.,
            10.,
            CutterShape::Ball,
            f,
            (0, 0),
        );
        v.cut(
            [-15., 0., 8.],
            [15., 0., 9.],
            30.,
            10.,
            CutterShape::Flat,
            f,
            (0, 0),
        );
        assert_eq!(RestStock::new(&v, f).top(b, 2., 12.), 12.);
        face(&mut v, f, 8., &[-10., -6., -2., 2., 6., 10.], 10.);
        let rest = RestStock::new(&v, f);
        assert!(!rest.covered(
            b,
            &(0..rest.cuts.len()).collect::<Vec<_>>(),
            None,
            0,
            &mut 0
        ));
    }
    #[test]
    fn a_region_is_beside_the_stock_only_when_it_shares_no_area_with_the_box() {
        let (mut v, f, _, _, _) = fixture();
        face(&mut v, f, 8., &[-10., -6., -2., 2., 6., 10.], 10.);
        let rest = RestStock::new(&v, f);
        let column = |x: f32, y: f32| Bounds {
            min: [x - 3., y - 3., 0.],
            max: [x + 3., y + 3., 12.],
        };
        for (x, y) in [(13., 0.), (0., -13.), (-14., 20.)] {
            assert!(rest.beside(column(x, y)), "({x}, {y})");
        }
        for (x, y) in [(12.99, 0.), (0., -12.99), (0., 0.)] {
            assert!(!rest.beside(column(x, y)), "({x}, {y})");
        }
    }
    /// A part well inside the fixture's stock box.
    const PART: Bounds = Bounds {
        min: [-5., -5., 1.],
        max: [5., 5., 6.],
    };
    fn near(a: Bounds, b: Bounds) -> bool {
        (0..3).all(|k| (a.min[k] - b.min[k]).abs() < 0.001 && (a.max[k] - b.max[k]).abs() < 0.001)
    }
    /// Outside-contour passes along X with their centres at each `ys`, one at
    /// each tip height of `levels`.
    fn contour(v: &mut Volume, f: Frame, ys: &[f32], levels: &[f32]) {
        for &z in levels {
            face(v, f, z, ys, 5.);
        }
    }
    #[test]
    fn with_no_earlier_cuts_the_leftover_box_is_the_original_box() {
        let (v, f, b, _, _) = fixture();
        assert_eq!(RestStock::new(&v, f).left(PART), b);
    }
    #[test]
    fn a_facing_pass_over_the_whole_top_lowers_the_top_to_its_depth() {
        let (mut v, f, b, mut s, mesh) = fixture();
        face(&mut v, f, 10., &[-10., -6., -2., 2., 6., 10.], 10.);
        let left = RestStock::new(&v, f).left(PART);
        let mut expected = b;
        expected.max[2] = 10.;
        assert!(near(left, expected), "{left:?}");
        // Flipped, the faced top is the bottom: it moves up by the same 2 mm.
        s.p.insert("rx".into(), "180".into());
        let flipped = Frame::new(&s, &mesh);
        let carried = v.transformed_bounds(flipped);
        let part = f.transfer_bounds(PART, flipped);
        let left = RestStock::new(&v, flipped).left(part);
        let mut expected = carried;
        expected.min[2] += 2.;
        assert!(near(left, expected), "{left:?} {carried:?}");
    }
    #[test]
    fn full_depth_side_contours_move_those_sides_in_by_the_removed_width() {
        let (mut v, f, b, _, _) = fixture();
        // Tool centres on the stock's ±Y faces, radius 2.5, tips from 2 mm
        // below the stock's bottom up past its top.
        contour(&mut v, f, &[-10., 10.], &[-2., 2., 6., 10.]);
        let left = RestStock::new(&v, f).left(PART);
        let mut expected = b;
        expected.min[1] = -7.5;
        expected.max[1] = 7.5;
        assert!(near(left, expected), "{left:?}");
    }
    #[test]
    fn side_contours_stopping_above_the_bottom_leave_the_sides() {
        let (mut v, f, b, _, _) = fixture();
        contour(&mut v, f, &[-10., 10.], &[6., 10.]);
        assert_eq!(RestStock::new(&v, f).left(PART), b);
    }
    #[test]
    fn ball_cuts_ramps_and_an_exhausted_budget_never_trim() {
        let (mut ball, f, b, _, _) = fixture();
        ball.cut(
            [-15., 0., 10.],
            [15., 0., 10.],
            30.,
            10.,
            CutterShape::Ball,
            f,
            (0, 0),
        );
        assert_eq!(RestStock::new(&ball, f).left(PART), b);
        let (mut ramp, f, b, _, _) = fixture();
        ramp.cut(
            [-15., 0., 10.],
            [15., 0., 10.5],
            30.,
            10.,
            CutterShape::Flat,
            f,
            (0, 0),
        );
        assert_eq!(RestStock::new(&ramp, f).left(PART), b);
        let (mut faced, f, b, _, _) = fixture();
        face(&mut faced, f, 10., &[-10., -6., -2., 2., 6., 10.], 10.);
        contour(&mut faced, f, &[-10., 10.], &[-2., 2., 6., 10.]);
        assert_eq!(RestStock::new(&faced, f).trimmed(PART, 0), b);
    }
    /// Jaws centred along the sides, longer than the fixture's stock.
    const LONG: (f32, f32) = (0., 100.);
    #[test]
    fn the_band_a_vise_grips_loses_the_sides_cut_through_all_its_heights() {
        let (mut v, f, b, mut s, mesh) = fixture();
        // Outside-contour passes on ±Y down to 2 mm above the stock's bottom
        // (their flutes reach past the top): the 2 mm under them keep the
        // box's width.
        contour(&mut v, f, &[-10., 10.], &[2., 6., 10.]);
        let rest = RestStock::new(&v, f);
        assert_eq!(rest.left(PART), b);
        // Gripping the 2 mm left, or 4 mm whose upper half alone was cut,
        // the jaws close on the box's width.
        assert_eq!(rest.band(PART, b, 2., 1, LONG), b);
        assert_eq!(rest.band(PART, b, 4., 1, LONG), b);
        // Turned over, the jaws grip what was the top, which the contour cut
        // through on ±Y: the band is 2.5 mm narrower on each (less the
        // 0.01 mm it keeps short of a cut face), as wide as ever across X,
        // and as tall as the box.
        s.p.insert("rx".into(), "180".into());
        let flipped = Frame::new(&s, &mesh);
        let part = f.transfer_bounds(PART, flipped);
        let rest = RestStock::new(&v, flipped);
        let stock = rest.left(part);
        assert!(near(stock, v.transformed_bounds(flipped)), "{stock:?}");
        let band = rest.band(part, stock, 8., 1, LONG);
        let mut expected = stock;
        expected.min[1] += 2.5 - BAND_SLACK;
        expected.max[1] -= 2.5 - BAND_SLACK;
        assert!(near(band, expected), "{band:?} {stock:?}");
        // Gripping 11 mm reaches into the 2 mm the contour left: the box.
        assert_eq!(rest.band(part, stock, 11., 1, LONG), stock);
        // Neither a grip of nothing nor an exhausted budget narrows it.
        assert_eq!(rest.band(part, stock, 0., 1, LONG), stock);
        assert_eq!(rest.band_within(part, stock, 8., 1, LONG, 0), stock);
    }
    #[test]
    fn the_jaws_close_on_what_lies_within_their_reach_along_the_sides() {
        let (mut v, f, _, mut s, mesh) = fixture();
        // Passes on ±Y that stop short of the stock's ±X faces, as a small
        // cutter leaves the corners: turned over, the band's ±Y sides are cut
        // through only between x = -6 and 6 (the passes' round ends reach
        // 8.5 mm out on their centre lines, not beside them).
        for z in [2., 6., 10.] {
            for y in [-10., 10.] {
                v.cut(
                    [-6., y, z],
                    [6., y, z],
                    2.5,
                    5.,
                    CutterShape::Flat,
                    f,
                    (0, 0),
                );
            }
        }
        s.p.insert("rx".into(), "180".into());
        let flipped = Frame::new(&s, &mesh);
        let part = f.transfer_bounds(PART, flipped);
        let rest = RestStock::new(&v, flipped);
        let stock = v.transformed_bounds(flipped);
        // Jaws 10 mm wide in the middle close on what the passes left; so do
        // jaws moved 1 mm along, and the corners hold open jaws as long as
        // the stock, or moved to reach them.
        let across = |jaws| rest.band(part, stock, 8., 1, jaws).dimensions()[1];
        let narrow = 15. + 2. * BAND_SLACK;
        assert!(
            (across((0., 10.)) - narrow).abs() < 0.001,
            "{}",
            across((0., 10.))
        );
        assert!((across((1., 10.)) - narrow).abs() < 0.001);
        assert_eq!(across(LONG), stock.dimensions()[1]);
        assert_eq!(across((6., 10.)), stock.dimensions()[1]);
        // The sides along the jaws keep the box's length either way.
        let band = rest.band(part, stock, 8., 1, (0., 10.));
        assert_eq!((band.min[0], band.max[0]), (stock.min[0], stock.max[0]));
        // Jaws beside the stock touch nothing: the box.
        assert_eq!(rest.band(part, stock, 8., 1, (40., 10.)), stock);
    }
    #[test]
    fn the_band_never_reaches_into_the_part_and_ball_cuts_never_narrow_it() {
        let (mut v, f, _, mut s, mesh) = fixture();
        contour(&mut v, f, &[-10., 10.], &[2., 6., 10.]);
        s.p.insert("rx".into(), "180".into());
        let flipped = Frame::new(&s, &mesh);
        let rest = RestStock::new(&v, flipped);
        let stock = v.transformed_bounds(flipped);
        // A part reaching 9 mm out on Y: the band stops at it.
        let wide = Bounds {
            min: [-5., -9., 1.],
            max: [5., 9., 6.],
        };
        let part = f.transfer_bounds(wide, flipped);
        let band = rest.band(part, stock, 8., 1, LONG);
        assert!(
            (band.min[1] - part.min[1]).abs() < 0.001 && (band.max[1] - part.max[1]).abs() < 0.001,
            "{band:?} {part:?}"
        );
        // Ball-end passes over the same sides prove nothing.
        let (mut ball, f, _, _, _) = fixture();
        for y in [-10., 10.] {
            for z in [2., 6., 10.] {
                ball.cut(
                    [-15., y, z],
                    [15., y, z],
                    2.5,
                    5.,
                    CutterShape::Ball,
                    f,
                    (0, 0),
                );
            }
        }
        let rest = RestStock::new(&ball, flipped);
        let part = f.transfer_bounds(PART, flipped);
        assert_eq!(rest.band(part, stock, 8., 1, LONG), stock);
    }
    #[test]
    fn the_bottom_rises_past_the_layer_cuts_of_any_shape_can_have_taken_off_it() {
        // Nothing cut: the box's bottom.
        let (v, f, b, mut s, mesh) = fixture();
        assert_eq!(RestStock::new(&v, f).bottom(b), b.min[2]);
        // A Ø5 ball end mill faces the whole top 2 mm deep (tips at 10), a
        // pass every 2 mm: the box keeps the layer, since ball cuts prove
        // nothing, but turned over, the stock stands on the faced surface,
        // 2 mm above the box's bottom.
        let ys = [-10., -8., -6., -4., -2., 0., 2., 4., 6., 8., 10.];
        let ball_face = |v: &mut Volume, f: Frame, x: [f32; 2], z: f32| {
            for &y in &ys {
                v.cut(
                    [x[0], y, z],
                    [x[1], y, z],
                    2.5,
                    5.,
                    CutterShape::Ball,
                    f,
                    (0, 0),
                );
            }
        };
        let (mut ball, f, b, _, _) = fixture();
        ball_face(&mut ball, f, [-15., 15.], 10.);
        s.p.insert("rx".into(), "180".into());
        let flipped = Frame::new(&s, &mesh);
        let part = f.transfer_bounds(PART, flipped);
        let rest = RestStock::new(&ball, flipped);
        let stock = rest.left(part);
        assert_eq!(stock, ball.transformed_bounds(flipped));
        let bottom = rest.bottom(stock);
        assert!(
            (bottom - (stock.min[2] + 2.)).abs() < 0.001,
            "{bottom} {stock:?}"
        );
        // Seen the same way up, the faced layer is at the top: the bottom stays.
        assert_eq!(RestStock::new(&ball, f).bottom(b), b.min[2]);
        // A second ball facing 2 mm deeper stacks on the first: 4 mm.
        ball_face(&mut ball, f, [-15., 15.], 8.);
        let rest = RestStock::new(&ball, flipped);
        let bottom = rest.bottom(stock);
        assert!(
            (bottom - (stock.min[2] + 4.)).abs() < 0.001,
            "{bottom} {stock:?}"
        );
        // Flat passes prove the layer gone and the box's bottom follows it:
        // nothing is left to raise it past...
        let (mut flat, f, _, _, _) = fixture();
        face(&mut flat, f, 10., &[-10., -6., -2., 2., 6., 10.], 10.);
        let rest = RestStock::new(&flat, flipped);
        let stock = rest.left(part);
        let carried = flat.transformed_bounds(flipped);
        assert!(
            (stock.min[2] - (carried.min[2] + 2.)).abs() < 0.001,
            "{stock:?}"
        );
        assert_eq!(rest.bottom(stock), stock.min[2]);
        // ...until a ball finishing pass takes another 0.5 mm off the faced
        // top, which the box keeps.
        ball_face(&mut flat, f, [-15., 15.], 9.5);
        let rest = RestStock::new(&flat, flipped);
        assert_eq!(rest.left(part), stock);
        let bottom = rest.bottom(stock);
        assert!(
            (bottom - (stock.min[2] + 0.5)).abs() < 0.001,
            "{bottom} {stock:?}"
        );
        // Faced only between y = -4 and 4 (the passes reach 2.5 mm out), the
        // stock stands on its uncut ends: the bottom stays.
        let (mut partial, f, _, _, _) = fixture();
        for y in [-4., -2., 0., 2., 4.] {
            partial.cut(
                [-15., y, 10.],
                [15., y, 10.],
                2.5,
                5.,
                CutterShape::Ball,
                f,
                (0, 0),
            );
        }
        let rest = RestStock::new(&partial, flipped);
        let stock = rest.left(part);
        assert_eq!(rest.bottom(stock), stock.min[2]);
        // So it does under a footprint that leaves 0.3 mm of those ends,
        // which the 64 fixed columns all miss: the proof finds the rim
        // exactly, and without a work budget the columns raise the bottom.
        let mut narrow = stock;
        (narrow.min[1], narrow.max[1]) = (-6.8, 6.8);
        assert_eq!(rest.bottom(narrow), stock.min[2]);
        let sampled = rest.bottom_within(narrow, 0);
        assert!(
            (sampled - (stock.min[2] + 2.)).abs() < 0.001,
            "{sampled} {stock:?}"
        );
        // Cut through everywhere (tips below the bottom, flutes past the
        // top), nothing stands: the box's top.
        let (mut gone, f, _, _, _) = fixture();
        for &y in &ys {
            gone.cut(
                [-15., y, -1.],
                [15., y, -1.],
                2.5,
                14.,
                CutterShape::Ball,
                f,
                (0, 0),
            );
        }
        let stock = gone.transformed_bounds(flipped);
        assert_eq!(RestStock::new(&gone, flipped).bottom(stock), stock.max[2]);
    }
    #[test]
    fn the_leftover_box_never_reaches_into_the_part() {
        let (mut v, f, b, _, _) = fixture();
        face(&mut v, f, 8., &[-10., -6., -2., 2., 6., 10.], 10.);
        contour(&mut v, f, &[-10., 10.], &[-2., 2., 6., 10.]);
        // The part's top is above the facing; its -Y face is inside the
        // band the -Y contour removed.
        let part = Bounds {
            min: [-5., -8.5, 1.],
            max: [5., 5., 10.],
        };
        let left = RestStock::new(&v, f).left(part);
        let expected = Bounds {
            min: [b.min[0], -8.5, b.min[2]],
            max: [b.max[0], 7.5, 10.],
        };
        assert!(near(left, expected), "{left:?}");
    }
}
