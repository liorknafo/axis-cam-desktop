//! Air moves: the stretches of a milling operation's path where the cutter
//! takes nothing from the stock it starts from (the stock box or bar less
//! the earlier operations' cuts and its own moves before). Where a retract
//! to just over the stock top, a rapid across and a descent are quicker
//! than feeding through such a stretch, they replace it (`trim`): the
//! strategies' own link between their cuts, kept lower.
//!
//! A stretch cuts where material reaches `DEPTH` into the cutter: the
//! cutter's surface, drawn in by that much, is sampled round and up its
//! flutes every `step` along the path and tested exactly against the cuts
//! (`volume::swept`), as the stock left was `lag` back along the path, so
//! that a thin wall the cutter shaves (`DEPTH` or more) is found however
//! short the moves it is shaved by. A stretch found cutting is kept with
//! `pad` of path either side. A level move that repeats one the operation
//! already made where that one cut, a finishing pass taken again (Pocket's
//! and Contour's finishing passes), is kept whatever it cuts.
//!
//! A Pocket support pass (`Plan::keep_air`) clears the stock over a band
//! before its first stepdown, and is taken out only where it is proven
//! clear (`Detector::clear`), not where no sample finds material: a strip
//! thinner than `DEPTH`, or one short enough to lie between the samples,
//! can still stand a whole stepdown tall, which the level after the pass
//! would meet as well as its own.
use crate::holding::HoldBox;
use crate::mesh::Bounds;
use crate::model::{Operation, Setup, Strategy, num};
use crate::pocket::Polygon;
use crate::simulation::{CutterShape, Motion, Move, Plan};
use crate::stock_surface::Cut;
use crate::volume::{Frame, History, Volume, swept};
use clipper2::{EndType, FillRule, JoinType, Milli, Paths, difference, intersect};

/// How far into the cutter material must reach for the cutter to cut it,
/// mm: a thinner sliver, the chords of two passes over one curve, is air.
pub const DEPTH: f32 = 0.02;
/// The band at its edge a support pass proven clear is not proven over, mm:
/// what it leaves uncut lies within this of its edge, and the level after
/// it meets no more than a stepdown but there.
pub const SUPPORT_GAP: f32 = 0.003;
/// The most of a support pass's path proven clear at once, mm.
const RUN: f32 = 5.;
/// How much nearer than it must, mm, a support pass's path is to a cut's
/// for its stretch to be proven exactly (`reach`): a stretch's ends are
/// worked out to far closer than that.
const EXACT: f64 = 1e-6;
/// clipper2's arcs: their chords at most this share of their radius inside
/// them (its `arc_const`).
const ARC: f64 = 0.002;
/// How far a polygon's vertex moves rounded to `Milli`'s 0.001 mm, mm.
const ROUND: f64 = 0.0008;
/// How much more than they can be off the polygons a support pass is
/// proven with err on the safe side, mm: five of `Milli`'s units.
const MARGIN: f64 = 0.005;
/// The rapid rate plans are timed at, mm/min (`cam::Builder`).
const RAPID: f32 = 6000.;
/// The most cutter positions a path is tested at: a longer one is tested
/// more coarsely.
const MOST: f32 = 200_000.;
/// The least a replacement saves, s, and the most of the stretch's time
/// it takes.
const SAVING: f32 = 0.5;
const SHARE: f32 = 0.9;
/// How far apart up the flutes the cutter's side is sampled, mm.
const RISE: f32 = 0.5;
/// How much farther than the setup's gap and the controller's blend (by
/// which the export grows the workholding) a link keeps from the vise
/// jaws or clamps, mm.
const HOLDING: f32 = 0.1;

/// What `trim` changed in a plan.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Trimmed {
    /// The stretches replaced.
    pub stretches: usize,
    /// The feed through air they held, mm.
    pub air: f32,
    /// The links of the plan's own lowered to just over the stock top.
    pub lowered: usize,
    /// The time saved, s.
    pub saved: f32,
    /// Whether no move of the plan cuts anything: the plan is left as it
    /// is, for the user to see (a spring pass, or an operation the earlier
    /// ones made needless).
    pub nothing: bool,
}

/// Whether `trim` looks at `plan` of `setup`: a 3-axis milling operation
/// with a flat or ball end mill, on a machine that moves its axes together.
fn trimmed(setup: &Setup, plan: &Plan) -> bool {
    matches!(
        plan.strategy,
        Strategy::Face
            | Strategy::Pocket
            | Strategy::Adaptive
            | Strategy::Contour
            | Strategy::Parallel
            | Strategy::Waterline
    ) && matches!(plan.shape, CutterShape::Flat | CutterShape::Ball)
        && plan.radius > 0.
        && plan.flute_length > DEPTH
        && !crate::manual::is_manual(&setup.machine)
        && plan
            .moves
            .iter()
            .all(|m| m.a.is_none() && m.motion != Motion::Index)
}

fn length(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|k| (b[k] - a[k]).powi(2)).sum::<f32>().sqrt()
}
/// The point fraction `t` from `a` to `b`: `a` and `b` themselves at the
/// ends, so that pieces of a move meet the moves either side exactly.
fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    if t <= 0. {
        a
    } else if t >= 1. {
        b
    } else {
        std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
    }
}

/// A feed move of the plan along the path: its index, where along the
/// path (mm of feed) it starts, and its length.
#[derive(Clone, Copy)]
struct Feed {
    index: usize,
    at: f32,
    length: f32,
}

/// `plan` with every stretch of air `trim` finds replaced, and what was
/// changed: None when nothing was, or the plan is not one it looks at.
/// `volume` is the stock the operation starts from, `frame` the setup's,
/// `source` the operation's (`Simulation::source`), `holding` the setup's
/// vise jaws or clamps (`workholding`), which no link comes near.
pub fn trim(
    plan: Plan,
    setup: &Setup,
    op: &Operation,
    volume: &Volume,
    frame: Frame,
    source: (u16, u16),
    holding: &[HoldBox],
) -> Result<(Plan, Option<Trimmed>), String> {
    if !trimmed(setup, &plan) {
        return Ok((plan, None));
    }
    let Some(cutting) = cutting(&plan, volume, frame, source)? else {
        return Ok((plan, None));
    };
    // The controller's blend: the export grows the workholding and the
    // column a rapid descends through by it.
    let blend = crate::post::PostSettings::for_machine(&setup.machine).map_or(1., |s| s.blend);
    let gap = crate::holding::gap(setup).unwrap_or_default().max(0.) + blend + HOLDING;
    Ok(relink(
        plan,
        op,
        volume,
        frame,
        &cutting,
        (holding, gap),
        blend,
    ))
}

/// The vise jaws, clamps, chuck or hold-down screws of `setup`, as the
/// export models them (`Document::holding_boxes`, though a problem with
/// them, which the export refuses for, keeps them here), round the stock
/// it starts from: `volume` as the setups before it left it in `frame`.
/// None that cannot be modelled.
pub fn workholding(
    doc: &crate::model::Document,
    setup: &Setup,
    mesh: &crate::mesh::Mesh,
    volume: Option<&Volume>,
    frame: Frame,
) -> Vec<HoldBox> {
    let Ok(stock) = crate::stock::resolve(setup, Some(mesh)) else {
        return vec![];
    };
    let bands = volume
        .filter(|v| !v.cuts.is_empty())
        .map(|v| crate::holding::Bands::new(v, frame, crate::simulation::part_box(setup, mesh)));
    let held = crate::holding::held_by(doc, setup, stock.stock, bands.as_ref());
    if crate::chuck::holds(setup) {
        let a = setup.turn.map_or(0., |t| t.a);
        return (doc.chuck_rig(setup, held.stock).map(|rig| rig.boxes(a))).unwrap_or_default();
    }
    let holder = doc.holder(crate::model::get(&setup.p, "holder"));
    let mut boxes = match (crate::holding::layout(setup), holder) {
        (Ok(layout), Some(holder)) => {
            crate::holding::boxes(&layout, holder, held).unwrap_or_default()
        }
        _ => vec![],
    };
    boxes.extend(crate::sheet::holddown_boxes(setup, held.stock).unwrap_or_default());
    boxes
}

