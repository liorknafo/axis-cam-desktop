//! Contextual help shared by every command and form field.
use crate::{Message, Target, model::Strategy};
use iced::{
    Border, Color, Element, Font, Length, Point, Rectangle, Renderer, Theme, mouse,
    widget::{self, canvas, column, container, mouse_area, text, tooltip},
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Diagram {
    Operation(Strategy),
    Heights,
    FixtureClearance,
    Offsets,
    Stepover,
    Stepdown,
    Orientation,
    Ramp,
    Tool,
    Allowance,
    StockOffset(usize, bool),
    MillingDirection,
    MillingChoice(bool, bool),
    Spindle(bool),
    FacingPattern(bool),
    FacingLink(bool),
    Entry(bool),
    DrillCycle(u8),
    Containment(i8),
}
pub struct Help {
    pub title: String,
    pub body: String,
    pub diagram: Option<Diagram>,
}
impl Help {
    pub fn text(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            diagram: None,
        }
    }
    fn animated(mut self, d: Diagram) -> Self {
        self.diagram = Some(d);
        self
    }
}
pub struct HelpButton<'a> {
    button: widget::Button<'a, Message>,
    help: Help,
}
impl<'a> HelpButton<'a> {
    pub fn new(
        button: widget::Button<'a, Message>,
        message: &Message,
        label: impl Into<String>,
    ) -> Self {
        Self {
            button,
            help: command(message, label.into()),
        }
    }
    pub fn operation(button: widget::Button<'a, Message>, strategy: Strategy, name: &str) -> Self {
        Self {
            button,
            help: Help::text(
                name,
                format!(
                    "{}\n\nClick to edit this operation's tool, geometry and cutting parameters.",
                    strategy.help()
                ),
            )
            .animated(Diagram::Operation(strategy)),
        }
    }
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.button = self.button.width(width);
        self
    }
}
impl<'a> From<HelpButton<'a>> for Element<'a, Message> {
    fn from(b: HelpButton<'a>) -> Self {
        wrap(b.button, b.help, tooltip::Position::Bottom)
    }
}
pub fn card<'a>(h: Help) -> Element<'a, Message> {
    let mut c = column![
        text(h.title).size(14).font(Font {
            weight: iced::font::Weight::Semibold,
            ..Font::DEFAULT
        }),
        text(h.body).size(12)
    ]
    .spacing(9)
    .width(278);
    if let Some(d) = h.diagram {
        c = c.push(canvas(Illustration(d)).width(278).height(155)).push(
            text("Concept illustration · not a computed toolpath")
                .size(10)
                .color(Color::from_rgb8(159, 189, 209)),
        );
    }
    container(c)
        .padding(13)
        .style(|_| container::Style {
            background: Some(Color::from_rgb8(27, 44, 61).into()),
            text_color: Some(Color::from_rgb8(236, 242, 249)),
            border: Border {
                color: Color::from_rgb8(71, 95, 118),
                width: 1.,
                radius: 7.into(),
            },
            ..Default::default()
        })
        .into()
}
pub fn wrap<'a>(
    content: impl Into<Element<'a, Message>>,
    h: Help,
    position: tooltip::Position,
) -> Element<'a, Message> {
    let animated = h.diagram.is_some();
    let tip = tooltip(content, card(h), position)
        .gap(9)
        .padding(0)
        .delay(Duration::from_millis(600))
        .snap_within_viewport(true);
    if animated {
        mouse_area(tip)
            .on_enter(Message::HelpHover(true))
            .on_exit(Message::HelpHover(false))
            .into()
    } else {
        tip.into()
    }
}
pub fn field<'a>(
    content: impl Into<Element<'a, Message>>,
    key: &str,
    label: &str,
    target: Target,
) -> Element<'a, Message> {
    wrap(
        content,
        field_help(key, label, target),
        tooltip::Position::Left,
    )
}
pub fn command(m: &Message, label: String) -> Help {
    use Message::*;
    if label == "Cancel" && matches!(m, Page(crate::Page::Machines)) {
        return Help::text(
            label,
            "Leave the machine wizard without saving this draft to the library.",
        );
    }
    if let AddOp(s) = m {
        return Help::text(s.name(),format!("{}\n\n{}\n\nConfigure Tool → Geometry → Heights → Passes/Cycle → Linking. Unavailable strategies open Operation support for imported jobs; their animations are educational.",s.help(),s.availability())).animated(Diagram::Operation(*s));
    }
    if let StockDetail(detail) = m {
        return crate::choice_help::selected(
            "stock_detail",
            "Stock surface detail",
            &detail.to_string(),
            Target::Operation,
            &crate::model::Fields::new(),
        );
    }
    let description = match m {
        NewProject => {
            "Start an empty project with no model, setups or operations. Keep saved machines and tools; archive the current project first. Shortcut: ⌘N."
        }
        ResumeProject => {
            "Open the previous workspace and reload its source model. Nothing is loaded automatically from the start screen."
        }
        OpenSample => {
            "Open the optional bearing-housing example with sample profiles and operations. The current project is archived first."
        }
        RecoverPrevious => {
            "Reopen the project archived before the most recent switch. Your current work is archived before restoring it."
        }
        Page(crate::Page::Audit) => {
            "See the scope of every operation and run generation checks for every enabled operation in every setup."
        }
        Page(crate::Page::Home) => {
            "Return to the start screen to create a blank project, open a file, resume work or explore the optional example."
        }
        Import => {
            "Open STEP/STP, 3MF, STL or OBJ geometry for viewing. STEP and 3MF units are converted to mm. Import runs in the background; machining feature recognition is a future step."
        }
        OpenHistory => {
            "Browse archived projects, including unsaved work preserved before New project or a project switch. Choose an .axis.json file to recover it."
        }
        OpenProject => {
            "Open a saved .axis.json project, including machine profiles, tools, setups and operation settings. The previous workspace is archived first. Shortcut: ⌘O."
        }
        SaveProject => {
            "Save a portable JSON project containing your settings. Keep the original model file available: the project stores its path, not embedded geometry. Shortcut: ⌘S."
        }
        Page(crate::Page::Machines) => {
            "Manage reusable machine profiles. Use the wizard to define travel, rotary axes and controller, then assign a saved version to a setup."
        }
        Page(crate::Page::Tools) => {
            "Define cutters and holders, then choose a saved tool inside each operation."
        }
        Page(crate::Page::Materials) => {
            "Manage the material library: each material's group, notes and the surface speed and chip load ranges for carbide tools that speeds and feeds are checked against. Choose the project material in the setup."
        }
        Page(crate::Page::Workholding) => {
            "Manage the library of vises and toe clamps: their sizes and notes. Choose one for a setup under Workholding in the setup editor; that setup's jaws or clamps are then checked as boxes with a clearance gap. The starter entries have typical sizes: measure yours."
        }
        Page(crate::Page::Ai) => {
            "Sign in with ChatGPT and let the model propose a complete job for the imported part. The app checks every proposal before you can apply it; nothing is sent until you click Plan."
        }
        Page(crate::Page::Work) => {
            "Return to the part viewport, setup browser and operation editor. Your current edits remain in the workspace."
        }
        Page(crate::Page::Post) => {
            "Check each setup and export LinuxCNC G-code or a setup sheet. Checks cover generation, machine limits, rapids into material and holder clearance, and, in a setup that models a vise or clamps, the jaws or clamps as boxes with a clearance gap over every program move, links, approaches and retracts included; parallels, bolts, the table, the machine body and other fixtures are not checked. Speeds and feeds outside the project material's cutting data are listed as warnings and never block export."
        }
        Page(crate::Page::Wizard) | NewMachine => {
            "Create a reusable machine profile in five steps: identity, travel, kinematics, controller and review."
        }
        AddSetup => {
            "Add another machining position with its own machine, stock, work offset and operations. Use a new setup when you reclamp, flip or reorient the part. Without a setup, choose or configure your machine first."
        }
        StockDetail(_) => {
            "Adaptive octree meshing keeps uniform regions coarse and refines the surface. Dual contouring preserves planes and cutter-radius curves. Each detail step doubles sampling resolution per axis. This is display resolution, not machining tolerance; features thinner than a cell can be missed. Updates run in the background. Updating means the last completed surface is still shown. Removed volume is a separate coarse estimate."
        }
        SetupEditor => {
            "Set the machine, work coordinate system, stock, part orientation and workholding for the current setup."
        }
        SelectSetup(_) => {
            "Make this setup active and show its part orientation and operations. Each setup represents a machining position or reclamping stage."
        }
        SelectOp(_) => {
            "Open this operation's parameters. The tabs guide you through the same tool, geometry, heights, passes and linking workflow used by CAM systems."
        }
        DuplicateOp => {
            "Copy this operation, including its tool, geometry and cutting settings, into the current setup. Adjust the copy for another pass."
        }
        RemoveOp => {
            "Delete this operation from the current setup. References from later rest-machining operations are cleared."
        }
        MoveOp(n) if *n < 0 => {
            "Move the operation earlier in the setup. Review rest-machining dependencies after changing the cutting order."
        }
        MoveOp(_) => {
            "Move the operation later in the setup. Review rest-machining dependencies after changing the cutting order."
        }
        ToggleOp => {
            "Exclude or include this operation in generation and review while keeping its parameters saved."
        }
        Feature(_) => "Include this built-in housing feature in the example operation.",
        Surface(_) => {
            "Toggle this planar region on your imported part. Blue marks selected surfaces; amber shows the region under the pointer. Selection is saved with this operation and tied to this model's geometry."
        }
        ClearSurfaces => {
            "Remove this operation's saved surface selections so you can choose new surfaces on the current part."
        }
        PickMode => {
            "Select geometry on the current part. Click a surface to toggle it, drag to orbit, and click Finish picking when done. Imported Facing/Pocket selections are connected planar mesh regions, not exact CAD faces."
        }
        Generate => {
            "Generate the selected supported imported-part operation with sampled stock removal. The example project uses illustrative paths. G-code export is on the NC program page."
        }
        GenerateAll => {
            "Generate and check every enabled operation in every setup. The report lists successful previews and specific failures; disabled operations are skipped. Remaining stock carries through earlier operations and setups. Failed upstream operations block downstream stock; playback remains one operation at a time."
        }
        DuplicateSetup => {
            "Copy this setup and its operations to prepare a second machining position. Set its new orientation, stock source and work offset."
        }
        RemoveSetup | ConfirmDeleteSetup(true) => {
            "Remove this setup and its operations. Later stock is rebuilt from the revised setup order."
        }
        ConfirmDeleteSetup(false) => "Keep this setup and close the deletion confirmation.",
        MoveSetup(n) if *n < 0 => {
            "Move this setup earlier in the job. Stock can only be inherited from a preceding setup."
        }
        MoveSetup(_) => {
            "Move this setup later in the job. Check that the stock and reclamping sequence still matches your intended process."
        }
        Flip("rx") => {
            "Set the part rotation to 180° about X to expose the opposite side. Review the origin, fixture and stock source after reclamping."
        }
        Flip(_) => {
            "Set the part rotation to 90° about Y to expose a side. Review the origin, fixture and stock source for this position."
        }
        AssignMachine(_) => {
            "Use this machine to create the first setup, or copy its saved version into the current setup. Later library edits will not silently change that setup's machine settings."
        }
        EditMachine(_) => {
            "Open this saved profile in the wizard. Saving creates an updated library version; assign it again to update a setup snapshot."
        }
        SaveMachine => {
            "Validate and save the machine profile locally for reuse in future setups and projects."
        }
        DeleteMachine(_) => {
            "Remove this profile from the reusable library. Existing setup snapshots remain available."
        }
        WizardStep(0) => {
            "Name the machine and choose 3, 4 or 5 axes and the units used by its specification."
        }
        WizardStep(1) => {
            "Record linear travel, spindle speed and feed limits from the machine specification."
        }
        WizardStep(2) => {
            "Describe rotary axis arrangement, angular limits, direction and pivot positions. These are saved settings; kinematic solving is still planned."
        }
        WizardStep(3) => {
            "Choose the intended controller, tool-change method and tool center point capability."
        }
        WizardStep(_) => {
            "Review the profile and resolve configuration issues before saving it to your local machine library."
        }
        NewTool => {
            "Create a cutter definition with diameter, flute count, cutting length, stickout and holder dimensions."
        }
        EditTool(_) => "Edit this saved cutter. Tool dimensions are stored in millimeters.",
        SaveTool => "Validate the cutter and save it to the project's tool library.",
        DeleteTool(_) => {
            "Remove this cutter if no operation uses it. Assign another tool to those operations first."
        }
        NewMaterial => {
            "Create a material with a name, group, notes and optional cutting data. It joins the library when you save it."
        }
        EditMaterial(_) => {
            "Edit this library material. Its cutting data are starting points for carbide tools; compare them with your tool maker's data."
        }
        SaveMaterial => {
            "Validate the material and save it to the library. The name must be new and the cutting data complete or empty."
        }
        DeleteMaterial(_) => {
            "Remove this material from the library. The project material cannot be deleted while it is selected: choose another one in the setup first."
        }
        PickMaterial(_) => {
            "Use this library material for the project. Its cutting data are used to check speeds and feeds; its name, group, hardness, notes and cutting data are sent to the AI planner."
        }
        NewHolder("Clamp") => {
            "Create a toe clamp with its length, width, height and bite. It joins the library when you save it. Enter the sizes of your own clamp."
        }
        NewHolder(_) => {
            "Create a vise with its jaw sizes, maximum opening and grip. It joins the library when you save it. Enter the sizes of your own vise."
        }
        EditHolder(_) => "Edit this library entry. Changing it changes every setup that uses it.",
        SaveHolder => {
            "Validate the entry and save it to the library. The name must be new and every size a positive number; a clamp's bite must be shorter than its length."
        }
        DeleteHolder(_) => {
            "Remove this entry from the library. It cannot be deleted while a setup uses it: choose other workholding in that setup first."
        }
        SuggestLayout => {
            "Place this setup's vise jaws or clamps. A vise goes on the X or the Y sides, whichever its generated toolpaths leave clearer, at its grip or less. Clamps are chosen among the middle and quarter points of each side: four, or two on a small stock. The status says how many operations still come within the clearance gap. The setup's operations are generated for this and never changed. Jaws and clamps are checked as boxes with the gap; parallels, bolts, the table and the machine body are not. Choosing Vise or Clamps, or another library entry, does this too; other edits are kept until you press this again."
        }
        SetOpTool(_) => {
            "Use this saved cutter for the current operation. Check diameter, reach, feeds and speeds before generating a preview."
        }
        RestSource(_) => {
            "Choose the earlier operation whose remaining material should guide rest machining. Earlier cuts automatically lower the cutting start when the area above it is fully cleared. Selecting isolated leftover regions for rest machining is not implemented."
        }
        StockSource(s) if s == "fresh" => {
            "Begin this setup from the defined stock dimensions instead of inheriting a previous setup's remaining material."
        }
        StockSource(_) => {
            "Imported jobs automatically inherit all earlier enabled operations and setups in browser order. Setup rotation and translation reposition that same remaining material."
        }
        Export("sheet") => {
            "Save a Markdown setup sheet with machine, workholding, repositioning instructions and operation parameters."
        }
        ExportGcode => {
            "Choose a folder and write one LinuxCNC .ngc program per setup. Every check and the replay run again first; nothing is written if any setup fails."
        }
        Camera("ISO") => {
            "Reset to an isometric view that shows the top and two sides. Drag to orbit, right-drag to pan, and scroll to zoom."
        }
        Camera("TOP") => {
            "Look down the setup Z axis onto its XY plane. This changes the camera, not the machining setup."
        }
        Camera("FRONT") => "View the setup from the front along Y. This changes the camera only.",
        Camera(_) => "View the setup from its right side along X. This changes the camera only.",
        Play => {
            "Play or pause cutter motion. For imported Facing or Pocket, generate the preview if needed and show sampled stock removal. Other imported strategies are not supported yet. The example project uses illustrative motion."
        }
        Scrub(_) => {
            "Return to this point in the preview and pause. Simulated stock is recomputed when you rewind; Restart restores the uncut stock."
        }
        ToggleStock => {
            "Show or hide stock. During Facing or Pocket simulation this displays the remaining sampled stock; hide Stock to see the finished reference model. Outside simulation it shows the transparent stock box. Relative stock extends from each side of the placed part by its offset; fixed stock uses explicit dimensions. The box and part share the same display scale."
        }
        TogglePaths => {
            "Show or hide the cutter path. Pocket shows cuts on the current depth layer; Facing shows the latest cutting pass. Both show the active rapid; the example project shows illustrative paths."
        }
        Plan => {
            "Ask the model for a complete job: setups, operations, tools and cutting parameters. The app checks every proposal with its generators, the stock simulation, the export checks and the speeds and feeds against the project material's cutting data, and sends problems back for correction, up to four requests. Requests count against your ChatGPT plan. Nothing changes until you apply the plan."
        }
        PlanCancel => "Stop planning. The job is unchanged.",
        ApplyPlan => {
            "Replace the operations of setup 1 and remove later setups, adding the planned ones. Setup 1 keeps its name, machine, stock, work zero and fixture. The previous job is saved to History first and can be undone on this page."
        }
        DiscardPlan => "Close this result. The job is unchanged.",
        UndoPlan => {
            "Put back the setups and operations from before the plan was applied, and any tool they use that was deleted since (keeping its own number when no tool has it, otherwise with a free one). The applied job, with your edits since, is saved to History first. Tools, machines, the project title and the material changed since stay. Importing a part or opening or creating a project ends the Undo; the job stays in History."
        }
        Tab(t) => {
            return Help::text(
                label,
                match t.as_str() {
                    "Tool" => "Choose a cutter and set spindle speed, feed rate and coolant.",
                    "Geometry" => {
                        "Select the faces, contours or holes to machine, and define the allowed cutting boundary."
                    }
                    "Heights" => {
                        "Set clearance and retract planes, plus the top and bottom offsets that bound the cut."
                    }
                    "Orientation" => {
                        "Set indexed rotary angles or the tool-axis strategy for multi-axis cutting."
                    }
                    "Cycle" => {
                        "Choose how the drill feeds, pecks, pauses and retracts between holes."
                    }
                    "Passes" => {
                        "Control cutting depth, spacing, finishing allowance and tolerance."
                    }
                    "Linking" => {
                        "Control how the tool enters a cut and moves between cutting passes."
                    }
                    _ => "Open this operation parameter group.",
                },
            );
        }
        SignIn => {
            "Open your browser to sign in with ChatGPT. AXIS CAM Studio receives your name, email and profile picture. Planning requests then count against your ChatGPT plan."
        }
        SignInCancel => "Stop waiting for the browser sign-in. Nothing is saved.",
        SignOut => {
            "Sign out of ChatGPT on this computer and ask OpenAI to revoke this sign-in. The saved sign-in is deleted."
        }
        CheckConnection => {
            "Send one tiny request to the selected model to confirm that your ChatGPT plan works with this app. It uses a very small amount of your plan's usage."
        }
        LoadModels => "Ask OpenAI again which models your ChatGPT plan can use in this app.",
        PlanNoticeDismissed => "Close this notice. It is shown once.",
        OpenUrl(_) => {
            "Open your ChatGPT usage settings in the browser, where you can see and manage how much of your plan this app uses."
        }
        _ => "Change this setting in the current workspace.",
    };
    let h = Help::text(label, description);
    match m {
        Flip(_) | AddSetup | DuplicateSetup => h.animated(Diagram::Orientation),
        _ => h,
    }
}

