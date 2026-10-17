//! Adaptive clearing of a selected planar pocket floor: each depth level is
//! cleared along a path that keeps the cutter's radial engagement near the
//! Optimal load. A walker steps the tool centre along the edge of what is
//! already cut and turns each step so the material it removes per mm of
//! travel is the load: into the material where too little is left, away
//! where a corner would wrap round the cutter. What each level still holds
//! is tracked on a grid of cells; the tool centre stays inside the pocket's
//! allowed region (walls, islands and radial stock to leave), and every
//! move at depth is checked against that region exactly. Each level ends
//! with one pass round its walls at the stock to leave.
//!
//! The zones (floors, openings cut through, what earlier cuts left) are
//! Pocket's (`pocket::job`).
use crate::{
    mesh::Mesh,
    model::{
        ADAPTIVE_ENTRIES, Operation, Setup, Strategy, Tool, facing_setting, get, num,
        pocket_setting,
    },
    pocket::{self, ENTRY, Polygon},
    simulation::{Motion, Move, Plan},
};
use clipper2::{EndType, FillRule, JoinType, Milli, difference, union};

type P = (f64, f64);
/// The most moves an adaptive plan may hold.
const MAX_MOVES: usize = 300_000;
/// The most cells a zone's grid may hold; a larger zone uses larger cells.
const MAX_CELLS: f64 = 4_000_000.;
/// Directions the walker weighs at each step, evenly over its turn limit,
/// before it narrows in on the one that removes the load.
const DIRECTIONS: usize = 9;
/// Bisections between the two directions either side of the load.
const REFINE: usize = 4;
/// The pieces of a step whose engagement is weighed separately.
const PIECES: usize = 3;
/// Engagement over the load at which a settled pass stops rather than step
/// on: the corner it is going into is left for a pass from elsewhere.
const OVERLOAD: f64 = 2.;
/// Engagement under the load, as a share of it, below which a pass has run
/// out of material and stops.
const STARVED: f64 = 0.05;
/// The longest link, in tool radii, that stays down at depth through the
/// cleared area; a longer one lifts and moves at rapid.
const STAY_DOWN: f64 = 6.;
/// The sharpest turn the walker makes in one step without a Minimum cutting
/// radius, radians.
const MAX_TURN: f64 = 2.1;
/// Steps in a row that cut nothing after which a pass stops.
const DRY: usize = 8;
/// The most places whose first step is weighed as the start of a pass,
/// and the most of those whose pass is tried (`Level::promise`).
const START_JUDGED: usize = 60;
const START_TRIES: usize = 12;
/// How far ahead, in tool radii, a pass is tried to weigh its start.
const PROMISE: f64 = 2.;
/// The least share of the load a pass must promise; below it, what is left
/// is crumbs.
const CRUMB: f64 = 0.01;
/// The engagement, over the load, past which a wall pass slows down.
const WALL_LOAD: f64 = 1.25;
/// The least share of the feed a wall pass slows to.
const SLOWEST: f64 = 0.2;

/// A grid of bits, a row of 64-bit words per row of cells.
#[derive(Clone)]
struct Bits {
    nx: usize,
    ny: usize,
    words: usize,
    bits: Vec<u64>,
}
impl Bits {
    fn new(nx: usize, ny: usize) -> Self {
        let words = nx.div_ceil(64);
        Self {
            nx,
            ny,
            words,
            bits: vec![0; words * ny],
        }
    }
    fn get(&self, i: isize, j: isize) -> bool {
        if i < 0 || j < 0 || i as usize >= self.nx || j as usize >= self.ny {
            return false;
        }
        let (i, j) = (i as usize, j as usize);
        self.bits[j * self.words + i / 64] >> (i % 64) & 1 == 1
    }
    /// The masks of cells `i0..=i1` in each word of a row they touch.
    fn masks(i0: usize, i1: usize) -> impl Iterator<Item = (usize, u64)> {
        (i0 / 64..=i1 / 64).map(move |w| {
            let lo = if w == i0 / 64 { i0 % 64 } else { 0 };
            let hi = if w == i1 / 64 { i1 % 64 } else { 63 };
            let mask = if hi - lo == 63 {
                u64::MAX
            } else {
                ((1u64 << (hi - lo + 1)) - 1) << lo
            };
            (w, mask)
        })
    }
    fn count(&self, j: usize, i0: usize, i1: usize) -> u32 {
        let row = &self.bits[j * self.words..];
        Self::masks(i0, i1)
            .map(|(w, m)| (row[w] & m).count_ones())
            .sum()
    }
    /// `count` of the cells also set in `other`.
    fn count_and(&self, other: &Bits, j: usize, i0: usize, i1: usize) -> u32 {
        let (row, mask) = (&self.bits[j * self.words..], &other.bits[j * self.words..]);
        Self::masks(i0, i1)
            .map(|(w, m)| (row[w] & mask[w] & m).count_ones())
            .sum()
    }
    fn clear(&mut self, j: usize, i0: usize, i1: usize) -> u32 {
        let row = &mut self.bits[j * self.words..];
        let mut n = 0;
        for (w, m) in Self::masks(i0, i1) {
            n += (row[w] & m).count_ones();
            row[w] &= !m;
        }
        n
    }
    fn fill(&mut self, j: usize, i0: usize, i1: usize) {
        let row = &mut self.bits[j * self.words..];
        for (w, m) in Self::masks(i0, i1) {
            row[w] |= m;
        }
    }
    fn total(&self) -> u64 {
        self.bits.iter().map(|w| w.count_ones() as u64).sum()
    }
    /// Keep only the bits also set in `other`.
    fn and(&mut self, other: &Bits) {
        for (a, b) in self.bits.iter_mut().zip(&other.bits) {
            *a &= b;
        }
    }
}

/// Where a zone's cells lie: cell (i, j) has its centre at
/// `(x0 + (i + ½)h, y0 + (j + ½)h)`, mm in the setup's frame.
struct Cells {
    x0: f64,
    y0: f64,
    h: f64,
    nx: usize,
    ny: usize,
}
/// Narrow `span` to the `x` with `k·x + c` in `lo..=hi`.
fn clip(span: &mut (f64, f64), k: f64, c: f64, lo: f64, hi: f64) {
    if k.abs() < 1e-12 {
        if c < lo || c > hi {
            *span = (f64::INFINITY, f64::NEG_INFINITY);
        }
        return;
    }
    let (a, b) = ((lo - c) / k, (hi - c) / k);
    span.0 = span.0.max(a.min(b));
    span.1 = span.1.min(a.max(b));
}
impl Cells {
    fn centre(&self, i: usize, j: usize) -> P {
        (
            self.x0 + (i as f64 + 0.5) * self.h,
            self.y0 + (j as f64 + 0.5) * self.h,
        )
    }
    fn cell(&self, p: P) -> (isize, isize) {
        (
            ((p.0 - self.x0) / self.h).floor() as isize,
            ((p.1 - self.y0) / self.h).floor() as isize,
        )
    }
    /// The first and last cell of a row whose centres lie from `lo` to
    /// `hi` (mm across), if any.
    fn run(&self, lo: f64, hi: f64) -> Option<(usize, usize)> {
        let i0 = ((lo - self.x0) / self.h - 0.5).ceil().max(0.);
        let i1 = ((hi - self.x0) / self.h - 0.5)
            .floor()
            .min(self.nx as f64 - 1.);
        (i0 <= i1).then_some((i0 as usize, i1 as usize))
    }
    /// Each row's run of the cells whose centres lie within `r` of the
    /// segment `a`–`b`: the row, its first cell and its last.
    fn capsule(&self, a: P, b: P, r: f64, mut f: impl FnMut(usize, usize, usize)) {
        self.capsule_until(a, b, r, |j, i0, i1| {
            f(j, i0, i1);
            false
        });
    }
    /// `capsule`, stopping at the first row `f` says true for; whether one
    /// did.
    fn capsule_until(
        &self,
        a: P,
        b: P,
        r: f64,
        mut f: impl FnMut(usize, usize, usize) -> bool,
    ) -> bool {
        let j0 = ((a.1.min(b.1) - r - self.y0) / self.h - 0.5).ceil().max(0.) as usize;
        let j1 = ((a.1.max(b.1) + r - self.y0) / self.h - 0.5).floor();
        if j1 < 0. {
            return false;
        }
        let j1 = (j1 as usize).min(self.ny.saturating_sub(1));
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let l = dx.hypot(dy);
        for j in j0..=j1 {
            let cy = self.y0 + (j as f64 + 0.5) * self.h;
            let mut span = (f64::INFINITY, f64::NEG_INFINITY);
            for c in [a, b] {
                let d = cy - c.1;
                if d.abs() <= r {
                    let w = (r * r - d * d).sqrt();
                    span = (span.0.min(c.0 - w), span.1.max(c.0 + w));
                }
            }
            if l > 1e-9 {
                // Beside the segment: along it within its length, across it
                // within `r`.
                let mut strip = (f64::NEG_INFINITY, f64::INFINITY);
                clip(&mut strip, dx, -a.0 * dx + (cy - a.1) * dy, 0., l * l);
                clip(&mut strip, dy, -a.0 * dy - (cy - a.1) * dx, -r * l, r * l);
                if strip.0 <= strip.1 {
                    span = (span.0.min(strip.0), span.1.max(strip.1));
                }
            }
            if let Some((i0, i1)) = (span.0 <= span.1)
                .then(|| self.run(span.0, span.1))
                .flatten()
                && f(j, i0, i1)
            {
                return true;
            }
        }
        false
    }
    /// The cells whose centres lie inside `region` (each loop's inside to
    /// its left, holes clockwise).
    fn raster(&self, region: &[Polygon]) -> Bits {
        let mut bits = Bits::new(self.nx, self.ny);
        let mut rows: Vec<Vec<f64>> = vec![vec![]; self.ny];
        for path in region {
            for (a, b) in path
                .iter()
                .zip(path.iter().cycle().skip(1))
                .take(path.len())
            {
                let (lo, hi) = if a.1 < b.1 { (a, b) } else { (b, a) };
                // Rows whose centre line lies in [lo.y, hi.y).
                let j0 = ((lo.1 - self.y0) / self.h - 0.5).ceil().max(0.);
                let j1 = ((hi.1 - self.y0) / self.h - 0.5).ceil().min(self.ny as f64);
                for j in j0 as usize..j1.max(j0) as usize {
                    let cy = self.y0 + (j as f64 + 0.5) * self.h;
                    rows[j].push(lo.0 + (cy - lo.1) / (hi.1 - lo.1) * (hi.0 - lo.0));
                }
            }
        }
        for (j, xs) in rows.iter_mut().enumerate() {
            xs.sort_by(f64::total_cmp);
            for pair in xs.chunks_exact(2) {
                if let Some((i0, i1)) = self.run(pair[0], pair[1]) {
                    bits.fill(j, i0, i1);
                }
            }
        }
        bits
    }
}

