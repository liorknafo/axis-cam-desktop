//! A panic on the window's thread cannot unwind through the system's event
//! loop, so it aborts the whole app (and drops the machine's connection
//! mid-run). What the app does with a message, the page it lays out and
//! each canvas it paints run under `catch`, which turns a panic into a
//! fault the app shows while it keeps running.

use iced::widget::canvas::{self, Action, Event, Frame, Geometry, Program};
use iced::{Color, Point, Rectangle, Renderer, Theme, mouse};
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};

thread_local! {
    /// The last panic on this thread: its message and where it was raised.
    static LAST: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Keeps each panic's message and place for `catch`, then reports it as
/// before (on standard error).
pub fn install() {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let what = (info.payload().downcast_ref::<&str>().map(|s| s.to_string()))
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "a panic".into());
        let at = info.location().map_or(String::new(), |l| format!(" ({l})"));
        LAST.with(|last| *last.borrow_mut() = Some(format!("{what}{at}")));
        before(info);
    }));
}

/// `f`'s result, or what went wrong where it panicked.
pub fn catch<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    LAST.with(|last| last.borrow_mut().take());
    catch_unwind(AssertUnwindSafe(f)).map_err(|payload| {
        LAST.with(|last| last.borrow_mut().take())
            .unwrap_or_else(|| {
                (payload.downcast_ref::<&str>().map(|s| s.to_string()))
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "a panic".into())
            })
    })
}

/// `catch` in the app; under test a panic goes on and fails the test, as
/// it did before there was `catch`.
pub fn guard<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    if cfg!(test) { Ok(f()) } else { catch(f) }
}

/// A canvas of `program` that paints what went wrong in place of a
/// drawing that panics, and ignores an event whose handling does.
pub fn canvas<M, P: Program<M>>(program: P) -> canvas::Canvas<Guarded<P>, M> {
    canvas::Canvas::new(Guarded(program))
}

/// `canvas`'s program.
pub struct Guarded<P>(P);

impl<M, P: Program<M>> Program<M> for Guarded<P> {
    type State = P::State;
    fn update(
        &self,
        state: &mut P::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<M>> {
        guard(|| self.0.update(state, event, bounds, cursor)).unwrap_or(None)
    }
    fn draw(
        &self,
        state: &P::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        guard(|| self.0.draw(state, renderer, theme, bounds, cursor)).unwrap_or_else(|fault| {
            let mut frame = Frame::new(renderer, bounds.size());
            frame.fill_text(canvas::Text {
                content: format!("This drawing failed: {fault}"),
                position: Point::new(8., 8.),
                color: Color::from_rgb8(194, 75, 73),
                size: 12.into(),
                ..Default::default()
            });
            vec![frame.into_geometry()]
        })
    }
    fn mouse_interaction(
        &self,
        state: &P::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        guard(|| self.0.mouse_interaction(state, bounds, cursor)).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_is_caught_with_its_message_and_place() {
        install();
        let fault = catch(|| {
            let empty: &[f32] = &[];
            empty[1]
        })
        .unwrap_err();
        assert!(fault.contains("index out of bounds"), "{fault}");
        assert!(fault.contains("src/fault.rs"), "{fault}");
        assert_eq!(catch(|| 2 + 2), Ok(4));
    }
}