pub fn field_help(key: &str, label: &str, target: Target) -> Help {
    use Diagram::*;
    let (body, diagram) = match (target, key) {
        (Target::Machine, "rpm") => (
            "Maximum spindle speed allowed by this machine, in revolutions per minute. Operations above this limit are flagged.",
            None,
        ),
        (Target::Machine, "feed") => (
            "Maximum feed rate in the machine profile's units per minute. Use the manufacturer's specification; operation feeds are in mm/min.",
            None,
        ),
        (Target::Machine, "direction" | "direction2") => (
            "Choose the sign convention for this rotary axis from the machine documentation. It affects how a positive angle is interpreted.",
            Some(Orientation),
        ),
        (Target::Setup, "clearance") => (
            "Requested minimum gap between the fixture and the outside of the cutter or holder, in mm. The illustration shows a side gap measured from the wider holder. This value is only saved with the setup; it does not change operation heights or toolpaths. The check of modelled vise jaws or clamps uses the Clearance gap beside them instead.",
            Some(FixtureClearance),
        ),
        (Target::Material, "name") => (
            "The material's name in the library and in the project material list. Each material needs its own name of up to 60 characters; capitals do not make a name different. When this is the project material, its name is sent to the AI planner.",
            None,
        ),
        (Target::Material, "group") => (
            "The kind of material. It is saved with the material, shown in the library list and, when this is the project material, sent to the AI planner.",
            None,
        ),
        (Target::Material, "hardness") => (
            "Optional hardness of the material, such as 95 HB. It does not change any limit. When this is the project material, it is sent to the AI planner.",
            None,
        ),
        (Target::Material, "notes") => (
            "Optional notes on the material, up to 500 characters: coolant, chip clearing or anything else to remember. They are saved with the material and, when this is the project material, sent to the AI planner.",
            None,
        ),
        (Target::Material, "vc_min") => (
            "Lowest recommended surface speed for carbide tools, in m/min. Surface speed = π × tool diameter × spindle rpm ÷ 1000. A slower operation is flagged, unless the machine's top spindle speed is the limit.",
            None,
        ),
        (Target::Material, "vc_max") => (
            "Highest recommended surface speed for carbide tools, in m/min. Surface speed = π × tool diameter × spindle rpm ÷ 1000. A faster operation is flagged.",
            None,
        ),
        (Target::Material, k) if k.starts_with("fz") && k.ends_with("_min") => (
            "Lowest recommended chip load per tooth, in mm, for tools in this diameter range. Chip load = feed ÷ (rpm × flutes). A lower value is flagged, unless the machine's top feed is the limit.",
            None,
        ),
        (Target::Material, k) if k.starts_with("fz") && k.ends_with("_max") => (
            "Highest recommended chip load per tooth, in mm, for tools in this diameter range. Chip load = feed ÷ (rpm × flutes). A higher value is flagged.",
            None,
        ),
        (Target::Holder, "name") => (
            "The entry's name in the workholding library and in the setup's list. Each entry needs its own name of up to 60 characters; capitals do not make a name different.",
            None,
        ),
        (Target::Holder, "notes") => (
            "Optional notes on the entry, up to 500 characters, such as where its sizes were measured. The starter entries say to measure yours.",
            None,
        ),
        (Target::Holder, "jaw_width") => (
            "Length of each jaw along the jaws, in mm. The jaws are centred on the stock, or shifted by the setup's jaw offset.",
            None,
        ),
        (Target::Holder, "jaw_height") => (
            "Height of the jaws above the parallels' top, in mm. Kept for reference: the modelled jaws reach from far below the stock up to the setup's grip height.",
            None,
        ),
        (Target::Holder, "jaw_thickness") => (
            "How thick each jaw is, measured outward from the stock, in mm.",
            None,
        ),
        (Target::Holder, "max_opening") => (
            "The widest stock the jaws open to, in mm. A setup whose stock is wider across the jaws shows an issue.",
            None,
        ),
        (Target::Holder, "grip") => (
            "How much of the stock the jaws hold, as a height above the stock's bottom, in mm. A setup starts from it and can change its own grip.",
            None,
        ),
        (Target::Holder, "length") => (
            "Length of the clamp from its tip outward, in mm; the bite is part of it. The rest of it reaches beyond the stock's edge.",
            None,
        ),
        (Target::Holder, "width") => (
            "Width of the clamp, in mm, along the side of the stock it sits on.",
            None,
        ),
        (Target::Holder, "height") => ("Height of the clamp above the stock's top, in mm.", None),
        (Target::Holder, "bite") => (
            "How far the clamp reaches onto the stock from its edge, in mm. It must be shorter than the clamp's length.",
            None,
        ),
        (Target::Setup, "holding") => (
            "The workholding to model for this setup: a Vise from the library, with its jaws on two opposite sides of the stock, or Clamps placed around it. Each is kept as boxes with a clearance gap, and every move is checked against them: the cutter from its tip up to its stickout at its cutting diameter, the tool holder above that at the holder diameter (a wider shank or a reduced neck is not modelled). A move that comes within the gap is warned of in the operation preview, which covers that operation's own toolpath, and refused at export, which checks every program move, the links between operations, approaches and retracts included. Parallels, bolts, the table and the machine body are not modelled. In a later setup the jaws or clamps sit at the stock box carried over from setup 1, not at the walls earlier operations cut. None models nothing; the Fixture note below is only a description.",
            None,
        ),
        (Target::Setup, "holder") => (
            "The library entry used: a vise for Vise, a clamp for Clamps. Entries are made on the Workholding page; changing one changes every setup that uses it.",
            None,
        ),
        (Target::Setup, "vise_axis") => (
            "Which sides of the stock the jaws face: X sides put one jaw on the −X side and one on the +X side, Y sides on −Y and +Y. A stock wider across the jaws than the vise's maximum opening shows an issue.",
            None,
        ),
        (Target::Setup, "vise_offset") => (
            "Shifts the jaws along their length from the middle of the stock, in mm: toward +Y for jaws on the X sides, toward +X for jaws on the Y sides. 0 centres them.",
            None,
        ),
        (Target::Setup, "vise_grip") => (
            "How much of the stock the jaws hold, as a height above the stock's bottom, in mm. The jaws are modelled from far below the stock up to it; the stock above it is not covered by the modelled jaws. It starts at the vise's own grip.",
            None,
        ),
        (Target::Setup, "holding_gap") => (
            "The clearance to keep around the jaws or clamps, in mm. 0 or more; left empty it is 2 mm. A move whose cutter or holder comes within it is warned of in the operation preview (that operation's own toolpath) and refused at export (every program move, the links between operations, approaches and retracts included).",
            None,
        ),
        (Target::Setup, "clamps") => (
            "Where the clamps sit, written side:offset and separated by semicolons, for example -X:0; +X:20. The sides are -X, +X, -Y and +Y of the stock; the offset, in mm, moves the clamp along that side from its middle (toward +Y on the X sides, toward +X on the Y sides). Place 1 to 8 clamps. The text is saved only when every clamp reads correctly; what is wrong is shown under the field.",
            None,
        ),
        (_, "name") => (
            "A descriptive name shown in the browser, library and setup sheet. Use names that identify the part position, operation or hardware.",
            None,
        ),
        (_, "rpm") => (
            "Spindle revolutions per minute. LinuxCNC export writes it as S, rounded to a whole number. Surface speed = π × tool diameter × rpm ÷ 1000. When the project material has cutting data, a surface speed outside its range is flagged on this tab and on the NC program page; export still goes ahead. The app does not calculate a speed for you.",
            Some(Tool),
        ),
        (_, "spindle_rotation") => (
            "CW or CCW, viewed from the spindle toward the workpiece along setup -Z. Match the cutter's cutting hand. Changing rotation reverses Pocket contours and one-way Facing strokes for the same milling method. The illustration uses CW for both examples.",
            Some(MillingDirection),
        ),
        (_, "facing_milling") => (
            "Climb (down milling) starts with a thicker chip and exits thin; Conventional (up milling) starts thin and exits thicker. With One way, the preview chooses travel relative to the nominal uncut side and saved spindle rotation. Entry and full-width engagement may include both modes. Both ways is mixed. Illustration: CW spindle in both examples, viewed toward the part.",
            Some(MillingDirection),
        ),
        (_, "pocket_milling") => (
            "Climb or Conventional relative to the remaining material. Reverses contour, waterline and pocket loops while retaining spindle rotation. CW climb runs counterclockwise around an internal wall and clockwise around an outside profile; CCW reverses these directions. Entry and full-width engagement can include both modes.",
            Some(MillingDirection),
        ),
        (_, "pocket_entry") => (
            "Profile ramp descends along the closed pocket contour at the specified ramp angle, then cuts a complete loop at depth. Plunge feeds straight down and needs a plunge-capable end mill. Both use 25% cutting feed for entry. Transfers between loops retract to clearance to avoid crossing islands at depth.",
            Some(Ramp),
        ),
        (_, "feed") => (
            "Cutter travel speed during cutting, in mm/min. Feed per tooth = feed ÷ (rpm × flute count). When the project material has cutting data, a feed per tooth outside the range for the tool's diameter is flagged on this tab and on the NC program page; export still goes ahead. Plunge and ramp feeds are not checked.",
            Some(Tool),
        ),
        (_, "coolant") => (
            "Coolant for this operation. LinuxCNC export writes M8 for Flood, M7 for Mist, the machine's Air output for Air (refused when it has none) and nothing for Off, then M9 when the coolant changes, at a tool change and at the end of the program. The preview does not simulate coolant.",
            None,
        ),
        (_, "hole_diameter") => (
            "Filter detected circular openings by nominal diameter in mm (±0.1 mm). Zero includes all detected holes on the selected rim. The drill must fit every included opening. Select the horizontal rim face first.",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "boundary") => (
            "Defines the region that should contain machining: selected geometry, stock outline or model outline. Facing uses rectangular extents. Pocket follows the selected floor. Contour and finishing use the imported model silhouette.",
            Some(Operation(Strategy::Contour)),
        ),
        (_, "containment") => (
            "Places the cutter inside, on, or outside the boundary. Tool-center placement differs from where the cutter's edge reaches.",
            Some(Operation(Strategy::Contour)),
        ),
        (_, "clearance") => (
            "Height above stock top used for longer non-cutting moves, in mm. It must be at least as high as the retract plane. Where the machine retracts to a clearance plane, export refuses a setup whose plane, the highest height its program reaches, is not above every modelled jaw and clamp and its gap; other fixture clearance still needs verification.",
            Some(Heights),
        ),
        (_, "retract") => (
            "Height above stock top for lifting between cutting moves, in mm. Keep it below or equal to clearance and above obstacles.",
            Some(Heights),
        ),
        (_, "top") => (
            "Positive margin above the cutting start. Earlier cuts automatically lower the start when the whole machining area above it is cleared. Otherwise Facing/Pocket/Contour/Drill use stock top and Waterline uses model top. Parallel uses this offset for approach heights. Clearance/retract keep their original reference; negative values do not force a deeper entry.",
            Some(Offsets),
        ),
        (_, "bottom") => (
            "Offset from the selected bottom geometry, in mm. Negative extends the cut below it; positive leaves the bottom higher.",
            Some(Offsets),
        ),
        (_, "through") => (
            "Request extra drill travel through the selected bottom. Check the extra depth and the material or fixture underneath.",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "breakthrough") => (
            "Additional drill travel past the bottom reference, in mm. The real required distance also depends on drill-point geometry.",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "angle_a" | "angle_b") => (
            "Indexed position of the primary or secondary rotary axis, in degrees. The axis is positioned before cutting and held fixed during the cut.",
            Some(Operation(Strategy::Indexed)),
        ),
        (_, "rotary_axis") => (
            "A rotates about X, B about Y, and C about Z. Choose the axis that matches the setup's saved machine arrangement.",
            Some(Operation(Strategy::Rotary)),
        ),
        (_, "rotary_start" | "rotary_end") => (
            "Start or end of the requested angular sweep, in degrees. Keep the interval within the saved rotary travel limits.",
            Some(Operation(Strategy::Rotary)),
        ),
        (_, "rotary_style") => (
            "Line, circular or spiral motion around the rotary axis. This records the intended pattern for a future rotary toolpath engine.",
            Some(Operation(Strategy::Rotary)),
        ),
        (_, "axis_control") => (
            "Align the tool with the surface normal or add lead and lean. Actual multi-axis orientation and collision solving are not connected yet.",
            Some(Operation(Strategy::FiveAxis)),
        ),
        (_, "tilt_min" | "tilt_max") => (
            "Allowed tool tilt range in degrees. Use it to constrain the intended orientation; it does not replace machine-axis limits or collision checks.",
            Some(Operation(Strategy::FiveAxis)),
        ),
        (_, "lead_angle") => (
            "Tool tilt forward or backward along the cutting direction, in degrees. It changes where the cutter contacts the surface.",
            Some(Operation(Strategy::FiveAxis)),
        ),
        (_, "lean_angle") => (
            "Tool tilt sideways across the cutting direction, in degrees. It is independent of forward/backward lead.",
            Some(Operation(Strategy::FiveAxis)),
        ),
        (_, "collision") => (
            "Preferred response to an obstruction: retract and relink, or tilt within limits. This is a saved preference; no collision solver is active yet.",
            None,
        ),
        (_, "cycle") => (
            "Drilling feeds once to depth. Chip breaking retracts slightly between pecks; deep drilling retracts clear of the hole. Counterboring adds a bottom dwell.",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "peck") => (
            "Maximum depth of each drilling increment, in mm. Smaller pecks shorten the chip; retract behavior is selected by the drilling cycle.",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "dwell") => (
            "Pause duration at the bottom of a counterboring cycle, in seconds, before retracting.",
            Some(Operation(Strategy::Drill)),
        ),
        (_, "hole_order") => (
            "Visit holes in their selection order or request shorter travel between them. Route optimization is a future CAM-engine feature.",
            None,
        ),
        (_, "ramp") => (
            "How the cutter enters material: a helix, back-and-forth ramp, straight plunge, or profile ramp. Match the entry to the cutter's cutting capabilities.",
            Some(Ramp),
        ),
        (_, "ramp_angle") => (
            "Slope of the entry move in degrees relative to the horizontal plane. A smaller angle takes a longer distance to reach cutting depth.",
            Some(Ramp),
        ),
        (_, "lead_in") => (
            "Radius of the curved approach into the cut, in mm. It gives the cutter a gradual entry; available space must be checked.",
            Some(Operation(Strategy::Contour)),
        ),
        (_, "keep_down") => (
            "Request links close to cutting depth where possible to reduce retracts. This does not authorize crossing uncut stock or fixtures.",
            None,
        ),
        (_, "facing_pattern") => (
            "Both ways alternates travel on neighboring rows, mixing climb and conventional cutting without reversing the spindle. One way enables the Climb / Conventional choice and retracts for return moves. The pass-angle field rotates either pattern in setup XY.",
            Some(Operation(Strategy::Face)),
        ),
        (_, "facing_depth_order") => (
            "Continue from previous end starts the next depth at the last pass's X/Y, with the row order and travel directions reversed. Return to first start repeats each depth from the original X/Y. Applies to Both ways; One way always returns.",
            None,
        ),
        (_, "facing_link") => (
            "Keep tool down feeds sideways between adjacent rows, then cuts back across the stock. Continue from previous end also feeds vertically down at the last X/Y for the next depth. Retract each pass lifts to clearance between rows. Applies to Both ways and the rectangular Facing preview; fixtures are not avoided, and a move within the clearance gap of a modelled vise jaw or clamp is refused at export.",
            None,
        ),
        (_, "direction") => (
            "Angle of the parallel cutting passes in the setup XY plane, in degrees. Rotates the pass pattern across the face.",
            Some(Operation(Strategy::Parallel)),
        ),
        (_, "stepover") => (
            "Sideways spacing of adjacent passes, as a percentage of cutter diameter. For a 10 mm tool, 40% means 4 mm. Smaller spacing increases overlap and pass count.",
            Some(Stepover),
        ),
        (_, "surface_stepover") => (
            "Spacing between adjacent finishing passes, in mm. Smaller spacing usually reduces scallop height but increases machining time.",
            Some(Stepover),
        ),
        (_, "optimal_load") => (
            "Target radial engagement for adaptive clearing, in mm. It controls how much of the cutter's side is engaged, rather than its full diameter.",
            Some(Operation(Strategy::Adaptive)),
        ),
        (_, "minimum_radius") => (
            "Smallest allowed radius of the cutting path, in mm. Larger radii can soften abrupt direction changes, where the geometry allows.",
            Some(Operation(Strategy::Adaptive)),
        ),
        (_, "stepdown") => (
            "Maximum depth removed in one Z layer, in mm. A 12 mm pocket with 3 mm stepdown needs at least four depth levels.",
            Some(Stepdown),
        ),
        (_, "side") => (
            "Choose which side of the selected contour the cutter should follow: inside, outside or centered on the chain.",
            Some(Operation(Strategy::Contour)),
        ),
        (_, "compensation") => (
            "Choose whether tool-radius offset is calculated in CAM or adjusted at the control. Wear modes need a matching, validated controller/postprocessor convention.",
            None,
        ),
        (_, "finish_passes") => (
            "Number of finishing passes along the boundary after roughing. Extra passes may improve consistency; cutting strategy still needs validation.",
            Some(Operation(Strategy::Contour)),
        ),
        (_, "slope_min" | "slope_max") => (
            "Surface slope range to include, in degrees from horizontal. 0° is a flat floor; 90° is a vertical wall.",
            Some(Operation(Strategy::Waterline)),
        ),
        (_, "pattern") => (
            "Choose a parallel or scallop-style finishing pattern. Parallel sweeps in one direction; scallop follows surface spacing.",
            Some(Operation(Strategy::Scallop)),
        ),
        (_, "allowance") => (
            "Material intentionally left on walls, in mm, for a later finishing operation. Zero requests the nominal wall position.",
            Some(Allowance),
        ),
        (_, "axial_allowance") => (
            "Material intentionally left on floors, in mm. A finishing operation removes it to reach the nominal depth.",
            Some(Allowance),
        ),
        (_, "tolerance") => (
            "Requested maximum toolpath approximation deviation, in mm. A smaller value can require more path segments. It is not a guarantee of finished-part accuracy.",
            None,
        ),
        (_, "rest") => (
            "Target material remaining from an earlier operation, often with a smaller tool. Select the previous operation as the source; remaining-stock calculation is not connected.",
            Some(Operation(Strategy::Adaptive)),
        ),
        (_, "offset") => (
            "Work coordinate register for this setup, G54–G59. Its origin must correspond to the origin probed on the real machine.",
            Some(Orientation),
        ),
        (Target::Tool, "number") => (
            "Tool number written as T# and, with a tool table, G43 H#. It must match the entry in LinuxCNC's tool table. Each library tool needs a unique number from 1 to 9999.",
            None,
        ),
        (_, "post_tool_length") => (
            "How programs apply tool length: from LinuxCNC's tool table with T# M6 and G43 H#, or one program per tool that you touch off before running it.",
            None,
        ),
        (_, "post_retract") => (
            "Where the tool goes before a tool change and at the end: machine Z0 with G53, or the setup's highest clearance height.",
            None,
        ),
        (_, "post_blend") => (
            "Path blending tolerance for LinuxCNC's trajectory planner. A positive value writes G64 P with this tolerance in mm, so corners may round by up to that much at speed. 0 writes G61 exact path.",
            None,
        ),
        (_, "post_dwell") => (
            "Seconds to wait after starting or changing the spindle before the first move. Use it when the spindle has no at-speed feedback. 0 writes no dwell.",
            None,
        ),
        (_, "post_air") => (
            "LinuxCNC output used for operations set to Air: M7 (mist) or M8 (flood), whichever is wired to the air blast.",
            None,
        ),
        (_, "origin") => (
            "Work zero for this setup's G-code. Choose a point you can reproduce when touching off the stock; arbitrary point selection is planned.",
            Some(Orientation),
        ),
        (_, "stock_type") => (
            "Relative size box adds independent offsets to the six sides of the part's bounding box after setup rotation. Fixed size box uses explicit X, Y and Z dimensions. Values for each mode are kept when switching.",
            None,
        ),
        (_, "stock_x" | "stock_y" | "stock_z") => (
            "Overall stock box extent along this setup axis, in mm. Fixed stock is centered on X and Y with its bottom on Z=0 in the preview. Use relative stock for extra material measured from each part side.",
            None,
        ),
        (_, "stock_top") => (
            "Extra material above the part's highest point along setup +Z, in mm. 10 adds 10 mm only at the top. Z size = part height + top + bottom. 0 is flush with the part.",
            Some(StockOffset(2, true)),
        ),
        (_, "stock_bottom") => (
            "Extra material below the part's lowest point along setup −Z, in mm. This is independent of the top offset. 0 is flush with the part; negative values are not allowed.",
            Some(StockOffset(2, false)),
        ),
        (_, "stock_left") => (
            "Extra material beyond the part's minimum X along setup −X, in mm. X size = part width + left + right. This side follows the setup axis, regardless of the camera view.",
            Some(StockOffset(0, false)),
        ),
        (_, "stock_right") => (
            "Extra material beyond the part's maximum X along setup +X, in mm. 2 adds 2 mm only on the right. It does not also add material on the left.",
            Some(StockOffset(0, true)),
        ),
        (_, "stock_front") => (
            "Extra material beyond the part's minimum Y along setup −Y, in mm. Y size = part depth + front + back. The front offset is independent of the back offset.",
            Some(StockOffset(1, false)),
        ),
        (_, "stock_back") => (
            "Extra material beyond the part's maximum Y along setup +Y, in mm. 0 is flush with the part. Sides are measured after the part's setup rotation, regardless of the camera view.",
            Some(StockOffset(1, true)),
        ),
        (_, "rx" | "ry" | "rz") => (
            "Rotate the part about this axis, in degrees. Rotation order is X, then Y, then Z. Used to describe a new machining position; this is not a rotary motion command.",
            Some(Orientation),
        ),
        (_, "tx" | "ty" | "tz") => (
            "Translate the part along this setup axis, in mm. Relative stock follows the part and keeps its offsets; fixed stock stays in place. Exact CAD-to-machine origin placement remains to be implemented.",
            Some(Orientation),
        ),
        (_, "fixture") => (
            "Describes the workholding used in this setup: a vise, soft jaws, clamps, chuck or custom fixture. It is printed on the setup sheet and in the program header and is not checked; to have a vise's jaws or clamps checked, model them under Modelled workholding above.",
            None,
        ),
        (_, "reposition") => (
            "Action needed before this setup: initial loading, reclamping and probing, rotary indexing, or changing the fixture. Included in the setup sheet.",
            Some(Orientation),
        ),
        (_, "notes") => (
            "Instructions for the operator: how to reclamp, which surfaces to locate, and where to probe the new origin.",
            None,
        ),
        (_, "axes") => (
            "Number of controllable axes: XYZ for 3-axis, one rotary for 4-axis, or two rotaries for 5-axis. Select indexed or simultaneous motion separately.",
            Some(Orientation),
        ),
        (_, "units") => (
            "Units for machine travel, feed and pivot dimensions. The machining project and tool dimensions remain in millimeters.",
            None,
        ),
        (_, "x" | "y" | "z") => (
            "Available linear travel along this machine axis, in the selected machine units. Enter the usable stroke from the machine specification.",
            None,
        ),
        (_, "mode") => (
            "Indexed motion positions the rotary axes before cutting; simultaneous motion changes rotary and linear positions together. This prototype saves the capability but does not solve it.",
            Some(Operation(Strategy::Indexed)),
        ),
        (_, "arrangement") => (
            "Physical rotary configuration: axes on the table, spindle head, or both. A rotates about X, B about Y, and C about Z.",
            Some(Orientation),
        ),
        (_, "rotary_min" | "rotary_max" | "rotary2_min" | "rotary2_max") => (
            "Lower or upper travel bound of this rotary axis, in degrees. Enter mechanical limits, including negative angles where supported.",
            Some(Orientation),
        ),
        (_, "pivot_x" | "pivot_y" | "pivot_z" | "pivot2_x" | "pivot2_y" | "pivot2_z") => (
            "Location of this rotary axis pivot in the machine coordinate frame, using the selected machine units. Accurate measured pivots will be required by the future kinematic solver.",
            Some(Orientation),
        ),
        (_, "controller") => (
            "Controller family. LinuxCNC has a 3-axis postprocessor; other controllers are saved for planning and cannot export G-code yet.",
            None,
        ),
        (_, "tool_change") => (
            "Whether the operator changes tools manually or the machine has an automatic tool changer. Used for planning the setup sequence.",
            None,
        ),
        (_, "tcp") => (
            "Tool center point control lets the controller compensate linear axes while rotating, to maintain the tool tip's position. Requires supported kinematics and a matching postprocessor.",
            Some(Operation(Strategy::FiveAxis)),
        ),
        (_, "type") => (
            "Cutter geometry: flat end mill, ball end mill, drill or face mill. Choose a type suited to the operation.",
            Some(Tool),
        ),
        (_, "diameter") => (
            "Cutting diameter in mm, not the holder or shank diameter. It determines slot access, radial engagement and stepover in percent.",
            Some(Tool),
        ),
        (_, "flutes") => (
            "Number of cutting edges. Used with spindle rpm and feed to calculate feed per tooth.",
            Some(Tool),
        ),
        (_, "length") => (
            "Length of the cutting flutes in mm. The cut must not rely on the non-cutting shank to remove material.",
            Some(Tool),
        ),
        (_, "stickout") => (
            "Distance from the holder nose to the cutter tip, in mm. More stickout increases reach but generally reduces rigidity.",
            Some(Tool),
        ),
        (_, "holder") => (
            "Maximum holder diameter in mm. The holder is wider than the cutter and needs its own clearance around walls and fixtures.",
            Some(Tool),
        ),
        _ => (
            "Saved parameter for the current setup or operation. See the field label for its units.",
            None,
        ),
    };
    Help {
        title: crate::material::label(key)
            .filter(|_| target == Target::Material)
            .unwrap_or_else(|| label.into()),
        body: body.into(),
        diagram,
    }
}

