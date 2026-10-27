use crate::{
    App, Message, Page, Target, chamfer,
    help::{self, Help, HelpButton},
    holding::Held,
    lathe, manual,
    material::SpeedFeed,
    model::*,
    planner::Dropped,
    settings::{Layout, Side},
    viewport::{self, Viewport},
};
use iced::{
    Alignment, Border, Color, Element, Font,
    Length::{self, Fill},
    Point, Rectangle, Theme, mouse,
    widget::{
        self, Action, Column, Row, button, canvas, checkbox, column, container, mouse_area,
        pick_list, responsive, row, rule, scrollable, slider, space, svg, text, text::Wrapping,
        text_input,
    },
};
pub(crate) const INK: Color = Color::from_rgb(0.13, 0.20, 0.28);
/// Secondary text. At least 4.5:1 on every surface below
/// (`muted_text_meets_contrast_on_every_surface`).
pub(crate) const MUTED: Color = Color::from_rgb(0.30, 0.37, 0.44);
pub(crate) const BLUE: Color = Color::from_rgb(0.13, 0.42, 0.71);
pub(crate) const LINE: Color = Color::from_rgb(0.85, 0.88, 0.91);
pub(crate) const BG: Color = Color::from_rgb(0.95, 0.96, 0.975);
// The fills that MUTED text is drawn on besides white panels and BG.
const SELECTED_ROW: Color = Color::from_rgb8(218, 235, 249);
const ACTIVE_TILE: Color = Color::from_rgb8(223, 237, 249);
const SETUP_ROW: Color = Color::from_rgb8(230, 239, 248);
const HOVER_ROW: Color = Color::from_rgb8(239, 244, 249);
pub(crate) const HOVER_BUTTON: Color = Color::from_rgb8(235, 240, 246);
/// The opacity of an icon whose command is unavailable.
const DIMMED: f32 = 0.45;
/// The label of an unavailable command: lighter than `MUTED`, so it reads as
/// greyed out beside its neighbours, and still 3:1 on white.
const DIMMED_TEXT: Color = Color::from_rgb8(128, 140, 152);
/// The amber of a warning: its box, its heading and its text.
pub(crate) const WARNING: [Color; 3] = [
    Color::from_rgb8(255, 247, 232),
    Color::from_rgb8(166, 101, 32),
    Color::from_rgb8(141, 90, 39),
];
/// The red of an error: its box, its heading and its text.
const ERROR: [Color; 3] = [
    Color::from_rgb8(253, 236, 236),
    Color::from_rgb8(172, 44, 44),
    Color::from_rgb8(146, 48, 48),
];
/// The inspector's scrollable, by id: a Tab with nothing focused enters its
/// first field (`control::focus_first`).
pub(crate) const INSPECTOR_ID: &str = "inspector";
pub(crate) fn bold<'a>(s: impl Into<String>, size: u32) -> widget::Text<'a> {
    text(s.into()).size(size).font(Font {
        weight: iced::font::Weight::Semibold,
        ..Font::DEFAULT
    })
}
pub(crate) fn muted<'a>(s: impl Into<String>) -> widget::Text<'a> {
    text(s.into()).size(12).color(MUTED)
}
fn panel_style(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::WHITE.into()),
        border: Border {
            color: LINE,
            width: 1.,
            radius: 5.into(),
        },
        text_color: Some(INK),
        ..Default::default()
    }
}
pub(crate) fn btn<'a>(label: impl Into<String>, msg: Message, active: bool) -> HelpButton<'a> {
    let label = label.into();
    let action = msg.clone();
    let b = button(text(label.clone()).size(12))
        .on_press(msg)
        .padding([7, 10])
        .style(move |_, status| flat(active, status));
    HelpButton::new(b, &action, label)
}
/// A flat button's look: blue while `active`, shaded under the pointer.
fn flat(active: bool, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(
            if active {
                ACTIVE_TILE
            } else if hovered {
                HOVER_BUTTON
            } else {
                Color::TRANSPARENT
            }
            .into(),
        ),
        text_color: if active { BLUE } else { INK },
        border: Border {
            radius: 4.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
/// The top and bottom padding of a filled or outlined button: the same, so
/// one stands as tall as the other.
const BUTTON_PAD: u16 = 8;
/// The edge of an outlined button: darker than `LINE`, so the button is seen
/// as one on a white panel.
pub(crate) const BUTTON_EDGE: Color = Color::from_rgb8(190, 201, 212);
/// The edge of a button that deletes.
const DANGER_EDGE: Color = Color::from_rgb8(226, 172, 172);
/// A command beside other content: white with a border, so it reads as a
/// button and not as a label. `btn` stays flat for tabs, tiles and list rows,
/// `primary` is the one filled command of a region.
pub(crate) fn secondary<'a>(label: impl Into<String>, msg: Message) -> HelpButton<'a> {
    outlined_btn(label.into(), msg, false)
}
/// A command that deletes or discards: red text and edge, red tint under the
/// pointer, so it is not mistaken for its safe neighbours.
pub(crate) fn danger<'a>(label: impl Into<String>, msg: Message) -> HelpButton<'a> {
    outlined_btn(label.into(), msg, true)
}
fn outlined_btn<'a>(label: String, msg: Message, deletes: bool) -> HelpButton<'a> {
    let action = msg.clone();
    // The text and height of `primary`'s, so the two line up in a row.
    let b = button(text(label.clone()).size(13))
        .on_press(msg)
        .padding([BUTTON_PAD, 12])
        .style(move |_, status| outlined(deletes, status));
    HelpButton::new(b, &action, label)
}
/// The line icons of `icon_btn`, drawn black and tinted to the button's ink.
pub(crate) const IC_DUPLICATE: &[u8] = include_bytes!("../assets/ic-duplicate.svg");
pub(crate) const IC_SUPPRESS: &[u8] = include_bytes!("../assets/ic-suppress.svg");
pub(crate) const IC_ENABLE: &[u8] = include_bytes!("../assets/ic-enable.svg");
pub(crate) const IC_UP: &[u8] = include_bytes!("../assets/ic-up.svg");
pub(crate) const IC_DOWN: &[u8] = include_bytes!("../assets/ic-down.svg");
pub(crate) const IC_DELETE: &[u8] = include_bytes!("../assets/ic-delete.svg");
/// `secondary` or `danger` with an icon for its text, for a row of commands
/// too many to name: `label` is still the hover help's title.
pub(crate) fn icon_btn<'a>(
    icon: &'static [u8],
    label: impl Into<String>,
    msg: Message,
    deletes: bool,
) -> HelpButton<'a> {
    let action = msg.clone();
    let ink = if deletes { ERROR[1] } else { INK };
    // 17 px: the line height of `outlined_btn`'s 13 px text, so a text
    // button beside it stands as tall.
    let b = button(
        svg(svg::Handle::from_memory(icon))
            .width(17)
            .height(17)
            .style(move |_, _| svg::Style { color: Some(ink) }),
    )
    .on_press(msg)
    .padding([BUTTON_PAD, 10])
    .style(move |_, status| outlined(deletes, status));
    HelpButton::new(b, &action, label.into())
}
fn outlined(deletes: bool, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let (ink, edge, hover) = if deletes {
        (ERROR[1], DANGER_EDGE, ERROR[0])
    } else {
        (INK, BUTTON_EDGE, HOVER_BUTTON)
    };
    button::Style {
        background: Some(if hovered { hover } else { Color::WHITE }.into()),
        text_color: ink,
        border: Border {
            color: if hovered && deletes { ERROR[1] } else { edge },
            width: 1.,
            radius: 4.into(),
        },
        ..Default::default()
    }
}
/// A drop-down list drawn like a text field (`field_style`): Iced's own fill
/// for it is the tint of a disabled control.
pub(crate) fn choice_style(theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let mut style = pick_list::default(theme, status);
    style.background = theme.extended_palette().background.base.color.into();
    style.border.radius = 4.into();
    style
}
/// A toolbar button with its icon over the label, like Fusion 360's.
fn tool_btn<'a>(icon: &'static [u8], label: &str, msg: Message, active: bool) -> HelpButton<'a> {
    let action = msg.clone();
    let b = button(
        column![tool_icon(icon), text(label.to_string()).size(12)]
            .spacing(2)
            .align_x(Alignment::Center),
    )
    .on_press(msg)
    .padding([4, 8])
    .style(move |_, status| flat(active, status));
    HelpButton::new(b, &action, label)
}
fn tool_icon<'a>(icon: &'static [u8]) -> widget::Svg<'a> {
    // ponytail: hashes the few hundred bytes every frame; keep handles on App if that ever shows.
    svg(svg::Handle::from_memory(icon)).width(26).height(26)
}
/// A button on the dark application bar.
fn bar_btn<'a>(label: &str, msg: Message, active: bool) -> HelpButton<'a> {
    let action = msg.clone();
    let b = button(text(label.to_string()).size(12))
        .on_press(msg)
        .padding([6, 10])
        .style(move |_, status| button::Style {
            background: (active
                || matches!(status, button::Status::Hovered | button::Status::Pressed))
            .then(|| Color::from_rgba8(255, 255, 255, 0.14).into()),
            text_color: Color::from_rgb8(226, 236, 245),
            border: Border {
                radius: 4.into(),
                ..Default::default()
            },
            ..Default::default()
        });
    HelpButton::new(b, &action, label)
}
/// The workspace switcher at the start of the toolbar, like Fusion 360's:
/// MANUFACTURE plans the job, MACHINE (the Control page) runs it. While the
/// machine moves it stays on MACHINE.
/// On the dark MACHINE bar (`dark`) it is smaller, with light text.
fn switcher(app: &App, dark: bool) -> Element<'_, Message> {
    const WORKSPACES: [&str; 2] = ["MANUFACTURE", "MACHINE"];
    let font = Font {
        weight: iced::font::Weight::Bold,
        ..Font::DEFAULT
    };
    let picker: Element<_> = if app.locked() {
        row![
            container(text(WORKSPACES[1]).size(14).font(font).color(if dark {
                ON_DARK
            } else {
                MUTED
            }))
            .padding([7, 12]),
            muted("Manufacture opens when the machine stops.").color(if dark {
                ON_DARK
            } else {
                MUTED
            })
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .into()
    } else {
        let (page, back) = (app.page, app.cam_page);
        let machine = page == Page::Control;
        // Picking the open workspace again keeps its page.
        pick_list(
            WORKSPACES,
            Some(WORKSPACES[usize::from(machine)]),
            move |w| {
                Message::Page(match w {
                    "MACHINE" => Page::Control,
                    _ if machine => back,
                    _ => page,
                })
            },
        )
        .font(font)
        .text_size(if dark { 13 } else { 14 })
        .style(choice_style)
        .padding(if dark { [4, 10] } else { [7, 12] })
        .into()
    };
    if dark {
        return help::wrap(
            picker,
            Help::text(
                "Workspace",
                "MANUFACTURE plans the job: the part, setups, operations, the libraries, the AI planner and the NC review. MACHINE drives the CNC through the UC100: connect, jog, zero, probe and run a setup, and set up its profile. While the machine moves (a tool run, held or paused, a probe, a jog or homing) only MACHINE is open.",
            ),
            widget::tooltip::Position::Bottom,
        );
    }
    let icon = if app.page == Page::Control || app.locked() {
        include_bytes!("../assets/tb-machine.svg").as_slice()
    } else {
        include_bytes!("../assets/tb-manufacture.svg")
    };
    help::wrap(
        row![tool_icon(icon), picker]
            .spacing(6)
            .align_y(Alignment::Center),
        Help::text(
            "Workspace",
            "MANUFACTURE plans the job: the part, setups, operations, the libraries, the AI planner and the NC review. MACHINE drives the CNC through the UC100: connect, jog, zero, probe and run a setup, and set up its profile. While the machine moves (a tool run, held or paused, a probe, a jog or homing) only MACHINE is open.",
        ),
        // Beside it, not over the list it opens.
        widget::tooltip::Position::Right,
    )
}
pub(crate) fn primary<'a>(label: impl Into<String>, msg: Message) -> HelpButton<'a> {
    let label = label.into();
    let action = msg.clone();
    let b = button(text(label.clone()).size(13))
        .on_press(msg)
        .padding([BUTTON_PAD, 14])
        .style(|_, status| button::Style {
            background: Some(
                if matches!(status, button::Status::Hovered) {
                    Color::from_rgb8(32, 97, 155)
                } else {
                    BLUE
                }
                .into(),
            ),
            text_color: Color::WHITE,
            border: Border {
                radius: 4.into(),
                ..Default::default()
            },
            ..Default::default()
        });
    HelpButton::new(b, &action, label)
}
/// The Holding step's field for the gap the checks keep round the modelled
/// jaws and clamps, saved as `holding_gap`: its label says what it does.
pub(crate) const HOLDING_GAP_LABEL: &str = "Checked gap to jaws and clamps · mm";
/// The setup's requested fixture clearance, saved as `clearance`: a note for
/// the operator that nothing checks, and its label says so.
pub(crate) const FIXTURE_CLEARANCE_LABEL: &str = "Requested fixture clearance · mm · not checked";
/// The Holding step's section of fields that are recorded on the setup sheet
/// and in the program header and checked by nothing.
pub(crate) const SETUP_NOTES_SECTION: &str = "Setup notes · recorded, not checked";
fn field<'a>(
    p: &'a Fields,
    key: &'static str,
    label: &'static str,
    target: Target,
) -> Element<'a, Message> {
    checked_field(p, key, label, target, &[])
}
/// A text field with the checks about it: a red border for an error, amber
/// for a warning, and each message under it with its label.
fn checked_field<'a>(
    p: &'a Fields,
    key: &'static str,
    label: &'static str,
    target: Target,
    checks: &[&Check],
) -> Element<'a, Message> {
    hinted_field(p, key, label, label, target, checks)
}
/// A checked text field that shows `hint` while it is empty.
fn hinted_field<'a>(
    p: &'a Fields,
    key: &'static str,
    label: &'static str,
    hint: &'static str,
    target: Target,
    checks: &[&Check],
) -> Element<'a, Message> {
    let mark = checks
        .iter()
        .find(|c| c.error)
        .or(checks.first())
        .map(|c| c.colors()[1]);
    let mut c = column![
        muted(label),
        text_input(hint, get(p, key))
            .on_input(move |v| Message::Edit(target, key, v))
            .padding([7, 9])
            .size(13)
            .style(move |theme, status| field_style(theme, status, mark))
    ]
    .spacing(5);
    for check in checks {
        c = c.push(check_line(check, false));
    }
    help::field(c, key, label, target)
}
/// Iced's look for a text field, with the border in a check's colour when one
/// marks the field, and 2 px wide while the field has the keyboard focus, so
/// the keyboard's place is seen.
fn field_style(
    theme: &Theme,
    status: text_input::Status,
    mark: Option<Color>,
) -> text_input::Style {
    let mut style = text_input::default(theme, status);
    if let Some(color) = mark {
        style.border.color = color;
    }
    if matches!(status, text_input::Status::Focused { .. }) {
        style.border.width = 2.;
    }
    style
}
fn choice<'a>(
    p: &Fields,
    key: &'static str,
    label: &'static str,
    options: &[&str],
    target: Target,
) -> Element<'a, Message> {
    choice_value(p, get(p, key), key, label, options, target)
}
fn choice_value<'a>(
    p: &Fields,
    value: &str,
    key: &'static str,
    label: &'static str,
    options: &[&str],
    target: Target,
) -> Element<'a, Message> {
    let list: Vec<String> = options.iter().map(|s| s.to_string()).collect();
    help::wrap(
        column![
            muted(label),
            pick_list(list, Some(value.to_string()), move |v| Message::Edit(
                target, key, v
            ))
            .width(Fill)
            .padding([7, 9])
            .text_size(13)
            .style(choice_style)
        ]
        .spacing(5),
        crate::choice_help::selected(key, label, value, target, p),
        widget::tooltip::Position::Left,
    )
}
pub(crate) fn note<'a>(s: impl Into<String>) -> Element<'a, Message> {
    container(
        text(s.into())
            .size(12)
            .color(Color::from_rgb8(65, 103, 139)),
    )
    .padding(12)
    .width(Fill)
    .style(|_| container::Style {
        background: Some(Color::from_rgb8(237, 245, 252).into()),
        border: Border {
            radius: 4.into(),
            ..Default::default()
        },
        ..Default::default()
    })
    .into()
}
fn warnings<'a>(issues: Vec<String>) -> Element<'a, Message> {
    if issues.is_empty() {
        return container(
            muted("Configuration checks passed").color(Color::from_rgb8(39, 130, 93)),
        )
        .padding([8, 0])
        .into();
    }
    caution(
        format!(
            "{} configuration issue{}",
            issues.len(),
            if issues.len() == 1 { "" } else { "s" }
        ),
        issues,
    )
}
/// An amber box: a heading and one bullet per line.
pub(crate) fn caution<'a>(heading: impl Into<String>, lines: Vec<String>) -> Element<'a, Message> {
    let mut c = column![bold(heading, 12).color(Color::from_rgb8(166, 101, 32))].spacing(5);
    for line in lines {
        c = c.push(
            text(format!("• {line}"))
                .size(12)
                .color(Color::from_rgb8(141, 90, 39)),
        )
    }
    container(c)
        .padding(12)
        .style(|_| container::Style {
            background: Some(Color::from_rgb8(255, 247, 232).into()),
            border: Border {
                radius: 4.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .width(Fill)
        .into()
}
/// One line of the selected operation's checks: what it says; whether it is
/// an error, which stops Generate and the export, or a warning, which only
/// advises (a speed or feed outside the material's cutting data); and the
/// text field its message names, when it names one (`model::issue_field`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Check {
    pub text: String,
    pub error: bool,
    pub key: Option<&'static str>,
}
impl Check {
    fn new(text: String, error: bool) -> Self {
        Self {
            key: issue_field(&text),
            text,
            error,
        }
    }
    /// Its label: Error or Warning.
    pub(crate) fn label(&self) -> &'static str {
        if self.error { "Error" } else { "Warning" }
    }
    fn colors(&self) -> [Color; 3] {
        if self.error { ERROR } else { WARNING }
    }
}
/// The checks of the selected operation: its configuration issues as errors,
/// then its speed and feed problems as warnings.
pub(crate) fn op_checks(app: &App) -> Vec<Check> {
    let mut checks: Vec<Check> = app
        .op_issues()
        .into_iter()
        .map(|text| Check::new(text, true))
        .collect();
    if let Some(o) = app.operation()
        && let Some(SpeedFeed::Out(problems)) = app.doc.speed_feed(app.doc.setup(), o)
    {
        checks.extend(problems.into_iter().map(|text| Check::new(text, false)));
    }
    if let Some(o) = app.operation()
        && let Some(m) = app.doc.selected_material()
        && let Some(text) = crate::finishing::issue(m, o)
    {
        checks.push(Check::new(text, false));
    }
    // Holes a Pocket picked by their walls that its cutter cannot get into.
    if let Some(o) = app.operation()
        && matches!(o.strategy, Strategy::Pocket | Strategy::Contour)
        && let Some(mesh) = &app.imported_mesh
        && let Some((diameter, fits, cleared)) = app.pocket_fits(o, mesh).as_ref()
        && let Ok(holes) = app.pocket_holes(o, mesh).as_ref()
    {
        for ((h, fits), cleared) in holes.iter().zip(fits).zip(cleared) {
            checks.extend(
                h.uncut(*diameter, fits, *cleared)
                    .into_iter()
                    .map(|text| Check::new(text, false)),
            );
        }
    }
    // A generated outside Contour that goes through the stock's bottom all
    // the way round leaves the part loose for the cutter to throw.
    if let Some(o) = app.operation()
        && o.strategy == Strategy::Contour
        && get(&o.p, "boundary") == "Model silhouette"
        && get(&o.p, "side") != crate::model::CONTOUR_HOLES
        && pocket_setting(&o.p, "tabs") == CONTOUR_TABS[0]
        && let Some(plan) = app.active_simulation().map(|s| &s.plan)
        && plan.target <= plan.stock.min[2] + 0.001
    {
        checks.push(Check::new(
            "Bottom offset takes the contour through the stock's bottom all the way round: the part comes loose and the cutter can throw it. Turn on Holding tabs, or raise Bottom offset to leave a skin.".into(),
            false,
        ));
    }
    // A face below the part's top would cut through its higher features.
    if let Some(o) = app.operation()
        && o.strategy == Strategy::Face
        && let Some(mesh) = &app.imported_mesh
    {
        let setup = app.doc.setup();
        if crate::simulation::facing_faces_down(setup, o, mesh) {
            checks.push(Check::new(
                "The selected surface faces down, away from the spindle: Facing faces the part's top instead. Select the top surface, or the surface to face in a setup that turns it up.".into(),
                false,
            ));
        } else if let Some(raise) = crate::simulation::facing_raise(setup, o, mesh) {
            checks.push(Check::new(
                format!("Facing stops at the part's top, {raise:.2} mm above the selected surface plus bottom offset: going lower would cut the part's higher features. Below that, mill the selected surface with a Pocket or Adaptive clearing."),
                false,
            ));
        }
    }
    checks
}
/// The drilling cycles a Drill operation offers on `machine`: a drill of the
/// pin holes only drills; the lathe and the manual mill drill and tap by
/// hand; a CNC drills and mills (helically, or a thread) but does not tap;
/// a CNC and the manual mill ream and bore.
fn drill_cycles(pin_holes: bool, machine: &Machine) -> Vec<&'static str> {
    DRILL_CYCLES
        .into_iter()
        .enumerate()
        .filter(|&(i, cycle)| match cycle {
            _ if i < 4 => true,
            TAPPING => !pin_holes && manual::is_hand(machine),
            REAMING | BORING => !pin_holes && !lathe::is_lathe(machine),
            _ => !pin_holes && !manual::is_hand(machine),
        })
        .map(|(_, cycle)| cycle)
        .collect()
}
/// `issue_tab` for an operation of `strategy`: a Drill operation has no
/// Passes tab, and Helical milling's stepdown, stepover and tolerance are on
/// its Cycle tab.
pub(crate) fn operation_issue_tab(strategy: Strategy, issue: &str) -> Option<&'static str> {
    match issue_tab(issue) {
        Some("Passes") if strategy == Strategy::Drill => Some("Cycle"),
        tab => tab,
    }
}
/// The operation editor's tab a check's message is about: the one with the
/// field it names, Geometry for the selection it asks for, Tool for a tool;
/// None for a message about no tab, such as the operation's name.
pub(crate) fn issue_tab(issue: &str) -> Option<&'static str> {
    let tab = match issue_field(issue) {
        Some("name") | None => {
            return if issue.contains("feature or surface") {
                Some("Geometry")
            } else if [
                manual::HELICAL_ON_MANUAL,
                crate::lathe::HELICAL_ON_LATHE,
                crate::model::HELICAL_PIN_DRILL,
                crate::model::THREAD_MILL_ON_HAND,
                crate::model::TAP_ON_CNC,
            ]
            .contains(&issue)
                || [
                    "thread depth ",
                    "thread passes ",
                    "Choose the thread",
                    "Choose Yes or No for the spring",
                ]
                .iter()
                .any(|start| issue.starts_with(start))
            {
                Some("Cycle")
            } else if issue.starts_with("Choose a tool")
                || issue.starts_with("Choose a drill")
                || issue.starts_with("Choose a flat end mill")
                || issue.contains(" is not on ")
                || (issue.starts_with("Choose a ") && issue.ends_with(" turn."))
            {
                Some("Tool")
            } else if issue.starts_with("The box must be") {
                Some("Box")
            } else if issue == lathe::DRILL_ON_CENTRE {
                Some("Centre")
            } else {
                None
            };
        }
        Some("rpm" | "feed" | "tooth_force") => "Tool",
        Some("hole_diameter" | "silhouette_extend") => "Geometry",
        Some("x_min" | "x_max" | "y_min" | "y_max" | "z_top" | "z_bottom") => "Box",
        Some(
            "turn_kind" | "face_off" | "to_diameter" | "diameter" | "z_end" | "z" | "edge" | "size"
            | "width" | "thread_side" | "pitch",
        ) => "Turn",
        Some(
            "clearance" | "retract" | "top" | "bottom" | "breakthrough" | "openings" | "tab_count"
            | "tab_width" | "tab_height",
        ) => "Heights",
        Some(
            "rotary_start" | "rotary_end" | "tilt_min" | "tilt_max" | "lead_angle" | "lean_angle",
        ) => "Orientation",
        Some(
            "peck" | "dwell" | "thread" | "thread_hand" | "thread_depth" | "thread_passes"
            | "spring_pass",
        ) => "Cycle",
        Some("ramp_angle" | "lead_in") => "Linking",
        Some(_) => "Passes",
    };
    Some(tab)
}
/// The selected operation's checks as the editor draws them: each at its
/// field when the open tab has it, the rest listed under the form.
pub(crate) struct Checks {
    pub(crate) list: Vec<Check>,
    /// The keys of the fields drawn.
    pub(crate) drawn: Vec<&'static str>,
}
impl Checks {
    fn new(app: &App) -> Self {
        Self {
            list: op_checks(app),
            drawn: vec![],
        }
    }
    /// An operation field, with its checks.
    fn field<'a>(
        &mut self,
        p: &'a Fields,
        key: &'static str,
        label: &'static str,
    ) -> Element<'a, Message> {
        self.hinted(p, key, label, label)
    }
    /// An operation field that shows `hint` while it is empty.
    fn hinted<'a>(
        &mut self,
        p: &'a Fields,
        key: &'static str,
        label: &'static str,
        hint: &'static str,
    ) -> Element<'a, Message> {
        self.drawn.push(key);
        let at: Vec<&Check> = self.list.iter().filter(|c| c.key == Some(key)).collect();
        hinted_field(p, key, label, hint, Target::Operation, &at)
    }
    /// The checks no field drawn shows.
    pub(crate) fn rest(&self) -> Vec<&Check> {
        self.list
            .iter()
            .filter(|c| !c.key.is_some_and(|k| self.drawn.contains(&k)))
            .collect()
    }
}
/// A check as one line: its label in its colour, then its text, `short` at
/// its first sentence.
fn check_line<'a>(check: &Check, short: bool) -> Element<'a, Message> {
    let [_, heading, body] = check.colors();
    let first = leading_sentences(&check.text, 0);
    let said = if short && first.len() < check.text.len() {
        format!("{first} …")
    } else {
        check.text.clone()
    };
    row![
        bold(check.label(), 12).color(heading),
        text(said).size(12).color(body).width(Fill)
    ]
    .spacing(6)
    .into()
}
/// Under the form: that the checks passed, or the errors and the warnings no
/// field on the open tab shows, in a box each, with how many the fields show.
/// The warnings, which only advise, say their first sentence unless `open`:
/// the box is pinned under the form and takes the form's room.
fn checks_summary<'a>(checks: &Checks, open: bool) -> Element<'a, Message> {
    if checks.list.is_empty() {
        return container(
            muted("Configuration checks passed").color(Color::from_rgb8(39, 130, 93)),
        )
        .padding([8, 0])
        .into();
    }
    let rest = checks.rest();
    let mut c = Column::new().spacing(8);
    for error in [true, false] {
        let total = checks.list.iter().filter(|x| x.error == error).count();
        if total == 0 {
            continue;
        }
        let lines: Vec<&Check> = rest.iter().copied().filter(|x| x.error == error).collect();
        let [bg, heading, body] = if error { ERROR } else { WARNING };
        let plural = if total == 1 { "" } else { "s" };
        let title = if error {
            let verb = if total == 1 { "stops" } else { "stop" };
            format!("{total} error{plural} {verb} Generate and export")
        } else {
            format!("{total} warning{plural} · nothing is blocked")
        };
        let title = bold(title, 12).color(heading);
        let folds =
            !error && (lines.iter()).any(|l| leading_sentences(&l.text, 0).len() < l.text.len());
        let mut lines_column = column![if folds {
            row![
                title.width(Fill),
                secondary(
                    if open { "Hide details" } else { "Details" },
                    Message::ToggleWarningDetails,
                )
            ]
            .align_y(Alignment::Center)
            .into()
        } else {
            Element::from(title)
        }]
        .spacing(5);
        let at_fields = total - lines.len();
        if at_fields > 0 {
            let (whose, s) = if at_fields == 1 {
                ("its", "")
            } else {
                ("their", "s")
            };
            lines_column = lines_column.push(
                text(format!("{at_fields} shown at {whose} field{s} above"))
                    .size(12)
                    .color(body),
            );
        }
        for line in lines {
            lines_column = lines_column.push(check_line(line, folds && !open));
        }
        c = c.push(
            container(lines_column)
                .padding(12)
                .width(Fill)
                .style(move |_| container::Style {
                    background: Some(bg.into()),
                    border: Border {
                        radius: 4.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        );
    }
    c.into()
}
/// The selected operation's checks as the open tab places them (tests).
#[cfg(test)]
pub(crate) fn editor_checks(app: &App) -> Checks {
    let mut checks = Checks::new(app);
    if let Some(o) = app.operation() {
        drop(operation_editor_with(app, o, &mut checks));
    }
    checks
}
fn section<'a>(title: impl Into<String>) -> Element<'a, Message> {
    column![space().height(3), bold(title, 13), rule::horizontal(1)]
        .spacing(8)
        .into()
}
fn icon<'a>(app: &App, s: Strategy, size: f32) -> widget::Svg<'a> {
    svg(app.icons[s.index()].clone()).width(size).height(size)
}

/// The footer's status: the whole sentences that fit one line
/// (`STATUS_CHARS`), all of it in the hover card, and greyed
/// (`DIMMED_TEXT`) on another page than the one it was set on, where it
/// tells of an earlier event.
fn status_line(app: &App) -> Element<'_, Message> {
    let full = app.status.trim();
    let first = leading_sentences(full, STATUS_CHARS);
    let line = text(if first.len() < full.len() {
        format!("{first} …")
    } else {
        first.to_string()
    })
    .size(12)
    .wrapping(Wrapping::None)
    .color(if app.status_page == app.page {
        INK
    } else {
        DIMMED_TEXT
    });
    let line = container(line).width(Fill).clip(true);
    if first.len() < full.len() {
        help::wrap(
            line,
            Help::text("Status", full),
            widget::tooltip::Position::Top,
        )
    } else {
        line.into()
    }
}
/// The characters of status the footer shows on one line at the smallest
/// window, where its 12 px text leaves room for about 140.
const STATUS_CHARS: usize = 120;
/// The leading whole sentences of `s` within `limit` characters, and at least
/// the first. A sentence ends at a full stop, question or exclamation mark
/// followed by a space ("33.4 min" does not end one), so an error's reason
/// after "Import failed." stays in sight when it is short.
fn leading_sentences(s: &str, limit: usize) -> &str {
    let mut end = s.len();
    for (i, c) in s.char_indices() {
        if matches!(c, '.' | '?' | '!') && s[i + 1..].starts_with(' ') {
            if s[..=i].chars().count() > limit && end < s.len() {
                break;
            }
            end = i + 1;
            if s[..=i].chars().count() > limit {
                break;
            }
        }
    }
    if s.chars().count() <= limit {
        s
    } else {
        &s[..end]
    }
}
/// The header's line on the automatic save of the workspace.
pub fn workspace_note(app: &App) -> String {
    match (&app.save_error, app.dirty) {
        (Some(e), _) => format!("Workspace not saved: {e}"),
        (None, true) => "Saving workspace…".into(),
        (None, false) => "Workspace saved locally".into(),
    }
}
/// Which file the project is saved in, and whether it has changes since.
/// A long name keeps its start and its end; the tooltip has the whole path.
pub fn project_note(app: &App) -> String {
    const SHOWN: usize = 32;
    match &app.project_path {
        Some(p) => {
            let name: Vec<char> = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .chars()
                .collect();
            let mut shown: String = name.iter().collect();
            if name.len() > SHOWN {
                let head: String = name[..SHOWN / 2].iter().collect();
                let tail: String = name[name.len() - SHOWN / 2 + 1..].iter().collect();
                shown = format!("{head}…{tail}");
            }
            // Unsaved first: a narrow header cuts off the name, not the mark.
            if app.project_dirty {
                format!("Unsaved · {shown}")
            } else {
                shown
            }
        }
        None => "No project file yet".into(),
    }
}
pub fn view(app: &App) -> Element<'_, Message> {
    if app.smoke_dir.is_some() && (13..=14).contains(&app.smoke_step) {
        return help::gallery(u8::from(app.smoke_step == 14));
    }
    if app.smoke_dir.is_some() && app.smoke_step == 25 {
        return help::gallery(2);
    }
    if app.smoke_dir.is_some() && app.smoke_step == 62 {
        return help::gallery(3);
    }
    if app.smoke_dir.is_some() && app.smoke_step == 71 {
        return help::gallery(4);
    }
    if app.smoke_dir.is_some() && (91..=97).contains(&app.smoke_step) {
        return help::gallery((app.smoke_step - 86) as u8);
    }
    // Two workspaces, like Fusion 360's: Manufacture plans the job, Machine
    // (the Control page) runs it. The application bar on top has the
    // project's file actions in Manufacture; the toolbar below starts with
    // the workspace switcher and holds the workspace's own pages.
    let manufacture = app.page != Page::Control;
    let mut brand = row![
        bold("AXIS", 23).color(Color::WHITE),
        text("CAM STUDIO")
            .size(11)
            .color(Color::from_rgb8(164, 190, 213)),
        space().width(18),
    ]
    .spacing(7)
    .align_y(Alignment::Center);
    brand = if manufacture {
        brand.push(
            row![
                bar_btn("Start", Message::Page(Page::Home), app.page == Page::Home),
                bar_btn("New project", Message::NewProject, false),
                help::wrap(
                    text_input("Project name", &app.doc.title)
                        .on_input(Message::Title)
                        .width(180)
                        .size(13)
                        .padding([5, 8]),
                    Help::text(
                        "Project name",
                        "The name shown in the browser and used as the suggested project filename."
                    ),
                    widget::tooltip::Position::Bottom
                ),
                bar_btn("Import model", Message::Import, false),
                bar_btn("Open", Message::OpenProject, false),
                bar_btn("Save", Message::SaveProject, false),
                bar_btn("Save as…", Message::SaveProjectAs, false),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        )
    } else {
        brand.push(
            text(&app.doc.title)
                .size(13)
                .color(Color::from_rgb8(214, 228, 240)),
        )
    };
    let where_saved = match &app.project_path {
        Some(p) => p.display().to_string(),
        None => "Use Save or Save as… to choose the file this project is kept in. Until then it lives only in the workspace.".into(),
    };
    // The notes keep to one line on the right; a narrow window cuts off
    // their end, never the label after them.
    let notes = row![
        space().width(Fill),
        help::wrap(
            text(project_note(app))
                .size(12)
                .wrapping(Wrapping::None)
                .color(Color::from_rgb8(214, 228, 240)),
            Help::text("Project file", where_saved),
            widget::tooltip::Position::Bottom,
        ),
        space().width(10),
        text(workspace_note(app))
            .size(12)
            .wrapping(Wrapping::None)
            .color(Color::from_rgb8(183, 204, 222)),
    ]
    .align_y(Alignment::Center);
    let brand = brand
        .push(container(notes).width(Fill).clip(true))
        .push(space().width(10))
        .push(
            text("NATIVE PROTOTYPE")
                .size(10)
                .wrapping(Wrapping::None)
                .color(Color::from_rgb8(228, 193, 122)),
        );
    let brand = container(brand)
        .padding([8, 18])
        .style(|_| container::Style {
            background: Some(Color::from_rgb8(29, 46, 63).into()),
            ..Default::default()
        });
    let mut top_row = row![switcher(app, false), rule::vertical(1)]
        .spacing(5)
        .align_y(Alignment::Center);
    let pages = [
        (
            Page::Machines,
            "Machine library",
            include_bytes!("../assets/tb-machine.svg").as_slice(),
        ),
        (
            Page::Tools,
            "Tools",
            include_bytes!("../assets/tb-tools.svg"),
        ),
        (
            Page::Materials,
            "Materials",
            include_bytes!("../assets/tb-materials.svg"),
        ),
        (
            Page::Workholding,
            "Workholding",
            include_bytes!("../assets/tb-workholding.svg"),
        ),
        (
            Page::Ai,
            "AI planner",
            include_bytes!("../assets/tb-ai.svg"),
        ),
        (
            Page::Audit,
            "Operation support",
            include_bytes!("../assets/tb-support.svg"),
        ),
    ];
    top_row = if manufacture {
        top_row.extend(pages.map(|(page, label, icon)| {
            tool_btn(icon, label, Message::Page(page), app.page == page).into()
        }))
    } else {
        top_row
            .push(tool_btn(
                include_bytes!("../assets/tb-control.svg"),
                "Control",
                Message::ControlSetup(false),
                !app.machine_setup,
            ))
            .push(tool_btn(
                include_bytes!("../assets/tb-setup.svg"),
                "Controller configuration",
                Message::ControlSetup(true),
                app.machine_setup,
            ))
    };
    top_row = top_row.push(space().width(Fill));
    if app.control.is_some() {
        top_row = top_row.push(estop());
    }
    let docbar = container(top_row)
        .height(66)
        .padding([6, 12])
        .style(panel_style);
    let body: Element<_> = match app.page {
        Page::Home => start_screen(app),
        Page::Work if !app.doc.has_model() && app.doc.setups.is_empty() => blank_project(app),
        Page::Work => work_page(app),
        Page::Machines => machines(app),
        Page::Control => control(app),
        Page::Wizard => wizard(app),
        Page::Tools => tools(app),
        Page::Materials => materials(app),
        Page::Workholding => workholding(app),
        Page::Ai => ai(app),
        Page::Post => post(app),
        Page::Audit => operation_audit(app),
    };
    let foot = container(
        row![
            status_line(app),
            // What the open workspace works with: MACHINE drives the UC100,
            // MANUFACTURE previews and posts.
            text(if app.page == Page::Control {
                "mm  |  UC100 machine control"
            } else {
                "mm  |  Bounded mesh previews · LinuxCNC 3-axis and indexed A post"
            })
            .size(12)
            .color(MUTED)
        ]
        .spacing(15)
        .align_y(Alignment::Center),
    )
    .padding([4, 14])
    .style(panel_style);
    // MACHINE has one bar over the Control page (LIO-274): every pixel of a
    // 13" laptop's window goes to the console under it.
    let mut layout = if manufacture {
        column![brand, docbar]
    } else {
        column![machine_bar(app)]
    };
    if manufacture && app.page != Page::Home && !app.doc.setups.is_empty() {
        layout = layout.push(ribbon(app));
    }
    let page = container(layout.push(body).push(foot))
        .width(Fill)
        .height(Fill)
        .style(|_| container::Style {
            background: Some(BG.into()),
            text_color: Some(INK),
            ..Default::default()
        });
    // The help F1 pinned, in the lower right corner, over the page and
    // above the status bar.
    match &app.pinned_help {
        None => page.into(),
        Some(tag) => widget::Stack::with_children([
            page.into(),
            container(help::pinned(
                tag,
                app.pinned_expanded,
                app.pinned_paused,
                app.esc_closes_pinned_help(),
            ))
            .width(Fill)
            .height(Fill)
            .align_x(iced::alignment::Horizontal::Right)
            .align_y(iced::alignment::Vertical::Bottom)
            .padding([48, 18])
            .into(),
        ])
        .into(),
    }
}
fn start_screen(app: &App) -> Element<'_, Message> {
    let intro = column![
        muted("YOUR NEXT MACHINING PROJECT"),
        bold("Start with your part.", 36),
        text("Create a blank project, open saved work, or explore the example when you want a guided tour.").size(16),
        space().height(12),
        primary("+ New blank project", Message::NewProject),
        secondary("Open a saved project…", Message::OpenProject),
        space().height(12),
        rule::horizontal(1),
        muted("LEARN THE WORKFLOW"),
        secondary("Explore example project", Message::OpenSample),
        muted("The example includes a housing, sample machines and operations.")
    ].spacing(14).width(Fill);
    let mut recent = column![bold("Continue where you left off", 20)].spacing(16);
    if app.doc.has_model()
        || !app.doc.setups.is_empty()
        || !app.doc.machines.is_empty()
        || !app.doc.tools.is_empty()
        || app.doc.title != "Untitled project"
    {
        recent =
            recent
                .push(bold(&app.doc.title, 24))
                .push(muted(
                    app.doc.file.as_ref().map(|f| f.name.as_str()).unwrap_or(
                        if app.doc.sample_model {
                            "Example housing"
                        } else {
                            "No model imported yet"
                        },
                    ),
                ))
                .push(muted(format!(
                    "{} · {}",
                    plural(app.doc.setups.len(), "setup"),
                    plural(
                        app.doc
                            .setups
                            .iter()
                            .map(|s| s.operations.len())
                            .sum::<usize>(),
                        "operation"
                    )
                )))
                .push(primary("Continue project", Message::ResumeProject));
    } else {
        recent = recent.push(muted("There is no project to resume yet. Your new project starts with no model, setups or operations."));
    }
    recent = recent.push(rule::horizontal(1)).push(muted("REUSABLE LIBRARIES"))
        .push(muted(format!("{} · {}",plural(app.doc.machines.len(),"saved machine"),plural(app.doc.tools.len(),"saved tool"))))
        .push(note("New projects keep your saved libraries. The previous project is archived before you start again."));
    recent = recent.push(secondary("Browse project history…", Message::OpenHistory));
    if app.last_archived.is_some() {
        recent = recent.push(secondary(
            "Recover previous project",
            Message::RecoverPrevious,
        ));
    }
    // A short screen's window (LIO-315) scrolls it, never squeezes it. The
    // bar is embedded: only while it shows does it take `SCROLLBAR_GUTTER`
    // beside the content, never the card's edge (LIO-327); at full size
    // there is none and the content is centred in the whole width.
    container(
        scrollable(
            container(
                container(
                    row![
                        container(intro).padding(28).width(Fill),
                        container(recent).padding(26).width(380).style(panel_style)
                    ]
                    .spacing(36),
                )
                .max_width(1060),
            )
            .center_x(Fill),
        )
        // The bar's own 10 and the spacing make the gutter.
        .spacing(SCROLLBAR_GUTTER - 10.),
    )
    .padding(32)
    .center(Fill)
    .into()
}
fn blank_project(app: &App) -> Element<'_, Message> {
    let mut body = column![
        muted("NEW PROJECT"),
        bold("Your workspace is empty.", 32),
        text("Import a model to start planning your machining job.").size(16),
        space().height(8),
        primary(if app.import_busy { "Loading model…" } else { "Import your first model" }, Message::Import),
        muted("STEP / STP · 3MF · STL · OBJ"),
        space().height(16),
        section("Build your job in order"),
        text("1   Import the part you want to machine.").size(14),
        text("2   Configure a machine and create the first setup.").size(14),
        text("3   Set stock and workholding, then add tools and operations.").size(14),
        space().height(12),
        row![secondary("New machine profile",Message::NewMachine),secondary("Saved machines",Message::Page(Page::Machines)),secondary("Tool library",Message::Page(Page::Tools))].spacing(8),
        note("No sample geometry, setups or operations have been added. Saved machines, tools, materials and workholding are available when you need them.")
    ].spacing(13).max_width(730);
    if app.last_archived.is_some() {
        body = body.push(secondary(
            "Recover previous project",
            Message::RecoverPrevious,
        ));
    }
    // As the Home page's (LIO-327): the embedded bar takes its own room beside
    // the panel only while it shows, never the panel's edge (LIO-329).
    container(
        scrollable(container(body).padding(28).width(Fill).style(panel_style))
            .spacing(SCROLLBAR_GUTTER - 10.),
    )
    .padding(36)
    .center(Fill)
    .into()
}
fn first_setup(app: &App) -> Panel<'_> {
    Panel {
        head: row![bold("No setups yet", 22).width(Fill), fold(app, Side::Inspector)]
            .align_y(Alignment::Center)
            .into(),
        body: column![
            text("Your model is ready to view. Choose the machine and workholding for its first machining position.").size(14),
            primary("Create first setup",Message::AddSetup).width(Fill),
            secondary(if app.doc.machines.is_empty() {"New machine profile"} else {"Open machine library"},if app.doc.machines.is_empty(){Message::NewMachine}else{Message::Page(Page::Machines)}).width(Fill),
            note("Each setup starts with zero operations. Add another setup when the part needs to be flipped, indexed or reclamped.")
        ].spacing(18).into(),
        foot: None,
    }
}
fn ribbon(app: &App) -> Element<'_, Message> {
    let mut groups = Row::new().spacing(9).align_y(Alignment::Center);
    let setup = column![
        btn("+ Setup", Message::AddSetup, false),
        btn(
            "Edit setup",
            Message::SetupEditor,
            app.page == Page::Work && app.selected.is_none()
        ),
        muted("SETUP").size(10)
    ]
    .spacing(2)
    .align_x(Alignment::Center);
    groups = groups.push(setup).push(rule::vertical(1));
    let manual_setup = app
        .doc
        .active_setup()
        .is_some_and(|s| manual::is_manual(&s.machine));
    let lathe_setup = app
        .doc
        .active_setup()
        .is_some_and(|s| lathe::is_lathe(&s.machine));
    let all = &Strategy::ALL;
    for (label, group) in [
        (
            "2D MILLING",
            &[&all[0..5], &[Strategy::Chamfer][..]].concat()[..],
        ),
        ("UNDERCUT", &all[13..14]),
        ("3D FINISHING", &all[5..8]),
        ("MULTI-AXIS", &all[8..11]),
        ("MANUAL", &all[11..12]),
        ("LATHE", &all[12..13]),
    ] {
        let mut buttons = Row::new().spacing(1);
        for s in group {
            let s = *s;
            // Why the strategy is dimmed on the active setup's machine.
            let reason = if lathe_setup {
                (!matches!(s, Strategy::Turn | Strategy::Drill)).then_some(lathe::ONLY_TURN)
            } else if s == Strategy::Turn {
                Some(lathe::TURN_ON_MILL)
            } else if s == Strategy::Box {
                (!manual_setup).then_some(manual::BOX_ON_CNC)
            } else if s == Strategy::Indexed {
                // A pointer to indexed setups, not an operation to add.
                Some(crate::indexed::POINTER)
            } else {
                (manual_setup && s != Strategy::Drill).then_some(manual::ONE_AXIS)
            };
            let available = reason.is_none() && (app.is_sample() || s.imported_preview());
            // An unavailable strategy is dimmed: the icon at low opacity and
            // the label in a light grey, with its reason in the hover help.
            let glyph = icon(app, s, 29.).opacity(if available { 1. } else { DIMMED });
            // Greyed like a disabled command, with no mark that reads as
            // part of the name.
            let label = text(s.short()).size(12);
            let tile = button(
                column![
                    glyph,
                    if available {
                        label
                    } else {
                        label.color(DIMMED_TEXT)
                    }
                ]
                .spacing(4)
                .align_x(Alignment::Center),
            )
            .padding([5, 5])
            .on_press(match (available, reason) {
                (true, _) => Message::AddOp(s),
                (false, Some(_)) => Message::Noop,
                (false, None) => Message::Page(Page::Audit),
            })
            .style(button::text);
            buttons = buttons.push(match (available, reason) {
                (true, _) => HelpButton::new(tile, &Message::AddOp(s), s.name()),
                (false, Some(reason)) => HelpButton::dimmed(tile, s, reason),
                (false, None) => HelpButton::unavailable(tile, s),
            });
        }
        groups = groups
            .push(
                column![buttons, muted(label).size(10)]
                    .spacing(4)
                    .align_x(Alignment::Center),
            )
            .push(rule::vertical(1));
    }
    groups = groups.push(space().width(Fill)).push(
        column![
            btn("Generate all", Message::GenerateAll, false),
            btn(
                "NC review",
                Message::Page(Page::Post),
                app.page == Page::Post
            ),
            muted("ACTIONS").size(10)
        ]
        .align_x(Alignment::Center)
        .spacing(2),
    );
    container(groups)
        .padding([8, 14])
        .height(92)
        .width(Fill)
        .style(panel_style)
        .into()
}
/// Room kept at the right of a scrolled column for its scrollbar, which
/// Iced's `scrollable` draws over the content: the bar is 10 wide.
const SCROLLBAR_GUTTER: f32 = 14.;
/// The browser's mark column: wide enough for the widest mark, "✓·".
const MARK_WIDTH: f32 = 22.;
/// How many characters of an operation's or a setup's name the browser shows
/// before an ellipsis; the clipped row is the net for wide letters. The full
/// name is in the row's hover help.
const OP_NAME_CHARS: usize = 20;
const SETUP_NAME_CHARS: usize = 14;
/// `name` cut to `max` characters, ending in "…" when it was longer. Iced
/// 0.14 text has no ellipsis of its own.
pub(crate) fn ellipsize(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        return name.to_string();
    }
    let kept: String = name.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}
/// A sidebar's widths: the least it can be dragged to, what it starts at,
/// and the most.
pub(crate) struct Widths {
    pub min: f32,
    pub default: f32,
    pub max: f32,
}
pub(crate) const BROWSER: Widths = Widths {
    min: 170.,
    default: 230.,
    max: 420.,
};
pub(crate) const INSPECTOR: Widths = Widths {
    min: 320.,
    default: 350.,
    max: 560.,
};
/// The least the 3D view is wide between the sidebars, whatever the window.
pub(crate) const VIEW_MIN: f32 = 500.;
/// The handle between a sidebar and the 3D view.
const GRIP: f32 = 7.;
/// A closed sidebar: the strip that holds its Show button.
const STRIP: f32 = 28.;
/// The inspector's padding; its scrolling body keeps `SCROLLBAR_GUTTER` too.
const INSPECTOR_PAD: f32 = 15.;
fn widths(side: Side) -> Widths {
    match side {
        Side::Browser => BROWSER,
        Side::Inspector => INSPECTOR,
    }
}
/// What a sidebar takes before the view's room is counted: its strip when
/// closed, else its saved width cut to its range.
fn wanted(layout: &Layout, side: Side) -> f32 {
    if layout.open(side) {
        let w = widths(side);
        layout.width(side).clamp(w.min, w.max)
    } else {
        STRIP
    }
}
/// The least a sidebar can be narrowed to: its strip when closed, else its
/// minimum.
fn least(layout: &Layout, side: Side) -> f32 {
    if layout.open(side) {
        widths(side).min
    } else {
        STRIP
    }
}
/// The widths the Work page gives the browser and the inspector in a window
/// `window` wide: what each wants (`wanted`), and when that leaves the 3D
/// view between them less than `VIEW_MIN`, the browser narrowed to its
/// least first and the inspector after it. At the window's least width
/// (`crate::MIN_WINDOW`) the defaults leave the view wider than `VIEW_MIN`.
pub(crate) fn fitted(layout: &Layout, window: f32) -> (f32, f32) {
    let room = window - VIEW_MIN - 2. * GRIP;
    let (mut b, mut i) = (
        wanted(layout, Side::Browser),
        wanted(layout, Side::Inspector),
    );
    if b + i > room {
        b = b.min((room - i).max(least(layout, Side::Browser)));
    }
    if b + i > room {
        i = i.min((room - b).max(least(layout, Side::Inspector)));
    }
    (b, i)
}
/// What the 3D view is wide between the sidebars in a window `window` wide,
/// the handles of the open sidebars counted.
#[cfg(test)]
pub(crate) fn view_width(layout: &Layout, window: f32) -> f32 {
    let (b, i) = fitted(layout, window);
    let grips = [Side::Browser, Side::Inspector]
        .into_iter()
        .filter(|&s| layout.open(s))
        .count() as f32;
    window - b - i - grips * GRIP
}
/// `width` as a sidebar may be dragged to in a window `window` wide: within
/// its range, and leaving the 3D view `VIEW_MIN`. The inspector may push the
/// browser down to its least (`fitted` narrows the browser first); the
/// browser stops where the inspector keeps its width.
pub(crate) fn clamped(layout: &Layout, side: Side, width: f32, window: f32) -> f32 {
    let w = widths(side);
    let keep = match side {
        Side::Browser => wanted(layout, Side::Inspector),
        Side::Inspector => least(layout, Side::Browser),
    };
    let most = (window - VIEW_MIN - 2. * GRIP - keep).min(w.max).max(w.min);
    width.clamp(w.min, most)
}
/// The handle between a sidebar and the 3D view: dragging it sets the
/// sidebar's width. `width` is what the sidebar is drawn at, which a drag
/// starts from.
struct Grip {
    side: Side,
    width: f32,
}
/// Where the pointer pressed and the sidebar's width then, while it drags.
#[derive(Default)]
struct GripDrag(Option<(f32, f32)>);
impl Grip {
    /// The sidebar's width with the pointer moved from `from` to `x`: the
    /// browser's edge follows the pointer, the inspector's goes the other
    /// way.
    fn dragged(&self, from: f32, width: f32, x: f32) -> f32 {
        match self.side {
            Side::Browser => width + (x - from),
            Side::Inspector => width - (x - from),
        }
    }
}
impl canvas::Program<Message> for Grip {
    type State = GripDrag;
    fn update(
        &self,
        state: &mut GripDrag,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let p = cursor.position_over(bounds)?;
                state.0 = Some((p.x, self.width));
                Some(Action::capture())
            }
            // The drag goes on wherever the pointer is, as in the 3D view.
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let (from, width) = state.0?;
                let p = cursor.position()?;
                Some(
                    Action::publish(Message::SidebarWidth(
                        self.side,
                        self.dragged(from, width, p.x),
                        false,
                    ))
                    .and_capture(),
                )
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let (from, width) = state.0.take()?;
                let to = cursor
                    .position()
                    .map_or(width, |p| self.dragged(from, width, p.x));
                Some(Action::publish(Message::SidebarWidth(self.side, to, true)).and_capture())
            }
            _ => None,
        }
    }
    fn draw(
        &self,
        state: &GripDrag,
        renderer: &iced::Renderer,
        _: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let live = state.0.is_some() || cursor.is_over(bounds);
        let width = if live { 3. } else { 1. };
        frame.fill_rectangle(
            Point::new(((bounds.width - width) / 2.).floor(), 0.),
            iced::Size::new(width, bounds.height),
            if live { BLUE } else { LINE },
        );
        vec![frame.into_geometry()]
    }
    fn mouse_interaction(
        &self,
        state: &GripDrag,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.0.is_some() || cursor.is_over(bounds) {
            mouse::Interaction::ResizingHorizontally
        } else {
            mouse::Interaction::default()
        }
    }
}
fn grip<'a>(side: Side, width: f32) -> Element<'a, Message> {
    canvas(Grip { side, width }).width(GRIP).height(Fill).into()
}
/// A sidebar's Hide or Show button: ‹ and › point the way the sidebar goes.
fn fold<'a>(app: &App, side: Side) -> Element<'a, Message> {
    let (glyph, label) = match (side, app.settings.layout.open(side)) {
        (Side::Browser, true) => ("‹", "Hide the browser"),
        (Side::Browser, false) => ("›", "Show the browser"),
        (Side::Inspector, true) => ("›", "Hide the inspector"),
        (Side::Inspector, false) => ("‹", "Show the inspector"),
    };
    let msg = Message::ToggleSidebar(side);
    let b = button(text(glyph).size(15).align_x(Alignment::Center).width(Fill))
        .on_press(msg.clone())
        .padding([1, 2])
        .width(22)
        .style(|_, st| flat(false, st));
    HelpButton::new(b, &msg, label).into()
}
/// A closed sidebar: a strip at the window's edge with its Show button.
fn strip<'a>(app: &App, side: Side) -> Element<'a, Message> {
    container(fold(app, side))
        .padding([10, 3])
        .width(STRIP)
        .height(Fill)
        .style(panel_style)
        .into()
}
/// The Work page: the browser, the 3D view and the inspector, with a handle
/// between the view and each open sidebar.
fn work_page(app: &App) -> Element<'_, Message> {
    let layout = &app.settings.layout;
    let (bw, iw) = fitted(layout, app.window_width);
    let mut r = Row::new().height(Fill).push(browser(app));
    if layout.open(Side::Browser) {
        r = r.push(grip(Side::Browser, bw));
    }
    r = r.push(workspace(app));
    if layout.open(Side::Inspector) {
        r = r.push(grip(Side::Inspector, iw));
    }
    r.push(inspector(app)).into()
}
pub(crate) fn browser(app: &App) -> Element<'_, Message> {
    let layout = &app.settings.layout;
    if !layout.open(Side::Browser) {
        return strip(app, Side::Browser);
    }
    let (width, _) = fitted(layout, app.window_width);
    // The count, the Hide button and the rule stay put above the scrolling
    // tree, so the scrollbar, drawn over the content's right edge, never
    // covers them.
    let header = column![
        row![
            bold("BROWSER", 12),
            space().width(Fill),
            muted(plural(app.doc.setups.len(), "setup")),
            fold(app, Side::Browser)
        ]
        .spacing(6)
        .align_y(Alignment::Center),
        rule::horizontal(1)
    ]
    .spacing(9);
    let mut tree =
        column![
            bold(&app.doc.title, 14),
            muted(app.doc.file.as_ref().map(|f| f.name.as_str()).unwrap_or(
                if app.doc.sample_model {
                    "Sample · bearing_housing.step"
                } else {
                    "No model imported"
                }
            )),
            muted(if app.doc.material.is_empty() {
                "Material not set"
            } else {
                &app.doc.material
            }),
            space().height(5)
        ]
        .spacing(9);
    for (si, s) in app.doc.setups.iter().enumerate() {
        let active = s.id == app.doc.active;
        let label = row![
            text(if active { "▾" } else { "▸" }).size(12),
            container(
                column![
                    bold(ellipsize(s.name(), SETUP_NAME_CHARS), 12).wrapping(Wrapping::None),
                    muted(format!(
                        "{} · {}-axis",
                        get(&s.p, "offset"),
                        s.machine.axes()
                    ))
                    .size(12)
                    .wrapping(Wrapping::None)
                ]
                // The setup's A angle, on a line of its own: a list of them
                // is long.
                .push(crate::indexed::label(s).map(|a| muted(a).size(12).wrapping(Wrapping::Word)))
                .spacing(2)
            )
            .width(Fill)
            .clip(true),
            stock_picture(app, si, false).map(|p| widget::image(p).width(64).height(48))
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        tree = tree.push(HelpButton::new(
            button(label)
                .on_press(Message::SelectSetup(si))
                .width(Fill)
                .padding(8)
                .style(move |t, st| {
                    let mut style = button::text(t, st);
                    if active {
                        style.background = Some(SETUP_ROW.into())
                    }
                    style.text_color = INK;
                    style
                }),
            &Message::SelectSetup(si),
            s.name(),
        ));
        if active {
            tree = tree.push(
                container(muted(format!("{} · v{}", s.machine.name(), s.machine.version)).size(12))
                    .padding([0, 16]),
            );
            if manual::is_hand(&s.machine) {
                tree = tree.push(
                    container(
                        secondary("Steps ▶", Message::Guide(crate::GuideAction::Open(si)))
                            .width(Fill),
                    )
                    .padding([0, 10]),
                );
            }
            let checks = app.current_ready();
            for (i, o) in s.operations.iter().enumerate() {
                let selected = app.selected == Some(i) && app.page == Page::Work;
                let mark = op_mark(app, checks, i);
                let (glyph, color) = (mark.glyph(), mark.color());
                let content = row![
                    icon(app, o.strategy, 23.),
                    container(
                        text(ellipsize(o.name(), OP_NAME_CHARS))
                            .size(12)
                            .wrapping(Wrapping::None)
                    )
                    .width(Fill)
                    .clip(true),
                    container(text(glyph).size(13).color(color))
                        .width(MARK_WIDTH)
                        .align_x(iced::alignment::Horizontal::Right)
                ]
                .spacing(6)
                .align_y(Alignment::Center);
                tree = tree.push(HelpButton::operation(
                    button(content)
                        .width(Fill)
                        .padding([7, 10])
                        .on_press(Message::SelectOp(i))
                        .style(move |_, st| button::Style {
                            background: Some(
                                if selected {
                                    SELECTED_ROW
                                } else if matches!(st, button::Status::Hovered) {
                                    HOVER_ROW
                                } else {
                                    Color::TRANSPARENT
                                }
                                .into(),
                            ),
                            text_color: if o.enabled { INK } else { MUTED },
                            border: Border {
                                radius: 4.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    o.strategy,
                    o.name(),
                    mark_help(app, checks, i, mark),
                ));
            }
            if let Some(picture) = stock_picture(app, si, true) {
                let selected = app.setup_end && app.selected.is_none() && app.page == Page::Work;
                tree = tree.push(help::wrap(
                    button(
                        row![
                            muted("Stock after this setup").size(12).width(Fill),
                            widget::image(picture).width(64).height(48)
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    )
                    .width(Fill)
                    .padding([2, 10])
                    .on_press(Message::SetupEnd)
                    .style(move |_, st| button::Style {
                        background: Some(
                            if selected {
                                SELECTED_ROW
                            } else if matches!(st, button::Status::Hovered) {
                                HOVER_ROW
                            } else {
                                Color::TRANSPARENT
                            }
                            .into(),
                        ),
                        text_color: INK,
                        border: Border {
                            radius: 4.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    Help::text(
                        "Stock after this setup",
                        "The stock as this setup's operations leave it, in its jaws or clamps, seen as its 3D view first shows it. Cut faces are green. Click it to see that stock in the 3D view.",
                    ),
                    widget::tooltip::Position::Right,
                ));
            }
        }
    }
    if let Some(block) = threads_block(app) {
        tree = tree.push(block);
    }
    if let Some(block) = uncut_block(app) {
        tree = tree.push(block);
    }
    tree = tree
        .push(space().height(6))
        .push(secondary("+ Add setup", Message::AddSetup).width(Fill));
    if app.doc.machines.iter().any(manual::is_manual) {
        tree = tree.push(secondary("+ Add manual setup", Message::AddManualSetup).width(Fill));
    }
    if app.doc.machines.iter().any(lathe::is_lathe) {
        tree = tree.push(secondary("+ Add lathe setup", Message::AddLatheSetup).width(Fill));
    }
    let footer = column![
        rule::horizontal(1),
        muted("PROJECT UNITS"),
        text("Millimeters (mm)").size(12),
        muted("Machine limits can use mm or inch. Tool and operation values use mm.").size(12)
    ]
    .spacing(6);
    let tree = container(tree).width(Fill).padding(iced::Padding {
        right: SCROLLBAR_GUTTER,
        ..Default::default()
    });
    container(column![header, scrollable(tree).height(Fill), footer].spacing(12))
        .padding(14)
        .width(width)
        .height(Fill)
        .style(panel_style)
        .into()
}
/// What the whole job leaves uncut (`LeftStock::uncut`), and whether that
/// was worked out for the job as it is now. While an edited job is worked
/// out again, the last result on the same part, so the magenta faces, the
/// checkbox and the browser's list stay where they are until it is done.
/// None before it is first worked out, and without a setup or an imported
/// part.
fn shown_uncut(app: &App) -> Option<(&Result<std::sync::Arc<crate::uncut::Uncut>, String>, bool)> {
    let key = app.left_key()?;
    let left = &app.left_stock;
    let found = left.uncut.as_ref()?;
    if left.key == key {
        Some((found, true))
    } else {
        (app.part_mesh()?.fingerprint == left.part).then_some((found, false))
    }
}
/// One line on what the whole job leaves uncut, under the 3D views: None
/// without a setup or an imported part.
pub(crate) fn uncut_line(app: &App) -> Option<String> {
    app.left_key()?;
    Some(match shown_uncut(app) {
        None => "Completeness not checked yet.".into(),
        Some((Ok(found), false)) if app.show_uncut && found.faces.iter().any(|f| *f) => {
            "Checking completeness again; the magenta faces are the last check's.".into()
        }
        Some((_, false)) => "Checking completeness again…".into(),
        Some((Err(why), true)) => format!("Completeness not checked: {why}"),
        Some((Ok(found), true)) if found.regions.is_empty() => {
            "Every face of the part is cut by the job.".into()
        }
        Some((Ok(found), true)) if app.show_uncut => format!(
            "Magenta faces are still under stock after the whole job: {:.0}% of the part's surface.",
            found.share * 100.
        ),
        Some((Ok(found), true)) => format!(
            "{:.0}% of the part's surface is still under stock after the whole job; Show uncut faces is off.",
            found.share * 100.
        ),
    })
}
/// The "Show uncut faces" checkbox, beside Stock: None unless the job leaves
/// some of the part under stock, as there is nothing to show then. While an
/// edited job is simulated again it stays as the last run had it
/// (`shown_uncut`), so the checkboxes beside it do not move.
pub(crate) fn uncut_toggle(app: &App) -> Option<Element<'_, Message>> {
    shown_uncut(app)?
        .0
        .as_ref()
        .ok()
        .filter(|found| found.faces.iter().any(|f| *f))?;
    Some(help::wrap(
        checkbox(app.show_uncut)
            .label("Show uncut faces")
            .on_toggle(|_| Message::ToggleUncut)
            .text_size(12),
        crate::choice_help::selected(
            "uncut_visibility",
            "Show uncut faces",
            if app.show_uncut { "Shown" } else { "Hidden" },
            Target::Setup,
            &Fields::new(),
        ),
        widget::tooltip::Position::Top,
    ))
}
/// One line on what the colours of the 3D view mean: tan original stock,
/// green machined faces, the part's hatched section while it is on,
/// and magenta faces still under stock while those are shown. None with
/// the stock hidden, no section and nothing magenta.
pub(crate) fn uncut_legend(app: &App) -> Option<Element<'_, Message>> {
    let magenta = app.show_uncut && uncut_toggle(app).is_some();
    let section = app.view_section().is_some();
    if !app.stock && !magenta && !section {
        return None;
    }
    let key = |colour: Color, name: &'static str| {
        row![text("■").size(12).color(colour), muted(name).size(12)].spacing(3)
    };
    let mut legend = row![].spacing(10);
    if app.stock {
        legend = legend
            .push(key(Color::from_rgb8(219, 183, 113), "original stock"))
            .push(key(Color::from_rgb8(103, 181, 151), "machined faces"));
    }
    if section {
        legend = legend.push(key(Color::from_rgb8(64, 88, 118), "part's section"));
    }
    if magenta {
        legend = legend.push(key(Color::from_rgb8(214, 72, 166), "still under stock"));
    }
    Some(legend.into())
}
/// The browser's list of the regions of the part the whole job leaves under
/// stock, or why that is not checked. While the job is worked out again,
/// the last list in grey under a Checking heading (`shown_uncut`).
/// The threads modeled in the part, as the active setup holds it: each size
/// once with its count and where it runs, and Add threading for the holes
/// that open upwards.
fn threads_block(app: &App) -> Option<Element<'_, Message>> {
    let mesh = app.part_mesh().filter(|_| app.doc.file.is_some())?;
    let setup = app.doc.active_setup()?;
    let found = crate::thread::in_setup(setup, mesh);
    if found.is_empty() {
        return None;
    }
    let lathe = lathe::is_lathe(&setup.machine);
    let place = |t: &crate::thread::Thread| match (t.internal, t.axis) {
        (false, _) if !lathe => "on the lathe, or a die by hand",
        (_, 2) => "along Z",
        _ => "on a side: add a setup with it up",
    };
    let mut lines: Vec<(String, usize)> = vec![];
    for t in &found {
        let line = format!("{} · {:.1} mm · {}", t.label(), t.length(), place(t));
        match lines.iter_mut().find(|(l, _)| *l == line) {
            Some((_, n)) => *n += 1,
            None => lines.push((line, 1)),
        }
    }
    let mut block = column![muted("THREADS").size(12)].spacing(4);
    for (line, n) in &lines {
        block = block.push(muted(format!("• {n}× {line}")).size(12));
    }
    let addable = if lathe {
        crate::lathe::thread_from_part(setup, mesh).is_some()
    } else {
        found.iter().any(|t| t.internal && t.axis == 2)
    };
    if addable {
        block = block.push(secondary("+ Add threading", Message::AddThreading).width(Fill));
    }
    Some(help::wrap(
        container(block).padding([4, 10]),
        Help::text(
            "Threads",
            "The threads modeled in the part, found from its shape: size, hand, inside (a hole) or outside, and how long. Add threading drills each threaded hole that opens upwards to its tap drill and then threads it: thread milling on the CNC, tapping by hand on the manual mill.",
        )
        .more(
            "Only modeled threads are found: a hole drawn plain at a tap-drill size is a plain hole (choose its thread in a Drill's Thread). Outside threads are cut on the lathe or with a die. The tools come from the library; an operation whose tool is missing is added without one, and the status line says which tool to add.",
        ),
        widget::tooltip::Position::Right,
    ))
}
fn uncut_block(app: &App) -> Option<Element<'_, Message>> {
    let shown = shown_uncut(app);
    let current = shown.is_none_or(|(_, current)| current);
    let mut block = column![
        muted(if current {
            "STILL UNCUT"
        } else {
            "STILL UNCUT · CHECKING"
        })
        .size(12)
    ]
    .spacing(4);
    match shown {
        Some((Ok(found), _)) if !found.regions.is_empty() => {
            block = block.push(
                text(format!(
                    "{} region{}, {:.0}% of the part's surface, magenta in the 3D view:",
                    found.regions.len(),
                    if found.regions.len() == 1 { "" } else { "s" },
                    found.share * 100.
                ))
                .size(12)
                .color(if current {
                    Color::from_rgb8(166, 46, 124)
                } else {
                    MUTED
                }),
            );
            for region in found.regions.iter().take(5) {
                block = block.push(muted(format!("• {}", region.line())).size(12));
            }
            if found.regions.len() > 5 {
                block =
                    block.push(muted(format!("• and {} more", found.regions.len() - 5)).size(12));
            }
        }
        _ => block = block.push(muted(uncut_line(app)?).size(12)),
    }
    Some(help::wrap(
        container(block).padding([4, 10]),
        Help::text(
            "Still uncut",
            "The faces of the part that the stock left after every setup still covers: what the job has not machined yet. Until every enabled operation generates, completeness is not checked.",
        )
        .more(
            "Each face is sampled 0.5 mm outside itself against the simulated cuts, so a thinner finishing allowance counts as cut. Positions are in setup 1's axes from the part's -X/-Y/bottom corner, in mm.",
        ),
        widget::tooltip::Position::Right,
    ))
}
/// The picture of setup `si`'s stock in its jaws or clamps, for the browser.
/// As it starts (`end` false): what the earlier setups left it
/// (`LeftStock::before`), else its box (`App::known_stock`). As its
/// operations leave it (`end`): `LeftStock::after`; None until that is worked
/// out and when nothing was cut by then. While the job is worked out again,
/// from the stock last worked out.
pub(crate) fn stock_picture(app: &App, si: usize, end: bool) -> Option<widget::image::Handle> {
    use std::hash::{Hash, Hasher};
    let setup = app.doc.setups.get(si)?;
    let left = &app.left_stock;
    let cut = app
        .left_key()
        .and(if end {
            left.after.get(si)
        } else {
            left.before.get(si)
        })
        .cloned()
        .flatten()
        .and_then(|v| {
            let (_, surface) = left
                .surfaces
                .iter()
                .find(|(s, _)| std::sync::Arc::ptr_eq(s, &v))?;
            Some((v, surface.clone()))
        });
    if end && cut.is_none() {
        return None;
    }
    let stock = app.known_stock(si).0?;
    let jaws = pictured_holding(app, si, stock);
    // What the picture shows: the stock meshed, else its box, and the jaws.
    // A meshed stock is named by the job it was worked out for, not by its
    // address, which a later stock can take once this one is freed.
    let mut hasher = std::hash::DefaultHasher::new();
    match &cut {
        Some(_) => left.key.hash(&mut hasher),
        None => stock
            .min
            .iter()
            .chain(&stock.max)
            .for_each(|v| v.to_bits().hash(&mut hasher)),
    }
    (si, end).hash(&mut hasher);
    for b in &jaws {
        b.bounds
            .min
            .iter()
            .chain(&b.bounds.max)
            .for_each(|v| v.to_bits().hash(&mut hasher));
    }
    let key = hasher.finish();
    let mut drawn = app.stock_pictures.borrow_mut();
    if let Some((_, picture)) = drawn.iter().find(|(k, _)| *k == key) {
        return Some(picture.clone());
    }
    let mut triangles = match cut {
        Some((v, surface)) => {
            let frame = crate::volume::Frame::new(setup, app.part_mesh()?);
            crate::viewport::stock_triangles(&v, &surface, frame).collect()
        }
        None => crate::viewport::box_triangles(stock, crate::viewport::BLOCK),
    };
    for b in &jaws {
        triangles.extend(crate::viewport::box_triangles(
            b.bounds,
            crate::viewport::JAWS,
        ));
    }
    let picture = crate::viewport::stock_picture(&triangles, crate::viewport::PICTURE)?;
    // ponytail: two per setup, so all are kept until there are many.
    if drawn.len() > 64 {
        drawn.clear();
    }
    drawn.push((key, picture.clone()));
    Some(picture)
}
/// The jaws or clamps and the fixture pins in setup `si`'s pictures of its
/// `stock` (`stock_picture`, the Tool runs panel's), in its frame in mm.
pub(crate) fn pictured_holding(
    app: &App,
    si: usize,
    stock: crate::mesh::Bounds,
) -> Vec<crate::holding::HoldBox> {
    let Some(setup) = app.doc.setups.get(si) else {
        return vec![];
    };
    // A lathe's chuck is drawn by the lathe view, not as boxes.
    let modelled = match get(&setup.p, "holding") {
        "Vise" | "Clamps" => true,
        "Chuck" => crate::chuck::holds(setup),
        // A round bar's holder, or a sheet's hold-down screw heads.
        holding => crate::round_hold::Kind::of(holding).is_some() || crate::sheet::screwed(setup),
    };
    let mut jaws = if modelled {
        let held = app
            .known_held(si)
            .filter(|held| held.stock == stock)
            .unwrap_or(crate::holding::Gripped::whole(stock));
        app.doc
            .holding_boxes(setup, held)
            .map(|b| crate::holding::shown(&b, stock))
            .unwrap_or_default()
    } else {
        vec![]
    };
    jaws.extend(pin_boxes(app, setup, stock));
    jaws
}
/// The part's holes offered for the fixture pins in the 3D view: shown
/// while the Work zero step of a setup on the pins is open, each hole not
/// chosen numbered as `pins_block` lists it.
fn pin_hole_marks(app: &App) -> viewport::PinHolePick {
    let open = app.page == Page::Work
        && app.selected.is_none()
        && app.setup_step == "Work zero"
        && !app.is_sample();
    let (true, Some(s), Some(mesh)) = (open, app.doc.active_setup(), app.part_mesh()) else {
        return viewport::PinHolePick::default();
    };
    if !crate::post::sits_on_pins(s) || crate::post::pins_in_leftover(s) {
        return viewport::PinHolePick::default();
    }
    let pick = app.picking_pin_holes();
    let holes = match &*app.bottom_holes(mesh) {
        Ok(holes) => (holes.iter().enumerate())
            .take(48)
            .filter(|(_, h)| crate::post::chosen_index(s, mesh, h).is_none())
            .map(|(i, h)| {
                (
                    i + 1,
                    [h.center[0], h.center[1], h.z],
                    h.diameter.max(h.opening) / 2.,
                )
            })
            .collect(),
        Err(_) => vec![],
    };
    viewport::PinHolePick { pick, holes }
}
/// The pins being placed in the active setup's leftover, while "Place the
/// pins on the leftover" is on (`App::placing_leftover_pins`): the plate's
/// pins on the bottom of the stock it starts from, where they stand and,
/// under the pointer, whether they fit there (`post::leftover_issue`).
pub(crate) fn pin_aim(app: &App) -> viewport::PinAim {
    let si = app.doc.index();
    let (true, Some(s), Some(mesh), (Some(stock), _)) = (
        app.placing_leftover_pins(),
        app.doc.active_setup(),
        app.part_mesh(),
        app.known_stock(si),
    ) else {
        return viewport::PinAim::default();
    };
    let Ok(Some(pins)) = crate::control::pins_settings(&s.machine.p) else {
        return viewport::PinAim::default();
    };
    let offsets = crate::post::plate_places(&pins, [0., 0.]);
    let placed = crate::post::leftover_places(&app.doc, mesh, si).map(|at| at[0]);
    let ghost = app.pin_aim.map(|pin1| {
        let places = crate::post::plate_places(&pins, pin1);
        let fit = match crate::post::leftover_issue(s, mesh, stock, &pins, &places) {
            Some(why) => Err(why),
            None => {
                let r = crate::post::leftover_hole_radius(&pins);
                let gaps = crate::post::leftover_gaps(s, mesh, stock, &places);
                Ok(gaps
                    .iter()
                    .fold((f32::INFINITY, f32::INFINITY), |(p, e), g| {
                        (p.min(g.0 - r), e.min(g.1 - r))
                    }))
            }
        };
        (pin1, fit)
    });
    viewport::PinAim {
        on: true,
        z: stock.min[2],
        offsets,
        diameter: pins.diameter as f32,
        height: pins.height as f32,
        placed,
        ghost,
    }
}
/// The 3D view of the active setup, as MANUFACTURE shows it: the part,
/// stock, workholding, the preview and the machine's tool while a tool run of
/// the setup cuts, its camera fitting the scene in `frame` (`Viewport::frame`).
/// None without a part to show.
fn scene(app: &App, frame: Option<f32>) -> Option<Element<'_, Message>> {
    let view = Viewport {
        frame,
        ..scene_viewport(app)?
    };
    let canvas: Element<viewport::Event> = view.view(Fill);
    Some(canvas.map(Message::Viewport))
}
/// The room (logical pixels) the toolbar and the simulation strip under the
/// 3D view take at their tallest as a rule: the strip playing a preview,
/// with its check (some 220), and the toolbar with the uncut faces' lines
/// (100 to 130). The camera
/// fits the scene in the rest (`scene_frame`); taller panels scroll.
const PANELS: f32 = 320.;
/// How high the frame the 3D view's camera fits the scene in is, when the
/// view, its toolbar and the simulation strip share `height`: all of it but
/// `PANELS`, and at least half. It does not depend on how tall the panels
/// are now, so switching operations or leaving a simulation, which changes
/// them, does not move the view.
pub(crate) fn scene_frame(height: f32) -> f32 {
    (height - PANELS).max(height * 0.5)
}
/// The room (logical pixels) under the 3D view that shows the toolbar, the
/// strip's heading and a refusal's heading over the strip's pinned controls
/// (some 80, 65 and 95). With less, a refusal goes above the toolbar.
const REFUSAL_IN_SIGHT: f32 = 250.;
/// What the section's controls add to the toolbar.
const SECTION_LINE: f32 = 40.;
/// The 3D view `scene` shows. None without a part to show.
pub(crate) fn scene_viewport(app: &App) -> Option<Viewport<'_>> {
    let s = app.doc.active_setup();
    Some(Viewport {
        setup: s,
        mesh: app.part_mesh()?,
        operation: app.operation(),
        simulation: app.active_simulation(),
        sample: app.is_sample(),
        camera: app.camera,
        picking: app.picking || app.picking_pin_holes(),
        pin_holes: pin_hole_marks(app),
        hole_outlines: app.hole_outlines(),
        pin_aim: pin_aim(app),
        show_stock: app.stock && s.is_some(),
        show_paths: app.paths && app.playback_ready(),
        progress: if app.operation().is_some_and(|o| o.preview) {
            Some(app.progress)
        } else {
            None
        },
        holding: holding_boxes(app),
        first_setup: app.doc.setups.first(),
        framed: framed_holding(app),
        leftover: leftover_view(app),
        live: app.live_tool(),
        a: app.live_a(),
        live_stock: app.live_stock(),
        start_stock: app.start_stock(),
        live_volume: app.live_volume(),
        uncut: shown_uncut(app)
            .and_then(|(found, _)| found.as_ref().ok())
            .map(|found| found.faces.as_slice())
            .unwrap_or(&[]),
        uncut_pieces: shown_uncut(app)
            .and_then(|(found, _)| found.as_ref().ok())
            .map(|found| found.pieces.as_slice())
            .unwrap_or(&[]),
        show_uncut: app.show_uncut,
        section: viewport::Sectioning {
            cut: app.view_section(),
            picking: app.picking_section,
        },
        frame: None,
    })
}
fn workspace(app: &App) -> Element<'_, Message> {
    let s = app.doc.active_setup();
    // The camera buttons keep their width; the setup's lines on the left
    // get what is left and are cut off in a narrow view.
    let heading = container(
        row![
            container(
                column![
                    bold(s.map(|s| s.name()).unwrap_or("Imported model"), 15)
                        .wrapping(Wrapping::None),
                    match s {
                        // The work zero, one click from its step, first so a
                        // narrow view cuts off the offset and holding instead.
                        Some(s) => Element::from(
                            row![
                                muted("Work zero:").wrapping(Wrapping::None),
                                button(
                                    text(format!("{} · Change", get(&s.p, "origin")))
                                        .size(12)
                                        .wrapping(Wrapping::None)
                                )
                                .padding([2, 6])
                                .on_press(Message::SetupStep("Work zero"))
                                .style(|_, status| flat(false, status)),
                                muted(format!(
                                    "{} / {}{}",
                                    get(&s.p, "offset"),
                                    app.doc.holding_label(s),
                                    crate::indexed::label(s)
                                        .map(|a| format!(" / {a}"))
                                        .unwrap_or_default(),
                                ))
                                .wrapping(Wrapping::None),
                            ]
                            .spacing(4)
                            .align_y(Alignment::Center)
                        ),
                        None => muted("Create your first machining setup when you are ready.")
                            .wrapping(Wrapping::None)
                            .into(),
                    }
                ]
                .spacing(3)
            )
            .width(Fill)
            .clip(true),
            space().width(6),
            btn("Fit", Message::Camera("FIT"), false),
            space().width(6),
            btn("ISO", Message::Camera("ISO"), false),
            btn("TOP", Message::Camera("TOP"), false),
            btn("FRONT", Message::Camera("FRONT"), false),
            btn("RIGHT", Message::Camera("RIGHT"), false)
        ]
        .spacing(1)
        .align_y(Alignment::Center),
    )
    .padding([12, 14]);
    // The view, its toolbar and the simulation strip share what is left;
    // the camera's frame is worked out from all of it (`scene_frame`). The
    // panels scroll past the room the frame leaves them, so open details
    // and a long run check never cut the scene off. The strip's controls
    // stay under the scrolled part, in sight (LIO-310): with a fixed width
    // in a Shrink-wide column, iced lays them out before the Fill-wide
    // scrollable, which gets what they leave.
    let middle = responsive(move |size| {
        let (toolbar, simulation, controls, refused) = scene_panels(app);
        let frame = scene_frame(size.height);
        let scene = scene(app, Some(frame)).unwrap_or_else(|| no_scene(app));
        // Where the room is too short to show the toolbar and then why Run
        // will refuse, the strip goes first: the toggles scroll (LIO-318).
        let short =
            size.height - frame < REFUSAL_IN_SIGHT + if app.section_on { SECTION_LINE } else { 0. };
        let scrolled = if refused && short {
            column![simulation, toolbar]
        } else {
            column![toolbar, simulation]
        };
        let panels = column![scrollable(scrolled), container(controls).width(size.width)]
            .width(Length::Shrink);
        let panels = container(panels).max_height(size.height - frame);
        column![scene, panels].into()
    });
    // A UC100 machine takes no G-code file: its setups run from MACHINE.
    let uc100 =
        (app.doc.active_setup()).is_some_and(|s| get(&s.machine.p, "controller") == "UC100");
    let workflow = container(
        row![
            btn("01  Setup", Message::SetupEditor, app.selected.is_none()),
            btn(
                "02  Operations",
                Message::OperationsStep,
                app.selected.is_some()
            ),
            btn("03  Preview", Message::PreviewStep, false),
            btn(
                if uc100 {
                    "04  Review & run"
                } else {
                    "04  NC review"
                },
                Message::Page(Page::Post),
                false
            )
        ]
        .spacing(2),
    )
    .width(Fill)
    .padding([5, 8])
    .style(panel_style);
    column![heading, middle, workflow]
        .height(Fill)
        .width(Fill)
        .into()
}
/// What the workspace shows in place of the 3D view without a part.
fn no_scene(app: &App) -> Element<'_, Message> {
    container(column![bold(if app.import_busy { "Loading geometry…" } else if !app.doc.has_model() { "Import your part" } else { "Model preview unavailable" },25),muted(app.doc.file.as_ref().map(|f|f.name.clone()).unwrap_or_default()),note(app.import_error.clone().unwrap_or_else(|| "Import STEP, STP, 3MF, STL, OBJ or DXF to view your model. Geometry loads in the background; a STEP conversion may take up to two minutes.".into())),primary("Import model",Message::Import),secondary("Start screen",Message::Page(Page::Home))].spacing(20).max_width(380)).center(Fill).into()
}
/// The section's controls under the 3D view while it is on: the axis it is
/// square to or the face it goes through, the slider that moves it across
/// the scene, where it is, and the side cut away. None while it is off.
fn section_controls(app: &App) -> Option<Element<'_, Message>> {
    use crate::section::{AXES, Edit, Through};
    if !app.section_on {
        return None;
    }
    let view = scene_viewport(app)?;
    let cut = view.section.cut;
    let mut controls = row![muted("Cut").size(12)]
        .spacing(2)
        .align_y(Alignment::Center);
    for (k, name) in AXES.iter().enumerate() {
        let on = !app.picking_section && cut.is_some_and(|c| c.through == Through::Axis(k));
        controls = controls.push(btn(*name, Message::Section(Edit::Axis(k)), on));
    }
    let on_face = cut.is_some_and(|c| matches!(c.through, Through::Surface(_)));
    // The example's demo geometry has no surfaces to pick: Face… is greyed.
    let faces = !view.mesh.surfaces.is_empty();
    let pick = Message::Section(Edit::PickFace);
    controls = controls.push(if faces {
        btn("Face…", pick, app.picking_section || on_face)
    } else {
        help::HelpButton::new(
            button(text("Face…").size(12).color(MUTED))
                .padding([7, 10])
                .style(|_, status| flat(false, status)),
            &pick,
            "Face…",
        )
    });
    let Some((cut, (low, high))) =
        cut.and_then(|c| Some((c, c.range(view.mesh, view.setup, view.scene_bounds())?)))
    else {
        return Some(
            controls
                .push(space().width(8))
                .push(muted("Click a face of the part to cut through it.").size(12))
                .into(),
        );
    };
    let high = high.max(low + 0.1);
    Some(
        controls
            .push(space().width(8))
            .push(help::wrap(
                slider(low..=high, cut.at.clamp(low, high), |at| {
                    Message::Section(Edit::At(at))
                })
                .step(0.1_f32)
                .width(Length::Fixed(170.)),
                help::command(&Message::Section(Edit::At(cut.at)), "Section plane".into()),
                widget::tooltip::Position::Top,
            ))
            .push(space().width(8))
            .push(muted(cut.place()).size(12))
            .push(space().width(Fill))
            .push(muted(format!("Cut away: {}", cut.side())).size(12))
            .push(btn("Flip", Message::Section(Edit::Flip), false))
            .into(),
    )
}
/// The 3D view's toolbar, the simulation strip under it, and the strip's
/// controls under that: Play and the timeline, the stats and Details, or
/// what to do for a preview. The open details go above the controls, so
/// that Details keeps its place. Last, whether the strip says why Run will
/// refuse.
fn scene_panels(
    app: &App,
) -> (
    Element<'_, Message>,
    Element<'_, Message>,
    Element<'_, Message>,
    bool,
) {
    let detail = if app.is_sample() && app.section_on {
        "Sample housing · selectable demo geometry · cutting through a face needs an imported part"
            .to_string()
    } else if app.is_sample() {
        "Sample housing · selectable demo geometry".to_string()
    } else if let Some(m) = &app.imported_mesh {
        format!(
            "{} triangles · {:.1} × {:.1} × {:.1} {}{}",
            m.triangles,
            m.dimensions[0],
            m.dimensions[1],
            m.dimensions[2],
            m.units,
            if m.sampled { " · reduced preview" } else { "" }
        )
    } else {
        "No geometry preview loaded".into()
    };
    let toggles = row![
        help::wrap(
            checkbox(app.stock)
                .label("Stock")
                .on_toggle(|_| Message::ToggleStock)
                .text_size(12),
            crate::choice_help::selected(
                "stock_visibility",
                "Stock",
                if app.stock { "Shown" } else { "Hidden" },
                Target::Setup,
                &Fields::new()
            ),
            widget::tooltip::Position::Top
        ),
        uncut_toggle(app),
        help::wrap(
            checkbox(app.section_on)
                .label("Section")
                .on_toggle(|_| Message::Section(crate::section::Edit::Toggle))
                .text_size(12),
            help::command(
                &Message::Section(crate::section::Edit::Toggle),
                "Section".into()
            ),
            widget::tooltip::Position::Top
        ),
        help::wrap(
            checkbox(app.paths)
                .label(if app.is_sample() {
                    "Sample paths"
                } else {
                    "Toolpath"
                })
                .on_toggle(|_| Message::TogglePaths)
                .text_size(12),
            crate::choice_help::selected(
                "path_visibility",
                "Toolpath",
                if app.paths { "Shown" } else { "Hidden" },
                Target::Operation,
                &Fields::new()
            ),
            widget::tooltip::Position::Top
        ),
        space().width(Fill),
        muted(detail).size(12)
    ]
    .spacing(15)
    .align_y(Alignment::Center);
    // The part's size beside the checkboxes and the hint beside the legend:
    // every line here is a line the 3D view above loses.
    let hint = row![
        uncut_legend(app),
        space().width(Fill),
        muted(if app.picking_section && app.section_on {
            "Click a face of the part to cut through it · drag to orbit"
        } else if app.picking {
            "Click a highlighted feature to select it · drag to orbit"
        } else {
            "Drag to orbit · right-drag to pan · scroll to zoom"
        })
        .size(12)
    ]
    .spacing(15)
    .align_y(Alignment::Center);
    let toolbar = container(
        column![
            toggles,
            section_controls(app),
            uncut_line(app).map(|line| muted(line).size(12)),
            hint
        ]
        .spacing(7),
    )
    .padding([10, 16]);
    let ready = app.playback_ready();
    let imported_facing = !app.is_sample()
        && app
            .operation()
            .is_some_and(|o| o.enabled && o.strategy.imported_preview());
    let mut playback = column![
        row![
            bold(
                if app.is_sample() {
                    "SAMPLE MOTION"
                } else if imported_facing {
                    "OPERATION SIMULATION"
                } else {
                    "SIMULATION"
                },
                12
            ),
            space().width(Fill),
            muted(if app.is_sample() {
                "Illustrative motion".to_string()
            } else {
                fixture_note(app, Note::Panel)
            })
            .size(12)
        ]
        .spacing(8)
    ]
    .spacing(8);
    // Folded, a simulation's strip says the check's first line and one line
    // of stats; Details brings back the rest (`Message::ToggleSimDetails`).
    let folded = !app.sim_details && ready && app.active_simulation().is_some();
    let mut refused = false;
    if let Some((heading, mut lines, ok, checking)) = run_check(app) {
        if folded && ok {
            lines.clear();
        } else if folded && lines.len() > 1 {
            let more = lines.len() - 1;
            lines.truncate(1);
            lines.push(format!("{more} more under Details"));
        }
        let said: Element<'_, Message> = if ok {
            let heading = if checking {
                muted(heading).size(12)
            } else {
                bold(heading, 12).color(GOOD)
            };
            let mut c = column![heading].spacing(3);
            for line in lines {
                c = c.push(muted(line).size(12));
            }
            c.into()
        } else if lines.is_empty() {
            muted(heading).size(12).into()
        } else {
            refused = true;
            caution(heading, lines)
        };
        playback = playback.push(said);
    }
    let mut controls = column![].spacing(8);
    // A generation under way, off the window's thread: how far it has got,
    // and Cancel.
    if let Some(generating) = generation_progress(app) {
        controls = controls.push(generating);
    }
    if ready {
        controls = controls.push(row![
            btn(if app.playing || app.catching_up.is_some_and(|c| c.2) { "Pause" } else if app.progress >= 1. { "Replay" } else { "Play" }, Message::Play, app.playing),
            btn("Restart", Message::Scrub(0.), false),
            help::wrap(slider(0.0..=1.0, app.progress, Message::Scrub).on_release(Message::ScrubDone).step(0.001_f32).width(Fill), Help::text("Simulation progress", "Scrub forward or backward. Playback recomputes the remaining stock and cutter position at that time. Restart restores the stock left by all preceding enabled operations and setups."), widget::tooltip::Position::Top),
            // As wide as "100%", so the slider keeps its length.
            muted(format!("{}%", (app.progress * 100.) as u32)).width(34).align_x(Alignment::End),
            help::wrap(pick_list(if app.active_simulation().is_some() { vec![1f32, 10., 30., 60., 120.] } else { vec![0.5f32, 1., 2., 4.] }, Some(app.speed), Message::Speed).text_size(12).style(choice_style).padding(5),crate::choice_help::selected("speed","Playback speed",&app.speed.to_string(),Target::Operation,&Fields::new()),widget::tooltip::Position::Top),
            muted("×")
        ].spacing(8).align_y(Alignment::Center));
        if app.scrub_pending {
            controls = controls.push(
                muted("A long toolpath: the stock catches up when the slider stops.").size(12),
            );
        } else if app.catching_up.is_some() {
            controls = controls.push(muted("Replaying the toolpath to here…").size(12));
        }
        if let Some(sim) = app.active_simulation() {
            // A slow stock update is said here while folded, as the Detail
            // row that says it otherwise is hidden.
            let summary = match stock_surface_notice(app).filter(|_| folded) {
                Some(notice) => bold(notice, 12).color(WARN),
                None => muted(format!(
                    "{:.1} / {:.1} min · ≈{:.1} cm³ this operation · ≈{:.1} cm³ total removed",
                    sim.seconds / 60.,
                    sim.plan.duration / 60.,
                    (sim.removed_volume() - sim.inherited_volume()) / 1000.,
                    sim.removed_volume() / 1000.
                ))
                .size(12),
            };
            controls = controls.push(
                row![
                    container(summary.wrapping(Wrapping::None))
                        .width(Fill)
                        .clip(true),
                    secondary(
                        if folded { "Details" } else { "Hide details" },
                        Message::ToggleSimDetails,
                    )
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
            // Cutting nothing is a warning, so it stays when folded.
            if folded && sim.air.is_some_and(|air| air.nothing) {
                controls = controls.push(
                    bold(
                        "Cuts nothing on the stock it starts from. Disable it, or keep it as a spring pass.",
                        12,
                    )
                    .color(WARN),
                );
            }
        }
        if let Some(sim) = app.active_simulation().filter(|_| !folded) {
            playback = playback.push(
                muted(format!(
                    "{} · Spindle {} · Stock after {} earlier operations{}",
                    sim.plan.milling_direction,
                    if sim.plan.spindle_clockwise {
                        "CW"
                    } else {
                        "CCW"
                    },
                    sim.preceding,
                    sim.plan
                        .rest_top
                        .map_or(String::new(), |z| format!(" · Cutting start Z {z:.2} mm"))
                ))
                .size(12),
            );
            // What `air::trim` took out of the path.
            if let Some(air) = sim.air {
                playback = playback.push(if air.nothing {
                    bold(
                        "Cuts nothing on the stock it starts from. Disable it, or keep it as a spring pass.",
                        12,
                    )
                    .color(WARN)
                } else {
                    muted(format!(
                        "Air taken out: {} stretch{} ({:.0} mm of feed) replaced by rapids, {:.1} min saved",
                        air.stretches,
                        if air.stretches == 1 { "" } else { "es" },
                        air.air,
                        air.saved / 60.
                    ))
                    .size(12)
                });
            }
            // The scale the path's colours are in (`tooth_load::colour`).
            if sim.tooth_load.is_none()
                && app
                    .tooth_load_job
                    .as_ref()
                    .is_some_and(|(id, _)| *id == sim.id)
            {
                playback = playback.push(muted("Working out the load on each tooth…").size(12));
            }
            if let Some(held) = &sim.held {
                playback = playback.push(
                    muted(if held.slowest < held.fastest {
                        format!(
                            "Feed {:.0}–{:.0} mm/min to hold {:.0} N per tooth",
                            held.slowest, held.fastest, held.target
                        )
                    } else {
                        format!(
                            "Feed {:.0} mm/min holds {:.0} N per tooth or less",
                            held.fastest, held.target
                        )
                    })
                    .size(12),
                );
            }
            if let Some(load) = &sim.tooth_load {
                playback = playback.push(
                    muted(if load.peak > 0. {
                        format!(
                            "Path coloured by the force on one tooth: blue light, red {:.0} N, this operation's most · grey cuts nothing",
                            load.peak
                        )
                    } else {
                        "Path grey: no tooth of this operation meets material".into()
                    })
                    .size(12),
                );
            }
            // A slow update is said in place of the details, on their one
            // line: the strip keeps its height while Play meshes each frame.
            let surface_status = match stock_surface_notice(app) {
                Some(notice) => bold(notice, 12).color(WARN),
                None => muted(stock_surface_details(app).unwrap_or_default()).size(12),
            };
            let surface_status = container(surface_status.wrapping(Wrapping::None))
                .width(Fill)
                .clip(true);
            let mut detail = row![muted("Detail").size(12)]
                .spacing(5)
                .align_y(Alignment::Center);
            for choice in crate::stock_surface::Detail::ALL {
                detail = detail.push(btn(
                    choice.to_string(),
                    Message::StockDetail(choice),
                    choice == app.stock_detail,
                ));
            }
            playback = playback.push(detail.push(surface_status));
        }
    } else if app.generating.is_none() {
        let mut empty = row![muted(app.playback_reason()).size(12).width(Fill)]
            .spacing(10)
            .align_y(Alignment::Center);
        if imported_facing {
            // The inspector's Generate is the region's filled command.
            empty = empty.push(secondary("Generate & play", Message::Play));
        } else if app.is_sample() && app.operation().is_some_and(|o| o.enabled) {
            empty = empty.push(secondary("Generate sample preview", Message::Generate));
        }
        controls = controls.push(empty);
    }
    let simulation = container(playback).padding([12, 16]).style(panel_style);
    let controls = container(controls).padding([12, 16]).style(panel_style);
    (toolbar.into(), simulation.into(), controls.into(), refused)
}
/// How long the stock mesher works before the strip says so: Play meshes
/// the stock again for each frame, mostly in well under this.
pub(crate) const STOCK_NOTICE_AFTER: std::time::Duration = std::time::Duration::from_secs(1);
/// What the simulation strip's Detail line says once the stock mesher has
/// run a while (`STOCK_NOTICE_AFTER`), or at once after a job that did, so
/// that slow updates one after another keep it: for how long it has been
/// working, and what is drawn meanwhile. The time comes first, so that a
/// narrow strip cuts off the words rather than it.
pub(crate) fn stock_surface_notice(app: &App) -> Option<String> {
    let elapsed = app.stock_mesh_elapsed()?;
    if elapsed < STOCK_NOTICE_AFTER && !app.stock_mesh_slow() {
        return None;
    }
    let seconds = elapsed.as_secs();
    let sim = app.active_simulation()?;
    app.stock_mesh_key()?;
    Some(if sim.stock_surface.is_some() {
        format!("Updating the stock surface · {seconds} s, showing the last finished one")
    } else {
        format!("Building the stock surface · {seconds} s")
    })
}
/// The finished stock surface's cell size and triangle count, the details
/// line under the timeline; "Building stock surface…" before there is one.
pub(crate) fn stock_surface_details(app: &App) -> Option<String> {
    let sim = app.active_simulation()?;
    Some(match &sim.stock_surface {
        Some(surface) => format!(
            "{:.2} mm finest cell · {} triangles",
            surface.cell_mm,
            surface.triangles.len()
        ),
        None => "Building stock surface…".into(),
    })
}
/// What the inspector shows: `head` and `foot` stay put, `body` scrolls.
struct Panel<'a> {
    head: Element<'a, Message>,
    body: Element<'a, Message>,
    foot: Option<Element<'a, Message>>,
}
pub(crate) fn inspector(app: &App) -> Element<'_, Message> {
    let layout = &app.settings.layout;
    if !layout.open(Side::Inspector) {
        return strip(app, Side::Inspector);
    }
    let (_, width) = fitted(layout, app.window_width);
    let panel = if app.doc.active_setup().is_none() {
        first_setup(app)
    } else if let Some(o) = app.operation() {
        operation_editor(app, o)
    } else {
        setup_editor(app)
    };
    // The body's scrollbar is drawn over its right edge: the content stops
    // short of it, and the fixed parts line up with the content.
    let inner = width - 2. * INSPECTOR_PAD - SCROLLBAR_GUTTER;
    let mut c = column![
        container(panel.head).width(inner),
        scrollable(container(panel.body).width(inner))
            .id(INSPECTOR_ID)
            .width(Fill)
            .height(Fill)
    ]
    .spacing(12);
    if let Some(foot) = panel.foot {
        c = c.push(container(foot).width(inner));
    }
    container(c)
        .padding(INSPECTOR_PAD)
        .width(width)
        .height(Fill)
        .style(panel_style)
        .into()
}
/// A tab of the operation inspector: its label centred, so that a row of
/// them, each a `FillPortion`, reads as one width.
fn tab<'a>(label: &'static str, active: bool) -> HelpButton<'a> {
    let msg = Message::Tab(label.to_string());
    let b = button(text(label).size(11).align_x(Alignment::Center).width(Fill))
        .on_press(msg.clone())
        .padding([7, 2])
        .style(move |_, st| flat(active, st));
    HelpButton::new(b, &msg, label).width(Length::FillPortion(1))
}
/// A row that opens or closes what follows it: ▸ closed, ▾ open.
fn about<'a>(label: String, open: bool) -> Element<'a, Message> {
    let msg = Message::ToggleStrategyHelp;
    let b = button(
        row![
            text(if open { "▾" } else { "▸" }).size(11),
            text(label.clone()).size(12)
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .on_press(msg.clone())
    .padding([5, 8])
    .width(Fill)
    .style(|_, st| flat(false, st));
    HelpButton::new(b, &msg, label).into()
}
fn operation_editor<'a>(app: &'a App, o: &'a Operation) -> Panel<'a> {
    let mut checks = Checks::new(app);
    operation_editor_with(app, o, &mut checks)
}
/// The editor, placing `checks` at their fields as it draws them.
fn operation_editor_with<'a>(app: &'a App, o: &'a Operation, checks: &mut Checks) -> Panel<'a> {
    // Pinned: the operation's identity and its tabs, in rows of one width.
    let identity = row![
        icon(app, o.strategy, 30.),
        column![bold(o.strategy.name(), 17), muted("Operation parameters")]
            .spacing(2)
            .width(Fill),
        fold(app, Side::Inspector)
    ]
    .spacing(9)
    .align_y(Alignment::Center);
    let all = o.tabs(&app.doc.setup().machine);
    let mut tabs = Column::new().spacing(2);
    // Five tabs fit one row; the sixth, Orientation, takes two rows of three.
    for chunk in all.chunks(if all.len() > 5 { 3 } else { all.len().max(1) }) {
        let mut r = Row::new().spacing(1);
        for t in chunk {
            r = r.push(tab(t, app.tab == *t));
        }
        tabs = tabs.push(r)
    }
    let head = column![identity, tabs, rule::horizontal(1)].spacing(13);
    let mut c = column![checks.field(&o.p, "name", "Operation name")].spacing(13);
    // The strategy's explanation and availability, behind a disclosure that
    // stays as it was left.
    let open = app.settings.layout.strategy_help_open;
    c = c.push(about(format!("About {}", o.strategy.name()), open));
    if open {
        c = c.push(note(format!(
            "{}\n\n{}",
            o.strategy.help(),
            o.strategy.availability()
        )));
    }
    let p = &o.p;
    let t = Target::Operation;
    let machine = &app.doc.setup().machine;
    let lathe = lathe::is_lathe(machine);
    match app.tab.as_str() {
        "Tool" => {
            c = c.push(section("Cutter"));
            // The operation offers the tools of the types it cuts with (a
            // lathe's, those that fit the lathe), and the one it has.
            let offered: Vec<_> = (app.doc.tools.iter())
                .filter(|tool| {
                    o.tool == tool.id || (o.takes(tool, machine) && (!lathe || tool.fits(machine)))
                })
                .collect();
            if !offered.iter().any(|tool| o.takes(tool, machine)) {
                let types: Vec<String> = (o.tool_types(machine).iter())
                    .map(|k| k.to_lowercase())
                    .collect();
                c = c.push(
                    muted(format!(
                        "{} cuts with a {}: add one in the tool library.",
                        o.strategy.name(),
                        types.join(" or ")
                    ))
                    .size(12),
                );
            }
            for tool in offered {
                c = c.push(
                    btn(
                        tool_label(tool),
                        Message::SetOpTool(tool.id.clone()),
                        o.tool == tool.id,
                    )
                    .width(Fill),
                );
            }
            c = c
                .push(secondary("Open tool library", Message::Page(Page::Tools)))
                .push(section("Feeds & speeds"))
                .push(checks.field(p, "rpm", "Spindle speed · rpm"));
            if o.strategy.imported_preview()
                && !matches!(o.strategy, Strategy::Box | Strategy::Turn)
                && !lathe
                && !app.is_sample()
            {
                c = c
                    .push(choice_value(
                        p,
                        facing_setting(p, "spindle_rotation"),
                        "spindle_rotation",
                        "Spindle rotation",
                        &SPINDLE_ROTATIONS,
                        t,
                    ))
                    .push(
                        muted(
                            "Viewed from spindle toward the part. Match the cutter's cutting hand.",
                        )
                        .size(12),
                    );
            }
            if o.strategy == Strategy::Turn {
                c = c.push(muted("Feed: the carriage power feed, or by hand.").size(12));
            } else {
                c = c.push(checks.field(p, "feed", "Cutting feed · mm/min"));
            }
            if crate::tooth_load::offered(o.strategy, machine) {
                let follows = crate::tooth_load::follows(o, machine);
                c = c.push(choice_value(
                    p,
                    crate::tooth_load::FEED_FOLLOWS[follows as usize],
                    "feed_follows",
                    "Feed along the path",
                    &crate::tooth_load::FEED_FOLLOWS,
                    t,
                ));
                if follows {
                    c = c.push(checks.hinted(
                        p,
                        "tooth_force",
                        "Force per tooth to hold · N",
                        "Automatic",
                    ));
                    let tool = app.doc.tools.iter().find(|tool| tool.id == o.tool);
                    let automatic = tool.and_then(|tool| {
                        // No nominal cut: the force along the path, once measured.
                        if crate::tooth_load::from_the_path(o.strategy) {
                            return Some("Automatic: the force one tooth takes along most of the path at the cutting feed, measured when the operation is generated. The simulation says the force held.".into());
                        }
                        let cut = crate::tooth_load::nominal_cut(o, tool, None)?;
                        let force = crate::tooth_load::nominal(
                            &cut,
                            num(&tool.p, "flutes")?,
                            num(&o.p, "rpm")?,
                            num(&o.p, "feed")?,
                            app.doc.selected_material(),
                        )?;
                        Some(format!(
                            "Automatic: one tooth's force at the cutting feed on the nominal cut, {} as deep as a level: {force:.0} N at the full {:.1} mm stepdown. The simulation says the force held.",
                            cut.width
                                .map_or("a full slot".into(), |w| format!("{w:.1} mm wide")),
                            cut.depth
                        ))
                    });
                    c = c.push(
                        muted(automatic.unwrap_or_else(|| {
                            "Automatic: needs the tool's flutes, the spindle speed and the feed"
                                .into()
                        }))
                        .size(12),
                    );
                }
            }
            c = c.push(choice(
                p,
                "coolant",
                "Coolant",
                &["Flood", "Mist", "Air", "Off"],
                t,
            ));
            // A speed or feed outside the material's cutting data is a
            // warning at its field; why none is checked is said here.
            if let Some(SpeedFeed::Unchecked(why)) = app.doc.speed_feed(app.doc.setup(), o) {
                c = c.push(muted(format!("Not checked. {why}")));
            }
        }
        "Geometry" => {
            if o.strategy == Strategy::Chamfer {
                c = c.push(choice_value(
                    p,
                    chamfer::setting(p, "chamfer_source"),
                    "chamfer_source",
                    "Put the cone's side on",
                    &chamfer::SOURCES,
                    t,
                ));
            }
            if let Some(file) = &app.doc.file {
                c = c.push(muted(format!("Part · {}", file.name)));
                if o.strategy == Strategy::Drill {
                    c = c.push(choice_value(
                        p,
                        DRILL_HOLES
                            .into_iter()
                            .find(|h| *h == get(p, "holes"))
                            .unwrap_or(DRILL_HOLES[0]),
                        "holes",
                        "Holes",
                        &DRILL_HOLES,
                        t,
                    ));
                }
                if o.drills_pin_holes() {
                    c = c.push(pin_hole_drilling(app, o));
                } else if !o.needs_surfaces() && crate::picks_faces(o) {
                    c = c.push(note("Finishes the whole imported part in this setup. To finish only some of its faces, flat or curved, set Machining boundary to Selected geometry and pick them."));
                } else if o.strategy == Strategy::Chamfer && !o.needs_surfaces() {
                    // A line needs no pick: its fields are below.
                } else if !o.needs_surfaces() && o.strategy.imported_preview() {
                    c = c.push(note("Uses the current imported model in this setup. No demonstration geometry or saved sample features are used."));
                } else if o.strategy.imported_preview() {
                    c = c.push(note(if o.strategy == Strategy::Chamfer { "Click a level face of your part, one facing up: its outline and the mouths of its holes are the edges to chamfer. Click it again to drop it. To break only one edge, also click the wall beside it: only the edge the level face shares with that wall is chamfered. Amber = hover; blue = selected. Click Finish picking when done." } else if o.strategy == Strategy::Groove { "Click the wall of the bore the groove goes in: the whole hole is picked, its walls blue. For a groove round a boss, click every face of the boss's outside wall instead. Amber = hover. Click Finish picking when done." } else if o.strategy == Strategy::Drill { "While picking, every round hole of the part is outlined in teal with the diameter it drills. Click a hole (its outline, its wall or inside it) to drill it: its outline turns blue. Click it again to drop it. Or click a horizontal rim face: every round opening in it is drilled. The diameter filter can restrict the holes. Amber = hover." } else if matches!(o.strategy, Strategy::Pocket | Strategy::Contour) { "Click the recessed pocket floor on your part: its outline defines the walls, and holes in the floor preserve islands. Or pick a whole hole: hold the pointer over it until its outline shows, then click the outline or the hole's wall. Its outline is cut down to its floor, or through the part if nothing is under it. Picked holes' outlines are blue; click one again to drop the hole. In a Pocket, a sloped face with no hole past it (a ramp) is cleared in steps a stepdown apart. Amber = hover; blue = selected. Click Finish picking when done." } else if o.strategy == Strategy::Adaptive { "Click the recessed pocket floor on your part. Its outline defines the walls; holes in the floor preserve islands. Or click a sloped face (a ramp, a fillet): the whole face is picked, and the stock over it is cleared in steps a stepdown apart, for a finishing pass. Amber = hover; blue = selected. Click Finish picking when done." } else if crate::picks_faces(o) { "Click any face of your part, flat or curved: the whole face, up to its sharp edges, is picked. Click it again to drop it. Amber = hover; blue = selected. Drag to orbit, then click Finish picking." } else { "Click a planar surface on your part. Amber = hover; blue = selected. Drag to orbit, then click Finish picking." }));
                    if app.imported_mesh.is_some() {
                        c = c.push(
                            btn(
                                if app.picking {
                                    "Finish picking"
                                } else {
                                    "Select surfaces on part"
                                },
                                Message::PickMode,
                                app.picking,
                            )
                            .width(Fill),
                        );
                    }
                    if app.finding_holes() {
                        c = c.push(muted("Finding the part's holes: their outlines show when the pointer is over them once found."));
                    }
                    let selection = o.surfaces.as_ref();
                    // A Pocket or an inside Contour lists the holes its
                    // selected walls pick, and its floors as surfaces; a
                    // Drill lists the holes it drills (below).
                    let picks_holes = crate::picks_holes(o);
                    let drill = o.strategy == Strategy::Drill;
                    let wall = |id: usize| {
                        picks_holes
                            && app
                                .imported_mesh
                                .as_ref()
                                .is_some_and(|m| crate::pocket::is_wall(app.doc.setup(), m, id))
                    };
                    // Parallel and Waterline finishing list whole faces: a
                    // curved one is many surfaces of one face.
                    let faces = crate::picks_faces(o);
                    let face_of = |id: usize| {
                        (app.imported_mesh.as_ref())
                            .filter(|_| faces)
                            .and_then(|m| m.regions.get(id).copied())
                    };
                    let count = if faces {
                        (selection.iter())
                            .flat_map(|s| s.ids.iter().map(|&id| face_of(id).unwrap_or(id)))
                            .collect::<std::collections::HashSet<_>>()
                            .len()
                    } else {
                        selection.map_or(0, |s| s.ids.iter().filter(|&&id| !wall(id)).count())
                    };
                    let walls = selection.is_some_and(|s| s.ids.iter().any(|&id| wall(id)));
                    // Floors are surfaces; holes are listed by their walls.
                    if faces {
                        c = c.push(muted(format!("{} selected", plural(count, "face"))));
                    } else if !picks_holes {
                        c = c.push(muted(format!("{} selected", plural(count, "surface"))));
                    } else if drill {
                        if count > 0 {
                            c = c.push(muted(format!("{} selected", plural(count, "rim face"))));
                        }
                    } else if count > 0 || !walls {
                        c = c.push(muted(format!(
                            "{} selected",
                            plural(count, "floor surface")
                        )));
                    }
                    if let (true, false, Some(selected), Some(mesh)) =
                        (walls, drill, selection, &app.imported_mesh)
                        && selected.matches(mesh)
                    {
                        match &*app.pocket_holes(o, mesh) {
                            Ok(holes) => {
                                c = c.push(muted(format!(
                                    "{} picked by their walls",
                                    plural(holes.len(), "hole")
                                )));
                                for (i, h) in holes.iter().enumerate().take(64) {
                                    let (x, y) = h.centre();
                                    let (w, d) = h.size();
                                    let mut depth = (h.levels.iter())
                                        .map(|l| h.depth(l))
                                        .collect::<Vec<_>>()
                                        .join(", ");
                                    // What the cutter cannot get into.
                                    if let Some((diameter, fits, cleared)) =
                                        app.pocket_fits(o, mesh).as_ref()
                                    {
                                        let fits = &fits[i];
                                        if fits.iter().all(|f| !f) {
                                            depth += &match cleared[i] {
                                                Some(c) => format!(
                                                    " · too narrow for Ø{diameter}: cleared to {} only",
                                                    c.short()
                                                ),
                                                None => format!(
                                                    " · too narrow for Ø{diameter}: left uncut"
                                                ),
                                            };
                                        } else if fits.iter().any(|f| !f) {
                                            depth += &format!(" · part too narrow for Ø{diameter}");
                                        }
                                    }
                                    c = c.push(
                                        btn(
                                            format!("✓ Hole X{x:.2} Y{y:.2} · {w:.2} × {d:.2} mm · {depth} · remove"),
                                            Message::DropHole(i),
                                            true,
                                        )
                                        .width(Fill),
                                    );
                                }
                            }
                            Err(error) => c = c.push(note(error.clone())),
                        }
                    }
                    if let (true, Some(selected), Some(mesh)) =
                        (faces, selection, &app.imported_mesh)
                        && selected.matches(mesh)
                    {
                        // Each face once, by its first surface picked, with
                        // the area of all of its surfaces.
                        let mut listed: Vec<(usize, usize, f32)> = vec![];
                        for &id in &selected.ids {
                            let Some(surface) = mesh.surfaces.get(id) else {
                                continue;
                            };
                            let face = face_of(id).unwrap_or(id);
                            match listed.iter_mut().find(|l| l.0 == face) {
                                Some(l) => l.2 += surface.area,
                                None => listed.push((face, id, surface.area)),
                            }
                        }
                        for (face, id, area) in listed.into_iter().take(64) {
                            c = c.push(
                                btn(
                                    format!(
                                        "✓ Face {} · {:.1} {}² · remove",
                                        face + 1,
                                        area / mesh.display_scale().powi(2),
                                        mesh.units
                                    ),
                                    Message::Surface(id),
                                    true,
                                )
                                .width(Fill),
                            );
                        }
                    } else if let (Some(selected), Some(mesh)) = (selection, &app.imported_mesh)
                        && selected.matches(mesh)
                    {
                        for &id in selected.ids.iter().filter(|&&id| !wall(id)).take(64) {
                            if let Some(surface) = mesh.surfaces.get(id) {
                                c = c.push(
                                    btn(
                                        format!(
                                            "✓ Surface {} · {:.1} {}² · remove",
                                            id + 1,
                                            surface.area / mesh.display_scale().powi(2),
                                            mesh.units
                                        ),
                                        Message::Surface(id),
                                        true,
                                    )
                                    .width(Fill),
                                );
                            }
                        }
                    }
                    if selection.is_some() {
                        c = c.push(secondary("Clear surface selection", Message::ClearSurfaces));
                    }
                    if o.strategy == Strategy::Drill {
                        c = c.push(checks.field(
                            p,
                            "hole_diameter",
                            "Hole diameter filter · mm (0 = all)",
                        ));
                        if let Some(mesh) = &app.imported_mesh {
                            match &*app.drill_holes(o, mesh) {
                                Ok(holes) => {
                                    // A hole picked by its walls has a remove
                                    // button; a rim face's go with the face.
                                    let picked = app.pocket_holes(o, mesh);
                                    let by_walls = |h: &crate::cam::Hole| {
                                        (picked.as_ref().as_ref().ok())
                                            .and_then(|p| picked_row(p, h))
                                    };
                                    c = c.push(muted(format!(
                                        "{} to drill",
                                        plural(holes.len(), "hole")
                                    )));
                                    for h in holes.iter().take(64) {
                                        let said = format!(
                                            "Ø{:.2} (opening Ø{:.2}) · X{:.2} Y{:.2}{} · depth {:.2} mm · {}{}",
                                            h.diameter,
                                            h.opening,
                                            // No "-0.00" for a centre on an axis.
                                            if h.center[0].abs() < 0.005 {
                                                0.
                                            } else {
                                                h.center[0]
                                            },
                                            if h.center[1].abs() < 0.005 {
                                                0.
                                            } else {
                                                h.center[1]
                                            },
                                            // At several A angles, read at its own.
                                            crate::indexed::at_angle(h.a),
                                            h.top - h.bottom,
                                            if h.through { "through" } else { "blind" },
                                            h.countersink.map_or(String::new(), |c| format!(
                                                " · {:.1}° countersink {:.2} mm deep",
                                                c.angle, c.depth
                                            ))
                                        );
                                        c = c.push(match by_walls(h) {
                                            Some(i) => Element::from(
                                                btn(
                                                    format!("✓ {said} · remove"),
                                                    Message::DropHole(i),
                                                    true,
                                                )
                                                .width(Fill),
                                            ),
                                            None => muted(said).into(),
                                        });
                                    }
                                }
                                Err(error) => c = c.push(note(error.clone())),
                            }
                        }
                    }
                    c = c.push(note(match o.strategy {
                        Strategy::Adaptive => "Adaptive clearing works on the same geometry as Pocket: the selected recessed flat floors, with the walls and islands of the part. Vertical access only; overhanging geometry is rejected. The path keeps the cutter's side load near the optimal load, so it can take a deep stepdown. It inherits the stock left by all earlier enabled operations and setups.".to_string(),
                        Strategy::Pocket => "Pocket uses offset contours of the selected floor triangles, including islands, and of each picked hole's outline. Supports open, vertical-wall pockets with a flat floor; overhanging geometry is rejected. A picked hole is cut down to the highest of the part under it, or through the part with the breakthrough; one too narrow for the cutter has only the stock over it cleared, down to its rim. Simulation inherits the stock left by all earlier enabled operations and setups.".to_string(),
                        Strategy::Contour => "Inside contour offsets only the walls and islands of this floor, or the wall of each picked hole. It does not clear the interior. The floor must be flat and accessible along setup Z; overhangs are rejected.".to_string(),
                        Strategy::Drill => "Hole detection uses tessellated circular openings and vertical rays. Noncircular openings and obstructions are excluded. Confirm the detected centers and depths before previewing.".to_string(),
                        Strategy::Chamfer => "Chamfer milling runs the side of the cone along the edges of the picked level faces, level, with the tool in the air beside each edge. The path is checked against the part for gouging.".to_string(),
                        Strategy::Parallel => "Parallel finishing cuts only where the cutter touches a picked face. The rest of the part still holds the cutter up, so it never cuts into a face it was not given; where a picked face meets one that is not, the cutter stops at the corner between them, which keeps its radius.".to_string(),
                        Strategy::Waterline => "Waterline finishing cuts each level's loop only where the face nearest the cutter is a picked one, and goes no lower than the lowest picked face. Where a picked face meets one that is not, the cutter finishes up to the corner between them. The rest of the part still keeps the cutter out, so it never cuts into a face it was not given.".to_string(),
                        _ => fixture_note(app, Note::Facing),
                    }));
                } else {
                    c = c.push(note("This operation is unavailable for imported parts. Open Operation support for its status."));
                }
            } else if app.is_sample() {
                c = c.push(muted("Sample machining features"));
                for f in o.strategy.features() {
                    let n = *f;
                    c = c.push(help::wrap(
                        checkbox(o.geometry.contains(&n))
                            .label(FEATURES[n])
                            .on_toggle(move |_| Message::Feature(n))
                            .text_size(13),
                        help::command(&Message::Feature(n), FEATURES[n].into()),
                        widget::tooltip::Position::Left,
                    ));
                }
                c = c
                    .push(
                        btn(
                            if app.picking {
                                "Finish picking"
                            } else {
                                "Select in sample viewport"
                            },
                            Message::PickMode,
                            app.picking,
                        )
                        .width(Fill),
                    )
                    .push(muted(format!(
                        "{} selected",
                        plural(o.geometry.len(), "feature")
                    )));
            } else if o.strategy != Strategy::Chamfer || o.needs_surfaces() {
                c = c.push(note("Import a part before selecting geometry."));
            }
            if o.strategy == Strategy::Chamfer {
                c = c.push(chamfer_geometry(o, checks));
            }
            if !app.is_sample() && o.faces_outline() {
                c = c.push(note("Model silhouette faces the part's outline seen from above, holes through it included, grown by Clear past outline: how far the cutter's edge reaches beyond it. The stock outside stays at its height for a later operation. On the part's outer corners a round cutter needs some room past the outline, about a third of its radius for a square corner and more for a sharper one; Generate refuses less and says the least that clears them."));
            } else if !app.is_sample() && o.strategy == Strategy::Face {
                c = c.push(note("Facing preview covers the XY bounding rectangle of the selected surfaces or the stock. It includes holes and gaps within that rectangle. Containment and radial allowance adjust the cutter-center limits."));
            }
            if app.is_sample()
                || matches!(
                    o.strategy,
                    Strategy::Face | Strategy::Adaptive | Strategy::Pocket
                )
            {
                c = c.push(choice(
                    p,
                    "boundary",
                    "Machining boundary",
                    if matches!(o.strategy, Strategy::Adaptive | Strategy::Pocket)
                        && !app.is_sample()
                    {
                        &["Selected geometry"]
                    } else {
                        &["Selected geometry", "Stock silhouette", "Model silhouette"]
                    },
                    t,
                ));
                // A Facing of the outline reaches a distance past it instead.
                if o.faces_outline() && !app.is_sample() {
                    c = c.push(checks.hinted(
                        p,
                        "silhouette_extend",
                        "Clear past outline · mm",
                        crate::model::OUTLINE_EXTEND,
                    ));
                } else {
                    c = c.push(choice(
                        p,
                        "containment",
                        "Tool containment",
                        if matches!(o.strategy, Strategy::Adaptive | Strategy::Pocket)
                            && !app.is_sample()
                        {
                            &["Tool inside boundary"]
                        } else {
                            &[
                                "Tool inside boundary",
                                "Tool center on boundary",
                                "Tool outside boundary",
                            ]
                        },
                        t,
                    ));
                }
            } else if o.strategy.imported_preview()
                && !matches!(o.strategy, Strategy::Drill | Strategy::Chamfer)
            {
                c = c.push(choice(
                    p,
                    "boundary",
                    "Machining boundary",
                    if matches!(
                        o.strategy,
                        Strategy::Contour | Strategy::Parallel | Strategy::Waterline
                    ) {
                        &["Model silhouette", "Selected geometry"]
                    } else {
                        &["Model silhouette"]
                    },
                    t,
                ));
            }
        }
        "Box" => {
            c = c.push(note("The block to remove, in the setup's frame from the work zero. Passes run along the longer side."));
            for (a, b) in [
                (("x_min", "X min · mm"), ("x_max", "X max · mm")),
                (("y_min", "Y min · mm"), ("y_max", "Y max · mm")),
                (("z_top", "Z top · mm"), ("z_bottom", "Z bottom · mm")),
            ] {
                c = c.push(row![checks.field(p, a.0, a.1), checks.field(p, b.0, b.1)].spacing(9));
            }
            if app.part_mesh().is_some() && !app.is_sample() {
                let count = o.surfaces.as_ref().map_or(0, |s| s.ids.len());
                c = c
                    .push(section("From the part"))
                    .push(note("Pick one flat face: a floor facing up, or a wall facing sideways. Drag to orbit, then click Finish picking."))
                    .push(
                        btn(
                            if app.picking {
                                "Finish picking"
                            } else {
                                "Select face on part"
                            },
                            Message::PickMode,
                            app.picking,
                        )
                        .width(Fill),
                    )
                    .push(muted(format!("{} selected", plural(count, "face"))))
                    .push(secondary("From selected face", Message::BoxFromFace).width(Fill));
            }
        }
        "Turn" => {
            c = c.push(turn_fields(o, checks));
        }
        "Groove" => {
            c = c
                .push(row![checks.field(p, "groove_diameter", "Groove diameter · mm"), checks.field(p, "groove_width", "Groove width · mm")].spacing(9))
                .push(choice(p, "groove_from", "Where it is", &crate::groove::GROOVE_FROM, t))
                .push(checks.field(
                    p,
                    "groove_land",
                    if get(p, "groove_from") == crate::groove::GROOVE_FROM[2] {
                        "Depth of its top edge below the stock top · mm"
                    } else {
                        "Land: from the bore's end to the groove · mm"
                    },
                ))
                .push(choice(p, "spring_pass", "Spring pass", &["Yes", "No"], t))
                .push(checks.field(p, "tolerance", "Tolerance · mm"))
                .push(checks.field(p, "hole_diameter", "Bore diameter · mm (0: the bore picked)"))
                .push(note("The keyseat head goes down the bore's axis to the groove, arcs out to its diameter, circles once (twice with a spring pass), arcs back to the axis and comes up it. A groove wider than the head takes as many passes along the axis as cover it. The head must fit down the bore, its neck must clear the bore while it cuts, and it must reach the groove's far edge below its shank. Round a boss it goes down outside it."));
        }
        "Centre" => {
            c = c
                .push(note(
                    "Drills on the centre line from the tailstock, in from the free end.",
                ))
                .push(checks.field(p, "depth", "Depth · mm"));
        }
        "Heights" => {
            c = c.push(note(if o.strategy == Strategy::Box {
                "Clearance and retract are measured above the stock top. The block's own depths are set in Box."
            } else if !app.is_sample() && o.strategy == Strategy::Face {
                "Facing uses remaining stock to skip fully cleared top layers. Bottom offset sets the target relative to the selected plane; it must not cut below the highest model feature. Clearance/retract retain their original stock reference."
            } else if !app.is_sample() && matches!(o.strategy, Strategy::Adaptive | Strategy::Pocket) {
                "Each selected floor is cleared to its own height, the highest first. Cutting starts from the remaining stock after earlier operations, plus positive Top offset, and goes only where earlier cuts left material, so a smaller cutter after a larger one cleans up what that one could not reach. Bottom offset and axial stock to leave are added to each floor. Clearance/retract stay referenced to the original stock top."
            } else if !app.is_sample() && o.strategy==Strategy::Contour && get(p,"boundary")=="Selected geometry" {
                "Inside contour skips fully cleared top layers and ends at the selected floor + bottom offset. Negative bottom offsets are rejected. Profile ramp/plunge are configured in Linking; the pocket interior is not cleared. Clearance/retract retain the original stock reference."
            } else if !app.is_sample() && matches!(o.strategy, Strategy::Contour | Strategy::Waterline) {
                "Bottom offset is relative to model bottom. Contour normally starts at stock top; Waterline at model top. Earlier cuts lower the start only when the whole machining area is cleared. Positive top offset adds an approach margin. Clearance/retract retain the original stock reference."
            } else if !app.is_sample() && o.strategy == Strategy::Parallel {
                "Bottom offset is added to the cutter-compensated surface height; stock to leave is a vertical offset. Top offset raises approach heights. Finishing starts with the stock left by the earlier roughing operations."
            } else if o.strategy == Strategy::Chamfer {
                "The cut's own height is set in Passes: by the chamfer's width or by the tip's Z. Clearance and retract are measured above the stock top: the heights the tool travels at between cuts."
            } else if o.drills_pin_holes() {
                "Each pin hole goes down from the face the pins stand on, as earlier cuts leave it, 1 mm deeper than the pins stand at its full width: a drill goes on by its point under that. Top offset raises the approach. Clearance/retract retain their original reference."
            } else if !app.is_sample() && o.strategy == Strategy::Drill {
                "Hole bottoms are detected from the model. Bottom offset raises that target; breakthrough extends below it. With a chamfer mill each opening is countersunk instead: the point goes down until its cone is as wide as the opening at the rim. A hole countersunk in the model takes only a chamfer mill of its countersink's angle. Peck levels start from remaining stock at each hole, accounting for previous facing. Clearance/retract retain their original reference. Removal follows the drill's 118° point or the chamfer mill's cone."
            } else { "All heights are in the current setup coordinate system. Clearance/retract are measured above stock top; offsets are relative to selected geometry." }));
            if let Some(z) = app.active_simulation().and_then(|s| s.plan.rest_top) {
                c = c.push(note(format!("Automatic cutting start: Z {z:.2} mm. Earlier cuts cover the material above this height.")));
            }
            for (k, label) in [
                ("clearance", "Clearance above stock · mm"),
                ("retract", "Retract above stock · mm"),
                ("top", "Top offset · mm"),
                ("bottom", "Bottom offset · mm"),
            ] {
                // A Box's depths are its own, a pin hole's the pins'.
                let own = match k {
                    "top" => o.strategy != Strategy::Box,
                    "bottom" => {
                        !matches!(o.strategy, Strategy::Box | Strategy::Chamfer)
                            && !o.drills_pin_holes()
                    }
                    _ => true,
                };
                if own {
                    c = c.push(checks.field(p, k, label));
                }
            }
            if !app.is_sample()
                && o.strategy == Strategy::Contour
                && get(p, "boundary") == "Model silhouette"
                && get(p, "side") == crate::model::CONTOUR_HOLES
            {
                c = c
                    .push(checks.field(p, "hole_min", "Smallest hole to cut · mm"))
                    .push(muted("Holes narrower than this are left for a Drill; empty cuts every hole the tool fits in. The slugs come free: Holes leave no tabs."));
            } else if !app.is_sample()
                && o.strategy == Strategy::Contour
                && get(p, "boundary") == "Model silhouette"
            {
                c = c.push(choice_value(
                    p,
                    pocket_setting(p, "tabs"),
                    "tabs",
                    "Holding tabs",
                    &CONTOUR_TABS,
                    t,
                ));
                if pocket_setting(p, "tabs") == CONTOUR_TABS[1] {
                    c = c
                        .push(checks.field(p, "tab_count", "Tabs on each outline"))
                        .push(checks.field(p, "tab_width", "Tab width · mm"))
                        .push(checks.field(p, "tab_height", "Tab height · mm"));
                }
            }
            // A hole picked by its walls that nothing is under goes through,
            // as an opening does.
            let through = !app.is_sample()
                && matches!(o.strategy, Strategy::Pocket | Strategy::Contour)
                && app.imported_mesh.as_ref().is_some_and(|mesh| {
                    o.surfaces.as_ref().is_some_and(|s| {
                        s.matches(mesh)
                            && s.ids
                                .iter()
                                .any(|&id| crate::pocket::is_wall(app.doc.setup(), mesh, id))
                    }) && app
                        .pocket_holes(o, mesh)
                        .as_ref()
                        .as_ref()
                        .is_ok_and(|holes| holes.iter().any(|h| h.through()))
                });
            if !app.is_sample() && matches!(o.strategy, Strategy::Adaptive | Strategy::Pocket) {
                c = c.push(choice_value(
                    p,
                    pocket_setting(p, "openings"),
                    "openings",
                    "Openings through the floor",
                    &POCKET_OPENINGS,
                    t,
                ));
            }
            if through
                || (!app.is_sample()
                    && matches!(o.strategy, Strategy::Adaptive | Strategy::Pocket)
                    && pocket_setting(p, "openings") == POCKET_OPENINGS[1])
            {
                c = c.push(checks.field(p, "breakthrough", "Breakthrough · mm"));
            }
            if o.strategy == Strategy::Drill && !o.drills_pin_holes() {
                c = c.push(choice(
                    p,
                    "through",
                    if o.helical() {
                        "Cutter through bottom"
                    } else {
                        "Drill tip through bottom"
                    },
                    &["Yes", "No"],
                    t,
                ));
                if get(p, "through") == "Yes" {
                    c = c.push(checks.field(p, "breakthrough", "Breakthrough · mm"));
                }
            }
        }
        "Orientation" => match o.strategy {
            Strategy::Rotary => {
                c = c
                    .push(choice(
                        p,
                        "rotary_axis",
                        "Rotary axis",
                        &["A around X", "B around Y", "C around Z"],
                        t,
                    ))
                    .push(checks.field(p, "rotary_start", "Start angle · degrees"))
                    .push(checks.field(p, "rotary_end", "End angle · degrees"))
                    .push(choice(
                        p,
                        "rotary_style",
                        "Machining style",
                        &["Line", "Spiral", "Circular"],
                        t,
                    ));
            }
            Strategy::FiveAxis => {
                c = c
                    .push(choice(
                        p,
                        "axis_control",
                        "Tool axis control",
                        &["Surface normal", "Lead and lean"],
                        t,
                    ))
                    .push(checks.field(p, "tilt_min", "Minimum tilt · degrees"))
                    .push(checks.field(p, "tilt_max", "Maximum tilt · degrees"));
                if get(p, "axis_control") == "Lead and lean" {
                    c = c
                        .push(checks.field(p, "lead_angle", "Lead angle · degrees"))
                        .push(checks.field(p, "lean_angle", "Lean angle · degrees"));
                }
                c=c.push(choice(p,"collision","Collision avoidance preference",&["Retract and relink","Tilt within limits"],t)).push(note("This stores your intended strategy. Machine kinematics and collision solving are not connected yet."));
            }
            _ => {}
        },
        "Cycle" if o.drills_pin_holes() && !op_tool_is(app, o, "Drill") => {
            c = c.push(note(
                "A flat end mill mills the pin holes: pin 1's hole on a helix, a slot round the others, ramping down a quarter of its diameter a lap, then a lap at the bottom. The drilling cycles are for a drill.",
            ));
        }
        "Cycle" => {
            let cycles = drill_cycles(o.drills_pin_holes(), machine);
            c = c.push(choice(p, "cycle", "Drilling cycle", &cycles, t));
            if o.helical() {
                c = c
                    .push(checks.field(p, "stepdown", "Stepdown per turn · mm"))
                    .push(checks.field(p, "stepover", "Ring stepover · % of tool diameter"))
                    .push(checks.field(p, "tolerance", "Tolerance · mm"))
                    .push(checks.field(
                        p,
                        "bore_allowance",
                        "Leave for a finish · mm on the diameter (blank: none)",
                    ));
            }
            if o.threads() {
                // The model's own thread is read from each hole.
                let sizes: Vec<&str> = std::iter::once(FROM_MODEL)
                    .chain(crate::thread::SPECS.iter().map(|s| s.name))
                    .collect();
                c = c
                    .push(choice(p, "thread", "Thread", &sizes, t))
                    .push(choice(p, "thread_hand", "Hand", &THREAD_HANDS, t))
                    .push(checks.field(p, "thread_depth", "Thread depth · mm (blank: as modeled)"));
            }
            if o.thread_milling() {
                c = c
                    .push(checks.field(p, "thread_passes", "Thread passes"))
                    .push(choice(p, "spring_pass", "Spring pass", &["Yes", "No"], t))
                    .push(checks.field(p, "tolerance", "Tolerance · mm"));
            }
            if ["Chip breaking", "Deep drilling"].contains(&get(p, "cycle")) {
                c = c.push(checks.field(p, "peck", "Peck depth · mm"));
            }
            if get(p, "cycle") == "Counterboring" || o.finishes_holes() {
                c = c.push(checks.field(p, "dwell", "Dwell at bottom · seconds"));
            }
            if o.boring() {
                c = c
                    .push(checks.field(p, "bore_cut", "Cut per pass · mm on the radius"))
                    .push(checks.field(p, "finish_cut", "Last pass · mm on the diameter"));
            }
            if o.finishes_holes() {
                c = c.push(secondary(
                    if o.boring() {
                        "+ Add drilling and roughing before it"
                    } else {
                        "+ Add the drilling before it"
                    },
                    Message::AddHoleStages,
                ));
            }
            if !lathe {
                c = c.push(choice(
                    p,
                    "hole_order",
                    "Hole order",
                    &["Optimize travel", "By X then Y"],
                    t,
                ));
            }
            // A drill plunges the pin holes whatever the cycle says (its
            // issue says so when the cycle is Helical milling), and a lathe drills
            // on the centre.
            let milled = o.helical() && !lathe;
            c = c.push(note(match get(p, "cycle") {
                HELICAL_MILLING if !milled => "Feed to the bottom in one move, then retract.",
                "Chip breaking" => "Retract slightly after each peck to break the chip.",
                "Deep drilling" => "Retract clear of the hole between pecks to evacuate chips.",
                "Counterboring" => "Feed to depth and dwell before retracting.",
                HELICAL_MILLING => "Mill each hole with a flat end mill narrower than it: down a helix to a flat bottom, once round there, in rings out to the hole's size when it is over twice the cutter's.",
                THREAD_MILLING => "Mill each hole's thread with a thread mill: down the drilled hole, arc in to the wall, one pitch a turn, arc back out. Drill the hole first.",
                TAPPING => "Tap each drilled hole by hand: the spindle (off) or the tailstock holds the tap square. The CNC does not tap.",
                REAMING => "Ream each drilled hole to size: feed in, dwell, and feed back out, at a reamer's low speed. Drill the hole a little under size first (Ø9.8 for a Ø10 reamer).",
                BORING => "Bore each roughed hole of the Hole diameter with a boring head, a pass at each size up to it: feed in, dwell, then out with the spindle stopped or fed out, as the head says. The run pauses before each pass to measure the bore and set the head out.",
                _ => "Feed to the bottom in one move, then retract.",
            }));
        }
        "Passes" => {
            c = c.push(pass_fields(app, o, checks));
        }
        "Linking"
            if !app.is_sample()
                && (o.strategy == Strategy::Pocket
                    || (o.strategy == Strategy::Contour && get(p, "side") == "Inside")) =>
        {
            c = c.push(choice_value(
                p,
                pocket_setting(p, "pocket_entry"),
                "pocket_entry",
                "Contour entry",
                &POCKET_ENTRIES,
                t,
            ));
            if pocket_setting(p, "pocket_entry") == "Profile ramp" {
                c = c.push(checks.field(p, "ramp_angle", "Ramp angle · degrees"));
            }
            c = c.push(note("Profile ramp descends around the contour it enters at 25% feed, then completes the loop at depth. Plunge feeds straight down at 25% feed and requires a plunge-capable cutter. Between contours the tool stays down: it follows the contour just cut, then steps across at most 1.5 stepovers along a line clear of every wall and island; otherwise it retracts to clearance and enters again."));
            c = c.push(note(fixture_note(app, Note::Pocket)));
        }
        "Linking" if !app.is_sample() && o.faces_outline() => {
            c = c.push(checks.field(p, "ramp_angle", "Entry ramp angle · degrees"));
            if facing_setting(p, "facing_pattern") == FACING_PATTERNS[0] {
                c = c.push(choice_value(
                    p,
                    facing_setting(p, "facing_link"),
                    "facing_link",
                    "Between facing passes",
                    &FACING_LINKS,
                    t,
                ));
            }
            c = c.push(note("Each level enters on the pass round the outline's edge: from beside the stock where that edge runs off it, otherwise ramping down round the edge at the entry ramp angle (a loop too short for that plunges). The rows then start and end where that pass cut. With Keep tool down the cutter feeds to the next row at depth when it is near and the way stays inside the outline; otherwise it lifts just over the level before, or to clearance where the way leaves the outline, and feeds down where the edge pass cut. Entries and depth changes feed at 25% of cutting feed."));
            c = c.push(note(fixtures(Note::Pocket, modelled(app))));
        }
        "Linking" if !app.is_sample() && o.strategy == Strategy::Face => {
            if facing_setting(p, "facing_pattern") == FACING_PATTERNS[0] {
                c = c.push(choice_value(
                    p,
                    facing_setting(p, "facing_link"),
                    "facing_link",
                    "Between facing passes",
                    &FACING_LINKS,
                    t,
                ));
                c = c.push(note(if facing_setting(p,"facing_link") == FACING_LINKS[0] {
                    "Keep tool down feeds sideways to the adjacent row at cutting depth, then cuts back in the opposite direction. With Continue from previous end, the next depth feeds down at the last endpoint and reverses the entire row sequence."
                } else {
                    "Retract each pass lifts to clearance before moving to the next row. Continue from previous end still reverses the next depth's row order; Return to first start repeats each depth from the original X/Y."
                }));
            } else {
                c = c.push(note("One way uses the selected Climb or Conventional direction. It retracts to clearance for each return and starts each depth at the first row. Switch Passes → Pass pattern to Both ways for keep-down links and continuation from the previous endpoint."));
            }
            c = c.push(note(fixture_note(app, Note::Entry)));
        }
        "Linking" if !app.is_sample() && o.strategy == Strategy::Adaptive => {
            c = c.push(choice_value(
                p,
                get(p, "ramp"),
                "ramp",
                "Entry",
                &ADAPTIVE_ENTRIES,
                t,
            ));
            if get(p, "ramp") == ADAPTIVE_ENTRIES[0] {
                c = c.push(checks.field(p, "ramp_angle", "Helix ramp angle · degrees"));
            }
            c = c.push(note("Where the cutter cannot come in from the side it enters the material with a Helix at the ramp angle, or a straight Plunge that needs a plunge-capable cutter. Where the floor runs off the stock, or earlier cuts opened it, the cutter comes in from the side with no entry. Between passes the tool stays down through cleared area where the move is short; otherwise it lifts just above the level before (inside the pocket) or to clearance."));
            c = c.push(note(fixtures(Note::Pocket, modelled(app))));
        }
        "Linking" if !app.is_sample() && o.strategy.imported_preview() => {
            c = c.push(note(fixture_note(app, Note::Separate)));
        }
        "Linking" => {
            if o.strategy == Strategy::Drill {
                c=c.push(note("Drilling transitions use the Cycle and Heights settings. Set the hole order in Cycle and the retract plane in Heights."));
            } else {
                if matches!(o.strategy, Strategy::Adaptive | Strategy::Pocket) {
                    c = c.push(choice(
                        p,
                        "ramp",
                        "Entry ramp",
                        &["Helix", "Zigzag", "Plunge", "Profile"],
                        t,
                    ));
                    if get(p, "ramp") != "Plunge" {
                        c = c.push(checks.field(p, "ramp_angle", "Ramp angle · degrees"));
                    }
                }
                c = c
                    .push(checks.field(p, "lead_in", "Lead-in radius · mm"))
                    .push(choice(p, "keep_down", "Keep tool down", &["Yes", "No"], t));
            }
        }
        _ => {}
    }
    // Under the form, scrolling with it: the checks no field on the open tab
    // shows. Pinned, they could take the whole height and leave none to it.
    c = c.push(checks_summary(checks, app.warning_details));
    if let Some((heading, lines)) = holding_warning(app) {
        c = c.push(caution(heading, lines));
    }
    // Pinned under the body: Generate and the operation's actions.
    let mut foot = column![rule::horizontal(1)].spacing(13);
    foot = foot.push(
        primary(
            if app.is_sample() {
                "Generate sample preview"
            } else if o.strategy.imported_preview() {
                "Generate operation preview"
            } else {
                "Unavailable · see Operation support"
            },
            Message::Generate,
        )
        .width(Fill),
    );
    foot = if app.confirm_delete_op {
        // Asked in the place of the commands, as a setup's Delete is
        // (`setup_editor`).
        foot.push(note(format!(
            "Delete {}? Later rest-machining operations that start from it lose that reference.",
            o.name()
        )))
        .push(
            row![
                secondary("Cancel", Message::ConfirmDeleteOp(false)),
                danger("Delete operation", Message::ConfirmDeleteOp(true))
            ]
            .spacing(6),
        )
    } else {
        let (toggle, toggle_icon) = if o.enabled {
            ("Suppress", IC_SUPPRESS)
        } else {
            ("Enable", IC_ENABLE)
        };
        foot.push(
            row![
                icon_btn(IC_DUPLICATE, "Duplicate", Message::DuplicateOp, false),
                icon_btn(toggle_icon, toggle, Message::ToggleOp, false),
                icon_btn(IC_UP, "Move up", Message::MoveOp(-1), false),
                icon_btn(IC_DOWN, "Move down", Message::MoveOp(1), false),
                space().width(Fill),
                // Apart from the safe commands, at the end of the row.
                icon_btn(IC_DELETE, "Delete…", Message::AskRemoveOp, true)
            ]
            .spacing(6),
        )
    };
    Panel {
        head: head.into(),
        body: c.into(),
        foot: Some(foot.into()),
    }
}
/// A Chamfer milling's Geometry fields after its source and the picking:
/// the edges or the line, and the copies round an axis.
fn chamfer_geometry<'a>(o: &'a Operation, checks: &mut Checks) -> Element<'a, Message> {
    let p = &o.p;
    let t = Target::Operation;
    let mut c = Column::new().spacing(12);
    if chamfer::on_edges(p) {
        c = c
            .push(choice_value(
                p,
                chamfer::setting(p, "chamfer_edges"),
                "chamfer_edges",
                "Edges",
                &chamfer::EDGES,
                t,
            ))
            .push(checks.field(p, "extend", "Run past open ends · mm"));
    } else {
        c = c
            .push(note("The line is in work coordinates, from the setup's work zero. The cone's side is put on it, level, at the line's height; Passes sets how deep the tip goes."))
            .push(
                row![
                    checks.field(p, "from_x", "Line from X · mm"),
                    checks.field(p, "from_y", "Line from Y · mm")
                ]
                .spacing(9),
            )
            .push(
                row![
                    checks.field(p, "to_x", "Line to X · mm"),
                    checks.field(p, "to_y", "Line to Y · mm")
                ]
                .spacing(9),
            )
            .push(checks.field(p, "line_z", "Line height Z · mm"))
            .push(choice_value(
                p,
                chamfer::setting(p, "chamfer_side"),
                "chamfer_side",
                "Tool side, looking along the line",
                &chamfer::SIDES,
                t,
            ));
    }
    c.push(section("Copies round an axis"))
        .push(checks.field(p, "copies", "Copies"))
        .push(
            row![
                checks.field(p, "copy_pitch", "Angle between copies · °"),
                checks.field(p, "first_angle", "First copy's angle · °")
            ]
            .spacing(9),
        )
        .push(
            row![
                checks.field(p, "axis_x", "Axis X · mm"),
                checks.field(p, "axis_y", "Axis Y · mm")
            ]
            .spacing(9),
        )
        .push(note("Each copy is the cut turned that many degrees anticlockwise, seen from above, about the vertical axis through Axis X, Axis Y, from the work zero: copy k is turned First copy's angle plus k times the angle between copies."))
        .into()
}
/// A Turn's kind and the sizes of that kind, in lathe work coordinates.
fn turn_fields<'a>(o: &'a Operation, checks: &mut Checks) -> Element<'a, Message> {
    let p = &o.p;
    let t = Target::Operation;
    let mut c =
        Column::new()
            .spacing(12)
            .push(choice(p, "turn_kind", "Kind", &lathe::TURN_KINDS, t));
    c = c.push(note(
        "Z 0 is the free end of the bar, negative toward the chuck. Sizes are diameters.",
    ));
    match get(p, "turn_kind") {
        "Face" => {
            c = c
                .push(checks.field(p, "face_off", "Take off the end · mm"))
                .push(checks.field(p, "to_diameter", "Face to diameter · mm (0 = centre)"));
        }
        "Outside" | "Bore" => {
            c = c.push(
                row![
                    checks.field(p, "diameter", "Diameter · mm"),
                    checks.field(p, "z_end", "Z end · mm")
                ]
                .spacing(9),
            );
        }
        "Chamfer" => {
            c = c
                .push(choice(p, "edge", "Edge", &lathe::EDGES, t))
                .push(checks.field(p, "size", "Size · mm"));
        }
        "Groove" => {
            c = c
                .push(
                    row![
                        checks.field(p, "z", "Z, left side · mm"),
                        checks.field(p, "width", "Width · mm")
                    ]
                    .spacing(9),
                )
                .push(checks.field(p, "diameter", "Bottom diameter · mm"));
        }
        "Part off" => {
            c = c.push(checks.field(p, "z", "Z · mm"));
        }
        "Thread" => {
            c = c
                .push(choice(
                    p,
                    "thread_side",
                    "Thread on",
                    &lathe::THREAD_SIDES,
                    t,
                ))
                .push(
                    row![
                        checks.field(p, "diameter", "Major diameter · mm"),
                        checks.field(p, "pitch", "Pitch · mm")
                    ]
                    .spacing(9),
                )
                .push(checks.field(p, "z_end", "Z end · mm"));
            c = c.push(secondary("From the part", Message::ThreadFromPart).width(Fill));
        }
        _ => {}
    }
    c.into()
}
fn pass_fields<'a>(app: &'a App, o: &'a Operation, checks: &mut Checks) -> Element<'a, Message> {
    let p = &o.p;
    let t = Target::Operation;
    let s = o.strategy;
    let mut c = Column::new().spacing(12);
    if s == Strategy::Turn {
        return match get(p, "turn_kind") {
            "Outside" | "Bore" => c
                .push(
                    row![
                        checks.field(p, "depth", "Depth per pass · mm"),
                        checks.field(p, "finish", "Finish pass · mm")
                    ]
                    .spacing(9),
                )
                .push(checks.field(p, "allowance", "Stock to leave · mm"))
                .push(note("Equal passes down to the diameter plus the stock to leave, then the finish pass."))
                .into(),
            "Thread" => c
                .push(checks.hinted(p, "passes", "Passes", "Auto"))
                .push(note("Leave Passes empty for the usual number for the pitch."))
                .into(),
            _ => c.into(),
        };
    }
    if s == Strategy::Box {
        return c
            .push(checks.field(p, "allowance", "Stock to leave · mm"))
            .push(checks.field(p, "stepover", "Stepover · % tool diameter"))
            .push(checks.hinted(p, "stepdown", "Maximum stepdown · mm", "Auto"))
            .push(note("Stock to leave stays on every side that touches the part, for the CNC to finish; 0 makes the cut final. Leave the stepdown empty for Auto."))
            .into();
    }
    if s == Strategy::Chamfer {
        // Edges are always sized by the chamfer's width; a line also by
        // the tip's depth.
        if !chamfer::on_edges(p) {
            c = c.push(choice_value(
                p,
                chamfer::setting(p, "chamfer_size"),
                "chamfer_size",
                "Size by",
                &chamfer::SIZES,
                t,
            ));
        }
        let by_width =
            chamfer::on_edges(p) || chamfer::setting(p, "chamfer_size") == chamfer::SIZES[0];
        if by_width {
            c = c
                .push(checks.field(p, "chamfer_width", "Chamfer width · mm"))
                .push(checks.field(p, "tip_offset", "Tip below the chamfer · mm"));
        } else {
            c = c.push(checks.field(p, "tip_z", "Tip Z · mm"));
        }
        if chamfer::on_edges(p) {
            c = c.push(choice_value(
                p,
                pocket_setting(p, "pocket_milling"),
                "pocket_milling",
                "Milling direction",
                &FACING_MILLING,
                t,
            ));
        }
        return c
            .push(note(if by_width {
                "The chamfer is this wide on the face. The tip goes the tip offset below the chamfer's lower end, so the cone's side cuts all of it, and the axis is half the flat tip plus the offset times the tangent of half the tip angle away from the edge, towards the air."
            } else {
                "The tip is at this Z, from the work zero. The axis is off the line, towards the air, by the cone's radius at the line's height: half the flat tip plus the height above the tip times the tangent of half the tip angle."
            }))
            .into();
    }
    if !app.is_sample()
        && matches!(
            s,
            Strategy::Contour | Strategy::Parallel | Strategy::Waterline
        )
    {
        if s == Strategy::Parallel {
            c=c.push(checks.field(p, "direction", "Pass direction · degrees"))
                .push(checks.field(p, "surface_stepover", "Surface stepover · mm"))
                .push(checks.field(p, "slope_min", "Minimum surface slope · degrees"))
                .push(checks.field(p, "slope_max", "Maximum surface slope · degrees"))
                .push(checks.field(p, "tolerance", "Sampling tolerance · mm"))
                .push(checks.field(p, "allowance", "Vertical stock to leave · mm"))
                .push(note("Flat and ball cutter envelopes are evaluated against mesh triangles on a bounded grid. Zigzag passes follow the upper surface, so they cannot reach an undercut (cut a groove below a bore's mouth with Internal groove and a keyseat cutter); spacing/tolerance controls sampling, not certified machining accuracy."));
        } else {
            c = c
                .push(choice_value(
                    p,
                    pocket_setting(p, "pocket_milling"),
                    "pocket_milling",
                    "Milling direction",
                    &FACING_MILLING,
                    t,
                ))
                .push(checks.field(p, "stepdown", "Maximum stepdown · mm"));
            if s == Strategy::Waterline {
                c = c.push(checks.field(p, "surface_stepover", "Maximum stepover · mm"));
            }
            c = c.push(checks.field(p, "allowance", "Radial stock to leave · mm"));
            if s == Strategy::Contour {
                c=c.push(choice(p,"side","Contour side",&crate::model::CONTOUR_SIDES,t))
                    .push(choice(p,"compensation","Cutter compensation",&["In computer"],t))
                    .push(checks.field(p, "finish_passes", "Repeat finishing passes"))
                    .push(note("Outside follows the outer model silhouette at successive depths. Inside follows only the walls and islands of a selected recessed floor; select that floor in Geometry. Holes follows the inside of every hole through the part's outline, a flat part's cut-outs. Computer compensation includes the cutter radius and allowance. Repeat passes revisit the contour."));
            } else {
                c=c.push(note("Constant-Z loops follow the projected geometry above each level, with cutter-radius compensation. Levels are the stepdown apart, closer on sloping faces so neighbouring loops stay within the stepover of each other across them; faces flatter than 1° are left for Facing or Parallel. This conservative vertical-access preview supports flat cutters. Overhung detail remains unmachined."));
            }
        }
        return c.into();
    }
    if s == Strategy::Pocket && !app.is_sample() {
        c = c.push(choice_value(p, pocket_setting(p,"pocket_milling"), "pocket_milling", "Milling direction", &FACING_MILLING, t))
            .push(muted(format!("Spindle: {} · set in Tool", facing_setting(p,"spindle_rotation"))).size(12))
            .push(note("Climb / Conventional changes the direction around walls and islands. Offset contours clear from the interior outward, with a retract between loops."));
    }
    if s == Strategy::Adaptive && !app.is_sample() {
        c = c.push(choice_value(p, pocket_setting(p,"pocket_milling"), "pocket_milling", "Milling direction", &FACING_MILLING, t))
            .push(muted(format!("Spindle: {} · set in Tool", facing_setting(p,"spindle_rotation"))).size(12))
            .push(note("Climb / Conventional sets which way the cutter goes round walls and islands, and which side of the cutter the material is kept on. With a clockwise spindle, Climb keeps the material on the right. In open areas the path sweeps and loops to hold the load."));
    }
    if s == Strategy::Face && !app.is_sample() {
        c = c.push(choice_value(
            p,
            facing_setting(p, "facing_pattern"),
            "facing_pattern",
            "Pass pattern",
            &FACING_PATTERNS,
            t,
        ));
        if o.faces_outline() {
            // Each level starts with the pass round its edge, which climbs
            // or not either way.
            c = c.push(choice_value(
                p,
                facing_setting(p, "facing_milling"),
                "facing_milling",
                "Milling direction",
                &FACING_MILLING,
                t,
            ));
            c = c.push(note(if facing_setting(p, "facing_pattern") == FACING_PATTERNS[0] {
                "Each level goes once round the edge of the outline grown by Clear past outline (Geometry), in this milling direction, then clears inside it in rows both ways, mixing climb and conventional. Every row starts and ends in what that edge pass cut."
            } else {
                "Each level goes once round the edge of the outline grown by Clear past outline (Geometry), then clears inside it in rows all cut one way, in this milling direction, lifting between them. Every row starts and ends in what that edge pass cut."
            }));
        } else if facing_setting(p, "facing_pattern") == FACING_PATTERNS[0] {
            c = c.push(note("Mixed climb / conventional. The cutter travels both ways while the spindle keeps the same rotation. Choose One way to control the milling direction."));
            c = c.push(choice_value(
                p,
                facing_setting(p, "facing_depth_order"),
                "facing_depth_order",
                "Next depth level",
                &FACING_DEPTH_ORDER,
                t,
            ));
        } else {
            c = c.push(choice_value(
                p,
                facing_setting(p, "facing_milling"),
                "facing_milling",
                "Milling direction",
                &FACING_MILLING,
                t,
            ));
            c = c.push(note("Returns at clearance between passes. Direction is relative to the uncut side; entry and full-width engagement can contain both cutting modes."));
        }
        c = c.push(
            muted(format!(
                "Spindle: {} · set in Tool",
                facing_setting(p, "spindle_rotation")
            ))
            .size(12),
        );
    }
    if matches!(s, Strategy::Face | Strategy::Indexed | Strategy::Parallel) {
        c = c.push(checks.field(p, "direction", "Pass direction · degrees"));
    }
    if matches!(s, Strategy::Face | Strategy::Indexed | Strategy::Pocket) {
        c = c.push(checks.field(p, "stepover", "Stepover · % tool diameter"));
    }
    if s == Strategy::Adaptive {
        c = c
            .push(checks.field(p, "optimal_load", "Optimal load · mm"))
            .push(checks.field(p, "minimum_radius", "Minimum cutting radius · mm"));
    }
    if matches!(
        s,
        Strategy::Face
            | Strategy::Indexed
            | Strategy::Adaptive
            | Strategy::Pocket
            | Strategy::Contour
            | Strategy::Waterline
    ) {
        c = c.push(checks.field(p, "stepdown", "Maximum stepdown · mm"));
    }
    if s == Strategy::Contour {
        c = c
            .push(choice(
                p,
                "side",
                "Contour side",
                &["Inside", "Outside", "On center"],
                t,
            ))
            .push(choice(
                p,
                "compensation",
                "Cutter compensation",
                &["In computer", "In control", "Wear", "Inverse wear"],
                t,
            ));
    }
    if matches!(s, Strategy::Pocket | Strategy::Contour) {
        c = c.push(checks.field(p, "finish_passes", "Finishing passes"));
    }
    if matches!(
        s,
        Strategy::Parallel
            | Strategy::Waterline
            | Strategy::Scallop
            | Strategy::Rotary
            | Strategy::FiveAxis
    ) {
        c = c
            .push(checks.field(p, "surface_stepover", "Surface stepover · mm"))
            .push(checks.field(p, "slope_min", "Minimum surface slope · degrees"))
            .push(checks.field(p, "slope_max", "Maximum surface slope · degrees"));
    }
    if s == Strategy::FiveAxis {
        c = c.push(choice(
            p,
            "pattern",
            "Surface pattern",
            &["Parallel", "Scallop"],
            t,
        ));
    }
    // A Facing of the outline clears past it by Clear past outline.
    if !(o.faces_outline() && !app.is_sample()) {
        c = c.push(checks.field(p, "allowance", "Radial stock to leave · mm"));
    }
    if matches!(s, Strategy::Adaptive | Strategy::Pocket) {
        c = c.push(checks.field(p, "axial_allowance", "Axial stock to leave · mm"));
    }
    if !app.is_sample() && s == Strategy::Face {
        c = c.push(note("The preview supports flat end mills and face mills on the highest planar face. Continuous cutter sweeps preserve removed stock; fully cleared top layers are skipped automatically. This does not establish machining accuracy or collision clearance."));
    } else if !app.is_sample() && s == Strategy::Pocket {
        c = c.push(note(fixture_note(app, Note::Passes)));
    } else {
        c = c.push(checks.field(p, "tolerance", "Tolerance · mm"));
    }
    if s == Strategy::Adaptive && !app.is_sample() {
        c = c.push(note("Optimal load is the side load the path holds (2% to 50% of the tool diameter), so the stepdown can be deeper than for Pocket (a new operation starts at 15% of the diameter and two diameters deep). Minimum cutting radius keeps turns in open material no tighter than that radius; at a wall it turns as sharply as it must. Tolerance is how far the path's straight segments may stray from the computed path. Each level ends with one pass round the walls and islands at the radial stock to leave; where that pass meets more than the load, in corners, it slows its feed to match. A notch narrower than twice the cutter can only be slotted. Rest machining is automatic: it cuts only where earlier operations and setups left material, so a smaller cutter after a larger one cleans up the corners."));
    }
    if s == Strategy::Scallop || (s == Strategy::Adaptive && app.is_sample()) {
        c = c.push(choice(p, "rest", "Rest machining", &["No", "Yes"], t));
        if get(p, "rest") == "Yes" {
            c = c.push(muted("Stock remaining after"));
            for prev in app
                .doc
                .setup()
                .operations
                .iter()
                .take(app.selected.unwrap_or(0))
                .filter(|o| o.enabled)
            {
                c = c.push(
                    btn(
                        prev.name(),
                        Message::RestSource(prev.id.clone()),
                        get(p, "rest_source") == prev.id,
                    )
                    .width(Fill),
                );
            }
        }
    }
    c.into()
}
/// The setup editor's steps, one tab each, in order.
pub(crate) const SETUP_STEPS: [&str; 5] = ["Machine", "Stock", "Position", "Holding", "Work zero"];
/// Whether the active setup's `step` needs nothing more: its tab's ✓, else ○.
pub(crate) fn setup_step_done(app: &App, step: &str) -> bool {
    let s = app.doc.setup();
    match step {
        "Machine" => s.machine.issues().is_empty(),
        "Work zero" => {
            !get(&s.p, "offset").is_empty()
                && (crate::post::references(get(&s.p, "origin")).is_some()
                    || crate::indexed::ORIGINS.contains(&get(&s.p, "origin")))
                && crate::post::pins_issue(s).is_none()
                && !app
                    .current_ready()
                    .and_then(|r| r.zero.as_ref())
                    .is_some_and(|z| !z.refused.is_empty())
        }
        "Stock" => {
            app.doc.selected_material().is_some()
                && crate::stock::resolve(s, app.part_mesh())
                    .is_ok_and(|resolved| !outside_fixed_stock(s, &resolved))
        }
        "Position" => ["rx", "ry", "rz", "tx", "ty", "tz"]
            .iter()
            .all(|k| num(&s.p, k).is_some()),
        "Holding" => {
            crate::holding::library_kind(get(&s.p, "holding")).is_some()
                && holding_lines(app).is_empty()
        }
        _ => false,
    }
}
/// Whether the part reaches outside the setup's fixed stock box.
fn outside_fixed_stock(s: &Setup, resolved: &crate::stock::ResolvedStock) -> bool {
    !crate::stock::relative(s)
        && get(&s.p, "stock_type") != "Relative round bar"
        && (resolved.part.is_some_and(|part| {
            (0..3).any(|i| {
                part.min[i] < resolved.stock.min[i] - 0.001
                    || part.max[i] > resolved.stock.max[i] + 0.001
            })
        }) || resolved
            .part_radius
            .is_some_and(|r| r > crate::stock::bar(resolved.stock).1 + 0.001))
}
fn setup_editor(app: &App) -> Panel<'_> {
    let s = app.doc.setup();
    let p = &s.p;
    let t = Target::Setup;
    let mut steps = Column::new().spacing(2);
    for pair in SETUP_STEPS.chunks(2) {
        let mut r = Row::new().spacing(1);
        for &step in pair {
            let on = app.setup_step == step;
            let mark = if setup_step_done(app, step) {
                "✓"
            } else {
                "○"
            };
            let b = button(text(format!("{mark}  {step}")).size(12))
                .on_press(Message::SetupStep(step))
                .padding([7, 10])
                .style(move |_, status| flat(on, status));
            r = r.push(
                HelpButton::new(b, &Message::SetupStep(step), step).width(Length::FillPortion(1)),
            );
        }
        steps = steps.push(r);
    }
    // Pinned: the title and the steps; the step's fields scroll under them.
    let head = column![
        row![bold("Setup", 20).width(Fill), fold(app, Side::Inspector)].align_y(Alignment::Center),
        muted(
            "One position of the part on the machine: its machine, stock, placement and holding."
        ),
        steps,
    ]
    .spacing(12);
    let c = column![field(p, "name", "Setup name", t)].spacing(12);
    let c = match app.setup_step {
        "Stock" => setup_stock(app, c),
        "Position" => setup_position(app, c),
        "Holding" => setup_holding(app, c),
        "Work zero" => setup_zero(app, c),
        _ => setup_machine(app, c),
    };
    // Under the form, scrolling with it: the checks (`operation_editor`).
    let mut issues = s.issues();
    issues.extend(app.doc.skew_issue(app.doc.index()));
    issues.extend(app.doc.index_issues(app.doc.index()));
    let c = c.push(warnings(issues));
    // Pinned under the body: the setup's actions.
    let mut foot = Column::new().spacing(12);
    if app.confirm_delete_setup {
        foot=foot.push(note("Delete this setup and its operations? Later stock will be rebuilt from the revised setup order.")).push(row![secondary("Cancel",Message::ConfirmDeleteSetup(false)),danger("Delete setup",Message::ConfirmDeleteSetup(true))].spacing(5));
    } else {
        // The operation editor's row of icons (`operation_editor`).
        foot = foot.push(
            row![
                icon_btn(
                    IC_DUPLICATE,
                    "Duplicate setup",
                    Message::DuplicateSetup,
                    false
                ),
                icon_btn(IC_UP, "Move up", Message::MoveSetup(-1), false),
                icon_btn(IC_DOWN, "Move down", Message::MoveSetup(1), false),
                space().width(Fill),
                icon_btn(IC_DELETE, "Delete setup…", Message::RemoveSetup, true)
            ]
            .spacing(6),
        );
    }
    Panel {
        head: head.into(),
        body: c.into(),
        foot: Some(foot.into()),
    }
}
fn setup_machine<'a>(app: &'a App, c: Column<'a, Message>) -> Column<'a, Message> {
    let s = app.doc.setup();
    c.push(section("Machine"))
        .push(
            btn(
                format!(
                    "{} · {}-axis · v{}",
                    s.machine.name(),
                    s.machine.axes(),
                    s.machine.version
                ),
                Message::Page(Page::Machines),
                true,
            )
            .width(Fill),
        )
        .push(muted(
            "The work zero and its offset (G54–G59) are in the Work zero step.",
        ))
}
/// The setup editor's Work zero step: where the program's X0 Y0 Z0 is, on
/// the stock's top or bottom at a corner, an edge's middle or the centre
/// (picked on a grid of the stock seen from above), or Model origin; and the
/// work offset it is kept in.
fn setup_zero<'a>(app: &'a App, c: Column<'a, Message>) -> Column<'a, Message> {
    use crate::post::At::{Max, Mid, Min};
    let s = app.doc.setup();
    let p = &s.p;
    let t = Target::Setup;
    let origin = get(p, "origin");
    if let Some(base) = app.doc.index_base(app.doc.index()) {
        return c.push(section("Work zero")).push(note(format!(
            "{origin} in {}, as setup {} ({}) has it: A turns the stock about the A centreline and leaves the work zero where it is, so nothing is probed again.",
            get(p, "offset"),
            base + 1,
            app.doc.setups[base].name()
        )));
    }
    let zero = crate::post::stock_zero(origin);
    let model = origin == "Model origin";
    // A spot keeps the face picked, the top for Model origin.
    let top = zero.is_none_or(|(_, top)| top);
    let set = |v: String| Some(Message::Edit(t, "origin", v));
    let spot_name = |refs: [crate::post::At; 2]| {
        crate::post::SPOTS
            .iter()
            .find(|(_, r)| *r == refs)
            .map_or("center", |(name, _)| name)
    };
    let at = |refs, top: bool| {
        format!(
            "Stock {} {}",
            if top { "top" } else { "bottom" },
            spot_name(refs)
        )
    };
    let xy = zero.map_or([Mid, Mid], |(xy, _)| xy);
    let faces = row![
        act_sized(
            "Stock top",
            set(at(xy, true)),
            zero.is_some() && top,
            Some(Length::FillPortion(1))
        ),
        act_sized(
            "Stock bottom",
            set(at(xy, false)),
            zero.is_some() && !top,
            Some(Length::FillPortion(1))
        ),
        act_sized(
            "Model origin",
            set("Model origin".into()),
            model,
            Some(Length::FillPortion(1))
        ),
    ]
    .spacing(4);
    let label = |refs: [crate::post::At; 2]| match refs {
        [Mid, Mid] => "Centre".to_string(),
        _ => {
            let s = spot_name(refs);
            s[..1].to_uppercase() + &s[1..]
        }
    };
    let mut grid = column![].spacing(4);
    for y in [Max, Mid, Min] {
        let mut line = row![].spacing(4);
        for x in [Min, Mid, Max] {
            let refs = [x, y];
            line = line.push(act_sized(
                label(refs),
                set(at(refs, top)),
                zero.is_some_and(|(xy, _)| xy == refs),
                Some(Length::FillPortion(1)),
            ));
        }
        grid = grid.push(line);
    }
    c.push(section("Work zero"))
        .push(muted(
            "Where the program's X0 Y0 Z0 is: the point you touch off or probe on the machine. The 3D view marks it with a cross.",
        ))
        .push(faces)
        .push(muted(if model {
            "Model origin is the part file's own 0,0,0. Pick a spot below to put the zero on the stock instead."
        } else {
            "On the stock, seen from above (back at the top, front toward you):"
        }))
        .push(grid)
        .push(rotary_zeros(s, origin))
        .push(bar_zeros(s, origin))
        .push(choice(
            p,
            "offset",
            "Work offset",
            &["G54", "G55", "G56", "G57", "G58", "G59"],
            t,
        ))
        .push(pins_block(app, s))
        .push(zero_note(app))
}
/// The Work zero step's zeros on the A axis (`indexed::ORIGINS`), on a
/// machine with one, and how the chosen one is found.
fn rotary_zeros<'a>(s: &Setup, origin: &str) -> Element<'a, Message> {
    if !s.machine.has_a() {
        return column![].into();
    }
    let set = |v: &str| Some(Message::Edit(Target::Setup, "origin", v.into()));
    let mut c = column![
        muted("On the A axis: Y0 Z0 on the A centreline, X0 at"),
        row![
            act_sized(
                "Stock end",
                set(crate::indexed::ORIGINS[0]),
                origin == crate::indexed::ORIGINS[0],
                Some(Length::FillPortion(1))
            ),
            act_sized(
                "Chuck face",
                set(crate::indexed::ORIGINS[1]),
                origin == crate::indexed::ORIGINS[1],
                Some(Length::FillPortion(1))
            ),
        ]
        .spacing(4)
    ]
    .spacing(4);
    if let Some(words) = crate::indexed::zero_words(origin) {
        c = c.push(muted(words));
    }
    c.into()
}
/// The Work zero step's zeros on a lying round bar's axis
/// (`round_hold::ORIGINS`), on a machine without a rotary A axis whose
/// setup 1 stock is such a bar, and how the chosen one is found.
fn bar_zeros<'a>(s: &Setup, origin: &str) -> Element<'a, Message> {
    if s.machine.has_a() || !crate::stock::round_bar(s) {
        return column![].into();
    }
    let set = |v: &str| Some(Message::Edit(Target::Setup, "origin", v.into()));
    let zeros = crate::round_hold::ORIGINS;
    let mut c = column![
        muted("On the bar's axis: Y0 Z0 on it, X0 at"),
        row![
            act_sized(
                "Left end",
                set(zeros[0]),
                origin == zeros[0],
                Some(Length::FillPortion(1))
            ),
            act_sized(
                "Middle",
                set(zeros[1]),
                origin == zeros[1],
                Some(Length::FillPortion(1))
            ),
        ]
        .spacing(4)
    ]
    .spacing(4);
    if let Some(words) = crate::round_hold::zero_words(origin) {
        c = c.push(muted(words));
    }
    c.into()
}
/// The Work zero step's fixture pins: whether the setup sits on its
/// machine's pins, the part's holes chosen to drop over them (one button
/// per round hole it opens on its bottom), and where that puts every pin.
/// Shown when the machine has pins or the setup sits on them.
fn pins_block<'a>(app: &'a App, s: &'a Setup) -> Element<'a, Message> {
    let (p, t) = (&s.p, Target::Setup);
    let pins = crate::control::pins_settings(&s.machine.p);
    let on = crate::post::sits_on_pins(s);
    if !on && !matches!(pins, Ok(Some(_))) {
        return column![].into();
    }
    let mut c = column![choice_value(
        p,
        if on { "Yes" } else { "No" },
        "on_pins",
        "Sits on the fixture pins",
        &["No", "Yes"],
        t,
    )]
    .spacing(8);
    if !on {
        return c
            .push(muted(
                "Yes when the part drops over the machine's fixture pins by holes in it: the pins then give this setup's X and Y work offset, with no probing.",
            ))
            .into();
    }
    // Without pins that read there is nothing to choose holes for.
    if !matches!(pins, Ok(Some(_)))
        && let Some(issue) = crate::post::pins_issue(s)
    {
        return c.push(caution("Not on the pins yet", vec![issue])).into();
    }
    let leftover = crate::post::pins_in_leftover(s);
    c = c.push(choice_value(
        p,
        crate::post::PIN_SOURCES[usize::from(leftover)],
        "pin_source",
        "Pin holes in",
        &crate::post::PIN_SOURCES,
        t,
    ));
    let Some(mesh) = app.part_mesh() else {
        return c
            .push(muted(if leftover {
                "Load the part to place the pins on the stock's leftover."
            } else {
                "Load the part to choose the holes it drops over the pins by."
            }))
            .into();
    };
    let stock = app.known_stock(app.doc.index()).0;
    let datum = stock.and_then(|stock| crate::post::datum(s, stock, mesh).ok());
    // In work coordinates; in the setup's own frame, said so, until the
    // stock it starts from (and so its work zero) is known.
    let work = |c: [f32; 2]| match datum {
        Some(d) => format!(
            "X{} Y{}",
            crate::post::exact_mm(c[0] - d[0]),
            crate::post::exact_mm(c[1] - d[1])
        ),
        None => format!(
            "X{} Y{} in the part's place, work zero not known yet",
            crate::post::exact_mm(c[0]),
            crate::post::exact_mm(c[1])
        ),
    };
    let placed = stock
        .zip(datum)
        .map(|(stock, d)| crate::post::placed_on_pins(s, mesh, stock, d));
    if leftover {
        return leftover_pins(app, c, s, mesh, placed, &work).into();
    }
    let chosen = crate::post::chosen_pin_holes(s);
    c = c.push(section("Pin holes")).push(muted(
        "Choose the part's holes that drop over the pins, as drawn in the part: the round holes it opens on its bottom in this setup, ringed and numbered in the 3D view. Click them here, or Pick holes on the part and click inside each hole in the 3D view (turn the view to see under the part). Which hole takes which pin follows from the plate.",
    ));
    c = c.push(
        btn(
            if app.picking_pin_holes() {
                "Finish picking"
            } else {
                "Pick holes on the part"
            },
            Message::PickPinHoles,
            app.picking_pin_holes(),
        )
        .width(Fill),
    );
    match &*app.bottom_holes(mesh) {
        Ok(holes) if holes.is_empty() => {
            c = c.push(note(
                "The part opens no round holes on its bottom in this setup: it needs holes in its design that drop over the pins, opening on the face this setup sits on.",
            ));
        }
        Ok(holes) => {
            for (n, h) in holes.iter().enumerate().take(48) {
                let mine = crate::post::chosen_index(s, mesh, h).is_some();
                let pin = match &placed {
                    Some(Ok(on)) if mine => (on.places.iter())
                        .position(|c| (c[0] - h.center[0]).hypot(c[1] - h.center[1]) <= 0.05)
                        .map(|i| format!(" · pin {}", i + 1)),
                    _ => None,
                }
                .unwrap_or_default();
                let deep = if h.through {
                    "through".to_string()
                } else {
                    format!("{} mm deep", crate::post::exact_mm(h.depth))
                };
                let label = format!(
                    "{}Hole {} · Ø{} · {} · {deep}{pin} · {}",
                    if mine { "✓ " } else { "" },
                    n + 1,
                    crate::post::exact_mm(h.diameter.max(h.opening)),
                    work(h.center),
                    if mine { "remove" } else { "choose" }
                );
                c = c.push(
                    btn(
                        label,
                        Message::Edit(t, "pin_holes", crate::post::pin_holes_toggled(s, mesh, h)),
                        mine,
                    )
                    .width(Fill),
                );
            }
        }
        Err(e) => c = c.push(note(e.clone())),
    }
    if !chosen.is_empty() {
        c = c.push(secondary(
            "Clear the pin holes",
            Message::Edit(t, "pin_holes", String::new()),
        ));
    }
    match (placed, &pins) {
        (Some(Err(issue)), _) => c = c.push(caution("Not on the pins yet", vec![issue])),
        (Some(Ok(on)), Ok(Some(pins))) => {
            let at: Vec<String> = (on.places.iter().enumerate())
                .map(|(i, c)| format!("pin {} {}", i + 1, work(*c)))
                .collect();
            c = c.push(muted(format!(
                "The plate, square to X and Y, puts the Ø{} mm pins, {} mm tall, in these holes: {} in work coordinates. The pins give X and Y: on the Control page, Set X Y from the pins sets them exactly; Z is still touched off on the stock.",
                crate::post::exact_mm(pins.diameter as f32),
                crate::post::exact_mm(pins.height as f32),
                at.join(", ")
            )));
        }
        _ => {}
    }
    c.into()
}
/// Whether the tool of `o` is a `kind` ("Drill", "Flat end mill" and the
/// like).
fn op_tool_is(app: &App, o: &Operation, kind: &str) -> bool {
    (app.doc.tools.iter()).any(|t| t.id == o.tool && get(&t.p, "type") == kind)
}
/// What a Drill of the pin holes in the stock's leftover makes: the holes
/// of the setup on the pins it names (`pin_setup`) as its Work zero step
/// placed them, how they are made with its tool; or the later setups on
/// pins in their leftover to choose from.
fn pin_hole_drilling<'a>(app: &'a App, o: &'a Operation) -> Element<'a, Message> {
    let mut c = column![].spacing(8);
    let named = get(&o.p, "pin_setup");
    let mine = (app.doc.setups.iter()).position(|s| s.id == named);
    let Some(si) = mine.filter(|&si| {
        let s = &app.doc.setups[si];
        crate::post::sits_on_pins(s) && crate::post::pins_in_leftover(s)
    }) else {
        c = c.push(note(
            "Choose the setup whose pins these holes are for: a later setup that sits on the pins, with its pin holes in the stock's leftover (its Work zero step).",
        ));
        for s in app.doc.setups.iter().skip(app.doc.index() + 1) {
            if crate::post::sits_on_pins(s) && crate::post::pins_in_leftover(s) {
                c = c.push(
                    btn(
                        format!("For {}", s.name()),
                        Message::Edit(Target::Operation, "pin_setup", s.id.clone()),
                        false,
                    )
                    .width(Fill),
                );
            }
        }
        return c.into();
    };
    let s = &app.doc.setups[si];
    let points = crate::post::drill_pin_points(o);
    let issue = get(&o.p, "pin_issue");
    c = c.push(note(if !issue.is_empty() {
        format!(
            "No holes for {}'s pins: they do not fit where they are placed. {issue} Move them in its Work zero step.",
            s.name()
        )
    } else if points.is_empty() {
        format!(
            "The holes for {}'s pins, which are not placed yet: place them in its Work zero step.",
            s.name()
        )
    } else {
        format!(
            "The holes for {}'s {} pins in the stock's leftover, where its Work zero step places them; they follow the pins when they move. Each goes {} mm deeper than the pins stand. A drill from the pins' diameter to {} mm over drills them all round; a flat end mill mills them {} mm over the pins: pin 1's hole round, a slot along the line from pin 1 for the others.",
            s.name(),
            points.len(),
            crate::post::exact_mm(crate::post::PIN_DEPTH_EXTRA),
            crate::post::exact_mm(crate::post::PIN_FIT),
            crate::post::exact_mm(crate::post::PIN_CLEARANCE)
        )
    }));
    c.into()
}
/// The Work zero step's pins in the stock's leftover, under `c`: placing
/// them by a click in the 3D view, where they stand and why they do not fit
/// (`placed`, `post::placed_on_pins`), and the Drill of their holes in an
/// earlier setup, or a button that adds one. `work` says a plan point in
/// work coordinates.
fn leftover_pins<'a>(
    app: &'a App,
    mut c: widget::Column<'a, Message>,
    s: &'a Setup,
    mesh: &'a crate::mesh::Mesh,
    placed: Option<Result<crate::post::OnPins, String>>,
    work: &dyn Fn([f32; 2]) -> String,
) -> widget::Column<'a, Message> {
    let t = Target::Setup;
    c = c.push(section("Pins in the leftover")).push(muted(
        "Holes for the pins in the stock beside the part, cut away with it later: an earlier setup drills them in the face this setup sits on. Place the pins on the leftover, then click in the 3D view: pin 1 goes where you click and the plate puts the others. They follow the pointer, red where they do not fit. Drag the placed pins to move them.",
    ));
    let placing = app.placing_leftover_pins();
    c = c.push(
        btn(
            if placing {
                "Finish placing"
            } else if crate::post::leftover_pin(s).is_some() {
                "Move the pins"
            } else {
                "Place the pins on the leftover"
            },
            Message::PlacePins,
            placing,
        )
        .width(Fill),
    );
    let si = app.doc.index();
    let drills = crate::post::pin_drillings(&app.doc, si);
    // What makes the holes: a drill all round, an end mill a slot for each
    // pin after pin 1.
    let tool = (drills.first())
        .map(|&(i, j)| &app.doc.setups[i].operations[j].tool)
        .and_then(|id| app.doc.tools.iter().find(|t| &t.id == id))
        .map(|t| get(&t.p, "type"));
    match &placed {
        Some(Err(issue)) => c = c.push(caution("Not on the pins yet", vec![issue.clone()])),
        Some(Ok(on)) => {
            let at: Vec<String> = (on.places.iter().enumerate())
                .map(|(i, c)| format!("pin {} {}", i + 1, work(*c)))
                .collect();
            let holes = match tool {
                Some("Drill") => "A drill makes every hole round, at most 0.2 mm over the pins.",
                Some("Flat end mill") => {
                    "Pin 1's hole is round; the others are slots along the line from pin 1, so the part is placed by pin 1 and turned by the others without binding."
                }
                _ => {
                    "With a flat end mill pin 1's hole is round and the others are slots along the line from pin 1, so the pins never bind; a drill makes them all round."
                }
            };
            c = c.push(muted(format!(
                "The Ø{} mm pins, {} mm tall, stand in the leftover at {} in work coordinates. {holes} The pins give X and Y: on the Control page, Set X Y from the pins sets them exactly; Z is still touched off on the stock.",
                crate::post::exact_mm(on.diameter),
                crate::post::exact_mm(on.height),
                at.join(", ")
            )));
        }
        None => {}
    }
    if crate::post::leftover_pin(s).is_some() {
        c = c.push(secondary(
            "Clear the pins' place",
            Message::Edit(t, "pin_place", String::new()),
        ));
    }
    if let Some(&(i, j)) = drills.first() {
        let (setup, op) = (&app.doc.setups[i], &app.doc.setups[i].operations[j]);
        c = c.push(muted(format!(
            "{}'s {} makes the holes, following the pins wherever they are placed.",
            setup.name(),
            op.name()
        )));
    } else if matches!(placed, Some(Ok(_))) {
        c = match crate::post::pin_drilling_setup(&app.doc, mesh, si) {
            Some(to) => c.push(
                btn(
                    format!("Add the pin holes to {}", app.doc.setups[to].name()),
                    Message::AddPinDrilling,
                    false,
                )
                .width(Fill),
            ),
            None => c.push(note(
                "No earlier setup has the face this setup sits on facing up: add one before it to drill the pin holes in.",
            )),
        };
    }
    c
}
/// Where the active setup's work zero is on the stock it starts from, how
/// it is touched off and probed, and what the checks before a run found of
/// it (`run::Ready::zero`): that it is not on the stock, or that no probe
/// finds it.
fn zero_note(app: &App) -> Element<'_, Message> {
    let si = app.doc.index();
    let setup = app.doc.setup();
    let origin = get(&setup.p, "origin");
    let (Some(stock), known) = app.known_stock(si) else {
        return column![].into();
    };
    let size = stock.dimensions();
    let from = format!(
        "{} × {} × {} mm{}",
        crate::post::exact_mm(size[0]),
        crate::post::exact_mm(size[1]),
        crate::post::exact_mm(size[2]),
        match (si, known) {
            (0, _) => "",
            (_, true) => ", as the earlier setups leave it",
            _ => ", the original block until the stock the earlier setups leave is worked out",
        }
    );
    let refs = crate::post::references(origin);
    let mut lines = vec![match (crate::post::touch_off_spots(origin, stock), refs) {
        (Some(spots), _) => format!(
            "On the stock this setup starts from ({from}): touch off {spots}.{}",
            crate::round_hold::side_note(setup, stock)
                .map(|note| format!(" {note}"))
                .unwrap_or_default()
        ),
        (None, Some(refs)) => format!(
            "Model origin is the part file's own 0,0,0. A probe finds it from the stock this setup starts from ({from}): {}. By hand, the program header locates it from the part.",
            crate::post::probe_spots(refs, stock)
        ),
        _ => match crate::indexed::zero_words(origin)
            .or_else(|| crate::round_hold::zero_words(origin))
        {
            Some(words) => format!("On the stock this setup starts from ({from}): {words}"),
            None => "Choose a work zero.".into(),
        },
    }];
    if let Some(refs) = refs {
        lines.push(format!(
            "On the Control page, {} sets exactly this work zero, probed from its own spot or, under Probe from, from another corner, edge middle or the centre a probe can reach.",
            crate::control::probe_name(refs)
        ));
    }
    if crate::post::sits_on_pins(setup) {
        lines.insert(0, "On the pins, the pins locate X and Y and the touch-off or probe sets Z: Probe corner, Probe edge and Probe centre stay available to check the part's place.".into());
    }
    let mut c = column![muted(lines.join(" "))].spacing(8);
    if let Some(zero) = app.current_ready().and_then(|r| r.zero.as_ref()) {
        if !zero.warnings.is_empty() {
            c = c.push(caution("Pin holes", zero.warnings.clone()));
        }
        if !zero.refused.is_empty() {
            c = c.push(caution(
                "The work zero is not on the stock",
                zero.refused.clone(),
            ));
        } else if let Err(why) = &zero.probe {
            c = c.push(caution("No probe finds this work zero", vec![why.clone()]));
        }
    }
    c.into()
}
fn setup_stock<'a>(app: &'a App, c: Column<'a, Message>) -> Column<'a, Message> {
    let s = app.doc.setup();
    let p = &s.p;
    let t = Target::Setup;
    let mut c = c.push(material_picker(app)).push(section("Stock"));
    // A DXF part is a flat profile, extruded to the thickness set here.
    if app.doc.index() == 0
        && let Some(file) = app.doc.file.as_ref().filter(|f| f.format == "dxf")
    {
        c = c
            .push(hinted_field(
                p,
                crate::sheet::DXF_THICKNESS_KEY,
                "DXF part thickness · mm",
                "Part thickness · mm",
                t,
                &[],
            ))
            .push(muted(format!(
                "The flat profile is extruded {} mm thick: its outer loop is the part, its inner loops holes through it.",
                file.thickness.unwrap_or(crate::mesh::DXF_THICKNESS)
            )));
    }
    if app.doc.index() == 0 || app.is_sample() {
        c = c.push(choice(
            p,
            "stock_type",
            "Initial stock mode",
            crate::stock::modes(s),
            t,
        ));
        let round = crate::stock::is_round(s);
        // A 3-axis "Round bar" stands along Z or lies along X (LIO-249).
        if get(p, "stock_type") == "Round bar" && !s.machine.has_a() {
            let axes: Vec<Named> = (crate::stock::AXES.iter())
                .map(|(id, name)| Named {
                    id: id.to_string(),
                    name: name.to_string(),
                })
                .collect();
            let chosen = axes[usize::from(crate::stock::lying(s))].clone();
            c = c.push(help::field(
                column![
                    muted("The bar"),
                    pick_list(axes, Some(chosen), |a| Message::Edit(
                        Target::Setup,
                        "stock_axis",
                        a.id
                    ))
                    .width(Fill)
                    .padding([7, 9])
                    .text_size(13)
                    .style(choice_style)
                ]
                .spacing(5),
                "stock_axis",
                "The bar",
                t,
            ));
        }
        if crate::stock::round_bar(s) {
            c = c.push(
                row![
                    container(field(p, "stock_diameter", "Bar diameter · mm", t))
                        .width(Length::FillPortion(1)),
                    container(field(p, "stock_length", "Bar length · mm", t))
                        .width(Length::FillPortion(1))
                ]
                .spacing(10),
            );
            c = c.push(muted(if s.machine.has_a() {
                "A bar along X, centred on the part's X and Y, standing on Z0. Hold it in the chuck (Workholding)."
            } else {
                "A bar lying along X, centred on the part's X and Y, standing on Z0. Hold it in a V-block or soft jaws bored along X (Workholding)."
            }));
        } else if get(p, "stock_type") == "Relative round bar" {
            c = c.push(muted("Extra material round the part · mm"));
            c = c.push(field(p, "stock_radial", "Radial · mm", t));
            c = c.push(
                row![
                    container(field(p, "stock_top", "Top (+Z) · mm", t))
                        .width(Length::FillPortion(1)),
                    container(field(p, "stock_bottom", "Bottom (−Z) · mm", t))
                        .width(Length::FillPortion(1))
                ]
                .spacing(10),
            );
            c = c.push(muted("Bar along setup Z, on X 0 Y 0."));
        } else if round {
            for (k, l) in [
                ("stock_diameter", "Bar Ø · mm"),
                ("stock_length", "Length · mm"),
            ] {
                c = c.push(field(p, k, l, t));
            }
        } else if crate::sheet::is_sheet(s) {
            c = c.push(
                row![
                    container(field(p, "stock_x", "Sheet X · mm", t)).width(Length::FillPortion(1)),
                    container(field(p, "stock_y", "Sheet Y · mm", t)).width(Length::FillPortion(1))
                ]
                .spacing(10),
            );
            c = c
                .push(hinted_field(
                    p,
                    "sheet_thickness",
                    "Sheet thickness · mm",
                    "The part's thickness",
                    t,
                    &[],
                ))
                .push(hinted_field(
                    p,
                    "spoilboard",
                    "Spoilboard allowance · mm",
                    "0.5",
                    t,
                    &[],
                ))
                .push(muted(
                    "The sheet lies on a spoilboard at the part's bottom, centred on X0 Y0. A cut through it may go on into the board by the allowance, never deeper. Hold it down under Workholding.",
                ));
        } else if crate::stock::relative(s) {
            c = c.push(muted("Extra material from each side of the part · mm"));
            for pair in [
                [
                    ("stock_top", "Top (+Z) · mm"),
                    ("stock_bottom", "Bottom (−Z) · mm"),
                ],
                [
                    ("stock_left", "Left (−X) · mm"),
                    ("stock_right", "Right (+X) · mm"),
                ],
                [
                    ("stock_front", "Front (−Y) · mm"),
                    ("stock_back", "Back (+Y) · mm"),
                ],
            ] {
                c = c.push(
                    row![
                        container(field(p, pair[0].0, pair[0].1, t)).width(Length::FillPortion(1)),
                        container(field(p, pair[1].0, pair[1].1, t)).width(Length::FillPortion(1))
                    ]
                    .spacing(10),
                );
            }
            c = c.push(muted(
                "Sides follow setup axes after part rotation. 0 = flush with the part.",
            ));
        } else {
            for (k, l) in [
                ("stock_x", "Stock X · mm"),
                ("stock_y", "Stock Y · mm"),
                ("stock_z", "Stock Z · mm"),
            ] {
                c = c.push(field(p, k, l, t));
            }
        }
        match crate::stock::resolve(s, app.part_mesh()) {
            Ok(resolved) => {
                let (title, size) = if crate::stock::round_bar(s) {
                    let d = resolved.stock.dimensions();
                    (
                        "Resulting stock · round bar",
                        format!("Ø{:.3} × {:.3} mm", d[1], d[0]),
                    )
                } else if round {
                    (
                        "Resulting stock · Ø × length",
                        crate::stock::bar_text(resolved.stock),
                    )
                } else {
                    (
                        "Resulting stock · X × Y × Z",
                        crate::stock::dimensions_text(resolved.stock.dimensions()),
                    )
                };
                let mut summary =
                    column![bold(title, 12), text(size).size(14).color(BLUE)].spacing(6);
                if let Some(part) = resolved.part {
                    summary = summary.push(muted(format!(
                        "Part: {}",
                        crate::stock::dimensions_text(part.dimensions())
                    )));
                }
                if let Some(r) = resolved.part_radius {
                    summary =
                        summary.push(muted(format!("Part Ø round the axis: {:.3} mm", 2. * r)));
                }
                c = c.push(
                    container(summary)
                        .padding(12)
                        .width(Fill)
                        .style(panel_style),
                );
                if outside_fixed_stock(s, &resolved) {
                    c = c.push(note(if crate::stock::round_bar(s) {
                        "The part extends outside the box round this bar. Increase its diameter or length, adjust placement, or choose a box."
                    } else if round {
                        "The part extends outside this bar. Increase its size, adjust placement, or choose Relative round bar."
                    } else {
                        "The part extends outside this fixed stock box. Increase its dimensions, adjust placement, or choose Relative size box."
                    }));
                }
            }
            Err(error) => c = c.push(note(error)),
        }
        if app.part_mesh().is_some_and(|m| m.units != "mm") {
            c = c.push(note("STL/OBJ source coordinates are treated as mm for stock sizing. Check the part dimensions."));
        }
    } else {
        c=c.push(note("Remaining stock from every preceding setup is inherited automatically. Rotation and translation reposition the same 3D material, including cavities and holes. Define the initial stock in the first setup."));
        if let (Some(stock), _) = app.known_stock(app.doc.index()) {
            c = c.push(
                container(
                    column![
                        bold("Stock before this setup · X × Y × Z", 12),
                        text(crate::stock::dimensions_text(stock.dimensions()))
                            .size(14)
                            .color(BLUE)
                    ]
                    .spacing(6),
                )
                .padding(12)
                .width(Fill)
                .style(panel_style),
            );
        }
    }
    c
}
fn setup_position<'a>(app: &'a App, c: Column<'a, Message>) -> Column<'a, Message> {
    let p = &app.doc.setup().p;
    let t = Target::Setup;
    if let Some(base) = app.doc.index_base(app.doc.index()) {
        return c.push(section("Part position")).push(note(format!(
            "Placed as setup {} ({}) places it, turned about the A centreline to {}: A turns the stock in that clamping, with no re-clamp. To place this setup yourself, choose Reclamp and probe origin under Holding.",
            base + 1,
            app.doc.setups[base].name(),
            crate::indexed::label(app.doc.setup()).unwrap_or_default()
        )));
    }
    let mut c = c.push(section("Part position"));
    let i = app.doc.index();
    c = if i == 0 {
        c.push(
            row![
                secondary("Flip X 180°", Message::Flip("rx")),
                secondary("Side Y 90°", Message::Flip("ry"))
            ]
            .spacing(4),
        )
    } else {
        stock_turn(app, i, c)
    };
    for (k, l) in [
        ("rx", "Rotation X · degrees"),
        ("ry", "Rotation Y · degrees"),
        ("rz", "Rotation Z · degrees"),
        ("tx", "Translation X · mm"),
        ("ty", "Translation Y · mm"),
        ("tz", "Translation Z · mm"),
    ] {
        c = c.push(field(p, k, l, t));
    }
    c
}
/// How a later setup has the stock that setup 1 defined: which of its faces
/// is up and how it is turned on the table, chosen from setup 1's frame so
/// the setup is always square to the stock; or, when its angles leave it a
/// few degrees off, the way to square it.
fn stock_turn<'a>(app: &'a App, i: usize, mut c: Column<'a, Message>) -> Column<'a, Message> {
    use crate::volume::{FACES, TABLE_TURNS, face_and_table, turn_between};
    let first = &app.doc.setups[0];
    let now = turn_between(first, &app.doc.setups[i]).map(face_and_table);
    let faces: Vec<String> = FACES.iter().map(|(name, _)| name.to_string()).collect();
    let tables: Vec<String> = TABLE_TURNS.iter().map(|t| t.to_string()).collect();
    let (face, table) = now.unwrap_or((0, 0));
    let face_list = pick_list(
        faces.clone(),
        now.map(|(f, _)| faces[f].clone()),
        move |v: String| {
            Message::TurnStock(FACES.iter().position(|(n, _)| *n == v).unwrap_or(0), table)
        },
    )
    .placeholder("Not square to the stock")
    .width(Fill)
    .padding([7, 9])
    .text_size(13)
    .style(choice_style);
    let table_list = pick_list(
        tables.clone(),
        now.map(|(_, q)| tables[q].clone()),
        move |v: String| {
            Message::TurnStock(face, TABLE_TURNS.iter().position(|t| *t == v).unwrap_or(0))
        },
    )
    .placeholder("—")
    .width(Fill)
    .padding([7, 9])
    .text_size(13)
    .style(choice_style);
    c = c.push(help::wrap(
        row![
            column![muted("Stock face up"), face_list].spacing(5).width(Length::FillPortion(3)),
            column![muted("Turned on the table"), table_list].spacing(5).width(Length::FillPortion(2)),
        ]
        .spacing(8),
        Help::text(
            "Stock face up",
            "Which face of the stock this setup has on top, and how far it is turned about Z on the table, as setup 1 has the stock. The rotations below are set from these and setup 1's rotation, so this setup stays square to the stock and its jaws, clamps and stock-based work zero can be used.",
        ),
        widget::tooltip::Position::Left,
    ));
    if now.is_none() {
        c = c.push(note(format!(
            "Not quarter turns from {}, where the stock was defined: the stock sits skewed.",
            first.name()
        )));
        c = c.push(row![secondary(
            "Square to the stock",
            Message::SquareToStock
        )]);
    }
    c
}
fn setup_holding<'a>(app: &'a App, c: Column<'a, Message>) -> Column<'a, Message> {
    let p = &app.doc.setup().p;
    let t = Target::Setup;
    let si = app.doc.index();
    let mut c = c.push(section("Workholding"));
    match app.doc.index_base(si) {
        Some(base) => {
            c = c.push(note(format!(
                "Held as setup {} ({}) holds it: {}. A turns the stock in that clamping, so it is not re-clamped or probed again.",
                base + 1,
                app.doc.setups[base].name(),
                app.doc.holding_label(&app.doc.setups[base])
            )))
        }
        None => {
            for control in holding_controls(app) {
                c = c.push(control);
            }
        }
    }
    if si == 0 && crate::sheet::is_sheet(app.doc.setup()) {
        c = holddown_controls(app, c);
    }
    // What follows is recorded for the operator and checked by nothing, so
    // it sits under its own heading, apart from the checked workholding.
    c = c
        .push(section(SETUP_NOTES_SECTION))
        .push(field(p, "clearance", FIXTURE_CLEARANCE_LABEL, t))
        .push(choice(
            p,
            "reposition",
            "Reposition action",
            &[
                "Initial placement",
                "Reclamp and probe origin",
                "Index rotary axis",
                "Change fixture",
            ],
            t,
        ));
    let s = app.doc.setup();
    if crate::indexed::is_indexed(s) {
        c = c
            .push(field(p, crate::indexed::ANGLES, "A angles · degrees", t))
            .push(muted(
                "One angle, or several for the same operations (0, 90, 180, -90). Positive A turns the part clockwise seen from the chuck along +X (the right-hand rule about +X): +Y up to +Z at A90.",
            ));
        let wrong = crate::indexed::issues(s);
        if !wrong.is_empty() {
            c = c.push(caution("A angles", wrong));
        }
        let warned = app.doc.index_warnings(si);
        if !warned.is_empty() {
            c = c.push(caution("Not ready to index", warned));
        }
        c = c.push(muted(
            "Its programs turn A to each angle, A alone with the tool above all that A swings: LinuxCNC exports them, and a run on the UC100 turns its A once the A centreline is measured.",
        ));
    } else if crate::indexed::is_label_only(s) {
        c = c.push(note(
            "Saved before indexing turned the stock: this setup keeps its own placement and work zero. Choose Index rotary axis again to turn the stock about A in the clamping before it.",
        ));
    }
    c.push(field(p, "notes", "Setup instructions", t))
}
/// An item of a pick list: what is sent when it is chosen, and what it says.
#[derive(Clone, PartialEq)]
struct Named {
    id: String,
    name: String,
}
impl std::fmt::Display for Named {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}
fn material_picker(app: &App) -> Element<'_, Message> {
    let choices: Vec<Named> = app
        .doc
        .materials
        .iter()
        .map(|m| Named {
            id: m.id.clone(),
            name: m.name().to_string(),
        })
        .collect();
    let selected = app.doc.selected_material();
    let chosen = selected.and_then(|m| choices.iter().find(|c| c.id == m.id).cloned());
    let mut c = column![
        muted("Project material"),
        help::wrap(
            pick_list(choices, chosen, |c| Message::PickMaterial(c.id))
                .placeholder("Choose a material")
                .width(Fill)
                .padding([7, 9])
                .text_size(13).style(choice_style),
            Help::text(
                "Project material",
                "Material from the library. Choosing it sets each operation's speed and feed (drilling aside) from its cutting data, for the operation's tool and machine. The data also check speeds and feeds; its name, group, hardness, notes and cutting data are sent to the AI planner."
            ),
            widget::tooltip::Position::Left
        ),
        secondary(
            "Open material library",
            Message::Page(Page::Materials),
        )
        .width(Fill)
    ]
    .spacing(5);
    if let Some(note) = material_note(&app.doc) {
        c = c.push(muted(note));
    }
    c.into()
}
/// Why the project's speeds and feeds are not checked, if they are not.
pub(crate) fn material_note(doc: &Document) -> Option<String> {
    match doc.selected_material() {
        Some(m) if m.cutting_data().is_none() => Some(format!(
            "{} has no cutting data: speeds and feeds are not checked.",
            m.name()
        )),
        None => Some("No material is chosen: speeds and feeds are not checked.".into()),
        _ => None,
    }
}
/// The NC page's list: one line per speed or feed problem of an enabled
/// operation (true), and one per operation that could not be checked (false).
pub(crate) fn speed_feed_lines(app: &App) -> Vec<(bool, String)> {
    let mut lines = vec![];
    for (i, setup) in app.doc.setups.iter().enumerate() {
        for op in setup.operations.iter().filter(|o| o.enabled) {
            let at = format!("Setup {} · {}", i + 1, op.name());
            match app.doc.speed_feed(setup, op) {
                Some(SpeedFeed::Out(problems)) => {
                    lines.extend(problems.into_iter().map(|p| (true, format!("{at}: {p}"))))
                }
                Some(SpeedFeed::Unchecked(why)) => {
                    lines.push((false, format!("{at}: Not checked. {why}")))
                }
                _ => {}
            }
        }
    }
    lines
}
/// The NC page's heading for `lines`: a problem anywhere makes it a problem list.
pub(crate) fn speed_feed_heading(lines: &[(bool, String)]) -> &'static str {
    if lines.iter().any(|(problem, _)| *problem) {
        "Speeds and feeds outside the material's cutting data (export still allowed)"
    } else {
        "Speeds and feeds not checked (export still allowed)"
    }
}
/// Which of the holes `picked` by their walls (`App::pocket_holes`) the
/// drill's hole `h` (`App::drill_holes`) is, for its remove button: the one
/// at its centre, and at several A angles at its angle too, as each angle
/// reads its holes in its own frame (`Hole::a`).
pub(crate) fn picked_row(picked: &[crate::pocket::Hole], h: &crate::cam::Hole) -> Option<usize> {
    picked.iter().position(|w| {
        let (x, y) = w.centre();
        w.a == h.a && (x as f32 - h.center[0]).hypot(y as f32 - h.center[1]) <= 0.1
    })
}
/// The workholding a setup can model: a lathe setup a chuck, a machine with
/// a rotary A axis a vise, clamps or the chuck on A, any other a vise,
/// clamps, or a round bar's V-block, soft jaws or collet block.
pub(crate) fn holding_choices(machine: &crate::model::Machine) -> &'static [&'static str] {
    if crate::lathe::is_lathe(machine) {
        &["None", "Chuck"]
    } else if machine.has_a() {
        // The chuck on the rotary A axis (`chuck`), which holds its round bar.
        &crate::holding::CHOICES[..4]
    } else {
        // A round part's V-block, soft jaws or collet block (`round_hold`).
        &[
            "None",
            "Vise",
            "Clamps",
            crate::round_hold::V_BLOCK,
            crate::round_hold::SOFT_JAWS,
            crate::round_hold::COLLET_BLOCK,
        ]
    }
}
/// A sheet's hold-down: screws through its waste into the spoilboard, whose
/// heads every move is kept clear of, or tape; and what is wrong with it.
fn holddown_controls<'a>(app: &'a App, c: Column<'a, Message>) -> Column<'a, Message> {
    let s = app.doc.setup();
    let (p, t) = (&s.p, Target::Setup);
    let mut c = c.push(section("Hold-down")).push(choice_value(
        p,
        crate::sheet::holddown(s),
        "holddown",
        "Sheet hold-down",
        &crate::sheet::HOLDDOWNS,
        t,
    ));
    if crate::sheet::screwed(s) {
        c = c
            .push(hinted_field(
                p,
                "holddown_screws",
                "Screws · X,Y from the sheet's front-left corner · mm",
                "Empty: one in from each corner",
                t,
                &[],
            ))
            .push(
                row![
                    container(hinted_field(p, "screw_head", "Head Ø · mm", "12", t, &[]))
                        .width(Length::FillPortion(1)),
                    container(hinted_field(p, "screw_height", "Head height · mm", "5", t, &[]))
                        .width(Length::FillPortion(1))
                ]
                .spacing(10),
            )
            .push(muted(
                "Put the screws in the waste outside the part's outline. Every move keeps clear of their heads by the gap; the screws hold only the waste, so an outside contour that cuts through leaves tabs.",
            ));
    } else if crate::sheet::holddown(s) == "Tape" {
        c = c.push(muted(
            "Double-sided tape under the sheet holds the part and the waste: nothing stands in the cutter's way and tabs are optional.",
        ));
    }
    let stock = app.known_stock(0).0;
    let mut lines = crate::sheet::holddown_issues(s, stock);
    if let (Some(stock), Some(mesh)) = (stock, app.part_mesh()) {
        lines.extend(crate::sheet::screws_on_part(s, stock, mesh));
    }
    if !lines.is_empty() {
        c = c.push(caution("Hold-down", lines));
    }
    c
}
/// The setup editor's controls for the modelled workholding, above the
/// fixture description.
fn holding_controls(app: &App) -> Vec<Element<'_, Message>> {
    let p = &app.doc.setup().p;
    let t = Target::Setup;
    let holding = match get(p, "holding") {
        "" => "None",
        value => value,
    };
    let lathe = crate::lathe::is_lathe(&app.doc.setup().machine);
    let mut c: Vec<Element<_>> = vec![
        muted(if lathe {
            "The chuck that holds the bar, from the workholding library."
        } else if app.doc.setup().machine.has_a() {
            "The vise or clamps that hold the stock, from the workholding library, or the chuck on the machine's rotary A axis."
        } else {
            "The vise or clamps that hold the stock, or the V-block, soft jaws or collet block that hold a round bar, from the workholding library."
        })
        .into(),
        choice_value(
            p,
            holding,
            "holding",
            "Modelled workholding",
            holding_choices(&app.doc.setup().machine),
            t,
        ),
    ];
    let rotary_chuck = crate::chuck::holds(app.doc.setup());
    match holding {
        "Chuck" if !rotary_chuck => {
            c.push(holder_picker(app, "Chuck"));
            // Changing the chuck is slow: say so wherever the 4-jaw is on.
            if app
                .doc
                .holder(get(p, "holder"))
                .is_some_and(|h| h.kind() == "Chuck" && get(&h.p, "jaws") == "4")
            {
                c.push(
                    text(crate::holding::CHANGE_CHUCK)
                        .size(12)
                        .color(WARN)
                        .into(),
                );
            }
            c.push(
                row![
                    container(field(p, "grip", "Grip · mm", t)).width(Length::FillPortion(1)),
                    container(field(p, "stickout", "Stickout · mm", t))
                        .width(Length::FillPortion(1))
                ]
                .spacing(10)
                .into(),
            );
        }
        "Chuck" => {
            c.push(choice(p, "chuck_jaws", "Chuck", &crate::chuck::JAWS, t));
            c.push(
                row![
                    container(field(
                        p,
                        "chuck_stickout",
                        "Stick-out from the jaws · mm",
                        t
                    ))
                    .width(Length::FillPortion(1)),
                    container(field(p, "chuck_jaw_angle", "Jaw 1 at A0 · ° from up", t))
                        .width(Length::FillPortion(1))
                ]
                .spacing(10)
                .into(),
            );
            c.push(choice(
                p,
                "chuck_tailstock",
                "Tailstock",
                &crate::chuck::TAILSTOCK,
                t,
            ));
            c.push(
                muted(
                    "The chuck, its jaws, the rotary head and the tailstock are sized from the machine's rotary unit, the jaws' width and height and the bodies' lengths assumed from it, and checked as the boxes round them with the gap, at A0 for today's operations. The stock in the jaws is kept clear too.",
                )
                .into(),
            );
        }
        "Vise" => {
            c.push(holder_picker(app, "Vise"));
            let axes: Vec<Named> = [("X", "X sides"), ("Y", "Y sides")]
                .iter()
                .map(|(id, name)| Named {
                    id: id.to_string(),
                    name: name.to_string(),
                })
                .collect();
            let chosen = axes[usize::from(get(p, "vise_axis") == "Y")].clone();
            c.push(help::field(
                column![
                    muted("Jaws face"),
                    pick_list(axes, Some(chosen), |a| Message::Edit(
                        Target::Setup,
                        "vise_axis",
                        a.id
                    ))
                    .width(Fill)
                    .padding([7, 9])
                    .text_size(13)
                    .style(choice_style)
                ]
                .spacing(5),
                "vise_axis",
                "Jaws face",
                t,
            ));
            c.push(
                row![
                    container(field(p, "vise_offset", "Jaw offset · mm", t))
                        .width(Length::FillPortion(1)),
                    container(field(p, "vise_grip", "Grip · mm", t)).width(Length::FillPortion(1))
                ]
                .spacing(10)
                .into(),
            );
        }
        holding if crate::round_hold::Kind::of(holding).is_some() => {
            round_controls(app, &mut c, holding)
        }
        "Clamps" => {
            c.push(holder_picker(app, "Clamp"));
            let (typed, error) = app.clamps_field();
            c.push(clamp_editor(&typed));
            if let Some(error) = error {
                c.push(text(error).size(12).color(WARN).into());
            }
        }
        _ => {
            // Only a description: a modelled vise or clamps replace it.
            let note = match get(p, "fixture") {
                "" => "None",
                value => value,
            };
            c.push(choice_value(
                p,
                note,
                "fixture",
                "Fixture note · not checked",
                &[
                    "None",
                    "Low profile clamps",
                    "Vise",
                    "Soft jaws",
                    "Chuck",
                    "Custom fixture",
                ],
                t,
            ));
        }
    }
    if holding != "None" {
        // A lathe's chuck has no clearance gap to set; Suggest picks its
        // library entry. The chuck on A has a gap, and its layout is the
        // rotary unit's, not suggested.
        let library_chuck = holding == "Chuck" && !rotary_chuck;
        if !library_chuck {
            c.push(field(p, "holding_gap", HOLDING_GAP_LABEL, t));
        }
        let round = crate::round_hold::Kind::of(holding).is_some();
        if (holding != "Chuck" || library_chuck) && !round {
            c.push(
                secondary(
                    if library_chuck {
                        "Suggest chuck"
                    } else {
                        "Suggest layout"
                    },
                    Message::SuggestLayout,
                )
                .width(Fill)
                .into(),
            );
        }
        let lines = editor_holding_lines(app);
        if !lines.is_empty() {
            let heading = format!(
                "{} workholding issue{}",
                lines.len(),
                if lines.len() == 1 { "" } else { "s" }
            );
            c.push(caution(heading, lines));
        }
        for note in holding_warnings(app) {
            c.push(text(note).size(12).color(WARN).into());
        }
        let margins = holding_margins(app);
        if !margins.is_empty() {
            let mut m = column![muted("Against the cutting forces").size(12)].spacing(3);
            for line in margins {
                m = m.push(muted(line).size(12));
            }
            c.push(m.into());
        }
    }
    if let Some(note) = stock_note(app) {
        c.push(muted(note).into());
    }
    c.push(
        secondary("Open workholding library", Message::Page(Page::Workholding))
            .width(Fill)
            .into(),
    );
    c
}
/// What the button that adds a clamp says.
pub(crate) const ADD_CLAMP: &str = "+ Add clamp";
/// The active setup's clamp positions, a row per clamp of `typed` (what
/// `App::clamps_field` shows): its side, its offset along that side and a
/// Remove button while there is another clamp, then + Add clamp. Every edit
/// sends the whole text (`Message::SetClamps`), which is saved once it reads.
fn clamp_editor<'a>(typed: &str) -> Element<'a, Message> {
    use crate::holding::{MAX_CLAMPS, Side, clamp_rows, rows_text};
    let rows = clamp_rows(typed);
    let sides: Vec<String> = Side::ALL.iter().map(|s| s.label().to_string()).collect();
    let mut c = column![muted("Clamp positions · side, offset along it in mm")].spacing(5);
    for (i, [side, offset]) in rows.iter().enumerate() {
        let edit = |cell: usize| {
            let rows = rows.clone();
            move |value: String| {
                let mut rows = rows.clone();
                rows[i][cell] = value;
                Message::SetClamps(rows_text(&rows))
            }
        };
        let clamp = row![
            pick_list(sides.clone(), Some(side.clone()), edit(0))
                .width(Length::FillPortion(1))
                .padding([7, 9])
                .text_size(13)
                .style(choice_style),
            text_input("0", offset)
                .on_input(edit(1))
                .width(Length::FillPortion(1))
                .padding([7, 9])
                .size(13)
        ]
        .spacing(5);
        let mut line = row![help::field(
            clamp,
            "clamps",
            "Clamp positions",
            Target::Setup
        )]
        .spacing(5)
        .align_y(Alignment::Center);
        if rows.len() > 1 {
            let mut rest = rows.clone();
            rest.remove(i);
            line = line.push(danger("Remove", Message::SetClamps(rows_text(&rest))));
        }
        c = c.push(line);
    }
    if rows.len() < MAX_CLAMPS {
        let mut more = rows.clone();
        more.push([Side::ALL[rows.len() % 4].label().into(), "0".into()]);
        c = c.push(secondary(ADD_CLAMP, Message::SetClamps(rows_text(&more))).width(Fill));
    }
    c.into()
}
/// A check box per library machine for the `machines` of a tool or holder
/// (`p`): none checked fits every CNC machine, or every lathe for a lathe
/// tool or a chuck (`lathe`).
fn machine_boxes<'a>(
    app: &'a App,
    p: &Fields,
    lathe: bool,
    toggle: fn(String, bool) -> Message,
) -> Element<'a, Message> {
    let listed: Vec<&str> = get(p, "machines").split(',').map(str::trim).collect();
    let mut c = column![muted("Machines")].spacing(5);
    for m in &app.doc.machines {
        let id = m.id.clone();
        c = c.push(help::wrap(
            checkbox(listed.contains(&m.id.as_str()))
                .label(m.name().to_string())
                .on_toggle(move |on| toggle(id.clone(), on))
                .text_size(12),
            help::command(&toggle(m.id.clone(), true), m.name().to_string()),
            widget::tooltip::Position::Right,
        ));
    }
    c.push(
        muted(if lathe {
            "None checked: every lathe."
        } else {
            "None checked: every CNC machine."
        })
        .size(12),
    )
    .into()
}
/// The library's vises or clamps (`kind`) that fit the active setup's machine.
pub(crate) fn fitting_holders<'a>(app: &'a App, kind: &str) -> Vec<&'a crate::holding::Holder> {
    let machine = &app.doc.setup().machine;
    app.doc
        .workholding
        .iter()
        .filter(|h| h.kind() == kind && h.fits(machine))
        .collect()
}
/// The picker of the library's vises or clamps (`kind`) for the active setup.
/// The setup editor's controls for a round bar's V-block, soft jaws or
/// collet block (`round_hold`): the library entry, where it sits along the
/// bar, how deep it grips, which way soft jaws are bored and what a collet
/// closes on.
fn round_controls<'a>(app: &'a App, c: &mut Vec<Element<'a, Message>>, holding: &str) {
    use crate::round_hold::{COLLET_BLOCK, Kind, SOFT_JAWS, V_BLOCK};
    let (p, t) = (&app.doc.setup().p, Target::Setup);
    let kind = Kind::of(holding).unwrap_or(Kind::VBlock);
    c.push(holder_picker(
        app,
        match kind {
            Kind::VBlock => V_BLOCK,
            Kind::SoftJaws => SOFT_JAWS,
            Kind::Collet => COLLET_BLOCK,
        },
    ));
    let mut fields = row![].spacing(10);
    if kind != Kind::Collet {
        fields = fields.push(
            container(field(p, "round_offset", "Offset along the bar · mm", t))
                .width(Length::FillPortion(1)),
        );
    }
    match kind {
        Kind::VBlock => {}
        Kind::SoftJaws => {
            fields = fields.push(
                container(field(p, "round_grip", "Jaw top above bar bottom · mm", t))
                    .width(Length::FillPortion(1)),
            )
        }
        Kind::Collet => {
            fields = fields
                .push(
                    container(field(p, "round_grip", "Depth in the collet · mm", t))
                        .width(Length::FillPortion(1)),
                )
                .push(
                    container(field(
                        p,
                        "collet_diameter",
                        "Grips Ø · mm (empty: the bar)",
                        t,
                    ))
                    .width(Length::FillPortion(1)),
                )
        }
    }
    c.push(fields.into());
    if kind == Kind::SoftJaws {
        c.push(choice(p, "jaw_bore", "Bored along", &["X", "Z"], t));
    }
    c.push(
        muted(match kind {
            Kind::VBlock => "The bar lies along X in the V, the clamp's strap over it at the V-block's middle. The block, its flanks and the clamp are kept clear with the gap; the V's two line contacts and the clamp hold it against the cut.",
            Kind::SoftJaws => "Jaws bored to the bar's diameter close round it: along X round a lying bar, up Z round a standing one. The jaws are kept clear with the gap; their arc contact holds it against the cut.",
            Kind::Collet => "The bar stands in an ER collet in a block stood in the vise. The nut and the block are kept clear with the gap; the collet's grip holds it against the cut.",
        })
        .into(),
    );
}
fn holder_picker<'a>(app: &'a App, kind: &'static str) -> Element<'a, Message> {
    let choices: Vec<Named> = fitting_holders(app, kind)
        .into_iter()
        .map(|h| Named {
            id: h.id.clone(),
            name: h.name().to_string(),
        })
        .collect();
    let current = get(&app.doc.setup().p, "holder");
    let chosen = choices.iter().find(|c| c.id == current).cloned();
    help::field(
        column![
            muted(kind),
            pick_list(choices, chosen, |c| Message::Edit(
                Target::Setup,
                "holder",
                c.id
            ))
            .placeholder(format!("Choose a {}", kind.to_lowercase()))
            .width(Fill)
            .padding([7, 9])
            .text_size(13)
            .style(choice_style)
        ]
        .spacing(5),
        "holder",
        kind,
        Target::Setup,
    )
}
/// The jaws or clamps the 3D view draws with the stock: the active setup's,
/// round the stock of the preview shown or else the box the setup starts from
/// as far as it is known (`App::known_stock`), a vise's on the band it closes
/// on (`held_of`), from the stock's bottom up (`holding::shown`). None when the
/// stock is hidden, the setup has no workholding or it cannot be modelled
/// (the setup editor says why).
pub(crate) fn holding_boxes(app: &App) -> Vec<crate::holding::HoldBox> {
    let boxes = || {
        let setup = app.doc.active_setup().filter(|_| app.stock)?;
        let stock = app
            .active_simulation()
            .map(|sim| sim.plan.stock)
            .or_else(|| app.known_stock(app.doc.index()).0)?;
        Some(holding_on(app, setup, stock, held_of(app, stock)))
    };
    boxes().unwrap_or_default()
}
/// The jaws or clamps the 3D view's camera fits: the active setup's round
/// its own stock (`Document::setup_stock`), gripping all of it, whether they
/// are drawn or not. So neither the Stock toggle, a simulation nor the box
/// the earlier setups leave, once worked out, moves the view.
pub(crate) fn framed_holding(app: &App) -> Vec<crate::holding::HoldBox> {
    let si = app.doc.index();
    match (
        app.doc.active_setup(),
        app.doc.setup_stock(si, app.part_mesh()),
    ) {
        (Some(setup), Some(stock)) => {
            holding_on(app, setup, stock, crate::holding::Gripped::whole(stock))
        }
        _ => vec![],
    }
}
/// `setup`'s jaws or clamps round `stock`, a vise's closed on `held`, from
/// the stock's bottom up (`holding::shown`), and the pins it sits on.
fn holding_on(
    app: &App,
    setup: &Setup,
    stock: crate::mesh::Bounds,
    held: crate::holding::Gripped,
) -> Vec<crate::holding::HoldBox> {
    // An indexed setup's chuck and jaws turn to the A shown.
    let mut posed = setup.clone();
    if let (Some(t), Some(a)) = (setup.turn, app.shown_a()) {
        let turn = crate::indexed::sign(&setup.machine) * a;
        posed.turn = Some(crate::indexed::Turn { a, turn, ..t });
    }
    // A lathe's chuck is drawn by the lathe view, not as boxes.
    let modelled = match get(&setup.p, "holding") {
        "Vise" | "Clamps" => true,
        "Chuck" => crate::chuck::holds(setup),
        // A round bar's holder, or a sheet's hold-down screw heads.
        holding => crate::round_hold::Kind::of(holding).is_some() || crate::sheet::screwed(setup),
    };
    let mut boxes = if modelled {
        let jaws = app.doc.holding_boxes(&posed, held);
        crate::holding::shown(&jaws.unwrap_or_default(), stock)
    } else {
        vec![]
    };
    boxes.extend(pin_boxes(app, setup, stock));
    boxes
}
/// The fixture pins `setup` sits on, standing under `stock`, the box it
/// starts from (`holding::pin_boxes`): drawn with its jaws or clamps,
/// whichever holds it down. None when it does not sit on them.
pub(crate) fn pin_boxes(
    app: &App,
    setup: &Setup,
    stock: crate::mesh::Bounds,
) -> Vec<crate::holding::HoldBox> {
    let Some(mesh) = app.part_mesh() else {
        return vec![];
    };
    let on = crate::post::datum(setup, stock, mesh)
        .ok()
        .and_then(|datum| crate::post::on_pins(setup, mesh, stock, datum));
    crate::holding::pin_boxes(on.as_ref(), stock)
}
/// What the active setup's workholding holds of `stock`, the box it starts
/// from: with the band its vise closes on when the editor knows it for that
/// box (`App::known_held`), else the whole box.
fn held_of(app: &App, stock: crate::mesh::Bounds) -> crate::holding::Gripped {
    app.known_held(app.doc.index())
        .filter(|held| held.stock == stock)
        .unwrap_or(crate::holding::Gripped::whole(stock))
}
/// The box the 3D view draws as the stock of the active setup when it is a
/// later setup whose box the earlier setups left has been worked out for the
/// job as it is (`App::known_stock`): in its frame, in mm. None for the first
/// setup, the example (which draws its own) and a box not worked out or out of
/// date, where the view draws what it always did.
pub(crate) fn leftover_view(app: &App) -> Option<crate::mesh::Bounds> {
    let si = app.doc.index();
    if si == 0 || app.is_sample() {
        return None;
    }
    match app.known_stock(si) {
        (Some(stock), true) => Some(stock),
        _ => None,
    }
}
/// The inspector's warning about the preview shown: a heading and a line for
/// each place where the generated operation's own toolpath comes within the
/// gap of the setup's jaws or clamps, in work coordinates (in the setup's
/// frame, said so, when the work zero cannot be told), or why that was not
/// checked. None when there is nothing to say. It never blocks: the export
/// refuses these hits too, and those of the links, approaches and retracts.
/// Worked out once for a preview, its plan and the workholding, and kept in
/// the app: the view asks on every frame.
pub(crate) fn holding_warning(app: &App) -> crate::HoldingNote {
    let (sim, op, setup) = (
        app.active_simulation()?,
        app.operation()?,
        app.doc.active_setup()?,
    );
    let key = format!(
        "{}|{}|{:?}|{:?}",
        sim.id,
        op.name(),
        setup.p,
        app.doc.holder(get(&setup.p, "holder")).map(|h| &h.p)
    );
    if let Some((cached, note)) = &*app.holding_note.borrow()
        && *cached == key
    {
        return note.clone();
    }
    let note = holding_note(app, &sim.plan, op.name(), setup);
    *app.holding_note.borrow_mut() = Some((key, note.clone()));
    note
}
/// What the browser shows of an operation of the active setup. Each state
/// has a glyph of its own as well as a colour (`OpMark::glyph`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum OpMark {
    /// —
    Suppressed,
    /// ○: not generated.
    Pending,
    /// Grey …: generated (its preview is on screen) and the checks before a
    /// run are still being worked out.
    Checking,
    /// Grey ✓·: generated, but the setup will not run for a reason the
    /// simulation panel lists, or (the example) cannot be run at all.
    Generated,
    /// Amber !: those checks refuse the setup's workholding for it, or for
    /// the whole setup (its clearance plane): a jaw or clamp in the way, or
    /// one that would not hold the stock against the operation's cut.
    Collides,
    /// Green ✓: those checks pass, so the setup will run.
    Ready,
}
impl OpMark {
    #[cfg(test)]
    pub(crate) const ALL: [OpMark; 6] = [
        OpMark::Suppressed,
        OpMark::Pending,
        OpMark::Checking,
        OpMark::Generated,
        OpMark::Collides,
        OpMark::Ready,
    ];
    pub(crate) fn glyph(self) -> &'static str {
        match self {
            OpMark::Suppressed => "—",
            OpMark::Pending => "○",
            OpMark::Checking => "…",
            OpMark::Generated => "✓·",
            OpMark::Collides => "!",
            OpMark::Ready => "✓",
        }
    }
    pub(crate) fn color(self) -> Color {
        match self {
            OpMark::Collides => Color::from_rgb8(196, 112, 22),
            OpMark::Ready => Color::from_rgb8(38, 146, 111),
            _ => MUTED,
        }
    }
    /// The state in words, as the hover help starts.
    pub(crate) fn state(self) -> &'static str {
        match self {
            OpMark::Suppressed => "Suppressed",
            OpMark::Pending => "Not generated",
            OpMark::Checking => "Generated, being checked",
            OpMark::Generated => "Generated, will not run",
            OpMark::Collides => "Workholding refused",
            OpMark::Ready => "Ready to run",
        }
    }
}
/// The mark of operation `i` of the active setup from `checks`, the checks
/// before a run of the setup when they are current (`App::current_ready`).
/// Until they are, generated means its preview is on screen (or, for the
/// example, its sample path), which is never Ready: nothing says it runs.
pub(crate) fn op_mark(app: &App, checks: Option<&crate::run::Ready>, i: usize) -> OpMark {
    let Some(o) = app.doc.active_setup().and_then(|s| s.operations.get(i)) else {
        return OpMark::Pending;
    };
    if !o.enabled {
        return OpMark::Suppressed;
    }
    let Some(r) = checks else {
        let shown = o.preview
            || app
                .active_simulation()
                .is_some_and(|sim| sim.plan.operation == o.id);
        return if !shown {
            OpMark::Pending
        } else if app.holding_key().is_some() {
            OpMark::Checking
        } else {
            // The example is never checked: its sample paths cannot be run.
            OpMark::Generated
        };
    };
    if r.held.iter().any(|h| h.operation.is_none_or(|k| k == i)) {
        OpMark::Collides
    } else if !r.generated.contains(&i) {
        OpMark::Pending
    } else if r.problems.is_empty() {
        OpMark::Ready
    } else {
        OpMark::Generated
    }
}
/// The hover help of operation `i`'s mark: its glyph and state in words, and
/// for a mark that refuses the setup, the Run panel's reason (the lines the
/// simulation panel lists, the first three of them).
pub(crate) fn mark_help(
    app: &App,
    checks: Option<&crate::run::Ready>,
    i: usize,
    mark: OpMark,
) -> String {
    let head = format!("Mark {}: {}.", mark.glyph(), mark.state());
    let Some(setup) = app.doc.active_setup() else {
        return head;
    };
    let uc100 = get(&setup.machine.p, "controller") == "UC100";
    let (by, verb) = if uc100 {
        ("Run", "run")
    } else {
        ("Export", "export")
    };
    let name = setup.name();
    match mark {
        OpMark::Suppressed => {
            format!("{head} The operation is switched off and left out of the job.")
        }
        OpMark::Pending => {
            format!("{head} Generate it to see its toolpath and have {name} checked.")
        }
        OpMark::Checking => {
            format!("{head} Checking {name} as {verb} will; the mark changes when that is done.")
        }
        OpMark::Ready => format!("{head} {by}'s checks of {name} pass."),
        OpMark::Collides | OpMark::Generated => {
            let Some(r) = checks else {
                return format!("{head} The example's sample paths cannot be run.");
            };
            let lines = refusal_lines(r, i);
            let mut help = format!("{head} {by} will refuse {name}");
            if lines.is_empty() {
                help.push('.');
            } else {
                help.push(':');
            }
            for line in lines.iter().take(3) {
                help.push_str("\n- ");
                help.push_str(line);
            }
            if lines.len() > 3 {
                help.push_str(&format!(
                    "\n- and {} more, listed above the playback controls.",
                    lines.len() - 3
                ));
            }
            help
        }
    }
}
/// What the checks before a run refuse of operation `i`'s setup, in the Run
/// panel's words: the operation's own workholding lines first, then the
/// setup's, then the other problems, none twice. Holding not checked for a
/// problem listed too names its operation, last, instead of repeating it.
fn refusal_lines(r: &crate::run::Ready, i: usize) -> Vec<String> {
    let own = r.held.iter().filter(|h| h.operation == Some(i));
    let setup_wide = r.held.iter().filter(|h| h.operation.is_none());
    let mut lines: Vec<String> = vec![];
    for problem in own.chain(setup_wide).map(|h| &h.message).chain(&r.problems) {
        if !lines.contains(problem) {
            lines.push(problem.clone());
        }
    }
    let listed = lines.clone();
    let repeated = |line: &String| {
        listed
            .iter()
            .find(|other| *line == crate::post::not_checked(std::slice::from_ref(*other)))
    };
    if let Some(at) = lines.iter().position(|line| repeated(line).is_some()) {
        let problem = repeated(&lines[at]).unwrap();
        let what = match problem.split_once(": ") {
            Some((op, _)) => format!("the {op} problem"),
            None => "that problem".into(),
        };
        lines.remove(at);
        lines.push(format!("Holding not checked until {what} is fixed."));
    }
    lines
}
#[cfg(test)]
mod refusal_tests {
    use super::*;
    #[test]
    fn holding_not_checked_does_not_repeat_the_problem_it_is_for() {
        let problem = "Facing: Enter a positive tool diameter.";
        let ready = |held: &str| crate::run::Ready {
            key: 0,
            setup: 0,
            generated: vec![],
            held: vec![crate::post::Held {
                operation: None,
                message: held.into(),
            }],
            problems: vec![problem.into()],
            zero: None,
        };
        let r = ready(&crate::post::not_checked(&[problem.into()]));
        assert_eq!(
            refusal_lines(&r, 0),
            [
                problem,
                "Holding not checked until the Facing problem is fixed."
            ]
        );
        // Holding not checked for another reason is kept as it is.
        let other = "Holding not checked: Drill: Select holes first.";
        assert_eq!(refusal_lines(&ready(other), 0), [other, problem]);
    }
}
/// What the simulation panel says of the selected operation's setup above
/// its controls: a heading, the lines of the checks before a run (the Run
/// panel's, export's for a machine that is not a UC100; the selected
/// operation's workholding problems first, then the setup's), whether the
/// setup will run, and whether it is being checked. While a changed job is
/// checked again the lines and the verdict are the last check's, under a
/// Checking heading, so the panel keeps its height and the view above it
/// does not move. None for the example and a suppressed operation.
pub(crate) fn run_check(app: &App) -> Option<(String, Vec<String>, bool, bool)> {
    let i = app.selected?;
    let setup = app.doc.active_setup()?;
    let uc100 = get(&setup.machine.p, "controller") == "UC100";
    let (by, verb) = if uc100 {
        ("Run", "run")
    } else {
        ("Export", "export")
    };
    let name = setup.name();
    let checking = format!("Checking {name} as {verb} will…");
    let (r, checked) = match (app.current_ready(), app.stale_ready()) {
        (Some(r), _) => (r, true),
        (None, Some(r)) => (r, false),
        (None, None) => {
            // Nothing is checked for the example.
            app.holding_key()?;
            return Some((checking, vec![], false, true));
        }
    };
    let mark = op_mark(app, Some(r), i);
    let lines = refusal_lines(r, i);
    let machine = "The machine's position and the work zero are checked on the Control page.";
    let (heading, lines, ok) = match mark {
        OpMark::Suppressed => return None,
        OpMark::Ready => (
            format!("Ready to {verb}: {by}'s checks of {name} pass."),
            if uc100 { vec![machine.into()] } else { vec![] },
            true,
        ),
        OpMark::Collides => (
            format!("Workholding problem: {by} will refuse {name}"),
            lines,
            false,
        ),
        _ => (format!("{by} will refuse {name}"), lines, false),
    };
    Some(if checked {
        (heading, lines, ok, false)
    } else {
        (checking, lines, ok, true)
    })
}
/// `holding_warning` of the plan of the operation `name`, worked out.
fn holding_note(
    app: &App,
    plan: &crate::simulation::Plan,
    name: &str,
    setup: &Setup,
) -> crate::HoldingNote {
    // A lathe setup has no export to refuse anything.
    let on_lathe = lathe::is_lathe(&setup.machine);
    let boxes = match app.doc.holding_boxes(setup, held_of(app, plan.stock)) {
        Ok(boxes) => boxes,
        Err(error) => {
            return Some((
                if on_lathe {
                    "Workholding cannot be checked until it is fixed"
                } else {
                    "Workholding cannot be checked; export will refuse this setup until it is fixed"
                }
                .into(),
                vec![error],
            ));
        }
    };
    let gap = crate::holding::gap(setup).unwrap_or_default();
    // As export checks: the machine's blend tolerance is added to the gap.
    let reach = gap + crate::post::blend(&setup.machine);
    let datum = app
        .part_mesh()
        .and_then(|mesh| crate::post::datum(setup, plan.stock, mesh).ok());
    // A chuck is told what this operation's moves do to it: a cut in the
    // grip, a move into the body, a stickout short of the cut.
    let chuck = crate::holding::layout(setup)
        .ok()
        .filter(|l| on_lathe && matches!(l, crate::holding::Layout::Chuck { .. }));
    let mut lines: Vec<String> = match chuck {
        Some(layout) => {
            crate::holding::chuck_problems(&layout, plan.stock, reach, &boxes, &[(0, name, plan)])
                .into_iter()
                .map(|(_, line)| line)
                .collect()
        }
        None => crate::holding::hits(plan, &boxes, reach)
            .iter()
            .map(|hit| crate::holding::message(name, hit, gap, datum.unwrap_or_default()))
            .collect(),
    };
    let hit = !lines.is_empty();
    lines.extend(bites_lost(app, plan, setup));
    (!lines.is_empty()).then(|| {
        let frame = if datum.is_none() {
            "; coordinates in the setup frame"
        } else {
            ""
        };
        let what = if on_lathe {
            "This cut conflicts with the chuck".to_string()
        } else if hit {
            format!("Too close to the workholding (export will refuse this{frame})")
        } else {
            format!("A clamp would lose the stock under it (export will refuse this{frame})")
        };
        (what, lines)
    })
}
/// Where the operation whose toolpath is `plan` cuts away the stock under a
/// clamp of `setup`, as the export says it (`holding::holdable` with the
/// setup's own cuts); the vise and a setup without clamps have none.
fn bites_lost(app: &App, plan: &crate::simulation::Plan, setup: &Setup) -> Vec<String> {
    let (Some(mesh), Ok(layout), Some(holder)) = (
        app.part_mesh(),
        crate::holding::layout(setup),
        app.doc.holder(get(&setup.p, "holder")),
    ) else {
        return vec![];
    };
    if !matches!(layout, crate::holding::Layout::Clamps { .. }) {
        return vec![];
    }
    let si = app.doc.index();
    let oi = app.selected.unwrap_or_default();
    let frame = crate::volume::Frame::new(setup, mesh);
    let cuts = crate::rest::Cuts::of_plans([(oi, plan)], si, frame);
    // One operation's own cuts; whether the setup's operations together cut
    // the part free is the export's and the preview's red to tell.
    let own = crate::holding::Own {
        cuts: &cuts,
        part: crate::simulation::part_box(setup, mesh),
        after: None,
    };
    let held = held_of(app, plan.stock);
    let height = crate::holding::height_issue(&layout, plan.stock);
    let names = |s, o| crate::holding::operation_name(&app.doc, s, o);
    crate::holding::holdable(si, &layout, holder, held, None, Some(&own), &names)
        .into_iter()
        .filter(|line| Some(line) != height.as_ref())
        .collect()
}
/// What is wrong with the active setup's workholding (`holding_lines_in`).
pub(crate) fn holding_lines(app: &App) -> Vec<String> {
    holding_lines_in(app, app.left_boxes(), app.current_checks(), app.doc.index())
}
/// What the setup editor lists under the workholding controls: first, while
/// the setup is as it was then, why Suggest layout placed no clamps on it
/// (`App::clamp_refusal_of`), then what is wrong with its workholding
/// (`holding_lines`). The reason is advice: clamps typed by hand are still
/// checked, so it does not count against them.
pub(crate) fn editor_holding_lines(app: &App) -> Vec<String> {
    let mut lines: Vec<String> = app
        .clamp_refusal_of(app.doc.index())
        .map(String::from)
        .into_iter()
        .collect();
    lines.extend(holding_lines(app));
    lines
}
/// What is wrong with setup `si`'s workholding as the editor knows it, in
/// the export's order: `Document::holding_issues_with` on the box the setup
/// starts from and the band its vise closes on (`App::known_held_in`, from
/// `left` and `checks`), and when that finds nothing, what the holding
/// checks found the earlier setups cut away, once they are worked out for
/// the job as it is (`checks`).
pub(crate) fn holding_lines_in(
    app: &App,
    left: Option<&[Option<crate::mesh::Bounds>]>,
    checks: Option<&[Option<crate::SetupHolding>]>,
    si: usize,
) -> Vec<String> {
    let held = app.known_held_in(left, checks, si);
    let issues = app.doc.holding_issues_with(si, held);
    if !issues.is_empty() {
        return issues;
    }
    checks
        .and_then(|c| c.get(si)?.as_ref())
        .filter(|c| Some(c.held) == held)
        .map(|c| c.problems.clone())
        .unwrap_or_default()
}
/// What the active setup's chuck warns of without refusing anything
/// (`Document::holding_warnings`), for the stock the editor knows.
pub(crate) fn holding_warnings(app: &App) -> Vec<String> {
    let si = app.doc.index();
    app.doc.holding_warnings(si, app.known_held(si))
}
/// Each operation's margin against the cutting forces (`forces::line`), as
/// the setup editor lists it under the workholding's issues: from the
/// holding checks (`holding_lines_in`) once they are worked out for the job
/// as it is and what the active setup's workholding holds passes.
pub(crate) fn holding_margins(app: &App) -> Vec<String> {
    let si = app.doc.index();
    let checks = app.current_checks();
    let held = app.known_held_in(app.left_boxes(), checks, si);
    checks
        .and_then(|c| c.get(si)?.as_ref())
        .filter(|c| Some(c.held) == held)
        .map(|c| c.margins.clone())
        .unwrap_or_default()
}
/// What the setup editor says of a stock box it shows that is the original
/// one.
pub(crate) const ORIGINAL_STOCK: &str =
    "Original stock size until the earlier setups are simulated.";
/// The note under the active setup's workholding controls: the stock they
/// are checked against is the original box, not yet the one the earlier
/// setups left. None for the first setup, when the box is the one left and
/// when there is no box.
pub(crate) fn stock_note(app: &App) -> Option<&'static str> {
    let (stock, current) = app.known_stock(app.doc.index());
    (stock.is_some() && !current).then_some(ORIGINAL_STOCK)
}
fn page_header<'a>(title: &str, subtitle: &str) -> Element<'a, Message> {
    row![
        column![bold(title, 26), muted(subtitle)].spacing(5),
        space().width(Fill),
        secondary("← Back to machining", Message::Page(Page::Work))
    ]
    .align_y(Alignment::Center)
    .into()
}
fn machines(app: &App) -> Element<'_, Message> {
    let mut c = column![
        page_header(
            "Machine library",
            "Saved locally. Each setup keeps a snapshot of its assigned profile."
        ),
        row![
            primary("+ New machine profile", Message::NewMachine),
            secondary("+ Jingyan CNC4030 + rotary", Message::AddRotaryMachine)
        ]
        .spacing(8),
        space().height(5)
    ]
    .spacing(18);
    for m in &app.doc.machines {
        let id = m.id.clone();
        let card = column![
            row![
                column![
                    bold(m.name(), 18),
                    muted({
                        // A 3-axis machine's mode is "3-axis" again.
                        let axes = format!("{}-axis", m.axes());
                        match get(&m.p, "mode") {
                            mode if mode == axes => format!("{axes}  ·  Profile v{}", m.version),
                            mode => format!("{axes}  ·  {mode}  ·  Profile v{}", m.version),
                        }
                    })
                ]
                .spacing(5),
                space().width(Fill),
                secondary("Edit profile", Message::EditMachine(id.clone())),
                // One per card: outlined, so the page's one filled
                // command stays New machine profile.
                secondary(
                    if app.doc.active_setup().is_some() {
                        "Assign to current setup"
                    } else {
                        "Create first setup"
                    },
                    Message::AssignMachine(id.clone())
                ),
                // Apart from the safe commands, at the end of the row.
                danger("Remove", Message::DeleteMachine(id))
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            rule::horizontal(1),
            row![
                muted(if lathe::is_lathe(m) {
                    format!(
                        "Swing  {} × {} {}",
                        get(&m.p, "swing"),
                        get(&m.p, "centres"),
                        get(&m.p, "units")
                    )
                } else {
                    format!(
                        "Travel  {} × {} × {} {}",
                        get(&m.p, "x"),
                        get(&m.p, "y"),
                        get(&m.p, "z"),
                        get(&m.p, "units")
                    )
                }),
                space().width(35),
                muted(format!("Spindle  {} rpm", get(&m.p, "rpm"))),
                space().width(35),
                muted(if lathe::is_lathe(m) {
                    "Manual lathe  ·  no G-code".to_string()
                } else if manual::is_manual(m) {
                    "Manual mill  ·  no G-code".to_string()
                } else {
                    format!("Controller  {}", get(&m.p, "controller"))
                }),
                space().width(35),
                muted(get(&m.p, "arrangement"))
            ]
            .spacing(8)
        ]
        .spacing(14);
        c = c.push(container(card).padding(20).width(Fill).style(panel_style));
    }
    c = c.push(note(if let Some(s) = app.doc.active_setup() {
        format!(
            "Current setup: {} · {} v{}. Assigning a profile affects this setup only.",
            s.name(),
            s.machine.name(),
            s.machine.version
        )
    } else {
        "No setup yet. Save a profile, then choose Create first setup to begin a machining job."
            .into()
    }));
    if app.doc.machines.is_empty() {
        c = c.push(note(
            "Your machine library is empty. Use New machine profile to add your own specification.",
        ));
    }
    // The cards and the header's Back clear of the scrollbar, which would
    // draw over their right edges.
    container(scrollable(
        c.padding(iced::Padding::ZERO.right(SCROLLBAR_GUTTER)),
    ))
    .padding(28)
    .width(Fill)
    .height(Fill)
    .into()
}
/// The wizard's steps for `m`: a manual mill has no kinematics or controller
/// to describe, and one step of its own in their place; a manual lathe has two
/// of its own.
pub(crate) fn wizard_steps(m: &Machine) -> Vec<&'static str> {
    if lathe::is_lathe(m) {
        vec!["Identity", "Spindle & travel", "Dials & feeds", "Review"]
    } else if manual::is_manual(m) {
        vec!["Identity", "Travel & spindle", "Manual mill", "Review"]
    } else {
        vec![
            "Identity",
            "Travel & spindle",
            "Kinematics",
            "Controller",
            "Review",
        ]
    }
}
fn wizard(app: &App) -> Element<'_, Message> {
    let m = &app.machine_draft;
    let p = &m.p;
    let t = Target::Machine;
    let steps = wizard_steps(m);
    let step = app.wizard_step.min(steps.len() - 1);
    let mut nav = Row::new().spacing(8);
    for (i, label) in steps.iter().enumerate() {
        nav = nav.push(btn(
            format!("{}  {label}", i + 1),
            Message::WizardStep(i),
            i == step,
        ));
    }
    let mut form = column![bold(steps[step], 22)].spacing(16);
    match steps[step] {
        "Identity" => {
            form = form
                .push(field(p, "name", "Machine name", t))
                .push(choice_value(
                    p,
                    if lathe::is_lathe(m) {
                        "Lathe"
                    } else if manual::is_manual(m) {
                        "Manual"
                    } else {
                        "CNC"
                    },
                    "kind",
                    "Machine kind",
                    &["CNC", "Manual", "Lathe"],
                    t,
                ));
            if !manual::is_hand(m) {
                form = form.push(choice(p, "axes", "Number of axes", &["3", "4", "5"], t));
            }
            form=form.push(choice(p,"units","Machine specification units",&["mm","inch"],t)).push(note("Machine profile values use these units. The CAM project and tool definitions use millimeters."));
        }
        "Travel & spindle" => {
            let spindle = if manual::is_manual(m) {
                vec![
                    ("power", "Spindle power · W"),
                    ("speeds", "Spindle speeds · rpm, or a range"),
                    ("hand_feed", "Hand feed · mm/min"),
                    ("hand_rapid", "Hand rapid · mm/min"),
                    ("turn_x", "X per handwheel turn · mm"),
                    ("turn_y", "Y per handwheel turn · mm"),
                    ("turn_z", "Z per handwheel turn · mm"),
                ]
            } else {
                vec![
                    ("rpm", "Maximum spindle speed · rpm"),
                    ("power", "Spindle power · W"),
                    ("feed", "Maximum feed · machine units/min"),
                ]
            };
            for (k, l) in [("x", "X travel"), ("y", "Y travel"), ("z", "Z travel")]
                .into_iter()
                .chain(spindle)
            {
                form = form.push(field(p, k, l, t));
            }
            if !manual::is_manual(m) {
                form = form
                    .push(bold("Clearances", 15))
                    .push(note("How tall and wide a part fits: the setup check refuses a part and tool that do not, naming these numbers. Measure them with the spindle jogged to Z top (nose to table: from the spindle nose's face to the table's top) and under the gantry beam (from the table's top to the beam's underside, and between its uprights). Empty: not checked."));
                for (k, l, _) in crate::envelope::CLEARANCES {
                    form = form.push(field(p, k, l, t));
                }
                form = form.push(bold("Measured or ASSUMED", 15));
                for (_, source, name) in crate::envelope::FIGURES {
                    let shown = match get(p, source) {
                        crate::envelope::MEASURED => crate::envelope::MEASURED,
                        _ => crate::envelope::ASSUMED,
                    };
                    form = form.push(choice_value(
                        p,
                        shown,
                        source,
                        name,
                        &[crate::envelope::ASSUMED, crate::envelope::MEASURED],
                        t,
                    ));
                }
                let assumed = crate::envelope::assumed(m);
                if !assumed.is_empty() {
                    form = form.push(caution(
                        "ASSUMED, not measured",
                        assumed.iter().map(|s| s.to_string()).collect(),
                    ));
                }
            }
        }
        "Spindle & travel" => {
            for (k, l) in [
                ("power", "Spindle power · W"),
                ("speeds", "Spindle speeds · rpm, or a range"),
                ("swing", "Swing over the bed · mm"),
                ("centres", "Distance between centres · mm"),
            ] {
                form = form.push(field(p, k, l, t));
            }
        }
        "Dials & feeds" => {
            form = form
                .push(choice_value(
                    p,
                    lathe::cross_reads(m).unwrap_or("Not known yet"),
                    "cross_reads",
                    "Cross-slide dial reads",
                    &["Not known yet", "Diameter", "Radius"],
                    t,
                ))
                .push(
                    row![
                        field(p, "cross_turn", "Cross slide · dial per turn", t),
                        field(p, "cross_div", "Cross slide · divisions", t),
                    ]
                    .spacing(8),
                )
                .push(
                    row![
                        field(p, "top_turn", "Top slide · dial per turn", t),
                        field(p, "top_div", "Top slide · divisions", t),
                    ]
                    .spacing(8),
                )
                .push(
                    row![
                        field(p, "carriage_turn", "Carriage · dial per turn", t),
                        field(p, "carriage_div", "Carriage · divisions", t),
                    ]
                    .spacing(8),
                )
                .push(
                    row![
                        field(p, "quill_turn", "Tailstock quill · dial per turn", t),
                        field(p, "quill_div", "Tailstock quill · divisions", t),
                    ]
                    .spacing(8),
                )
                .push(field(p, "feeds", "Power feeds · mm per revolution", t))
                .push(field(p, "pitches", "Threads the gearbox cuts · mm", t))
                .push(field(p, "pass_time", "Time per pass · s", t))
                .push(field(p, "tool_time", "Time per tool change · min", t))
                .push(field(
                    p,
                    "move_time",
                    "Time to move the stock to another machine · min",
                    t,
                ))
                .push(note("Manual lathe — no G-code; MACHINE shows the steps."));
        }
        "Manual mill" => {
            form = form
                .push(choice(
                    p,
                    "dro_comp",
                    "DRO tool radius compensation",
                    &["Yes", "No"],
                    t,
                ))
                .push(field(p, "edge_finder", "Edge finder diameter · mm", t))
                .push(field(p, "pass_time", "Time per pass · s", t))
                .push(field(p, "tool_time", "Time per tool change · min", t))
                .push(field(
                    p,
                    "move_time",
                    "Time to move the stock to another machine · min",
                    t,
                ))
                .push(note("Manual mill — no G-code; MACHINE shows the steps."));
        }
        "Kinematics" => {
            let modes = match m.axes() {
                4 => vec!["Indexed (3+1)", "Continuous 4-axis"],
                5 => vec!["Indexed (3+2)", "Simultaneous 5-axis"],
                _ => vec!["3-axis"],
            };
            form = form.push(choice(p, "mode", "Motion capability", &modes, t));
            if m.axes() > 3 {
                let arrangements = if m.axes() == 4 {
                    vec!["A table around X", "B table around Y", "C table around Z"]
                } else {
                    vec![
                        "A/C table-table",
                        "B/C table-table",
                        "A/B head-head",
                        "B/C head-table",
                    ]
                };
                form = form.push(choice(
                    p,
                    "arrangement",
                    "Axis arrangement",
                    &arrangements,
                    t,
                ));
                for (k, l) in [
                    ("rotary_min", "Primary rotary minimum · degrees"),
                    ("rotary_max", "Primary rotary maximum · degrees"),
                ] {
                    form = form.push(field(p, k, l, t));
                }
                form = form.push(choice(
                    p,
                    "direction",
                    "Primary axis direction",
                    &["Positive", "Negative"],
                    t,
                ));
                if m.axes() == 5 {
                    form = form
                        .push(field(
                            p,
                            "rotary2_min",
                            "Secondary rotary minimum · degrees",
                            t,
                        ))
                        .push(field(
                            p,
                            "rotary2_max",
                            "Secondary rotary maximum · degrees",
                            t,
                        ))
                        .push(choice(
                            p,
                            "direction2",
                            "Secondary axis direction",
                            &["Positive", "Negative"],
                            t,
                        ));
                }
                for (k, l) in [
                    ("pivot_x", "Primary pivot X · machine units"),
                    ("pivot_y", "Primary pivot Y · machine units"),
                    ("pivot_z", "Primary pivot Z · machine units"),
                ] {
                    form = form.push(field(p, k, l, t));
                }
                if m.axes() == 5 {
                    for (k, l) in [
                        ("pivot2_x", "Secondary pivot X · machine units"),
                        ("pivot2_y", "Secondary pivot Y · machine units"),
                        ("pivot2_z", "Secondary pivot Z · machine units"),
                    ] {
                        form = form.push(field(p, k, l, t));
                    }
                }
                if m.has_a() {
                    form = form
                        .push(bold("Rotary unit", 15))
                        .push(note("Where the A axis sits and how big its chuck and tailstock are, all in mm in machine coordinates (Z is zero at the top and negative down). New values are ASSUMED for a 100 mm 3-jaw chuck on a NEMA23 rotary unit and the 800 W spindle's Ø65 mm body, and still to be measured: calibration will replace them."));
                    for (k, l, _) in crate::model::ROTARY_GEOMETRY {
                        form = form.push(field(p, k, l, t));
                    }
                    form = form.push(warnings(m.rotary_issues()));
                }
            } else {
                form = form.push(note(
                    "Fixed spindle, linear X/Y/Z motion. No rotary kinematics are needed.",
                ));
            }
        }
        "Controller" => {
            form=form.push(choice(p,"controller","Controller / intended postprocessor",&["LinuxCNC","UC100","GRBL","Mach3 / Mach4","Fanuc","Haas","Siemens","Heidenhain","Custom"],t)).push(choice(p,"tool_change","Tool change",&["Manual","Automatic"],t)).push(choice(p,"tcp","Tool center point control",&["No","Yes"],t)).push(note("LinuxCNC has a 3-axis postprocessor. AXIS CAM drives a UC100 machine itself. Other controllers are saved for planning; exporting G-code for them is refused."));
            if get(p, "controller") == "LinuxCNC" {
                form = form
                    .push(bold("Postprocessor", 15))
                    .push(choice_value(p, post_setting(p, "post_tool_length"), "post_tool_length", "Tool length", &POST_TOOL_LENGTH, t))
                    .push(choice_value(p, post_setting(p, "post_retract"), "post_retract", "Safe retract", &POST_RETRACT, t))
                    .push(field(p, "post_blend", "Path blending tolerance · mm", t))
                    .push(field(p, "post_dwell", "Spindle start dwell · s", t))
                    .push(choice_value(p, post_setting(p, "post_air"), "post_air", "Air blast output", &POST_AIR, t))
                    .push(warnings(post_setting_issues(p)))
                    .push(note("Setups keep a snapshot of the machine. After saving, use Assign to current setup so existing setups use these settings."));
            }
            if get(p, "controller") == "UC100" {
                form = form.push(bold("UC100", 15));
                if app.control.as_ref().is_some_and(|c| c.machine_id == m.id) {
                    form = form
                        .push(muted(format!("Profile: {}", get(p, "uc100_profile"))))
                        .push(muted(format!("Connection: {}", crate::control::connection_label(m))))
                        .push(note("Connected on the Control page. Disconnect there to change the profile or the connection."));
                } else {
                    // ponytail: reads the profile on each redraw of this step; cache it if that ever shows.
                    let check: Element<_> = match crate::control::profile(m) {
                        Ok(_) => muted("UC100 profile checked.").into(),
                        Err(e) => warnings(vec![e]),
                    };
                    form = form
                        .push(
                            row![
                                field(p, "uc100_profile", "UC100 profile file", t),
                                secondary("Browse…", Message::BrowseUc100Profile)
                            ]
                            .spacing(8)
                            .align_y(Alignment::End),
                        )
                        .push(choice_value(
                            p,
                            crate::control::connection_label(m),
                            "uc100_connection",
                            "Connection",
                            &["USB", "Simulated"],
                            t,
                        ))
                        .push(check);
                }
                form = form
                    .push(field(p, "post_dwell", "Spindle start dwell · s", t))
                    .push(choice_value(p, post_setting(p, "post_air"), "post_air", "Air blast output", &POST_AIR, t))
                    .push(warnings(post_setting_issues(p)))
                    .push(bold("Probing", 15))
                    .push(row![
                        field(p, "probe_plate", "Touch plate thickness · mm", t),
                        field(p, "probe_tip", "Probe tip diameter · mm", t),
                        field(p, "probe_feed", "Probe feed · mm/min", t),
                    ].spacing(8))
                    .push(row![
                        field(p, "probe_distance", "Maximum probe distance · mm", t),
                        field(p, "probe_retract", "Retract after the probe · mm", t),
                        field(p, "probe_length", "Probe length, spindle nose to tip · mm", t),
                    ].spacing(8))
                    .push(warnings(crate::control::probe_settings(p).err().unwrap_or_default()))
                    .push(bold("Fixture pins", 15))
                    .push(field(p, "pins", "Pins on the plate · X Y; X Y; … mm, pin 1 first", t))
                    .push(row![
                        field(p, "pin_diameter", "Pin diameter · mm", t),
                        field(p, "pin_height", "Pin height above the plate · mm", t),
                    ].spacing(8))
                    .push(match crate::control::pins_settings(p) {
                        Ok(Some(crate::control::Pins { reference: Some(at), .. })) => muted(format!(
                            "Pin 1's machine place: X {:.3} Y {:.3}, recorded on the Control page.",
                            at[0], at[1]
                        )).into(),
                        Ok(Some(_)) => muted("Pin 1's machine place is not recorded yet: on the Control page, set machine zero, then Record the pins' place.").into(),
                        Ok(None) => muted("No fixture pins.").into(),
                        Err(e) => warnings(e),
                    })
                    .push(note("Dowel pins pressed into a plate on the table, square to X and Y, typed in the plate's own coordinates as drilled. A part drops over them by holes in it, so it sits in the same place and turn every time, also after a flip: a setup that sits on the pins (its Work zero step) gets its X and Y work offset from them without probing; Z is still touched off. Pin 1 is the reference pin: its machine place is recorded once on the Control page, and after every E-stop or power cycle Reference X Y from pin 1 makes machine X and Y, every saved work offset and the soft limits hold again."))
                    .push(note("AXIS CAM drives this machine itself from the Control page, one tool run at a time with a manual tool change in between. The profile is the UC100 driver's machine file: I/O pins, steps per mm, speeds, limits and homing. Set this machine's X, Y and Z travel to the profile's soft limits: Cycle Start checks that they agree."));
            }
        }
        _ if lathe::is_lathe(m) => {
            form = form
                .push(bold(m.name(), 18))
                .push(muted(format!("Manual lathe · speeds {} rpm", get(p, "speeds"))))
                .push(muted(format!(
                    "Swing {} · centres {} {}",
                    get(p, "swing"),
                    get(p, "centres"),
                    get(p, "units")
                )))
                .push(muted(match lathe::cross_reads(m) {
                    Some(reads) => format!("Cross-slide dial reads the {}", reads.to_lowercase()),
                    None => "Cross-slide dial: checked on the first lathe setup".to_string(),
                }))
                .push(warnings(m.issues()))
                .push(note("Save this profile to your local library. Assign it to a setup, or add a lathe setup, to turn on it."));
        }
        _ if manual::is_manual(m) => {
            form = form
                .push(bold(m.name(), 18))
                .push(muted(format!("Manual mill · speeds {} rpm", get(p, "speeds"))))
                .push(muted(format!("Travel: {} × {} × {} {}", get(p, "x"), get(p, "y"), get(p, "z"), get(p, "units"))))
                .push(muted(format!("Hand feed {} · hand rapid {} mm/min", get(p, "hand_feed"), get(p, "hand_rapid"))))
                .push(warnings(m.issues()))
                .push(note("Save this profile to your local library. Assign it to a setup, or add a manual setup, to cut on it."));
        }
        _ => {
            form=form.push(bold(m.name(),18)).push(muted(format!("{}-axis · {} · {}",m.axes(),get(p,"mode"),get(p,"controller")))).push(muted(format!("Travel: {} × {} × {} {}",get(p,"x"),get(p,"y"),get(p,"z"),get(p,"units")))).push(muted(format!("Spindle: {} rpm · Feed: {} {}/min",get(p,"rpm"),get(p,"feed"),get(p,"units")))).push(muted(format!("Post: {} · {} · G64 P{} · dwell {} s", post_setting(p, "post_tool_length"), post_setting(p, "post_retract"), post_setting(p, "post_blend"), post_setting(p, "post_dwell")))).push(warnings(m.issues())).push(note("Save this profile to your local library. Assign it to a setup to capture this version of its settings."));
        }
    }
    let mut actions = row![
        secondary("Cancel", Message::Page(Page::Machines)),
        space().width(Fill)
    ]
    .spacing(8);
    if step > 0 {
        actions = actions.push(secondary("Previous", Message::WizardStep(step - 1)));
    }
    actions = actions.push(if step + 1 < steps.len() {
        primary("Continue", Message::WizardStep(step + 1))
    } else {
        primary(
            app.resume_label().unwrap_or("Save machine profile".into()),
            Message::SaveMachine,
        )
    });
    let body = column![
        page_header(
            "Machine profile wizard",
            "Configure once. Reuse across future jobs."
        ),
        nav,
        scrollable(container(form).padding(24).width(Fill).style(panel_style)).height(Fill),
        actions
    ]
    .spacing(20)
    .max_width(940);
    container(body)
        .padding(28)
        .center_x(Fill)
        .height(Fill)
        .into()
}
/// The red E-stop: in the top bar on every page while connected, and on the Control page.
fn estop<'a>() -> Element<'a, Message> {
    button(bold("E-STOP", 14).color(Color::WHITE))
        .on_press(Message::MachineEStop)
        .padding([8, 16])
        .style(|_, status| button::Style {
            background: Some(
                if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                    Color::from_rgb8(160, 20, 20)
                } else {
                    Color::from_rgb8(200, 30, 30)
                }
                .into(),
            ),
            text_color: Color::WHITE,
            border: Border {
                radius: 4.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// A Control page button: greyed out without a message, when the machine
/// cannot do it now; `on` marks the selected one of a group.
pub(crate) fn act<'a>(
    label: impl Into<String>,
    msg: Option<Message>,
    on: bool,
) -> Element<'a, Message> {
    act_sized(label, msg, on, None)
}

/// `act`, `width` px wide with its label centred, for a grid of buttons
/// whose columns line up.
pub(crate) fn act_sized<'a>(
    label: impl Into<String>,
    msg: Option<Message>,
    on: bool,
    width: Option<Length>,
) -> Element<'a, Message> {
    let label = label.into();
    let face = text(label.clone()).size(12);
    let face = match width {
        Some(_) => face.width(Fill).align_x(Alignment::Center),
        None => face,
    };
    let b = button(face)
        .width(width.unwrap_or(Length::Shrink))
        .padding([7, 12])
        .on_press_maybe(msg.clone())
        .style(move |_, status| {
            let (background, text_color) = match status {
                _ if on => (ACTIVE_TILE, BLUE),
                button::Status::Disabled => (BG, MUTED),
                button::Status::Hovered | button::Status::Pressed => (HOVER_BUTTON, INK),
                _ => (Color::WHITE, INK),
            };
            button::Style {
                background: Some(background.into()),
                text_color,
                border: Border {
                    color: if on { BLUE } else { LINE },
                    width: 1.,
                    radius: 4.into(),
                },
                ..Default::default()
            }
        });
    match msg {
        Some(m) => HelpButton::new(b, &m, label).into(),
        None => b.into(),
    }
}

/// A titled box on the Control page.
pub(crate) fn machine_panel<'a>(title: &str, body: Column<'a, Message>) -> Element<'a, Message> {
    container(column![bold(title, 14), body.spacing(10)].spacing(10))
        .padding(14)
        .width(Fill)
        .style(panel_style)
        .into()
}

const AXES: [&str; 6] = crate::control::AXIS_NAMES;

/// A position on axis `a` as the Control page shows it: mm bare, as ever,
/// and a rotary axis's degrees with their sign.
fn reading(a: usize, v: f64) -> String {
    if crate::control::rotary(a) {
        format!("{v:.3}°")
    } else {
        format!("{v:.3}")
    }
}

/// "X", "X and Y", "X, Y and Z".
pub(crate) fn axis_names(axes: &[usize]) -> String {
    let names: Vec<&str> = axes.iter().map(|&a| AXES[a]).collect();
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => names.concat(),
    }
}

/// The amber warning above the Control page when nothing stops an axis at the
/// end of its travel: the profile sets no soft limits on X, Y and Z, or on
/// some of them (`unlimited`, #164), or an axis with them has no machine
/// position yet (`unset`). All are the same prominent box with their own
/// sentence on what to do (#73). None while the soft limits are on.
pub(crate) fn limits_off_warning(
    unlimited: &[usize],
    unset: &[usize],
    home_switches: bool,
) -> Option<(&'static str, Vec<String>)> {
    let mut lines = vec![];
    if unlimited.len() >= 3 {
        lines.push("This profile sets no soft limits, so nothing stops an axis at the end of its travel. Jog slowly and watch the machine.".into());
        lines.push(
            "Add soft_min and soft_max for X, Y and Z to the profile to turn them on.".into(),
        );
    } else if !unlimited.is_empty() {
        let one = unlimited.len() == 1;
        lines.push(format!(
            "{} {} no soft limits in this profile, so nothing stops {} at the end of {} travel. Jog slowly and watch the machine.",
            axis_names(unlimited),
            if one { "has" } else { "have" },
            if one { "it" } else { "them" },
            if one { "its" } else { "their" },
        ));
        lines.push(format!(
            "Add soft_min and soft_max for {} to the profile to turn them on.",
            axis_names(unlimited)
        ));
    }
    // An axis without limits is not waiting for its position.
    let unset: Vec<usize> = unset
        .iter()
        .copied()
        .filter(|a| !unlimited.contains(a))
        .collect();
    if !unset.is_empty() {
        let one = unset.len() == 1;
        lines.push(format!(
            "{} {} no machine position (every E-stop clears it), so nothing stops {} at the end of {} travel. Jog slowly and watch the machine.",
            axis_names(&unset),
            if one { "has" } else { "have" },
            if one { "it" } else { "them" },
            if one { "its" } else { "their" },
        ));
        lines.push(
            if home_switches {
                "Home below to turn them on."
            } else {
                "Set machine zero below to turn them on."
            }
            .into(),
        );
    }
    (!lines.is_empty()).then_some(("Soft limits are OFF", lines))
}

/// The tabs of the Control page's centre (LIO-274): one task at a time, the
/// jog, position and spindle staying on the left and the operations on the
/// right whichever is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlTab {
    /// The 3D view of the job or of the machine.
    Job,
    /// The manual tool change between tool runs, while one is due.
    ToolChange,
    /// The checks before a start at a step (LIO-275), while one is picked.
    Start,
    WorkZero,
    Probe,
    /// Rotary A's calibration, on a machine with the rotary unit.
    Rotary,
    Mdi,
    /// The connection, the machine zero, the soft limits and the log.
    Machine,
}
impl ControlTab {
    fn label(self) -> &'static str {
        match self {
            ControlTab::Job => "Job",
            ControlTab::ToolChange => "Tool change",
            ControlTab::Start => "Start at a step",
            ControlTab::WorkZero => "Work zero",
            ControlTab::Probe => "Probe",
            ControlTab::Rotary => "Rotary A",
            ControlTab::Mdi => "MDI",
            ControlTab::Machine => "Machine",
        }
    }
}
/// The machine's phase as the Control page's tabs follow it: a change of
/// phase, and only that, switches the tab (`App::phase_tab`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    /// A tool run is being cut.
    Cutting,
    /// The next tool run wants another tool (`run::Run::tool_change`).
    ToolChange,
    /// Cycle Start starts at a step (`run::Run::at_step`).
    Start,
}
/// The phase of the machine connected, if any.
pub(crate) fn phase(c: Option<&crate::control::Connection>) -> Phase {
    match c.and_then(|c| c.run.as_ref()) {
        Some(r) if r.running.is_some() => Phase::Cutting,
        Some(r) if r.tool_change().is_some() => Phase::ToolChange,
        Some(r) if r.at_step().is_some() => Phase::Start,
        _ => Phase::Idle,
    }
}
/// What the Job tab's view shows: the setup's stock in 3D, the machine's
/// travel with the tool in it, or the stock from above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobView {
    Stock,
    Machine,
    Top,
}
/// How the Control page fits the window (LIO-275), from the window's size,
/// not a setting. Short, under 860 px of height (a 13" laptop's 790, or
/// larger text): thinner bars, smaller DRO digits and jog keys, tighter
/// cards and no hint lines. Narrow, under 1360 px of width: narrower
/// columns, tighter tabs, and Hold and Resume one button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Fit {
    pub short: bool,
    pub narrow: bool,
}
impl Fit {
    pub(crate) fn window(width: f32, height: f32) -> Fit {
        Fit {
            short: height < 860.,
            narrow: width < 1360.,
        }
    }
    pub(crate) fn of(app: &App) -> Fit {
        Fit::window(app.window_width, app.window_height)
    }
    /// The bar of the MACHINE workspace and the run bar under it.
    pub(crate) fn machine_bar(self) -> f32 {
        if self.short { 38. } else { 44. }
    }
    pub(crate) fn run_bar(self) -> f32 {
        if self.short { 54. } else { 64. }
    }
    /// The widths of the left (the hands: position, jog, spindle) and right
    /// (the operations) columns; the centre takes the rest.
    pub(crate) fn hands(self) -> f32 {
        if self.narrow { 310. } else { 340. }
    }
    pub(crate) fn program(self) -> f32 {
        if self.narrow { 350. } else { 400. }
    }
    /// The gap between the cards and round them.
    fn gap(self) -> f32 {
        if self.short { 8. } else { 10. }
    }
    /// A card's padding, top and bottom.
    fn pad(self) -> f32 {
        if self.short { 8. } else { 10. }
    }
    /// A DRO line of the Position card and its digits' size. With A the
    /// four lines are a little lower, so the left column still fits.
    fn dro_row(self, axes: usize) -> (f32, u32) {
        match (self.short, axes > 3) {
            (false, false) => (46., 24),
            (false, true) => (42., 21),
            (true, false) => (38., 20),
            (true, true) => (34., 18),
        }
    }
    /// A hint line's characters, `n` in the full centre: narrow, the centre
    /// at the least window is about 3/5 as wide.
    fn chars(self, n: usize) -> usize {
        if self.narrow { n * 3 / 5 } else { n }
    }
    /// The work value's width in the Position card: a sign, four digits,
    /// three decimals and a degree sign in 24 px monospace; narrow, 16 px
    /// less, so that Move to's target field and Go fit beside it (the
    /// figures of A, 21 px or smaller, take less).
    fn dro_value(self) -> f32 {
        if self.narrow { 140. } else { 156. }
    }
    /// A jog button.
    fn jog(self) -> (f32, f32) {
        if self.short { (46., 30.) } else { (50., 36.) }
    }
}
const GREEN: Color = Color::from_rgb8(39, 130, 93);
const AMBER: Color = Color::from_rgb8(166, 101, 32);
const RUN_BLUE: Color = Color::from_rgb8(37, 99, 201);
/// The light text of the dark MACHINE bar.
const ON_DARK: Color = Color::from_rgb8(176, 190, 206);

/// One line of `size` px text in `color`, cut at `chars` characters with an
/// ellipsis and clipped, so that it never wraps and nothing under or beside
/// it moves as it changes; the whole text is on hover when it was cut.
fn one_line<'a>(
    title: &str,
    s: impl Into<String>,
    size: u32,
    color: Color,
    chars: usize,
) -> Element<'a, Message> {
    let s = s.into();
    let cut = ellipsize(&s, chars);
    let line = container(
        text(cut.clone())
            .size(size)
            .color(color)
            .wrapping(Wrapping::None),
    )
    .width(Fill)
    .clip(true);
    if cut == s {
        line.into()
    } else {
        help::wrap(line, Help::text(title, s), widget::tooltip::Position::Top)
    }
}
/// A short muted line whose whole text, often a paragraph, is on hover.
fn hint<'a>(title: &str, s: impl Into<String>, chars: usize) -> Element<'a, Message> {
    one_line(title, s, 12, MUTED, chars)
}

/// `act` with its padding: `pad` top and bottom, and at the sides.
fn act_with<'a>(
    label: impl Into<String>,
    msg: Option<Message>,
    on: bool,
    width: Option<Length>,
    pad: [u16; 2],
) -> Element<'a, Message> {
    let label = label.into();
    let face = text(label.clone()).size(12);
    let face = match width {
        Some(_) => face.width(Fill).align_x(Alignment::Center),
        None => face,
    };
    let b = button(face)
        .width(width.unwrap_or(Length::Shrink))
        .padding(pad)
        .on_press_maybe(msg.clone())
        .style(move |_, status| {
            let (background, text_color) = match status {
                _ if on => (ACTIVE_TILE, BLUE),
                button::Status::Disabled => (BG, MUTED),
                button::Status::Hovered | button::Status::Pressed => (HOVER_BUTTON, INK),
                _ => (Color::WHITE, INK),
            };
            button::Style {
                background: Some(background.into()),
                text_color,
                border: Border {
                    color: if on { BLUE } else { LINE },
                    width: 1.,
                    radius: 4.into(),
                },
                ..Default::default()
            }
        });
    match msg {
        Some(m) => HelpButton::new(b, &m, label).into(),
        None => b.into(),
    }
}
/// A small `act`, for the dense rows of the Control page.
fn small<'a>(label: impl Into<String>, msg: Option<Message>, on: bool) -> Element<'a, Message> {
    act_with(label, msg, on, None, [4, 8])
}
/// A card of the Control page: white, framed, `height` px tall when given,
/// so that what it holds never pushes the cards under it.
fn card<'a>(
    body: impl Into<Element<'a, Message>>,
    height: Option<f32>,
    fit: Fit,
) -> Element<'a, Message> {
    let c = container(body)
        .padding([fit.pad(), fit.pad() + 2.])
        .width(Fill)
        .style(panel_style)
        .clip(true);
    match height {
        Some(h) => c.height(h).into(),
        None => c.height(Fill).into(),
    }
}
/// A coloured tag: SIMULATED, TOOL CHANGE.
fn chip<'a>(label: &str, background: Color) -> Element<'a, Message> {
    container(bold(label.to_string(), 11).color(Color::WHITE))
        .padding([3, 8])
        .style(move |_| container::Style {
            background: Some(background.into()),
            border: Border {
                radius: 4.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}
/// The machine's state on the MACHINE bar, a fixed width so that nothing
/// beside it moves as it changes; "Tool change" while one is due.
fn state_pill<'a>(c: &crate::control::Connection) -> Element<'a, Message> {
    use uc100::controller::State;
    let state = c.snapshot.state.as_ref();
    let tool_change = phase(Some(c)) == Phase::ToolChange && state == Some(&State::Idle);
    let (label, background) = match state {
        _ if tool_change => ("Tool change", Color::from_rgb8(209, 139, 22)),
        None => ("Connecting…", Color::from_rgb8(106, 118, 134)),
        Some(State::Idle) => ("Idle", Color::from_rgb8(47, 138, 87)),
        Some(State::EStop) => ("E-stop", Color::from_rgb8(200, 30, 30)),
        Some(State::Hold | State::Paused(_)) => (
            state.map_or("", State::label),
            Color::from_rgb8(209, 139, 22),
        ),
        Some(State::Disconnected) => ("Disconnected", Color::from_rgb8(106, 118, 134)),
        Some(s) => (s.label(), RUN_BLUE),
    };
    container(bold(label.to_string(), 12).color(Color::WHITE))
        .center_x(112)
        .padding([4, 0])
        .style(move |_| container::Style {
            background: Some(background.into()),
            border: Border {
                radius: 12.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}
/// The bar of the MACHINE workspace, the one bar over the Control page: the
/// workspace switcher, Control and Controller configuration, the project,
/// and while connected the machine, SIMULATED, its state and the E-stop,
/// the one on the page (LIO-274).
fn machine_bar(app: &App) -> Element<'_, Message> {
    let fit = Fit::of(app);
    let mut bar = row![
        bold("AXIS", 19).color(Color::WHITE),
        switcher(app, true),
        bar_btn("Control", Message::ControlSetup(false), !app.machine_setup),
        bar_btn(
            if fit.narrow {
                "Configuration"
            } else {
                "Controller configuration"
            },
            Message::ControlSetup(true),
            app.machine_setup
        ),
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    let file = Help::text(
        "Project file",
        format!(
            "{}{}: {}",
            if fit.narrow {
                format!("{} · ", workspace_note(app))
            } else {
                String::new()
            },
            project_note(app),
            match &app.project_path {
                Some(p) => p.display().to_string(),
                None => "save it from MANUFACTURE: Save or Save as… there choose the file.".into(),
            }
        ),
    );
    let title = container(
        text(ellipsize(&app.doc.title, if fit.narrow { 22 } else { 30 }))
            .size(12)
            .color(ON_DARK)
            .wrapping(Wrapping::None),
    )
    .width(Fill)
    .clip(true);
    // Narrow, the project file's note is the title's hover alone.
    bar = if fit.narrow {
        bar.push(help::wrap(title, file, widget::tooltip::Position::Bottom))
    } else {
        bar.push(title).push(help::wrap(
            text(project_note(app)).size(11).color(ON_DARK),
            file,
            widget::tooltip::Position::Bottom,
        ))
    };
    // Narrow, the workspace's note is on the title's hover unless saving failed.
    if !fit.narrow || app.save_error.is_some() {
        bar = bar.push(text(workspace_note(app)).size(11).color(ON_DARK));
    }
    if let Some(c) = &app.control {
        use uc100::controller::State;
        let name = (app.doc.machines.iter())
            .find(|m| m.id == c.machine_id)
            .map_or("", |m| m.name());
        bar = bar.push(
            text(ellipsize(name, 18))
                .size(12)
                .color(Color::from_rgb8(223, 230, 239)),
        );
        if c.simulated {
            bar = bar.push(chip("SIMULATED", Color::from_rgb8(194, 122, 18)));
        }
        bar = bar.push(state_pill(c));
        if c.snapshot.state == Some(State::EStop) {
            bar = bar.push(primary("Reset", Message::MachineReset));
        }
        bar = bar.push(estop());
    }
    container(bar)
        .height(fit.machine_bar())
        .width(Fill)
        .padding([0, 12])
        .center_y(fit.machine_bar())
        .style(|_| container::Style {
            background: Some(Color::from_rgb8(29, 43, 58).into()),
            ..Default::default()
        })
        .into()
}

/// The machine picker, Real or Simulated, Connect or Disconnect, and the
/// question before switching to another machine while one is connected:
/// the whole page while not connected, the Machine tab while connected.
fn connect_controls(app: &App) -> Column<'_, Message> {
    let target = app.control_target();
    let mut col = column![].spacing(10);
    let machines: Vec<&Machine> = app
        .doc
        .machines
        .iter()
        .filter(|m| get(&m.p, "controller") == "UC100")
        .collect();
    if machines.is_empty() {
        col = col
            .push(note("No UC100 machine yet. In the Machine library, set a machine's controller to UC100 and choose its UC100 profile file."))
            .push(btn("Machine library", Message::Page(Page::Machines), false));
    }
    let mut picker = Row::new().spacing(6);
    for m in &machines {
        picker = picker.push(btn(
            m.name(),
            Message::ControlMachine(m.id.clone()),
            target.is_some_and(|t| t.id == m.id),
        ));
    }
    col = col.push(picker.wrap());
    if let (Some(c), Some(to)) = (
        &app.control,
        (app.switch_machine.as_ref()).and_then(|id| app.doc.machines.iter().find(|m| &m.id == id)),
    ) {
        let from = (app.doc.machines.iter())
            .find(|m| m.id == c.machine_id)
            .map_or("the machine", |m| m.name());
        col = col.push(
            container(
                column![
                    bold(format!("Switch to {}?", to.name()), 14),
                    muted(format!(
                        "This disconnects from {from} (it is left in E-stop) and connects to {}.",
                        to.name()
                    ))
                    .size(13),
                    row![
                        primary("Disconnect and switch", Message::SwitchMachine(true)),
                        secondary("Cancel", Message::SwitchMachine(false))
                    ]
                    .spacing(8)
                ]
                .spacing(8),
            )
            .padding(12)
            .width(Fill)
            .style(panel_style),
        );
    }
    if let Some(s) = (app.doc.active_setup()).filter(|s| crate::manual::is_hand(&s.machine)) {
        let steps = if crate::lathe::is_lathe(&s.machine) {
            "Manual lathe steps"
        } else {
            "Manual mill steps"
        };
        col = col.push(secondary(steps, Message::ShowGuide));
    }
    if let Some(m) = target {
        let sim = crate::control::simulated(m);
        col = col.push(
            row![
                muted("Connection"),
                btn("Real (USB)", Message::MachineConnection(false), !sim),
                btn("Simulated", Message::MachineConnection(true), sim),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        );
        if app.control.as_ref().is_some_and(|c| c.machine_id == m.id) {
            col = col.push(muted(if sim {
                "Real (USB) disconnects the simulator and connects to the machine itself."
            } else {
                "Simulated disconnects the machine (it is left in E-stop) and connects to the simulator."
            }));
        }
        // The profile is read only while disconnected, not on every 33 ms tick.
        col = match &app.control {
            Some(_) => col.push(secondary("Disconnect", Message::Disconnect)),
            None => match crate::control::profile(m) {
                Ok(_) => col.push(primary("Connect", Message::Connect)),
                Err(e) => col.push(warnings(vec![e])),
            },
        };
    } else if app.control.is_some() {
        // The machine left the library while connected: the connection is
        // still live and still ends here.
        col = col.push(secondary("Disconnect", Message::Disconnect));
    }
    col
}
/// The driver's log, the last 40 lines, newest at the bottom.
// ponytail: the last 40 lines; keep more if someone needs older ones.
fn machine_log(c: &crate::control::Connection) -> Element<'_, Message> {
    let s = &c.snapshot;
    let log = (s.log.iter().skip(s.log.len().saturating_sub(40)))
        .fold(column![].spacing(2), |col, line| {
            col.push(text(line.clone()).size(12).font(Font::MONOSPACE))
        });
    scrollable(log)
        .anchor_bottom()
        .height(Fill)
        .width(Fill)
        .into()
}

/// The Control page: a console that does not scroll (LIO-274). The run bar
/// across the top; under it the hands on the left (position, jog, spindle
/// and feed), always there; one task at a time in the centre's tabs; the
/// operations on the right. Not connected, the machine to connect to.
fn control(app: &App) -> Element<'_, Message> {
    // A setup on the manual mill or the lathe shows its steps, with or
    // without the CNC connected.
    if app.steps_page() {
        return crate::steps::view(app);
    }
    if app.machine_setup {
        let mut col = column![bold("Machine", 18), connect_controls(app)].spacing(10);
        if let Some(c) = &app.control {
            col = col
                .push(rule::horizontal(1))
                .push(bold("Log", 14))
                .push(machine_log(c));
        }
        let sidebar = container(col)
            .padding(16)
            .width(300)
            .height(Fill)
            .style(panel_style);
        return row![sidebar, crate::setup::view(app)]
            .spacing(12)
            .padding(12)
            .height(Fill)
            .into();
    }
    let Some(c) = &app.control else {
        return container(
            container(
                column![
                    bold("Machine", 18),
                    connect_controls(app),
                    muted("Not connected.")
                ]
                .spacing(12),
            )
            .padding(18)
            .width(460)
            .style(panel_style),
        )
        .padding(24)
        .center_x(Fill)
        .height(Fill)
        .into();
    };
    let fit = Fit::of(app);
    column![
        run_bar(app, c),
        row![
            hands(c, fit),
            centre(app, c),
            container(program(app, c)).width(fit.program()).height(Fill)
        ]
        .spacing(fit.gap())
        .padding(fit.gap())
        .height(Fill)
    ]
    .height(Fill)
    .into()
}

/// The green Cycle Start button, greyed out without a message.
fn cycle_start<'a>(msg: Option<Message>, fit: Fit) -> Element<'a, Message> {
    let on = msg.is_some();
    let b = button(
        bold("Cycle Start", if fit.short { 13 } else { 14 })
            .color(Color::WHITE)
            .width(Fill)
            .align_x(Alignment::Center),
    )
    .width(if fit.narrow { 108 } else { 132 })
    .on_press_maybe(msg.clone())
    .padding(if fit.short { [7, 10] } else { [10, 14] })
    .style(move |_, status| button::Style {
        background: Some(
            if !on {
                Color::from_rgb8(166, 186, 176)
            } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                Color::from_rgb8(28, 108, 74)
            } else {
                GREEN
            }
            .into(),
        ),
        text_color: Color::WHITE,
        border: Border {
            radius: 4.into(),
            ..Default::default()
        },
        ..Default::default()
    });
    match msg {
        Some(m) => HelpButton::new(b, &m, "Cycle Start").into(),
        None => b.into(),
    }
}

/// The setups the run bar lists for machine `machine_id`: those that use
/// it, except a manual mill's, which have steps instead of a run.
pub(crate) fn run_setups<'a>(doc: &'a Document, machine_id: &str) -> Vec<(usize, &'a Setup)> {
    (doc.setups.iter().enumerate())
        .filter(|(_, x)| x.machine.id == machine_id && !crate::manual::is_hand(&x.machine))
        .collect()
}
/// A setup as the run bar names it.
fn setup_label(si: usize, x: &Setup) -> String {
    format!(
        "{}. {}{}",
        si + 1,
        x.name(),
        crate::indexed::label(x)
            .map(|a| format!(" · {a}"))
            .unwrap_or_default()
    )
}

/// What the run bar says: the line of what comes next, is being cut, or
/// what holds Cycle Start back (the first reason; all of them on hover and
/// under the operations), as its parts and colours.
pub(crate) fn run_line(
    app: &App,
    c: &crate::control::Connection,
) -> (Vec<(String, Color)>, Vec<String>) {
    use uc100::controller::State;
    let s = &c.snapshot;
    let state = s.state.as_ref();
    let plain = |t: &str| vec![(t.to_string(), INK)];
    if state == Some(&State::EStop) {
        let why = s
            .estop_reason
            .clone()
            .unwrap_or_else(|| "press Reset to enable the machine.".into());
        return (vec![(format!("E-stop: {why}"), ERROR[1])], vec![]);
    }
    if let Some(State::Paused(reason)) = state {
        return (vec![(format!("Paused: {reason}"), AMBER)], vec![]);
    }
    if run_setups(&app.doc, &c.machine_id).is_empty() {
        return (
            plain(
                "No setup uses this machine: in MANUFACTURE, choose it as a setup's machine to run that setup here.",
            ),
            vec![],
        );
    }
    let Some(run) = &c.run else {
        return (
            plain("Pick a setup to run: it is checked as export would check it."),
            vec![],
        );
    };
    let programs = match run.programs.as_deref() {
        None if run.stale => return (plain(crate::run::CHECK_WAITS), vec![]),
        None => {
            return (
                plain("Checking the setup: simulating the job up to it…"),
                vec![],
            );
        }
        Some(Err(problems)) => {
            let first = problems.first().cloned().unwrap_or_default();
            return (
                vec![("This setup cannot run: ".into(), AMBER), (first, AMBER)],
                problems.clone(),
            );
        }
        Some(Ok(programs)) => programs,
    };
    if let Some((p, op, block)) = run.progress(s.current_line) {
        let i = run.running.as_ref().map_or(0, |r| r.index);
        let blocks = run.running_blocks().map_or(1, <[_]>::len);
        // The step being cut, of how many, when the operation has steps.
        let step = (run.step_at(s.current_line))
            .and_then(|(oi, n, _)| Some((oi, n, run.op_steps(oi)?.steps.len())))
            .filter(|t| t.2 > 1)
            .map_or(String::new(), |(oi, n, count)| {
                format!(" · {} of {count}", run.step_label(oi, n))
            });
        let mut parts = vec![
            ("Cutting ".into(), INK),
            (p.ops[op].name.clone(), INK),
            (
                format!(
                    "{step} · tool run {} of {} · operation {} of {} · block {} of {blocks}",
                    i + 1,
                    programs.len(),
                    op + 1,
                    p.ops.len(),
                    block + 1
                ),
                MUTED,
            ),
        ];
        if run.stop_after {
            parts.insert(2, (" · stops after this step".into(), AMBER));
        }
        return (parts, vec![]);
    }
    let mut problems = run.problems(c, &app.doc);
    problems.extend(app.planning_busy().map(String::from));
    let mut parts = match (run.tool_change(), programs.get(run.next)) {
        (Some(p), _) => {
            let o = &p.ops[0];
            vec![(
                format!(
                    "Tool change: {} for {}",
                    crate::model::tool_label(o.tool, &o.tool_name),
                    o.name
                ),
                INK,
            )]
        }
        (None, Some(p)) => {
            let o = &p.ops[run.from.min(p.ops.len() - 1)];
            let line = match run.at_step() {
                Some((oi, n)) => format!("Start {} at {}", o.name, run.step_label(oi, n)),
                None => format!(
                    "Next: {} · {}",
                    o.name,
                    crate::model::tool_label(o.tool, &o.tool_name)
                ),
            };
            vec![(line, INK)]
        }
        (None, None) => vec![],
    };
    match problems.first() {
        Some(first) if parts.is_empty() => parts.push((first.clone(), INK)),
        Some(first) => parts.push((format!("   ✗ {first}"), AMBER)),
        None => parts.push(("   ✓ ready for Cycle Start".into(), GREEN)),
    }
    (parts, problems)
}

/// The run bar across the top of the Control page, one fixed height: the
/// setup to run, what comes next or is being cut (or what holds Cycle Start
/// back), the setup's progress and time left, and Cycle Start, Hold,
/// Resume and Stop now, always in the same places, greyed when they cannot
/// be used (LIO-274).
pub(crate) fn run_bar<'a>(app: &'a App, c: &'a crate::control::Connection) -> Element<'a, Message> {
    use crate::run::Action as R;
    use uc100::controller::State;
    let s = &c.snapshot;
    let state = s.state.as_ref();
    let fit = Fit::of(app);
    let msg = |on: bool, a: R| on.then_some(Message::Run(a));
    let setups = run_setups(&app.doc, &c.machine_id);
    let picked = c.run.as_ref().map(|r| r.setup);
    let labels: Vec<String> = setups.iter().map(|(si, x)| setup_label(*si, x)).collect();
    let index: Vec<usize> = setups.iter().map(|(si, _)| *si).collect();
    let current = picked
        .and_then(|p| index.iter().position(|&si| si == p))
        .map(|k| labels[k].clone());
    let picker_width = if fit.narrow { 170 } else { 200 };
    let picker: Element<_> = if setups.is_empty() || c.setup_locked() {
        container(
            text(current.clone().unwrap_or_else(|| "No setup".into()))
                .size(13)
                .wrapping(Wrapping::None),
        )
        .width(picker_width)
        .clip(true)
        .into()
    } else {
        let options = labels.clone();
        help::wrap(
            pick_list(labels, current, move |l: String| {
                let k = options.iter().position(|o| *o == l).unwrap_or(0);
                Message::Run(R::Setup(index[k]))
            })
            .placeholder("Pick a setup")
            .text_size(13)
            .width(picker_width)
            .style(choice_style),
            help::command(&Message::Run(R::Setup(0)), "Setup to run".into()),
            widget::tooltip::Position::Bottom,
        )
    };
    let (parts, problems) = run_line(app, c);
    let mut line = Row::new();
    let whole: String = parts.iter().map(|(t, _)| t.as_str()).collect();
    // The line's characters at 13 px, about 6.7 px each, in what the setup
    // list and the buttons leave of the window: 118 at 1440 px.
    let mut left = ((app.window_width - if fit.narrow { 500. } else { 650. }) / 6.7) as usize;
    for (t, color) in parts {
        let cut = ellipsize(&t, left.max(1));
        left = left.saturating_sub(cut.chars().count());
        line = line.push(text(cut).size(13).color(color).wrapping(Wrapping::None));
    }
    let line = container(line).width(Fill).clip(true);
    let line: Element<_> = if problems.is_empty() && left > 0 {
        line.into()
    } else if problems.is_empty() {
        help::wrap(
            line,
            Help::text("Run", whole.trim()),
            widget::tooltip::Position::Bottom,
        )
    } else {
        help::wrap(
            line,
            Help::text("Before Cycle Start", problems.join("\n")),
            widget::tooltip::Position::Bottom,
        )
    };
    let (done, times) = match c.run.as_ref().and_then(|r| run_times(r, s)) {
        Some((done, short, long)) => (
            done,
            help::wrap(
                text(short).size(12).color(MUTED).wrapping(Wrapping::None),
                Help::text("Time left", long),
                widget::tooltip::Position::Bottom,
            ),
        ),
        None => (0., Element::from(space().width(0))),
    };
    let what = column![
        line,
        row![
            widget::progress_bar(0.0..=1.0, done).girth(8),
            container(times).width(Length::Shrink)
        ]
        .spacing(10)
        .align_y(Alignment::Center)
    ]
    .spacing(if fit.short { 4 } else { 6 })
    .width(Fill);
    let run = c.run.as_ref();
    let can_start = run.is_some_and(|r| {
        r.checked().is_some() && r.problems(c, &app.doc).is_empty() && app.planning_busy().is_none()
    });
    let pad = if fit.short { [8, 10] } else { [11, 14] };
    let big = |label: &str, m: Option<Message>| act_with(label.to_string(), m, false, None, pad);
    let hold = msg(run.is_some() && state == Some(&State::Run), R::Hold);
    let resume = msg(
        run.is_some() && matches!(state, Some(State::Hold | State::Paused(_))),
        R::Resume,
    );
    let mut buttons = row![cycle_start(
        can_start.then_some(Message::Run(R::CycleStart)),
        fit
    )]
    .spacing(6)
    .align_y(Alignment::Center);
    // Narrow, Hold and Resume share one button of one width: Hold while
    // it runs, Resume while it holds.
    buttons = if !fit.narrow {
        buttons.push(big("Hold", hold)).push(big("Resume", resume))
    } else if resume.is_some() {
        buttons.push(act_with(
            "Resume",
            resume,
            false,
            Some(Length::Fixed(76.)),
            pad,
        ))
    } else {
        buttons.push(act_with("Hold", hold, false, Some(Length::Fixed(76.)), pad))
    };
    // Stop after step, armed or not, one width beside Stop now: short
    // where the bar is narrow.
    let armed = run.is_some_and(|r| r.stop_after);
    let after = act_with(
        if fit.narrow {
            "After step"
        } else {
            "Stop after step"
        },
        msg(
            run.is_some_and(|r| r.running.is_some())
                && matches!(state, Some(State::Run | State::Hold)),
            R::StopAfterStep,
        ),
        armed,
        Some(Length::Fixed(if fit.narrow { 78. } else { 116. })),
        pad,
    );
    let buttons = buttons
        .push(space().width(if fit.narrow { 6 } else { 14 }))
        .push(after)
        .push(big(
            "Stop now",
            msg(run.is_some() && crate::run::busy(state), R::Stop),
        ));
    container(
        row![picker, what, buttons]
            .spacing(if fit.narrow { 12 } else { 16 })
            .align_y(Alignment::Center),
    )
    .height(fit.run_bar())
    .width(Fill)
    .padding([0, 12])
    .center_y(fit.run_bar())
    .style(|_| container::Style {
        background: Some(Color::WHITE.into()),
        border: Border {
            color: LINE,
            width: 1.,
            radius: 0.into(),
        },
        text_color: Some(INK),
        ..Default::default()
    })
    .into()
}

/// The run bar's times: how much of the setup is cut (0 to 1), a short line
/// (the setup's estimate and what is left), and in full what is left of
/// the tool run and its operation and what the estimate counts. None until
/// the check is timed.
fn run_times(
    run: &crate::run::Run,
    s: &uc100::controller::Snapshot,
) -> Option<(f32, String, String)> {
    use crate::run::clock;
    let total = run.setup_time()?;
    let left = run.left(s)?;
    let count = run.checked()?.len();
    let done = if total > 0. {
        (1. - left.setup / total).clamp(0., 1.)
    } else {
        0.
    };
    let short = format!("Setup {} · {} left", clock(total), clock(left.setup));
    let mut parts = vec![format!("Setup {} in all", clock(total))];
    let running = run.running.as_ref();
    let i = running.map_or(run.next, |r| r.index);
    if i >= count {
        parts.push("all tool runs cut".into());
    } else {
        parts.push(format!("{} left", clock(left.setup)));
        parts.push(format!("tool run {}: {} left", i + 1, clock(left.run)));
        if let Some((op, op_left, op_all)) = left.op {
            parts.push(format!(
                "operation {}: {} left of {}",
                op + 1,
                clock(op_left),
                clock(op_all)
            ));
        }
    }
    let feed = (s.feed_override * 100.).round();
    let note = if (feed - 100.).abs() < 0.5 {
        "Estimated from the machine profile's axis speeds and accelerations, with the spindle spin-up and dwells. Manual tool changes and holds are not counted.".to_string()
    } else {
        format!(
            "Estimated from the machine profile's axis speeds and accelerations, with the spindle spin-up and dwells; what is left at the feed override of {feed:.0} %, the setup in all at 100 %. Manual tool changes and holds are not counted."
        )
    };
    Some((done, short, format!("{}.\n\n{note}", parts.join(" · "))))
}

/// The left column of the Control page: the position, the jog and the
/// spindle and feed, on screen in every phase. Each card has a fixed
/// height and every changing text one line of fixed width, so that nothing
/// around the jog pad moves while the machine does: leaving a jog button
/// stops the jog (LIO-274). In a window shorter than the cards, a short
/// screen's (LIO-315), the column scrolls, the Spindle card at its least,
/// its thin scroll bar over the cards' padding.
fn hands(c: &crate::control::Connection, fit: Fit) -> Element<'_, Message> {
    let axes = (0..6).filter(|&a| c.snapshot.axes_enabled[a]).count();
    let least = position_height(axes, fit)
        + jog_height(c.axis(3), fit)
        + spindle_height(fit)
        + 2. * fit.gap();
    responsive(move |size| {
        let cards = column![position_card(c, fit), jog_card(c, fit)].spacing(fit.gap());
        if size.height >= least {
            cards.push(spindle_card(c, fit, None)).height(Fill).into()
        } else {
            let bar = scrollable::Scrollbar::new().width(4).scroller_width(4);
            scrollable(cards.push(spindle_card(c, fit, Some(spindle_height(fit)))))
                .direction(scrollable::Direction::Vertical(bar))
                .height(Fill)
                .into()
        }
    })
    .width(fit.hands())
    .height(Fill)
    .into()
}

/// The Position card: each axis's work position large, its machine
/// position small with whether it is set, Zero and ½; Zero all and Go to X0
/// Y0 under them. With Move to (LIO-275) a target field and Go in place of
/// Zero and ½, and under them the Go of every axis typed, Z up first, and
/// the presets X0 Y0 and Z top. The card's height is the same either way.
fn position_card(c: &crate::control::Connection, fit: Fit) -> Element<'_, Message> {
    use crate::control::{Action, Go};
    let s = &c.snapshot;
    let enabled: Vec<usize> = (0..6).filter(|&a| s.axes_enabled[a]).collect();
    let manual = |a: Action| c.manual().then_some(Message::Machine(a));
    let mono = |bold: bool| Font {
        weight: if bold {
            iced::font::Weight::Bold
        } else {
            iced::font::Weight::Normal
        },
        ..Font::MONOSPACE
    };
    // A Go's button, greyed with why on hover while the machine cannot
    // move; a target that cannot be gone to is refused on the click, in
    // the status line (`Connection::go_to`).
    let go_button = |label: String, go: Go, width: Option<Length>| {
        let ready = c.go_ready(&c.go_axes(go)).and_then(|()| match go {
            Go::Axis(a) if c.targets[a].trim().is_empty() => {
                Err(format!("Type the work {} to go to first.", AXES[a]))
            }
            Go::All if c.go_axes(go) == [2] && c.targets[2].trim().is_empty() => {
                Err("Type a work position to go to first.".into())
            }
            _ => Ok(()),
        });
        let msg = ready.is_ok().then_some(Message::Machine(Action::GoTo(go)));
        let b = act_with(label.clone(), msg, false, width, [4, 8]);
        match ready {
            Ok(()) => b,
            Err(why) => help::wrap(b, Help::text(&label, why), widget::tooltip::Position::Top),
        }
    };
    let toggle: Element<_> = if c.move_to {
        act_with(
            "Move to ✕",
            Some(Message::Machine(Action::MoveTo(false))),
            true,
            None,
            [2, 8],
        )
    } else if c.manual() {
        act_with(
            "Move to…",
            Some(Message::Machine(Action::MoveTo(true))),
            false,
            None,
            [2, 8],
        )
    } else {
        help::wrap(
            act_with("Move to…", None, false, None, [2, 8]),
            Help::text(
                "Move to",
                "Move to moves the machine only while it is Idle: not while it runs, holds or probes, nor in E-stop.",
            ),
            widget::tooltip::Position::Top,
        )
    };
    let mut col = column![
        row![
            bold("Position", 14),
            space().width(Fill),
            muted(format!("Work G{}", 54 + s.active_wcs)),
            toggle,
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .height(22)
    ];
    let (line, size) = fit.dro_row(enabled.len());
    for &a in &enabled {
        let mark: Element<_> = if s.referenced[a] {
            text("✓").size(11).color(GREEN).into()
        } else {
            text("not set").size(10).color(AMBER).into()
        };
        let values = column![
            text(reading(a, s.work_pos[a]))
                .size(size)
                .font(mono(true))
                .width(Fill)
                .align_x(Alignment::End)
                .wrapping(Wrapping::None),
            row![
                text(format!("M {}", reading(a, s.machine_pos[a])))
                    .size(11)
                    .font(mono(false))
                    .color(MUTED)
                    .width(Fill)
                    .align_x(Alignment::End)
                    .wrapping(Wrapping::None),
                container(mark).width(40),
            ]
            .spacing(4)
        ]
        .width(fit.dro_value());
        let line_row = if c.move_to {
            let unit = if crate::control::rotary(a) {
                "°"
            } else {
                "mm"
            };
            let field = text_input(AXES[a], &c.targets[a])
                .on_input_maybe(
                    c.manual()
                        .then_some(move |v| Message::Machine(Action::TargetText(a, v))),
                )
                .on_submit(target_enter(a))
                .width(Fill)
                .padding([4, 6])
                .size(12);
            row![
                bold(AXES[a], 18).width(20),
                values,
                help::field(
                    field,
                    "move_target",
                    &format!("Move to {} · {unit}", AXES[a]),
                    Target::Machine,
                ),
                go_button(format!("Go {}", AXES[a]), Go::Axis(a), None),
            ]
        } else {
            row![
                bold(AXES[a], 18).width(20),
                values,
                space().width(Fill),
                small("Zero", manual(Action::WorkZero(Some(a))), false),
                small("½", manual(Action::WorkHalf(a)), false),
            ]
        };
        col = col.push(
            container(line_row.spacing(8).align_y(Alignment::Center))
                .height(line)
                .center_y(line),
        );
    }
    let mut foot = row![].spacing(6).align_y(Alignment::Center);
    if c.move_to {
        let typed: Vec<String> = (enabled.iter())
            .filter(|&&a| !c.targets[a].trim().is_empty())
            .map(|&a| format!("{}{}", AXES[a], c.targets[a].trim()))
            .collect();
        // Too long for the card, the targets are cut and "Z up" kept.
        let room = if fit.narrow { 25 } else { 30 };
        let to = format!("Go {}", typed.join(" "));
        let label = format!("{} · Z up first", to.trim_end());
        let label = if label.chars().count() <= room {
            label
        } else {
            format!("{} · Z up", ellipsize(&to, room - 7))
        };
        foot = foot.push(go_button(label, Go::All, Some(Fill)));
        if c.axis(0) && c.axis(1) {
            foot = foot.push(go_button("X0 Y0".into(), Go::WorkXY0, None));
        }
        if c.axis(2) && c.config.axes.get(2).is_some_and(|z| z.soft_max.is_some()) {
            foot = foot.push(go_button("Z top".into(), Go::ZTop, None));
        }
    } else {
        foot = foot.push(small("Zero all", manual(Action::WorkZero(None)), false));
        if c.axis(0) && c.axis(1) {
            let to = c.to_work_xy();
            foot = foot.push(small(
                "Go to X0 Y0",
                (to.as_ref().is_ok()).then_some(Message::Machine(Action::GoTo(Go::WorkXY0))),
                false,
            ));
            if let (Err(why), true) = (to, c.manual()) {
                foot = foot.push(hint("Go to X0 Y0", why, fit.chars(30)));
            }
        }
    }
    let foot_height = if fit.short { 28. } else { 32. };
    col = col.push(container(foot).height(foot_height).center_y(foot_height));
    card(col, Some(position_height(enabled.len(), fit)), fit)
}
/// What Enter in an axis's Move to target sends: the value is kept, and
/// the machine does not move; only a Go moves it.
pub(crate) fn target_enter(a: usize) -> Message {
    Message::Machine(crate::control::Action::TargetEnter(a))
}
/// The Position card's height for `axes` axes: the title, the DRO lines,
/// the foot, the padding and the frame.
fn position_height(axes: usize, fit: Fit) -> f32 {
    let foot = if fit.short { 28. } else { 32. };
    22. + fit.dro_row(axes).0 * axes as f32 + foot + 2. * fit.pad() + 2.
}

/// The key that jogs `axis` toward `negative` (`main::jog_key`), as the jog
/// button's tooltip names it.
pub(crate) fn jog_key_name(axis: usize, negative: bool) -> &'static str {
    match (axis, negative) {
        (0, true) => "← Left arrow",
        (0, false) => "→ Right arrow",
        (1, false) => "↑ Up arrow",
        (1, true) => "↓ Down arrow",
        (2, false) => "Page Up",
        (2, true) => "Page Down",
        (3, false) => "Home",
        _ => "End",
    }
}

/// The Jog card: the step, the pad and the speed. Hold a button to jog;
/// letting go, anywhere, or leaving the button stops it. Each button's
/// tooltip names its key; the keys jog while the pad is on screen
/// (`App::jog_pad_shown`).
fn jog_card(c: &crate::control::Connection, fit: Fit) -> Element<'_, Message> {
    use crate::control::Action;
    use uc100::controller::State;
    let s = &c.snapshot;
    let can_jog = c.manual() || s.state == Some(State::Jog);
    let a_axis = c.axis(3);
    let keys = if a_axis {
        "Hold a button, an arrow key (X, Y), Page Up/Down (Z) or Home/End (A) to jog, and let go to stop. The keys jog while the Control page is open and no text field is being edited."
    } else {
        "Hold a button, an arrow key (X, Y) or Page Up/Down (Z) to jog, and let go to stop. The keys jog while the Control page is open and no text field is being edited."
    };
    let badge = container(
        text(if can_jog { "keys jog" } else { "locked" })
            .size(11)
            .color(if can_jog { GREEN } else { MUTED }),
    )
    .padding([1, 7])
    .style(move |_| container::Style {
        background: Some(
            if can_jog {
                Color::from_rgb8(230, 244, 236)
            } else {
                Color::from_rgb8(237, 240, 244)
            }
            .into(),
        ),
        border: Border {
            radius: 9.into(),
            ..Default::default()
        },
        ..Default::default()
    });
    let head = row![
        bold("Jog", 14),
        container(help::wrap(
            badge,
            Help::text("Jog keys", keys),
            widget::tooltip::Position::Right
        ))
        .width(70),
        help::wrap(
            slider(1.0..=100.0, c.jog_speed, |v| Message::Machine(
                Action::JogSpeed(v)
            ))
            .step(1.0_f32),
            help::command(&Message::Machine(Action::JogSpeed(0.)), "Jog speed".into()),
            widget::tooltip::Position::Top,
        ),
        help::wrap(
            text(format!("{:.0} %", c.jog_speed))
                .size(11)
                .color(MUTED)
                .width(36)
                .align_x(Alignment::End)
                .wrapping(Wrapping::None),
            Help::text(
                "Jog speed",
                format!("X jogs at {:.0} mm/min at this speed.", c.jog_feed(0))
            ),
            widget::tooltip::Position::Top,
        ),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .height(22);
    let mut modes = row![small(
        "Cont.",
        Some(Message::Machine(Action::JogStep(None))),
        c.jog_step.is_none()
    )]
    .spacing(4);
    for step in [0.01, 0.1, 1., 10.] {
        modes = modes.push(small(
            if step == 10. {
                "10 mm".to_string()
            } else {
                format!("{step}")
            },
            Some(Message::Machine(Action::JogStep(Some(step)))),
            c.jog_step == Some(step),
        ));
    }
    let mut col = column![head, modes].spacing(jog_gap(fit));
    // A steps in degrees, on its own row: a step in mm means nothing to it.
    if a_axis {
        let mut angles = row![
            text("A").size(12).width(14),
            small(
                "Cont.",
                Some(Message::Machine(Action::JogAngle(None))),
                c.jog_angle.is_none()
            )
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        for step in [0.1, 1., 15., 90.] {
            angles = angles.push(small(
                format!("{step}°"),
                Some(Message::Machine(Action::JogAngle(Some(step)))),
                c.jog_angle == Some(step),
            ));
        }
        // Why A does not jog, in the row's own width: nothing moves.
        angles = angles.push(one_line(
            "A",
            c.jog_blocked(3).unwrap_or_default(),
            11,
            AMBER,
            14,
        ));
        col = col.push(angles);
    }
    let (w, h) = fit.jog();
    let jog = |axis: usize, negative: bool| -> Element<'_, Message> {
        if !c.axis(axis) {
            return space().width(w).height(h).into();
        }
        let can_jog = can_jog && c.jog_blocked(axis).is_none();
        let held = c.held() == Some((axis, negative));
        let label = format!("{}{}", AXES[axis], if negative { "−" } else { "+" });
        let face = container(bold(label, 14).color(if held {
            Color::WHITE
        } else if can_jog {
            INK
        } else {
            MUTED
        }))
        .center_x(w)
        .center_y(h)
        .style(move |_| container::Style {
            background: Some(
                if held {
                    BLUE
                } else if can_jog {
                    Color::from_rgb8(244, 246, 249)
                } else {
                    BG
                }
                .into(),
            ),
            border: Border {
                color: if held { BLUE } else { BUTTON_EDGE },
                width: 1.,
                radius: 6.into(),
            },
            ..Default::default()
        });
        let release = Message::Machine(Action::JogRelease);
        // The key's tooltip floats over the page: it moves nothing.
        widget::tooltip(
            mouse_area(face)
                .on_press(Message::Machine(Action::JogPress(axis, negative)))
                .on_release(release.clone())
                .on_exit(release),
            container(
                text(jog_key_name(axis, negative))
                    .size(12)
                    .color(Color::WHITE),
            )
            .padding([3, 8])
            .style(|_| container::Style {
                background: Some(Color::from_rgb8(29, 43, 58).into()),
                border: Border {
                    radius: 5.into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
            widget::tooltip::Position::Top,
        )
        .gap(4)
        .into()
    };
    let label = |t: &'static str| {
        container(text(t).size(11).color(DIMMED_TEXT))
            .center_x(w)
            .center_y(h)
    };
    let gap = || space().width(w).height(h);
    let mut pad = row![
        column![gap(), jog(0, true), gap()].spacing(6),
        column![jog(1, false), label("XY"), jog(1, true)].spacing(6),
        column![gap(), jog(0, false), gap()].spacing(6),
        space().width(10),
        column![jog(2, false), label("Z"), jog(2, true)].spacing(6),
    ]
    .spacing(6);
    if a_axis {
        pad = pad
            .push(space().width(10))
            .push(column![jog(3, false), label("A"), jog(3, true)].spacing(6));
    }
    col = col.push(container(pad).center_x(Fill));
    card(col, Some(jog_height(a_axis, fit)), fit)
}
/// The space between the Jog card's rows.
fn jog_gap(fit: Fit) -> f32 {
    if fit.short { 5. } else { 7. }
}
/// The Jog card's height, with or without A: the title, the steps, the pad
/// of three rows, the padding and the frame.
fn jog_height(a_axis: bool, fit: Fit) -> f32 {
    let gap = jog_gap(fit);
    let rows = 22. + gap + 26. + gap + (fit.jog().1 * 3. + 12.) + 2. * fit.pad() + 2.;
    if a_axis { rows + gap + 26. } else { rows }
}

/// The least height the Spindle card's rows need: the title, the speed,
/// the rule, the feed and its line, the gaps, the padding and the frame.
fn spindle_height(fit: Fit) -> f32 {
    24. + 28. + 1. + 26. + 15. + 4. * if fit.short { 4. } else { 6. } + 2. * fit.pad() + 2.
}
/// The Spindle and feed card: the spindle's state, speed and direction,
/// Spindle off, the coolant, the feed override and the feed now; `height`
/// tall, or the rest of the column.
fn spindle_card(
    c: &crate::control::Connection,
    fit: Fit,
    height: Option<f32>,
) -> Element<'_, Message> {
    use crate::control::Action;
    use uc100::gcode::Spindle;
    let s = &c.snapshot;
    let start = |state| {
        (c.spindle_ok(state) && c.rpm_value().is_some())
            .then_some(Message::Machine(Action::Spindle(state)))
    };
    let (turning, color) = if !c.rpm.trim().is_empty() && c.rpm_value().is_none() {
        (
            format!("RPM 1 to {:.0}", c.config.outputs.spindle_max_rpm),
            AMBER,
        )
    } else {
        match s.spindle {
            Some(Spindle::Cw) => (format!("CW {:.0} rpm", s.rpm), INK),
            Some(Spindle::Ccw) => (format!("CCW {:.0} rpm", s.rpm), INK),
            _ => ("Off".into(), MUTED),
        }
    };
    let mut notes = vec![];
    if !c.reversible() {
        notes.push("No CCW: the profile has no spindle direction output.");
    }
    if c.config.outputs.flood.is_none() && c.config.outputs.mist.is_none() {
        notes.push("The profile has no coolant outputs.");
    }
    let title: Element<_> = if notes.is_empty() {
        bold("Spindle", 14).into()
    } else {
        help::wrap(
            bold("Spindle", 14),
            Help::text("Spindle and coolant", notes.join(" ")),
            widget::tooltip::Position::Right,
        )
    };
    let mut head = row![
        title,
        text(turning)
            .size(14)
            .font(Font::MONOSPACE)
            .color(color)
            .width(Fill)
            .wrapping(Wrapping::None),
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    let coolant = |on: bool, label: &'static str, f: fn(bool) -> Action| {
        checkbox(on)
            .label(label)
            .on_toggle(move |b| Message::Machine(f(b)))
            .text_size(12)
    };
    if c.config.outputs.flood.is_some() {
        head = head.push(coolant(s.flood, "Flood", Action::Flood));
    }
    if c.config.outputs.mist.is_some() {
        head = head.push(coolant(s.mist, "Mist", Action::Mist));
    }
    let mut spin = row![
        help::field(
            text_input("RPM", &c.rpm)
                .on_input(|v| Message::Machine(Action::Rpm(v)))
                .width(80)
                .padding([5, 8])
                .size(13),
            "control_rpm",
            "Spindle speed · RPM",
            Target::Machine
        ),
        small("CW", start(Spindle::Cw), s.spindle == Some(Spindle::Cw))
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    if c.reversible() {
        spin = spin.push(small(
            "CCW",
            start(Spindle::Ccw),
            s.spindle == Some(Spindle::Ccw),
        ));
    }
    spin = spin.push(small(
        "Spindle off",
        Some(Message::Machine(Action::Spindle(Spindle::Off))),
        false,
    ));
    let feed = row![
        bold("Feed", 14),
        text(format!("{:.0} %", c.feed_override))
            .size(13)
            .font(Font::MONOSPACE)
            .width(48),
        help::wrap(
            slider(10.0..=200.0, c.feed_override, |v| Message::Machine(
                Action::FeedOverride(v)
            ))
            .step(5.0_f32),
            help::command(
                &Message::Machine(Action::FeedOverride(100.)),
                "Feed override".into()
            ),
            widget::tooltip::Position::Top,
        ),
        small(
            "100 %",
            Some(Message::Machine(Action::FeedOverride(100.))),
            false
        ),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    card(
        column![
            head.height(24),
            spin,
            rule::horizontal(1),
            feed,
            text(format!("Feed now {:.0} mm/min", s.feed_rate))
                .size(11)
                .color(MUTED)
                .wrapping(Wrapping::None),
        ]
        .spacing(if fit.short { 4 } else { 6 }),
        height,
        fit,
    )
}

/// A tab of the Control page's centre.
fn control_tab<'a>(tab: ControlTab, shown: bool, alert: bool, fit: Fit) -> Element<'a, Message> {
    let color = match (shown, alert) {
        (_, true) => AMBER,
        (true, false) => BLUE,
        _ => MUTED,
    };
    let face = text(tab.label()).size(13).color(color).font(Font {
        weight: if shown {
            iced::font::Weight::Semibold
        } else {
            iced::font::Weight::Normal
        },
        ..Font::DEFAULT
    });
    button(
        column![
            face,
            container(space().height(2)).width(Fill).style(move |_| {
                container::Style {
                    background: shown.then(|| color.into()),
                    ..Default::default()
                }
            })
        ]
        .spacing(7)
        .width(Length::Shrink),
    )
    .padding(if fit.narrow { [8, 7] } else { [8, 10] })
    .on_press(Message::ControlTab(tab))
    .style(|_, status| button::Style {
        background: matches!(status, button::Status::Hovered | button::Status::Pressed)
            .then(|| HOVER_BUTTON.into()),
        ..Default::default()
    })
    .into()
}
/// The tabs the centre offers now: Tool change while one is due, Rotary A
/// on a machine with the rotary unit.
pub(crate) fn control_tabs(c: &crate::control::Connection) -> Vec<ControlTab> {
    let mut tabs = vec![ControlTab::Job];
    match phase(Some(c)) {
        Phase::ToolChange => tabs.push(ControlTab::ToolChange),
        Phase::Start => tabs.push(ControlTab::Start),
        _ => {}
    }
    tabs.extend([ControlTab::WorkZero, ControlTab::Probe]);
    if c.rotary.is_some() && c.axis(3) {
        tabs.push(ControlTab::Rotary);
    }
    tabs.extend([ControlTab::Mdi, ControlTab::Machine]);
    tabs
}
/// The centre of the Control page: one task at a time in its tabs.
fn centre<'a>(app: &'a App, c: &'a crate::control::Connection) -> Element<'a, Message> {
    let fit = Fit::of(app);
    let tabs = control_tabs(c);
    let shown = if tabs.contains(&app.control_tab) {
        app.control_tab
    } else {
        ControlTab::Job
    };
    let mut bar = Row::new().spacing(2).align_y(Alignment::End);
    for &t in &tabs {
        let alert = matches!(t, ControlTab::ToolChange | ControlTab::Start);
        bar = bar.push(control_tab(t, t == shown, alert, fit));
    }
    // Narrow, the view switcher leaves the tabs their room: it is in the
    // Job tab's foot.
    if shown == ControlTab::Job && !fit.narrow {
        bar = bar
            .push(space().width(Fill))
            .push(job_views(app))
            .push(space().width(6));
    }
    let body: Element<_> = match shown {
        ControlTab::Job => job_tab(app, c),
        ControlTab::ToolChange => tool_change_tab(app, c),
        ControlTab::Start => start_tab(app, c),
        ControlTab::WorkZero => work_zero_tab(app, c),
        ControlTab::Probe => probe_tab(c, fit),
        ControlTab::Rotary => rotary_tab(c).unwrap_or_else(|| space().into()),
        ControlTab::Mdi => mdi_tab(c, fit),
        ControlTab::Machine => machine_tab(app, c),
    };
    container(
        column![
            container(bar.align_y(Alignment::Center)).padding([0, 6]),
            rule::horizontal(1),
            container(body)
                .padding(if fit.short { [8, 12] } else { [10, 14] })
                .height(Fill)
                .width(Fill)
        ]
        .height(Fill),
    )
    .width(Fill)
    .height(Fill)
    .style(panel_style)
    .clip(true)
    .into()
}
/// The safety net of a tab whose content may grow past its area (a run that
/// starts partway on pins): it scrolls inside the tab, never the page.
fn tab_scroll<'a>(body: Column<'a, Message>) -> Element<'a, Message> {
    scrollable(body.padding(iced::Padding {
        right: SCROLLBAR_GUTTER,
        ..Default::default()
    }))
    .height(Fill)
    .width(Fill)
    .into()
}

/// The Job tab: the one 3D view, of the setup's stock as it is cut (the
/// machine's tool red), or of the machine's travel with the tool in it.
/// The Job tab's view switcher: the stock, the machine, the stock from above.
fn job_views(app: &App) -> Element<'_, Message> {
    let mut views = row![].spacing(2);
    for (view, label) in [
        (JobView::Stock, "Stock"),
        (JobView::Machine, "Machine"),
        (JobView::Top, "Top"),
    ] {
        views = views.push(small(
            label,
            Some(Message::ControlView(view)),
            app.control_view == view,
        ));
    }
    views.into()
}
fn job_tab<'a>(app: &'a App, c: &'a crate::control::Connection) -> Element<'a, Message> {
    let fit = Fit::of(app);
    let picked = c.run.as_ref().filter(|r| {
        app.doc
            .setups
            .get(r.setup)
            .is_some_and(|x| x.id == app.doc.active)
    });
    let stock = (app.control_view != JobView::Machine && picked.is_some())
        .then(|| scene(app, None))
        .flatten();
    let mut col = column![].spacing(6).height(Fill);
    // The line under the view, how to turn it: short, a hover card on the
    // switcher's left; narrow, the switcher is on its right.
    let foot = |title: &str, line: String| -> Option<Element<'a, Message>> {
        match (fit.short, fit.narrow) {
            (false, false) => Some(hint(title, line, 110)),
            (false, true) => Some(
                row![hint(title, line, 60), job_views(app)]
                    .spacing(8)
                    .into(),
            ),
            (true, false) => None,
            (true, true) => Some(row![space().width(Fill), job_views(app)].into()),
        }
    };
    match stock {
        Some(view) => {
            col = col.push(container(view).height(Fill).width(Fill));
            col = col.extend(foot(
                "Job",
                format!(
                    "{}Drag to orbit · right-drag to pan · scroll to zoom. The machine's tool is red while a tool run cuts, and the stock is cut away where it goes.",
                    (app.doc.active_setup().and_then(crate::indexed::label))
                        .map(|a| format!("{} at {a} · ", app.doc.setup().name()))
                        .unwrap_or_default()
                ),
            ));
            if let Some(line) = uncut_line(app) {
                col = col.push(hint("Completeness", line, fit.chars(110)));
            }
            if let Some(legend) = uncut_legend(app) {
                col = col.push(legend);
            }
        }
        None => {
            col = col
                .push(container(canvas(machine_model(app, c)).width(Fill).height(Fill)).height(Fill))
                .extend(foot(
                    "Machine",
                    if picked.is_none() && app.control_view != JobView::Machine {
                        "The machine's travel, the tool in it: pick a setup in the run bar to see its stock."
                    } else {
                        "The box is where the tool tip can travel; the red dot is the tool tip, the green ring the work zero. Drag to turn."
                    }
                    .into(),
                ));
        }
    }
    col.into()
}
/// The machine's travel and the tool in it, for the Job tab.
fn machine_model<'a>(
    app: &'a App,
    c: &'a crate::control::Connection,
) -> crate::machine_view::MachineView {
    let s = &c.snapshot;
    let (envelope, assumed) = crate::machine_view::MachineView::envelope(&c.config);
    crate::machine_view::MachineView {
        pos: [
            s.machine_pos[0] as f32,
            s.machine_pos[1] as f32,
            s.machine_pos[2] as f32,
        ],
        known: (0..3).all(|a| !c.axis(a) || s.referenced[a]),
        envelope,
        assumed,
        block: c
            .block
            .map(|(lo, hi)| [lo.map(|v| v as f32), hi.map(|v| v as f32)]),
        zero: std::array::from_fn(|k| s.work_offset[k] as f32),
        rotary: c.rotary.as_ref().filter(|_| c.axis(3)).map(|r| {
            let sign = (app.doc.machines.iter())
                .find(|m| m.id == c.machine_id)
                .map_or(1., crate::indexed::sign);
            crate::chuck::Rig::on_machine(r.unit, sign)
        }),
        a: s.machine_pos[3] as f32,
    }
}

/// A numbered step of the Tool change or Work zero tab: done, the one to do
/// now, or one to come, with its buttons.
fn step_box<'a>(
    n: usize,
    done: bool,
    now: bool,
    body: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mark = container(
        text(if done {
            "✓".to_string()
        } else {
            n.to_string()
        })
        .size(12)
        .color(if done || now { Color::WHITE } else { INK }),
    )
    .center_x(22)
    .center_y(22)
    .style(move |_| container::Style {
        background: Some(
            if done {
                Color::from_rgb8(47, 138, 87)
            } else if now {
                Color::from_rgb8(209, 139, 22)
            } else {
                Color::from_rgb8(237, 240, 244)
            }
            .into(),
        ),
        border: Border {
            radius: 11.into(),
            ..Default::default()
        },
        ..Default::default()
    });
    container(
        row![mark, body.into()]
            .spacing(10)
            .align_y(Alignment::Start),
    )
    .padding([7, 10])
    .width(Fill)
    .style(move |_| container::Style {
        background: now.then(|| Color::from_rgb8(255, 250, 240).into()),
        border: Border {
            color: if now {
                Color::from_rgb8(230, 184, 102)
            } else {
                LINE
            },
            width: 1.,
            radius: 7.into(),
        },
        ..Default::default()
    })
    .into()
}
/// The reference surface's line and its buttons: Record reference, Set Z
/// from reference, and with a probe input the same by probe. For a setup of
/// more than one tool run.
fn reference_rows<'a>(
    c: &'a crate::control::Connection,
    run: &'a crate::run::Run,
    fit: Fit,
) -> Column<'a, Message> {
    use crate::run::Action as R;
    let s = &c.snapshot;
    let running = run.running.is_some();
    let msg = |on: bool, a: R| on.then_some(Message::Run(a));
    let manual = c.manual() && !running;
    let set_ok = run.reference.is_some() && run.reference_a_problem(c).is_none();
    let mut col = column![
        row![
            small("Record reference", msg(manual, R::RecordReference), false),
            small(
                "Set Z from reference",
                msg(manual && set_ok, R::ZFromReference),
                false
            ),
            muted(match (run.reference, run.reference_a, fit.narrow) {
                (Some(z), Some(a), false) =>
                    format!("Reference: Z {z:.3} (G{}) at A {a:.3}°", 54 + s.active_wcs),
                (Some(z), Some(a), true) => format!("Z {z:.3} at A {a:.1}°"),
                (Some(z), None, _) => format!("Reference: Z {z:.3} (G{})", 54 + s.active_wcs),
                (None, _, false) => "Reference surface: not recorded".into(),
                (None, _, true) => "Not recorded".into(),
            })
            .wrapping(Wrapping::None),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
    ]
    .spacing(6);
    if c.config.inputs.probe.is_some() {
        let probe = c.probe_blocked().is_none() && !running;
        col = col.push(
            row![
                small("Probe reference", msg(probe, R::ProbeReference), false),
                small(
                    "Probe Z from reference",
                    msg(probe && set_ok, R::ProbeFromReference),
                    false
                ),
                hint(
                    "Probe reference",
                    if c.direct() {
                        "The same with the plate's lead on the reference surface, which the tool touches."
                    } else {
                        "The same with the touch plate on the reference surface."
                    },
                    fit.chars(40)
                ),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        );
        if let Some(note) = probe_note(c) {
            col = col.push(note);
        }
    }
    if let Some(why) = run.reference_a_problem(c).filter(|_| !running) {
        col = col.push(hint("Reference", why, fit.chars(90)));
    }
    col
}
/// Touch off on the A reference, for a next tool run that turns A: its
/// tool touched off on the Z reference the probe touched with the A
/// centreline (LIO-207). None for one that does not turn A.
fn a_reference_row<'a>(
    app: &'a App,
    c: &'a crate::control::Connection,
    run: &'a crate::run::Run,
) -> Option<Element<'a, Message>> {
    use crate::run::Action as R;
    let fit = Fit::of(app);
    let p =
        (run.checked()?.get(run.next)).filter(|p| !crate::run::a_angles(&p.blocks).is_empty())?;
    let running = run.running.is_some();
    let o = &p.ops[0];
    let ready = c.a_reference(&app.doc);
    let said = match run.length.as_ref().filter(|(id, ..)| *id == o.tool_id) {
        Some((_, d, _)) => format!(
            "T{} is touched off on the A centreline's Z reference: its tip is {:.3} mm {} the probe's, and its work Z is on the centreline.",
            o.tool,
            d.abs(),
            if *d >= 0. { "under" } else { "over" }
        ),
        None => format!(
            "T{} {}: with it in the spindle, jog over the Z reference the probe touched with the A centreline, put the touch plate on it and press Touch off on the A reference. That sets its work Z on the centreline, whatever its length.",
            o.tool, o.tool_name
        ),
    };
    let mut col = column![
        row![
            small(
                "Touch off on the A reference",
                (ready.is_ok() && !running).then_some(Message::Run(R::ProbeCentreReference)),
                false
            ),
            hint("A reference", said, fit.chars(60)),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
    ]
    .spacing(4);
    if let (Err(why), false) = (&ready, running) {
        col = col.push(hint("A reference", why.clone(), fit.chars(90)));
    }
    Some(col.into())
}

/// The Tool change tab, while the next tool run wants another tool: what
/// to do, step by step, with its buttons; it opens by itself when the tool
/// change is due.
fn tool_change_tab<'a>(app: &'a App, c: &'a crate::control::Connection) -> Element<'a, Message> {
    let fit = Fit::of(app);
    let Some(run) = c.run.as_ref() else {
        return space().into();
    };
    let (Some(p), Some(programs)) = (run.tool_change(), run.checked()) else {
        return muted("No tool change is due.").into();
    };
    let s = &c.snapshot;
    let o = &p.ops[0];
    let new = crate::model::tool_label(o.tool, &o.tool_name);
    let old = (run.next.checked_sub(1))
        .and_then(|i| programs.get(i))
        .map(|q| crate::model::tool_label(q.ops[0].tool, &q.ops[0].tool_name));
    let recorded = run.reference.is_some();
    let touched = run.touched;
    let mut col = column![
        row![
            chip("TOOL CHANGE", Color::from_rgb8(209, 139, 22)),
            bold(
                format!(
                    "{}next {new} for tool run {}: {}",
                    old.map(|o| format!("{o} done · ")).unwrap_or_default(),
                    run.next + 1,
                    o.name
                ),
                14
            )
            .width(Fill)
            // Narrow, a long tool or operation name wraps, not cut.
            .wrapping(if fit.narrow {
                Wrapping::Word
            } else {
                Wrapping::None
            }),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
    ]
    .spacing(8);
    col = col.push(step_box(
        1,
        true,
        false,
        text("The last tool run ended with the spindle and coolant off at the clearance plane. X and Y zero stay as they are.").size(12),
    ));
    col = col.push(step_box(
        2,
        recorded,
        !recorded,
        column![
            text("If no reference is recorded, touch the old tool on a surface that stays put (a vise jaw, the table, a block) and record it.").size(12),
            reference_rows(c, run, fit)
        ]
        .spacing(6),
    ));
    col = col.push(step_box(
        3,
        touched,
        recorded && !touched,
        text(format!(
            "Jog up and put in {new}, stickout {:.1} mm. Jog is live on the left.",
            o.stickout
        ))
        .size(12),
    ));
    let mut four = column![text("Touch the new tool on the same surface and press Set Z from reference above (or Probe Z from reference), or Zero Z on the stock top if no tool run has cut it.").size(12)].spacing(6);
    let with_a = app
        .doc
        .setups
        .get(run.setup)
        .is_some_and(|x| x.machine.has_a())
        && c.axis(3);
    if with_a {
        // A turns the stock and the chuck: a reference on them is touched
        // off at the angle it was recorded at.
        let first = crate::run::a_angles(&p.blocks);
        let back = match first.first() {
            Some(a) => format!(
                "Cycle Start's first moves retract Z, then turn A alone back to {}° for this tool run. It turns A, so touch the new tool off with Touch off on the A reference: Set Z from reference does not relate it to the probed centreline.",
                crate::indexed::degrees(*a)
            ),
            None => "This tool run does not turn A: turn A back to A0 before Cycle Start.".into(),
        };
        four = four.push(hint(
            "A axis",
            match run.reference_a {
                Some(ra) => format!(
                    "A axis: the reference was recorded with A at {ra:.3}°. Touch the new tool off with A there (Set Z from reference waits for it), then {}",
                    back[..1].to_lowercase() + &back[1..]
                ),
                None => format!(
                    "A axis: record the reference and touch both tools off at the same A angle; A where it is now ({:.3}°) will do. {back}",
                    s.work_pos[3]
                ),
            },
            fit.chars(100),
        ));
    }
    if let Some(row) = a_reference_row(app, c, run) {
        four = four.push(row);
    }
    col = col.push(step_box(4, touched, false, four));
    col = col.push(step_box(
        5,
        false,
        touched,
        text(format!(
            "Jog clear and press Cycle Start: it continues with {}.",
            o.name
        ))
        .size(12),
    ));
    tab_scroll(col)
}

/// The Start at a step tab (LIO-275): the checks before Cycle Start starts
/// at a step, whether picked in the Operations list or where a run stopped
/// (`run::Run::resumed`), each done or to do with its buttons: the machine
/// position, the work zero the steps before were cut with, the tool's Z;
/// and what Cycle Start then does.
fn start_tab<'a>(app: &'a App, c: &'a crate::control::Connection) -> Element<'a, Message> {
    use crate::control::Action;
    use crate::run::Action as R;
    let fit = Fit::of(app);
    let Some(run) = c.run.as_ref() else {
        return space().into();
    };
    let (Some((oi, n)), Some(p)) = (run.at_step(), run.checked().and_then(|c| c.get(run.next)))
    else {
        return muted("No start at a step is picked.").into();
    };
    let s = &c.snapshot;
    let o = &p.ops[run.from.min(p.ops.len() - 1)];
    let step = run.step_label(oi, n);
    let before = run.step_label(oi, n - 1);
    let wrap = if fit.narrow {
        Wrapping::Word
    } else {
        Wrapping::None
    };
    let mut col =
        column![bold(format!("▶ Start {} at {step}", o.name), 14).wrapping(wrap)].spacing(8);
    if let Some(j) = (run.resumed.as_ref()).filter(|j| j.op == oi && j.step == Some(n - 1)) {
        col = col.push(
            text(format!(
                "It stopped after {before} {} ({}), and the program is unchanged, so {step} is picked for you. Pick another step in the Operations list if you like: the checks are the same.",
                j.ago(crate::run::now()),
                j.why
            ))
            .size(12)
            .color(MUTED),
        );
    }
    let idle = |a: Action| c.idle().then_some(Message::Machine(a));
    // 1. The machine position, which the work zero is kept in.
    let placed = (0..3).all(|a| s.referenced[a]);
    let mut one = column![
        text(if placed {
            "Machine position: X, Y and Z have theirs, which the work zero is kept in."
        } else {
            "Machine position: X, Y or Z has none (an E-stop or a new session clears it). Set it where it was, so that the work zero below is where it was:"
        })
        .size(12)
    ]
    .spacing(6);
    if !placed {
        let mut b = row![].spacing(6);
        if c.home_switches() {
            b = b.push(small("Home again", idle(Action::Home), false));
        } else {
            b = b
                .push(small(
                    "Set machine zero here",
                    idle(Action::MachineZero(None)),
                    false,
                ))
                .push(small(
                    "Keep the last position",
                    idle(Action::KeepPosition),
                    false,
                ));
        }
        one = one.push(b);
    }
    col = col.push(step_box(1, placed, !placed, one));
    // 2. The work zero the steps before were cut with.
    let manual = run.running.is_none() && c.manual();
    let offset = &p.offset;
    let recorded = (run.resumed.as_ref())
        .filter(|j| j.op == oi)
        .filter(|j| (0..3).any(|a| (j.offset[a] - s.work_offset[a]).abs() > 1e-3));
    let mut two = column![
        text(format!(
            "Work zero {offset}, the one {before} was cut with: keep it, or set it again on the stock the steps left."
        ))
        .size(12)
    ]
    .spacing(6);
    if let Some(j) = recorded {
        two = two.push(
            text(format!(
                "{offset} is not the work offset recorded when it stopped: X {:.3} Y {:.3} Z {:.3} then, X {:.3} Y {:.3} Z {:.3} now.",
                j.offset[0],
                j.offset[1],
                j.offset[2],
                s.work_offset[0],
                s.work_offset[1],
                s.work_offset[2]
            ))
            .size(12)
            .color(AMBER),
        );
    }
    two = two.push(
        row![
            small(
                "Keep it",
                (manual && !run.zeroed).then_some(Message::Run(R::KeepZero)),
                run.zeroed
            ),
            small(
                "Probe the stock again",
                Some(Message::ControlTab(ControlTab::WorkZero)),
                false
            ),
        ]
        .spacing(6),
    );
    col = col.push(step_box(2, run.zeroed, placed && !run.zeroed, two));
    // 3. The tool's Z.
    let tool = crate::model::tool_label(o.tool, &o.tool_name);
    let kept = run.tool_kept || run.touched;
    let three = column![
        text(format!(
            "{tool}'s Z: the same tool, not taken out since, or touched off again."
        ))
        .size(12),
        row![
            small(
                "Same tool, keep Z",
                (manual && !kept).then_some(Message::Run(R::KeepToolZ)),
                run.tool_kept
            ),
            small(
                "Set Z from reference",
                (manual && run.reference.is_some() && run.reference_a_problem(c).is_none())
                    .then_some(Message::Run(R::ZFromReference)),
                run.touched
            ),
        ]
        .spacing(6)
    ]
    .spacing(6);
    col = col.push(step_box(3, kept, run.zeroed && !kept, three));
    // 4. What Cycle Start does.
    let over = (p.from(run.from).ok().into_iter().flatten()).find_map(|b| match b {
        crate::post::Block::Descend { xy, .. } => Some(*xy),
        _ => None,
    });
    let at = over.map_or(String::new(), |xy| {
        format!(
            " to X {:.1} Y {:.1}",
            xy[0] - p.datum[0],
            xy[1] - p.datum[1]
        )
    });
    col = col.push(step_box(
        4,
        false,
        false,
        text(format!(
            "Cycle Start: spindle on, rapid at the clearance plane{at}, down over the entry, then it waits there; Resume feeds in and cuts on from {step}."
        ))
        .size(12),
    ));
    tab_scroll(col)
}

/// The Work zero tab: each axis set to a value, the work system, the work
/// zero on the stock for the setup picked, step by step until its first
/// tool run, and the reference surface for its tool changes.
const WORK_ZERO_NOTE: &str = "Zero makes the current position work zero in the active coordinate system; ½ halves the position shown, Set gives it the value typed. For the middle of a part by hand, zero X with an edge finder on its left side, touch its right side and press ½ on X. G54 to G59 each keep their own; they change only while Idle.";
fn work_zero_tab<'a>(app: &'a App, c: &'a crate::control::Connection) -> Element<'a, Message> {
    let fit = Fit::of(app);
    use crate::control::Action;
    use crate::run::Action as R;
    let s = &c.snapshot;
    let idle = |a: Action| c.idle().then_some(Message::Machine(a));
    let manual = |a: Action| c.manual().then_some(Message::Machine(a));
    let mut set = row![muted("Set").width(28)]
        .spacing(6)
        .align_y(Alignment::Center);
    for a in (0..6).filter(|&a| s.axes_enabled[a]) {
        set = set
            .push(help::field(
                text_input(AXES[a], &c.presets[a])
                    .on_input(move |v| Message::Machine(Action::WorkText(a, v)))
                    .on_submit_maybe(
                        (c.manual() && c.preset(a).is_some())
                            .then_some(Message::Machine(Action::WorkSet(a))),
                    )
                    .width(if fit.narrow { 56 } else { 70 })
                    .padding([5, 8])
                    .size(13),
                "work_set",
                &format!(
                    "Set {} · {}",
                    AXES[a],
                    if crate::control::rotary(a) {
                        "°"
                    } else {
                        "mm"
                    }
                ),
                Target::Machine,
            ))
            .push(small(
                format!("Set {}", AXES[a]),
                manual(Action::WorkSet(a)).filter(|_| c.preset(a).is_some()),
                false,
            ))
            .push(space().width(if fit.narrow { 2 } else { 6 }));
    }
    // Narrow, X Y Z and A wrap rather than run off the tab.
    let set: Element<_> = if fit.narrow {
        set.wrap().into()
    } else {
        set.into()
    };
    let mut systems = row![help::wrap(
        muted("Work system").width(80),
        Help::text("Work zero", WORK_ZERO_NOTE),
        widget::tooltip::Position::Top,
    )]
    .spacing(4)
    .align_y(Alignment::Center);
    for i in 0..6 {
        systems = systems.push(small(
            format!("G{}", 54 + i),
            idle(Action::Wcs(i)),
            s.active_wcs == i,
        ));
    }
    let mut col = column![set, systems].spacing(8);
    // Short, the hint is on the work systems' hover.
    if !fit.short {
        col = col.push(hint("Work zero", WORK_ZERO_NOTE, fit.chars(110)));
    }
    let Some(run) = &c.run else {
        return tab_scroll(col.push(muted(
            "Pick a setup in the run bar: its work zero on the stock is set here.",
        )));
    };
    let running = run.running.is_some();
    if let Some(steps) = work_zero_steps(app, c) {
        // The buttons act on the checked setup the steps are from, which an
        // edit may have changed: like the probes, they wait while its check
        // is out of date, and say so (#39).
        let checked = c.run_program();
        let mut buttons = row![].spacing(6).align_y(Alignment::Center);
        if c.sim_zero().is_some() {
            let ready = checked.is_ok() && c.idle() && c.unset_limited_axes().is_empty();
            buttons = buttons.push(small(
                "Move there",
                ready.then_some(Message::Machine(Action::ToSimZero)),
                false,
            ));
        }
        let manual = checked.is_ok() && c.manual();
        let pins = checked.as_ref().is_ok_and(|p| p.pins.is_some());
        if pins {
            buttons = buttons.push(small(
                "Set X Y from the pins",
                (manual && c.pin_offset().is_ok()).then_some(Message::Machine(Action::PinZero)),
                false,
            ));
        }
        let blocked = c.partway_blocked();
        buttons = buttons.push(match c.touch_off() {
            _ if blocked.is_some() => small("Zero Z on the stock top", None, false),
            Some((_, work)) if pins || run.partway() => small(
                format!("Set Z{}", crate::post::exact_mm(work[2] as f32)),
                manual.then_some(Message::Machine(Action::TouchOff)),
                false,
            ),
            Some((_, work)) => small(
                touch_label(work),
                manual.then_some(Message::Machine(Action::TouchOff)),
                false,
            ),
            None => small(
                "Zero all",
                manual.then_some(Message::Machine(Action::WorkZero(None))),
                false,
            ),
        });
        if run.partway() {
            buttons = buttons.push(small(
                "Keep this work zero",
                (checked.is_ok() && !run.zeroed && !running).then_some(Message::Run(R::KeepZero)),
                run.zeroed,
            ));
        }
        if let Err(why) = checked {
            buttons = buttons.push(hint("Work zero", why, fit.chars(50)));
        }
        let mut list = column![bold("Work zero on the stock", 13)].spacing(4);
        for line in steps {
            list = list.push(text(line).size(12));
        }
        col = col.push(rule::horizontal(1)).push(list).push(buttons);
    } else if run.checked().is_some() {
        col = col.push(rule::horizontal(1)).push(muted(
            "The setup's work zero was set on the stock before its first tool run: X and Y stay as they are from here.",
        ));
    }
    if run.checked().is_some_and(|p| p.len() > 1) {
        col = col
            .push(rule::horizontal(1))
            .push(bold("Reference surface, for the tool changes", 13))
            .push(reference_rows(c, run, Fit::of(app)));
    }
    if let Some(row) = a_reference_row(app, c, run) {
        col = col.push(row);
    }
    tab_scroll(col)
}

/// The Probe tab: Probe Z, the setup's work zero from its stock, and a
/// feature at the tip. Without a probe input in the profile, one line and
/// the way to the Controller configuration (LIO-273).
fn probe_tab(c: &crate::control::Connection, fit: Fit) -> Element<'_, Message> {
    use crate::control::{Action, Feature};
    if c.config.inputs.probe.is_none() {
        return column![
            muted(
                "This machine's UC100 profile has no probe input: add one under Inputs to probe."
            ),
            secondary("Controller configuration", Message::ControlSetup(true)),
        ]
        .spacing(10)
        .into();
    }
    // What stops every probe, said once at the top; each button below says
    // only what stops it alone.
    let global = c.probe_ready(&[2]).filter(|_| !c.probing());
    let own_why = |why: Option<String>| why.filter(|w| Some(w) != global.as_ref());
    let blocked = c.probe_z_blocked();
    // The stock top's work Z the plate lies on.
    let top = c.stock_top().unwrap_or_default();
    let mut col = column![].spacing(10);
    if let Some(why) = &global {
        col = col.push(one_line("Probe", why.clone(), 12, AMBER, fit.chars(110)));
    }
    if let Some(note) = probe_note(c) {
        col = col.push(note);
    }
    col = col.push(
        row![
            small(
                "Probe Z",
                blocked.is_none().then_some(Message::Machine(Action::ProbeZ)),
                c.probing() && !c.edging() && c.feature().is_none()
            ),
            hint(
                "Probe Z",
                match (own_why(blocked.clone()), &c.probe) {
                    (Some(why), _) if !c.probing() => why,
                    (_, Ok(p)) if c.direct() => format!(
                        "{} conducts: no plate. The clip stays on the tool and the plate's lead goes on bare metal of the stock; touch the tool to the stock by hand first and see the probe input go active. Feeds down at {} mm/min, up to {} mm. Work Z where the tool touches the stock top becomes {}, then the tool lifts {} mm.",
                        c.material.as_ref().map_or("The stock", |(name, _)| name.as_str()),
                        p.feed,
                        p.distance,
                        crate::post::exact_mm(top as f32),
                        p.retract
                    ),
                    (_, Ok(p)) => format!(
                        "{}: touch plate on the stock top under the tool, the clip on the tool. Feeds down at {} mm/min, up to {} mm. Work Z at the plate's top becomes {} mm, the stock top Z {}, then the tool lifts {} mm.",
                        match &c.material {
                            Some((name, _)) => format!("{name} does not conduct"),
                            None => "No material chosen".into(),
                        },
                        p.feed,
                        p.distance,
                        crate::post::exact_mm((top + p.plate.unwrap_or_default()) as f32),
                        crate::post::exact_mm(top as f32),
                        p.retract
                    ),
                    _ => String::new(),
                },
                fit.chars(80)
            )
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    );
    // The setup's own work zero, from the spot picked.
    let spots = c.probe_spots();
    let setup_probe: Element<_> = match c.edges_setup() {
        Err(why) => hint("Setup work zero", why, fit.chars(90)),
        Ok((refs, p)) => {
            let blocked = c.edges_blocked();
            let name = crate::control::probe_name(refs);
            row![
                small(
                    name,
                    blocked.is_none().then_some(Message::Machine(Action::ProbeEdges)),
                    c.edging()
                ),
                hint(
                    name,
                    match (own_why(blocked.clone()), &c.probe) {
                        (Some(why), _) if !c.edging() => why,
                        (_, Ok(pr)) => {
                            let own = crate::post::references(&p.origin) == Some(refs)
                                && crate::post::stock_zero(&p.origin).is_some();
                            format!(
                                "Jog the probe above the stock top, over {}, and press {name}: it touches off {}, allowing for the {} mm tip, at {} mm/min, and sets the X, Y and Z work zero on {}{}.",
                                crate::control::probe_spot(refs),
                                crate::post::probe_spots(refs, p.stock),
                                pr.tip.unwrap_or_default(),
                                pr.feed,
                                p.origin,
                                if own {
                                    String::new()
                                } else {
                                    let size = p.stock.dimensions();
                                    format!(
                                        ", placed from there by the setup's {} × {} × {} mm stock: measure the stock first",
                                        crate::post::exact_mm(size[0]),
                                        crate::post::exact_mm(size[1]),
                                        crate::post::exact_mm(size[2])
                                    )
                                }
                            )
                        }
                        _ => String::new(),
                    },
                    fit.chars(70)
                )
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        }
    };
    let mut setup_col = column![bold("Setup work zero, from its stock", 13)].spacing(6);
    if !spots.is_empty() {
        setup_col = setup_col.push(probe_from_grid(c, &spots));
    }
    col = col.push(setup_col.push(setup_probe));
    // Features found where the tip is, without a setup.
    let feature = |f: Feature| {
        small(
            f.label(),
            c.feature_blocked(f)
                .is_none()
                .then_some(Message::Machine(Action::ProbeFeature(f))),
            c.feature() == Some(f),
        )
    };
    let corners = column![
        row![feature(Feature::CORNERS[0]), feature(Feature::CORNERS[1])].spacing(4),
        row![feature(Feature::CORNERS[2]), feature(Feature::CORNERS[3])].spacing(4),
    ]
    .spacing(4);
    let mut edges = row![].spacing(4);
    for f in Feature::EDGES {
        edges = edges.push(feature(f));
    }
    let features = row![
        column![muted("Surface"), feature(Feature::Top)].spacing(4),
        column![muted("Edge, toward"), edges].spacing(4),
        column![muted("Outside corner"), corners].spacing(4),
        column![
            muted("Inside / outside"),
            row![feature(Feature::Bore), feature(Feature::Boss)].spacing(4)
        ]
        .spacing(4),
    ]
    .spacing(14);
    // Narrow, the corners and the bore wrap under the edges.
    let features: Element<_> = if fit.narrow {
        features.wrap().vertical_spacing(8).into()
    } else {
        features.into()
    };
    let why = own_why(c.feature_blocked(Feature::Bore)).filter(|_| !c.probing());
    col = col.push(
        column![
            bold("Feature at the tip, no setup needed", 13),
            features,
            hint(
                "Feature at the tip",
                match (why, c.feature()) {
                    (_, Some(f)) => f.how(),
                    (Some(why), None) => why,
                    (None, None) => "Put the tip as each button's card says and press it: the surface, side, corner or centre found becomes work zero in the active work system, on the axes it sets.".into(),
                },
                fit.chars(110)
            ),
        ]
        .spacing(6),
    );
    // The test bar on A stands where the block would (Rotary A).
    if let Some((lo, hi)) = c.block.filter(|_| c.sim_bar().is_none()) {
        col = col.push(hint(
            "Simulated",
            format!(
                "Simulated: a block at machine X {:.0}…{:.0}, Y {:.0}…{:.0}, its top at Z {:.3}, stands for the stock and trips the probe.",
                lo[0], hi[0], lo[1], hi[1], hi[2],
            ),
            fit.chars(110),
        ));
    }
    tab_scroll(col)
}

/// The MDI tab: one G-code line to run.
fn mdi_tab(c: &crate::control::Connection, fit: Fit) -> Element<'_, Message> {
    use crate::control::Action;
    let send =
        (c.manual() && !c.mdi.trim().is_empty()).then_some(Message::Machine(Action::MdiSend));
    let mut col = column![
        row![
            help::field(
                text_input("One G-code line, for example G0 X10 Y10", &c.mdi)
                    .on_input(|v| Message::Machine(Action::MdiText(v)))
                    .on_submit_maybe(send.clone())
                    .padding([7, 9])
                    .size(13),
                "mdi_line",
                "MDI line",
                Target::Machine
            ),
            act("Send", send, false)
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    ]
    .spacing(10)
    .height(Fill);
    if !fit.short {
        col = col.push(muted(
            "Runs the line when Idle, or while a program is paused. Enter sends it too.",
        ));
    }
    col.push(bold("Log", 13)).push(machine_log(c)).into()
}

/// The Machine tab: the machine and its connection, the firmware, the soft
/// limits, the machine zero and the fixture pins, beside the log.
fn machine_tab<'a>(app: &'a App, c: &'a crate::control::Connection) -> Element<'a, Message> {
    use crate::control::Action;
    let s = &c.snapshot;
    let mut col = connect_controls(app);
    if let Some(id) = &s.identity {
        let v = |o: &Option<String>| o.clone().unwrap_or_else(|| "?".into());
        col = col.push(muted(format!(
            "UC100 firmware {} · hardware {} · bootloader {}",
            v(&id.firmware),
            v(&id.hardware_id),
            v(&id.bootloader)
        )));
    }
    let (unlimited, unset) = (c.unlimited_axes(), c.unset_limited_axes());
    col = col.push(
        if let Some((heading, lines)) = limits_off_warning(&unlimited, &unset, c.home_switches()) {
            caution(heading, lines)
        } else {
            let range = |a: usize| {
                let x = c.config.axes.get(a)?;
                Some(format!("{} {}…{}", AXES[a], x.soft_min?, x.soft_max?))
            };
            let ranges: Vec<String> = (0..3).filter_map(range).collect();
            // A rotary axis's range is in degrees, after the mm.
            let angles: String = (3..6)
                .filter_map(range)
                .map(|r| format!(", {r}°"))
                .collect();
            muted(format!("Soft limits on: {} mm{angles}.", ranges.join(", ")))
                .color(GREEN)
                .into()
        },
    );
    let idle = |a: Action| c.idle().then_some(Message::Machine(a));
    let enabled: Vec<usize> = (0..6).filter(|&a| s.axes_enabled[a]).collect();
    // A declared position references only an axis without a home switch.
    let declared: Vec<usize> = enabled
        .iter()
        .copied()
        .filter(|&a| c.switchless(a))
        .collect();
    let mut zero = column![bold("Machine zero", 13)].spacing(8);
    if !declared.is_empty() {
        let mut here = row![small(
            "Set machine zero here",
            idle(Action::MachineZero(None)),
            false
        )]
        .spacing(6)
        .align_y(Alignment::Center)
        .push(muted("or one axis"));
        for &a in &declared {
            here = here.push(small(AXES[a], idle(Action::MachineZero(Some(a))), false));
        }
        zero = zero.push(here).push(small(
            "Keep the last position",
            idle(Action::KeepPosition),
            false,
        ));
    }
    if c.home_switches() {
        zero = zero.push(small("Home", idle(Action::Home), false));
    }
    zero = zero.push(muted(if declared.is_empty() {
        HOME_NOTE
    } else {
        MACHINE_ZERO_NOTE
    }));
    if let Some(pins) = pin_block(c) {
        zero = zero.push(pins);
    }
    col = col.push(rule::horizontal(1)).push(zero);
    if c.limit_inputs() {
        col = col.push(
            checkbox(s.limits_override)
                .label("Override limit switches")
                .on_toggle(|b| Message::Machine(Action::OverrideLimits(b)))
                .text_size(12),
        );
        col = col.push(muted("To drive off a limit switch that tripped. It turns itself off a second after no switch is active."));
    }
    // Narrow, the log is the MDI tab's alone.
    if Fit::of(app).narrow {
        return tab_scroll(col);
    }
    row![
        tab_scroll(col),
        column![bold("Log", 13), machine_log(c)]
            .spacing(8)
            .width(Fill)
            .height(Fill)
    ]
    .spacing(14)
    .height(Fill)
    .into()
}

/// The icon of an operation's row: done, being cut, next, to come, skipped.
fn stage_icon<'a>(stage: crate::run::Stage) -> Element<'a, Message> {
    use crate::run::Stage;
    let (glyph, fill, edge) = match stage {
        Stage::Done => ("✓", Some(Color::from_rgb8(47, 138, 87)), None),
        Stage::Cutting => ("▶", Some(RUN_BLUE), None),
        Stage::Next => ("", None, Some(RUN_BLUE)),
        Stage::ToCome => ("", None, Some(BUTTON_EDGE)),
        Stage::Skipped => ("–", Some(BUTTON_EDGE), None),
    };
    container(text(glyph).size(10).color(Color::WHITE))
        .center_x(18)
        .center_y(18)
        .style(move |_| container::Style {
            background: fill.map(Into::into),
            border: Border {
                color: edge.unwrap_or(Color::TRANSPARENT),
                width: if edge.is_some() { 2. } else { 0. },
                radius: 9.into(),
            },
            ..Default::default()
        })
        .into()
}
/// One operation of the setup in the Operations list, the `n`th it cuts: its
/// stage, name and time, the part of it cut while it is cut, and `ends`
/// after the time: Start here, which makes it the one Cycle Start begins
/// with, and ⋯, its skips; or for one the run skips (`skipped`), Don't skip.
/// `open`, its skips, takes the place of its time and ends, its name kept,
/// cut short. With steps (LIO-275), ▸ or ▾ before the name unfolds or folds
/// them (`toggle`: whether they are unfolded, and the message), and `note`
/// after it says how many. The picture of the stock it leaves, its strategy
/// and tool are on hover.
#[allow(clippy::too_many_arguments)]
fn op_row<'a>(
    n: usize,
    o: &'a crate::post::OpState,
    picture: Option<&widget::image::Handle>,
    stage: crate::run::Stage,
    time: Option<String>,
    ends: Vec<Element<'a, Message>>,
    open: Option<Element<'a, Message>>,
    angles: Option<String>,
    cut: Option<f32>,
    skipped: Option<crate::run::Skip>,
    steps: Option<(bool, Message, String)>,
    fit: Fit,
) -> Element<'a, Message> {
    use crate::run::{Skip, Stage};
    let (w, h) = OP_PICTURE;
    let picture: Option<Element<'a, Message>> = match picture {
        Some(p) => Some(widget::image(p.clone()).width(w).height(h).into()),
        // Material left is not simulated: there is no picture to wait for.
        None if skipped == Some(Skip::Left) => None,
        None => Some(
            container(text("Picturing the stock…").size(11))
                .center_x(w)
                .center_y(h)
                .into(),
        ),
    };
    let said = match (skipped, stage) {
        (Some(Skip::Cut), _) => "Skipped: already cut, its stock taken as removed",
        (Some(Skip::Left), _) => "Skipped: material left, the setup checked without it",
        (_, Stage::Done) => "Done",
        (_, Stage::Cutting) => "Cutting now",
        (_, Stage::Next) => "Next: Cycle Start cuts it",
        (_, Stage::Skipped) => "Skipped: taken as cut",
        (_, Stage::ToCome) => "To come",
    };
    let mut tip = column![
        column![].push(picture),
        text(format!("{n}. {}", o.name)).size(12),
        text(format!(
            "{} · {}",
            o.strategy,
            crate::model::tool_label(o.tool, &o.tool_name)
        ))
        .size(11),
        text(said).size(11),
    ]
    .spacing(3)
    .width(w);
    if let Some(angles) = &angles {
        tip = tip.push(text(angles.clone()).size(11));
    }
    let strong = matches!(stage, Stage::Cutting | Stage::Next) && skipped.is_none();
    let name = text(format!("{n}. {}", ellipsize(&o.name, 30)))
        .size(13)
        .color(if stage == Stage::Skipped || skipped.is_some() {
            MUTED
        } else {
            INK
        })
        .font(Font {
            weight: if strong {
                iced::font::Weight::Semibold
            } else {
                iced::font::Weight::Normal
            },
            ..Font::DEFAULT
        })
        .wrapping(Wrapping::None);
    // A skipped one says how in place of its time.
    let time: Element<'a, Message> = match skipped {
        Some(how) => container(
            text(match how {
                Skip::Cut => "skipped: already cut",
                Skip::Left => "skipped: material left",
            })
            .size(10)
            .color(MUTED)
            .wrapping(Wrapping::None),
        )
        .padding([1, 4])
        .style(|_| container::Style {
            background: Some(Color::from_rgb8(237, 240, 244).into()),
            border: Border {
                radius: 3.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into(),
        None => text(time.unwrap_or_default())
            .size(11)
            .color(MUTED)
            .width(if fit.narrow { 84 } else { 96 })
            .align_x(Alignment::End)
            .wrapping(Wrapping::None)
            .into(),
    };
    let mut named = row![].spacing(4).align_y(Alignment::Center);
    if let Some((unfolded, m, note)) = steps {
        let toggle = button(text(if unfolded { "▾" } else { "▸" }).size(11).color(MUTED))
            .padding([0, 2])
            .on_press(m.clone())
            .style(|_, _| button::Style::default());
        named = named
            .push(HelpButton::new(toggle, &m, note.clone()))
            .push(name)
            .push(text(note).size(10).color(MUTED).wrapping(Wrapping::None));
    } else {
        named = named.push(name);
    }
    let named = container(named).width(Fill).clip(true);
    let line = match open {
        Some(open) => row![stage_icon(stage), named, open],
        None => row![stage_icon(stage), named, time].extend(ends),
    }
    .spacing(if fit.narrow { 6 } else { 8 })
    .align_y(Alignment::Center)
    .height(if fit.short { 26 } else { 28 });
    let mut body = column![line].spacing(3);
    if let Some(cut) = cut {
        body = body.push(widget::progress_bar(0.0..=1.0, cut).girth(5));
    }
    let row = container(body).padding([2, 6]).style(move |_| {
        let (background, border) = match stage {
            Stage::Cutting => (
                Some(Color::from_rgb8(238, 244, 254)),
                Color::from_rgb8(188, 210, 245),
            ),
            Stage::Next => (
                Some(Color::from_rgb8(241, 250, 244)),
                Color::from_rgb8(191, 227, 204),
            ),
            _ => (None, Color::TRANSPARENT),
        };
        container::Style {
            background: background.map(Into::into),
            border: Border {
                color: border,
                width: 1.,
                radius: 6.into(),
            },
            text_color: Some(INK),
            ..Default::default()
        }
    });
    widget::tooltip(
        row,
        container(tip).padding(8).style(|_| container::Style {
            background: Some(Color::from_rgb8(27, 44, 61).into()),
            text_color: Some(Color::from_rgb8(236, 242, 249)),
            border: Border {
                radius: 7.into(),
                ..Default::default()
            },
            ..Default::default()
        }),
        widget::tooltip::Position::Left,
    )
    .gap(8)
    .into()
}
/// The size the Operations list's hover shows its picture of the stock at.
const OP_PICTURE: (f32, f32) = (150., 112.5);

/// What the step list of an operation in the Operations list shows with:
/// the operation's stage and estimated seconds, the setup's work zero (plan
/// Z), whether steps can be picked, and how many lines it may take.
struct StepsShown {
    stage: crate::run::Stage,
    op_time: Option<f32>,
    datum: f32,
    may: bool,
    lines: usize,
}

/// How many lines an operation's step list may take so that the Operations
/// list does not scroll (LIO-275): what the card leaves under its head, the
/// tool runs' headings and operations' rows (`listed`), the picked steps'
/// buttons and room for the checks under the list. Three at the least.
fn step_budget(app: &App, (runs, ops): (usize, usize), picked: bool) -> usize {
    let fit = Fit::of(app);
    let card = app.window_height
        - fit.machine_bar()
        - fit.run_bar()
        - 26.
        - 2. * fit.gap()
        - 2. * fit.pad()
        - 30.;
    let row = if fit.short { 32. } else { 34. };
    let used = ops as f32 * row + runs as f32 * 26. + if picked { 64. } else { 0. } + 96.;
    (((card - used) / STEP_LINE).floor() as usize).clamp(3, 16)
}

/// A step list line's height with its spacing.
const STEP_LINE: f32 = 22.;

/// Which of `n` steps get a line of their own when `max` lines are all
/// there is room for (LIO-275): those in `keep`, the first first, while
/// they fit, then their neighbours; each run of the others is one folded
/// line. Each line is the steps it shows.
fn step_lines(n: usize, keep: &[usize], max: usize) -> Vec<std::ops::RangeInclusive<usize>> {
    let lines = |shown: &[bool]| {
        (0..shown.len())
            .filter(|&i| shown[i] || i == 0 || shown[i - 1])
            .count()
    };
    let mut shown = vec![n <= max; n];
    for &k in keep.iter().filter(|&&k| k < n) {
        if !shown[k] {
            shown[k] = true;
            if lines(&shown) > max {
                shown[k] = false;
            }
        }
    }
    let mut grew = true;
    while grew {
        grew = false;
        for i in 0..n {
            if !shown[i] && (i > 0 && shown[i - 1] || i + 1 < n && shown[i + 1]) {
                shown[i] = true;
                if lines(&shown) > max {
                    shown[i] = false;
                } else {
                    grew = true;
                }
            }
        }
    }
    let mut out = vec![];
    let mut i = 0;
    while i < n {
        let a = i;
        i += 1;
        while !shown[a] && i < n && !shown[i] {
            i += 1;
        }
        out.push(a..=i - 1);
    }
    out
}

/// The steps of operation `os` under its row (LIO-275): a line each, its
/// mark, label, Z and how it stands, and its estimated time; long runs of
/// them folded into one line (`step_lines`). A click picks a step, a
/// Shift-click the steps up to it; under the steps picked, Skip: already
/// cut (or Don't skip), Start at the first and Clear.
fn step_list<'a>(
    run: &'a crate::run::Run,
    os: &'a crate::run::OpSteps,
    shown: StepsShown,
    line: Option<u32>,
    fit: Fit,
) -> Element<'a, Message> {
    use crate::run::{Action as R, StepStage as S, clock};
    let oi = os.op;
    let stages = run.step_stages(oi, shown.stage, line);
    // The plan's seconds scaled to the operation's estimate over the steps
    // it sends.
    let sent: f32 = (os.steps.iter().zip(&stages))
        .filter(|(_, s)| !matches!(s, S::Cut | S::Before))
        .map(|(s, _)| s.seconds)
        .sum();
    let scale = shown.op_time.filter(|_| sent > 0.).map_or(1., |t| t / sent);
    let seconds = |r: &std::ops::RangeInclusive<usize>| -> f32 {
        os.steps[r.clone()].iter().map(|s| s.seconds * scale).sum()
    };
    let z = |s: usize| {
        os.steps[s]
            .z
            .map(|z| format!("Z {:.2}", z - shown.datum).replace('-', "−"))
    };
    let picked = run.picked.filter(|p| p.0 == oi).map(|p| p.1..=p.2);
    let is_picked = |s: usize| picked.as_ref().is_some_and(|p| p.contains(&s));
    let mut keep: Vec<usize> = vec![];
    keep.extend(
        picked
            .clone()
            .into_iter()
            .flat_map(|p| [*p.start(), *p.end()]),
    );
    keep.extend((0..stages.len()).filter(|&s| matches!(stages[s], S::Next | S::Cutting)));
    // Where done meets the rest.
    if let Some(d) = (0..stages.len()).rposition(|s| stages[s] == S::Done) {
        keep.extend([d, d + 1]);
    }
    keep.extend(picked.clone().into_iter().flatten());
    let said = |s: S| match s {
        S::Done => "done",
        S::Cutting => "cutting",
        S::Next => "starts here",
        S::Cut => "skipped",
        S::Before => "taken as cut",
        S::ToCome => "",
    };
    let mut col = column![].spacing(2);
    let lh = STEP_LINE - 2.;
    for r in step_lines(os.steps.len(), &keep, shown.lines) {
        let first = *r.start();
        let one = r.start() == r.end();
        let stage = stages[first];
        let uniform = r.clone().all(|s| stages[s] == stage);
        let detail = if one {
            let parts: Vec<String> = z(first)
                .into_iter()
                .chain(Some(said(stage).to_string()).filter(|s| !s.is_empty()))
                .collect();
            parts.join(" · ")
        } else if uniform && stage != S::ToCome {
            said(stage).to_string()
        } else {
            match (z(first), z(*r.end())) {
                (Some(a), Some(b)) => format!("{a} … {}", b.trim_start_matches("Z ")),
                _ => String::new(),
            }
        };
        let (glyph, fill, edge) = match stage {
            _ if one && is_picked(first) => ("✓", Some(RUN_BLUE), RUN_BLUE),
            S::Done => ("✓", Some(Color::from_rgb8(47, 138, 87)), Color::TRANSPARENT),
            S::Cutting | S::Next => ("▶", Some(RUN_BLUE), RUN_BLUE),
            S::Cut | S::Before => ("–", Some(BUTTON_EDGE), Color::TRANSPARENT),
            S::ToCome => ("", None, BUTTON_EDGE),
        };
        let mark: Element<_> = if one {
            container(text(glyph).size(9).color(Color::WHITE))
                .center_x(13)
                .center_y(13)
                .style(move |_| container::Style {
                    background: fill.map(Into::into),
                    border: Border {
                        color: edge,
                        width: 1.5,
                        radius: 3.into(),
                    },
                    ..Default::default()
                })
                .into()
        } else {
            space().width(13).into()
        };
        let label = if one {
            os.label(first)
        } else {
            os.labels(r.clone())
        };
        let font = Font {
            style: if one {
                iced::font::Style::Normal
            } else {
                iced::font::Style::Italic
            },
            weight: if one && stage == S::Next {
                iced::font::Weight::Semibold
            } else {
                iced::font::Weight::Normal
            },
            ..Font::DEFAULT
        };
        let ink = if one { INK } else { MUTED };
        let face = row![
            mark,
            text(label)
                .size(11)
                .font(font)
                .color(ink)
                .width(if fit.narrow { 74 } else { 84 })
                .wrapping(Wrapping::None),
            container(
                text(detail)
                    .size(11)
                    .color(MUTED)
                    .font(font)
                    .wrapping(Wrapping::None)
            )
            .width(Fill)
            .clip(true),
            text(clock(seconds(&r)))
                .size(11)
                .color(MUTED)
                .wrapping(Wrapping::None),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        let lit = one && is_picked(first);
        let m = Message::Run(R::PickStep(oi, first));
        let b = button(face)
            .height(lh)
            .width(Fill)
            .padding([0, 6])
            .on_press_maybe(shown.may.then(|| m.clone()))
            .style(move |_, status| button::Style {
                background: Some(
                    match status {
                        _ if lit => Color::from_rgb8(227, 237, 253),
                        button::Status::Hovered if !lit => HOVER_BUTTON,
                        _ => Color::TRANSPARENT,
                    }
                    .into(),
                ),
                text_color: INK,
                border: Border {
                    radius: 4.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        // Its card to the left, as the operations' pictures: below, it
        // would cover the steps after it and take their clicks (LIO-312).
        col = col.push(help::wrap(
            b,
            help::command(&m, os.label(first)),
            widget::tooltip::Position::Left,
        ));
    }
    let mut out = column![
        container(col)
            .padding(iced::Padding {
                left: 6.,
                ..Default::default()
            })
            .style(|_| container::Style {
                border: Border {
                    color: LINE,
                    width: 0.,
                    radius: 0.into(),
                },
                ..Default::default()
            })
    ]
    .spacing(4)
    .padding(iced::Padding {
        left: if fit.narrow { 22. } else { 28. },
        right: 4.,
        bottom: 4.,
        ..Default::default()
    });
    // The steps picked: their name and time, and what can be done to them.
    if let Some(p) = picked {
        let all_cut = p.clone().all(|s| stages[s] == S::Cut);
        let done = shown.stage == crate::run::Stage::Done;
        let first = *p.start();
        let skip = if all_cut {
            small(
                "Don't skip",
                shown.may.then_some(Message::Run(R::SkipSteps(false))),
                false,
            )
        } else {
            // Not every step: then it is the operation that is skipped.
            let left = (0..stages.len())
                .any(|s| !p.contains(&s) && !matches!(stages[s], S::Cut | S::Before));
            small(
                "Skip: already cut",
                (shown.may && left && !done).then_some(Message::Run(R::SkipSteps(true))),
                false,
            )
        };
        let start = small(
            format!("Start at {}", os.label(first).to_lowercase()),
            (shown.may && !matches!(stages[first], S::Cut | S::Next))
                .then_some(Message::Run(R::StartStep)),
            false,
        );
        let mut head = row![
            bold(os.labels(p.clone()), 12),
            muted(format!("· {}", clock(seconds(&p)))).size(11),
            space().width(Fill),
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        if !fit.narrow {
            head = head.push(muted("Shift-click picks a range").size(10));
        }
        out = out.push(
            container(
                column![
                    head,
                    row![
                        skip,
                        start,
                        small("Clear", Some(Message::Run(R::ClearSteps)), false)
                    ]
                    .spacing(6)
                ]
                .spacing(5),
            )
            .padding([6, 8])
            .width(Fill)
            .style(|_| container::Style {
                background: Some(Color::from_rgb8(247, 249, 252).into()),
                border: Border {
                    color: LINE,
                    width: 1.,
                    radius: 6.into(),
                },
                ..Default::default()
            }),
        );
    }
    out.into()
}

/// The right column of the Control page: the setup's operations in the
/// order they are cut, grouped by tool run, where each stands, and under
/// them what holds Cycle Start back, in full.
pub(crate) fn program<'a>(app: &'a App, c: &'a crate::control::Connection) -> Element<'a, Message> {
    use crate::run::Action as R;
    use uc100::controller::State;
    let s = &c.snapshot;
    let fit = Fit::of(app);
    let msg = |on: bool, a: R| on.then_some(Message::Run(a));
    let mut head = row![bold("Operations", 14), space().width(Fill)].align_y(Alignment::Center);
    let mut list = column![].spacing(2);
    let Some(run) = &c.run else {
        return card(
            column![
                head,
                muted("Pick a setup in the run bar: its operations are listed here.")
            ]
            .spacing(10),
            None,
            fit,
        );
    };
    let running = run.running.is_some();
    let state = s.state.as_ref();
    match run.programs.as_deref() {
        None => {
            list = list.push(muted(if run.stale {
                crate::run::CHECK_WAITS
            } else {
                "Checking the setup…"
            }))
        }
        Some(Err(problems)) => {
            list = list
                .push(caution("This setup cannot run", problems.clone()))
                .push(act("Check again", msg(true, R::Check), false));
            // A skip may be why: each can be undone here.
            for &(oi, _) in &run.skips {
                let name = (app.doc.setups.get(run.setup))
                    .and_then(|s| s.operations.get(oi))
                    .map_or("", |o| o.name());
                list = list.push(act(
                    format!("Don't skip {name}"),
                    msg(c.manual() && !running, R::Skip(oi, None)),
                    false,
                ));
            }
        }
        Some(Ok(programs)) => {
            let count = programs.len();
            let ops: usize = programs.iter().map(|p| p.ops.len()).sum();
            let tools = format!("{} · {}", plural(ops, "op"), plural(count, "tool run"));
            head = head.push(muted(match run.setup_time() {
                Some(t) => format!("{} · {tools}", crate::run::clock(t)),
                None => tools,
            }));
            let left = run.left(s);
            // Where A is, on a machine with A, and while an index move turns it.
            let with_a = app
                .doc
                .setups
                .get(run.setup)
                .is_some_and(|x| x.machine.has_a())
                && c.axis(3);
            if with_a {
                let turning = run
                    .progress(s.current_line)
                    .and_then(|(_, _, b)| match run.running_blocks()?.get(b)? {
                        crate::post::Block::Index { a, .. } => Some(*a),
                        _ => None,
                    })
                    .filter(|_| running);
                let mut line = if s.referenced[3] {
                    format!(
                        "A now: {:.3}° in G{} (machine A {:.3}°)",
                        s.work_pos[3],
                        54 + s.active_wcs,
                        s.machine_pos[3]
                    )
                } else {
                    "A now: no machine position".to_string()
                };
                if let Some(a) = turning {
                    line += &format!(
                        " · index move: turning A alone to {}°, X Y Z still",
                        crate::indexed::degrees(a)
                    );
                }
                list = list.push(one_line("A", line, 12, INK, 60));
            }
            // The rows are the tool runs as checked without a skip: a skipped
            // operation keeps its place, and a tool run of skipped ones its
            // heading. Each other is found in the programs sent.
            let rows = (run.layout.as_deref())
                .and_then(|l| l.as_ref().ok())
                .map_or(programs.as_slice(), Vec::as_slice);
            let skipped = |oi: usize| run.skips.iter().find(|s| s.0 == oi).map(|s| s.1);
            // Each row's place in the programs: they send the rows not
            // skipped, in order.
            let mut sent = (programs.iter().enumerate())
                .flat_map(|(i, p)| (0..p.ops.len()).map(move |k| (i, k)));
            let places: Vec<Vec<Option<(usize, usize)>>> = (rows.iter())
                .map(|p| {
                    (p.indexes.iter())
                        .map(|&oi| match skipped(oi) {
                            Some(_) => None,
                            None => sent.next(),
                        })
                        .collect()
                })
                .collect();
            let stages: Vec<_> = (0..count).map(|i| run.stages(i, s.current_line)).collect();
            let times: Vec<_> = (0..count).map(|i| run.op_times(i)).collect();
            let firsts: Vec<_> = (programs.iter().enumerate())
                .map(|(i, p)| run.times.as_ref().and_then(|t| t.ops(i, p)))
                .collect();
            // Skips are offered only while the machine stands in a manual
            // state, nothing running or probing.
            let may_skip = c.manual() && !running && !c.probing();
            // The tool runs and operations listed, for the steps' room.
            let listed = (rows.len(), rows.iter().map(|p| p.ops.len()).sum::<usize>());
            // Operations are numbered through the setup, across its tool runs.
            let mut number = 0;
            for (w, p) in rows.iter().enumerate() {
                let sent: Vec<(usize, usize)> = places[w].iter().flatten().copied().collect();
                // A tool run's time is its operations' cut from its first.
                let time = if sent.is_empty() {
                    "skipped".to_string()
                } else {
                    (sent.iter())
                        .map(|&(i, k)| Some(*firsts[i].as_ref()?.get(k)?))
                        .sum::<Option<f32>>()
                        .map(crate::run::clock)
                        .unwrap_or_default()
                };
                list = list.push(
                    container(
                        row![
                            text(format!(
                                "TOOL RUN {} · {}",
                                w + 1,
                                crate::model::tool_label(p.ops[0].tool, &p.ops[0].tool_name)
                            ))
                            .size(11)
                            .color(MUTED)
                            .width(Fill)
                            .wrapping(Wrapping::None),
                            muted(time).size(11),
                        ]
                        .spacing(6),
                    )
                    .padding(iced::Padding {
                        top: 8.,
                        bottom: 2.,
                        left: 6.,
                        right: 6.,
                    }),
                );
                for ((o, &oi), place) in p.ops.iter().zip(&p.indexes).zip(&places[w]) {
                    number += 1;
                    use crate::run::Stage;
                    let start_width = Some(Length::Fixed(74.));
                    let more_width = Length::Fixed(22.);
                    if let Some(how) = skipped(oi) {
                        list = list.push(op_row(
                            number,
                            o,
                            None,
                            Stage::Skipped,
                            None,
                            vec![
                                act_with(
                                    "Don't skip",
                                    msg(may_skip, R::Skip(oi, None)),
                                    false,
                                    start_width,
                                    [3, 4],
                                ),
                                space().width(more_width).into(),
                            ],
                            None,
                            None,
                            None,
                            Some(how),
                            None,
                            fit,
                        ));
                        continue;
                    }
                    let Some((i, k)) = *place else {
                        continue;
                    };
                    let p = &programs[i];
                    let stage = stages[i].get(k).copied().unwrap_or(Stage::ToCome);
                    let left = left.filter(|_| run.running.as_ref().is_some_and(|r| r.index == i));
                    let start = (!matches!(stage, Stage::Next | Stage::Cutting) && !c.probing())
                        .then(|| msg(!running, if k == 0 { R::Next(i) } else { R::From(i, k) }));
                    // An operation not cut yet in this run can be skipped.
                    let skippable = may_skip && matches!(stage, Stage::Next | Stage::ToCome);
                    let open = run.menu == Some(oi) && skippable;
                    let mut ends = vec![
                        match start {
                            Some(m) => act_with("Start here", m, false, start_width, [3, 4]),
                            None => space().width(74).into(),
                        },
                        if skippable {
                            act_with(
                                "⋯",
                                msg(true, R::More(Some(oi))),
                                false,
                                Some(more_width),
                                [3, 0],
                            )
                        } else {
                            space().width(more_width).into()
                        },
                    ];
                    let open = open.then(|| {
                        ends.clear();
                        let leave = act_with(
                            "Leave the material",
                            (!run.beneath.iter().any(|b| b.0 == oi))
                                .then_some(Message::Run(R::Skip(oi, Some(crate::run::Skip::Left)))),
                            false,
                            None,
                            [3, 6],
                        );
                        let leave = match run.beneath.iter().find(|b| b.0 == oi) {
                            Some((_, why)) => help::wrap(
                                leave,
                                Help::text("Leave the material", why.clone()),
                                widget::tooltip::Position::Top,
                            ),
                            None => leave,
                        };
                        // Its name stays, cut short: narrow, without "Skip:".
                        row![]
                            .push((!fit.narrow).then(|| muted("Skip:")))
                            .push(act_with(
                                "Already cut",
                                msg(true, R::Skip(oi, Some(crate::run::Skip::Cut))),
                                false,
                                None,
                                [3, 6],
                            ))
                            .push(leave)
                            .push(act_with(
                                "✕",
                                msg(true, R::More(None)),
                                false,
                                Some(more_width),
                                [3, 0],
                            ))
                            .spacing(if fit.narrow { 4 } else { 6 })
                            .align_y(Alignment::Center)
                            .into()
                    });
                    let (time, cut) = match (left.and_then(|l| l.op), &times[i]) {
                        (Some((op, op_left, op_all)), _) if op == k => (
                            Some(format!(
                                "{} left of {}",
                                crate::run::clock(op_left),
                                crate::run::clock(op_all)
                            )),
                            Some(if op_all > 0. {
                                1. - op_left / op_all
                            } else {
                                0.
                            }),
                        ),
                        (_, Some(t)) => (t.get(k).map(|&t| crate::run::clock(t)), None),
                        _ => (None, None),
                    };
                    // The setup and the A angles the operation is cut at.
                    let angles = crate::run::op_angles(&p.blocks, k);
                    let at = with_a.then(|| {
                        let a = if angles.is_empty() {
                            "A 0°".to_string()
                        } else {
                            crate::indexed::angles_text(&angles)
                        };
                        format!("Setup {} · {a}", run.setup + 1)
                    });
                    // Its steps (LIO-275), unfolded under it on ▾.
                    let steps = run.op_steps(oi).filter(|s| s.steps.len() > 1);
                    let unfolded = steps.is_some() && run.open == Some(oi);
                    let toggle = steps.map(|s| {
                        let n = s.steps.len();
                        let what = s.kind().1.to_lowercase();
                        let m = Message::Run(R::Steps((!unfolded).then_some(oi)));
                        (unfolded, m, format!("{n} {what}"))
                    });
                    let op_time = times[i].as_ref().and_then(|t| t.get(k).copied());
                    list = list.push(op_row(
                        number,
                        o,
                        run.picture(oi),
                        stage,
                        time,
                        ends,
                        open,
                        at,
                        cut,
                        None,
                        toggle,
                        fit,
                    ));
                    if let Some(os) = steps.filter(|_| unfolded) {
                        let shown = StepsShown {
                            stage,
                            op_time,
                            datum: p.datum[2],
                            may: may_skip,
                            lines: step_budget(app, listed, run.picked.is_some_and(|p| p.0 == oi)),
                        };
                        list = list.push(step_list(run, os, shown, s.current_line, fit));
                    }
                }
            }
        }
    }
    // What holds Cycle Start back, in full: the run bar says the first.
    let mut checks = column![].spacing(8);
    if run.stale && run.programs.is_some() {
        checks = checks
            .push(caution(
                "The project changed since this setup was checked",
                vec!["Check it again before the next tool run.".into()],
            ))
            .push(act("Check again", msg(!running, R::Check), false));
    }
    if run.checked().is_some() && !crate::run::busy(state) {
        let mut problems = run.problems(c, &app.doc);
        problems.extend(app.planning_busy().map(String::from));
        if !problems.is_empty() {
            checks = checks.push(caution("Before Cycle Start", problems));
            let off = run.coolant_off(c, &app.doc);
            if !off.is_empty() {
                let ops = &app.doc.setups[run.setup].operations;
                let names: Vec<&str> = off.iter().map(|&i| ops[i].name()).collect();
                checks = checks.push(act(
                    format!("Set coolant to Off: {}", names.join(", ")),
                    msg(!running, R::CoolantOff),
                    false,
                ));
            }
        }
        if let Some(line) = crate::run::limits_unchecked(c) {
            checks = checks.push(caution("Soft limits are off for Cycle Start", vec![line]));
        }
    }
    if let Some(State::Paused(reason)) = state {
        checks = checks.push(note(format!("Paused: {reason}")));
    }
    if let Some(n) = &run.journal_note {
        checks = checks.push(note(n.clone()));
    }
    if let Some((ok, message)) = &run.outcome {
        checks = checks.push(
            text(message.clone())
                .size(12)
                .color(if *ok { GREEN } else { AMBER }),
        );
    }
    card(
        column![
            head.height(24),
            scrollable(column![list, checks].spacing(12).padding(iced::Padding {
                right: SCROLLBAR_GUTTER,
                ..Default::default()
            }))
            .id("operations")
            .height(Fill)
        ]
        .spacing(6),
        None,
        fit,
    )
}
/// Work zero on the stock, in the Run panel until the setup's first tool run
/// starts: how to put the work zero its tool runs are cut from on the stock,
/// by hand or with the probe, and where the simulator's block is. The soft
/// limits errors point here. None without a checked setup, or once a tool
/// run has started.
pub(crate) fn work_zero_steps(app: &App, c: &crate::control::Connection) -> Option<Vec<String>> {
    let run = c.run.as_ref().filter(|r| !r.started())?;
    let p = run.checked()?.first()?;
    let setup = app.doc.setups.get(run.setup)?;
    let s = &c.snapshot;
    let mut steps = vec![format!(
        "Every tool run is cut from this setup's work zero, {} in {}. Before the first Cycle Start, put it on the stock:",
        p.origin, p.offset
    )];
    steps.push(if (0..3).all(|a| s.referenced[a]) {
        "1. X, Y and Z have a machine position.".into()
    } else if c.home_switches() {
        "1. Optional: Home in the Machine tab, so that the soft limits are on. Without it Cycle Start runs with them off.".into()
    } else {
        "1. Optional: Set machine zero in the Machine tab, so that the soft limits are on: jog X to its left end, Y to its front end and Z to the top first. Without it Cycle Start runs with them off.".into()
    });
    steps.push(if p.offset == format!("G{}", 54 + s.active_wcs) {
        format!("2. {} is the active work system.", p.offset)
    } else {
        format!("2. Select {} under Work zero.", p.offset)
    });
    let checks = fits_check(c);
    let blocked = c.partway_blocked();
    if let Some(on) = &p.pins {
        steps.truncate(1);
        steps.extend(pin_steps(c, p, on));
        if let Some(why) = &blocked {
            steps.pop();
            steps.push(format!(
                "4. {why} Set Z by hand on a surface whose height you know (Set under Work zero)."
            ));
        }
    } else if let Some(why) = &blocked {
        steps.push(format!(
            "3. {why} Zero Z on the stock top and Probe Z are off: set Z by hand on a surface whose height you know (Set under Work zero), or from a recorded reference (Set Z from reference)."
        ));
    } else if let (true, Some((_, work)), Some(refs)) = (
        run.partway(),
        c.touch_off(),
        crate::post::references(&p.origin),
    ) {
        // Z alone: X and Y are those the operations before the start were
        // cut with, which a tip jogged by eye would not find again.
        let on = if work[2] < (p.stock.max[2] - p.datum[2]) as f64 - 1e-4 {
            "the floor there"
        } else {
            "the stock top"
        };
        let spot = match crate::control::probe_spot(refs).as_str() {
            "its centre" => "the centre".to_string(),
            spot => spot.to_string(),
        };
        steps.push(format!(
            "3. Jog the tool tip over {spot} of the stock top, then down in 0.1 mm and 0.01 mm steps until it just pinches a sheet of paper on {on}."
        ));
        steps.push(format!(
            "4. Set Z{}: that point becomes Z{}. X and Y stay as they are: they must be those the operations before the start were cut with (the stock not moved since), or probed from a spot they left. {checks}",
            crate::post::exact_mm(work[2] as f32),
            crate::post::exact_mm(work[2] as f32)
        ));
    } else {
        match (c.touch_off(), crate::post::references(&p.origin)) {
            (Some((_, work)), Some(refs)) => {
                let spot = match crate::control::probe_spot(refs).as_str() {
                    "its centre" => "the centre".to_string(),
                    spot => spot.to_string(),
                };
                steps.push(format!("3. Jog the tool tip over {spot} of the stock top, then down in 0.1 mm and 0.01 mm steps until it just pinches a sheet of paper on the stock top."));
                let zero = match crate::post::stock_zero(&p.origin) {
                    None => " on Model origin".to_string(),
                    Some((_, false)) => format!(
                        " on the stock bottom, {} mm below the top",
                        crate::post::exact_mm(work[2] as f32)
                    ),
                    Some(_) => String::new(),
                };
                steps.push(format!(
                    "4. {}: that point becomes {}, putting the work zero{zero}. {checks}",
                    touch_label(work),
                    work_words(work)
                ));
            }
            _ => {
                steps.push(match work_zero_line(setup, p.located).split_once(" — ") {
                    Some((_, at)) => format!("3. Jog the tool tip to the work zero: {at}."),
                    None => "3. Jog the tool tip to the work zero.".into(),
                });
                steps.push(format!(
                    "4. Zero all: that point becomes X0 Y0 Z0. {checks}"
                ));
            }
        }
    }
    if c.config.inputs.probe.is_some() && p.pins.is_none() {
        // On stock that does not conduct the probe touches only the plate:
        // Probe Z sets Z, and X and Y are zeroed by hand.
        let probe_z = |top: f64| {
            format!(
                "{}, Probe Z in the Probe tab sets Z{}.",
                if c.direct() {
                    "With the plate's lead on the stock"
                } else {
                    "With the touch plate on the stock top"
                },
                crate::post::exact_mm(top as f32)
            )
        };
        steps.push(match (c.edges_setup(), c.stock_top()) {
            (Ok((refs, _)), _) if c.sides_blocked().is_none() => format!(
                "With the plate's lead on the stock and the clip on the tool, {} in the Probe tab does 3 and 4.",
                crate::control::probe_name(refs)
            ),
            (Ok(_), Ok(top)) => {
                format!("{} {}", c.sides_blocked().unwrap_or_default(), probe_z(top))
            }
            (Err(why), Ok(top)) => format!("{why} {}", probe_z(top)),
            (Err(why), Err(_)) | (Ok(_), Err(why)) => why,
        });
    }
    if run.partway() {
        steps.splice(1..1, partway_steps(run));
    }
    if let (Some((lo, hi)), Some(z), Some(at)) = (c.block, c.sim_zero(), c.sim_touch()) {
        let touched = if (0..3).all(|k| (at[k] - z[k]).abs() < 1e-6) {
            String::new()
        } else {
            format!(
                ", touched off at machine X {:.3} Y {:.3} Z {:.3}",
                at[0], at[1], at[2]
            )
        };
        steps.push(format!(
            "Simulated: the stock is a block at machine X {:.0}…{:.0}, Y {:.0}…{:.0}, its top at Z {:.3}. This setup's work zero on it is at machine X {:.3} Y {:.3} Z {:.3}{touched}: Move there does 3.",
            lo[0], hi[0], lo[1], hi[1], hi[2], z[0], z[1], z[2]
        ));
    }
    Some(steps)
}
/// What Work zero on the stock says first of a run that starts partway
/// (`run::Run::partway`): where it starts, which operations it takes as
/// cut and the tool to load, what they cut where the work zero is found,
/// and that Cycle Start waits for the work zero to be set for that stock or
/// kept (Keep this work zero).
fn partway_steps(run: &crate::run::Run) -> Vec<String> {
    use crate::post::Partly;
    let Some(programs) = run.checked() else {
        return vec![];
    };
    let Some(p) = programs.get(run.next) else {
        return vec![];
    };
    let o = &p.ops[run.from.min(p.ops.len() - 1)];
    let skipped: Vec<&str> = programs[..run.next]
        .iter()
        .flat_map(|p| &p.ops)
        .chain(&p.ops[..run.from])
        .map(|o| o.name.as_str())
        .collect();
    let mut steps = vec![format!(
        "This run starts at {} (tool run {}), taking {} as cut: nothing of {} is sent. Load {} and set the work zero on the stock they left.",
        o.name,
        run.next + 1,
        skipped.join(", "),
        if skipped.len() == 1 { "it" } else { "them" },
        crate::model::tool_label(o.tool, &o.tool_name)
    )];
    let datum = p.datum[2];
    // Where the check cannot tell the surface, step 3 says so.
    match run.partly().as_deref() {
        Some(partly) if partly.cut.is_empty() && partly.surface.is_some() => steps.push(
            "They did not cut the stock where the work zero is found: find it as before them."
                .into(),
        ),
        Some(Partly {
            cut,
            surface: Some(z),
            probe_from,
            ..
        }) => steps.push(format!(
            "They cut the stock's {}: the stock top's touch-off spot is now the floor they left, at Z{} (the stock top was Z{}), and the touch-off below sets Z on it.{}",
            cut.join(", "),
            crate::post::exact_mm(z - datum),
            crate::post::exact_mm(p.stock.max[2] - datum),
            if probe_from.iter().any(|(_, ok)| ok.is_ok()) {
                " Probe from only the spots they left."
            } else {
                ""
            }
        )),
        _ => {}
    }
    steps.push(if run.zeroed {
        "The work zero is set for this start.".into()
    } else {
        "Cycle Start waits until the work zero is set below after this start was chosen, or until you press Keep this work zero: when it is still the one they were cut with (the stock has not moved since), or you set Z by hand on a surface they left.".into()
    });
    steps
}
/// Work zero on the stock for a setup that sits on the fixture pins `on`:
/// X and Y referenced from pin 1, its work system, X Y set from the pins,
/// and Z touched off on the stock, each said done once it is.
fn pin_steps(
    c: &crate::control::Connection,
    p: &crate::post::Checked,
    on: &crate::post::OnPins,
) -> Vec<String> {
    let s = &c.snapshot;
    let recorded = matches!(&c.pins, Ok(Some(pins)) if pins.reference.is_some());
    let mut steps = vec![format!(
        "The part sits on the fixture pins, pin 1 at X{} Y{} in work coordinates: the pins give X and Y exactly, and Z is touched off on the stock.",
        crate::post::exact_mm(on.pin1[0]),
        crate::post::exact_mm(on.pin1[1])
    )];
    steps.push(match (recorded, c.pin_referenced, s.referenced[2]) {
        (_, true, true) => "1. X and Y are referenced from pin 1 since the last E-stop, and Z has a machine position.".into(),
        (false, ..) => "1. Under Position, set machine zero, then with the part off the pins Record the pins' place (once for the plate).".into(),
        (true, _, false) => "1. Under Position, set machine zero on Z at the top of its travel, then with the part off the pins Reference X Y from pin 1 (after every E-stop or power cycle).".into(),
        (true, false, true) => "1. With the part off the pins, jog the probe a few mm above the middle of pin 1 and press Reference X Y from pin 1 in the Machine tab: every E-stop clears it.".into(),
    });
    steps.push(if p.offset == format!("G{}", 54 + s.active_wcs) {
        format!("2. {} is the active work system.", p.offset)
    } else {
        format!("2. Select {} under Work zero.", p.offset)
    });
    steps.push(match c.pin_offset() {
        Ok(want) if (0..2).all(|a| (s.work_offset[a] - want[a]).abs() <= 1e-3) => format!(
            "3. X and Y are set from the pins: the work offset of {} is machine X {:.3} Y {:.3}.",
            p.offset, want[0], want[1]
        ),
        Ok(want) => format!(
            "3. Put the part on the pins and press Set X Y from the pins: the work offset of {} becomes machine X {:.3} Y {:.3}, pin 1's recorded place less its X Y in work coordinates. Nothing is probed.",
            p.offset, want[0], want[1]
        ),
        Err(why) => format!("3. Set X Y from the pins: {why}"),
    });
    steps.push(match c.touch_off() {
        Some((_, work)) => {
            let spot = crate::post::references(&p.origin).map_or("the stock top".to_string(), |refs| {
                match crate::control::probe_spot(refs).as_str() {
                    "its centre" => "the centre of the stock top".to_string(),
                    spot => format!("{spot} of the stock top"),
                }
            });
            format!(
                "4. Jog the tool tip over {spot}, down until it just pinches a sheet of paper, and press Set Z{}, or put the touch plate there and press Probe Z in the Probe tab. {}",
                crate::post::exact_mm(work[2] as f32),
                fits_check(c)
            )
        }
        None => "4. Touch Z off on the part with Set under Work zero: the program header says where Z0 is.".into(),
    });
    steps
}
/// What the checks say once the work zero is on the stock: whether every
/// tool run fits the soft limits, which needs a machine position.
fn fits_check(c: &crate::control::Connection) -> &'static str {
    if c.snapshot.referenced[..3].iter().all(|r| *r) {
        "The checks under Operations then say whether every tool run fits the soft limits from there."
    } else {
        "Without a machine position the checks cannot say whether every tool run fits the machine's travel."
    }
}
/// A work position (mm) as it is said: "X0 Y0 Z12.5".
fn work_words(work: [f64; 3]) -> String {
    let axis = |a: usize| format!("{}{}", AXES[a], crate::post::exact_mm(work[a] as f32));
    format!("{} {} {}", axis(0), axis(1), axis(2))
}
/// The button that sets the touch-off point's work position `work`
/// (`control::Action::TouchOff`): Zero all for X0 Y0 Z0.
fn touch_label(work: [f64; 3]) -> String {
    if work.iter().all(|v| crate::post::exact_mm(*v as f32) == "0") {
        "Zero all".into()
    } else {
        format!("Set {}", work_words(work))
    }
}
/// The stock-top spots Probe corner, Probe edge or Probe centre can find
/// the setup's work zero from, laid out as the top is seen from above (back
/// at the top): a spot with a jaw or clamp in its way, or cut by an earlier
/// setup, is greyed out, and the reason shows under the grid.
fn probe_from_grid<'a>(
    c: &'a crate::control::Connection,
    spots: &[([crate::post::At; 2], Result<(), String>)],
) -> Element<'a, Message> {
    use crate::control::Action;
    use crate::post::At::{Max, Mid, Min};
    let program = c.run_program().ok();
    // The zero's own spot, reachable or not: the one a jaw may stand at.
    let own = program.and_then(|p| crate::post::references(&p.origin));
    let model = program.is_some_and(|p| crate::post::stock_zero(&p.origin).is_none());
    let reachable = |refs| spots.iter().any(|(r, ok)| *r == refs && ok.is_ok());
    // What Probe corner will probe from: none lit when no spot it may use.
    let picked = c
        .probe_spot
        .filter(|&r| reachable(r))
        .or(own.filter(|&r| reachable(r)));
    let name = |refs: [crate::post::At; 2]| match refs {
        [Mid, Mid] => "Centre".to_string(),
        [Mid, _] | [_, Mid] => {
            let k = usize::from(refs[0] == Mid);
            let s = crate::post::ENDS[k][usize::from(refs[k] == Max)];
            s[..1].to_uppercase() + &s[1..]
        }
        _ => {
            let s = crate::post::corner_name(refs);
            s[..1].to_uppercase() + &s[1..]
        }
    };
    let mut grid = column![].spacing(4);
    for y in [Max, Mid, Min] {
        let mut line = row![].spacing(4);
        for x in [Min, Mid, Max] {
            let refs = [x, y];
            let ok = reachable(refs);
            // Model origin is set off from its spot, not on it.
            let label = match (Some(refs) == own, model) {
                (true, false) => format!("{} (zero)", name(refs)),
                (true, true) => format!("{} (default)", name(refs)),
                _ => name(refs),
            };
            let pick = if Some(refs) == own { None } else { Some(refs) };
            line = line.push(act_sized(
                label,
                (ok && !c.probing()).then_some(Message::Machine(Action::ProbeFrom(pick))),
                picked == Some(refs),
                Some(Length::Fixed(150.)),
            ));
        }
        grid = grid.push(line);
    }
    let mut out = column![
        muted("Probe from (the stock top seen from above, back at the top)"),
        grid
    ]
    .spacing(6);
    let refused: Vec<String> = spots
        .iter()
        .filter_map(|(r, ok)| ok.as_ref().err().map(|why| format!("{}: {why}", name(*r))))
        .collect();
    if !refused.is_empty() {
        out = out.push(muted(refused.join(" ")));
    }
    out.into()
}
/// How the last probe ended, or that one is under way.
fn probe_note<'a>(c: &crate::control::Connection) -> Option<Element<'a, Message>> {
    if c.probing() {
        return Some(text("Probing: E-stop stops it.").size(12).into());
    }
    let (ok, message) = c.probed.as_ref()?;
    Some(
        text(message.clone())
            .size(12)
            .color(if *ok {
                Color::from_rgb8(39, 130, 93)
            } else {
                Color::from_rgb8(166, 101, 32)
            })
            .into(),
    )
}
/// The fixture pins under Machine zero: Record the pins' place and
/// Reference X Y from pin 1, why either cannot start, and how the last one
/// ended. None for a machine without pins.
fn pin_block<'a>(c: &crate::control::Connection) -> Option<Element<'a, Message>> {
    use crate::control::Action;
    let pins = match &c.pins {
        Ok(None) => return None,
        Ok(Some(pins)) => Some(pins),
        Err(_) => None,
    };
    let (record, reference) = (c.record_blocked(), c.reference_blocked());
    let mut col = column![
        bold("Fixture pins", 12),
        row![
            act(
                "Record the pins' place",
                record
                    .is_none()
                    .then_some(Message::Machine(Action::RecordPins)),
                c.pinning() == Some(true)
            ),
            act(
                "Reference X Y from pin 1",
                reference
                    .is_none()
                    .then_some(Message::Machine(Action::ReferencePins)),
                c.pinning() == Some(false)
            ),
        ]
        .spacing(6),
    ]
    .spacing(6);
    let green = Color::from_rgb8(39, 130, 93);
    let state = match pins.and_then(|p| p.reference) {
        _ if c.pinning().is_some() => text("Probing pin 1: E-stop stops it.").size(12),
        Some(at) if c.pin_referenced => text(format!(
            "X and Y are referenced from pin 1, recorded at machine X {:.3} Y {:.3}.",
            at[0], at[1]
        ))
        .size(12)
        .color(green),
        Some(at) => muted(format!(
            "Pin 1 is recorded at machine X {:.3} Y {:.3}. After every E-stop or power cycle, take the part off the pins, jog the probe a few mm above the middle of pin 1 and press Reference X Y from pin 1: X and Y need no machine position first, Z does.{}",
            at[0],
            at[1],
            reference.map_or(String::new(), |why| format!(" {why}"))
        )),
        None => muted(format!(
            "Not recorded yet. Once, after setting machine zero: take the part off the pins, jog the probe a few mm above the middle of pin 1 and press Record the pins' place. It finds pin 1, then pin 2, and says how square the plate is.{}",
            record.map_or(String::new(), |why| format!(" {why}"))
        )),
    };
    col = col.push(state);
    if let Some(top) = c.sim_pin1_top() {
        col = col.push(muted(format!(
            "Simulated: the simulator's pin 1 stands at machine X {:.3} Y {:.3}, its top at Z {:.3}. The MDI line G53 G0 X{:.1} Y{:.1} Z{:.1} puts the tip 3 mm over it.",
            top[0],
            top[1],
            top[2],
            top[0],
            top[1],
            top[2] + 3.
        )));
    }
    if let Some((ok, said)) = &c.pin_said {
        col = col.push(text(said.clone()).size(12).color(if *ok {
            green
        } else {
            Color::from_rgb8(166, 101, 32)
        }));
    }
    Some(col.into())
}
/// The Rotary A tab on a machine with the rotary unit (LIO-186): how the
/// A centreline was found, the calibration probes, Save the centreline and
/// Rotary unit remounted, why a probe cannot start and how the last ended.
fn rotary_tab<'a>(c: &crate::control::Connection) -> Option<Element<'a, Message>> {
    use crate::control::{Action, Calibrate};
    let rot = c.rotary.as_ref().filter(|_| c.axis(3))?;
    let (green, amber) = (
        Color::from_rgb8(39, 130, 93),
        Color::from_rgb8(166, 101, 32),
    );
    let (y, z) = (rot.unit.centre_y, rot.unit.centre_z);
    let state = if rot.measured() {
        text(format!(
            "Centreline measured: machine Y {y:.3}, Z {z:.3}. Probe it again after the rotary unit is moved."
        ))
        .size(12)
        .color(green)
    } else if rot.mark.starts_with("stale") {
        text(format!(
            "The measured centreline (machine Y {y:.3}, Z {z:.3}) is {}: probe it again before cutting on A.",
            rot.mark
        ))
        .size(12)
        .color(amber)
    } else {
        text(format!(
            "The centreline is ASSUMED, machine Y {y:.3}, Z {z:.3}: probe it before cutting on A. An error of e mm puts a feature cut at A 180° off by 2e."
        ))
        .size(12)
        .color(amber)
    };
    let field = |value: &str, hint: &'a str, key: &str, label: &str, on: fn(String) -> Action| {
        help::field(
            text_input(hint, value)
                .on_input(move |v| Message::Machine(on(v)))
                .width(90)
                .padding([7, 9])
                .size(13),
            key,
            label,
            Target::Machine,
        )
    };
    let fields = row![
        muted("Bar or stock Ø"),
        field(
            &c.bar_text,
            "mm",
            "rotary_bar",
            "Bar or stock diameter · mm",
            Action::BarText
        ),
        muted("Touch spacing"),
        field(
            &c.spacing_text,
            "mm",
            "rotary_spacing",
            "Touch spacing · mm",
            Action::SpacingText
        ),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let probe = |what: Calibrate| {
        act(
            what.label(),
            c.calibrate_blocked(what)
                .is_none()
                .then_some(Message::Machine(Action::Calibrate(what))),
            c.calibrating() == Some(what),
        )
    };
    let idle = c.calibrating().is_none();
    let buttons = row![
        probe(Calibrate::Centreline),
        probe(Calibrate::Alignment),
        probe(Calibrate::AZero),
        act(
            "Touch the Z reference",
            c.centre_reference_blocked()
                .is_none()
                .then_some(Message::Machine(Action::TouchCentreReference)),
            false
        ),
        act(
            "Save the centreline",
            (idle && c.centre.is_some_and(|x| x.reference.is_some()) && !c.centre_saved)
                .then_some(Message::Machine(Action::SaveCentreline)),
            false
        ),
        act(
            "Rotary unit remounted",
            (idle && rot.measured()).then_some(Message::Machine(Action::Remounted)),
            false
        ),
    ]
    .spacing(6)
    .wrap()
    .vertical_spacing(6);
    let how = match c.calibrating() {
        Some(what) => format!("{} E-stop stops it.", what.how()),
        None => match c.calibrate_blocked(Calibrate::Centreline) {
            Some(why) => format!("{} {why}", Calibrate::Centreline.how()),
            None => Calibrate::Centreline.how(),
        },
    };
    let mut col = column![state, fields, buttons, muted(how)].spacing(8);
    if c.centre.is_some_and(|x| x.reference.is_none()) && idle {
        col = col.push(muted(crate::control::NO_CENTRE_REFERENCE));
    }
    if let Some((ok, said)) = &c.rotary_said {
        col = col.push(
            text(said.clone())
                .size(12)
                .color(if *ok { green } else { amber }),
        );
    }
    if let Some(bar) = c.sim_bar() {
        let r = c.bar_diameter().unwrap_or_default() / 2.;
        let x = bar.x[0] + crate::control::CHUCK_GAP + 5.;
        let top = z as f64 + r + 3.;
        col = col.push(muted(format!(
            "Simulated: a Ø{} mm test bar is in the chuck, from X {:.0} to {:.0}, on an A axis a little off the assumed one, with a 1 mm deep flat on top at about A −45°; the simulator's block is off the table meanwhile. The MDI line G53 G0 X{x:.1} Y{y:.1} Z{top:.1} puts the tip about 3 mm over it near the chuck.",
            2. * r,
            bar.x[0],
            bar.x[1],
        )));
    }
    Some(tab_scroll(col))
}
const HOME_NOTE: &str = "Home references every axis on its switch. Every E-stop clears the machine position: home again after one.";
const MACHINE_ZERO_NOTE: &str = "Every E-stop clears the machine position: set it again after one. Without home switches, jog X to its left end, Y to its front end (gantry toward you) and Z to the top, then Set machine zero here. Keep the last position takes the position AXIS CAM saved at the last disconnect: only if nothing moved the machine since.";
/// The note under a library editor's fields when its draft is not what the
/// library holds: `saved` is the entry with the draft's id, if there is one.
fn unsaved_note(draft: &Fields, saved: Option<&Fields>, what: &str) -> Option<String> {
    match saved {
        None => Some(format!(
            "This {what} is not in the library yet: Save adds it."
        )),
        Some(saved) if saved != draft => Some(format!(
            "Unsaved changes to this {what}: Save keeps them, choosing another entry drops them."
        )),
        Some(_) => None,
    }
}
pub(crate) fn tool_unsaved(app: &App) -> Option<String> {
    let draft = &app.tool_draft;
    let saved = app.doc.tools.iter().find(|t| t.id == draft.id);
    unsaved_note(&draft.p, saved.map(|t| &t.p), "tool")
}
pub(crate) fn material_unsaved(app: &App) -> Option<String> {
    let draft = &app.material_draft;
    let saved = app.doc.materials.iter().find(|m| m.id == draft.id);
    unsaved_note(&draft.p, saved.map(|m| &m.p), "material")
}
pub(crate) fn holder_unsaved(app: &App) -> Option<String> {
    let draft = &app.holder_draft;
    let saved = app.doc.workholding.iter().find(|h| h.id == draft.id);
    unsaved_note(&draft.p, saved.map(|h| &h.p), holder_word(draft))
}
/// What a workholding entry is called on its buttons and notes.
fn holder_word(h: &crate::holding::Holder) -> &'static str {
    match h.kind() {
        "Vise" => "vise",
        "Clamp" => "clamp",
        "Chuck" => "chuck",
        "V-block" => "V-block",
        "Soft jaws" => "soft jaws",
        "Collet block" => "collet block",
        _ => "entry",
    }
}
/// The labels of the Save and Delete buttons of the workholding editor.
pub(crate) fn holder_labels(h: &crate::holding::Holder) -> (String, String) {
    let word = holder_word(h);
    (format!("Save {word}"), format!("Delete {word}"))
}
/// The one layout of the library editors (Tools, Materials, Workholding): the
/// fields scroll, and under them, outside the scroll, the notes (an unsaved
/// draft, an entry that cannot be deleted) and the Save and Delete buttons
/// stay in sight at any window height.
/// A library entry's editor: its fields, scrolled, and under them, always
/// in view, what keeps Save from saving it (`blocking`, its own checks; the
/// form lists them too, at its end), the notes, and the buttons.
fn library_editor<'a>(
    form: Column<'a, Message>,
    blocking: &[String],
    notes: impl IntoIterator<Item = String>,
    save: HelpButton<'a>,
    delete: HelpButton<'a>,
) -> Element<'a, Message> {
    let mut actions = column![].spacing(8);
    if !blocking.is_empty() {
        actions = actions.push(
            text(format!("Save is blocked: {}", blocking.join(" ")))
                .size(12)
                .color(Color::from_rgb8(180, 35, 24)),
        );
    }
    for note in notes {
        actions = actions.push(text(note).size(12).color(Color::from_rgb8(166, 101, 32)));
    }
    actions = actions.push(row![save, delete].spacing(10));
    let form = container(form).width(Fill).padding(iced::Padding {
        right: SCROLLBAR_GUTTER,
        ..Default::default()
    });
    container(column![scrollable(form).height(Fill), actions].spacing(14))
        .padding(24)
        .width(Fill)
        .height(Fill)
        .style(panel_style)
        .into()
}
pub(crate) fn tools(app: &App) -> Element<'_, Message> {
    if app.doc.tools.is_empty() && app.tool_draft.name().is_empty() {
        return container(
            column![
                page_header("Tool library", "Save cutters you actually use."),
                space().height(40),
                bold("No tools yet", 28),
                muted(match &app.pending {
                    Some((_, Message::AddOp(s))) => format!(
                        "{} needs a cutter. Create one: saving it adds {}.",
                        s.name(),
                        s.name()
                    ),
                    _ =>
                        "Create your first cutter, then select it when adding an operation.".into(),
                }),
                primary("+ Create first tool", Message::NewTool)
            ]
            .spacing(18),
        )
        .padding(28)
        .width(Fill)
        .height(Fill)
        .into();
    }
    let mut list = column![
        bold("Tool library", 17),
        primary("+ New tool", Message::NewTool)
    ]
    .spacing(12);
    for t in &app.doc.tools {
        list = list.push(
            btn(
                format!(
                    "{}\n{}",
                    // "T4 · name", or the name when it begins with its number.
                    if crate::model::names_number(get(&t.p, "number"), t.name()) {
                        t.name().to_string()
                    } else {
                        format!("T{} · {}", get(&t.p, "number"), t.name())
                    },
                    tool_details(t)
                ),
                Message::EditTool(t.id.clone()),
                app.tool_draft.id == t.id,
            )
            .width(Fill),
        );
    }
    let p = &app.tool_draft.p;
    let t = Target::Tool;
    let mut form = column![
        bold("Tool definition", 21),
        field(p, "name", "Tool name", t),
        choice(
            p,
            "type",
            "Tool type",
            &[
                "Flat end mill",
                "Ball end mill",
                "Bull nose end mill",
                "Face mill",
                "Drill",
                "Chamfer mill",
                "Thread mill",
                crate::model::KEYSEAT,
                "Tap",
                "Reamer",
                "Boring head",
                "Turning tool",
                "Boring bar",
                "Grooving tool",
                "Parting blade",
                "Threading tool",
                "Die"
            ],
            t
        )
    ]
    .spacing(14);
    let lathe = crate::lathe::is_tool(get(p, "type"));
    form = form.push(field(p, "number", "Tool number · T", t));
    // A lathe tool has no mill diameter, flutes, flute length, stickout or holder.
    let keyseat = get(p, "type") == crate::model::KEYSEAT;
    // A boring head's diameter is its body's, its flute length how far its
    // bar reaches below the body.
    let head = get(p, "type") == "Boring head";
    if !lathe {
        for (k, l) in [
            (
                "diameter",
                if keyseat {
                    "Head diameter · mm"
                } else if head {
                    "Head body diameter · mm"
                } else {
                    "Diameter · mm"
                },
            ),
            ("flutes", if head { "Cutting edges" } else { "Flutes" }),
            (
                "length",
                if head {
                    "Reach (bar below the body) · mm"
                } else {
                    "Flute length · mm"
                },
            ),
            ("stickout", "Stickout · mm"),
            ("holder", "Holder diameter · mm"),
            (
                "holder_length",
                "Holder length to the spindle nose · mm (empty: 10, assumed)",
            ),
        ] {
            // A keyseat cutter's flutes are its head, its width long.
            if !(keyseat && k == "length") {
                form = form.push(field(p, k, l, t));
            }
        }
    }
    if get(p, "type") == "Chamfer mill" {
        form = form
            .push(field(p, "tip_angle", "Tip angle · °", t))
            .push(hinted_field(
                p,
                "tip_flat",
                "Flat tip · mm across",
                "0 (pointed)",
                t,
                &[],
            ));
    }
    match get(p, "type") {
        "Turning tool" => {
            form = form
                .push(choice(p, "hand", "Hand", &crate::lathe::HANDS, t))
                .push(field(p, "nose_radius", "Nose radius · mm", t))
                .push(field(p, "lead_angle", "Lead angle · °", t));
        }
        "Boring bar" => {
            form = form
                .push(field(p, "nose_radius", "Nose radius · mm", t))
                .push(field(p, "min_bore", "Smallest bore · mm", t))
                .push(field(p, "reach", "Reach · mm", t));
        }
        "Grooving tool" | "Parting blade" => {
            form = form
                .push(field(p, "width", "Blade width · mm", t))
                .push(field(p, "reach", "Deepest plunge · mm", t));
        }
        "Threading tool" => {
            form = form
                .push(choice(
                    p,
                    "thread_angle",
                    "Thread angle · °",
                    &["60", "55"],
                    t,
                ))
                .push(field(p, "pitches", "Pitches · mm, min-max", t));
        }
        "Thread mill" => {
            form = form
                .push(choice(
                    p,
                    "thread_angle",
                    "Thread angle · °",
                    &["60", "55"],
                    t,
                ))
                .push(field(p, "teeth", "Teeth (stacked along it)", t))
                .push(field(p, "pitches", "Pitches · mm (e.g. 0.5-1.25)", t))
                .push(field(p, "reach", "Reach (neck) · mm", t))
                .push(field(p, "shank", "Shank diameter · mm", t));
        }
        "Tap" => {
            let sizes: Vec<&str> = crate::thread::SPECS.iter().map(|s| s.name).collect();
            form = form
                .push(choice(p, "thread", "Thread", &sizes, t))
                .push(choice(p, "tap_kind", "Tap kind", &TAP_KINDS, t));
        }
        crate::model::KEYSEAT => {
            form = form
                .push(field(p, "width", "Head width · mm", t))
                .push(field(p, "neck", "Neck diameter · mm", t))
                .push(field(p, "reach", "Reach (head bottom to shank) · mm", t))
                .push(field(p, "shank", "Shank diameter · mm", t));
        }
        "Die" => {
            let sizes: Vec<&str> = crate::thread::SPECS.iter().map(|s| s.name).collect();
            form = form
                .push(choice(p, "thread", "Thread", &sizes, t))
                .push(field(p, "od", "Die outside diameter · mm", t));
        }
        "Reamer" => {
            form = form.push(choice(
                p,
                "tolerance",
                "Tolerance class",
                &crate::model::REAMER_CLASSES,
                t,
            ));
        }
        "Boring head" => {
            form = form
                .push(field(p, "bore_range", "Bore range · mm, min-max", t))
                .push(field(p, "bar_diameter", "Bar diameter · mm", t))
                .push(field(p, "dial", "Dial graduation · mm on the diameter", t))
                .push(choice(
                    p,
                    "bore_retract",
                    "Out of the bore",
                    &crate::model::BORE_RETRACTS,
                    t,
                ));
        }
        _ => {}
    }
    form = form.push(machine_boxes(app, p, lathe, Message::ToolMachine));
    let mut tool_issues = app.tool_draft.issues();
    tool_issues.extend(app.doc.tool_number_issues(&app.tool_draft));
    form = form.push(warnings(tool_issues.clone()));
    let editor = library_editor(
        form,
        &tool_issues,
        tool_unsaved(app)
            .into_iter()
            .chain(app.doc.tool_in_use_note(&app.tool_draft.id)),
        primary(
            app.resume_label().unwrap_or("Save tool".into()),
            Message::SaveTool,
        ),
        danger(
            "Delete tool",
            Message::DeleteTool(app.tool_draft.id.clone()),
        ),
    );
    container(column![page_header("Tools","Cutter dimensions are stored in millimeters. Tool edits invalidate generated previews."),row![container(scrollable(list)).padding(20).width(320).height(Fill).style(panel_style),editor].spacing(20).height(Fill)].spacing(24)).padding(28).height(Fill).into()
}
pub(crate) fn materials(app: &App) -> Element<'_, Message> {
    let mut list = column![
        bold("Material library", 17),
        primary("+ New material", Message::NewMaterial)
    ]
    .spacing(12);
    for m in &app.doc.materials {
        let data = m.cutting_data().map_or("no cutting data".to_string(), |d| {
            format!("Vc {}–{} m/min", d.vc.0, d.vc.1)
        });
        list = list.push(
            btn(
                format!("{}\n{} · {}", m.name(), get(&m.p, "group"), data),
                Message::EditMaterial(m.id.clone()),
                app.material_draft.id == m.id,
            )
            .width(Fill),
        );
    }
    let p = &app.material_draft.p;
    let t = Target::Material;
    let mut form = column![
        bold("Material definition", 21),
        field(p, "name", "Material name", t),
        choice(p, "group", "Material group", &crate::material::GROUPS, t),
        field(p, "hardness", "Hardness", t),
        field(p, "notes", "Notes", t),
        section("Cutting data for carbide tools"),
        muted("Fill in every field, or leave them all empty.")
    ]
    .spacing(14);
    let pair = |label: &'static str, min: &'static str, max: &'static str| {
        column![
            muted(label),
            row![
                container(field(p, min, "Minimum", t)).width(Length::FillPortion(1)),
                container(field(p, max, "Maximum", t)).width(Length::FillPortion(1))
            ]
            .spacing(10)
        ]
        .spacing(5)
    };
    form = form
        .push(pair("Surface speed · m/min", "vc_min", "vc_max"))
        .push(muted("Chip load per tooth · mm"));
    for (label, min, max) in [
        ("Ø up to 3", "fz3_min", "fz3_max"),
        ("Ø 3–6", "fz6_min", "fz6_max"),
        ("Ø 6–10", "fz10_min", "fz10_max"),
        ("Ø 10–16", "fz16_min", "fz16_max"),
        ("Ø over 16", "fzbig_min", "fzbig_max"),
    ] {
        form = form.push(pair(label, min, max));
    }
    let mut issues = app.material_draft.issues();
    issues.extend(app.doc.material_name_issues(&app.material_draft));
    form = form.push(warnings(issues.clone()));
    let editor = library_editor(
        form,
        &issues,
        material_unsaved(app),
        primary("Save material", Message::SaveMaterial),
        danger(
            "Delete material",
            Message::DeleteMaterial(app.material_draft.id.clone()),
        ),
    );
    container(
        column![
            page_header(
                "Materials",
                "Cutting data are for carbide tools and are starting points."
            ),
            row![
                container(scrollable(list))
                    .padding(20)
                    .width(320)
                    .height(Fill)
                    .style(panel_style),
                editor
            ]
            .spacing(20)
            .height(Fill)
        ]
        .spacing(24),
    )
    .padding(28)
    .height(Fill)
    .into()
}
const VISE_FIELDS: [(&str, &str); 7] = [
    ("jaw_width", "Jaw width · mm"),
    ("jaw_height", "Jaw height · mm"),
    ("jaw_thickness", "Jaw thickness · mm"),
    ("max_opening", "Maximum opening · mm"),
    ("grip", "Grip height · mm"),
    ("base_height", "Stock rests above the table · mm"),
    ("force", "Clamping force · N"),
];
const CHUCK_FIELDS: [(&str, &str); 4] = [
    ("min_grip", "Smallest grip · Ø mm"),
    ("max_grip", "Largest grip · Ø mm"),
    ("jaw_length", "Jaw length · mm"),
    ("body", "Chuck body · Ø mm"),
];
const CLAMP_FIELDS: [(&str, &str); 5] = [
    ("length", "Length · mm"),
    ("width", "Width · mm"),
    ("height", "Height above the stock · mm"),
    ("bite", "Bite onto the stock · mm"),
    ("force", "Clamping force · N"),
];
const V_BLOCK_FIELDS: [(&str, &str); 8] = [
    ("angle", "V angle · °"),
    ("v_width", "V width at the top · mm"),
    ("width", "Block width · mm"),
    ("height", "Block height · mm"),
    ("length", "Block length along the bar · mm"),
    ("clamp_width", "Clamp strap width · mm"),
    ("clamp_height", "Clamp strap thickness · mm"),
    ("force", "Clamping force · N"),
];
const SOFT_JAW_FIELDS: [(&str, &str); 6] = [
    ("bore", "Bored to · Ø mm"),
    ("jaw_width", "Jaw width · mm"),
    ("jaw_height", "Jaw height · mm"),
    ("jaw_thickness", "Jaw thickness · mm"),
    ("max_opening", "Maximum opening · mm"),
    ("force", "Clamping force · N"),
];
const COLLET_BLOCK_FIELDS: [(&str, &str); 4] = [
    ("er", "ER size (11, 16, 20, 25, 32, 40)"),
    ("block", "Block width · mm"),
    ("height", "Block height · mm"),
    ("force", "Grip force · N (empty: the size's)"),
];
/// A library entry's key sizes, for the list.
fn holder_summary(h: &crate::holding::Holder) -> String {
    let n = |key| get(&h.p, key);
    match h.kind() {
        "Vise" => format!(
            "Vise · jaws {} wide, {} thick · opens to {} mm",
            n("jaw_width"),
            n("jaw_thickness"),
            n("max_opening")
        ),
        "Clamp" => format!(
            "Clamp · {} long, {} wide · bite {} mm",
            n("length"),
            n("width"),
            n("bite")
        ),
        "Chuck" => format!(
            "Chuck · {} jaws · grips Ø{}–Ø{} mm",
            n("jaws"),
            n("min_grip"),
            n("max_grip")
        ),
        "V-block" => format!(
            "V-block · {}° V {} wide · {} long",
            n("angle"),
            n("v_width"),
            n("length")
        ),
        "Soft jaws" => format!("Soft jaws · bored Ø{} · {} wide", n("bore"), n("jaw_width")),
        "Collet block" => format!("Collet block · ER{} · {} mm block", n("er"), n("block")),
        other => other.to_string(),
    }
}
pub(crate) fn workholding(app: &App) -> Element<'_, Message> {
    let mut list = column![
        bold("Workholding library", 17),
        row![
            // Two equal choices: neither is the panel's one filled command.
            secondary("+ New vise", Message::NewHolder("Vise")),
            secondary("+ New clamp", Message::NewHolder("Clamp")),
            secondary("+ New chuck", Message::NewHolder("Chuck"))
        ]
        .spacing(8),
        // A round part's holdings (`round_hold`).
        row![
            secondary("+ New V-block", Message::NewHolder("V-block")),
            secondary("+ New soft jaws", Message::NewHolder("Soft jaws")),
            secondary("+ New collet block", Message::NewHolder("Collet block"))
        ]
        .spacing(8)
    ]
    .spacing(12);
    for h in &app.doc.workholding {
        list = list.push(
            btn(
                format!("{}\n{}", h.name(), holder_summary(h)),
                Message::EditHolder(h.id.clone()),
                app.holder_draft.id == h.id,
            )
            .width(Fill),
        );
    }
    let draft = &app.holder_draft;
    let p = &draft.p;
    let t = Target::Holder;
    let (title, sizes): (&str, &[(&str, &str)]) = match draft.kind() {
        "Vise" => ("Vise definition", &VISE_FIELDS),
        "Clamp" => ("Clamp definition", &CLAMP_FIELDS),
        "Chuck" => ("Chuck definition", &CHUCK_FIELDS),
        "V-block" => ("V-block definition", &V_BLOCK_FIELDS),
        "Soft jaws" => ("Soft jaws definition", &SOFT_JAW_FIELDS),
        "Collet block" => ("Collet block definition", &COLLET_BLOCK_FIELDS),
        _ => ("Workholding definition", &[]),
    };
    let mut form = column![bold(title, 21), field(p, "name", "Name", t)].spacing(14);
    if draft.kind() == "Chuck" {
        form = form.push(choice(p, "jaws", "Jaws", &["3", "4"], t));
    }
    for (key, label) in sizes {
        form = form.push(field(p, key, label, t));
    }
    let mut issues = draft.issues();
    issues.extend(app.doc.holder_name_issues(draft));
    form = form
        .push(machine_boxes(
            app,
            p,
            draft.kind() == "Chuck",
            Message::HolderMachine,
        ))
        .push(field(p, "notes", "Notes", t))
        .push(warnings(issues.clone()));
    let (save, delete) = holder_labels(draft);
    let editor = library_editor(
        form,
        &issues,
        holder_unsaved(app),
        primary(save, Message::SaveHolder),
        danger(delete, Message::DeleteHolder(draft.id.clone())),
    );
    container(
        column![
            page_header(
                "Workholding",
                "Vises, toe clamps, lathe chucks, and the V-blocks, soft jaws and collet blocks that hold round parts, your setups can model. The starter entries have typical sizes: measure yours."
            ),
            row![
                container(scrollable(list))
                    .padding(20)
                    .width(320)
                    .height(Fill)
                    .style(panel_style),
                editor
            ]
            .spacing(20)
            .height(Fill)
        ]
        .spacing(24),
    )
    .padding(28)
    .height(Fill)
    .into()
}
fn account_panel(app: &App) -> Element<'_, Message> {
    use crate::Account;
    let manage = || Message::OpenUrl(crate::openai::USAGE_URL);
    let mut c = column![bold("ChatGPT account", 18)].spacing(12);
    // Without plan usage the note says what to do next, so it sits above
    // Sign out; everywhere else the note closes the panel.
    let note_above = matches!(app.account, Account::SignedIn { granted: false, .. })
        && !app.usage_limit
        && !app.account_note.is_empty();
    match &app.account {
        Account::SignedOut if app.sign_out_pending => c = c.push(muted("Signing out…")),
        Account::SignedOut if app.restoring => c = c.push(muted("Looking for a saved sign-in…")),
        Account::SignedOut => {
            c = c
                .push(primary("Continue with ChatGPT", Message::SignIn).width(Fill))
                .push(muted("Sign in to plan with your ChatGPT plan. AXIS CAM Studio receives your name, email and profile picture. Planning requests count against your ChatGPT plan."));
        }
        Account::SigningIn if app.saving_sign_in() => {
            c = c.push(muted("Finishing sign-in…"));
        }
        Account::SigningIn => {
            c = c
                .push(muted("Waiting for sign-in in your browser…"))
                .push(secondary("Cancel", Message::SignInCancel));
        }
        Account::SignedIn {
            name,
            email,
            granted,
        } => {
            // The once-only notice is for a session that really uses the plan.
            if *granted && !app.settings.plan_notice_shown {
                c = c.push(
                    container(
                        column![
                            bold("You're using your ChatGPT plan", 15),
                            text("Eligible AI requests in this app use your ChatGPT plan. Manage usage in your ChatGPT settings.").size(12),
                            primary("Got it", Message::PlanNoticeDismissed)
                        ]
                        .spacing(10),
                    )
                    .padding(14)
                    .width(Fill)
                    .style(panel_style),
                );
            }
            let title = if name.is_empty() { email } else { name };
            let mut who = column![bold(title.as_str(), 14)].spacing(3);
            if !name.is_empty() {
                who = who.push(muted(email.clone()));
            }
            c = c.push(who);
            if *granted {
                c = c.push(
                    row![
                        muted("Using ChatGPT plan"),
                        secondary("Manage usage", manage())
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                );
                if app.models.is_empty() {
                    if !app.settings.model.is_empty() {
                        c = c.push(muted(format!(
                            "Model: {} (the model list is not loaded)",
                            app.settings.model
                        )));
                    }
                    c = c.push(secondary("Load models", Message::LoadModels).width(Fill));
                } else {
                    c = c.push(
                        column![
                            muted("Model"),
                            pick_list(
                                app.models.clone(),
                                Some(app.settings.model.clone()),
                                Message::PickModel
                            )
                            .width(Fill)
                            .padding([7, 9])
                            .text_size(13)
                            .style(choice_style)
                        ]
                        .spacing(5),
                    );
                }
                if !app.settings.model.is_empty() {
                    c = if app.account_busy {
                        c.push(muted("Checking the connection…"))
                    } else {
                        c.push(secondary("Check connection", Message::CheckConnection).width(Fill))
                    };
                }
            }
            if note_above {
                c = c.push(note(app.account_note.clone()));
            }
            c = c.push(secondary("Sign out", Message::SignOut).width(Fill));
        }
    }
    if app.usage_limit {
        c = c
            .push(bold("Usage limit reached", 13))
            .push(primary("Manage usage", manage()).width(Fill));
    } else if !note_above && !app.account_note.is_empty() {
        c = c.push(note(app.account_note.clone()));
    }
    c.into()
}
/// The stage of the current run, naming the model it asks.
pub(crate) fn running_text(app: &App, stage: crate::planner::Progress) -> String {
    use crate::planner::{MAX_REQUESTS, Progress};
    match stage {
        Progress::Asking(n) if edits_plan(app) => format!(
            "Asking {} for the change ({n} of {MAX_REQUESTS})…",
            app.plan_model
        ),
        _ => stage_text(stage, &app.plan_model),
    }
}
/// Whether the run, or the preview or failure it left, is an asked change to
/// an existing plan: the job's or the open preview's.
pub(crate) fn edits_plan(app: &App) -> bool {
    app.plan_asked.is_some()
        && (app.plan_refined.is_some() || app.doc.setups.iter().any(|s| !s.operations.is_empty()))
}
/// A timeline step that has ended or is running, in the words of the run:
/// `done_text` or `stage_text`, but a change asked of the planner is not
/// "a plan".
fn step_text(app: &App, step: &crate::Step) -> String {
    use crate::planner::{MAX_REQUESTS, Progress};
    let ended = step.ended.is_some();
    match step.stage {
        Progress::Asking(n) if ended && edits_plan(app) => format!(
            "Asked {} for the change ({n} of {MAX_REQUESTS})",
            app.plan_model
        ),
        stage if ended => done_text(stage, &app.plan_model),
        stage => running_text(app, stage),
    }
}
pub(crate) fn stage_text(stage: crate::planner::Progress, model: &str) -> String {
    use crate::planner::{MAX_REQUESTS, Progress};
    match stage {
        Progress::Analyzing => "Analyzing the part…".into(),
        Progress::Asking(n) => format!("Asking {model} for a plan ({n} of {MAX_REQUESTS})…"),
        Progress::Repairing(n) => format!(
            "The last proposal did not pass; asking {model} for a correction ({n} of {MAX_REQUESTS})…"
        ),
        Progress::Extending(n) => format!(
            "The plan passes; asking {model} about faces still covered ({n} of {MAX_REQUESTS})…"
        ),
        Progress::Checking(n) => {
            format!(
                "Checking the plan with the simulation and export checks ({n} of {MAX_REQUESTS})…"
            )
        }
        Progress::Previewing => "The plan passes; building the 3D view of the stock…".into(),
    }
}
const GOOD: Color = Color::from_rgb(0.15, 0.51, 0.36);
const WARN: Color = Color::from_rgb8(166, 101, 32);
pub(crate) const SENT_TO_OPENAI: &str = "Sent to OpenAI: the part's dimensions and feature list, your tools, machine limits, the project material (its name, group, hardness, notes and cutting data), stock size, the sizes and positions of setup 1's vise jaws or clamps and their clearance gap (not their names), the sizes of every vise and clamp in your workholding library (sizes only, never their names or notes), and the app's check reports and tool answers while the model works. The CAD file, file paths, the project title and your setup names are not sent. What you type in the chat with the planner is sent too, and, when you ask for a change, the current plan's setups and operations, with their names.";
pub(crate) const PICTURES_SENT: &str = " With the box below ticked, pictures the app draws of the part, the stock and setup 1's vise or clamps, with surface and hole labels, are sent too, and, when the model asks, pictures of its own toolpaths.";
/// What the timeline says when OpenAI refused a request carrying `what`
/// and the run sends it again without: the line under the request while
/// the run goes, and the words that end the request's line when it is
/// over. Any 400 counts as a refusal, so neither names a cause.
pub(crate) fn refusal(what: Dropped) -> (&'static str, &'static str) {
    match what {
        Dropped::Tool => (
            "OpenAI refused the request; sending it again without its tools.",
            "sent again without tools",
        ),
        Dropped::Pictures => (
            "OpenAI refused the request; sending it again without pictures.",
            "sent again without pictures",
        ),
    }
}
/// The pictures checkbox's hover help.
pub(crate) const PICTURES_HELP: &str = "Ticked, the planner also sends OpenAI pictures the app draws of the part, the stock and setup 1's vise or clamps, with surface and hole labels: an overview with the first request, and the views the model asks for, its own toolpaths among them. Unticked, it sends text only. The CAD file is never sent.";
/// What More… adds to the pictures box's help.
pub(crate) const PICTURES_HELP_MORE: &str =
    "A run keeps the choice it started with: the box cannot be changed until the run is over.";
/// What the page beside the Plan button says is sent to OpenAI.
pub(crate) fn sent_to_openai(pictures: bool) -> String {
    let mut text = SENT_TO_OPENAI.to_string();
    if pictures {
        text.push_str(PICTURES_SENT);
    }
    text
}
fn planner_controls(app: &App) -> Element<'_, Message> {
    let checklist = app.plan_checklist();
    let ready = checklist.iter().all(|line| line.ok);
    let mut c = column![bold("Plan this job", 18)].spacing(10);
    for line in checklist {
        let mark = if line.ok {
            text("✓").size(13).color(GOOD)
        } else {
            text("•").size(13).color(MUTED)
        };
        let mut item = column![text(line.label).size(13)].spacing(2);
        if !line.ok {
            item = item.push(muted(line.hint));
        }
        c = c.push(row![mark, item].spacing(8));
    }
    let pictures = !app.settings.plan_pictures_off;
    c = c.push(muted(sent_to_openai(pictures)));
    // A run keeps the choice it started with, so the box waits for it.
    let running = matches!(app.planner, crate::Planner::Running(_));
    c = c.push(help::wrap(
        checkbox(pictures)
            .label("Let the AI see pictures of the part")
            .on_toggle_maybe((!running).then_some(Message::PlanPictures))
            .text_size(13),
        Help::text("Pictures for the AI planner", PICTURES_HELP).more(PICTURES_HELP_MORE),
        widget::tooltip::Position::Top,
    ));
    if !ready {
        c = c.push(muted(
            "Plan becomes available when every item above is met.",
        ));
    }
    if app.undo.is_some() {
        c = c.push(secondary("Undo applied plan", Message::UndoPlan).width(Fill));
    }
    c.into()
}
/// Model text is shown as plain text, shortened to keep the layout stable.
/// One character more than shown is read, to know whether to add "…".
pub(crate) fn clip(s: &str, limit: usize) -> String {
    let flat = crate::proposal::clip(s, limit.saturating_add(1));
    if flat.chars().count() > limit {
        format!("{}…", flat.chars().take(limit).collect::<String>())
    } else {
        flat
    }
}
/// One planned operation as the preview shows it: the strategy (app text),
/// the model's name for it in quotes, the tool and the time; then the values
/// that will run, read from the built operation; then the model's reason.
pub(crate) fn operation_lines(
    op: &Operation,
    tool: &str,
    minutes: f32,
    reason: &str,
) -> [String; 3] {
    let p = &op.p;
    let value = |key: &str| num(p, key).unwrap_or(0.);
    let n = |v: f32| format!("{}", (v * 1000.).round() / 1000.);
    let strategy = match op.strategy {
        Strategy::Contour if get(p, "side") == "Inside" => "Contour, inside floor",
        Strategy::Contour if get(p, "side") == crate::model::CONTOUR_HOLES => "Contour, holes",
        Strategy::Contour => "Contour, outside",
        s => s.name(),
    };
    // The post rounds the spindle speed to a whole number.
    let mut values = vec![
        format!("{} rpm", n(value("rpm").round())),
        format!("{} mm/min", n(value("feed"))),
        format!("coolant {}", get(p, "coolant")),
    ];
    let mut push = |text: String| values.push(text);
    let stepdown = format!("stepdown {} mm", n(value("stepdown")));
    let stepover = format!("stepover {}%", n(value("stepover")));
    let finish = match value("finish_passes") as u32 {
        1 => "1 finish pass".to_string(),
        passes => format!("{passes} finish passes"),
    };
    let nonzero = |key: &str, label: &str| {
        let v = value(key);
        (v != 0.).then(|| format!("{label} {} mm", n(v)))
    };
    let radial = nonzero("allowance", "radial allowance");
    let axial = nonzero("axial_allowance", "axial allowance");
    let bottom = nonzero("bottom", "bottom offset");
    let entry = if pocket_setting(p, "pocket_entry") == "Profile ramp" {
        format!("entry Profile ramp at {}°", n(value("ramp_angle")))
    } else {
        "entry Plunge".into()
    };
    match op.strategy {
        Strategy::Face => {
            if op.faces_outline() {
                push(format!(
                    "facing area model silhouette + {} mm",
                    n(op.outline_extend().parse().unwrap_or(f32::NAN))
                ));
            } else {
                push(format!("facing area {}", get(p, "boundary").to_lowercase()));
            }
            push(stepdown);
            push(stepover);
            bottom.into_iter().for_each(&mut push);
        }
        Strategy::Adaptive => {
            push(stepdown);
            push(format!("optimal load {} mm", n(value("optimal_load"))));
            push(format!("minimum radius {} mm", n(value("minimum_radius"))));
            [radial, axial, bottom]
                .into_iter()
                .flatten()
                .for_each(&mut push);
            push(if get(p, "ramp") == ADAPTIVE_ENTRIES[1] {
                "entry Plunge".into()
            } else {
                format!("entry Helix at {}°", n(value("ramp_angle")))
            });
        }
        Strategy::Pocket => {
            push(stepdown);
            push(stepover);
            [radial, axial, bottom]
                .into_iter()
                .flatten()
                .for_each(&mut push);
            push(entry);
            push(finish);
        }
        // An inside-floor Contour runs as a floor profile: Pocket's entry and
        // ramp, with a stepover the app fixes at 40%.
        Strategy::Contour if get(p, "side") == "Inside" => {
            push(stepdown);
            push("stepover 40% (fixed)".into());
            [radial, bottom].into_iter().flatten().for_each(&mut push);
            push(entry);
            push(finish);
        }
        Strategy::Contour => {
            push(stepdown);
            [radial, bottom].into_iter().flatten().for_each(&mut push);
            push(finish);
        }
        Strategy::Drill if op.drills_pin_holes() => {
            push("pin holes in the leftover".into());
            bottom.into_iter().for_each(&mut push);
        }
        Strategy::Drill => {
            let cycle = get(p, "cycle");
            push(if ["Chip breaking", "Deep drilling"].contains(&cycle) {
                format!("{cycle}, peck {} mm", n(value("peck")))
            } else if cycle == HELICAL_MILLING {
                format!("{cycle}, {} mm per turn", n(value("stepdown")))
            } else if [THREAD_MILLING, TAPPING].contains(&cycle) {
                let thread = get(p, "thread");
                format!(
                    "{cycle} · {}",
                    if thread == FROM_MODEL {
                        "as modeled"
                    } else {
                        thread
                    }
                )
            } else {
                cycle.into()
            });
            push(if get(p, "through") == "Yes" {
                format!("through, breakthrough {} mm", n(value("breakthrough")))
            } else {
                "not through".into()
            });
            if value("hole_diameter") > 0. {
                push(format!("holes of Ø{} mm only", n(value("hole_diameter"))));
            }
            bottom.into_iter().for_each(&mut push);
        }
        Strategy::Parallel => {
            push(format!(
                "surface stepover {} mm",
                n(value("surface_stepover"))
            ));
            push(format!("tolerance {} mm", n(value("tolerance"))));
            [radial, bottom].into_iter().flatten().for_each(&mut push);
        }
        Strategy::Waterline => {
            push(stepdown);
            [radial, bottom].into_iter().flatten().for_each(&mut push);
        }
        Strategy::Chamfer => {
            let setting = |key: &str| chamfer::setting(p, key).trim().parse::<f32>().unwrap_or(0.);
            push(
                if chamfer::setting(p, "chamfer_size") == chamfer::SIZES[1] {
                    format!("tip Z {} mm", n(setting("tip_z")))
                } else {
                    format!("{} mm wide", n(setting("chamfer_width")))
                },
            );
            push(if chamfer::on_edges(p) {
                chamfer::setting(p, "chamfer_edges").to_lowercase()
            } else {
                format!(
                    "line, side {}",
                    chamfer::setting(p, "chamfer_side").to_lowercase()
                )
            });
            if setting("copies") > 1. {
                push(format!("{} copies", n(setting("copies"))));
            }
        }
        _ => {}
    }
    // The travel height the planner may raise above the default.
    if value("clearance") != crate::model::CLEARANCE {
        push(format!("clearance {} mm", n(value("clearance"))));
    }
    [
        format!("{strategy} · “{}” · {tool} · {minutes:.1} min", op.name()),
        values.join(" · "),
        if reason.is_empty() {
            "The planner gave no reason.".into()
        } else {
            format!("The planner's reason: {reason}")
        },
    ]
}
/// What is still covered by stock: report planes, then the other surface by
/// the direction it faces (in setup 1's frame), then how it was measured.
pub(crate) fn coverage_lines(cover: &crate::plan_check::Coverage) -> Vec<String> {
    let mut lines = vec![];
    for plane in cover.covered_planes.iter().take(8) {
        let face = if plane.depth_below_top.abs() < 0.005 {
            "the top face".to_string()
        } else {
            format!("a face {} mm below the top", plane.depth_below_top)
        };
        lines.push(format!(
            "• Still covered: {face}, seen from {} ({:.0}%)",
            plane.orientation, plane.covered_percent
        ));
    }
    if cover.covered_planes.len() > 8 {
        let more = cover.covered_planes.len() - 8;
        lines.push(format!(
            "• and {more} more face{}",
            if more == 1 { "" } else { "s" }
        ));
    }
    for side in &cover.covered_other {
        let facing = match side.facing {
            "top" => "up".to_string(),
            "bottom" => "down".to_string(),
            axis => axis.to_string(),
        };
        lines.push(format!(
            "• Still covered: walls and curved surface facing {facing} in setup 1 ({:.0}% of {:.0} mm²)",
            side.covered_percent, side.area
        ));
    }
    lines.push(cover.note.to_string());
    lines
}
/// The preview's totals, and a follow-up request's error when one failed.
pub(crate) fn preview_totals(outcome: &crate::planner::Outcome) -> Vec<String> {
    let (job, check) = (&outcome.job, &outcome.check);
    let mut lines = vec![format!(
        "{} in {} · about {:.0} min of machining · {}",
        plural(check.operations.len(), "operation"),
        plural(job.setups.len(), "setup"),
        check.total_minutes,
        plural(outcome.requests, "model request"),
    )];
    lines.extend(check.split_line());
    if let Some(error) = &outcome.note {
        lines.push(format!("A follow-up request failed: {}", error.message()));
    }
    lines
}
/// A tool in a list: its name, with its diameter when the name does not
/// already give it ("Flat end mill · Ø10", not "… Ø10 · Ø10 mm").
fn tool_label(tool: &Tool) -> String {
    let name = tool.name();
    if name.contains('Ø') {
        name.to_string()
    } else {
        format!("{name} · Ø{} mm", get(&tool.p, "diameter"))
    }
}
/// The line under a tool's name in the library: what the name may not say.
fn tool_details(tool: &Tool) -> String {
    if tool.is_lathe() {
        return get(&tool.p, "type").to_string();
    }
    match get(&tool.p, "type") {
        "Boring head" => {
            return format!(
                "Boring head · Ø{} mm · bar Ø{} mm",
                get(&tool.p, "bore_range"),
                get(&tool.p, "bar_diameter")
            );
        }
        "Reamer" => {
            return format!(
                "Reamer · Ø{} {} · {}",
                get(&tool.p, "diameter"),
                get(&tool.p, "tolerance"),
                plural(get(&tool.p, "flutes").trim().parse().unwrap_or(0), "flute")
            );
        }
        _ => {}
    }
    format!(
        "{} · Ø{} mm · {}",
        get(&tool.p, "type"),
        get(&tool.p, "diameter"),
        plural(get(&tool.p, "flutes").trim().parse().unwrap_or(0), "flute")
    )
}
/// "1 setup", "2 setups".
pub(crate) fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}
/// What Apply replaces in the user's job, and how to get it back.
pub(crate) fn apply_note(doc: &crate::model::Document, job: &crate::model::Document) -> String {
    let first = doc.setups.first().map_or(0, |s| s.operations.len());
    let later = doc.setups.len().saturating_sub(1);
    let later_ops: usize = doc.setups.iter().skip(1).map(|s| s.operations.len()).sum();
    let removed = if later == 0 {
        String::new()
    } else {
        format!(
            " and removes your later {} ({})",
            plural(later, "setup"),
            plural(later_ops, "operation")
        )
    };
    format!(
        "Applying replaces the {} of setup 1{removed}; the plan has {}. Setup 1 keeps its name, machine, stock, work zero and fixture. The previous job is kept in History and can be undone here.",
        plural(first, "operation"),
        plural(job.setups.len(), "setup"),
    )
}
/// What Apply does with a change asked of the planner: the job becomes the
/// preview, which kept what the request did not mention.
pub(crate) const CHANGE_NOTE: &str = "Applying makes the changes listed above; everything else stays as it is. The previous job is kept in History and can be undone here.";
/// The planner's workholding note on a setup's card, saying where it goes:
/// setup 1's ends its notes on Apply, in place of an earlier planner note
/// that is unchanged; a later setup's are its notes.
pub(crate) fn workholding_line(index: usize, note: &str) -> String {
    if index == 0 {
        format!(
            "Workholding (from the planner; on Apply it goes at the end of setup 1's notes, replacing an earlier planner note if it is unchanged): {note}"
        )
    } else {
        format!("Workholding (from the planner): {note}")
    }
}
/// A planned setup's modelled workholding on its card (`holding_text`: what
/// holds the stock, where, with what gap), and what is checked of it. A later
/// setup's was laid out by the plan check from its toolpaths. None without
/// modelled workholding.
pub(crate) fn plan_holding_line(job: &crate::model::Document, index: usize) -> Option<String> {
    let text = job.holding_text(job.setups.get(index)?)?;
    let by = if index == 0 {
        ""
    } else {
        ", laid out by the app from this setup's toolpaths"
    };
    if crate::chuck::holds(job.setups.get(index)?) {
        return Some(format!(
            "Modelled workholding{by}: {text} The chuck, its jaws, the rotary head, the tailstock and the stock in the jaws are checked as the boxes round them with the gap, at A0; the table and the machine body are not."
        ));
    }
    Some(format!(
        "Modelled workholding{by}: {text} Jaws and clamps are checked as boxes with the gap; parallels, bolts, the table and the machine body are not."
    ))
}
/// A planned setup's card title. Setup 1 keeps the user's name; a later
/// setup's name was chosen by the planner and is labelled as its text, the
/// way operation names are.
pub(crate) fn setup_title(index: usize, name: &str) -> String {
    if index == 0 {
        format!("Setup 1: {name}")
    } else {
        format!("Setup {} · named by the planner: “{name}”", index + 1)
    }
}
/// A planned setup's rotation and work zero. A Model-origin zero is located
/// from the part's −X −Y corner and the stock top, given `zero`: the part's
/// box and the stock top in work coordinates (mm). When an earlier setup has
/// cut the stock top (None), Z0 is located from the part's top instead and
/// the line says to touch off on a part face.
pub(crate) fn work_zero_line(
    setup: &crate::model::Setup,
    zero: Option<(crate::mesh::Bounds, Option<f32>)>,
) -> String {
    let origin = get(&setup.p, "origin");
    let mut line = format!(
        "Rotation [{}, {}, {}]° · work zero: {origin}",
        get(&setup.p, "rx"),
        get(&setup.p, "ry"),
        get(&setup.p, "rz"),
    );
    if let (Some((part, top)), "Model origin") = (zero, origin) {
        let n = |v: f32| {
            let v = (v * 1000.).round() / 1000.;
            if v == 0. {
                "0".to_string()
            } else {
                format!("{v}")
            }
        };
        let signed = |v: f32| {
            let text = n(v);
            if text.starts_with('-') || text == "0" {
                text
            } else {
                format!("+{text}")
            }
        };
        let size = part.dimensions();
        let (height, reference) = match top {
            Some(top) => (top, "the stock top"),
            None => (part.max[2], "the part's top"),
        };
        let z = if n(height) == "0" {
            format!("Z0 on {reference}")
        } else if height > 0. {
            format!("Z0 {} mm below {reference}", n(height))
        } else {
            format!("Z0 {} mm above {reference}", n(-height))
        };
        line += &format!(
            " — X0 Y0 at X{} Y{} mm from the part's −X −Y corner (part {} × {} mm), {z}",
            signed(-part.min[0]),
            signed(-part.min[1]),
            n(size[0]),
            n(size[1]),
        );
        if top.is_none() {
            line += &format!(". {}", crate::post::STOCK_TOP_CUT);
        }
    }
    line
}
/// The checks a plan passed before it is shown, as the preview lists them:
/// a UC100 machine's (`uc100`) setups get a run's checks, not export's.
pub(crate) fn plan_check_lines(uc100: bool) -> [&'static str; 4] {
    [
        "Every operation generates with the stock left by the ones before it",
        if uc100 {
            "UC100 run checks pass: machine rpm and feed limits, travel, rapids into material, holder clearance, the profile's spindle and coolant outputs"
        } else {
            "LinuxCNC export checks pass: machine rpm and feed limits, travel, rapids into material, holder clearance"
        },
        "No tool centre line cuts into the part, and nothing goes below the stock",
        "No gouging detected on the stock grid (coarse check)",
    ]
}
/// What the preview's checks leave out, under the check lines. A plan's
/// setups all hold their stock the way setup 1 does, so they model their
/// vise or clamps together or not at all. A UC100 machine's (`uc100`) runs
/// retract to the clearance plane, which is checked, not to machine home.
pub(crate) fn plan_not_checked(held: Held, uc100: bool) -> &'static str {
    match held {
        Held::Every if uc100 => {
            "Not checked: cutting loads, tool deflection, parallels, bolts, the table, the machine body and tool-change motion. Vise jaws and clamps are checked as boxes with a clearance gap in every setup, retracts to the clearance plane included. Speeds and feeds are checked against the material's cutting data, which are starting points."
        }
        Held::Every => {
            "Not checked: cutting loads, tool deflection, parallels, bolts, the table, the machine body and tool-change motion. Vise jaws and clamps are checked as boxes with a clearance gap in every setup; retracts to machine home are assumed to clear everything. Speeds and feeds are checked against the material's cutting data, which are starting points."
        }
        _ => {
            "Not checked: cutting loads, tool deflection, fixtures, clamps and the machine body. Speeds and feeds are checked against the material's cutting data, which are starting points."
        }
    }
}
/// Setup `si`'s rotation and work-zero line on the plan preview, with the
/// Model-origin values the plan check found (see `work_zero_line`).
pub(crate) fn plan_work_zero_line(outcome: &crate::planner::Outcome, si: usize) -> String {
    work_zero_line(
        &outcome.job.setups[si],
        outcome.check.zeros.get(si).copied().flatten(),
    )
}
/// The NC review page's note on what export checks, for a job whose setups
/// model their vise or clamps as `held` says.
pub(crate) fn program_note(held: Held) -> &'static str {
    match held {
        Held::No => {
            "LinuxCNC 3-axis or indexed A output, one .ngc program per setup. Export repeats the export checks — limits, travel, rapids into material, holder clearance — and a replay of each program against the simulated moves, and writes nothing if any setup fails. The planner's tool-axis and gouge checks covered the plan as applied, not later edits. Fixtures, clamps and the machine body are not checked: preview each program in LinuxCNC and air-cut first."
        }
        Held::Every => {
            "LinuxCNC 3-axis or indexed A output, one .ngc program per setup. Export repeats the export checks — limits, travel, rapids into material, holder clearance — and a replay of each program against the simulated moves, and writes nothing if any setup fails. The planner's tool-axis and gouge checks covered the plan as applied, not later edits. Every program move, the links between operations, approaches and retracts included, is checked against each setup's vise jaws or clamps as boxes with a clearance gap; parallels, bolts, the table, the machine body and tool-change motion are not checked, and retracts to machine home are assumed to clear everything: preview each program in LinuxCNC and air-cut first."
        }
        Held::Mixed => {
            "LinuxCNC 3-axis or indexed A output, one .ngc program per setup. Export repeats the export checks — limits, travel, rapids into material, holder clearance — and a replay of each program against the simulated moves, and writes nothing if any setup fails. The planner's tool-axis and gouge checks covered the plan as applied, not later edits. Every program move of a setup that models a vise or clamps, the links between operations, approaches and retracts included, is checked against its jaws or clamps as boxes with a clearance gap; parallels, bolts, the table, the machine body and tool-change motion are not checked, and neither are fixtures and clamps in the setups whose workholding is not modelled or cannot be checked; retracts to machine home are assumed to clear everything: preview each program in LinuxCNC and air-cut first."
        }
    }
}
/// The notes that say what is checked of fixtures, in the words for a setup
/// that models none (`modelled` false: the old wording) and for one that
/// models a vise or clamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Note {
    /// The status after an operation preview is generated, after "Sampled removal; ".
    Preview,
    /// Under the operation's tabs for Facing and the strategies without their own note.
    Facing,
    /// Under Pocket's linking tab.
    Pocket,
    /// Under the entry and links of a Facing preview.
    Entry,
    /// Under the separate-paths links.
    Separate,
    /// The label over the simulation.
    Panel,
    /// The end of the Pocket passes note.
    Passes,
    /// The end of what the playback strip says before a preview is generated.
    Playback,
}
/// The whole text of a note that says what is checked of the fixtures, for the
/// active setup: what comes before the words of `fixtures`, then those.
pub(crate) fn fixture_note(app: &App, note: Note) -> String {
    let before = match note {
        Note::Preview | Note::Panel => "",
        Note::Facing => "Selections use connected planar regions of the imported mesh. ",
        Note::Pocket => {
            "Offset contours run from the interior toward the walls at each depth. Entry/full-width cuts can contain both milling modes. "
        }
        Note::Entry => {
            "Entry and depth changes feed down at 25% of cutting feed. Keep-down links use cutting feed within the rectangular preview boundary. "
        }
        Note::Separate => {
            "Separate paths retract vertically to clearance, move in XY, then approach at retract height. Entry feed is 25% of cutting feed. "
        }
        Note::Passes => {
            "Use stepover up to 50%. Finishing passes repeat the wall/island contours at the requested allowance. Earlier cuts determine the cutting start. A small additional margin (max of 0.02 mm or 1% of cutter radius) covers polygon rounding; "
        }
        Note::Playback => {
            "Generate the operation preview, or press Generate & play. Stock uses adaptive surface meshing and inherits earlier cuts; "
        }
    };
    format!("{before}{}", fixtures(note, modelled(app)))
}
/// How many of the job's setups have their vise or clamps checked: those that
/// model them and have nothing wrong with how, as the setup editor lists it
/// (`holding_lines_in`).
pub(crate) fn held(app: &App) -> Held {
    let (left, checks) = (app.left_boxes(), app.current_checks());
    crate::holding::held_of(app.doc.setups.iter().enumerate().map(|(i, setup)| {
        crate::holding::modelled(setup) && holding_lines_in(app, left, checks, i).is_empty()
    }))
}
/// Whether the active setup's vise or clamps are checked: it models them and
/// there is nothing wrong with how (`holding_lines`).
pub(crate) fn modelled(app: &App) -> bool {
    app.doc
        .setups
        .get(app.doc.index())
        .is_some_and(crate::holding::modelled)
        && holding_lines(app).is_empty()
}
/// Whether the active setup's chuck is checked: it holds the bar in one and
/// there is nothing wrong with how (`holding_lines`).
pub(crate) fn chuck_modelled(app: &App) -> bool {
    app.doc
        .active_setup()
        .is_some_and(|s| get(&s.p, "holding") == "Chuck")
        && holding_lines(app).is_empty()
}
pub(crate) fn fixtures(note: Note, modelled: bool) -> &'static str {
    match (note, modelled) {
        (Note::Preview, false) => "fixture and machine collisions are not checked.",
        (Note::Preview, true) => {
            "this operation's own toolpath is checked against the vise jaws or clamps as boxes with a clearance gap (a warning shows if it comes too close); parallels, bolts, the table and the machine body are not."
        }
        (Note::Facing, false) => {
            "Exact CAD faces and fixture or machine collision checks are not implemented; the moves can be exported as LinuxCNC G-code."
        }
        (Note::Facing, true) => {
            "Exact CAD faces are not implemented. This operation's own toolpath is checked against the vise jaws or clamps as boxes with a clearance gap; parallels, bolts, the table and the machine body are not. The moves can be exported as LinuxCNC G-code."
        }
        (Note::Pocket, false) => "No fixture or holder collision solver is connected.",
        (Note::Pocket, true) => {
            "Export refuses a move within the clearance gap of the vise jaws or clamps instead of rerouting it; no other fixture or machine collision solver is connected."
        }
        (Note::Entry, false) => {
            "Lead-in curves, fixture avoidance and collision checking are not implemented."
        }
        (Note::Entry, true) => {
            "Lead-in curves and fixture avoidance are not implemented: export refuses a move within the clearance gap of the vise jaws or clamps instead of rerouting it."
        }
        (Note::Separate, false) => {
            "No lead arcs, stay-down links, fixture avoidance or holder collision checks are available."
        }
        (Note::Separate, true) => {
            "No lead arcs, stay-down links or fixture avoidance are available: export refuses a move within the clearance gap of the vise jaws or clamps instead of rerouting it."
        }
        (Note::Passes, false) => "exact CAD accuracy and collision verification are not available.",
        (Note::Passes, true) => {
            "exact CAD accuracy and collision verification beyond the modelled vise jaws and clamps are not available."
        }
        (Note::Playback, false) => "collision checking is not implemented.",
        (Note::Playback, true) => {
            "collision checks beyond the modelled vise jaws and clamps are not implemented."
        }
        (Note::Panel, false) => "Adaptive stock · no collision checking",
        (Note::Panel, true) => "Adaptive stock · jaws and clamps checked as boxes",
    }
}
/// A text of the plan preview that `clip` shortens and the user can open in
/// full: the planner's summary, a setup's workholding note, an operation's
/// reason, an unmachined feature, a warning (by setup, operation or list
/// position).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Expand {
    Summary,
    Workholding(usize),
    Reason(usize, usize),
    Unmachined(usize),
    Warning(usize),
}
/// The most characters an opened text shows: the layout stays bounded for a
/// reply that is all one huge sentence.
const FULL_TEXT: usize = 4000;
/// The "Show full text" / "Show less" link under a clipped text.
fn full_text_button<'a>(open: bool, key: Expand) -> Element<'a, Message> {
    button(text(if open { "Show less" } else { "Show full text" }).size(12))
        .on_press(Message::PlanExpand(key))
        .padding(0)
        .style(|_, status| button::Style {
            text_color: if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                Color::from_rgb8(32, 97, 155)
            } else {
                BLUE
            },
            ..Default::default()
        })
        .into()
}
/// The model's `parts` as the preview shows them, each cut at its limit, or
/// at `FULL_TEXT` once `key` is open, and the link that opens or closes them
/// when any of them was cut.
fn disclosure<'a, const N: usize>(
    app: &App,
    key: Expand,
    parts: [(&str, usize); N],
) -> ([String; N], Option<Element<'a, Message>>) {
    let open = app.plan_expanded.contains(&key);
    let mut cut = false;
    let shown = parts.map(|(full, limit)| {
        let short = clip(full, limit);
        // `clip` ends a cut text with "…", one character past the limit.
        cut |= short.chars().count() > limit;
        if open { clip(full, FULL_TEXT) } else { short }
    });
    (shown, cut.then(|| full_text_button(open, key)))
}
fn plan_preview<'a>(app: &'a App, preview: &'a crate::PlanPreview) -> Element<'a, Message> {
    let outcome = &preview.outcome;
    let (job, check, proposal) = (&outcome.job, &outcome.check, &outcome.proposal);
    let actions = || {
        row![
            primary("Apply plan", Message::ApplyPlan),
            danger("Discard", Message::DiscardPlan)
        ]
        .spacing(10)
    };
    let mut c = column![bold(
        if edits_plan(app) {
            "Change ready to review"
        } else {
            "Plan ready to review"
        },
        21
    )]
    .spacing(12);
    for line in preview_totals(outcome) {
        c = c.push(muted(line));
    }
    if let Some(asked) = &app.plan_asked {
        c = c.push(muted(format!("You asked: {}", clip(asked, 300))));
    }
    if app.plan_asked.is_some() {
        let lines = crate::plan_changes::changes(&app.doc, job);
        if !lines.is_empty() {
            let mut block = column![bold("What changes against the current job", 15)].spacing(4);
            for line in lines {
                block = block.push(text(format!("• {line}")).size(13));
            }
            c = c.push(block);
        }
    }
    let mut checks = column![].spacing(4);
    // Every setup of a plan uses setup 1's machine.
    let uc100 = get(&job.setups[0].machine.p, "controller") == "UC100";
    for line in plan_check_lines(uc100) {
        checks = checks.push(row![text("✓").size(13).color(GOOD), text(line).size(13)].spacing(8));
    }
    checks = checks.push(muted(plan_not_checked(
        crate::holding::held(&job.setups),
        uc100,
    )));
    c = c.push(checks).push(actions());
    // The model's words are labelled as the planner's; the app's are not.
    if !proposal.summary.is_empty() {
        let ([summary], more) = disclosure(app, Expand::Summary, [(&proposal.summary, 600)]);
        let mut block =
            column![bold("The planner's summary", 15), text(summary).size(13)].spacing(6);
        if let Some(more) = more {
            block = block.push(more);
        }
        c = c.push(block);
    }
    for (si, setup) in job.setups.iter().enumerate() {
        let proposed = proposal.setups.get(si);
        let mut card = column![
            row![
                bold(setup_title(si, setup.name()), 16),
                space().width(Fill),
                muted(get(&setup.p, "offset"))
            ],
            muted(plan_work_zero_line(outcome, si))
        ]
        .spacing(8);
        if let Some(notes) = proposed
            .map(|s| s.workholding.as_str())
            .filter(|n| !n.is_empty())
        {
            let ([notes], more) = disclosure(app, Expand::Workholding(si), [(notes, 500)]);
            card = card.push(muted(workholding_line(si, &notes)));
            if let Some(more) = more {
                card = card.push(more);
            }
        }
        if let Some(line) = plan_holding_line(job, si) {
            card = card.push(muted(line));
        }
        for (oi, o) in setup.operations.iter().enumerate() {
            let tool = job
                .tools
                .iter()
                .find(|t| t.id == o.tool)
                .map(|t| crate::model::tool_label(get(&t.p, "number"), t.name()))
                .unwrap_or_default();
            let minutes = check
                .operations
                .iter()
                .find(|r| r.setup == si + 1 && r.operation == oi + 1)
                .map_or(0., |r| r.minutes);
            let ([reason], more) = disclosure(
                app,
                Expand::Reason(si, oi),
                [(
                    proposed
                        .and_then(|s| s.operations.get(oi))
                        .map_or("", |p| p.reason.as_str()),
                    160,
                )],
            );
            let [head, values, why] = operation_lines(o, &tool, minutes, &reason);
            let mut lines = column![text(head).size(13), muted(values), muted(why)].spacing(2);
            if let Some(more) = more {
                lines = lines.push(more);
            }
            card = card.push(row![icon(app, o.strategy, 22.), lines].spacing(8));
        }
        c = c.push(container(card).padding(16).width(Fill).style(panel_style));
    }
    if let Some(cover) = &check.coverage {
        let left = &cover.leftover_cm3;
        let mut block = column![
            bold("What the plan leaves", 15),
            text(format!("Part surface exposed: {:.0}% (approximate)", cover.exposed_percent)).size(13),
            muted(format!(
                "Stock left: {} cm³ above the part, {} around it, {} inside its bounding box, {} below.",
                left.above, left.around, left.inside_bounding_box, left.below
            )),
        ]
        .spacing(6);
        for line in coverage_lines(cover) {
            block = block.push(muted(line));
        }
        c = c.push(block);
    }
    if !outcome.proposal.unmachined.is_empty() {
        let mut block = column![bold("Not machined (as listed by the planner)", 15)].spacing(6);
        for (i, item) in outcome.proposal.unmachined.iter().enumerate() {
            let ([feature, reason], more) = disclosure(
                app,
                Expand::Unmachined(i),
                [(&item.feature, 80), (&item.reason, 240)],
            );
            block = block.push(text(format!("• {feature}: {reason}")).size(13));
            if let Some(more) = more {
                block = block.push(more);
            }
        }
        if outcome.more_unmachined > 0 {
            block = block.push(muted(format!("• and {} more", outcome.more_unmachined)));
        }
        c = c.push(block);
    }
    if !outcome.proposal.warnings.is_empty() {
        let mut block = column![bold("Warnings from the planner", 15)].spacing(6);
        for (i, warning) in outcome.proposal.warnings.iter().enumerate() {
            let ([warning], more) = disclosure(app, Expand::Warning(i), [(warning, 240)]);
            block = block.push(text(format!("• {warning}")).size(13));
            if let Some(more) = more {
                block = block.push(more);
            }
        }
        if outcome.more_warnings > 0 {
            block = block.push(muted(format!("• and {} more", outcome.more_warnings)));
        }
        c = c.push(block);
    }
    if let (Some(view), Some(mesh)) = (&preview.view, app.imported_mesh.as_ref()) {
        let setup = job.setups.iter().find(|s| s.id == view.plan.setup);
        let operation =
            setup.and_then(|s| s.operations.iter().find(|o| o.id == view.plan.operation));
        let scene: Element<viewport::Event> = Viewport {
            setup,
            operation,
            mesh,
            simulation: Some(view),
            sample: false,
            camera: app.plan_camera,
            picking: false,
            pin_holes: Default::default(),
            hole_outlines: vec![],
            pin_aim: Default::default(),
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            first_setup: None,
            framed: vec![],
            leftover: None,
            live: None,
            a: None,
            live_stock: None,
            start_stock: None,
            live_volume: None,
            uncut: &[],
            uncut_pieces: &[],
            show_uncut: true,
            section: Default::default(),
            frame: None,
        }
        .view(380);
        c = c
            .push(bold("Stock after the whole plan", 15))
            .push(muted("Drag to orbit, right-drag to pan, scroll to zoom."))
            .push(
                container(scene.map(Message::PlanViewport))
                    .width(Fill)
                    .style(panel_style),
            );
    }
    let applying = if app.plan_asked.is_some() && has_operations(app) {
        CHANGE_NOTE.to_string()
    } else {
        apply_note(&app.doc, job)
    };
    c.push(note(applying)).push(actions()).into()
}
/// A failed run's problems as page lines.
pub(crate) fn problem_lines(problems: &[crate::proposal::Problem]) -> Vec<String> {
    let mut lines: Vec<String> = problems
        .iter()
        .take(12)
        .map(|problem| format!("• {}", problem.line()))
        .collect();
    if problems.len() > 12 {
        let more = problems.len() - 12;
        lines.push(format!(
            "• and {more} more problem{}",
            if more == 1 { "" } else { "s" }
        ));
    }
    lines
}
/// The mark beside a timeline row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mark {
    /// The step is running: a bullet.
    Open,
    /// The step ended, and a check on it, if any, passed: a tick.
    Done,
    /// The step ended with a check that found problems, a reply that could
    /// not be read, or no result it should have had: an amber "!".
    Warn,
}
/// One row of the planner's timeline.
pub(crate) struct TimelineRow {
    pub mark: Mark,
    /// The step and how long it took (so far, while it runs).
    pub text: String,
    /// Under it: what the model has written while a request runs, or how its
    /// check went. Empty in the finished view, which puts a summary in `text`.
    pub notes: Vec<String>,
    /// Under the notes, while the run goes: the pictures the request's model
    /// looked at, and the refusals it met. Empty in the finished view, which
    /// counts them in `text`.
    pub looks: Vec<crate::Shown>,
}
/// A step that has ended, in the past tense; `stage_text` words the one that
/// is running.
pub(crate) fn done_text(stage: crate::planner::Progress, model: &str) -> String {
    use crate::planner::{MAX_REQUESTS, Progress};
    match stage {
        Progress::Analyzing => "Analyzed the part".into(),
        Progress::Asking(n) => format!("Asked {model} for a plan ({n} of {MAX_REQUESTS})"),
        Progress::Repairing(n) => {
            format!("Asked {model} for a correction ({n} of {MAX_REQUESTS})")
        }
        Progress::Extending(n) => {
            format!("Asked {model} about faces still covered ({n} of {MAX_REQUESTS})")
        }
        Progress::Checking(n) => format!("Checked the plan ({n} of {MAX_REQUESTS})"),
        Progress::Previewing => "Built the 3D view".into(),
    }
}
/// Why the ended step `i` has no result it should have: a check that did not
/// finish, a request that no check follows (so the model never replied), or
/// an analysis that nothing follows. None for a step that went through.
fn lost(steps: &[crate::Step], i: usize) -> Option<&'static str> {
    use crate::planner::Progress::*;
    let step = &steps[i];
    if step.ended.is_none() || step.result.is_some() {
        return None;
    }
    let next = steps.get(i + 1).map(|next| next.stage);
    match step.stage {
        Checking(_) => Some("did not finish"),
        Asking(n) | Repairing(n) | Extending(n) if next != Some(Checking(n)) => Some("no reply"),
        Analyzing if next.is_none() => Some("did not finish"),
        _ => None,
    }
}
/// How a reply fared, in a few words.
fn step_summary(result: &crate::planner::StepResult) -> String {
    use crate::planner::StepResult;
    match result {
        StepResult::Passed => "passed".into(),
        StepResult::Problems { shown, more } => match shown.len() + more {
            0 => "did not pass".into(),
            n => plural(n, "problem"),
        },
        StepResult::Unreadable(_) => "reply could not be read".into(),
    }
}
/// The most setups a draft lists; the rest are counted.
const DRAFT_SETUPS: usize = 6;
/// What the model has written of its reply so far, as timeline lines.
pub(crate) fn draft_lines(draft: &crate::proposal::Draft) -> Vec<String> {
    // A summary whose opening quote has just arrived is not written yet.
    let summary = draft
        .summary
        .as_deref()
        .map(|summary| clip(summary, 300))
        .filter(|summary| !summary.is_empty());
    if summary.is_none() && draft.setups.is_empty() {
        return vec!["Thinking: the model has not started writing yet.".into()];
    }
    let mut lines = vec![];
    if let Some(summary) = summary {
        lines.push(format!("Plan: {summary}"));
    }
    let last = draft.setups.len().saturating_sub(1);
    for (i, setup) in draft.setups.iter().enumerate().take(DRAFT_SETUPS) {
        let mut parts = vec![format!("Setup {}", i + 1)];
        if let Some(up) = &setup.orientation {
            parts.push(format!("{} up", clip(up, 4)));
        }
        let n = setup.operations.len();
        for (k, op) in setup.operations.iter().enumerate() {
            let mut words = format!(
                "{} \"{}\"",
                clip(op.strategy.as_deref().unwrap_or("…"), 20),
                clip(op.name.as_deref().unwrap_or("…"), 40)
            );
            if let Some(t) = op.tool {
                words += &format!(" T{t}");
            }
            // The operation still being written also shows the numbers so far.
            if draft.writing && i == last && k + 1 == n {
                if let Some(rpm) = op.rpm {
                    words += &format!(", {rpm:.0} rpm");
                }
                if let Some(feed) = op.feed {
                    words += &format!(", {feed:.0} mm/min");
                }
                words = format!("writing: {words}");
            }
            parts.push(words);
        }
        lines.push(parts.join(" · "));
    }
    if draft.setups.len() > DRAFT_SETUPS {
        let more = draft.setups.len() - DRAFT_SETUPS;
        lines.push(format!(
            "and {more} more setup{}",
            if more == 1 { "" } else { "s" }
        ));
    }
    lines
}
/// A picture the model looked at as one line; a look with no picture is an
/// error sentence and reads bare.
pub(crate) fn look_line(look: &crate::planner::Look) -> String {
    match (&look.png, look.purpose.is_empty()) {
        (Some(_), false) => format!(
            "Looked: {} — \"{}\"",
            clip(&look.description, 200),
            clip(&look.purpose, 120)
        ),
        (Some(_), true) => format!("Looked: {}", clip(&look.description, 200)),
        (None, _) => clip(&look.description, 300),
    }
}
/// The timeline's rows as of `now`. While the run goes (`finished` false) a
/// check's problems and an unreadable reply's error are listed under their
/// row, and the open request shows what the model has written; a finished
/// run's rows are one line each, ending in a short summary, the number of
/// pictures looked at and what OpenAI refused. A step that ended without the result it should have
/// gets the warning mark and says why (see `lost`).
pub(crate) fn timeline_rows(
    app: &App,
    now: std::time::Instant,
    finished: bool,
) -> Vec<TimelineRow> {
    use crate::planner::{Progress, StepResult};
    let request = |stage| {
        matches!(
            stage,
            Progress::Asking(_) | Progress::Repairing(_) | Progress::Extending(_)
        )
    };
    let open_request = |step: &crate::Step| step.ended.is_none() && request(step.stage);
    // Read the text only while a request is open, and not for longer than
    // the parse takes: the reply goes on streaming into it.
    let draft = if app.plan_timeline.iter().any(open_request) {
        let text = app
            .plan_stream
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        draft_lines(&crate::proposal::partial(&text))
    } else {
        vec![]
    };
    app.plan_timeline
        .iter()
        .enumerate()
        .map(|(i, step)| {
            let lost = lost(&app.plan_timeline, i);
            let took = step
                .ended
                .unwrap_or(now)
                .saturating_duration_since(step.started);
            let head = step_text(app, step);
            let mut text = format!("{head} · {:.0} s", took.as_secs_f32());
            let mut notes = vec![];
            if open_request(step) {
                notes.extend(draft.iter().cloned());
            }
            match (&step.result, finished) {
                (None, true) => {
                    if let Some(why) = lost {
                        text = format!("{text} · {why}");
                    }
                }
                (None, false) => notes.extend(lost.map(String::from)),
                (Some(result), true) => text = format!("{text} · {}", step_summary(result)),
                (Some(StepResult::Passed), false) => notes.push("passed".into()),
                (Some(StepResult::Problems { shown, more }), false) => {
                    notes.extend(shown.iter().map(|line| format!("• {line}")));
                    if *more > 0 {
                        notes.push(format!("• and {more} more"));
                    }
                    if notes.is_empty() {
                        notes.push("did not pass".into());
                    }
                }
                (Some(StepResult::Unreadable(why)), false) => {
                    notes.push(format!("The reply could not be read: {}", clip(why, 300)))
                }
            }
            if finished {
                // The one line says what the request was sent again without
                // and ends with what it saw; the lines themselves are shown
                // only while the run goes.
                for warning in step.looks.iter().filter(|l| l.warn) {
                    let said = [Dropped::Tool, Dropped::Pictures]
                        .map(refusal)
                        .into_iter()
                        .find(|(line, _)| *line == warning.line)
                        .map_or(warning.line.as_str(), |(_, short)| short);
                    text = format!("{text} · {said}");
                }
                let seen = step.looks.iter().filter(|l| l.thumb.is_some()).count();
                if seen > 0 {
                    text = format!("{text} · {}", plural(seen, "look"));
                }
            }
            let mark = match (step.ended, &step.result) {
                (None, _) => Mark::Open,
                (Some(_), Some(StepResult::Problems { .. } | StepResult::Unreadable(_))) => {
                    Mark::Warn
                }
                (Some(_), None) if lost.is_some() => Mark::Warn,
                // A refusal is worth a mark when nothing else is said of the row.
                (Some(_), None) if step.looks.iter().any(|l| l.warn) => Mark::Warn,
                (Some(_), _) => Mark::Done,
            };
            TimelineRow {
                mark,
                text,
                notes,
                looks: if finished { vec![] } else { step.looks.clone() },
            }
        })
        .collect()
}
/// The timeline as page rows, or None before the run's first stage.
fn timeline_view<'a>(app: &App, finished: bool) -> Option<Column<'a, Message>> {
    let rows = timeline_rows(app, std::time::Instant::now(), finished);
    if rows.is_empty() {
        return None;
    }
    let mut c = column![].spacing(6);
    for (step, row) in rows.into_iter().enumerate() {
        let mark = match row.mark {
            Mark::Open => text("•").size(13).color(MUTED),
            Mark::Done => text("✓").size(13).color(GOOD),
            Mark::Warn => text("!").size(13).color(WARN),
        };
        let mut item = column![text(row.text).size(13)].spacing(2);
        for line in row.notes {
            item = item.push(muted(line));
        }
        for (index, look) in row.looks.into_iter().enumerate() {
            let line = if look.warn {
                text(look.line).size(12).color(WARN)
            } else {
                muted(look.line)
            };
            item = item.push(match look.thumb {
                Some(handle) => Element::from(
                    row![
                        thumbnail(handle, 120, PlanPicture::Look { step, look: index }),
                        line
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                ),
                None => line.into(),
            });
        }
        c = c.push(row![mark, item].spacing(8));
    }
    Some(c)
}
/// A picture of the running page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanPicture {
    /// The overview the first request carries.
    Overview,
    /// Picture `look` of the timeline's step `step`.
    Look { step: usize, look: usize },
}
/// A picture shrunk to `width`: a click opens it at full size, or closes it.
fn thumbnail<'a>(
    handle: widget::image::Handle,
    width: u32,
    picture: PlanPicture,
) -> Element<'a, Message> {
    button(widget::image(handle).width(width))
        .on_press(Message::PlanPicture(picture))
        .padding(0)
        .style(|_, _| button::Style::default())
        .into()
}
/// The picture `picture` stands for, if the timeline still has it.
fn shown_picture(app: &App, picture: PlanPicture) -> Option<&crate::Shown> {
    match picture {
        PlanPicture::Overview => app.plan_overview.as_ref(),
        PlanPicture::Look { step, look } => app.plan_timeline.get(step)?.looks.get(look),
    }
}
/// The picture the user opened, at its natural size (the planner's pictures
/// are 768 px) in a panel that scrolls both ways, with a Close button.
fn enlarged_view<'a>(app: &App) -> Option<Element<'a, Message>> {
    let picture = app.plan_enlarged?;
    let shown = shown_picture(app, picture)?;
    let handle = shown.thumb.clone()?;
    let both = scrollable::Direction::Both {
        vertical: scrollable::Scrollbar::default(),
        horizontal: scrollable::Scrollbar::default(),
    };
    Some(
        container(
            column![
                row![
                    bold("The picture at full size", 15),
                    space().width(Fill),
                    secondary("Close", Message::PlanPicture(picture))
                ]
                .align_y(Alignment::Center),
                muted(shown.line.clone()),
                scrollable(widget::image(handle))
                    .direction(both)
                    .width(Fill)
                    .height(Length::Fixed(420.))
            ]
            .spacing(8),
        )
        .padding(16)
        .width(Fill)
        .style(panel_style)
        .into(),
    )
}
/// The id of the scrollable that holds the timeline.
const TIMELINE: &str = "plan-timeline";
/// A finished run's timeline under `heading`, in a panel: one line a step,
/// in a list that scrolls when it is longer than the panel.
fn finished_timeline<'a>(app: &App, heading: &'static str) -> Option<Element<'a, Message>> {
    let rows = timeline_view(app, true)?;
    let height = (app.plan_timeline.len() as f32 * 26.).min(240.);
    Some(
        container(
            column![
                bold(heading, 15),
                scrollable(container(rows).padding(iced::Padding::default().right(14)))
                    .id(TIMELINE)
                    .width(Fill)
                    .height(Length::Fixed(height))
            ]
            .spacing(8),
        )
        .padding(16)
        .width(Fill)
        .style(panel_style)
        .into(),
    )
}
/// The running page: the stage, the time since the run started and Cancel
/// stay put above; the picture opened at full size, if any, follows; the
/// overview picture and the timeline scroll below.
fn running_view(app: &App, stage: crate::planner::Progress) -> Element<'_, Message> {
    let elapsed = app.plan_timeline.first().map_or(0., |first| {
        std::time::Instant::now()
            .saturating_duration_since(first.started)
            .as_secs_f32()
    });
    let mut pinned = column![
        bold(running_text(app, stage), 20),
        muted(format!("{elapsed:.0} s since the run started")),
    ]
    .spacing(10);
    if let Some(asked) = &app.plan_asked {
        pinned = pinned.push(muted(format!("You asked: {}", clip(asked, 300))));
    }
    pinned = pinned
        .push(muted(if edits_plan(app) {
            "This can take a few minutes. Nothing changes until you apply it."
        } else {
            "This can take a few minutes. Nothing changes until you apply a plan."
        }))
        .push(secondary("Cancel", Message::PlanCancel));
    let pictures = app
        .plan_overview
        .as_ref()
        .is_some_and(|o| o.thumb.is_some())
        || app
            .plan_timeline
            .iter()
            .any(|step| step.looks.iter().any(|l| l.thumb.is_some()));
    if pictures {
        pinned = pinned.push(muted("Click a picture to see it at full size."));
    }
    let mut scrolled = column![].spacing(14);
    if let Some(overview) = &app.plan_overview {
        let mut picture = column![bold("The AI starts with this picture", 15)].spacing(6);
        if let Some(handle) = &overview.thumb {
            picture = picture.push(thumbnail(handle.clone(), 240, PlanPicture::Overview));
        }
        scrolled = scrolled.push(picture.push(muted(overview.line.clone())));
    }
    if let Some(rows) = timeline_view(app, false) {
        scrolled = scrolled.push(rows);
    }
    let mut c = column![pinned].spacing(14);
    if let Some(enlarged) = enlarged_view(app) {
        c = c.push(enlarged);
    }
    container(
        c.push(
            scrollable(container(scrolled).padding(iced::Padding::default().right(14)))
                .id(TIMELINE)
                .width(Fill)
                .height(Fill),
        ),
    )
    .padding(35)
    .width(Fill)
    .height(Fill)
    .style(panel_style)
    .into()
}
/// Whether the open job has an operation, so the planner can change a plan.
fn has_operations(app: &App) -> bool {
    app.doc.setups.iter().any(|s| !s.operations.is_empty())
}
/// The intro card's last element: the Plan button, with a line saying it
/// replaces a plan the job already has, or what the checklist still lacks.
fn plan_this_job(app: &App) -> Element<'_, Message> {
    if !app.plan_checklist().iter().all(|line| line.ok) {
        return muted("Complete the checklist on the left to plan.").into();
    }
    let button = primary("Plan this job", Message::Plan);
    if has_operations(app) {
        column![
            muted("Plan this job replaces the current plan; to change it instead, ask below."),
            button
        ]
        .spacing(8)
        .into()
    } else {
        button.into()
    }
}
/// The chat with the planner: the exchanges so far, the box for the next
/// request and Send. Shown while no run goes.
fn plan_chat(app: &App) -> Element<'_, Message> {
    let ready = app.plan_checklist().iter().all(|line| line.ok);
    let send = (ready && !app.plan_request.trim().is_empty()).then_some(Message::PlanSend);
    let previewing = matches!(app.planner, crate::Planner::Preview(_));
    let editing = previewing || has_operations(app);
    let mut c = column![bold("Talk to the planner", 15)].spacing(8);
    for turn in &app.plan_chat {
        c = c
            .push(text(format!("You: {}", clip(&turn.request, 400))).size(13))
            .push(muted(format!("Planner: {}", clip(&turn.answer, 400))));
    }
    let mut input = text_input(
        if editing {
            "Ask for a change, e.g. use T3 for the pocket, or add a finishing pass"
        } else {
            "Tell the planner what to plan, then Send"
        },
        &app.plan_request,
    )
    .on_input(Message::PlanAsk)
    .padding([7, 9])
    .size(13);
    if let Some(send) = &send {
        input = input.on_submit(send.clone());
    }
    c = c
        .push(
            row![
                input,
                match send {
                    Some(send) => Element::from(primary("Send", send)),
                    None => act("Send", None, false),
                }
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .push(muted(if previewing {
            "A change asked now starts from this preview. Nothing changes until you apply."
        } else if editing {
            "The planner changes the current plan and keeps what you do not mention. Nothing changes until you apply."
        } else {
            "Or press Plan this job to plan without a request."
        }));
    container(c)
        .padding(16)
        .width(Fill)
        .style(panel_style)
        .into()
}
fn plan_result(app: &App) -> Element<'_, Message> {
    use crate::Planner;
    match &app.planner {
        Planner::Idle => column![
            container(
            column![
                bold("One click plans the whole job", 22),
                muted(
                    "1. The app describes the part: faces, pockets and holes in six orientations"
                ),
                muted("2. The model proposes setups, operations, tools and cutting parameters"),
                muted(
                    "3. The app generates and simulates every operation and runs the export checks, or a UC100 machine's run checks"
                ),
                muted("4. Problems go back to the model for correction, up to four requests"),
                muted("5. You review the result and apply it; nothing changes before that"),
                plan_this_job(app)
            ]
            .spacing(16),
        )
        .padding(35)
        .width(Fill)
        .style(panel_style),
            plan_chat(app)
        ]
        .spacing(12)
        .into(),
        Planner::Running(stage) => running_view(app, *stage),
        // A finished run puts its outcome and the actions first and the
        // timeline under them.
        Planner::Failed(failure) => {
            let mut c = column![
                plan_chat(app),
                bold(
                    if edits_plan(app) {
                        "The change could not be made"
                    } else {
                        "No plan this time"
                    },
                    21
                ),
                text(failure.message.clone()).size(13)
            ]
            .spacing(12);
            for line in problem_lines(&failure.problems) {
                c = c.push(muted(line));
            }
            c = c.push(
                row![
                    primary("Try again", Message::Plan),
                    secondary("Dismiss", Message::DiscardPlan)
                ]
                .spacing(10),
            );
            if let Some(tried) = finished_timeline(app, "What the planner tried") {
                c = c.push(tried);
            }
            c.into()
        }
        Planner::Preview(preview) => match finished_timeline(app, "How the plan was made") {
            Some(made) => column![plan_chat(app), plan_preview(app, preview), made]
                .spacing(12)
                .into(),
            None => column![plan_chat(app), plan_preview(app, preview)]
                .spacing(12)
                .into(),
        },
    }
}
/// The page's right side. A running plan fills it and keeps its stage and
/// Cancel in view, scrolling only the timeline (`running_view`); the rest
/// scroll as a whole.
pub(crate) fn plan_side(app: &App) -> Element<'_, Message> {
    if matches!(app.planner, crate::Planner::Running(_)) {
        return container(plan_result(app))
            .padding(iced::Padding::default().right(14))
            .width(Fill)
            .height(Fill)
            .into();
    }
    scrollable(container(plan_result(app)).padding(iced::Padding::default().right(14)))
        .id("planner-result")
        .height(Fill)
        .width(Fill)
        .into()
}
fn ai(app: &App) -> Element<'_, Message> {
    container(
        column![
            page_header(
                "AI machining planner",
                "Sign in with ChatGPT, then let the planner propose and check a complete job."
            ),
            row![
                container(scrollable(
                    container(
                        column![
                            account_panel(app),
                            rule::horizontal(1),
                            planner_controls(app)
                        ]
                        .spacing(18)
                    )
                    .padding(iced::Padding::default().right(14))
                ))
                .padding(22)
                .width(370)
                .height(Fill)
                .style(panel_style),
                plan_side(app)
            ]
            .spacing(24)
            .height(Fill)
        ]
        .spacing(24),
    )
    .padding(28)
    .height(Fill)
    .into()
}
/// Which controllers the job's setups use, as far as the NC page words
/// itself: LinuxCNC export, a run on a UC100 machine, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NcMachine {
    LinuxCnc,
    Uc100,
    Mixed,
}
pub(crate) fn nc_machine(app: &App) -> NcMachine {
    let uc100 = |s: &crate::model::Setup| get(&s.machine.p, "controller") == "UC100";
    let count = app.doc.setups.iter().filter(|s| uc100(s)).count();
    match count {
        0 => NcMachine::LinuxCnc,
        n if n == app.doc.setups.len() => NcMachine::Uc100,
        _ => NcMachine::Mixed,
    }
}
/// The NC page's note on what a UC100 run checks, for setups that model their
/// vise or clamps as `held` says.
pub(crate) fn run_note(held: Held) -> &'static str {
    match held {
        Held::No => {
            "A UC100 machine takes no G-code file: each setup is checked as a run checks it before Cycle Start — limits, travel, rapids into material, holder clearance, the profile's spindle and coolant outputs — and runs from MACHINE. The checks that need the connected machine, its work zero and position, are made there. The planner's tool-axis and gouge checks covered the plan as applied, not later edits. Fixtures, clamps and the machine body are not checked: air-cut first."
        }
        Held::Every => {
            "A UC100 machine takes no G-code file: each setup is checked as a run checks it before Cycle Start — limits, travel, rapids into material, holder clearance, the profile's spindle and coolant outputs — and runs from MACHINE. The checks that need the connected machine, its work zero and position, are made there. The planner's tool-axis and gouge checks covered the plan as applied, not later edits. Every program move, the links between operations, approaches and retracts included, is checked against each setup's vise jaws or clamps as boxes with a clearance gap; parallels, bolts, the table, the machine body and tool-change motion are not checked: air-cut first."
        }
        Held::Mixed => {
            "A UC100 machine takes no G-code file: each setup is checked as a run checks it before Cycle Start — limits, travel, rapids into material, holder clearance, the profile's spindle and coolant outputs — and runs from MACHINE. The checks that need the connected machine, its work zero and position, are made there. The planner's tool-axis and gouge checks covered the plan as applied, not later edits. Every program move of a setup that models a vise or clamps, the links between operations, approaches and retracts included, is checked against its jaws or clamps as boxes with a clearance gap; parallels, bolts, the table, the machine body and tool-change motion are not checked, and neither are fixtures and clamps in the setups whose workholding is not modelled or cannot be checked: air-cut first."
        }
    }
}
/// What a UC100 setup's card lists as needed before Cycle Start, beside the
/// checks it has passed.
pub(crate) const RUN_PREREQUISITES: &str = "To run: connect the machine, set machine zero and the work zero, and have the tool run inside the soft limits at that zero; Cycle Start stays grey with the reasons listed until it can start.";
/// Why a UC100 setup has no export file, as plain explanation.
pub(crate) const NO_EXPORT_FILE: &str =
    "A UC100 machine takes no G-code file: this setup runs from MACHINE.";
/// One setup's card on the NC page.
pub(crate) enum NcCard<'a> {
    /// A LinuxCNC setup whose programs were produced.
    Programs(&'a [crate::post::Program]),
    /// A LinuxCNC setup that failed its export checks.
    Refused(&'a [String]),
    /// A UC100 setup: the run checks' problems (none when it passes) and the
    /// setup Run on the machine opens.
    Run {
        problems: &'a [String],
        setup: usize,
    },
    /// A manual mill's setup: no program, its steps are on the MACHINE page.
    Manual,
}
/// The NC review page, before it is drawn: its words and its setups' cards.
pub(crate) struct NcPage<'a> {
    pub title: &'static str,
    pub intro: &'static str,
    pub heading: &'static str,
    pub note: &'static str,
    pub errors: &'a [String],
    /// Each setup's name with its card; empty before the checks are in.
    pub cards: Vec<(&'a str, NcCard<'a>)>,
    pub machine: NcMachine,
    pub ready: bool,
}
#[cfg(test)]
impl NcPage<'_> {
    /// Every problem the page shows as an error.
    pub(crate) fn problems(&self) -> Vec<&str> {
        let cards = self.cards.iter().flat_map(|(_, card)| match card {
            NcCard::Refused(p) | NcCard::Run { problems: p, .. } => p.iter(),
            NcCard::Programs(_) | NcCard::Manual => [].iter(),
        });
        self.errors
            .iter()
            .chain(cards)
            .map(String::as_str)
            .collect()
    }
    /// The setups whose card offers Run on the machine.
    pub(crate) fn run_buttons(&self) -> Vec<usize> {
        (self.cards.iter())
            .filter_map(|(_, card)| match card {
                NcCard::Run { setup, .. } => Some(*setup),
                _ => None,
            })
            .collect()
    }
}
pub(crate) fn nc_page(app: &App) -> NcPage<'_> {
    let machine = nc_machine(app);
    let (title, intro, heading, note) = match machine {
        NcMachine::LinuxCnc => (
            "NC review",
            "Check each setup, then export LinuxCNC G-code.",
            "Setup checks",
            program_note(held(app)),
        ),
        NcMachine::Uc100 => (
            "Run on the machine",
            "Check each setup for a run on the UC100, then run it from MACHINE. A UC100 machine takes no G-code file.",
            "Run checks",
            run_note(held(app)),
        ),
        NcMachine::Mixed => (
            "NC review",
            "Check each setup: a LinuxCNC setup exports G-code, a UC100 setup runs on the machine.",
            "Setup checks",
            program_note(held(app)),
        ),
    };
    let mut page = NcPage {
        title,
        intro,
        heading,
        note,
        errors: &[],
        cards: vec![],
        machine,
        ready: false,
    };
    if let Some(job) = &app.post {
        page.errors = &job.errors;
        page.ready = job.ready();
        for s in &job.setups {
            let uc100 = (app.doc.setups.get(s.setup))
                .is_some_and(|x| get(&x.machine.p, "controller") == "UC100");
            let manual =
                (app.doc.setups.get(s.setup)).is_some_and(|x| crate::manual::is_hand(&x.machine));
            let card = if manual {
                NcCard::Manual
            } else if uc100 {
                NcCard::Run {
                    problems: match &s.run {
                        Some(Err(problems)) => problems,
                        _ => &[],
                    },
                    setup: s.setup,
                }
            } else {
                match &s.result {
                    Ok(programs) => NcCard::Programs(programs),
                    Err(problems) => NcCard::Refused(problems),
                }
            };
            page.cards.push((s.name.as_str(), card));
        }
    }
    page
}
fn post(app: &App) -> Element<'_, Message> {
    let page = nc_page(app);
    let mut summary = column![bold(page.heading, 20), note(page.note),].spacing(14);
    let lines = speed_feed_lines(app);
    if !lines.is_empty() {
        summary = summary.push(caution(
            speed_feed_heading(&lines),
            lines.into_iter().map(|(_, line)| line).collect(),
        ));
    }
    let mut code = column![].spacing(18);
    let mut shown = 0;
    match &app.post {
        None => {
            summary = summary.push(muted(if app.post_busy {
                "Checking every setup and replaying its program…"
            } else {
                "Checks run when this page opens."
            }))
        }
        Some(_) => {
            if !page.errors.is_empty() {
                summary = summary.push(warnings(page.errors.to_vec()));
            }
            for (name, card) in &page.cards {
                match card {
                    NcCard::Programs(programs) => {
                        for program in programs.iter() {
                            summary = summary.push(
                                column![
                                    bold(format!("✓  {}", program.file_name), 13),
                                    muted(format!(
                                        "{} lines · {} · {}",
                                        program.lines,
                                        program.tools.join(", "),
                                        program.operations.join(" → ")
                                    ))
                                ]
                                .spacing(4),
                            );
                            shown += 1;
                            code = code.push(
                                text(crate::post::preview(&program.text, 120))
                                    .font(Font::MONOSPACE)
                                    .size(12)
                                    .color(Color::from_rgb8(183, 211, 221)),
                            );
                        }
                    }
                    NcCard::Refused(problems) => {
                        summary = summary
                            .push(bold(format!("!  {name}"), 13))
                            .push(warnings(problems.to_vec()));
                    }
                    NcCard::Manual => {
                        let line = match app.doc.setups.iter().find(|x| x.name() == *name) {
                            Some(x) => crate::manual::no_gcode_on(&x.machine, name),
                            None => crate::manual::no_gcode(name),
                        };
                        summary = summary.push(muted(line));
                    }
                    NcCard::Run { problems, setup } => {
                        let mark = if problems.is_empty() { "✓" } else { "!" };
                        summary = summary.push(bold(format!("{mark}  Run on UC100: {name}"), 13));
                        if !problems.is_empty() {
                            summary = summary.push(warnings(problems.to_vec()));
                        }
                        summary = summary
                            .push(muted(RUN_PREREQUISITES))
                            .push(muted(NO_EXPORT_FILE))
                            .push(
                                primary("Run on the machine", Message::RunOnMachine(*setup))
                                    .width(Fill),
                            );
                    }
                }
            }
        }
    }
    summary = if page.ready {
        summary.push(primary("Export LinuxCNC G-code…", Message::ExportGcode).width(Fill))
    } else {
        summary.push(muted(match page.machine {
            NcMachine::LinuxCnc => "Export becomes available when every setup passes.",
            NcMachine::Uc100 => "A UC100 machine takes no G-code file: run each setup on the machine.",
            NcMachine::Mixed => {
                "Export becomes available when every setup passes; a UC100 setup takes no G-code file and runs on the machine."
            }
        }))
    };
    summary = summary
        .push(secondary("Export setup sheet (.md)", Message::Export("sheet")).width(Fill))
        .push(secondary("Save complete project (.json)", Message::SaveProject).width(Fill));
    if shown == 0 {
        // An empty dark pane reads as a failed load: say what will be here.
        code = code.push(
            text(if app.post_busy {
                "Writing the programs…"
            } else if page.machine == NcMachine::Uc100 {
                "A UC100 machine takes no G-code file: each setup's program goes to the machine from MACHINE, through Run on the machine."
            } else {
                "Each setup's G-code appears here once its checks pass."
            })
            .size(13)
            .color(Color::from_rgb8(150, 176, 190)),
        );
    }
    let code = container(scrollable(code).height(Fill))
        .padding(22)
        .width(Fill)
        .height(Fill)
        .style(|_| container::Style {
            background: Some(Color::from_rgb8(29, 43, 55).into()),
            border: Border {
                radius: 5.into(),
                ..Default::default()
            },
            ..Default::default()
        });
    container(
        column![
            page_header(page.title, page.intro),
            row![
                container(scrollable(summary))
                    .padding(22)
                    .width(375)
                    .height(Fill)
                    .style(panel_style),
                code
            ]
            .spacing(22)
            .height(Fill)
        ]
        .spacing(22),
    )
    .padding(28)
    .height(Fill)
    .into()
}

/// An audit report's row: a panel, shaded under the pointer.
fn audit_row(status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(if hovered { HOVER_ROW } else { Color::WHITE }.into()),
        text_color: INK,
        border: Border {
            color: LINE,
            width: 1.,
            radius: 5.into(),
        },
        ..Default::default()
    }
}
/// The generation under way off the window's thread, if any: what is being
/// generated, how far it has got, a progress bar and Cancel. The simulation
/// panel and the operation audit show it.
fn generation_progress(app: &App) -> Option<Element<'_, Message>> {
    let g = app.generating.as_ref()?;
    let done = g.progress.fraction();
    let doing = g.progress.label();
    let what = match g.what {
        crate::Generation::All => "Generating every enabled operation",
        crate::Generation::Operation(..) => "Generating",
        crate::Generation::Layout(..) => "Generating the setup's toolpaths for its holding",
    };
    Some(
        column![
            row![
                muted(if doing.is_empty() {
                    format!("{what}…")
                } else {
                    format!("{what} · {doing}")
                })
                .size(12)
                .width(Fill),
                muted(format!("{:.0}%", done * 100.)).size(12),
                secondary("Cancel", Message::CancelGenerate),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            widget::progress_bar(0.0..=1.0, done).girth(6),
        ]
        .spacing(8)
        .into(),
    )
}
fn operation_audit(app: &App) -> Element<'_, Message> {
    let mut c=column![bold("Operation support & job checks",24),note("Available means an imported-mesh preview is implemented and regression-tested within the scope below. Their moves can be exported as LinuxCNC 3-axis programs after checks for limits, rapids into material and holder clearance, and, in a setup that models a vise or clamps, for the jaws and clamps as boxes with a clearance gap. Parallels, bolts, the table, the machine body and other fixtures are not checked. A sample animation does not establish operation support."),row![primary("Check all enabled operations",Message::GenerateAll),secondary("Back to machining",Message::Page(Page::Work))].spacing(12)].spacing(14).max_width(1020);
    if let Some(generating) = generation_progress(app) {
        c = c.push(generating);
    }
    for (i, row) in app.audit_report.iter().enumerate() {
        let strategy = (app.doc.setups.get(row.setup))
            .and_then(|s| s.operations.get(row.operation))
            .map(|o| o.strategy);
        let tab = match strategy {
            Some(strategy) => operation_issue_tab(strategy, &row.detail),
            None => issue_tab(&row.detail),
        };
        let where_ = match tab {
            Some(tab) => format!(
                "Opens {} in the editor, on its {tab} tab, where the setting its line names is.",
                row.name
            ),
            None => format!("Opens {} in the editor.", row.name),
        };
        c = c.push(help::wrap(
            button(
                column![
                    bold(
                        format!("{}  {}", if row.ok { "✓" } else { "!" }, row.name),
                        14
                    ),
                    text(&row.detail).size(12)
                ]
                .spacing(5),
            )
            .on_press(Message::AuditRow(i))
            .padding(12)
            .width(Fill)
            .style(|_, status| audit_row(status)),
            Help::text("Open the operation", where_),
            widget::tooltip::Position::Bottom,
        ));
    }
    for strategy in Strategy::ALL {
        c = c.push(
            container(
                row![
                    icon(app, strategy, 32.),
                    column![
                        bold(strategy.name(), 15),
                        text(strategy.availability()).size(12)
                    ]
                    .spacing(5)
                ]
                .spacing(16)
                .align_y(Alignment::Center),
            )
            .padding(12)
            .width(Fill)
            .style(panel_style),
        );
    }
    c=c.push(note("Remaining 3D stock carries through all enabled operations and setups, including flips. Fully cleared top layers are skipped automatically, and a milling path's stretches through air are replaced by a retract and a rapid where that is quicker. Leftover-region rest machining, collision checks beyond modelled vise jaws and clamps, multi-axis kinematics and controllers other than LinuxCNC remain unavailable. Unavailable strategies stay visible for planning, but cannot be added as imported-part machining operations."));
    container(scrollable(c))
        .padding(24)
        .width(Fill)
        .height(Fill)
        .into()
}

/// Reads a page as the widget tree lays it out, without a window: the texts
/// and scrollables in the order they sit on the page, and what a click at
/// each point of the page sends.
#[cfg(test)]
pub(crate) mod probe {
    use crate::Message;
    use iced::{
        Element, Event, Font, Pixels, Point, Size, mouse,
        widget::{self, Id},
    };
    use iced_runtime::{
        core::{clipboard, widget::Operation, widget::operation::Scrollable},
        user_interface::{Cache, UserInterface},
    };

    /// What the page holds, in tree order.
    #[derive(Debug, Clone, PartialEq)]
    pub(crate) enum Seen {
        Text(String),
        /// A scrollable: its id and the size of what it scrolls.
        Scroll(Option<Id>, Size),
    }
    struct Reader(Vec<Seen>);
    impl Operation for Reader {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self);
        }
        fn text(&mut self, _: Option<&Id>, _: iced::Rectangle, text: &str) {
            self.0.push(Seen::Text(text.to_string()));
        }
        fn scrollable(
            &mut self,
            id: Option<&Id>,
            _: iced::Rectangle,
            content: iced::Rectangle,
            _: iced::Vector,
            _: &mut dyn Scrollable,
        ) {
            self.0.push(Seen::Scroll(id.cloned(), content.size()));
        }
    }
    fn renderer() -> iced::Renderer {
        iced_renderer::fallback::Renderer::Secondary(iced_tiny_skia::Renderer::new(
            Font::DEFAULT,
            Pixels(16.),
        ))
    }
    /// The texts and scrollables of `page`, in tree order.
    pub(crate) fn seen(page: Element<'_, Message>, window: Size) -> Vec<Seen> {
        let mut renderer = renderer();
        let mut ui = UserInterface::build(page, window, Cache::default(), &mut renderer);
        let mut reader = Reader(vec![]);
        ui.operate(&renderer, &mut reader);
        reader.0
    }
    /// The texts of `page`, in tree order.
    pub(crate) fn texts(page: Element<'_, Message>, window: Size) -> Vec<String> {
        seen(page, window)
            .into_iter()
            .filter_map(|item| match item {
                Seen::Text(text) => Some(text),
                Seen::Scroll(..) => None,
            })
            .collect()
    }
    struct Bounds(Vec<(Option<Id>, iced::Rectangle)>);
    impl Operation for Bounds {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self);
        }
        fn scrollable(
            &mut self,
            id: Option<&Id>,
            bounds: iced::Rectangle,
            _: iced::Rectangle,
            _: iced::Vector,
            _: &mut dyn Scrollable,
        ) {
            self.0.push((id.cloned(), bounds));
        }
    }
    /// The scrollables of `page`: their ids and where they are.
    pub(crate) fn scrollables(
        page: Element<'_, Message>,
        window: Size,
    ) -> Vec<(Option<Id>, iced::Rectangle)> {
        let mut renderer = renderer();
        let mut ui = UserInterface::build(page, window, Cache::default(), &mut renderer);
        let mut bounds = Bounds(vec![]);
        ui.operate(&renderer, &mut bounds);
        bounds.0
    }
    /// What pressing the left button, then releasing it, gives at every
    /// `step` px of `area`, with the point it was pressed at.
    pub(crate) fn presses(
        page: Element<'_, Message>,
        window: Size,
        area: iced::Rectangle,
        step: f32,
    ) -> Vec<(Point, Message)> {
        presses_after(page, window, None, area, step)
    }
    /// `presses`, after the pointer came to `rest` and stayed there long
    /// enough for a hover card there to open.
    pub(crate) fn presses_after(
        page: Element<'_, Message>,
        window: Size,
        rest: Option<Point>,
        area: iced::Rectangle,
        step: f32,
    ) -> Vec<(Point, Message)> {
        let mut renderer = renderer();
        let mut ui = UserInterface::build(page, window, Cache::default(), &mut renderer);
        if let Some(at) = rest {
            let off = Point::new(-10., -10.);
            for (event, cursor) in [
                (mouse::Event::CursorMoved { position: off }, off),
                (mouse::Event::CursorMoved { position: at }, at),
            ] {
                let _ = ui.update(
                    &[Event::Mouse(event)],
                    mouse::Cursor::Available(cursor),
                    &mut renderer,
                    &mut clipboard::Null,
                    &mut vec![],
                );
            }
            std::thread::sleep(crate::help::HELP_DELAY + std::time::Duration::from_millis(100));
            let redraw = Event::Window(iced::window::Event::RedrawRequested(
                std::time::Instant::now(),
            ));
            let _ = ui.update(
                &[redraw],
                mouse::Cursor::Available(at),
                &mut renderer,
                &mut clipboard::Null,
                &mut vec![],
            );
        }
        let mut out = vec![];
        let mut x = area.x;
        while x < area.x + area.width {
            let mut y = area.y;
            while y < area.y + area.height {
                let at = Point::new(x, y);
                let events = [
                    Event::Mouse(mouse::Event::CursorMoved { position: at }),
                    Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                ];
                let mut messages = vec![];
                let _ = ui.update(
                    &events,
                    mouse::Cursor::Available(at),
                    &mut renderer,
                    &mut clipboard::Null,
                    &mut messages,
                );
                let _ = ui.update(
                    &[Event::Mouse(mouse::Event::ButtonReleased(
                        mouse::Button::Left,
                    ))],
                    mouse::Cursor::Available(at),
                    &mut renderer,
                    &mut clipboard::Null,
                    &mut messages,
                );
                out.extend(messages.into_iter().map(|m| (at, m)));
                y += step;
            }
            x += step;
        }
        out
    }
    /// The messages that pressing and releasing the left button gives, at
    /// every eighth point of `page`.
    pub(crate) fn clicks(page: Element<'_, Message>, window: Size) -> Vec<Message> {
        clicks_in(page, window, iced::Rectangle::with_size(window), 8.)
    }
    /// The messages that pressing and releasing the left button gives, at
    /// every `step` px of `area`.
    pub(crate) fn clicks_in(
        page: Element<'_, Message>,
        window: Size,
        area: iced::Rectangle,
        step: f32,
    ) -> Vec<Message> {
        let mut renderer = renderer();
        let mut ui = UserInterface::build(page, window, Cache::default(), &mut renderer);
        let mut messages = vec![];
        let mut x = area.x;
        while x < area.x + area.width {
            let mut y = area.y;
            while y < area.y + area.height {
                let at = Point::new(x, y);
                let events = [
                    Event::Mouse(mouse::Event::CursorMoved { position: at }),
                    Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                    Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                ];
                let _ = ui.update(
                    &events,
                    mouse::Cursor::Available(at),
                    &mut renderer,
                    &mut clipboard::Null,
                    &mut messages,
                );
                y += step;
            }
            x += step;
        }
        messages
    }
    /// The id a scrollable of the app is given by name.
    pub(crate) fn id(name: &'static str) -> Option<Id> {
        Some(widget::Id::new(name))
    }
}

#[cfg(test)]
mod tests {
    use crate::model::Strategy;
    use std::path::Path;

    /// The Control page is short under 860 px of height and narrow under
    /// 1360 px of width (LIO-275).
    #[test]
    fn the_control_page_is_short_and_narrow_from_the_window_size() {
        use super::Fit;
        let fit = |w, h| {
            let f = Fit::window(w, h);
            (f.short, f.narrow)
        };
        assert_eq!(fit(1920., 1080.), (false, false));
        assert_eq!(fit(1440., 930.), (false, false));
        assert_eq!(fit(1440., 900.), (false, false));
        assert_eq!(fit(1440., 860.), (false, false));
        assert_eq!(fit(1440., 859.), (true, false));
        assert_eq!(fit(1440., 790.), (true, false));
        assert_eq!(fit(1360., 900.), (false, false));
        assert_eq!(fit(1359., 900.), (false, true));
        assert_eq!(fit(1280., 690.), (true, true));
        assert_eq!(fit(1120., 720.), (true, true));
        // Each compact frame is smaller than the full one.
        let (full, short, narrow) = (
            Fit::window(1440., 900.),
            Fit::window(1440., 790.),
            Fit::window(1280., 900.),
        );
        assert!(short.machine_bar() < full.machine_bar() && short.run_bar() < full.run_bar());
        assert_eq!(narrow.machine_bar(), full.machine_bar());
        assert!(narrow.hands() < full.hands() && narrow.program() < full.program());
        assert_eq!(short.hands(), full.hands());
        // The status line under the page: 12 px text, 4 px over and under.
        const STATUS_BAR: f32 = 24.;
        // The left column's fixed cards leave the Spindle card its room
        // under the bars and over the status line, with and without A.
        for (w, h) in [
            (1440., 900.),
            (1440., 790.),
            (1280., 690.),
            (1120., 720.),
            (1120., 690.),
        ] {
            let fit = Fit::window(w, h);
            for axes in [3, 4] {
                let left = h
                    - STATUS_BAR
                    - fit.machine_bar()
                    - fit.run_bar()
                    - 4. * fit.gap()
                    - super::position_height(axes, fit)
                    - super::jog_height(axes > 3, fit);
                assert!(
                    left >= super::spindle_height(fit),
                    "{w} x {h}, {axes} axes: {left} px for the Spindle card"
                );
            }
        }
    }

    /// The window opens no larger than its screen leaves it, and never
    /// under its least size where the screen holds it.
    #[test]
    fn the_window_opens_no_larger_than_the_screen() {
        use crate::{MIN_WINDOW, least_window, opening_window};
        use iced::Size;
        assert_eq!(opening_window(Size::new(1920., 1080.)), None);
        assert_eq!(opening_window(Size::new(2560., 1440.)), None);
        assert_eq!(opening_window(Size::new(1440., 1040.)), None);
        // A 13" MacBook: 900 points, less the menu bar, the Dock and the
        // title bar.
        assert_eq!(
            opening_window(Size::new(1440., 900.)),
            Some(Size::new(1440., 790.))
        );
        assert_eq!(
            opening_window(Size::new(1366., 768.)),
            Some(Size::new(1366., 690.))
        );
        let fits = opening_window(Size::new(1280., 800.)).unwrap();
        assert_eq!(fits, Size::new(1280., 690.));
        // A screen narrower than the least window: the window is as wide as
        // the screen and can be made no wider (LIO-309).
        assert_eq!(
            opening_window(Size::new(1000., 900.)),
            Some(Size::new(1000., 790.))
        );
        assert_eq!(
            least_window(Size::new(1000., 900.)),
            Some(Size::new(1000., MIN_WINDOW.height))
        );
        assert_eq!(least_window(Size::new(1280., 800.)), None);
        assert_eq!(least_window(Size::new(1366., 768.)), None);
        assert_eq!(least_window(Size::new(1120., 768.)), None);
        // A screen too short for the least window and its bars: the window
        // is as high as the screen leaves it, and can be made no higher
        // (LIO-315). 1366 × 768 at 125%, 1920 × 1080 at 175% and 150%,
        // 1600 × 900 at 150%.
        assert_eq!(
            least_window(Size::new(1120., 767.)),
            Some(Size::new(1120., 689.))
        );
        for (screen, window) in [
            ((1093., 614.), (1093., 536.)),
            ((1097., 617.), (1097., 539.)),
            ((1280., 720.), (1280., 642.)),
            ((1067., 600.), (1067., 522.)),
        ] {
            let (screen, window) = (Size::new(screen.0, screen.1), Size::new(window.0, window.1));
            assert_eq!(opening_window(screen), Some(window), "{screen:?}");
            assert_eq!(
                least_window(screen),
                Some(Size::new(window.width.min(MIN_WINDOW.width), window.height)),
                "{screen:?}"
            );
        }
    }

    /// The window opens centred at the size it opens at, and on a short
    /// screen at its top, so that its title bar and its status bar are both
    /// on the screen (LIO-315).
    #[test]
    fn the_window_opens_on_the_screen() {
        use crate::{WINDOW, opening_place};
        use iced::{Point, Size};
        let at = |w, h| opening_place(WINDOW, Size::new(w, h));
        assert_eq!(at(2560., 1440.), Point::new(560., 255.));
        // Centred, its foot would be on the taskbar.
        assert_eq!(at(1920., 1080.), Point::new(240., 40.));
        assert_eq!(at(1440., 900.), Point::new(0., 0.));
        assert_eq!(at(1366., 768.), Point::new(0., 0.));
        assert_eq!(at(1093., 614.), Point::new(0., 0.));
        assert_eq!(at(1000., 900.), Point::new(0., 0.));
    }

    #[test]
    fn a_drill_operations_pass_and_helical_checks_open_its_cycle_tab() {
        // A Drill operation has no Passes tab: Helical milling's settings
        // are on its Cycle tab, as is why a manual mill cannot run it.
        let stepdown = "stepdown must be greater than zero.";
        assert_eq!(super::issue_tab(stepdown), Some("Passes"));
        assert_eq!(
            super::operation_issue_tab(Strategy::Pocket, stepdown),
            Some("Passes")
        );
        assert_eq!(
            super::operation_issue_tab(Strategy::Drill, stepdown),
            Some("Cycle")
        );
        assert_eq!(
            super::operation_issue_tab(Strategy::Drill, crate::manual::HELICAL_ON_MANUAL),
            Some("Cycle")
        );
        assert_eq!(
            super::operation_issue_tab(Strategy::Drill, crate::lathe::HELICAL_ON_LATHE),
            Some("Cycle")
        );
        assert_eq!(
            super::operation_issue_tab(Strategy::Drill, crate::model::HELICAL_PIN_DRILL),
            Some("Cycle")
        );
        assert_eq!(
            super::operation_issue_tab(
                Strategy::Drill,
                "Choose a flat end mill for Helical milling."
            ),
            Some("Tool")
        );
    }
    // Written as escapes so this file does not contain the text it hunts for.
    const MOJIBAKE: [&str; 3] = ["\u{e2}\u{20ac}", "\u{c3}", "\u{c2}"];

    fn scan(path: &Path, hits: &mut Vec<String>) {
        if path.is_dir() {
            for entry in std::fs::read_dir(path).unwrap() {
                scan(&entry.unwrap().path(), hits);
            }
        } else if matches!(path.extension().and_then(|e| e.to_str()), Some("rs" | "md")) {
            let text = std::fs::read_to_string(path).unwrap();
            for (number, line) in text.lines().enumerate() {
                if MOJIBAKE.iter().any(|bad| line.contains(bad)) {
                    hits.push(format!("{}:{}", path.display(), number + 1));
                }
            }
        }
    }

    /// The browser and the inspector leave the 3D view its least width in
    /// the window at its least: the browser gives way first (#76).
    #[test]
    fn the_3d_view_keeps_its_least_width_beside_the_sidebars() {
        use super::*;
        use crate::settings::{Layout, Side};
        let d = Layout::default();
        let min = crate::MIN_WINDOW.width;
        // The defaults fit the window at its least, the view wider than its least.
        assert_eq!(fitted(&d, min), (BROWSER.default, INSPECTOR.default));
        assert!(view_width(&d, min) > VIEW_MIN);
        // Both at their most: the browser gives way first, down to its
        // least, then the inspector; the view keeps its least exactly.
        let wide = Layout {
            browser_width: BROWSER.max,
            inspector_width: INSPECTOR.max,
            ..d.clone()
        };
        let (b, i) = fitted(&wide, min);
        assert_eq!(b, BROWSER.min);
        assert!(i < INSPECTOR.max && i > INSPECTOR.min, "{i}");
        assert_eq!(view_width(&wide, min), VIEW_MIN);
        // A wide window gives both their widths.
        assert_eq!(fitted(&wide, 1920.), (BROWSER.max, INSPECTOR.max));
        // A closed sidebar is its strip, narrower than its least.
        let closed = Layout {
            browser_open: false,
            ..wide.clone()
        };
        let (b, i) = fitted(&closed, min);
        assert!(b < BROWSER.min);
        assert_eq!(i, INSPECTOR.max);
        assert!(view_width(&closed, min) > VIEW_MIN);
        // Saved widths outside the range are cut to it.
        let odd = Layout {
            browser_width: 5.,
            inspector_width: 5000.,
            ..d.clone()
        };
        assert_eq!(fitted(&odd, 1920.), (BROWSER.min, INSPECTOR.max));
        // A drag is cut to the range, and to the view's least: the browser
        // stops where the inspector keeps its width, the inspector where the
        // browser is at its least.
        assert_eq!(clamped(&d, Side::Browser, 9999., 1920.), BROWSER.max);
        assert_eq!(clamped(&d, Side::Inspector, 0., 1920.), INSPECTOR.min);
        assert_eq!(
            clamped(&d, Side::Browser, 9999., min),
            min - VIEW_MIN - 2. * GRIP - INSPECTOR.default
        );
        assert_eq!(
            clamped(&d, Side::Inspector, 9999., min),
            min - VIEW_MIN - 2. * GRIP - BROWSER.min
        );
        for (side, width) in [(Side::Browser, 9999.), (Side::Inspector, 9999.)] {
            let mut l = d.clone();
            *l.width_mut(side) = clamped(&d, side, width, min);
            assert!(view_width(&l, min) >= VIEW_MIN, "{side:?}");
        }
    }

    /// WCAG 2.x relative luminance of an sRGB colour given as 0-1 channels.
    fn luminance([r, g, b]: [f64; 3]) -> f64 {
        let linear = |c: f64| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }

    fn contrast(a: [f64; 3], b: [f64; 3]) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    fn rgb(c: iced::Color) -> [f64; 3] {
        [c.r, c.g, c.b].map(f64::from)
    }

    /// `#rrggbb` as 0-1 channels.
    fn hex(h: &str) -> [f64; 3] {
        let n = u32::from_str_radix(h.trim_start_matches('#'), 16).unwrap();
        [n >> 16, (n >> 8) & 255, n & 255].map(|c| f64::from(c & 255) / 255.)
    }

    /// The `#rrggbb` value of a tag's `name="#..."` attribute.
    fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
        let at = tag.find(&format!(" {name}=\"#"))? + name.len() + 3;
        Some(&tag[at..at + 7])
    }

    #[test]
    fn the_status_line_keeps_whole_sentences_within_its_line() {
        use super::leading_sentences;
        // "33.4 min" does not end a sentence.
        let s = "Facing preview ready: 5 levels, 33.4 min estimated. Inherits stock after 0 earlier operations across setups.";
        assert_eq!(
            leading_sentences(s, 60),
            "Facing preview ready: 5 levels, 33.4 min estimated."
        );
        // A short first sentence keeps the reason after it.
        let s = "Import failed. The file is not a STEP file. Pick another.";
        assert_eq!(
            leading_sentences(s, 45),
            "Import failed. The file is not a STEP file."
        );
        // Text that fits, or one long sentence, stays whole.
        assert_eq!(leading_sentences(s, 200), s);
        assert_eq!(
            leading_sentences("No full stop here", 5),
            "No full stop here"
        );
        assert_eq!(
            leading_sentences("Too long a first one. Second.", 5),
            "Too long a first one."
        );
    }
    #[test]
    fn muted_text_meets_contrast_on_every_surface() {
        use super::*;
        let surfaces = [
            ("white panel", Color::WHITE),
            ("application background", BG),
            ("selected operation row", SELECTED_ROW),
            ("setup row", SETUP_ROW),
            ("hovered row", HOVER_ROW),
            ("active toolbar button", ACTIVE_TILE),
            ("hovered button", HOVER_BUTTON),
        ];
        let low: Vec<_> = surfaces
            .iter()
            .map(|(name, surface)| (name, contrast(rgb(MUTED), rgb(*surface))))
            .filter(|(_, ratio)| *ratio < 4.5)
            .collect();
        assert!(low.is_empty(), "muted text below 4.5:1 on {low:?}");
        // It stays secondary: lighter than the body ink.
        assert!(luminance(rgb(MUTED)) > luminance(rgb(INK)));
        // A greyed label is lighter still, and readable on its white tile.
        assert!(luminance(rgb(DIMMED_TEXT)) > luminance(rgb(MUTED)));
        assert!(contrast(rgb(DIMMED_TEXT), rgb(Color::WHITE)) >= 3.);
    }

    /// Every colour a toolbar tile's glyph is drawn in, against the tile.
    #[test]
    fn toolbar_glyphs_meet_contrast_on_their_tiles() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let mut checked = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let file = path.file_name().unwrap().to_str().unwrap().to_string();
            if !(file.starts_with("tb-") && file.ends_with(".svg")) {
                continue;
            }
            let svg = std::fs::read_to_string(&path).unwrap();
            let tags: Vec<_> = svg.split('<').skip(2).collect();
            let tile = attribute(svg.split('<').nth(2).unwrap(), "fill")
                .unwrap_or_else(|| panic!("{file}: the first shape is the tile"));
            assert!(!svg.contains("opacity"), "{file}: glyphs are solid");
            for tag in tags.iter().skip(1) {
                for kind in ["fill", "stroke"] {
                    let Some(colour) = attribute(tag, kind) else {
                        continue;
                    };
                    if colour == tile {
                        continue; // a cut-out or a gap in the tile's own colour
                    }
                    let ratio = contrast(hex(colour), hex(tile));
                    assert!(
                        ratio >= 4.5,
                        "{file}: {kind} {colour} on tile {tile} is {ratio:.2}:1"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked >= 9, "only {checked} glyph colours were read");
    }

    #[test]
    fn the_control_tile_is_a_panel_not_a_play_triangle() {
        let svg = include_str!("../assets/tb-control.svg");
        assert!(svg.matches("<circle").count() >= 2, "a dial and a button");
        assert!(!svg.contains("l13-8"), "the play triangle is gone");
    }

    #[test]
    fn an_unavailable_strategy_says_why_on_hover() {
        for s in Strategy::ALL.iter().filter(|s| !s.imported_preview()) {
            let help = crate::help::unavailable_help(*s);
            let reason = s
                .availability()
                .strip_prefix("Unavailable · ")
                .expect("an unavailable strategy says so");
            assert!(help.title.contains("unavailable"), "{}", help.title);
            assert!(help.body.contains(reason), "{}", help.body);
            assert!(help.body.contains("Operation support"), "{}", help.body);
        }
    }

    #[test]
    fn sources_and_docs_have_no_mojibake() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut hits = Vec::new();
        for path in ["src", "docs", "README.md"] {
            scan(&root.join(path), &mut hits);
        }
        assert!(hits.is_empty(), "mojibake found:\n{}", hits.join("\n"));
    }

    #[test]
    fn the_run_strip_lists_no_manual_setup() {
        use crate::model::{Document, Machine, Setup};
        let (cnc, big) = (Machine::new(3), crate::manual::defaults());
        let doc = Document {
            setups: vec![
                Setup::new(big.clone(), "G54"),
                Setup::new(cnc.clone(), "G55"),
                Setup::new(cnc.clone(), "G56"),
            ],
            ..Default::default()
        };
        let listed = |id: &str| -> Vec<usize> {
            super::run_setups(&doc, id)
                .iter()
                .map(|(si, _)| *si)
                .collect()
        };
        assert_eq!(listed(&cnc.id), vec![1, 2]);
        assert!(listed(&big.id).is_empty());
        // Even a manual mill that claims the UC100 controller.
        let mut clash = doc.clone();
        clash.setups[0]
            .machine
            .p
            .insert("controller".into(), "UC100".into());
        assert!(super::run_setups(&clash, &big.id).is_empty());
    }
}