/// What a plan's cutter meets along its feed path: the stock it starts
/// from, and the cuts before each point of the path (`found`).
struct Detector<'a> {
    plan: &'a Plan,
    volume: &'a Volume,
    frame: Frame,
    feeds: Vec<Feed>,
    /// Each feed's number among the cuts `history` keeps.
    ids: Vec<u32>,
    history: History,
    /// The stock's box in `frame`, and its bar's centre and radius in the
    /// volume's frame when it is round.
    stock: Bounds,
    bar: Option<([f32; 2], f32)>,
    /// The feed path's length, mm.
    total: f32,
    /// How far apart along the path it is tested, how far back the stock
    /// it is tested against is as it was, and how much path either side of
    /// what is found cutting is kept, mm.
    step: f32,
    lag: f32,
    pad: f32,
}
impl<'a> Detector<'a> {
    /// None for a plan without feed.
    fn new(plan: &'a Plan, volume: &'a Volume, frame: Frame, source: (u16, u16)) -> Option<Self> {
        let (radius, flute, shape) = (plan.radius, plan.flute_length, plan.shape);
        let mut feeds = vec![];
        let mut total = 0.;
        for (index, m) in plan.moves.iter().enumerate() {
            let l = length(m.from, m.to);
            if m.motion == Motion::Feed && m.feed > 0. && l > 1e-6 {
                feeds.push(Feed {
                    index,
                    at: total,
                    length: l,
                });
                total += l;
            }
        }
        if feeds.is_empty() {
            return None;
        }
        let stock = volume.frame.transfer_bounds(volume.bounds, frame);
        let bar = volume.round.then(|| crate::stock::bar(volume.bounds));
        let extent = (stock.max[0] - stock.min[0]).max(stock.max[1] - stock.min[1]);
        let mut history = History::new(frame, stock, radius.max(extent / 400.));
        for cut in volume.cuts.iter() {
            history.add(cut.clone());
        }
        let ids: Vec<u32> = feeds
            .iter()
            .map(|f| {
                let m = &plan.moves[f.index];
                history.add(Cut {
                    a: m.from,
                    b: m.to,
                    radius,
                    flute,
                    shape,
                    frame,
                    source,
                })
            })
            .collect();
        let step = (radius / 4.).clamp(0.25, 1.).max(total / MOST);
        Some(Self {
            plan,
            volume,
            frame,
            feeds,
            ids,
            history,
            stock,
            bar,
            total,
            step,
            // Far enough back that the cutter's side, `DEPTH` in, is clear
            // of where it was: a wall it shaves is still there to be found.
            lag: step.max(2. * (2. * radius * DEPTH).sqrt()),
            pad: step.max(radius / 2.),
        })
    }
    /// The feed `s` mm along the path, and how far along it.
    fn locate(&self, s: f32) -> (usize, f32) {
        let k = self.feeds.partition_point(|f| f.at <= s).saturating_sub(1);
        let f = self.feeds[k];
        (k, ((s - f.at) / f.length).clamp(0., 1.))
    }
    fn at(&self, k: usize, t: f32) -> [f32; 3] {
        let m = &self.plan.moves[self.feeds[k].index];
        lerp(m.from, m.to, t)
    }
    /// The stretches of the plan's support passes (`Plan::keep_air`) that
    /// are not proven clear (`clear`), in runs of at most `RUN` mm along
    /// one level path, none of them in `cutting`, what was found cutting.
    fn unproven(&self, cutting: &[[f32; 2]]) -> Result<Vec<[f32; 2]>, String> {
        let feeds = &self.feeds;
        let support = |k: usize| self.plan.keep_air.get(feeds[k].index) == Some(&true);
        let within = |a: f32, b: f32| {
            let i = cutting.partition_point(|c| c[1] < b - 1e-4);
            cutting.get(i).is_some_and(|c| c[0] <= a + 1e-4)
        };
        // The plan's own feeds surely kept: those all within what cuts. A
        // support pass proven clear is not: it is proven only `SUPPORT_GAP`
        // in from its edge, and strips that thin left by several side by
        // side would add up.
        let kept: Vec<bool> = (0..feeds.len())
            .map(|k| within(feeds[k].at, feeds[k].at + feeds[k].length))
            .collect();
        // Runs of support feeds, each `[from, to]` mm along the path and
        // the feed it starts in: a long feed cut in pieces.
        let mut runs: Vec<([f32; 2], usize)> = vec![];
        for k in 0..feeds.len() {
            let (f, m) = (feeds[k], &self.plan.moves[feeds[k].index]);
            if !support(k) || (m.from[2] - m.to[2]).abs() > 1e-4 {
                continue;
            }
            let pieces = (f.length / RUN).ceil().max(1.) as usize;
            for i in 0..pieces {
                let a = f.at + f.length * i as f32 / pieces as f32;
                let b = f.at + f.length * (i + 1) as f32 / pieces as f32;
                match runs.last_mut() {
                    Some(([from, to], start))
                        if i == 0
                            && *to == f.at
                            && f.index == feeds[k - 1].index + 1
                            && self.plan.moves[feeds[*start].index].from[2] == m.from[2]
                            && b - *from <= RUN =>
                    {
                        *to = b
                    }
                    _ => runs.push(([a, b], k)),
                }
            }
        }
        runs.retain(|([a, b], _)| !within(*a, *b));
        let workers = std::thread::available_parallelism().map_or(1, |n| n.get());
        let progress = crate::progress::current();
        let unproven = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..workers)
                .map(|w| {
                    let (runs, kept, progress) = (&runs, &kept, &progress);
                    scope.spawn(move || {
                        let mut out = vec![];
                        for &(run, start) in runs.iter().skip(w).step_by(workers) {
                            if progress.as_ref().is_some_and(|p| p.cancelled()) {
                                return None;
                            }
                            if !self.clear(run, start, kept) {
                                out.push(run);
                            }
                        }
                        Some(out)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().ok().flatten())
                .collect::<Option<Vec<_>>>()
        })
        .ok_or("Generation cancelled.")?;
        Ok(unproven.into_iter().flatten().collect())
    }
    /// Whether a level run of support feeds, `[from, to]` mm along the path
    /// from within feed `start`, is proven clear: all its flat cutter would
    /// sweep, from its tip up its flutes, inside the stock's box and
    /// `SUPPORT_GAP` in from its edge, lies in what flat cuts before it
    /// swept, each from no higher than its tip up to no lower than its
    /// flutes reach (or the stock's top): the earlier operations' cuts and
    /// the feeds of the plan's own that are `kept`. Along the run, a stretch
    /// near enough to one of those cuts' paths that the cutter there, so
    /// drawn in, lies inside that cut is proven exactly (`reach`). The rest
    /// is proven with polygons, each erring on the safe side: what the run
    /// sweeps grown, and what the cuts swept shrunk, by `MARGIN` more than
    /// the arcs' chords and the points' rounding can be off.
    fn clear(&self, [from, to]: [f32; 2], start: usize, kept: &[bool]) -> bool {
        // The feed the run ends in: at a feed's end, that one, not the next.
        let end = self.feeds.partition_point(|f| f.at < to).saturating_sub(1);
        let t = ((to - self.feeds[end].at) / self.feeds[end].length).clamp(0., 1.);
        let tip = self.at(start, 0.)[2];
        let high = (tip + self.plan.flute_length).min(self.stock.max[2]);
        if high <= tip + 1e-4 {
            return true;
        }
        let xy = |p: [f32; 3]| (p[0] as f64, p[1] as f64);
        let (_, t0) = self.locate(from);
        let path: Polygon = std::iter::once(xy(self.at(start, t0)))
            .chain((start..end).map(|k| xy(self.at(k, 1.))))
            .chain(std::iter::once(xy(self.at(end, t))))
            .collect();
        let radius = self.plan.radius;
        let near = path.iter().fold(
            [
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ],
            |b, &(x, y)| {
                let (x, y) = (x as f32, y as f32);
                [
                    b[0].min(x - radius),
                    b[1].min(y - radius),
                    b[2].max(x + radius),
                    b[3].max(y + radius),
                ]
            },
        );
        let first = self.ids[0];
        // The cuts that clear the run's column, and the same by radius, a
        // cut that goes on from where the one before it ended (the cuts
        // come in order) carrying on its path.
        let mut cuts: Vec<([(f64, f64); 2], f64)> = vec![];
        let mut by: Vec<(f32, Vec<Polygon>)> = vec![];
        for (id, c) in self.history.near(near, self.ids[start]) {
            let own = id >= first;
            if c.frame != self.frame
                || c.shape != CutterShape::Flat
                || (own && !kept[(id - first) as usize])
                || c.a[2].max(c.b[2]) > tip + 1e-4
                || c.a[2].min(c.b[2]) + c.flute < high - 1e-4
            {
                continue;
            }
            cuts.push(([xy(c.a), xy(c.b)], c.radius as f64));
            match by.iter_mut().find(|(r, _)| *r == c.radius) {
                Some((_, paths)) if paths.last().and_then(|p| p.last()) == Some(&xy(c.a)) => {
                    paths.last_mut().unwrap().push(xy(c.b))
                }
                Some((_, paths)) => paths.push(vec![xy(c.a), xy(c.b)]),
                None => by.push((c.radius, vec![vec![xy(c.a), xy(c.b)]])),
            }
        }
        // The stretches of the run not proven exactly.
        let swept = (radius - SUPPORT_GAP) as f64;
        let mut rest: Vec<Polygon> = vec![];
        for w in path.windows(2) {
            let (a, b) = (w[0], w[1]);
            let mut spans: Vec<[f64; 2]> = (cuts.iter())
                .filter_map(|&([c, d], r)| reach(a, b, c, d, r - swept - EXACT))
                .collect();
            spans.sort_by(|p, q| p[0].total_cmp(&q[0]));
            if spans.is_empty() {
                rest.push(vec![a, b]);
                continue;
            }
            let length = (b.0 - a.0).hypot(b.1 - a.1);
            let mut done = 0.;
            let gap = |t0: f64, t1: f64, rest: &mut Vec<Polygon>| {
                if t1 > t0 {
                    let at = |t: f64| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                    rest.push(vec![at(t0), at(t1)]);
                }
            };
            for [t0, t1] in spans {
                // A gap under `EXACT` long beside a proven stretch is within
                // `EXACT` of a point proven with `EXACT` to spare.
                if (t0 - done) * length > EXACT {
                    gap(done, t0, &mut rest);
                }
                done = f64::max(done, t1);
            }
            if (1. - done) * length > EXACT {
                gap(done, 1., &mut rest);
            }
        }
        if rest.is_empty() {
            return true;
        }
        // The polygons: an arc's vertices are on it and its chords within
        // `ARC` of its radius inside it (clipper2's own tolerance), and each
        // point, given or worked out, is rounded within `ROUND` of where it
        // was. What the run sweeps is grown and what the cuts swept shrunk
        // by `MARGIN` more than that, so that any point the cuts left within
        // the run's sweep brings a disc `MARGIN` wide into what is left,
        // which no rounding of the difference can lose. A piece of the run
        // too short to sweep a polygon of its own is lengthened.
        for piece in rest.iter_mut() {
            let (a, b) = (piece[0], piece[1]);
            if (b.0 - a.0).hypot(b.1 - a.1) < 0.001 {
                piece[1] = (a.0 + 0.001, a.1);
            }
        }
        let grow = MARGIN + ROUND;
        let (lo, hi) = (self.stock.min, self.stock.max);
        let (lo, hi) = (lo.map(|v| v as f64 - grow), hi.map(|v| v as f64 + grow));
        let stock = vec![vec![
            (lo[0], lo[1]),
            (hi[0], lo[1]),
            (hi[0], hi[1]),
            (lo[0], hi[1]),
        ]];
        let Ok(region) = intersect::<Milli>(
            outline(rest, (swept + MARGIN + 2. * ROUND) / (1. - ARC)),
            stock,
            FillRule::NonZero,
        ) else {
            return false;
        };
        if !region.contains_points() {
            return true;
        }
        let mut cleared: Paths<Milli> = Paths::new(vec![]);
        for (r, paths) in by {
            cleared.append(outline(paths, r as f64 - MARGIN - 2. * ROUND));
        }
        difference::<Milli>(region, cleared, FillRule::NonZero)
            .is_ok_and(|left| !left.contains_points())
    }
    /// A point of material `DEPTH` inside the cutter `s` mm along the path
    /// that was there `lag` before: None where it cuts nothing.
    fn found(&self, s: f32) -> Option<[f32; 3]> {
        let (radius, flute, shape) = (self.plan.radius, self.plan.flute_length, self.plan.shape);
        let (k, t) = self.locate(s);
        let here = self.at(k, t);
        let m = &self.plan.moves[self.feeds[k].index];
        let d = length(m.from, m.to);
        let v: [f32; 3] = std::array::from_fn(|i| (m.to[i] - m.from[i]) / d);
        let (before, partial) = if s - self.lag <= 0. {
            (self.ids[0], None)
        } else {
            let (kb, tb) = self.locate(s - self.lag);
            let mb = &self.plan.moves[self.feeds[kb].index];
            (self.ids[kb], Some((mb.from, self.at(kb, tb))))
        };
        let (volume, top) = (self.volume, self.stock.max[2]);
        let solid = |p: &[f32; 3]| {
            let p = *p;
            if p[2] > top {
                return false;
            }
            let q = self.frame.transfer(p, volume.frame);
            (0..3).all(|k| (volume.bounds.min[k]..=volume.bounds.max[k]).contains(&q[k]))
                && self
                    .bar
                    .is_none_or(|(c, r)| (q[0] - c[0]).hypot(q[1] - c[1]) <= r)
                && !partial.is_some_and(|(a, b)| swept(p, a, b, radius, flute, shape))
                && !self.history.swept(p, before)
        };
        surface(here, v, radius, flute, shape, top).find(solid)
    }
}

