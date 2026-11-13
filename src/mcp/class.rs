//! What each `Message` is to an agent (LIO-470): reachable through a named
//! command, refused at the machine boundary, the app's own traffic, or the
//! AI planner's. Every `match` here names every variant and has no
//! wildcard arm, so a new message does not compile until it is classified.
//! The rules are the spec's *The machine boundary (allow-list)*.
use crate::{GuideAction, Message, Page, Target, control, run, section, setup, ui, viewport};

/// The class of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Reachable through the command of this name (`commands`).
    Command(&'static str),
    /// Refused: it moves the machine, energises it, changes what its
    /// controller holds, or records the machinist's progress at it.
    Machine,
    /// The app's own: results of background work, ticks, window events,
    /// keys, hovers and frames.
    Internal,
    /// The AI planner's, until it is removed (M4).
    Planner,
}

/// Why a `Machine` message is refused: the machine boundary.
pub const MACHINE: &str = "This moves the machine or changes what its controller holds, so it is done in the app by the user.";
/// Why the step guide's progress and the machinist's measurements are
/// refused: they are the record of what was done at the machine.
pub const SHOP_FLOOR: &str = "This records the machinist's progress at the machine (the step guide's place, a measurement or a dial reading), so it is done in the app by the user.";
/// Why a planner message is refused.
pub const PLANNER: &str =
    "The AI planner is not driven through MCP: plan with the commands, as the user does.";
/// Why an internal message is refused.
pub const INTERNAL: &str = "This is the app's own event, not something a user does.";

/// Why `m` is refused to an agent, or None when a command sends it.
pub fn refusal(m: &Message) -> Option<&'static str> {
    match class(m) {
        Class::Command(_) => None,
        Class::Machine if shop_floor(m) => Some(SHOP_FLOOR),
        Class::Machine => Some(MACHINE),
        Class::Planner => Some(PLANNER),
        Class::Internal => Some(INTERNAL),
    }
}

/// Whether `m` is the step guide's progress or a measurement.
fn shop_floor(m: &Message) -> bool {
    matches!(
        m,
        Message::Guide(
            GuideAction::Next
                | GuideAction::Back
                | GuideAction::Go(_)
                | GuideAction::Section(_)
                | GuideAction::Reached(_)
                | GuideAction::Resume
                | GuideAction::Open(_)
                | GuideAction::Measured(..)
                | GuideAction::Typed(..)
                | GuideAction::CrossReads(_)
        )
    )
}

/// Whether `key` is a machine field that reaches the controller: refused in
/// `edit_field` for any machine.
pub fn machine_key(key: &str) -> bool {
    matches!(key, "uc100_profile" | "uc100_connection") || key.starts_with("pi_")
}

