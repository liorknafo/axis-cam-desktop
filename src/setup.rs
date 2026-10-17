//! Controller configuration on the Control page: the UC100 profile form, the inputs
//! and the output test, the driver's audit, and steps-per-mm calibration.
use crate::control::{Action, Connection};
use crate::model::{Machine, get};
use crate::ui::{act, bold, btn, caution, machine_panel, muted, note};
use crate::{App, Message, Target, help};
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
    /// Put the rotary A axis in the profile, on the free step/direction pair.
    AddAxis(String),
    /// Take an axis out of the profile; its section and comments go with it.
    RemoveAxis(String),
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
        Self::from_text(m.id.clone(), path.into(), original)
    }

    /// A profile given as text, which is `path`'s content.
    pub fn from_text(machine_id: String, path: PathBuf, original: String) -> Result<Self, String> {
        // toml_edit writes LF, so a CRLF file is read as LF and written back as CRLF.
        let crlf = original.contains("\r\n");
        let doc: DocumentMut = original.replace("\r\n", "\n").parse().map_err(|e| {
            format!(
                "{} is not valid TOML, so it cannot be edited here; fix it in a text editor: {e}",
                path.display()
            )
        })?;
        let mut d = Draft {
            machine_id,
            path,
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

    /// The document as it is now.
    pub fn to_text(&self) -> String {
        self.doc.to_string()
    }

    fn check(&mut self) {
        self.checked = MachineConfig::from_toml(&self.doc.to_string()).map_err(|e| e.to_string());
    }

    pub fn edit(&mut self, edit: Edit) {
        self.note = None;
        match edit {
            Edit::Number(path, text) => {
                let t = text.trim();
                let (parent, key) = split(&path);
                let k = scale(&path);
                if t.is_empty() {
                    if let Some(table) = self.table(parent) {
                        table.remove(key);
                    }
                } else if let (Ok(i), true) = (t.parse::<i64>(), k == 1.) {
                    self.put(parent, key, i.into());
                } else if let Some(f) = t.parse::<f64>().ok().filter(|f| f.is_finite()) {
                    self.put(parent, key, (f * k).into());
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
            Edit::AddAxis(letter) => self.add_axis(&letter),
            Edit::RemoveAxis(letter) => self.remove_axis(&letter),
        }
        self.check();
    }

    /// The output pins the document gives to an axis step or direction or
    /// to an output, with what uses them: (8, "A step").
    fn used_outputs(&self) -> Vec<(u8, String)> {
        let pin_of = |item: &Item| {
            let n = item.as_table_like()?.get("pin")?.as_integer()?;
            u8::try_from(n).ok()
        };
        let mut used = Vec::new();
        let table = |path| self.item(path).and_then(Item::as_table_like);
        for (letter, axis) in table("axes").into_iter().flat_map(|t| t.iter()) {
            for (key, what) in [("step", "step"), ("dir", "direction")] {
                if let Some(n) = axis
                    .as_table_like()
                    .and_then(|a| a.get(key))
                    .and_then(pin_of)
                {
                    used.push((n, format!("{} {what}", letter.to_uppercase())));
                }
            }
        }
        for (key, item) in table("outputs").into_iter().flat_map(|t| t.iter()) {
            if let Some(n) = pin_of(item) {
                used.push((n, key.replace('_', " ")));
            }
        }
        used
    }

    /// Adds the rotary A axis, in degrees, on pins 8 and 9, the free step and
    /// direction pair of this UC100. It is refused when a pin is taken. A
    /// has no verified wiring and no measured steps per degree, so the
    /// commissioning checks that cover them are unticked, and Reset, which
    /// every motion needs, is refused until they are checked again.
    fn add_axis(&mut self, letter: &str) {
        if letter != "a" {
            self.note = Some(format!("Only the A axis can be added here, not {letter}."));
            return;
        }
        if self.has("axes.a") {
            self.note = Some("The A axis is already in the profile.".into());
            return;
        }
        let used = self.used_outputs();
        for pin in A_PINS {
            if let Some((_, what)) = used.iter().find(|(n, _)| *n == pin) {
                self.note = Some(format!(
                    "The A axis needs pins {} and {}, and pin {pin} is already used by {what}. Give that role another pin first.",
                    A_PINS[0], A_PINS[1]
                ));
                return;
            }
        }
        let mut axis = crate::control::ROTARY_AXIS
            .parse::<DocumentMut>()
            .expect("ROTARY_AXIS is TOML");
        let Some(Item::Table(a)) = axis
            .remove("axes")
            .and_then(|mut axes| axes.as_table_mut()?.remove("a"))
        else {
            return;
        };
        let mut a = a;
        // After the last axis in the file, not first or last of the document.
        let after = self
            .item("axes")
            .and_then(Item::as_table)
            .into_iter()
            .flat_map(|t| t.iter())
            .filter_map(|(_, t)| t.as_table()?.position())
            .max();
        if let Some(after) = after {
            shift_tables(self.doc.as_table_mut(), after);
            a.set_position(Some(after + 1));
        } else {
            a.set_position(None);
        }
        let Some(axes) = self.table("axes") else {
            self.note = Some("axes is not a table, so the A axis cannot be added.".into());
            return;
        };
        axes.insert("a", Item::Table(a));
        self.put("commissioning", "required", true.into());
        for key in ["pinout_verified", "steps_per_unit_verified"] {
            self.put("commissioning", key, false.into());
        }
        self.note = Some(format!(
            "A axis added on pins {} and {}. Reset is refused, for every axis, until the commissioning checks are ticked again: check A's pins and calibrate its steps per degree first.",
            A_PINS[0], A_PINS[1]
        ));
    }

    /// Takes an axis out of the profile, with the comments inside its
    /// section, and its letter out of the homing order.
    fn remove_axis(&mut self, letter: &str) {
        if !self.has(&format!("axes.{letter}")) {
            return;
        }
        if let Some(axes) = self.table("axes") {
            axes.remove(letter);
        }
        let order = self.text("homing.order");
        let kept: String = order
            .chars()
            .filter(|c| !c.to_string().eq_ignore_ascii_case(letter))
            .collect();
        if kept != order {
            self.put("homing", "order", kept.as_str().into());
        }
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
        let k = scale(path);
        match self.item(path).and_then(Item::as_value) {
            Some(Value::Integer(i)) if k == 1. => i.value().to_string(),
            Some(Value::Float(f)) if k == 1. => f.value().to_string(),
            Some(Value::Integer(i)) => trimmed(*i.value() as f64 / k),
            Some(Value::Float(f)) => trimmed(f.value() / k),
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
/// The A axis' step and direction pins: the free pair on this UC100
/// (`control::ROTARY_AXIS` is the section added).
const A_PINS: [u8; 2] = [8, 9];
/// Moves every table placed after `after` one place down, to make room.
fn shift_tables(t: &mut Table, after: isize) {
    for (_, item) in t.iter_mut() {
        if let Item::Table(sub) = item {
            if let Some(p) = sub.position().filter(|p| *p > after) {
                sub.set_position(Some(p + 1));
            }
            shift_tables(sub, after);
        }
    }
}

/// What the form's number is multiplied by to get the file's: the A axis'
/// speed is shown in degrees per second and the driver reads degrees per minute.
fn scale(path: &str) -> f64 {
    if path == "axes.a.max_velocity" {
        60.
    } else {
        1.
    }
}

/// A number shown without float noise: 59.99999999 is not what was typed.
fn trimmed(v: f64) -> String {
    let s = format!("{v:.6}");
    s.trim_end_matches('0').trim_end_matches('.').into()
}

/// Steps per degree of a rotary axis from the motor's full steps per turn,
/// its microstepping and the gear ratio (4 for a 4:1 unit).
pub fn steps_per_degree(full_steps: f64, microsteps: f64, ratio: f64) -> f64 {
    full_steps * microsteps * ratio / 360.
}

/// The unit of an axis: the A axis is a rotary one.
fn unit(axis: &str) -> &'static str {
    if axis == "a" { "°" } else { "mm" }
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
            flag(d, "commissioning.pinout_verified".into(), "Step and direction pins and polarity checked (A included, once it is added)", false),
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

    const GATED: &str = "[controller]\n\
step_rate = \"25kHz\"\n\
\n\
[commissioning]\n\
required = true # the JY5800 is gated\n\
pinout_verified = true\n\
estop_verified = true\n\
drive_enable_verified = true\n\
steps_per_unit_verified = true\n\
\n\
[axes.x]\n\
steps_per_unit = 320\n\
max_velocity = 1000\n\
acceleration = 50\n\
step = { pin = 2 }\n\
dir = { pin = 3 }\n\
\n\
[axes.z]\n\
steps_per_unit = 320\n\
max_velocity = 500\n\
acceleration = 50\n\
step = { pin = 6 }\n\
dir = { pin = 7 }\n\
\n\
[inputs]\n\
estop = { pin = 10 }\n\
\n\
[homing]\n\
order = \"X\"\n";

    #[test]
    fn adding_the_a_axis_keeps_the_comments_and_the_other_axes() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = draft(dir.path(), PROFILE);
        d.edit(Edit::AddAxis("a".into()));
        let text = d.doc.to_string();
        // Every line of the file is still there, in order.
        let mut lines = text.lines();
        for l in PROFILE.lines() {
            assert!(lines.any(|t| t == l), "{l} is lost or moved: {text}");
        }
        // A sits after the last axis, before the homing section.
        assert!(
            text.find("[axes.x]") < text.find("[axes.a]")
                && text.find("[axes.a]") < text.find("[homing]"),
            "{text}"
        );
        assert!(text.contains("[axes.a]"), "{text}");
        let cfg = d.checked.clone().unwrap();
        let a = cfg.axes.a.as_ref().unwrap();
        assert_eq!((a.step.pin, a.dir.pin), (8, 9));
        assert_eq!((a.soft_min, a.soft_max), (Some(-180.), Some(180.)));
        assert_eq!(cfg.axes.x.as_ref().unwrap().steps_per_unit, 320.);
        assert_eq!((a.steps_per_unit, a.max_velocity / 60.), (44.444, 60.));
        // The form shows it in degrees per second.
        assert_eq!(d.number("axes.a.max_velocity"), "60");
        d.edit(Edit::Number("axes.a.max_velocity".into(), "30".into()));
        assert_eq!(
            d.checked
                .as_ref()
                .unwrap()
                .axes
                .a
                .as_ref()
                .unwrap()
                .max_velocity,
            1800.
        );
        // Taking it out again gives back the file it was.
        d.edit(Edit::RemoveAxis("a".into()));
        // What is left of it is the commissioning section the gate needs.
        assert_eq!(
            d.doc.to_string(),
            format!(
                "{PROFILE}\n[commissioning]\nrequired = true\npinout_verified = false\nsteps_per_unit_verified = false\n"
            )
        );
        assert!(d.checked.as_ref().unwrap().axes.a.is_none());
    }

    #[test]
    fn removing_a_leaves_the_comments_of_the_others_and_drops_a_from_the_homing_order() {
        let dir = tempfile::tempdir().unwrap();
        // Hand-written: A with a comment of its own, between X and Z.
        let with_a = GATED.replace("order = \"X\"", "order = \"XA\"").replace(
            "[axes.z]",
            "[axes.a]\n# the chuck\nsteps_per_unit = 35.5\nmax_velocity = 3600\nacceleration = 100\nstep = { pin = 8 }\ndir = { pin = 9 }\n\n[axes.z]",
        );
        let mut d = draft(dir.path(), &with_a);
        assert!(d.checked.is_ok(), "{:?}", d.checked);
        d.edit(Edit::RemoveAxis("a".into()));
        assert_eq!(d.doc.to_string(), GATED);
        assert!(d.checked.is_ok(), "{:?}", d.checked);
        assert!(d.checked.as_ref().unwrap().axes.z.is_some());
    }

    #[test]
    fn adding_a_is_refused_on_a_pin_that_is_already_used() {
        let dir = tempfile::tempdir().unwrap();
        for (pins, pin, who) in [
            ("step = { pin = 8 }", 8, "X step"),
            ("dir = { pin = 9 }", 9, "X direction"),
        ] {
            let text = PROFILE.replacen("step = { pin = 2, active_low = true }", pins, 1);
            let text = if pin == 9 {
                PROFILE.replacen("dir = { pin = 3, active_low = true }", pins, 1)
            } else {
                text
            };
            let mut d = draft(dir.path(), &text);
            d.edit(Edit::AddAxis("a".into()));
            let why = d.note.clone().unwrap();
            assert!(
                why.contains(&format!("pin {pin} is already used by {who}")),
                "{why}"
            );
            assert!(!d.has("axes.a") && !d.dirty());
        }
        // An output role counts too.
        let mut d = draft(
            dir.path(),
            &format!("{PROFILE}[outputs]\nflood = {{ pin = 9 }}\n"),
        );
        d.edit(Edit::AddAxis("a".into()));
        assert!(d.note.unwrap().contains("pin 9 is already used by flood"));
        // B and C are not offered, and a second A is not added.
        let mut d = draft(dir.path(), PROFILE);
        d.edit(Edit::AddAxis("b".into()));
        assert!(!d.has("axes.b") && !d.dirty());
        d.edit(Edit::AddAxis("a".into()));
        let once = d.doc.to_string();
        d.edit(Edit::AddAxis("a".into()));
        assert_eq!(d.doc.to_string(), once);
    }

    #[test]
    fn an_a_pin_taken_by_another_role_is_refused_by_the_driver_check() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = draft(dir.path(), PROFILE);
        d.edit(Edit::AddAxis("a".into()));
        d.edit(Edit::Pin("axes.a.step".into(), Some(2)));
        assert!(
            d.blocker()
                .unwrap()
                .contains("axis A step: pin 2 is already used by axis X step")
        );
        assert!(d.save().is_err());
        d.edit(Edit::Pin("axes.a.step".into(), Some(8)));
        assert!(d.checked.is_ok());
    }

    #[test]
    fn adding_a_withdraws_the_pinout_and_steps_checks_so_reset_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = draft(dir.path(), GATED);
        assert!(
            d.checked
                .as_ref()
                .unwrap()
                .commissioning_blocker()
                .is_none()
        );
        // X and Z were verified; A is not in the file yet.
        d.edit(Edit::AddAxis("a".into()));
        let cfg = d.checked.clone().unwrap();
        let why = cfg.commissioning_blocker().unwrap();
        assert!(why.contains("step/direction pins and polarity"), "{why}");
        assert!(why.contains("steps per unit"), "{why}");
        // What was checked on the other axes stays recorded.
        assert!(cfg.commissioning.estop_verified && cfg.commissioning.drive_enable_verified);
        assert!(
            d.doc
                .to_string()
                .contains("required = true # the JY5800 is gated")
        );
        // Only ticking them again opens it.
        d.edit(Edit::Flag("commissioning.pinout_verified".into(), true));
        assert!(
            d.checked
                .as_ref()
                .unwrap()
                .commissioning_blocker()
                .is_some()
        );
        d.edit(Edit::Flag(
            "commissioning.steps_per_unit_verified".into(),
            true,
        ));
        assert!(
            d.checked
                .as_ref()
                .unwrap()
                .commissioning_blocker()
                .is_none()
        );
    }

    #[test]
    fn a_profile_without_a_gate_is_gated_once_a_is_added() {
        let dir = tempfile::tempdir().unwrap();
        let mut d = draft(dir.path(), PROFILE);
        assert!(
            d.checked
                .as_ref()
                .unwrap()
                .commissioning_blocker()
                .is_none()
        );
        d.edit(Edit::AddAxis("a".into()));
        let why = d.checked.as_ref().unwrap().commissioning_blocker().unwrap();
        assert!(why.contains("unverified"), "{why}");
    }

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

    #[test]
    fn the_rotary_machines_profile_shows_a_in_degrees_and_its_assumed_values_are_not_verified() {
        let dir = tempfile::tempdir().unwrap();
        let m = crate::control::rotary_machine(dir.path()).unwrap();
        let mut d = Draft::open(&m).unwrap();
        let cfg = d.checked.clone().unwrap();
        assert!(cfg.axes.a.is_some() && cfg.axes.x.is_some());
        // X, Y, Z keep their evidence, A's pins and steps/deg are not checked.
        assert!(cfg.commissioning.estop_verified && cfg.commissioning.drive_enable_verified);
        assert!(!cfg.commissioning.pinout_verified && !cfg.commissioning.steps_per_unit_verified);
        let why = cfg.commissioning_blocker().unwrap();
        assert!(
            why.contains("pins and polarity") && why.contains("steps per unit"),
            "{why}"
        );
        assert_eq!(d.number("axes.a.steps_per_unit"), "44.444");
        assert_eq!(d.number("axes.a.max_velocity"), "60");
        assert_eq!(d.number("axes.a.acceleration"), "360");
        assert_eq!(
            (d.number("axes.a.soft_min"), d.number("axes.a.soft_max")),
            ("-180".into(), "180".into())
        );
        // The form's own gate stays shut until both are ticked.
        d.edit(Edit::Flag("commissioning.pinout_verified".into(), true));
        assert!(
            d.checked
                .as_ref()
                .unwrap()
                .commissioning_blocker()
                .is_some()
        );
        d.edit(Edit::Flag(
            "commissioning.steps_per_unit_verified".into(),
            true,
        ));
        assert!(
            d.checked
                .as_ref()
                .unwrap()
                .commissioning_blocker()
                .is_none()
        );
        // Taking A out leaves the owner's XYZ profile text.
        d.edit(Edit::RemoveAxis("a".into()));
        assert!(d.checked.as_ref().unwrap().axes.a.is_none());
        assert!(!d.to_text().contains("[axes.a]"));
        assert_eq!(
            d.checked
                .as_ref()
                .unwrap()
                .axes
                .x
                .as_ref()
                .unwrap()
                .steps_per_unit,
            320.
        );
        assert!(d.to_text().contains("# MEASURED 2026-10-10: 32000 pulses"));
    }

    #[test]
    fn a_rotary_profile_copy_written_before_the_gate_is_replaced_but_an_edited_one_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jy5800-rotary.toml");
        let old = format!(
            "{}{}",
            include_str!("../assets/jy5800.toml"),
            crate::control::ROTARY_AXIS
        );
        std::fs::write(&path, &old).unwrap();
        assert!(
            MachineConfig::from_toml(&old)
                .unwrap()
                .commissioning_blocker()
                .is_none()
        );
        crate::control::rotary_machine(dir.path()).unwrap();
        let now = std::fs::read_to_string(&path).unwrap();
        assert!(
            MachineConfig::from_toml(&now)
                .unwrap()
                .commissioning_blocker()
                .is_some()
        );
        let edited = old.replace("44.444", "35.556");
        std::fs::write(&path, &edited).unwrap();
        crate::control::rotary_machine(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    }
}
