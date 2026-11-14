//! Checkpoints (LIO-470): the work state kept by name, for the agent to go
//! back to (`checkpoint`, `restore_checkpoint`, `list_checkpoints`).
//!
//! A checkpoint holds the work state (the `Document`, every tracked draft
//! and the project's file), the selection with its page and tab, the part's
//! fingerprint (its file is in the `Document`), the revision it was taken at
//! and the project it belongs to: its `Document::project_id`, and how many
//! times the project had been replaced (`Log::projects`). A headless session
//! keeps them in its folder (`checkpoints/`, indexed in `session.json`); the
//! window keeps them in memory. At most `MOST` are kept, the oldest dropped.
//!
//! A restore is validated first and committed once (`engine`):
//! - refused for another project, or with the user's changes since unless
//!   they are to be discarded;
//! - a part other than the one loaded is loaded off the thread as a
//!   *candidate* (`Candidate`), never into the `App`, and must be the
//!   checkpoint's part, or the restore is refused with nothing changed;
//! - in the turn that commits, the user's changes and a running machine
//!   are checked for again; then `App::restore` puts the checkpoint back
//!   through `App::job_replaced`, keeping the machinist's current records,
//!   and works out whether the project is saved by comparing it with its
//!   file by value (`saved_by_value`).
use super::state::DIAL;
use crate::{App, Page, mesh::Mesh, model::Document};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// The most checkpoints kept; the oldest goes when one more is taken.
pub const MOST: usize = 20;

/// The refusal of a checkpoint of another project.
pub const ANOTHER_PROJECT: &str = "Another project was opened or started since this checkpoint, so it can no longer be restored. Take a new checkpoint in the project you are working on.";
/// The refusal when the checkpoint's part file is not that part any more.
pub const MODEL_CHANGED: &str = "The model file changed since the checkpoint; restoring it would put picks on a different part.";

/// The editors' drafts, as the work state tracks them (`WORK_STATE`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Drafts {
    pub machine: crate::model::Machine,
    pub tool: crate::model::Tool,
    pub material: crate::material::Material,
    pub holder: crate::holding::Holder,
    /// A clamp positions field typed but not yet read: its setup, its
    /// text, and the setup's own text it was typed over.
    pub clamps: Option<(String, String, String)>,
    /// A lathe step's measure box, typed: its key and text.
    pub guide_typed: Option<(String, String)>,
}
impl Drafts {
    pub fn of(app: &App) -> Drafts {
        Drafts {
            machine: app.machine_draft.clone(),
            tool: app.tool_draft.clone(),
            material: app.material_draft.clone(),
            holder: app.holder_draft.clone(),
            clamps: (app.clamps_draft.as_ref())
                .map(|c| (c.setup.clone(), c.text.clone(), c.over.clone())),
            guide_typed: app.guide_typed.clone(),
        }
    }
    pub fn put(self, app: &mut App) {
        app.machine_draft = self.machine;
        app.tool_draft = self.tool;
        app.material_draft = self.material;
        app.holder_draft = self.holder;
        app.clamps_draft =
            (self.clamps).map(|(setup, text, over)| crate::ClampsDraft { setup, text, over });
        app.guide_typed = self.guide_typed;
    }
}

/// The selection, by id, with its page and tab.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub setup: Option<String>,
    pub operation: Option<String>,
    pub page: String,
    pub tab: String,
}
/// The pages, by the names results give them.
const PAGES: [Page; 11] = [
    Page::Home,
    Page::Work,
    Page::Machines,
    Page::Control,
    Page::Tools,
    Page::Materials,
    Page::Workholding,
    Page::Wizard,
    Page::Ai,
    Page::Post,
    Page::Audit,
];
impl Place {
    pub fn of(app: &App) -> Place {
        let s = super::engine::Selection::of(app);
        Place {
            setup: s.setup,
            operation: s.operation,
            page: s.page,
            tab: s.tab,
        }
    }
    /// The selection put back where its setup and operation still are; the
    /// page and the tab as they were.
    pub fn put(&self, app: &mut App) {
        if let Some(id) = &self.setup
            && app.doc.setups.iter().any(|s| s.id == *id)
        {
            app.doc.active = id.clone();
        }
        app.selected = match (&self.operation, app.doc.active_setup()) {
            (Some(op), Some(s)) => s.operations.iter().position(|o| o.id == *op),
            _ => None,
        };
        app.setup_end = false;
        if let Some(page) = PAGES.into_iter().find(|p| format!("{p:?}") == self.page) {
            app.page = page;
        }
        if !self.tab.is_empty() {
            app.tab = self.tab.clone();
        }
    }
}