struct Illustration(Diagram);
impl canvas::Program<Message> for Illustration {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut f = canvas::Frame::new(renderer, bounds.size());
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        let time = START.get_or_init(Instant::now).elapsed().as_secs_f32();
        let phase = (time / 5.).fract();
        let p = |a: [f32; 3]| {
            Point::new(
                139. + (a[0] - a[1]) * 1.40,
                111. + (a[0] + a[1]) * 0.52 - a[2] * 1.35,
            )
        };
        let line = |f: &mut canvas::Frame, a: [f32; 3], b: [f32; 3], color: Color, width: f32| {
            f.stroke(
                &canvas::Path::line(p(a), p(b)),
                canvas::Stroke::default()
                    .with_color(color)
                    .with_width(width),
            );
        };
        let polygon = |f: &mut canvas::Frame, points: &[[f32; 3]], color: Color| {
            let path = canvas::Path::new(|b| {
                b.move_to(p(points[0]));
                for a in &points[1..] {
                    b.line_to(p(*a))
                }
                b.close()
            });
            f.fill(&path, color);
            f.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(Color::from_rgb8(157, 182, 198))
                    .with_width(0.7),
            );
        };
        let cyan = Color::from_rgb8(66, 211, 234);
        let gold = Color::from_rgb8(255, 191, 85);
        if matches!(
            self.0,
            Diagram::MillingDirection | Diagram::MillingChoice(..)
        ) {
            let stroke = canvas::Stroke::default().with_color(cyan).with_width(2.);
            let clockwise = match self.0 {
                Diagram::MillingChoice(_, cw) => cw,
                _ => true,
            };
            let examples = match self.0 {
                Diagram::MillingChoice(climb, _) => vec![(
                    75.,
                    if climb { -1. } else { 1. },
                    if climb { "CLIMB" } else { "CONVENTIONAL" },
                )],
                _ => vec![(5., -1., "CLIMB"), (145., 1., "CONVENTIONAL")],
            };
            for (left, sign, label) in examples {
                let sign = if clockwise { sign } else { -sign };
                f.fill_text(canvas::Text {
                    content: label.into(),
                    position: Point::new(left + 7., 3.),
                    color: cyan,
                    size: 10.into(),
                    ..Default::default()
                });
                f.fill_rectangle(
                    Point::new(left, 28.),
                    iced::Size::new(126., 44.),
                    Color::from_rgb8(105, 90, 62),
                );
                f.fill_text(canvas::Text {
                    content: "UNCUT STOCK".into(),
                    position: Point::new(left + 21., 34.),
                    color: gold,
                    size: 9.into(),
                    ..Default::default()
                });
                let center = Point::new(left + 63. + sign * (phase - 0.5) * 42., 72.);
                f.fill(
                    &canvas::Path::circle(center, 20.),
                    Color::from_rgb8(52, 76, 95),
                );
                f.stroke(
                    &canvas::Path::circle(center, 20.),
                    canvas::Stroke::default().with_color(gold).with_width(1.5),
                );
                // Screen Y points down, so increasing angle shows CW in top view.
                for tooth in 0..6 {
                    let a = time * if clockwise { 2. } else { -2. }
                        + tooth as f32 * std::f32::consts::TAU / 6.;
                    f.stroke(
                        &canvas::Path::line(
                            Point::new(center.x + 11. * a.cos(), center.y + 11. * a.sin()),
                            Point::new(center.x + 20. * a.cos(), center.y + 20. * a.sin()),
                        ),
                        canvas::Stroke::default().with_color(gold).with_width(2.),
                    );
                }
                f.fill_text(canvas::Text {
                    content: if clockwise { "CW" } else { "CCW" }.into(),
                    position: Point::new(center.x - 8., center.y - 6.),
                    color: gold,
                    size: 10.into(),
                    ..Default::default()
                });
                let end = Point::new(left + 63. + sign * 36., 111.);
                f.stroke(
                    &canvas::Path::line(Point::new(left + 63. - sign * 36., 111.), end),
                    stroke,
                );
                for dy in [-4., 4.] {
                    f.stroke(
                        &canvas::Path::line(end, Point::new(end.x - sign * 7., end.y + dy)),
                        stroke,
                    );
                }
                f.fill_text(canvas::Text {
                    content: "CUTTER TRAVEL".into(),
                    position: Point::new(left + 22., 119.),
                    color: cyan,
                    size: 9.into(),
                    ..Default::default()
                });
            }
            f.fill_text(canvas::Text {
                content: if matches!(self.0, Diagram::MillingChoice(..)) {
                    "TOP VIEW · SELECTED METHOD AND SPINDLE"
                } else {
                    "TOP VIEW · SAME SPINDLE, OPPOSITE FEED"
                }
                .into(),
                position: Point::new(25., 143.),
                color: Color::from_rgb8(185, 207, 223),
                size: 9.into(),
                ..Default::default()
            });
            return vec![f.into_geometry()];
        }
        if crate::choice_diagram::draw(self.0, &mut f, time) {
            return vec![f.into_geometry()];
        }
        if let Diagram::StockOffset(axis, positive) = self.0 {
            let vertices = |min: [f32; 3], max: [f32; 3]| {
                let [x, y, z] = min;
                let [xx, yy, zz] = max;
                [
                    [x, y, z],
                    [xx, y, z],
                    [xx, yy, z],
                    [x, yy, z],
                    [x, y, zz],
                    [xx, y, zz],
                    [xx, yy, zz],
                    [x, yy, zz],
                ]
            };
            let part_min = [-34., -22., 0.];
            let part_max = [34., 22., 24.];
            let part = vertices(part_min, part_max);
            for (indices, color) in [
                ([0, 1, 5, 4], Color::from_rgb8(88, 115, 135)),
                ([1, 2, 6, 5], Color::from_rgb8(116, 144, 163)),
                ([4, 5, 6, 7], Color::from_rgb8(146, 172, 189)),
            ] {
                polygon(&mut f, &indices.map(|i| part[i]), color);
            }
            let offset = 4. + 9. * (0.5 - 0.5 * (phase * std::f32::consts::TAU).cos());
            let mut min = part_min;
            let mut max = part_max;
            if positive {
                max[axis] += offset;
            } else {
                min[axis] -= offset;
            }
            let stock = vertices(min, max);
            for (a, b) in [
                (0, 1),
                (1, 2),
                (2, 3),
                (3, 0),
                (4, 5),
                (5, 6),
                (6, 7),
                (7, 4),
                (0, 4),
                (1, 5),
                (2, 6),
                (3, 7),
            ] {
                line(&mut f, stock[a], stock[b], gold, 1.2);
            }
            let mut from = [0., 0., 12.];
            from[axis] = if positive {
                part_max[axis]
            } else {
                part_min[axis]
            };
            let mut to = from;
            to[axis] += if positive { offset } else { -offset };
            line(&mut f, from, to, cyan, 3.);
            for v in [from, to] {
                f.fill(&canvas::Path::circle(p(v), 2.4), cyan);
            }
            f.fill_text(canvas::Text {
                content: "EXTRA MATERIAL ON ONE SIDE".into(),
                position: Point::new(8., 7.),
                color: gold,
                size: 10.into(),
                ..Default::default()
            });
            return vec![f.into_geometry()];
        }
        if matches!(self.0, Diagram::Tool) {
            let x = 119.;
            let tip_y = 126.;
            f.fill_rectangle(
                Point::new(x - 21., 12.),
                iced::Size::new(42., 31.),
                Color::from_rgb8(148, 172, 191),
            );
            f.fill_rectangle(
                Point::new(x - 7., 43.),
                iced::Size::new(14., 83.),
                Color::from_rgb8(226, 234, 240),
            );
            f.fill_rectangle(Point::new(x - 7., 85.), iced::Size::new(14., 41.), gold);
            for i in 0..5 {
                let y = 85. + ((i as f32 * 10. + time * 10.) % 41.);
                f.stroke(
                    &canvas::Path::line(Point::new(x - 7., y), Point::new(x + 7., y - 4.)),
                    canvas::Stroke::default()
                        .with_color(Color::from_rgb8(155, 108, 39))
                        .with_width(2.),
                );
            }
            for (x, a, b, label) in [(160., 43., tip_y, "Stickout"), (74., 85., tip_y, "Flutes")] {
                f.stroke(
                    &canvas::Path::line(Point::new(x, a), Point::new(x, b)),
                    canvas::Stroke::default().with_color(cyan).with_width(1.5),
                );
                for y in [a, b] {
                    f.stroke(
                        &canvas::Path::line(Point::new(x - 4., y), Point::new(x + 4., y)),
                        canvas::Stroke::default().with_color(cyan).with_width(1.5),
                    );
                }
                f.fill_text(canvas::Text {
                    content: label.into(),
                    position: Point::new(if x > 100. { x + 8. } else { 13. }, (a + b) / 2. - 6.),
                    color: cyan,
                    size: 11.into(),
                    ..Default::default()
                });
            }
            f.fill_text(canvas::Text {
                content: "HOLDER".into(),
                position: Point::new(158., 20.),
                color: Color::WHITE,
                size: 10.into(),
                ..Default::default()
            });
            f.stroke(
                &canvas::Path::line(Point::new(x - 7., 134.), Point::new(x + 7., 134.)),
                canvas::Stroke::default().with_color(gold).with_width(2.),
            );
            f.fill_text(canvas::Text {
                content: "CUTTING DIAMETER".into(),
                position: Point::new(139., 130.),
                color: gold,
                size: 9.into(),
                ..Default::default()
            });
            return vec![f.into_geometry()];
        }
        let s = match self.0 {
            Diagram::Operation(s) => s,
            Diagram::Stepover => Strategy::Face,
            Diagram::Stepdown | Diagram::Allowance | Diagram::Ramp => Strategy::Pocket,
            Diagram::Orientation => Strategy::Indexed,
            Diagram::Heights | Diagram::Offsets => Strategy::Drill,
            Diagram::Tool
            | Diagram::StockOffset(_, _)
            | Diagram::MillingDirection
            | Diagram::MillingChoice(..)
            | Diagram::Spindle(_)
            | Diagram::FacingPattern(_)
            | Diagram::FacingLink(_)
            | Diagram::Entry(_)
            | Diagram::DrillCycle(_)
            | Diagram::FixtureClearance
            | Diagram::Containment(_) => Strategy::Face,
        };
        let orientation = matches!(
            self.0,
            Diagram::Orientation | Diagram::Operation(Strategy::Indexed)
        );
        let angle = if orientation {
            (phase * 2.5).min(1.) * std::f32::consts::FRAC_PI_2
        } else {
            0.
        };
        let pose = |a: [f32; 3]| {
            if orientation {
                [
                    a[0],
                    a[1] * angle.cos() - (a[2] - 12.) * angle.sin(),
                    12. + a[1] * angle.sin() + (a[2] - 12.) * angle.cos(),
                ]
            } else {
                a
            }
        };
        if s != Strategy::Rotary {
            for (face, c) in [
                (
                    [
                        [-40., -24., 0.],
                        [40., -24., 0.],
                        [40., -24., 24.],
                        [-40., -24., 24.],
                    ],
                    Color::from_rgb8(88, 115, 135),
                ),
                (
                    [
                        [40., -24., 0.],
                        [40., 24., 0.],
                        [40., 24., 24.],
                        [40., -24., 24.],
                    ],
                    Color::from_rgb8(103, 132, 153),
                ),
                (
                    [
                        [-40., 24., 0.],
                        [40., 24., 0.],
                        [40., 24., 24.],
                        [-40., 24., 24.],
                    ],
                    Color::from_rgb8(116, 144, 163),
                ),
                (
                    [
                        [-40., -24., 24.],
                        [40., -24., 24.],
                        [40., 24., 24.],
                        [-40., 24., 24.],
                    ],
                    Color::from_rgb8(146, 170, 186),
                ),
            ] {
                polygon(&mut f, &face.map(pose), c);
            }
        }
        if s == Strategy::Rotary {
            let spin = std::f32::consts::FRAC_PI_2 - phase * std::f32::consts::TAU * 4.;
            let cyl = |x: f32, a: f32| [x, 18. * a.cos(), 20. + 18. * a.sin()];
            for i in 0..32 {
                let a = i as f32 * std::f32::consts::TAU / 32.;
                let b = (i + 1) as f32 * std::f32::consts::TAU / 32.;
                let shade = 0.54 + 0.10 * (a + spin).sin();
                polygon(
                    &mut f,
                    &[cyl(-40., a), cyl(40., a), cyl(40., b), cyl(-40., b)],
                    Color::from_rgb(shade, shade + 0.10, shade + 0.16),
                );
            }
            let cap: Vec<_> = (0..32)
                .map(|i| cyl(40., i as f32 * std::f32::consts::TAU / 32.))
                .collect();
            polygon(&mut f, &cap, Color::from_rgb8(124, 155, 177));
            line(&mut f, [40., 0., 20.], cyl(40., spin), gold, 2.);
        }
        if matches!(
            s,
            Strategy::Parallel | Strategy::FiveAxis | Strategy::Scallop | Strategy::Waterline
        ) {
            // A curved surface makes surface-following and changing tilt visible.
            for yi in 0..8 {
                for xi in 0..12 {
                    let x = -36. + xi as f32 * 6.;
                    let y = -20. + yi as f32 * 5.;
                    let surf =
                        |x: f32, y: f32| [x, y, 25. + 8. * (x / 26.).cos() * (y / 28.).cos()];
                    polygon(
                        &mut f,
                        &[
                            surf(x, y),
                            surf(x + 6., y),
                            surf(x + 6., y + 5.),
                            surf(x, y + 5.),
                        ],
                        Color::from_rgb8(
                            133 + (xi * 2) as u8,
                            161 + (xi * 2) as u8,
                            181 + (xi * 2) as u8,
                        ),
                    );
                }
            }
        }
        if matches!(s, Strategy::Pocket | Strategy::Adaptive) {
            polygon(
                &mut f,
                &[
                    [-28., -16., 24.],
                    [28., -16., 24.],
                    [24., -12., 13.],
                    [-24., -12., 13.],
                ],
                Color::from_rgb8(71, 96, 117),
            );
            polygon(
                &mut f,
                &[
                    [-24., -12., 13.],
                    [24., -12., 13.],
                    [24., 12., 13.],
                    [-24., 12., 13.],
                ],
                Color::from_rgb8(102, 128, 146),
            );
        }
        let n = 180usize;
        let mut path = vec![];
        for i in 0..=n {
            let t = i as f32 / n as f32;
            let a = t * std::f32::consts::TAU;
            let q = match s {
                Strategy::Face | Strategy::Parallel | Strategy::Indexed => {
                    let row = (t * 6.).floor().min(5.);
                    let x = if row as i32 % 2 == 0 {
                        -37. + 74. * (t * 6. - row)
                    } else {
                        37. - 74. * (t * 6. - row)
                    };
                    [
                        x,
                        -20. + row * 8.,
                        if s == Strategy::Parallel {
                            26. + 8. * (x / 26.).cos() * ((-20. + row * 8.) / 28.).cos()
                        } else {
                            25.
                        },
                    ]
                }
                Strategy::Adaptive => {
                    let r = 5. + 14. * t;
                    [
                        r * (a * 4.).cos() + 5. * (a * 15.).cos(),
                        r * 0.65 * (a * 4.).sin() + 3. * (a * 15.).sin(),
                        15.,
                    ]
                }
                Strategy::Pocket | Strategy::Scallop => {
                    let r = 5. + 21. * t;
                    [
                        r * (a * 4.).cos(),
                        r * 0.55 * (a * 4.).sin(),
                        if matches!(self.0, Diagram::Stepdown) {
                            24. - 3. * (t * 4.).floor().min(3.)
                        } else if matches!(self.0, Diagram::Ramp) {
                            24. - 12. * t
                        } else if s == Strategy::Scallop {
                            let x = r * (a * 4.).cos();
                            let y = r * 0.55 * (a * 4.).sin();
                            26. + 8. * (x / 26.).cos() * (y / 28.).cos()
                        } else {
                            15.
                        },
                    ]
                }
                Strategy::Contour => [44. * a.cos(), 27. * a.sin(), 21.],
                Strategy::Drill => {
                    let hole = (t * 4.).floor().min(3.) as usize;
                    let xy = [[-23., -12.], [23., -12.], [23., 12.], [-23., 12.]][hole];
                    let k = (t * 4.).fract();
                    [xy[0], xy[1], 40. - 28. * (k * std::f32::consts::PI).sin()]
                }
                Strategy::Waterline => {
                    let level = (t * 4.).floor().min(3.);
                    [42. * (a * 4.).cos(), 26. * (a * 4.).sin(), 23. - level * 5.]
                }
                Strategy::Rotary => {
                    let angle =
                        a * 4. + std::f32::consts::FRAC_PI_2 - phase * std::f32::consts::TAU * 4.;
                    [-32. + 64. * t, 19. * angle.cos(), 20. + 19. * angle.sin()]
                }
                Strategy::FiveAxis => {
                    let x = -34. + 68. * t;
                    let y = 12. * a.sin();
                    [x, y, 26. + 8. * (x / 26.).cos() * (y / 28.).cos()]
                }
            };
            // Indexed cutting only starts after the workpiece is held in its new position.
            path.push(if orientation {
                [q[0], q[1] * 0.5, 38.]
            } else {
                q
            });
        }
        if matches!(self.0, Diagram::Heights) {
            for (z, c) in [(47., gold), (37., cyan)] {
                for (a, b) in [
                    ([-40., -24., z], [40., -24., z]),
                    ([40., -24., z], [40., 24., z]),
                    ([40., 24., z], [-40., 24., z]),
                ] {
                    line(&mut f, a, b, c, 1.);
                }
            }
        }
        if matches!(self.0, Diagram::Offsets) {
            for (z, c) in [
                (24., cyan),
                (7., cyan),
                (7. + 5. * (phase * std::f32::consts::TAU).sin(), gold),
            ] {
                line(&mut f, [-40., 24., z], [40., 24., z], c, 2.);
            }
            f.fill_text(canvas::Text {
                content: "REFERENCE / OFFSET PLANE".into(),
                position: Point::new(8., 8.),
                color: gold,
                size: 10.into(),
                ..Default::default()
            });
        }
        let cutting_phase = if orientation {
            ((phase - 0.4) / 0.6).max(0.)
        } else {
            phase
        };
        let end = ((cutting_phase * n as f32) as usize).min(n);
        for (i, pair) in path.windows(2).enumerate() {
            if orientation && phase < 0.4 {
                continue;
            }
            line(
                &mut f,
                pair[0],
                pair[1],
                if i <= end {
                    cyan
                } else {
                    Color::from_rgba8(35, 101, 137, 0.38)
                },
                if i <= end { 1.6 } else { 0.8 },
            );
        }
        if matches!(
            self.0,
            Diagram::Stepover | Diagram::Stepdown | Diagram::Allowance
        ) {
            let (a, b, label) = match self.0 {
                Diagram::Stepover => ([28., -20., 26.], [28., -12., 26.], "SIDEWAYS SPACING"),
                Diagram::Stepdown => ([29., 10., 24.], [29., 10., 21.], "DEPTH PER LAYER"),
                _ => ([27., -16., 25.], [33., -16., 25.], "STOCK TO LEAVE"),
            };
            line(&mut f, a, b, gold, 3.);
            for a in [a, b] {
                f.fill(&canvas::Path::circle(p(a), 2.5), gold)
            }
            f.fill_text(canvas::Text {
                content: label.into(),
                position: Point::new(8., 8.),
                color: gold,
                size: 10.into(),
                ..Default::default()
            });
        }
        let tip = if orientation && phase < 0.4 {
            [-30., -20., 47.]
        } else {
            path[end]
        };
        let tilt = if s == Strategy::FiveAxis {
            0.5 * (phase * std::f32::consts::TAU).sin()
        } else {
            0.
        };
        let endtool = [tip[0] + tilt * 23., tip[1], tip[2] + 23.];
        let mid = [tip[0] + tilt * 11., tip[1], tip[2] + 11.];
        line(&mut f, mid, endtool, Color::from_rgb8(221, 231, 236), 9.);
        line(&mut f, tip, mid, gold, 7.);
        f.fill(&canvas::Path::circle(p(tip), 3.2), gold);
        for z in 0..4 {
            let z = z as f32 * 2.6;
            let a = [tip[0] + tilt * z, tip[1], tip[2] + z];
            let q = p(a);
            f.stroke(
                &canvas::Path::line(
                    Point::new(q.x - 3., q.y + 1.),
                    Point::new(q.x + 3., q.y - 2.),
                ),
                canvas::Stroke::default()
                    .with_color(Color::from_rgb8(142, 100, 40))
                    .with_width(1.),
            );
        }
        let caption = if orientation {
            "REPOSITION → HOLD → CUT"
        } else if matches!(self.0, Diagram::Heights) {
            "AMBER: CLEARANCE   CYAN: RETRACT"
        } else if matches!(self.0, Diagram::Offsets) {
            "CYAN: GEOMETRY   AMBER: OFFSET"
        } else if s == Strategy::FiveAxis {
            "TOOL AXIS CHANGES ALONG THE SURFACE"
        } else if s == Strategy::Rotary {
            "ROTARY SWEEP AROUND THE AXIS"
        } else {
            "CYAN: CUTTING MOTION    AMBER: CUTTER"
        };
        f.fill_text(canvas::Text {
            content: caption.into(),
            position: Point::new(8., 143.),
            color: Color::from_rgb8(185, 207, 223),
            size: 9.into(),
            ..Default::default()
        });
        vec![f.into_geometry()]
    }
}

