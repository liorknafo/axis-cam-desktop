use crate::model::Document;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
pub fn state_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("AXIS CAM Studio")
        .join("workspace.json")
}
/// First of the notes `load_reporting` returns when it numbered the tools.
pub const NUMBERED_TOOLS: &str = "Tools in this project had no tool numbers and were numbered T1–Tn in library order. Check each one against your LinuxCNC tool table before exporting.";
pub fn load(path: &Path) -> Result<Document, String> {
    load_reporting(path).map(|(d, _)| d)
}
/// Loads a project with what the load changed worth saying: tools saved
/// without a tool number numbered in library order (`NUMBERED_TOOLS`), then
/// the note of linking a legacy material to the library. An empty workholding
/// library is given the starters without a note.
pub fn load_reporting(path: &Path) -> Result<(Document, Vec<String>), String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > 20_000_000 {
        return Err("Project exceeds 20 MB.".into());
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("Could not read AXIS project: {e}"))?;
    let legacy = value.get("sample_model").is_none();
    let mut d: Document =
        serde_json::from_value(value).map_err(|e| format!("Could not read AXIS project: {e}"))?;
    // Older projects displayed the built-in housing when no file was attached.
    if legacy {
        d.sample_model = d.file.is_none() && !d.setups.is_empty();
    }
    for setup in &mut d.setups {
        // Earlier "relative" mode stored absolute dimensions. Preserve them as
        // fixed stock instead of silently interpreting them as new offsets.
        if crate::stock::OFFSETS
            .iter()
            .all(|(key, _)| !setup.p.contains_key(*key))
        {
            if crate::stock::relative(setup) {
                setup.p.insert("stock_type".into(), "Fixed size box".into());
            }
            for (key, _) in crate::stock::OFFSETS {
                setup.p.insert(key.into(), "0".into());
            }
        }
    }
    let mut notes = vec![];
    if d.number_tools() {
        notes.push(NUMBERED_TOOLS.to_string());
    }
    notes.extend(d.adopt_materials());
    d.adopt_holders();
    d.validate_structure()?;
    Ok((d, notes))
}
pub fn archive(state_path: &Path, doc: &Document) -> Result<PathBuf, String> {
    let folder = state_path
        .parent()
        .ok_or("Missing workspace folder")?
        .join("history");
    let name: String = doc
        .title
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(60)
        .collect();
    let path = folder.join(format!(
        "{}-{}.axis.json",
        if name.is_empty() { "project" } else { &name },
        uuid::Uuid::new_v4()
    ));
    save(&path, doc)?;
    Ok(path)
}
pub fn save(path: &Path, doc: &Document) -> Result<(), String> {
    let data = serde_json::to_vec_pretty(doc).map_err(|e| e.to_string())?;
    write_atomic(path, &data)
}
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("Missing parent directory")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp = parent.join(format!(".axis-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })()
    .map_err(|e: std::io::Error| e.to_string());
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_roundtrip() {
        let p = std::env::temp_dir().join(format!("axis-{}.json", uuid::Uuid::new_v4()));
        let d = Document::default();
        save(&p, &d).unwrap();
        let restored = load(&p).unwrap();
        assert_eq!(d.title, restored.title);
        fs::remove_file(p).unwrap();
    }
    #[test]
    fn bad_file_not_replaced() {
        let p = std::env::temp_dir().join(format!("axis-{}.json", uuid::Uuid::new_v4()));
        fs::write(&p, b"bad").unwrap();
        assert!(load(&p).is_err());
        assert_eq!(fs::read(&p).unwrap(), b"bad");
        fs::remove_file(p).unwrap();
    }
    #[test]
    fn the_bearing_housing_example_links_to_a_starter_material() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/bearing-housing.axis.json");
        let (doc, notes) = load_reporting(&path).unwrap();
        let material: Vec<_> = notes.iter().filter(|n| n.contains("material")).collect();
        assert_eq!(
            material,
            ["The project material \"Aluminium 6061-T6\" was linked to the library."]
        );
        assert_eq!(doc.materials.len(), 15);
        assert!(doc.selected_material().unwrap().cutting_data().is_some());
    }
    #[test]
    fn loading_refuses_materials_that_share_an_id_or_have_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("materials.axis.json");
        let mut doc = Document::default();
        doc.adopt_materials();
        save(&path, &doc).unwrap();
        assert_eq!(load(&path).unwrap().materials.len(), 15);
        let first = doc.materials[0].id.clone();
        doc.materials[1].id = first;
        save(&path, &doc).unwrap();
        let error = load(&path).unwrap_err();
        assert!(error.contains("duplicate library identifier"), "{error}");
        doc.materials[1].id.clear();
        save(&path, &doc).unwrap();
        assert!(load(&path).is_err());
    }
    #[test]
    fn a_project_without_a_workholding_library_gets_the_starters_silently() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.axis.json");
        let mut value = serde_json::to_value(Document::sample()).unwrap();
        value.as_object_mut().unwrap().remove("workholding");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let (doc, notes) = load_reporting(&path).unwrap();
        let names: Vec<_> = doc.workholding.iter().map(|h| h.name()).collect();
        assert_eq!(names, ["100 mm machine vise", "Toe clamp 60 mm"]);
        assert!(
            notes.iter().all(|n| !n.contains("workholding")),
            "{notes:?}"
        );
        // Its setups model no workholding.
        let setup = &doc.setups[0];
        assert!(!setup.p.contains_key("holding"));
        assert_eq!(
            crate::holding::layout(setup),
            Ok(crate::holding::Layout::None)
        );
    }
    #[test]
    fn a_workholding_library_with_entries_is_loaded_as_it_is() {
        use crate::holding::Holder;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("holders.axis.json");
        let mut doc = Document::default();
        doc.workholding.push(Holder::new("Clamp", "Strap clamp"));
        save(&path, &doc).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.workholding, doc.workholding);
        // Two entries with one identifier, or one without, are refused.
        doc.workholding.push(doc.workholding[0].clone());
        save(&path, &doc).unwrap();
        let error = load(&path).unwrap_err();
        assert!(error.contains("duplicate library identifier"), "{error}");
        doc.workholding[1].id.clear();
        save(&path, &doc).unwrap();
        assert!(load(&path).is_err());
    }
    #[test]
    fn legacy_relative_dimensions_are_preserved_and_new_offsets_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stock.axis.json");
        let mut doc = Document::sample();
        for (key, _) in crate::stock::OFFSETS {
            doc.setup_mut().p.remove(key);
        }
        doc.setup_mut().p.insert("stock_x".into(), "177".into());
        save(&path, &doc).unwrap();
        let mut reopened = load(&path).unwrap();
        assert_eq!(
            crate::model::get(&reopened.setup().p, "stock_type"),
            "Fixed size box"
        );
        assert_eq!(crate::model::get(&reopened.setup().p, "stock_x"), "177");
        reopened
            .setup_mut()
            .p
            .insert("stock_type".into(), "Relative size box".into());
        reopened
            .setup_mut()
            .p
            .insert("stock_right".into(), "2".into());
        reopened
            .setup_mut()
            .p
            .insert("stock_top".into(), "10".into());
        save(&path, &reopened).unwrap();
        let restored = load(&path).unwrap();
        assert_eq!(restored.setup().p, reopened.setup().p);
    }
    #[test]
    fn legacy_sample_loads_but_new_empty_document_stays_empty() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("old.axis.json");
        let mut value = serde_json::to_value(Document::sample()).unwrap();
        value.as_object_mut().unwrap().remove("sample_model");
        fs::write(&p, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(load(&p).unwrap().sample_model);
        save(&p, &Document::default()).unwrap();
        let doc = load(&p).unwrap();
        assert!(!doc.sample_model && doc.setups.is_empty());
    }
    #[test]
    fn loading_numbers_legacy_tools_once() {
        let p = std::env::temp_dir().join(format!("axis-{}.json", uuid::Uuid::new_v4()));
        let mut d = Document::sample();
        for t in &mut d.tools {
            t.p.remove("number");
        }
        save(&p, &d).unwrap();
        let loaded = load(&p).unwrap();
        let numbers: Vec<_> = loaded.tools.iter().map(|t| t.number()).collect();
        assert_eq!(numbers, vec![Some(1), Some(2), Some(3)]);
        fs::remove_file(p).unwrap();
    }
    #[test]
    fn loading_reports_whether_tools_were_numbered() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("old.axis.json");
        let mut d = Document::sample();
        for t in &mut d.tools {
            t.p.remove("number");
        }
        save(&p, &d).unwrap();
        let (loaded, notes) = load_reporting(&p).unwrap();
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert_eq!(notes[0], NUMBERED_TOOLS);
        assert!(notes[1].contains("linked to the library"), "{notes:?}");
        // Saved with its numbers and its material link, it loads without notes.
        save(&p, &loaded).unwrap();
        let (again, notes) = load_reporting(&p).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(again.tools[2].number(), Some(3));
        assert!(load_reporting(&dir.path().join("missing.json")).is_err());
    }
}