/// A checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub name: String,
    /// When it was taken, s since 1970.
    pub taken: u64,
    /// The revision it was taken at.
    pub revision: u64,
    /// How many times the project had been replaced (`Log::projects`).
    pub projects: u64,
    pub doc: Document,
    pub drafts: Drafts,
    pub project_path: Option<PathBuf>,
    pub place: Place,
    /// The fingerprint of the part loaded (`App::part_mesh`).
    pub model: Option<String>,
}

/// A checkpoint as the index lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub name: String,
    /// The file it is kept in, in the session's `checkpoints/`.
    pub file: String,
    pub taken: u64,
    pub revision: u64,
    pub projects: u64,
    pub project_id: String,
    pub title: String,
}

/// The checkpoints taken: their index, oldest first, and in the window
/// (no session folder) the checkpoints themselves.
#[derive(Debug, Default)]
pub struct Kept {
    pub list: Vec<Meta>,
    memory: HashMap<String, Box<Checkpoint>>,
}
impl Kept {
    /// The index as `session.json` keeps it.
    pub fn from_list(list: Vec<Meta>) -> Kept {
        Kept {
            list,
            memory: HashMap::new(),
        }
    }
    /// Every checkpoint dropped: the agent who took them is gone (the
    /// window's write lease changed hands), so none can restore over
    /// another's work.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn drop_all(&mut self, folder: Option<&Path>) {
        for meta in std::mem::take(&mut self.list) {
            forget(folder, &mut self.memory, &meta);
        }
    }
}
fn forget(folder: Option<&Path>, memory: &mut HashMap<String, Box<Checkpoint>>, meta: &Meta) {
    memory.remove(&meta.file);
    if let Some(folder) = folder {
        let _ = std::fs::remove_file(folder.join(DIR).join(&meta.file));
    }
}
/// Where a session keeps its checkpoints.
const DIR: &str = "checkpoints";

/// `t` as UTC, `YYYY-MM-DDTHH:MM:SSZ`.
pub(crate) fn utc(t: u64) -> String {
    let (days, s) = (t / 86_400, t % 86_400);
    // Days since 1970 to a civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s % 3600 / 60,
        s % 60
    )
}

fn now_s() -> u64 {
    (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)).map_or(0, |d| d.as_secs())
}

/// The work state of `app` taken as checkpoint `name`, kept in `folder`
/// (or in memory without one): one of the same name before is replaced,
/// and the oldest goes past `MOST`. Its index entry.
pub fn take(app: &mut App, name: &str, folder: Option<&Path>) -> Result<Meta, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 120 {
        return Err("Give the checkpoint a name (at most 120 characters).".into());
    }
    if app.doc.file.is_some() && app.imported_mesh.is_none() {
        return Err(
            "The part is not loaded (it is loading, or could not be read), so a checkpoint could not tell it again. Wait for it, or reimport it, then take the checkpoint."
                .into(),
        );
    }
    let checkpoint = Checkpoint {
        name: name.into(),
        taken: now_s(),
        revision: app.mcp.log.revision,
        projects: app.mcp.log.projects,
        doc: app.doc.clone(),
        drafts: Drafts::of(app),
        project_path: app.project_path.clone(),
        place: Place::of(app),
        model: app.part_mesh().map(|m| m.fingerprint.clone()),
    };
    let meta = Meta {
        name: name.into(),
        file: format!("{}.json", crate::model::id()),
        taken: checkpoint.taken,
        revision: checkpoint.revision,
        projects: checkpoint.projects,
        project_id: checkpoint.doc.project_id.clone(),
        title: checkpoint.doc.title.clone(),
    };
    match folder {
        Some(folder) => {
            let data = serde_json::to_vec(&checkpoint).map_err(|e| e.to_string())?;
            crate::storage::write_atomic(&folder.join(DIR).join(&meta.file), &data)
                .map_err(|e| format!("Could not keep the checkpoint: {e}"))?;
        }
        None => {
            let kept = &mut app.mcp.checkpoints;
            kept.memory.insert(meta.file.clone(), Box::new(checkpoint));
        }
    }
    let kept = &mut app.mcp.checkpoints;
    let (same, others): (Vec<Meta>, Vec<Meta>) = std::mem::take(&mut kept.list)
        .into_iter()
        .partition(|m| m.name == meta.name);
    kept.list = others;
    for old in &same {
        forget(folder, &mut kept.memory, old);
    }
    kept.list.push(meta.clone());
    while kept.list.len() > MOST {
        let oldest = kept.list.remove(0);
        forget(folder, &mut kept.memory, &oldest);
    }
    Ok(meta)
}

