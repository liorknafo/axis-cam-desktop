mod analysis;
mod cam;
mod choice_diagram;
mod choice_help;
#[cfg(test)]
mod fake_openai;
mod help;
mod mesh;
mod model;
mod ngc;
mod oauth;
mod openai;
mod plan_check;
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
const NOT_GRANTED: &str =
    "ChatGPT plan usage was not granted. Sign out, then sign in again and allow plan usage.";
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
    Material(String),
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
    SetOpTool(String),
    RestSource(String),
    StockSource(String),
    SaveProject,
    OpenProject,
    OpenHistory,
    Import,
    FileImported(Option<Result<(ModelFile, Option<mesh::Mesh>), String>>),
    ProjectOpened(Option<Result<(Document, PathBuf), String>>),
    ModelRestored(Result<mesh::Mesh, String>),
    Saved(Option<Result<PathBuf, String>>),
    Export(&'static str),
    ToggleStock,
    TogglePaths,
    Camera(&'static str),
    Viewport(viewport::Event),
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
    AiCoverage(String),
    AiPriority(String),
    AiBrief(String),
    Plan,
    ApplyPlan,
    UndoPlan,
    Exported(PathBuf),
    ExportError(String),
    PostChecked(u64, std::sync::Arc<post::JobPost>),
    ExportGcode,
    GcodeExported(Result<(PathBuf, usize), String>),
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
type StockMeshKey = (String, u64, u64, stock_surface::Detail);
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
    pub ai_coverage: String,
    pub ai_priority: String,
    pub ai_brief: String,
    pub plan: Vec<Setup>,
    pub undo: Option<Document>,
    pub confirm_delete_setup: bool,
    pub icons: Vec<iced::widget::svg::Handle>,
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
        let (mut doc, status) = if state_path.exists() && smoke_dir.is_none() {
            match storage::load(&state_path) {
                Ok(d) => (
                    d,
                    "Choose New project, Open, or Continue your previous project.".into(),
                ),
                Err(e) => {
                    state_path =
                        state_path.with_file_name(format!("workspace-recovery-{}.json", id()));
                    (
                        Document::default(),
                        format!(
                            "{e} Original file preserved. A separate recovery workspace will be used."
                        ),
                    )
                }
            }
        } else {
            (
                Document::default(),
                "Start a new project or open an existing one.".into(),
            )
        };
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
                .unwrap_or_else(|| Tool::new("", "Flat end mill", "6")),
            doc,
            page: Page::Home,
            selected: None,
            tab: "Tool".into(),
            status,
            audit_report: vec![],
            post: None,
            post_busy: false,
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
            dirty: false,
            last_edit: Instant::now(),
            state_path,
            project_path: None,
            project_dirty: false,
            wizard_step: 0,
            ai_coverage: "Top and bottom".into(),
            ai_priority: "Balanced".into(),
            ai_brief: String::new(),
            plan: vec![],
            undo: None,
            confirm_delete_setup: false,
            icons,
        }
    }
    fn boot() -> (Self, Task<Message>) {
        let mut app = Self::new();
        if app.smoke_dir.is_some() {
            app.replace_document(Document::sample(), None);
            app.selected = Some(0);
            if std::env::args().any(|arg| arg == "--help-only") {
                app.smoke_step = 14;
            }
        }
        (app, Task::none())
    }
    fn archive_current(&mut self) -> bool {
        match storage::archive(&self.state_path, &self.doc) {
            Ok(path) => {
                self.last_archived = Some(path);
                true
            }
            Err(e) => {
                self.status = format!(
                    "Could not preserve your project: {e}. Your current work is unchanged."
                );
                false
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
            .unwrap_or_else(|| Tool::new("", "Flat end mill", "6"));
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
        self.plan.clear();
        self.undo = None;
        self.ai_brief.clear();
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
    fn touch(&mut self) {
        if !self.is_sample() {
            self.doc.sync_stock_sources();
        }
        self.audit_report.clear();
        self.invalidate_post();
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
        "Generate the operation preview, or press Generate & play. Stock uses adaptive surface meshing and inherits earlier cuts; collision checking is not implemented.".into()
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
    fn account_error(&mut self, error: openai::ApiError) {
        self.usage_limit = error == openai::ApiError::UsageLimit;
        if matches!(error, openai::ApiError::SignedOut(_)) {
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
                let doc = Document {
                    machines: self.doc.machines.clone(),
                    tools: self.doc.tools.clone(),
                    ..Document::default()
                };
                self.replace_document(doc, None);
                self.status = "Blank project created. Saved machines and tools are available; your previous project is in History.".into();
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
                let mut doc = Document::sample();
                for m in &self.doc.machines {
                    if !doc.machines.iter().any(|x| x.p == m.p) {
                        doc.machines.push(m.clone());
                    }
                }
                for t in &self.doc.tools {
                    if !doc.tools.iter().any(|x| x.p == t.p) {
                        doc.tools.push(t.clone());
                    }
                }
                self.replace_document(doc, None);
                self.selected = Some(0);
                self.status =
                    "Example project opened. Use New project to return to a blank job.".into();
            }
            Message::RecoverPrevious => {
                if let Some(path) = self.last_archived.clone() {
                    let result = storage::load(&path).map(|d| (d, path));
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
                    self.doc.setup_mut().p.insert(k.into(), v);
                    self.touch()
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
            },
            Message::Title(v) => {
                self.doc.title = v;
                self.touch()
            }
            Message::Material(v) => {
                self.doc.material = v;
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
                                "{} preview ready: {} levels, {:.1} min estimated. Inherits stock after {} earlier operations across setups. Sampled removal; fixture and machine collisions are not checked. Export G-code from the NC program page.",
                                plan.strategy.name(),
                                plan.levels,
                                plan.duration / 60.,
                                sim.preceding
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
                        .unwrap_or_else(|| Tool::new("", "Flat end mill", "6"));
                    self.touch()
                }
            }
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
                        Some(storage::load(&p).map(|d| (d, p)))
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
            Message::ProjectOpened(Some(Ok((d, p)))) => {
                self.import_busy = false;
                if !self.archive_current() {
                    return Task::none();
                }
                self.replace_document(d, Some(p));
                self.status = "Project opened. Your previous workspace is in History.".into();
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
            Message::Viewport(e) => match e {
                viewport::Event::Orbit(x, y) => {
                    self.camera.yaw += x;
                    self.camera.pitch = (self.camera.pitch + y).clamp(-1.5, 1.57)
                }
                viewport::Event::Pan(x, y) => {
                    self.camera.pan[0] += x;
                    self.camera.pan[1] += y
                }
                viewport::Event::Zoom(v) => {
                    self.camera.zoom = (self.camera.zoom * (1. + v).clamp(0.5, 1.5)).clamp(0.3, 4.)
                }
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
                return Task::batch([self.refresh_stock_surface(), self.refresh_post()]);
            }
            Message::AiCoverage(v) => {
                self.ai_coverage = v;
                self.plan.clear()
            }
            Message::AiPriority(v) => {
                self.ai_priority = v;
                self.plan.clear()
            }
            Message::AiBrief(v) => {
                self.ai_brief = v;
                self.plan.clear()
            }
            Message::Plan => {
                if self.doc.tools.is_empty() || !self.is_sample() {
                    self.status = "The example planner needs the sample project and a saved tool. Choose Explore example from Start.".into();
                    return Task::none();
                }
                self.plan = self.doc.sample_plan(&self.ai_coverage, &self.ai_priority);
                self.status =
                    "Local example plan prepared for review. No OpenAI API request was made.".into()
            }
            Message::ApplyPlan => {
                if !self.plan.is_empty() {
                    self.undo = Some(self.doc.clone());
                    self.doc.setups = self.plan.clone();
                    self.doc.active = self.doc.setups[0].id.clone();
                    self.stop_selection();
                    self.page = Page::Work;
                    self.touch();
                    self.status = "Example plan applied. Undo is available in AI planner.".into()
                }
            }
            Message::UndoPlan => {
                if let Some(d) = self.undo.take() {
                    self.doc = d;
                    self.stop_selection();
                    self.touch();
                    self.plan.clear();
                    self.status = "Previous setups restored.".into()
                }
            }
            Message::Close(id) => {
                self.persist();
                if self.dirty {
                    self.status="Could not save before closing. Save Project to another location, then close again.".into()
                } else {
                    return iced::window::close(id);
                }
            }
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
                    |r| r.map_or(Message::Noop, Message::GcodeExported),
                );
            }
            Message::GcodeExported(Ok((folder, count))) => {
                self.status = format!(
                    "Exported {count} LinuxCNC program{} to {}. Preview each in LinuxCNC and air-cut before cutting.",
                    if count == 1 { "" } else { "s" },
                    folder.display()
                )
            }
            Message::GcodeExported(Err(e)) => self.status = format!("G-code export failed: {e}"),
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
            }
            // Results of an earlier client's requests change nothing.
            Message::ModelsLoaded(epoch, _) | Message::ConnectionChecked(epoch, _)
                if epoch != self.account_epoch => {}
            Message::ModelsLoaded(_, Ok(models)) => {
                self.models = models;
                if self.models.is_empty() {
                    self.account_note = NO_MODELS.into();
                } else {
                    if self.account_note == NO_MODELS {
                        self.account_note.clear();
                    }
                    if !self.models.contains(&self.settings.model) {
                        self.settings.model = self
                            .models
                            .iter()
                            .find(|m| *m == "gpt-6.1-sol")
                            .or(self.models.first())
                            .cloned()
                            .unwrap_or_default();
                        self.save_settings();
                    }
                }
            }
            Message::ModelsLoaded(_, Err(e)) => self.account_error(e),
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
                let client = self.client.take();
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
                self.account_note = "Signed out.".into();
            }
            Message::SignedOut(Err(e)) => {
                self.sign_out_pending = false;
                self.account_note = e;
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
fn part_report(project: Option<&String>) -> Result<String, String> {
    let project = project.ok_or("Usage: axis-cam --part-report <project.json>")?;
    let (doc, mesh) = planning_project(project)?;
    serde_json::to_string_pretty(&analysis::report(&doc, &mesh)?).map_err(|e| e.to_string())
}
/// Checks a proposal file against a saved project with every planner check.
/// Prints the report as JSON; `Err` when the plan does not pass.
fn check_plan(project: Option<&String>, proposal: Option<&String>) -> Result<String, String> {
    let (Some(project), Some(proposal)) = (project, proposal) else {
        return Err("Usage: axis-cam --check-plan <project.json> <proposal.json>".into());
    };
    let (doc, mesh) = planning_project(project)?;
    let report = analysis::report(&doc, &mesh)?;
    let text = std::fs::read_to_string(proposal).map_err(|e| format!("{proposal}: {e}"))?;
    let proposal = proposal::parse(&text)?;
    let (check, _) = plan_check::check(&doc, &mesh, &report, &proposal);
    println!(
        "{}",
        serde_json::to_string_pretty(&check).map_err(|e| e.to_string())?
    );
    if check.passed {
        Ok(format!(
            "The plan passes every check: {} operations, {} min.",
            check.operations.len(),
            check.total_minutes
        ))
    } else {
        Err(format!(
            "The plan does not pass: {} problem(s).",
            check.problems.len()
        ))
    }
}
/// Headless export of a saved project with the same checks and files as the UI.
fn export_gcode(project: Option<&String>, folder: Option<&String>) -> Result<String, String> {
    let (Some(project), Some(folder)) = (project, folder) else {
        return Err("Usage: axis-cam --export-gcode <project.json> <output folder>".into());
    };
    let mut doc = storage::load(std::path::Path::new(project))?;
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
fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().collect();
    let command = match args.get(1).map(String::as_str) {
        Some("--export-gcode") => Some(export_gcode(args.get(2), args.get(3))),
        Some("--check-plan") => Some(check_plan(args.get(2), args.get(3))),
        Some("--part-report") => Some(part_report(args.get(2))),
        Some("--proposal-schema") => {
            Some(serde_json::to_string_pretty(&proposal::schema()).map_err(|e| e.to_string()))
        }
        _ => None,
    };
    if let Some(result) = command {
        match result {
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
