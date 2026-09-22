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
pub fn load(path: &Path) -> Result<Document, String> {
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
    d.validate_structure()?;
    Ok(d)
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
}
