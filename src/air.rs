//! Air moves: the stretches of a milling operation's path where the cutter
//! takes nothing from the stock it starts from (the stock box or bar less
//! the earlier operations' cuts and its own moves before). Where a retract
//! to the plan's clearance, a rapid across and a descent are quicker than
//! feeding through such a stretch, they replace it (`trim`): the same link
//! the strategies make between their own cuts.
//!
//! A stretch cuts where material reaches `DEPTH` into the cutter: the
//! cutter's surface, drawn in by that much, is sampled round and up its
//! flutes every `step` along the path and tested exactly against the cuts
//! (`volume::swept`), as the stock left was `lag` back along the path, so
//! that a thin wall the cutter shaves (`DEPTH` or more) is found however
//! short the moves it is shaved by. A stretch found cutting is kept with
//! `pad` of path either side. A move that repeats one the operation already
//! made, a finishing pass taken again (Pocket's and Contour's finishing
//! passes), is kept whatever it cuts.
use crate::holding::HoldBox;
use crate::mesh::Bounds;
use crate::model::{Operation, Setup, Strategy, num};
use crate::simulation::{CutterShape, Motion, Move, Plan};
use crate::stock_surface::Cut;
use crate::volume::{Frame, History, Volume, swept};

/// How far into the cutter material must reach for the cutter to cut it,
/// mm: a thinner sliver, the chords of two passes over one curve, is air.
pub const DEPTH: f32 = 0.02;
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
/// How much farther than the setup's gap a link keeps from the vise jaws
/// or clamps, mm: the controller's blend at its corners.
const HOLDING: f32 = 1.;