/// The stretches of `plan`'s feed path (mm of feed from its first, sorted,
/// apart) that cut, `pad` wide either side: None for a plan without feed.
fn cutting(
    plan: &Plan,
    volume: &Volume,
    frame: Frame,
    source: (u16, u16),
) -> Result<Option<Vec<[f32; 2]>>, String> {
    let Some(detector) = Detector::new(plan, volume, frame, source) else {
        return Ok(None);
    };
    let Detector {
        total,
        step,
        lag,
        pad,
        ..
    } = detector;
    let feeds = &detector.feeds;
    // Each position's answer, the positions dealt round the cores in turn.
    let positions = (total / step).ceil() as usize + 1;
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get());
    let progress = crate::progress::current();
    let found = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|w| {
                let (detector, progress) = (&detector, &progress);
                scope.spawn(move || {
                    let mut out = vec![];
                    for j in (w..positions).step_by(workers) {
                        if progress.as_ref().is_some_and(|p| p.cancelled()) {
                            return None;
                        }
                        let s = (j as f32 * step).min(total);
                        if detector.found(s).is_some() {
                            out.push(s);
                        }
                    }
                    Some(out)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().ok().flatten())
            .collect::<Option<Vec<_>>>()
    })
    .ok_or("Generation cancelled.")?;
    let mut found: Vec<f32> = found.into_iter().flatten().collect();
    found.sort_by(f32::total_cmp);
    // A level move made again where it cut the first time, a finishing
    // pass, is kept; made again where it cut nothing, it is air too. A ramp
    // made again is no finishing pass: Pocket's bands that share a top
    // each ramp down from it the same way, and only the first finds metal
    // all the way.
    let mut first = std::collections::HashMap::new();
    let key = |p: [f32; 3]| p.map(|v| (v * 1e4).round() as i64);
    let mut again = vec![];
    for f in feeds {
        let m = &plan.moves[f.index];
        if (m.from[2] - m.to[2]).abs() >= 1e-4 {
            continue;
        }
        let [a, b] = *first
            .entry((key(m.from), key(m.to)))
            .or_insert([f.at, f.at + f.length]);
        let i = found.partition_point(|&s| s < a);
        if f.at > a && found.get(i).is_some_and(|&s| s <= b) {
            again.push([f.at, f.at + f.length]);
        }
    }
    let cuts = found.into_iter().map(|s| [s - lag - pad, s + pad]);
    let cutting = merged(cuts.chain(again), total);
    let unproven = detector.unproven(&cutting)?;
    Ok(Some(merged(cutting.into_iter().chain(unproven), total)))
}

/// The area a cutter of `radius` sweeps along `paths` in XY, its arcs'
/// vertices on them.
fn outline(paths: Vec<Polygon>, radius: f64) -> Paths<Milli> {
    Paths::<Milli>::from(paths).inflate(radius, JoinType::Round, EndType::Round, 2.)
}

/// The span of the segment `a` to `b` (as fractions of it) within `within`
/// of the segment `c` to `d`, None where none is: the points within that of
/// a segment make a convex set, which the segment crosses once.
fn reach(
    a: (f64, f64),
    b: (f64, f64),
    c: (f64, f64),
    d: (f64, f64),
    within: f64,
) -> Option<[f64; 2]> {
    if within < 0. {
        return None;
    }
    let v = (b.0 - a.0, b.1 - a.1);
    let vv = v.0 * v.0 + v.1 * v.1;
    // Where `a + t v` lies within `within` of the point `q`.
    let disc = |q: (f64, f64)| -> Option<[f64; 2]> {
        let w = (a.0 - q.0, a.1 - q.1);
        let (bb, cc) = (
            v.0 * w.0 + v.1 * w.1,
            w.0 * w.0 + w.1 * w.1 - within * within,
        );
        if vv < 1e-18 {
            return (cc <= 0.).then_some([0., 1.]);
        }
        let root = bb * bb - vv * cc;
        (root >= 0.).then(|| [(-bb - root.sqrt()) / vv, (-bb + root.sqrt()) / vv])
    };
    // Where it lies over the segment itself, within `within` across it.
    let band = || -> Option<[f64; 2]> {
        let e = (d.0 - c.0, d.1 - c.1);
        let l = e.0.hypot(e.1);
        if l < 1e-12 {
            return None;
        }
        let (u, n) = ((e.0 / l, e.1 / l), (-e.1 / l, e.0 / l));
        let w = (a.0 - c.0, a.1 - c.1);
        let mut span = [f64::NEG_INFINITY, f64::INFINITY];
        for (axis, low, high) in [(u, 0., l), (n, -within, within)] {
            let (p, q) = (w.0 * axis.0 + w.1 * axis.1, v.0 * axis.0 + v.1 * axis.1);
            if q.abs() < 1e-18 {
                if p < low || p > high {
                    return None;
                }
            } else {
                let (s, t) = ((low - p) / q, (high - p) / q);
                span = [span[0].max(s.min(t)), span[1].min(s.max(t))];
            }
        }
        (span[0] <= span[1]).then_some(span)
    };
    let [lo, hi] = [disc(c), disc(d), band()]
        .into_iter()
        .flatten()
        .fold([f64::INFINITY, f64::NEG_INFINITY], |s, t| {
            [s[0].min(t[0]), s[1].max(t[1])]
        });
    let (lo, hi) = (lo.max(0.), hi.min(1.));
    (lo <= hi).then_some([lo, hi])
}

/// `stretches` (mm of feed) sorted, the overlapping joined, within the
/// path's `total`.
fn merged(stretches: impl IntoIterator<Item = [f32; 2]>, total: f32) -> Vec<[f32; 2]> {
    let mut stretches: Vec<[f32; 2]> = stretches.into_iter().collect();
    stretches.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut merged: Vec<[f32; 2]> = vec![];
    for [a, b] in stretches {
        let (a, b) = (a.max(0.), b.min(total));
        match merged.last_mut() {
            Some(last) if a <= last[1] => last[1] = last[1].max(b),
            _ => merged.push([a, b]),
        }
    }
    merged
}

/// The points of a cutter of `shape`, `radius` and `flute` with its tip at
/// `tip`, moving along `v`, where material `DEPTH` inside its surface is:
/// round its end and up its side to `top`, every 10° round from the way it
/// moves (so straight across it too), and up its side every `RISE`.
fn surface(
    tip: [f32; 3],
    v: [f32; 3],
    radius: f32,
    flute: f32,
    shape: CutterShape,
    top: f32,
) -> impl Iterator<Item = [f32; 3]> {
    let h = v[0].hypot(v[1]);
    let (u, n) = if h > 1e-6 {
        ([v[0] / h, v[1] / h], [-v[1] / h, v[0] / h])
    } else {
        ([1., 0.], [0., 1.])
    };
    let round = move |every: usize| {
        (0..every).map(move |j| {
            let (s, c) = (j as f32 * std::f32::consts::TAU / every as f32).sin_cos();
            [u[0] * c + n[0] * s, u[1] * c + n[1] * s]
        })
    };
    let rho = radius - DEPTH;
    let ring = move |r: f32, z: f32, every: usize| {
        round(every).map(move |d| [tip[0] + r * d[0], tip[1] + r * d[1], z])
    };
    // The end: a flat end's face, or a ball's lower half.
    let end: Vec<[f32; 3]> = match shape {
        CutterShape::Ball => {
            let centre = tip[2] + radius;
            std::iter::once([tip[0], tip[1], tip[2] + DEPTH])
                .chain((1..=9).flat_map(move |i| {
                    let (s, c) = (i as f32 * 10_f32.to_radians()).sin_cos();
                    ring(rho * s, centre - rho * c, if i <= 3 { 12 } else { 36 })
                }))
                .collect()
        }
        _ => {
            let z = tip[2] + DEPTH;
            std::iter::once([tip[0], tip[1], z])
                .chain(ring(rho / 3., z, 12))
                .chain(ring(rho * 2. / 3., z, 12))
                .chain(ring(rho, z, 36))
                .collect()
        }
    };
    let from = tip[2]
        + match shape {
            CutterShape::Ball => radius,
            _ => DEPTH,
        };
    let to = (tip[2] + flute - DEPTH).min(top);
    let rises = if to > from {
        ((to - from) / RISE).ceil() as usize
    } else {
        0
    };
    let side = (1..=rises).flat_map(move |i| {
        let z = (from + i as f32 * RISE).min(to);
        ring(rho, z, 36)
    });
    end.into_iter().chain(side)
}