/// Checkpoint `name`, read back.
pub fn load(app: &App, name: &str, folder: Option<&Path>) -> Result<Checkpoint, String> {
    let kept = &app.mcp.checkpoints;
    let Some(meta) = kept.list.iter().find(|m| m.name == name) else {
        let names: Vec<&str> = kept.list.iter().map(|m| m.name.as_str()).collect();
        return Err(match names.is_empty() {
            true => format!("There is no checkpoint \"{name}\": none was taken."),
            false => format!(
                "There is no checkpoint \"{name}\". There are: {}.",
                names.join(", ")
            ),
        });
    };
    match folder {
        Some(folder) => {
            let path = folder.join(DIR).join(&meta.file);
            let bytes = std::fs::read(&path)
                .map_err(|e| format!("Checkpoint \"{name}\" could not be read: {e}"))?;
            serde_json::from_slice(&bytes)
                .map_err(|e| format!("Checkpoint \"{name}\" could not be read: {e}"))
        }
        None => (kept.memory.get(&meta.file))
            .map(|c| (**c).clone())
            .ok_or_else(|| format!("Checkpoint \"{name}\" is gone.")),
    }
}

/// Why a checkpoint of `projects` and `project_id` cannot be restored in
/// the project `app` has: it was replaced since, or is another.
pub fn other_project(app: &App, projects: u64, project_id: &str) -> Option<&'static str> {
    (app.mcp.log.projects != projects || app.doc.project_id != project_id)
        .then_some(ANOTHER_PROJECT)
}

/// Whether the part a checkpoint of part `model` restores must be loaded:
/// its file's part is not the one loaded now.
pub fn needs_part(app: &App, checkpoint: &Checkpoint) -> bool {
    checkpoint.doc.file.is_some()
        && app.imported_mesh.as_ref().map(|m| &m.fingerprint) != checkpoint.model.as_ref()
}

/// A checkpoint's part, being loaded off the `App`'s thread: its file,
/// which must still be the part the checkpoint was taken with.
#[derive(Clone, Default)]
pub struct Candidate(Arc<Mutex<Option<Result<Mesh, String>>>>);
impl Candidate {
    /// Starts loading `checkpoint`'s part (`mesh::load_file`, as the app
    /// restores a project's part), after `slow` (tests).
    pub fn load(checkpoint: &Checkpoint, slow: Option<std::time::Duration>) -> Candidate {
        let slot = Candidate::default();
        let into = slot.0.clone();
        let (file, model) = (checkpoint.doc.file.clone(), checkpoint.model.clone());
        let spawned = std::thread::Builder::new()
            .name("mcp-checkpoint-part".into())
            .spawn(move || {
                if let Some(slow) = slow {
                    std::thread::sleep(slow);
                }
                let loaded = match (&file, &model) {
                    (Some(file), Some(model)) => match crate::mesh::load_file(file) {
                        Ok(mesh) if mesh.fingerprint == *model => Ok(mesh),
                        Ok(_) => Err(MODEL_CHANGED.to_string()),
                        Err(e) => Err(format!(
                            "The model file {} could not be read ({e}); restoring the checkpoint would put picks on a part that is not there.",
                            file.path.display()
                        )),
                    },
                    _ => Err(MODEL_CHANGED.to_string()),
                };
                if let Ok(mut slot) = into.lock() {
                    *slot = Some(loaded);
                }
            });
        if let Err(e) = spawned
            && let Ok(mut s) = slot.0.lock()
        {
            *s = Some(Err(format!("The part could not be loaded: {e}")));
        }
        slot
    }
    /// The part, once it is loaded or could not be.
    pub fn take(&self) -> Option<Result<Mesh, String>> {
        self.0.lock().ok()?.take()
    }
}