/// The class of `m`.
pub fn class(m: &Message) -> Class {
    use Class::{Command as C, Internal, Machine, Planner};
    match m {
        Message::NewProject => C("new_project"),
        Message::ResumeProject => C("resume_project"),
        Message::OpenSample => C("open_sample"),
        Message::RecoverPrevious => C("recover_previous_project"),
        // The Control page enters the manual setups' steps, which moves the
        // guide's place; the agent reads the machine with machine_status.
        Message::Page(Page::Control) => Machine,
        Message::Page(Page::Ai) => Planner,
        Message::Page(
            Page::Home
            | Page::Work
            | Page::Machines
            | Page::Tools
            | Page::Materials
            | Page::Workholding
            | Page::Wizard
            | Page::Post
            | Page::Audit,
        ) => C("goto_page"),
        Message::SelectSetup(_) => C("select_setup"),
        // The workflow footer's steps select the operation, the first when
        // none is.
        Message::SelectOp(_) | Message::OperationsStep | Message::PreviewStep => {
            C("select_operation")
        }
        Message::SetupEditor => C("open_setup_editor"),
        Message::SetupEnd => C("show_stock_after_setup"),
        Message::SetupStep(_) => C("set_setup_step"),
        Message::Tab(_) => C("set_tab"),
        Message::Edit(target, key, _) => edit(*target, key),
        Message::PutBackEdit => C("undo_last_edit"),
        Message::Title(_) => C("set_project_title"),
        Message::AddOp(_) => C("add_operation"),
        Message::AddTurn(_) => C("add_turn"),
        Message::DuplicateOp => C("duplicate_operation"),
        // The editor's question is answered by the command itself.
        Message::RemoveOp | Message::AskRemoveOp | Message::ConfirmDeleteOp(_) => {
            C("remove_operation")
        }
        Message::MoveOp(_) => C("move_operation"),
        Message::ToggleOp => C("toggle_operation"),
        Message::Feature(_) => C("pick_features"),
        Message::Surface(_) | Message::PickMode => C("pick_surfaces"),
        Message::DropHole(_) => C("drop_hole"),
        Message::ClearSurfaces => C("clear_picks"),
        Message::BoxFromFace => C("box_from_face"),
        Message::AllCorners => C("pick_all_corners"),
        Message::PickPinHoles | Message::PinHoleSurface(_) => C("pick_pin_holes"),
        Message::PlacePins => C("place_pins"),
        Message::AddPinDrilling => C("add_pin_drilling"),
        Message::AddThreading => C("add_threading"),
        Message::AddNeededTool => C("add_needed_tool"),
        Message::AddHoleStages => C("add_hole_stages"),
        Message::AddCentring => C("add_centring"),
        Message::ThreadFromPart => C("thread_from_part"),
        Message::Generate => C("generate"),
        Message::GenerateAll => C("generate_all"),
        Message::Generated(..)
        | Message::GeneratedAll(..)
        | Message::GeneratedLayout(..)
        | Message::GenerationFailed(..) => Internal,
        Message::CancelGenerate => C("cancel_generation"),
        Message::AddSetup | Message::AddSetupOn(_) => C("add_setup"),
        Message::AddManualSetup => C("add_manual_setup"),
        Message::AddLatheSetup => C("add_lathe_setup"),
        Message::DuplicateSetup => C("duplicate_setup"),
        Message::RemoveSetup | Message::ConfirmDeleteSetup(_) => C("remove_setup"),
        Message::MoveSetup(_) => C("move_setup"),
        Message::Flip(_) => C("flip_setup"),
        Message::TurnStock(..) => C("turn_stock"),
        Message::SquareToStock => C("square_to_stock"),
        Message::AssignMachine(_) => C("assign_machine"),
        Message::NewMachine => C("new_machine"),
        // It writes a UC100 profile file.
        Message::AddRotaryMachine => Machine,
        Message::EditMachine(_) => C("edit_machine"),
        Message::WizardStep(_) => C("machine_wizard_step"),
        Message::SaveMachine => C("save_machine"),
        Message::DeleteMachine(_) => C("delete_machine"),
        Message::ToolMachine(..) => C("tool_machine"),
        Message::HolderMachine(..) => C("holder_machine"),
        // The machine's UC100 profile (`uc100_profile`).
        Message::BrowseUc100Profile | Message::Uc100ProfilePicked(_) => Machine,
        Message::ControlMachine(_)
        | Message::SwitchMachine(_)
        | Message::MachineConnection(_)
        | Message::PiAddress(_)
        | Message::PiCode(_)
        | Message::PiPair
        | Message::PiPaired(_)
        | Message::PiForget
        | Message::PiTakeControl
        | Message::Connect
        | Message::Disconnect
        | Message::MachineReset => Machine,
        // Back to the manual mill's steps on the Control page.
        Message::ShowGuide => Machine,
        Message::MachineEStop => C("estop"),
        Message::Machine(a) => machine(a),
        // The Control page's Setup side is the UC100 profile editor.
        Message::ControlSetup(_) => Machine,
        Message::ControlTab(t) => control_tab(*t),
        Message::ControlPhaseTab(..) => Internal,
        Message::ControlView(v) => job_view(*v),
        Message::Setup(msg) => setup_msg(msg),
        Message::JogKey(..) | Message::JogKeyChecked(..) | Message::JogKeyUp | Message::HoldKey => {
            Machine
        }
        Message::TabKey(_)
        | Message::Shift(_)
        | Message::HelpKey
        | Message::HelpFor(_)
        | Message::EscapeKey => Internal,
        Message::AuditRow(_) => C("open_audit_row"),
        Message::OpenIssue(..) => C("open_issue"),
        Message::Run(a) => run_action(a),
        Message::RunOnMachine(_) => Machine,
        Message::RunChecked(..)
        | Message::RunLayout(..)
        | Message::RunCheckedStock(..)
        | Message::RunPictures(..) => Internal,
        Message::EditTool(_) => C("edit_tool"),
        Message::NewTool => C("new_tool"),
        Message::SaveTool => C("save_tool"),
        Message::DeleteTool(_) => C("delete_tool"),
        Message::EditMaterial(_) => C("edit_material"),
        Message::NewMaterial => C("new_material"),
        Message::SaveMaterial => C("save_material"),
        Message::DeleteMaterial(_) => C("delete_material"),
        Message::PickMaterial(_) => C("set_project_material"),
        Message::NewHolder(_) => C("new_holder"),
        Message::EditHolder(_) => C("edit_holder"),
        Message::SaveHolder => C("save_holder"),
        Message::DeleteHolder(_) => C("delete_holder"),
        Message::SetClamps(_) => C("set_clamps"),
        Message::SuggestLayout => C("suggest_layout"),
        Message::SetOpTool(_) => C("set_operation_tool"),
        Message::RecommendSpeeds => C("recommend_speeds"),
        Message::RestSource(_) => C("set_rest_source"),
        // The file commands do the dialog's work and send its answer.
        Message::SaveProject | Message::SaveProjectAs | Message::Saved(_) => C("save_project"),
        Message::SaveForTablet | Message::TabletSaved(_) => C("save_for_tablet"),
        Message::OpenProject | Message::OpenHistory | Message::ProjectOpened(_) => {
            C("open_project")
        }
        Message::Import
        | Message::ImportPathOpen(_)
        | Message::ImportPath(_)
        | Message::ImportFromPath => C("import_model"),
        Message::FileImported(_) | Message::ModelReimported(..) | Message::ModelRestored(..) => {
            Internal
        }
        Message::Reimport => C("reimport_model"),
        Message::PicksChecked => C("dismiss_pick_note"),
        Message::Export(_) | Message::Exported(_) | Message::ExportError(_) => {
            C("export_setup_sheet")
        }
        Message::ToggleStock => C("toggle_stock"),
        Message::ToggleUncut => C("toggle_uncut"),
        Message::ToggleSimDetails => C("toggle_sim_details"),
        Message::ToggleWarningDetails => C("toggle_warning_details"),
        Message::Section(e) => section_edit(e),
        Message::TogglePaths => C("toggle_paths"),
        Message::Camera(_) => C("set_camera"),
        Message::ToggleSidebar(_) => C("toggle_sidebar"),
        Message::SidebarWidth(..) => C("sidebar_width"),
        Message::ToggleStrategyHelp => C("toggle_about"),
        Message::Resized(_) | Message::WindowOpened(..) => Internal,
        Message::Viewport(e) => viewport_event(e),
        Message::PlanViewport(_) => Planner,
        Message::Play => C("play"),
        Message::Scrub(_) | Message::ScrubDone => C("scrub"),
        Message::Replayed(..)
        | Message::ToothLoaded(..)
        | Message::PartHoles(..)
        | Message::ReplayFailed(_) => Internal,
        Message::Speed(_) => C("playback_speed"),
        Message::StockDetail(_) => C("stock_detail"),
        Message::StockMeshed { .. }
        | Message::StartMeshed { .. }
        | Message::RunMeshed { .. }
        | Message::Tick => Internal,
        Message::Plan
        | Message::PlanAsk(_)
        | Message::PlanSend
        | Message::PlanProgress(..)
        | Message::PlanFinished(..)
        | Message::PlanCancel
        | Message::PlanPicture(_)
        | Message::PlanExpand(_)
        | Message::ApplyPlan
        | Message::DiscardPlan
        | Message::UndoPlan => Planner,
        Message::PostChecked(..) | Message::LeftStock(_) | Message::Ready(_) => Internal,
        Message::ExportGcode | Message::GcodeExported(_) => C("export_gcode"),
        Message::AccountRestored(_)
        | Message::SignIn
        | Message::SignInCancel
        | Message::Approved(..)
        | Message::SignedIn(..)
        | Message::SessionSaved(_)
        | Message::SignOut
        | Message::SignedOut(_)
        | Message::ModelsLoaded(..)
        | Message::LoadModels
        | Message::PickModel(_)
        | Message::CheckConnection
        | Message::ConnectionChecked(..)
        | Message::PlanNoticeDismissed
        | Message::PlanPictures(_)
        | Message::OpenUrl(_) => Planner,
        Message::Close(_)
        | Message::Captured(_)
        | Message::Frame(_)
        | Message::WindowSized(_)
        | Message::HelpHover(_)
        | Message::HelpMore(_)
        | Message::HelpPause(_)
        | Message::Noop => Internal,
        Message::Guide(a) => guide(a),
    }
}