/// What `trim` changed in a plan.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Trimmed {
    /// The stretches replaced.
    pub stretches: usize,
    /// The feed through air they held, mm.
    pub air: f32,
    /// The time saved, s.
    pub saved: f32,
    /// Whether no move of the plan cut anything: only a retract is left.
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
    let gap = crate::holding::gap(setup).unwrap_or_default().max(0.) + HOLDING;
    Ok(relink(plan, op, volume, frame, &cutting, (holding, gap)))
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
    let mut stretches: Vec<[f32; 2]> = found
        .into_iter()
        .flatten()
        .map(|s| [s - lag - pad, s + pad])
        .collect();
    // A move made again, a finishing pass, is kept.
    let mut seen = std::collections::HashSet::new();
    let key = |p: [f32; 3]| p.map(|v| (v * 1e4).round() as i64);
    for f in feeds {
        let m = &plan.moves[f.index];
        if !seen.insert((key(m.from), key(m.to))) {
            stretches.push([f.at, f.at + f.length]);
        }
    }
    stretches.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut merged: Vec<[f32; 2]> = vec![];
    for [a, b] in stretches {
        let (a, b) = (a.max(0.), b.min(total));
        match merged.last_mut() {
            Some(last) if a <= last[1] => last[1] = last[1].max(b),
            _ => merged.push([a, b]),
        }
    }
    Ok(Some(merged))
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
) -> (Plan, Option<Trimmed>) {
    // The plan's clearance: where its rapids cross.
    let clearance = (plan.moves.iter())
        .filter(|m| m.motion == Motion::Rapid)
        .map(|m| m.from[2].max(m.to[2]))
        .fold(f32::NEG_INFINITY, f32::max);
    if !clearance.is_finite() {
        return (plan, None);
    }
    // A descent rapids down to the retract margin over the highest
    // material left (as the export's check takes it), then feeds.
    let margin = num(&op.p, "retract").filter(|v| *v > 0.).unwrap_or(2.);
    let high = if volume.cuts.is_empty() {
        plan.stock.max[2]
    } else {
        let unbounded = Bounds {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        };
        crate::rest::RestStock::new(volume, frame)
            .left(unbounded)
            .max[2]
    };
    let approach = (high.min(plan.stock.max[2]) + margin).min(clearance);
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
    let first = pieces.iter().position(|p| p.cuts);
    let mut out: Vec<Out> = vec![];
    let keep = |p: &Piece, out: &mut Vec<_>| {
        let m = &moves[p.index];
        out.push((Some(*p), from(p), to(p), m.motion, m.feed));
    };
    let mut trimmed = Trimmed::default();
    let mut i = 0;
    while i < pieces.len() {
        if pieces[i].cuts {
            keep(&pieces[i], &mut out);
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
        // The link: up to clearance, across, down at rapid to the approach,
        // then fed down at the feed of the move it leads into: the column
        // over a point the cutter reached through air is clear (as
        // Adaptive's links take it), and the point is `pad` short of what
        // it cuts.
        let mut link: Vec<([f32; 3], Motion, f32)> = vec![];
        let mut at = start;
        let mut go = |p: [f32; 3], motion: Motion, rate: f32, at: &mut [f32; 3]| {
            if length(*at, p) > 1e-5 {
                link.push((p, motion, rate));
                *at = p;
            }
        };
        go(
            [at[0], at[1], at[2].max(clearance)],
            Motion::Rapid,
            RAPID,
            &mut at,
        );
        if j < pieces.len() {
            let feed = moves[pieces[j].index].feed;
            go([end[0], end[1], at[2]], Motion::Rapid, RAPID, &mut at);
            if end[2] >= approach {
                go(end, Motion::Rapid, RAPID, &mut at);
            } else {
                go([end[0], end[1], approach], Motion::Rapid, RAPID, &mut at);
                go(end, Motion::Feed, feed, &mut at);
            }
        }
        let mut was = start;
        let link_seconds: f32 = (link.iter())
            .map(|&(p, _, rate)| {
                let l = length(was, p);
                was = p;
                l / rate * 60.
            })
            .sum();
        let stretch_seconds: f32 = stretch.iter().map(seconds).sum();
        let whole = first.is_none();
        let clear = || {
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
        let replace = air > 0.
            && !dwells
            && (whole
                || (stretch_seconds - link_seconds > SAVING
                    && link_seconds < SHARE * stretch_seconds))
            && clear();
        if replace {
            trimmed.stretches += 1;
            trimmed.air += air;
            let mut was = start;
            for (p, motion, rate) in link {
                out.push((None, was, p, motion, rate));
                was = p;
            }
        } else {
            for p in stretch {
                keep(p, &mut out);
            }
        }
        i = j;
    }
    if trimmed.stretches == 0 {
        return (plan, None);
    }
    trimmed.nothing = first.is_none();
    // Whole moves again where a move's pieces were all kept, and timed.
    let mut moves_out: Vec<Move> = vec![];
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
    if moves_out.is_empty() {
        // Nothing cut and the tool already clear: it stays where it is.
        let at = plan.moves[0].from;
        moves_out.push(Move {
            from: at,
            to: at,
            motion: Motion::Rapid,
            cutting_pass: false,
            feed: RAPID,
            start: 0.,
            end: 0.,
            a: None,
        });
    }
    let mut plan = plan;
    trimmed.saved = plan.duration - clock;
    plan.moves = moves_out;
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
        // Up to clearance, across it, down to 5 mm over the stock top and
        // fed down from there at the pass's feed.
        let up = (plan.moves.iter())
            .position(|m| m.motion == Motion::Rapid && m.to[2] == 50. && m.from[2] == 30.)
            .expect("a retract");
        let [across, down, feed] = [up + 1, up + 2, up + 3].map(|i| &plan.moves[i]);
        assert_eq!((across.motion, across.to[2]), (Motion::Rapid, 50.));
        assert_eq!((down.motion, down.to[2]), (Motion::Rapid, 45.));
        assert_eq!(
            (feed.motion, feed.feed, feed.to[2]),
            (Motion::Feed, 600., 30.)
        );
        assert!(plan.moves[up].from[0] > -66. && feed.to[0] < 66.);
        assert!((air.saved - (original.duration - plan.duration)).abs() < 1e-3);
        assert!(air.saved > 3., "{air:?}");
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
        // 0.01 mm, under `DEPTH`: the pass is air, so it is all taken out
        // but the retract.
        let (plan, air) = trimmed(shave(-0.01), &volume);
        let air = air.expect("trimmed");
        assert!(air.nothing, "{air:?}");
        assert!(plan.moves.iter().all(|m| m.motion == Motion::Rapid));
        continuous(&plan);
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