/// Whether `doc` is what the project file at `path` holds: both as
/// `storage::save` would write them, compared by value, the file read with
/// `storage::load`. A file saved before projects had an identity is taken
/// to be this project's. With no file, or one that cannot be read, no.
pub fn saved_by_value(doc: &Document, path: Option<&Path>) -> bool {
    let Some(path) = path else {
        return false;
    };
    let Ok(mut on_disk) = crate::storage::load(path) else {
        return false;
    };
    if on_disk.project_id.is_empty() {
        on_disk.project_id = doc.project_id.clone();
    }
    match (serde_json::to_value(doc), serde_json::to_value(&on_disk)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

impl App {
    /// Puts `checkpoint` back, in one turn (`engine` checked it may be):
    /// the job through `job_replaced`, with the machinist's records as they
    /// are now (the step guide's places, the measurements and the dial
    /// readings), its part `mesh` (the one loaded when it is the same
    /// part); then its drafts, selection, page, tab and project file.
    /// Saved or not is worked out against the file (`saved_by_value`).
    pub(crate) fn restore(&mut self, checkpoint: &Checkpoint, mesh: Option<Mesh>) {
        let mut doc = checkpoint.doc.clone();
        keep_records(&self.doc, &mut doc);
        let mesh = match mesh {
            Some(mesh) => Some(mesh),
            None if doc.file.is_some() => self.imported_mesh.take(),
            None => None,
        };
        self.job_replaced(doc, mesh);
        checkpoint.drafts.clone().put(self);
        self.stop_selection();
        checkpoint.place.put(self);
        self.set_project_file(checkpoint.project_path.clone());
        self.project_dirty = !saved_by_value(&self.doc, self.project_path.as_deref());
        // Saved with the project's file just now: it settles after a pause.
        self.dirty = true;
        self.last_edit = std::time::Instant::now();
        self.status = format!(
            "Checkpoint \"{}\" restored{}.",
            checkpoint.name,
            if self.project_dirty {
                ""
            } else {
                "; the project is as its file holds it"
            }
        );
    }
}

/// The machinist's records of `now` in `doc` (`state::Records`): the step
/// guide's place and the measurements of each setup `doc` has, and the
/// cross-slide dial readings of its library machines and its setups'
/// machines, whatever `doc` held.
fn keep_records(now: &Document, doc: &mut Document) {
    let setups: std::collections::BTreeSet<String> =
        doc.setups.iter().map(|s| s.id.clone()).collect();
    let setup_of = |key: &str| key.split_once('/').map_or(key, |(s, _)| s).to_string();
    doc.guide = (now.guide.iter())
        .filter(|(id, _)| setups.contains(*id))
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    doc.measured = (now.measured.iter())
        .filter(|(key, _)| setups.contains(&setup_of(key)))
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    let library = |id: &str| {
        (now.machines.iter())
            .find(|m| m.id == id)
            .and_then(|m| m.p.get(DIAL).cloned())
    };
    let set = |m: &mut crate::model::Machine, dial: Option<String>| match dial {
        Some(d) => {
            m.p.insert(DIAL.into(), d);
        }
        None => {
            m.p.remove(DIAL);
        }
    };
    for m in &mut doc.machines {
        if now.machines.iter().any(|x| x.id == m.id) {
            let dial = library(&m.id);
            set(m, dial);
        }
    }
    for s in &mut doc.setups {
        let current = (now.setups.iter()).find(|x| x.id == s.id && x.machine.id == s.machine.id);
        let dial = match current {
            Some(x) => x.machine.p.get(DIAL).cloned(),
            None => library(&s.machine.id),
        };
        set(&mut s.machine, dial);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::commands::tests::{Direct, call, direct, run};
    use crate::mcp::engine::{Agent, Answer};
    use crate::{Message, Target};
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    type Slot = Arc<Mutex<Option<Answer>>>;
    /// A tool call whose answer may come later (`settle`).
    fn begin(d: &mut Direct, tool: &str, args: Value) -> Slot {
        let slot: Slot = Arc::default();
        let into = slot.clone();
        let mut agent: Agent = d.agent.take().unwrap();
        agent.call(
            d,
            tool,
            args,
            Box::new(move |a| *into.lock().unwrap() = Some(a)),
        );
        d.agent = Some(agent);
        slot
    }
    /// Polls the agent until `slot` has its answer.
    fn settle(d: &mut Direct, slot: &Slot) -> Value {
        let end = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(a) = slot.lock().unwrap().take() {
                return a.unwrap();
            }
            assert!(
                Instant::now() < end,
                "no answer; pending {:?}",
                d.app.pending()
            );
            let mut agent = d.agent.take().unwrap();
            agent.poll(d);
            d.agent = Some(agent);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn revision(v: &Value) -> u64 {
        v["revision"].as_u64().unwrap()
    }
    fn inspect(d: &mut Direct) -> Value {
        call(d, "inspect", json!({})).unwrap()
    }
    fn restore(d: &mut Direct, name: &str) -> Value {
        let slot = begin(d, "restore_checkpoint", json!({"name": name}));
        settle(d, &slot)
    }
    fn take(d: &mut Direct, name: &str) -> Value {
        let r = call(d, "checkpoint", json!({"name": name})).unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
        r
    }

    #[test]
    fn a_project_keeps_its_identity_and_an_old_file_gets_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.axis.json");
        let mut doc = Document::sample();
        doc.adopt_materials();
        doc.adopt_holders();
        assert!(!doc.project_id.is_empty());
        assert_ne!(doc.project_id, Document::sample().project_id);
        crate::storage::save(&path, &doc).unwrap();
        assert_eq!(
            crate::storage::load(&path).unwrap().project_id,
            doc.project_id
        );
        // A file from before: it loads, with none, and is given one.
        let mut old = serde_json::to_value(&doc).unwrap();
        old.as_object_mut().unwrap().remove("project_id");
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        let mut loaded = crate::storage::load(&path).unwrap();
        assert!(loaded.project_id.is_empty());
        assert!(saved_by_value(&loaded, Some(&path)));
        assert!(loaded.own_id() && !loaded.project_id.is_empty());
        assert!(!loaded.own_id());
        // An old workspace gets one at the start, and saves it.
        let workspace = dir.path().join("workspace.json");
        std::fs::write(&workspace, serde_json::to_vec(&old).unwrap()).unwrap();
        let app = App::from_workspace(workspace.clone(), None);
        assert!(!app.doc.project_id.is_empty());
        assert!(app.dirty, "saved with the next persist");
        // A project opened from such a file gets one.
        let mut d = direct();
        let opened = json!({"path": path.display().to_string(), "discard_unsaved": true});
        assert_eq!(
            run(&mut d, "open_project", opened).unwrap()["outcome"],
            "done"
        );
        assert!(!d.app.doc.project_id.is_empty());
    }

    /// Save A, checkpoint, edit and save B, restore: A is shown not saved
    /// (the disk holds B), at a new revision, and an old if_revision is a
    /// conflict. A restore over its own file is saved.
    #[test]
    fn a_restore_works_out_saved_against_the_file() {
        let mut d = direct();
        let path = d.folder.join("job.axis.json");
        d.app.doc.title = "A".into();
        let save = json!({"path": path.display().to_string()});
        assert_eq!(
            run(&mut d, "save_project", save).unwrap()["outcome"],
            "done"
        );
        let taken = take(&mut d, "a");
        let at_a = revision(&taken);
        assert_eq!(taken["checkpoint"]["revision"], at_a);
        let same = restore(&mut d, "a");
        assert_eq!(same["outcome"], "done", "{same}");
        assert!(!d.app.project_dirty, "{same}");
        assert_eq!(same["session"]["saved_to_project_file"], true, "{same}");
        let title = json!({"title": "B"});
        assert_eq!(
            run(&mut d, "set_project_title", title).unwrap()["outcome"],
            "done"
        );
        let saved = run(&mut d, "save_project", json!({})).unwrap();
        assert_eq!(saved["outcome"], "done", "{saved}");
        let before = revision(&saved);
        let back = restore(&mut d, "a");
        assert_eq!(back["outcome"], "done", "{back}");
        assert_eq!(d.app.doc.title, "A");
        assert!(d.app.project_dirty, "the file holds B");
        assert_eq!(d.app.project_path.as_ref(), Some(&path));
        assert!(revision(&back) > before, "{back}");
        let entry = d.app.mcp.log.entries.last().unwrap().clone();
        assert_eq!(entry.source, crate::mcp::state::Source::Agent);
        assert!(
            entry
                .note
                .as_deref()
                .unwrap()
                .contains("checkpoint restored")
        );
        let read = inspect(&mut d);
        assert_eq!(read["project"]["dirty"], true, "{read}");
        let stale =
            json!({"command": "set_project_title", "args": {"title": "C"}, "if_revision": before});
        let r = call(&mut d, "execute", stale).unwrap();
        assert_eq!(r["outcome"], "conflict", "{r}");
        assert_eq!(d.app.doc.title, "A");
        // Listed, restorable.
        let list = call(&mut d, "list_checkpoints", json!({})).unwrap();
        assert_eq!(list["checkpoints"][0]["name"], "a", "{list}");
        assert_eq!(list["checkpoints"][0]["restorable"], true, "{list}");
        let missing = restore(&mut d, "nope");
        assert_eq!(missing["outcome"], "refused", "{missing}");
    }

    /// The fixture block, imported, its Face alone and selected.
    fn block() -> Direct {
        let mut d = direct();
        let (mut doc, mesh) = crate::post::fixture::job();
        doc.setups[0].operations.truncate(1);
        doc.adopt_materials();
        doc.adopt_holders();
        let aluminium = (doc.materials.iter())
            .find(|m| m.name() == "Aluminium 6061-T6")
            .map(|m| m.id.clone())
            .unwrap();
        assert!(doc.select_material(&aluminium));
        d.app.replace_document(doc, None);
        d.app.imported_mesh = Some(mesh);
        d.app.selected = Some(0);
        d.app.dirty = false;
        d
    }

    /// Checkpoint A; B on the same part and ids, generated, audited and NC
    /// reviewed; restored: none of B's results is shown, and a generation
    /// of B that comes back late is dropped.
    #[test]
    fn nothing_worked_out_for_the_job_after_the_checkpoint_survives_it() {
        let mut d = block();
        inspect(&mut d);
        take(&mut d, "a");
        let a = serde_json::to_value(&d.app.doc.setups[0].operations[0]).unwrap();
        let feed = json!({"target": "operation", "key": "feed", "value": "777"});
        assert_eq!(run(&mut d, "edit_field", feed).unwrap()["outcome"], "done");
        let g = run(&mut d, "generate", json!({"operation": 0})).unwrap();
        assert_eq!(g["outcome"], "done", "{g}");
        assert!(d.app.simulation.is_some());
        let all = run(&mut d, "generate_all", json!({})).unwrap();
        assert_eq!(all["outcome"], "done", "{all}");
        assert!(!d.app.audit_report.is_empty());
        let post = run(&mut d, "goto_page", json!({"page": "Post"})).unwrap();
        assert_eq!(post["outcome"], "done", "{post}");
        assert!(d.app.post.is_some(), "the NC review ran");
        // The user's generation of B, under way when the restore commits.
        d.app.generate_inline = false;
        d.app.selected = Some(0);
        d.app.page = crate::Page::Work;
        let late = d.app.update(Message::Generate);
        assert!(d.app.generating.is_some(), "{}", d.app.status);
        let back = restore(&mut d, "a");
        assert_eq!(back["outcome"], "done", "{back}");
        assert!(d.app.generating.is_none());
        for m in d.outputs(late) {
            let _ = d.app.update(m);
        }
        assert!(d.app.simulation.is_none(), "B's generation was applied");
        assert!(d.app.audit_report.is_empty());
        assert!(d.app.post.is_none());
        assert_eq!(d.app.page, crate::Page::Work, "the checkpoint's page");
        let now = serde_json::to_value(&d.app.doc.setups[0].operations[0]).unwrap();
        assert_eq!(now, a);
        // A's are worked out afresh.
        d.app.generate_inline = true;
        let g = run(&mut d, "generate", json!({"operation": 0})).unwrap();
        assert_eq!(g["outcome"], "done", "{g}");
    }

    /// `pocket::chamfered_hole` as STL text, moved `dx` along X.
    fn hole_stl(dx: f32) -> String {
        let mut text = String::from("solid hole\n");
        for t in crate::pocket::chamfered_hole() {
            text += "facet normal 0 0 0\nouter loop\n";
            for p in t {
                text += &format!("vertex {} {} {}\n", p[0] + dx, p[1], p[2]);
            }
            text += "endloop\nendfacet\n";
        }
        text + "endsolid hole\n"
    }
    /// The sample's setup on `hole.stl` (A), imported, with a Pocket.
    fn hole_job() -> (Direct, PathBuf) {
        let mut d = direct();
        let stl = d.folder.join("hole.stl");
        let text = hole_stl(0.);
        std::fs::write(&stl, &text).unwrap();
        let mut doc = Document::sample();
        doc.adopt_materials();
        doc.adopt_holders();
        doc.sample_model = false;
        doc.file = Some(crate::model::ModelFile {
            name: "hole.stl".into(),
            path: stl.clone(),
            format: "stl".into(),
            bytes: text.len() as u64,
            thickness: None,
        });
        let s = &mut doc.setups[0];
        s.operations.truncate(1);
        s.operations[0] = crate::model::Operation::new(
            crate::model::Strategy::Pocket,
            s.operations[0].tool.clone(),
        );
        d.app.replace_document(doc, None);
        d.app.imported_mesh = Some(crate::mesh::load(&stl).unwrap());
        d.app.selected = Some(0);
        d.app.dirty = false;
        inspect(&mut d);
        (d, stl)
    }
    /// The part file changed to B (moved 5 mm) and reimported.
    fn reimport_b(d: &mut Direct, stl: &Path) {
        std::fs::write(stl, hole_stl(5.)).unwrap();
        let r = run(d, "reimport_model", json!({})).unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
    }

    /// After a reimport between the checkpoint and the restore (the file
    /// put back to A's part), the part and the picks are A's.
    #[test]
    fn a_restore_after_a_reimport_brings_back_the_part_and_its_picks() {
        let (mut d, stl) = hole_job();
        let mesh = d.app.imported_mesh.clone().unwrap();
        let holes = crate::pocket::part_holes(&d.app.doc.setups[0], &mesh);
        let hole = holes.iter().find(|h| h.walls.len() == 24).unwrap();
        let picks = json!({"operation": 0, "ids": [hole.walls[0]], "mode": "add"});
        let picked = run(&mut d, "pick_surfaces", picks).unwrap();
        assert_eq!(picked["outcome"], "done", "{picked}");
        let a = serde_json::to_value(&d.app.doc.setups[0]).unwrap();
        let a_places = serde_json::to_value(&d.app.doc.places).unwrap();
        let fingerprint = mesh.fingerprint.clone();
        take(&mut d, "a");
        reimport_b(&mut d, &stl);
        assert_ne!(
            d.app.imported_mesh.as_ref().unwrap().fingerprint,
            fingerprint
        );
        std::fs::write(&stl, hole_stl(0.)).unwrap();
        let back = restore(&mut d, "a");
        assert_eq!(back["outcome"], "done", "{back}");
        assert_eq!(
            d.app.imported_mesh.as_ref().unwrap().fingerprint,
            fingerprint
        );
        assert_eq!(d.app.part_mesh().unwrap().fingerprint, fingerprint);
        assert_eq!(serde_json::to_value(&d.app.doc.setups[0]).unwrap(), a);
        assert_eq!(serde_json::to_value(&d.app.doc.places).unwrap(), a_places);
    }

    /// A restore whose part file was changed or deleted is refused, with
    /// the job, the part and the revision as they were.
    #[test]
    fn a_restore_whose_part_file_changed_or_went_is_refused() {
        let (mut d, stl) = hole_job();
        take(&mut d, "a");
        reimport_b(&mut d, &stl);
        for gone in [false, true] {
            if gone {
                std::fs::remove_file(&stl).unwrap();
            }
            let before = serde_json::to_value(&d.app.doc).unwrap();
            let part = d.app.imported_mesh.as_ref().unwrap().fingerprint.clone();
            let at = inspect(&mut d)["revision"].clone();
            let r = restore(&mut d, "a");
            assert_eq!(r["outcome"], "refused", "{r}");
            let why = r["reason"].as_str().unwrap();
            match gone {
                false => assert_eq!(why, MODEL_CHANGED),
                true => assert!(why.contains("could not be read"), "{why}"),
            }
            assert_eq!(serde_json::to_value(&d.app.doc).unwrap(), before);
            assert_eq!(d.app.imported_mesh.as_ref().unwrap().fingerprint, part);
            assert_eq!(r["revision"], at, "{r}");
        }
    }

    /// While the checkpoint's part loads slowly, the user edits the tool
    /// draft: the restore is refused and the edit kept. Then a run started
    /// while it loads refuses it too.
    #[test]
    fn a_restore_checks_again_in_the_turn_that_commits() {
        let (mut d, stl) = hole_job();
        // The machine connected and still, before the checkpoint.
        crate::mcp::reads::tests::jogging(&mut d);
        crate::mcp::reads::tests::stop(&mut d);
        inspect(&mut d);
        take(&mut d, "a");
        reimport_b(&mut d, &stl);
        std::fs::write(&stl, hole_stl(0.)).unwrap();
        d.app.mcp.slow_candidate = Some(Duration::from_millis(600));
        let b_part = d.app.imported_mesh.as_ref().unwrap().fingerprint.clone();

        let slot = begin(&mut d, "restore_checkpoint", json!({"name": "a"}));
        assert!(slot.lock().unwrap().is_none(), "it waits for the part");
        // Busy meanwhile.
        let busy = run(&mut d, "set_project_title", json!({"title": "X"})).unwrap();
        assert_eq!(busy["outcome"], "busy", "{busy}");
        let _ = d.app.update(Message::Edit(
            Target::Tool,
            "name",
            "Typed meanwhile".into(),
        ));
        let r = settle(&mut d, &slot);
        assert_eq!(r["outcome"], "refused", "{r}");
        assert!(
            r["reason"]
                .as_str()
                .unwrap()
                .contains("user: edit_field tool name"),
            "{r}"
        );
        assert_eq!(
            crate::model::get(&d.app.tool_draft.p, "name"),
            "Typed meanwhile"
        );
        assert_eq!(d.app.imported_mesh.as_ref().unwrap().fingerprint, b_part);
        let theirs = r["user_changed"].as_array().unwrap();
        assert!(!theirs.is_empty(), "{r}");

        // The user's changes since need discarding; a run during the load
        // refuses it all the same.
        let slot = begin(
            &mut d,
            "restore_checkpoint",
            json!({"name": "a", "discard_user_changes": true}),
        );
        crate::mcp::reads::tests::jogging(&mut d);
        let r = settle(&mut d, &slot);
        assert_eq!(r["outcome"], "refused", "{r}");
        assert!(
            r["reason"].as_str().unwrap().contains("machine is running"),
            "{r}"
        );
        assert_eq!(d.app.imported_mesh.as_ref().unwrap().fingerprint, b_part);
        crate::mcp::reads::tests::stop(&mut d);
        // Still, it goes.
        let again = json!({"name": "a", "discard_user_changes": true});
        let slot = begin(&mut d, "restore_checkpoint", again);
        let r = settle(&mut d, &slot);
        assert_eq!(r["outcome"], "done", "{r}");
        assert_ne!(d.app.imported_mesh.as_ref().unwrap().fingerprint, b_part);
        d.app.disconnect_machine();
    }

    /// A checkpoint of A, then a new project: refused, the new project
    /// unchanged; A opened again is another replacement, still refused. A
    /// checkpoint taken in it goes back within it.
    #[test]
    fn a_checkpoint_of_another_project_is_refused() {
        let mut d = direct();
        let path = d.folder.join("a.axis.json");
        let first = d.app.doc.setups[0].id.clone();
        d.app.doc.guide.insert(first, 7);
        let save = json!({"path": path.display().to_string()});
        assert_eq!(
            run(&mut d, "save_project", save).unwrap()["outcome"],
            "done"
        );
        take(&mut d, "a");
        assert_eq!(
            run(&mut d, "new_project", json!({})).unwrap()["outcome"],
            "done"
        );
        let blank = serde_json::to_value(&d.app.doc).unwrap();
        let r = restore(&mut d, "a");
        assert_eq!(r["outcome"], "refused", "{r}");
        assert_eq!(r["reason"], ANOTHER_PROJECT);
        assert_eq!(serde_json::to_value(&d.app.doc).unwrap(), blank);
        let open = json!({"path": path.display().to_string(), "discard_unsaved": true});
        assert_eq!(
            run(&mut d, "open_project", open).unwrap()["outcome"],
            "done"
        );
        assert_eq!(d.app.doc.guide.values().next(), Some(&7));
        let r = restore(&mut d, "a");
        assert_eq!(r["reason"], ANOTHER_PROJECT, "{r}");
        let list = call(&mut d, "list_checkpoints", json!({})).unwrap();
        assert_eq!(list["checkpoints"][0]["restorable"], false, "{list}");
        take(&mut d, "a again");
        let t = json!({"title": "Changed"});
        assert_eq!(
            run(&mut d, "set_project_title", t).unwrap()["outcome"],
            "done"
        );
        let r = restore(&mut d, "a again");
        assert_eq!(r["outcome"], "done", "{r}");
        assert_eq!(d.app.doc.guide.values().next(), Some(&7));
    }

    #[test]
    fn times_are_said_in_utc() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(951_825_599), "2000-02-29T11:59:59Z");
        assert_eq!(utc(1_794_614_400), "2026-11-14T00:00:00Z");
    }

    /// At most `MOST` are kept, the oldest dropped with its file; the same
    /// name replaces.
    #[test]
    fn the_oldest_checkpoint_goes_past_twenty() {
        let mut d = direct();
        for n in 0..=MOST {
            take(&mut d, &format!("c{n}"));
        }
        take(&mut d, "c5");
        let names: Vec<&str> = (d.app.mcp.checkpoints.list.iter())
            .map(|m| m.name.as_str())
            .collect();
        assert_eq!(names.len(), MOST);
        assert_eq!(names[0], "c1");
        assert_eq!(names[MOST - 1], "c5");
        let files = || std::fs::read_dir(d.session.join(DIR)).unwrap().count();
        assert_eq!(files(), MOST);
        // The hook for a write lease that changes hands.
        let folder = d.session.clone();
        d.app.mcp.checkpoints.drop_all(Some(&folder));
        assert!(d.app.mcp.checkpoints.list.is_empty());
        assert_eq!(files(), 0);
    }
}
