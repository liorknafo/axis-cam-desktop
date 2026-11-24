//! What changed, and who changed it (LIO-470): the work state, its
//! snapshots, the revisions and their change log, the external events, and
//! the machinist's records an agent's command may not change.
//!
//! - **The work state** is everything the user could lose or see change:
//!   the `Document`, every editable draft, the project file's path and the
//!   loaded model's fingerprint (`WORK_STATE`). A test holds every field of
//!   `App` to `WORK_STATE` or `NOT_WORK_STATE`, so a new draft is not missed.
//! - **A snapshot** hashes the work state's saved serialization, streamed
//!   into the hasher with no string built, item by item: each setup,
//!   operation, library entry, shop-floor record and draft, so the change log
//!   says what changed. It is taken only at the edges of an agent's
//!   ownership window (`engine`), and by a read while no window is open.
//! - **The revision** goes up by one each time a snapshot differs from the
//!   one before, and each such step is a change-log `Entry` with its source:
//!   `agent` (found when a window closed, with no external event in it),
//!   `mixed` (the same, with external events, which it names) or `user`
//!   (found when a window opened, or by a read outside one).
//! - **External events** are recorded where they happen (`App::external`):
//!   a message from the window's own handlers that is not view-only
//!   (`App::mcp_arrive`, which the engine's own dispatches skip:
//!   `App::update_by_agent`), a controller result written into the work
//!   state, and the completion of a generation the user started.
//! - **The machinist's records** (`Records`): the step guide's place, the
//!   measurements and the cross-slide dial readings. An agent's command
//!   that changes them with no external event in its window has them put
//!   back and fails.
use crate::{
    App, Message,
    model::{Document, Setup},
};
use iced::Task;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, hash_map::DefaultHasher},
    hash::Hasher,
    io::Write,
};

/// The fields of `App` that are the work state, with what each holds.
/// The test over `App`'s fields reads them.
#[cfg_attr(not(test), allow(dead_code))]
pub const WORK_STATE: &[(&str, &str)] = &[
    (
        "doc",
        "the job, the libraries, the guide's saved place and the measurements",
    ),
    ("machine_draft", "the machine editor's draft"),
    ("tool_draft", "the tool editor's draft"),
    ("material_draft", "the material editor's draft"),
    ("holder_draft", "the workholding editor's draft"),
    (
        "clamps_draft",
        "a clamp positions field typed but not yet read",
    ),
    ("guide_typed", "a lathe step's measure box, typed"),
    ("project_path", "the project file"),
    (
        "imported_mesh",
        "the imported part, by its fingerprint (`part_mesh`)",
    ),
    (
        "sample_mesh",
        "the sample's part, by its fingerprint (`part_mesh`)",
    ),
];

#[cfg_attr(not(test), allow(dead_code))]
const VIEW: &str = "how the job is shown, not the job";
#[cfg_attr(not(test), allow(dead_code))]
const SELECTION: &str = "the selection, page and tab, which every result reports apart";
#[cfg_attr(not(test), allow(dead_code))]
const STATUS: &str = "what the app last said";
#[cfg_attr(not(test), allow(dead_code))]
const WORKED: &str = "worked out from the job, and worked out again when it changes";
#[cfg_attr(not(test), allow(dead_code))]
const JOB: &str = "background work under way, not its result";
#[cfg_attr(not(test), allow(dead_code))]
const ACCOUNT: &str = "the AI planner's account, removed in M4";
#[cfg_attr(not(test), allow(dead_code))]
const PLANNER: &str = "the AI planner's, removed in M4";
#[cfg_attr(not(test), allow(dead_code))]
const CONTROLLER: &str = "the machine controller's: the agent cannot touch it";
#[cfg_attr(not(test), allow(dead_code))]
const WINDOW: &str = "the window's own";
#[cfg_attr(not(test), allow(dead_code))]
const SMOKE: &str = "the renderer smoke run's";
#[cfg_attr(not(test), allow(dead_code))]
const FLAG: &str = "a flag of the work's progress, not the work";
/// The shop tablet's pairing and its own state (LIO-459, LIO-485): the
/// agent cannot touch them, and the tablet build serves no MCP.
const DEVICE: &str =
    "pairing with the shop tablet, or the tablet build's own: the agent cannot touch it";

/// The fields of `App` that are not the work state, and why.
#[cfg_attr(not(test), allow(dead_code))]
pub const NOT_WORK_STATE: &[(&str, &str)] = &[
    ("smoke_dir", SMOKE),
    ("smoke_step", SMOKE),
    ("help_animating", WINDOW),
    ("smoke_pending", SMOKE),
    ("smoke_frames", SMOKE),
    ("smoke_changed", SMOKE),
    ("smoke_resize", SMOKE),
    ("smoke_window", SMOKE),
    ("smoke_last", SMOKE),
    ("page", SELECTION),
    ("cam_page", SELECTION),
    ("selected", SELECTION),
    ("setup_end", SELECTION),
    ("tab", SELECTION),
    ("setup_step", SELECTION),
    ("status", STATUS),
    (
        "typed_last",
        "the last edit, kept to undo it; the edit itself is in the job",
    ),
    ("status_page", STATUS),
    ("status_problem", STATUS),
    ("audit_report", WORKED),
    ("pinned_help", VIEW),
    ("pinned_expanded", VIEW),
    ("pinned_paused", VIEW),
    ("post", WORKED),
    ("post_busy", JOB),
    ("left_stock", WORKED),
    ("holding_checks", WORKED),
    ("left_job", JOB),
    ("guide", WORKED),
    ("guide_job", JOB),
    (
        "guide_item",
        "the readout the Steps page is at within its step: not saved",
    ),
    (
        "guide_reached",
        "the hole the Steps page is at within its step: not saved",
    ),
    ("steps_entry", VIEW),
    ("steps_pending", JOB),
    ("guide_clock", VIEW),
    ("ready", WORKED),
    ("ready_job", JOB),
    ("holding_note", WORKED),
    ("drill_holes", WORKED),
    ("bottom_holes", WORKED),
    ("pocket_holes", WORKED),
    ("later_holes", WORKED),
    ("later_drills", WORKED),
    ("part_holes", WORKED),
    ("part_holes_job", JOB),
    ("pocket_fits", WORKED),
    ("scrub_pending", VIEW),
    ("scrub_live", VIEW),
    ("catching_up", JOB),
    ("last_scrub", VIEW),
    ("catch_up_epoch", JOB),
    ("generating", JOB),
    ("generation_epoch", JOB),
    ("generate_inline", "how the app runs its work"),
    ("tooth_load_job", JOB),
    ("stock_pictures", VIEW),
    ("post_epoch", JOB),
    (
        "post_for",
        "the job the NC review's check is of, worked out with it",
    ),
    ("picture_job", JOB),
    (
        "settings",
        "the app's settings (sidebars, last project): not the job",
    ),
    ("settings_path", "where the settings are kept"),
    ("account", ACCOUNT),
    ("account_note", ACCOUNT),
    ("account_busy", ACCOUNT),
    ("settings_failure", ACCOUNT),
    ("sign_out_pending", ACCOUNT),
    ("restoring", ACCOUNT),
    ("account_epoch", ACCOUNT),
    ("usage_limit", ACCOUNT),
    ("models", ACCOUNT),
    ("endpoints", ACCOUNT),
    ("store", ACCOUNT),
    ("client", ACCOUNT),
    ("sign_in", ACCOUNT),
    ("sign_in_attempt", ACCOUNT),
    ("open_browser", ACCOUNT),
    ("account_restored", ACCOUNT),
    ("wizard_step", SELECTION),
    (
        "clamp_refusal",
        "why Suggest layout placed no clamps: said, not kept",
    ),
    ("camera", VIEW),
    ("window_width", WINDOW),
    ("window_height", WINDOW),
    ("import_busy", JOB),
    ("save_busy", JOB),
    (
        "last_archived",
        "where the last project went in Project history",
    ),
    ("import_error", STATUS),
    ("import_path_open", VIEW),
    (
        "import_path",
        "the Import from path field: an import's path, not the job",
    ),
    ("import_path_error", STATUS),
    ("stock", VIEW),
    ("show_uncut", VIEW),
    ("sim_details", VIEW),
    ("warning_details", VIEW),
    ("section", VIEW),
    ("section_on", VIEW),
    ("picking_section", VIEW),
    ("paths", VIEW),
    ("picking", VIEW),
    ("box_refused", STATUS),
    ("fresh", STATUS),
    ("picking_pins", VIEW),
    ("placing_pins", VIEW),
    ("pin_aim", VIEW),
    ("playing", VIEW),
    ("simulation", WORKED),
    ("stock_detail", VIEW),
    ("stock_mesh_job", JOB),
    ("stock_mesh_started", JOB),
    ("stock_mesh_slow", JOB),
    ("stock_mesh_failed", JOB),
    ("start_mesh", WORKED),
    ("start_mesh_job", JOB),
    ("start_mesh_failed", JOB),
    ("run_mesh_job", JOB),
    ("run_mesh_failed", JOB),
    ("run_mesh_started", JOB),
    ("run_mesh_waits", JOB),
    ("progress", VIEW),
    ("speed", VIEW),
    ("last_tick", WINDOW),
    ("dirty", FLAG),
    ("save_error", STATUS),
    ("workspace_save", JOB),
    ("last_edit", FLAG),
    ("state_path", "where the workspace is kept"),
    ("control", CONTROLLER),
    ("machine_setup", VIEW),
    ("control_tab", VIEW),
    ("control_phase", CONTROLLER),
    ("control_view", VIEW),
    ("profile_draft", CONTROLLER),
    ("control_machine", CONTROLLER),
    ("switch_machine", CONTROLLER),
    ("pi_code", CONTROLLER),
    ("opened_notes", STATUS),
    ("tablet_code", DEVICE),
    ("tablet_firewall", DEVICE),
    ("shop_projects", DEVICE),
    ("computer_address", DEVICE),
    ("computer_code", DEVICE),
    ("computer", DEVICE),
    ("kept", DEVICE),
    ("computer_busy", DEVICE),
    ("sync_note", DEVICE),
    ("pi_pairing", CONTROLLER),
    ("pi_run", CONTROLLER),
    ("cnc_over_guide", VIEW),
    ("run_on_connect", CONTROLLER),
    ("run_cuts", CONTROLLER),
    ("run_away", CONTROLLER),
    ("cut_journals", CONTROLLER),
    ("others_unknown", CONTROLLER),
    ("cuts_path", CONTROLLER),
    ("cuts_kept", CONTROLLER),
    ("cuts_failed", CONTROLLER),
    ("cuts_seen", CONTROLLER),
    ("cuts_grew", CONTROLLER),
    ("journal_changes_seen", CONTROLLER),
    ("run_pending", CONTROLLER),
    ("run_declared", CONTROLLER),
    ("jog_key", CONTROLLER),
    ("run_epoch", JOB),
    ("shift", WINDOW),
    ("project_dirty", FLAG),
    ("planner", PLANNER),
    ("plan_run", PLANNER),
    ("plan_epoch", PLANNER),
    ("plan_account_epoch", PLANNER),
    ("plan_model", PLANNER),
    ("plan_timeline", PLANNER),
    ("plan_stream", PLANNER),
    ("plan_overview", PLANNER),
    ("plan_enlarged", PLANNER),
    ("plan_expanded", PLANNER),
    ("plan_busy", PLANNER),
    ("plan_dropped", PLANNER),
    ("plan_camera", PLANNER),
    ("plan_request", PLANNER),
    ("plan_chat", PLANNER),
    ("plan_asked", PLANNER),
    ("plan_refined", PLANNER),
    ("undo", PLANNER),
    ("confirm_delete_setup", VIEW),
    ("confirm_delete_op", VIEW),
    (
        "pending",
        "an action waiting on another page: carried out as a command when it is",
    ),
    ("icons", WINDOW),
    ("closing", WINDOW),
    ("signing_out", ACCOUNT),
    ("gcode_exports", JOB),
    ("mcp", "this change log itself"),
    ("test_dir", "the tests'"),
    ("slow_generation", "the tests'"),
];

