//! Conservative start-height planning from earlier continuous cutter sweeps.
//! A cap is skipped only when its whole volume is covered by known cuts.
//! Coarse display voxels and samples never establish that material is absent.
use crate::{
    mesh::Bounds,
    model::Setup,
    simulation::CutterShape,
    stock_surface::Cut,
    volume::{Frame, Volume},
};

/// The units of `covered` work `left` and `band` spend on each side.
const BUDGET: usize = 200_000;
/// How far short of a capsule's face, mm, `band` takes a candidate side:
/// a slab whose face lies this far inside the removed material is proven
/// with far fewer, larger boxes than one a hair inside it, and the jaws
/// close at most this much outside a cut wall.
const BAND_SLACK: f32 = 0.01;
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
    extent: Bounds,
    cuts: Vec<Capsule>,
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
        for k in 0..3 {
            out.min[k] = out.min[k].min(p[k]);
            out.max[k] = out.max[k].max(p[k]);
        }
    }
    out
}
/// Box, in `current`, around each earlier cut of any cutter shape: material can
/// only have been removed inside these boxes.
pub fn cut_boxes(volume: &Volume, current: Frame) -> Vec<Bounds> {
    volume
        .cuts
        .iter()
        .map(|c| {
            let b = Bounds {
                min: std::array::from_fn(|k| c.a[k].min(c.b[k]) - c.radius),
                max: std::array::from_fn(|k| {
                    c.a[k].max(c.b[k]) + c.radius + if k == 2 { c.flute } else { 0. }
                }),
            };
            transformed(b, c.frame, current)
        })
        .collect()
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
/// `removal_boxes` for one cut.
fn removal_box(c: &Cut, current: Frame) -> Bounds {
    let b = Bounds {
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
    };
    transformed(b, c.frame, current)
}
/// The `source` of the first of `cuts` (with their `removal_box`es) that
/// removed material at `p`, in the `current` frame. A point within 0.001 mm
/// of a cut counts as removed, which keeps float error in the frame
/// transforms on the safe side.
fn first_removing(
    cuts: &[Cut],
    boxes: &[Bounds],
    current: Frame,
    p: [f32; 3],
) -> Option<(u16, u16)> {
    const GUARD: f32 = 0.001;
    cuts.iter()
        .zip(boxes)
        .find(|(cut, b)| {
            (0..3).all(|k| p[k] >= b.min[k] - 2. * GUARD && p[k] <= b.max[k] + 2. * GUARD) && {
                let q = transfer(p, current, cut.frame);
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
        })
        .map(|(cut, _)| cut.source)
}
/// What can tell which operation removed material at a point: the cuts of
/// earlier setups (`Removed`) or a setup's own (`Cuts`).
pub trait Removes {
    /// The operation whose cut removed material at `p` (current frame, mm),
    /// as its `Cut::source`, or None.
    fn by(&self, p: [f32; 3]) -> Option<(u16, u16)>;
    fn at(&self, p: [f32; 3]) -> bool {
        self.by(p).is_some()
    }
}
impl Removes for Removed<'_> {
    fn by(&self, p: [f32; 3]) -> Option<(u16, u16)> {
        Removed::by(self, p)
    }
}
/// The cuts a setup's own operations make, from their toolpaths, as
/// `Volume::cut` records them when the setup is simulated: each feed move a
/// sweep of the plan's cutter, made in the setup's frame. Which of them
/// removes material at a point, exactly as `Removed` tells it of earlier
/// setups' cuts.
pub struct Cuts {
    cuts: Vec<Cut>,
    boxes: Vec<Bounds>,
    current: Frame,
}
impl Cuts {
    /// The feed moves of `plans` (each with its operation's index) of setup
    /// `setup`, whose frame is `frame`.
    pub fn of_plans<'p>(
        plans: impl IntoIterator<Item = (usize, &'p crate::simulation::Plan)>,
        setup: usize,
        frame: Frame,
    ) -> Self {
        let index = |i: usize| u16::try_from(i).unwrap_or(u16::MAX);
        let cuts: Vec<Cut> = plans
            .into_iter()
            .flat_map(|(oi, plan)| {
                plan.moves
                    .iter()
                    .filter(|m| m.motion == crate::simulation::Motion::Feed)
                    .map(move |m| Cut {
                        a: m.from,
                        b: m.to,
                        radius: plan.radius,
                        flute: plan.flute_length,
                        shape: plan.shape,
                        frame,
                        source: (index(setup), index(oi)),
                    })
            })
            .collect();
        let boxes = cuts.iter().map(|c| removal_box(c, frame)).collect();
        Self {
            cuts,
            boxes,
            current: frame,
        }
    }
}
impl Cuts {
    /// The cutter radius of the cuts operation `source` makes (its first
    /// cut's), or None when it makes none.
    pub fn radius(&self, source: (u16, u16)) -> Option<f32> {
        self.cuts
            .iter()
            .find(|c| c.source == source)
            .map(|c| c.radius)
    }
}
impl Removes for Cuts {
    fn by(&self, p: [f32; 3]) -> Option<(u16, u16)> {
        first_removing(&self.cuts, &self.boxes, self.current, p)
    }
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
    /// `self` knowing the part, whose bounds are `part` (current frame, mm):
    /// which of the stock's voxels still hold material joined to it
    /// (`joined`). A flood fill through the occupied voxels (6-connectivity)
    /// from those whose centres lie within `part`, each voxel visited once
    /// (at most 96³). Nothing is filled without earlier cuts, the stock then
    /// being one piece, nor when no material is left within `part`.
    pub fn with_part(mut self, part: Bounds) -> Self {
        let v = self.volume;
        if v.cuts.is_empty() {
            return self;
        }
        let part = transformed(part, self.current, v.frame);
        let mut joined = vec![false; v.occupied.len()];
        let mut stack = vec![];
        for z in 0..v.n[2] {
            for y in 0..v.n[1] {
                for x in 0..v.n[0] {
                    let i = v.index([x, y, z]);
                    let c = v.point([x, y, z], 0.5);
                    if v.occupied[i] != 0
                        && (0..3).all(|k| c[k] >= part.min[k] && c[k] <= part.max[k])
                    {
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
        first_removing(self.cuts, &self.boxes, self.current, p)
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
        Self {
            bounds: volume.bounds,
            reference: volume.frame,
            current,
            extent: volume.transformed_bounds(current),
            cuts,
        }
    }
    /// Whether `region` lies wholly beside the stock box, where no stock ever was.
    pub fn beside(&self, region: Bounds) -> bool {
        (0..2).any(|k| region.max[k] <= self.extent.min[k] || region.min[k] >= self.extent.max[k])
    }
    /// Upper bound on remaining material within an operation's XY envelope.
    /// Uncertain coverage, thin remnants, or exhausted work budgets keep the
    /// higher starting height. Clearance/retract planes are independent.
    pub fn top(&self, mut region: Bounds, target: f32, original: f32) -> f32 {
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
        let ids: Vec<_> = (0..self.cuts.len()).collect();
        let (mut lo, mut hi) = (0, candidates.len() - 1);
        let mut budget = BUDGET;
        while lo < hi && budget > 0 {
            let mid = lo + (hi - lo) / 2;
            region.min[2] = candidates[mid];
            if self.covered(region, &ids, 0, &mut budget) {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        // hi always identifies a proven cap (or the original height).
        candidates[hi]
    }
    /// The stock box a later setup starts from, in the current frame (mm):
    /// the original block's box with each of its six sides moved in past
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
        self.trim(self.extent, &SIX, part, budget, None)
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
                    || !self.covered(slab(candidates[hi - 1]), ids, 0, &mut budget)
                {
                    continue;
                }
                hi -= 1;
                let mut lo = 0;
                while lo < hi && budget > 0 {
                    let mid = lo + (hi - lo) / 2;
                    if self.covered(slab(candidates[mid]), ids, 0, &mut budget) {
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
    fn covered(&self, b: Bounds, ids: &[usize], depth: u8, budget: &mut usize) -> bool {
        if b.min[2] > b.max[2] {
            return true;
        }
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
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
        // A box whose centre lies inside the stock but in no capsule cannot be
        // proven: the part of it round its centre would have to lie inside
        // one. Said before splitting it, so that a slab reaching into
        // material is given up at the first such box, not after proving all
        // the rest of it.
        let centre: [f32; 3] = std::array::from_fn(|k| (b.min[k] + b.max[k]) / 2.);
        let at = transfer(centre, self.current, self.reference);
        let in_stock = (0..3)
            .all(|k| at[k] > self.bounds.min[k] + guard && at[k] < self.bounds.max[k] - guard);
        if in_stock
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
        self.covered(left, &overlap, depth + 1, budget)
            && self.covered(right, &overlap, depth + 1, budget)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mesh, model::Machine};
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
        // Without earlier cuts the stock is one piece: nothing is said.
        assert_eq!(
            Removed::new(&v, f).with_part(part).joined([9., 0., 6.]),
            None
        );
        ring(&mut v, f, 6., &[-1., 2., 5., 8., 11.]);
        let removed = Removed::new(&v, f);
        // Not knowing the part, nothing is said either.
        assert_eq!(removed.joined([0., 0., 6.]), None);
        let removed = removed.with_part(part);
        assert_eq!(removed.joined([0., 0., 6.]), Some(true));
        assert_eq!(removed.joined([9.5, 0., 0.5]), Some(false));
        assert_eq!(removed.joined([-9.5, 9.5, 11.5]), Some(false));
        // In the ring, and outside the stock.
        assert_eq!(removed.joined([6., 0., 6.]), None);
        assert_eq!(removed.joined([0., 0., 13.]), None);
        // The same from a flipped setup.
        s.p.insert("rx".into(), "180".into());
        let flipped = Frame::new(&s, &mesh);
        let turned = Removed::new(&v, flipped).with_part(f.transfer_bounds(part, flipped));
        assert_eq!(turned.joined(f.transfer([0., 0., 6.], flipped)), Some(true));
        assert_eq!(
            turned.joined(f.transfer([9.5, 0., 0.5], flipped)),
            Some(false)
        );
        // Stopped 2 mm above the bottom, the ring leaves a skin joining them.
        let (mut v, f, _, _, _) = fixture();
        ring(&mut v, f, 6., &[2., 5., 8., 11.]);
        let removed = Removed::new(&v, f).with_part(part);
        assert_eq!(removed.joined([9.5, 0., 6.]), Some(true));
        assert_eq!(removed.joined([6., 0., 6.]), None);
        // A part where no stock is left cannot be told apart from the rest.
        let gone = Bounds {
            min: [5.8, -1., 6.],
            max: [6.2, 1., 7.],
        };
        assert_eq!(
            Removed::new(&v, f).with_part(gone).joined([9.5, 0., 6.]),
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
        assert!(!rest.covered(b, &(0..rest.cuts.len()).collect::<Vec<_>>(), 0, &mut 0));
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
