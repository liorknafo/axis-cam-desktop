mod analysis;
mod cam;
mod choice_diagram;
mod choice_help;
#[cfg(test)]
mod fake_openai;
mod help;
mod holding;
mod material;
mod mesh;
mod model;
mod ngc;
mod oauth;
mod openai;
mod plan_check;
mod planner;
mod pocket;
mod post;
mod post_check;
mod proposal;
mod rest;
mod session;
mod settings;
mod simulation;
mod smoke;
mod step;
mod stock;
mod stock_surface;
mod storage;
mod surface;
mod three_mf;
mod ui;
mod viewport;
mod volume;
use iced::{Element, Size, Subscription, Task, Theme};
use model::*;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Work,
    Machines,
    Tools,
    Materials,
    Workholding,
    Wizard,
    Ai,
    Post,
    Audit,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Operation,
    Setup,
    Machine,
    Tool,
    Material,
    Holder,
}
#[derive(Debug, Clone, PartialEq)]
pub enum Account {
    SignedOut,
    SigningIn,
    SignedIn {
        name: String,
        email: String,
        /// Whether the user allowed requests to use their ChatGPT plan.
        granted: bool,
    },
}
const NO_MODELS: &str = "Your ChatGPT plan lists no models for this app.";
/// The end of the note that says which model is used without a list.
const FALLBACK: &str = "Check connection shows whether it answers.";
const PLAN_STOPPED: &str = "Planning stopped because the job changed.";
const PREVIEW_DISCARDED: &str = "The plan preview was discarded because the job changed.";
const PLAN_SIGNED_OUT: &str = "Planning stopped because you are signed out.";
const PLAN_BUSY: &str = "The previous plan check is still finishing; try again in a moment.";
/// The model the planner was designed for; used when the list cannot be read.
const DEFAULT_MODEL: &str = "gpt-6.1-sol";
const NOT_GRANTED: &str =
    "ChatGPT plan usage was not granted. Sign out, then sign in again and allow plan usage.";
