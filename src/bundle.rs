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
const MACHINE: &str = "machine/";
/// Where the tablet finds a bundle it was given, beside its workspace.
pub const INBOX: &str = "inbox.axisjob";
pub const EXTENSION: &str = "axisjob";

/// The project, its part file, and for a STEP part the part converted as
/// `step::load` reads it: the tablet has no converter (`step::load`).
pub fn write(path: &Path, doc: &Document) -> Result<(), String> {
    crate::storage::write_atomic(path, &pack(doc)?)
}

/// The tablet file `write` writes, in memory: what the computer sends a
/// tablet over Wi-Fi (`tablet`).
pub fn pack(doc: &Document) -> Result<Vec<u8>, String> {
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
    // A UC100 machine's profile is a path on this computer too (LIO-456):
    // without it the tablet cannot connect.
    let mut profiles: Vec<(String, &Path)> = vec![];
    for path in profiles_of(doc) {
        let path = Path::new(path);
        let name = file_name(path)?;
        match profiles.iter().find(|(n, _)| *n == name) {
            Some((_, p)) if *p != path => {
                return Err(format!(
                    "Two machine profiles are named {name}: rename one."
                ));
            }
            Some(_) => {}
            None => profiles.push((name, path)),
        }
    }
    for (name, path) in profiles {
        let bytes = fs::read(path)
            .map_err(|e| format!("Could not read the machine profile {}: {e}", path.display()))?;
        add(&format!("{MACHINE}{name}"), &bytes)?;
    }
    Ok(w.finish().map_err(|e| e.to_string())?.into_inner())
}

/// The UC100 profile path of every machine in `doc`, the library's and the
/// setups' copies.
pub(crate) fn profiles_of(doc: &Document) -> impl Iterator<Item = &str> {
    let setups = doc.setups.iter().map(|s| &s.machine);
    (doc.machines.iter().chain(setups))
        .map(|m| crate::model::get(&m.p, "uc100_profile").trim())
        .filter(|p| !p.is_empty())
}

/// Unpacks the bundle at `zip` into `dir`: its part files into `dir/part`,
/// and the project as `dir/tablet.axis.json`, its part's path pointing
/// there. The project, that file, and what its load changed.
pub fn open(zip: &Path, dir: &Path) -> Result<(Document, PathBuf, Vec<String>), String> {
    let bad = |e: zip::result::ZipError| format!("Not an AXIS tablet file: {e}");
    let mut archive =
        zip::ZipArchive::new(fs::File::open(zip).map_err(|e| e.to_string())?).map_err(bad)?;
    let parts = dir.join("part");
    let machines = dir.join("machine");
    let mut profiles = vec![];
    let mut project = None;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(bad)?;
        let entry_name = entry.name().to_owned();
        let into = match &entry_name {
            n if n == PROJECT => None,
            n if n.starts_with(PART) => Some(&parts),
            n if n.starts_with(MACHINE) => Some(&machines),
            _ => continue,
        };
        let mut bytes = vec![];
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        match into {
            None => project = Some(bytes),
            // Only the file's name: a path in the zip never leaves its folder.
            Some(folder) => {
                let name = file_name(Path::new(&entry_name))?;
                crate::storage::write_atomic(&folder.join(&name), &bytes)?;
                if folder == &machines {
                    profiles.push(name);
                }
            }
        }
    }
    let project = project.ok_or("Not an AXIS tablet file: it has no project.")?;
    let path = dir.join("tablet.axis.json");
    crate::storage::write_atomic(&path, &project)?;
    let (mut doc, notes) = crate::storage::load_reporting(&path)?;
    if let Some(file) = &mut doc.file {
        file.path = parts.join(file_name(&file.path)?);
    }
    // Each machine's profile, where it came along (a bundle saved before
    // LIO-456 has none).
    let setups = doc.setups.iter_mut().map(|s| &mut s.machine);
    for m in doc.machines.iter_mut().chain(setups) {
        let path = crate::model::get(&m.p, "uc100_profile").trim();
        if let Ok(name) = file_name(Path::new(path))
            && profiles.contains(&name)
        {
            let here = machines.join(name).display().to_string();
            m.p.insert("uc100_profile".into(), here);
        }
    }
    crate::storage::save(&path, &doc)?;
    Ok((doc, path, notes))
}

