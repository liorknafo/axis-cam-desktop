//! Controller configuration on the Control page: the UC100 profile form, the inputs
//! and the output test, the driver's audit, and steps-per-mm calibration.
use crate::control::{Action, Connection};
use crate::model::{Machine, get};
use crate::ui::{act, bold, btn, caution, machine_panel, muted, note};
use crate::{App, Message};
use iced::widget::{
    Column, checkbox, column, container, pick_list, row, scrollable, text, text_input,
};
use iced::{Alignment, Color, Element, Fill, Font};
use std::collections::BTreeMap;
use std::path::PathBuf;
use toml_edit::{DocumentMut, InlineTable, Item, Table, TableLike, Value};
use uc100::config::MachineConfig;

/// A UC100 profile being edited. It is the file's own document, so the
/// comments (the JY5800's commissioning evidence lives in them) and the
/// layout survive, and only the values changed are rewritten.
pub struct Draft {
    pub machine_id: String,
    pub path: PathBuf,
    /// The file as read, to refuse saving over a change made since.
    original: String,
    /// The document as opened or last saved, to tell whether it changed.
    shown: String,
    /// The file has Windows line endings, which Save keeps.
    crlf: bool,
    doc: DocumentMut,
    /// Number fields as typed, by path, so "0." stays on screen while the
    /// document already holds 0. Those that are not numbers block Save.
    typed: BTreeMap<String, String>,
    /// The driver's check of the document.
    pub checked: Result<MachineConfig, String>,
    /// What the last Save did.
    pub note: Option<String>,
}

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

/// One change in the form. Paths are dotted keys: `axes.x.steps_per_unit`.
#[derive(Debug, Clone)]
pub enum Edit {
    /// A number field; empty clears an optional value.
    Number(String, String),
    /// A text field (the homing order, the step rate); empty clears it.
    Text(String, String),
    Flag(String, bool),
    /// A pin's number, or no pin.
    Pin(String, Option<u8>),
    /// A pin's polarity.
    ActiveLow(String, bool),
}

impl Draft {
    /// The profile of a UC100 machine.
    pub fn open(m: &Machine) -> Result<Self, String> {
        let path = get(&m.p, "uc100_profile").trim();
        if path.is_empty() {
            return Err("Choose the machine's UC100 profile file in the Machine library.".into());
        }
        let original =
            std::fs::read_to_string(path).map_err(|e| format!("Cannot read {path}: {e}"))?;
        // toml_edit writes LF, so a CRLF file is read as LF and written back as CRLF.
        let crlf = original.contains("\r\n");
        let doc: DocumentMut = original.replace("\r\n", "\n").parse().map_err(|e| {
            format!("{path} is not valid TOML, so it cannot be edited here; fix it in a text editor: {e}")
        })?;
        let mut d = Draft {
            machine_id: m.id.clone(),
            path: path.into(),
            original,
            shown: doc.to_string(),
            crlf,
            doc,
            typed: BTreeMap::new(),
            checked: Err(String::new()),
            note: None,
        };
        d.check();
        Ok(d)
    }

    fn check(&mut self) {
        self.checked = MachineConfig::from_toml(&self.doc.to_string()).map_err(|e| e.to_string());
    }

    pub fn edit(&mut self, edit: Edit) {
        match edit {
            Edit::Number(path, text) => {
                let t = text.trim();
                let (parent, key) = split(&path);
                if t.is_empty() {
                    if let Some(table) = self.table(parent) {
                        table.remove(key);
                    }
                } else if let Ok(i) = t.parse::<i64>() {
                    self.put(parent, key, i.into());
                } else if let Some(f) = t.parse::<f64>().ok().filter(|f| f.is_finite()) {
                    self.put(parent, key, f.into());
                }
                self.typed.insert(path, text);
            }
            Edit::Text(path, text) => {
                let (parent, key) = split(&path);
                if text.trim().is_empty() {
                    if let Some(table) = self.table(parent) {
                        table.remove(key);
                    }
                } else {
                    self.put(parent, key, text.trim().into());
                }
            }
            Edit::Flag(path, on) => {
                let (parent, key) = split(&path);
                self.put(parent, key, on.into());
            }
            Edit::Pin(path, pin) => {
                let (parent, key) = split(&path);
                let Some(table) = self.table(parent) else {
                    return;
                };
                match (pin, table.get_mut(key).and_then(Item::as_table_like_mut)) {
                    (None, _) => {
                        table.remove(key);
                    }
                    (Some(n), Some(old)) => put(old, "pin", i64::from(n).into()),
                    (Some(n), None) => {
                        let mut new = InlineTable::new();
                        new.insert("pin", i64::from(n).into());
                        table.insert(key, Item::Value(new.into()));
                    }
                }
            }
            Edit::ActiveLow(path, low) => {
                if let Some(pin) = self.table(&path) {
                    put(pin, "active_low", low.into());
                }
            }
        }
        self.check();
    }

