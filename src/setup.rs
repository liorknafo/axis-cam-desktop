//! Controller configuration on the Control page: the UC100 profile form, the inputs
//! and the output test, the driver's audit, and steps-per-mm calibration.
use crate::control::{Action, Connection};
use crate::ui::{act, bold, btn, caution, machine_panel, muted, note};
use crate::{App, Message, Target, help};
use iced::widget::{
    Column, checkbox, column, container, pick_list, row, scrollable, text, text_input,
};
use iced::{Alignment, Color, Element, Fill, Font};
use uc100::config::MachineConfig;

// The profile editor itself is core (LIO-729); this module is its form.
use crate::profile_draft::{AXES, PINOUT, unit};
pub use crate::profile_draft::{Draft, Edit, a_direction, steps_per_degree};

/// What the Setup view asks.
#[derive(Debug, Clone)]
pub enum Msg {
    Edit(Edit),
    Save,
    /// Throw the changes away and read the file again.
    Revert,
    /// Put the calibrated steps per mm in the form.
    UseCalibration,
}

const OUTPUT_PINS: [u8; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 14, 16, 17];
const INPUT_PINS: [u8; 5] = uc100::diagnostics::INPUT_PINS;

/// A pin in a pick list.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PinChoice(Option<u8>);

impl std::fmt::Display for PinChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(n) => write!(f, "Pin {n}"),
            None => f.write_str("None"),
        }
    }
}

fn edit(e: Edit) -> Message {
    Message::Setup(Msg::Edit(e))
}

fn label<'a>(s: &str) -> iced::widget::Text<'a> {
    text(s.to_string()).size(12).width(200)
}