/// Feeds what is written into a hasher: a saved serialization is hashed
/// as it is written, with no string built.
struct Sink<'a>(&'a mut DefaultHasher);
impl Write for Sink<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The hash of `value`'s saved serialization (SipHash, std's own).
pub(crate) fn hash_of(value: &impl Serialize) -> u64 {
    let mut hasher = DefaultHasher::new();
    {
        // serde_json writes a token at a time: buffered, the hasher takes
        // it in pieces of 16 kB.
        let mut out = std::io::BufWriter::with_capacity(1 << 14, Sink(&mut hasher));
        // What the app saves serializes: a value that does not still has
        // the bytes written before its error hashed.
        let _ = serde_json::to_writer(&mut out, value);
        let _ = out.flush();
    }
    hasher.finish()
}

/// One item of a snapshot: its kind, its id (empty for a draft or the
/// project), and the hash of its saved serialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub kind: String,
    pub id: String,
    pub hash: u64,
}

/// A snapshot of the work state: the hash of it all, and each item's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub hash: u64,
    pub items: Vec<Item>,
}

/// A snapshot of `app`'s work state. Every snapshot hashes everything:
/// setups and operations have no version, and the controller changes a
/// machine without changing its `version`.
pub fn snapshot(app: &App) -> Snapshot {
    // Every field named, none with `..`: a new field of the `Document` or
    // a `Setup` does not compile until it is hashed here.
    let Document {
        schema,
        project_id,
        title,
        material,
        materials,
        material_id,
        starter_materials,
        starter_tools,
        starter_drill_fixed,
        workholding,
        file,
        sample_model,
        machines,
        tools,
        setups,
        active,
        guide,
        measured,
        places,
        pick_checks,
    } = &app.doc;
    let mut items = Vec::with_capacity(
        16 + setups.iter().map(|s| 1 + s.operations.len()).sum::<usize>()
            + machines.len()
            + tools.len()
            + materials.len()
            + workholding.len()
            + guide.len()
            + measured.len(),
    );
    let mut add = |kind: &str, id: &str, hash: u64| {
        items.push(Item {
            kind: kind.into(),
            id: id.into(),
            hash,
        })
    };
    let order: Vec<&str> = setups.iter().map(|s| s.id.as_str()).collect();
    add(
        "project",
        "",
        hash_of(&(
            schema,
            project_id,
            title,
            material,
            material_id,
            starter_materials,
            starter_tools,
            starter_drill_fixed,
            file,
            sample_model,
            active,
            places,
            pick_checks,
            order,
        )),
    );
    for s in setups {
        let Setup {
            preview_stock: _,
            preview_remaining: _,
            turn: _,
            job_material: _,
            id,
            machine,
            p,
            operations,
        } = s;
        let ops: Vec<&str> = operations.iter().map(|o| o.id.as_str()).collect();
        add("setup", id, hash_of(&(id, machine, p, ops)));
        for o in operations {
            add("operation", &o.id, hash_of(o));
        }
    }
    for m in machines {
        add("machine", &m.id, hash_of(m));
    }
    for t in tools {
        add("tool", &t.id, hash_of(t));
    }
    for m in materials {
        add("material", &m.id, hash_of(m));
    }
    for h in workholding {
        add("holder", &h.id, hash_of(h));
    }
    for (setup, at) in guide {
        add("guide_place", setup, hash_of(at));
    }
    for (key, value) in measured {
        add("measurement", key, hash_of(value));
    }
    add("machine_draft", "", hash_of(&app.machine_draft));
    add("tool_draft", "", hash_of(&app.tool_draft));
    add("material_draft", "", hash_of(&app.material_draft));
    add("holder_draft", "", hash_of(&app.holder_draft));
    let clamps = (app.clamps_draft.as_ref()).map(|c| (&c.setup, &c.text, &c.over));
    add("clamps_draft", "", hash_of(&clamps));
    add("guide_typed", "", hash_of(&app.guide_typed));
    add("project_file", "", hash_of(&app.project_path));
    let model = app.part_mesh().map(|m| m.fingerprint.as_str());
    add("model", "", hash_of(&model));
    let mut whole = DefaultHasher::new();
    for i in &items {
        whole.write(i.kind.as_bytes());
        whole.write_u8(0xff);
        whole.write(i.id.as_bytes());
        whole.write_u8(0xff);
        whole.write_u64(i.hash);
    }
    Snapshot {
        hash: whole.finish(),
        items,
    }
}

/// What became of an item between two snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum What {
    Added,
    Removed,
    Changed,
}

/// An item that differs between two snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    pub what: What,
}

/// The items that differ from `before` to `after`: those added or changed
/// in `after`'s order, then those removed. A difference in order alone is
/// the project's.
pub fn diff(before: &Snapshot, after: &Snapshot) -> Vec<Change> {
    let key = |i: &Item| (i.kind.clone(), i.id.clone());
    let was: BTreeMap<(String, String), u64> =
        before.items.iter().map(|i| (key(i), i.hash)).collect();
    let now: BTreeSet<(String, String)> = after.items.iter().map(key).collect();
    let change = |i: &Item, what| Change {
        kind: i.kind.clone(),
        id: i.id.clone(),
        what,
    };
    let mut changed: Vec<Change> = (after.items.iter())
        .filter_map(|i| match was.get(&key(i)) {
            None => Some(change(i, What::Added)),
            Some(h) if *h != i.hash => Some(change(i, What::Changed)),
            Some(_) => None,
        })
        .collect();
    changed.extend(
        (before.items.iter())
            .filter(|i| !now.contains(&key(i)))
            .map(|i| change(i, What::Removed)),
    );
    if changed.is_empty() && before.hash != after.hash {
        changed.push(Change {
            kind: "project".into(),
            id: String::new(),
            what: What::Changed,
        });
    }
    changed
}

