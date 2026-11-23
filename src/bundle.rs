//! A project with its part in one file, for the shop tablet (LIO-453): a
//! zip named `.axisjob`, as Android's Files app opens a `.zip` itself
//! instead of offering AXIS Shop.
//! A project only points to its part by a path on the computer that planned
//! it; the bundle carries the part along. The computer writes it (Save for
//! tablet…), and the tablet opens it from "Open with" (`ShopActivity.java`
//! copies it to `INBOX` beside the workspace). It also carries the tool runs
//! worked out here for the TypeScript tablet app (`Runs`, LIO-475), and the
//! hand setups' Steps screens drawn here (`steps::tablet`, LIO-477).
use crate::model::Document;
use std::{
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};

const PROJECT: &str = "project.json";
const PART: &str = "part/";
const MACHINE: &str = "machine/";
/// The tool runs worked out for the tablet (`Runs`).
pub const RUNS: &str = "runs.json";
/// The hand setups' Steps screens (`steps::tablet::Steps`); the PNGs they
/// name sit under `steps/`.
pub const STEPS: &str = "steps.json";
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
    let mesh = part_mesh(doc);
    pack_with(doc, mesh.as_ref().map_err(String::as_str))
}

/// `pack` with the part's `mesh` given: the runs and the steps are worked
/// out on it.
pub fn pack_with(
    doc: &Document,
    mesh: Result<&crate::mesh::Mesh, &str>,
) -> Result<Vec<u8>, String> {
    let mut w = zip::ZipWriter::new(Cursor::new(vec![]));
    let opt = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    // A PNG is compressed already: stored as it is, the tablet takes it out
    // without inflating it, which is slow on it (LIO-477).
    let stored = opt.compression_method(zip::CompressionMethod::Stored);
    let mut add = |name: &str, bytes: &[u8]| -> Result<(), String> {
        let opt = if name.ends_with(".png") { stored } else { opt };
        w.start_file(name, opt).map_err(|e| e.to_string())?;
        w.write_all(bytes).map_err(|e| e.to_string())
    };
    let json = serde_json::to_vec_pretty(doc).map_err(|e| e.to_string())?;
    add(PROJECT, &json)?;
    let runs = serde_json::to_vec_pretty(&runs(doc, mesh)).map_err(|e| e.to_string())?;
    add(RUNS, &runs)?;
    let (steps, pictures) = crate::steps::tablet::steps(doc, mesh);
    add(
        STEPS,
        &serde_json::to_vec_pretty(&steps).map_err(|e| e.to_string())?,
    )?;
    let mut pictures: Vec<_> = pictures.into_iter().collect();
    pictures.sort();
    for (name, bytes) in pictures {
        add(&name, &bytes)?;
    }
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

/// What the TypeScript tablet app runs (LIO-475): the tool runs of each
/// setup on a UC100 machine, worked out here, as the tablet plans nothing.
/// `format` changes when a reader of an older one would run it wrong.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Runs {
    pub format: u32,
    pub setups: Vec<crate::run::TabletSetup>,
    /// Every setup of the project by id, in order, on whatever machine
    /// (LIO-610): "Setup N" is a setup's place in it, as the computer counts
    /// it, where `setups` holds only the UC100 ones. Added without a new
    /// `format`, as a tablet that does not know it reads the file as before
    /// (`Runs.plan` in axis-shop's `runs.ts`); none in a file from before.
    #[serde(default)]
    pub plan: Vec<String>,
}

pub fn runs(doc: &Document, mesh: Result<&crate::mesh::Mesh, &str>) -> Runs {
    let setups = (0..doc.setups.len())
        .filter(|&si| crate::model::get(&doc.setups[si].machine.p, "controller") == "UC100")
        .map(|si| crate::run::tablet_runs(doc, mesh, si))
        .collect();
    let plan = doc.setups.iter().map(|s| s.id.clone()).collect();
    Runs {
        format: 1,
        setups,
        plan,
    }
}

