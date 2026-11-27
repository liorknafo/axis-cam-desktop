//! How far a job generated off the window's thread has got, and its Cancel.
//! One `Progress` is shared by the window and the thread doing the work; the
//! generators report into it through `part`, `expect` and `step` without it
//! being passed down to them (it is the thread's, set by `Progress::run`).
//! On a thread with none, `step` costs nothing and never cancels.
use std::cell::RefCell;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

/// What a generation that was cancelled ends with.
pub const CANCELLED: &str = "Generation cancelled.";

#[derive(Default)]
pub struct Progress {
    /// How much is done, in millionths.
    done: AtomicU32,
    cancelled: AtomicBool,
    /// What is being done now: the operation being generated.
    label: Mutex<String>,
}

/// The current thread's progress: what it reports into, the share of it the
/// part under way stands for, about how many steps that part takes (0: not
/// known), and the steps since it last looked.
struct Current {
    progress: Arc<Progress>,
    /// The share of the whole the part under way stands for.
    part: (f32, f32),
    /// The share the stage of it under way stands for (`stage`).
    from: f32,
    to: f32,
    total: usize,
    calls: usize,
}
thread_local! {
    static CURRENT: RefCell<Option<Current>> = const { RefCell::new(None) };
}

impl Progress {
    /// How much is done, 0 to 1.
    pub fn fraction(&self) -> f32 {
        self.done.load(Ordering::Relaxed) as f32 / 1e6
    }
    /// What is being done now.
    pub fn label(&self) -> String {
        self.label.lock().map(|l| l.clone()).unwrap_or_default()
    }
    /// Asks the work to stop: its next `step` fails with `CANCELLED`.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
    /// Runs `work` on this thread, reporting into this progress.
    pub fn run<T>(self: &Arc<Self>, work: impl FnOnce() -> T) -> T {
        let now = Current {
            progress: self.clone(),
            part: (0., 1.),
            from: 0.,
            to: 1.,
            total: 0,
            calls: 0,
        };
        let before = CURRENT.with(|c| c.replace(Some(now)));
        let result = work();
        CURRENT.with(|c| *c.borrow_mut() = before);
        self.done.store(1_000_000, Ordering::Relaxed);
        result
    }
    fn set(&self, fraction: f32) {
        let value = (fraction.clamp(0., 1.) * 1e6) as u32;
        self.done.fetch_max(value, Ordering::Relaxed);
    }
}

/// The work from here on is part `index` (from 0) of `count` equal parts of
/// the whole, named `label` (only worked out when someone is watching).
pub fn part(index: usize, count: usize, label: impl FnOnce() -> String) {
    CURRENT.with(|c| {
        if let Some(current) = c.borrow_mut().as_mut() {
            let count = count.max(1) as f32;
            current.from = index as f32 / count;
            current.to = (index + 1) as f32 / count;
            current.part = (current.from, current.to);
            current.total = 0;
            current.progress.set(current.from);
            if let Ok(mut l) = current.progress.label.lock() {
                *l = label();
            }
        }
    });
}

/// The work from here on is stage `index` (from 0) of `count` equal stages
/// of the part under way (a finishing grid, then its rows).
pub fn stage(index: usize, count: usize) {
    CURRENT.with(|c| {
        if let Some(current) = c.borrow_mut().as_mut() {
            let (from, to) = current.part;
            let count = count.max(1) as f32;
            current.from = from + (to - from) * index as f32 / count;
            current.to = from + (to - from) * (index + 1) as f32 / count;
            current.total = 0;
            current.progress.set(current.from);
        }
    });
}

/// The part under way takes about `total` steps.
pub fn expect(total: usize) {
    CURRENT.with(|c| {
        if let Some(current) = c.borrow_mut().as_mut() {
            current.total = total;
        }
    });
}

/// The part under way has done `done` of its steps, or `CANCELLED` once the
/// window asked it to stop. Cheap enough for every move: it looks only every
/// 256th call. A loop of few long steps (a layer, a hole) uses `report`.
pub fn step(done: usize) -> Result<(), String> {
    look(done, 256)
}

/// `CANCELLED` once the window asked the work to stop, for a loop whose
/// steps are not what the part under way counts (a builder's moves while a
/// Waterline counts its layers). It looks every 256th call.
pub fn check() -> Result<(), String> {
    if stopped() {
        return Err(CANCELLED.into());
    }
    Ok(())
}

