mod adaptive;
mod analysis;
mod cam;
mod choice_diagram;
mod choice_help;
mod chuck;
mod control;
mod facing;
#[cfg(test)]
mod fake_openai;
mod field_art;
mod forces;
mod gpu_mesh;
mod guide;
mod help;
mod holding;
mod illustration;
mod indexed;
mod lathe;
mod look;
mod machine_view;
mod manual;
mod material;
mod mesh;
mod model;
mod ngc;
mod oauth;
mod openai;
mod plan_changes;
mod plan_check;
mod planner;
mod planner_tools;
mod pocket;
mod post;
mod post_check;
mod progress;
mod proposal;
mod rest;
mod rotary_check;
mod run;
mod section;
mod session;
mod settings;
mod setup;
mod simulation;
mod smoke;
mod step;
mod steps;
mod stock;
mod stock_surface;
mod storage;
mod surface;
mod thread;
mod threading;
mod three_mf;
mod tooth_load;
mod turn;
mod ui;
mod uncut;
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
    Control,
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
/// A saved sign-in whose client ID is gone cannot be refreshed, and OpenAI
/// only revokes a token for the client it was issued to.
const LOST_CLIENT: &str = "A saved ChatGPT sign-in was found, but the app has lost the ID OpenAI gave it, so it can neither use the sign-in nor end it at OpenAI.";
const LOST_CLIENT_CLEARED: &str = "A saved ChatGPT sign-in was found, but the app has lost the ID OpenAI gave it, so it can neither use the sign-in nor end it at OpenAI. It was deleted from this computer. Sign in again; OpenAI may still list the old access in your ChatGPT settings, where you can remove it.";
/// Reads the saved sign-in. With no client ID (`client_lost`) a saved one is
/// deleted instead, and the error says so.
fn read_saved_session(
    store: &dyn session::SessionStore,
    client_lost: bool,
) -> Result<Option<session::Session>, String> {
    let saved = store.load();
    if client_lost && matches!(saved, Ok(Some(_))) {
        return Err(match store.clear() {
            Ok(()) => LOST_CLIENT_CLEARED.into(),
            Err(e) => format!("{LOST_CLIENT} It could not be deleted: {e}"),
        });
    }
    saved
}
/// How long edits must pause before the project is saved and the stock the
/// earlier setups left is worked out again.
const EDIT_PAUSE: Duration = Duration::from_millis(700);
/// The window's size when it opens, and the least it can be made: the Work
/// page's sidebars leave the 3D view `ui::VIEW_MIN` wide at the least
/// (`ui::fitted`).
pub(crate) const WINDOW: Size = Size::new(1440., 930.);
pub(crate) const MIN_WINDOW: Size = Size::new(1120., 720.);
/// Sends the user to a URL in their browser; see `App::open_browser`.
type BrowserOpener = std::sync::Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;
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
    /// The browser's "Stock after this setup": the 3D view shows the stock
    /// the active setup's operations leave (`App::setup_end`).
    SetupEnd,
    /// The workflow footer's 02 step.
    OperationsStep,
    /// The workflow footer's 03 step.
    PreviewStep,
    SetupStep(&'static str),
    Tab(String),
    Edit(Target, &'static str, String),
    Title(String),
    AddOp(Strategy),
    DuplicateOp,
    RemoveOp,
    /// The operation editor's Delete: asks before `RemoveOp`, as a setup's
    /// Delete does (`ConfirmDeleteSetup`).
    AskRemoveOp,
    ConfirmDeleteOp(bool),
    MoveOp(isize),
    ToggleOp,
    Feature(usize),
    Surface(usize),
    /// The Geometry list's remove button: drop this hole (its index in
    /// `App::pocket_holes`) from a Pocket or Contour, keeping the walls
    /// another picked hole shares.
    DropHole(usize),
    ClearSurfaces,
    BoxFromFace,
    PickMode,
    /// "Pick holes on the part" (and "Finish picking") in the Work zero step.
    PickPinHoles,
    /// A click on the part's surface while picking the pin holes.
    PinHoleSurface(usize),
    /// "Place the pins on the leftover" (and "Finish placing") in the Work
    /// zero step of a setup on pins in its stock's leftover.
    PlacePins,
    /// "Add the pin holes to …": a Drill of the pin holes in the earlier
    /// setup that has the face the pins stand on facing up.
    AddPinDrilling,
    /// Add the operations that drill and thread the setup's threaded holes
    /// (`threading::add`).
    AddThreading,
    /// Fill the selected Turn thread's side, diameter, pitch and end from
    /// the thread on the part's axis (`lathe::thread_from_part`).
    ThreadFromPart,
    Generate,
    GenerateAll,
    /// A generation off the window's thread is done (`App::generate`): its
    /// epoch and the operation's simulation.
    Generated(u64, Made),
    /// Generate all is done: its epoch and the audit's lines.
    GeneratedAll(u64, Vec<AuditRow>),
    /// A setup's operations are generated for its holding: the epoch and
    /// what they made (`App::suggest_layout`).
    GeneratedLayout(u64, Laid),
    /// A generation's thread failed: its epoch and why.
    GenerationFailed(u64, String),
    /// The progress bar's Cancel.
    CancelGenerate,
    AddSetup,
    /// Inserts a setup on the first manual machine, before the CNC setups.
    AddManualSetup,
    /// Inserts a setup on the first lathe, with a round bar in a chuck,
    /// before the CNC setups.
    AddLatheSetup,
    DuplicateSetup,
    RemoveSetup,
    MoveSetup(isize),
    Flip(&'static str),
    /// Turns a later setup so the stock lies with face `.0` of
    /// `volume::FACES` up, turned `.1` quarter turns on the table, from
    /// setup 1: square to the stock.
    TurnStock(usize, usize),
    /// Snaps a later setup a few degrees off to the quarter turn of the
    /// stock nearest it (`volume::nearest_turn`).
    SquareToStock,
    AssignMachine(String),
    NewMachine,
    /// Adds the Jingyan CNC4030 with its rotary unit to the Machine library.
    AddRotaryMachine,
    EditMachine(String),
    WizardStep(usize),
    SaveMachine,
    DeleteMachine(String),
    /// A machine's check box in the tool editor (machine id, checked).
    ToolMachine(String, bool),
    /// A machine's check box in the workholding editor (machine id, checked).
    HolderMachine(String, bool),
    /// Opens a file dialog for the machine draft's UC100 profile.
    BrowseUc100Profile,
    /// The chosen UC100 profile; None when the dialog was cancelled.
    Uc100ProfilePicked(Option<PathBuf>),
    /// Chooses the UC100 machine for the Control page (machine id). While
    /// another machine is connected it asks first (`SwitchMachine`).
    ControlMachine(String),
    /// The answer to "disconnect and switch?": true disconnects, chooses the
    /// machine asked about and connects to it.
    SwitchMachine(bool),
    /// Back to the manual mill's steps from the CNC's Control page.
    ShowGuide,
    /// The Control page's Real | Simulated switch for its machine (true:
    /// simulated). Connected the other way, it reconnects.
    MachineConnection(bool),
    Connect,
    Disconnect,
    MachineReset,
    MachineEStop,
    /// An action on the connected machine, from the Control page.
    Machine(control::Action),
    /// The Control page's Control | Setup switch.
    ControlSetup(bool),
    Setup(setup::Msg),
    /// A jog key went down and no widget took it (axis, negative, repeat).
    JogKey(usize, bool, bool),
    /// Whether a text field had the focus when a jog key went down (axis,
    /// negative, typing).
    JogKeyChecked(usize, bool, bool),
    /// A jog key went up.
    JogKeyUp,
    /// Space went down and no widget took it: Hold, or Resume.
    HoldKey,
    /// Tab went down: the next text field, or the previous one with Shift;
    /// with none focused, the form's first field.
    TabKey(bool),
    /// F1 went down: the help of the focused text field, pinned.
    HelpKey,
    /// The focused text field F1 found, or none: pins its help, or puts the
    /// pinned help away.
    HelpFor(Option<help::FieldTag>),
    /// Esc went down: the E-stop while a machine is connected; off the
    /// Control page, it puts the pinned help away.
    EscapeKey,
    /// A row of the operation audit's report: opens its operation, at the
    /// tab its line names.
    AuditRow(usize),
    /// An action of the Run panel on the Control page.
    Run(run::Action),
    /// NC review's Run on the machine for a UC100 setup: the MACHINE
    /// workspace with this setup picked in the Run panel, at once when its
    /// machine is connected, otherwise once Connect opens it.
    RunOnMachine(usize),
    /// A setup's tool runs, checked for a UC100 run (the check's epoch).
    RunChecked(u64, std::sync::Arc<Result<Vec<post::Checked>, Vec<String>>>),
    /// `RunChecked`, with the stock the check left after each operation of
    /// the setup, to picture (`run::check`), and how long its tool runs take
    /// on the connected machine (`run::Times`).
    RunCheckedStock(
        u64,
        std::sync::Arc<Result<Vec<post::Checked>, Vec<String>>>,
        std::sync::Arc<run::Left>,
        Option<std::sync::Arc<run::Times>>,
    ),
    /// The pictures of the stock after each operation of the setup being
    /// run, by its index in the setup (`run::op_pictures`), for the check
    /// whose epoch it is.
    RunPictures(u64, Vec<(usize, iced::widget::image::Handle)>),
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
    /// Saves the project to its file, asking for one the first time.
    SaveProject,
    /// Saves the project to a file the user chooses, which becomes its file.
    SaveProjectAs,
    OpenProject,
    OpenHistory,
    Import,
    FileImported(Option<Result<(ModelFile, Option<mesh::Mesh>), String>>),
    ProjectOpened(Option<Opened>),
    ModelRestored(Result<mesh::Mesh, String>),
    Saved(Option<PathBuf>),
    Export(&'static str),
    ToggleStock,
    ToggleUncut,
    /// A change to the 3D view's section.
    Section(section::Edit),
    TogglePaths,
    /// A named view of the 3D view, or "FIT": the scene fitted again in the
    /// view it has.
    Camera(&'static str),
    /// Opens or closes the browser or the inspector beside the 3D view.
    ToggleSidebar(settings::Side),
    /// A sidebar's handle dragged to this width; true once the drag has
    /// ended, when the layout is saved.
    SidebarWidth(settings::Side, f32, bool),
    /// Opens or closes the operation inspector's About disclosure.
    ToggleStrategyHelp,
    /// The window is this wide, in logical pixels.
    Resized(f32),
    Viewport(viewport::Event),
    /// Orbit, pan or zoom in the plan preview's 3D view.
    PlanViewport(viewport::Event),
    Play,
    Scrub(f32),
    /// The playback slider let go: the stock catches up with a scrub too
    /// long to replay while it moved (`SCRUB_LIVE`).
    ScrubDone,
    /// The simulation replayed to where the slider was let go: the epoch
    /// of that catch-up and the simulation.
    Replayed(u64, Replayed),
    /// The load on each tooth along the path of the simulation of this id
    /// (`App::measure_tooth_load`).
    ToothLoaded(String, Option<std::sync::Arc<tooth_load::ToothLoad>>),
    /// The part's holes worked out off the window's thread for picking by
    /// their outline (`App::refresh_part_holes`), with their key.
    PartHoles(u64, PartHoles),
    /// The replay of that catch-up failed: its epoch.
    ReplayFailed(u64),
    Speed(f32),
    StockDetail(stock_surface::Detail),
    StockMeshed {
        key: StockMeshKey,
        seconds: f32,
        result: Result<std::sync::Arc<stock_surface::SurfaceMesh>, String>,
    },
    /// The stock the selected operation starts from, meshed off the window's
    /// thread (`App::refresh_start_mesh`): its key, the job's stock it was
    /// cut from and the surface, or why it could not be meshed.
    StartMeshed {
        key: StartKey,
        volume: std::sync::Arc<volume::Volume>,
        result: Result<std::sync::Arc<stock_surface::SurfaceMesh>, String>,
    },
    Tick,
    Plan,
    /// The text of the planner chat's box changes.
    PlanAsk(String),
    /// The chat's request goes to the planner: a change to the current plan,
    /// or to the preview when one is open, or what to plan.
    PlanSend,
    PlanProgress(u64, planner::Event),
    PlanFinished(u64, Result<std::sync::Arc<PlanPreview>, planner::Failure>),
    PlanCancel,
    /// A picture of the running page opens at full size, or closes if it is
    /// the one open.
    PlanPicture(ui::PlanPicture),
    /// A clipped text of the plan preview opens in full, or closes again.
    PlanExpand(ui::Expand),
    ApplyPlan,
    DiscardPlan,
    UndoPlan,
    Exported(PathBuf),
    ExportError(String),
    PostChecked(u64, std::sync::Arc<post::JobPost>),
    /// The stock each setup starts from and its holding checks, each worked
    /// out for the job with its fingerprint.
    LeftStock(Box<(LeftStock, HoldingChecks)>),
    /// The checks before a run of the active setup, worked out (`run::ready`).
    Ready(Box<run::Ready>),
    ExportGcode,
    /// None when no folder was chosen.
    GcodeExported(Option<Result<(PathBuf, usize), String>>),
    AccountRestored(Result<Option<session::Session>, String>),
    SignIn,
    SignInCancel,
    /// The browser step of a sign-in; see `oauth::approve`. The first value is
    /// the `sign_in_attempt` that started it.
    Approved(u64, Result<oauth::Approval, String>),
    /// The exchange of an approved sign-in; see `oauth::complete`. The first
    /// value is the `sign_in_attempt` that started it.
    SignedIn(u64, Result<(session::Session, String), String>),
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
    /// Whether the AI planner may show the model pictures of the part.
    PlanPictures(bool),
    OpenUrl(&'static str),
    ConfirmDeleteSetup(bool),
    Close(iced::window::Id),
    Captured(iced::window::Screenshot),
    /// The renderer smoke run: the window drew a frame (at the instant) while
    /// the run waited for one, and the window took a logical size.
    Frame(Instant),
    WindowSized(iced::Size),
    HelpHover(bool),
    /// More… and Pause on a hover card: taken by the card's overlay
    /// (`help::HoverHelp`); from the card F1 pinned (`help::pinned`), the
    /// app's, for that card.
    HelpMore(bool),
    HelpPause(bool),
    /// The Steps page of a manual setup.
    Guide(GuideAction),
    Noop,
}
/// What the Steps page of a setup on a manual mill or lathe does
/// (`steps::view`).
#[derive(Debug, Clone)]
pub enum GuideAction {
    /// The next step, or the next hole of a Holes step.
    Next,
    /// The step before, or the hole before.
    Back,
    /// Step `i` of the guide.
    Go(usize),
    /// The first unfinished step of section `i`: the place when it is in
    /// the section, else the section's first step.
    Section(usize),
    /// The readout `i` of the step shown is reached: the next is lit.
    Reached(usize),
    /// Setup `i` on the MACHINE page: its steps, or the CNC's control.
    Open(usize),
    /// The page's clock, for the handwheel's beat.
    Beat(Instant),
    /// The guide of the job with this fingerprint (`App::guide_key`), built.
    Built(u64, Result<std::sync::Arc<steps::Shown>, String>),
    /// The lathe's dial check answered: what its cross-slide dial reads,
    /// saved to the machine in the library and in every setup on it.
    CrossReads(lathe::Reads),
    /// What is typed in a lathe step's measure box (its `guide::measure_key`).
    Typed(String, String),
    /// A diameter measured at a lathe step, mm, kept under its key in
    /// `Document::measured`.
    Measured(String, f32),
}
/// A preview a change asked in the chat started from, as the page showed it.
#[derive(Debug, Clone)]
struct Refined {
    preview: std::sync::Arc<PlanPreview>,
    timeline: Vec<Step>,
    overview: Option<Shown>,
    asked: Option<String>,
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
    /// The pictures the model looked at in this request, and what OpenAI
    /// refused of it.
    pub looks: Vec<Shown>,
}
/// One picture of the timeline, or a line about one: what it says, whether it
/// is a warning, and the picture's handle, made once when the event arrived.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub line: String,
    pub warn: bool,
    pub thumb: Option<iced::widget::image::Handle>,
}
/// Clamp positions typed over a setup's `over` that `holding::layout` refuses.
#[derive(Debug, Clone, PartialEq)]
pub struct ClampsDraft {
    pub setup: String,
    pub text: String,
    pub over: String,
}
type StockMeshKey = (String, u64, u64, stock_surface::Detail);
/// What the stock an operation starts from is meshed for: the fingerprint
/// of what that stock depends on (`op_fingerprint`), the setup and the
/// operation (by index) and the detail.
type StartKey = (u64, usize, usize, stock_surface::Detail);
/// How many of those the app keeps meshed (`App::start_mesh`).
const START_MESHES: usize = 8;
/// A heading and the lines of the box the inspector shows.
pub type HoldingNote = Option<(String, Vec<String>)>;
/// What the status of a generated preview ends with on a CNC machine, and on
/// a hand machine (the manual mill, the lathe), which has no G-code.
pub(crate) const CNC_NEXT: &str = "Export G-code from the NC review page.";
pub(crate) const HAND_NEXT: &str = "Follow the steps on the MACHINE page.";
/// What a lathe preview's status says is checked of a modelled chuck.
pub(crate) const CHUCK_CHECKED: &str = "this cut is checked against the chuck as a box with a clearance gap (a warning shows if it comes too close); the tailstock, the tool post and the machine body are not.";
/// The stock each setup of a job starts from once the earlier setups have cut
/// it, worked out in the background (`App::refresh_left_stock`) for the job
/// whose fingerprint is `key` (`App::left_key`): one entry per setup, None for
/// the first, which starts from its own stock, and for one after an operation
/// or setup that failed. Only good for the job it was worked out for.
#[derive(Debug, Clone, Default)]
pub struct LeftStock {
    pub key: u64,
    pub boxes: Vec<Option<mesh::Bounds>>,
    /// The stock the earlier setups left each setup, cells and cuts, from
    /// which its holding checks are made: None where `boxes` is, and where
    /// nothing was cut before it.
    pub before: Vec<Option<std::sync::Arc<volume::Volume>>>,
    /// The stock each setup leaves, cells and cuts: None where nothing was
    /// cut by its end, and from an operation that failed on.
    pub after: Vec<Option<std::sync::Arc<volume::Volume>>>,
    /// The surface of each stock in `before` and `after`, meshed at Draft
    /// detail for the browser's pictures (`ui::stock_picture`).
    pub surfaces: Vec<(
        std::sync::Arc<volume::Volume>,
        std::sync::Arc<stock_surface::SurfaceMesh>,
    )>,
    /// What each operation that generated cuts (`forces::bites`), by setup:
    /// its index in the setup and its bites.
    pub bites: Vec<Vec<(usize, Vec<forces::Bite>)>>,
    /// The faces of the part the stock the last setup leaves still covers
    /// (`uncut::uncut`), or why that is not checked: no operation has cut
    /// yet, or one could not be generated. None until worked out.
    pub uncut: Option<Result<std::sync::Arc<uncut::Uncut>, String>>,
    /// The stock the job leaves, cells and cuts, as far as every operation
    /// worked: up to the one that failed (`broken`). Each cut says which
    /// operation made it, so the stock any operation up to that one starts
    /// from is its cuts made before it (`App::start_source`). None where
    /// nothing was cut.
    pub job: Option<std::sync::Arc<volume::Volume>>,
    /// The first operation that could not be generated, by setup and index.
    pub broken: Option<(usize, usize)>,
    /// The `op_fingerprint` of each operation of each setup when this was
    /// worked out, and last of the setup's end: the stock it starts from, or
    /// the setup leaves, stays good while it is the same.
    pub ops: Vec<Vec<u64>>,
    /// The `start_fingerprint` of each setup when this was worked out: a
    /// setup's box and `before` stay good while it is unchanged, whatever
    /// the later setups and its own operations become.
    pub starts: Vec<u64>,
    /// The fingerprint of the part it was worked out on.
    pub part: String,
}
/// What the holding checks found for each setup of a job, worked out with
/// `LeftStock` (`holding_checks`) for the job and its workholding whose
/// fingerprint is `key` (`App::holding_key`): one entry per setup, None
/// wherever `LeftStock` knows no box (the first setup's is its own). Only
/// good for the job and the workholding it was worked out for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HoldingChecks {
    pub key: u64,
    pub setups: Vec<Option<SetupHolding>>,
    /// The `grip_fingerprint` of each setup when these were worked out: a
    /// setup's `held` stays good while it is unchanged.
    pub grips: Vec<u64>,
}
/// The holding checks of a setup.
#[derive(Debug, Clone, PartialEq)]
pub struct SetupHolding {
    /// The box it starts from and the band its vise closes on
    /// (`holding::gripped`; the box itself for clamps or no workholding).
    pub held: holding::Gripped,
    /// What `holding::holdable` finds of its vise or clamps on what the
    /// earlier setups left, as the export does: empty when it holds, and
    /// when its workholding has a problem `Document::holding_issues_with`
    /// says first. Then each operation's issue against the cutting forces
    /// (`forces::issue`).
    pub problems: Vec<String>,
    /// Each operation's margin against the cutting forces (`forces::line`),
    /// once what it holds passes.
    pub margins: Vec<String>,
}
/// Setup settings that decide how the setup is held and nothing the earlier
/// setups' cuts leave behind: an edit of one keeps the stock worked out.
const HOLDING_KEYS: [&str; 11] = [
    "holding",
    "holder",
    "vise_axis",
    "vise_offset",
    "vise_grip",
    "clamps",
    "holding_gap",
    "chuck_jaws",
    "chuck_stickout",
    "chuck_tailstock",
    "chuck_jaw_angle",
];
/// What `cam::holes` answers for the Drill editor's list.
pub(crate) type DrillHoles = Result<Vec<cam::Hole>, String>;
pub(crate) type BottomHoles = Result<Vec<cam::BottomHole>, String>;
pub(crate) type PocketHoles = Result<Vec<pocket::Hole>, String>;
/// The cutter's diameter and, for each picked hole, which of its levels it
/// fits and, in a Pocket, how low it clears the stock over one it fits none
/// of (`pocket::Hole::cleared_to`); None without holes or a flat end mill.
pub(crate) type PocketFits = Option<(f32, Vec<Vec<bool>>, Vec<Option<pocket::Cleared>>)>;
#[cfg(test)]
thread_local! {
    /// How many times this thread has run `cam::holes` for the Drill editor.
    pub(crate) static DRILL_HOLE_RUNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
/// A writer that feeds what is written to a hasher.
struct HashWriter<'a>(&'a mut std::hash::DefaultHasher);
impl std::io::Write for HashWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        use std::hash::Hasher;
        self.0.write(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
/// A fingerprint of what the stock each setup of `doc` starts from depends
/// on: its setups with their operations (less how they are held), its tools
/// and the part. None when it cannot be written.
fn left_fingerprint(doc: &Document, mesh: &mesh::Mesh) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    let mut out = HashWriter(&mut hasher);
    for setup in &doc.setups {
        let p: Vec<_> = setup
            .p
            .iter()
            .filter(|(k, _)| !HOLDING_KEYS.contains(&k.as_str()))
            .collect();
        serde_json::to_writer(&mut out, &(&setup.id, &setup.machine, p, &setup.operations)).ok()?;
    }
    serde_json::to_writer(&mut out, &doc.tools).ok()?;
    mesh.fingerprint.hash(&mut hasher);
    Some(hasher.finish())
}
/// A fingerprint of what the stock setup `si` of `doc` starts from depends
/// on (`left_fingerprint` of the setups before it), with the setup's own
/// placement but not its operations, which leave it as it is. None when it
/// cannot be written.
fn start_fingerprint(doc: &Document, mesh: &mesh::Mesh, si: usize) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    let mut out = HashWriter(&mut hasher);
    let placed = |setup: &Setup| -> Vec<(String, String)> {
        setup
            .p
            .iter()
            .filter(|(k, _)| !HOLDING_KEYS.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    };
    for setup in doc.setups.get(..si)? {
        serde_json::to_writer(
            &mut out,
            &(&setup.id, &setup.machine, placed(setup), &setup.operations),
        )
        .ok()?;
    }
    let setup = doc.setups.get(si)?;
    serde_json::to_writer(&mut out, &(&setup.id, &setup.machine, placed(setup))).ok()?;
    serde_json::to_writer(&mut out, &doc.tools).ok()?;
    mesh.fingerprint.hash(&mut hasher);
    Some(hasher.finish())
}
/// A fingerprint of what the stock operation `oi` of setup `si` of `doc`
/// starts from depends on: what the setup's does (`start_fingerprint`) and
/// the operations before it in the setup, but not it or those after it.
/// `oi` may be the number of operations: the stock the setup leaves. None
/// when it cannot be written.
pub(crate) fn op_fingerprint(
    doc: &Document,
    mesh: &mesh::Mesh,
    si: usize,
    oi: usize,
) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    start_fingerprint(doc, mesh, si)?.hash(&mut hasher);
    let before = doc.setups.get(si)?.operations.get(..oi)?;
    serde_json::to_writer(HashWriter(&mut hasher), before).ok()?;
    Some(hasher.finish())
}
/// A fingerprint of what the band setup `si`'s vise closes on depends on
/// (`holding::held_by`): the stock it starts from (`start`, its
/// `start_fingerprint`), how it is held and the workholding library. None
/// when it cannot be written.
fn grip_fingerprint(doc: &Document, start: u64, si: usize) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    start.hash(&mut hasher);
    let mut out = HashWriter(&mut hasher);
    let setup = doc.setups.get(si)?;
    let held: Vec<_> = HOLDING_KEYS.iter().map(|k| get(&setup.p, k)).collect();
    serde_json::to_writer(&mut out, &held).ok()?;
    serde_json::to_writer(&mut out, &doc.workholding).ok()?;
    Some(hasher.finish())
}
/// A fingerprint of what the holding checks of `doc` depend on beyond the
/// stock (`left`, its `left_fingerprint`): how each setup is held, the
/// workholding library and the material the cutting forces are worked out
/// for. None when it cannot be written.
fn holding_fingerprint(doc: &Document, left: u64) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    left.hash(&mut hasher);
    let mut out = HashWriter(&mut hasher);
    for setup in &doc.setups {
        let held: Vec<_> = HOLDING_KEYS.iter().map(|k| get(&setup.p, k)).collect();
        serde_json::to_writer(&mut out, &held).ok()?;
    }
    serde_json::to_writer(&mut out, &doc.workholding).ok()?;
    serde_json::to_writer(&mut out, &doc.selected_material()).ok()?;
    Some(hasher.finish())
}
/// The box each setup of `doc` starts from when the earlier ones are
/// simulated on `mesh` (`simulation::sequence_walk`, as the simulation itself
/// gives it to the setup's operations), and the stock they left it: None for
/// the first setup, and for a setup after an operation or setup that failed.
/// A setup without an enabled operation has its box too. The whole job is
/// simulated, for the stock each setup leaves too, and each stock is meshed
/// for the browser's pictures. What the last stock still covers of the part
/// is worked out too. For the job whose fingerprint is `key`.
fn leftover_stock(doc: &Document, mesh: &mesh::Mesh, key: u64) -> LeftStock {
    use std::{
        cell::{Cell, RefCell},
        sync::Arc,
    };
    let mut boxes = vec![None; doc.setups.len()];
    let mut before = vec![None; doc.setups.len()];
    // The stock after the last operation so far.
    let last: RefCell<Option<Arc<volume::Volume>>> = Default::default();
    // The stock each setup leaves while every operation so far has worked:
    // what it starts from, then what each of its operations leaves.
    let after = RefCell::new(vec![None; doc.setups.len()]);
    let failed = Cell::new(false);
    // The first operation that could not be generated.
    let mut broken = None;
    let mut bites = vec![vec![]; doc.setups.len()];
    simulation::sequence_walk(
        doc,
        mesh,
        None,
        |si, oi, result| match result {
            Ok(sim) => bites[si].push((oi, forces::bites(&sim))),
            Err(_) => {
                failed.set(true);
                after.borrow_mut()[si] = None;
                broken.get_or_insert((si, oi));
            }
        },
        |si, _, stock| {
            let stock = Arc::new(stock.clone());
            after.borrow_mut()[si] = Some(stock.clone());
            *last.borrow_mut() = Some(stock);
        },
        |si, stock| {
            boxes[si] = stock;
            if stock.is_some() {
                before[si] = last.borrow().clone();
            }
            if !failed.get() {
                after.borrow_mut()[si] = last.borrow().clone();
            }
        },
    );
    let after = after.into_inner();
    let job = last.into_inner();
    let uncut = Some(match (broken, &job) {
        (Some((si, oi)), _) => Err(format!(
            "{} / {} could not be generated.",
            doc.setups[si].name(),
            doc.setups[si].operations[oi].name()
        )),
        (None, None) => Err("No operation has cut the stock yet.".into()),
        (None, Some(stock)) => Ok(Arc::new(uncut::uncut(stock, &doc.setups[0], mesh))),
    });
    // A setup starts from the very stock the one before it leaves: each is
    // meshed once. ponytail: in the run the holding checks wait for; a run
    // of its own if that delay ever shows.
    let mut surfaces: Vec<(Arc<volume::Volume>, Arc<stock_surface::SurfaceMesh>)> = vec![];
    for volume in before.iter().chain(&after).flatten() {
        if surfaces.iter().any(|(v, _)| Arc::ptr_eq(v, volume)) {
            continue;
        }
        let surface = stock_surface::Snapshot::of(volume, stock_surface::Detail::Draft).build();
        if let Ok(surface) = surface {
            surfaces.push((volume.clone(), Arc::new(surface)));
        }
    }
    LeftStock {
        key,
        boxes,
        before,
        after,
        surfaces,
        bites,
        uncut,
        job,
        broken,
        ops: (doc.setups.iter().enumerate())
            .map(|(si, s)| {
                (0..=s.operations.len())
                    .map(|oi| op_fingerprint(doc, mesh, si, oi).unwrap_or_default())
                    .collect()
            })
            .collect(),
        starts: (0..doc.setups.len())
            .map(|si| start_fingerprint(doc, mesh, si).unwrap_or_default())
            .collect(),
        part: mesh.fingerprint.clone(),
    }
}
/// The holding checks of each setup of `doc` whose box `left` knows (the
/// first setup's being its own), as the export makes them
/// (`post::held_walked`): the band its vise closes on, and, when
/// `Document::holding_issues_with` finds nothing wrong with its workholding
/// on that, what `holding::holdable` finds on the stock the earlier setups
/// left and the part `mesh`, then, when that holds, each operation's margin
/// against the cutting forces (`forces::margins`). For the job and
/// workholding whose fingerprint is `key`.
fn holding_checks(doc: &Document, mesh: &mesh::Mesh, left: &LeftStock, key: u64) -> HoldingChecks {
    let names = |s, o| holding::operation_name(doc, s, o);
    // The part on the stock's grid, once: the stock every setup starts from
    // is on setup 1's grid (`leftover_stock`).
    let mut grid: Option<plan_check::PartGrid> = None;
    let setups = doc
        .setups
        .iter()
        .enumerate()
        .map(|(si, setup)| {
            let stock = match si {
                0 => doc.setup_stock(0, Some(mesh))?,
                _ => left.boxes.get(si).copied().flatten()?,
            };
            let before = left.before.get(si).cloned().flatten();
            let frame = volume::Frame::new(setup, mesh);
            let part = simulation::part_box(setup, mesh);
            let layout = holding::layout(setup).ok();
            let bands = before
                .as_deref()
                .filter(|v| !v.cuts.is_empty())
                .map(|v| holding::Bands::new(v, frame, part));
            let held = holding::held_by(doc, setup, stock, bands.as_ref());
            let (mut problems, mut margins) = (vec![], vec![]);
            if chuck::holds(setup)
                && doc.holding_issues_with(si, Some(held)).is_empty()
                && let Ok(rig) = doc.chuck_rig(setup, held.stock)
            {
                let removed = before.as_deref().map(|v| rest::Removed::new(v, frame));
                problems.extend(rig.grip_issue(removed.as_ref()));
                let bites = left.bites.get(si).into_iter().flatten();
                let bites = bites.map(|(oi, b)| (*oi, b.as_slice()));
                if problems.is_empty() {
                    for (oi, margin) in forces::chuck_margins(doc, si, bites, &rig) {
                        let name = setup.operations[oi].name();
                        margins.push(forces::chuck_line(name, margin.as_ref()));
                        if let Some(m) = margin {
                            problems.extend(forces::chuck_issues(name, &m));
                        }
                    }
                }
            }
            if let (Some(layout), Some(holder)) = (layout, doc.holder(get(&setup.p, "holder")))
                && doc.holding_issues_with(si, Some(held)).is_empty()
            {
                let removed = before.as_deref().map(|v| {
                    let grid =
                        grid.get_or_insert_with(|| plan_check::PartGrid::in_job(v, doc, mesh));
                    rest::Removed::new(v, frame).with_part(grid)
                });
                problems = holding::holdable(si, &layout, holder, held, removed.as_ref(), &names);
                let bites = left.bites.get(si).into_iter().flatten();
                let bites = bites.map(|(oi, b)| (*oi, b.as_slice()));
                if problems.is_empty() {
                    for (oi, margin) in forces::margins(doc, si, bites, held, removed.as_ref()) {
                        let name = setup.operations[oi].name();
                        margins.push(forces::line(name, margin.as_ref()));
                        problems.extend(margin.and_then(|m| forces::issue(name, &m)));
                    }
                }
            }
            Some(SetupHolding {
                held,
                problems,
                margins,
            })
        })
        .collect();
    let grips = (0..doc.setups.len())
        .map(|si| {
            left.starts
                .get(si)
                .and_then(|&start| grip_fingerprint(doc, start, si))
                .unwrap_or_default()
        })
        .collect();
    HoldingChecks { key, setups, grips }
}
pub struct App {
    pub smoke_dir: Option<PathBuf>,
    pub smoke_step: usize,
    pub help_animating: bool,
    pub smoke_pending: bool,
    /// The renderer smoke run's wait for the frame that shows its last step
    /// (`smoke::frames`): the frames drawn since, when the wait began,
    /// whether the window is to take a new size first, the window's logical
    /// size, and the last capture's hash.
    pub smoke_frames: u32,
    pub smoke_changed: Instant,
    pub smoke_resize: bool,
    pub smoke_window: Option<iced::Size>,
    pub smoke_last: Option<u64>,
    pub doc: Document,
    pub page: Page,
    /// The Manufacture page the workspace switcher goes back to from the
    /// Machine workspace (the Control page).
    pub cam_page: Page,
    pub selected: Option<usize>,
    /// Whether the browser's "Stock after this setup" is selected: with no
    /// operation selected, the 3D view shows the stock the active setup
    /// leaves (`start_source`). Dropped with the selection
    /// (`stop_selection`) and once an operation is selected.
    pub setup_end: bool,
    pub tab: String,
    /// The setup editor's step shown (`ui::SETUP_STEPS`).
    pub setup_step: &'static str,
    pub status: String,
    /// The page the status line was last set on: on another page it is an
    /// earlier event's, and the footer shows it as such.
    pub status_page: Page,
    pub audit_report: Vec<AuditRow>,
    /// The field whose help the F1 key pinned (`ui::view` draws it), and
    /// whether its card is open to its More… text and its animation paused.
    pub pinned_help: Option<help::FieldTag>,
    pub pinned_expanded: bool,
    pub pinned_paused: bool,
    pub post: Option<std::sync::Arc<post::JobPost>>,
    pub post_busy: bool,
    /// The stock the earlier setups left each later setup, once worked out.
    pub left_stock: LeftStock,
    /// The holding checks of each later setup on that stock, once worked out.
    pub holding_checks: HoldingChecks,
    /// The fingerprint of the job a background run is working out the stock
    /// for.
    left_job: Option<u64>,
    /// The step guide of the active setup on a manual mill and its
    /// pictures, built in the background (`refresh_guide`) for the job whose
    /// fingerprint is the first value (`guide_key`).
    pub guide: Option<(u64, Result<std::sync::Arc<steps::Shown>, String>)>,
    /// The fingerprint of the job a background run is building a guide for.
    guide_job: Option<u64>,
    /// The readout the Steps page is at within its step, or the hole of a
    /// Holes step.
    pub guide_item: usize,
    /// What is typed in a lathe step's measure box: its key and the text.
    pub guide_typed: Option<(String, String)>,
    /// When the step shown was opened, and the page's clock since
    /// (`GuideAction::Beat`): the handwheel's beat.
    pub guide_clock: (Instant, Instant),
    /// The checks before a run of a setup, once worked out (`current_ready`).
    pub ready: run::Ready,
    /// The fingerprint of the job a background run is checking a setup of.
    ready_job: Option<u64>,
    /// The workholding warning of the preview shown (`ui::holding_warning`)
    /// and what it was worked out from.
    holding_note: std::cell::RefCell<Option<(String, HoldingNote)>>,
    /// The Drill editor's hole list and what it was worked out from
    /// (`App::drill_holes`).
    drill_holes: std::cell::RefCell<Option<(u64, std::sync::Arc<DrillHoles>)>>,
    /// The round holes the part opens on the current setup's bottom, kept
    /// as `drill_holes` is (`App::bottom_holes`).
    bottom_holes: std::cell::RefCell<Option<(u64, std::sync::Arc<BottomHoles>)>>,
    /// The holes a Pocket's or Contour's selected walls pick, kept as
    /// `drill_holes` is (`App::pocket_holes`).
    pocket_holes: std::cell::RefCell<Option<(u64, std::sync::Arc<PocketHoles>)>>,
    /// Every hole of the part a Pocket or a Contour can pick in the current
    /// setup (`pocket::part_holes`), with what it was worked out from
    /// (`App::part_holes_key`): their outlines are what the 3D view picks a
    /// hole by. Worked out off the window's thread (`part_holes_job`).
    part_holes: Option<(u64, std::sync::Arc<FoundHoles>)>,
    part_holes_job: Option<u64>,
    /// Which levels of those holes the operation's cutter fits (`Hole::fits`),
    /// kept as they are (`App::pocket_fits`).
    pocket_fits: std::cell::RefCell<Option<(u64, std::sync::Arc<PocketFits>)>>,
    /// The playback slider moved further than `SCRUB_LIVE` replays: the stock
    /// shown is where it was until the slider is let go (`ScrubDone`).
    pub(crate) scrub_pending: bool,
    /// The most moves a scrub replays while the slider moves (`SCRUB_LIVE`).
    pub(crate) scrub_live: usize,
    /// The catch-up replaying off the window's thread (`App::catch_up`): its
    /// epoch, where it goes, and whether playback starts when it is done.
    pub(crate) catching_up: Option<(u64, f32, bool)>,
    /// When the playback slider last moved: a scrub still to be replayed is
    /// replayed once it has been still for `SCRUB_SETTLE` (the arrow keys
    /// and the wheel move it without letting go).
    pub(crate) last_scrub: Instant,
    catch_up_epoch: u64,
    /// The generation under way off the window's thread, if any.
    pub(crate) generating: Option<Generating>,
    /// The last generation started: one that comes back with another is
    /// stale.
    generation_epoch: u64,
    /// Generate where it is asked, not on a thread of its own: in the
    /// headless smoke run and the unit tests, as their other work is.
    pub(crate) generate_inline: bool,
    /// The load on each tooth being worked out off the window's thread for
    /// the simulation of this id, and its Cancel (`App::measure_tooth_load`).
    pub(crate) tooth_load_job: Option<(String, std::sync::Arc<progress::Progress>)>,
    /// The browser's pictures of stock, each drawn once for what it shows
    /// (`ui::stock_picture`).
    pub(crate) stock_pictures: std::cell::RefCell<Vec<(u64, iced::widget::image::Handle)>>,
    post_epoch: u64,
    /// The check epoch of the run whose stock is being pictured
    /// (`picture_run`).
    picture_job: Option<u64>,
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
    /// Counts the sign-ins started, so a result of an earlier one is not taken
    /// for the current one's.
    sign_in_attempt: u64,
    /// Sends the user to the sign-in page; tests replace it.
    open_browser: BrowserOpener,
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
    /// The window's width, in logical pixels, from its last resize; the
    /// Work page's sidebars are fitted to it (`ui::fitted`).
    pub window_width: f32,
    pub sample_mesh: mesh::Mesh,
    pub imported_mesh: Option<mesh::Mesh>,
    pub import_busy: bool,
    pub save_busy: bool,
    pub last_archived: Option<PathBuf>,
    pub import_error: Option<String>,
    pub stock: bool,
    /// "Show uncut faces": the part's faces the whole job leaves under stock
    /// are drawn magenta (`uncut::Uncut::faces`), over the simulated stock too.
    pub show_uncut: bool,
    /// The 3D view's section (`section`), with the fingerprint of the part
    /// it was set on: a face's number means another face on another part.
    /// Kept while the section is off, so turning it on again brings it back.
    pub section: Option<(section::Section, String)>,
    /// Whether the 3D view is cut on `section`.
    pub section_on: bool,
    /// Whether a click on the part picks the face the section goes through.
    pub picking_section: bool,
    pub paths: bool,
    pub picking: bool,
    /// "Pick holes on the part" in the Work zero step: a click on the part
    /// chooses or removes the hole it is in for the fixture pins, while
    /// that step of a setup on the pins is open (`picking_pin_holes`).
    pub picking_pins: bool,
    /// "Place the pins on the leftover" in the Work zero step: the pins
    /// follow the pointer over the 3D view and a click puts pin 1 there,
    /// while that step of a setup on pins in its leftover is open
    /// (`placing_leftover_pins`).
    pub placing_pins: bool,
    /// Where pin 1 would stand under the pointer while placing the pins
    /// (plan mm): drawn as the pins there, red where they do not fit.
    pub pin_aim: Option<[f32; 2]>,
    pub playing: bool,
    pub simulation: Option<simulation::Simulation>,
    pub stock_detail: stock_surface::Detail,
    stock_mesh_job: Option<StockMeshKey>,
    /// When the running mesher job started, for the elapsed time the strip names.
    stock_mesh_started: Option<Instant>,
    /// The last mesher job took long enough to be named
    /// (`ui::STOCK_NOTICE_AFTER`): the next one is named at once.
    stock_mesh_slow: bool,
    stock_mesh_failed: Option<StockMeshKey>,
    /// The stock the operations last selected start from, meshed
    /// (`refresh_start_mesh`): each one's key, the job's stock and the
    /// surface, the latest last. A few are kept, so that going back to an
    /// operation shows it at once.
    start_mesh: Vec<(
        StartKey,
        std::sync::Arc<volume::Volume>,
        std::sync::Arc<stock_surface::SurfaceMesh>,
    )>,
    /// The key of the one being meshed, and of the last that could not be.
    start_mesh_job: Option<StartKey>,
    start_mesh_failed: Option<StartKey>,
    pub progress: f32,
    pub speed: f32,
    pub last_tick: Instant,
    pub dirty: bool,
    /// Why the last automatic save of the workspace failed, until one succeeds.
    pub save_error: Option<String>,
    pub last_edit: Instant,
    pub state_path: PathBuf,
    /// The machine controller, while connected.
    pub control: Option<control::Connection>,
    /// The Control page shows the controller configuration.
    pub machine_setup: bool,
    /// The profile being edited in the controller configuration, for the machine with this id.
    pub profile_draft: Option<(String, Result<setup::Draft, String>)>,
    /// The UC100 machine chosen on the Control page (machine id).
    pub control_machine: Option<String>,
    /// The machine (id) picked on the Control page while another is
    /// connected: the page asks whether to disconnect and switch to it.
    pub switch_machine: Option<String>,
    /// The manual-mill setup (id) whose Control page shows the CNC instead of
    /// its steps, after a CNC machine was picked there. Making another setup
    /// active shows the steps again.
    pub cnc_over_guide: Option<String>,
    /// The setup (id) Run on the machine picked while disconnected: Connect
    /// picks it in the Run panel.
    run_on_connect: Option<String>,
    /// The jog key down on the Control page (axis, negative).
    pub jog_key: Option<(usize, bool)>,
    /// Counts the checks of setups to run, so a late answer is ignored.
    run_epoch: u64,
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
    /// The text of the model's reply streamed so far in the current run.
    pub plan_stream: std::sync::Arc<std::sync::Mutex<String>>,
    /// The picture the current run's first request carries.
    pub plan_overview: Option<Shown>,
    /// The picture of the running page shown at full size, if any.
    pub plan_enlarged: Option<ui::PlanPicture>,
    /// The clipped texts of the plan preview shown in full.
    pub plan_expanded: std::collections::BTreeSet<ui::Expand>,
    /// Blocking stages still running, also those of cancelled runs.
    pub plan_busy: planner::Busy,
    /// Set by an edit that dropped a running plan or a preview, or by a
    /// sign-out that stopped a run: why the plan went.
    plan_dropped: Option<&'static str>,
    /// The plan preview's own camera: the machining viewport's stays put.
    pub plan_camera: viewport::Camera,
    /// The text in the planner chat's box.
    pub plan_request: String,
    /// The planner chat's exchanges so far in this project, oldest first.
    pub plan_chat: Vec<planner::Turn>,
    /// The chat request of the current run, or of the preview or failure it
    /// left; None for a run of Plan this job.
    pub plan_asked: Option<String>,
    /// The preview a running change started from, which comes back when the
    /// change fails or is cancelled.
    plan_refined: Option<Refined>,
    pub undo: Option<Document>,
    pub confirm_delete_setup: bool,
    /// The operation editor asks before deleting the selected operation.
    pub confirm_delete_op: bool,
    /// An action that sent the user to a page to make what it needs first
    /// (Face to the Tool library), and that page: saving there carries the
    /// action out; leaving the page drops it.
    pub pending: Option<(Page, Message)>,
    pub icons: Vec<iced::widget::svg::Handle>,
    /// A close request waiting for a sign-out, a sign-in save or a refresh
    /// of the sign-in, and when it first came.
    closing: Option<(iced::window::Id, Instant)>,
    /// The client a pending sign-out is signing out, so that a close that
    /// stops waiting for it can delete the saved sign-in through it.
    signing_out: Option<std::sync::Arc<openai::Client>>,
    /// G-code exports started and not yet reported (folder choice and write).
    gcode_exports: usize,
    /// The folder a test app is rooted in, removed when the app is dropped.
    #[cfg(test)]
    test_dir: Option<tempfile::TempDir>,
}
/// A line of the operation audit's report (`Message::GenerateAll`): the setup
/// and operation it is about, by index, their names, whether the operation
/// generated, and what happened.
/// The most moves a scrub of the playback slider replays on the window's
/// thread while it moves: 200,000 took 0.13–0.5 s, so this is a few
/// hundredths. A longer one waits until the slider is let go and is then
/// replayed off the window's thread (`App::catch_up`).
pub(crate) const SCRUB_LIVE: usize = 50_000;
/// How long the playback slider stays still before a scrub still to be
/// replayed is (`App::last_scrub`).
const SCRUB_SETTLE: Duration = Duration::from_millis(400);
/// A simulation replayed off the window's thread to where the slider was
/// let go (`App::catch_up`).
#[derive(Clone)]
pub struct Replayed(std::sync::Arc<simulation::Simulation>);
impl std::fmt::Debug for Replayed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Replayed")
    }
}
/// The part's holes worked out off the window's thread (`App::part_holes`).
#[derive(Clone)]
pub struct PartHoles(std::sync::Arc<FoundHoles>);
impl std::fmt::Debug for PartHoles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PartHoles({})", self.0.holes.len())
    }
}
/// Every hole of the part a wall click picks (`pocket::part_holes`), and
/// each read as a drill reads it (`cam::drill_holes_of`): None where it is
/// not round.
#[derive(Default)]
pub struct FoundHoles {
    pub holes: Vec<pocket::Hole>,
    pub round: Vec<Option<cam::Hole>>,
}
/// Whether a click on a wall of the part picks its whole hole in `op`: a
/// Pocket, a Contour, and a Drill of the part's holes (not of the pins').
pub(crate) fn picks_holes(op: &Operation) -> bool {
    matches!(op.strategy, Strategy::Pocket | Strategy::Contour)
        || (op.strategy == Strategy::Drill && !op.drills_pin_holes())
}
/// Whether a click on the part picks the whole smooth face it lands on in
/// `op` (`mesh::Mesh::region_of`), a curved one included: Parallel and
/// Waterline finishing.
pub(crate) fn picks_faces(op: &Operation) -> bool {
    matches!(op.strategy, Strategy::Parallel | Strategy::Waterline)
}
/// The part's holes as `setup` places it, for picking by their outline.
pub(crate) fn find_holes(setup: &model::Setup, mesh: &mesh::Mesh) -> FoundHoles {
    let holes = pocket::part_holes(setup, mesh);
    let round = cam::drill_holes_of(setup, mesh, &holes);
    FoundHoles { holes, round }
}
/// What a generation off the window's thread makes (`App::generate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Generation {
    /// The selected operation of the current setup, by index, with the stock
    /// the operations before it leave.
    Operation(usize, usize),
    /// Every enabled operation of the job, for the audit (Generate all).
    All,
    /// The operations of setup `0` (by index), for where its vise jaws or
    /// clamps go (`App::suggest_layout`).
    Layout(usize),
}
/// What `Generation::Layout` made: the setup's toolpaths (None for one that
/// could not be generated) and the band a vise closes on.
#[derive(Clone)]
pub struct Laid(std::sync::Arc<(Vec<Option<simulation::Plan>>, Option<holding::Bands>)>);
impl std::fmt::Debug for Laid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Laid({} plans)", self.0.0.len())
    }
}
/// A generation under way: what it makes, its progress and Cancel, and
/// whether playback starts when it is done (Generate & play).
pub(crate) struct Generating {
    pub epoch: u64,
    pub what: Generation,
    pub progress: std::sync::Arc<progress::Progress>,
    pub play: bool,
}
/// What `Generation::Operation` made: the operation's simulation, or why
/// it could not be generated.
#[derive(Clone)]
pub struct Made(std::sync::Arc<Result<simulation::Simulation, String>>);
impl std::fmt::Debug for Made {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match &*self.0 {
            Ok(_) => "Made(Ok)",
            Err(_) => "Made(Err)",
        })
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct AuditRow {
    pub setup: usize,
    pub operation: usize,
    pub name: String,
    pub ok: bool,
    pub detail: String,
}
pub const LOCKED: &str = "The machine is running: planning is locked until it stops.";
/// What still works while planning is locked (`App::locked`): the Machine
/// workspace (the Control page), looking around, saving, the account, and the
/// answers of work started before. Everything else waits, and the Manufacture
/// workspace does not open.
fn while_locked(m: &Message) -> bool {
    use Message::*;
    matches!(
        m,
        Page(crate::Page::Control)
            | Connect
            | Disconnect
            | MachineReset
            | MachineEStop
            | Machine(_)
            | ControlSetup(_)
            | Setup(_)
            | JogKey(..)
            | JogKeyChecked(..)
            | JogKeyUp
            | HoldKey
            | TabKey(_)
            | HelpKey
            | HelpFor(_)
            | EscapeKey
            | Run(_)
            | RunChecked(..)
            | RunCheckedStock(..)
            | RunPictures(..)
            | ToggleStock
            | ToggleUncut
            | Section(_)
            | TogglePaths
            | Camera(_)
            | ToggleSidebar(_)
            | SidebarWidth(..)
            | ToggleStrategyHelp
            | Resized(_)
            | Viewport(_)
            | PlanViewport(_)
            | Speed(_)
            | StockMeshed { .. }
            | Tick
            | PlanProgress(..)
            | PlanFinished(..)
            | PlanCancel
            | PlanPicture(_)
            | PlanExpand(_)
            | PostChecked(..)
            | LeftStock(_)
            | PartHoles(..)
            | SaveProject
            | SaveProjectAs
            | Saved(_)
            | FileImported(_)
            | ProjectOpened(_)
            | ModelRestored(_)
            | Exported(_)
            | ExportError(_)
            | GcodeExported(_)
            | AccountRestored(_)
            | SignIn
            | SignInCancel
            | Approved(..)
            | SignedIn(..)
            | SessionSaved(_)
            | SignOut
            | SignedOut(_)
            | ModelsLoaded(..)
            | LoadModels
            | PickModel(_)
            | CheckConnection
            | ConnectionChecked(..)
            | PlanNoticeDismissed
            | PlanPictures(_)
            | PlanAsk(_)
            | OpenUrl(_)
            | Close(_)
            | Captured(_)
            | HelpHover(_)
            | HelpMore(_)
            | HelpPause(_)
            | Guide(_)
            | Noop
    )
}
type PlanSender = iced::futures::channel::mpsc::Sender<Message>;
/// The Plan task: runs the planner, sends each stage as it starts, builds the
/// 3D view of a plan that passed and sends the result. Every message carries
/// the run's `epoch`, so the app can ignore those of a dropped run.
pub(crate) async fn run_plan(
    backend: &(impl planner::Backend + Sync),
    doc: Document,
    mesh: mesh::Mesh,
    chat: planner::Chat,
    busy: planner::Busy,
    epoch: u64,
    sender: PlanSender,
) {
    let view = planner::final_view;
    run_plan_with(backend, doc, mesh, chat, busy, epoch, sender, view).await
}
/// `run_plan` with the 3D view's blocking stage given, so tests can hold it.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_plan_with(
    backend: &(impl planner::Backend + Sync),
    doc: Document,
    mesh: mesh::Mesh,
    chat: planner::Chat,
    busy: planner::Busy,
    epoch: u64,
    mut sender: PlanSender,
    view: impl FnOnce(&Document, &mesh::Mesh) -> Option<simulation::Simulation> + Send + 'static,
) {
    use iced::futures::SinkExt;
    let mut stages = sender.clone();
    let result = planner::plan_chat(backend, doc, mesh.clone(), chat, &busy, move |event| {
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
/// How far the 3D views zoom out and in from the fitted scene.
const ZOOM_OUT: f32 = 0.3;
const ZOOM_IN: f32 = 100.;
/// Orbit, pan or zoom `camera` as a viewport asks; picks do not move it.
fn steer(camera: &mut viewport::Camera, event: &viewport::Event) {
    match *event {
        viewport::Event::Orbit(x, y, shift) => {
            *camera = viewport::orbited(*camera, x, y);
            for (pan, shift) in camera.pan.iter_mut().zip(shift) {
                *pan += shift;
            }
        }
        viewport::Event::Pan(x, y) => {
            camera.pan[0] += x;
            camera.pan[1] += y
        }
        viewport::Event::Zoom(v, at) => {
            let zoom = (camera.zoom * (1. + v).clamp(0.5, 1.5)).clamp(ZOOM_OUT, ZOOM_IN);
            // The point under the pointer stays there: the centre moves
            // toward the pointer as the view grows and away as it shrinks.
            let k = zoom / camera.zoom;
            for (pan, at) in camera.pan.iter_mut().zip(at) {
                *pan += at * (1. - k);
            }
            camera.zoom = zoom
        }
        viewport::Event::Pick(_)
        | viewport::Event::PickSurface(_)
        | viewport::Event::SectionSurface(_)
        | viewport::Event::AimPins(_)
        | viewport::Event::PlacePins(_) => {}
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
        let (settings, settings_note) = settings::load_reporting(&settings_path);
        let shared = storage::data_dir().1;
        let store = session::default_store(
            state_path.parent().unwrap_or(std::path::Path::new(".")),
            isolated,
            shared,
        );
        // A load that changed something has notes; the workspace is then
        // saved once, so they are not repeated and what the load added, a
        // material for a legacy name, keeps its identifier.
        let (mut doc, mut status, mut migrated) = if state_path.exists() && smoke_dir.is_none() {
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
                    // The unreadable file is moved aside, so the recovery
                    // workspace is saved as `workspace.json` and the next
                    // start resumes it. When it cannot be moved, a separate
                    // file is used for this start only.
                    let aside =
                        state_path.with_file_name(format!("workspace-unreadable-{}.json", id()));
                    let kept = match std::fs::rename(&state_path, &aside) {
                        Ok(()) => format!("Original file preserved as {}.", aside.display()),
                        Err(re) => {
                            state_path = state_path
                                .with_file_name(format!("workspace-recovery-{}.json", id()));
                            format!(
                                "Original file preserved (it could not be moved aside: {re}). A separate recovery workspace will be used."
                            )
                        }
                    };
                    (Document::default(), format!("{e} {kept}"), false)
                }
            }
        } else {
            (
                Document::default(),
                "Start a new project or open an existing one.".into(),
                false,
            )
        };
        // The project's file from the last run, while the workspace resumed is
        // still there to be saved to it; the job has unsaved changes when the
        // two files differ.
        let workspace = std::fs::read(&state_path)
            .ok()
            .filter(|_| smoke_dir.is_none());
        let project_path = settings
            .project_file
            .clone()
            .filter(|p| workspace.is_some() && p.is_file());
        let project_dirty = project_path
            .as_ref()
            .is_some_and(|p| std::fs::read(p).ok() != workspace);
        if shared && !isolated {
            status += " No home directory was found, so the workspace is in the shared temp directory and signing in is unavailable.";
        }
        // A blank workspace has the starter materials and workholding too.
        doc.adopt_materials();
        doc.adopt_holders();
        // The owner's threading tools; never in tests.
        if !isolated {
            doc.adopt_tools();
        }
        // The owner's CNC for a library with no machine; never in tests.
        if !isolated && doc.machines.is_empty() {
            match control::owners_machine(state_path.parent().unwrap_or(std::path::Path::new(".")))
            {
                Ok(m) => {
                    status += " The Jingyan CNC4030 is in the Machine library.";
                    doc.machines.push(m);
                    migrated = true;
                }
                Err(e) => status += &format!(" Could not write the CNC4030's UC100 profile: {e}."),
            }
        }
        if !doc.sample_model {
            doc.sync_stock_sources();
        }
        doc.sync_turns(None);
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
            include_bytes!("../assets/op-11.svg"),
            include_bytes!("../assets/op-12.svg"),
        ]
        .iter()
        .map(|b| iced::widget::svg::Handle::from_memory(b.to_vec()))
        .collect();
        let generate_inline = smoke_dir.is_some() || cfg!(test);
        Self {
            smoke_dir,
            smoke_step: 0,
            help_animating: false,
            control: None,
            machine_setup: false,
            profile_draft: None,
            control_machine: None,
            switch_machine: None,
            cnc_over_guide: None,
            run_on_connect: None,
            jog_key: None,
            run_epoch: 0,
            smoke_pending: false,
            smoke_frames: 0,
            smoke_changed: Instant::now(),
            smoke_resize: false,
            smoke_window: None,
            smoke_last: None,
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
            cam_page: Page::Home,
            selected: None,
            setup_end: false,
            tab: "Tool".into(),
            setup_step: "Machine",
            status,
            audit_report: vec![],
            pinned_help: None,
            pinned_expanded: true,
            pinned_paused: false,
            post: None,
            post_busy: false,
            left_stock: LeftStock::default(),
            holding_checks: HoldingChecks::default(),
            left_job: None,
            guide: None,
            guide_job: None,
            guide_item: 0,
            guide_typed: None,
            guide_clock: (Instant::now(), Instant::now()),
            ready: run::Ready::default(),
            ready_job: None,
            holding_note: std::cell::RefCell::new(None),
            drill_holes: std::cell::RefCell::new(None),
            bottom_holes: std::cell::RefCell::new(None),
            pocket_holes: std::cell::RefCell::new(None),
            part_holes: None,
            part_holes_job: None,
            pocket_fits: std::cell::RefCell::new(None),
            scrub_pending: false,
            scrub_live: SCRUB_LIVE,
            catching_up: None,
            catch_up_epoch: 0,
            last_scrub: Instant::now(),
            generating: None,
            generation_epoch: 0,
            generate_inline,
            tooth_load_job: None,
            stock_pictures: Default::default(),
            post_epoch: 0,
            picture_job: None,
            settings,
            settings_path,
            account: Account::SignedOut,
            account_note: settings_note.unwrap_or_default(),
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
            sign_in_attempt: 0,
            open_browser: std::sync::Arc::new(oauth::open_browser),
            account_restored: false,
            camera: viewport::Camera::default(),
            window_width: WINDOW.width,
            sample_mesh: mesh::sample(),
            imported_mesh,
            import_busy: false,
            save_busy: false,
            last_archived: None,
            import_error: None,
            stock: true,
            show_uncut: true,
            section: None,
            section_on: false,
            picking_section: false,
            paths: true,
            picking: false,
            picking_pins: false,
            placing_pins: false,
            pin_aim: None,
            playing: false,
            simulation: None,
            stock_detail: Default::default(),
            stock_mesh_job: None,
            stock_mesh_started: None,
            stock_mesh_slow: false,
            stock_mesh_failed: None,
            start_mesh: vec![],
            start_mesh_job: None,
            start_mesh_failed: None,
            progress: 0.,
            speed: 1.,
            last_tick: Instant::now(),
            dirty: migrated,
            save_error: None,
            last_edit: Instant::now(),
            state_path,
            project_path,
            project_dirty,
            wizard_step: 0,
            planner: Planner::Idle,
            plan_run: None,
            plan_epoch: 0,
            plan_account_epoch: 0,
            plan_model: String::new(),
            plan_timeline: vec![],
            plan_stream: Default::default(),
            plan_overview: None,
            plan_enlarged: None,
            plan_expanded: Default::default(),
            plan_camera: viewport::Camera::default(),
            plan_request: String::new(),
            plan_chat: vec![],
            plan_asked: None,
            plan_refined: None,
            plan_busy: planner::Busy::default(),
            plan_dropped: None,
            undo: None,
            confirm_delete_setup: false,
            confirm_delete_op: false,
            status_page: Page::Home,
            pending: None,
            icons,
            closing: None,
            signing_out: None,
            gcode_exports: 0,
            #[cfg(test)]
            test_dir: None,
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
            app.smoke_arm();
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
    /// The project's file, remembered for the next start. The workspace is
    /// written first: a start after a crash must not pair the file with the
    /// job that was in the workspace before, and save that job over it.
    fn set_project_file(&mut self, path: Option<PathBuf>) {
        self.project_path = path;
        if self.settings.project_file != self.project_path {
            self.dirty = true;
            self.persist();
            self.settings.project_file = self.project_path.clone();
            self.save_settings();
        }
    }
    fn replace_document(&mut self, doc: Document, path: Option<PathBuf>) {
        // What is being generated is the old project's.
        self.cancel_generation();
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
        // What was worked out for the old project is not even a last result
        // of this one (`stale_ready`, `ui::stock_picture`).
        self.ready = run::Ready::default();
        self.ready_job = None;
        self.left_stock = LeftStock::default();
        self.holding_checks = HoldingChecks::default();
        self.imported_mesh = None;
        self.sync_turns();
        self.import_error = None;
        self.project_dirty = path.is_none();
        self.set_project_file(path);
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
        // The chat was about the project that is gone.
        self.plan_chat.clear();
        self.plan_request.clear();
    }
    /// Says which later setups turned with setup 1 to stay square to the
    /// stock (`Document::rotate_first`).
    fn say_turned_with_first(&mut self, turned: &[String]) {
        if !turned.is_empty() {
            self.status = format!(
                "{} turned with {} to stay square to the stock.",
                crate::post::listed(turned),
                self.doc.setups[0].name()
            );
        }
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
        // A later setup starts with the stock as setup 1 has it, top up and
        // square to it, whatever setup 1's rotation.
        if let Some(first) = self.doc.setups.first() {
            let (_, _, top) = crate::volume::turns().next().expect("the stock top up");
            for (k, v) in ["rx", "ry", "rz"]
                .iter()
                .zip(crate::volume::turned(first, top))
            {
                setup.p.insert((*k).into(), v.to_string());
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
        let path = file.path.clone();
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
    /// The section the 3D view's part is cut on, when it is on: the one set
    /// on this part, or across Z when it was set on another.
    pub fn view_section(&self) -> Option<section::Section> {
        if !self.section_on {
            return None;
        }
        let mesh = self.part_mesh()?;
        Some(match &self.section {
            Some((cut, part)) if *part == mesh.fingerprint => *cut,
            // Set on another part, as after opening the example: across Z
            // through the middle of this one, its top half cut away.
            _ => section::Section::across(2, mesh, self.doc.active_setup(), [0., 0., 1.]),
        })
    }
    /// The 3D view's section changed from its controls.
    fn edit_section(&mut self, edit: section::Edit) {
        let Some(mesh) = self.part_mesh() else {
            return;
        };
        let part = mesh.fingerprint.clone();
        let (setup, toward) = (self.doc.active_setup(), self.camera.toward());
        let across = |k| section::Section::across(k, mesh, setup, toward);
        // From the section there is, unless it was set on another part.
        let mut cut = match (edit, &self.section) {
            (section::Edit::Axis(k), _) => across(k),
            (_, Some((cut, on))) if *on == part => *cut,
            // The cut shown on this part (across Z) is the one edited.
            (section::Edit::Toggle, _) => across(2),
            _ => self.view_section().unwrap_or_else(|| across(2)),
        };
        match edit {
            section::Edit::Toggle => {
                self.section_on = !self.section_on;
                self.picking_section = false;
            }
            section::Edit::Axis(_) => {
                self.section_on = true;
                self.picking_section = false;
            }
            section::Edit::PickFace => {
                self.picking_section = !self.picking_section;
                self.section_on = true;
            }
            section::Edit::At(at) => cut.at = at,
            section::Edit::Flip => cut.positive = !cut.positive,
        }
        self.section = Some((cut, part));
    }
    /// Whether a click on the part picks a hole for the fixture pins: "Pick
    /// holes on the part" is on, and the Work zero step of a setup on the
    /// pins is open on an imported part.
    pub fn picking_pin_holes(&self) -> bool {
        self.picking_pins
            && self.page == Page::Work
            && self.selected.is_none()
            && self.setup_step == "Work zero"
            && !self.is_sample()
            && self.part_mesh().is_some()
            && (self.doc.active_setup())
                .is_some_and(|s| post::sits_on_pins(s) && !post::pins_in_leftover(s))
    }
    /// Whether the pointer places the pins in the stock's leftover: "Place
    /// the pins on the leftover" is on, and the Work zero step of a setup on
    /// pins in its leftover is open on an imported part.
    pub fn placing_leftover_pins(&self) -> bool {
        self.placing_pins
            && self.page == Page::Work
            && self.selected.is_none()
            && self.setup_step == "Work zero"
            && !self.is_sample()
            && self.part_mesh().is_some()
            && (self.doc.active_setup())
                .is_some_and(|s| post::sits_on_pins(s) && post::pins_in_leftover(s))
    }
    pub fn can_pick(&self) -> bool {
        self.operation().is_some_and(|o| {
            // A lathe drills on its centre and turns by numbers: nothing to pick.
            !o.on_centre(&self.doc.setup().machine)
                && o.strategy != Strategy::Turn
                && (self.is_sample()
                    || (self.doc.file.is_some()
                        && (o.needs_surfaces() || o.strategy == Strategy::Box)
                        && o.strategy.imported_preview()
                        && self.imported_mesh.is_some()))
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
    /// never changed. Without toolpaths the stock alone decides (that of a
    /// later setup is the box the earlier setups left when that is known,
    /// `known_stock`); without a stock nothing is written. Whether the setup changed (and was marked
    /// so): nothing is written or marked when the suggestion is what it has.
    fn suggest_layout(&mut self) -> Task<Message> {
        let si = self.doc.index();
        let setup = self.doc.setup();
        if get(&setup.p, "holding") == "Chuck" {
            // A chuck is chosen for the stock alone: no toolpaths are needed.
            self.suggest_chuck(si);
            return Task::none();
        }
        if let Err(error) = holding::request(setup, self.doc.holder(get(&setup.p, "holder"))) {
            self.status = error;
            return Task::none();
        }
        let last = setup.operations.iter().rposition(|o| o.enabled);
        let (Some(mesh), Some(last)) = (
            self.imported_mesh.clone().filter(|_| !self.is_sample()),
            last,
        ) else {
            self.place_layout(si, &[], None);
            return Task::none();
        };
        // Generated off the window's thread, as Generate is.
        let doc = self.doc.clone();
        self.status = "Generating the setup's toolpaths to place its holding…".into();
        self.generate(
            Generation::Layout(si),
            move || {
                let mut generated: Vec<Option<simulation::Plan>> = vec![];
                // What tells the bands a vise closes on: the setup's first
                // simulation starts from the stock the earlier setups left.
                let mut bands = None;
                let setup = &doc.setups[si];
                simulation::sequence(&doc, &mesh, Some((si, last)), |s, _, result| {
                    if s == si {
                        if generated.is_empty()
                            && let Ok(first) = &result
                        {
                            bands = post::bands(setup, &mesh, first);
                        }
                        generated.push(result.ok().map(|sim| sim.plan));
                    }
                });
                Laid(std::sync::Arc::new((generated, bands)))
            },
            Message::GeneratedLayout,
        )
    }
    /// The rest of `suggest_layout`, once setup `si`'s toolpaths are
    /// generated (`generated`, None for one that could not be).
    fn place_layout(
        &mut self,
        si: usize,
        generated: &[Option<simulation::Plan>],
        bands: Option<&holding::Bands>,
    ) -> bool {
        let Some(setup) = self.doc.setups.get(si) else {
            return false;
        };
        let request = match holding::request(setup, self.doc.holder(get(&setup.p, "holder"))) {
            Ok(request) => request,
            Err(error) => {
                self.status = error;
                return false;
            }
        };
        let (refs, failed) = holding::gathered(generated.iter().map(Option::as_ref));
        // Without toolpaths, the stock the earlier setups left when that is known.
        let known;
        let bands = if refs.is_empty() {
            known = self.known_bands(si);
            known.as_ref()
        } else {
            bands
        };
        let Some((stock, layout)) = request.suggest_on(&refs, || self.known_stock(si).0, bands)
        else {
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
        // Clear by the gap and the blend, as export checks and the suggestion kept.
        let holder = &request.holder;
        let held = holding::gripped(&layout, holder, stock, bands);
        let boxes = holding::boxes(&layout, holder, held).unwrap_or_default();
        let clashing = holding::clashes(&refs, &boxes, request.reach);
        let opening = holding::opening_issue(&layout, holder, held);
        let message = holding::summary(&layout, refs.len(), clashing, failed, opening.as_deref());
        if changed {
            self.touch();
        }
        self.status = message;
        changed
    }
    /// Chooses the chuck of lathe setup `si` from the library
    /// (`holding::suggest_chuck`): the 3-jaw when the bar in its jaws is
    /// round and within its range, else the 4-jaw, which the status says to
    /// change the chuck for. The grip and the stickout are the user's. Never a
    /// vise. Whether the setup changed (and was marked so).
    fn suggest_chuck(&mut self, si: usize) -> bool {
        let setup = &self.doc.setups[si];
        let Some(held) = self.known_held(si) else {
            self.status = "No chuck suggested: this setup's stock is not known yet. Check the stock settings.".into();
            return false;
        };
        let (id, status) = match holding::chuck_for(&self.doc, setup, held) {
            Ok(found) => found,
            Err(error) => {
                self.status = error;
                return false;
            }
        };
        let changed = get(&setup.p, "holder") != id;
        self.doc.setups[si].p.insert("holder".into(), id);
        if changed {
            self.touch();
        }
        self.status = status;
        changed
    }
    /// Keeps pin 1's machine X Y just recorded on the Control page in the
    /// profile of machine `id`, and in its draft if it is being edited. It
    /// is a measurement, not a setting: no setup's check depends on it.
    fn record_pins(&mut self, id: &str, at: [f64; 2]) {
        let keep = |m: &mut Machine| {
            if m.id == id {
                m.p.insert("pin_ref_x".into(), format!("{:.3}", at[0]));
                m.p.insert("pin_ref_y".into(), format!("{:.3}", at[1]));
            }
        };
        self.doc.machines.iter_mut().for_each(keep);
        keep(&mut self.machine_draft);
        self.dirty = true;
        self.project_dirty = true;
        self.last_edit = Instant::now();
    }
    /// Runs `work` off the window's thread with a progress the window shows
    /// (`progress`), and sends what it makes back as `done` with this
    /// generation's epoch; a generation under way is cancelled first. Inline
    /// in the smoke run and the tests (`generate_inline`).
    fn generate<T: Send + 'static>(
        &mut self,
        what: Generation,
        work: impl FnOnce() -> T + Send + 'static,
        done: fn(u64, T) -> Message,
    ) -> Task<Message> {
        if let Some(g) = self.generating.take() {
            g.progress.cancel();
        }
        if let Some((_, job)) = self.tooth_load_job.take() {
            job.cancel();
        }
        self.generation_epoch += 1;
        let epoch = self.generation_epoch;
        let progress = std::sync::Arc::new(progress::Progress::default());
        self.generating = Some(Generating {
            epoch,
            what,
            progress: progress.clone(),
            play: false,
        });
        if self.generate_inline {
            let made = progress.run(work);
            return self.update(done(epoch, made));
        }
        Task::perform(
            async move { tokio::task::spawn_blocking(move || progress.run(work)).await },
            move |made| match made {
                Ok(made) => done(epoch, made),
                Err(e) => Message::GenerationFailed(epoch, format!("Generation failed: {e}")),
            },
        )
    }
    /// Replays the simulation to where the slider stands after a scrub too
    /// long to replay while it moved (`scrub_pending`): off the window's
    /// thread, on a copy (a few milliseconds even at a million moves), put
    /// back when done if nothing moved meanwhile (`Message::Replayed`).
    /// Playback starts then with `play`. Inline in the smoke run and tests.
    fn catch_up(&mut self, play: bool) -> Task<Message> {
        if !std::mem::take(&mut self.scrub_pending) {
            return Task::none();
        }
        let Some(simulation) = &mut self.simulation else {
            return Task::none();
        };
        let at = self.progress;
        if self.generate_inline {
            simulation.seek(at);
            if play {
                self.playing = true;
                self.last_tick = Instant::now();
            }
            return Task::none();
        }
        self.catch_up_epoch += 1;
        let epoch = self.catch_up_epoch;
        self.catching_up = Some((epoch, at, play));
        let mut copy = simulation.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    copy.seek(at);
                    copy
                })
                .await
            },
            move |replayed| match replayed {
                Ok(simulation) => {
                    Message::Replayed(epoch, Replayed(std::sync::Arc::new(simulation)))
                }
                Err(_) => Message::ReplayFailed(epoch),
            },
        )
    }
    /// Works out the load on each tooth along the simulation's path
    /// (`tooth_load::of`) off the window's thread, on a copy, after its
    /// preview is shown: the path takes its colours when it is done
    /// (`Message::ToothLoaded`). One for an earlier simulation is cancelled.
    /// Inline in the smoke run and tests.
    fn measure_tooth_load(&mut self) -> Task<Message> {
        if let Some((_, job)) = self.tooth_load_job.take() {
            job.cancel();
        }
        let Some(simulation) = &mut self.simulation else {
            return Task::none();
        };
        if self.generate_inline {
            simulation.tooth_load = tooth_load::of(simulation, &self.doc).map(std::sync::Arc::new);
            return Task::none();
        }
        let progress = std::sync::Arc::new(progress::Progress::default());
        let id = simulation.id.clone();
        self.tooth_load_job = Some((id.clone(), progress.clone()));
        let (copy, doc) = (simulation.clone(), self.doc.clone());
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    progress.run(|| tooth_load::of(&copy, &doc).map(std::sync::Arc::new))
                })
                .await
            },
            move |load| Message::ToothLoaded(id.clone(), load.ok().flatten()),
        )
    }
    /// Stops the generation under way, if any: what it makes is dropped.
    fn cancel_generation(&mut self) {
        if let Some(g) = self.generating.take() {
            g.progress.cancel();
            if self.status.starts_with("Generating") {
                self.status.clear();
            }
        }
    }
    /// The generation under way that `epoch` started, taken off: None when
    /// it is stale (another started since, or it was cancelled).
    fn finished(&mut self, epoch: u64) -> Option<Generating> {
        self.generating.take_if(|g| g.epoch == epoch)
    }
    /// Keeps what the Control page found for the rotary unit of machine
    /// `id` (LIO-186): its centreline probed, marked measured, or the mark
    /// gone stale; in its draft too if it is being edited. Like the pins'
    /// place it is a measurement: no setup's check is made out of date.
    fn keep_rotary(&mut self, id: &str, keep: control::RotaryKeep) {
        let keep = |m: &mut Machine| {
            if m.id != id {
                return;
            }
            match keep {
                control::RotaryKeep::Centreline([y, z], [rx, ry, rz]) => {
                    m.p.insert("a_centre_y".into(), format!("{y:.3}"));
                    m.p.insert("a_centre_z".into(), format!("{z:.3}"));
                    m.p.insert(model::CENTRE_REF.into(), format!("{rx:.3} {ry:.3} {rz:.3}"));
                    m.p.insert(control::CENTRE_MARK.into(), control::MEASURED.into());
                }
                control::RotaryKeep::Stale(why)
                    if get(&m.p, control::CENTRE_MARK) == control::MEASURED =>
                {
                    m.p.insert(control::CENTRE_MARK.into(), format!("stale after {why}"));
                }
                control::RotaryKeep::Stale(_) => {}
            }
        };
        self.doc.machines.iter_mut().for_each(keep);
        keep(&mut self.machine_draft);
        self.dirty = true;
        self.project_dirty = true;
        self.last_edit = Instant::now();
    }
    /// Works out how A turns each indexed setup (`Document::sync_turns`),
    /// with the part when it is there.
    fn sync_turns(&mut self) {
        let mesh = if self.doc.file.is_some() {
            self.imported_mesh.as_ref()
        } else if self.doc.sample_model {
            Some(&self.sample_mesh)
        } else {
            None
        };
        self.doc.sync_turns(mesh);
    }
    fn touch(&mut self) {
        // A generation under way makes the job as it was.
        let placing = self
            .generating
            .as_ref()
            .is_some_and(|g| matches!(g.what, Generation::Layout(_)));
        self.cancel_generation();
        if placing {
            self.status = "The vise or clamps were not placed: the setup changed while its toolpaths were generated. Press Suggest layout to place them.".into();
        }
        if !self.is_sample() {
            self.doc.sync_stock_sources();
            if self.doc.file.is_some()
                && let Some(mesh) = &self.imported_mesh
            {
                post::sync_pin_drilling(&mut self.doc, mesh);
            }
        }
        self.sync_turns();
        self.audit_report.clear();
        self.invalidate_post();
        // A running plan and a preview describe the job as it was.
        match self.planner {
            Planner::Running(_) => self.plan_dropped = Some(PLAN_STOPPED),
            Planner::Preview(_) => self.plan_dropped = Some(PREVIEW_DISCARDED),
            _ => {}
        }
        self.drop_plan();
        // A setup being run was checked as it was, and may have been moved.
        if let Some(run) = self.control.as_mut().and_then(|c| c.run.as_mut()) {
            run.stale = true;
            run.follow(&self.doc);
        }
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
    /// iced's runtime waits for it on exit. A model import is one (a STEP
    /// conversion may take two minutes).
    fn blocking_work(&self) -> bool {
        self.import_busy
            || self.plan_busy.running() > 0
            || self.post_busy
            || self.left_job.is_some()
            || self.ready_job.is_some()
            || self.stock_mesh_job.is_some()
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
    /// The machine the Control page connects to: the chosen UC100 machine, or the first one.
    pub fn control_target(&self) -> Option<&Machine> {
        let uc100 = |m: &&Machine| get(&m.p, "controller") == "UC100";
        self.doc
            .machines
            .iter()
            .filter(uc100)
            .find(|m| self.control_machine.as_ref() == Some(&m.id))
            .or_else(|| self.doc.machines.iter().find(uc100))
    }
    /// The machine's tool for the 3D view, while a tool run of the active
    /// setup cuts: tip in plan coordinates, radius, stickout.
    pub fn live_tool(&self) -> Option<([f32; 3], f32, f32)> {
        let c = self.control.as_ref()?;
        let live = c.run.as_ref()?.live(&c.snapshot)?;
        let setup = self.doc.setups.get(live.setup)?;
        (setup.id == self.doc.active).then_some((live.tip, live.radius, live.stickout))
    }
    /// The machine's A (degrees, in work coordinates, as programs command
    /// it) while a tool run of the active setup cuts, on a machine with A.
    pub fn live_a(&self) -> Option<f32> {
        let c = self.control.as_ref()?;
        let live = c.run.as_ref()?.live(&c.snapshot)?;
        let setup = self.doc.setups.get(live.setup)?;
        (setup.id == self.doc.active && c.axis(3)).then_some(c.snapshot.work_pos[3] as f32)
    }
    /// The A the Job view draws the active indexed setup's part, stock and
    /// chuck at: the machine's while a run of it cuts (`live_a`), else the
    /// simulation's (`Simulation::a_now`). None for a setup not indexed.
    pub fn shown_a(&self) -> Option<f32> {
        self.doc.active_setup()?.turn?;
        self.live_a().or_else(|| self.active_simulation()?.a_now())
    }
    /// The stock the run of the active setup has carved (`run::Run::stock`),
    /// for the Control page's 3D view: MANUFACTURE keeps the part and the
    /// planned stock.
    pub fn live_stock(&self) -> Option<(&simulation::StockGrid, u64)> {
        if self.page != Page::Control {
            return None;
        }
        let r = self.control.as_ref()?.run.as_ref()?;
        let setup = self.doc.setups.get(r.setup)?;
        if setup.id != self.doc.active {
            return None;
        }
        r.stock()
    }
    /// The stock a run of the active setup that turns A has carved, as a
    /// volume (`run::Run::carved_volume`), for the Control page's 3D view.
    pub fn live_volume(&self) -> Option<(&volume::Volume, u64)> {
        if self.page != Page::Control {
            return None;
        }
        let r = self.control.as_ref()?.run.as_ref()?;
        let setup = self.doc.setups.get(r.setup)?;
        if setup.id != self.doc.active {
            return None;
        }
        r.carved_volume()
    }
    /// The machine moves, or a tool run of the Run panel or a probe is under way:
    /// planning is locked (`while_locked`) and no preview or check is worked
    /// out in the background, the Run panel's check of its setup included
    /// (`check_waits`, #167), so that none competes with the driver for the
    /// processor while it feeds the machine its motion.
    pub fn locked(&self) -> bool {
        use uc100::controller::State;
        self.control.as_ref().is_some_and(|c| {
            c.run.as_ref().is_some_and(|r| r.running.is_some())
                || c.probing()
                || matches!(
                    c.snapshot.state,
                    Some(
                        State::Jog
                            | State::Homing
                            | State::Run
                            | State::Hold
                            | State::Stopping
                            | State::Paused(_)
                    )
                )
        })
    }
    /// Planning work still under way in the background, which Cycle Start
    /// waits for (see `locked`).
    pub fn planning_busy(&self) -> Option<&'static str> {
        if matches!(self.planner, Planner::Running(_)) || self.plan_busy.running() > 0 {
            Some("The AI planner is working: wait for it, or cancel it.")
        } else if self.picturing() {
            Some("The stock after each operation is still being pictured: wait a moment.")
        } else if self.stock_mesh_job.is_some()
            || self.post_busy
            || self.left_job.is_some()
            || self.ready_job.is_some()
        {
            Some("A preview or check is still being worked out: wait a moment.")
        } else {
            None
        }
    }
    /// Opens the Control page machine's profile for Setup: again when asked,
    /// for another machine, or when there are no changes to keep.
    fn open_profile(&mut self, again: bool) {
        let Some(m) = self.control_target().cloned() else {
            self.profile_draft = None;
            return;
        };
        let keep = match &self.profile_draft {
            Some((id, Ok(d))) => *id == m.id && d.dirty(),
            _ => false,
        };
        if again || !keep {
            self.profile_draft = Some((m.id.clone(), setup::Draft::open(&m)));
        }
    }
    /// Saves the profile being edited. A connected machine is reconnected
    /// with it: the driver keeps machine positions across `ApplyConfig`,
    /// which a new steps per mm would make wrong.
    fn save_profile(&mut self) -> Task<Message> {
        let Some((id, Ok(d))) = &mut self.profile_draft else {
            return Task::none();
        };
        let here = self.control.as_ref().filter(|c| c.machine_id == *id);
        if here.is_some_and(|c| !c.stopped()) {
            d.note = Some("Stop the machine before saving the profile.".into());
            return Task::none();
        }
        let reconnect = here.is_some();
        match d.save() {
            Err(e) => d.note = Some(e),
            Ok(_) if reconnect => {
                d.note = Some(
                    "Saved, and the machine was reconnected with it: Reset, then set the machine position again.".into(),
                );
                self.disconnect_machine();
                return self.update(Message::Connect);
            }
            Ok(_) => d.note = Some("Saved.".into()),
        }
        Task::none()
    }
    /// Disables the machine and stops its controller, if connected.
    pub fn disconnect_machine(&mut self) {
        self.switch_machine = None;
        if let Some(c) = self.control.take() {
            c.close();
            self.status = "Disconnected from the machine.".into();
        }
    }
    /// Saves the workspace, then carries out `close_decision`: closes the
    /// window, waits, or ends the process.
    fn request_close(&mut self, id: iced::window::Id) -> Task<Message> {
        self.disconnect_machine();
        self.persist();
        match self.close_decision(id, Instant::now()) {
            CloseAction::Stay => {
                self.status = "Could not save before closing. Use Save as… to save the project to another location, then close again.".into();
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
            // nothing is lost. A STEP converter of an import in progress
            // would outlive the process, so it is ended first.
            CloseAction::Exit => {
                step::kill_running();
                std::process::exit(0)
            }
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
                self.save_error = None;
            }
            Err(e) => {
                // The retry after each pause fails the same way; say it once.
                if self.save_error.as_deref() != Some(e.as_str()) {
                    self.status = format!("Could not save the workspace: {e}");
                    self.save_error = Some(e);
                }
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
                    let mut issues =
                        self.doc
                            .setup()
                            .operation_issues_in(i, &self.doc.tools, self.is_sample());
                    if self.doc.file.is_some() {
                        issues.extend(indexed::surface_issues(
                            &self.doc,
                            self.doc.index(),
                            o,
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
            for (si, setup) in self.doc.setups.iter().enumerate() {
                for operation in setup.operations.iter().filter(|o| o.enabled) {
                    issues.extend(indexed::surface_issues(
                        &self.doc,
                        si,
                        operation,
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
    /// How long the stock mesher has been running, while it is.
    pub fn stock_mesh_elapsed(&self) -> Option<Duration> {
        self.stock_mesh_job.as_ref()?;
        self.stock_mesh_started.map(|t| t.elapsed())
    }
    /// Whether the last mesher job took long enough to be named.
    pub fn stock_mesh_slow(&self) -> bool {
        self.stock_mesh_slow
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
        if self.stock_mesh_job.is_some() || self.locked() {
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
        let snapshot = stock_surface::Snapshot::of(volume, self.stock_detail);
        let seconds = sim.seconds;
        self.stock_mesh_job = Some(key.clone());
        self.stock_mesh_started = Some(Instant::now());
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
    /// What the stock the selected operation starts from is, or with "Stock
    /// after this setup" selected (`setup_end`) the stock the setup leaves,
    /// when the job was worked out (`left_stock`) with the setups and
    /// operations before it as they are now: its key, the job's stock and
    /// the setup and operation whose cuts and later ones are not made yet
    /// (for the setup's end, one past its last). None for the example, on
    /// another page than MANUFACTURE's, with neither selected, while that is
    /// not worked out, after an operation that failed and when nothing is
    /// cut before it, where the setup's own stock is all there is.
    pub(crate) fn start_source(
        &self,
    ) -> Option<(StartKey, std::sync::Arc<volume::Volume>, (u16, u16))> {
        if self.page != Page::Work || self.is_sample() {
            return None;
        }
        let si = self.doc.index();
        let oi = match self.selected {
            Some(oi) => oi,
            None if self.setup_end => self.doc.active_setup()?.operations.len(),
            None => return None,
        };
        let left = &self.left_stock;
        let key = op_fingerprint(&self.doc, self.part_mesh()?, si, oi)?;
        if left.ops.get(si)?.get(oi) != Some(&key) || left.broken.is_some_and(|b| b < (si, oi)) {
            return None;
        }
        let job = left.job.clone()?;
        let before = (u16::try_from(si).ok()?, u16::try_from(oi).ok()?);
        job.cuts.iter().any(|c| c.source < before).then_some(())?;
        Some(((key, si, oi, self.stock_detail), job, before))
    }
    /// The stock the selected operation starts from for the 3D view, once
    /// meshed (`refresh_start_mesh`): the job's stock and the surface.
    pub fn start_stock(&self) -> Option<(&volume::Volume, &stock_surface::SurfaceMesh)> {
        let (key, ..) = self.start_source()?;
        let (_, volume, surface) = self.start_mesh.iter().find(|(k, ..)| *k == key)?;
        Some((volume.as_ref(), surface.as_ref()))
    }
    /// Meshes the stock the selected operation starts from (`start_source`)
    /// off the window's thread, at the simulation's detail, when it is not
    /// yet (in line in the smoke run): the job's stock with only the cuts of
    /// the operations before it.
    pub(crate) fn refresh_start_mesh(&mut self) -> Task<Message> {
        let Some((key, volume, before)) = self.start_source() else {
            return Task::none();
        };
        if self.start_mesh.iter().any(|(k, ..)| *k == key)
            || self.start_mesh_job.is_some()
            || self.start_mesh_failed == Some(key)
            || self.locked()
        {
            return Task::none();
        }
        let mut snapshot = stock_surface::Snapshot::of(&volume, key.3);
        let cuts = volume.cuts.iter().filter(|c| c.source < before);
        snapshot.cuts = std::sync::Arc::new(cuts.cloned().collect());
        let build = move || snapshot.build().map(std::sync::Arc::new);
        if self.smoke_dir.is_some() {
            return self.update(Message::StartMeshed {
                key,
                volume,
                result: build(),
            });
        }
        self.start_mesh_job = Some(key);
        Task::perform(
            async move {
                let result = tokio::task::spawn_blocking(build)
                    .await
                    .unwrap_or_else(|e| Err(format!("Stock mesher failed: {e}")));
                (key, volume, result)
            },
            |(key, volume, result)| Message::StartMeshed {
                key,
                volume,
                result,
            },
        )
    }
    /// Checks the setup being run, in the background: simulates the job up to
    /// it and checks its tool runs as export would.
    fn check_run(&mut self) -> Task<Message> {
        let Some(c) = self.control.as_mut() else {
            return Task::none();
        };
        let config = c.config.clone();
        let Some(r) = c.run.as_mut() else {
            return Task::none();
        };
        // The setup is checked where it is now: setups may have been moved,
        // or it removed, since it was picked.
        let Some(si) = r.follow(&self.doc) else {
            return Task::none();
        };
        let epoch = r.epoch;
        let doc = self.doc.clone();
        let mesh = self.part_mesh().cloned();
        let check = move || {
            let (programs, left) = match mesh {
                Some(mesh) => run::check(&doc, &mesh, si),
                None => (
                    Err(vec![
                        "Load the imported part before running a setup.".into(),
                    ]),
                    run::Left::default(),
                ),
            };
            let times = run::Times::checked(&programs, &config);
            (programs, left, times)
        };
        Task::perform(
            async move {
                tokio::task::spawn_blocking(check)
                    .await
                    .unwrap_or_else(|e| {
                        (
                            Err(vec![format!("The check failed: {e}")]),
                            run::Left::default(),
                            None,
                        )
                    })
            },
            move |(programs, left, times)| {
                Message::RunCheckedStock(
                    epoch,
                    std::sync::Arc::new(programs),
                    std::sync::Arc::new(left),
                    times,
                )
            },
        )
    }
    /// Pictures the stock the last check of the setup being run left after
    /// each of its operations (`run::Run::left`), for the Tool runs panel: in
    /// the background (in line in the smoke run), one run at a time, and
    /// only while the machine stands still, as the check is (#167): the Tick
    /// starts it once the machine stops. Each stock is meshed at Draft detail
    /// and seen in its jaws or clamps as the setup browser's pictures are.
    fn picture_run(&mut self) -> Task<Message> {
        if self.picture_job.is_some() || self.locked() {
            return Task::none();
        }
        let Some(r) = self.control.as_ref().and_then(|c| c.run.as_ref()) else {
            return Task::none();
        };
        let (si, epoch) = (r.setup, r.epoch);
        let (Some(_), Some(setup), Some(mesh)) =
            (&r.left, self.doc.setups.get(si), self.part_mesh())
        else {
            return Task::none();
        };
        let frame = volume::Frame::new(setup, mesh);
        let jaws: Vec<mesh::Bounds> = self
            .known_stock(si)
            .0
            .map(|stock| ui::pictured_holding(self, si, stock))
            .unwrap_or_default()
            .into_iter()
            .map(|b| b.bounds)
            .collect();
        let Some(left) = self
            .control
            .as_mut()
            .and_then(|c| c.run.as_mut()?.left.take())
        else {
            return Task::none();
        };
        self.picture_job = Some(epoch);
        let work = move || run::op_pictures(&left, frame, &jaws);
        if self.smoke_dir.is_some() {
            return self.update(Message::RunPictures(epoch, work()));
        }
        Task::perform(
            async move { tokio::task::spawn_blocking(work).await.unwrap_or_default() },
            move |pictures| Message::RunPictures(epoch, pictures),
        )
    }
    /// The stock of the setup being run is being pictured, or waits to be
    /// (`picture_run`): Cycle Start waits for it.
    fn picturing(&self) -> bool {
        self.picture_job.is_some()
            || self
                .control
                .as_ref()
                .and_then(|c| c.run.as_ref())
                .is_some_and(|r| r.left.is_some())
    }
    /// While the machine moves (`locked`), the setup being run is only marked
    /// to be checked: the Tick checks it once the machine stops, so that the
    /// simulation of the job never competes with the driver feeding it
    /// (#167). Whether the check waits.
    fn check_waits(&mut self) -> bool {
        if !self.locked() {
            return false;
        }
        if let Some(r) = self.control.as_mut().and_then(|c| c.run.as_mut()) {
            r.stale = true;
        }
        true
    }
    /// Checks the setup being run again, keeping its progress.
    fn recheck_run(&mut self) -> Task<Message> {
        if self.check_waits() {
            return Task::none();
        }
        self.run_epoch += 1;
        if let Some(r) = self.control.as_mut().and_then(|c| c.run.as_mut()) {
            (r.epoch, r.programs, r.stale) = (self.run_epoch, None, false);
        }
        self.check_run()
    }
    fn invalidate_post(&mut self) {
        self.post = None;
        self.post_epoch = self.post_epoch.wrapping_add(1);
    }
    /// Check and emit the whole job while the NC review page is open.
    fn refresh_post(&mut self) -> Task<Message> {
        if self.page != Page::Post || self.post.is_some() || self.post_busy || self.locked() {
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
    /// The holes the Drill operation `op` finds on `mesh` in the current
    /// setup. `cam::holes` passes over every triangle per opening, and the
    /// editor asks on every redraw, so the answer is kept until the part, the
    /// setup's placement or the selected surfaces change.
    pub(crate) fn drill_holes(
        &self,
        op: &Operation,
        mesh: &mesh::Mesh,
    ) -> std::sync::Arc<DrillHoles> {
        use std::hash::{Hash, Hasher};
        let setup = self.doc.setup();
        let mut hasher = std::hash::DefaultHasher::new();
        mesh.fingerprint.hash(&mut hasher);
        self.hash_placement(&mut hasher);
        op.surfaces
            .as_ref()
            .map(|s| (&s.mesh_fingerprint, &s.ids))
            .hash(&mut hasher);
        // The cycle says which holes it lists: the bores a flat end mill
        // fits, the holes to thread.
        get(&op.p, "cycle").hash(&mut hasher);
        op.helical().hash(&mut hasher);
        let key = hasher.finish();
        let mut kept = self.drill_holes.borrow_mut();
        if let Some((k, found)) = kept.as_ref()
            && *k == key
        {
            return found.clone();
        }
        #[cfg(test)]
        DRILL_HOLE_RUNS.with(|n| n.set(n.get() + 1));
        // At several A angles, each hole read at the angle that cuts it.
        let found = (self
            .doc
            .holes_listed(self.doc.index(), op, mesh, |posed, op| {
                cam::holes(posed, op, mesh)
            }))
        .map(|listed| {
            listed.map(|l| (l.into_iter().map(|(a, h)| cam::Hole { a: Some(a), ..h })).collect())
        })
        .unwrap_or_else(|| cam::holes(setup, op, mesh));
        let found = std::sync::Arc::new(found);
        *kept = Some((key, found.clone()));
        found
    }
    /// The holes the selected walls of a Pocket or an inside Contour pick
    /// (`pocket::holes`), for its Geometry tab: kept until the part, the
    /// setup's placement or the selected surfaces change, as `drill_holes` is.
    pub(crate) fn pocket_holes(
        &self,
        op: &Operation,
        mesh: &mesh::Mesh,
    ) -> std::sync::Arc<PocketHoles> {
        let key = self.holes_key(op, mesh);
        let mut kept = self.pocket_holes.borrow_mut();
        if let Some((k, found)) = kept.as_ref()
            && *k == key
        {
            return found.clone();
        }
        let ids = op
            .surfaces
            .iter()
            .flat_map(|s| s.ids.iter().copied())
            .collect();
        let found = (self
            .doc
            .holes_listed(self.doc.index(), op, mesh, |posed, op| {
                let ids = (op.surfaces.iter()).flat_map(|s| s.ids.iter().copied());
                pocket::holes(posed, &ids.collect(), mesh)
            }))
        .map(|listed| {
            listed.map(|l| (l.into_iter().map(|(a, h)| pocket::Hole { a: Some(a), ..h })).collect())
        })
        .unwrap_or_else(|| pocket::holes(self.doc.setup(), &ids, mesh));
        let found = std::sync::Arc::new(found);
        *kept = Some((key, found.clone()));
        found
    }
    /// What the part's holes the 3D view outlines are worked out from (the
    /// part and the setup's placement), while a Pocket, a Contour or a
    /// Drill of the part's holes is being picked on the part; None otherwise.
    pub(crate) fn part_holes_key(&self) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        let op = self.operation()?;
        if !self.picking || self.is_sample() || !picks_holes(op) {
            return None;
        }
        let mesh = self.part_mesh()?;
        let setup = self.doc.setup();
        let mut hasher = std::hash::DefaultHasher::new();
        mesh.fingerprint.hash(&mut hasher);
        for key in ["rx", "ry", "rz", "tx", "ty", "tz"] {
            get(&setup.p, key).hash(&mut hasher);
        }
        Some(hasher.finish())
    }
    /// Works out the part's holes for picking by their outline when they
    /// are wanted and not known: off the window's thread (a second or more
    /// on a part of thousands of walls), or here in a smoke run and tests.
    /// None when it starts no work off the window's thread.
    fn refresh_part_holes(&mut self) -> Option<Task<Message>> {
        let key = self.part_holes_key()?;
        if self.part_holes.as_ref().is_some_and(|(k, _)| *k == key)
            || self.part_holes_job == Some(key)
        {
            return None;
        }
        let (setup, mesh) = (self.doc.setup().clone(), self.part_mesh().unwrap().clone());
        if self.generate_inline {
            let found = find_holes(&setup, &mesh);
            self.part_holes = Some((key, std::sync::Arc::new(found)));
            return None;
        }
        self.part_holes_job = Some(key);
        Some(Task::perform(
            async move {
                tokio::task::spawn_blocking(move || find_holes(&setup, &mesh))
                    .await
                    .unwrap_or_default()
            },
            move |found| Message::PartHoles(key, PartHoles(std::sync::Arc::new(found))),
        ))
    }
    /// Whether the part's holes for the outlines are being worked out.
    pub(crate) fn finding_holes(&self) -> bool {
        self.part_holes_job.is_some() && self.part_holes_job == self.part_holes_key()
    }
    /// The part's holes as the 3D view outlines them while a Pocket, a
    /// Contour or a Drill is being picked, each marked when the operation
    /// has it. A Drill's are its round holes only, each once, all shown
    /// with the diameter it drills.
    pub(crate) fn hole_outlines(&self) -> Vec<viewport::HoleOutline> {
        let (Some(key), Some((known, found)), Some(op)) =
            (self.part_holes_key(), &self.part_holes, self.operation())
        else {
            return vec![];
        };
        if key != *known {
            return vec![];
        }
        let ids: std::collections::HashSet<usize> = (op.surfaces.iter())
            .flat_map(|s| s.ids.iter().copied())
            .collect();
        let outline = |h: &pocket::Hole| {
            let loops: Vec<Vec<(f64, f64)>> = h.levels[0].open.clone().into();
            viewport::HoleOutline {
                loops: (loops.iter())
                    .map(|l| l.iter().map(|q| [q.0 as f32, q.1 as f32]).collect())
                    .collect(),
                z: h.top,
                walls: h.walls.clone(),
                picked: h.walls.iter().any(|w| ids.contains(w)),
                label: None,
            }
        };
        if op.strategy != Strategy::Drill {
            return found.holes.iter().map(outline).collect();
        }
        // What the Drill drills: by its walls or in a selected rim face.
        let drilled: Vec<[f32; 2]> = self
            .part_mesh()
            .and_then(|mesh| self.drill_holes(op, mesh).as_ref().as_ref().ok().cloned())
            .map_or(vec![], |holes| holes.iter().map(|h| h.center).collect());
        let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) <= 0.05;
        // A counterbore and its bore are one hole to a drill, read alike:
        // the widest outline stands for it.
        let mut order: Vec<usize> = (0..found.holes.len()).collect();
        let area = |i: usize| found.holes[i].levels[0].open.signed_area().abs();
        order.sort_by(|&a, &b| area(b).total_cmp(&area(a)));
        let mut seen: Vec<[f32; 2]> = vec![];
        let mut out = vec![];
        for i in order {
            let (h, Some(round)) = (&found.holes[i], &found.round[i]) else {
                continue;
            };
            if seen.iter().any(|&c| near(c, round.center)) {
                continue;
            }
            seen.push(round.center);
            let mut o = outline(h);
            o.picked = drilled.iter().any(|&c| near(c, round.center));
            // The largest drill in 0.1 mm steps it takes, as drills come:
            // never a size the drill check then refuses.
            let d = cam::largest_drill(round);
            o.label = Some(format!("Ø{}", crate::post::exact_mm(d)));
            out.push(o);
        }
        out
    }
    /// What the active setup's holes are read in, for the kept hole lists:
    /// its placement; for an indexed setup all its fields and its machine's
    /// (its A angles, its Reposition action, the A centreline and limits),
    /// its A turn, and its base setup's fields and machine's.
    fn hash_placement(&self, hasher: &mut impl std::hash::Hasher) {
        use std::hash::Hash;
        let si = self.doc.index();
        let setup = self.doc.setup();
        for key in ["rx", "ry", "rz", "tx", "ty", "tz"] {
            get(&setup.p, key).hash(hasher);
        }
        if indexed::is_indexed(setup) || indexed::is_label_only(setup) {
            (&setup.p, &setup.machine.p).hash(hasher);
            format!("{:?}", setup.turn).hash(hasher);
            if let Some(base) = self.doc.index_base(si).and_then(|b| self.doc.setups.get(b)) {
                (&base.p, &base.machine.p).hash(hasher);
            }
        }
    }
    /// What the holes a Pocket's or Contour's walls pick are worked out from:
    /// the part, the setup's placement and the selected surfaces.
    fn holes_key(&self, op: &Operation, mesh: &mesh::Mesh) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::hash::DefaultHasher::new();
        mesh.fingerprint.hash(&mut hasher);
        self.hash_placement(&mut hasher);
        op.surfaces
            .as_ref()
            .map(|s| (&s.mesh_fingerprint, &s.ids))
            .hash(&mut hasher);
        hasher.finish()
    }
    /// Which levels of the holes a Pocket's or Contour's walls pick its flat
    /// end mill fits with its radial allowance (`pocket::Hole::fits`), and
    /// how low a Pocket clears over each it fits none of: kept as
    /// `pocket_holes` is, and until the tool, the allowances, the offsets or
    /// the setup change.
    pub(crate) fn pocket_fits(
        &self,
        op: &Operation,
        mesh: &mesh::Mesh,
    ) -> std::sync::Arc<PocketFits> {
        use std::hash::{Hash, Hasher};
        let tool = self.doc.tools.iter().find(|t| t.id == op.tool);
        let diameter = tool
            .filter(|t| get(&t.p, "type") == "Flat end mill")
            .and_then(|t| num(&t.p, "diameter"))
            .filter(|d| *d > 0.);
        let radial = num(&op.p, "allowance").filter(|a| *a >= 0.);
        let holes = self.pocket_holes(op, mesh);
        let mut hasher = std::hash::DefaultHasher::new();
        self.holes_key(op, mesh).hash(&mut hasher);
        diameter.map(f32::to_bits).hash(&mut hasher);
        radial.map(f32::to_bits).hash(&mut hasher);
        let pocket = op.strategy == Strategy::Pocket;
        pocket.hash(&mut hasher);
        (get(&op.p, "bottom"), get(&op.p, "axial_allowance")).hash(&mut hasher);
        self.doc.setup().p.hash(&mut hasher);
        let key = hasher.finish();
        let mut kept = self.pocket_fits.borrow_mut();
        if let Some((k, found)) = kept.as_ref()
            && *k == key
        {
            return found.clone();
        }
        let found = std::sync::Arc::new(match (&*holes, diameter, radial) {
            (Ok(holes), Some(diameter), Some(radial)) if !holes.is_empty() => {
                let fits: Vec<Vec<bool>> = holes.iter().map(|h| h.fits(diameter, radial)).collect();
                let cleared = (holes.iter().zip(&fits))
                    .map(|(h, fits)| {
                        (pocket && fits.iter().all(|f| !f))
                            .then(|| h.cleared_to(self.doc.setup(), op, mesh, diameter, radial))
                            .flatten()
                    })
                    .collect();
                Some((diameter, fits, cleared))
            }
            _ => None,
        });
        *kept = Some((key, found.clone()));
        found
    }
    /// The round holes the part opens on the current setup's bottom
    /// (`cam::bottom_holes`), the holes it can drop over fixture pins by:
    /// kept until the part or the setup's placement changes, as
    /// `drill_holes` is.
    pub(crate) fn bottom_holes(&self, mesh: &mesh::Mesh) -> std::sync::Arc<BottomHoles> {
        use std::hash::{Hash, Hasher};
        let setup = self.doc.setup();
        let mut hasher = std::hash::DefaultHasher::new();
        mesh.fingerprint.hash(&mut hasher);
        for key in ["rx", "ry", "rz", "tx", "ty", "tz"] {
            get(&setup.p, key).hash(&mut hasher);
        }
        let key = hasher.finish();
        let mut kept = self.bottom_holes.borrow_mut();
        if let Some((k, found)) = kept.as_ref()
            && *k == key
        {
            return found.clone();
        }
        let found = std::sync::Arc::new(cam::bottom_holes(setup, mesh));
        *kept = Some((key, found.clone()));
        found
    }
    /// The fingerprint of the job as the stock the earlier setups leave is
    /// concerned (`left_fingerprint`). None when there is nothing to work out:
    /// no setup, the example part (which is not simulated) or no part loaded.
    pub(crate) fn left_key(&self) -> Option<u64> {
        if self.doc.setups.is_empty() || self.is_sample() {
            return None;
        }
        left_fingerprint(&self.doc, self.part_mesh()?)
    }
    /// The stock of each setup as worked out, when that was for the job as it
    /// is now.
    fn left_boxes(&self) -> Option<&[Option<mesh::Bounds>]> {
        let key = self.left_key()?;
        (self.left_stock.key == key).then_some(self.left_stock.boxes.as_slice())
    }
    /// The box setup `si` starts from, in its frame (mm), and whether it is the
    /// real one. The first setup starts from its own stock. A later setup
    /// starts from what the earlier ones left (`left_stock`) when that was
    /// worked out for the job as it is now; until then, and when its
    /// simulation failed, from the original stock carried into its frame
    /// (`Document::setup_stock`), which is not current. The example part is not
    /// simulated: its original stock is all there is.
    pub fn known_stock(&self, si: usize) -> (Option<mesh::Bounds>, bool) {
        self.known_stock_in(self.start_boxes(si), si)
    }
    /// `left_boxes`, or, while the job is worked out again, the boxes last
    /// worked out when setup `si` and the setups before it are as they were
    /// then (`start_fingerprint`): an edit of a later setup or of its own
    /// operations leaves its box as it is. Only `si`'s entry is good then.
    fn start_boxes(&self, si: usize) -> Option<&[Option<mesh::Bounds>]> {
        self.left_boxes().or_else(|| {
            let key = start_fingerprint(&self.doc, self.part_mesh()?, si)?;
            (self.left_stock.starts.get(si) == Some(&key))
                .then_some(self.left_stock.boxes.as_slice())
        })
    }
    /// `current_checks`, or, while they are worked out again, those last
    /// worked out when setup `si`'s grip is as it was then
    /// (`grip_fingerprint`): good for `si`'s `held` alone.
    fn start_checks(&self, si: usize) -> Option<&[Option<SetupHolding>]> {
        self.current_checks().or_else(|| {
            let start = start_fingerprint(&self.doc, self.part_mesh()?, si)?;
            let key = grip_fingerprint(&self.doc, start, si)?;
            (self.left_stock.starts.get(si) == Some(&start)
                && self.holding_checks.grips.get(si) == Some(&key))
            .then_some(self.holding_checks.setups.as_slice())
        })
    }
    /// `known_stock` for the `left` boxes of `left_boxes`, worked out once for
    /// several setups.
    pub(crate) fn known_stock_in(
        &self,
        left: Option<&[Option<mesh::Bounds>]>,
        si: usize,
    ) -> (Option<mesh::Bounds>, bool) {
        if si > 0
            && !self.is_sample()
            && let Some(stock) = left.and_then(|boxes| boxes.get(si).copied().flatten())
        {
            return (Some(stock), true);
        }
        (
            self.doc.setup_stock(si, self.part_mesh()),
            si == 0 || self.is_sample(),
        )
    }
    /// The fingerprint of the job as its holding checks are concerned
    /// (`holding_fingerprint`): the stock's (`left_key`), how each setup is
    /// held and the workholding library. None when `left_key` is.
    pub(crate) fn holding_key(&self) -> Option<u64> {
        holding_fingerprint(&self.doc, self.left_key()?)
    }
    /// The holding checks of each setup as worked out, when that was for the
    /// job and its workholding as they are now.
    pub(crate) fn current_checks(&self) -> Option<&[Option<SetupHolding>]> {
        let key = self.holding_key()?;
        (self.holding_checks.key == key).then_some(self.holding_checks.setups.as_slice())
    }
    /// What setup `si`'s workholding holds as far as the editor knows: the
    /// box `known_stock_in` gives (`left`, from `left_boxes`) and the band
    /// its vise closes on that the holding checks found for that box
    /// (`checks`, from `current_checks`); the band is the box itself until
    /// they are worked out, for setup 1 and without a vise. None without a
    /// box.
    pub(crate) fn known_held_in(
        &self,
        left: Option<&[Option<mesh::Bounds>]>,
        checks: Option<&[Option<SetupHolding>]>,
        si: usize,
    ) -> Option<holding::Gripped> {
        let stock = self.known_stock_in(left, si).0?;
        let found = checks.and_then(|c| c.get(si)?.as_ref());
        Some(
            found
                .map(|c| c.held)
                .filter(|held| held.stock == stock)
                .unwrap_or(holding::Gripped::whole(stock)),
        )
    }
    /// `known_held_in` for the job as it is now.
    pub(crate) fn known_held(&self, si: usize) -> Option<holding::Gripped> {
        self.known_held_in(self.start_boxes(si), self.start_checks(si), si)
    }
    /// What tells the bands setup `si`'s vise closes on, from the stock the
    /// earlier setups left it as worked out for the job as it is now. None
    /// for setup 1, until it is worked out and where nothing was cut.
    fn known_bands(&self, si: usize) -> Option<holding::Bands> {
        let mesh = self.part_mesh()?;
        let volume = self
            .start_boxes(si)
            .and(self.left_stock.before.get(si))?
            .as_deref()
            .filter(|v| si > 0 && !v.cuts.is_empty())?;
        let setup = self.doc.setups.get(si)?;
        Some(holding::Bands::new(
            volume,
            volume::Frame::new(setup, mesh),
            simulation::part_box(setup, mesh),
        ))
    }
    /// Works out the stock each later setup starts from and its holding
    /// checks (`leftover_stock`, `holding_checks`), when the job or its
    /// workholding has changed since and no run is working on it: on the
    /// blocking pool (in line in the smoke run), as `refresh_post` checks the
    /// program. A change of the workholding alone keeps the stock worked out
    /// and checks it again. Only in the Manufacture workspace: on the Control
    /// page a run of it would hold Cycle Start back between tool runs.
    fn refresh_left_stock(&mut self) -> Task<Message> {
        let (Some(key), Some(held)) = (self.left_key(), self.holding_key()) else {
            return Task::none();
        };
        let known = self.left_stock.key == key;
        if self.left_job.is_some()
            || self.locked()
            || self.page == Page::Control
            || (known && self.holding_checks.key == held)
        {
            return Task::none();
        }
        let Some(mesh) = self.part_mesh().cloned() else {
            return Task::none();
        };
        let doc = self.doc.clone();
        let left = known.then(|| self.left_stock.clone());
        let run = move || {
            let left = left.unwrap_or_else(|| leftover_stock(&doc, &mesh, key));
            let checks = holding_checks(&doc, &mesh, &left, held);
            (left, checks)
        };
        if self.smoke_dir.is_some() {
            (self.left_stock, self.holding_checks) = run();
            return Task::none();
        }
        self.left_job = Some(key);
        Task::perform(
            async move {
                // A run that failed leaves no boxes and no checks: the
                // original stock is kept, and the job is not run again.
                tokio::task::spawn_blocking(run).await.unwrap_or_else(|_| {
                    let left = LeftStock {
                        key,
                        ..Default::default()
                    };
                    let checks = HoldingChecks {
                        key: held,
                        ..Default::default()
                    };
                    (left, checks)
                })
            },
            |found| Message::LeftStock(Box::new(found)),
        )
    }
    /// The fingerprint of the job as the step guide of the active setup is
    /// concerned: the stock it starts from and how it is held
    /// (`holding_fingerprint`), which setup it is and what was measured on
    /// it. None when the active setup is not on a hand machine (the manual
    /// mill, the lathe), and when `left_key` is.
    pub(crate) fn guide_key(&self) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        let setup = self.doc.active_setup()?;
        if !manual::is_hand(&setup.machine) {
            return None;
        }
        let mut hasher = std::hash::DefaultHasher::new();
        (self.holding_key()?, &setup.id).hash(&mut hasher);
        let mine = guide::measure_key(&setup.id, "");
        for (k, v) in self
            .doc
            .measured
            .iter()
            .filter(|(k, _)| k.starts_with(&mine))
        {
            (k, v.to_bits()).hash(&mut hasher);
        }
        Some(hasher.finish())
    }
    /// MACHINE shows the Steps page: the active setup is on a hand machine.
    pub(crate) fn steps_page(&self) -> bool {
        self.page == Page::Control
            && !self.machine_setup
            && (self.doc.active_setup()).is_some_and(|s| {
                manual::is_hand(&s.machine) && self.cnc_over_guide.as_ref() != Some(&s.id)
            })
    }
    /// The guide shown worked out again at once from its own simulation
    /// (`steps::Shown::again`), for the job as it is now: after a measurement
    /// or the dial check's answer, which change no cut.
    fn guide_again(&mut self, shown: &steps::Shown) {
        let (Some(key), Some(mesh)) = (self.guide_key(), self.part_mesh()) else {
            return;
        };
        let again = shown.again(&self.doc, mesh).map(std::sync::Arc::new);
        self.guide = Some((key, again));
    }
    /// The guide as built, when that was for the job as it is now.
    pub(crate) fn current_guide(&self) -> Option<&Result<std::sync::Arc<steps::Shown>, String>> {
        let key = self.guide_key()?;
        let (built, result) = self.guide.as_ref()?;
        (*built == key).then_some(result)
    }
    /// The guide shown and the step it is at (`guide::place`).
    pub(crate) fn guide_at(&self) -> Option<(std::sync::Arc<steps::Shown>, usize)> {
        let shown = self.current_guide()?.as_ref().ok()?.clone();
        let at = guide::place(&self.doc, &shown.guide);
        Some((shown, at))
    }
    /// Builds the active manual setup's guide (`steps::shown`) while the
    /// Steps page is open, when the job changed since and no run is building
    /// it: on the blocking pool, in line in the smoke run. Not held back
    /// while the machine moves: the page is for the manual mill beside it,
    /// and editing, which would build it again, is locked then.
    fn refresh_guide(&mut self) -> Task<Message> {
        if !self.steps_page() {
            return Task::none();
        }
        let Some(key) = self.guide_key() else {
            return Task::none();
        };
        if self.guide_job == Some(key) || self.guide.as_ref().is_some_and(|(k, _)| *k == key) {
            return Task::none();
        }
        let Some(mesh) = self.part_mesh().cloned() else {
            return Task::none();
        };
        let (doc, si) = (self.doc.clone(), self.doc.index());
        let run = move || steps::shown(&doc, &mesh, si).map(std::sync::Arc::new);
        if self.smoke_dir.is_some() {
            self.guide = Some((key, run()));
            return Task::none();
        }
        self.guide_job = Some(key);
        Task::perform(
            async move {
                tokio::task::spawn_blocking(run)
                    .await
                    .unwrap_or_else(|_| Err("The steps could not be worked out.".into()))
            },
            move |result| Message::Guide(GuideAction::Built(key, result)),
        )
    }
    /// Puts the guide of `shown` at step `at`, saved with the project as
    /// `SelectSetup` saves the active setup: no undo step.
    fn guide_to(&mut self, shown: &steps::Shown, at: usize) {
        let at = at.min(shown.guide.steps.len().saturating_sub(1));
        if let Some(setup) = self.doc.setups.get(shown.guide.setup)
            && self.doc.guide.get(&setup.id) != Some(&at)
        {
            self.doc.guide.insert(setup.id.clone(), at);
            self.dirty = true;
            self.last_edit = Instant::now();
        }
        self.guide_item = 0;
        self.guide_clock.0 = Instant::now();
    }
    fn guide_action(&mut self, action: GuideAction) -> Task<Message> {
        match action {
            GuideAction::Built(key, result) => {
                self.guide_job = None;
                // A job edited while it was built gets a guide of its own.
                if self.guide_key() == Some(key) {
                    self.guide = Some((key, result));
                    self.guide_clock.0 = Instant::now();
                }
                return Task::none();
            }
            GuideAction::Beat(at) => {
                self.guide_clock.1 = at;
                return Task::none();
            }
            GuideAction::Open(si) => {
                let Some(s) = self.doc.setups.get(si) else {
                    return Task::none();
                };
                self.cnc_over_guide = None;
                if s.id != self.doc.active {
                    self.doc.active = s.id.clone();
                    self.stop_selection();
                    self.dirty = true;
                    self.last_edit = Instant::now();
                    self.guide_item = 0;
                }
                self.machine_setup = false;
                return self.update(Message::Page(Page::Control));
            }
            _ => {}
        }
        let Some((shown, at)) = self.guide_at() else {
            return Task::none();
        };
        let holes = match &shown.guide.steps[at] {
            guide::Step::Holes { holes, .. } => holes.len(),
            _ => 0,
        };
        match action {
            GuideAction::Next if self.guide_item + 1 < holes => self.guide_item += 1,
            GuideAction::Next => self.guide_to(&shown, at + 1),
            GuideAction::Back if holes > 0 && self.guide_item > 0 => self.guide_item -= 1,
            GuideAction::Back => self.guide_to(&shown, at.saturating_sub(1)),
            GuideAction::Go(i) => self.guide_to(&shown, i),
            GuideAction::Section(i) => {
                if let Some(s) = shown.guide.sections.get(i)
                    && !s.steps.contains(&at)
                {
                    self.guide_to(&shown, s.steps.start);
                }
            }
            GuideAction::Reached(i) => self.guide_item = i + 1,
            GuideAction::Typed(key, text) => self.guide_typed = Some((key, text)),
            GuideAction::Measured(key, value) => {
                if !(value.is_finite() && value > 0. && value < 10_000.) {
                    self.status = "Type the diameter the calipers read, in mm.".into();
                    return Task::none();
                }
                let mine = (self.doc.setups.get(shown.guide.setup))
                    .is_some_and(|s| key.starts_with(&guide::measure_key(&s.id, "")));
                if !mine {
                    return Task::none();
                }
                // A measurement, not a setting: kept with the project, as the
                // place is, with no undo step.
                if self.doc.measured.get(&key) != Some(&value) {
                    self.doc.measured.insert(key.clone(), value);
                    self.dirty = true;
                    self.last_edit = Instant::now();
                }
                if self.guide_typed.as_ref().is_some_and(|(k, _)| *k == key) {
                    self.guide_typed = None;
                }
                self.status = format!("Measured Ø{value:.2} kept.");
                self.guide_again(&shown);
            }
            GuideAction::CrossReads(reads) => {
                let Some(setup) = self.doc.setups.get(shown.guide.setup) else {
                    return Task::none();
                };
                if !lathe::is_lathe(&setup.machine) {
                    return Task::none();
                }
                let id = setup.machine.id.clone();
                let set = |m: &mut Machine| {
                    if m.id == id {
                        m.p.insert("cross_reads".into(), reads.key().into());
                    }
                };
                self.doc.machines.iter_mut().for_each(set);
                for s in &mut self.doc.setups {
                    set(&mut s.machine);
                }
                set(&mut self.machine_draft);
                // An edit of the machine. While the CNC runs, nothing it
                // runs depends on the lathe: its run is not marked stale.
                if self.locked() {
                    self.dirty = true;
                    self.project_dirty = true;
                    self.last_edit = Instant::now();
                } else {
                    self.touch();
                }
                self.status = format!(
                    "The cross-slide dial reads the {}: saved to {}.",
                    reads.key().to_lowercase(),
                    self.doc.setups[shown.guide.setup].machine.name()
                );
                self.guide_again(&shown);
            }
            _ => {}
        }
        Task::none()
    }
    /// The checks before a run of the active setup, when they were worked out
    /// for the job as it is now.
    pub(crate) fn current_ready(&self) -> Option<&run::Ready> {
        let key = self.holding_key()?;
        (self.ready.key == key && self.ready.setup == self.doc.index()).then_some(&self.ready)
    }
    /// The last checks of the active setup, when the job has changed since
    /// they were worked out: what the simulation panel keeps the shape of
    /// while they are worked out again. None before the setup was checked
    /// (the key of `Ready::default` is 0).
    pub(crate) fn stale_ready(&self) -> Option<&run::Ready> {
        (self.current_ready().is_none()
            && self.ready.key != 0
            && self.ready.setup == self.doc.index())
        .then_some(&self.ready)
    }
    /// Checks the active setup as a run would (`run::ready`), when the job
    /// changed since or another setup is active, as `refresh_left_stock` works
    /// out the stock: on the blocking pool (in line in the smoke run), only in
    /// the Manufacture workspace and not while the machine moves.
    fn refresh_ready(&mut self) -> Task<Message> {
        let Some(key) = self.holding_key() else {
            return Task::none();
        };
        if self.ready_job.is_some()
            || self.locked()
            || self.page == Page::Control
            || self.current_ready().is_some()
        {
            return Task::none();
        }
        let Some(mesh) = self.part_mesh().cloned() else {
            return Task::none();
        };
        let (doc, si) = (self.doc.clone(), self.doc.index());
        let check = move || run::ready(&doc, &mesh, si, key);
        if self.smoke_dir.is_some() {
            self.ready = check();
            return Task::none();
        }
        self.ready_job = Some(key);
        Task::perform(
            async move {
                // A check that failed is not run again for the same job.
                tokio::task::spawn_blocking(check)
                    .await
                    .unwrap_or_else(|e| run::Ready {
                        key,
                        setup: si,
                        problems: vec![format!("The check failed: {e}")],
                        ..Default::default()
                    })
            },
            |ready| Message::Ready(Box::new(ready)),
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
    /// Starts a planner run with `chat`: Plan this job when it has no
    /// request, else a change. A change starts from the preview when one is
    /// open (as Apply would make it), else from the job.
    fn start_plan(&mut self, chat: planner::Chat) -> Task<Message> {
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
        let (Some(client), Some(mesh)) = (self.client.clone(), self.imported_mesh.clone()) else {
            return Task::none();
        };
        let asked = (!chat.request.is_empty()).then(|| chat.request.clone());
        let refined = match (&self.planner, &asked) {
            (Planner::Preview(preview), Some(_)) => Some(Refined {
                preview: preview.clone(),
                timeline: self.plan_timeline.clone(),
                overview: self.plan_overview.clone(),
                asked: self.plan_asked.clone(),
            }),
            _ => None,
        };
        let doc = refined.as_ref().map_or_else(
            || self.doc.clone(),
            |r| planner::applied(&r.preview.outcome),
        );
        let edit = planner::editing(&doc, &chat);
        self.drop_plan();
        self.plan_refined = refined;
        let epoch = self.plan_epoch;
        self.plan_account_epoch = self.account_epoch;
        self.plan_model = self.settings.model.clone();
        self.planner = Planner::Running(planner::Progress::Analyzing);
        if asked.is_some() {
            self.plan_request.clear();
        }
        self.plan_asked = asked;
        self.status = if edit {
            "Asking the planner for the change. Nothing changes until you apply it."
        } else {
            "Planning. Nothing changes until you apply a plan."
        }
        .into();
        let backend = self.plan_backend(client);
        let busy = self.plan_busy.clone();
        let run = iced::stream::channel(16, async move |sender: PlanSender| {
            run_plan(&backend, doc, mesh, chat, busy, epoch, sender).await
        });
        let (task, handle) = Task::run(run, |message| message).abortable();
        self.plan_run = Some(handle);
        task
    }
    /// Shows again the preview a change started from.
    fn restore_refined(&mut self, refined: Refined) {
        self.planner = Planner::Preview(refined.preview);
        self.plan_timeline = refined.timeline;
        self.plan_overview = refined.overview;
        self.plan_asked = refined.asked;
    }
    /// Stop a running plan and forget a preview: both describe the job as it was.
    fn drop_plan(&mut self) {
        if let Some(handle) = self.plan_run.take() {
            handle.abort();
        }
        self.plan_epoch = self.plan_epoch.wrapping_add(1);
        self.planner = Planner::Idle;
        self.plan_timeline.clear();
        self.plan_overview = None;
        self.plan_enlarged = None;
        self.plan_expanded.clear();
        self.plan_asked = None;
        self.plan_refined = None;
        // The aborted run may still write to the old text.
        self.plan_stream = Default::default();
    }
    /// Ends the timeline's open step, if any, now.
    fn end_plan_step(&mut self) {
        if let Some(step) = self.plan_timeline.last_mut() {
            step.ended.get_or_insert_with(Instant::now);
        }
    }
    /// The latest step of request `n`, of whichever kind.
    fn request_step(&mut self, n: usize) -> Option<&mut Step> {
        use planner::Progress;
        self.plan_timeline.iter_mut().rev().find(|step| {
            matches!(step.stage,
                Progress::Asking(at) | Progress::Repairing(at) | Progress::Extending(at)
                if at == n)
        })
    }
    /// Adds a run's event to the timeline: a stage ends the open step and
    /// starts its own; a result joins the step of its request and ends it; a
    /// picture or a refusal joins the request it came with.
    fn record_plan_event(&mut self, event: planner::Event) {
        use planner::{Event, Progress, StepResult};
        // A picture's handle is made once, here, not on every frame.
        let thumb = |look: &planner::Look| {
            look.png
                .as_ref()
                .map(|png| iced::widget::image::Handle::from_bytes(png.to_vec()))
        };
        match event {
            Event::Stage(stage) => {
                self.end_plan_step();
                self.plan_timeline.push(Step {
                    stage,
                    started: Instant::now(),
                    ended: None,
                    result: None,
                    looks: vec![],
                });
                self.planner = Planner::Running(stage);
            }
            Event::Checked(n, result) => {
                let reply = matches!(result, StepResult::Unreadable(_));
                let found = if reply {
                    self.request_step(n)
                } else {
                    self.plan_timeline
                        .iter_mut()
                        .rev()
                        .find(|step| step.stage == Progress::Checking(n))
                };
                if let Some(step) = found {
                    step.result = Some(result);
                    step.ended.get_or_insert_with(Instant::now);
                }
            }
            Event::Overview(look) => {
                self.plan_overview = Some(Shown {
                    line: look.description.clone(),
                    warn: false,
                    thumb: thumb(&look),
                });
            }
            Event::Looked(n, look) => {
                let shown = Shown {
                    line: ui::look_line(&look),
                    warn: false,
                    thumb: thumb(&look),
                };
                if let Some(step) = self.request_step(n) {
                    step.looks.push(shown);
                }
            }
            Event::Dropped(n, what) => {
                let (line, _) = ui::refusal(what);
                if let Some(step) = self.request_step(n) {
                    step.looks.push(Shown {
                        line: line.into(),
                        warn: true,
                        thumb: None,
                    });
                }
            }
        }
    }
    /// The backend a run started now talks to: through `client`, with the
    /// chosen model, streaming into the timeline's text, and with pictures
    /// unless they are turned off. The run keeps them as they are now.
    pub(crate) fn plan_backend(
        &self,
        client: std::sync::Arc<openai::Client>,
    ) -> planner::OpenAiBackend {
        planner::OpenAiBackend::new(
            client,
            self.settings.model.clone(),
            self.plan_stream.clone(),
            !self.settings.plan_pictures_off,
        )
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
    /// Without a client ID a saved session cannot be refreshed or revoked, so
    /// it is deleted in the same step and the result says so, as an error.
    fn restore_account(&mut self) -> Task<Message> {
        if std::mem::replace(&mut self.account_restored, true) {
            return Task::none();
        }
        self.restoring = true;
        let store = self.store.clone();
        let client_lost = self.settings.client_id.is_empty();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || read_saved_session(&*store, client_lost))
                    .await
                    .unwrap_or_else(|e| Err(format!("Could not read the saved sign-in: {e}")))
            },
            Message::AccountRestored,
        )
    }
    fn stop_selection(&mut self) {
        self.selected = None;
        self.setup_end = false;
        self.picking = false;
        self.playing = false;
        self.progress = 0.;
        self.simulation = None;
        self.confirm_delete_setup = false;
        self.confirm_delete_op = false;
    }
    /// Carries out the pending action, once its page has saved what the
    /// action needed.
    fn resume(&mut self) -> Task<Message> {
        match self.pending.take() {
            Some((_, action)) => self.update(action),
            None => Task::none(),
        }
    }
    /// The Save button on the pending action's page: it says what follows.
    pub fn resume_label(&self) -> Option<String> {
        Some(match &self.pending.as_ref()?.1 {
            Message::AddOp(s) => format!("Save and continue to {}", s.name()),
            Message::AddSetup => "Save and create first setup".into(),
            _ => return None,
        })
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        let before = self.status.clone();
        let (selected, page) = (self.selected, self.page);
        let task = self.handle(message);
        // Batched only when it starts one: a batch is never an empty task.
        let task = match self.refresh_part_holes() {
            Some(holes) => Task::batch([task, holes]),
            None => task,
        };
        // The probes follow the project's material: the touch plate on stock
        // that does not conduct, the plate's lead on stock that does.
        if let Some(c) = &mut self.control {
            c.material =
                (self.doc.selected_material()).map(|m| (m.name().to_string(), m.conducts()));
        }
        // Another operation or page closes the delete question.
        if self.selected != selected || self.page != page {
            self.confirm_delete_op = false;
        }
        // An operation selected, however, leaves the stock after the setup.
        if self.selected.is_some() {
            self.setup_end = false;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|(page, _)| *page != self.page)
        {
            self.pending = None;
        }
        // A message that dropped a running plan or a preview says why. When
        // its handler set a status of its own, both are shown, its own first.
        if let Some(dropped) = self.plan_dropped.take() {
            self.status = if self.status != before && !self.status.is_empty() {
                format!("{} {dropped}", self.status)
            } else {
                dropped.into()
            };
        }
        if self.status != before {
            self.status_page = self.page;
        }
        task
    }
    fn handle(&mut self, message: Message) -> Task<Message> {
        if self.locked() && !while_locked(&message) {
            self.status = LOCKED.into();
            return Task::none();
        }
        // These actions require an explicitly created setup; no hidden defaults.
        if self.doc.active_setup().is_none()
            && matches!(
                &message,
                Message::SelectOp(_)
                    | Message::OperationsStep
                    | Message::PreviewStep
                    | Message::Edit(Target::Setup, _, _)
                    | Message::AddOp(_)
                    | Message::DuplicateOp
                    | Message::RemoveOp
                    | Message::AskRemoveOp
                    | Message::ConfirmDeleteOp(_)
                    | Message::MoveOp(_)
                    | Message::Generate
                    | Message::GenerateAll
                    | Message::DuplicateSetup
                    | Message::RemoveSetup
                    | Message::ConfirmDeleteSetup(_)
                    | Message::MoveSetup(_)
                    | Message::Flip(_)
                    | Message::TurnStock(..)
                    | Message::SquareToStock
                    | Message::SetClamps(_)
                    | Message::SuggestLayout
                    | Message::Plan
                    | Message::PlanSend
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
                    starter_materials: self.doc.starter_materials,
                    starter_tools: self.doc.starter_tools,
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
            Message::Frame(at) => return self.smoke_frame(at),
            Message::WindowSized(size) => self.smoke_sized(size),
            Message::Noop => {}
            Message::Page(p) => {
                self.help_animating = false;
                self.pinned_help = None;
                if p == Page::Control && self.page != Page::Control {
                    self.cam_page = self.page;
                }
                self.page = p;
                self.picking = false;
                self.playing = false;
                let restore = if p == Page::Ai {
                    self.restore_account()
                } else {
                    Task::none()
                };
                return Task::batch([self.refresh_post(), restore, self.refresh_guide()]);
            }
            Message::SelectSetup(i) => {
                // An operation's preview is generated for the setup it is in.
                if i != self.doc.index()
                    && self
                        .generating
                        .as_ref()
                        .is_some_and(|g| matches!(g.what, Generation::Operation(..)))
                {
                    self.cancel_generation();
                }
                if let Some(s) = self.doc.setups.get(i) {
                    self.doc.active = s.id.clone();
                    self.stop_selection();
                    self.page = Page::Work;
                    self.dirty = true;
                    self.last_edit = Instant::now();
                }
            }
            Message::SelectOp(i) => {
                if self
                    .generating
                    .as_ref()
                    .is_some_and(|g| matches!(g.what, Generation::Operation(_, oi) if oi != i))
                {
                    self.cancel_generation();
                }
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
                        .tabs(&self.doc.setup().machine)
                        .contains(&self.tab.as_str())
                    {
                        self.tab = "Tool".into();
                    }
                    self.picking = self.tab == "Geometry" && self.can_pick();
                }
            }
            Message::AuditRow(i) => {
                if let Some(row) = self.audit_report.get(i).cloned() {
                    let setup = self.update(Message::SelectSetup(row.setup));
                    let operation = self.update(Message::SelectOp(row.operation));
                    let tab = match self.operation() {
                        Some(o) => ui::operation_issue_tab(o.strategy, &row.detail),
                        None => ui::issue_tab(&row.detail),
                    };
                    if let Some(tab) = tab {
                        // A tab the operation does not have (a lathe Drill has
                        // no Passes) gives its place to the one with its sizes.
                        let tabs = (self.operation()).map(|o| o.tabs(&self.doc.setup().machine));
                        let tab = match tabs {
                            Some(tabs) if !tabs.contains(&tab) => {
                                tabs.get(1).copied().unwrap_or(tab)
                            }
                            _ => tab,
                        };
                        let _ = self.update(Message::Tab(tab.into()));
                    }
                    return Task::batch([setup, operation]);
                }
            }
            Message::SetupEditor => {
                self.selected = None;
                self.setup_end = false;
                self.page = Page::Work;
                self.picking = false;
                self.playing = false;
                self.simulation = None;
                self.progress = 0.;
            }
            Message::SetupEnd => {
                let _ = self.update(Message::SetupEditor);
                self.setup_end = true;
            }
            // The footer's 02 and 03 steps are navigation only: the operation
            // selected stays selected (the first when none is), and the
            // preview strip under the 3D view offers Generate & play.
            Message::OperationsStep | Message::PreviewStep => {
                if self.selected.is_some() {
                    self.page = Page::Work;
                } else {
                    return self.update(Message::SelectOp(0));
                }
            }
            // A step tab, or the workspace heading's link to the work zero:
            // the setup editor opens on that step.
            Message::SetupStep(step) => {
                if self.selected.is_some() || self.page != Page::Work {
                    let _ = self.update(Message::SetupEditor);
                }
                self.setup_step = step;
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
                    // A Turn's kind brings its own fields, speed and tool.
                    if k == "turn_kind"
                        && lathe::tool_for(&v).is_some()
                        && self
                            .operation()
                            .is_some_and(|o| o.strategy == Strategy::Turn)
                    {
                        let machine = self.doc.setup().machine.clone();
                        let has = |tool: &model::Tool| lathe::takes(&v, get(&tool.p, "type"));
                        let fitting = (self.doc.tools.iter())
                            .find(|t| t.fits(&machine) && has(t))
                            .map(|t| t.id.clone());
                        let current = self.operation().map(|o| o.tool.clone());
                        let right = self
                            .doc
                            .tools
                            .iter()
                            .any(|t| Some(&t.id) == current.as_ref() && has(t));
                        if let Some(o) = self.operation_mut() {
                            o.set_turn_kind(&v);
                            manual::snap(&machine, &mut o.p);
                            if let (false, Some(id)) = (right, fitting) {
                                o.tool = id;
                            }
                        }
                        self.touch();
                        return Task::none();
                    }
                    if let Some(o) = self.operation_mut() {
                        // A project saved before tabs has no sizes for them.
                        if k == "tabs" {
                            for (key, value) in model::TAB_DEFAULTS {
                                o.p.entry(key.into()).or_insert_with(|| value.into());
                            }
                        }
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
                    if self.doc.index() == 0 && matches!(k, "rx" | "ry" | "rz") {
                        let turned = self.doc.rotate_first(k, v);
                        self.say_turned_with_first(&turned);
                        self.touch();
                        return Task::none();
                    }
                    let holder = get(&self.doc.setup().p, "holder").to_string();
                    // Index rotary axis turns the stock about A at the angles
                    // the setup lists; another action takes them away.
                    if k == "reposition" {
                        indexed::set_reposition(&mut self.doc.setup_mut().p, &v);
                    } else {
                        self.doc.setup_mut().p.insert(k.into(), v);
                    }
                    if matches!(k, "holding" | "holder") {
                        self.doc.settle_holding(self.doc.index());
                    }
                    // Only the first Vise or Clamps and a different library
                    // entry place the jaws or clamps; later edits are kept.
                    let setup = self.doc.setup();
                    let suggest = match (k, get(&setup.p, "holding")) {
                        ("holding", "Vise" | "Clamps") => !holding::placed(setup),
                        ("holder", "Vise" | "Clamps") => get(&setup.p, "holder") != holder,
                        _ => false,
                    };
                    self.touch();
                    if suggest {
                        return self.suggest_layout();
                    }
                }
                Target::Machine => {
                    if k == "kind" {
                        manual::set_kind(&mut self.machine_draft, &v);
                    } else if k == "cross_reads" && !["Diameter", "Radius"].contains(&v.as_str()) {
                        // "Not known yet": the dial check will ask.
                        self.machine_draft.p.insert(k.into(), String::new());
                    } else {
                        self.machine_draft.p.insert(k.into(), v.clone());
                    }
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
                    if matches!(k, "axes" | "arrangement") {
                        self.machine_draft.fill_rotary_defaults();
                    }
                }
                Target::Tool => {
                    if k == "type" {
                        // A lathe type takes its sizes' defaults, as the editor shows them.
                        self.tool_draft.set_type(&v);
                    } else {
                        // A tap's size is its diameter.
                        let major = (k == "thread" && get(&self.tool_draft.p, "type") == "Tap")
                            .then(|| crate::thread::spec(&v))
                            .flatten()
                            .map(|s| s.major);
                        self.tool_draft.p.insert(k.into(), v);
                        if let Some(major) = major {
                            self.tool_draft
                                .p
                                .insert("diameter".into(), major.to_string());
                        }
                    }
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
                let machine = &self.doc.setup().machine;
                let mut operation = Operation::for_machine(strategy, String::new(), machine);
                // The first tool of the operation's first type, else of
                // another type it takes (`Operation::tool_types`).
                let types = operation.tool_types(machine);
                let fitting = |t: &&model::Tool| t.fits(machine) && operation.takes(t, machine);
                let Some(tool) = (self.doc.tools.iter())
                    .filter(fitting)
                    .find(|t| types.first() == Some(&get(&t.p, "type")))
                    .or_else(|| self.doc.tools.iter().find(fitting))
                    .cloned()
                else {
                    self.page = Page::Tools;
                    self.pending = Some((Page::Tools, Message::AddOp(strategy)));
                    self.status = format!(
                        "{} needs a cutter. Create one: saving it adds {}.",
                        strategy.name(),
                        strategy.name()
                    );
                    return Task::none();
                };
                operation.tool = tool.id.clone();
                operation
                    .p
                    .insert("coolant".into(), control::default_coolant(machine).into());
                if !self.is_sample() {
                    operation.geometry.clear();
                }
                // Adaptive clearing starts sized to its cutter: a load of
                // 15 % of its diameter and a stepdown of two diameters,
                // within its flutes. A Facing clears far enough past the
                // part's outline, when it faces that, for the cutter to
                // reach its corners (`outline_extend_for`).
                if matches!(strategy, Strategy::Adaptive | Strategy::Face)
                    && !self.is_sample()
                    && let Some(d) = num(&tool.p, "diameter").filter(|d| *d > 0.)
                {
                    if strategy == Strategy::Adaptive {
                        let load = ((d * 0.15 / 0.05).round() * 0.05).max(0.05);
                        let flutes = num(&tool.p, "length").unwrap_or(2. * d);
                        operation
                            .p
                            .insert("optimal_load".into(), format!("{load:.2}"));
                        operation
                            .p
                            .insert("stepdown".into(), format!("{:.1}", (2. * d).min(flutes)));
                    } else {
                        operation
                            .p
                            .insert("silhouette_extend".into(), model::outline_extend_for(d));
                    }
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
            Message::AskRemoveOp => self.confirm_delete_op = self.selected.is_some(),
            Message::ConfirmDeleteOp(yes) => {
                self.confirm_delete_op = false;
                if yes {
                    return self.handle(Message::RemoveOp);
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
                    || !self.operation().is_some_and(|o| {
                        o.strategy.imported_preview()
                            && o.strategy != Strategy::Turn
                            && !o.on_centre(&self.doc.setup().machine)
                    })
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
                // An indexed setup reads the click at the A angle that turns
                // its hole up, which may not be the one shown.
                let setup = self.doc.picking_pose(self.doc.index(), mesh, n);
                // A Pocket, an inside Contour or a Drill takes a wall as its
                // hole: every wall of it, added or removed together. A face
                // that is neither a floor nor a wall (a cone, a chamfer, a
                // fillet) takes the hole it borders, or nothing.
                let picks_holes = self.operation().is_some_and(picks_holes);
                let drill = self
                    .operation()
                    .is_some_and(|o| o.strategy == Strategy::Drill);
                // As the setup turns the part, by A too for an indexed one.
                let angles = crate::stock::Placement::new(mesh, Some(&setup)).rotation();
                let up = mesh::rotation(mesh.surfaces[n].normal, angles)[2].abs();
                let wall = pocket::is_wall(&setup, mesh, n);
                let sloped = !wall && up < 0.5f32.to_radians().cos();
                if picks_holes && (wall || sloped) {
                    // What a click here picks: the wall's hole, or the hole
                    // past a sloping face (a cone, a chamfer, a fillet) or past
                    // the one a wall's outline crosses (a screw hole's lip).
                    let found = match wall.then(|| pocket::hole_walls(&setup, mesh, n)) {
                        Some(Err(e)) if e == pocket::LEANING => {
                            pocket::hole_near(&setup, mesh, n).unwrap_or(Err(e))
                        }
                        Some(found) => found,
                        None => match pocket::hole_near(&setup, mesh, n) {
                            Some(Ok(walls)) => Ok(walls),
                            _ => Err(format!(
                                "That face slopes {:.0}°: it is no floor, and no hole past it can be picked from it. Click a flat floor, or a wall of a hole.",
                                up.clamp(0., 1.).acos().to_degrees()
                            )),
                        },
                    };
                    // A click on a picked hole drops it, whether or not its own
                    // outline picks a hole (`pocket::dropped`).
                    let picked: std::collections::HashSet<usize> = (self.operation())
                        .and_then(|o| o.surfaces.as_ref())
                        .filter(|s| s.mesh_fingerprint == fingerprint)
                        .map_or(Default::default(), |s| s.ids.iter().copied().collect());
                    let holes = self.operation().map(|o| self.pocket_holes(o, mesh));
                    let now: &[pocket::Hole] = holes
                        .as_ref()
                        .and_then(|h| h.as_ref().as_ref().ok())
                        .map_or(&[], |h| h.as_slice());
                    let clicked: Option<std::collections::HashSet<usize>> =
                        found.as_ref().ok().map(|w| w.iter().copied().collect());
                    let this = (clicked.as_ref())
                        .and_then(|c| pocket::holes(&setup, c, mesh).ok())
                        .unwrap_or_default();
                    let drop = pocket::dropped(now, &picked, n, clicked.as_ref(), &this);
                    // A Drill drills round holes: it reads the hole as it
                    // drills it, and names it by the diameter it drills.
                    let round = if drill {
                        cam::drill_holes_of(&setup, mesh, &this)
                    } else {
                        vec![]
                    };
                    if drill
                        && drop.is_none()
                        && let Some(h) = this.first()
                        && round.first().is_none_or(|r| r.is_none())
                    {
                        let ((x, y), (w, d)) = (h.centre(), h.size());
                        self.status = format!(
                            "The hole at X{x:.2} Y{y:.2} ({w:.2} × {d:.2} mm) is not round: a Drill drills round holes. Pick it in a Pocket."
                        );
                        return Task::none();
                    }
                    let named = |h: &pocket::Hole| {
                        let ((x, y), (w, d)) = (h.centre(), h.size());
                        match round.first().and_then(|r| r.as_ref()) {
                            Some(r) if drill => format!(
                                "the Ø{} hole at X{x:.2} Y{y:.2}",
                                crate::post::exact_mm(cam::largest_drill(r))
                            ),
                            _ => format!("the hole at X{x:.2} Y{y:.2} ({w:.2} × {d:.2} mm)"),
                        }
                    };
                    let said = match (&drop, this.first()) {
                        (Some(_), _) => (0..now.len())
                            .find(|&i| now[i].walls.contains(&n))
                            .map(|i| named(&now[i]))
                            .or_else(|| this.first().map(named))
                            .map_or("Dropped the hole.".into(), |h| format!("Dropped {h}.")),
                        (None, Some(h)) => format!("Picked {}: its walls are blue.", named(h)),
                        (None, None) if clicked.is_some() => {
                            "Picked the hole: its walls are blue.".into()
                        }
                        (None, None) => String::new(),
                    };
                    let dropping = drop.is_some();
                    let walls = match (drop.is_some(), found) {
                        (true, _) => vec![],
                        (false, Ok(walls)) => walls,
                        (false, Err(e)) => {
                            self.status = e;
                            return Task::none();
                        }
                    };
                    if let Some(o) = self.operation_mut() {
                        let selected = o.surfaces.get_or_insert_with(|| surface::Selection {
                            mesh_fingerprint: fingerprint.clone(),
                            ids: vec![],
                        });
                        if selected.mesh_fingerprint != fingerprint {
                            selected.mesh_fingerprint = fingerprint;
                            selected.ids.clear();
                        }
                        match drop {
                            Some((drop, keep)) => selected
                                .ids
                                .retain(|id| !drop.contains(id) || keep.contains(id)),
                            None => {
                                selected.ids.retain(|id| !walls.contains(id));
                                selected.ids.extend(walls);
                            }
                        }
                        o.geometry.clear();
                    }
                    self.touch();
                    if !said.is_empty() {
                        self.status = said;
                    }
                    // A hole a Drill still drills after its walls were dropped:
                    // a selected rim face, or another of its walls, holds it.
                    if drill
                        && dropping
                        && let Some(r) = round.first().and_then(|r| r.as_ref())
                        && let Some(mesh) = self.imported_mesh.take()
                    {
                        // At several A angles, the same hole at the angle
                        // it was read at (`picking_pose`).
                        let at = (self.operation())
                            .filter(|op| indexed::holes_at_angles(op, self.doc.setup()))
                            .and(setup.turn.map(|t| t.a));
                        let held = (self.operation())
                            .and_then(|op| self.drill_holes(op, &mesh).as_ref().clone().ok())
                            .is_some_and(|holes| {
                                holes.iter().any(|h| {
                                    h.a == at
                                        && (h.center[0] - r.center[0])
                                            .hypot(h.center[1] - r.center[1])
                                            <= 0.05
                                })
                            });
                        self.imported_mesh = Some(mesh);
                        if held {
                            self.status += " It is still drilled: a selected rim face (or another wall of it) holds it. Remove that face to drop it.";
                        }
                    }
                    return Task::none();
                }
                // Parallel and Waterline finishing take a whole face, curved or
                // flat: every surface of the smooth face clicked, added or
                // removed together.
                let face = if self.operation().is_some_and(picks_faces) {
                    mesh.region_of(n)
                } else {
                    vec![n]
                };
                if let Some(o) = self.operation_mut() {
                    let selected = o.surfaces.get_or_insert_with(|| surface::Selection {
                        mesh_fingerprint: fingerprint.clone(),
                        ids: vec![],
                    });
                    if selected.mesh_fingerprint != fingerprint {
                        selected.mesh_fingerprint = fingerprint;
                        selected.ids.clear();
                    }
                    let had = selected.ids.contains(&n);
                    // A Box takes one face: a click replaces the one before.
                    if o.strategy == Strategy::Box {
                        selected.ids.clear();
                    } else {
                        selected.ids.retain(|id| !face.contains(id));
                    }
                    if !had {
                        selected.ids.extend(face);
                    }
                    o.geometry.clear();
                }
                self.touch();
            }
            Message::DropHole(i) => {
                let Some(mesh) = self.imported_mesh.clone() else {
                    return Task::none();
                };
                let Some(holes) = self.operation().map(|o| self.pocket_holes(o, &mesh)) else {
                    return Task::none();
                };
                let Some(Some(hole)) = holes.as_ref().as_ref().ok().map(|h| h.get(i)) else {
                    return Task::none();
                };
                let ((x, y), (w, d)) = (hole.centre(), hole.size());
                let said = format!(
                    "Dropped the hole at X{x:.2} Y{y:.2}{} ({w:.2} × {d:.2} mm).",
                    indexed::at_angle(hole.a)
                );
                let drop: std::collections::HashSet<usize> = hole.walls.iter().copied().collect();
                let keep: std::collections::HashSet<usize> =
                    (holes.as_ref().as_ref().unwrap().iter())
                        .enumerate()
                        .filter(|(j, _)| *j != i)
                        .flat_map(|(_, h)| h.walls.iter().copied())
                        .collect();
                if let Some(o) = self.operation_mut()
                    && let Some(selected) = &mut o.surfaces
                {
                    selected
                        .ids
                        .retain(|id| !drop.contains(id) || keep.contains(id));
                }
                self.touch();
                self.status = said;
            }
            Message::ClearSurfaces => {
                if let Some(o) = self.operation_mut() {
                    o.surfaces = None;
                }
                self.touch();
            }
            Message::PickPinHoles => {
                self.picking_pins = !self.picking_pins;
                self.picking = false;
                self.placing_pins = false;
            }
            Message::PlacePins => {
                self.placing_pins = !self.placing_pins;
                self.picking = false;
                self.picking_pins = false;
                self.pin_aim = None;
            }
            Message::ThreadFromPart => {
                let Some(mesh) = self.part_mesh() else {
                    return Task::none();
                };
                match lathe::thread_from_part(self.doc.setup(), mesh) {
                    Some(fields) => {
                        if let Some(o) = self.operation_mut() {
                            for (k, v) in fields {
                                o.p.insert(k.into(), v);
                            }
                        }
                        self.touch();
                        self.status = "Filled the thread from the part.".into();
                    }
                    None => {
                        self.status =
                            "The part has no thread on the lathe's axis to fill in.".into()
                    }
                }
            }
            Message::AddThreading => {
                let si = self.doc.index();
                let Some(mesh) = self.part_mesh() else {
                    return Task::none();
                };
                match threading::add(&self.doc, si, mesh) {
                    Ok(added) => {
                        self.doc.setups[si].operations.extend(added.ops);
                        self.touch();
                        self.status = added.status;
                    }
                    Err(why) => self.status = why,
                }
            }
            Message::AddPinDrilling => {
                let si = self.doc.index();
                let Some(mesh) = self.part_mesh() else {
                    return Task::none();
                };
                let Some(to) = post::pin_drilling_setup(&self.doc, mesh, si) else {
                    self.status = "No earlier setup has the face the pins stand on facing up: add one before this setup to drill the pin holes in.".into();
                    return Task::none();
                };
                let setup = &self.doc.setups[to];
                let pins = control::pins_settings(&setup.machine.p).ok().flatten();
                let tool = (pins.as_ref()).and_then(|pins| {
                    post::pin_drilling_tool(&self.doc.tools, &setup.machine, pins)
                });
                let mut op = Operation::new(
                    Strategy::Drill,
                    tool.map(|t| t.id.clone()).unwrap_or_default(),
                );
                op.p.insert("name".into(), "Pin holes".into());
                op.p.insert("holes".into(), model::DRILL_HOLES[1].into());
                op.p.insert("pin_setup".into(), self.doc.setup().id.clone());
                if tool.is_some_and(|t| get(&t.p, "type") == "Flat end mill") {
                    op.p.insert("rpm".into(), "8000".into());
                    op.p.insert("feed".into(), "400".into());
                }
                op.p.insert(
                    "coolant".into(),
                    control::default_coolant(&setup.machine).into(),
                );
                op.geometry.clear();
                let named = setup.name().to_string();
                self.status = match (tool, pins) {
                    (Some(t), _) => format!("Added Pin holes to {named}, with {}.", t.name()),
                    (None, Some(pins)) => format!(
                        "Added Pin holes to {named}: choose its tool, a Ø{} to Ø{} drill or a flat end mill from Ø{} to Ø{}.",
                        post::exact_mm(pins.diameter as f32),
                        post::exact_mm(pins.diameter as f32 + post::PIN_FIT),
                        post::exact_mm((pins.diameter as f32 + post::PIN_CLEARANCE) / 2. + 0.05),
                        post::exact_mm(pins.diameter as f32 + post::PIN_CLEARANCE - 0.05)
                    ),
                    (None, None) => format!("Added Pin holes to {named}: choose its tool."),
                };
                let status = std::mem::take(&mut self.status);
                self.doc.setups[to].operations.push(op);
                self.touch();
                self.status = status;
            }
            Message::PinHoleSurface(n) => {
                let Some(mesh) = self.part_mesh().filter(|_| self.picking_pin_holes()) else {
                    return Task::none();
                };
                let setup = self.doc.setup();
                let found = self.bottom_holes(mesh);
                let Ok(holes) = &*found else {
                    return Task::none();
                };
                let Some(i) = cam::surface_hole(setup, mesh, holes, n) else {
                    self.status = "That is not in a round hole the part opens on its bottom in this setup: click inside a hole, on its wall or its end.".into();
                    return Task::none();
                };
                let text = post::pin_holes_toggled(setup, mesh, &holes[i]);
                return self.update(Message::Edit(Target::Setup, "pin_holes", text));
            }
            Message::PickMode => {
                self.picking = !self.picking && self.can_pick();
                if self.picking {
                    self.playing = false;
                    self.simulation = None;
                    self.progress = 0.;
                }
                // A Box is picked on its own tab.
                if self.operation().is_none_or(|o| o.strategy != Strategy::Box) {
                    self.tab = "Geometry".into();
                }
            }
            Message::BoxFromFace => {
                let found = (|| {
                    let mesh = self.part_mesh().ok_or("Load the part first.")?;
                    let selection = self
                        .operation()
                        .filter(|o| o.strategy == Strategy::Box)
                        .ok_or("Select a Box operation first.")?
                        .surfaces
                        .as_ref()
                        .ok_or(manual::ONE_FACE)?;
                    let stock = self
                        .known_stock(self.doc.index())
                        .0
                        .ok_or("The stock is not known yet.")?;
                    manual::box_from_face(self.doc.setup(), mesh, selection, stock)
                })();
                match found {
                    Ok(block) => {
                        if let Some(o) = self.operation_mut() {
                            for (k, v) in model::BOX_FIELDS.into_iter().zip(block) {
                                o.p.insert(k.into(), v.to_string());
                            }
                        }
                        self.touch();
                        self.status = "Box filled from the selected face.".into();
                    }
                    Err(error) => self.status = error,
                }
            }
            Message::Generate => {
                let mut issues = self.doc.setup().issues();
                issues.extend(self.op_issues());
                if !issues.is_empty() {
                    self.status = issues[0].clone();
                } else if !self.is_sample() {
                    self.playing = false;
                    self.simulation = None;
                    let (Some(mesh), Some(oi)) = (self.imported_mesh.clone(), self.selected) else {
                        self.status = if self.imported_mesh.is_none() {
                            "Load the part first.".into()
                        } else {
                            "Select an operation first.".into()
                        };
                        return Task::none();
                    };
                    let (doc, si) = (self.doc.clone(), self.doc.index());
                    self.status = format!(
                        "Generating {}…",
                        self.operation().map_or("the operation", |o| o.name())
                    );
                    return self.generate(
                        Generation::Operation(si, oi),
                        move || {
                            Made(std::sync::Arc::new(simulation::prepare(
                                &doc, &mesh, si, oi,
                            )))
                        },
                        Message::Generated,
                    );
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
            Message::Generated(epoch, made) => {
                let Some(g) = self.finished(epoch) else {
                    return Task::none();
                };
                // Made for the operation picked when it began.
                if g.what
                    != Generation::Operation(self.doc.index(), self.selected.unwrap_or(usize::MAX))
                {
                    return Task::none();
                }
                match (*made.0).clone() {
                    Ok(sim) => {
                        let plan = &sim.plan;
                        // A hand machine has no program to export, and a lathe
                        // setup's cut is checked against its chuck.
                        let next = if manual::is_hand(&self.doc.setup().machine) {
                            HAND_NEXT
                        } else {
                            CNC_NEXT
                        };
                        let fixtures = if ui::chuck_modelled(self) {
                            CHUCK_CHECKED
                        } else {
                            ui::fixtures(ui::Note::Preview, ui::modelled(self))
                        };
                        self.status = format!(
                            "{} preview ready: {} levels, {:.1} min estimated. Inherits stock after {} earlier operations across setups. Sampled removal; {} {next}",
                            plan.strategy.name(),
                            plan.levels,
                            plan.duration / 60.,
                            sim.preceding,
                            fixtures
                        );
                        self.simulation = Some(sim);
                        self.progress = 0.;
                        self.speed = 30.;
                        self.paths = true;
                        self.stock = true;
                        self.picking = false;
                        if g.play {
                            self.playing = true;
                            self.last_tick = Instant::now();
                        }
                        return self.measure_tooth_load();
                    }
                    Err(error) => self.status = error,
                }
            }
            Message::GeneratedAll(epoch, report) => {
                if self.finished(epoch).is_none() {
                    return Task::none();
                }
                self.status = format!(
                    "Checked {} enabled operations: {} generated, {} need attention. Remaining stock is carried through operations and setups.",
                    report.len(),
                    report.iter().filter(|r| r.ok).count(),
                    report.iter().filter(|r| !r.ok).count()
                );
                self.audit_report = report;
                self.page = Page::Audit;
            }
            Message::GeneratedLayout(epoch, laid) => {
                let Some(g) = self.finished(epoch) else {
                    return Task::none();
                };
                if let Generation::Layout(si) = g.what {
                    let (generated, bands) = &*laid.0;
                    self.place_layout(si, generated, bands.as_ref());
                }
            }
            Message::GenerationFailed(epoch, error) => {
                if self.finished(epoch).is_some() {
                    self.status = error;
                }
            }
            Message::CancelGenerate => {
                if self.generating.is_some() {
                    self.cancel_generation();
                    self.status = progress::CANCELLED.into();
                }
            }
            Message::GenerateAll => {
                if !self.is_sample() {
                    self.audit_report.clear();
                    let Some(mesh) = self.imported_mesh.clone() else {
                        self.status = "Load the imported part first.".into();
                        self.page = Page::Audit;
                        return Task::none();
                    };
                    let doc = self.doc.clone();
                    self.status = "Generating every enabled operation…".into();
                    return self.generate(
                        Generation::All,
                        move || {
                            let mut report = vec![];
                            simulation::sequence(&doc, &mesh, None, |si, oi, result| {
                                let setup = &doc.setups[si];
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
                                report.push(AuditRow {
                                    setup: si,
                                    operation: oi,
                                    name: format!("{} / {}", setup.name(), op.name()),
                                    ok,
                                    detail,
                                });
                            });
                            report
                        },
                        Message::GeneratedAll,
                    );
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
                } else if let [machine] = &self.doc.machines[..] {
                    self.create_setup(machine.clone());
                } else if self.doc.machines.is_empty() {
                    let task = self.update(Message::NewMachine);
                    self.pending = Some((Page::Wizard, Message::AddSetup));
                    self.status = "Set up your machine: saving it creates the first setup.".into();
                    return task;
                } else {
                    self.page = Page::Machines;
                    self.status = "Choose the machine for the first setup.".into();
                }
            }
            Message::AddManualSetup => match self.doc.add_manual_setup() {
                Ok(()) => {
                    self.stop_selection();
                    self.touch();
                    self.status = "Manual setup added before the CNC setups.".into();
                }
                Err(e) => self.status = e,
            },
            Message::AddLatheSetup => match self.doc.add_lathe_setup() {
                Ok(()) => {
                    self.stop_selection();
                    self.touch();
                    self.status = "Lathe setup added before the CNC setups.".into();
                }
                Err(e) => self.status = e,
            },
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
                        Ok(rechained) => {
                            self.stop_selection();
                            self.touch();
                            if let Some(line) = rechained {
                                self.status = line;
                            }
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
                let angle = if axis == "ry" { "90" } else { "180" };
                let i = self.doc.index();
                if i == 0 {
                    let turned = self.doc.rotate_first(axis, angle.into());
                    self.say_turned_with_first(&turned);
                } else {
                    // A later setup turns the stock, as setup 1 has it.
                    let face = if axis == "ry" { 5 } else { 1 };
                    let turn = crate::volume::turns()
                        .find(|&(f, q, _)| (f, q) == (face, 0))
                        .map(|(_, _, t)| t)
                        .expect("every face is a turn");
                    self.doc.turn_setup(i, turn);
                }
                indexed::set_reposition(&mut self.doc.setup_mut().p, "Reclamp and probe origin");
                self.touch();
            }
            Message::TurnStock(face, table) => {
                if let Some((_, _, turn)) =
                    crate::volume::turns().find(|&(f, q, _)| (f, q) == (face, table))
                {
                    self.doc.turn_setup(self.doc.index(), turn);
                    self.touch();
                }
            }
            Message::SquareToStock => {
                let i = self.doc.index();
                if i > 0 {
                    let turn =
                        crate::volume::nearest_turn(&self.doc.setups[0], &self.doc.setups[i]);
                    self.doc.turn_setup(i, turn);
                    let (face, table) = crate::volume::face_and_table(turn);
                    self.status = format!(
                        "{} now has the stock's {} up, turned {} on the table, square to the stock {} defined.",
                        self.doc.setups[i].name(),
                        crate::volume::FACES[face].0,
                        crate::volume::TABLE_TURNS[table],
                        self.doc.setups[0].name()
                    );
                    self.touch();
                }
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
            Message::AddRotaryMachine => {
                let dir = self
                    .state_path
                    .parent()
                    .unwrap_or(std::path::Path::new("."));
                match control::rotary_machine(dir) {
                    Ok(m) => {
                        self.status = format!(
                            "{} is in the Machine library, with an assumed rotary unit still to be measured.",
                            m.name()
                        );
                        self.doc.machines.push(m);
                        self.touch();
                    }
                    Err(e) => {
                        self.status = format!("Could not write the rotary UC100 profile: {e}.")
                    }
                }
            }
            Message::EditMachine(id) => {
                if let Some(m) = self.doc.machines.iter().find(|m| m.id == id) {
                    self.machine_draft = m.clone();
                    self.machine_draft.fill_post_defaults();
                    self.machine_draft.fill_rotary_defaults();
                    self.wizard_step = 0;
                    self.page = Page::Wizard;
                }
            }
            Message::WizardStep(s) => {
                self.wizard_step = s.min(ui::wizard_steps(&self.machine_draft).len() - 1)
            }
            Message::BrowseUc100Profile => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Choose the UC100 profile")
                            .add_filter("UC100 profile", &["toml"])
                            .pick_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::Uc100ProfilePicked,
                );
            }
            Message::Uc100ProfilePicked(path) => {
                if let Some(path) = path {
                    return self.update(Message::Edit(
                        Target::Machine,
                        "uc100_profile",
                        path.display().to_string(),
                    ));
                }
            }
            Message::ControlMachine(id) => {
                if let Some(s) = (self.doc.active_setup()).filter(|s| manual::is_hand(&s.machine)) {
                    self.cnc_over_guide = Some(s.id.clone());
                }
                match &self.control {
                    None => self.control_machine = Some(id),
                    Some(c) => {
                        self.switch_machine = (c.machine_id != id).then_some(id);
                    }
                }
                if self.machine_setup {
                    self.open_profile(false);
                }
            }
            Message::SwitchMachine(yes) => {
                let Some(id) = self.switch_machine.take() else {
                    return Task::none();
                };
                if !yes {
                    self.run_on_connect = None;
                } else if self.doc.machines.iter().any(|m| m.id == id) {
                    self.disconnect_machine();
                    self.control_machine = Some(id);
                    return self.update(Message::Connect);
                }
            }
            Message::ShowGuide => self.cnc_over_guide = None,
            Message::MachineConnection(sim) => {
                let Some(id) = self.control_target().map(|m| m.id.clone()) else {
                    return Task::none();
                };
                let value = if sim { "Simulated" } else { "USB" };
                // Not a planning setting: the setups' snapshots keep their
                // version. A machine being edited takes the choice too.
                for m in (self.doc.machines.iter_mut())
                    .chain(std::iter::once(&mut self.machine_draft))
                    .filter(|m| m.id == id)
                {
                    m.p.insert("uc100_connection".into(), value.into());
                }
                self.dirty = true;
                self.project_dirty = true;
                self.last_edit = Instant::now();
                if let Some(c) = &self.control
                    && c.machine_id == id
                    && c.simulated != sim
                {
                    self.disconnect_machine();
                    return self.update(Message::Connect);
                }
            }
            Message::ControlSetup(on) => {
                self.machine_setup = on;
                if on {
                    self.open_profile(false);
                }
            }
            Message::Setup(setup::Msg::Edit(e)) => {
                if let Some((_, Ok(d))) = &mut self.profile_draft {
                    d.edit(e);
                }
            }
            Message::Setup(setup::Msg::Revert) => self.open_profile(true),
            Message::Setup(setup::Msg::UseCalibration) => {
                if let (Some(c), Some((id, Ok(d)))) = (&self.control, &mut self.profile_draft)
                    && c.machine_id == *id
                    && let Some(Ok(v)) = c.calibrated()
                {
                    let axis = ["x", "y", "z", "a", "b", "c"][c.calibration.axis];
                    d.edit(setup::Edit::Number(
                        format!("axes.{axis}.steps_per_unit"),
                        setup::steps_text(v),
                    ));
                }
            }
            Message::Setup(setup::Msg::Save) => return self.save_profile(),
            Message::Connect => {
                if self.control.is_some() {
                    return Task::none();
                }
                let Some(m) = self.control_target().cloned() else {
                    self.status = "Add a UC100 machine in the Machine library first.".into();
                    return Task::none();
                };
                let dir = self.state_path.parent().unwrap().join("uc100");
                match control::Connection::open(&m, &dir) {
                    Ok(c) => {
                        self.status = format!("Connecting to {}…", m.name());
                        self.control = Some(c);
                        if let Some(si) = self.run_on_connect.take().and_then(|id| {
                            (self.doc.setups.iter())
                                .position(|s| s.id == id && s.machine.id == m.id)
                        }) {
                            return self.update(Message::Run(run::Action::Setup(si)));
                        }
                    }
                    Err(e) => self.status = format!("Cannot connect to {}: {e}", m.name()),
                }
            }
            Message::Disconnect => self.disconnect_machine(),
            Message::MachineReset => {
                if let Some(c) = &self.control {
                    c.send(uc100::controller::Command::Reset);
                }
            }
            Message::MachineEStop => {
                if let Some(c) = &self.control {
                    c.send(uc100::controller::Command::EStop);
                }
            }
            Message::Machine(action) => {
                if let Some(c) = &mut self.control {
                    c.act(action);
                }
            }
            // Keys jog only while the Control page shows the jog controls,
            // not in its Controller configuration view, and not while a
            // text field has the focus: iced's text field keeps the left and
            // right arrows but lets up, down and Page Up/Down through.
            Message::JogKey(axis, negative, repeat) => {
                // A held key is kept going by the tick; a step is one per press.
                if repeat
                    || self.page != Page::Control
                    || self.machine_setup
                    || self.control.is_none()
                {
                    return Task::none();
                }
                self.jog_key = Some((axis, negative));
                return iced::advanced::widget::operate(control::typing())
                    .map(move |typing| Message::JogKeyChecked(axis, negative, typing));
            }
            Message::JogKeyChecked(axis, negative, typing) => {
                // The key must still be down: a release before the answer
                // must not start a jog that nothing would stop.
                if !typing
                    && self.jog_key == Some((axis, negative))
                    && self.page == Page::Control
                    && !self.machine_setup
                    && let Some(c) = &mut self.control
                {
                    c.act(control::Action::JogPress(axis, negative));
                }
            }
            // Fields in layout order, such as the stock offsets, arriving with
            // the value selected so typing replaces it. With nothing focused,
            // the form's first field, except while the keys jog the machine:
            // a Tab there must not give a field the jog keys.
            Message::TabKey(back) => {
                let jogging =
                    self.page == Page::Control && !self.machine_setup && self.control.is_some();
                return iced::advanced::widget::operate(control::typing()).then(move |typing| {
                    let step = match tab_move(typing, back, jogging) {
                        TabMove::Stay => return Task::none(),
                        TabMove::Next => iced::widget::operation::focus_next(),
                        TabMove::Previous => iced::widget::operation::focus_previous(),
                        TabMove::First => {
                            iced::advanced::widget::operate(control::focus_first()).discard()
                        }
                    };
                    step.chain(iced::advanced::widget::operate(control::select_focused()).discard())
                });
            }
            Message::HelpKey => {
                return iced::advanced::widget::operate(help::focused_field())
                    .map(Message::HelpFor);
            }
            // F1 on the pinned field puts its help away; on another, switches.
            Message::HelpFor(tag) => {
                self.pinned_help = match tag {
                    Some(tag) if self.pinned_help.as_ref() != Some(&tag) => {
                        self.pinned_expanded = true;
                        self.pinned_paused = false;
                        Some(tag)
                    }
                    _ => None,
                }
            }
            // E-stop from any page, even while typing, while a machine is
            // connected. Not connected, Esc is free off the Control page, which
            // keeps it for the E-stop.
            Message::EscapeKey => {
                if let Some(c) = &self.control {
                    c.send(uc100::controller::Command::EStop);
                } else if self.esc_closes_pinned_help() {
                    self.pinned_help = None;
                }
            }
            Message::JogKeyUp => {
                if self.jog_key.take().is_some()
                    && let Some(c) = &mut self.control
                {
                    c.act(control::Action::JogRelease);
                }
            }
            // Space holds a job or move from any page, and resumes from that
            // hold only on the Control page with its run panel on screen, not
            // in the Controller configuration view, so a Space meant for
            // something else cannot start the machine.
            Message::HoldKey => {
                use uc100::controller::{Command, State};
                if let Some(c) = &self.control {
                    match c.snapshot.state {
                        Some(State::Run) => c.send(Command::FeedHold),
                        Some(State::Hold) if self.page == Page::Control && !self.machine_setup => {
                            c.send(Command::Resume)
                        }
                        _ => {}
                    }
                }
            }
            Message::Run(run::Action::Setup(si)) => {
                let Some(c) = &mut self.control else {
                    return Task::none();
                };
                // Not while a tool run or a probe is under way: the probe
                // sets the work zero of the setup it started with (#165).
                if si >= self.doc.setups.len()
                    || c.setup_locked()
                    || manual::is_hand(&self.doc.setups[si].machine)
                {
                    return Task::none();
                }
                self.run_epoch += 1;
                c.run = Some(run::Run::new(&self.doc, si, self.run_epoch));
                // A spot picked to probe from was for the setup before.
                c.probe_spot = None;
                // The setup picked to run is the one the 3D views show.
                if self.doc.setups[si].id != self.doc.active {
                    self.doc.active = self.doc.setups[si].id.clone();
                    self.stop_selection();
                    self.dirty = true;
                    self.last_edit = Instant::now();
                }
                if self.check_waits() {
                    return Task::none();
                }
                return self.check_run();
            }
            Message::RunOnMachine(si) => {
                let Some(s) = self.doc.setups.get(si) else {
                    return Task::none();
                };
                let (setup, machine) = (s.id.clone(), s.machine.id.clone());
                let page = self.update(Message::Page(Page::Control));
                return match &self.control {
                    None => {
                        self.control_machine = Some(machine);
                        self.run_on_connect = Some(setup);
                        page
                    }
                    Some(c) if c.machine_id == machine => {
                        Task::batch([page, self.update(Message::Run(run::Action::Setup(si)))])
                    }
                    // Connected to another machine: the page asks whether to
                    // switch, and the setup is picked after connecting.
                    Some(_) => {
                        self.switch_machine = Some(machine);
                        self.run_on_connect = Some(setup);
                        page
                    }
                };
            }
            Message::Run(run::Action::Check) => return self.recheck_run(),
            Message::Run(run::Action::CoolantOff) => {
                let Some((c, r)) = self
                    .control
                    .as_ref()
                    .and_then(|c| Some((c, c.run.as_ref()?)))
                else {
                    return Task::none();
                };
                let (si, ops) = (r.setup, r.coolant_off(c, &self.doc));
                if ops.is_empty() || r.running.is_some() {
                    return Task::none();
                }
                let mut names = vec![];
                for i in ops {
                    let o = &mut self.doc.setups[si].operations[i];
                    o.p.insert("coolant".into(), "Off".into());
                    names.push(o.name().to_string());
                }
                self.touch();
                self.status = format!("Coolant set to Off: {}.", names.join(", "));
                return self.recheck_run();
            }
            Message::Run(action) => {
                if matches!(action, run::Action::CycleStart) && self.planning_busy().is_some() {
                    return Task::none();
                }
                if let Some(c) = &mut self.control {
                    c.run_act(action, &self.doc);
                }
            }
            Message::RunChecked(epoch, programs) => {
                // Timed here: the smoke run sends it, and runs no tasks.
                let times =
                    (self.control.as_ref()).and_then(|c| run::Times::checked(&programs, &c.config));
                return self.update(Message::RunCheckedStock(
                    epoch,
                    programs,
                    Default::default(),
                    times,
                ));
            }
            Message::RunCheckedStock(epoch, programs, left, times) => {
                if let Some(c) = self.control.as_mut()
                    && let Some(r) = c.run.as_mut()
                    && r.epoch == epoch
                {
                    // A check without stock (the smoke run's) is not pictured.
                    let left = (!left.after.is_empty() || left.last.is_some()).then_some(left);
                    r.checked_as(programs, left, times);
                    c.fit_sim_block();
                    return self.picture_run();
                }
            }
            Message::RunPictures(epoch, pictures) => {
                self.picture_job = None;
                if let Some(r) = self.control.as_mut().and_then(|c| c.run.as_mut())
                    && r.epoch == epoch
                {
                    r.pictures = pictures;
                }
                // A check answered meanwhile is pictured next.
                return self.picture_run();
            }
            Message::SaveMachine => {
                if manual::is_hand(&self.machine_draft) {
                    manual::settle(&mut self.machine_draft);
                }
                let errors = self.machine_draft.issues();
                if errors.is_empty() {
                    let mut m = self.machine_draft.clone();
                    // A centreline typed over the probed one is no longer
                    // the measurement.
                    let saved = self.doc.machines.iter().find(|x| x.id == m.id);
                    if saved.is_some_and(|x| {
                        ["a_centre_y", "a_centre_z"]
                            .iter()
                            .any(|k| get(&x.p, k) != get(&m.p, k))
                    }) {
                        m.p.remove(control::CENTRE_MARK);
                        m.p.remove(model::CENTRE_REF);
                    }
                    if let Some(c) = self.control.as_mut().filter(|c| c.machine_id == m.id) {
                        c.probe = control::probe_settings(&m.p);
                        c.pins = control::pins_settings(&m.p);
                        c.rotary = control::Rotary::of(&m);
                    }
                    if let Some(i) = self.doc.machines.iter().position(|x| x.id == m.id) {
                        m.version += 1;
                        self.doc.machines[i] = m;
                    } else {
                        self.doc.machines.push(m);
                    }
                    self.page = Page::Machines;
                    self.touch();
                    self.status="Machine profile saved. Use Assign to current setup to update an existing setup snapshot.".into();
                    return self.resume();
                } else {
                    self.status = errors.join(" ");
                }
            }
            Message::ToolMachine(id, on) => {
                let p = &mut self.tool_draft.p;
                let list = manual::toggle(get(p, "machines"), &id, on);
                p.insert("machines".into(), list);
            }
            Message::HolderMachine(id, on) => {
                let p = &mut self.holder_draft.p;
                let list = manual::toggle(get(p, "machines"), &id, on);
                p.insert("machines".into(), list);
            }
            Message::DeleteMachine(id) => {
                if self.control.as_ref().is_some_and(|c| c.machine_id == id) {
                    self.status = "Disconnect before removing the connected machine.".into();
                    return Task::none();
                }
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
                    self.status = "Tool saved to the local library.".into();
                    return self.resume();
                } else {
                    self.status = issues.join(" ")
                }
            }
            Message::DeleteTool(id) => {
                if let Some(note) = self.doc.tool_in_use_note(&id) {
                    self.status = format!("This tool is not deleted. {note}")
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
                    let said = self.doc.fit_speeds();
                    if !said.is_empty() {
                        self.status = said;
                    }
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
                    matches!(get(&s.p, "holding"), "Vise" | "Clamps" | "Chuck")
                        && get(&s.p, "holder") == id
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
                return self.suggest_layout();
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
            Message::SaveProject if self.project_path.is_some() => {
                if self.save_busy {
                    return Task::none();
                }
                let path = self.project_path.clone();
                return self.update(Message::Saved(path));
            }
            Message::SaveProject | Message::SaveProjectAs => {
                if self.save_busy || self.import_busy {
                    return Task::none();
                }
                self.save_busy = true;
                // Save as starts beside the project's file, under its name.
                let mut dialog = rfd::AsyncFileDialog::new()
                    .set_title("Save AXIS CAM project")
                    .add_filter("AXIS project", &["json"]);
                dialog = match &self.project_path {
                    Some(p) => {
                        let name = p.file_name().unwrap_or_default().to_string_lossy();
                        let dialog = dialog.set_file_name(name);
                        match p.parent() {
                            Some(folder) => dialog.set_directory(folder),
                            None => dialog,
                        }
                    }
                    None => dialog
                        .set_file_name(format!("{}.axis.json", sanitize_name(&self.doc.title))),
                };
                return Task::perform(
                    async move {
                        let file = dialog.save_file().await?;
                        Some(file.path().to_path_buf())
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
                            path: p.to_path_buf(),
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
                self.import_busy = false;
                // From the Start screen the import starts a new project, once
                // a file is chosen: until then the resumable one stays.
                if self.page == Page::Home {
                    let _ = self.update(Message::NewProject);
                    if self.page == Page::Home {
                        return Task::none();
                    }
                }
                self.doc.title = std::path::Path::new(&f.name)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                self.import_error = None;
                self.status = format!(
                    "{} geometry loaded. Facing and Pocket can select planar surfaces, Parallel finishing any face, curved ones included, and generate bounded previews.",
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
                        self.sync_turns();
                        self.undo = None;
                        self.invalidate_post();
                        self.status = "Saved model geometry restored.".into();
                        // The pin holes follow the pins as the part places them.
                        if self.doc.file.is_some()
                            && let Some(mesh) = &self.imported_mesh
                        {
                            post::sync_pin_drilling(&mut self.doc, mesh);
                        }
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
                // A copy from History is a recovery, not the project's file:
                // Save asks where to keep it.
                let history = self.state_path.with_file_name("history");
                self.replace_document(d, (!p.starts_with(&history)).then_some(p));
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
            Message::Saved(Some(p)) => {
                self.save_busy = false;
                // The document as it is now: edits made while the dialog was
                // open are in the file.
                match storage::save(&p, &self.doc) {
                    Ok(()) => {
                        self.status = format!("Saved {}", p.display());
                        self.project_dirty = false;
                        self.set_project_file(Some(p));
                    }
                    Err(e) => self.status = format!("Save failed: {e}"),
                }
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
            Message::ToggleUncut => self.show_uncut = !self.show_uncut,
            Message::Section(edit) => self.edit_section(edit),
            Message::TogglePaths => self.paths = !self.paths,
            Message::Camera(view) => {
                self.camera = match view {
                    // The scene fitted again, from where the view looks.
                    "FIT" => viewport::Camera {
                        zoom: 1.,
                        pan: [0., 0.],
                        ..self.camera
                    },
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
            Message::ToggleSidebar(side) => {
                let open = self.settings.layout.open_mut(side);
                *open = !*open;
                self.save_settings();
            }
            Message::SidebarWidth(side, width, done) => {
                let layout = &mut self.settings.layout;
                *layout.width_mut(side) = ui::clamped(layout, side, width, self.window_width);
                if done {
                    self.save_settings();
                }
            }
            Message::ToggleStrategyHelp => {
                let layout = &mut self.settings.layout;
                layout.strategy_help_open = !layout.strategy_help_open;
                self.save_settings();
            }
            Message::Resized(width) => self.window_width = width,
            Message::PlanViewport(e) => steer(&mut self.plan_camera, &e),
            Message::Viewport(e) => match e {
                viewport::Event::Orbit(..)
                | viewport::Event::Zoom(..)
                | viewport::Event::Pan(..) => steer(&mut self.camera, &e),
                viewport::Event::Pick(n) => {
                    if self.picking {
                        return self.update(Message::Feature(n));
                    }
                }
                viewport::Event::AimPins(at) => {
                    if self.placing_leftover_pins() {
                        self.pin_aim = at;
                    }
                }
                viewport::Event::PlacePins(at) => {
                    let si = self.doc.index();
                    if self.placing_leftover_pins() {
                        self.pin_aim = Some(at);
                    }
                    if let (true, Some(mesh), (Some(stock), _)) = (
                        self.placing_leftover_pins(),
                        self.part_mesh(),
                        self.known_stock(si),
                    ) {
                        // Where the pins do not fit they stay where they were.
                        let setup = self.doc.setup();
                        if let Ok(Some(pins)) = control::pins_settings(&setup.machine.p)
                            && let Some(issue) = post::leftover_issue(
                                setup,
                                mesh,
                                stock,
                                &pins,
                                &post::plate_places(&pins, at),
                            )
                        {
                            self.status = format!("The pins do not fit there: {issue}");
                            return Task::none();
                        }
                        let text = post::leftover_pin_text(setup, mesh, stock, at);
                        return self.update(Message::Edit(Target::Setup, "pin_place", text));
                    }
                }
                viewport::Event::SectionSurface(n) => {
                    if self.picking_section
                        && let Some(mesh) = self.part_mesh()
                    {
                        let setup = self.doc.active_setup();
                        let cut = section::Section::picked(n, mesh, setup);
                        self.section = Some((cut, mesh.fingerprint.clone()));
                        self.section_on = true;
                        self.picking_section = false;
                    }
                }
                viewport::Event::PickSurface(n) => {
                    if self.picking_pin_holes() {
                        return self.update(Message::PinHoleSurface(n));
                    }
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
                    let task = self.update(Message::Generate);
                    // Off the window's thread: playback starts when it is done.
                    if let Some(g) = &mut self.generating {
                        g.play = true;
                        return task;
                    }
                }
                if self.playback_ready() {
                    // A scrub still to be replayed is, first: playback starts
                    // when it is done.
                    if self.scrub_pending && !self.playing {
                        return self.catch_up(true);
                    }
                    // One replaying off the window's thread: playback starts
                    // when it is put back.
                    if let Some(catching) = &mut self.catching_up
                        && !self.playing
                    {
                        // Again: not when it is put back after all.
                        catching.2 = !catching.2;
                        return Task::none();
                    }
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
                self.last_scrub = Instant::now();
                // A catch-up under way replays to where the slider was.
                self.catching_up = None;
                // A scrub that would replay many moves waits for the slider
                // to be let go, so dragging it does not stall the window.
                if let Some(simulation) = &mut self.simulation {
                    if simulation.replay(self.progress) <= self.scrub_live {
                        simulation.seek(self.progress);
                        self.scrub_pending = false;
                    } else {
                        self.scrub_pending = true;
                    }
                }
            }
            Message::ScrubDone => return self.catch_up(false),
            Message::ReplayFailed(epoch) => {
                if self.catching_up.take_if(|c| c.0 == epoch).is_some() {
                    self.status =
                        "Replaying the toolpath to the slider failed: move the slider again."
                            .into();
                }
            }
            Message::PartHoles(key, found) => {
                if self.part_holes_job == Some(key) {
                    self.part_holes_job = None;
                }
                self.part_holes = Some((key, found.0));
            }
            Message::ToothLoaded(id, load) => {
                // Only into the simulation it was worked out for, still there.
                if self
                    .tooth_load_job
                    .as_ref()
                    .is_some_and(|(job, _)| *job == id)
                {
                    self.tooth_load_job = None;
                }
                if let Some(simulation) = self.simulation.as_mut().filter(|s| s.id == id) {
                    simulation.tooth_load = load;
                }
            }
            Message::Replayed(epoch, replayed) => {
                let Some((_, at, play)) = self.catching_up.take_if(|c| c.0 == epoch) else {
                    return Task::none();
                };
                // Only into the simulation it was replayed from, still there.
                if self
                    .simulation
                    .as_ref()
                    .is_some_and(|s| s.id == replayed.0.id)
                    && self.progress == at
                {
                    let mut replayed =
                        std::sync::Arc::try_unwrap(replayed.0).unwrap_or_else(|s| (*s).clone());
                    // The load on each tooth may have come while it replayed.
                    if let Some(current) = &self.simulation
                        && replayed.tooth_load.is_none()
                    {
                        replayed.tooth_load = current.tooth_load.clone();
                    }
                    self.simulation = Some(replayed);
                    if play {
                        self.playing = true;
                        self.last_tick = Instant::now();
                    }
                }
            }
            Message::Speed(v) => {
                if v.is_finite() {
                    self.speed = v.clamp(0.5, 120.);
                }
            }
            Message::StockDetail(detail) => {
                self.stock_detail = detail;
                // How long the last detail took says nothing of this one.
                self.stock_mesh_slow = false;
            }
            Message::StockMeshed {
                key,
                seconds,
                result,
            } => {
                if self.stock_mesh_job.as_ref() == Some(&key) {
                    self.stock_mesh_job = None;
                    // Only a job at the detail shown now says how slow the next is.
                    self.stock_mesh_slow = (self.stock_mesh_started.take())
                        .is_some_and(|t| t.elapsed() >= ui::STOCK_NOTICE_AFTER)
                        && key.3 == self.stock_detail;
                }
                if let Some(sim) = &mut self.simulation
                    && sim.id == key.0
                    && sim.stock_epoch == key.1
                    && self.stock_detail == key.3
                {
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
            Message::StartMeshed {
                key,
                volume,
                result,
            } => {
                if self.start_mesh_job == Some(key) {
                    self.start_mesh_job = None;
                }
                match result {
                    Ok(surface) => {
                        self.start_mesh.retain(|(k, ..)| *k != key);
                        if self.start_mesh.len() >= START_MESHES {
                            self.start_mesh.remove(0);
                        }
                        self.start_mesh.push((key, volume, surface));
                    }
                    Err(e) => {
                        self.status = e;
                        self.start_mesh_failed = Some(key);
                    }
                }
            }
            Message::HelpHover(on) => self.help_animating = on,
            Message::HelpMore(expanded) => self.pinned_expanded = expanded,
            Message::HelpPause(paused) => self.pinned_paused = paused,
            Message::Tick => {
                if let Some(c) = &mut self.control {
                    c.refresh();
                    // "Connecting to <machine>…" until the driver reports a
                    // state, then "Connected to <machine>: <state>" following
                    // it until another message replaces it.
                    let connecting = self
                        .status
                        .strip_prefix("Connecting to ")
                        .map(|name| name.trim_end_matches('…'));
                    let name = connecting.or_else(|| {
                        Some(
                            self.status
                                .strip_prefix("Connected to ")?
                                .rsplit_once(": ")?
                                .0,
                        )
                    });
                    if let (Some(name), Some(state)) = (name, &c.snapshot.state) {
                        use uc100::controller::State;
                        self.status = match state {
                            State::Disconnected if connecting.is_some() => format!(
                                "Could not connect to {name}; see the log on the Control page."
                            ),
                            State::Disconnected => format!(
                                "Lost the connection to {name}; see the log on the Control page."
                            ),
                            s => format!("Connected to {name}: {}", s.label()),
                        };
                    }
                }
                if let Some((id, at)) = (self.control.as_mut())
                    .and_then(|c| Some((c.machine_id.clone(), c.recorded.take()?)))
                {
                    self.record_pins(&id, at);
                }
                if let Some((id, keep)) = (self.control.as_mut())
                    .and_then(|c| Some((c.machine_id.clone(), c.rotary_keep.take()?)))
                {
                    self.keep_rotary(&id, keep);
                }
                if self.status == LOCKED && !self.locked() {
                    self.status.clear();
                }
                if self.smoke_dir.is_some() && self.stock_mesh_key().is_some() {
                    assert!(
                        self.stock_mesh_failed.as_ref() != self.stock_mesh_key().as_ref(),
                        "Smoke stock meshing failed: {}",
                        self.status
                    );
                    return self.refresh_stock_surface();
                }
                // The smoke run captures each screen from a frame drawn after
                // its step (`smoke_frame`); a step whose frame does not come
                // fails the run here.
                if self.smoke_dir.is_some() {
                    self.smoke_overdue();
                }
                let now = Instant::now();
                let dt = now.duration_since(self.last_tick).as_secs_f32().min(0.2);
                self.last_tick = now;
                self.playing &= !self.locked();
                // The simulation is being replayed to the slider off the
                // window's thread: playback waits for it.
                if self.playing && self.catching_up.is_none() {
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
                // A scrub the slider was not let go after (arrow keys, the
                // wheel) is replayed once it has been still a moment.
                let settled =
                    if self.scrub_pending && now.duration_since(self.last_scrub) > SCRUB_SETTLE {
                        self.catch_up(false)
                    } else {
                        Task::none()
                    };
                let paused = now.duration_since(self.last_edit) > EDIT_PAUSE;
                if self.dirty && paused {
                    self.persist()
                }
                // A close waiting for the sign-in is checked again, so that
                // it happens when the wait is over.
                let close = self.close_if_waiting();
                let left = if paused {
                    Task::batch([self.refresh_left_stock(), self.refresh_ready()])
                } else {
                    Task::none()
                };
                // The Run panel's checks follow the project: a setup changed
                // since it was checked is checked again once the edits pause,
                // and the machine stands still (#167).
                let stale = self.control.as_ref().and_then(|c| c.run.as_ref());
                let recheck = if paused
                    && self.page == Page::Control
                    && !self.locked()
                    && stale.is_some_and(|r| r.stale && r.running.is_none())
                {
                    self.recheck_run()
                } else {
                    Task::none()
                };
                return Task::batch([
                    close,
                    settled,
                    self.picture_run(),
                    self.refresh_stock_surface(),
                    self.refresh_start_mesh(),
                    self.refresh_post(),
                    left,
                    recheck,
                    self.refresh_guide(),
                ]);
            }
            Message::Plan => {
                // Try again after a failed change asks for the same change.
                let request = match &self.planner {
                    Planner::Failed(_) => self.plan_asked.clone().unwrap_or_default(),
                    _ => String::new(),
                };
                let turns = if request.is_empty() {
                    vec![]
                } else {
                    self.plan_chat.clone()
                };
                return self.start_plan(planner::Chat { turns, request });
            }
            Message::PlanAsk(text) => self.plan_request = text,
            Message::PlanSend => {
                let request = self.plan_request.trim().to_string();
                if request.is_empty() || matches!(self.planner, Planner::Running(_)) {
                    return Task::none();
                }
                let chat = planner::Chat {
                    turns: self.plan_chat.clone(),
                    request,
                };
                return self.start_plan(chat);
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
                if let Some(request) = self.plan_asked.clone() {
                    let answer = match &result {
                        Ok(preview) if preview.outcome.proposal.summary.is_empty() => {
                            "Here is the plan to review.".into()
                        }
                        Ok(preview) => preview.outcome.proposal.summary.clone(),
                        Err(failure) => failure.message.clone(),
                    };
                    self.plan_chat.push(planner::Turn { request, answer });
                }
                match result {
                    Ok(preview) => {
                        self.status = format!(
                            "{} ready to review: {}, {}.",
                            if ui::edits_plan(self) {
                                "Change"
                            } else {
                                "Plan"
                            },
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
                        // A failed change leaves the preview it started from;
                        // the chat says why.
                        if let Some(refined) = self.plan_refined.take() {
                            self.status = format!("{} The preview is as it was.", failure.message);
                            self.restore_refined(refined);
                        } else {
                            self.planner = Planner::Failed(failure);
                        }
                        // Only the client the run started with can be signed out by it.
                        if let (Some(error), true) = (api, same_client) {
                            self.account_error(error);
                        }
                    }
                }
            }
            Message::PlanCancel => {
                if matches!(self.planner, Planner::Running(_)) {
                    // The request goes back in the box, to send again.
                    if let Some(asked) = self.plan_asked.take()
                        && self.plan_request.trim().is_empty()
                    {
                        self.plan_request = asked;
                    }
                    let refined = self.plan_refined.take();
                    self.drop_plan();
                    self.status = if let Some(refined) = refined {
                        self.restore_refined(refined);
                        "Change cancelled. The preview is as it was."
                    } else {
                        "Planning cancelled. The job is unchanged."
                    }
                    .into();
                }
            }
            Message::PlanPicture(picture) => {
                self.plan_enlarged = (self.plan_enlarged != Some(picture)).then_some(picture);
            }
            Message::PlanExpand(text) => {
                if !self.plan_expanded.remove(&text) {
                    self.plan_expanded.insert(text);
                }
            }
            Message::DiscardPlan => {
                if matches!(self.planner, Planner::Preview(_) | Planner::Failed(_)) {
                    self.planner = Planner::Idle;
                    self.plan_timeline.clear();
                    self.plan_expanded.clear();
                    self.plan_asked = None;
                    self.plan_refined = None;
                }
            }
            Message::ApplyPlan => {
                let Planner::Preview(preview) = &self.planner else {
                    return Task::none();
                };
                // Undo removes setup 1's planner note with the rest of the plan.
                let job = planner::applied(&preview.outcome);
                // The previous job goes to History first, as on New project.
                // Applying is not a project switch: Recover previous keeps
                // pointing at the project before the last switch.
                if self.archive_copy().is_none() {
                    return Task::none();
                }
                self.undo = Some(self.doc.clone());
                self.planner = Planner::Idle;
                // A workholding entry the plan's setups use that was deleted
                // since the preview comes back, as Undo does it.
                let restored = self.doc.restore_holders(&job, &job.setups);
                self.doc.setups = job.setups;
                self.doc.active = job.active;
                self.stop_selection();
                self.page = Page::Work;
                self.touch();
                self.status = format!(
                    "Plan applied. The previous job is in History; Undo is in AI planner. Review each operation, then export from NC review.{restored}"
                );
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
                status += &self.doc.restore_holders(&before, &before.setups);
                self.doc.setups = before.setups;
                self.doc.active = before.active;
                self.stop_selection();
                self.touch();
                self.status = status;
            }
            Message::Close(id) => return self.request_close(id),
            Message::Exported(p) => self.status = format!("Exported {}", p.display()),
            Message::ExportError(e) => self.status = format!("Export failed: {e}"),
            Message::Guide(action) => return self.guide_action(action),
            Message::LeftStock(found) => {
                self.left_job = None;
                // A job edited while it ran gets a run of its own at the next pause.
                let (left, checks) = *found;
                if self.left_key() == Some(left.key) {
                    self.left_stock = left;
                }
                if self.holding_key() == Some(checks.key) {
                    self.holding_checks = checks;
                }
            }
            Message::Ready(ready) => {
                // A job edited while it was checked gets a check of its own at
                // the next pause (`current_ready`). A check of a project
                // replaced since is dropped: it is not this one's last.
                if self.ready_job == Some(ready.key) {
                    self.ready_job = None;
                    self.ready = *ready;
                }
            }
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
                    // The note about an unreadable settings file stays.
                    Err(e) if !self.account_note.is_empty() => {
                        self.account_note = format!("{} {e}", self.account_note)
                    }
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
                // A sign-in that was superseded stops waiting for its browser.
                if let Some(old) = self.sign_in.take() {
                    old.abort();
                }
                self.sign_in_attempt = self.sign_in_attempt.wrapping_add(1);
                let attempt = self.sign_in_attempt;
                let (endpoints, settings) = (self.endpoints.clone(), self.settings.clone());
                let open = self.open_browser.clone();
                // Only the wait for the browser is abortable: the exchange
                // that follows may already hold issued tokens.
                let (task, handle) = Task::perform(
                    async move {
                        let limit = Duration::from_secs(300);
                        oauth::approve(&endpoints, &settings, None, limit, |url| open(url)).await
                    },
                    move |result| Message::Approved(attempt, result),
                )
                .abortable();
                self.sign_in = Some(handle);
                return task;
            }
            // The browser step is done. The exchange is a step of its own
            // that cannot be aborted, so the tokens it gets are never
            // dropped: a cancel or sign-out during it only means that its
            // result arrives with nobody waiting and is revoked, below. The
            // handle stays until then, so Cancel still works.
            Message::Approved(attempt, Ok(approval))
                if attempt == self.sign_in_attempt
                    && self.account == Account::SigningIn
                    && self.sign_in.is_some() =>
            {
                let endpoints = self.endpoints.clone();
                return Task::perform(
                    async move {
                        let http = oauth::http_client(Duration::from_secs(60));
                        oauth::complete(&http, &endpoints, approval).await
                    },
                    move |result| Message::SignedIn(attempt, result),
                );
            }
            // An approval nobody is waiting for holds no token; it is dropped.
            Message::Approved(_, Ok(_)) => {}
            Message::Approved(attempt, Err(e)) => {
                if attempt == self.sign_in_attempt
                    && self.account == Account::SigningIn
                    && self.sign_in.is_some()
                {
                    self.sign_in = None;
                    self.account = Account::SignedOut;
                    self.account_note = e;
                }
            }
            Message::SignInCancel => {
                // Only the browser step and the exchange can be cancelled.
                // Once the sign-in is being saved there is no handle, and the
                // save finishes.
                let Some(handle) = self.sign_in.take() else {
                    return Task::none();
                };
                handle.abort();
                if self.account == Account::SigningIn {
                    self.account = Account::SignedOut;
                    self.account_note = "Sign-in cancelled.".into();
                }
            }
            // A sign-in nobody is waiting for any more (cancelled, signed out,
            // or superseded by a newer attempt), or a second result while the
            // first is being saved: its refresh token was never saved, so
            // revoke it instead of leaving it valid.
            Message::SignedIn(attempt, Ok((session, client_id)))
                if attempt != self.sign_in_attempt
                    || self.account != Account::SigningIn
                    || self.saving_sign_in() =>
            {
                let endpoints = self.endpoints.clone();
                return Task::perform(
                    async move {
                        oauth::revoke_unused(&endpoints, &client_id, &session.refresh_token).await
                    },
                    |()| Message::Noop,
                );
            }
            Message::SignedIn(attempt, Err(_))
                if attempt != self.sign_in_attempt
                    || self.account != Account::SigningIn
                    || self.saving_sign_in() => {}
            // The browser step is done. Saving is a step of its own that
            // cannot be cancelled, so a cancel can never land in the middle
            // of it; the account shows nothing until it reports.
            Message::SignedIn(_, Ok((session, client_id))) => {
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
            Message::SignedIn(_, Err(e)) => {
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
            // A run keeps the choice it started with; the box is disabled
            // while it goes, and so is this.
            Message::PlanPictures(on) => {
                if !matches!(self.planner, Planner::Running(_)) {
                    self.settings.plan_pictures_off = !on;
                    self.save_settings();
                }
            }
            Message::OpenUrl(url) => {
                if let Err(e) = oauth::open_browser(url) {
                    self.account_note = e;
                }
            }
        }
        Task::none()
    }
    /// Whether Esc puts the F1-pinned help away: with no machine connected
    /// and off the Control page. Otherwise Esc is kept for the E-stop, and
    /// the pinned card's hint says so (issue #185).
    pub fn esc_closes_pinned_help(&self) -> bool {
        self.control.is_none() && self.page != Page::Control
    }
    /// Whether the app ticks every 33 ms rather than every 500 ms: while a
    /// simulation plays, a hover card animates, a machine is connected, or
    /// the F1-pinned card shows an animation that is not paused (issue #186).
    pub fn ticks_fast(&self) -> bool {
        self.playing
            || self.generating.is_some()
            || self.scrub_pending
            || self.help_animating
            || self.control.is_some()
            || (self
                .pinned_help
                .as_ref()
                .is_some_and(|t| t.help().diagram.is_some())
                && !self.pinned_paused)
    }
    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            iced::time::every(if self.ticks_fast() {
                Duration::from_millis(33)
            } else {
                Duration::from_millis(500)
            })
            .map(|_| Message::Tick),
            iced::event::listen_with(on_event),
        ];
        if self.smoke_dir.is_some() {
            subscriptions.push(smoke::frames());
        }
        // The handwheel's beat on a pass, only while its page shows.
        if self.steps_page()
            && let Some((shown, at)) = self.guide_at()
            && matches!(shown.guide.steps[at], guide::Step::Pass { turns_per_minute, .. } if turns_per_minute > 0.)
        {
            subscriptions.push(
                iced::time::every(Duration::from_millis(100))
                    .map(|at| Message::Guide(GuideAction::Beat(at))),
            );
        }
        Subscription::batch(subscriptions)
    }
    fn view(&self) -> Element<'_, Message> {
        ui::view(self)
    }
}
/// The message a window event sends, if any: shortcuts, the E-stop, jog
/// and hold keys, Tab between fields, and closing the window.
fn on_event(
    event: iced::Event,
    status: iced::event::Status,
    window: iced::window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Window(iced::window::Event::CloseRequested) => Some(Message::Close(window)),
        iced::Event::Window(
            iced::window::Event::Resized(size) | iced::window::Event::Opened { size, .. },
        ) => Some(Message::Resized(size.width)),
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Character(c),
            modifiers,
            ..
        }) if modifiers.command() && modifiers.shift() && c.to_lowercase() == "s" => {
            Some(Message::SaveProjectAs)
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
        // Esc from any page, even while typing: the E-stop while connected.
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            ..
        }) => Some(Message::EscapeKey),
        // F1 pins the focused field's help, even while typing.
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::F1),
            ..
        }) => Some(Message::HelpKey),
        // Tab and Shift+Tab move between text fields, even from inside one.
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
            modifiers,
            ..
        }) => Some(Message::TabKey(modifiers.shift())),
        // A focused text field takes Space as a character.
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Space),
            repeat: false,
            ..
        }) if status == iced::event::Status::Ignored => Some(Message::HoldKey),
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(key),
            repeat,
            ..
        }) if status == iced::event::Status::Ignored => {
            jog_key(key).map(|(axis, negative)| Message::JogKey(axis, negative, repeat))
        }
        iced::Event::Keyboard(iced::keyboard::Event::KeyReleased {
            key: iced::keyboard::Key::Named(key),
            ..
        }) => jog_key(key).map(|_| Message::JogKeyUp),
        // A held jog stops on any left-button release, wherever it
        // happens, and when the window loses the focus.
        iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left))
        | iced::Event::Window(iced::window::Event::Unfocused) => {
            Some(Message::Machine(control::Action::JogRelease))
        }
        _ => None,
    }
}
/// What the Tab key does (`Message::TabKey`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TabMove {
    /// The next text field.
    Next,
    /// The previous text field (Shift+Tab).
    Previous,
    /// The form's first text field (`control::focus_first`).
    First,
    /// Nothing.
    Stay,
}
/// What Tab does when a text field is focused (`typing`), with Shift (`back`)
/// and while the keys jog the machine (`jogging`): between the fields while
/// one is focused; with none, into the form's first field, unless the keys
/// jog, which a field would take from them.
pub(crate) fn tab_move(typing: bool, back: bool, jogging: bool) -> TabMove {
    match (typing, back, jogging) {
        (true, true, _) => TabMove::Previous,
        (true, false, _) => TabMove::Next,
        (false, _, true) => TabMove::Stay,
        (false, _, false) => TabMove::First,
    }
}
/// The axis and direction a jog key moves: the arrows jog X and Y (up is
/// +Y, away from the operator), Page Up and Page Down jog Z, Home and End
/// jog A (on a machine without A they move nothing).
fn jog_key(key: iced::keyboard::key::Named) -> Option<(usize, bool)> {
    use iced::keyboard::key::Named;
    match key {
        Named::ArrowLeft => Some((0, true)),
        Named::ArrowRight => Some((0, false)),
        Named::ArrowUp => Some((1, false)),
        Named::ArrowDown => Some((1, true)),
        Named::PageUp => Some((2, false)),
        Named::PageDown => Some((2, true)),
        Named::Home => Some((3, false)),
        Named::End => Some((3, true)),
        _ => None,
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
        let split = check
            .split_line()
            .map_or(String::new(), |l| format!(" ({l})"));
        format!(
            "The plan passes every check: {}, {} min{split}.",
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
        if manual::is_hand(&doc.setups[setup.setup].machine) {
            println!(
                "{}",
                manual::no_gcode_on(&doc.setups[setup.setup].machine, &setup.name)
            );
            continue;
        }
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
    doc.starter_materials = user.starter_materials;
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
/// sign-out is pending. `blocking`: a plan stage, the NC review refresh or
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
    // The renderer smoke run imports the STEP fixture at screen 11; --help-only
    // stops before it.
    if args.get(1).map(String::as_str) == Some("--smoke-test")
        && !args.iter().any(|arg| arg == "--help-only")
        && !step::available()
    {
        eprintln!(
            "The renderer smoke run needs Open CASCADE for its STEP screens, and this build has no STEP helper (AXIS_SKIP_STEP). Rebuild with Open CASCADE, or run it with --help-only."
        );
        std::process::exit(2)
    }
    // The stock mesher's GPU takes a moment to open: open it now, not on the
    // first simulation's stock.
    std::thread::spawn(gpu_mesh::shared);
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
            size: WINDOW,
            min_size: Some(MIN_WINDOW),
            exit_on_close_request: false,
            ..Default::default()
        })
        .centered()
        .antialiasing(true)
        .default_font(iced::Font::with_name("Helvetica Neue"))
        .run()
}
