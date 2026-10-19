//! Field pictures: machine. Placeholder until drawn.
use crate::illustration::*;
use iced::widget::canvas::Frame;

pub fn manual(f: &mut Frame, _time: f32, _phase: f32, n: u8) {
    heading(f, &format!("TODO manual {n}"));
}

pub fn machine(f: &mut Frame, _time: f32, _phase: f32, n: u8) {
    heading(f, &format!("TODO machine {n}"));
}

pub fn cutting(f: &mut Frame, _time: f32, _phase: f32, n: u8) {
    heading(f, &format!("TODO cutting {n}"));
}