/// The edges of a region, bucketed on a coarse grid, to tell quickly
/// whether a short segment crosses one.
struct Edges {
    x0: f64,
    y0: f64,
    size: f64,
    nx: usize,
    ny: usize,
    buckets: Vec<Vec<(P, P)>>,
}
impl Edges {
    fn new(region: &[Polygon], size: f64) -> Self {
        let (mut lo, mut hi) = (
            (f64::INFINITY, f64::INFINITY),
            (f64::NEG_INFINITY, f64::NEG_INFINITY),
        );
        for p in region.iter().flatten() {
            lo = (lo.0.min(p.0), lo.1.min(p.1));
            hi = (hi.0.max(p.0), hi.1.max(p.1));
        }
        let nx = (((hi.0 - lo.0) / size).ceil() as usize).max(1);
        let ny = (((hi.1 - lo.1) / size).ceil() as usize).max(1);
        let mut edges = Self {
            x0: lo.0,
            y0: lo.1,
            size,
            nx,
            ny,
            buckets: vec![vec![]; nx * ny],
        };
        for path in region {
            for (a, b) in path
                .iter()
                .zip(path.iter().cycle().skip(1))
                .take(path.len())
            {
                let ((i0, j0), (i1, j1)) = edges.span(*a, *b);
                for j in j0..=j1 {
                    for i in i0..=i1 {
                        edges.buckets[j * nx + i].push((*a, *b));
                    }
                }
            }
        }
        edges
    }
    /// The buckets the box round `a` and `b` overlaps, first and last.
    fn span(&self, a: P, b: P) -> ((usize, usize), (usize, usize)) {
        let at =
            |v: f64, o: f64, n: usize| (((v - o) / self.size).floor().max(0.) as usize).min(n - 1);
        (
            (
                at(a.0.min(b.0), self.x0, self.nx),
                at(a.1.min(b.1), self.y0, self.ny),
            ),
            (
                at(a.0.max(b.0), self.x0, self.nx),
                at(a.1.max(b.1), self.y0, self.ny),
            ),
        )
    }
    /// Whether the segment `a`–`b` crosses or touches an edge.
    fn crosses(&self, a: P, b: P) -> bool {
        let ((i0, j0), (i1, j1)) = self.span(a, b);
        (j0..=j1).any(|j| {
            (i0..=i1).any(|i| {
                self.buckets[j * self.nx + i]
                    .iter()
                    .any(|&(c, d)| pocket::meets(a, b, c, d))
            })
        })
    }
}

/// The exact Euclidean distance, in cells, from each cell's centre to the
/// nearest centre of a cell `seed` holds (Felzenszwalb and Huttenlocher's
/// two passes of the lower envelope of parabolas).
fn distances(cells: &Cells, seed: impl Fn(usize, usize) -> bool) -> Vec<f32> {
    const FAR: f64 = 1e20;
    let (nx, ny) = (cells.nx, cells.ny);
    let mut d = vec![0f64; nx * ny];
    for j in 0..ny {
        for i in 0..nx {
            d[j * nx + i] = if seed(i, j) { 0. } else { FAR };
        }
    }
    let n = nx.max(ny);
    let (mut f, mut out) = (vec![0f64; n], vec![0f64; n]);
    let (mut v, mut z) = (vec![0usize; n], vec![0f64; n + 1]);
    let mut pass = |f: &[f64], out: &mut [f64]| {
        let n = f.len();
        let mut k = 0;
        v[0] = 0;
        z[0] = f64::NEG_INFINITY;
        z[1] = f64::INFINITY;
        for q in 1..n {
            loop {
                let p = v[k];
                let s = ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64))
                    / (2. * q as f64 - 2. * p as f64);
                if s <= z[k] {
                    k -= 1;
                    continue;
                }
                k += 1;
                v[k] = q;
                z[k] = s;
                z[k + 1] = f64::INFINITY;
                break;
            }
        }
        k = 0;
        for (q, o) in out.iter_mut().enumerate().take(n) {
            while z[k + 1] < q as f64 {
                k += 1;
            }
            let p = v[k];
            *o = (q as f64 - p as f64).powi(2) + f[p];
        }
    };
    for i in 0..nx {
        for j in 0..ny {
            f[j] = d[j * nx + i];
        }
        pass(&f[..ny], &mut out[..ny]);
        for j in 0..ny {
            d[j * nx + i] = out[j];
        }
    }
    for j in 0..ny {
        f[..nx].copy_from_slice(&d[j * nx..(j + 1) * nx]);
        pass(&f[..nx], &mut out[..nx]);
        d[j * nx..(j + 1) * nx].copy_from_slice(&out[..nx]);
    }
    d.into_iter().map(|v| v.sqrt() as f32).collect()
}

/// How the tool gets to the next place it cuts at a level.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Lift {
    /// At depth, through what is already cut.
    Stay,
    /// Up to just over the level before, across inside the pocket (at the
    /// feed below the stock top) and down.
    Low,
    /// Up to clearance, across and down.
    High,
}
#[derive(Debug, Clone, PartialEq)]
enum Event {
    /// To `to` by `lift`; with `above`, the tool stops at the entry height
    /// over it, for an entry. `slow` descends at the entry feed: material
    /// is under the tool there.
    Link {
        to: P,
        lift: Lift,
        above: bool,
        slow: bool,
    },
    /// A helix of `radius` round `centre` from the angle `from`, turning
    /// `turn` (1 anticlockwise, -1 clockwise), down to the level and once
    /// round there; a plunge at `centre` for radius 0.
    Enter {
        centre: P,
        radius: f64,
        from: f64,
        turn: f64,
    },
    /// Cut through these points at depth, from where the tool is.
    Cut(Vec<P>),
    /// A pass round a wall through these points at depth, from where the
    /// tool is, each move at its share of the feed: less where it would
    /// take more than the load, in a corner.
    Wall(Vec<P>, Vec<f32>),
}