    /// The table at a dotted path, made when missing; None when something
    /// on the way is not a table.
    fn table(&mut self, path: &str) -> Option<&mut dyn TableLike> {
        let mut t: &mut dyn TableLike = self.doc.as_table_mut();
        for k in path.split('.').filter(|k| !k.is_empty()) {
            if t.get(k).is_none() {
                let mut new = Table::new();
                new.set_implicit(true);
                t.insert(k, Item::Table(new));
            }
            t = t.get_mut(k)?.as_table_like_mut()?;
        }
        Some(t)
    }

    fn put(&mut self, parent: &str, key: &str, v: Value) {
        if let Some(table) = self.table(parent) {
            put(table, key, v);
        }
    }

    fn item(&self, path: &str) -> Option<&Item> {
        let (parent, key) = split(path);
        let mut t: &dyn TableLike = self.doc.as_table();
        for k in parent.split('.').filter(|k| !k.is_empty()) {
            t = t.get(k)?.as_table_like()?;
        }
        t.get(key)
    }

    pub fn has(&self, path: &str) -> bool {
        self.item(path).is_some()
    }

    /// A number field's text: as typed, else the file's value, else empty.
    pub fn number(&self, path: &str) -> String {
        if let Some(t) = self.typed.get(path) {
            return t.clone();
        }
        match self.item(path).and_then(Item::as_value) {
            Some(Value::Integer(i)) => i.value().to_string(),
            Some(Value::Float(f)) => f.value().to_string(),
            _ => String::new(),
        }
    }

    pub fn text(&self, path: &str) -> String {
        self.item(path)
            .and_then(Item::as_str)
            .unwrap_or_default()
            .into()
    }

    /// A flag, or None when the file leaves it to the driver's default.
    pub fn flag(&self, path: &str) -> Option<bool> {
        self.item(path).and_then(Item::as_bool)
    }

    /// A pin's number and whether it is active LOW.
    pub fn pin(&self, path: &str) -> Option<(u8, bool)> {
        let t = self.item(path)?.as_table_like()?;
        let n = t.get("pin")?.as_integer()?;
        let low = t.get("active_low").and_then(Item::as_bool);
        Some((u8::try_from(n).ok()?, low.unwrap_or(false)))
    }

    /// Number fields whose text is not a number.
    pub fn bad_numbers(&self) -> Vec<&str> {
        self.typed
            .iter()
            .filter(|(_, t)| {
                let t = t.trim();
                !t.is_empty() && t.parse::<f64>().ok().filter(|f| f.is_finite()).is_none()
            })
            .map(|(p, _)| p.as_str())
            .collect()
    }

    pub fn dirty(&self) -> bool {
        self.doc.to_string() != self.shown
    }

    /// Why Save is not offered, if it is not.
    pub fn blocker(&self) -> Option<String> {
        if let Some(path) = self.bad_numbers().first() {
            Some(format!("{path} is not a number."))
        } else if let Err(e) = &self.checked {
            Some(e.clone())
        } else if !self.dirty() {
            Some("No changes.".into())
        } else {
            None
        }
    }

    /// Writes the profile, unless it is not valid or the file changed on
    /// disk since it was opened. Returns the saved profile.
    pub fn save(&mut self) -> Result<MachineConfig, String> {
        if let Some(why) = self.blocker() {
            return Err(why);
        }
        let now = std::fs::read_to_string(&self.path)
            .map_err(|e| format!("Cannot read {}: {e}", self.path.display()))?;
        if now != self.original {
            return Err("The profile file changed since it was opened. Revert to load it, then make the changes again.".into());
        }
        let shown = self.doc.to_string();
        let text = if self.crlf {
            shown.replace('\n', "\r\n")
        } else {
            shown.clone()
        };
        crate::storage::write_atomic(&self.path, text.as_bytes())?;
        self.original = text;
        self.shown = shown;
        self.typed.clear();
        self.checked.clone()
    }
}