/// An edit of a field: the work zero and the pins' setup have commands of
/// their own, the pin holes and the pins' place are picked (`pick_pin_holes`,
/// `place_pins`), and a machine's controller fields are refused.
fn edit(target: Target, key: &str) -> Class {
    match (target, key) {
        (Target::Setup, "origin") => Class::Command("set_origin"),
        (Target::Operation, "pin_setup") => Class::Command("set_pin_setup"),
        (Target::Setup, "pin_holes") => Class::Command("pick_pin_holes"),
        (Target::Setup, "pin_place") => Class::Command("place_pins"),
        (_, key) if machine_key(key) => Class::Machine,
        (
            Target::Operation
            | Target::Setup
            | Target::Machine
            | Target::Tool
            | Target::Material
            | Target::Holder,
            _,
        ) => Class::Command("edit_field"),
    }
}

/// The step guide of a manual mill or lathe: only opening the editor at a
/// problem is the agent's; moving the place and the readings are the
/// machinist's, and the guide's own beat and build are internal.
pub fn guide(a: &GuideAction) -> Class {
    match a {
        GuideAction::Fix(..) => Class::Command("steps_fix"),
        GuideAction::Next
        | GuideAction::Back
        | GuideAction::Go(_)
        | GuideAction::Section(_)
        | GuideAction::Reached(_)
        | GuideAction::Resume
        | GuideAction::Open(_)
        | GuideAction::Measured(..)
        | GuideAction::Typed(..)
        | GuideAction::CrossReads(_) => Class::Machine,
        GuideAction::Beat(_) | GuideAction::Built(..) => Class::Internal,
    }
}

