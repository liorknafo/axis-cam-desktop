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
        .map(|c| {
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
        })
        .collect()
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
}
impl<'a> Removed<'a> {
    pub fn new(volume: &'a Volume, current: Frame) -> Self {
        Self {
            cuts: &volume.cuts[..],
            current,
            boxes: removal_boxes(volume, current),
        }
    }
    /// Whether an earlier cut can have removed material at `p`, in the
    /// current frame. A point within 0.001 mm of a cut counts as removed,
    /// which keeps float error in the frame transforms on the safe side.
    pub fn at(&self, p: [f32; 3]) -> bool {
        const GUARD: f32 = 0.001;
        self.cuts.iter().zip(&self.boxes).any(|(cut, b)| {
            (0..3).all(|k| p[k] >= b.min[k] - 2. * GUARD && p[k] <= b.max[k] + 2. * GUARD) && {
                let q = transfer(p, self.current, cut.frame);
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
        let mut budget = 200_000usize;
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
        self.trimmed(part, 200_000)
    }
    /// `left` with `budget` units of `covered` work for each side.
    fn trimmed(&self, part: Bounds, budget: usize) -> Bounds {
        let mut b = self.extent;
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
        for (k, high) in [
            (2, true),
            (2, false),
            (0, false),
            (0, true),
            (1, false),
            (1, true),
        ] {
            let side = if high { b.max[k] } else { b.min[k] };
            let toward = if high { margin } else { -margin };
            // How far a candidate face lies inside the box from this side.
            let depth = |v: f32| if high { side - v } else { v - side };
            let mut candidates: Vec<f32> = self
                .cuts
                .iter()
                .flat_map(|c| [c.bounds.min[k], c.bounds.max[k]])
                .flat_map(|v| [v, v + toward])
                .filter(|v| v.is_finite() && *v > b.min[k] && *v < b.max[k])
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
        let mut span = [0_f32; 3];
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
            for k in 0..3 {
                span[k] = span[k].max(c.bounds.max[k] - c.bounds.min[k]);
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
        // Split where the query exceeds a typical sweep's extent. A long
        // facing stroke usually needs subdivision across its rows, not along X.
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