const AXES: [&str; 6] = ["x", "y", "z", "a", "b", "c"];
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
            flag(d, "commissioning.pinout_verified".into(), "Step and direction pins and polarity checked", false),
            flag(d, "commissioning.estop_verified".into(), "E-stop input and polarity checked", false),
            flag(d, "commissioning.drive_enable_verified".into(), "Drive enable and charge pump need checked", false),
            flag(d, "commissioning.steps_per_unit_verified".into(), "Steps per mm checked", false),
            muted("These record what was checked at the machine. AXIS CAM does not check the wiring itself."),
        ],
    ));
    for l in AXES.iter().filter(|l| d.has(&format!("axes.{l}"))) {
        let p = |k: &str| format!("axes.{l}.{k}");
        let numbers = column![
            number(d, p("steps_per_unit"), "Steps per mm", ""),
            number(d, p("max_velocity"), "Maximum speed · mm/min", ""),
            number(d, p("acceleration"), "Acceleration · mm/s²", ""),
            number(d, p("soft_min"), "Soft limit, low end · mm", "none"),
            number(d, p("soft_max"), "Soft limit, high end · mm", "none"),
            number(
                d,
                p("home_speed_percent"),
                "Homing speed · % of maximum",
                "20"
            ),
            number(d, p("home_position"), "Machine position at home · mm", "0"),
        ]
        .spacing(6);
        let pins = column![
            pin(d, p("step"), "Step", &OUTPUT_PINS),
            pin(d, p("dir"), "Direction", &OUTPUT_PINS),
            pin(d, p("limit_neg"), "Limit switch, low end", &INPUT_PINS),
            pin(d, p("limit_pos"), "Limit switch, high end", &INPUT_PINS),
            pin(d, p("home"), "Home switch", &INPUT_PINS),
            flag(d, p("reversed"), "Reverse the direction", false),
            flag(d, p("home_negative"), "Home toward the low end", true),
        ]
        .spacing(6);
        main = main.push(machine_panel(
            &format!("{} axis", l.to_uppercase()),
            column![row![numbers, pins].spacing(24)],
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
            flag(d, "homing.override_limits".into(), "Ignore the limit switches while homing", true),
            row![
                label("Step rate"),
                pick_list(rates, rate.or(Some("100kHz".into())), |r| edit(Edit::Text(
                    "controller.step_rate".into(),
                    r
                )))
                .text_size(12)
                .padding([5, 8])
                .width(110)
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            muted("Adding or removing an axis, motion tuning and the tool table stay edits of the file."),
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

/// Steps per mm from a commanded move and the travel measured at the machine.
fn calibration(c: &Connection) -> Element<'_, Message> {
    let cal = &c.calibration;
    let axis = cal.axis;
    let mut axes = row![label("Axis")].spacing(6).align_y(Alignment::Center);
    for (i, l) in AXES.iter().enumerate().filter(|(i, _)| c.axis(*i)) {
        axes = axes.push(act(
            l.to_uppercase(),
            Some(Message::Machine(Action::CalAxis(i))),
            i == axis,
        ));
    }
    let field = |name: &str, value: &str, f: fn(String) -> Action| {
        row![
            label(name),
            text_input("mm", value)
                .on_input(move |v| Message::Machine(f(v)))
                .width(110)
                .padding([5, 8])
                .size(12)
        ]
        .spacing(8)
        .align_y(Alignment::Center)
    };
    let go = (c.idle() && c.cal_distance().is_some()).then_some(Message::Machine(Action::CalMove));
    let spu = c.config.axes.get(axis).map_or(1., |a| a.steps_per_unit);
    let mut body = column![
        axes,
        field("Commanded travel · mm", &cal.distance, Action::CalDistance)
            .push(act("Move", go, false)),
    ];
    if let Some(pulses) = c.cal_pulses() {
        body = body.push(muted(format!(
            "{pulses} step pulses since the move began: {:.3} mm at the profile's {} steps per mm.",
            pulses as f64 / spu,
            steps_text(spu)
        )));
    }
    body = body.push(field(
        "Measured travel · mm",
        &cal.measured,
        Action::CalMeasured,
    ));
    match c.calibrated() {
        Some(Ok(v)) if c.idle() => {
            body = body.push(
                row![
                    bold(format!("{} steps per mm", steps_text(v)), 13),
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
    machine_panel(
        "Calibrate steps per mm",
        body.push(muted("Measure how far the carriage really moved, with a ruler or a dial indicator, not the position on screen: that is counted from the pulses. A longer move gives a better figure. The move runs at the jog speed, and the soft limits apply.")),
    )
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
                "High and Low work while Idle; E-stop releases every forced pin. The level is what the controller reports applying, not a measured voltage."
            ),
        ],
    )
}

/// `axes.x.step` → (`axes.x`, `step`).
fn split(path: &str) -> (&str, &str) {
    path.rsplit_once('.').unwrap_or(("", path))
}

/// Sets a value, keeping the old one's spacing and trailing comment.
fn put(t: &mut dyn TableLike, key: &str, v: Value) {
    match t.get_mut(key).and_then(Item::as_value_mut) {
        Some(old) => {
            let decor = old.decor().clone();
            *old = v;
            *old.decor_mut() = decor;
        }
        None => {
            t.insert(key, Item::Value(v));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: &str = "# Investigation profile: measured 2026-10-10.\n\
[controller]\n\
step_rate = \"25kHz\" # what the box ran\n\
\n\
[axes.x]\n\
# 32000 pulses moved about 100 mm on a ruler.\n\
steps_per_unit = 320\n\
max_velocity = 1000 # tests peaked near 900\n\
acceleration = 50\n\
step = { pin = 2, active_low = true }\n\
dir = { pin = 3, active_low = true }\n\
soft_min = 0\n\
soft_max = 390\n\
\n\
[homing]\n\
order = \"X\"\n";

    fn draft(dir: &std::path::Path, text: &str) -> Draft {
        let path = dir.join("machine.toml");
        std::fs::write(&path, text).unwrap();
        let mut m = Machine::new(3);
        m.p.insert("uc100_profile".into(), path.display().to_string());
        Draft::open(&m).unwrap()
    }

    fn changed_lines(a: &str, b: &str) -> Vec<String> {
        let a: Vec<&str> = a.lines().collect();
        b.lines()
            .filter(|l| !a.contains(l))
            .map(String::from)
            .collect()
    }

    #[test]
    fn an_edit_changes_its_line_only_and_keeps_the_comments() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = draft(dir.path(), PROFILE);
        assert!(d.checked.is_ok(), "{:?}", d.checked);
        assert_eq!(d.number("axes.x.steps_per_unit"), "320");
        d.edit(Edit::Number("axes.x.steps_per_unit".into(), "321.5".into()));
        d.edit(Edit::Number("axes.x.max_velocity".into(), "1200".into()));
        let text = d.doc.to_string();
        assert_eq!(
            changed_lines(PROFILE, &text),
            [
                "steps_per_unit = 321.5",
                "max_velocity = 1200 # tests peaked near 900"
            ]
        );
        assert_eq!(text.lines().count(), PROFILE.lines().count());
        assert_eq!(
            d.checked
                .as_ref()
                .unwrap()
                .axes
                .x
                .as_ref()
                .unwrap()
                .steps_per_unit,
            321.5
        );
        // Typing a decimal point keeps it on screen.
        d.edit(Edit::Number("axes.x.acceleration".into(), "50.".into()));
        assert_eq!(d.number("axes.x.acceleration"), "50.");
        assert!(d.blocker().is_none());
        d.edit(Edit::Text("controller.step_rate".into(), "50kHz".into()));
        assert_eq!(d.text("controller.step_rate"), "50kHz");
        assert!(
            d.doc
                .to_string()
                .contains("step_rate = \"50kHz\" # what the box ran")
        );
    }

    #[test]
    fn pins_change_appear_and_go() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = draft(dir.path(), PROFILE);
        assert_eq!(d.pin("axes.x.step"), Some((2, true)));
        d.edit(Edit::Pin("axes.x.step".into(), Some(9)));
        assert_eq!(d.pin("axes.x.step"), Some((9, true)));
        d.edit(Edit::Pin("axes.x.limit_neg".into(), Some(10)));
        d.edit(Edit::ActiveLow("axes.x.limit_neg".into(), true));
        assert_eq!(d.pin("axes.x.limit_neg"), Some((10, true)));
        // A table the file does not have yet.
        d.edit(Edit::Pin("inputs.estop".into(), Some(11)));
        assert!(d.checked.is_ok(), "{:?}", d.checked);
        let cfg = d.checked.clone().unwrap();
        assert_eq!(cfg.inputs.estop.unwrap().pin, 11);
        assert!(cfg.axes.x.as_ref().unwrap().limit_neg.unwrap().active_low);
        d.edit(Edit::Pin("axes.x.limit_neg".into(), None));
        assert_eq!(d.pin("axes.x.limit_neg"), None);
        // Two roles on one pin: the driver refuses it, and Save is not offered.
        d.edit(Edit::Pin("axes.x.dir".into(), Some(9)));
        assert!(d.checked.as_ref().unwrap_err().contains("already used"));
        assert!(d.blocker().unwrap().contains("already used"));
        assert!(d.save().is_err());
    }

    #[test]
    fn an_empty_optional_field_removes_its_key_and_bad_numbers_block_save() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = draft(dir.path(), PROFILE);
        d.edit(Edit::Number("axes.x.soft_min".into(), "".into()));
        assert!(!d.doc.to_string().contains("soft_min"));
        assert!(d.checked.is_ok());
        d.edit(Edit::Number("axes.x.acceleration".into(), "fast".into()));
        assert_eq!(d.number("axes.x.acceleration"), "fast");
        assert_eq!(d.blocker().unwrap(), "axes.x.acceleration is not a number.");
        d.edit(Edit::Number("axes.x.acceleration".into(), "60".into()));
        assert!(d.blocker().is_none());
        // A missing required number is the driver's error.
        d.edit(Edit::Number("axes.x.steps_per_unit".into(), "".into()));
        assert!(d.checked.as_ref().unwrap_err().contains("steps_per_unit"));
    }

    #[test]
    fn save_writes_the_file_and_refuses_one_changed_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = draft(dir.path(), PROFILE);
        assert_eq!(d.blocker().unwrap(), "No changes.");
        d.edit(Edit::Flag("commissioning.required".into(), true));
        let saved = d.save().unwrap();
        assert!(saved.commissioning.required);
        let text = std::fs::read_to_string(&d.path).unwrap();
        assert!(text.starts_with("# Investigation profile"));
        assert!(text.contains("[commissioning]\nrequired = true"));
        assert!(!d.dirty());
        // Someone edits the file by hand meanwhile.
        d.edit(Edit::Number("axes.x.soft_max".into(), "380".into()));
        std::fs::write(&d.path, PROFILE).unwrap();
        assert!(
            d.save()
                .unwrap_err()
                .contains("changed since it was opened")
        );
        assert_eq!(std::fs::read_to_string(&d.path).unwrap(), PROFILE);
    }

    #[test]
    fn a_windows_file_opens_unchanged_and_keeps_its_line_endings() {
        let dir = tempfile::tempdir().unwrap();
        let crlf = PROFILE.replace('\n', "\r\n");
        let mut d = draft(dir.path(), &crlf);
        assert!(!d.dirty());
        assert_eq!(d.blocker().unwrap(), "No changes.");
        d.edit(Edit::Number("axes.x.soft_max".into(), "380".into()));
        d.save().unwrap();
        let text = std::fs::read_to_string(&d.path).unwrap();
        assert_eq!(text, crlf.replace("soft_max = 390", "soft_max = 380"));
        assert!(!d.dirty());
        // Saving again checks the disk against what was written.
        d.edit(Edit::Number("axes.x.soft_max".into(), "370".into()));
        d.save().unwrap();
    }

    #[test]
    fn a_file_that_is_not_toml_is_not_opened() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("machine.toml");
        std::fs::write(&path, "[axes.x\n").unwrap();
        let mut m = Machine::new(3);
        m.p.insert("uc100_profile".into(), path.display().to_string());
        assert!(Draft::open(&m).err().unwrap().contains("not valid TOML"));
        m.p.insert("uc100_profile".into(), " ".into());
        assert!(Draft::open(&m).err().unwrap().contains("Machine library"));
    }
}