/// Who made a change-log entry's difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Found when an agent's window closed, with no external event in it.
    Agent,
    /// Found when an agent's window closed, with external events in it.
    Mixed,
    /// Found when a window opened, or by a read outside one: everything
    /// since the last window closed.
    User,
}

/// A step of the revision: what differed, who made it, and the external
/// events that made it, named.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub revision: u64,
    pub source: Source,
    pub changed: Vec<Change>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
    /// What the step was, when the change log says it: "checkpoint
    /// restored".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The revision, the last snapshot and the change log. Kept with the
/// `App` across clients, so edits made while no agent was connected are
/// found when the next one works; a headless session saves it
/// (`session.json`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Log {
    /// Up by one each time a snapshot differs from the one before; never
    /// down.
    pub revision: u64,
    pub last: Option<Snapshot>,
    pub entries: Vec<Entry>,
    /// The revision last reported to the agent.
    pub reported: u64,
    /// How many times the project was replaced (`App::replace_document`):
    /// New, Open, the example, Recover previous. A checkpoint taken before
    /// a replacement belongs to another project.
    #[serde(default)]
    pub projects: u64,
}
impl Log {
    /// Takes `now` as the last snapshot. When it differs from the one
    /// before, the revision goes up and the entry for it is made, from
    /// `source` (the first snapshot only sets where the log starts).
    pub fn record(&mut self, now: Snapshot, source: Source, events: Vec<String>) -> Option<&Entry> {
        let changed = match &self.last {
            Some(last) if last.hash != now.hash => Some(diff(last, &now)),
            _ => None,
        };
        self.last = Some(now);
        let changed = changed?;
        self.revision += 1;
        self.entries.push(Entry {
            revision: self.revision,
            source,
            changed,
            events,
            note: None,
        });
        self.entries.last()
    }
    /// Takes `now` as the last snapshot and goes up a revision whether or
    /// not it differs, with an entry saying `note`: a checkpoint restored
    /// is a step of its own, so an `if_revision` from before it is stale.
    pub fn step(&mut self, now: Snapshot, source: Source, note: &str) -> &Entry {
        let changed = match &self.last {
            Some(last) => diff(last, &now),
            None => vec![],
        };
        self.last = Some(now);
        self.revision += 1;
        self.entries.push(Entry {
            revision: self.revision,
            source,
            changed,
            events: vec![],
            note: Some(note.into()),
        });
        let last = self.entries.len() - 1;
        &self.entries[last]
    }
    /// The entries after `revision`.
    pub fn since(&self, revision: u64) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(move |e| e.revision > revision)
    }
    /// The `user` and `mixed` entries since the revision last reported to
    /// the agent, which this reports.
    pub fn report(&mut self) -> Vec<Entry> {
        let theirs = (self.since(self.reported))
            .filter(|e| e.source != Source::Agent)
            .cloned()
            .collect();
        self.reported = self.revision;
        theirs
    }
}

/// The event of a resumed session's change that its `session.json` did not
/// record (`Tracker::unrecorded`).
pub const UNRECORDED: &str = "agent: a change made before the session was resumed, whose record (session.json) was not saved";

/// The app's record of who changes the job (`App::mcp`): the log, and the
/// external events since the last snapshot.
#[derive(Debug, Default)]
pub struct Tracker {
    pub log: Log,
    /// The external events since the last snapshot, each once.
    events: Vec<String>,
    /// How deep in `App::update` the app is: only the outermost message
    /// arrived from outside.
    depth: u32,
    /// The next outermost message is the engine's (`App::update_by_agent`).
    agent_turn: bool,
    /// The outermost message is an external event: the work it starts is
    /// the user's.
    user_turn: bool,
    /// The epoch of the generation under way, when the user started it.
    user_generation: Option<u64>,
    /// The heavy reads' workers still running (`reads`), which Cycle Start
    /// waits for (`App::planning_busy`), cancelled ones too.
    pub heavy: super::reads::Heavy,
    /// The checkpoints the agent took (`checkpoint`), oldest first.
    pub checkpoints: super::checkpoint::Kept,
    /// What the file commands' completion messages said since the window
    /// opened (`Tracker::note`): the effect checks judge by these, not by
    /// state that may already match.
    pub completions: Vec<Completion>,
    /// What the project save handler's last run came to: the path it
    /// wrote, or why it could not (set in `Message::Saved`; cleared when a
    /// window opens).
    pub last_save: Option<Result<std::path::PathBuf, String>>,
    /// The paths of the last saves and exports the agent made.
    pub outputs: Vec<super::session::Output>,
    /// A headless session resumed whose workspace holds a change its
    /// `session.json` was not written with (that write failed, or the
    /// process ended first): the next difference found is the server's own
    /// work for the agent, not the user's (`mcp_outside`).
    pub unrecorded: bool,
    /// Notes for the next result about the job (`engine`), then dropped.
    pub notes: Vec<String>,
    /// Whether the job is saved to its project file, as last worked out
    /// (`App::saved_to_file`).
    pub saved: super::checkpoint::SavedCache,
    /// How long each read's worker waits before it works (tests).
    #[cfg(test)]
    pub slow_reads: Option<std::time::Duration>,
    /// How long a checkpoint's part takes to load before it loads (tests).
    #[cfg(test)]
    pub slow_candidate: Option<std::time::Duration>,
}

/// Which file work a completion message finishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Import,
    Reimport,
    Restore,
    Open,
    Gcode,
    Sheet,
    Tablet,
}

/// A file work's completion message as the app received it: the path it
/// came to, or the app's error text.
#[derive(Debug, Clone)]
pub struct Completion {
    pub kind: Kind,
    pub result: Result<Option<std::path::PathBuf>, String>,
}

/// The most completions kept between windows.
const MOST_COMPLETIONS: usize = 32;

impl Tracker {
    /// Records `m` when it completes file work, whoever started it.
    pub(crate) fn note(&mut self, m: &Message) {
        let none = || "No file was chosen.".to_string();
        let (kind, result) = match m {
            Message::FileImported(r) => (
                Kind::Import,
                match r {
                    Some(Ok((f, _))) => Ok(Some(f.path.clone())),
                    Some(Err(e)) => Err(e.clone()),
                    None => Err(none()),
                },
            ),
            Message::ModelReimported(_, r) => (
                Kind::Reimport,
                match r {
                    Ok((f, _)) => Ok(Some(f.path.clone())),
                    Err(e) => Err(e.clone()),
                },
            ),
            Message::ModelRestored(_, r) => (
                Kind::Restore,
                r.as_ref().map(|_| None).map_err(Clone::clone),
            ),
            Message::ProjectOpened(r) => (
                Kind::Open,
                match r {
                    Some(Ok((_, p, _))) => Ok(Some(p.clone())),
                    Some(Err(e)) => Err(e.clone()),
                    None => Err(none()),
                },
            ),
            Message::GcodeExported(r) => (
                Kind::Gcode,
                match r {
                    Some(Ok((p, _))) => Ok(Some(p.clone())),
                    Some(Err(e)) => Err(e.clone()),
                    None => Err(none()),
                },
            ),
            Message::Exported(p) => (Kind::Sheet, Ok(Some(p.clone()))),
            Message::ExportError(e) => (Kind::Sheet, Err(e.clone())),
            Message::TabletSaved(r) => (
                Kind::Tablet,
                match r {
                    Some(Ok(p)) => Ok(Some(p.clone())),
                    Some(Err(e)) => Err(e.clone()),
                    None => Err(none()),
                },
            ),
            _ => return,
        };
        if self.completions.len() >= MOST_COMPLETIONS {
            self.completions.remove(0);
        }
        self.completions.push(Completion { kind, result });
    }
    /// A command's window opens: what came before is not its.
    pub(crate) fn begin(&mut self) {
        self.completions.clear();
        self.last_save = None;
    }
    /// The completion just noted turned out not to have done its work.
    pub(crate) fn failed_last(&mut self, why: String) {
        if let Some(c) = self.completions.last_mut() {
            c.result = Err(why);
        }
    }
}

/// The most external events an entry names: with no agent connected for
/// long, the user's work is many of them.
const MOST_EVENTS: usize = 40;

/// Commands that send these are view-only: the user using them while an
/// agent works changes nothing of the job.
const VIEW_ONLY: &[&str] = &[
    "goto_page",
    "set_tab",
    "set_setup_step",
    "open_setup_editor",
    "show_stock_after_setup",
    "select_operation",
    "open_audit_row",
    "open_issue",
    "machine_wizard_step",
    "toggle_stock",
    "toggle_uncut",
    "toggle_sim_details",
    "toggle_warning_details",
    "toggle_paths",
    "toggle_sidebar",
    "sidebar_width",
    "toggle_about",
    "set_camera",
    "orbit_camera",
    "pan_camera",
    "zoom_camera",
    "section",
    "play",
    "scrub",
    "playback_speed",
    "stock_detail",
    "control_tab",
    "job_view",
    "run_details",
];

