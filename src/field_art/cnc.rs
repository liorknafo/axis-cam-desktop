//! Field pictures: cnc. Placeholder until drawn.
use crate::illustration::*;
use iced::widget::canvas::Frame;

pub fn probe(f: &mut Frame, _time: f32, _phase: f32, n: u8) {
    heading(f, &format!("TODO probe {n}"));
}

pub fn post(f: &mut Frame, _time: f32, _phase: f32, n: u8) {
    heading(f, &format!("TODO post {n}"));
}

pub fn linking(f: &mut Frame, _time: f32, _phase: f32, n: u8) {
    heading(f, &format!("TODO linking {n}"));
}