/// The part, for the runs and the steps worked out for the tablet; not
/// loaded when no setup needs it.
fn part_mesh(doc: &Document) -> Result<crate::mesh::Mesh, String> {
    let needed = doc.setups.iter().any(|s| {
        crate::model::get(&s.machine.p, "controller") == "UC100"
            || crate::manual::is_hand(&s.machine)
    });
    match (&doc.file, needed) {
        (_, false) => Err(String::new()),
        (Some(file), true) => {
            crate::mesh::load_file(file).map_err(|e| format!("The part could not be loaded: {e}"))
        }
        (None, true) => Err(crate::run::NO_PART.into()),
    }
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
/// its machine profiles into `dir/machine`, and the project as
/// `dir/tablet.axis.json`, its part's and machines' paths pointing there.
/// The bundle's profile wins, as the job was planned with it; a different
/// copy the tablet had is kept as `.old` (LIO-464). The project, that file,
/// and notes on what changed.
pub fn open(zip: &Path, dir: &Path) -> Result<(Document, PathBuf, Vec<String>), String> {
    let bad = |e: zip::result::ZipError| format!("Not an AXIS tablet file: {e}");
    let mut archive =
        zip::ZipArchive::new(fs::File::open(zip).map_err(|e| e.to_string())?).map_err(bad)?;
    let parts = dir.join("part");
    let machines = dir.join("machine");
    let mut project = None;
    let mut kept = vec![];
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
                let to = folder.join(&name);
                if folder == &machines && fs::read(&to).is_ok_and(|old| old != bytes) {
                    fs::rename(&to, folder.join(format!("{name}.old")))
                        .map_err(|e| e.to_string())?;
                    kept.push(format!(
                        "This tablet's {name} differed from the file's; it is kept as {name}.old."
                    ));
                }
                crate::storage::write_atomic(&to, &bytes)?;
            }
        }
    }
    let project = project.ok_or("Not an AXIS tablet file: it has no project.")?;
    let path = dir.join("tablet.axis.json");
    crate::storage::write_atomic(&path, &project)?;
    let (mut doc, mut notes) = crate::storage::load_reporting(&path)?;
    notes.extend(kept);
    if let Some(file) = &mut doc.file {
        file.path = parts.join(file_name(&file.path)?);
    }
    // Each machine's profile, where it came along or an earlier bundle left
    // it (one saved before LIO-456 carries none: LIO-464).
    let setups = doc.setups.iter_mut().map(|s| &mut s.machine);
    for m in doc.machines.iter_mut().chain(setups) {
        let path = crate::model::get(&m.p, "uc100_profile").trim();
        if let Ok(name) = file_name(Path::new(path))
            && let here = machines.join(name)
            && here.exists()
        {
            m.p.insert("uc100_profile".into(), here.display().to_string());
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
        // A copy changed on the tablet is kept, not lost (LIO-464); opened
        // again unchanged, nothing is said.
        fs::write(&profile, b"changed at the machine").unwrap();
        let (_, _, notes) = open(&zip, tablet.path()).unwrap();
        assert!(
            notes.iter().any(|n| n.contains("jy5800.toml.old")),
            "{notes:?}"
        );
        let old = tablet.path().join("machine").join("jy5800.toml.old");
        assert_eq!(fs::read(old).unwrap(), b"changed at the machine");
        let (_, _, notes) = open(&zip, tablet.path()).unwrap();
        assert!(notes.iter().all(|n| !n.contains(".old")), "{notes:?}");
        // A bundle saved before profiles travelled uses the tablet's copy.
        let older = mac.path().join("Older.axisjob");
        let mut w = zip::ZipWriter::new(fs::File::create(&older).unwrap());
        (w.start_file(PROJECT, zip::write::SimpleFileOptions::default())).unwrap();
        w.write_all(&serde_json::to_vec(&doc).unwrap()).unwrap();
        w.finish().unwrap();
        let (opened_older, _, _) = open(&older, tablet.path()).unwrap();
        let m = &opened_older.setups[0].machine;
        assert_eq!(m.p["uc100_profile"], profile.display().to_string());
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

    /// The tablet file lists every setup of the project by id, in order,
    /// whatever machine it is on (LIO-610): with Setup 1 on a lathe and
    /// Setups 2 and 3 on a UC100, the tablet counts the third "Setup 3" as
    /// the computer does, not "Setup 2" from the UC100 setups alone.
    #[test]
    fn the_tablet_file_lists_every_setup_whatever_its_machine() {
        let mut doc = Document::sample();
        let mut uc100 = doc.setups[0].machine.clone();
        uc100.p.insert("controller".into(), "UC100".into());
        let lathe = crate::model::Setup::new(crate::lathe::defaults(), "G54");
        let second = crate::model::Setup::new(uc100.clone(), "G54");
        let third = crate::model::Setup::new(uc100, "G55");
        doc.setups = vec![lathe, second, third];
        let ids: Vec<String> = doc.setups.iter().map(|s| s.id.clone()).collect();
        let bytes = pack_with(&doc, Err("no part")).unwrap();
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut json = vec![];
        (zip.by_name(RUNS).unwrap().read_to_end(&mut json)).unwrap();
        // As the tablet reads it (`Runs.plan?: string[]`): an array of the
        // setup ids under `plan`, the format unchanged.
        let raw: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(raw["plan"], serde_json::json!(ids));
        assert_eq!(raw["format"], 1);
        let runs: Runs = serde_json::from_slice(&json).unwrap();
        assert_eq!(runs.plan, ids);
        let on_uc100: Vec<&str> = runs.setups.iter().map(|s| s.setup_id.as_str()).collect();
        assert_eq!(on_uc100, [&ids[1], &ids[2]]);
        assert!(!on_uc100.contains(&ids[0].as_str()));
        // A tablet file from before it still reads, with no plan.
        let older: Runs = serde_json::from_str(r#"{"format":1,"setups":[]}"#).unwrap();
        assert!(older.plan.is_empty() && older.setups.is_empty());
    }

    /// The tablet file carries the hand setups' Steps screens and every
    /// picture they name (LIO-477).
    #[test]
    fn the_tablet_file_carries_the_steps_screens() {
        let (mut doc, _) = crate::guide::tests::job("Yes");
        doc.file = Some(crate::model::ModelFile {
            name: "four-setup-block.3mf".into(),
            path: Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/four-setup-block.3mf"),
            format: "3mf".into(),
            bytes: 1,
            thickness: None,
        });
        let started = std::time::Instant::now();
        let bytes = pack(&doc).unwrap();
        eprintln!("Packed in {:?}: {} bytes", started.elapsed(), bytes.len());
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut read = |name: &str| {
            let mut bytes = vec![];
            zip.by_name(name).unwrap().read_to_end(&mut bytes).unwrap();
            bytes
        };
        let steps: crate::steps::tablet::Steps = serde_json::from_slice(&read(STEPS)).unwrap();
        let setup = &steps.setups[0];
        assert_eq!(setup.setup, doc.setups[0].id);
        assert!(setup.problem.is_none(), "{:?}", setup.problem);
        assert!(!setup.screens.is_empty());
        let named = (setup.screens.iter())
            .flat_map(|s| [&s.picture, &s.numbers])
            .chain(setup.sections.iter().map(|s| &s.head));
        for png in named {
            let name = &png.path;
            assert!(read(name).starts_with(&[0x89, b'P', b'N', b'G']), "{name}");
        }
        // Stored as they are: the tablet copies them out, inflating nothing.
        let first = &setup.screens[0].picture.path;
        let stored = zip.by_name(first).unwrap().compression();
        assert_eq!(stored, zip::CompressionMethod::Stored);
    }
}
