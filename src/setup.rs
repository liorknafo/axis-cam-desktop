//! Machine setup on the Control page: the UC100 profile form, the inputs
//! and the output test, the driver's audit, and steps-per-mm calibration.
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
    doc: DocumentMut,
    /// Number fields as typed, by path, so "0." stays on screen while the
    /// document already holds 0. Those that are not numbers block Save.
    typed: BTreeMap<String, String>,
    /// The driver's check of the document.
    pub checked: Result<MachineConfig, String>,
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
        let doc: DocumentMut = original.parse().map_err(|e| {
            format!("{path} is not valid TOML, so it cannot be edited here; fix it in a text editor: {e}")
        })?;
        let mut d = Draft {
            machine_id: m.id.clone(),
            path: path.into(),
            original,
            doc,
            typed: BTreeMap::new(),
            checked: Err(String::new()),
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
        self.doc.to_string() != self.original
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
        let text = self.doc.to_string();
        crate::storage::write_atomic(&self.path, text.as_bytes())?;
        self.original = text;
        self.typed.clear();
        self.checked.clone()
    }
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