// Renderer smoke preview uses exactly the same cards as hover overlays.
pub fn gallery<'a>(kind: u8) -> Element<'a, Message> {
    let cards: Vec<Help> = if kind >= 5 {
        crate::choice_help::gallery(kind)
    } else if kind == 4 {
        [
            ("pocket_milling", "Pocket milling direction"),
            ("pocket_entry", "Pocket entry"),
            ("spindle_rotation", "Spindle rotation"),
        ]
        .iter()
        .map(|(k, l)| field_help(k, l, Target::Operation))
        .collect()
    } else if kind == 3 {
        [
            ("facing_milling", "Milling direction"),
            ("spindle_rotation", "Spindle rotation"),
            ("facing_pattern", "Pass pattern"),
        ]
        .iter()
        .map(|(k, l)| field_help(k, l, Target::Operation))
        .collect()
    } else if kind == 2 {
        crate::stock::OFFSETS
            .iter()
            .map(|(key, label)| field_help(key, label, Target::Setup))
            .collect()
    } else if kind == 1 {
        [
            ("clearance", "Fixture clearance · mm", Target::Setup),
            ("clearance", "Clearance above stock · mm", Target::Operation),
            ("stepover", "Stepover · % tool diameter", Target::Operation),
            ("stepdown", "Maximum stepdown · mm", Target::Operation),
            ("stickout", "Stickout · mm", Target::Tool),
            ("bottom", "Bottom offset · mm", Target::Operation),
        ]
        .iter()
        .map(|(k, l, target)| field_help(k, l, *target))
        .collect()
    } else {
        [
            Strategy::Face,
            Strategy::Adaptive,
            Strategy::Drill,
            Strategy::Indexed,
            Strategy::Rotary,
            Strategy::FiveAxis,
        ]
        .iter()
        .map(|s| command(&Message::AddOp(*s), s.name().into()))
        .collect()
    };
    let mut rows = widget::Column::new().spacing(10);
    let mut row = widget::Row::new().spacing(10);
    for (i, h) in cards.into_iter().enumerate() {
        row = row.push(card(h));
        if i % 3 == 2 {
            rows = rows.push(row);
            row = widget::Row::new().spacing(10)
        }
    }
    container(widget::scrollable(rows))
        .padding(20)
        .center(Length::Fill)
        .into()
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_commands_have_specific_help() {
        let generic = command(&Message::Noop, "x".into()).body;
        for message in [
            Message::SignIn,
            Message::SignInCancel,
            Message::SignOut,
            Message::CheckConnection,
            Message::LoadModels,
            Message::PlanNoticeDismissed,
            Message::OpenUrl(crate::openai::USAGE_URL),
        ] {
            let help = command(&message, "x".into());
            assert_ne!(help.body, generic, "{message:?}");
        }
        assert!(
            command(&Message::SignIn, "x".into())
                .body
                .contains("name, email and profile picture")
        );
        assert!(
            command(&Message::CheckConnection, "x".into())
                .body
                .contains("ChatGPT plan")
        );
        assert_eq!(
            command(&Message::SignOut, "x".into()).body,
            "Sign out of ChatGPT on this computer and ask OpenAI to revoke this sign-in. The saved sign-in is deleted."
        );
    }
    #[test]
    fn material_commands_and_fields_have_specific_help() {
        use crate::Page::Materials;
        let generic = command(&Message::Noop, "x".into()).body;
        for message in [
            Message::NewMaterial,
            Message::EditMaterial("m".into()),
            Message::SaveMaterial,
            Message::DeleteMaterial("m".into()),
            Message::PickMaterial("m".into()),
            Message::Page(Materials),
        ] {
            let body = command(&message, "x".into()).body;
            assert_ne!(body, generic, "{message:?}");
            for claim in ["collision", "gouge", "cutting load", "cutting-load"] {
                assert!(!body.contains(claim), "{message:?}: {body}");
            }
        }
        let pick = command(&Message::PickMaterial("m".into()), "x".into()).body;
        assert!(
            pick.contains(
                "its name, group, hardness, notes and cutting data are sent to the AI planner"
            ),
            "{pick}"
        );
        // What the planner is sent of the project material is said on every
        // field that goes: the name, group, hardness and notes.
        for key in ["name", "group", "hardness", "notes"] {
            let body = field_help(key, "x", Target::Material).body;
            assert!(
                body.contains("project material") && body.contains("sent to the AI planner"),
                "{key}: {body}"
            );
        }
        assert!(
            field_help("name", "x", Target::Material)
                .body
                .contains("up to 60 characters")
        );
        assert!(
            command(&Message::DeleteMaterial("m".into()), "x".into())
                .body
                .contains("project material")
        );
        let unknown = field_help("zz", "x", Target::Material).body;
        assert!(unknown.starts_with("Saved parameter"));
        let keys = ["name", "group", "hardness", "notes", "vc_min", "vc_max"]
            .into_iter()
            .map(String::from)
            .chain(
                ["fz3", "fz6", "fz10", "fz16", "fzbig"]
                    .into_iter()
                    .flat_map(|b| [format!("{b}_min"), format!("{b}_max")]),
            );
        for key in keys {
            let help = field_help(&key, "Minimum", Target::Material);
            assert_ne!(help.body, unknown, "{key}");
            assert!(!help.body.contains("reclamp"), "{key}: {}", help.body);
        }
        // A cutting-data field names itself in full, whatever its label says.
        assert_eq!(
            field_help("fz6_min", "Minimum", Target::Material).title,
            "Chip load up to 6 mm, minimum"
        );
        assert_eq!(
            field_help("vc_max", "Maximum", Target::Material).title,
            "Surface speed, maximum"
        );
        assert_eq!(
            field_help("name", "Material name", Target::Material).title,
            "Material name"
        );
        assert_eq!(
            field_help("fz6_min", "Minimum", Target::Tool).title,
            "Minimum"
        );
    }
    #[test]
    fn workholding_commands_and_fields_have_specific_help() {
        use crate::Page::Workholding;
        let generic = command(&Message::Noop, "x".into()).body;
        for message in [
            Message::NewHolder("Vise"),
            Message::NewHolder("Clamp"),
            Message::EditHolder("h".into()),
            Message::SaveHolder,
            Message::DeleteHolder("h".into()),
            Message::SuggestLayout,
            Message::Page(Workholding),
        ] {
            let body = command(&message, "x".into()).body;
            assert_ne!(body, generic, "{message:?}");
        }
        let suggest = command(&Message::SuggestLayout, "x".into()).body;
        assert!(suggest.contains("never changed"), "{suggest}");
        assert!(suggest.contains("middle and quarter points"), "{suggest}");
        assert!(suggest.contains("parallels, bolts, the table"), "{suggest}");
        let delete = command(&Message::DeleteHolder("h".into()), "x".into()).body;
        assert!(delete.contains("setup uses"), "{delete}");
        let unknown = field_help("zz", "x", Target::Holder).body;
        assert!(unknown.starts_with("Saved parameter"));
        for key in [
            "name",
            "notes",
            "jaw_width",
            "jaw_height",
            "jaw_thickness",
            "max_opening",
            "grip",
            "length",
            "width",
            "height",
            "bite",
        ] {
            let body = field_help(key, "x", Target::Holder).body;
            assert_ne!(body, unknown, "{key}");
            // The tool's and the operator's texts of the same keys are not shown.
            for other in ["flutes", "reclamp", "holder diameter"] {
                assert!(!body.contains(other), "{key}: {body}");
            }
        }
        assert!(
            field_help("name", "x", Target::Holder)
                .body
                .contains("up to 60 characters")
        );
        for key in [
            "holding",
            "holder",
            "vise_axis",
            "vise_offset",
            "vise_grip",
            "holding_gap",
            "clamps",
        ] {
            let body = field_help(key, "x", Target::Setup).body;
            assert_ne!(body, unknown, "{key}");
            // Only the texts that say what the check covers name the cutter.
            if !matches!(key, "holding" | "holding_gap") {
                assert!(!body.contains("cutter"), "{key}: {body}");
            }
        }
        // Only the texts about the setup's workholding and its gap say that
        // toolpaths are checked against it; none promises what the model
        // leaves free.
        for (target, key) in [
            (Target::Setup, "holding"),
            (Target::Setup, "holder"),
            (Target::Setup, "vise_axis"),
            (Target::Setup, "vise_offset"),
            (Target::Setup, "vise_grip"),
            (Target::Setup, "holding_gap"),
            (Target::Setup, "clamps"),
            (Target::Holder, "jaw_width"),
            (Target::Holder, "jaw_height"),
            (Target::Holder, "jaw_thickness"),
            (Target::Holder, "max_opening"),
            (Target::Holder, "grip"),
            (Target::Holder, "length"),
            (Target::Holder, "bite"),
        ] {
            let body = field_help(key, "x", target).body;
            for claim in ["free to cut", "safe"] {
                assert!(!body.contains(claim), "{key}: {body}");
            }
            if !matches!(key, "holding" | "holding_gap") {
                for claim in ["check", "refus", "toolpath"] {
                    assert!(!body.contains(claim), "{key}: {body}");
                }
            }
        }
        assert_eq!(
            field_help("holding_gap", "x", Target::Setup).body,
            "The clearance to keep around the jaws or clamps, in mm. 0 or more; left empty it is 2 mm. A move whose cutter or holder comes within it is warned of in the operation preview (that operation's own toolpath) and refused at export (every program move, the links between operations, approaches and retracts included)."
        );
        // What the check is, and what it is not, in the field that chooses it.
        let holding = field_help("holding", "x", Target::Setup).body;
        for part in [
            "every move is checked against them",
            "the cutter from its tip up to its stickout at its cutting diameter",
            "the tool holder above that at the holder diameter",
            "a wider shank or a reduced neck is not modelled",
            "warned of in the operation preview, which covers that operation's own toolpath, and refused at export, which checks every program move, the links between operations, approaches and retracts included",
            "Parallels, bolts, the table and the machine body are not modelled",
            "In a later setup the jaws or clamps sit at the stock box carried over from setup 1, not at the walls earlier operations cut",
            "the Fixture note below is only a description",
        ] {
            assert!(holding.contains(part), "{part}: {holding}");
        }
        // The old fixture description and clearance are records only; the
        // texts that said fixtures are never checked say what is.
        let fixture = field_help("fixture", "x", Target::Setup).body;
        assert!(
            fixture.contains("is not checked") && fixture.contains("Modelled workholding"),
            "{fixture}"
        );
        let clearance = field_help("clearance", "x", Target::Setup).body;
        assert!(
            clearance.contains("only saved with the setup")
                && clearance.contains("uses the Clearance gap beside them instead")
                && !clearance.contains("not connected yet"),
            "{clearance}"
        );
        let heights = field_help("clearance", "x", Target::Operation).body;
        assert!(
            heights.contains("export refuses a setup whose plane")
                && heights.contains("every modelled jaw and clamp and its gap"),
            "{heights}"
        );
        let post = command(&Message::Page(crate::Page::Post), "x".into()).body;
        for part in [
            "in a setup that models a vise or clamps, the jaws or clamps as boxes with a clearance gap over every program move, links, approaches and retracts included",
            "parallels, bolts, the table, the machine body and other fixtures are not checked",
        ] {
            assert!(post.contains(part), "{part}: {post}");
        }
        let page = command(&Message::Page(Workholding), "x".into()).body;
        assert!(
            page.contains("checked as boxes with a clearance gap"),
            "{page}"
        );
        assert!(
            field_help("vise_grip", "x", Target::Setup)
                .body
                .contains("the stock above it is not covered by the modelled jaws")
        );
        let clamps = field_help("clamps", "x", Target::Setup).body;
        for part in ["side:offset", "-X", "+X", "-Y", "+Y", "8"] {
            assert!(clamps.contains(part), "{part}: {clamps}");
        }
        // The tool's holder diameter and flute length keep their own help.
        assert!(
            field_help("holder", "x", Target::Tool)
                .body
                .contains("Maximum holder diameter")
        );
        assert!(
            field_help("length", "x", Target::Tool)
                .body
                .contains("flutes")
        );
    }
    #[test]
    fn speed_and_feed_help_says_what_the_material_check_does() {
        let rpm = field_help("rpm", "Spindle speed", Target::Operation).body;
        let feed = field_help("feed", "Cutting feed", Target::Operation).body;
        for body in [&rpm, &feed] {
            assert!(
                body.contains("project material has cutting data")
                    && body.contains("flagged on this tab and on the NC program page")
                    && body.contains("export still goes ahead"),
                "{body}"
            );
            assert!(!body.contains("recommended speed"), "{body}");
        }
        assert!(rpm.contains("π × tool diameter × rpm ÷ 1000"), "{rpm}");
        assert!(
            feed.contains("Plunge and ramp feeds are not checked"),
            "{feed}"
        );
        let post = command(&Message::Page(crate::Page::Post), "x".into()).body;
        assert!(
            post.contains("outside the project material's cutting data")
                && post.contains("never block export"),
            "{post}"
        );
        let plan = command(&Message::Plan, "x".into()).body;
        assert!(
            plan.contains("speeds and feeds against the project material's cutting data"),
            "{plan}"
        );
    }
    #[test]
    fn planner_commands_describe_the_live_planner() {
        let generic = command(&Message::Noop, "x".into()).body;
        for message in [
            Message::Plan,
            Message::PlanCancel,
            Message::ApplyPlan,
            Message::DiscardPlan,
            Message::UndoPlan,
            Message::Page(crate::Page::Ai),
        ] {
            let body = command(&message, "x".into()).body;
            assert_ne!(body, generic, "{message:?}");
            for stale in ["not connected", "example plan", "sample housing"] {
                assert!(!body.contains(stale), "{message:?}: {body}");
            }
        }
        let plan = command(&Message::Plan, "x".into()).body;
        assert!(plan.contains("four requests") && plan.contains("Nothing changes until you apply"));
        assert!(
            command(&Message::ApplyPlan, "x".into())
                .body
                .contains("History")
        );
        let undo = command(&Message::UndoPlan, "x".into()).body;
        assert!(undo.contains("Tools, machines") && undo.contains("saved to History first"));
        assert!(
            command(&Message::Page(crate::Page::Ai), "x".into())
                .body
                .contains("Sign in with ChatGPT")
        );
    }
}