/// What the walker's next step is.
enum Next {
    /// Its heading, where it ends and the engagement it takes.
    Go(f64, P, f64),
    /// Every way on cuts next to nothing.
    Starved,
    /// Every way on leaves the allowed region.
    Blocked,
    /// Every way on would overload the cutter.
    Overloaded,
}
/// The walker's settings, the same at every level of a zone.
struct Walk {
    cells: Cells,
    /// Where the tool centre may stand: well inside the allowed region.
    allowed: Bits,
    /// The edges of the exact allowed region, which no step may cross.
    edges: Edges,
    /// The material the walker must clear: more than the load inside what
    /// the walls' pass reaches, so that pass takes at most the load.
    inner: Bits,
    /// How far each allowed cell's centre lies from the nearest one that is
    /// not, in cells: the room an entry helix has.
    room: Vec<f32>,
    r: f64,
    load: f64,
    delta: f64,
    turn: f64,
    /// The spacing, in cells, of the places weighed as a pass's start:
    /// about a step.
    lattice: usize,
    /// 1 to keep the material on the right of travel (climb milling with
    /// the spindle clockwise), -1 on the left.
    side: f64,
    helix: bool,
}
/// One level of a zone: the walk and the material it still holds.
struct Level<'a> {
    walk: &'a Walk,
    material: Bits,
    at: P,
    events: Vec<Event>,
    steps: usize,
    /// While a pass is tried: each word of `material` it changes, as it
    /// was.
    undo: Option<Vec<(usize, u64)>>,
    /// Which way the last pass was going as it ended.
    heading: Option<f64>,
    /// The cells a pass began at and took no step from.
    barren: std::collections::HashSet<(isize, isize)>,
}
impl Level<'_> {
    fn count(&self, a: P, b: P, r: f64) -> u32 {
        let mut n = 0;
        self.walk
            .cells
            .capsule(a, b, r, |j, i0, i1| n += self.material.count(j, i0, i1));
        n
    }
    /// Whether any of the material the walker must clear lies within `r`
    /// of `p`.
    fn inner(&self, p: P, r: f64) -> bool {
        let w = self.walk;
        w.cells.capsule_until(p, p, r, |j, i0, i1| {
            self.material.count_and(&w.inner, j, i0, i1) > 0
        })
    }
    /// Whether any material lies within `r` of `p`.
    fn any(&self, p: P, r: f64) -> bool {
        self.walk
            .cells
            .capsule_until(p, p, r, |j, i0, i1| self.material.count(j, i0, i1) > 0)
    }
    /// Whether any material the walker must clear is left.
    fn unfinished(&self) -> bool {
        self.material
            .bits
            .iter()
            .zip(&self.walk.inner.bits)
            .any(|(a, b)| a & b != 0)
    }
    fn remove(&mut self, a: P, b: P) -> u32 {
        let mut n = 0;
        let (material, undo) = (&mut self.material, &mut self.undo);
        self.walk.cells.capsule(a, b, self.walk.r, |j, i0, i1| {
            if let Some(log) = undo.as_mut() {
                for (w, _) in Bits::masks(i0, i1) {
                    let k = j * material.words + w;
                    log.push((k, material.bits[k]));
                }
            }
            n += material.clear(j, i0, i1)
        });
        n
    }
    fn ok(&self, p: P) -> bool {
        let (i, j) = self.walk.cells.cell(p);
        self.walk.allowed.get(i, j)
    }
    /// The material the tool removes per mm stepping from `p` to `q`, mm:
    /// its radial engagement on a straight cut, the most over any of the
    /// step's `PIECES` (a step that runs through air and then into the
    /// material takes all of it at its end). Infinite where `q` is not
    /// allowed.
    fn engagement(&self, p: P, q: P) -> f64 {
        if !self.ok(q) || self.walk.edges.crosses(p, q) {
            return f64::INFINITY;
        }
        let h = self.walk.cells.h;
        let (mut before, mut most) = (0, 0);
        for k in 1..=PIECES {
            let t = k as f64 / PIECES as f64;
            let to = (p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1));
            let upto = self.count(p, to, self.walk.r);
            most = most.max(upto - before);
            before = upto;
        }
        most as f64 * h * h / (self.walk.delta / PIECES as f64)
    }
    /// The walker's next step from `p` heading `heading`, turning at most
    /// `turn` either way: the turn toward the material that removes the
    /// load, else the nearest it comes to that. Once `settled`, a step
    /// every way on would overload the cutter is not taken.
    fn step(&self, p: P, heading: f64, turn: f64, settled: bool) -> Next {
        let w = self.walk;
        let at = |a: f64| {
            let t = heading - w.side * a;
            let q = (p.0 + w.delta * t.cos(), p.1 + w.delta * t.sin());
            (t, q, self.engagement(p, q))
        };
        let tries: Vec<(f64, (f64, P, f64))> = (0..DIRECTIONS)
            .map(|i| {
                let a = -turn + 2. * turn * i as f64 / (DIRECTIONS - 1) as f64;
                (a, at(a))
            })
            .collect();
        // Where the engagement rises past the load turning toward the
        // material, the crossing nearest straight on.
        let crossing = (0..DIRECTIONS - 1)
            .filter(|&i| tries[i].1.2 <= w.load && tries[i + 1].1.2 > w.load)
            .min_by(|&a, &b| tries[a].0.abs().total_cmp(&tries[b].0.abs()));
        if let Some(i) = crossing {
            let (mut lo, mut hi) = ((tries[i].0, tries[i].1), tries[i + 1].0);
            for _ in 0..REFINE {
                let mid = (lo.0 + hi) / 2.;
                let m = at(mid);
                if m.2 <= w.load {
                    lo = (mid, m);
                } else {
                    hi = mid;
                }
            }
            let (t, q, e) = lo.1;
            return if e >= STARVED * w.load {
                Next::Go(t, q, e)
            } else {
                Next::Starved
            };
        }
        let finite = tries.iter().filter(|t| t.1.2.is_finite());
        if tries.iter().any(|t| t.1.2.is_finite() && t.1.2 > w.load) {
            // Every way on takes more than the load: the least.
            let Some(least) = finite.min_by(|a, b| a.1.2.total_cmp(&b.1.2)) else {
                return Next::Blocked;
            };
            if settled && least.1.2 > OVERLOAD * w.load {
                return Next::Overloaded;
            }
            Next::Go(least.1.0, least.1.1, least.1.2)
        } else {
            // Every way on takes less: the most, if any.
            let Some(most) = finite.max_by(|a, b| a.1.2.total_cmp(&b.1.2)) else {
                return Next::Blocked;
            };
            if most.1.2 < STARVED * w.load {
                return Next::Starved;
            }
            Next::Go(most.1.0, most.1.1, most.1.2)
        }
    }
    /// Walk from where the tool is, heading `heading`, until the material
    /// runs out or the next step would overload the cutter.
    fn pass(&mut self, heading: f64, settled: bool, budget: usize) -> Result<Vec<P>, String> {
        let (path, _) = self.walk(heading, settled, usize::MAX, budget)?;
        Ok(path)
    }
    /// `pass`, at most `limit` steps: the path and the cells it cleared.
    fn walk(
        &mut self,
        heading: f64,
        mut settled: bool,
        limit: usize,
        budget: usize,
    ) -> Result<(Vec<P>, u64), String> {
        let mut path = vec![self.at];
        let mut heading = heading;
        let mut cleared = 0u64;
        // Steps since the last that cut anything.
        let mut dry = 0;
        // A pass begins standing still: its first step may turn a right
        // angle either way, as sharply as it must at a wall.
        let mut turn = std::f64::consts::FRAC_PI_2;
        while path.len() <= limit {
            let mut next = self.step(self.at, heading, turn, settled);
            // A wall, or a corner too tight for the Minimum cutting radius,
            // turns the walker as sharply as it must to keep near the load.
            let heavy = match next {
                Next::Go(_, _, e) => e > 1.25 * self.walk.load,
                Next::Blocked | Next::Overloaded => true,
                Next::Starved => false,
            };
            if turn < std::f64::consts::PI && heavy {
                let sharper = self.step(self.at, heading, std::f64::consts::PI, settled);
                next = match (&next, &sharper) {
                    (Next::Go(_, _, e), Next::Go(_, _, f)) if f >= e => next,
                    (_, Next::Go(..)) => sharper,
                    _ => next,
                };
            }
            let Next::Go(t, q, e) = next else {
                break;
            };
            turn = self.walk.turn;
            if self.undo.is_none() {
                self.steps += 1;
                if self.steps > budget {
                    return Err("Adaptive clearing exceeds its step limit. Increase the optimal load or use a larger tool.".into());
                }
            }
            let n = self.remove(self.at, q);
            cleared += n as u64;
            if n == 0 {
                dry += 1;
            } else {
                dry = 0;
            }
            settled |= e <= 1.25 * self.walk.load;
            path.push(q);
            self.at = q;
            heading = t;
            if dry > DRY {
                break;
            }
        }
        // The steps that cut nothing at the end are left out.
        path.truncate(path.len() - dry);
        self.at = *path.last().unwrap();
        if path.len() > 1 && self.undo.is_none() {
            let (a, b) = (path[path.len() - 2], path[path.len() - 1]);
            self.heading = Some((b.1 - a.1).atan2(b.0 - a.0));
        }
        Ok((path, cleared))
    }
    /// How much of the load a pass from `p` heading `heading` takes over
    /// its first `PROMISE` tool radii, tried and undone: near 1 sweeping a
    /// long front, little for a hook into a wall, a slot's dead end or a
    /// crumb.
    fn promise(&mut self, p: P, heading: f64, settled: bool) -> f64 {
        let w = self.walk;
        let steps = (PROMISE * w.r / w.delta).ceil() as usize;
        let at = self.at;
        self.at = p;
        self.undo = Some(vec![]);
        let cleared = self
            .walk(heading, settled, steps, usize::MAX)
            .map_or(0, |(_, n)| n);
        for (k, v) in self.undo.take().unwrap().into_iter().rev() {
            self.material.bits[k] = v;
        }
        self.at = at;
        cleared as f64 * w.cells.h * w.cells.h / (steps as f64 * w.delta * w.load)
    }
    /// Whether the tool may stand at cell `(i, j)` clear of the material
    /// and within `w.lattice` cells of what the walker must clear: where a
    /// pass can begin.
    fn touches(&self, i: usize, j: usize) -> bool {
        let w = self.walk;
        let p = w.cells.centre(i, j);
        w.allowed.get(i as isize, j as isize)
            && !self.barren.contains(&(i as isize, j as isize))
            && !self.any(p, w.r + 0.5 * w.cells.h)
            && self.inner(p, w.r + (w.lattice as f64 + 0.5) * w.cells.h)
    }
    /// The heading at `p` that keeps the material beside it on the
    /// walker's side.
    fn along(&self, p: P) -> f64 {
        let w = self.walk;
        let mut g = (0., 0.);
        w.cells.capsule(p, p, w.r + 2.5 * w.cells.h, |j, i0, i1| {
            for i in i0..=i1 {
                if self.material.get(i as isize, j as isize) {
                    let c = w.cells.centre(i, j);
                    g = (g.0 + c.0 - p.0, g.1 + c.1 - p.1);
                }
            }
        });
        (w.side * g.0).atan2(-w.side * g.1)
    }
    /// How far `p` is from the tool, counted three times over behind the
    /// way the last pass went: the walk goes on round the front.
    fn ahead(&self, p: P) -> f64 {
        let (dx, dy) = (p.0 - self.at.0, p.1 - self.at.1);
        let d = dx.hypot(dy);
        match self.heading {
            Some(t) if dx * t.cos() + dy * t.sin() < 0. => 3. * d,
            _ => d,
        }
    }
    /// Where the next pass begins, its heading and whether it begins
    /// settled: the nearest place (`ahead`) whose pass promises at least
    /// half the load (`promise`); else the most promising place; else,
    /// where every place would overload the cutter, the one that overloads
    /// it least. None where only crumbs are left: the walls' pass and the
    /// levels below take them.
    fn next_start(&mut self) -> Option<(P, f64, bool)> {
        let w = self.walk;
        // A place's heading and first step's engagement, if it has one, as
        // `pass` takes it.
        let judge = |level: &Self, p: P| {
            let heading = level.along(p);
            let mut next = level.step(p, heading, std::f64::consts::FRAC_PI_2, false);
            if matches!(next, Next::Blocked) {
                next = level.step(p, heading, std::f64::consts::PI, false);
            }
            match next {
                Next::Go(_, _, e) => Some((e, heading)),
                _ => None,
            }
        };
        let mut best: Option<(f64, P, f64)> = None;
        let mut overloaded: Option<(f64, P, f64)> = None;
        let (mut judged, mut tried) = (0, 0);
        // One place weighed in each block a step wide.
        let mut seen = std::collections::HashSet::new();
        let block = |p: P| {
            (
                (p.0 / w.delta).floor() as i64,
                (p.1 / w.delta).floor() as i64,
            )
        };
        let mut weigh = |level: &mut Self, candidates: Vec<(f64, P)>| {
            for (_, p) in candidates {
                if judged >= START_JUDGED || tried >= START_TRIES {
                    break;
                }
                if !seen.insert(block(p)) {
                    continue;
                }
                judged += 1;
                let Some((e, heading)) = judge(level, p) else {
                    continue;
                };
                if e > 1.25 * w.load {
                    if overloaded.is_none_or(|o| e < o.0) {
                        overloaded = Some((e, p, heading));
                    }
                    continue;
                }
                tried += 1;
                let promise = level.promise(p, heading, true);
                if promise >= 0.5 {
                    return Some((p, heading, true));
                }
                if best.is_none_or(|b| promise > b.0) {
                    best = Some((promise, p, heading));
                }
            }
            None
        };
        // Over the whole grid: the distances to the material, on cells a
        // lattice spacing wide (16 times fewer), point to the blocks whose
        // middle cell may touch; each is then tried exactly.
        let k = w.lattice;
        let coarse = Cells {
            x0: w.cells.x0,
            y0: w.cells.y0,
            h: w.cells.h * k as f64,
            nx: w.cells.nx.div_ceil(k),
            ny: w.cells.ny.div_ceil(k),
        };
        let (mut solid, mut must) = (
            vec![false; coarse.nx * coarse.ny],
            vec![false; coarse.nx * coarse.ny],
        );
        for j in 0..w.cells.ny {
            for bi in 0..coarse.nx {
                let (i0, i1) = (bi * k, ((bi + 1) * k - 1).min(w.cells.nx - 1));
                let at = (j / k) * coarse.nx + bi;
                if !solid[at] && self.material.count(j, i0, i1) > 0 {
                    solid[at] = true;
                }
                if !must[at] && self.material.count_and(&w.inner, j, i0, i1) > 0 {
                    must[at] = true;
                }
            }
        }
        let gap = distances(&coarse, |i, j| solid[j * coarse.nx + i]);
        let near = distances(&coarse, |i, j| must[j * coarse.nx + i]);
        // How far, in blocks, a block's middle may be from its cells.
        let slack = 1.5;
        let (lo, hi) = (
            (w.r / coarse.h) as f32 - slack,
            ((w.r + (k as f64 + 0.5) * w.cells.h) / coarse.h) as f32 + slack,
        );
        let mut all = vec![];
        for bj in 0..coarse.ny {
            for bi in 0..coarse.nx {
                let at = bj * coarse.nx + bi;
                if gap[at] > lo && near[at] <= hi {
                    let (i, j) = (
                        (bi * k + k / 2).min(w.cells.nx - 1),
                        (bj * k + k / 2).min(w.cells.ny - 1),
                    );
                    if self.touches(i, j) {
                        let p = w.cells.centre(i, j);
                        all.push((self.ahead(p), p));
                    }
                }
            }
        }
        all.sort_by(|a, b| a.0.total_cmp(&b.0));
        if let Some(start) = weigh(self, all) {
            return Some(start);
        }
        match (best, overloaded) {
            (Some((promise, p, heading)), _) if promise >= CRUMB => Some((p, heading, true)),
            (_, Some((_, p, heading))) => Some((p, heading, false)),
            _ => None,
        }
    }
    /// Where to enter material no pass can reach from beside it: the
    /// allowed cell with the most room in solid material, and the helix
    /// radius that room gives (0 to plunge).
    fn entry(&self) -> Option<(P, f64)> {
        let w = self.walk;
        // Solid material at least half the tool radius all round, as
        // untouched material is; no crumb.
        let depth = distances(&w.cells, |i, j| !self.material.get(i as isize, j as isize));
        let solid = (0.5 * w.r / w.cells.h) as f32;
        let mut best: Option<(f32, usize, usize)> = None;
        for j in 0..w.cells.ny {
            for i in 0..w.cells.nx {
                let k = j * w.cells.nx + i;
                if w.allowed.get(i as isize, j as isize)
                    && depth[k] >= solid
                    && w.inner.get(i as isize, j as isize)
                    && best.is_none_or(|b| w.room[k] > b.0)
                {
                    best = Some((w.room[k], i, j));
                }
            }
        }
        let (room, i, j) = best?;
        let radius = if w.helix {
            (0.5 * w.r).min((room as f64 - 1.) * w.cells.h)
        } else {
            0.
        };
        Some((
            w.cells.centre(i, j),
            if radius < 0.1 * w.r { 0. } else { radius },
        ))
    }
    /// How the tool gets from where it is to `to`.
    fn lift(&self, to: P, exact: &[Polygon]) -> Lift {
        let w = self.walk;
        let length = (to.0 - self.at.0).hypot(to.1 - self.at.1);
        if length <= STAY_DOWN * w.r {
            // Grazing up to a few cells of what is left beside the way, as
            // a pass's end can.
            let clear = (w.r - 2.5 * w.cells.h).max(0.);
            let n = (length / (0.5 * w.cells.h)).ceil().max(1.) as usize;
            let free = (0..=n).all(|k| {
                let t = k as f64 / n as f64;
                let q = (
                    self.at.0 + t * (to.0 - self.at.0),
                    self.at.1 + t * (to.1 - self.at.1),
                );
                self.count(q, q, clear) == 0
            });
            if free && pocket::inside(exact, self.at) && !w.edges.crosses(self.at, to) {
                return Lift::Stay;
            }
        }
        if pocket::inside(exact, self.at)
            && pocket::inside(exact, to)
            && !w.edges.crosses(self.at, to)
        {
            Lift::Low
        } else {
            Lift::High
        }
    }
    fn link(&mut self, to: P, exact: &[Polygon], above: bool, first: &mut bool) {
        let lift = if std::mem::take(first) {
            Lift::High
        } else {
            self.lift(to, exact)
        };
        let slow = !above && self.count(to, to, self.walk.r) > 0;
        self.events.push(Event::Link {
            to,
            lift,
            above,
            slow,
        });
        self.at = to;
    }
    /// Clear the level: passes from beside what is cut, entries where none
    /// can reach, then the walls (`walls`, each loop to cut with the
    /// material on the walker's side). False when it holds no material.
    fn clear(
        &mut self,
        walls: &[Polygon],
        exact: &[Polygon],
        budget: usize,
    ) -> Result<bool, String> {
        if self.material.total() == 0 {
            return Ok(false);
        }
        let w = self.walk;
        let mut first = true;
        while self.unfinished() {
            if let Some((p, heading, settled)) = self.next_start() {
                self.link(p, exact, false, &mut first);
                let path = self.pass(heading, settled, budget)?;
                if path.len() == 1 {
                    // Nothing came of it: the walk moves on and does not
                    // begin there again.
                    self.barren.insert(w.cells.cell(p));
                    continue;
                }
                self.events.push(Event::Cut(path));
            } else if let Some((centre, radius)) = self.entry() {
                let from = 0f64;
                let start = (centre.0 + radius, centre.1);
                self.link(start, exact, true, &mut first);
                // Round the helix with the material outside it on the
                // walker's side.
                let turn = w.side;
                let segments = 64;
                let mut last = start;
                for k in 1..=segments {
                    let a = from + turn * std::f64::consts::TAU * k as f64 / segments as f64;
                    let q = (centre.0 + radius * a.cos(), centre.1 + radius * a.sin());
                    self.remove(last, q);
                    last = q;
                }
                self.remove(centre, centre);
                self.events.push(Event::Enter {
                    centre,
                    radius,
                    from,
                    turn,
                });
                self.at = start;
                let heading = if radius > 0. {
                    from + turn * std::f64::consts::FRAC_PI_2
                } else {
                    0.
                };
                self.heading = Some(heading);
                let path = self.pass(heading, radius > 0., budget)?;
                if path.len() > 1 {
                    self.events.push(Event::Cut(path));
                }
            } else {
                break;
            }
        }
        for wall in walls {
            let (_, start, edge) = pocket::nearest(wall, self.at);
            let mut around = pocket::begun(wall, start, edge);
            around.push(start);
            // Come down a load and two cells off the wall on the cleared
            // side and feed onto it, rather than beside what is left.
            let next = around
                .iter()
                .copied()
                .find(|q| (q.0 - start.0).hypot(q.1 - start.1) > 1e-6);
            let lead = next.and_then(|q| {
                let (dx, dy) = (q.0 - start.0, q.1 - start.1);
                let l = dx.hypot(dy);
                let off = w.load + 2. * w.cells.h;
                let p = (
                    start.0 - w.side * dy / l * off,
                    start.1 + w.side * dx / l * off,
                );
                (self.ok(p) && !w.edges.crosses(p, start)).then_some(p)
            });
            let mut points = vec![];
            if let Some(p) = lead {
                points.push(p);
            }
            points.extend(around);
            self.link(points[0], exact, false, &mut first);
            // Each move a step long at most, slowed where it takes more
            // than the load so the cutter removes no more per minute.
            let mut fine = vec![points[0]];
            let mut shares = vec![];
            for pair in points.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let l = (b.0 - a.0).hypot(b.1 - a.1);
                let n = (l / w.delta).ceil().max(1.) as usize;
                for k in 1..=n {
                    let t = k as f64 / n as f64;
                    let q = (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
                    let from = *fine.last().unwrap();
                    // The most it takes over any piece of the move.
                    let length = (q.0 - from.0).hypot(q.1 - from.1).max(1e-9);
                    let (mut before, mut most) = (0, 0);
                    for k in 1..=PIECES {
                        let t = k as f64 / PIECES as f64;
                        let to = (from.0 + t * (q.0 - from.0), from.1 + t * (q.1 - from.1));
                        let upto = self.count(from, to, w.r);
                        most = most.max(upto - before);
                        before = upto;
                    }
                    self.remove(from, q);
                    let e = most as f64 * w.cells.h * w.cells.h / (length / PIECES as f64);
                    shares.push(if e > WALL_LOAD * w.load {
                        (WALL_LOAD * w.load / e).max(SLOWEST) as f32
                    } else {
                        1.
                    });
                    fine.push(q);
                }
            }
            self.events.push(Event::Wall(fine, shares));
            self.at = start;
        }
        Ok(true)
    }
}