/// The UC100 profile editor: every action is the controller's.
pub fn setup_msg(m: &setup::Msg) -> Class {
    match m {
        setup::Msg::Edit(e) => setup_edit(e),
        setup::Msg::Save | setup::Msg::Revert | setup::Msg::UseCalibration => Class::Machine,
    }
}

/// A change in the UC100 profile editor's form: the controller's.
pub fn setup_edit(e: &setup::Edit) -> Class {
    match e {
        setup::Edit::Number(..)
        | setup::Edit::Text(..)
        | setup::Edit::Flag(..)
        | setup::Edit::Pin(..)
        | setup::Edit::ActiveLow(..)
        | setup::Edit::AddAxis(_)
        | setup::Edit::RemoveAxis(_)
        | setup::Edit::ADirection(_) => Class::Machine,
    }
}

/// The 3D view's section: a view, not the job.
pub fn section_edit(e: &section::Edit) -> Class {
    match e {
        section::Edit::Toggle
        | section::Edit::Axis(_)
        | section::Edit::PickFace
        | section::Edit::At(_)
        | section::Edit::Flip => Class::Command("section"),
    }
}

/// The 3D view's pointer: camera moves, picks, and the pins' aim.
pub fn viewport_event(e: &viewport::Event) -> Class {
    match e {
        viewport::Event::Orbit(..) => Class::Command("orbit_camera"),
        viewport::Event::Pan(..) => Class::Command("pan_camera"),
        viewport::Event::Zoom(..) => Class::Command("zoom_camera"),
        viewport::Event::Pick(_) => Class::Command("pick_features"),
        viewport::Event::PickSurface(_) => Class::Command("pick_surfaces"),
        viewport::Event::SectionSurface(_) => Class::Command("section"),
        viewport::Event::PlacePins(_) => Class::Command("place_pins"),
        // The pins drawn under the pointer while placing them: a hover.
        viewport::Event::AimPins(_) => Class::Internal,
    }
}