/// What `m`, arriving from the window's own handlers, is named as an
/// external event: None for one that is view-only, or the app's own (a
/// tick, a frame, a background result: the work that started it was
/// counted when it started).
pub fn external_name(m: &Message) -> Option<String> {
    use super::class::{Class, class};
    match class(m) {
        Class::Internal => None,
        Class::Command(name) if VIEW_ONLY.contains(&name) => None,
        Class::Command(name) => Some(match m {
            Message::Edit(target, key, _) => {
                format!(
                    "user: {name} {} {key}",
                    format!("{target:?}").to_lowercase()
                )
            }
            _ => format!("user: {name}"),
        }),
        // The planner's own traffic is not the job; applying a plan is.
        Class::Planner => {
            matches!(m, Message::ApplyPlan | Message::UndoPlan).then(|| format!("user: {m:?}"))
        }
        Class::Machine | Class::Device => {
            let said = format!("{m:?}");
            let cut = (said.char_indices().nth(60)).map_or(said.len(), |(i, _)| i);
            Some(format!("user: {}", &said[..cut]))
        }
    }
}

/// The generation a background result is of, and what it is called.
fn completion(m: &Message) -> Option<(u64, &'static str)> {
    match m {
        Message::Generated(epoch, _) => Some((*epoch, "generation")),
        Message::GeneratedAll(epoch, _) => Some((*epoch, "Generate all")),
        Message::GeneratedLayout(epoch, _) => Some((*epoch, "layout")),
        _ => None,
    }
}

impl App {
    /// Sends `message` as the engine's own dispatch: it is the agent's,
    /// and is not counted as an external event. Every other message that
    /// arrives at `update` (the window's) is.
    pub(crate) fn update_by_agent(&mut self, message: Message) -> Task<Message> {
        self.mcp.agent_turn = true;
        self.update(message)
    }
    /// Records an external event: the work state changed by the user or the
    /// machine, not by an agent, named as "user: …" or "machine: …".
    pub(crate) fn external(&mut self, what: impl Into<String>) {
        let what = what.into();
        let events = &mut self.mcp.events;
        if events.len() < MOST_EVENTS && !events.contains(&what) {
            events.push(what);
        } else if events.len() == MOST_EVENTS {
            events.push("and more".into());
        }
    }
    /// A message arrives at `update`: the outermost one, unless the engine
    /// sent it, is counted as an external event when it is not view-only,
    /// and a result of a generation the user started is counted when it is
    /// still current, so it will be applied.
    pub(crate) fn mcp_arrive(&mut self, message: &Message) {
        self.mcp.note(message);
        self.mcp.depth += 1;
        if self.mcp.depth > 1 {
            return;
        }
        if std::mem::take(&mut self.mcp.agent_turn) {
            return;
        }
        if let Some((epoch, what)) = completion(message)
            && self.mcp.user_generation == Some(epoch)
            && self.generating.as_ref().is_some_and(|g| g.epoch == epoch)
        {
            self.external(format!("user: {what} finished"));
        }
        if let Some(name) = external_name(message) {
            self.external(name);
            self.mcp.user_turn = true;
        }
    }
    /// The message `mcp_arrive` took has been handled.
    pub(crate) fn mcp_leave(&mut self) {
        self.mcp.depth = self.mcp.depth.saturating_sub(1);
        if self.mcp.depth == 0 {
            self.mcp.user_turn = false;
        }
    }
    /// A generation of `epoch` starts: the user's when an external event
    /// started it.
    pub(crate) fn mcp_generation(&mut self, epoch: u64) {
        self.mcp.user_generation = self.mcp.user_turn.then_some(epoch);
    }
    /// A snapshot when a window opens, or by a read while none is open: a
    /// difference since the last is the user's. Its entry, if any.
    /// A resumed session's change not recorded before (`Tracker::unrecorded`)
    /// is the agent's, named as such.
    pub(crate) fn mcp_outside(&mut self) -> Option<Entry> {
        let now = snapshot(self);
        let mut events = std::mem::take(&mut self.mcp.events);
        let source = if std::mem::take(&mut self.mcp.unrecorded) {
            events = vec![UNRECORDED.to_string()];
            Source::Agent
        } else {
            Source::User
        };
        self.mcp.log.record(now, source, events).cloned()
    }
    /// The external events since the window opened.
    pub(crate) fn mcp_events(&self) -> &[String] {
        &self.mcp.events
    }
    /// A checkpoint was restored, in the turn that committed it: a step of
    /// the revision of the agent's, whatever changed. Its entry.
    pub(crate) fn mcp_restored(&mut self, name: &str) -> Entry {
        let now = snapshot(self);
        let note = format!("checkpoint restored: {name}");
        self.mcp.log.step(now, Source::Agent, &note).clone()
    }
    /// A snapshot when a window closes: a difference is the agent's, or
    /// mixed with the external events during it. Its entry, if any.
    pub(crate) fn mcp_close(&mut self) -> Option<Entry> {
        let now = snapshot(self);
        let events = std::mem::take(&mut self.mcp.events);
        let source = if events.is_empty() {
            Source::Agent
        } else {
            Source::Mixed
        };
        self.mcp.log.record(now, source, events).cloned()
    }
}

/// One of the machinist's records.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Record {
    /// The step guide's place on the setup of this id.
    Place(String),
    /// A measurement, by its key (`guide::measure_key`).
    Measured(String),
    /// The cross-slide dial reading of a machine (id): in the library
    /// (`None`), or in the setup (id) of that machine.
    Dial(Option<String>, String),
}

/// The machinist's records: the step guide's places, the measurements and
/// the cross-slide dial readings, with the setups and library machines
/// they belong to. They belong to the project they were recorded in: a
/// command that replaces the project brings its own, and removing a
/// setup removes its own; nothing else an agent does may change them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Records {
    /// Each setup's id, with its machine's.
    setups: BTreeMap<String, String>,
    machines: BTreeSet<String>,
    places: BTreeMap<String, usize>,
    measured: BTreeMap<String, f32>,
    dials: BTreeMap<(Option<String>, String), String>,
}
/// Where a machine's cross-slide dial reading is kept.
pub(crate) const DIAL: &str = "cross_reads";
impl Records {
    pub fn of(doc: &Document) -> Records {
        let dial = |m: &crate::model::Machine| m.p.get(DIAL).cloned();
        let mut dials = BTreeMap::new();
        for m in &doc.machines {
            if let Some(d) = dial(m) {
                dials.insert((None, m.id.clone()), d);
            }
        }
        for s in &doc.setups {
            if let Some(d) = dial(&s.machine) {
                dials.insert((Some(s.id.clone()), s.machine.id.clone()), d);
            }
        }
        Records {
            setups: (doc.setups.iter())
                .map(|s| (s.id.clone(), s.machine.id.clone()))
                .collect(),
            machines: doc.machines.iter().map(|m| m.id.clone()).collect(),
            places: doc.guide.clone(),
            measured: doc.measured.clone(),
            dials,
        }
    }
    /// The records of setups (and library machines) there both before
    /// (`self`) and `now` that differ.
    pub fn changed(&self, now: &Records) -> Vec<Record> {
        let both = |id: &str| self.setups.contains_key(id) && now.setups.contains_key(id);
        let setup_of = |key: &str| key.split_once('/').map_or(key, |(s, _)| s).to_string();
        let mut changed = BTreeSet::new();
        for id in self.places.keys().chain(now.places.keys()) {
            if both(id) && self.places.get(id) != now.places.get(id) {
                changed.insert(Record::Place(id.clone()));
            }
        }
        for key in self.measured.keys().chain(now.measured.keys()) {
            if both(&setup_of(key)) && self.measured.get(key) != now.measured.get(key) {
                changed.insert(Record::Measured(key.clone()));
            }
        }
        for key in self.dials.keys().chain(now.dials.keys()) {
            let (place, machine) = key;
            let there = match place {
                None => self.machines.contains(machine) && now.machines.contains(machine),
                // A setup's copy of its machine made the library's again
                // (Assign to current setup) takes the machinist's own
                // reading from it.
                Some(s) => {
                    self.setups.get(s) == Some(machine)
                        && now.setups.get(s) == Some(machine)
                        && now.dials.get(key) != now.dials.get(&(None, machine.clone()))
                }
            };
            if there && self.dials.get(key) != now.dials.get(key) {
                changed.insert(Record::Dial(place.clone(), machine.clone()));
            }
        }
        changed.into_iter().collect()
    }
    /// Puts `records` in `doc` back as they are in `self`.
    pub fn put_back(&self, doc: &mut Document, records: &[Record]) {
        fn set<K: Ord + Clone, V: Clone>(map: &mut BTreeMap<K, V>, key: &K, was: Option<&V>) {
            match was {
                Some(v) => {
                    map.insert(key.clone(), v.clone());
                }
                None => {
                    map.remove(key);
                }
            }
        }
        for r in records {
            match r {
                Record::Place(id) => set(&mut doc.guide, id, self.places.get(id)),
                Record::Measured(key) => set(&mut doc.measured, key, self.measured.get(key)),
                Record::Dial(place, machine) => {
                    let was = self.dials.get(&(place.clone(), machine.clone()));
                    let m = match place {
                        None => doc.machines.iter_mut().find(|m| m.id == *machine),
                        Some(s) => (doc.setups.iter_mut())
                            .find(|x| x.id == *s)
                            .map(|x| &mut x.machine),
                    };
                    if let Some(m) = m {
                        set(&mut m.p, &DIAL.to_string(), was);
                    }
                }
            }
        }
    }
}
/// A record, as a result names it.
pub fn record_name(doc: &Document, r: &Record) -> String {
    let setup = |id: &str| match doc.setups.iter().position(|s| s.id == id) {
        Some(i) => format!("setup {}", i + 1),
        None => format!("setup {id}"),
    };
    match r {
        Record::Place(id) => format!("the step guide's place on {}", setup(id)),
        Record::Measured(key) => {
            let id = key.split_once('/').map_or(key.as_str(), |(s, _)| s);
            format!("a measurement on {} ({key})", setup(id))
        }
        Record::Dial(None, machine) => format!("the cross-slide dial reading of machine {machine}"),
        Record::Dial(Some(id), _) => {
            format!("the cross-slide dial reading in {}", setup(id))
        }
    }
}

