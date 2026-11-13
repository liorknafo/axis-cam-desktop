//! A project with its part in one file, for the shop tablet (LIO-453): a
//! zip named `.axisjob`, as Android's Files app opens a `.zip` itself
//! instead of offering AXIS Shop.
//! A project only points to its part by a path on the computer that planned
//! it; the bundle carries the part along. The computer writes it (Save for
//! tablet…), and the tablet opens it from "Open with" (`ShopActivity.java`
//! copies it to `INBOX` beside the workspace).
use crate::model::Document;
use std::{
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};

const PROJECT: &str = "project.json";
const PART: &str = "part/";
/// Where the tablet finds a bundle it was given, beside its workspace.
pub const INBOX: &str = "inbox.axisjob";
pub const EXTENSION: &str = "axisjob";

/// The project, its part file, and for a STEP part the part converted as
/// `step::load` reads it: the tablet has no converter (`step::load`).
pub fn write(path: &Path, doc: &Document) -> Result<(), String> {
    let mut w = zip::ZipWriter::new(Cursor::new(vec![]));
    let opt = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut add = |name: &str, bytes: &[u8]| -> Result<(), String> {
        w.start_file(name, opt).map_err(|e| e.to_string())?;
        w.write_all(bytes).map_err(|e| e.to_string())
    };
    let json = serde_json::to_vec_pretty(doc).map_err(|e| e.to_string())?;
    add(PROJECT, &json)?;
    if let Some(file) = &doc.file {
        let name = file_name(&file.path)?;
        let part = fs::read(&file.path)
            .map_err(|e| format!("Could not read the part {}: {e}", file.path.display()))?;
        add(&format!("{PART}{name}"), &part)?;
        if crate::step::is_step(&file.path) {
            add(
                &format!("{PART}{name}.stl"),
                &crate::step::converted(&file.path)?,
            )?;
        }
    }
    let zip = w.finish().map_err(|e| e.to_string())?.into_inner();
    crate::storage::write_atomic(path, &zip)
}

/// Unpacks the bundle at `zip` into `dir`: its part files into `dir/part`,
/// and the project as `dir/tablet.axis.json`, its part's path pointing
/// there. The project, that file, and what its load changed.
pub fn open(zip: &Path, dir: &Path) -> Result<(Document, PathBuf, Vec<String>), String> {
    let bad = |e: zip::result::ZipError| format!("Not an AXIS tablet file: {e}");
    let mut archive =
        zip::ZipArchive::new(fs::File::open(zip).map_err(|e| e.to_string())?).map_err(bad)?;
    let parts = dir.join("part");
    let mut project = None;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(bad)?;
        let entry_name = entry.name().to_owned();
        if entry_name != PROJECT && !entry_name.starts_with(PART) {
            continue;
        }
        let mut bytes = vec![];
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if entry_name == PROJECT {
            project = Some(bytes);
        } else {
            // Only the file's name: a path in the zip never leaves `parts`.
            let name = file_name(Path::new(&entry_name))?;
            crate::storage::write_atomic(&parts.join(name), &bytes)?;
        }
    }
    let project = project.ok_or("Not an AXIS tablet file: it has no project.")?;
    let path = dir.join("tablet.axis.json");
    crate::storage::write_atomic(&path, &project)?;
    let (mut doc, notes) = crate::storage::load_reporting(&path)?;
    if let Some(file) = &mut doc.file {
        file.path = parts.join(file_name(&file.path)?);
    }
    crate::storage::save(&path, &doc)?;
    Ok((doc, path, notes))
}

/// A path's last part, whichever computer's separators it was written with.
fn file_name(path: &Path) -> Result<String, String> {
    let text = path.to_string_lossy();
    text.rsplit(['/', '\\'])
        .next()
        .filter(|n| !n.is_empty() && *n != "." && *n != "..")
        .map(str::to_owned)
        .ok_or_else(|| format!("The part has no file name: {text}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_travels_with_project() {
        let mac = tempfile::tempdir().unwrap();
        let part = mac.path().join("block.stl");
        fs::write(&part, b"solid x\nendsolid x\n").unwrap();
        let mut doc = Document::sample();
        doc.title = "Block".into();
        doc.file = Some(crate::model::ModelFile {
            name: "block.stl".into(),
            path: part,
            format: "stl".into(),
            bytes: 19,
            thickness: None,
        });
        let zip = mac.path().join("Block.axisjob");
        write(&zip, &doc).unwrap();

        let tablet = tempfile::tempdir().unwrap();
        let (opened, path, _) = open(&zip, tablet.path()).unwrap();
        assert_eq!(opened.title, "Block");
        let file = opened.file.unwrap();
        assert_eq!(file.path, tablet.path().join("part").join("block.stl"));
        assert_eq!(fs::read(&file.path).unwrap(), b"solid x\nendsolid x\n");
        assert!(path.exists());
        assert_eq!(
            file_name(Path::new("/Users/x/part/a.step")).unwrap(),
            "a.step"
        );
        assert_eq!(file_name(Path::new(r"C:\x\..\b.stl")).unwrap(), "b.stl");
        assert!(file_name(Path::new("part/..")).is_err());
        assert!(open(&tablet.path().join("tablet.axis.json"), tablet.path()).is_err());
    }
}