/// The Control page's actions on the connected machine: all the user's,
/// but for ending a jog the user holds.
pub fn machine(a: &control::Action) -> Class {
    match a {
        control::Action::JogRelease => Class::Command("jog_release"),
        control::Action::Home
        | control::Action::MachineZero(_)
        | control::Action::KeepPosition
        | control::Action::JogPress(..)
        | control::Action::JogStep(_)
        | control::Action::JogAngle(_)
        | control::Action::JogSpeed(_)
        | control::Action::WorkZero(_)
        | control::Action::WorkHalf(_)
        | control::Action::WorkText(..)
        | control::Action::WorkSet(_)
        | control::Action::MoveTo(_)
        | control::Action::TargetText(..)
        | control::Action::TargetEnter(_)
        | control::Action::GoTo(_)
        | control::Action::Wcs(_)
        | control::Action::Rpm(_)
        | control::Action::Spindle(_)
        | control::Action::Flood(_)
        | control::Action::Mist(_)
        | control::Action::FeedOverride(_)
        | control::Action::OverrideLimits(_)
        | control::Action::MdiText(_)
        | control::Action::MdiSend
        | control::Action::TestOutput(..)
        | control::Action::CalAxis(_)
        | control::Action::CalDistance(_)
        | control::Action::CalMeasured(_)
        | control::Action::CalMove
        | control::Action::ProbeZ
        | control::Action::ProbeEdges
        | control::Action::ProbeFrom(_)
        | control::Action::ProbeFeature(_)
        | control::Action::TouchOff
        | control::Action::ToSimZero
        | control::Action::RecordPins
        | control::Action::ReferencePins
        | control::Action::PinZero
        | control::Action::BarText(_)
        | control::Action::SpacingText(_)
        | control::Action::Calibrate(_)
        | control::Action::TouchCentreReference
        | control::Action::SaveCentreline
        | control::Action::Remounted => Class::Machine,
    }
}

/// The Run panel: Hold and Stop, and folding its rows open, are the
/// agent's; every other action starts, steers or records the run.
pub fn run_action(a: &run::Action) -> Class {
    match a {
        run::Action::Hold => Class::Command("hold"),
        run::Action::Stop => Class::Command("stop"),
        run::Action::More(_) | run::Action::Steps(_) => Class::Command("run_details"),
        run::Action::Setup(_)
        | run::Action::Check
        | run::Action::Next(_)
        | run::Action::From(..)
        | run::Action::KeepZero
        | run::Action::CycleStart
        | run::Action::Resume
        | run::Action::RecordReference
        | run::Action::ZFromReference
        | run::Action::ProbeReference
        | run::Action::ProbeFromReference
        | run::Action::ProbeCentreReference
        | run::Action::CoolantOff
        | run::Action::Skip(..)
        | run::Action::PickStep(..)
        | run::Action::ClearSteps
        | run::Action::SkipSteps(_)
        | run::Action::StartStep
        | run::Action::KeepToolZ
        | run::Action::StopAfterStep => Class::Machine,
    }
}

/// A tab of the Control page's centre: which one shows, nothing more.
pub fn control_tab(t: ui::ControlTab) -> Class {
    match t {
        ui::ControlTab::Job
        | ui::ControlTab::ToolChange
        | ui::ControlTab::Start
        | ui::ControlTab::WorkZero
        | ui::ControlTab::Probe
        | ui::ControlTab::Rotary
        | ui::ControlTab::Mdi
        | ui::ControlTab::Machine => Class::Command("control_tab"),
    }
}

/// How the Control page's Job tab shows the stock.
pub fn job_view(v: ui::JobView) -> Class {
    match v {
        ui::JobView::Stock | ui::JobView::Top => Class::Command("job_view"),
    }
}