/// The commands that replace the project: they bring the incoming
/// project's records, so the guard does not hold them.
pub const REPLACES_PROJECT: [&str; 5] = [
    "new_project",
    "open_sample",
    "open_project",
    "resume_project",
    "recover_previous_project",
];

/// The outcome's reason when the guard puts the records back.
pub const PROGRESS: &str = "This would have changed the machinist's progress";

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::mcp::{NOT_WORK_STATE, WORK_STATE};

    /// The fields of `pub struct App { … }` in `main.rs`, read from its
    /// source: a field's name begins a line at the struct's own depth.
    fn app_fields() -> Vec<String> {
        let source = include_str!("../main.rs");
        let start = source.find("\npub struct App {").expect("App in main.rs");
        let body = &source[start..];
        let body = &body[body.find('{').unwrap() + 1..body.find("\n}\n").unwrap()];
        let mut depth = 0i32;
        let mut fields = vec![];
        for line in body.lines() {
            let line = line.trim();
            if line.starts_with("//") || line.starts_with("#[") {
                continue;
            }
            if depth == 0 {
                let line = line
                    .strip_prefix("pub(crate) ")
                    .or_else(|| line.strip_prefix("pub "))
                    .unwrap_or(line);
                let name: String = (line.chars())
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                let rest = &line[name.len()..];
                if !name.is_empty() && rest.starts_with(':') && !rest.starts_with("::") {
                    fields.push(name);
                }
            }
            let line = line.replace("->", "");
            for c in line.chars() {
                match c {
                    '<' | '(' | '[' | '{' => depth += 1,
                    '>' | ')' | ']' | '}' => depth -= 1,
                    _ => {}
                }
            }
        }
        fields
    }

    #[test]
    fn every_app_field_is_work_state_or_says_why_not() {
        let fields = app_fields();
        assert!(fields.len() > 150, "{fields:?}");
        for f in [
            "doc",
            "later_drills",
            "start_mesh",
            "slow_generation",
            "mcp",
        ] {
            assert!(fields.iter().any(|x| x == f), "{f} not read from App");
        }
        assert!(!fields.iter().any(|f| f == "std"), "{fields:?}");
        let listed: Vec<&str> = (WORK_STATE.iter().chain(NOT_WORK_STATE))
            .map(|(name, _)| *name)
            .collect();
        for f in &fields {
            let n = listed.iter().filter(|l| **l == f).count();
            assert!(
                n == 1,
                "App::{f} must be in exactly one of mcp::WORK_STATE and mcp::NOT_WORK_STATE, with a reason ({n})"
            );
        }
        for (name, reason) in WORK_STATE.iter().chain(NOT_WORK_STATE) {
            assert!(
                fields.iter().any(|f| f == name),
                "{name} is listed but App has no such field"
            );
            assert!(!reason.is_empty(), "{name} has no reason");
        }
        for draft in [
            "machine_draft",
            "tool_draft",
            "material_draft",
            "holder_draft",
            "clamps_draft",
            "guide_typed",
        ] {
            assert!(WORK_STATE.iter().any(|(n, _)| *n == draft), "{draft}");
        }
        for theirs in ["profile_draft", "pi_code"] {
            assert!(NOT_WORK_STATE.iter().any(|(n, _)| *n == theirs));
        }
    }

    /// The test's own reading of `App`'s fields fails a field in neither
    /// list: one added to a copy of the struct is found and is unlisted.
    #[test]
    fn a_new_field_is_found() {
        let fields = app_fields();
        let listed = |f: &str| (WORK_STATE.iter().chain(NOT_WORK_STATE)).any(|(n, _)| *n == f);
        assert!(fields.iter().all(|f| listed(f)));
        assert!(!listed("lathe_draft"));
    }

    fn app() -> App {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::from_workspace(dir.path().join("workspace.json"), None);
        app.test_dir = Some(dir);
        let mut doc = Document::sample();
        doc.adopt_materials();
        doc.adopt_holders();
        app.replace_document(doc, None);
        app
    }

    #[test]
    fn a_snapshot_says_which_items_changed() {
        let mut a = app();
        let before = snapshot(&a);
        assert_eq!(before, snapshot(&a), "a snapshot is the same twice");
        // Streamed, the hash is the one of the saved bytes.
        let mut whole = DefaultHasher::new();
        whole.write(&serde_json::to_vec(&a.doc).unwrap());
        assert_eq!(hash_of(&a.doc), whole.finish());
        let op = a.doc.setups[0].operations[1].id.clone();
        a.doc.setups[0].operations[1]
            .p
            .insert("feed".into(), "1234".into());
        a.tool_draft.p.insert("name".into(), "Typed".into());
        let after = snapshot(&a);
        assert_ne!(before.hash, after.hash);
        let changed = diff(&before, &after);
        assert_eq!(
            changed,
            [
                Change {
                    kind: "operation".into(),
                    id: op,
                    what: What::Changed
                },
                Change {
                    kind: "tool_draft".into(),
                    id: String::new(),
                    what: What::Changed
                },
            ]
        );
        // An operation moved is its setup's change.
        a.doc.setups[0].operations.swap(0, 1);
        let moved = diff(&after, &snapshot(&a));
        assert_eq!(moved.len(), 1, "{moved:?}");
        assert_eq!(moved[0].kind, "setup");
        // The log: a baseline, then one step a difference.
        let mut log = Log::default();
        assert!(log.record(before.clone(), Source::User, vec![]).is_none());
        assert!(log.record(before.clone(), Source::User, vec![]).is_none());
        let e = log.record(after, Source::Mixed, vec!["user: x".into()]);
        assert_eq!(e.map(|e| (e.revision, e.source)), Some((1, Source::Mixed)));
        assert_eq!(log.report().len(), 1);
        assert!(log.report().is_empty(), "reported once");
        assert!(log.record(before, Source::Agent, vec![]).is_some());
        assert_eq!(log.revision, 2);
        assert!(
            log.report().is_empty(),
            "the agent's own are not the user's"
        );
    }

    #[test]
    fn records_changed_on_setups_there_before_and_after_are_put_back() {
        let mut a = app();
        let first = a.doc.setups[0].id.clone();
        a.doc.guide.insert(first.clone(), 3);
        a.doc
            .measured
            .insert(crate::guide::measure_key(&first, "zero"), 40.);
        a.doc.setups[0]
            .machine
            .p
            .insert(DIAL.into(), "Diameter".into());
        let machine = a.doc.machines[0].id.clone();
        a.doc.machines[0].p.insert(DIAL.into(), "Diameter".into());
        let before = Records::of(&a.doc);
        let mut doc = a.doc.clone();
        doc.guide.insert(first.clone(), 4);
        doc.measured.clear();
        doc.machines[0].p.insert(DIAL.into(), "Radius".into());
        let changed = before.changed(&Records::of(&doc));
        assert_eq!(changed.len(), 3, "{changed:?}");
        before.put_back(&mut doc, &changed);
        assert_eq!(Records::of(&doc), before);
        assert!(record_name(&doc, &Record::Place(first.clone())).contains("setup 1"));
        assert!(record_name(&doc, &Record::Dial(None, machine)).contains("dial"));
        // A setup removed takes its records: none of them is a change.
        let mut gone = a.doc.clone();
        gone.setups.remove(0);
        gone.guide.remove(&first);
        gone.measured.clear();
        assert!(before.changed(&Records::of(&gone)).is_empty());
    }

    use crate::mcp::engine::{self, Agent, Backend};
    use crate::{GuideAction, Target};
    use serde_json::{Value, json};

    /// An `App` driven straight, as the window drives it: what the engine
    /// sends is the agent's, what `user` sends the window's. Each task runs
    /// to its end as it is sent, unless `hold` keeps it (and the window it
    /// belongs to) waiting until `release`.
    pub(crate) struct Bench {
        pub(crate) app: App,
        agent: Option<Agent>,
        runtime: tokio::runtime::Runtime,
        held: Vec<(Task<Message>, bool)>,
        pub(crate) hold: bool,
        /// A write the agent's path makes after each message: a hole in
        /// the boundary, for the guard to find.
        bug: Option<fn(&mut App)>,
        dir: tempfile::TempDir,
    }
    impl Bench {
        pub(crate) fn new(mut app: App) -> Bench {
            // Generation runs off the thread, as in the window.
            app.generate_inline = false;
            app.dirty = false;
            Bench {
                app,
                agent: Some(Agent::new()),
                runtime: tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap(),
                held: vec![],
                hold: false,
                bug: None,
                dir: tempfile::tempdir().unwrap(),
            }
        }
        /// On the sample job, its first operation selected.
        pub(crate) fn sample() -> Bench {
            let mut b = Bench::new(app());
            b.app.selected = Some(0);
            b
        }
        /// On the fixture block, imported, its Face alone and selected: a
        /// job whose generation is real.
        fn imported() -> Bench {
            let mut b = Bench::sample();
            let (mut doc, mesh) = crate::post::fixture::job();
            doc.setups[0].operations.truncate(1);
            doc.adopt_materials();
            doc.adopt_holders();
            let aluminium = (doc.materials.iter())
                .find(|m| m.name() == "Aluminium 6061-T6")
                .map(|m| m.id.clone())
                .unwrap();
            assert!(doc.select_material(&aluminium));
            b.app.replace_document(doc, None);
            b.app.imported_mesh = Some(mesh);
            b.app.selected = Some(0);
            b.app.dirty = false;
            b
        }
        fn deliver(&mut self, task: Task<Message>, by_agent: bool) {
            use futures_util::StreamExt;
            if self.hold {
                self.held.push((task, by_agent));
                return;
            }
            let Some(stream) = iced_runtime::task::into_stream(task) else {
                return;
            };
            let outputs: Vec<Message> = self.runtime.block_on(
                stream
                    .filter_map(|action| async move {
                        match action {
                            iced_runtime::Action::Output(m) => Some(m),
                            _ => None,
                        }
                    })
                    .collect(),
            );
            for m in outputs {
                let task = if by_agent {
                    self.app.update_by_agent(m)
                } else {
                    self.app.update(m)
                };
                self.deliver(task, by_agent);
            }
        }
        /// A message from the window's own handlers: the user's.
        pub(crate) fn user(&mut self, m: Message) {
            let task = self.app.update(m);
            self.deliver(task, false);
        }
        fn tick(&mut self) {
            let task = self.app.update(Message::Tick);
            self.deliver(task, false);
        }
        /// Runs the work held back.
        pub(crate) fn release(&mut self) {
            self.hold = false;
            for (task, by_agent) in std::mem::take(&mut self.held) {
                self.deliver(task, by_agent);
            }
        }
        /// A tool call's answer: polled until it comes.
        fn answer(&mut self, tool: &str, args: Value) -> engine::Answer {
            use std::sync::{Arc, Mutex};
            let mut agent = self.agent.take().unwrap();
            let answer: Arc<Mutex<Option<engine::Answer>>> = Arc::default();
            let into = answer.clone();
            let reply = Box::new(move |a| *into.lock().unwrap() = Some(a));
            agent.call(self, tool, args, reply);
            for _ in 0..200 {
                if answer.lock().unwrap().is_some() {
                    break;
                }
                agent.poll(self);
            }
            self.agent = Some(agent);
            let taken = answer.lock().unwrap().take();
            taken.unwrap_or_else(|| Err(format!("no answer; pending {:?}", self.pending())))
        }
        pub(crate) fn call(&mut self, tool: &str, args: Value) -> Value {
            self.answer(tool, args).unwrap()
        }
        pub(crate) fn run(&mut self, command: &str, args: Value) -> Value {
            self.call("execute", json!({"command": command, "args": args}))
        }
        /// The agent goes, and another connects later: the `App` stays.
        fn reconnect(&mut self) {
            self.agent = Some(Agent::new());
        }
    }
    impl Backend for Bench {
        fn app(&self) -> &App {
            &self.app
        }
        fn app_mut(&mut self) -> &mut App {
            &mut self.app
        }
        fn send(&mut self, message: Message) {
            let task = self.app.update_by_agent(message);
            if let Some(bug) = self.bug {
                bug(&mut self.app);
            }
            self.deliver(task, true);
        }
        fn run(&mut self, task: Task<Message>) {
            self.deliver(task, true);
        }
        /// The edit pause taken as over and a `Tick` run, unless work is held.
        fn quiet(&mut self) -> bool {
            if !self.held.is_empty() {
                return false;
            }
            if let Some(past) =
                std::time::Instant::now().checked_sub(std::time::Duration::from_secs(1))
            {
                self.app.last_edit = past;
            }
            self.tick();
            self.held.is_empty()
        }
        fn pending(&self) -> Vec<&'static str> {
            let mut pending = self.app.pending();
            if !self.held.is_empty() {
                pending.push("held work");
            }
            pending
        }
        fn kind(&self) -> &'static str {
            "headless"
        }
        fn session(&self) -> Value {
            json!({"id": "bench"})
        }
        /// The session's own folder, in the tests' folder: no path the
        /// agent gives is written in it.
        fn session_folder(&self) -> Option<std::path::PathBuf> {
            Some(self.dir.path().join("session"))
        }
    }

    /// The user and mixed entries a result reports.
    fn theirs(result: &Value) -> Vec<Entry> {
        serde_json::from_value(result["user_changed"].clone()).unwrap()
    }
    fn names(e: &Entry) -> Vec<(String, String)> {
        (e.changed.iter())
            .map(|c| (c.kind.clone(), c.id.clone()))
            .collect()
    }
    fn has(e: &Entry, kind: &str) -> bool {
        e.changed.iter().any(|c| c.kind == kind)
    }

    #[test]
    fn what_the_user_changes_is_reported_as_theirs() {
        let mut b = Bench::sample();
        let first = b.call("inspect", json!({}));
        let start = first["revision"].as_u64().unwrap();
        assert_eq!(theirs(&first), [], "{first}");
        assert_eq!(first["project"]["revision"], start);

        // A tool-draft edit.
        b.user(Message::Edit(Target::Tool, "name", "Typed by hand".into()));
        let read = b.call("inspect", json!({}));
        let [e] = &theirs(&read)[..] else {
            panic!("{read}")
        };
        assert_eq!((e.revision, e.source), (start + 1, Source::User));
        assert_eq!(names(e), [("tool_draft".into(), String::new())]);
        assert_eq!(e.events, ["user: edit_field tool name"]);
        // Told once.
        assert_eq!(theirs(&b.call("inspect", json!({}))), []);

        // A half-typed clamp positions field.
        b.user(Message::SetClamps("12, x".into()));
        assert!(b.app.clamps_draft.is_some(), "{}", b.app.status);
        let read = b.call("inspect", json!({}));
        let [e] = &theirs(&read)[..] else {
            panic!("{read}")
        };
        assert!(has(e, "clamps_draft"), "{e:?}");
        assert_eq!(e.events, ["user: set_clamps"]);

        // A project opened whose setups keep their ids: what changed in
        // them is named by id.
        let path = b.dir.path().join("same-ids.axis.json");
        let mut doc = b.app.doc.clone();
        let op = doc.setups[0].operations[1].id.clone();
        doc.setups[0].operations[1]
            .p
            .insert("feed".into(), "321".into());
        crate::storage::save(&path, &doc).unwrap();
        let opened = crate::storage::load_reporting(&path).map(|(d, n)| (d, path.clone(), n));
        b.user(Message::ProjectOpened(Some(opened)));
        assert_eq!(b.app.project_path.as_ref(), Some(&path));
        let read = b.call("inspect", json!({}));
        let [e] = &theirs(&read)[..] else {
            panic!("{read}")
        };
        assert!(
            e.changed
                .iter()
                .any(|c| c.kind == "operation" && c.id == op && c.what == What::Changed),
            "{e:?}"
        );
        assert!(has(e, "project_file"), "{e:?}");
        assert!(
            !e.changed
                .iter()
                .any(|c| c.kind == "setup" && c.what != What::Changed)
        );
        assert_eq!(e.events, ["user: open_project"]);

        // NewProject.
        b.user(Message::NewProject);
        let read = b.call("inspect", json!({}));
        let [e] = &theirs(&read)[..] else {
            panic!("{read}")
        };
        assert!(
            e.changed
                .iter()
                .any(|c| c.kind == "setup" && c.what == What::Removed),
            "{e:?}"
        );
        assert_eq!(e.events, ["user: new_project"]);

        // An edit while no agent is connected: the next one is told.
        b.reconnect();
        b.user(Message::Title("Edited with no agent".into()));
        let read = b.call("inspect", json!({}));
        let [e] = &theirs(&read)[..] else {
            panic!("{read}")
        };
        assert!(has(e, "project"), "{e:?}");
        assert_eq!(e.events, ["user: set_project_title"]);
        assert_eq!(read["revision"], e.revision);

        // What the user only looks at is not a change.
        b.user(Message::ToggleStock);
        b.user(Message::Camera("TOP"));
        assert_eq!(theirs(&b.call("inspect", json!({}))), []);
    }

    #[test]
    fn an_if_revision_older_than_a_user_change_is_a_conflict() {
        let mut b = Bench::sample();
        let seen = b.call("inspect", json!({}))["revision"].as_u64().unwrap();
        b.user(Message::Edit(Target::Tool, "name", "Mine".into()));
        let title = |r: u64| json!({"command": "set_project_title", "args": {"title": "Theirs"}, "if_revision": r});
        let r = b.call("execute", title(seen));
        assert_eq!(r["outcome"], "conflict", "{r}");
        let entries: Vec<Entry> = serde_json::from_value(r["entries"].clone()).unwrap();
        assert_eq!(entries.len(), 1, "{r}");
        assert_eq!(entries[0].source, Source::User);
        assert_ne!(b.app.doc.title, "Theirs");
        let now = r["revision"].as_u64().unwrap();
        assert_eq!(now, seen + 1);
        let done = b.call("execute", title(now));
        assert_eq!(done["outcome"], "done", "{done}");
        assert_eq!(done["revision"], now + 1);
        let changed: Vec<Change> = serde_json::from_value(done["changed"].clone()).unwrap();
        assert_eq!(changed[0].kind, "project");
        // The agent's own change is not the user's.
        assert_eq!(theirs(&done), []);
        let batch = json!({"steps": [{"command": "set_project_title", "args": {"title": "B"}}], "if_revision": now});
        assert_eq!(b.call("batch", batch)["outcome"], "conflict");
        let bad =
            json!({"command": "set_project_title", "args": {"title": "C"}, "if_revision": "x"});
        assert!(b.answer("execute", bad).is_err());
    }

    #[test]
    fn a_user_edit_during_generate_all_is_named_and_a_stale_result_is_dropped() {
        let mut b = Bench::sample();
        b.call("inspect", json!({}));
        b.hold = true;
        let first = b.call("execute", json!({"command": "generate_all", "wait_s": 0}));
        assert_eq!(first["outcome"], "still_running", "{first}");
        let id = first["command_id"].as_str().unwrap().to_string();
        // A read while the window is open takes no snapshot.
        let before = b.app.mcp.log.clone();
        let read = b.call("inspect", json!({}));
        assert_eq!(b.app.mcp.log.revision, before.revision);
        assert_eq!(theirs(&read), []);
        // The user edits a feed: the generation under way is dropped.
        b.user(Message::Edit(Target::Operation, "feed", "777".into()));
        assert!(b.app.generating.is_none());
        b.release();
        let done = b.call("wait", json!({"command_id": id}));
        let [e] = &theirs(&done)[..] else {
            panic!("{done}")
        };
        assert_eq!(e.source, Source::Mixed);
        assert_eq!(e.events, ["user: edit_field operation feed"]);
        assert!(
            done["notes"][0]
                .as_str()
                .unwrap()
                .contains("user: edit_field operation feed"),
            "{done}"
        );
        let feed = crate::model::get(&b.app.doc.setups[0].operations[0].p, "feed");
        assert_eq!(feed, "777");
    }

    #[test]
    fn a_stale_generation_is_not_applied_after_a_user_edit() {
        let mut b = Bench::imported();
        b.call("inspect", json!({}));
        // Unheld, it is applied.
        let done = b.run("generate", json!({"operation": 0}));
        assert_eq!(done["outcome"], "done", "{done}");
        assert!(b.app.simulation.is_some());
        // Its result come after a user edit: not applied.
        b.hold = true;
        let first = b.call(
            "execute",
            json!({"command": "generate", "args": {"operation": 0}, "wait_s": 0}),
        );
        assert_eq!(first["outcome"], "still_running", "{first}");
        assert!(b.app.simulation.is_none());
        let id = first["command_id"].as_str().unwrap().to_string();
        b.user(Message::Edit(Target::Operation, "feed", "778".into()));
        b.release();
        assert!(b.app.simulation.is_none(), "a stale generation was applied");
        let done = b.call("wait", json!({"command_id": id}));
        assert_ne!(done["outcome"], "done", "{done}");
        let [e] = &theirs(&done)[..] else {
            panic!("{done}")
        };
        assert_eq!(e.source, Source::Mixed);
        assert_eq!(e.events, ["user: edit_field operation feed"]);
    }

    #[test]
    fn a_generation_the_user_started_is_theirs_when_it_lands_in_a_window() {
        let mut b = Bench::imported();
        b.call("inspect", json!({}));
        b.hold = true;
        b.user(Message::Generate);
        assert!(b.app.generating.is_some(), "{}", b.app.status);
        // A command of the agent's opens a window; the user's generation
        // finishes in it, and is named as theirs.
        let r = b.call("execute", json!({"command": "toggle_stock", "wait_s": 0}));
        assert_eq!(r["outcome"], "still_running", "{r}");
        let id = r["command_id"].as_str().unwrap().to_string();
        b.release();
        assert!(b.app.simulation.is_some());
        assert_eq!(b.app.mcp_events(), ["user: generation finished"]);
        let done = b.call("wait", json!({"command_id": id}));
        assert_ne!(done["outcome"], "still_running", "{done}");
        // One the agent started is not: its result is the agent's.
        let r = b.run("generate", json!({"operation": 0}));
        assert_eq!(r["outcome"], "done", "{r}");
        assert!(b.app.mcp_events().is_empty());
    }

    /// The sample with a simulated UC100 in its library, connected as the
    /// user connects it.
    fn connected() -> Bench {
        let mut b = Bench::sample();
        let profile = b.dir.path().join("uc100.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let mut m = crate::model::Machine::new(4);
        m.p.insert("name".into(), "JY5800".into());
        m.p.insert("controller".into(), "UC100".into());
        m.p.insert("uc100_profile".into(), profile.display().to_string());
        m.p.insert("uc100_connection".into(), "Simulated".into());
        b.app.doc.machines.push(m);
        b.user(Message::Connect);
        assert!(b.app.control.is_some(), "{}", b.app.status);
        b
    }

    #[test]
    fn a_pin_reference_recorded_through_tick_is_the_machines() {
        let mut b = connected();
        b.call("inspect", json!({}));
        let id = b.app.control.as_ref().unwrap().machine_id.clone();
        // Outside a window: the user's work at the machine, found by the
        // next read.
        let c = b.app.control.as_mut().unwrap();
        c.recorded = Some([12.5, 34.25]);
        c.rotary_keep = Some(crate::control::RotaryKeep::Centreline(
            [1., 2.],
            [3., 4., 5.],
        ));
        b.tick();
        let read = b.call("inspect", json!({}));
        let [e] = &theirs(&read)[..] else {
            panic!("{read}")
        };
        assert_eq!(e.source, Source::User);
        assert_eq!(names(e), [("machine".into(), id.clone())]);
        assert_eq!(
            e.events,
            [
                "machine: pin reference recorded",
                "machine: rotary centreline saved"
            ]
        );
        // Queued before an agent request, applied in its window: mixed,
        // and named.
        b.app.control.as_mut().unwrap().recorded = Some([50., 60.]);
        let r = b.run("set_project_title", json!({"title": "Pinned"}));
        assert_eq!(r["outcome"], "done", "{r}");
        let [e] = &theirs(&r)[..] else { panic!("{r}") };
        assert_eq!(e.source, Source::Mixed);
        assert_eq!(e.events, ["machine: pin reference recorded"]);
        assert!(has(e, "project") && names(e).contains(&("machine".into(), id.clone())));
        let kept = &b.app.doc.machines.iter().find(|m| m.id == id).unwrap().p;
        assert_eq!(crate::model::get(kept, "pin_ref_x"), "50.000");
        b.app.disconnect_machine();
    }

    /// The machinist's records on setup `si` and its machine.
    fn record(app: &mut App, si: usize, at: usize) {
        let id = app.doc.setups[si].id.clone();
        app.doc.guide.insert(id.clone(), at);
        app.doc
            .measured
            .insert(crate::guide::measure_key(&id, "zero"), 30. + at as f32);
        app.doc.setups[si]
            .machine
            .p
            .insert(DIAL.into(), "Diameter".into());
        let machine = app.doc.setups[si].machine.id.clone();
        if let Some(m) = app.doc.machines.iter_mut().find(|m| m.id == machine) {
            m.p.insert(DIAL.into(), "Diameter".into());
        }
    }

    #[test]
    fn a_command_that_changes_the_machinists_records_fails_and_they_are_put_back() {
        let mut b = Bench::sample();
        record(&mut b.app, 0, 2);
        let before = Records::of(&b.app.doc);
        b.call("inspect", json!({}));
        b.bug = Some(|app| {
            let id = app.doc.setups[0].id.clone();
            app.doc.guide.insert(id, 9);
            app.doc.measured.clear();
        });
        let r = b.run("set_project_title", json!({"title": "Buggy"}));
        assert_eq!(r["outcome"], "failed", "{r}");
        let why = r["reason"].as_str().unwrap();
        assert!(why.starts_with(PROGRESS), "{why}");
        assert!(why.contains("step guide's place on setup 1"), "{why}");
        assert_eq!(Records::of(&b.app.doc), before);
        assert_eq!(b.app.doc.title, "Buggy");
    }

    #[test]
    fn records_on_other_setups_stay_through_remove_and_duplicate_setup() {
        let mut b = Bench::sample();
        for _ in 0..2 {
            assert_eq!(b.run("add_setup", json!({}))["outcome"], "done");
        }
        record(&mut b.app, 1, 3);
        record(&mut b.app, 2, 4);
        let ids: Vec<String> = b.app.doc.setups.iter().map(|s| s.id.clone()).collect();
        let r = b.run("remove_setup", json!({"setup": ids[0]}));
        assert_eq!(r["outcome"], "done", "{r}");
        let r = b.run("duplicate_setup", json!({"setup": ids[1]}));
        assert_eq!(r["outcome"], "done", "{r}");
        assert_eq!(b.app.doc.setups.len(), 3);
        for (id, at) in [(&ids[1], 3), (&ids[2], 4)] {
            assert_eq!(b.app.doc.guide.get(id), Some(&at));
            let key = crate::guide::measure_key(id, "zero");
            assert_eq!(b.app.doc.measured.get(&key), Some(&(30. + at as f32)));
        }
        // Removing the setup with records takes them with it, allowed.
        let r = b.run("remove_setup", json!({"setup": ids[2]}));
        assert_eq!(r["outcome"], "done", "{r}");
    }

    #[test]
    fn new_and_open_project_bring_their_own_records_clean_or_dirty() {
        for dirty in [false, true] {
            for command in ["new_project", "open_project"] {
                let mut b = Bench::sample();
                record(&mut b.app, 0, 2);
                let path = b.dir.path().join("job.axis.json");
                crate::storage::save(&path, &b.app.doc).unwrap();
                b.app.project_path = Some(path);
                // Unsaved is the job differing from its file by value.
                if dirty {
                    b.app.doc.title = "Changed since saved".into();
                }
                b.app.project_dirty = dirty;
                // The other project: the same setups, records of its own.
                let other = b.dir.path().join("other.axis.json");
                let mut doc = b.app.doc.clone();
                doc.guide.insert(doc.setups[0].id.clone(), 5);
                doc.measured.clear();
                doc.setups[0].machine.p.insert(DIAL.into(), "Radius".into());
                crate::storage::save(&other, &doc).unwrap();
                let args = |discard: bool| match command {
                    "open_project" => {
                        json!({"path": other.display().to_string(), "discard_unsaved": discard})
                    }
                    _ => json!({"discard_unsaved": discard}),
                };
                let mine = Records::of(&b.app.doc);
                let first = b.run(command, args(false));
                if dirty {
                    assert_eq!(first["outcome"], "refused", "{command}: {first}");
                    assert_eq!(Records::of(&b.app.doc), mine);
                    let second = b.run(command, args(true));
                    assert_eq!(second["outcome"], "done", "{command}: {second}");
                } else {
                    assert_eq!(first["outcome"], "done", "{command}: {first}");
                }
                match command {
                    "open_project" => assert_eq!(Records::of(&b.app.doc), Records::of(&doc)),
                    _ => assert!(b.app.doc.guide.is_empty() && b.app.doc.measured.is_empty()),
                }
            }
        }
    }

    /// A human Next on the manual mill's steps, and Measured on the lathe's,
    /// while the agent's generate_all is pending: kept, and reported.
    #[test]
    fn the_machinists_next_and_measured_during_generate_all_are_kept() {
        type Act = fn(&App) -> GuideAction;
        let cases: [(App, Act); 2] = [
            (crate::steps::tests::app("Yes"), |_| GuideAction::Next),
            (crate::steps::tests::lathe_app("Diameter", |_| {}), |a| {
                GuideAction::Measured(crate::guide::measure_key(&a.doc.setups[0].id, "zero"), 41.5)
            }),
        ];
        for (mut app, act) in cases {
            // The helpers build the guide in line, as the window's run
            // would have: none is still being built.
            app.guide_job = None;
            let mut b = Bench::new(app);
            b.call("inspect", json!({}));
            let before = Records::of(&b.app.doc);
            b.hold = true;
            let r = b.call("execute", json!({"command": "generate_all", "wait_s": 0}));
            assert_eq!(r["outcome"], "still_running", "{r}");
            let id = r["command_id"].as_str().unwrap().to_string();
            let action = act(&b.app);
            b.user(Message::Guide(action));
            let mid = Records::of(&b.app.doc);
            assert_ne!(mid, before, "{}", b.app.status);
            b.release();
            let done = b.call("wait", json!({"command_id": id}));
            let reason = done["reason"].as_str().unwrap_or_default();
            assert!(!reason.starts_with(PROGRESS), "{done}");
            assert_eq!(Records::of(&b.app.doc), mid, "kept");
            let [e] = &theirs(&done)[..] else {
                panic!("{done}")
            };
            assert_eq!(e.source, Source::Mixed);
            assert!(
                e.events.iter().any(|x| x.starts_with("user: Guide(")),
                "{e:?}"
            );
            let notes = done["notes"].to_string();
            assert!(notes.contains("kept as theirs"), "{done}");
        }
    }

    /// A job of about `bytes` saved: setups of operations whose fields
    /// hold long values, as a big real job's would.
    fn job_of(bytes: usize) -> App {
        let mut a = app();
        let template = a.doc.setups[0].clone();
        let mut setups = vec![];
        let mut size = serde_json::to_vec_pretty(&a.doc).unwrap().len();
        let mut n = 0;
        while size < bytes {
            let mut s = template.clone();
            s.id = crate::model::id();
            for o in &mut s.operations {
                o.id = crate::model::id();
                for k in 0..12 {
                    o.p.insert(
                        format!("note_{k}"),
                        format!("{n} {}", "machined by hand ".repeat(6)),
                    );
                }
            }
            size += serde_json::to_vec_pretty(&s).unwrap().len();
            setups.push(s);
            n += 1;
        }
        a.doc.setups.extend(setups);
        // Nested, each setup saves a little longer than alone.
        loop {
            let saved = serde_json::to_vec_pretty(&a.doc).unwrap().len();
            if saved <= bytes {
                return a;
            }
            let each = saved / a.doc.setups.len();
            let over = (saved - bytes) / each + 1;
            let keep = a.doc.setups.len() - over;
            a.doc.setups.truncate(keep);
        }
    }

    /// The snapshot's budget (the spec's *The work state and its
    /// snapshots*): a 1 MB project at most 15 ms, one at the 20 MB limit
    /// at most 250 ms, in a release build:
    /// `cargo test --release -- --ignored mcp_snapshot_bench`.
    #[test]
    #[ignore]
    fn mcp_snapshot_bench() {
        for (mb, budget_ms) in [(1usize, 15.), (20, 250.)] {
            let a = job_of(mb * 1_000_000);
            let saved = serde_json::to_vec_pretty(&a.doc).unwrap().len();
            assert!(saved <= 20_000_000 && saved > mb * 950_000, "{saved}");
            let first = snapshot(&a);
            let mut times = vec![];
            for _ in 0..7 {
                let t = std::time::Instant::now();
                let s = snapshot(&a);
                times.push(t.elapsed().as_secs_f64() * 1000.);
                assert_eq!(s.hash, first.hash);
            }
            times.sort_by(f64::total_cmp);
            let (best, median, worst) = (times[0], times[3], times[6]);
            // The hash alone over the saved bytes, for its speed.
            let bytes = serde_json::to_vec(&a.doc).unwrap();
            let t = std::time::Instant::now();
            let mut h = DefaultHasher::new();
            h.write(&bytes);
            std::hint::black_box(h.finish());
            let hash_ms = t.elapsed().as_secs_f64() * 1000.;
            eprintln!(
                "mcp_snapshot_bench: {:.1} MB saved, {} items: best {best:.1} ms, median {median:.1} ms, worst {worst:.1} ms (budget {budget_ms} ms); SipHash alone {:.0} MB/s",
                saved as f64 / 1e6,
                first.items.len(),
                bytes.len() as f64 / 1e6 / (hash_ms / 1000.),
            );
            if !cfg!(debug_assertions) {
                assert!(
                    median <= budget_ms,
                    "{mb} MB: {median:.1} ms > {budget_ms} ms"
                );
            }
        }
    }
}