/// A number field; the placeholder says what an empty field means.
fn number<'a>(d: &Draft, path: String, name: &str, empty: &str) -> Element<'a, Message> {
    let value = d.number(&path);
    row![
        label(name),
        text_input(empty, &value)
            .on_input(move |v| edit(Edit::Number(path.clone(), v)))
            .width(110)
            .padding([5, 8])
            .size(12)
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

fn pin<'a>(d: &Draft, path: String, name: &str, pins: &[u8]) -> Element<'a, Message> {
    let now = d.pin(&path);
    let options: Vec<PinChoice> = std::iter::once(None)
        .chain(pins.iter().copied().map(Some))
        .map(PinChoice)
        .collect();
    let to = path.clone();
    let mut r = row![
        label(name),
        pick_list(
            options,
            Some(PinChoice(now.map(|p| p.0))),
            move |c: PinChoice| { edit(Edit::Pin(to.clone(), c.0)) }
        )
        .text_size(12)
        .style(crate::ui::choice_style)
        .padding([5, 8])
        .width(110)
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if let Some((_, low)) = now {
        r = r.push(
            checkbox(low)
                .label("Active LOW")
                .on_toggle(move |b| edit(Edit::ActiveLow(path.clone(), b)))
                .text_size(12),
        );
    }
    r.into()
}

/// A checkbox; `default` is the driver's value when the file leaves it out.
fn flag<'a>(d: &Draft, path: String, name: &str, default: bool) -> Element<'a, Message> {
    checkbox(d.flag(&path).unwrap_or(default))
        .label(name.to_string())
        .on_toggle(move |b| edit(Edit::Flag(path.clone(), b)))
        .text_size(12)
        .into()
}

/// The Setup view of the Control page, right of the machine panel.
pub fn view(app: &App) -> Element<'_, Message> {
    let body: Element<_> = match &app.profile_draft {
        None => muted("Add a UC100 machine in the Machine library.").into(),
        Some((_, Err(e))) => column![
            caution("The profile cannot be edited", vec![e.clone()]),
            btn("Read it again", Message::Setup(Msg::Revert), false)
        ]
        .spacing(10)
        .into(),
        Some((_, Ok(d))) => form(app, d),
    };
    scrollable(container(body).padding(4))
        .height(Fill)
        .width(Fill)
        .into()
}

fn form<'a>(app: &'a App, d: &'a Draft) -> Element<'a, Message> {
    let here = app
        .control
        .as_ref()
        .filter(|c| c.machine_id == d.machine_id);
    // Which way the machine's X and Y point, as it is now (LIO-457): the
    // connected machine's, else the library's.
    let orient = here.map(|c| c.orient).unwrap_or_else(|| {
        app.doc
            .machines
            .iter()
            .find(|m| m.id == d.machine_id)
            .map(crate::orient::Orient::of)
            .unwrap_or_default()
    });
    let stopped = here.is_none_or(|c| c.stopped());
    let save = (d.blocker().is_none() && stopped).then_some(Message::Setup(Msg::Save));
    let mut head = column![
        row![
            bold("UC100 profile", 18),
            act(
                if here.is_some() {
                    "Save and reconnect"
                } else {
                    "Save"
                },
                save,
                false
            ),
            act(
                "Revert",
                d.dirty().then_some(Message::Setup(Msg::Revert)),
                false
            )
        ]
        .spacing(10)
        .align_y(Alignment::Center),
        muted(d.path.display().to_string()),
    ]
    .spacing(8);
    if let Some(path) = d.bad_numbers().first() {
        head = head.push(caution(
            "Not a number",
            vec![format!("{path} needs a number.")],
        ));
    }
    let verdict: Element<_> = match &d.checked {
        Ok(cfg) => match cfg.commissioning_blocker() {
            Some(why) => caution(
                "Reset is refused until commissioning is verified",
                vec![why],
            ),
            None => muted("The driver accepts this profile.")
                .color(Color::from_rgb8(39, 130, 93))
                .into(),
        },
        Err(e) => caution("The driver refuses this profile", vec![e.clone()]),
    };
    head = head.push(verdict);
    if let Some(n) = &d.note {
        head = head.push(note(n.clone()));
    }
    if here.is_some() {
        head = head.push(muted(if stopped {
            "Saving reconnects the machine: it starts in E-stop, and its machine position must be set again, since a new steps per mm moves it."
        } else {
            "Stop the machine to save the profile."
        }));
    }
    let mut main = column![head].spacing(12);
    match here {
        Some(c) => {
            main = main
                .push(row![inputs(c), outputs(c)].spacing(12))
                .push(calibration(c))
        }
        None => {
            main = main.push(muted(
                "Connect to watch the inputs, test the outputs and calibrate.",
            ))
        }
    }
    main = main.push(machine_panel(
        "Commissioning",
        column![
            flag(d, "commissioning.required".into(), "Require the checks below before Reset", false),
            flag(d, PINOUT.into(), "Step and direction pins and polarity checked (A included once it is added, after its direction)", false),
            flag(d, "commissioning.estop_verified".into(), "E-stop input and polarity checked", false),
            flag(d, "commissioning.drive_enable_verified".into(), "Drive enable and charge pump need checked", false),
            flag(d, "commissioning.steps_per_unit_verified".into(), "Steps per mm checked (steps per degree for A)", false),
            muted("These record what was checked at the machine. AXIS CAM does not check the wiring itself."),
        ],
    ));
    for l in AXES.iter().filter(|l| d.has(&format!("axes.{l}"))) {
        let p = |k: &str| format!("axes.{l}.{k}");
        let rotary = *l == "a";
        let u = unit(l);
        let mut numbers = column![
            number(
                d,
                p("steps_per_unit"),
                &format!("Steps per {}", if rotary { "°" } else { "mm" }),
                ""
            ),
            number(
                d,
                p("max_velocity"),
                &format!("Maximum speed · {u}/{}", if rotary { "s" } else { "min" }),
                ""
            ),
            number(d, p("acceleration"), &format!("Acceleration · {u}/s²"), ""),
            number(
                d,
                p("soft_min"),
                &format!("Soft limit, low end · {u}"),
                "none"
            ),
            number(
                d,
                p("soft_max"),
                &format!("Soft limit, high end · {u}"),
                "none"
            ),
        ]
        .spacing(6);
        let mut pins = column![
            pin(d, p("step"), "Step", &OUTPUT_PINS),
            pin(d, p("dir"), "Direction", &OUTPUT_PINS),
        ]
        .spacing(6);
        // The rotary axis has no switches: it is declared with Set machine.
        if !rotary {
            numbers = numbers
                .push(number(
                    d,
                    p("home_speed_percent"),
                    "Homing speed · % of maximum",
                    "20",
                ))
                .push(number(
                    d,
                    p("home_position"),
                    "Machine position at home · mm",
                    "0",
                ));
            pins = pins
                .push(pin(d, p("limit_neg"), "Limit switch, low end", &INPUT_PINS))
                .push(pin(
                    d,
                    p("limit_pos"),
                    "Limit switch, high end",
                    &INPUT_PINS,
                ))
                .push(pin(d, p("home"), "Home switch", &INPUT_PINS));
        }
        pins = pins.push(flag(d, p("reversed"), "Reverse the direction", false));
        if !rotary {
            pins = pins.push(flag(d, p("home_negative"), "Home toward the low end", true));
        }
        let mut body = column![row![numbers, pins].spacing(24)].spacing(10);
        if rotary {
            body = body
                .push(
                    checkbox(d.a_direction)
                        .label(a_direction(orient))
                        .on_toggle(|b| edit(Edit::ADirection(b)))
                        .text_size(12),
                )
                .push(muted(format!(
                    "Rotary axis, in degrees. Steps per ° = 200 × microsteps × gear ratio ÷ 360: {} for 16 microsteps and a 4:1 unit, {} for 6:1. Calibrate it below. A has no switches: declare its position with Set machine. With soft limits no program starts until A is declared.",
                    steps_text(steps_per_degree(200., 16., 4.)),
                    steps_text(steps_per_degree(200., 16., 6.))
                )))
                .push(act(
                    "Remove the A axis",
                    Some(edit(Edit::RemoveAxis("a".into()))),
                    false,
                ));
        }
        main = main.push(machine_panel(&format!("{} axis", l.to_uppercase()), body));
    }
    if !d.has("axes.a") {
        main = main.push(machine_panel(
            "Add an axis",
            column![
                act(
                    "Add the A axis (rotary, pins 8 and 9)",
                    Some(edit(Edit::AddAxis("a".into()))),
                    false
                ),
                muted("A fourth, rotary axis in degrees on the UC100's free step and direction pins. It is added unverified: Reset stays refused until its pins are checked and its steps per degree are calibrated. Output pins already used by another role are refused."),
            ],
        ));
    }
    main = main.push(machine_panel(
        "Inputs",
        column![
            pin(d, "inputs.estop".into(), "E-stop", &INPUT_PINS),
            pin(d, "inputs.probe".into(), "Probe", &INPUT_PINS),
            pin(d, "inputs.index".into(), "Spindle index pulse", &INPUT_PINS),
        ],
    ));
    main = main.push(machine_panel(
        "Spindle, coolant and drives",
        column![
            row![
                column![
                    pin(d, "outputs.spindle_pwm".into(), "Spindle speed (PWM)", &OUTPUT_PINS),
                    pin(d, "outputs.spindle_dir".into(), "Spindle direction (PWM)", &OUTPUT_PINS),
                    pin(d, "outputs.spindle_cw".into(), "Spindle CW relay (M3)", &OUTPUT_PINS),
                    pin(d, "outputs.spindle_ccw".into(), "Spindle CCW relay (M4)", &OUTPUT_PINS),
                    pin(d, "outputs.flood".into(), "Flood (M8)", &OUTPUT_PINS),
                    pin(d, "outputs.mist".into(), "Mist (M7)", &OUTPUT_PINS),
                    pin(d, "outputs.enable".into(), "Drive enable", &OUTPUT_PINS),
                    pin(d, "outputs.charge_pump".into(), "Charge pump", &OUTPUT_PINS),
                ]
                .spacing(6),
                column![
                    number(d, "outputs.spindle_max_rpm".into(), "Speed at 100 % PWM · rpm", "24000"),
                    number(d, "outputs.spindle_pwm_hz".into(), "PWM frequency · Hz", "5000"),
                    number(d, "outputs.spindle_spinup_s".into(), "Wait after starting · s", "0"),
                ]
                .spacing(6),
            ]
            .spacing(24),
            muted("No pin means the machine has no such output: the Control page then does not offer it."),
        ],
    ));
    let rates = vec!["25kHz".to_string(), "50kHz".into(), "100kHz".into()];
    let rate = Some(d.text("controller.step_rate")).filter(|r| !r.is_empty());
    let order = d.text("homing.order");
    main = main.push(machine_panel(
        "Homing and controller",
        column![
            row![
                label("Homing order (axis letters)"),
                text_input("ZXY", &order)
                    .on_input(|v| edit(Edit::Text("homing.order".into(), v)))
                    .width(110)
                    .padding([5, 8])
                    .size(12)
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            flag(
                d,
                "homing.override_limits".into(),
                "Ignore the limit switches while homing",
                true
            ),
            row![
                label("Step rate"),
                pick_list(rates, rate.or(Some("100kHz".into())), |r| edit(Edit::Text(
                    "controller.step_rate".into(),
                    r
                )))
                .text_size(12)
                .style(crate::ui::choice_style)
                .padding([5, 8])
                .width(110)
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            muted("Motion tuning and the tool table stay edits of the file."),
        ],
    ));
    if let Ok(cfg) = &d.checked {
        main = main.push(machine_panel(
            "Audit",
            column![
                text(uc100::diagnostics::setup_report(cfg))
                    .size(12)
                    .font(Font::MONOSPACE),
                muted("The driver's report on the profile as edited above."),
            ],
        ));
    }
    main.into()
}

/// A calibrated steps per mm as written in the profile.
pub fn steps_text(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').into()
}

/// Steps per mm (per degree for A) from a commanded move and the travel
/// measured at the machine.
fn calibration(c: &Connection) -> Element<'_, Message> {
    let cal = &c.calibration;
    let axis = cal.axis;
    let letter = AXES.get(axis).copied().unwrap_or("x");
    let (u, per) = (unit(letter), if letter == "a" { "°" } else { "mm" });
    let mut axes = row![label("Axis")].spacing(6).align_y(Alignment::Center);
    for (i, l) in AXES.iter().enumerate().filter(|(i, _)| c.axis(*i)) {
        axes = axes.push(act(
            l.to_uppercase(),
            Some(Message::Machine(Action::CalAxis(i))),
            i == axis,
        ));
    }
    let field = |name: &str, key: &str, value: &str, f: fn(String) -> Action| {
        row![
            label(name),
            help::field(
                text_input(u, value)
                    .on_input(move |v| Message::Machine(f(v)))
                    .width(110)
                    .padding([5, 8])
                    .size(12),
                key,
                name,
                Target::Machine
            )
        ]
        .spacing(8)
        .align_y(Alignment::Center)
    };
    // A moves only once it has a machine position (`Connection::jog_blocked`).
    let blocked = c.jog_blocked(axis);
    let go = (c.idle() && c.cal_distance().is_some() && blocked.is_none())
        .then_some(Message::Machine(Action::CalMove));
    let spu = c.config.axes.get(axis).map_or(1., |a| a.steps_per_unit);
    let mut travel = field(
        &format!("Commanded travel · {u}"),
        "cal_distance",
        &cal.distance,
        Action::CalDistance,
    );
    if letter == "a" {
        for turn in ["90", "360"] {
            travel = travel.push(act(
                format!("{turn}°"),
                Some(Message::Machine(Action::CalDistance(turn.into()))),
                cal.distance.trim() == turn,
            ));
        }
    }
    let mut body = column![axes, travel.push(act("Move", go, false))];
    if let Some(why) = blocked {
        body = body.push(muted(why));
    }
    if let Some(pulses) = c.cal_pulses() {
        body = body.push(muted(format!(
            "{pulses} step pulses since the move began: {:.3} {u} at the profile's {} steps per {per}.",
            pulses as f64 / spu,
            steps_text(spu)
        )));
    }
    body = body.push(field(
        &format!(
            "Measured {} · {u}",
            if letter == "a" { "turn" } else { "travel" }
        ),
        "cal_measured",
        &cal.measured,
        Action::CalMeasured,
    ));
    match c.calibrated() {
        Some(Ok(v)) if c.idle() => {
            body = body.push(
                row![
                    bold(format!("{} steps per {per}", steps_text(v)), 13),
                    act(
                        "Use in profile",
                        Some(Message::Setup(Msg::UseCalibration)),
                        false
                    )
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            )
        }
        Some(Ok(_)) => body = body.push(muted("Wait for the move to end.")),
        Some(Err(e)) => body = body.push(muted(e)),
        None => {}
    }
    let how = if letter == "a" {
        format!(
            "Turn A by 90° or 360°, then type the angle it really turned: a mark on the chuck against a fixed pointer, or an indicator on a flat of the part. A full turn back to the mark is the most exact; type what you measured if it fell short or long. The figure is not the angle on screen, which is counted from the pulses. A good value is 200 × microsteps × gear ratio ÷ 360: {} for 16 microsteps and a 4:1 unit, {} for 6:1. A move of A alone runs at A's own maximum speed, whatever the jog speed.",
            steps_text(steps_per_degree(200., 16., 4.)),
            steps_text(steps_per_degree(200., 16., 6.))
        )
    } else {
        "Measure how far the carriage really moved, with a ruler or a dial indicator, not the position on screen: that is counted from the pulses. A longer move gives a better figure. The move runs at the jog speed, and the soft limits apply.".into()
    };
    machine_panel(&format!("Calibrate steps per {per}"), body.push(muted(how)))
}

/// What the running profile wires to a pin: "X step", "E-stop".
fn roles(cfg: &MachineConfig, pin: u8) -> String {
    let mut names = Vec::new();
    for (i, axis) in AXES.iter().enumerate() {
        let Some(a) = cfg.axes.get(i) else { continue };
        for (p, what) in [
            (Some(a.step), "step"),
            (Some(a.dir), "direction"),
            (a.limit_neg, "limit, low end"),
            (a.limit_pos, "limit, high end"),
            (a.home, "home"),
        ] {
            if p.is_some_and(|p| p.pin == pin) {
                names.push(format!("{} {what}", axis.to_uppercase()));
            }
        }
    }
    let (i, o) = (&cfg.inputs, &cfg.outputs);
    for (p, what) in [
        (i.estop, "E-stop"),
        (i.probe, "probe"),
        (i.index, "spindle index"),
        (o.spindle_pwm, "spindle PWM"),
        (o.spindle_dir, "spindle direction"),
        (o.spindle_cw, "spindle CW"),
        (o.spindle_ccw, "spindle CCW"),
        (o.flood, "flood"),
        (o.mist, "mist"),
        (o.enable, "drive enable"),
        (o.charge_pump, "charge pump"),
    ] {
        if p.is_some_and(|p| p.pin == pin) {
            names.push(what.into());
        }
    }
    if names.is_empty() {
        "unused".into()
    } else {
        names.join(", ")
    }
}

fn level<'a>(high: Option<bool>) -> iced::widget::Text<'a> {
    let (s, c) = match high {
        Some(true) => ("HIGH", Color::from_rgb8(39, 130, 93)),
        Some(false) => ("LOW", Color::from_rgb8(110, 128, 145)),
        None => ("?", Color::from_rgb8(110, 128, 145)),
    };
    text(s).size(12).font(Font::MONOSPACE).color(c).width(44)
}

/// The input pins: level, role, changes, the last edges and the E-stop input.
fn inputs(c: &Connection) -> Element<'_, Message> {
    let Some(st) = &c.snapshot.status else {
        return machine_panel(
            "Inputs",
            column![muted("No status from the controller yet.")],
        );
    };
    let mut rows = Column::new().spacing(4);
    for &pin in &INPUT_PINS {
        rows = rows.push(
            row![
                text(format!("Pin {pin}")).size(12).width(50),
                level(st.input_level(pin)),
                text(roles(&c.config, pin)).size(12).width(Fill),
                muted(format!("{} changes", c.inputs.changes(pin))),
            ]
            .spacing(8),
        );
    }
    let b = st.bits;
    let yes = |on: bool| if on { "yes" } else { "no" };
    let mut edges = Column::new().spacing(2);
    for (t, pin, high) in c.inputs.edges.iter().rev().take(6) {
        edges = edges.push(
            text(format!(
                "{t:8.2} s  pin {pin} → {}",
                if *high { "HIGH" } else { "LOW" }
            ))
            .size(12)
            .font(Font::MONOSPACE),
        );
    }
    machine_panel(
        "Inputs",
        column![
            rows,
            text(uc100::diagnostics::estop_input_report(
                c.config.inputs.estop,
                st
            ))
            .size(12),
            muted(format!(
                "Controller stop latch: {} · hard limit: {} · soft limit: {}",
                yes(b.stop_latched()),
                yes(b.hard_limit()),
                yes(b.soft_limit())
            )),
            edges,
            muted(
                "Press and release a switch to see its pin change. Every input HIGH can also mean nothing is connected. A change shorter than about 1/30 s can be missed."
            ),
        ],
    )
}

/// The output pins: the level the controller applies, role, and the test.
fn outputs(c: &Connection) -> Element<'_, Message> {
    let Some(st) = &c.snapshot.status else {
        return machine_panel(
            "Outputs",
            column![muted("No status from the controller yet.")],
        );
    };
    let word = uc100::protocol::OutputWord(st.outputs);
    let mut rows = Column::new().spacing(4);
    for &pin in &OUTPUT_PINS {
        let test: Element<_> = if c.config.firmware_output_pin(pin) {
            muted("driven by the controller").into()
        } else {
            let forced = c.forced.get(&pin).copied();
            let force = |high| {
                c.can_force(pin)
                    .then_some(Message::Machine(Action::TestOutput(pin, Some(high))))
            };
            row![
                act("High", force(true), forced == Some(true)),
                act("Low", force(false), forced == Some(false)),
                act(
                    "Release",
                    forced.map(|_| Message::Machine(Action::TestOutput(pin, None))),
                    false
                ),
            ]
            .spacing(4)
            .into()
        };
        rows = rows.push(
            row![
                text(format!("Pin {pin}")).size(12).width(50),
                level(word.level(pin)),
                text(roles(&c.config, pin)).size(12).width(Fill),
                test,
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    }
    machine_panel(
        "Outputs",
        column![
            rows,
            muted(
                "High and Low work while Idle on a pin no step or direction uses; E-stop releases every forced pin. Check a step or direction pin by moving its axis. The level is what the controller reports applying, not a measured voltage."
            ),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile_draft::tests::{PROFILE, draft};

    #[test]
    fn steps_per_degree_follows_the_drive() {
        // 1/16 microstepping on a 200-step motor: 3200 steps a turn.
        assert!((steps_per_degree(200., 16., 1.) - 8.888_889).abs() < 1e-6);
        assert!((steps_per_degree(200., 16., 4.) - 35.555_556).abs() < 1e-6);
        assert!((steps_per_degree(200., 16., 6.) - 53.333_333).abs() < 1e-6);
        assert_eq!(steps_text(steps_per_degree(200., 16., 4.)), "35.556");
        // 360 degrees commanded = one turn of pulses; a turn that fell 1.5
        // degrees short measures more steps per degree.
        let pulses = (360. * 35.556f64).round() as i32;
        let v = uc100::diagnostics::measured_steps_per_unit(pulses, 358.5).unwrap();
        assert!((v - f64::from(pulses) / 358.5).abs() < 1e-9);
        assert!(v > 35.556);
        // A turn of 90 degrees measured exactly confirms the figure.
        let v = uc100::diagnostics::measured_steps_per_unit(1280, 36.0).unwrap();
        assert_eq!(steps_text(v), "35.556");
        assert!(uc100::diagnostics::measured_steps_per_unit(1280, 0.).is_err());
    }

    #[test]
    fn the_pin_list_names_a_step_and_direction_once_a_is_in_the_profile() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = draft(dir.path(), PROFILE);
        let before = d.checked.clone().unwrap();
        assert_eq!(
            (roles(&before, 8), roles(&before, 9)),
            ("unused".into(), "unused".into())
        );
        d.edit(Edit::AddAxis("a".into()));
        let cfg = d.checked.clone().unwrap();
        assert_eq!(roles(&cfg, 8), "A step");
        assert_eq!(roles(&cfg, 9), "A direction");
        assert!(OUTPUT_PINS.contains(&8) && OUTPUT_PINS.contains(&9));
        // The controller drives them, so the High/Low test leaves them alone.
        assert!(cfg.firmware_output_pin(8) && cfg.firmware_output_pin(9));
        assert!(!before.firmware_output_pin(8));
    }
}