/// How the tablet reaches each machine it already knows, kept in `doc` it
/// was sent (LIO-456): the connection and the Pi's address are this
/// device's, not the computer's that planned the job.
pub fn keep_connections(doc: &mut Document, here: &Document) {
    let setups = doc.setups.iter_mut().map(|s| &mut s.machine);
    for m in doc.machines.iter_mut().chain(setups) {
        let Some(mine) = here.machines.iter().find(|h| h.id == m.id) else {
            continue;
        };
        for key in ["uc100_connection", "uc100_pi"] {
            if let Some(value) = mine.p.get(key) {
                m.p.insert(key.into(), value.clone());
            }
        }
    }
}

/// A path's last part, whichever computer's separators it was written with.
fn file_name(path: &Path) -> Result<String, String> {
    let text = path.to_string_lossy();
    text.rsplit(['/', '\\'])
        .next()
        .filter(|n| !n.is_empty() && *n != "." && *n != "..")
        .map(str::to_owned)
        .ok_or_else(|| format!("No file name: {text}"))
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
        // A UC100 machine, in the library and as a setup's copy: its
        // profile is a path on this computer too (LIO-456).
        let uc100 = crate::control::owners_machine(mac.path()).unwrap();
        doc.machines.push(uc100.clone());
        doc.setups[0].machine = uc100;
        let zip = mac.path().join("Block.axisjob");
        write(&zip, &doc).unwrap();

        let tablet = tempfile::tempdir().unwrap();
        let (opened, path, _) = open(&zip, tablet.path()).unwrap();
        assert_eq!(opened.title, "Block");
        let file = opened.file.clone().unwrap();
        assert_eq!(file.path, tablet.path().join("part").join("block.stl"));
        assert_eq!(fs::read(&file.path).unwrap(), b"solid x\nendsolid x\n");
        assert!(path.exists());
        let profile = tablet.path().join("machine").join("jy5800.toml");
        let setup = &opened.setups[0].machine;
        for m in [opened.machines.last().unwrap(), setup] {
            assert_eq!(m.p["uc100_profile"], profile.display().to_string());
            assert!(crate::control::profile(m).is_ok());
        }
        assert_eq!(
            fs::read(&profile).unwrap(),
            fs::read(mac.path().join("jy5800.toml")).unwrap()
        );
        // The tablet's own way to the machine stays as it set it.
        let mut here = opened.clone();
        let mine = here.machines.last_mut().unwrap();
        mine.p
            .insert("uc100_connection".into(), crate::control::PI.into());
        mine.p.insert("uc100_pi".into(), "10.0.0.7".into());
        let mut sent = opened.clone();
        keep_connections(&mut sent, &here);
        for m in [sent.machines.last().unwrap(), &sent.setups[0].machine] {
            assert_eq!(crate::control::pi(m), Some("10.0.0.7"));
        }
        // Two different profiles of one name cannot both travel.
        let other = mac.path().join("other");
        let mut twin = crate::control::owners_machine(&other).unwrap();
        twin.id = "twin".into();
        doc.machines.push(twin);
        assert!(write(&zip, &doc).unwrap_err().contains("jy5800.toml"));
        assert_eq!(
            file_name(Path::new("/Users/x/part/a.step")).unwrap(),
            "a.step"
        );
        assert_eq!(file_name(Path::new(r"C:\x\..\b.stl")).unwrap(), "b.stl");
        assert!(file_name(Path::new("part/..")).is_err());
        assert!(open(&tablet.path().join("tablet.axis.json"), tablet.path()).is_err());
    }
}
