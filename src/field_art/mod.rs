//! Pictures for single fields: workholding (a round bar's too), stock, the
//! lathe and its tools, probing, the post, linking, manual machines and
//! cutting data.
//! Each group draws one moment of its 5 s phase, picked by its number;
//! the drawing toolkit is `illustration`'s.
use crate::help::Diagram;
use iced::widget::canvas::Frame;

mod cnc;
mod lathe;
mod machine;
mod round;
mod turning;
mod workholding;

pub fn draw(d: Diagram, f: &mut Frame, time: f32) -> bool {
    let phase = (time / 5.).fract();
    match d {
        Diagram::Vise(n) => workholding::vise(f, time, phase, n),
        Diagram::Clamp(n) => workholding::clamp(f, time, phase, n),
        Diagram::Stock(n) => workholding::stock(f, time, phase, n),
        Diagram::Chuck(n) => lathe::chuck(f, time, phase, n),
        Diagram::LatheTool(n) => lathe::tool(f, time, phase, n),
        Diagram::TurnOp(n) => turning::turn(f, time, phase, n),
        Diagram::Probe(n) => cnc::probe(f, time, phase, n),
        Diagram::Post(n) => cnc::post(f, time, phase, n),
        Diagram::Linking(n) => cnc::linking(f, time, phase, n),
        Diagram::Manual(n) => machine::manual(f, time, phase, n),
        Diagram::Machine(n) => machine::machine(f, time, phase, n),
        Diagram::Cutting(n) => machine::cutting(f, time, phase, n),
        Diagram::Round(n) => round::round(f, time, phase, n),
        _ => return false,
    }
    true
}