/// What a stretch of moves is: one cut, or not, of move `index` from
/// fraction `t[0]` of it to `t[1]`.
#[derive(Clone, Copy)]
struct Piece {
    index: usize,
    t: [f32; 2],
    cuts: bool,
}

/// A move of the plan `relink` makes: the piece of the plan's it keeps (None
/// for one of a link), from, to, its motion and its rate.
type Out = (Option<Piece>, [f32; 3], [f32; 3], Motion, f32);

/// `plan` with each stretch between `cutting` (as `cutting` returns it)
/// that a retract, a rapid and a descent are quicker than replaced by them.
fn relink(
    plan: Plan,
    op: &Operation,
    volume: &Volume,
    frame: Frame,
    cutting: &[[f32; 2]],
    (holding, gap): (&[HoldBox], f32),
    blend: f32,
) -> (Plan, Option<Trimmed>) {
    // The plan's clearance: where its rapids cross.
    let clearance = (plan.moves.iter())
        .filter(|m| m.motion == Motion::Rapid)
        .map(|m| m.from[2].max(m.to[2]))
        .fold(f32::NEG_INFINITY, f32::max);
    if !clearance.is_finite() {
        return (plan, None);
    }
    // A descent rapids down to the retract margin over what the export's
    // check proves its column clear to (`post_check`): the highest material
    // left, lower where the earlier cuts cleared the column (`RestStock`)
    // or where the tool already went down there; then it feeds.
    let margin = num(&op.p, "retract").filter(|v| *v > 0.).unwrap_or(2.);
    // A link goes across the retract margin over the stock top, and no
    // lower than the export lets a rapid go sideways (`post_check`): not up
    // to the plan's clearance to hop a stretch of air.
    let hop =
        (plan.stock.max[2] + margin.max(crate::post_check::BLEND_HEIGHT * blend)).min(clearance);
    let rest = (!volume.cuts.is_empty()).then(|| crate::rest::RestStock::new(volume, frame));
    let high = rest.as_ref().map_or(plan.stock.max[2], |rest| {
        let unbounded = Bounds {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        };
        rest.left(unbounded).max[2]
    });
    let high = high.min(plan.stock.max[2]);
    // The lowest the tool went at each point so far, as the check keeps it.
    let key = |p: [f32; 3]| ((p[0] * 1000.).round() as i64, (p[1] * 1000.).round() as i64);
    let mut visited: std::collections::HashMap<(i64, i64), f32> = Default::default();
    let visit = |p: [f32; 3], visited: &mut std::collections::HashMap<(i64, i64), f32>| {
        let z = visited.entry(key(p)).or_insert(p[2]);
        *z = z.min(p[2]);
    };
    // The height a descent onto `e` rapids down to, and whether the column
    // is proven clear all the way down to `e`.
    let approach = |e: [f32; 3], visited: &std::collections::HashMap<(i64, i64), f32>| {
        let r = plan.radius + blend;
        let column = Bounds {
            min: [e[0] - r, e[1] - r, e[2]],
            max: [e[0] + r, e[1] + r, high],
        };
        let been = visited.get(&key(e)).copied().unwrap_or(f32::INFINITY);
        // The proof only where the tool has not been down there already.
        let proven = match &rest {
            _ if been <= e[2] => e[2],
            Some(rest) if rest.beside(column) => e[2],
            Some(rest) => rest.top(column, e[2] - 0.001, high),
            None => high,
        };
        // Clear only where the earlier cuts or the tool's own moves took
        // the column down to `e`: the top of stock no cut touched is no
        // proof, as the stock may stand higher than its box.
        let floor = proven.min(been).min(high);
        (
            (floor.max(e[2]) + margin).min(clearance),
            floor <= e[2] + 0.001 && floor < high - 0.001,
        )
    };
    // The plan's moves in pieces, each cutting or not.
    let mut pieces = vec![];
    let mut along = 0.;
    for (index, m) in plan.moves.iter().enumerate() {
        let l = length(m.from, m.to);
        if !(m.motion == Motion::Feed && m.feed > 0. && l > 1e-6) {
            pieces.push(Piece {
                index,
                t: [0., 1.],
                cuts: false,
            });
            continue;
        }
        let (a, b) = (along, along + l);
        let mut t = 0.;
        for &[c0, c1] in cutting.iter().filter(|c| c[1] > a && c[0] < b) {
            let (t0, t1) = (((c0 - a) / l).max(0.), ((c1 - a) / l).min(1.));
            if t0 > t {
                pieces.push(Piece {
                    index,
                    t: [t, t0],
                    cuts: false,
                });
            }
            pieces.push(Piece {
                index,
                t: [t0.max(t), t1],
                cuts: true,
            });
            t = t1;
        }
        if t < 1. {
            pieces.push(Piece {
                index,
                t: [t, 1.],
                cuts: false,
            });
        }
        along = b;
    }
    let moves = &plan.moves;
    let from = |p: &Piece| lerp(moves[p.index].from, moves[p.index].to, p.t[0]);
    let to = |p: &Piece| lerp(moves[p.index].from, moves[p.index].to, p.t[1]);
    let seconds = |p: &Piece| (moves[p.index].end - moves[p.index].start) * (p.t[1] - p.t[0]);
    // A plan that cuts nothing is left as it is, and said.
    if !pieces.iter().any(|p| p.cuts) {
        let nothing = Trimmed {
            nothing: true,
            ..Default::default()
        };
        return (plan, Some(nothing));
    }
    // The level each move cuts at, or the one before it where it cuts at
    // none (a flat feed's Z), and the one after: a link from one level to
    // another keeps going up to clearance, where a run on the UC100 splits
    // the operation into the steps it can start at (`run::plan_steps`).
    let flat = |m: &Move| m.motion == Motion::Feed && (m.from[2] - m.to[2]).abs() < 1e-4;
    let mut before = vec![None; moves.len()];
    let mut level = None;
    for (k, m) in moves.iter().enumerate() {
        if flat(m) {
            level = Some(m.to[2]);
        }
        before[k] = level;
    }
    let mut after = vec![None; moves.len()];
    level = None;
    for (k, m) in moves.iter().enumerate().rev() {
        if flat(m) {
            level = Some(m.to[2]);
        }
        after[k] = level;
    }
    // A link's descent feeds at the plan's cutting feed down a column
    // proven clear, as Adaptive's own links feed down, ending `pad` short
    // of what the cutter meets; elsewhere, onto stock no earlier cut
    // touched, which may stand higher than its box, at the plunge rate the
    // strategies' own entries take (`cam::Builder::start_at`).
    let feed = (moves.iter())
        .filter(|m| m.motion == Motion::Feed && m.cutting_pass)
        .map(|m| m.feed)
        .fold(0., f32::max);
    let mut out: Vec<Out> = vec![];
    let keep = |p: &Piece, out: &mut Vec<_>, visited: &mut _| {
        let m = &moves[p.index];
        let (a, b) = (from(p), to(p));
        // A move's own ends only: where it is cut in pieces that are kept
        // whole again, the export never sees the point.
        if p.t[0] <= 0. {
            visit(a, visited);
        }
        if p.t[1] >= 1. {
            visit(b, visited);
        }
        out.push((Some(*p), a, b, m.motion, m.feed));
    };
    let mut trimmed = Trimmed::default();
    let mut i = 0;
    while i < pieces.len() {
        if pieces[i].cuts {
            keep(&pieces[i], &mut out, &mut visited);
            i += 1;
            continue;
        }
        let j = (i..pieces.len())
            .find(|&j| pieces[j].cuts)
            .unwrap_or(pieces.len());
        let stretch = &pieces[i..j];
        let air: f32 = (stretch.iter())
            .filter(|p| moves[p.index].motion == Motion::Feed)
            .map(|p| length(from(p), to(p)))
            .sum();
        let dwells = stretch
            .iter()
            .any(|p| moves[p.index].motion == Motion::Dwell);
        let start = from(&stretch[0]);
        let end = to(&stretch[stretch.len() - 1]);
        // The link: up to `hop`, across, down at rapid to the approach,
        // then fed down: the column over a point the cutter reached through
        // air is clear (as Adaptive's links take it), and the point is
        // `pad` short of what it cuts. Up to clearance where across at `hop`
        // would meet the workholding.
        let link_at = |height: f32| {
            let mut link: Vec<([f32; 3], Motion, f32)> = vec![];
            let mut at = start;
            let mut go = |p: [f32; 3], motion: Motion, rate: f32, at: &mut [f32; 3]| {
                if length(*at, p) > 1e-5 {
                    link.push((p, motion, rate));
                    *at = p;
                }
            };
            go(
                [at[0], at[1], at[2].max(height)],
                Motion::Rapid,
                RAPID,
                &mut at,
            );
            if j < pieces.len() {
                let (approach, clear) = approach(end, &visited);
                go([end[0], end[1], at[2]], Motion::Rapid, RAPID, &mut at);
                if end[2] >= approach {
                    go(end, Motion::Rapid, RAPID, &mut at);
                } else {
                    let rate = if clear { feed } else { feed * 0.25 };
                    go([end[0], end[1], approach], Motion::Rapid, RAPID, &mut at);
                    go(end, Motion::Feed, rate, &mut at);
                }
            }
            link
        };
        let clear = |link: &[([f32; 3], Motion, f32)]| {
            let mut was = start;
            let segments: Vec<([f32; 3], [f32; 3])> = (link.iter())
                .map(|&(p, _, _)| (std::mem::replace(&mut was, p), p))
                .collect();
            crate::holding::hits_segments(
                &segments,
                plan.radius,
                plan.tool_length,
                plan.holder_radius,
                holding,
                gap,
            )
            .is_empty()
        };
        // The plan's last retract stays at clearance, where the next
        // operation starts.
        let across = j < pieces.len()
            && match (before[stretch[0].index], after[pieces[j].index]) {
                (Some(a), Some(b)) => (a - b).abs() < 0.005,
                _ => false,
            };
        let link = Some(link_at(hop))
            .filter(|link| across && clear(link))
            .unwrap_or_else(|| link_at(clearance));
        let mut was = start;
        let link_seconds: f32 = (link.iter())
            .map(|&(p, _, rate)| {
                let l = length(was, p);
                was = p;
                l / rate * 60.
            })
            .sum();
        let stretch_seconds: f32 = stretch.iter().map(seconds).sum();
        let replace = air > 0.
            && !dwells
            && stretch_seconds - link_seconds > SAVING
            && link_seconds < SHARE * stretch_seconds
            && clear(&link);
        // Otherwise a link of the plan's own across higher than `hop`, as
        // Pocket's up to clearance between its loops, is lowered to it: no
        // rapid of it higher, its descents and feeds as they were.
        let lower = |q: [f32; 3]| [q[0], q[1], q[2].min(hop)];
        let rapid = |p: &Piece| moves[p.index].motion == Motion::Rapid;
        let lowered = !replace
            && !dwells
            && across
            && start[2] <= hop + 1e-4
            && stretch.iter().any(|p| {
                let (a, b) = (from(p), to(p));
                rapid(p)
                    && length([a[0], a[1], 0.], [b[0], b[1], 0.]) > 1e-5
                    && a[2].min(b[2]) > hop + 1e-4
            })
            && (stretch.iter()).all(|p| rapid(p) || from(p)[2].max(to(p)[2]) <= hop + 1e-4)
            && clear(
                &(stretch.iter())
                    .map(|p| (lower(to(p)), Motion::Rapid, RAPID))
                    .collect::<Vec<_>>(),
            );
        if replace {
            trimmed.stretches += 1;
            trimmed.air += air;
            let mut was = start;
            for (p, motion, rate) in link {
                visit(p, &mut visited);
                out.push((None, was, p, motion, rate));
                was = p;
            }
        } else {
            trimmed.lowered += usize::from(lowered);
            for p in stretch {
                let (a, b) = if lowered && rapid(p) {
                    (lower(from(p)), lower(to(p)))
                } else {
                    (from(p), to(p))
                };
                // A rapid all over `hop` is gone once lowered.
                if lowered && rapid(p) && length(a, b) <= 1e-6 {
                    continue;
                }
                // A rapid of the plan's own straight down into the stock,
                // Pocket's to just over the level before, may have been
                // proven by where the tool went down in a stretch replaced
                // earlier (LIO-306): where it no longer is, it stops at the
                // approach and feeds on down.
                let down = rapid(p)
                    && length([a[0], a[1], 0.], [b[0], b[1], 0.]) <= 1e-5
                    && b[2] < a[2]
                    && b[2] < high;
                let (approach, clear) = if down {
                    approach(b, &visited)
                } else {
                    (b[2], true)
                };
                if b[2] >= approach - 1e-4 {
                    if (a, b) == (from(p), to(p)) {
                        keep(p, &mut out, &mut visited);
                    } else {
                        out.push((None, a, b, Motion::Rapid, RAPID));
                        visit(b, &mut visited);
                    }
                    continue;
                }
                let mut was = a;
                if a[2] > approach {
                    let over = [b[0], b[1], approach];
                    out.push((None, a, over, Motion::Rapid, RAPID));
                    was = over;
                }
                let rate = if clear { feed } else { feed * 0.25 };
                out.push((None, was, b, Motion::Feed, rate));
                visit(b, &mut visited);
            }
        }
        i = j;
    }
    if trimmed.stretches == 0 && trimmed.lowered == 0 {
        return (plan, None);
    }
    // Whole moves again where a move's pieces were all kept, and timed.
    let mut moves_out: Vec<Move> = vec![];
    let mut keep_air = vec![];
    // The move and fraction the last one kept ends at.
    let mut ends: Option<(usize, f32)> = None;
    let mut clock = 0.;
    for (piece, from, to, motion, rate) in out {
        let seconds = match piece {
            Some(p) => seconds(&p),
            None => length(from, to) / rate * 60.,
        };
        if let (Some(p), Some((index, t)), Some(last)) = (piece, ends, moves_out.last_mut())
            && p.index == index
            && p.t[0] == t
        {
            last.to = to;
            last.end += seconds;
            clock = last.end;
            ends = Some((p.index, p.t[1]));
            continue;
        }
        let original = piece.map(|p| &moves[p.index]);
        keep_air.push(piece.is_some_and(|p| plan.keep_air.get(p.index).copied().unwrap_or(false)));
        moves_out.push(Move {
            from,
            to,
            motion,
            cutting_pass: original.is_some_and(|o| o.cutting_pass),
            feed: rate,
            start: clock,
            end: clock + seconds,
            a: original.and_then(|o| o.a),
        });
        clock += seconds;
        ends = piece.map(|p| (p.index, p.t[1]));
    }
    let mut plan = plan;
    trimmed.saved = plan.duration - clock;
    plan.moves = moves_out;
    plan.keep_air = keep_air;
    plan.duration = clock;
    (plan, Some(trimmed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Machine;
    const STOCK: Bounds = Bounds {
        min: [-100., -50., 0.],
        max: [100., 50., 40.],
    };
    fn frame() -> Frame {
        let mesh =
            crate::mesh::normalize(vec![[[0., 0., 0.], [20., 0., 0.], [0., 20., 10.]]]).unwrap();
        Frame::new(&Setup::new(Machine::new(3), "G54"), &mesh)
    }
    /// `STOCK` less a Ø10 flat end mill's cuts along `earlier`.
    fn stock(earlier: &[([f32; 3], [f32; 3])]) -> Volume {
        let mut volume = Volume::new(STOCK, frame());
        for &(a, b) in earlier {
            volume.cut(a, b, 5., 25., CutterShape::Flat, frame(), (0, 0));
        }
        volume
    }
    /// A Ø10 flat end mill's Pocket from clearance Z50 down at `points[0]`
    /// and fed at 600 mm/min through the rest, then up to clearance.
    fn plan(points: &[[f32; 3]]) -> Plan {
        let p0 = points[0];
        let mut moves = vec![
            (Motion::Rapid, [p0[0], p0[1], 45.], 6000.),
            (Motion::Feed, p0, 150.),
        ];
        moves.extend(points[1..].iter().map(|&p| (Motion::Feed, p, 600.)));
        let last = points[points.len() - 1];
        moves.push((Motion::Rapid, [last[0], last[1], 50.], 6000.));
        let mut plan = crate::post::fixture::plan([p0[0], p0[1], 50.], &moves);
        (plan.stock, plan.radius, plan.flute_length) = (STOCK, 5., 25.);
        plan.strategy = Strategy::Pocket;
        plan
    }
    fn trimmed(plan: Plan, volume: &Volume) -> (Plan, Option<Trimmed>) {
        let setup = Setup::new(Machine::new(3), "G54");
        let op = Operation::new(Strategy::Pocket, "tool".into());
        trim(plan, &setup, &op, volume, frame(), (0, 0), &[]).unwrap()
    }
    /// The feed moves at Z `z` that reach between X `x0` and `x1`.
    fn feeds_between(plan: &Plan, z: f32, x0: f32, x1: f32) -> usize {
        (plan.moves.iter())
            .filter(|m| m.motion == Motion::Feed && m.from[2] == z && m.to[2] == z)
            .filter(|m| m.from[0].max(m.to[0]) > x0 && m.from[0].min(m.to[0]) < x1)
            .count()
    }
    /// `from` to `to` in moves `step` long.
    fn fine(from: [f32; 3], to: [f32; 3], step: f32) -> Vec<[f32; 3]> {
        let n = (length(from, to) / step).round() as usize;
        (0..=n)
            .map(|i| lerp(from, to, i as f32 / n as f32))
            .collect()
    }
    fn continuous(plan: &Plan) {
        for w in plan.moves.windows(2) {
            assert_eq!(w[0].to, w[1].from);
            assert!((w[0].end - w[1].start).abs() < 1e-3);
        }
        assert!((plan.moves.last().unwrap().end - plan.duration).abs() < 1e-3);
    }

    #[test]
    fn a_support_pass_through_air_is_trimmed_and_its_flag_kept_where_it_cuts() {
        // A Pocket support pass along Y0 and a pass back along Y20, each
        // through a slot cleared from X-60 to 60: both cut only at the
        // ends (LIO-337 kept the support pass whole, so Pocket fed through
        // levels the bands before had cleared).
        let volume = stock(&[
            ([-60., 0., 30.], [60., 0., 30.]),
            ([-60., 20., 30.], [60., 20., 30.]),
        ]);
        let mut original = plan(&[
            [-100., 0., 30.],
            [100., 0., 30.],
            [100., 20., 30.],
            [-100., 20., 30.],
        ]);
        original.keep_air = vec![false; original.moves.len()];
        original.keep_air[2] = true;
        let middle = |plan: &Plan, y: f32| {
            plan.moves.iter().any(|m| {
                m.motion == Motion::Feed
                    && m.from[1] == y
                    && m.to[1] == y
                    && m.from[0].min(m.to[0]) < -20.
                    && m.from[0].max(m.to[0]) > 20.
            })
        };
        let (kept, air) = trimmed(original, &volume);
        assert_eq!(air.map(|air| air.stretches), Some(2), "{air:?}");
        assert!(!middle(&kept, 0.));
        assert!(!middle(&kept, 20.));
        // The flag stays on the support pass's ends and on nothing else.
        assert_eq!(kept.keep_air.len(), kept.moves.len());
        for (m, &keep) in kept.moves.iter().zip(&kept.keep_air) {
            let support =
                m.motion == Motion::Feed && m.from[1] == 0. && m.to[1] == 0. && m.from[2] == 30.;
            assert_eq!(keep, support, "{m:?}");
        }
        assert!(kept.keep_air.iter().filter(|&&keep| keep).count() == 2);
        continuous(&kept);
    }
    #[test]
    fn a_support_pass_shaving_a_strip_thinner_than_depth_is_kept() {
        // A Ø3 pass along Y-0.015 at Z36 leaves a strip 0.015 mm thick and
        // 4 mm tall beside a support pass along Y0 at Z36: under `DEPTH`,
        // but the Z32 level after it would meet it 8 mm tall (found by the
        // acceptance review of LIO-352). A plain pass there is still air.
        let mut volume = Volume::new(STOCK, frame());
        let flat = CutterShape::Flat;
        volume.cut(
            [-100., -0.015, 36.],
            [100., -0.015, 36.],
            1.5,
            25.,
            flat,
            frame(),
            (0, 0),
        );
        let strip = [0., 1.49, 38.];
        assert!(
            !volume
                .cuts
                .iter()
                .any(|c| swept(strip, c.a, c.b, c.radius, c.flute, c.shape))
        );
        let mut original = plan(&[
            [-80., 0., 36.],
            [80., 0., 36.],
            [80., 0., 32.],
            [-80., 0., 32.],
        ]);
        original.radius = 1.5;
        let shaves = |plan: &Plan| {
            plan.moves.iter().any(|m| {
                m.motion == Motion::Feed
                    && m.from[2] == 36.
                    && m.to[2] == 36.
                    && swept(
                        strip,
                        m.from,
                        m.to,
                        plan.radius,
                        plan.flute_length,
                        plan.shape,
                    )
            })
        };
        assert!(shaves(&original));
        let (plain, _) = trimmed(original.clone(), &volume);
        assert!(!shaves(&plain));
        original.keep_air = vec![false; original.moves.len()];
        original.keep_air[2] = true;
        let (support, _) = trimmed(original, &volume);
        assert!(shaves(&support));
        continuous(&support);
    }
    #[test]
    fn a_support_pass_over_a_remnant_between_its_samples_is_kept() {
        // Ø3 passes at Z36 along Y-0.015 and along Y0 but for a gap 0.3 mm
        // long at X0.0625: the remnant they leave, 0.0075 mm thick, lies
        // between the detector's samples of a support pass along Y0, and
        // the Z32 level after it would meet it 8 mm tall (found by the
        // acceptance review of LIO-352).
        let mut volume = Volume::new(STOCK, frame());
        let (c, flat) = (0.0625, CutterShape::Flat);
        for (a, b) in [
            ([-100., -0.015, 36.], [100., -0.015, 36.]),
            ([-100., 0., 36.], [c - 0.15, 0., 36.]),
            ([c + 0.15, 0., 36.], [100., 0., 36.]),
        ] {
            volume.cut(a, b, 1.5, 25., flat, frame(), (0, 0));
        }
        let remnant = [c, 1.496, 38.];
        assert!(
            !volume
                .cuts
                .iter()
                .any(|k| swept(remnant, k.a, k.b, k.radius, k.flute, k.shape))
        );
        let mut original = plan(&[
            [-80., 0., 36.],
            [80., 0., 36.],
            [80., 0., 32.],
            [-80., 0., 32.],
        ]);
        original.radius = 1.5;
        original.keep_air = vec![false; original.moves.len()];
        original.keep_air[2] = true;
        let (kept, _) = trimmed(original, &volume);
        assert!(kept.moves.iter().any(|m| {
            m.motion == Motion::Feed
                && m.from[2] == 36.
                && m.to[2] == 36.
                && swept(
                    remnant,
                    m.from,
                    m.to,
                    kept.radius,
                    kept.flute_length,
                    kept.shape,
                )
        }));
        // Away from the remnant, the support pass is still taken out.
        assert!(!kept.moves.iter().any(|m| {
            m.motion == Motion::Feed
                && m.from[2] == 36.
                && m.from[0].min(m.to[0]) < -40.
                && m.from[0].max(m.to[0]) > -20.
        }));
        continuous(&kept);
    }
    #[test]
    fn support_passes_taken_out_side_by_side_leave_no_wider_strip() {
        // Six support passes at Z36, each 0.002 mm to the side of the one
        // before, over a slot cut along Y0: one taken out may not count as
        // cutting all it would for the next, or their 0.003 mm bands add up
        // to a strip 0.012 mm wide that the Z32 level after them meets 8 mm
        // tall (found by the acceptance review of LIO-352).
        let mut volume = Volume::new(STOCK, frame());
        let flat = CutterShape::Flat;
        volume.cut(
            [-100., 0., 36.],
            [100., 0., 36.],
            1.5,
            25.,
            flat,
            frame(),
            (0, 0),
        );
        let mut points = vec![[-80., 0.002, 36.], [80., 0.002, 36.]];
        for j in 2..=6 {
            let x = points[points.len() - 1][0];
            points.extend([[x, j as f32 * 0.002, 36.], [-x, j as f32 * 0.002, 36.]]);
        }
        let end = points[points.len() - 1];
        points.extend([[end[0], end[1], 32.], [-end[0], end[1], 32.]]);
        let mut original = plan(&points);
        original.radius = 1.5;
        let level = |m: &Move, z: f32| m.motion == Motion::Feed && m.from[2] == z && m.to[2] == z;
        original.keep_air = original.moves.iter().map(|m| level(m, 36.)).collect();
        let (kept, _) = trimmed(original, &volume);
        let at = |p: &Plan, q: [f32; 3], z: f32| {
            (p.moves.iter())
                .any(|m| level(m, z) && swept(q, m.from, m.to, p.radius, p.flute_length, p.shape))
        };
        // Across the strip the passes add over the slot, X0, Y1.5 to 1.512:
        // all of it cut at Z36, or else not met first by the Z32 level.
        for i in 1..120 {
            let q = [0., 1.5 + i as f32 * 0.0001, 38.];
            assert!(at(&kept, q, 36.) || !at(&kept, q, 32.), "{q:?}");
        }
        continuous(&kept);
    }
    #[test]
    fn a_support_pass_rounding_its_end_over_a_cap_is_kept() {
        // A slot clears the stock up to Y1.4955 at Z36. A Ø3 support pass
        // comes in along it at an angle and stops at X0 Y0, where its round
        // end reaches Y1.5 over a cap 0.0045 mm thick, which the Z32 level
        // after it would meet 8 mm tall (found by the acceptance review of
        // LIO-352, with a Ø10 slot and a Ø3 one).
        for prior in [1.5, 5.] {
            for deg in [1_f32, 5., 10., 30., 60.] {
                let (sin, cos) = deg.to_radians().sin_cos();
                let mut volume = Volume::new(STOCK, frame());
                let (edge, flat) = (1.4955, CutterShape::Flat);
                let y = edge - prior;
                volume.cut(
                    [-150., y, 36.],
                    [150., y, 36.],
                    prior,
                    25.,
                    flat,
                    frame(),
                    (0, 0),
                );
                let a = [-80. * cos, -80. * sin, 36.];
                let mut original = plan(&[a, [0., 0., 36.], a, [a[0], a[1], 32.], [0., 0., 32.]]);
                original.radius = 1.5;
                let level =
                    |m: &Move, z: f32| m.motion == Motion::Feed && m.from[2] == z && m.to[2] == z;
                original.keep_air = original.moves.iter().map(|m| level(m, 36.)).collect();
                let (kept, _) = trimmed(original, &volume);
                let cap = [0., 1.4958, 38.];
                assert!(
                    !volume
                        .cuts
                        .iter()
                        .any(|c| swept(cap, c.a, c.b, c.radius, c.flute, c.shape))
                );
                let at = |z: f32| {
                    (kept.moves.iter()).any(|m| {
                        level(m, z)
                            && swept(
                                cap,
                                m.from,
                                m.to,
                                kept.radius,
                                kept.flute_length,
                                kept.shape,
                            )
                    })
                };
                assert!(at(36.) || !at(32.), "Ø{} at {deg}°", 2. * prior);
            }
        }
    }
    #[test]
    fn a_support_pass_over_a_cap_rounding_loses_is_kept() {
        // A Ø8 slot at an angle leaves a cap 0.0033 mm thick under the end
        // of a Ø3 support pass, 0.00307 mm inside its edge at `cap`: the
        // polygons' difference, rounded to `Milli`, came out empty (found
        // by the acceptance review of LIO-352).
        let mut volume = Volume::new(STOCK, frame());
        let (c, d) = ([-47.72959, 87.91031, 36.], [52.06938, -85.41068, 36.]);
        volume.cut(c, d, 4., 25., CutterShape::Flat, frame(), (0, 0));
        let (from, b) = ([-0.18829408_f32, 0.46892112], [0.000541_f32, 0.000689]);
        let l = (from[0] - b[0]).hypot(from[1] - b[1]);
        let a = [
            b[0] + (from[0] - b[0]) * 20. / l,
            b[1] + (from[1] - b[1]) * 20. / l,
            36.,
        ];
        let b = [b[0], b[1], 36.];
        let mut original = plan(&[a, b, a, [a[0], a[1], 32.], [b[0], b[1], 32.]]);
        original.radius = 1.5;
        let level = |m: &Move, z: f32| m.motion == Motion::Feed && m.from[2] == z && m.to[2] == z;
        original.keep_air = original.moves.iter().map(|m| level(m, 36.)).collect();
        let (kept, _) = trimmed(original, &volume);
        let cap = [-1.2967059, -0.7462714, 38.];
        assert!(
            !volume
                .cuts
                .iter()
                .any(|k| swept(cap, k.a, k.b, k.radius, k.flute, k.shape))
        );
        let at = |z: f32| {
            (kept.moves.iter()).any(|m| {
                level(m, z)
                    && swept(
                        cap,
                        m.from,
                        m.to,
                        kept.radius,
                        kept.flute_length,
                        kept.shape,
                    )
            })
        };
        assert!(at(36.) || !at(32.));
    }
    #[test]
    fn a_support_feed_that_goes_nowhere_over_stock_is_not_clear() {
        // Straight up 0.000004 mm, level by the plan's measure: a point of
        // the path, with no cut near it.
        let volume = Volume::new(STOCK, frame());
        let mut plan = plan(&[[0., 0., 36.], [0., 0., 36.000004]]);
        plan.moves = vec![plan.moves[2].clone()];
        plan.keep_air = vec![true];
        let detector = Detector::new(&plan, &volume, frame(), (0, 1)).unwrap();
        assert!(!detector.clear([0., detector.total], 0, &[false]));
    }
    #[test]
    fn a_support_pass_proven_clear_leaves_nothing_past_its_band() {
        // A Ø3 or Ø10 support pass ending beside a wider slot whose edge
        // runs past its end at every angle, less than `SUPPORT_GAP` short
        // of it or more: proven clear only where all the pass sweeps but its
        // band is cut.
        let mut checked = 0;
        for radius in [1.5_f32, 5.] {
            let gap = radius as f64 - (radius - SUPPORT_GAP) as f64;
            for j in 0..400 {
                let theta = (j as f64 * 2.399963229728653) % std::f64::consts::TAU;
                let (n, u) = ([theta.cos(), theta.sin()], [-theta.sin(), theta.cos()]);
                let prior = 2. * radius as f64 + 1.;
                // How far inside the pass's edge the slot's edge stops.
                let short = gap * (j % 8) as f64 / 4.;
                let centre = [
                    -n[0] * (prior - radius as f64 + short),
                    -n[1] * (prior - radius as f64 + short),
                ];
                let mut volume = Volume::new(STOCK, frame());
                let end = |k: f64| {
                    [
                        (centre[0] + u[0] * k) as f32,
                        (centre[1] + u[1] * k) as f32,
                        36.,
                    ]
                };
                volume.cut(
                    end(-100.),
                    end(100.),
                    prior as f32,
                    25.,
                    CutterShape::Flat,
                    frame(),
                    (0, 0),
                );
                let a = [
                    (-n[0] * 0.07 - u[0] * 0.5) as f32,
                    (-n[1] * 0.07 - u[1] * 0.5) as f32,
                    36.,
                ];
                let mut plan = plan(&[a, [0., 0., 36.]]);
                plan.radius = radius;
                plan.moves = vec![plan.moves[2].clone()];
                plan.keep_air = vec![true];
                let detector = Detector::new(&plan, &volume, frame(), (0, 1)).unwrap();
                if detector.clear([0., detector.total], 0, &[false]) {
                    // The point of the pass's sweep farthest across the slot.
                    let far = [n[0] * (radius as f64 - gap), n[1] * (radius as f64 - gap)];
                    let reach = (far[0] - centre[0]) * n[0] + (far[1] - centre[1]) * n[1];
                    assert!(
                        reach <= prior + 1e-9,
                        "Ø{} at {theta}: {short} short",
                        2. * radius
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 0);
    }
    #[test]
    fn reach_is_the_span_of_a_segment_within_a_distance_of_another() {
        let near = |a, b, c, d, w| reach(a, b, c, d, w).map(|s| s.map(|t| (t * 1e6).round() / 1e6));
        // Along a segment 1 above it: within 1 from X0 to 10 of 20.
        assert_eq!(
            near((-5., 1.), (15., 1.), (0., 0.), (10., 0.), 1.),
            Some([0.25, 0.75])
        );
        // Round its ends: within 2, over X-1.732 to 11.732.
        let s = near((-5., 1.), (15., 1.), (0., 0.), (10., 0.), 2.).unwrap();
        assert!(
            (s[0] - (5. - 3f64.sqrt()) / 20.).abs() < 1e-6
                && (s[1] - (15. + 3f64.sqrt()) / 20.).abs() < 1e-6
        );
        // Too far, and a point.
        assert_eq!(near((-5., 3.), (15., 3.), (0., 0.), (10., 0.), 2.), None);
        assert_eq!(
            near((3., 1.), (3., 1.), (0., 0.), (10., 0.), 1.),
            Some([0., 1.])
        );
        assert_eq!(near((3., 1.5), (3., 1.5), (0., 0.), (10., 0.), 1.), None);
    }
    #[test]
    fn a_ramp_made_again_over_air_is_air() {
        // A ramp from Z30 down to Z20 along X-40 to 40 and back, cutting,
        // then the same ramp again after the floor under it was cleared to
        // Z20: the second is air, though the first cut.
        let volume = stock(&[]);
        let ramp = [[-40., 0., 30.], [40., 0., 25.], [-40., 0., 20.]];
        let mut points = vec![ramp[0], ramp[1], ramp[2]];
        points.extend(fine([-40., 0., 20.], [40., 0., 20.], 5.));
        points.push([40., 0., 30.]);
        points.extend(fine([40., 0., 30.], [-40., 0., 30.], 5.));
        points.extend(ramp);
        let original = plan(&points);
        let (plan, air) = trimmed(original, &volume);
        assert!(air.is_some_and(|air| air.stretches >= 1), "{air:?}");
        let ramps = (plan.moves.iter())
            .filter(|m| m.motion == Motion::Feed && m.from[2] != m.to[2] && m.from[0] != m.to[0])
            .count();
        assert_eq!(ramps, 2, "{:?}", plan.moves);
        continuous(&plan);
    }
    #[test]
    fn a_pass_through_a_cleared_slot_rises_over_it() {
        // A slot cleared along X from -60 to 60 at Z30, and a pass along it
        // at its floor from X-100 to 100: it cuts at either end only.
        let volume = stock(&[([-60., 0., 30.], [60., 0., 30.])]);
        let original = plan(&[[-100., 0., 30.], [100., 0., 30.]]);
        let (plan, air) = trimmed(original.clone(), &volume);
        let air = air.expect("trimmed");
        assert_eq!(air.stretches, 1);
        assert!(!air.nothing);
        assert!(air.air > 100. && air.air < 130., "{air:?}");
        continuous(&plan);
        // The ends are still cut, and nothing is fed along the slot's middle.
        assert!(feeds_between(&plan, 30., -100., -66.) > 0);
        assert!(feeds_between(&plan, 30., 66., 100.) > 0);
        assert_eq!(feeds_between(&plan, 30., -50., 50.), 0, "{:#?}", plan.moves);
        // Up to 5 mm over the stock top, not to clearance (Z50), across it
        // there and fed down from there at the plunge rate: the slot is no
        // wider than the cutter, so no column over it is proven clear.
        let up = (plan.moves.iter())
            .position(|m| m.motion == Motion::Rapid && m.to[2] == 45. && m.from[2] == 30.)
            .expect("a retract");
        let [across, feed] = [up + 1, up + 2].map(|i| &plan.moves[i]);
        assert_eq!((across.motion, across.to[2]), (Motion::Rapid, 45.));
        assert_eq!(
            (feed.motion, feed.feed, feed.to[2]),
            (Motion::Feed, 150., 30.)
        );
        assert!(plan.moves[up].from[0] > -66. && feed.to[0] < 66.);
        assert!((air.saved - (original.duration - plan.duration)).abs() < 1e-3);
        assert!(air.saved > 3., "{air:?}");
    }
    #[test]
    fn a_plans_own_link_up_to_clearance_goes_across_just_over_the_stock() {
        // Two cuts into the stock at Z30, linked as Pocket links its loops:
        // up to clearance (Z50), across, down to 5 mm over the top and fed
        // in. Too little air to replace, so the link is lowered: across at
        // 5 mm over the top, its descent as it was, the last retract still
        // to clearance.
        let moves = [
            (Motion::Rapid, [-100., 0., 45.], 6000.),
            (Motion::Feed, [-100., 0., 30.], 150.),
            (Motion::Feed, [-70., 0., 30.], 600.),
            (Motion::Rapid, [-70., 0., 50.], 6000.),
            (Motion::Rapid, [70., 0., 50.], 6000.),
            (Motion::Rapid, [70., 0., 45.], 6000.),
            (Motion::Feed, [70., 0., 30.], 150.),
            (Motion::Feed, [100., 0., 30.], 600.),
            (Motion::Rapid, [100., 0., 50.], 6000.),
        ];
        let mut original = crate::post::fixture::plan([-100., 0., 50.], &moves);
        (original.stock, original.radius, original.flute_length) = (STOCK, 5., 25.);
        original.strategy = Strategy::Pocket;
        let (plan, air) = trimmed(original.clone(), &stock(&[]));
        let air = air.expect("lowered");
        assert_eq!((air.stretches, air.lowered), (0, 1), "{air:?}");
        continuous(&plan);
        let across: Vec<&Move> = (plan.moves.iter())
            .filter(|m| m.motion == Motion::Rapid && m.from[0] != m.to[0])
            .collect();
        assert_eq!(across.len(), 1);
        assert_eq!(
            (across[0].from, across[0].to),
            ([-70., 0., 45.], [70., 0., 45.])
        );
        let descent = (plan.moves.iter())
            .find(|m| m.to == [70., 0., 30.])
            .unwrap();
        assert_eq!(
            (descent.from, descent.motion, descent.feed),
            ([70., 0., 45.], Motion::Feed, 150.)
        );
        assert_eq!(plan.moves.last().unwrap().to, [100., 0., 50.]);
        assert!(air.saved > 0. && plan.duration < original.duration);
    }
    #[test]
    fn a_link_across_a_clamp_over_the_stock_top_goes_up_to_clearance() {
        // The slot of `a_pass_through_a_cleared_slot_rises_over_it` with a
        // clamp across its middle standing to Z44, 4 mm over the stock top:
        // across at 5 mm over the top the holder would pass within the gap
        // of it, so the link goes across at clearance.
        let volume = stock(&[([-60., 0., 30.], [60., 0., 30.])]);
        let clamp = HoldBox {
            label: "clamp".into(),
            bounds: Bounds {
                min: [-5., -30., 40.],
                max: [5., 30., 44.],
            },
            shape: Default::default(),
        };
        let setup = Setup::new(Machine::new(3), "G54");
        let op = Operation::new(Strategy::Pocket, "tool".into());
        let original = plan(&[[-100., 0., 30.], [100., 0., 30.]]);
        let (plan, air) = trim(original, &setup, &op, &volume, frame(), (0, 0), &[clamp]).unwrap();
        assert_eq!(air.expect("trimmed").stretches, 1);
        continuous(&plan);
        let across = (plan.moves.iter())
            .find(|m| m.motion == Motion::Rapid && m.from[2] == m.to[2] && m.from[0] < m.to[0])
            .expect("a rapid across");
        assert_eq!(across.to[2], 50.);
    }
    #[test]
    fn a_descent_into_a_column_the_earlier_cuts_cleared_rapids_to_just_over_it() {
        // A field cleared from Z10 up, X -65 to 65 and Y -20 to 20, and a
        // pass across it at Z10: the descent past it rapids down to 5 mm
        // over the floor, where the earlier cuts clear the column, not to 5
        // mm over the stock top, and feeds the rest at the pass's feed.
        let earlier: Vec<([f32; 3], [f32; 3])> = [10., 15.]
            .into_iter()
            .flat_map(|z| [-15., -7.5, 0., 7.5, 15.].map(|y| ([-60., y, z], [60., y, z])))
            .collect();
        let volume = stock(&earlier);
        let (plan, air) = trimmed(plan(&[[-100., 0., 10.], [100., 0., 10.]]), &volume);
        assert_eq!(air.expect("trimmed").stretches, 1);
        continuous(&plan);
        let up = (plan.moves.iter())
            .position(|m| m.motion == Motion::Rapid && m.from[2] == 10. && m.to[2] == 45.)
            .expect("a retract");
        let (down, feed) = (&plan.moves[up + 2], &plan.moves[up + 3]);
        assert_eq!(
            (down.to[2], feed.motion, feed.feed, feed.to[2]),
            (15., Motion::Feed, 600., 10.)
        );
        assert!(down.to[0] > 50. && down.to[0] < 60., "{down:?}");
    }
    #[test]
    fn a_pockets_descent_to_the_level_before_stays_proven_where_that_level_was_relinked() {
        // A slot cleared at Z30 from X-90 to 0, and a Pocket's two levels
        // along it from X-90: Z30, in the slot's air as far as X0, then Z29,
        // whose loop rapids down to 0.5 mm over the level before, where the
        // tool went down at Z30 (LIO-306).
        let volume = stock(&[([-90., 0., 30.], [0., 0., 30.])]);
        let moves = [
            (Motion::Rapid, [-90., 0., 45.], 6000.),
            (Motion::Feed, [-90., 0., 30.], 150.),
            (Motion::Feed, [90., 0., 30.], 600.),
            (Motion::Rapid, [90., 0., 50.], 6000.),
            (Motion::Rapid, [-90., 0., 50.], 6000.),
            (Motion::Rapid, [-90., 0., 30.5], 6000.),
            (Motion::Feed, [-90., 0., 29.], 150.),
            (Motion::Feed, [90., 0., 29.], 600.),
            (Motion::Rapid, [90., 0., 50.], 6000.),
        ];
        let mut original = crate::post::fixture::plan([-90., 0., 50.], &moves);
        (original.stock, original.radius, original.flute_length) = (STOCK, 5., 25.);
        original.strategy = Strategy::Pocket;
        let (plan, air) = trimmed(original, &volume);
        assert!(air.expect("trimmed").stretches >= 1);
        continuous(&plan);
        // Z30's way down at X-90 is gone, and the slot's walls stand inside
        // the column the export checks (the cutter's radius and the blend):
        // Z29's descent there no longer rapids to Z30.5, but stops where the
        // check proves the column clear, over the stock top, and feeds on.
        let rest = crate::rest::RestStock::new(&volume, frame());
        let mut been: Vec<[f32; 3]> = vec![];
        for m in &plan.moves {
            let down = m.motion == Motion::Rapid
                && m.from[..2] == m.to[..2]
                && m.to[2] < m.from[2]
                && m.to[2] < STOCK.max[2];
            if down {
                let r = 5. + 1.;
                let column = Bounds {
                    min: [m.to[0] - r, m.to[1] - r, m.to[2]],
                    max: [m.to[0] + r, m.to[1] + r, STOCK.max[2]],
                };
                let revisited =
                    (been.iter()).any(|p| p[..2] == m.to[..2] && p[2] <= m.to[2] + 1e-3);
                let proven = rest.beside(column)
                    || rest.top(column, m.to[2] - 0.001, STOCK.max[2]) <= m.to[2] + 1e-3;
                assert!(revisited || proven, "unproven rapid down to {:?}", m.to);
            }
            been.extend([m.from, m.to]);
        }
        assert!(
            !(plan.moves.iter()).any(|m| m.motion == Motion::Rapid && m.to == [-90., 0., 30.5]),
            "{:#?}",
            plan.moves
        );
        let fed = (plan.moves.iter())
            .find(|m| m.motion == Motion::Feed && m.to == [-90., 0., 30.5])
            .expect("Z29's entry");
        assert!(fed.from[2] > STOCK.max[2], "{fed:?}");
    }
    #[test]
    fn a_short_gap_is_fed_across() {
        // A 10 mm gap: the retract and the descent take longer.
        let volume = stock(&[([-5., 0., 30.], [5., 0., 30.])]);
        let (_, air) = trimmed(plan(&[[-60., 0., 30.], [60., 0., 30.]]), &volume);
        assert_eq!(air, None);
    }
    #[test]
    fn a_wall_shaved_by_short_moves_is_cut_and_one_too_thin_is_not() {
        // The slot's wall at Y-5 shaved 0.1 mm by a pass of 0.05 mm moves:
        // every one of them cuts.
        let volume = stock(&[([-90., 0., 30.], [90., 0., 30.])]);
        let shave = |y: f32| plan(&fine([-60., y, 30.], [60., y, 30.], 0.05));
        assert_eq!(trimmed(shave(-0.1), &volume).1, None);
        // 0.01 mm, under `DEPTH`: the pass cuts nothing, and is left as it
        // is for the user to see.
        let original = shave(-0.01);
        let (plan, air) = trimmed(original.clone(), &volume);
        assert_eq!(
            air,
            Some(Trimmed {
                nothing: true,
                ..Default::default()
            })
        );
        assert_eq!(plan.moves.len(), original.moves.len());
    }
    #[test]
    fn a_skin_left_on_a_floor_is_cut() {
        // The slot cleared to Z30.1: a pass at Z30 takes the 0.1 mm left.
        let volume = stock(&[([-90., 0., 30.1], [90., 0., 30.1])]);
        assert_eq!(
            trimmed(plan(&[[-60., 0., 30.], [60., 0., 30.]]), &volume).1,
            None
        );
    }
    #[test]
    fn a_pass_made_again_is_kept() {
        // Along the slot at Z30 and back, then again: the second time round
        // cuts nothing but is a finishing pass taken again.
        let volume = stock(&[]);
        let lap = [
            [-60., 0., 30.],
            [60., 0., 30.],
            [60., 8., 30.],
            [-60., 8., 30.],
            [-60., 0., 30.],
        ];
        let twice: Vec<[f32; 3]> = lap.iter().chain(&lap[1..]).copied().collect();
        assert_eq!(trimmed(plan(&twice), &volume).1, None);
    }
    #[test]
    fn a_pass_made_again_over_air_is_air() {
        // The slot along X at Y0 cut from X-60 to 60, and a pass that cuts
        // past it to X100 and goes back along it to X-60, then the same
        // again: the second time along the slot's cleared stretch runs
        // through air as the first time did, and is taken out with it.
        let volume = stock(&[([-60., 0., 30.], [60., 0., 30.])]);
        let lap = [
            [-60., 0., 30.],
            [-30., 0., 30.],
            [0., 0., 30.],
            [30., 0., 30.],
            [100., 0., 30.],
            [30., 0., 30.],
            [0., 0., 30.],
            [-30., 0., 30.],
            [-60., 0., 30.],
        ];
        let twice: Vec<[f32; 3]> = lap.iter().chain(&lap[1..]).copied().collect();
        let (trimmed, air) = trimmed(plan(&twice), &volume);
        assert!(
            air.is_some_and(|a| !a.nothing && a.stretches >= 2),
            "{air:?}"
        );
        continuous(&trimmed);
        // Out past the slot's end both times, where the first pass cut and
        // the second finishes; nothing fed along the slot's middle.
        assert!(feeds_between(&trimmed, 30., 70., 100.) >= 2);
        assert_eq!(
            feeds_between(&trimmed, 30., -25., 25.),
            0,
            "{:#?}",
            trimmed.moves
        );
    }
    #[test]
    fn the_fixture_jobs_facing_and_parallel_leave_the_stock_they_did() {
        // Facing's runs off the stock and Parallel's passes over what the
        // Pocket and the Contour finished are taken out; what each leaves
        // is what its whole path left.
        let (doc, mesh) = crate::post::fixture::job();
        let mut sims = vec![];
        crate::simulation::sequence(&doc, &mesh, Some((0, 4)), |_, oi, r| {
            sims.push((oi, r.unwrap()))
        });
        for (oi, sim) in sims.into_iter().filter(|(oi, _)| [0, 4].contains(oi)) {
            let air = sim.air.expect("air taken out");
            assert!(air.saved > 30. && !air.nothing, "{oi}: {air:?}");
            let setup = &doc.setups[0];
            let op = &setup.operations[oi];
            let tool = doc.tools.iter().find(|t| t.id == op.tool).unwrap();
            let plan = crate::simulation::generate(setup, op, tool, &mesh).unwrap();
            assert!(plan.duration - sim.plan.duration > 30.);
            continuous(&sim.plan);
            let mut whole = crate::simulation::Simulation::new(plan);
            (whole.volume, whole.initial_volume, whole.frame) = (
                sim.initial_volume.clone(),
                sim.initial_volume.clone(),
                sim.frame,
            );
            let mut trimmed = sim.clone();
            whole.seek(1.);
            trimmed.seek(1.);
            let (a, b) = (whole.removed_volume(), trimmed.removed_volume());
            assert!(a > 0. && (a - b).abs() <= 1e-3 * a, "{oi}: {a} vs {b}");
        }
    }
}