/// `points` with those that lie within `tolerance` of the line through
/// their neighbours left out (Douglas–Peucker); the ends stay.
fn simplify(points: &[P], tolerance: f64) -> Vec<P> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0, points.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (p, q) = (points[a], points[b]);
        let (dx, dy) = (q.0 - p.0, q.1 - p.1);
        let l = dx.hypot(dy);
        let mut worst = (0., a);
        for (k, r) in points.iter().enumerate().take(b).skip(a + 1) {
            let d = if l < 1e-12 {
                (r.0 - p.0).hypot(r.1 - p.1)
            } else {
                ((r.0 - p.0) * dy - (r.1 - p.1) * dx).abs() / l
            };
            if d > worst.0 {
                worst = (d, k);
            }
        }
        if worst.0 > tolerance {
            keep[worst.1] = true;
            stack.push((a, worst.1));
            stack.push((worst.1, b));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

/// Moves appended to a plan from where the tool is.
struct Motions {
    plan: Plan,
    at: [f32; 3],
}
impl Motions {
    fn push(
        &mut self,
        to: [f32; 3],
        motion: Motion,
        rate: f32,
        cutting: bool,
    ) -> Result<(), String> {
        if self.plan.moves.len() >= MAX_MOVES {
            return Err("Adaptive clearing exceeds 300,000 moves. Increase the optimal load or the stepdown, or use a larger cutter.".into());
        }
        let distance = (0..3)
            .map(|i| (self.at[i] - to[i]).powi(2))
            .sum::<f32>()
            .sqrt();
        if distance > 0.00001 {
            let end = self.plan.duration + distance / rate * 60.;
            self.plan.moves.push(Move {
                from: self.at,
                to,
                motion,
                cutting_pass: cutting,
                feed: rate,
                start: self.plan.duration,
                end,
            });
            self.plan.duration = end;
        }
        self.at = to;
        Ok(())
    }
}

pub fn generate(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    let job = pocket::job(setup, op, tool, mesh)?;
    let r = job.radius as f64;
    let load = num(&op.p, "optimal_load").ok_or("Enter a finite optimal load value.")? as f64;
    if !(load >= 0.04 * r && load <= r) {
        return Err(format!(
            "Optimal load must be from 2% to 50% of the tool diameter: {:.3} to {:.3} mm for this tool.",
            0.04 * r,
            r
        ));
    }
    let minimum =
        num(&op.p, "minimum_radius").ok_or("Enter a finite minimum cutting radius value.")? as f64;
    if !(0. ..=r).contains(&minimum) {
        return Err(format!(
            "Minimum cutting radius must be from 0 to the tool radius, {r:.3} mm."
        ));
    }
    let tolerance = num(&op.p, "tolerance")
        .filter(|t| *t > 0.)
        .ok_or("Enter a positive tolerance.")? as f64;
    let entry = get(&op.p, "ramp");
    if !ADAPTIVE_ENTRIES.contains(&entry) {
        return Err(
            "Adaptive clearing enters with a Helix or a Plunge; choose one in Linking.".into(),
        );
    }
    let helix = entry == ADAPTIVE_ENTRIES[0];
    let slope = if helix {
        let angle = num(&op.p, "ramp_angle").ok_or("Enter a finite ramp angle value.")?;
        if !(0.1..=15.).contains(&angle) {
            return Err("Adaptive clearing helix ramp angle must be between 0.1° and 15°.".into());
        }
        angle.to_radians().tan()
    } else {
        0.
    };
    let clockwise = facing_setting(&op.p, "spindle_rotation") == "Clockwise (CW)";
    let climb = pocket_setting(&op.p, "pocket_milling") == "Climb";
    let side = if clockwise == climb { 1. } else { -1. };
    let rest = setup.preview_remaining.as_deref();
    let feed = job.feed;
    let stepdown = job.stepdown;
    let margin = job.margin as f64;
    // Each zone's allowed centres and first cutting height.
    let mut zones = vec![];
    for zone in &job.zones {
        let centres = zone.centres.at(0.)?;
        if !centres.contains_points() {
            continue;
        }
        let top = pocket::zone_start(setup, op, job.top, job.radius, zone, &centres);
        zones.push((zone, centres, top));
    }
    if zones.is_empty() {
        return Err("No finite adaptive motion could be generated.".into());
    }
    let highest = zones.iter().map(|z| z.2).fold(f32::NEG_INFINITY, f32::max);
    let total_levels: usize = zones
        .iter()
        .map(|(zone, _, top)| ((top - zone.target - 0.0001) / stepdown).ceil().max(1.) as usize)
        .sum();
    if total_levels > 1000 {
        return Err("Adaptive clearing exceeds 1,000 depth levels. Increase the stepdown.".into());
    }
    let mut out = Motions {
        plan: Plan {
            shape: crate::simulation::CutterShape::Flat,
            strategy: Strategy::Adaptive,
            operation: op.id.clone(),
            setup: setup.id.clone(),
            moves: vec![],
            stock: job.stock,
            radius: job.radius,
            flute_length: job.flute_length,
            tool_length: job.tool_length,
            holder_radius: job.holder_radius,
            display_scale: job.scale,
            duration: 0.,
            target: zones
                .iter()
                .map(|z| z.0.target)
                .fold(f32::INFINITY, f32::min),
            levels: total_levels,
            rest_top: (highest < job.top - 0.0001).then_some(highest),
            spindle_clockwise: clockwise,
            milling_direction: if climb { "Climb" } else { "Conventional" },
        },
        at: [0., 0., job.clearance],
    };
    let approach = (highest + num(&op.p, "retract").unwrap_or(0.)).min(job.retract);
    let clearance = job.clearance;
    let mut placed = false;
    for (zone, centres, cutting_top) in zones {
        // Cells a quarter of the load wide, at most a sixth of the radius,
        // and few enough for the grid.
        let reach_by = r - margin;
        let reach = centres.inflate(reach_by, JoinType::Round, EndType::Polygon, 2.);
        let outline: Vec<Polygon> = reach.clone().into();
        let (mut lo, mut hi) = (
            (f64::INFINITY, f64::INFINITY),
            (f64::NEG_INFINITY, f64::NEG_INFINITY),
        );
        for p in outline.iter().flatten() {
            lo = (lo.0.min(p.0), lo.1.min(p.1));
            hi = (hi.0.max(p.0), hi.1.max(p.1));
        }
        let mut h = (load / 4.).min(r / 6.);
        while (hi.0 - lo.0) * (hi.1 - lo.1) / (h * h) > MAX_CELLS {
            h *= 1.25;
        }
        let tol = tolerance.clamp(0.001, h / 2.);
        let extent = job.stock;
        let stock: Vec<Polygon> = vec![vec![
            (extent.min[0] as f64, extent.min[1] as f64),
            (extent.max[0] as f64, extent.min[1] as f64),
            (extent.max[0] as f64, extent.max[1] as f64),
            (extent.min[0] as f64, extent.max[1] as f64),
        ]];
        // Where the walker may go: the zone's centres and, past an edge
        // where the floor runs off the stock, a few cells further out, so
        // the tool can stand clear of the stock there and come in from the
        // side. Outside the stock is neither material nor part.
        let walkable = match &zone.centres {
            pocket::Centres::Floor {
                area,
                above,
                reach,
                keepout,
            } => {
                let wider = pocket::Centres::Floor {
                    area: area.clone(),
                    above: above.clone(),
                    reach: reach + 3. * h + tol + margin,
                    keepout: *keepout,
                }
                .at(0.)?;
                let outside = difference::<Milli>(wider, stock.clone(), FillRule::NonZero)
                    .map_err(|e| e.to_string())?;
                union::<Milli>(centres.clone(), outside, FillRule::NonZero)
                    .map_err(|e| e.to_string())?
            }
            pocket::Centres::Region(_) => centres.clone(),
        };
        let bounds: Vec<Polygon> = walkable
            .inflate(r, JoinType::Round, EndType::Polygon, 2.)
            .into();
        for p in bounds.iter().flatten() {
            lo = (lo.0.min(p.0), lo.1.min(p.1));
            hi = (hi.0.max(p.0), hi.1.max(p.1));
        }
        let cells = Cells {
            x0: lo.0 - 2. * h,
            y0: lo.1 - 2. * h,
            h,
            nx: ((hi.0 - lo.0) / h).ceil() as usize + 4,
            ny: ((hi.1 - lo.1) / h).ceil() as usize + 4,
        };
        // The walker stays a cell and the tolerance inside where it may
        // go, so its simplified path does too; the material it can reach
        // from there, less a cell, is what it clears. The walls' pass takes
        // the rest.
        let deep: Vec<Polygon> = walkable
            .inflate(-(0.75 * h + tol), JoinType::Round, EndType::Polygon, 2.)
            .into();
        let allowed = cells.raster(&deep);
        let room = distances(&cells, |i, j| !allowed.get(i as isize, j as isize));
        // All the material the tool can reach, so the walker counts what
        // it cuts beside a wall truly.
        let reachable: Vec<Polygon> = centres
            .inflate(r, JoinType::Round, EndType::Polygon, 2.)
            .into();
        let mut within = cells.raster(&reachable);
        within.and(&cells.raster(&stock));
        let inner: Vec<Polygon> = centres
            .inflate(r - load, JoinType::Round, EndType::Polygon, 2.)
            .into();
        let mut inner = cells.raster(&inner);
        inner.and(&within);
        // Clear of every wall and island, half the margin outside where
        // the tool centre may go, so a loop on that limit is still inside.
        let exact: Vec<Polygon> = walkable
            .inflate(margin / 2., JoinType::Round, EndType::Polygon, 2.)
            .into();
        let edges = Edges::new(&exact, 2. * r);
        let walk = Walk {
            cells,
            allowed,
            edges,
            inner,
            room,
            r,
            load,
            delta: (2.5 * h).max(load.min(r / 3.)),
            turn: if minimum > 0. {
                ((2.5 * h).max(load.min(r / 3.)) / minimum).min(MAX_TURN)
            } else {
                MAX_TURN
            },
            lattice: ((2.5 * h).max(load.min(r / 3.)) / h).floor().max(1.) as usize,
            side,
            helix,
        };
        // The walls, each with the material on the walker's side:
        // Clipper's outlines run anticlockwise and its holes clockwise,
        // the free area on their left.
        let mut walls: Vec<Polygon> = centres.clone().into();
        if side < 0. {
            for wall in &mut walls {
                wall.reverse();
            }
        }
        let near = pocket::envelope(&centres, job.radius);
        let area = (hi.0 - lo.0) * (hi.1 - lo.1);
        let budget = (20. * area / (load * walk.delta)) as usize + 20_000;
        let levels = ((cutting_top - zone.target - 0.0001) / stepdown)
            .ceil()
            .max(1.) as usize;
        for level in 1..=levels {
            let z = (cutting_top - level as f32 * stepdown).max(zone.target);
            let previous_z = (cutting_top - (level - 1) as f32 * stepdown).max(zone.target);
            let mut material = within.clone();
            if let Some(rest) = rest {
                for outline in rest.cleared(near, z, previous_z) {
                    for (j, i0, i1) in runs(&walk.cells, &outline) {
                        material.clear(j, i0, i1);
                    }
                }
            }
            let mut level_walk = Level {
                walk: &walk,
                material,
                at: (out.at[0] as f64, out.at[1] as f64),
                events: vec![],
                steps: 0,
                undo: None,
                heading: None,
                barren: Default::default(),
            };
            if !level_walk.clear(&walls, &exact, budget)? {
                continue;
            }
            if !placed {
                // The plan starts over its first cut, at clearance.
                if let Some(Event::Link { to, .. }) = level_walk.events.first() {
                    out.at = [to.0 as f32, to.1 as f32, clearance];
                }
                placed = true;
            }
            let entry = if level == 1 {
                approach
            } else {
                (previous_z + ENTRY).min(approach)
            };
            emit(
                &mut out,
                &level_walk.events,
                &exact,
                &walk.edges,
                Heights {
                    z,
                    previous: previous_z,
                    entry,
                    clearance,
                },
                Feeds { feed, slope },
                tol,
            )?;
            out.push(
                [out.at[0], out.at[1], clearance],
                Motion::Rapid,
                6000.,
                false,
            )?;
        }
    }
    if out.plan.moves.is_empty() || !out.plan.duration.is_finite() {
        return Err("No material is left for this adaptive clearing to cut.".into());
    }
    Ok(out.plan)
}

/// The rows of cells inside the convex `outline`: each row, its first cell
/// and its last.
fn runs(cells: &Cells, outline: &[(f64, f64)]) -> Vec<(usize, usize, usize)> {
    let mut out = vec![];
    if outline.len() < 3 {
        return out;
    }
    let (lo, hi) = outline
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.1), hi.max(p.1))
        });
    let j0 = ((lo - cells.y0) / cells.h - 0.5).ceil().max(0.) as usize;
    let j1 = ((hi - cells.y0) / cells.h - 0.5).floor();
    if j1 < 0. {
        return out;
    }
    for j in j0..=(j1 as usize).min(cells.ny.saturating_sub(1)) {
        let cy = cells.y0 + (j as f64 + 0.5) * cells.h;
        let (mut a, mut b) = (f64::INFINITY, f64::NEG_INFINITY);
        for (p, q) in outline
            .iter()
            .zip(outline.iter().cycle().skip(1))
            .take(outline.len())
        {
            if (p.1 <= cy) != (q.1 <= cy) {
                let x = p.0 + (cy - p.1) / (q.1 - p.1) * (q.0 - p.0);
                a = a.min(x);
                b = b.max(x);
            }
        }
        if let Some((i0, i1)) = (a <= b).then(|| cells.run(a, b)).flatten() {
            out.push((j, i0, i1));
        }
    }
    out
}