/// `step` for a loop of few long steps: it looks every call.
pub fn report(done: usize) -> Result<(), String> {
    look(done, 1)
}

/// Whether the generation this thread works for was cancelled, for work
/// that cannot fail (replaying a simulation): what it makes is dropped, so
/// it may stop where it is. It looks every 256th call; never off a progress.
pub fn stopped() -> bool {
    CURRENT.with(|c| {
        c.borrow_mut().as_mut().is_some_and(|current| {
            current.calls += 1;
            current.calls % 256 == 0 && current.progress.cancelled()
        })
    })
}

/// The progress this thread reports into, for the threads it starts to
/// look at its Cancel (`Progress::cancelled`); None off a progress.
pub fn current() -> Option<Arc<Progress>> {
    CURRENT.with(|c| c.borrow().as_ref().map(|c| c.progress.clone()))
}

fn look(done: usize, every: usize) -> Result<(), String> {
    CURRENT.with(|c| {
        let mut current = c.borrow_mut();
        let Some(current) = current.as_mut() else {
            return Ok(());
        };
        current.calls += 1;
        if current.calls % every != 0 {
            return Ok(());
        }
        if current.progress.cancelled() {
            return Err(CANCELLED.into());
        }
        if current.total > 0 {
            let share = (done as f32 / current.total as f32).min(1.);
            current
                .progress
                .set(current.from + (current.to - current.from) * share);
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn work_reports_its_parts_and_stops_when_cancelled() {
        let progress = Arc::new(Progress::default());
        // Off a progress, steps do nothing.
        assert_eq!(step(0), Ok(()));
        let seen = progress.run(|| {
            part(1, 4, || "Setup 1 / Pocket".into());
            let at_start = progress.fraction();
            expect(1024);
            // Every 256th call looks: half the part is done at the 512th.
            for done in 1..=512 {
                step(done).unwrap();
            }
            let halfway = progress.fraction();
            progress.cancel();
            let mut stopped = None;
            for done in 513..=1024 {
                if let Err(e) = step(done) {
                    stopped = Some((done, e));
                    break;
                }
            }
            (at_start, halfway, stopped)
        });
        assert!((seen.0 - 0.25).abs() < 1e-6, "{}", seen.0);
        assert!((seen.1 - 0.375).abs() < 1e-4, "{}", seen.1);
        assert_eq!(seen.2, Some((768, CANCELLED.into())));
        // A loop of long steps looks at each.
        let long = Arc::new(Progress::default());
        let layers = long.run(|| {
            part(0, 1, String::new);
            expect(4);
            report(1).unwrap();
            let one = long.fraction();
            long.cancel();
            (one, report(2))
        });
        assert_eq!(layers, (0.25, Err(CANCELLED.into())));
        // Stages split a part: the second half of part 1 of 2 starts at 75 %.
        let staged = Arc::new(Progress::default());
        let at = staged.run(|| {
            part(1, 2, String::new);
            stage(1, 2);
            let start = staged.fraction();
            expect(2);
            report(1).unwrap();
            (start, staged.fraction())
        });
        assert_eq!(at, (0.75, 0.875));
        // Work that cannot fail stops once cancelled, and never off one.
        let stopping = Arc::new(Progress::default());
        stopping.cancel();
        let calls = stopping.run(|| (1..=1000).find(|_| stopped()));
        assert_eq!(calls, Some(256));
        assert!(!(1..=1000).any(|_| stopped()));
        let checking = Arc::new(Progress::default());
        let checked = checking.run(|| {
            part(0, 1, String::new);
            expect(10);
            report(5).unwrap();
            checking.cancel();
            // It moves nothing: only the strategy's own steps do.
            let first = (1..=1000).position(|_| check().is_err());
            (checking.fraction(), first)
        });
        assert_eq!(checked.0, 0.5);
        assert!(checked.1.is_some_and(|calls| calls < 256), "{checked:?}");
        assert_eq!(progress.label(), "Setup 1 / Pocket");
        // Done once the work returns; the thread watches nothing after.
        assert_eq!(progress.fraction(), 1.);
        assert_eq!(step(0), Ok(()));
    }
}
