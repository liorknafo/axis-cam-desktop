//! The inspector's fields, as an agent sees them (LIO-470): `list_fields`
//! lists them and `edit_field` checks its key and value against them, so a
//! wrong key never enters the project. They are what the editors draw, found
//! by building each editor's elements with `ui::collecting` on: no layout, no
//! renderer. The user's open tab, step, page and selection are not changed;
//! an editor reads the active setup and the selected operation, so for
//! another setup or operation they are put there for the build and back.
use super::{class, commands};
use crate::{
    App, Target, help,
    ui::{self, FieldInfo},
};
use serde_json::{Value, json};

/// The targets of `list_fields` and `edit_field`, by name.
pub const TARGETS: [(&str, Target); 6] = [
    ("operation", Target::Operation),
    ("setup", Target::Setup),
    ("machine", Target::Machine),
    ("tool", Target::Tool),
    ("material", Target::Material),
    ("holder", Target::Holder),
];

/// The target `args` name.
pub fn target_of(args: &Value) -> Option<Target> {
    let name = args.get("target")?.as_str()?;
    TARGETS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, t)| *t)
}

fn why(p: commands::Problem) -> String {
    match p {
        commands::Problem::Args(e) | commands::Problem::Refused(e) => e,
    }
}

/// `build` with setup `si` active and operation `oi` selected, then as it was.
fn viewing<R>(app: &mut App, si: usize, oi: Option<usize>, build: impl FnOnce(&App) -> R) -> R {
    let active = std::mem::replace(&mut app.doc.active, app.doc.setups[si].id.clone());
    let selected = std::mem::replace(&mut app.selected, oi);
    let found = build(app);
    app.doc.active = active;
    app.selected = selected;
    found
}

/// The fields of the target `args` name, each once, as the editors now draw
/// them. `args` may give `setup` (for a setup) or `operation` (an id, or an
/// index in `setup`, else the active setup); the active setup and the selected operation
/// are the default.
pub fn of(app: &mut App, args: &Value) -> Result<Vec<FieldInfo>, String> {
    let Some(target) = target_of(args) else {
        return Err(format!(
            "`target` is one of: {}.",
            TARGETS.map(|(n, _)| n).join(", ")
        ));
    };
    let mut found = match target {
        Target::Operation => {
            let (si, oi) = commands::address(app, args, true)
                .map_err(why)?
                .1
                .ok_or("No operation is selected: give `operation`.")?;
            viewing(app, si, Some(oi), ui::operation_fields)
        }
        Target::Setup => {
            let si = commands::address(app, args, false)
                .map_err(why)?
                .0
                .ok_or("There is no setup: add one first.")?;
            viewing(app, si, None, ui::setup_fields)
        }
        Target::Machine => ui::machine_fields(app),
        Target::Tool => ui::tool_fields(app),
        Target::Material => ui::material_fields(app),
        Target::Holder => ui::holder_fields(app),
    };
    // A key on two tabs is one field, listed at its first.
    let mut seen = std::collections::HashSet::new();
    found.retain(|f| seen.insert(f.key));
    Ok(found)
}

/// A help text's first sentence.
fn first_sentence(text: &str) -> &str {
    match text.find(". ") {
        Some(end) => &text[..=end],
        None => text,
    }
}

/// The `list_fields` result: `{target, fields: [{key, label, tab, value,
/// choices, unit, help, command?}]}`. A key with `command` is changed by
/// that command, not by `edit_field`.
pub fn list(app: &mut App, args: &Value) -> Result<Value, String> {
    let fields = of(app, args)?;
    let target = target_of(args).unwrap_or(Target::Operation);
    let fields: Vec<Value> = fields
        .iter()
        .map(|f| {
            let mut field = json!({
                "key": f.key,
                "label": f.label,
                "tab": f.tab,
                "value": f.value,
                "choices": f.choices,
                "unit": f.unit,
                "help": first_sentence(&help::field_help(f.key, &f.label, f.target).body),
            });
            if let class::Class::Command(name) = class::edit(f.target, f.key)
                && name != "edit_field"
            {
                field["command"] = json!(name);
            }
            field
        })
        .collect();
    Ok(json!({"target": format!("{target:?}").to_lowercase(), "fields": fields}))
}

/// Why `edit_field` may not set `key` to `value`, given the target's
/// `fields`: the key is not one of them, it has a command of its own, or the
/// value is not one of its choices.
pub fn check(fields: &[FieldInfo], target: Target, key: &str, value: &str) -> Result<(), String> {
    let Some(field) = fields.iter().find(|f| f.key == key) else {
        let keys: Vec<&str> = fields.iter().map(|f| f.key).collect();
        return Err(if keys.is_empty() {
            format!("There is no field {key}: the {target:?} has none open (list_fields).")
        } else {
            format!(
                "There is no field {key} on the {target:?}; its fields are {}.",
                keys.join(", ")
            )
        });
    };
    if let class::Class::Command(name) = class::edit(field.target, key)
        && name != "edit_field"
    {
        return Err(format!("The field {key} is set by the command {name}."));
    }
    match &field.choices {
        Some(choices) if !choices.iter().any(|c| c == value) => Err(format!(
            "{value:?} is not one of the choices of {key}: {}.",
            choices.join(", ")
        )),
        _ => Ok(()),
    }
}
