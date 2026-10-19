//! Field pictures: lathe. Placeholder until drawn.
use crate::illustration::*;
use iced::widget::canvas::Frame;

pub fn chuck(f: &mut Frame, _time: f32, _phase: f32, n: u8) {
    heading(f, &format!("TODO chuck {n}"));
}

pub fn tool(f: &mut Frame, _time: f32, _phase: f32, n: u8) {
    heading(f, &format!("TODO tool {n}"));
}
