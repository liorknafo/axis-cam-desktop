//! A UC100 profile being edited: the file's own TOML document, edited a value
//! at a time with its comments and layout kept. Pure editing, no UI: it sits
//! below the app (LIO-729), and `setup` is the form that drives it.
use crate::model::{Machine, get};
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
    /// A was jogged A+ and the top of the part seen to move toward −Y
    /// (`a_direction`): the pins-and-polarity tick waits for it once A is
    /// in the profile. Not in the file, whose sections the driver reads;
    /// a profile opened with that tick already on has it.
    pub a_direction: bool,
}

/// The A axis' commissioning step for its direction (LIO-210): +A turns the
/// part as the machine's Direction says (LIO-212).
pub fn a_direction(o: crate::orient::Orient) -> String {
    format!(
        "Jogged A+ and the top of the part moved the way the machine's Direction in the Machine library says: toward {}, for Positive (the right-hand rule about +X), toward {}, for Negative (if it moved the other way, tick Reverse the direction)",
        o.y_words(false),
        o.y_words(true)
    )
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
    /// A was jogged A+ and turned the top of the part toward −Y, or not.
    ADirection(bool),
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
            a_direction: false,
        };
        d.a_direction = d.has("axes.a") && d.flag(PINOUT) == Some(true);
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
            Edit::Flag(path, on)
                if path == PINOUT && on && self.has("axes.a") && !self.a_direction =>
            {
                // The side is said by the check itself, drawn by the machine
                // as it is now (LIO-457): a kept note never names a stale one.
                self.note = Some("Check A's direction first: jog A+, see that the top of the part moves the way the check under the A axis says, and tick it.".into());
            }
            Edit::Flag(path, on) => {
                let (parent, key) = split(&path);
                self.put(parent, key, on.into());
                // A turning the other way now must be seen again.
                if path == "axes.a.reversed" && self.a_direction {
                    self.a_direction = false;
                    self.put("commissioning", "pinout_verified", false.into());
                    self.note = Some("A's direction changed: jog A+ and check that the top of the part moves the way the check under the A axis says, then tick the pins and polarity again.".into());
                }
            }
            Edit::ADirection(on) => {
                self.a_direction = on;
                if !on && self.flag(PINOUT) == Some(true) {
                    self.put("commissioning", "pinout_verified", false.into());
                }
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
        let mut axis = crate::machine_profile::ROTARY_AXIS
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
        self.a_direction = false;
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

pub(crate) const AXES: [&str; 6] = ["x", "y", "z", "a", "b", "c"];
/// The pins-and-polarity tick.
pub(crate) const PINOUT: &str = "commissioning.pinout_verified";
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
pub(crate) fn unit(axis: &str) -> &'static str {
    if axis == "a" { "°" } else { "mm" }
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
pub(crate) mod tests {
    use super::*;

    pub(crate) const PROFILE: &str = "# Investigation profile: measured 2026-10-10.\n\
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

    pub(crate) fn draft(dir: &std::path::Path, text: &str) -> Draft {
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
        // Only ticking them again opens it, A's direction first.
        d.edit(Edit::ADirection(true));
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
    fn the_rotary_machines_profile_shows_a_in_degrees_and_its_assumed_values_are_not_verified() {
        let dir = tempfile::tempdir().unwrap();
        let m = crate::machine_profile::rotary_machine(dir.path()).unwrap();
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
        // The pins and polarity wait for A's direction (LIO-210).
        assert!(!d.a_direction);
        d.edit(Edit::Flag(PINOUT.into(), true));
        assert_eq!(d.flag(PINOUT), Some(false));
        assert!(
            d.note
                .as_ref()
                .unwrap()
                .contains("the way the check under the A axis says")
        );
        d.edit(Edit::ADirection(true));
        // The form's own gate stays shut until both are ticked.
        d.edit(Edit::Flag(PINOUT.into(), true));
        assert_eq!(d.flag(PINOUT), Some(true));
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
        // Reversing A unticks its direction and the pins and polarity.
        d.edit(Edit::Flag("axes.a.reversed".into(), true));
        assert!(!d.a_direction && d.flag(PINOUT) == Some(false));
        assert!(d.note.as_ref().unwrap().contains("direction changed"));
        d.edit(Edit::ADirection(true));
        d.edit(Edit::Flag(PINOUT.into(), true));
        // Unticking the direction unticks them too.
        d.edit(Edit::ADirection(false));
        assert_eq!(d.flag(PINOUT), Some(false));
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
            crate::machine_profile::ROTARY_AXIS
        );
        std::fs::write(&path, &old).unwrap();
        assert!(
            MachineConfig::from_toml(&old)
                .unwrap()
                .commissioning_blocker()
                .is_none()
        );
        crate::machine_profile::rotary_machine(dir.path()).unwrap();
        let now = std::fs::read_to_string(&path).unwrap();
        assert!(
            MachineConfig::from_toml(&now)
                .unwrap()
                .commissioning_blocker()
                .is_some()
        );
        let edited = old.replace("44.444", "35.556");
        std::fs::write(&path, &edited).unwrap();
        crate::machine_profile::rotary_machine(dir.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    }
}