struct Heights {
    z: f32,
    previous: f32,
    entry: f32,
    clearance: f32,
}
struct Feeds {
    feed: f32,
    slope: f32,
}
/// Whether `path` starts inside `exact` and no step of it crosses one of
/// its edges (`edges`).
fn inside(exact: &[Polygon], edges: &Edges, path: &[P]) -> bool {
    pocket::inside(exact, path[0]) && path.windows(2).all(|w| !edges.crosses(w[0], w[1]))
}
/// Whether the move from `a` to `b` at depth keeps the tool centre inside
/// `exact` (its edges `edges`).
fn within(exact: &[Polygon], edges: &Edges, a: P, b: P) -> bool {
    pocket::inside(exact, a) && !edges.crosses(a, b)
}
const OUTSIDE: &str = "Adaptive clearing found a move that would leave the pocket's allowed region. Report this part; use Pocket meanwhile.";
/// A level's events as moves at its heights.
fn emit(
    out: &mut Motions,
    events: &[Event],
    exact: &[Polygon],
    edges: &Edges,
    h: Heights,
    f: Feeds,
    tolerance: f64,
) -> Result<(), String> {
    let point = |p: P, z: f32| [p.0 as f32, p.1 as f32, z];
    for event in events {
        match event {
            Event::Link {
                to,
                lift,
                above,
                slow,
            } => {
                let from = (out.at[0] as f64, out.at[1] as f64);
                match lift {
                    Lift::Stay => {
                        if !within(exact, edges, from, *to) {
                            return Err(OUTSIDE.into());
                        }
                        out.push(point(*to, h.z), Motion::Feed, f.feed, false)?;
                    }
                    Lift::Low | Lift::High => {
                        let travel = if *lift == Lift::Low {
                            h.entry
                        } else {
                            h.clearance
                        };
                        let up = [out.at[0], out.at[1], travel.max(out.at[2])];
                        out.push(up, Motion::Rapid, 6000., false)?;
                        // Below the stock top the tool goes sideways at the
                        // feed, never at rapid.
                        let across = if up[2] < out.plan.stock.max[2] {
                            (Motion::Feed, f.feed)
                        } else {
                            (Motion::Rapid, 6000.)
                        };
                        out.push(point(*to, up[2]), across.0, across.1, false)?;
                        out.push(point(*to, h.entry), Motion::Rapid, 6000., false)?;
                        if !above {
                            let rate = if *slow { f.feed * 0.25 } else { f.feed };
                            out.push(point(*to, h.z), Motion::Feed, rate, false)?;
                        }
                    }
                }
            }
            Event::Enter {
                centre,
                radius,
                from,
                turn,
            } => {
                let start = (
                    centre.0 + radius * from.cos(),
                    centre.1 + radius * from.sin(),
                );
                if *radius <= 0. {
                    out.push(point(start, h.z), Motion::Feed, f.feed * 0.25, false)?;
                    continue;
                }
                out.push(point(start, h.previous), Motion::Feed, f.feed * 0.25, false)?;
                // Chords within the tolerance of the circle; whole turns down
                // at no more than the ramp angle, then once round at the
                // level, ending where it began.
                let per_turn = (std::f64::consts::PI / (1. - tolerance / radius).max(-1.).acos())
                    .ceil()
                    .max(12.) as usize;
                let lap = (std::f64::consts::TAU * radius) as f32;
                let turns = ((h.previous - h.z) / (lap * f.slope)).ceil().max(1.) as usize;
                let segments = per_turn * (turns + 1);
                for k in 1..=segments {
                    let a = from + turn * std::f64::consts::TAU * k as f64 / per_turn as f64;
                    let down = (k as f32 / (per_turn * turns) as f32).min(1.);
                    let z = h.previous - (h.previous - h.z) * down;
                    let p = (centre.0 + radius * a.cos(), centre.1 + radius * a.sin());
                    if !pocket::inside(exact, p) {
                        return Err(OUTSIDE.into());
                    }
                    out.push(point(p, z), Motion::Feed, f.feed * 0.25, true)?;
                }
            }
            Event::Cut(path) => {
                // Simplified where that stays inside, else as walked.
                let short = simplify(path, tolerance);
                let path = if inside(exact, edges, &short) {
                    &short
                } else {
                    path
                };
                if !inside(exact, edges, path) {
                    return Err(OUTSIDE.into());
                }
                for p in path.iter().skip(1) {
                    out.push(point(*p, h.z), Motion::Feed, f.feed, true)?;
                }
            }
            Event::Wall(path, shares) => {
                if !inside(exact, edges, path) {
                    return Err(OUTSIDE.into());
                }
                for (p, share) in path.iter().skip(1).zip(shares) {
                    out.push(point(*p, h.z), Motion::Feed, f.feed * share, true)?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mesh, model::Machine, simulation::Simulation, stock, surface};
    fn quad(tris: &mut Vec<[[f32; 3]; 3]>, x: f32, y: f32, xx: f32, yy: f32, z: f32) {
        tris.push([[x, y, z], [xx, y, z], [xx, yy, z]]);
        tris.push([[x, y, z], [xx, yy, z], [x, yy, z]]);
    }
    /// The faces of `tris` looking up at the heights `floors`, selected for
    /// an Adaptive clearing with a Ø`diameter` flat end mill.
    fn part(
        tris: Vec<[[f32; 3]; 3]>,
        floors: &[f32],
        diameter: f32,
    ) -> (Setup, Operation, Tool, Mesh) {
        let bottom = tris
            .iter()
            .flatten()
            .map(|p| p[2])
            .fold(f32::INFINITY, f32::min);
        let mesh = mesh::normalize(tris).unwrap();
        let mut setup = Setup::new(Machine::new(3), "G54");
        setup
            .p
            .insert("stock_type".into(), "Relative size box".into());
        for (k, _) in stock::OFFSETS {
            setup.p.insert(k.into(), "0".into());
        }
        let mut tool = Tool::new("Adaptive test cutter", "Flat end mill", "2");
        tool.p.insert("diameter".into(), diameter.to_string());
        let mut op = Operation::new(Strategy::Adaptive, tool.id.clone());
        op.geometry.clear();
        let scale = mesh.display_scale();
        let placement = stock::Placement::new(&mesh, Some(&setup));
        let dz = placement.bounds.min[2] / scale - bottom;
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: mesh
                .surfaces
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    let z = placement.transform(s.center)[2] / scale - dz;
                    s.normal[2] > 0.999 && floors.iter().any(|f| (z - f).abs() < 0.001)
                })
                .map(|(i, _)| i)
                .collect(),
        });
        for (k, v) in [
            ("allowance", "0"),
            ("axial_allowance", "0"),
            ("optimal_load", "0.4"),
            ("stepdown", "6"),
        ] {
            op.p.insert(k.into(), v.into());
        }
        (setup, op, tool, mesh)
    }
    /// A 20 × 16 mm pocket 6 mm deep round a 4 × 4 mm island, in a rim,
    /// cut with a Ø2 cutter at a 0.4 mm load.
    fn ring() -> (Setup, Operation, Tool, Mesh) {
        let mut tris = Vec::new();
        for (x, y, xx, yy) in [
            (-10., -8., 10., -2.),
            (-10., 2., 10., 8.),
            (-10., -2., -2., 2.),
            (2., -2., 10., 2.),
        ] {
            quad(&mut tris, x, y, xx, yy, 0.);
        }
        for (x, y, xx, yy) in [
            (-15., -12., 15., -8.),
            (-15., 8., 15., 12.),
            (-15., -8., -10., 8.),
            (10., -8., 15., 8.),
            (-2., -2., 2., 2.),
        ] {
            quad(&mut tris, x, y, xx, yy, 6.);
        }
        quad(&mut tris, -15., -12., 15., 12., -4.);
        part(tris, &[0.], 2.)
    }
    /// Whether the tool of `radius` centred at `p` stays inside the ring's
    /// pocket and clear of its island.
    fn in_ring(p: [f32; 3], radius: f32) -> bool {
        let dx = (p[0].abs() - 2.).max(0.);
        let dy = (p[1].abs() - 2.).max(0.);
        p[0].abs() + radius <= 10. + 0.001
            && p[1].abs() + radius <= 8. + 0.001
            && dx * dx + dy * dy >= radius * radius - 0.001
    }
    /// Of `engagements_all`, those of the horizontal cutting moves at least
    /// `shortest` long, each scaled by its share of the full feed: what the
    /// cutter takes per minute, as a load at the full feed.
    fn engagements(plan: &Plan, shortest: f32, solid: impl Fn(P) -> bool) -> Vec<f32> {
        let full = plan
            .moves
            .iter()
            .filter(|m| m.cutting_pass)
            .map(|m| m.feed)
            .fold(0., f32::max);
        engagements_all(plan, solid)
            .into_iter()
            .zip(&plan.moves)
            .filter(|(_, m)| {
                m.cutting_pass
                    && m.from[2] == m.to[2]
                    && (m.to[0] - m.from[0]).hypot(m.to[1] - m.from[1]) >= shortest
            })
            .map(|(e, m)| e * m.feed / full)
            .collect()
    }
    /// A top view of `plan`'s moves at its lowest level, written to `file`:
    /// each cutting move coloured by its engagement over `load` (blue light,
    /// green at the load, red at twice it), links yellow, rapids grey.
    fn picture(plan: &Plan, load: f32, solid: impl Fn(P) -> bool + Copy, file: &str) {
        let scale = 24.;
        let (w, h) = (
            ((plan.stock.max[0] - plan.stock.min[0]) * scale) as usize,
            ((plan.stock.max[1] - plan.stock.min[1]) * scale) as usize,
        );
        let mut rgb = vec![255u8; w * h * 3];
        let px = |p: [f32; 3]| {
            (
                (p[0] - plan.stock.min[0]) * scale,
                (plan.stock.max[1] - p[1]) * scale,
            )
        };
        for y in 0..h {
            for x in 0..w {
                let p = (
                    plan.stock.min[0] as f64 + (x as f64 + 0.5) / scale as f64,
                    plan.stock.max[1] as f64 - (y as f64 + 0.5) / scale as f64,
                );
                if !solid(p) {
                    rgb[(y * w + x) * 3..][..3].copy_from_slice(&[200, 200, 200]);
                }
            }
        }
        let all = engagements_all(plan, solid);
        let lowest = plan.target;
        let mut line = |a: [f32; 3], b: [f32; 3], c: [u8; 3]| {
            let (a, b) = (px(a), px(b));
            let n = ((b.0 - a.0).abs().max((b.1 - a.1).abs()) as usize).max(1);
            for k in 0..=n {
                let t = k as f32 / n as f32;
                let (x, y) = (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
                if x >= 0. && y >= 0. && (x as usize) < w && (y as usize) < h {
                    rgb[((y as usize) * w + x as usize) * 3..][..3].copy_from_slice(&c);
                }
            }
        };
        for (m, e) in plan.moves.iter().zip(all) {
            if m.motion == Motion::Rapid {
                continue;
            }
            if (m.to[2] - lowest).abs() > 0.001 && (m.from[2] - lowest).abs() > 0.001 {
                continue;
            }
            let c = if !m.cutting_pass {
                [230, 190, 0]
            } else {
                let v = (e / load).clamp(0., 2.);
                if v < 1. {
                    [0, (255. * v) as u8, (255. * (1. - v)) as u8]
                } else {
                    [(255. * (v - 1.)) as u8, (255. * (2. - v)) as u8, 0]
                }
            };
            line(m.from, m.to, c);
        }
        let file = std::fs::File::create(file).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&rgb).unwrap();
    }
    /// The radial engagement of every move of `plan` (mm removed per mm of
    /// travel, the most over any 0.05 mm of it; 0 for rapids), its moves
    /// replayed, level by level, on a fine grid of the material `solid` says
    /// each level starts with.
    fn engagements_all(plan: &Plan, solid: impl Fn(P) -> bool) -> Vec<f32> {
        let h = 0.02;
        let cells = Cells {
            x0: plan.stock.min[0] as f64,
            y0: plan.stock.min[1] as f64,
            h,
            nx: ((plan.stock.max[0] - plan.stock.min[0]) as f64 / h).ceil() as usize,
            ny: ((plan.stock.max[1] - plan.stock.min[1]) as f64 / h).ceil() as usize,
        };
        let fresh = || {
            let mut b = Bits::new(cells.nx, cells.ny);
            for j in 0..cells.ny {
                for i in 0..cells.nx {
                    if solid(cells.centre(i, j)) {
                        b.fill(j, i, i);
                    }
                }
            }
            b
        };
        let mut levels: Vec<f32> = vec![];
        for m in plan
            .moves
            .iter()
            .filter(|m| m.cutting_pass && m.from[2] == m.to[2])
        {
            if !levels.iter().any(|z| (z - m.to[2]).abs() < 0.001) {
                levels.push(m.to[2]);
            }
        }
        let mut material = fresh();
        let mut level = f32::INFINITY;
        let mut out = vec![];
        for m in &plan.moves {
            if m.motion != Motion::Feed {
                out.push(0.);
                continue;
            }
            let at = levels
                .iter()
                .copied()
                .filter(|z| *z <= m.to[2] + 0.001)
                .fold(f32::NEG_INFINITY, f32::max);
            if at < level - 0.001 {
                material = fresh();
                level = at;
            }
            let (a, b) = (
                (m.from[0] as f64, m.from[1] as f64),
                (m.to[0] as f64, m.to[1] as f64),
            );
            // The worst of each 0.05 mm of the move, not its mean.
            let length = (b.0 - a.0).hypot(b.1 - a.1);
            let pieces = (length / 0.05).ceil().max(1.) as usize;
            let mut worst = 0f32;
            let mut from = a;
            for k in 1..=pieces {
                let t = k as f64 / pieces as f64;
                let to = (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
                let mut n = 0;
                cells.capsule(from, to, plan.radius as f64, |j, i0, i1| {
                    n += material.clear(j, i0, i1)
                });
                let piece = ((to.0 - from.0).hypot(to.1 - from.1) as f32).max(1e-6);
                worst = worst.max(n as f32 * (h * h) as f32 / piece);
                from = to;
            }
            out.push(worst);
        }
        out
    }
    /// Writes /tmp/adaptive-ring.png, the ring's path coloured by
    /// engagement, and prints its time against Pocket's: for looking at.
    #[test]
    #[ignore]
    fn picture_the_ring() {
        let (setup, mut op, tool, mesh) = ring();
        let load: f32 = std::env::var("LOAD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.2);
        op.p.insert("optimal_load".into(), load.to_string());
        let started = std::time::Instant::now();
        let plan = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
        let took = started.elapsed();
        let solid = |p: P| p.0.abs() < 10. && p.1.abs() < 8. && !(p.0.abs() < 2. && p.1.abs() < 2.);
        picture(&plan, load, solid, "/tmp/adaptive-ring.png");
        let e = engagements(&plan, 0.1, solid);
        let stats = |plan: &Plan| {
            let cut: f32 = plan
                .moves
                .iter()
                .filter(|m| m.cutting_pass)
                .map(|m| m.end - m.start)
                .sum();
            let link: f32 = plan
                .moves
                .iter()
                .filter(|m| !m.cutting_pass && m.motion == Motion::Feed)
                .map(|m| m.end - m.start)
                .sum();
            format!(
                "{} moves, {:.0} s: cutting {cut:.0} s, links {link:.0} s",
                plan.moves.len(),
                plan.duration
            )
        };
        eprintln!(
            "adaptive {} in {took:?}; engagement p99 {:.3} most {:.3}",
            stats(&plan),
            quantile(&e, 0.99),
            quantile(&e, 1.)
        );
        let all = engagements_all(&plan, solid);
        let (mut first, mut second, mut later, mut slowed) = (0, 0, 0, 0);
        for (i, m) in plan.moves.iter().enumerate() {
            if !(m.cutting_pass && m.from[2] == m.to[2] && all[i] > 2. * load) {
                continue;
            }
            if m.feed < 999. {
                slowed += 1;
            } else if !plan.moves[i - 1].cutting_pass {
                first += 1;
            } else if !plan.moves[i - 2].cutting_pass {
                second += 1;
            } else {
                later += 1;
            }
        }
        eprintln!(
            "over 2x: first move {first}, second {second}, later {later}, wall (slowed) {slowed}"
        );
        let worst = (1..plan.moves.len())
            .filter(|&i| plan.moves[i].cutting_pass && plan.moves[i].from[2] == plan.moves[i].to[2])
            .max_by(|&a, &b| all[a].total_cmp(&all[b]))
            .unwrap();
        for k in worst.saturating_sub(3)..worst + 2 {
            let m = &plan.moves[k];
            eprintln!(
                "  {k}: {:?} -> {:?} {:?} cut {} feed {} e {:.3}",
                m.from, m.to, m.motion, m.cutting_pass, m.feed, all[k]
            );
        }
        let mut pocket = op.clone();
        pocket.strategy = Strategy::Pocket;
        pocket.p.insert("stepover".into(), "10".into());
        pocket.p.insert("pocket_entry".into(), "Plunge".into());
        let pocket = crate::simulation::generate(&setup, &pocket, &tool, &mesh).unwrap();
        let e = engagements(&pocket, 0.1, solid);
        eprintln!(
            "pocket 10% {}; engagement p99 {:.3} most {:.3}",
            stats(&pocket),
            quantile(&e, 0.99),
            quantile(&e, 1.)
        );
    }
    /// Writes /tmp/adaptive-totem.png, the keyboard case plate's path
    /// coloured by engagement: for looking at.
    #[test]
    #[ignore]
    fn picture_the_totem_case() {
        let mesh = mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("examples/totem-case-top-right.step"),
        )
        .unwrap();
        let (setup, mut op, mut tool, _) = ring();
        tool.p.insert("diameter".into(), "3".into());
        for (k, v) in [
            ("optimal_load", "0.45"),
            ("stepdown", "7"),
            ("allowance", "0.3"),
            ("axial_allowance", "0.2"),
        ] {
            op.p.insert(k.into(), v.into());
        }
        let placement = stock::Placement::new(&mesh, Some(&setup));
        let (top, bottom) = (placement.bounds.max[2], placement.bounds.min[2]);
        let (id, _) = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.normal[2] > 0.999999
                    && placement.transform(s.center)[2] < top - 0.1
                    && placement.transform(s.center)[2] > bottom + 0.1
            })
            .max_by(|(_, a), (_, b)| a.area.total_cmp(&b.area))
            .unwrap();
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: vec![id],
        });
        let started = std::time::Instant::now();
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        let took = started.elapsed();
        // The tool never reaches the part, so all of the stock may count
        // as material.
        let b = plan.stock;
        let solid = move |p: P| {
            p.0 > b.min[0] as f64
                && p.0 < b.max[0] as f64
                && p.1 > b.min[1] as f64
                && p.1 < b.max[1] as f64
        };
        picture(&plan, 0.45, solid, "/tmp/adaptive-totem.png");
        let e = engagements(&plan, 0.1, solid);
        let all = engagements_all(&plan, solid);
        let worst = (0..all.len())
            .filter(|&i| plan.moves[i].cutting_pass && plan.moves[i].from[2] == plan.moves[i].to[2])
            .max_by(|&a, &b| all[a].total_cmp(&all[b]))
            .unwrap();
        for m in &plan.moves[worst.saturating_sub(2)..worst + 2] {
            eprintln!(
                "  {:?} -> {:?} cut {} e {:.3}",
                m.from, m.to, m.cutting_pass, all[worst]
            );
        }
        eprintln!(
            "{} moves, {:.0} s, in {took:?}; engagement p99 {:.3} most {:.3}",
            plan.moves.len(),
            plan.duration,
            quantile(&e, 0.99),
            quantile(&e, 1.)
        );
    }
    fn quantile(values: &[f32], q: f32) -> f32 {
        let mut v = values.to_vec();
        v.sort_by(f32::total_cmp);
        v[((v.len() - 1) as f32 * q) as usize]
    }

    #[test]
    fn capsule_runs_and_rasters_hold_exactly_the_cells_inside() {
        let cells = Cells {
            x0: -3.,
            y0: -2.,
            h: 0.1,
            nx: 70,
            ny: 50,
        };
        let (a, b, r) = ((-1.23, -0.4), (1.7, 0.95), 0.83);
        let mut runs = vec![];
        cells.capsule(a, b, r, |j, i0, i1| runs.push((j, i0, i1)));
        let mut inside = Bits::new(cells.nx, cells.ny);
        for (j, i0, i1) in runs {
            inside.fill(j, i0, i1);
        }
        let segment = |p: P| {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0., 1.);
            (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
        };
        let triangle: Vec<Polygon> = vec![vec![(-2.5, -1.5), (3.2, -0.2), (-0.3, 2.4)]];
        let raster = cells.raster(&triangle);
        for j in 0..cells.ny {
            for i in 0..cells.nx {
                let p = cells.centre(i, j);
                let d = segment(p);
                if (d - r).abs() > 1e-9 {
                    assert_eq!(inside.get(i as isize, j as isize), d < r, "{p:?}");
                }
                assert_eq!(
                    raster.get(i as isize, j as isize),
                    pocket::inside(&triangle, p),
                    "{p:?}"
                );
            }
        }
        assert_eq!(runs_of(&cells, &triangle[0]), raster.total());
    }
    fn runs_of(cells: &Cells, outline: &[(f64, f64)]) -> u64 {
        runs(cells, outline)
            .iter()
            .map(|(_, i0, i1)| (i1 - i0 + 1) as u64)
            .sum()
    }
    #[test]
    fn distances_are_exact_to_the_nearest_seed() {
        let cells = Cells {
            x0: 0.,
            y0: 0.,
            h: 1.,
            nx: 37,
            ny: 23,
        };
        let seeds = [(3usize, 4usize), (30, 2), (18, 20), (36, 22)];
        let d = distances(&cells, |i, j| seeds.contains(&(i, j)));
        for j in 0..cells.ny {
            for i in 0..cells.nx {
                let want = seeds
                    .iter()
                    .map(|s| {
                        ((s.0 as f32 - i as f32).powi(2) + (s.1 as f32 - j as f32).powi(2)).sqrt()
                    })
                    .fold(f32::INFINITY, f32::min);
                assert!((d[j * cells.nx + i] - want).abs() < 1e-3, "{i} {j}");
            }
        }
    }
    #[test]
    fn simplify_keeps_the_ends_and_every_corner_past_the_tolerance() {
        let mut points: Vec<P> = (0..=10)
            .map(|k| (k as f64, 0.001 * (k % 2) as f64))
            .collect();
        points.extend((1..=10).map(|k| (10., k as f64)));
        let kept = simplify(&points, 0.01);
        assert_eq!(kept, vec![(0., 0.), (10., 0.), (10., 10.)]);
        // The zigzag stays; the straight run up X 10 goes.
        assert_eq!(simplify(&points, 0.0001).len(), 12);
    }

    #[test]
    fn adaptive_clears_the_ring_without_touching_walls_or_island_at_a_steady_load() {
        let (setup, op, tool, mesh) = ring();
        let plan = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(plan.strategy, Strategy::Adaptive);
        assert_eq!(plan.levels, 1);
        for pair in plan.moves.windows(2) {
            assert_eq!(pair[0].to, pair[1].from);
            assert_eq!(pair[0].end, pair[1].start);
        }
        for m in &plan.moves {
            if m.motion == Motion::Rapid && m.from[..2] != m.to[..2] {
                assert!(
                    m.from[2] > plan.stock.max[2] - 0.001 && m.to[2] > plan.stock.max[2] - 0.001
                );
            }
            if m.motion == Motion::Feed {
                for i in 0..=10 {
                    let p = m.at(m.start + (m.end - m.start) * i as f32 / 10.);
                    assert!(in_ring(p, plan.radius), "{p:?}");
                    assert!(p[2] >= plan.target - 0.001);
                }
            }
        }
        let load = 0.4;
        let e = engagements(&plan, 0.1, |p| {
            p.0.abs() < 10. && p.1.abs() < 8. && !(p.0.abs() < 2. && p.1.abs() < 2.)
        });
        assert!(e.len() > 100, "{} moves", e.len());
        let (p95, max) = (quantile(&e, 0.95), quantile(&e, 1.));
        assert!(p95 <= 1.5 * load, "95th percentile {p95}");
        assert!(max <= 2.5 * load, "most {max}");
        // Most of the path cuts near the load.
        let near = e
            .iter()
            .filter(|&&v| v > 0.5 * load && v < 1.5 * load)
            .count();
        assert!(near * 2 > e.len(), "{near} of {}", e.len());
        let mut sim = Simulation::new(plan);
        sim.seek(1.);
        let expected = (20. * 16. - 4. * 4.) * 6.;
        let removed = sim.stock.removed_volume();
        assert!(
            (removed - expected).abs() / expected < 0.04,
            "{removed} vs {expected}"
        );
        for y in 0..sim.stock.ny {
            for x in 0..sim.stock.nx {
                let px = sim.stock.bounds.min[0] + (x as f32 + 0.5) * sim.stock.dx;
                let py = sim.stock.bounds.min[1] + (y as f32 + 0.5) * sim.stock.dy;
                if px.abs() > 10. || py.abs() > 8. || (px.abs() < 2. && py.abs() < 2.) {
                    assert_eq!(
                        sim.stock.heights[y * sim.stock.nx + x],
                        sim.stock.bounds.max[2]
                    );
                }
            }
        }
    }
    #[test]
    fn pocket_offset_rings_overload_where_adaptive_does_not() {
        let (setup, mut op, tool, mesh) = ring();
        let adaptive = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
        op.strategy = Strategy::Pocket;
        op.p.insert("stepover".into(), "20".into());
        op.p.insert("pocket_entry".into(), "Plunge".into());
        let pocket = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
        let solid = |p: P| p.0.abs() < 10. && p.1.abs() < 8. && !(p.0.abs() < 2. && p.1.abs() < 2.);
        let most = |plan: &Plan| quantile(&engagements(plan, 0.1, solid), 1.);
        assert!(
            most(&pocket) > 1.5 * most(&adaptive),
            "{} vs {}",
            most(&pocket),
            most(&adaptive)
        );
    }
    #[test]
    fn levels_conventional_milling_and_a_plunge_entry_clear_the_same_pocket() {
        let (setup, mut op, tool, mesh) = ring();
        op.p.insert("stepdown".into(), "2".into());
        let climb = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(climb.levels, 3);
        let turning = |plan: &Plan| -> f32 {
            let cuts: Vec<&Move> = plan
                .moves
                .iter()
                .filter(|m| m.cutting_pass && m.from[2] == m.to[2])
                .collect();
            cuts.windows(2)
                .filter(|w| w[0].to == w[1].from)
                .map(|w| {
                    let a = (w[0].to[1] - w[0].from[1]).atan2(w[0].to[0] - w[0].from[0]);
                    let b = (w[1].to[1] - w[1].from[1]).atan2(w[1].to[0] - w[1].from[0]);
                    let mut d = b - a;
                    while d > std::f32::consts::PI {
                        d -= std::f32::consts::TAU;
                    }
                    while d < -std::f32::consts::PI {
                        d += std::f32::consts::TAU;
                    }
                    d
                })
                .sum()
        };
        op.p.insert("pocket_milling".into(), "Conventional".into());
        let conventional = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
        // Climbing with the spindle clockwise keeps the material on the
        // right: the path winds anticlockwise round what it has cleared.
        assert!(turning(&climb) > 0. && turning(&conventional) < 0.);
        op.p.insert("ramp".into(), "Plunge".into());
        let plunge = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
        assert!(
            !plunge
                .moves
                .iter()
                .any(|m| m.cutting_pass && m.from[2] != m.to[2])
        );
        let expected = (20. * 16. - 4. * 4.) * 6.;
        for plan in [climb, conventional, plunge] {
            for m in plan.moves.iter().filter(|m| m.motion == Motion::Feed) {
                assert!(in_ring(m.to, plan.radius), "{:?}", m.to);
            }
            let mut sim = Simulation::new(plan);
            sim.seek(1.);
            let removed = sim.stock.removed_volume();
            assert!(
                (removed - expected).abs() / expected < 0.04,
                "{removed} vs {expected}"
            );
        }
    }
    #[test]
    fn an_open_side_is_entered_from_beside_the_stock_without_a_helix() {
        // A slot open at X 15, the stock's edge, between walls up to Z 6.
        let mut tris = vec![];
        quad(&mut tris, -10., -6., 15., 6., 0.);
        quad(&mut tris, -15., -12., 15., -6., 6.);
        quad(&mut tris, -15., 6., 15., 12., 6.);
        quad(&mut tris, -15., -6., -10., 6., 6.);
        quad(&mut tris, -15., -12., 15., 12., -4.);
        let (setup, op, tool, mesh) = part(tris, &[0.], 2.);
        let plan = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
        assert!(
            !plan
                .moves
                .iter()
                .any(|m| m.cutting_pass && m.from[2] != m.to[2])
        );
        let e = engagements(&plan, 0.1, |p| p.0 > -10. && p.1.abs() < 6.);
        assert!(quantile(&e, 0.95) <= 1.5 * 0.4, "{}", quantile(&e, 0.95));
        let mut sim = Simulation::new(plan);
        sim.seek(1.);
        let expected = 25. * 12. * 6.;
        let removed = sim.stock.removed_volume();
        assert!(
            (removed - expected).abs() / expected < 0.04,
            "{removed} vs {expected}"
        );
    }
    #[test]
    fn after_a_larger_cutter_a_smaller_one_cuts_only_the_corners_it_left() {
        let mut tris = vec![];
        quad(&mut tris, -10., -8., 10., 8., 0.);
        for (x, y, xx, yy) in [
            (-15., -12., 15., -8.),
            (-15., 8., 15., 12.),
            (-15., -8., -10., 8.),
            (10., -8., 15., 8.),
        ] {
            quad(&mut tris, x, y, xx, yy, 6.);
        }
        quad(&mut tris, -15., -12., 15., 12., -4.);
        let (mut setup, mut op, tool, mesh) = part(tris, &[0.], 6.);
        op.p.insert("optimal_load".into(), "0.6".into());
        let large = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
        let mut small = tool.clone();
        small.p.insert("diameter".into(), "2".into());
        op.p.insert("optimal_load".into(), "0.4".into());
        let whole = crate::simulation::generate(&setup, &op, &small, &mesh).unwrap();
        let frame = crate::volume::Frame::new(&setup, &mesh);
        let mut sim = Simulation::new(large);
        sim.volume = Some(crate::volume::Volume::new(sim.plan.stock, frame));
        sim.initial_volume = sim.volume.clone();
        sim.frame = Some(frame);
        sim.seek(1.);
        setup.preview_remaining = Some(std::sync::Arc::new(crate::rest::RestStock::new(
            sim.volume.as_ref().unwrap(),
            frame,
        )));
        let rest = crate::simulation::generate(&setup, &op, &small, &mesh).unwrap();
        let length = |plan: &Plan| -> f32 {
            plan.moves
                .iter()
                .filter(|m| m.cutting_pass)
                .map(|m| (m.to[0] - m.from[0]).hypot(m.to[1] - m.from[1]))
                .sum()
        };
        assert!(
            length(&rest) * 4. < length(&whole),
            "{} vs {}",
            length(&rest),
            length(&whole)
        );
        // What it cuts besides the walls' pass is in the corners.
        let corner = |p: &[f32; 3]| {
            [(-10., -8.), (10., -8.), (-10., 8.), (10., 8.)]
                .iter()
                .any(|c: &(f32, f32)| (p[0] - c.0).hypot(p[1] - c.1) < 4.)
        };
        let wall =
            |p: &[f32; 3]| (p[0].abs() - 8.98).abs() < 0.01 || (p[1].abs() - 6.98).abs() < 0.01;
        for m in rest.moves.iter().filter(|m| m.cutting_pass) {
            assert!(corner(&m.to) || wall(&m.to), "{:?}", m.to);
        }
    }
    #[test]
    fn the_totem_keyboard_case_plate_clears_in_one_level_without_touching_the_part() {
        if crate::step::skip_without_helper() {
            return;
        }
        let mesh = mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("examples/totem-case-top-right.step"),
        )
        .unwrap();
        // A Ø3 flat end mill at a 0.45 mm load, the whole 6.85 mm in one
        // level, 0.3 mm radial and 0.2 mm axial stock to leave.
        let (setup, mut op, mut tool, _) = ring();
        tool.p.insert("diameter".into(), "3".into());
        for (k, v) in [
            ("optimal_load", "0.45"),
            ("stepdown", "7"),
            ("allowance", "0.3"),
            ("axial_allowance", "0.2"),
        ] {
            op.p.insert(k.into(), v.into());
        }
        let placement = stock::Placement::new(&mesh, Some(&setup));
        let scale = mesh.display_scale();
        let (top, bottom) = (placement.bounds.max[2], placement.bounds.min[2]);
        let (id, _) = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.normal[2] > 0.999999
                    && placement.transform(s.center)[2] < top - 0.1
                    && placement.transform(s.center)[2] > bottom + 0.1
            })
            .max_by(|(_, a), (_, b)| a.area.total_cmp(&b.area))
            .unwrap();
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: vec![id],
        });
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(plan.levels, 1);
        // No feed move brings the cutter nearer the part than the radial
        // stock to leave: every 0.25 mm of each, against what of every face
        // of the part stands above the floor.
        let faces: Vec<Polygon> = mesh
            .faces
            .iter()
            .map(|f| {
                let points: Vec<[f32; 3]> = f
                    .points
                    .iter()
                    .map(|p| placement.transform(*p).map(|v| v / scale))
                    .collect();
                pocket::above(&points, plan.target + 0.01)
            })
            .filter(|p| p.len() >= 2)
            .collect();
        let cell = 5.;
        let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
        for (i, face) in faces.iter().enumerate() {
            let (x0, x1) = face
                .iter()
                .fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.0), a.1.max(p.0)));
            let (y0, y1) = face
                .iter()
                .fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.1), a.1.max(p.1)));
            for gx in (x0 / cell).floor() as i64..=(x1 / cell).floor() as i64 {
                for gy in (y0 / cell).floor() as i64..=(y1 / cell).floor() as i64 {
                    grid.entry((gx, gy)).or_default().push(i);
                }
            }
        }
        let clear = (plan.radius + 0.3) as f64;
        let mut checked = 0;
        for m in plan.moves.iter().filter(|m| m.motion == Motion::Feed) {
            let length = (m.to[0] - m.from[0]).hypot(m.to[1] - m.from[1]);
            let pieces = (length / 0.25).ceil().max(1.) as usize;
            for k in 0..=pieces {
                let p = m.at(m.start + (m.end - m.start) * k as f32 / pieces as f32);
                if p[2] >= plan.stock.max[2] {
                    continue;
                }
                let xy = (p[0] as f64, p[1] as f64);
                let (gx, gy) = ((xy.0 / cell).floor() as i64, (xy.1 / cell).floor() as i64);
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for &i in grid.get(&(gx + dx, gy + dy)).into_iter().flatten() {
                            let face = &faces[i];
                            let d = if pocket::inside(&[face.clone()], xy) {
                                0.
                            } else {
                                pocket::nearest(face, xy).0
                            };
                            assert!(d >= clear - 0.001, "{d} from a face at {p:?}");
                        }
                    }
                }
                checked += 1;
            }
        }
        assert!(checked > 10_000, "{checked}");
        let mut sim = Simulation::new(plan);
        sim.seek(1.);
        assert!(sim.stock.removed_volume() > 10_000.);
    }
    #[test]
    fn the_example_blocks_pocket_cleared_adaptively_posts_and_replays_exactly() {
        // The fixture job's Face, then its Pocket as an Adaptive clearing
        // with the Ø10 at a 2 mm load, in two levels.
        let (mut doc, mesh) = crate::post::fixture::job();
        let setup = &mut doc.setups[0];
        setup.operations.truncate(2);
        let op = &mut setup.operations[1];
        op.strategy = Strategy::Adaptive;
        op.p.insert("name".into(), Strategy::Adaptive.name().into());
        op.p.insert("optimal_load".into(), "2".into());
        op.p.insert("stepdown".into(), "8".into());
        let job = crate::post::post_job(&doc, &mesh);
        let errors: Vec<String> = job
            .errors
            .iter()
            .cloned()
            .chain(
                job.setups
                    .iter()
                    .filter_map(|s| s.result.as_ref().err())
                    .flatten()
                    .cloned(),
            )
            .collect();
        assert!(job.ready(), "{errors:?}");
        let program = &job.setups[0].result.as_ref().unwrap()[0];
        assert_eq!(program.operations.len(), 2);
        assert!(program.text.contains("Adaptive clearing"));
        assert!(program.lines > 500, "{} lines", program.lines);
    }
    #[test]
    fn a_notch_in_a_wall_of_any_width_clears_and_its_corners_are_slowed() {
        // The 20 × 16 mm pocket with a notch 3 mm deep into its right wall.
        for width in [1.5, 2.5, 3., 4., 5.5, 7.] {
            let mut tris = vec![];
            let half = width / 2.;
            quad(&mut tris, -10., -8., 10., 8., 0.);
            quad(&mut tris, 10., -half, 13., half, 0.);
            for (x, y, xx, yy) in [
                (-15., -12., 15., -8.),
                (-15., 8., 15., 12.),
                (-15., -8., -10., 8.),
                (10., -8., 15., -half),
                (10., half, 15., 8.),
                (13., -half, 15., half),
            ] {
                quad(&mut tris, x, y, xx, yy, 6.);
            }
            quad(&mut tris, -15., -12., 15., 12., -4.);
            let (setup, op, tool, mesh) = part(tris, &[0.], 2.);
            let plan =
                generate(&setup, &op, &tool, &mesh).unwrap_or_else(|e| panic!("{width}: {e}"));
            let solid = |p: P| {
                (p.0.abs() < 10. && p.1.abs() < 8.)
                    || (p.0 >= 10. && p.0 < 13. && p.1.abs() < half as f64)
            };
            let e = engagements(&plan, 0., solid);
            let most = quantile(&e, 1.);
            // A notch narrower than twice the cutter is slotted (a slot is
            // twice the radius); elsewhere no move takes more than two and
            // a half times the load in any 0.05 mm, at its feed.
            let limit = if width <= 4. { 2. } else { 2.5 * 0.4 };
            assert!(most <= limit + 0.01, "{width}: most {most}");
        }
    }
    #[test]
    fn the_totem_keyboard_case_plate_clears_with_a_2_mm_cutter_too() {
        if crate::step::skip_without_helper() {
            return;
        }
        let mesh = mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("examples/totem-case-top-right.step"),
        )
        .unwrap();
        let (setup, mut op, mut tool, _) = ring();
        tool.p.insert("diameter".into(), "2".into());
        for (k, v) in [
            ("optimal_load", "0.5"),
            ("stepdown", "7"),
            ("allowance", "0.3"),
        ] {
            op.p.insert(k.into(), v.into());
        }
        let placement = stock::Placement::new(&mesh, Some(&setup));
        let (top, bottom) = (placement.bounds.max[2], placement.bounds.min[2]);
        let (id, _) = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.normal[2] > 0.999999
                    && placement.transform(s.center)[2] < top - 0.1
                    && placement.transform(s.center)[2] > bottom + 0.1
            })
            .max_by(|(_, a), (_, b)| a.area.total_cmp(&b.area))
            .unwrap();
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: vec![id],
        });
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(plan.levels, 1);
    }
    #[test]
    fn the_totem_plate_clears_with_a_2_mm_cutter_at_its_default_load_in_two_levels() {
        if crate::step::skip_without_helper() {
            return;
        }
        let mesh = mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("examples/totem-case-top-right.step"),
        )
        .unwrap();
        // What a new Adaptive clearing with a Ø2 starts with: a 0.3 mm load
        // and a 4 mm stepdown, with 0.3 mm stock to leave.
        let (setup, mut op, mut tool, _) = ring();
        tool.p.insert("diameter".into(), "2".into());
        for (k, v) in [
            ("optimal_load", "0.3"),
            ("stepdown", "4"),
            ("allowance", "0.3"),
        ] {
            op.p.insert(k.into(), v.into());
        }
        let placement = stock::Placement::new(&mesh, Some(&setup));
        let (top, bottom) = (placement.bounds.max[2], placement.bounds.min[2]);
        let (id, _) = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.normal[2] > 0.999999
                    && placement.transform(s.center)[2] < top - 0.1
                    && placement.transform(s.center)[2] > bottom + 0.1
            })
            .max_by(|(_, a), (_, b)| a.area.total_cmp(&b.area))
            .unwrap();
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: vec![id],
        });
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(plan.levels, 2);
        let moves = plan.moves.len();
        let started = std::time::Instant::now();
        let mut sim = Simulation::new(plan);
        sim.seek(1.);
        eprintln!("{moves} moves, simulated in {:?}", started.elapsed());
        assert!(sim.stock.removed_volume() > 10_000.);
    }
    #[test]
    fn settings_out_of_range_are_refused_with_their_limits() {
        let (setup, mut op, tool, mesh) = ring();
        for (k, v, says) in [
            ("optimal_load", "1.5", "from 2% to 50% of the tool diameter"),
            (
                "optimal_load",
                "0.01",
                "from 2% to 50% of the tool diameter",
            ),
            ("minimum_radius", "2", "Minimum cutting radius"),
            ("ramp", "Zigzag", "Helix or a Plunge"),
            ("ramp_angle", "30", "helix ramp angle"),
        ] {
            let mut o = op.clone();
            o.p.insert(k.into(), v.into());
            let e = generate(&setup, &o, &tool, &mesh).unwrap_err();
            assert!(e.contains(says), "{k}={v}: {e}");
        }
        op.enabled = false;
        assert!(
            generate(&setup, &op, &tool, &mesh)
                .unwrap_err()
                .contains("Adaptive clearing")
        );
    }
}