/// A loaded project, its path, and the notes of the load (`load_reporting`).
type Opened = Result<(Document, PathBuf, Vec<String>), String>;
#[derive(Debug, Clone)]
pub enum Message {
    NewProject,
    ResumeProject,
    OpenSample,
    RecoverPrevious,
    Page(Page),
    SelectSetup(usize),
    SelectOp(usize),
    SetupEditor,
    Tab(String),
    Edit(Target, &'static str, String),
    Title(String),
    AddOp(Strategy),
    DuplicateOp,
    RemoveOp,
    MoveOp(isize),
    ToggleOp,
    Feature(usize),
    Surface(usize),
    ClearSurfaces,
    PickMode,
    Generate,
    GenerateAll,
    AddSetup,
    DuplicateSetup,
    RemoveSetup,
    MoveSetup(isize),
    Flip(&'static str),
    AssignMachine(String),
    NewMachine,
    EditMachine(String),
    WizardStep(usize),
    SaveMachine,
    DeleteMachine(String),
    EditTool(String),
    NewTool,
    SaveTool,
    DeleteTool(String),
    EditMaterial(String),
    NewMaterial,
    SaveMaterial,
    DeleteMaterial(String),
    PickMaterial(String),
    /// A new library vise or clamp: its kind, one of `holding::KINDS`.
    NewHolder(&'static str),
    EditHolder(String),
    SaveHolder,
    DeleteHolder(String),
    /// The text of the clamp positions field of the active setup.
    SetClamps(String),
    /// Places the active setup's vise jaws or clamps away from its toolpaths.
    SuggestLayout,
    SetOpTool(String),
    RestSource(String),
    StockSource(String),
    SaveProject,
    OpenProject,
    OpenHistory,
    Import,
    FileImported(Option<Result<(ModelFile, Option<mesh::Mesh>), String>>),
    ProjectOpened(Option<Opened>),
    ModelRestored(Result<mesh::Mesh, String>),
    Saved(Option<Result<PathBuf, String>>),
    Export(&'static str),
    ToggleStock,
    TogglePaths,
    Camera(&'static str),
    Viewport(viewport::Event),
    /// Orbit, pan or zoom in the plan preview's 3D view.
    PlanViewport(viewport::Event),
    Play,
    Scrub(f32),
    Speed(f32),
    StockDetail(stock_surface::Detail),
    StockMeshed {
        key: StockMeshKey,
        seconds: f32,
        result: Result<std::sync::Arc<stock_surface::SurfaceMesh>, String>,
    },
    Tick,
    Plan,
    PlanProgress(u64, planner::Event),
    PlanFinished(u64, Result<std::sync::Arc<PlanPreview>, planner::Failure>),
    PlanCancel,
    ApplyPlan,
    DiscardPlan,
    UndoPlan,
    Exported(PathBuf),
    ExportError(String),
    PostChecked(u64, std::sync::Arc<post::JobPost>),
    ExportGcode,
    /// None when no folder was chosen.
    GcodeExported(Option<Result<(PathBuf, usize), String>>),
    AccountRestored(Result<Option<session::Session>, String>),
    SignIn,
    SignInCancel,
    SignedIn(Result<(session::Session, String), String>),
    /// The result of saving a finished sign-in; see `oauth::save_session`.
    SessionSaved(Result<session::Session, String>),
    SignOut,
    SignedOut(Result<(), String>),
    /// The first value is the `account_epoch` of the client that asked.
    ModelsLoaded(u64, Result<Vec<String>, openai::ApiError>),
    LoadModels,
    PickModel(String),
    CheckConnection,
    /// The first value is the `account_epoch` of the client that asked.
    ConnectionChecked(u64, Result<String, openai::ApiError>),
    PlanNoticeDismissed,
    OpenUrl(&'static str),
    ConfirmDeleteSetup(bool),
    Close(iced::window::Id),
    Captured(iced::window::Screenshot),
    HelpHover(bool),
    Noop,
}
/// A plan that passed every check, with the stock it leaves for the 3D view.
#[derive(Debug)]
pub struct PlanPreview {
    pub outcome: planner::Outcome,
    pub view: Option<simulation::Simulation>,
}
#[derive(Debug, Clone)]
pub enum Planner {
    Idle,
    Running(planner::Progress),
    Preview(std::sync::Arc<PlanPreview>),
    Failed(planner::Failure),
}
/// One stage of a planner run as the timeline shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub stage: planner::Progress,
    pub started: Instant,
    pub ended: Option<Instant>,
    /// How the reply of this request fared: on the checking step, or on the
    /// request step when the reply could not be read.
    pub result: Option<planner::StepResult>,
}
/// Clamp positions typed over a setup's `over` that `holding::layout` refuses.
#[derive(Debug, Clone, PartialEq)]
pub struct ClampsDraft {
    pub setup: String,
    pub text: String,
    pub over: String,
}
type StockMeshKey = (String, u64, u64, stock_surface::Detail);
/// A heading and the lines of the box the inspector shows.
pub type HoldingNote = Option<(String, Vec<String>)>;
pub struct App {
    pub smoke_dir: Option<PathBuf>,
    pub smoke_step: usize,
    pub help_animating: bool,
    pub smoke_pending: bool,
    pub doc: Document,
    pub page: Page,
    pub selected: Option<usize>,
    pub tab: String,
    pub status: String,
    pub audit_report: Vec<(String, bool, String)>,
    pub post: Option<std::sync::Arc<post::JobPost>>,
    pub post_busy: bool,
    /// The workholding warning of the preview shown (`ui::holding_warning`)
    /// and what it was worked out from.
    holding_note: std::cell::RefCell<Option<(String, HoldingNote)>>,
    post_epoch: u64,
    pub settings: settings::Settings,
    settings_path: PathBuf,
    pub account: Account,
    /// Latest sign-in or connection message for the account panel.
    pub account_note: String,
    /// A connection check is running.
    pub account_busy: bool,
    /// Why the new sign-in's client ID could not be remembered, until the
    /// sign-in is finished and can say so.
    settings_failure: Option<String>,
    /// The saved sign-in is still being deleted; signing in waits for it.
    pub sign_out_pending: bool,
    /// The saved sign-in is still being read; signing in waits for it, and
    /// only this read's result may fill the account.
    pub restoring: bool,
    /// Changes whenever the client does, so results of an earlier client's
    /// requests can be told apart and ignored.
    account_epoch: u64,
    pub usage_limit: bool,
    pub models: Vec<String>,
    pub(crate) endpoints: oauth::Endpoints,
    store: std::sync::Arc<dyn session::SessionStore>,
    client: Option<std::sync::Arc<openai::Client>>,
    sign_in: Option<iced::task::Handle>,
    account_restored: bool,
    pub machine_draft: Machine,
    pub wizard_step: usize,
    pub tool_draft: Tool,
    pub material_draft: material::Material,
    pub holder_draft: holding::Holder,
    /// What was typed in the active setup's clamp positions field while it
    /// does not read correctly; the setup keeps its last good text.
    pub clamps_draft: Option<ClampsDraft>,
    pub camera: viewport::Camera,
    pub sample_mesh: mesh::Mesh,
    pub imported_mesh: Option<mesh::Mesh>,
    pub import_busy: bool,
    pub save_busy: bool,
    pub last_archived: Option<PathBuf>,
    pub import_error: Option<String>,
    pub stock: bool,
    pub paths: bool,
    pub picking: bool,
    pub playing: bool,
    pub simulation: Option<simulation::Simulation>,
    pub stock_detail: stock_surface::Detail,
    stock_mesh_job: Option<StockMeshKey>,
    stock_mesh_failed: Option<StockMeshKey>,
    pub progress: f32,
    pub speed: f32,
    pub last_tick: Instant,
    pub dirty: bool,
    pub last_edit: Instant,
    pub state_path: PathBuf,
    pub project_path: Option<PathBuf>,
    pub project_dirty: bool,
    pub planner: Planner,
    plan_run: Option<iced::task::Handle>,
    plan_epoch: u64,
    /// The account epoch of the client the current run uses.
    plan_account_epoch: u64,
    /// The model the current run asks.
    pub plan_model: String,
    /// The stages of the current or last run, with their times and results;
    /// kept after the run for the preview or the failure.
    pub plan_timeline: Vec<Step>,
    /// Characters of the model's reply streamed so far in the current run.
    pub plan_received: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// Blocking stages still running, also those of cancelled runs.
    pub plan_busy: planner::Busy,
    /// Set by an edit that dropped a running plan or a preview, or by a
    /// sign-out that stopped a run: why the plan went.
    plan_dropped: Option<&'static str>,
    /// The plan preview's own camera: the machining viewport's stays put.
    pub plan_camera: viewport::Camera,
    pub undo: Option<Document>,
    pub confirm_delete_setup: bool,
    pub icons: Vec<iced::widget::svg::Handle>,
    /// A close request waiting for a sign-out, a sign-in save or a refresh
    /// of the sign-in, and when it first came.
    closing: Option<(iced::window::Id, Instant)>,
    /// The client a pending sign-out is signing out, so that a close that
    /// stops waiting for it can delete the saved sign-in through it.
    signing_out: Option<std::sync::Arc<openai::Client>>,
    /// G-code exports started and not yet reported (folder choice and write).
    gcode_exports: usize,
}
type PlanSender = iced::futures::channel::mpsc::Sender<Message>;
/// The Plan task: runs the planner, sends each stage as it starts, builds the
/// 3D view of a plan that passed and sends the result. Every message carries
/// the run's `epoch`, so the app can ignore those of a dropped run.
pub(crate) async fn run_plan(
    backend: &(impl planner::Backend + Sync),
    doc: Document,
    mesh: mesh::Mesh,
    busy: planner::Busy,
    epoch: u64,
    sender: PlanSender,
) {
    run_plan_with(backend, doc, mesh, busy, epoch, sender, planner::final_view).await
}
/// `run_plan` with the 3D view's blocking stage given, so tests can hold it.
pub(crate) async fn run_plan_with(
    backend: &(impl planner::Backend + Sync),
    doc: Document,
    mesh: mesh::Mesh,
    busy: planner::Busy,
    epoch: u64,
    mut sender: PlanSender,
    view: impl FnOnce(&Document, &mesh::Mesh) -> Option<simulation::Simulation> + Send + 'static,
) {
    use iced::futures::SinkExt;
    let mut stages = sender.clone();
    let result = planner::plan(backend, doc, mesh.clone(), &busy, move |event| {
        // Events are only shown; one that does not fit the channel is skipped.
        let _ = stages.try_send(Message::PlanProgress(epoch, event));
    })
    .await;
    let result = match result {
        Ok(outcome) => {
            let previewing = planner::Event::Stage(planner::Progress::Previewing);
            let _ = sender.send(Message::PlanProgress(epoch, previewing)).await;
            let job = outcome.job.clone();
            let view = planner::blocking(&busy, move || view(&job, &mesh))
                .await
                .ok()
                .flatten();
            Ok(std::sync::Arc::new(PlanPreview { outcome, view }))
        }
        Err(failure) => Err(failure),
    };
    let _ = sender.send(Message::PlanFinished(epoch, result)).await;
}
/// Orbit, pan or zoom `camera` as a viewport asks; picks do not move it.
fn steer(camera: &mut viewport::Camera, event: &viewport::Event) {
    match *event {
        viewport::Event::Orbit(x, y) => {
            camera.yaw += x;
            camera.pitch = (camera.pitch + y).clamp(-1.5, 1.57)
        }
        viewport::Event::Pan(x, y) => {
            camera.pan[0] += x;
            camera.pan[1] += y
        }
        viewport::Event::Zoom(v) => {
            camera.zoom = (camera.zoom * (1. + v).clamp(0.5, 1.5)).clamp(0.3, 4.)
        }
        viewport::Event::Pick(_) | viewport::Event::PickSurface(_) => {}
    }
}
impl App {
    fn new() -> Self {
        let smoke_dir = std::env::args()
            .nth(1)
            .filter(|a| a == "--smoke-test")
            .and_then(|_| std::env::args().nth(2))
            .map(PathBuf::from);
        let state_path = smoke_dir
            .as_ref()
            .map(|p| p.join("workspace.json"))
            .unwrap_or_else(storage::state_path);
        Self::from_workspace(state_path, smoke_dir)
    }
    fn from_workspace(mut state_path: PathBuf, smoke_dir: Option<PathBuf>) -> Self {
        #[cfg(test)]
        assert_ne!(
            state_path,
            storage::state_path(),
            "tests must use an isolated workspace"
        );
        // Tests and smoke runs keep sign-in data beside their own workspace,
        // never in the real settings file or the Keychain.
        let isolated = smoke_dir.is_some() || state_path != storage::state_path();
        let settings_path = settings::path(&state_path);
        let store = session::default_store(
            state_path.parent().unwrap_or(std::path::Path::new(".")),
            isolated,
        );
        // A load that changed something has notes; the workspace is then
        // saved once, so they are not repeated and what the load added, a
        // material for a legacy name, keeps its identifier.
        let (mut doc, status, migrated) = if state_path.exists() && smoke_dir.is_none() {
            match storage::load_reporting(&state_path) {
                Ok((d, notes)) => (
                    d,
                    with_notes(
                        "Choose New project, Open, or Continue your previous project.",
                        &notes,
                    ),
                    !notes.is_empty(),
                ),
                Err(e) => {
                    state_path =
                        state_path.with_file_name(format!("workspace-recovery-{}.json", id()));
                    (
                        Document::default(),
                        format!(
                            "{e} Original file preserved. A separate recovery workspace will be used."
                        ),
                        false,
                    )
                }
            }
        } else {
            (
                Document::default(),
                "Start a new project or open an existing one.".into(),
                false,
            )
        };
        // A blank workspace has the starter materials and workholding too.
        doc.adopt_materials();
        doc.adopt_holders();
        if !doc.sample_model {
            doc.sync_stock_sources();
        }
        let imported_mesh = None;
        let icons: Vec<_> = [
            include_bytes!("../assets/op-0.svg").as_slice(),
            include_bytes!("../assets/op-1.svg"),
            include_bytes!("../assets/op-2.svg"),
            include_bytes!("../assets/op-3.svg"),
            include_bytes!("../assets/op-4.svg"),
            include_bytes!("../assets/op-5.svg"),
            include_bytes!("../assets/op-6.svg"),
            include_bytes!("../assets/op-7.svg"),
            include_bytes!("../assets/op-8.svg"),
            include_bytes!("../assets/op-9.svg"),
            include_bytes!("../assets/op-10.svg"),
        ]
        .iter()
        .map(|b| iced::widget::svg::Handle::from_memory(b.to_vec()))
        .collect();
        Self {
            smoke_dir,
            smoke_step: 0,
            help_animating: false,
            smoke_pending: false,
            machine_draft: doc
                .machines
                .first()
                .cloned()
                .unwrap_or_else(|| Machine::new(3)),
            tool_draft: doc
                .tools
                .first()
                .cloned()
                .unwrap_or_else(|| blank_tool(&doc)),
            material_draft: material_draft(&doc),
            holder_draft: holder_draft(&doc),
            clamps_draft: None,
            doc,
            page: Page::Home,
            selected: None,
            tab: "Tool".into(),
            status,
            audit_report: vec![],
            post: None,
            post_busy: false,
            holding_note: std::cell::RefCell::new(None),
            post_epoch: 0,
            settings: settings::load(&settings_path),
            settings_path,
            account: Account::SignedOut,
            account_note: String::new(),
            account_busy: false,
            settings_failure: None,
            sign_out_pending: false,
            restoring: false,
            account_epoch: 0,
            usage_limit: false,
            models: vec![],
            endpoints: oauth::Endpoints::openai(),
            store,
            client: None,
            sign_in: None,
            account_restored: false,
            camera: viewport::Camera::default(),
            sample_mesh: mesh::sample(),
            imported_mesh,
            import_busy: false,
            save_busy: false,
            last_archived: None,
            import_error: None,
            stock: true,
            paths: true,
            picking: false,
            playing: false,
            simulation: None,
            stock_detail: Default::default(),
            stock_mesh_job: None,
            stock_mesh_failed: None,
            progress: 0.,
            speed: 1.,
            last_tick: Instant::now(),
            dirty: migrated,
            last_edit: Instant::now(),
            state_path,
            project_path: None,
            project_dirty: false,
            wizard_step: 0,
            planner: Planner::Idle,
            plan_run: None,
            plan_epoch: 0,
            plan_account_epoch: 0,
            plan_model: String::new(),
            plan_timeline: vec![],
            plan_received: Default::default(),
            plan_camera: viewport::Camera::default(),
            plan_busy: planner::Busy::default(),
            plan_dropped: None,
            undo: None,
            confirm_delete_setup: false,
            icons,
            closing: None,
            signing_out: None,
            gcode_exports: 0,
        }
    }
    fn boot() -> (Self, Task<Message>) {
        let mut app = Self::new();
        if app.smoke_dir.is_some() {
            let mut sample = Document::sample();
            sample.adopt_materials();
            sample.adopt_holders();
            app.replace_document(sample, None);
            app.selected = Some(0);
            if std::env::args().any(|arg| arg == "--help-only") {
                app.smoke_step = 14;
            }
        }
        (app, Task::none())
    }
    /// Archives the current project before a switch; Recover previous then
    /// reopens it.
    fn archive_current(&mut self) -> bool {
        match self.archive_copy() {
            Some(path) => {
                self.last_archived = Some(path);
                true
            }
            None => false,
        }
    }
    /// Keeps a copy of the current job in History. When that fails, the
    /// status says so and the caller must change nothing.
    fn archive_copy(&mut self) -> Option<PathBuf> {
        match storage::archive(&self.state_path, &self.doc) {
            Ok(path) => Some(path),
            Err(e) => {
                self.status = format!(
                    "Could not preserve your project: {e}. Your current work is unchanged."
                );
                None
            }
        }
    }
    fn replace_document(&mut self, doc: Document, path: Option<PathBuf>) {
        self.machine_draft = doc
            .machines
            .first()
            .cloned()
            .unwrap_or_else(|| Machine::new(3));
        self.tool_draft = doc
            .tools
            .first()
            .cloned()
            .unwrap_or_else(|| blank_tool(&doc));
        self.material_draft = material_draft(&doc);
        self.holder_draft = holder_draft(&doc);
        self.clamps_draft = None;
        self.doc = doc;
        if !self.is_sample() {
            self.doc.sync_stock_sources();
        }
        self.imported_mesh = None;
        self.import_error = None;
        self.project_path = path;
        self.project_dirty = self.project_path.is_none();
        self.page = Page::Work;
        self.audit_report.clear();
        self.invalidate_post();
        self.dirty = true;
        self.last_edit = Instant::now();
        self.camera = viewport::Camera::default();
        self.tab = "Tool".into();
        self.stop_selection();
        self.help_animating = false;
        self.drop_plan();
        self.undo = None;
    }
    fn create_setup(&mut self, machine: Machine) {
        let mut setup = Setup::new(machine, &self.doc.next_offset());
        setup.p.insert(
            "name".into(),
            format!("Setup {}", self.doc.setups.len() + 1),
        );
        if !self.doc.sample_model {
            for key in ["stock_x", "stock_y", "stock_z"] {
                setup.p.insert(key.into(), String::new());
            }
        }
        self.doc.active = setup.id.clone();
        self.doc.setups.push(setup);
        self.stop_selection();
        self.page = Page::Work;
        self.touch();
        self.status = "Setup created without operations. Set stock, origin and workholding, then add an operation.".into();
    }
    fn restore_model(&mut self) -> Task<Message> {
        self.imported_mesh = None;
        self.import_error = None;
        let Some(file) = &self.doc.file else {
            return Task::none();
        };
        let path = PathBuf::from(&file.path);
        self.import_busy = true;
        self.status = format!("Loading {}…", file.name);
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || mesh::load(&path))
                    .await
                    .unwrap_or_else(|e| Err(format!("Import worker failed: {e}")))
            },
            Message::ModelRestored,
        )
    }
    pub fn operation(&self) -> Option<&Operation> {
        self.doc
            .active_setup()
            .and_then(|s| self.selected.and_then(|i| s.operations.get(i)))
    }
    pub fn part_mesh(&self) -> Option<&mesh::Mesh> {
        if self.doc.file.is_some() {
            self.imported_mesh.as_ref()
        } else if self.doc.sample_model {
            Some(&self.sample_mesh)
        } else {
            None
        }
    }
    /// The active setup's clamp positions field: its text and what is wrong
    /// with it. The text is what was typed while that does not read (as long
    /// as the setup's own has not changed since), otherwise the setup's.
    pub fn clamps_field(&self) -> (String, Option<String>) {
        let setup = self.doc.setup();
        let saved = get(&setup.p, "clamps");
        match &self.clamps_draft {
            Some(d) if d.setup == setup.id && d.over == saved => {
                (d.text.clone(), holding::check_clamps(setup, &d.text).err())
            }
            _ => (saved.to_string(), None),
        }
    }
    /// The second sign-in step: the browser part is done and the session is being saved.
    pub fn saving_sign_in(&self) -> bool {
        self.account == Account::SigningIn && self.sign_in.is_none()
    }
    pub fn is_sample(&self) -> bool {
        self.doc.sample_model && self.doc.file.is_none()
    }
    pub fn can_pick(&self) -> bool {
        self.operation().is_some_and(|o| {
            self.is_sample()
                || (self.doc.file.is_some()
                    && o.needs_surfaces()
                    && o.strategy.imported_preview()
                    && self.imported_mesh.is_some())
        })
    }
    fn operation_mut(&mut self) -> Option<&mut Operation> {
        let selected = self.selected;
        self.doc
            .active_setup_mut()
            .and_then(|s| selected.and_then(|i| s.operations.get_mut(i)))
    }
    /// Places the active setup's vise jaws or clamps where its generated
    /// toolpaths stay clear of them, and says how the operations fare. The
    /// setup's operations up to its last enabled one are generated as
    /// Generate all does; those that cannot be are left out. Toolpaths are
    /// never changed. Without toolpaths the stock alone decides; without a
    /// stock nothing is written. Whether the setup changed (and was marked
    /// so): nothing is written or marked when the suggestion is what it has.
    fn suggest_layout(&mut self) -> bool {
        let si = self.doc.index();
        let setup = self.doc.setup();
        let request = match holding::request(setup, self.doc.holder(get(&setup.p, "holder"))) {
            Ok(request) => request,
            Err(error) => {
                self.status = error;
                return false;
            }
        };
        let mut generated: Vec<Option<simulation::Plan>> = vec![];
        let last = self.doc.setups[si]
            .operations
            .iter()
            .rposition(|o| o.enabled);
        if let (Some(mesh), Some(last)) = (
            self.imported_mesh.as_ref().filter(|_| !self.is_sample()),
            last,
        ) {
            simulation::sequence(&self.doc, mesh, Some((si, last)), |s, _, result| {
                if s == si {
                    generated.push(result.ok().map(|sim| sim.plan));
                }
            });
        }
        let (refs, failed) = holding::gathered(generated.iter().map(Option::as_ref));
        let Some((stock, layout)) = request.suggest(&self.doc, self.part_mesh(), si, &refs) else {
            self.status = "No layout suggested: this setup's stock is not known yet. Load the part and check the stock settings.".into();
            return false;
        };
        let changed = holding::place(&mut self.doc.setups[si], &layout);
        let id = &self.doc.setups[si].id;
        if !request.vise && self.clamps_draft.as_ref().is_some_and(|d| d.setup == *id) {
            self.clamps_draft = None;
        }
        // What is saved, to the hundredth, is what is checked and reported.
        let layout = holding::layout(&self.doc.setups[si]).unwrap_or(layout);
        let (holder, gap) = (&request.holder, request.gap);
        let boxes = holding::boxes(&layout, holder, stock).unwrap_or_default();
        let clashing = holding::clashes(&refs, &boxes, gap);
        let opening = holding::opening_issue(&layout, holder, stock);
        let message = holding::summary(&layout, refs.len(), clashing, failed, opening.as_deref());
        if changed {
            self.touch();
        }
        self.status = message;
        changed
    }
    fn touch(&mut self) {
        if !self.is_sample() {
            self.doc.sync_stock_sources();
        }
        self.audit_report.clear();
        self.invalidate_post();
        // A running plan and a preview describe the job as it was.
        match self.planner {
            Planner::Running(_) => self.plan_dropped = Some(PLAN_STOPPED),
            Planner::Preview(_) => self.plan_dropped = Some(PREVIEW_DISCARDED),
            _ => {}
        }
        self.drop_plan();
        self.dirty = true;
        self.project_dirty = true;
        self.last_edit = Instant::now();
        self.playing = false;
        self.simulation = None;
        self.progress = 0.;
        for s in &mut self.doc.setups {
            for o in &mut s.operations {
                o.preview = false
            }
        }
    }
    /// Blocking work that keeps the process alive after its window closes:
    /// iced's runtime waits for it on exit.
    fn blocking_work(&self) -> bool {
        self.plan_busy.running() > 0 || self.post_busy || self.stock_mesh_job.is_some()
    }
    /// Work that must not be cut short by ending the process: a G-code
    /// export. Its blocking part finishes on exit.
    fn finishing_work(&self) -> bool {
        self.gcode_exports > 0
    }
    /// What a close request does now (see `close_action`): a pending
    /// sign-out, sign-in save or refresh of the sign-in is waited for from
    /// the first request's time, `now` being this request's. After
    /// `CLOSE_WAIT`, a refresh still waiting for OpenAI no longer counts as
    /// pending, so blocking work ends the process; one saving its tokens
    /// still does, so the window closes and the save finishes. The wait is
    /// dropped unless the answer is to keep waiting.
    pub(crate) fn close_decision(&mut self, id: iced::window::Id, now: Instant) -> CloseAction {
        let client = self.client.as_ref();
        let (refreshing, saving_refresh) = (
            client.is_some_and(|c| c.refreshing()),
            client.is_some_and(|c| c.saving_refresh()),
        );
        let waited_out = self
            .closing
            .is_some_and(|(_, first)| now.saturating_duration_since(first) >= CLOSE_WAIT);
        let signing = self.sign_out_pending
            || self.saving_sign_in()
            || saving_refresh
            || (refreshing && !waited_out);
        let pending = signing.then(|| {
            let first = self.closing.get_or_insert((id, now)).1;
            (now.saturating_duration_since(first), self.sign_out_pending)
        });
        let action = close_action(
            !self.dirty,
            pending,
            self.blocking_work(),
            self.finishing_work(),
        );
        if action != CloseAction::Wait {
            self.closing = None;
        }
        action
    }
    /// Saves the workspace, then carries out `close_decision`: closes the
    /// window, waits, or ends the process.
    fn request_close(&mut self, id: iced::window::Id) -> Task<Message> {
        self.persist();
        match self.close_decision(id, Instant::now()) {
            CloseAction::Stay => {
                self.status = "Could not save before closing. Save Project to another location, then close again.".into();
                Task::none()
            }
            CloseAction::Wait => {
                self.status = if self.sign_out_pending {
                    "Closing when signing out has finished (at most 10 seconds)…"
                } else if self.saving_sign_in() {
                    "Closing when the sign-in has been saved (at most 10 seconds)…"
                } else {
                    "Closing when the sign-in refresh has finished (at most 10 seconds)…"
                }
                .into();
                Task::none()
            }
            CloseAction::Close => iced::window::close(id),
            // The sign-out may still wait behind a refresh for the session,
            // and the runtime drops it when the app ends. The saved sign-in
            // is deleted on the blocking pool instead, before the window
            // closes.
            CloseAction::ClearAndClose => {
                self.start_close_clear();
                iced::window::close(id)
            }
            // The workspace and History are written synchronously, so
            // nothing is lost.
            CloseAction::Exit => std::process::exit(0),
        }
    }
    /// Starts deleting the saved sign-in for a close that stopped waiting
    /// for a sign-out (`CloseAction::ClearAndClose`), on the blocking pool,
    /// which the runtime lets finish when the app ends: through the
    /// signing-out client, so that a refresh it still has running cannot
    /// save the sign-in again (`openai::Client::start_abandoning`), or from
    /// the store when there is no such client. Call it within a Tokio
    /// runtime: iced runs `update` inside its own.
    pub(crate) fn start_close_clear(&mut self) {
        match self.signing_out.take() {
            Some(client) => client.start_abandoning(),
            None => session::start_clear(self.store.clone()),
        }
    }
    fn close_if_waiting(&mut self) -> Task<Message> {
        match self.closing {
            Some((id, _)) => self.request_close(id),
            None => Task::none(),
        }
    }
    fn persist(&mut self) {
        if !self.dirty {
            return;
        }
        match storage::save(&self.state_path, &self.doc) {
            Ok(()) => {
                self.dirty = false;
            }
            Err(e) => {
                self.status = format!("Could not save the workspace: {e}");
                self.last_edit = Instant::now();
            }
        }
    }
    pub fn op_issues(&self) -> Vec<String> {
        if self.doc.active_setup().is_none() {
            return vec![];
        }
        self.selected
            .and_then(|i| {
                self.doc.setup().operations.get(i).map(|o| {
                    let mut issues = o.issues(
                        &self.doc.setup().machine,
                        &self.doc.tools,
                        &self.doc.setup().operations[..i],
                    );
                    if self.doc.file.is_some() {
                        issues.extend(surface::issues(
                            o,
                            self.doc.setup(),
                            self.imported_mesh.as_ref(),
                        ));
                    }
                    issues
                })
            })
            .unwrap_or_default()
    }
    pub fn job_issues(&self) -> Vec<String> {
        let mut issues = self.doc.issues();
        if self.doc.file.is_some() {
            for setup in &self.doc.setups {
                for operation in setup.operations.iter().filter(|o| o.enabled) {
                    issues.extend(surface::issues(
                        operation,
                        setup,
                        self.imported_mesh.as_ref(),
                    ));
                }
            }
        }
        issues
    }
    pub fn active_simulation(&self) -> Option<&simulation::Simulation> {
        self.simulation.as_ref().filter(|s| {
            self.operation()
                .is_some_and(|o| o.enabled && o.id == s.plan.operation)
                && self.doc.active == s.plan.setup
        })
    }
    pub fn playback_ready(&self) -> bool {
        self.active_simulation().is_some()
            || (self.is_sample()
                && self.operation().is_some_and(|o| o.enabled && o.preview)
                && self
                    .doc
                    .active_setup()
                    .is_some_and(|s| !s.is_repositioned()))
    }
    pub fn playback_reason(&self) -> String {
        let Some(operation) = self.operation() else {
            return "Select an operation to preview its motion.".into();
        };
        if !operation.enabled {
            return "This operation is suppressed. Enable it to preview.".into();
        }
        if self.is_sample() {
            if self.doc.setup().is_repositioned() {
                return "Sample animation is unavailable for repositioned setups.".into();
            }
            return "Generate the sample preview, then press Play.".into();
        }
        if !operation.strategy.imported_preview() {
            return operation.strategy.availability().into();
        }
        if let Some(issue) = self.op_issues().first() {
            return issue.clone();
        }
        ui::fixture_note(self, ui::Note::Playback)
    }
    fn stock_mesh_key(&self) -> Option<StockMeshKey> {
        let sim = self.active_simulation()?;
        let volume = sim.volume.as_ref()?;
        if sim.stock_surface.is_some()
            && sim.stock_surface_version == volume.version
            && sim.stock_surface_detail == self.stock_detail
        {
            return None;
        }
        Some((
            sim.id.clone(),
            sim.stock_epoch,
            volume.version,
            self.stock_detail,
        ))
    }
    fn refresh_stock_surface(&mut self) -> Task<Message> {
        if self.stock_mesh_job.is_some() {
            return Task::none();
        }
        let Some(key) = self.stock_mesh_key() else {
            return Task::none();
        };
        if self.stock_mesh_failed.as_ref() == Some(&key) {
            return Task::none();
        }
        let sim = self.active_simulation().unwrap();
        let volume = sim.volume.as_ref().unwrap();
        let snapshot = stock_surface::Snapshot {
            bounds: volume.bounds,
            frame: volume.frame,
            cuts: volume.cuts.clone(),
            detail: self.stock_detail,
        };
        let seconds = sim.seconds;
        self.stock_mesh_job = Some(key.clone());
        Task::perform(
            async move {
                let result =
                    tokio::task::spawn_blocking(move || snapshot.build().map(std::sync::Arc::new))
                        .await
                        .unwrap_or_else(|e| Err(format!("Stock mesher failed: {e}")));
                (key, seconds, result)
            },
            |(key, seconds, result)| Message::StockMeshed {
                key,
                seconds,
                result,
            },
        )
    }
    fn invalidate_post(&mut self) {
        self.post = None;
        self.post_epoch = self.post_epoch.wrapping_add(1);
    }
    /// Check and emit the whole job while the NC program page is open.
    fn refresh_post(&mut self) -> Task<Message> {
        if self.page != Page::Post || self.post.is_some() || self.post_busy {
            return Task::none();
        }
        let doc = self.doc.clone();
        let mesh = self.part_mesh().cloned();
        let run = move || match mesh {
            Some(mesh) => post::post_job(&doc, &mesh),
            None => post::JobPost::blocked("Load the imported part before posting."),
        };
        if self.smoke_dir.is_some() {
            self.post = Some(std::sync::Arc::new(run()));
            return Task::none();
        }
        let epoch = self.post_epoch;
        self.post_busy = true;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(run).await.unwrap_or_else(|e| {
                    post::JobPost::blocked(&format!("Postprocessor failed: {e}"))
                })
            },
            move |job| Message::PostChecked(epoch, std::sync::Arc::new(job)),
        )
    }
    fn write_settings(&self) -> Result<(), String> {
        settings::save(&self.settings_path, &self.settings)
    }
    fn save_settings(&mut self) {
        if let Err(e) = self.write_settings() {
            self.account_note = format!("Could not save settings: {e}");
        }
    }
    /// Use this session for requests and load the models its plan allows.
    fn adopt_session(&mut self, session: session::Session) -> Task<Message> {
        let granted = session.plan_usage_granted();
        self.account = Account::SignedIn {
            name: session.name.clone(),
            email: session.email.clone(),
            granted,
        };
        self.usage_limit = false;
        // A new client: results still on their way belong to the old one.
        self.account_epoch = self.account_epoch.wrapping_add(1);
        self.account_busy = false;
        let client = std::sync::Arc::new(openai::Client::new(
            self.endpoints.clone(),
            self.settings.client_id.clone(),
            self.store.clone(),
            session,
        ));
        self.client = Some(client);
        if !granted {
            self.account_note = NOT_GRANTED.into();
            return Task::none();
        }
        self.account_note.clear();
        self.load_models()
    }
    /// Ask which models the signed-in plan allows. The answer carries the
    /// epoch of the client that asked.
    fn load_models(&self) -> Task<Message> {
        let Account::SignedIn { granted: true, .. } = &self.account else {
            return Task::none();
        };
        let Some(client) = self.client.clone() else {
            return Task::none();
        };
        let epoch = self.account_epoch;
        Task::perform(async move { client.models().await }, move |result| {
            Message::ModelsLoaded(epoch, result)
        })
    }
    /// Stop a running plan and forget a preview: both describe the job as it was.
    fn drop_plan(&mut self) {
        if let Some(handle) = self.plan_run.take() {
            handle.abort();
        }
        self.plan_epoch = self.plan_epoch.wrapping_add(1);
        self.planner = Planner::Idle;
        self.plan_timeline.clear();
        // The aborted run may still hold the old count.
        self.plan_received = Default::default();
    }
    /// Ends the timeline's open step, if any, now.
    fn end_plan_step(&mut self) {
        if let Some(step) = self.plan_timeline.last_mut() {
            step.ended.get_or_insert_with(Instant::now);
        }
    }
    /// Adds a run's event to the timeline: a stage ends the open step and
    /// starts its own; a result joins the step of its request and ends it.
    fn record_plan_event(&mut self, event: planner::Event) {
        use planner::{Event, Progress, StepResult};
        match event {
            Event::Stage(stage) => {
                self.end_plan_step();
                self.plan_timeline.push(Step {
                    stage,
                    started: Instant::now(),
                    ended: None,
                    result: None,
                });
                self.planner = Planner::Running(stage);
            }
            Event::Checked(n, result) => {
                let reply = matches!(result, StepResult::Unreadable(_));
                let found = self
                    .plan_timeline
                    .iter_mut()
                    .rev()
                    .find(|step| match step.stage {
                        Progress::Checking(at) => !reply && at == n,
                        Progress::Asking(at)
                        | Progress::Repairing(at)
                        | Progress::Extending(at) => reply && at == n,
                        _ => false,
                    });
                if let Some(step) = found {
                    step.result = Some(result);
                    step.ended.get_or_insert_with(Instant::now);
                }
            }
        }
    }
    /// The planner's checklist: what must be in place before Plan is offered.
    pub fn plan_checklist(&self) -> Vec<analysis::Precondition> {
        let mut lines = analysis::preconditions(&self.doc, self.imported_mesh.as_ref());
        // The checks run on a copy whose setup 1 is named "Setup 1", so setup
        // 1's own settings, its name among them, are checked here as the
        // applied job will have them.
        if let Some(setup1) = self.doc.setups.first() {
            let issues = setup1.issues();
            match lines.iter_mut().find(|l| l.label.starts_with("Setup 1")) {
                Some(line) if line.ok && !issues.is_empty() => {
                    line.ok = false;
                    line.hint = issues.join(" ");
                }
                Some(_) => {}
                None => lines.push(analysis::Precondition {
                    label: "Setup 1",
                    ok: issues.is_empty(),
                    hint: issues.join(" "),
                }),
            }
        }
        let signed_in = matches!(self.account, Account::SignedIn { granted: true, .. })
            && self.client.is_some();
        let hint = if !signed_in {
            "Sign in with ChatGPT and allow plan usage."
        } else if self.settings.model.is_empty() {
            "Choose a model."
        } else {
            ""
        };
        lines.push(analysis::Precondition {
            label: "ChatGPT sign-in",
            ok: hint.is_empty(),
            hint: hint.into(),
        });
        lines
    }
    /// Without a model list the planner still needs a model: keep the saved
    /// one, or try the default, and say so. Check connection settles it.
    fn fall_back_to_default_model(&mut self) {
        if !matches!(self.account, Account::SignedIn { granted: true, .. }) {
            return;
        }
        if self.settings.model.is_empty() {
            self.settings.model = DEFAULT_MODEL.into();
            self.save_settings();
        }
        self.account_note = format!(
            "{} The app will use {}; {FALLBACK}",
            self.account_note, self.settings.model
        );
    }
    fn account_error(&mut self, error: openai::ApiError) {
        self.usage_limit = error == openai::ApiError::UsageLimit;
        if matches!(error, openai::ApiError::SignedOut(_)) {
            // A running plan uses the client that is gone, as on Sign out.
            if matches!(self.planner, Planner::Running(_)) {
                self.drop_plan();
                self.plan_dropped = Some(PLAN_SIGNED_OUT);
            }
            self.account = Account::SignedOut;
            self.client = None;
            self.models.clear();
            // Any check still running belonged to the client that is gone.
            self.account_epoch = self.account_epoch.wrapping_add(1);
            self.account_busy = false;
        }
        self.account_note = error.message();
    }
    /// Read the saved session the first time the planner page opens. The
    /// Keychain may ask for permission, so this never runs at launch. The
    /// first visit uses it up, whether or not there was anything to read:
    /// signing in and out later never reads the saved session behind it.
    /// The read can take as long as that permission dialog stays open, so
    /// `restoring` keeps the user from signing in until it has reported.
    fn restore_account(&mut self) -> Task<Message> {
        if std::mem::replace(&mut self.account_restored, true) || self.settings.client_id.is_empty()
        {
            return Task::none();
        }
        self.restoring = true;
        let store = self.store.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || store.load())
                    .await
                    .unwrap_or_else(|e| Err(format!("Could not read the saved sign-in: {e}")))
            },
            Message::AccountRestored,
        )
    }
    fn stop_selection(&mut self) {
        self.selected = None;
        self.picking = false;
        self.playing = false;
        self.progress = 0.;
        self.simulation = None;
        self.confirm_delete_setup = false;
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        let before = self.status.clone();
        let task = self.handle(message);
        // A message that dropped a running plan or a preview says why. When
        // its handler set a status of its own, both are shown, its own first.
        if let Some(dropped) = self.plan_dropped.take() {
            self.status = if self.status != before && !self.status.is_empty() {
                format!("{} {dropped}", self.status)
            } else {
                dropped.into()
            };
        }
        task
    }
    fn handle(&mut self, message: Message) -> Task<Message> {
        // These actions require an explicitly created setup; no hidden defaults.
        if self.doc.active_setup().is_none()
            && matches!(
                &message,
                Message::SelectOp(_)
                    | Message::Edit(Target::Setup, _, _)
                    | Message::AddOp(_)
                    | Message::DuplicateOp
                    | Message::RemoveOp
                    | Message::MoveOp(_)
                    | Message::Generate
                    | Message::GenerateAll
                    | Message::DuplicateSetup
                    | Message::RemoveSetup
                    | Message::ConfirmDeleteSetup(_)
                    | Message::MoveSetup(_)
                    | Message::Flip(_)
                    | Message::StockSource(_)
                    | Message::SetClamps(_)
                    | Message::SuggestLayout
                    | Message::Plan
            )
        {
            self.status =
                "Create a setup with your saved machine before adding or previewing operations."
                    .into();
            self.page = Page::Work;
            return Task::none();
        }
        if (self.import_busy || self.save_busy)
            && matches!(
                &message,
                Message::NewProject
                    | Message::OpenSample
                    | Message::RecoverPrevious
                    | Message::ResumeProject
            )
        {
            self.status =
                "Finish or cancel the current file action before switching projects.".into();
            return Task::none();
        }
        match message {
            Message::NewProject => {
                if !self.archive_current() {
                    return Task::none();
                }
                let mut doc = Document {
                    machines: self.doc.machines.clone(),
                    tools: self.doc.tools.clone(),
                    materials: self.doc.materials.clone(),
                    workholding: self.doc.workholding.clone(),
                    ..Document::default()
                };
                doc.adopt_materials();
                doc.adopt_holders();
                self.replace_document(doc, None);
                self.status = "Blank project created. Saved machines, tools, materials and workholding are available; your previous project is in History.".into();
            }
            Message::ResumeProject => {
                self.page = Page::Work;
                self.status = "Continue your project, or use New project to start again.".into();
                if self.imported_mesh.is_none() {
                    return self.restore_model();
                }
            }
            Message::OpenSample => {
                if !self.archive_current() {
                    return Task::none();
                }
                let (doc, status) = merge_sample(&self.doc);
                self.replace_document(doc, None);
                self.selected = Some(0);
                self.status = status;
            }
            Message::RecoverPrevious => {
                if let Some(path) = self.last_archived.clone() {
                    let result = storage::load_reporting(&path).map(|(d, n)| (d, path, n));
                    return self.update(Message::ProjectOpened(Some(result)));
                }
            }
            Message::Captured(image) => {
                return self.capture_smoke(image);
            }
            Message::Noop => {}
            Message::Page(p) => {
                self.help_animating = false;
                self.page = p;
                self.picking = false;
                self.playing = false;
                let restore = if p == Page::Ai {
                    self.restore_account()
                } else {
                    Task::none()
                };
                return Task::batch([self.refresh_post(), restore]);
            }
            Message::SelectSetup(i) => {
                if let Some(s) = self.doc.setups.get(i) {
                    self.doc.active = s.id.clone();
                    self.stop_selection();
                    self.page = Page::Work;
                    self.dirty = true;
                    self.last_edit = Instant::now();
                }
            }
            Message::SelectOp(i) => {
                if i < self.doc.setup().operations.len() {
                    if self.selected != Some(i) {
                        self.simulation = None;
                    }
                    self.selected = Some(i);
                    self.page = Page::Work;
                    self.playing = false;
                    self.progress = 0.;
                    if let Some(simulation) = &mut self.simulation {
                        simulation.seek(0.);
                    }
                    if !self
                        .operation()
                        .unwrap()
                        .strategy
                        .tabs()
                        .contains(&self.tab.as_str())
                    {
                        self.tab = "Tool".into();
                    }
                    self.picking = self.tab == "Geometry" && self.can_pick();
                }
            }
            Message::SetupEditor => {
                self.selected = None;
                self.page = Page::Work;
                self.picking = false;
                self.playing = false;
                self.simulation = None;
                self.progress = 0.;
            }
            Message::Tab(t) => {
                self.picking = t == "Geometry" && self.can_pick();
                if self.picking {
                    self.playing = false;
                    self.simulation = None;
                    self.progress = 0.;
                }
                self.tab = t;
            }
            Message::Edit(target, k, v) => match target {
                Target::Operation => {
                    if let Some(o) = self.operation_mut() {
                        o.p.insert(k.into(), v);
                    }
                    self.touch()
                }
                Target::Setup => {
                    // A pick_list repeats the chosen item, which may be the
                    // default it shows for an empty value: not a change.
                    let shown = match (k, get(&self.doc.setup().p, k)) {
                        ("holding", "") => "None",
                        ("vise_axis", "") => "X",
                        (_, value) => value,
                    };
                    if matches!(k, "holding" | "holder" | "vise_axis") && shown == v {
                        return Task::none();
                    }
                    let holder = get(&self.doc.setup().p, "holder").to_string();
                    self.doc.setup_mut().p.insert(k.into(), v);
                    if matches!(k, "holding" | "holder") {
                        self.doc.settle_holding(self.doc.index());
                    }
                    // Only the first Vise or Clamps and a different library
                    // entry place the jaws or clamps; later edits are kept.
                    let p = &self.doc.setup().p;
                    let placed = |key| !get(p, key).trim().is_empty();
                    let suggest = match (k, get(p, "holding")) {
                        ("holding", "Vise") => !placed("vise_axis"),
                        ("holding", "Clamps") => !placed("clamps"),
                        ("holder", "Vise" | "Clamps") => get(p, "holder") != holder,
                        _ => false,
                    };
                    if !suggest || !self.suggest_layout() {
                        self.touch()
                    }
                }
                Target::Machine => {
                    self.machine_draft.p.insert(k.into(), v.clone());
                    if k == "axes" {
                        let axes = self.machine_draft.axes();
                        self.machine_draft.p.insert(
                            "mode".into(),
                            match axes {
                                4 => "Indexed (3+1)",
                                5 => "Indexed (3+2)",
                                _ => "3-axis",
                            }
                            .into(),
                        );
                        self.machine_draft.p.insert(
                            "arrangement".into(),
                            match axes {
                                4 => "A table around X",
                                5 => "A/C table-table",
                                _ => "Fixed spindle",
                            }
                            .into(),
                        );
                    }
                }
                Target::Tool => {
                    self.tool_draft.p.insert(k.into(), v);
                }
                Target::Material => {
                    self.material_draft.p.insert(k.into(), v);
                }
                Target::Holder => {
                    self.holder_draft.p.insert(k.into(), v);
                }
            },
            Message::Title(v) => {
                self.doc.title = v;
                self.touch()
            }
            Message::AddOp(strategy) => {
                if !self.is_sample() && !strategy.imported_preview() {
                    self.stop_selection();
                    self.status = strategy.availability().into();
                    self.page = Page::Audit;
                    return Task::none();
                }
                let Some(tool) = self
                    .doc
                    .tools
                    .iter()
                    .find(|t| {
                        if strategy == Strategy::Drill {
                            get(&t.p, "type") == "Drill"
                        } else if strategy == Strategy::Parallel {
                            get(&t.p, "type") == "Ball end mill"
                        } else {
                            get(&t.p, "type") == "Flat end mill"
                                || (strategy == Strategy::Face && get(&t.p, "type") == "Face mill")
                        }
                    })
                    .or(self.doc.tools.first())
                else {
                    self.page = Page::Tools;
                    self.status = "Create and save a cutter first, then add your operation.".into();
                    return Task::none();
                };
                let mut operation = Operation::new(strategy, tool.id.clone());
                if !self.is_sample() {
                    operation.geometry.clear();
                }
                self.doc.setup_mut().operations.push(operation);
                self.selected = Some(self.doc.setup().operations.len() - 1);
                self.tab = "Tool".into();
                self.page = Page::Work;
                self.picking = false;
                self.touch();
                self.status = format!(
                    "Added {}. Choose a tool and machining geometry.",
                    strategy.name()
                );
            }
            Message::DuplicateOp => {
                if let Some(i) = self.selected {
                    let mut o = self.doc.setup().operations[i].clone();
                    o.id = id();
                    o.p.insert("name".into(), format!("{} · copy", o.name()));
                    self.doc.setup_mut().operations.insert(i + 1, o);
                    self.selected = Some(i + 1);
                    self.touch()
                }
            }
            Message::RemoveOp => {
                if let Some(i) = self.selected {
                    self.doc.setup_mut().operations.remove(i);
                    self.selected = if self.doc.setup().operations.is_empty() {
                        None
                    } else {
                        Some(i.min(self.doc.setup().operations.len() - 1))
                    };
                    self.touch()
                }
            }
            Message::MoveOp(d) => {
                if let Some(i) = self.selected {
                    let n = i as isize + d;
                    if n >= 0 && (n as usize) < self.doc.setup().operations.len() {
                        self.doc.setup_mut().operations.swap(i, n as usize);
                        self.selected = Some(n as usize);
                        self.touch()
                    }
                }
            }
            Message::ToggleOp => {
                if let Some(o) = self.operation_mut() {
                    o.enabled = !o.enabled;
                }
                self.touch()
            }
            Message::Feature(n) => {
                if self.is_sample()
                    && let Some(o) = self.operation_mut()
                {
                    o.toggle_feature(n)
                } else {
                    return Task::none();
                }
                self.touch()
            }
            Message::Surface(n) => {
                if self.doc.file.is_none()
                    || !self
                        .operation()
                        .is_some_and(|o| o.strategy.imported_preview())
                {
                    return Task::none();
                }
                let Some(mesh) = self.imported_mesh.as_ref() else {
                    return Task::none();
                };
                if n >= mesh.surfaces.len() {
                    return Task::none();
                }
                let fingerprint = mesh.fingerprint.clone();
                if let Some(o) = self.operation_mut() {
                    let selected = o.surfaces.get_or_insert_with(|| surface::Selection {
                        mesh_fingerprint: fingerprint.clone(),
                        ids: vec![],
                    });
                    if selected.mesh_fingerprint != fingerprint {
                        selected.mesh_fingerprint = fingerprint;
                        selected.ids.clear();
                    }
                    if selected.ids.contains(&n) {
                        selected.ids.retain(|id| *id != n);
                    } else {
                        selected.ids.push(n);
                    }
                    o.geometry.clear();
                }
                self.touch();
            }
            Message::ClearSurfaces => {
                if let Some(o) = self.operation_mut() {
                    o.surfaces = None;
                }
                self.touch();
            }
            Message::PickMode => {
                self.picking = !self.picking && self.can_pick();
                if self.picking {
                    self.playing = false;
                    self.simulation = None;
                    self.progress = 0.;
                }
                self.tab = "Geometry".into();
            }
            Message::Generate => {
                let mut issues = self.doc.setup().issues();
                issues.extend(self.op_issues());
                if !issues.is_empty() {
                    self.status = issues[0].clone();
                } else if !self.is_sample() {
                    self.playing = false;
                    self.simulation = None;
                    let result = self
                        .imported_mesh
                        .as_ref()
                        .ok_or_else(|| "Load the part first.".to_string())
                        .and_then(|mesh| {
                            simulation::prepare(
                                &self.doc,
                                mesh,
                                self.doc.index(),
                                self.selected.ok_or("Select an operation first.")?,
                            )
                        });
                    match result {
                        Ok(sim) => {
                            let plan = &sim.plan;
                            self.status = format!(
                                "{} preview ready: {} levels, {:.1} min estimated. Inherits stock after {} earlier operations across setups. Sampled removal; {} Export G-code from the NC program page.",
                                plan.strategy.name(),
                                plan.levels,
                                plan.duration / 60.,
                                sim.preceding,
                                ui::fixtures(ui::Note::Preview, ui::modelled(self))
                            );
                            self.simulation = Some(sim);
                            self.progress = 0.;
                            self.speed = 30.;
                            self.paths = true;
                            self.stock = true;
                            self.picking = false;
                        }
                        Err(error) => self.status = error,
                    }
                } else if self.doc.setup().is_repositioned() {
                    self.status="Setup saved. Toolpath computation for repositioned parts will require the CAM engine.".into()
                } else if let Some(o) = self.operation_mut() {
                    o.preview = true;
                    self.paths = true;
                    self.status =
                        "Illustrative sample path ready. No machining calculation was performed."
                            .into();
                }
            }
            Message::GenerateAll => {
                if !self.is_sample() {
                    self.audit_report.clear();
                    if let Some(mesh) = &self.imported_mesh {
                        let mut report = vec![];
                        simulation::sequence(&self.doc, mesh, None, |si, oi, result| {
                            let setup = &self.doc.setups[si];
                            let op = &setup.operations[oi];
                            let (ok, detail) = match result {
                                Ok(sim) => (
                                    true,
                                    format!(
                                        "Generated {} moves · {:.1} min · inherits stock after {} operations",
                                        sim.plan.moves.len(),
                                        sim.plan.duration / 60.,
                                        sim.preceding
                                    ),
                                ),
                                Err(error) => (false, error),
                            };
                            report.push((format!("{} / {}", setup.name(), op.name()), ok, detail));
                        });
                        self.audit_report = report;
                    } else {
                        self.status = "Load the imported part first.".into();
                        self.page = Page::Audit;
                        return Task::none();
                    }
                    self.status = format!(
                        "Checked {} enabled operations: {} generated, {} need attention. Remaining stock is carried through operations and setups.",
                        self.audit_report.len(),
                        self.audit_report.iter().filter(|r| r.1).count(),
                        self.audit_report.iter().filter(|r| !r.1).count()
                    );
                    self.page = Page::Audit;
                    return Task::none();
                }
                let issues = self.job_issues();
                if !issues.is_empty() {
                    self.status = format!("{} configuration issue(s). {}", issues.len(), issues[0]);
                } else if self.doc.setup().is_repositioned() || !self.is_sample() {
                    self.status="Configuration checked. Select a supported operation and generate its cumulative stock preview.".into()
                } else {
                    for o in &mut self.doc.setup_mut().operations {
                        o.preview = o.enabled
                    }
                    self.paths = true;
                    self.status =
                        "Enabled operations have sample previews. Configuration checks passed."
                            .into();
                }
            }
            Message::AddSetup => {
                if let Some(setup) = self.doc.active_setup() {
                    self.create_setup(setup.machine.clone());
                } else if self.doc.machines.is_empty() {
                    self.status =
                        "Set up and save your machine, then choose Create first setup.".into();
                    return self.update(Message::NewMachine);
                } else {
                    self.page = Page::Machines;
                    self.status = "Choose a saved machine and click Create first setup.".into();
                }
            }
            Message::DuplicateSetup => {
                let s = self.doc.setup().duplicate(&self.doc.next_offset());
                self.doc.active = s.id.clone();
                self.doc.setups.push(s);
                self.stop_selection();
                self.page = Page::Work;
                self.touch()
            }
            Message::RemoveSetup => {
                self.confirm_delete_setup = true;
            }
            Message::ConfirmDeleteSetup(yes) => {
                self.confirm_delete_setup = false;
                if yes {
                    let i = self.doc.index();
                    match self.doc.remove_setup(i) {
                        Ok(()) => {
                            self.stop_selection();
                            self.touch()
                        }
                        Err(e) => self.status = e,
                    }
                }
            }
            Message::MoveSetup(delta) => {
                if let Err(e) = self.doc.move_setup(self.doc.index(), delta) {
                    self.status = e
                } else {
                    self.touch()
                }
            }
            Message::Flip(axis) => {
                self.doc
                    .setup_mut()
                    .p
                    .insert(axis.into(), if axis == "ry" { "90" } else { "180" }.into());
                self.doc
                    .setup_mut()
                    .p
                    .insert("reposition".into(), "Reclamp and probe origin".into());
                self.touch();
            }
            Message::AssignMachine(id) => {
                if let Some(m) = self.doc.machines.iter().find(|m| m.id == id).cloned() {
                    if let Some(setup) = self.doc.active_setup_mut() {
                        setup.machine = m;
                        self.touch();
                        self.status = "Machine profile snapshot assigned to this setup.".into();
                    } else {
                        self.create_setup(m);
                    }
                }
            }
            Message::NewMachine => {
                self.machine_draft = Machine::new(3);
                self.machine_draft
                    .p
                    .insert("name".into(), "New machine".into());
                self.wizard_step = 0;
                self.page = Page::Wizard;
            }
            Message::EditMachine(id) => {
                if let Some(m) = self.doc.machines.iter().find(|m| m.id == id) {
                    self.machine_draft = m.clone();
                    self.machine_draft.fill_post_defaults();
                    self.wizard_step = 0;
                    self.page = Page::Wizard;
                }
            }
            Message::WizardStep(s) => self.wizard_step = s.min(4),
            Message::SaveMachine => {
                let errors = self.machine_draft.issues();
                if errors.is_empty() {
                    let mut m = self.machine_draft.clone();
                    if let Some(i) = self.doc.machines.iter().position(|x| x.id == m.id) {
                        m.version += 1;
                        self.doc.machines[i] = m;
                    } else {
                        self.doc.machines.push(m);
                    }
                    self.page = Page::Machines;
                    self.touch();
                    self.status="Machine profile saved. Use Assign to setup to update an existing setup snapshot.".into();
                } else {
                    self.status = errors.join(" ");
                }
            }
            Message::DeleteMachine(id) => {
                self.doc.machines.retain(|m| m.id != id);
                self.touch();
                self.status =
                    "Library profile removed; existing setup snapshots are preserved.".into();
            }
            Message::EditTool(id) => {
                if let Some(t) = self.doc.tools.iter().find(|t| t.id == id) {
                    self.tool_draft = t.clone();
                    self.page = Page::Tools
                }
            }
            Message::NewTool => {
                let mut tool = Tool::new("New tool", "Flat end mill", "6");
                tool.p
                    .insert("number".into(), self.doc.next_tool_number().to_string());
                self.tool_draft = tool;
                self.page = Page::Tools
            }
            Message::SaveTool => {
                let mut issues = self.tool_draft.issues();
                issues.extend(self.doc.tool_number_issues(&self.tool_draft));
                if issues.is_empty() {
                    if let Some(t) = self
                        .doc
                        .tools
                        .iter_mut()
                        .find(|t| t.id == self.tool_draft.id)
                    {
                        *t = self.tool_draft.clone()
                    } else {
                        self.doc.tools.push(self.tool_draft.clone())
                    }
                    self.touch();
                    self.status = "Tool saved to the local library.".into()
                } else {
                    self.status = issues.join(" ")
                }
            }
            Message::DeleteTool(id) => {
                if self
                    .doc
                    .setups
                    .iter()
                    .flat_map(|s| &s.operations)
                    .any(|o| o.tool == id)
                {
                    self.status="This tool is assigned to an operation. Reassign those operations before deleting it.".into()
                } else {
                    self.doc.tools.retain(|t| t.id != id);
                    self.tool_draft = self
                        .doc
                        .tools
                        .first()
                        .cloned()
                        .unwrap_or_else(|| blank_tool(&self.doc));
                    self.touch()
                }
            }
            Message::EditMaterial(id) => {
                if let Some(m) = self.doc.materials.iter().find(|m| m.id == id) {
                    self.material_draft = m.clone();
                    self.page = Page::Materials
                }
            }
            Message::NewMaterial => {
                self.material_draft = material::Material::new("New material");
                self.page = Page::Materials
            }
            Message::SaveMaterial => {
                let mut issues = self.material_draft.issues();
                issues.extend(self.doc.material_name_issues(&self.material_draft));
                if issues.is_empty() {
                    let name = self.material_draft.name().trim().to_string();
                    self.material_draft.p.insert("name".into(), name);
                    let saved = self.material_draft.clone();
                    if let Some(m) = self.doc.materials.iter_mut().find(|m| m.id == saved.id) {
                        *m = saved.clone()
                    } else {
                        self.doc.materials.push(saved.clone())
                    }
                    if self
                        .doc
                        .selected_material()
                        .is_some_and(|m| m.id == saved.id)
                    {
                        self.doc.select_material(&saved.id);
                    }
                    self.touch();
                    self.status = "Material saved to the library.".into()
                } else {
                    self.status = issues.join(" ")
                }
            }
            Message::DeleteMaterial(id) => {
                if self.doc.selected_material().is_some_and(|m| m.id == id) {
                    self.status = "This is the project material. Choose another project material before deleting it.".into()
                } else {
                    let before = self.doc.materials.len();
                    self.doc.materials.retain(|m| m.id != id);
                    self.material_draft = material_draft(&self.doc);
                    if self.doc.materials.len() < before {
                        self.touch();
                        self.status = "Material deleted from the library.".into()
                    } else {
                        self.status = "The unsaved material was discarded.".into()
                    }
                }
            }
            Message::PickMaterial(id) => {
                // A pick_list repeats the chosen item; that is not a change.
                if self.doc.material_id != id && self.doc.select_material(&id) {
                    self.touch()
                }
            }
            Message::EditHolder(id) => {
                if let Some(h) = self.doc.holder(&id) {
                    self.holder_draft = h.clone();
                    self.page = Page::Workholding
                }
            }
            Message::NewHolder(kind) => {
                let name = format!("New {}", kind.to_lowercase());
                self.holder_draft = holding::Holder::new(kind, &name);
                self.page = Page::Workholding
            }
            Message::SaveHolder => {
                let mut issues = self.holder_draft.issues();
                issues.extend(self.doc.holder_name_issues(&self.holder_draft));
                if issues.is_empty() {
                    let name = self.holder_draft.name().trim().to_string();
                    self.holder_draft.p.insert("name".into(), name);
                    let saved = self.holder_draft.clone();
                    if let Some(h) = self.doc.workholding.iter_mut().find(|h| h.id == saved.id) {
                        *h = saved
                    } else {
                        self.doc.workholding.push(saved)
                    }
                    self.touch();
                    self.status = "Workholding saved to the library.".into()
                } else {
                    self.status = issues.join(" ")
                }
            }
            Message::DeleteHolder(id) => {
                let using = self.doc.setups.iter().find(|s| {
                    matches!(get(&s.p, "holding"), "Vise" | "Clamps") && get(&s.p, "holder") == id
                });
                if let Some(setup) = using {
                    let name = self.doc.holder(&id).map_or("This entry", |h| h.name());
                    self.status = format!(
                        "{name} is used by the setup {}. Choose other workholding there before deleting it.",
                        setup.name()
                    )
                } else {
                    let before = self.doc.workholding.len();
                    self.doc.workholding.retain(|h| h.id != id);
                    self.holder_draft = holder_draft(&self.doc);
                    if self.doc.workholding.len() < before {
                        self.touch();
                        self.status = "Workholding deleted from the library.".into()
                    } else {
                        self.status = "The unsaved entry was discarded.".into()
                    }
                }
            }
            Message::SuggestLayout => {
                self.suggest_layout();
            }
            Message::SetClamps(text) => match holding::check_clamps(self.doc.setup(), &text) {
                Ok(()) => {
                    self.clamps_draft = None;
                    self.doc.setup_mut().p.insert("clamps".into(), text);
                    self.touch();
                    self.status = "Clamp positions saved with the setup.".into()
                }
                Err(error) => {
                    let setup = self.doc.setup();
                    self.clamps_draft = Some(ClampsDraft {
                        setup: setup.id.clone(),
                        over: get(&setup.p, "clamps").into(),
                        text,
                    });
                    self.status = error
                }
            },
            Message::SetOpTool(id) => {
                if let Some(o) = self.operation_mut() {
                    o.tool = id;
                }
                self.touch()
            }
            Message::RestSource(id) => {
                if let Some(o) = self.operation_mut() {
                    o.p.insert("rest_source".into(), id);
                }
                self.touch()
            }
            Message::StockSource(id) => {
                self.doc.setup_mut().p.insert("stock_source".into(), id);
                self.touch()
            }
            Message::SaveProject => {
                if self.save_busy || self.import_busy {
                    return Task::none();
                }
                self.save_busy = true;
                let doc = self.doc.clone();
                let filename = format!("{}.axis.json", sanitize_name(&self.doc.title));
                return Task::perform(
                    async move {
                        let file = rfd::AsyncFileDialog::new()
                            .set_title("Save AXIS CAM project")
                            .set_file_name(filename)
                            .add_filter("AXIS project", &["json"])
                            .save_file()
                            .await?;
                        let p = file.path().to_path_buf();
                        Some(storage::save(&p, &doc).map(|()| p))
                    },
                    Message::Saved,
                );
            }
            Message::OpenProject | Message::OpenHistory => {
                if self.import_busy || self.save_busy {
                    return Task::none();
                }
                let history = matches!(message, Message::OpenHistory)
                    .then(|| self.state_path.parent().unwrap().join("history"));
                self.import_busy = true;
                return Task::perform(
                    async move {
                        let mut dialog = rfd::AsyncFileDialog::new()
                            .set_title("Open AXIS CAM project")
                            .add_filter("AXIS project", &["json"]);
                        if let Some(folder) = history {
                            dialog = dialog.set_directory(folder);
                        }
                        let file = dialog.pick_file().await?;
                        let p = file.path().to_path_buf();
                        Some(storage::load_reporting(&p).map(|(d, n)| (d, p, n)))
                    },
                    Message::ProjectOpened,
                );
            }
            Message::Import => {
                if self.page == Page::Home && !self.import_busy && !self.save_busy {
                    let _ = self.update(Message::NewProject);
                    if self.page == Page::Home {
                        return Task::none();
                    }
                }
                if self.import_busy || self.save_busy {
                    return Task::none();
                }
                self.import_busy = true;
                self.status = "Choose a model. Geometry loads in the background…".into();
                return Task::perform(
                    async {
                        let file = rfd::AsyncFileDialog::new()
                            .set_title("Import CAD or mesh")
                            .add_filter("CAD and mesh", &["step", "stp", "3mf", "stl", "obj"])
                            .pick_file()
                            .await?;
                        let p = file.path();
                        let meta = match std::fs::metadata(p) {
                            Ok(m) => m,
                            Err(e) => return Some(Err(e.to_string())),
                        };
                        let ext = p
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_lowercase();
                        let info = ModelFile {
                            name: file.file_name(),
                            path: p.to_string_lossy().into_owned(),
                            format: ext.clone(),
                            bytes: meta.len(),
                        };
                        let path = p.to_path_buf();
                        let mesh =
                            match tokio::task::spawn_blocking(move || mesh::load(&path)).await {
                                Ok(Ok(m)) => Some(m),
                                Ok(Err(e)) => return Some(Err(e)),
                                Err(e) => return Some(Err(format!("Import worker failed: {e}"))),
                            };
                        Some(Ok((info, mesh)))
                    },
                    Message::FileImported,
                );
            }
            Message::FileImported(Some(Ok((f, m)))) => {
                self.doc.title = std::path::Path::new(&f.name)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                self.import_busy = false;
                self.import_error = None;
                self.status = format!(
                    "{} geometry loaded. Facing and Pocket can select planar surfaces and generate bounded previews.",
                    f.format.to_uppercase()
                );
                self.doc.sample_model = false;
                self.doc.file = Some(f);
                for setup in &mut self.doc.setups {
                    for operation in &mut setup.operations {
                        operation.geometry.clear();
                        operation.surfaces = None;
                    }
                }
                self.imported_mesh = m;
                // Undo would put the previous part's job under this one.
                self.undo = None;
                self.camera = viewport::Camera::default();
                self.picking = false;
                self.page = Page::Work;
                self.touch()
            }
            Message::FileImported(Some(Err(e))) => {
                self.import_busy = false;
                self.status = format!("Import failed: {e}");
            }
            Message::FileImported(None) => {
                self.import_busy = false;
                self.status = "Import cancelled.".into();
            }
            Message::ModelRestored(result) => {
                self.import_busy = false;
                match result {
                    Ok(mesh) => {
                        self.imported_mesh = Some(mesh);
                        self.undo = None;
                        self.invalidate_post();
                        self.status = "Saved model geometry restored.".into();
                        // Migrate legacy imported projects that stored sample
                        // feature IDs through the old viewport toggle.
                        if self.doc.file.is_some() {
                            let mut migrated = false;
                            for setup in &mut self.doc.setups {
                                for operation in &mut setup.operations {
                                    migrated |= !operation.geometry.is_empty();
                                    operation.geometry.clear();
                                }
                            }
                            if migrated {
                                self.touch();
                                self.status = "Model restored. Old sample-only selections were cleared; select surfaces on your part in Geometry.".into();
                            }
                        }
                    }
                    Err(e) => {
                        self.status = format!("Model could not be restored: {e}");
                        self.import_error = Some(e);
                    }
                }
            }
            Message::ProjectOpened(Some(Ok((d, p, notes)))) => {
                self.import_busy = false;
                if !self.archive_current() {
                    return Task::none();
                }
                self.replace_document(d, Some(p));
                self.status = with_notes(
                    "Project opened. Your previous workspace is in History.",
                    &notes,
                );
                return self.restore_model();
            }
            Message::ProjectOpened(Some(Err(e))) => {
                self.import_busy = false;
                self.status = e;
            }
            Message::ProjectOpened(None) => {
                self.import_busy = false;
            }
            Message::Saved(Some(Ok(p))) => {
                self.save_busy = false;
                self.status = format!("Saved {}", p.display());
                self.project_path = Some(p);
                self.project_dirty = false;
            }
            Message::Saved(Some(Err(e))) => {
                self.save_busy = false;
                self.status = format!("Save failed: {e}");
            }
            Message::Saved(None) => {
                self.save_busy = false;
            }
            Message::Export(_) => {
                let data = self.doc.setup_sheet(self.part_mesh());
                return Task::perform(
                    async move {
                        let file = rfd::AsyncFileDialog::new()
                            .set_title("Export setup sheet")
                            .set_file_name("axis-setup-sheet.md")
                            .save_file()
                            .await?;
                        let p = file.path().to_path_buf();
                        Some(storage::write_atomic(&p, data.as_bytes()).map(|()| p))
                    },
                    |r| match r {
                        Some(Ok(p)) => Message::Exported(p),
                        Some(Err(e)) => Message::ExportError(e),
                        None => Message::Noop,
                    },
                );
            }
            Message::ToggleStock => self.stock = !self.stock,
            Message::TogglePaths => self.paths = !self.paths,
            Message::Camera(view) => {
                self.camera = match view {
                    "TOP" => viewport::Camera {
                        yaw: 0.,
                        pitch: std::f32::consts::FRAC_PI_2,
                        ..Default::default()
                    },
                    "FRONT" => viewport::Camera {
                        yaw: 0.,
                        pitch: 0.,
                        ..Default::default()
                    },
                    "RIGHT" => viewport::Camera {
                        yaw: std::f32::consts::FRAC_PI_2,
                        pitch: 0.,
                        ..Default::default()
                    },
                    _ => Default::default(),
                };
            }
            Message::PlanViewport(e) => steer(&mut self.plan_camera, &e),
            Message::Viewport(e) => match e {
                viewport::Event::Orbit(..)
                | viewport::Event::Zoom(_)
                | viewport::Event::Pan(..) => steer(&mut self.camera, &e),
                viewport::Event::Pick(n) => {
                    if self.picking {
                        return self.update(Message::Feature(n));
                    }
                }
                viewport::Event::PickSurface(n) => {
                    if self.picking {
                        return self.update(Message::Surface(n));
                    }
                }
            },
            Message::Play => {
                if !self.playback_ready()
                    && !self.is_sample()
                    && self
                        .operation()
                        .is_some_and(|o| o.enabled && o.strategy.imported_preview())
                {
                    let _ = self.update(Message::Generate);
                }
                if self.playback_ready() {
                    self.playing = !self.playing;
                    if self.progress >= 1. {
                        self.progress = 0.;
                        if let Some(simulation) = &mut self.simulation {
                            simulation.seek(0.);
                        }
                    }
                    self.last_tick = Instant::now();
                    self.paths = true;
                } else if self.is_sample()
                    || !self
                        .operation()
                        .is_some_and(|o| o.strategy.imported_preview())
                {
                    self.status = self.playback_reason();
                }
            }
            Message::Scrub(v) => {
                if !self.playback_ready() {
                    return Task::none();
                }
                self.progress = if v.is_finite() { v.clamp(0., 1.) } else { 0. };
                self.playing = false;
                if let Some(simulation) = &mut self.simulation {
                    simulation.seek(self.progress);
                }
            }
            Message::Speed(v) => {
                if v.is_finite() {
                    self.speed = v.clamp(0.5, 120.);
                }
            }
            Message::StockDetail(detail) => {
                self.stock_detail = detail;
            }
            Message::StockMeshed {
                key,
                seconds,
                result,
            } => {
                if self.stock_mesh_job.as_ref() == Some(&key) {
                    self.stock_mesh_job = None;
                }
                if let Some(sim) = &mut self.simulation {
                    if sim.id == key.0 && sim.stock_epoch == key.1 && self.stock_detail == key.3 {
                        match result {
                            Ok(surface) => {
                                if self.smoke_dir.is_some() {
                                    eprintln!(
                                        "Stock surface: {} cuts, {} triangles, {:.3} mm, {} ms",
                                        sim.volume.as_ref().map_or(0, |v| v.cuts.len()),
                                        surface.triangles.len(),
                                        surface.cell_mm,
                                        surface.milliseconds
                                    );
                                }
                                sim.stock_surface = Some(surface);
                                sim.stock_surface_version = key.2;
                                sim.stock_surface_detail = key.3;
                                sim.stock_surface_seconds = seconds;
                                sim.revision = sim.revision.wrapping_add(1);
                            }
                            Err(e) => {
                                self.status = e;
                                self.stock_mesh_failed = Some(key);
                            }
                        }
                    }
                }
            }
            Message::HelpHover(on) => self.help_animating = on,
            Message::Tick => {
                if self.smoke_dir.is_some() && self.stock_mesh_key().is_some() {
                    assert!(
                        self.stock_mesh_failed.as_ref() != self.stock_mesh_key().as_ref(),
                        "Smoke stock meshing failed: {}",
                        self.status
                    );
                    return self.refresh_stock_surface();
                }
                if self.smoke_dir.is_some() && !self.smoke_pending {
                    self.smoke_pending = true;
                    return iced::window::latest()
                        .and_then(iced::window::screenshot)
                        .map(Message::Captured);
                }
                let now = Instant::now();
                let dt = now.duration_since(self.last_tick).as_secs_f32().min(0.2);
                self.last_tick = now;
                if self.playing {
                    let duration = self
                        .active_simulation()
                        .map_or(1. / 0.07, |s| s.plan.duration);
                    self.progress = (self.progress + dt * self.speed / duration).min(1.);
                    if let Some(simulation) = &mut self.simulation {
                        simulation.seek(self.progress);
                    }
                    if self.progress >= 1. {
                        self.playing = false;
                    }
                }
                if self.dirty && now.duration_since(self.last_edit) > Duration::from_millis(700) {
                    self.persist()
                }
                // A close waiting for the sign-in is checked again, so that
                // it happens when the wait is over.
                let close = self.close_if_waiting();
                return Task::batch([close, self.refresh_stock_surface(), self.refresh_post()]);
            }
            Message::Plan => {
                if matches!(self.planner, Planner::Running(_)) {
                    return Task::none();
                }
                // A cancelled run's blocking work cannot be interrupted.
                if self.plan_busy.running() > 0 {
                    self.status = PLAN_BUSY.into();
                    return Task::none();
                }
                if let Some(unmet) = self.plan_checklist().into_iter().find(|line| !line.ok) {
                    self.status = format!("{}: {}", unmet.label, unmet.hint);
                    return Task::none();
                }
                let (Some(client), Some(mesh)) = (self.client.clone(), self.imported_mesh.clone())
                else {
                    return Task::none();
                };
                self.drop_plan();
                let epoch = self.plan_epoch;
                self.plan_account_epoch = self.account_epoch;
                self.plan_model = self.settings.model.clone();
                self.planner = Planner::Running(planner::Progress::Analyzing);
                self.status = "Planning. Nothing changes until you apply a plan.".into();
                let backend = planner::OpenAiBackend::new(
                    client,
                    self.settings.model.clone(),
                    self.plan_received.clone(),
                );
                let doc = self.doc.clone();
                let busy = self.plan_busy.clone();
                let run = iced::stream::channel(16, async move |sender: PlanSender| {
                    run_plan(&backend, doc, mesh, busy, epoch, sender).await
                });
                let (task, handle) = Task::run(run, |message| message).abortable();
                self.plan_run = Some(handle);
                return task;
            }
            Message::PlanProgress(epoch, event) => {
                if epoch == self.plan_epoch && matches!(self.planner, Planner::Running(_)) {
                    self.record_plan_event(event);
                }
            }
            Message::PlanFinished(epoch, result) => {
                if epoch != self.plan_epoch || !matches!(self.planner, Planner::Running(_)) {
                    return Task::none();
                }
                self.plan_run = None;
                self.end_plan_step();
                let same_client = self.plan_account_epoch == self.account_epoch;
                match result {
                    Ok(preview) => {
                        self.status = format!(
                            "Plan ready to review: {}, {}.",
                            ui::plural(preview.outcome.job.setups.len(), "setup"),
                            ui::plural(preview.outcome.check.operations.len(), "operation")
                        );
                        let note = preview.outcome.note.clone();
                        self.planner = Planner::Preview(preview);
                        // Each preview starts framed on the part.
                        self.plan_camera = viewport::Camera::default();
                        // The run's requests worked, so the limit is no longer
                        // reached for that client, and the panel stops saying
                        // so, also when the fallback sentence follows it.
                        if same_client {
                            self.usage_limit = false;
                            if self
                                .account_note
                                .starts_with(&openai::ApiError::UsageLimit.message())
                            {
                                self.account_note.clear();
                            }
                        }
                        // A follow-up request that found the sign-in gone or
                        // the usage limit reached tells the account panel.
                        if let (
                            Some(
                                error @ (openai::ApiError::SignedOut(_)
                                | openai::ApiError::UsageLimit),
                            ),
                            true,
                        ) = (note, same_client)
                        {
                            self.account_error(error);
                        }
                    }
                    Err(failure) => {
                        self.status = failure.message.clone();
                        let api = failure.api.clone();
                        self.planner = Planner::Failed(failure);
                        // Only the client the run started with can be signed out by it.
                        if let (Some(error), true) = (api, same_client) {
                            self.account_error(error);
                        }
                    }
                }
            }
            Message::PlanCancel => {
                if matches!(self.planner, Planner::Running(_)) {
                    self.drop_plan();
                    self.status = "Planning cancelled. The job is unchanged.".into();
                }
            }
            Message::DiscardPlan => {
                if matches!(self.planner, Planner::Preview(_) | Planner::Failed(_)) {
                    self.planner = Planner::Idle;
                    self.plan_timeline.clear();
                }
            }
            Message::ApplyPlan => {
                let Planner::Preview(preview) = &self.planner else {
                    return Task::none();
                };
                let mut job = preview.outcome.job.clone();
                // The planner's workholding note for setup 1 joins setup 1's
                // own notes on the same line, in place of the exact note an
                // earlier Apply added, which setup 1 keeps for this; Undo
                // removes both with the rest of the plan.
                let note = preview.outcome.proposal.setups.first();
                if let Some(first) = job.setups.first_mut() {
                    let note = note.map_or("", |s| s.workholding.as_str());
                    let earlier = get(&first.p, planner::PLANNER_NOTE_KEY);
                    let (notes, added) =
                        planner::setup_1_notes(get(&first.p, "notes"), earlier, note);
                    first.p.insert("notes".into(), notes);
                    if added.is_empty() {
                        first.p.remove(planner::PLANNER_NOTE_KEY);
                    } else {
                        first.p.insert(planner::PLANNER_NOTE_KEY.into(), added);
                    }
                }
                // The previous job goes to History first, as on New project.
                // Applying is not a project switch: Recover previous keeps
                // pointing at the project before the last switch.
                if self.archive_copy().is_none() {
                    return Task::none();
                }
                self.undo = Some(self.doc.clone());
                self.planner = Planner::Idle;
                self.doc.setups = job.setups;
                self.doc.active = job.active;
                self.stop_selection();
                self.page = Page::Work;
                self.touch();
                self.status = "Plan applied. The previous job is in History; Undo is in AI planner. Review each operation, then export from NC program.".into();
            }
            Message::UndoPlan => {
                if self.undo.is_none() {
                    return Task::none();
                }
                // The applied job, with any edits since, goes to History
                // first; when it cannot be kept, nothing is undone.
                if self.archive_copy().is_none() {
                    return Task::none();
                }
                let Some(before) = self.undo.take() else {
                    return Task::none();
                };
                let mut status = String::from(
                    "The setups from before the plan are restored. The applied job is in History.",
                );
                // Only what Apply replaced comes back: tools, machines, the
                // title and the material edited since then stay. A tool
                // deleted since comes back, once.
                let mut back: Vec<Tool> = vec![];
                for op in before.setups.iter().flat_map(|s| &s.operations) {
                    if self.doc.tools.iter().chain(&back).any(|t| t.id == op.tool) {
                        continue;
                    }
                    if let Some(tool) = before.tools.iter().find(|t| t.id == op.tool) {
                        back.push(tool.clone());
                    }
                }
                // Each keeps its own number when no tool in the library has
                // it (the first, if two share one). Only the others are
                // renumbered, to numbers neither used nor kept.
                let used = |n: u32, tools: &[Tool]| tools.iter().any(|t| t.number() == Some(n));
                let mut kept: Vec<u32> = vec![];
                let keeps: Vec<bool> = back
                    .iter()
                    .map(|tool| match tool.number() {
                        Some(n) if !used(n, &self.doc.tools) && !kept.contains(&n) => {
                            kept.push(n);
                            true
                        }
                        Some(_) => false,
                        None => true,
                    })
                    .collect();
                for (mut tool, keep) in back.into_iter().zip(keeps) {
                    if !keep {
                        let free = (1..=TOOL_NUMBER_MAX)
                            .find(|n| !used(*n, &self.doc.tools) && !kept.contains(n))
                            .unwrap_or(TOOL_NUMBER_MAX);
                        tool.p.insert("number".into(), free.to_string());
                    }
                    status += &format!(
                        " {} is back in the library as T{}.",
                        tool.name(),
                        get(&tool.p, "number")
                    );
                    self.doc.tools.push(tool);
                }
                // The same for a workholding entry a restored setup uses; its
                // name is made new when another entry has taken it.
                for id in before
                    .setups
                    .iter()
                    .filter(|s| matches!(get(&s.p, "holding"), "Vise" | "Clamps"))
                    .map(|s| get(&s.p, "holder"))
                {
                    if let (None, Some(entry)) = (self.doc.holder(id), before.holder(id)) {
                        let mut entry = entry.clone();
                        if !self.doc.holder_name_issues(&entry).is_empty() {
                            let name = proposal::clip(entry.name(), material::NAME_LIMIT - 11);
                            entry.p.insert("name".into(), format!("{name} (restored)"));
                        }
                        status += &format!(" {} is back in the workholding library.", entry.name());
                        self.doc.workholding.push(entry);
                    }
                }
                self.doc.setups = before.setups;
                self.doc.active = before.active;
                self.stop_selection();
                self.touch();
                self.status = status;
            }
            Message::Close(id) => return self.request_close(id),
            Message::Exported(p) => self.status = format!("Exported {}", p.display()),
            Message::ExportError(e) => self.status = format!("Export failed: {e}"),
            Message::PostChecked(epoch, job) => {
                self.post_busy = false;
                if epoch == self.post_epoch {
                    self.post = Some(job);
                } else {
                    return self.refresh_post();
                }
            }
            Message::ExportGcode => {
                let Some(mesh) = self.part_mesh().cloned() else {
                    self.status = "Load the imported part before exporting G-code.".into();
                    return Task::none();
                };
                let doc = self.doc.clone();
                self.gcode_exports += 1;
                return Task::perform(
                    async move {
                        let folder = rfd::AsyncFileDialog::new()
                            .set_title("Choose a folder for the LinuxCNC programs")
                            .pick_folder()
                            .await?
                            .path()
                            .to_path_buf();
                        Some(
                            tokio::task::spawn_blocking(move || {
                                post::write_job(&post::post_job(&doc, &mesh), &folder)
                            })
                            .await
                            .unwrap_or_else(|e| Err(format!("Export worker failed: {e}"))),
                        )
                    },
                    Message::GcodeExported,
                );
            }
            Message::GcodeExported(result) => {
                self.gcode_exports = self.gcode_exports.saturating_sub(1);
                match result {
                    Some(Ok((folder, count))) => {
                        self.status = format!(
                            "Exported {count} LinuxCNC program{} to {}. Preview each in LinuxCNC and air-cut before cutting.",
                            if count == 1 { "" } else { "s" },
                            folder.display()
                        )
                    }
                    Some(Err(e)) => self.status = format!("G-code export failed: {e}"),
                    // No folder was chosen.
                    None => {}
                }
            }
            // Only the read that is pending can report a saved sign-in. A
            // result nobody is waiting for changes nothing, so a session the
            // user has signed out of since cannot come back.
            Message::AccountRestored(_) if !self.restoring => {}
            Message::AccountRestored(result) => {
                self.restoring = false;
                match result {
                    // A saved session only fills an empty account: not one
                    // that is signing in, signed in, or still deleting the
                    // old sign-in.
                    Ok(Some(session))
                        if self.account == Account::SignedOut
                            && self.client.is_none()
                            && !self.sign_out_pending =>
                    {
                        return self.adopt_session(session);
                    }
                    Ok(_) => {}
                    Err(e) => self.account_note = e,
                }
            }
            Message::SignIn => {
                // Signing in waits for the saved sign-in to be read and for
                // an old one to be deleted.
                if self.account != Account::SignedOut || self.sign_out_pending || self.restoring {
                    return Task::none();
                }
                self.account = Account::SigningIn;
                self.account_note.clear();
                let (endpoints, settings) = (self.endpoints.clone(), self.settings.clone());
                let (task, handle) = Task::perform(
                    async move {
                        let http = oauth::http_client(Duration::from_secs(60));
                        let limit = Duration::from_secs(300);
                        oauth::sign_in(
                            &http,
                            &endpoints,
                            &settings,
                            None,
                            limit,
                            oauth::open_browser,
                        )
                        .await
                    },
                    Message::SignedIn,
                )
                .abortable();
                self.sign_in = Some(handle);
                return task;
            }
            Message::SignInCancel => {
                // Only the browser step can be cancelled. Once the sign-in
                // is being saved there is no handle, and the save finishes.
                let Some(handle) = self.sign_in.take() else {
                    return Task::none();
                };
                handle.abort();
                if self.account == Account::SigningIn {
                    self.account = Account::SignedOut;
                    self.account_note = "Sign-in cancelled.".into();
                }
            }
            // A sign-in nobody is waiting for any more, or a second result
            // while the first is being saved: its refresh token was never
            // saved, so revoke it instead of leaving it valid.
            Message::SignedIn(Ok((session, client_id)))
                if self.account != Account::SigningIn || self.saving_sign_in() =>
            {
                let endpoints = self.endpoints.clone();
                return Task::perform(
                    async move {
                        oauth::revoke_unused(&endpoints, &client_id, &session.refresh_token).await
                    },
                    |()| Message::Noop,
                );
            }
            Message::SignedIn(Err(_))
                if self.account != Account::SigningIn || self.saving_sign_in() => {}
            // The browser step is done. Saving is a step of its own that
            // cannot be cancelled, so a cancel can never land in the middle
            // of it; the account shows nothing until it reports.
            Message::SignedIn(Ok((session, client_id))) => {
                self.sign_in = None;
                self.settings.client_id = client_id.clone();
                self.settings_failure = self.write_settings().err();
                return Task::perform(
                    oauth::save_session(
                        self.store.clone(),
                        self.endpoints.clone(),
                        client_id,
                        session,
                    ),
                    Message::SessionSaved,
                );
            }
            Message::SignedIn(Err(e)) => {
                self.sign_in = None;
                self.account = Account::SignedOut;
                self.account_note = e;
            }
            // Only the save step of a sign-in can report a save.
            Message::SessionSaved(_)
                if self.account != Account::SigningIn || self.sign_in.is_some() => {}
            Message::SessionSaved(Ok(session)) => {
                let settings_failure = self.settings_failure.take();
                let task = self.adopt_session(session);
                let task = Task::batch([task, self.close_if_waiting()]);
                if let Some(e) = settings_failure {
                    let sentence =
                        format!("Signed in for this session, but settings could not be saved: {e}");
                    self.account_note = if self.account_note.is_empty() {
                        sentence
                    } else {
                        format!("{} {sentence}", self.account_note)
                    };
                }
                return task;
            }
            Message::SessionSaved(Err(e)) => {
                self.settings_failure = None;
                self.account = Account::SignedOut;
                self.account_note = e;
                return self.close_if_waiting();
            }
            // Results of an earlier client's requests change nothing.
            Message::ModelsLoaded(epoch, _) | Message::ConnectionChecked(epoch, _)
                if epoch != self.account_epoch => {}
            Message::ModelsLoaded(_, Ok(models)) => {
                self.models = models;
                if self.models.is_empty() {
                    self.account_note = NO_MODELS.into();
                    self.fall_back_to_default_model();
                } else {
                    // A list replaces the fallback note, whatever came before
                    // it (no models, or why the list could not be read).
                    if self.account_note.starts_with(NO_MODELS)
                        || self.account_note.ends_with(FALLBACK)
                    {
                        self.account_note.clear();
                    }
                    if !self.models.contains(&self.settings.model) {
                        self.settings.model = self
                            .models
                            .iter()
                            .find(|m| *m == DEFAULT_MODEL)
                            .or(self.models.first())
                            .cloned()
                            .unwrap_or_default();
                        self.save_settings();
                    }
                }
            }
            Message::ModelsLoaded(_, Err(e)) => {
                self.account_error(e);
                self.fall_back_to_default_model();
            }
            Message::LoadModels => return self.load_models(),
            Message::PickModel(model) => {
                if self.models.contains(&model) {
                    self.settings.model = model;
                    self.save_settings();
                }
            }
            Message::CheckConnection => {
                let Account::SignedIn { granted: true, .. } = &self.account else {
                    return Task::none();
                };
                let Some(client) = self.client.clone() else {
                    return Task::none();
                };
                if self.account_busy || self.settings.model.is_empty() {
                    return Task::none();
                }
                // The panel says "Checking the connection…" itself while busy.
                self.account_busy = true;
                let (model, epoch) = (self.settings.model.clone(), self.account_epoch);
                return Task::perform(async move { client.check(&model).await }, move |result| {
                    Message::ConnectionChecked(epoch, result)
                });
            }
            Message::ConnectionChecked(_, Ok(message)) => {
                self.account_busy = false;
                self.usage_limit = false;
                self.account_note = message;
            }
            Message::ConnectionChecked(_, Err(e)) => {
                self.account_busy = false;
                self.account_error(e);
            }
            Message::SignOut => {
                // The save step cannot be aborted; signing out waits for it.
                if self.saving_sign_in() || self.sign_out_pending {
                    return Task::none();
                }
                if let Some(handle) = self.sign_in.take() {
                    handle.abort();
                }
                if matches!(self.planner, Planner::Running(_)) {
                    self.drop_plan();
                    self.plan_dropped = Some(PLAN_SIGNED_OUT);
                }
                let client = self.client.take();
                self.signing_out = client.clone();
                self.account = Account::SignedOut;
                // Sign-in stays disabled until the old sign-in is deleted, so
                // a late delete cannot remove a newer one.
                self.sign_out_pending = client.is_some();
                self.account_epoch = self.account_epoch.wrapping_add(1);
                self.account_busy = false;
                self.usage_limit = false;
                self.models.clear();
                // The panel says "Signing out…" by itself until the delete ends.
                self.account_note.clear();
                if let Some(client) = client {
                    return Task::perform(
                        async move { client.sign_out().await },
                        Message::SignedOut,
                    );
                }
            }
            Message::SignedOut(Ok(())) => {
                self.sign_out_pending = false;
                self.signing_out = None;
                self.account_note = "Signed out.".into();
                return self.close_if_waiting();
            }
            Message::SignedOut(Err(e)) => {
                self.sign_out_pending = false;
                self.signing_out = None;
                self.account_note = e;
                return self.close_if_waiting();
            }
            Message::PlanNoticeDismissed => {
                self.settings.plan_notice_shown = true;
                self.save_settings();
            }
            Message::OpenUrl(url) => {
                if let Err(e) = oauth::open_browser(url) {
                    self.account_note = e;
                }
            }
        }
        Task::none()
    }
    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(if self.playing || self.help_animating {
                Duration::from_millis(33)
            } else {
                Duration::from_millis(500)
            })
            .map(|_| Message::Tick),
            iced::event::listen_with(|event, _, window| match event {
                iced::Event::Window(iced::window::Event::CloseRequested) => {
                    Some(Message::Close(window))
                }
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Character(c),
                    modifiers,
                    ..
                }) if modifiers.command() && c.to_lowercase() == "s" => Some(Message::SaveProject),
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Character(c),
                    modifiers,
                    ..
                }) if modifiers.command() && c.to_lowercase() == "o" => Some(Message::OpenProject),
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Character(c),
                    modifiers,
                    ..
                }) if modifiers.command() && c.to_lowercase() == "n" => Some(Message::NewProject),
                _ => None,
            }),
        ])
    }
    fn view(&self) -> Element<'_, Message> {
        ui::view(self)
    }
}
fn sanitize_name(s: &str) -> String {
    let n: String = s
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(70)
        .collect();
    if n.is_empty() {
        "axis-project".into()
    } else {
        n
    }
}
/// A saved project and its imported part, ready for planning.
fn planning_project(project: &str) -> Result<(Document, mesh::Mesh), String> {
    let mut doc = storage::load(std::path::Path::new(project))?;
    doc.sync_stock_sources();
    let file = doc
        .file
        .clone()
        .ok_or("The project has no imported part.")?;
    let mesh = mesh::load(std::path::Path::new(&file.path))?;
    if let Some(unmet) = analysis::preconditions(&doc, Some(&mesh))
        .into_iter()
        .find(|p| !p.ok)
    {
        return Err(format!("{}: {}", unmet.label, unmet.hint));
    }
    Ok((doc, mesh))
}
/// The part report the planner sends to the model, as JSON.
fn part_report(args: &[String]) -> Result<String, String> {
    let [project] = args else {
        return Err("Usage: axis-cam --part-report <project.json>".into());
    };
    let (doc, mesh) = planning_project(project)?;
    serde_json::to_string_pretty(&analysis::report(&doc, &mesh)?).map_err(|e| e.to_string())
}
/// The JSON Schema a proposal must follow.
fn proposal_schema(args: &[String]) -> Result<String, String> {
    if !args.is_empty() {
        return Err("Usage: axis-cam --proposal-schema".into());
    }
    serde_json::to_string_pretty(&proposal::schema()).map_err(|e| e.to_string())
}
/// Checks a proposal file against a saved project with every planner check.
/// Returns the JSON report, a one-line summary and whether the plan passed;
/// `Err` when the check could not run.
fn check_plan(args: &[String]) -> Result<(String, String, bool), String> {
    let [project, proposal] = args else {
        return Err("Usage: axis-cam --check-plan <project.json> <proposal.json>".into());
    };
    let (doc, mesh) = planning_project(project)?;
    let report = analysis::report(&doc, &mesh)?;
    let text = std::fs::read_to_string(proposal).map_err(|e| format!("{proposal}: {e}"))?;
    let proposal = proposal::parse(&text)?;
    let (check, _) = plan_check::check(&doc, &mesh, &report, &proposal);
    let json = serde_json::to_string_pretty(&check).map_err(|e| e.to_string())?;
    let summary = if check.passed {
        format!(
            "The plan passes every check: {}, {} min.",
            ui::plural(check.operations.len(), "operation"),
            check.total_minutes
        )
    } else {
        format!(
            "The plan does not pass: {}.",
            ui::plural(check.problems.len(), "problem")
        )
    };
    Ok((json, summary, check.passed))
}
/// Exit code of a planning command: 0 when it succeeded (for `--check-plan`,
/// when the plan passes), 1 when the plan does not pass, 2 when the command
/// could not run.
fn exit_code(outcome: &Result<bool, String>) -> i32 {
    match outcome {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(_) => 2,
    }
}
/// Headless export of a saved project with the same checks and files as the UI.
fn export_gcode(project: Option<&String>, folder: Option<&String>) -> Result<String, String> {
    let (Some(project), Some(folder)) = (project, folder) else {
        return Err("Usage: axis-cam --export-gcode <project.json> <output folder>".into());
    };
    let (doc, notes) = storage::load_reporting(std::path::Path::new(project))?;
    let result = export_document(doc, folder);
    if notes.is_empty() {
        return result;
    }
    // The notes follow the outcome, on standard output or standard error.
    let note = |message: String| format!("{message}\n{}", notes.join(" "));
    result.map(note).map_err(note)
}
fn export_document(mut doc: Document, folder: &str) -> Result<String, String> {
    doc.sync_stock_sources();
    let file = doc
        .file
        .clone()
        .ok_or("The project has no imported part.")?;
    let mesh = mesh::load(std::path::Path::new(&file.path))?;
    let job = post::post_job(&doc, &mesh);
    for e in &job.errors {
        eprintln!("! {e}");
    }
    for setup in &job.setups {
        match &setup.result {
            Ok(programs) => programs
                .iter()
                .for_each(|p| println!("✓ {} ({} lines)", p.file_name, p.lines)),
            Err(problems) => problems
                .iter()
                .for_each(|e| eprintln!("! {}: {e}", setup.name)),
        }
    }
    let (path, count) = post::write_job(&job, std::path::Path::new(folder))?;
    Ok(format!(
        "Wrote {count} LinuxCNC program(s) to {}",
        path.display()
    ))
}
/// The example project with the user's tool, machine, material and workholding
/// libraries merged in, and the status line naming the user's entries it adds. A user
/// entry the example already has, ignoring tool numbers and post settings
/// saved before they existed, is not added twice: the example's tool takes
/// the user's number. Every other example tool whose number is then in use
/// gets the smallest free one; the example's operations refer to it by ID.
/// Materials match by name (`material::same_name`), and the user's entry is kept; the
/// example's material is then linked to the library, or added to it without
/// cutting data, and the status says which. Workholding matches by name too;
/// the starters are added only to an empty library.
fn merge_sample(user: &Document) -> (Document, String) {
    let mut doc = Document::sample();
    let (mut tools, mut machines, mut materials, mut holders) = (vec![], vec![], vec![], vec![]);
    for m in &user.machines {
        let mut filled = m.clone();
        filled.fill_post_defaults();
        if !doc.machines.iter().any(|x| x.p == filled.p) {
            doc.machines.push(m.clone());
            machines.push(m.name().to_string());
        }
    }
    let starters = crate::material::starters();
    let known = |list: &[crate::material::Material], name: &str| {
        list.iter()
            .any(|m| crate::material::same_name(m.name(), name))
    };
    for m in &user.materials {
        if !known(&doc.materials, m.name()) {
            doc.materials.push(m.clone());
            if !known(&starters, m.name()) {
                materials.push(m.name().to_string());
            }
        }
    }
    let material_note = doc.adopt_materials();
    let starter_holders = holding::starters();
    for h in &user.workholding {
        let known = |list: &[holding::Holder]| {
            list.iter()
                .any(|x| crate::material::same_name(x.name(), h.name()))
        };
        if !known(&doc.workholding) {
            doc.workholding.push(h.clone());
            if !known(&starter_holders) {
                holders.push(h.name().to_string());
            }
        }
    }
    doc.adopt_holders();
    let unnumbered = |t: &Tool| {
        let mut p = t.p.clone();
        p.remove("number");
        p
    };
    let examples = doc.tools.len();
    let mut taken = vec![false; examples];
    let mut renumber = vec![true; examples];
    for t in &user.tools {
        let same = (0..examples).find(|&i| !taken[i] && unnumbered(&doc.tools[i]) == unnumbered(t));
        if let Some(i) = same {
            taken[i] = true;
            if let Some(n) = t.number() {
                doc.tools[i].p.insert("number".into(), n.to_string());
                renumber[i] = false;
            }
        } else {
            doc.tools.push(t.clone());
            tools.push(t.name().to_string());
        }
    }
    for i in (0..examples).filter(|&i| renumber[i]) {
        let number = doc.tools[i].number();
        let used = doc
            .tools
            .iter()
            .enumerate()
            .any(|(j, t)| j != i && t.number() == number);
        if used {
            let free = doc.next_tool_number();
            doc.tools[i].p.insert("number".into(), free.to_string());
        }
    }
    let list = |names: &[String], one: &str, many: &str| {
        (!names.is_empty()).then(|| {
            format!(
                "{} {}",
                if names.len() == 1 { one } else { many },
                names.join(", ")
            )
        })
    };
    let added: Vec<String> = [
        list(&tools, "tool", "tools"),
        list(&machines, "machine", "machines"),
        list(&materials, "material", "materials"),
        list(&holders, "workholding entry", "workholding entries"),
    ]
    .into_iter()
    .flatten()
    .collect();
    let status = if added.is_empty() {
        "Example project opened. Use New project to return to a blank job.".to_string()
    } else {
        format!(
            "Example project opened. Added from your library: {}. Use New project to return to a blank job.",
            added.join("; ")
        )
    };
    (doc, with_notes(&status, material_note.as_slice()))
}
/// How long a close request waits for a sign-out, a sign-in save or a
/// refresh of the sign-in.
const CLOSE_WAIT: Duration = Duration::from_secs(10);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseAction {
    /// Keep the window: the workspace could not be saved.
    Stay,
    /// Wait for the sign-out, sign-in save or refresh, then ask again.
    Wait,
    /// Close the window.
    Close,
    /// Start deleting the saved sign-in on the blocking pool, then close the
    /// window: a sign-out is still pending after the wait.
    ClearAndClose,
    /// End the process: blocking work would keep it running without a window.
    Exit,
}
/// What a close request does once the workspace save was tried. `saved`:
/// the workspace is saved. `pending`: how long a pending sign-out, sign-in
/// save or refresh of the sign-in has been waited for, and whether a
/// sign-out is pending. `blocking`: a plan stage, the NC program refresh or
/// the stock mesher is still running. `finishing`: a G-code export is
/// running. Neither pending work nor finishing work is cut short by ending
/// the process: the window closes and the runtime lets their blocking parts
/// finish. After `CLOSE_WAIT`, a pending sign-out also starts deleting the
/// saved sign-in, which the process then waits for.
pub(crate) fn close_action(
    saved: bool,
    pending: Option<(Duration, bool)>,
    blocking: bool,
    finishing: bool,
) -> CloseAction {
    match pending {
        _ if !saved => CloseAction::Stay,
        Some((waited, _)) if waited < CLOSE_WAIT => CloseAction::Wait,
        Some((_, true)) => CloseAction::ClearAndClose,
        Some((_, false)) => CloseAction::Close,
        None if finishing => CloseAction::Close,
        None if blocking => CloseAction::Exit,
        None => CloseAction::Close,
    }
}
/// The tool editor's draft when the library is empty, with the next free
/// tool number so that its first Save is accepted.
fn blank_tool(doc: &Document) -> Tool {
    let mut tool = Tool::new("", "Flat end mill", "6");
    tool.p
        .insert("number".into(), doc.next_tool_number().to_string());
    tool
}
/// The workholding editor's draft: the first library entry, or a new vise.
fn holder_draft(doc: &Document) -> holding::Holder {
    doc.workholding
        .first()
        .cloned()
        .unwrap_or_else(|| holding::Holder::new("Vise", "New vise"))
}
/// The material editor's draft: the first library material, or a new one.
fn material_draft(doc: &Document) -> material::Material {
    doc.materials
        .first()
        .cloned()
        .unwrap_or_else(|| material::Material::new("New material"))
}
/// A status line, followed by the notes of loading the project.
fn with_notes(status: &str, notes: &[String]) -> String {
    std::iter::once(status)
        .chain(notes.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}
fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--export-gcode") {
        match export_gcode(args.get(2), args.get(3)) {
            Ok(message) => {
                println!("{message}");
                std::process::exit(0)
            }
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1)
            }
        }
    }
    // Planning commands: JSON alone on standard output, everything else on
    // standard error.
    let rest = args.get(2..).unwrap_or_default();
    let json = |result: Result<String, String>| {
        result.map(|json| {
            println!("{json}");
            true
        })
    };
    let outcome = match args.get(1).map(String::as_str) {
        Some("--check-plan") => Some(check_plan(rest).map(|(json, summary, passed)| {
            println!("{json}");
            eprintln!("{summary}");
            passed
        })),
        Some("--part-report") => Some(json(part_report(rest))),
        Some("--proposal-schema") => Some(json(proposal_schema(rest))),
        _ => None,
    };
    if let Some(outcome) = outcome {
        if let Err(message) = &outcome {
            eprintln!("{message}");
        }
        std::process::exit(exit_code(&outcome))
    }
    iced::application(App::boot, App::update, App::view)
        .title(|app: &App| format!("{} — AXIS CAM Studio", app.doc.title))
        .theme(Theme::custom(
            "Axis Light",
            iced::theme::Palette {
                background: iced::Color::from_rgb8(242, 245, 249),
                text: iced::Color::from_rgb8(33, 51, 71),
                primary: iced::Color::from_rgb8(33, 107, 181),
                success: iced::Color::from_rgb8(39, 146, 108),
                danger: iced::Color::from_rgb8(194, 75, 73),
                warning: iced::Color::from_rgb8(184, 126, 48),
            },
        ))
        .subscription(App::subscription)
        .window(iced::window::Settings {
            size: Size::new(1440., 930.),
            min_size: Some(Size::new(1120., 720.)),
            exit_on_close_request: false,
            ..Default::default()
        })
        .centered()
        .antialiasing(true)
        .default_font(iced::Font::with_name("Helvetica Neue"))
        .run()
}
