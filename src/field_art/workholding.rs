//! Field pictures: workholding. Placeholder until drawn.
use crate::illustration::*;
use iced::widget::canvas::Frame;

pub fn vise(f: &mut Frame, _time: f32, _phase: f32, n: u8) {
    heading(f, &format!("TODO vise {n}"));
}

pub fn clamp(f: &mut Frame, _time: f32, _phase: f32, n: u8) {
    heading(f, &format!("TODO clamp {n}"));
}

pub fn stock(f: &mut Frame, _time: f32, _phase: f32, n: u8) {
    heading(f, &format!("TODO stock {n}"));
}
