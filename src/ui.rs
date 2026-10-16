use crate::{
    App, Message, Page, Target,
    help::{self, Help, HelpButton},
    holding::Held,
    material::SpeedFeed,
    model::*,
    planner::Dropped,
    viewport::{self, Viewport},
};
use iced::{
    Alignment, Border, Color, Element, Font,
    Length::{self, Fill},
    Theme,
    widget::{
        self, Column, Row, button, canvas, checkbox, column, container, mouse_area, pick_list, row,
        rule, scrollable, slider, space, svg, text, text_input,
    },
};
const INK: Color = Color::from_rgb(0.13, 0.20, 0.28);
const MUTED: Color = Color::from_rgb(0.43, 0.50, 0.57);
const BLUE: Color = Color::from_rgb(0.13, 0.42, 0.71);
const LINE: Color = Color::from_rgb(0.85, 0.88, 0.91);
const BG: Color = Color::from_rgb(0.95, 0.96, 0.975);
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
                Color::from_rgb8(223, 237, 249)
            } else if hovered {
                Color::from_rgb8(235, 240, 246)
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
/// A toolbar button with its icon over the label, like Fusion 360's.
fn tool_btn<'a>(icon: &'static [u8], label: &str, msg: Message, active: bool) -> HelpButton<'a> {
    let action = msg.clone();
    let b = button(
        column![tool_icon(icon), text(label.to_string()).size(11)]
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
    svg(svg::Handle::from_memory(icon)).width(28).height(28)
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
fn switcher(app: &App) -> Element<'_, Message> {
    const WORKSPACES: [&str; 2] = ["MANUFACTURE", "MACHINE"];
    let font = Font {
        weight: iced::font::Weight::Bold,
        ..Font::DEFAULT
    };
    let picker: Element<_> = if app.locked() {
        row![
            container(text(WORKSPACES[1]).size(14).font(font).color(MUTED)).padding([7, 12]),
            muted("Manufacture opens when the machine stops.")
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
        .text_size(14)
        .padding([7, 12])
        .into()
    };
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
            "MANUFACTURE plans the job: the part, setups, operations, the libraries, the AI planner and the NC program. MACHINE drives the CNC through the UC100: connect, jog, zero, probe and run a setup, and set up its profile. While the machine moves (a tool run, held or paused, a probe, a jog or homing) only MACHINE is open.",
        ),
        // Beside it, not over the list it opens.
        widget::tooltip::Position::Right,
    )
}
fn primary<'a>(label: impl Into<String>, msg: Message) -> HelpButton<'a> {
    let label = label.into();
    let action = msg.clone();
    let b = button(text(label.clone()).size(13))
        .on_press(msg)
        .padding([9, 14])
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
fn field<'a>(
    p: &'a Fields,
    key: &'static str,
    label: &'static str,
    target: Target,
) -> Element<'a, Message> {
    help::field(
        column![
            muted(label),
            text_input(label, get(p, key))
                .on_input(move |v| Message::Edit(target, key, v))
                .padding([7, 9])
                .size(13)
        ]
        .spacing(5),
        key,
        label,
        target,
    )
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
fn section<'a>(title: impl Into<String>) -> Element<'a, Message> {
    column![space().height(3), bold(title, 13), rule::horizontal(1)]
        .spacing(8)
        .into()
}
fn icon<'a>(app: &App, s: Strategy, size: f32) -> widget::Svg<'a> {
    svg(app.icons[s.index()].clone()).width(size).height(size)
}

/// The header's line on the automatic save of the workspace.
pub fn workspace_note(app: &App) -> String {
    match (&app.save_error, app.dirty) {
        (Some(e), _) => format!("Workspace not saved: {e}"),
        (None, true) => "Saving workspace…".into(),
        (None, false) => "Workspace saved locally".into(),
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
                bar_btn("Save project", Message::SaveProject, false),
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
    let brand = brand
        .push(space().width(Fill))
        .push(
            text(workspace_note(app))
                .size(11)
                .color(Color::from_rgb8(183, 204, 222)),
        )
        .push(space().width(10))
        .push(
            text("NATIVE PROTOTYPE")
                .size(10)
                .color(Color::from_rgb8(228, 193, 122)),
        );
    let brand = container(brand)
        .padding([8, 18])
        .style(|_| container::Style {
            background: Some(Color::from_rgb8(29, 46, 63).into()),
            ..Default::default()
        });
    let mut top_row = row![switcher(app), rule::vertical(1)]
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
        Page::Work => row![browser(app), workspace(app), inspector(app)]
            .spacing(0)
            .height(Fill)
            .into(),
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
            text(&app.status).size(11).width(Fill),
            text("mm  |  Bounded mesh previews · LinuxCNC 3-axis post")
                .size(10)
                .color(MUTED)
        ]
        .spacing(15)
        .align_y(Alignment::Center),
    )
    .padding([8, 14])
    .style(panel_style);
    let mut layout = column![brand, docbar].spacing(0);
    if manufacture && app.page != Page::Home && !app.doc.setups.is_empty() {
        layout = layout.push(ribbon(app));
    }
    container(layout.push(body).push(foot))
        .width(Fill)
        .height(Fill)
        .style(|_| container::Style {
            background: Some(BG.into()),
            text_color: Some(INK),
            ..Default::default()
        })
        .into()
}
fn start_screen(app: &App) -> Element<'_, Message> {
    let intro = column![
        muted("YOUR NEXT MACHINING PROJECT"),
        bold("Start with your part.", 36),
        text("Create a blank project, open saved work, or explore the example when you want a guided tour.").size(16),
        space().height(12),
        primary("+ New blank project", Message::NewProject),
        btn("Open a saved project…", Message::OpenProject, false),
        space().height(12),
        rule::horizontal(1),
        muted("LEARN THE WORKFLOW"),
        btn("Explore example project", Message::OpenSample, false),
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
                    "{} setups · {} operations",
                    app.doc.setups.len(),
                    app.doc
                        .setups
                        .iter()
                        .map(|s| s.operations.len())
                        .sum::<usize>()
                )))
                .push(primary("Continue project", Message::ResumeProject));
    } else {
        recent = recent.push(muted("There is no project to resume yet. Your new project starts with no model, setups or operations."));
    }
    recent = recent.push(rule::horizontal(1)).push(muted("REUSABLE LIBRARIES"))
        .push(muted(format!("{} saved machines · {} saved tools",app.doc.machines.len(),app.doc.tools.len())))
        .push(note("New projects keep your saved libraries. The previous project is archived before you start again."));
    recent = recent.push(btn("Browse project history…", Message::OpenHistory, false));
    if app.last_archived.is_some() {
        recent = recent.push(btn(
            "Recover previous project",
            Message::RecoverPrevious,
            false,
        ));
    }
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
        row![btn("New machine profile",Message::NewMachine,false),btn("Saved machines",Message::Page(Page::Machines),false),btn("Tool library",Message::Page(Page::Tools),false)].spacing(8),
        note("No sample geometry, setups or operations have been added. Saved machines, tools, materials and workholding are available when you need them.")
    ].spacing(13).max_width(730);
    if app.last_archived.is_some() {
        body = body.push(btn(
            "Recover previous project",
            Message::RecoverPrevious,
            false,
        ));
    }
    container(scrollable(
        container(body).padding(28).width(Fill).style(panel_style),
    ))
    .padding(36)
    .center(Fill)
    .into()
}
fn first_setup(app: &App) -> Element<'_, Message> {
    column![
        bold("No setups yet", 22),
        text("Your model is ready to view. Choose the machine and workholding for its first machining position.").size(14),
        primary("Create first setup",Message::AddSetup).width(Fill),
        btn(if app.doc.machines.is_empty() {"New machine profile"} else {"Open machine library"},if app.doc.machines.is_empty(){Message::NewMachine}else{Message::Page(Page::Machines)},false).width(Fill),
        note("Each setup starts with zero operations. Add another setup when the part needs to be flipped, indexed or reclamped.")
    ].spacing(18).into()
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
        muted("SETUP").size(9)
    ]
    .spacing(2)
    .align_x(Alignment::Center);
    groups = groups.push(setup).push(rule::vertical(1));
    for (label, range) in [
        ("2D MILLING", 0..5),
        ("3D FINISHING", 5..8),
        ("MULTI-AXIS", 8..11),
    ] {
        let mut buttons = Row::new().spacing(1);
        for s in &Strategy::ALL[range] {
            let s = *s;
            buttons = buttons.push(HelpButton::new(
                button(
                    column![
                        icon(app, s, 29.),
                        text(if app.is_sample() || s.imported_preview() {
                            s.short().to_string()
                        } else {
                            format!("{} · —", s.short())
                        })
                        .size(10)
                    ]
                    .spacing(4)
                    .align_x(Alignment::Center),
                )
                .padding([5, 7])
                .on_press(if app.is_sample() || s.imported_preview() {
                    Message::AddOp(s)
                } else {
                    Message::Page(Page::Audit)
                })
                .style(button::text),
                &Message::AddOp(s),
                s.name(),
            ));
        }
        groups = groups
            .push(
                column![buttons, muted(label).size(9)]
                    .spacing(4)
                    .align_x(Alignment::Center),
            )
            .push(rule::vertical(1));
    }
    groups = groups.push(space().width(Fill)).push(
        column![
            btn("Generate all", Message::GenerateAll, false),
            btn(
                "NC program",
                Message::Page(Page::Post),
                app.page == Page::Post
            ),
            muted("ACTIONS").size(9)
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
fn browser(app: &App) -> Element<'_, Message> {
    let mut tree =
        column![
            row![
                bold("BROWSER", 11),
                space().width(Fill),
                muted(format!("{} setups", app.doc.setups.len()))
            ],
            rule::horizontal(1),
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
            column![
                bold(s.name(), 12),
                muted(format!(
                    "{} · {}-axis",
                    get(&s.p, "offset"),
                    s.machine.axes()
                ))
                .size(10)
            ]
            .spacing(2)
            .width(Fill),
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
                        style.background = Some(Color::from_rgb8(230, 239, 248).into())
                    }
                    style.text_color = INK;
                    style
                }),
            &Message::SelectSetup(si),
            s.name(),
        ));
        if active {
            tree = tree.push(
                container(muted(format!("{} · v{}", s.machine.name(), s.machine.version)).size(10))
                    .padding([0, 16]),
            );
            let checks = app.current_ready();
            for (i, o) in s.operations.iter().enumerate() {
                let selected = app.selected == Some(i) && app.page == Page::Work;
                let (mark, color) = match op_mark(app, checks, i) {
                    OpMark::Suppressed => ("—", MUTED),
                    OpMark::Pending => ("○", MUTED),
                    OpMark::Generated => ("✓", MUTED),
                    OpMark::Collides => ("!", Color::from_rgb8(196, 112, 22)),
                    OpMark::Ready => ("✓", Color::from_rgb8(38, 146, 111)),
                };
                let content = row![
                    icon(app, o.strategy, 23.),
                    text(o.name()).size(12).width(Fill),
                    text(mark).size(13).color(color)
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
                                    Color::from_rgb8(218, 235, 249)
                                } else if matches!(st, button::Status::Hovered) {
                                    Color::from_rgb8(239, 244, 249)
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
                ));
            }
            if let Some(picture) = stock_picture(app, si, true) {
                tree = tree.push(help::wrap(
                    row![
                        muted("Stock after this setup").size(11).width(Fill),
                        widget::image(picture).width(64).height(48)
                    ]
                    .spacing(6)
                    .padding([0, 10])
                    .align_y(Alignment::Center),
                    Help::text(
                        "Stock after this setup",
                        "The stock as this setup's operations leave it, in its jaws or clamps, seen as its 3D view first shows it. Cut faces are green.",
                    ),
                    widget::tooltip::Position::Right,
                ));
            }
        }
    }
    if let Some(block) = uncut_block(app) {
        tree = tree.push(block);
    }
    tree = tree
        .push(space().height(6))
        .push(btn("+ Add setup", Message::AddSetup, false).width(Fill));
    let footer = column![
        rule::horizontal(1),
        muted("PROJECT UNITS"),
        text("Millimeters (mm)").size(12),
        muted("Machine limits can use mm or inch. Tool and operation values use mm.").size(10)
    ]
    .spacing(6);
    container(column![scrollable(tree).height(Fill), footer].spacing(12))
        .padding(14)
        .width(230)
        .height(Fill)
        .style(panel_style)
        .into()
}
/// What the whole job leaves uncut as worked out for the job as it is now
/// (`LeftStock::uncut`): None while it is worked out, and without a setup or
/// an imported part.
fn job_uncut(app: &App) -> Option<&Result<std::sync::Arc<crate::uncut::Uncut>, String>> {
    let key = app.left_key()?;
    (app.left_stock.key == key)
        .then_some(app.left_stock.uncut.as_ref())
        .flatten()
}
/// One line on what the whole job leaves uncut, under the 3D views: None
/// without a setup or an imported part.
pub(crate) fn uncut_line(app: &App) -> Option<String> {
    app.left_key()?;
    Some(match job_uncut(app) {
        None => "Completeness not checked yet.".into(),
        Some(Err(why)) => format!("Completeness not checked: {why}"),
        Some(Ok(found)) if found.regions.is_empty() => {
            "Every face of the part is cut by the job.".into()
        }
        Some(Ok(found)) => format!(
            "Magenta faces are still under stock after the whole job: {:.0}% of the part's surface.",
            found.share * 100.
        ),
    })
}
/// The browser's list of the regions of the part the whole job leaves under
/// stock, or why that is not checked.
fn uncut_block(app: &App) -> Option<Element<'_, Message>> {
    let mut block = column![muted("STILL UNCUT").size(10)].spacing(4);
    match job_uncut(app) {
        Some(Ok(found)) if !found.regions.is_empty() => {
            block = block.push(
                text(format!(
                    "{} region{}, {:.0}% of the part's surface, magenta in the 3D view:",
                    found.regions.len(),
                    if found.regions.len() == 1 { "" } else { "s" },
                    found.share * 100.
                ))
                .size(11)
                .color(Color::from_rgb8(166, 46, 124)),
            );
            for region in found.regions.iter().take(5) {
                block = block.push(muted(format!("• {}", region.line())).size(10));
            }
            if found.regions.len() > 5 {
                block =
                    block.push(muted(format!("• and {} more", found.regions.len() - 5)).size(10));
            }
        }
        _ => block = block.push(muted(uncut_line(app)?).size(11)),
    }
    Some(help::wrap(
        container(block).padding([4, 10]),
        Help::text(
            "Still uncut",
            "The faces of the part that the stock left after every setup still covers: what the job has not machined yet. Each face is sampled 0.5 mm outside itself against the simulated cuts, so a thinner finishing allowance counts as cut. Positions are in setup 1's axes from the part's -X/-Y/bottom corner, in mm. Until every enabled operation generates, completeness is not checked.",
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
    let jaws = if matches!(get(&setup.p, "holding"), "Vise" | "Clamps") {
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
    let picture = crate::viewport::stock_picture(&triangles)?;
    // ponytail: two per setup, so all are kept until there are many.
    if drawn.len() > 64 {
        drawn.clear();
    }
    drawn.push((key, picture.clone()));
    Some(picture)
}
/// The 3D view of the active setup, as the Work page shows it: the part,
/// stock, workholding, the preview and the machine's tool while a tool run of
/// the setup cuts. None without a part to show.
fn scene(app: &App) -> Option<Element<'_, Message>> {
    let s = app.doc.active_setup();
    let canvas: Element<viewport::Event> = canvas(Viewport {
        setup: s,
        mesh: app.part_mesh()?,
        operation: app.operation(),
        simulation: app.active_simulation(),
        sample: app.is_sample(),
        camera: app.camera,
        picking: app.picking,
        show_stock: app.stock && s.is_some(),
        show_paths: app.paths && app.playback_ready(),
        progress: if app.operation().is_some_and(|o| o.preview) {
            Some(app.progress)
        } else {
            None
        },
        holding: holding_boxes(app),
        leftover: leftover_view(app),
        live: app.live_tool(),
        uncut: job_uncut(app)
            .and_then(|found| found.as_ref().ok())
            .map(|found| found.faces.as_slice())
            .unwrap_or(&[]),
    })
    .width(Fill)
    .height(Fill)
    .into();
    Some(canvas.map(Message::Viewport))
}
fn workspace(app: &App) -> Element<'_, Message> {
    let s = app.doc.active_setup();
    let heading = container(
        row![
            column![
                bold(s.map(|s| s.name()).unwrap_or("Imported model"), 15),
                muted(
                    s.map(|s| format!(
                        "{} / {} / {}",
                        get(&s.p, "offset"),
                        app.doc.holding_label(s),
                        get(&s.p, "origin")
                    ))
                    .unwrap_or_else(|| {
                        "Create your first machining setup when you are ready.".into()
                    })
                )
            ]
            .spacing(3),
            space().width(Fill),
            btn("ISO", Message::Camera("ISO"), false),
            btn("TOP", Message::Camera("TOP"), false),
            btn("FRONT", Message::Camera("FRONT"), false),
            btn("RIGHT", Message::Camera("RIGHT"), false)
        ]
        .spacing(1)
        .align_y(Alignment::Center),
    )
    .padding([12, 14]);
    let scene = scene(app).unwrap_or_else(|| {
        container(column![bold(if app.import_busy { "Loading geometry…" } else if !app.doc.has_model() { "Import your part" } else { "Model preview unavailable" },25),muted(app.doc.file.as_ref().map(|f|f.name.clone()).unwrap_or_default()),note(app.import_error.clone().unwrap_or_else(|| "Import STEP, STP, 3MF, STL or OBJ to view your model. Geometry loads in the background; a STEP conversion may take up to two minutes.".into())),primary("Import model",Message::Import),btn("Start screen",Message::Page(Page::Home),false)].spacing(20).max_width(380)).center(Fill).into()
    });
    let detail = if app.is_sample() {
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
                .text_size(11),
            crate::choice_help::selected(
                "stock_visibility",
                "Stock",
                if app.stock { "Shown" } else { "Hidden" },
                Target::Setup,
                &Fields::new()
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
                .text_size(11),
            crate::choice_help::selected(
                "path_visibility",
                "Toolpath",
                if app.paths { "Shown" } else { "Hidden" },
                Target::Operation,
                &Fields::new()
            ),
            widget::tooltip::Position::Top
        ),
        space().width(Fill)
    ]
    .spacing(15)
    .align_y(Alignment::Center);
    let toolbar = container(
        column![
            toggles,
            muted(detail).size(10),
            uncut_line(app).map(|line| muted(line).size(10)),
            muted(if app.picking {
                "Click a highlighted feature to select it · drag to orbit"
            } else {
                "Drag to orbit · right-drag to pan · scroll to zoom"
            })
            .size(10)
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
                10
            ),
            space().width(Fill),
            muted(if app.is_sample() {
                "Illustrative motion".to_string()
            } else {
                fixture_note(app, Note::Panel)
            })
            .size(10)
        ]
        .spacing(8)
    ]
    .spacing(8);
    if let Some((heading, lines, ok)) = run_check(app) {
        let said: Element<'_, Message> = if ok {
            let mut c = column![bold(heading, 12).color(GOOD)].spacing(3);
            for line in lines {
                c = c.push(muted(line).size(10));
            }
            c.into()
        } else if lines.is_empty() {
            muted(heading).size(11).into()
        } else {
            caution(heading, lines)
        };
        playback = playback.push(said);
    }
    if ready {
        playback = playback.push(row![
            btn(if app.playing { "Pause" } else if app.progress >= 1. { "Replay" } else { "Play" }, Message::Play, app.playing),
            btn("Restart", Message::Scrub(0.), false),
            help::wrap(slider(0.0..=1.0, app.progress, Message::Scrub).step(0.001_f32).width(Fill), Help::text("Simulation progress", "Scrub forward or backward. Playback recomputes the remaining stock and cutter position at that time. Restart restores the stock left by all preceding enabled operations and setups."), widget::tooltip::Position::Top),
            muted(format!("{:03}%", (app.progress * 100.) as u32)),
            help::wrap(pick_list(if app.active_simulation().is_some() { vec![1f32, 10., 30., 60., 120.] } else { vec![0.5f32, 1., 2., 4.] }, Some(app.speed), Message::Speed).text_size(11).padding(5),crate::choice_help::selected("speed","Playback speed",&app.speed.to_string(),Target::Operation,&Fields::new()),widget::tooltip::Position::Top),
            muted("×")
        ].spacing(8).align_y(Alignment::Center));
        if let Some(sim) = app.active_simulation() {
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
                .size(10),
            );
            playback = playback.push(
                muted(format!(
                    "{:.1} / {:.1} min · ≈{:.1} cm³ this operation · ≈{:.1} cm³ total removed",
                    sim.seconds / 60.,
                    sim.plan.duration / 60.,
                    (sim.removed_volume() - sim.inherited_volume()) / 1000.,
                    sim.removed_volume() / 1000.
                ))
                .size(10),
            );
            let surface_status = if let Some(surface) = &sim.stock_surface {
                format!(
                    "{:.2} mm finest cell · {} triangles{}",
                    surface.cell_mm,
                    surface.triangles.len(),
                    if app.stock_mesh_key().is_some() {
                        " · Updating…"
                    } else {
                        ""
                    }
                )
            } else {
                "Building stock surface…".into()
            };
            let mut detail = row![muted("Detail").size(10)]
                .spacing(5)
                .align_y(Alignment::Center);
            for choice in crate::stock_surface::Detail::ALL {
                detail = detail.push(btn(
                    choice.to_string(),
                    Message::StockDetail(choice),
                    choice == app.stock_detail,
                ));
            }
            playback = playback.push(detail.push(muted(surface_status).size(10)));
        }
    } else {
        let mut empty = row![muted(app.playback_reason()).size(11).width(Fill)]
            .spacing(10)
            .align_y(Alignment::Center);
        if imported_facing {
            empty = empty.push(primary("Generate & play", Message::Play));
        } else if app.is_sample() && app.operation().is_some_and(|o| o.enabled) {
            empty = empty.push(btn("Generate sample preview", Message::Generate, false));
        }
        playback = playback.push(empty);
    }
    let simulation = container(playback).padding([12, 16]).style(panel_style);
    // A UC100 machine takes no G-code file: its setups run from MACHINE.
    let uc100 =
        (app.doc.active_setup()).is_some_and(|s| get(&s.machine.p, "controller") == "UC100");
    let workflow = container(
        row![
            btn("01  Setup", Message::SetupEditor, app.selected.is_none()),
            btn(
                "02  Operations",
                Message::SelectOp(0),
                app.selected.is_some()
            ),
            btn("03  Preview", Message::Generate, false),
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
    .padding([5, 8])
    .style(panel_style);
    column![heading, scene, toolbar, simulation, workflow]
        .height(Fill)
        .width(Fill)
        .into()
}
fn inspector(app: &App) -> Element<'_, Message> {
    let content = if app.doc.active_setup().is_none() {
        first_setup(app)
    } else if let Some(o) = app.operation() {
        operation_editor(app, o)
    } else {
        setup_editor(app)
    };
    container(
        scrollable(container(content).width(304))
            .id("inspector")
            .width(Fill)
            .height(Fill),
    )
    .padding(15)
    .width(350)
    .height(Fill)
    .style(panel_style)
    .into()
}
fn operation_editor<'a>(app: &'a App, o: &'a Operation) -> Element<'a, Message> {
    let mut c = column![
        row![
            icon(app, o.strategy, 30.),
            column![bold(o.strategy.name(), 17), muted("Operation parameters")].spacing(2)
        ]
        .spacing(9)
        .align_y(Alignment::Center),
        field(&o.p, "name", "Operation name", Target::Operation),
        note(o.strategy.help())
    ]
    .spacing(13);
    let mut tabs = Column::new().spacing(2);
    let all = o.strategy.tabs();
    for chunk in all.chunks(3) {
        let mut r = Row::new().spacing(1);
        for t in chunk {
            r = r.push(
                btn(*t, Message::Tab(t.to_string()), app.tab == *t).width(Length::FillPortion(1)),
            );
        }
        tabs = tabs.push(r)
    }
    c = c.push(tabs).push(rule::horizontal(1));
    c = c.push(note(o.strategy.availability()));
    let p = &o.p;
    let t = Target::Operation;
    match app.tab.as_str() {
        "Tool" => {
            c = c.push(section("Cutter"));
            for tool in &app.doc.tools {
                c = c.push(
                    btn(
                        format!("{}  ·  Ø{} mm", tool.name(), get(&tool.p, "diameter")),
                        Message::SetOpTool(tool.id.clone()),
                        o.tool == tool.id,
                    )
                    .width(Fill),
                );
            }
            c = c
                .push(btn("Open tool library", Message::Page(Page::Tools), false))
                .push(section("Feeds & speeds"))
                .push(field(p, "rpm", "Spindle speed · rpm", t));
            if o.strategy.imported_preview() && !app.is_sample() {
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
                        .size(11),
                    );
            }
            c = c
                .push(field(p, "feed", "Cutting feed · mm/min", t))
                .push(choice(
                    p,
                    "coolant",
                    "Coolant",
                    &["Flood", "Mist", "Air", "Off"],
                    t,
                ));
            match app.doc.speed_feed(app.doc.setup(), o) {
                Some(SpeedFeed::Out(problems)) => {
                    c = c.push(caution(
                        "Speed or feed outside the material's cutting data",
                        problems,
                    ))
                }
                Some(SpeedFeed::Unchecked(why)) => c = c.push(muted(format!("Not checked. {why}"))),
                _ => {}
            }
        }
        "Geometry" => {
            if let Some(file) = &app.doc.file {
                c = c.push(muted(format!("Part · {}", file.name)));
                if !o.needs_surfaces() && o.strategy.imported_preview() {
                    c = c.push(note("Uses the current imported model in this setup. No demonstration geometry or saved sample features are used."));
                } else if o.strategy.imported_preview() {
                    c = c.push(note(if o.strategy == Strategy::Drill { "Select a horizontal rim face containing circular hole openings. Geometry detects the holes in that face; the diameter filter can restrict the group." } else if matches!(o.strategy, Strategy::Pocket | Strategy::Contour) { "Click the recessed pocket floor on your part. Its outline defines the walls; holes in the floor preserve islands. Amber = hover; blue = selected. Click Finish picking when done." } else { "Click a planar surface on your part. Amber = hover; blue = selected. Drag to orbit, then click Finish picking." }));
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
                    let selection = o.surfaces.as_ref();
                    let count = selection.map_or(0, |s| s.ids.len());
                    c = c.push(muted(format!("{count} surface(s) selected")));
                    if let (Some(selected), Some(mesh)) = (selection, &app.imported_mesh) {
                        if selected.matches(mesh) {
                            for &id in selected.ids.iter().take(64) {
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
                    }
                    if selection.is_some() {
                        c = c.push(btn(
                            "Clear surface selection",
                            Message::ClearSurfaces,
                            false,
                        ));
                    }
                    if o.strategy == Strategy::Drill {
                        c = c.push(field(
                            p,
                            "hole_diameter",
                            "Hole diameter filter · mm (0 = all)",
                            t,
                        ));
                        if let Some(mesh) = &app.imported_mesh {
                            match crate::cam::holes(app.doc.setup(), o, mesh) {
                                Ok(holes) => {
                                    for h in holes.iter().take(24) {
                                        c = c.push(muted(format!(
                                            "Bore Ø{:.2} / opening Ø{:.2} · X{:.2} Y{:.2} · depth {:.2} mm · {}",
                                            h.diameter, h.opening,
                                            h.center[0],
                                            h.center[1],
                                            h.top - h.bottom,
                                            if h.through { "through" } else { "blind" }
                                        )));
                                    }
                                }
                                Err(error) => c = c.push(note(error)),
                            }
                        }
                    }
                    c = c.push(note(match o.strategy {
                        Strategy::Pocket => "Pocket uses offset contours of the selected floor triangles, including islands. Supports open, vertical-wall pockets with a flat floor; overhanging geometry is rejected. Simulation inherits the stock left by all earlier enabled operations and setups.".to_string(),
                        Strategy::Contour => "Inside contour offsets only the walls and islands of this floor. It does not clear the interior. The floor must be flat and accessible along setup Z; overhangs are rejected.".to_string(),
                        Strategy::Drill => "Hole detection uses tessellated circular openings and vertical rays. Noncircular openings and obstructions are excluded. Confirm the detected centers and depths before previewing.".to_string(),
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
                    .push(muted(format!("{} feature(s) selected", o.geometry.len())));
            } else {
                c = c.push(note("Import a part before selecting geometry."));
            }
            if !app.is_sample() && o.strategy == Strategy::Face {
                c = c.push(note("Facing preview covers the XY bounding rectangle of the selected surfaces, model or stock. It includes holes and gaps within that rectangle. Containment and radial allowance adjust the cutter-center limits."));
            }
            if app.is_sample() || matches!(o.strategy, Strategy::Face | Strategy::Pocket) {
                c = c
                    .push(choice(
                        p,
                        "boundary",
                        "Machining boundary",
                        if o.strategy == Strategy::Pocket && !app.is_sample() {
                            &["Selected geometry"]
                        } else {
                            &["Selected geometry", "Stock silhouette", "Model silhouette"]
                        },
                        t,
                    ))
                    .push(choice(
                        p,
                        "containment",
                        "Tool containment",
                        if o.strategy == Strategy::Pocket && !app.is_sample() {
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
            } else if o.strategy.imported_preview() && o.strategy != Strategy::Drill {
                c = c.push(choice(
                    p,
                    "boundary",
                    "Machining boundary",
                    if o.strategy == Strategy::Contour {
                        &["Model silhouette", "Selected geometry"]
                    } else {
                        &["Model silhouette"]
                    },
                    t,
                ));
            }
        }
        "Heights" => {
            c = c.push(note(if !app.is_sample() && o.strategy == Strategy::Face {
                "Facing uses remaining stock to skip fully cleared top layers. Bottom offset sets the target relative to the selected plane; it must not cut below the highest model feature. Clearance/retract retain their original stock reference."
            } else if !app.is_sample() && o.strategy == Strategy::Pocket {
                "Cutting starts from the remaining stock after earlier operations, plus positive Top offset. A fully faced-off layer is skipped; uncut regions keep the higher start. Bottom offset and axial stock to leave are added to the selected floor. Clearance/retract stay referenced to the original stock top."
            } else if !app.is_sample() && o.strategy==Strategy::Contour && get(p,"boundary")=="Selected geometry" {
                "Inside contour skips fully cleared top layers and ends at the selected floor + bottom offset. Negative bottom offsets are rejected. Profile ramp/plunge are configured in Linking; the pocket interior is not cleared. Clearance/retract retain the original stock reference."
            } else if !app.is_sample() && matches!(o.strategy, Strategy::Contour | Strategy::Waterline) {
                "Bottom offset is relative to model bottom. Contour normally starts at stock top; Waterline at model top. Earlier cuts lower the start only when the whole machining area is cleared. Positive top offset adds an approach margin. Clearance/retract retain the original stock reference."
            } else if !app.is_sample() && o.strategy == Strategy::Parallel {
                "Bottom offset is added to the cutter-compensated surface height; stock to leave is a vertical offset. Top offset raises approach heights. Finishing starts with the stock left by the earlier roughing operations."
            } else if !app.is_sample() && o.strategy == Strategy::Drill {
                "Hole bottoms are detected from the model. Bottom offset raises that target; breakthrough extends below it. Peck levels start from remaining stock at each hole, accounting for previous facing. Clearance/retract retain their original reference. Removal is cylindrical without the drill-point cone."
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
                c = c.push(field(p, k, label, t));
            }
            if o.strategy == Strategy::Drill {
                c = c.push(choice(
                    p,
                    "through",
                    "Drill tip through bottom",
                    &["Yes", "No"],
                    t,
                ));
                if get(p, "through") == "Yes" {
                    c = c.push(field(p, "breakthrough", "Breakthrough · mm", t));
                }
            }
        }
        "Orientation" => match o.strategy {
            Strategy::Indexed => {
                c=c.push(note("Position the rotary axis before cutting. Angles are checked against the setup’s saved machine limits.")).push(field(p,"angle_a","Primary rotary angle · degrees",t));
                if app.doc.setup().machine.axes() == 5 {
                    c = c.push(field(p, "angle_b", "Secondary rotary angle · degrees", t));
                }
            }
            Strategy::Rotary => {
                c = c
                    .push(choice(
                        p,
                        "rotary_axis",
                        "Rotary axis",
                        &["A around X", "B around Y", "C around Z"],
                        t,
                    ))
                    .push(field(p, "rotary_start", "Start angle · degrees", t))
                    .push(field(p, "rotary_end", "End angle · degrees", t))
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
                    .push(field(p, "tilt_min", "Minimum tilt · degrees", t))
                    .push(field(p, "tilt_max", "Maximum tilt · degrees", t));
                if get(p, "axis_control") == "Lead and lean" {
                    c = c
                        .push(field(p, "lead_angle", "Lead angle · degrees", t))
                        .push(field(p, "lean_angle", "Lean angle · degrees", t));
                }
                c=c.push(choice(p,"collision","Collision avoidance preference",&["Retract and relink","Tilt within limits"],t)).push(note("This stores your intended strategy. Machine kinematics and collision solving are not connected yet."));
            }
            _ => {}
        },
        "Cycle" => {
            c = c.push(choice(
                p,
                "cycle",
                "Drilling cycle",
                &[
                    "Drilling",
                    "Chip breaking",
                    "Deep drilling",
                    "Counterboring",
                ],
                t,
            ));
            if ["Chip breaking", "Deep drilling"].contains(&get(p, "cycle")) {
                c = c.push(field(p, "peck", "Peck depth · mm", t));
            }
            if get(p, "cycle") == "Counterboring" {
                c = c.push(field(p, "dwell", "Dwell at bottom · seconds", t));
            }
            c = c
                .push(choice(
                    p,
                    "hole_order",
                    "Hole order",
                    &["Optimize travel", "By X then Y"],
                    t,
                ))
                .push(note(match get(p, "cycle") {
                    "Chip breaking" => "Retract slightly after each peck to break the chip.",
                    "Deep drilling" => "Retract clear of the hole between pecks to evacuate chips.",
                    "Counterboring" => "Feed to depth and dwell before retracting.",
                    _ => "Feed to the bottom in one move, then retract.",
                }));
        }
        "Passes" => {
            c = c.push(pass_fields(app, o));
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
                c = c.push(field(p, "ramp_angle", "Ramp angle · degrees", t));
            }
            c = c.push(note("Profile ramp descends around each closed contour at 25% feed, then completes the loop at depth. Plunge feeds straight down at 25% feed and requires a plunge-capable cutter. All transfers between contours retract to clearance; no sideways shortcut crosses an island."));
            c = c.push(note(fixture_note(app, Note::Pocket)));
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
                        c = c.push(field(p, "ramp_angle", "Ramp angle · degrees", t));
                    }
                }
                c = c
                    .push(field(p, "lead_in", "Lead-in radius · mm", t))
                    .push(choice(p, "keep_down", "Keep tool down", &["Yes", "No"], t));
            }
        }
        _ => {}
    }
    c = c.push(rule::horizontal(1)).push(warnings(app.op_issues()));
    if let Some((heading, lines)) = holding_warning(app) {
        c = c.push(caution(heading, lines));
    }
    c = c
        .push(
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
        )
        .push(
            row![
                btn("Duplicate", Message::DuplicateOp, false),
                btn(
                    if o.enabled { "Suppress" } else { "Enable" },
                    Message::ToggleOp,
                    false
                ),
                btn("Delete", Message::RemoveOp, false)
            ]
            .spacing(1),
        )
        .push(
            row![
                btn("Move up", Message::MoveOp(-1), false),
                btn("Move down", Message::MoveOp(1), false)
            ]
            .spacing(5),
        );
    c.into()
}
fn pass_fields<'a>(app: &'a App, o: &'a Operation) -> Element<'a, Message> {
    let p = &o.p;
    let t = Target::Operation;
    let s = o.strategy;
    let mut c = Column::new().spacing(12);
    if !app.is_sample()
        && matches!(
            s,
            Strategy::Contour | Strategy::Parallel | Strategy::Waterline
        )
    {
        if s == Strategy::Parallel {
            c=c.push(field(p,"direction","Pass direction · degrees",t))
                .push(field(p,"surface_stepover","Surface stepover · mm",t))
                .push(field(p,"slope_min","Minimum surface slope · degrees",t))
                .push(field(p,"slope_max","Maximum surface slope · degrees",t))
                .push(field(p,"tolerance","Sampling tolerance · mm",t))
                .push(field(p,"allowance","Vertical stock to leave · mm",t))
                .push(note("Flat and ball cutter envelopes are evaluated against mesh triangles on a bounded grid. Zigzag passes follow the upper surface. Undercuts are unreachable; spacing/tolerance controls sampling, not certified machining accuracy."));
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
                .push(field(p, "stepdown", "Maximum stepdown · mm", t))
                .push(field(p, "allowance", "Radial stock to leave · mm", t));
            if s == Strategy::Contour {
                c=c.push(choice(p,"side","Contour side",&["Outside","Inside"],t))
                    .push(choice(p,"compensation","Cutter compensation",&["In computer"],t))
                    .push(field(p,"finish_passes","Repeat finishing passes",t))
                    .push(note("Outside follows the outer model silhouette at successive depths. Inside follows only the walls and islands of a selected recessed floor; select that floor in Geometry. Computer compensation includes the cutter radius and allowance. Repeat passes revisit the contour."));
            } else {
                c=c.push(note("Constant-Z loops follow the projected geometry above each level, with cutter-radius compensation. This conservative vertical-access preview supports flat cutters. Overhung detail remains unmachined."));
            }
        }
        return c.into();
    }
    if s == Strategy::Pocket && !app.is_sample() {
        c = c.push(choice_value(p, pocket_setting(p,"pocket_milling"), "pocket_milling", "Milling direction", &FACING_MILLING, t))
            .push(muted(format!("Spindle: {} · set in Tool", facing_setting(p,"spindle_rotation"))).size(11))
            .push(note("Climb / Conventional changes the direction around walls and islands. Offset contours clear from the interior outward, with a retract between loops."));
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
        if facing_setting(p, "facing_pattern") == FACING_PATTERNS[0] {
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
            .size(11),
        );
    }
    if matches!(s, Strategy::Face | Strategy::Indexed | Strategy::Parallel) {
        c = c.push(field(p, "direction", "Pass direction · degrees", t));
    }
    if matches!(s, Strategy::Face | Strategy::Indexed | Strategy::Pocket) {
        c = c.push(field(p, "stepover", "Stepover · % tool diameter", t));
    }
    if s == Strategy::Adaptive {
        c = c
            .push(field(p, "optimal_load", "Optimal load · mm", t))
            .push(field(p, "minimum_radius", "Minimum cutting radius · mm", t));
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
        c = c.push(field(p, "stepdown", "Maximum stepdown · mm", t));
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
        c = c.push(field(p, "finish_passes", "Finishing passes", t));
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
            .push(field(p, "surface_stepover", "Surface stepover · mm", t))
            .push(field(p, "slope_min", "Minimum surface slope · degrees", t))
            .push(field(p, "slope_max", "Maximum surface slope · degrees", t));
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
    c = c.push(field(p, "allowance", "Radial stock to leave · mm", t));
    if matches!(s, Strategy::Adaptive | Strategy::Pocket) {
        c = c.push(field(p, "axial_allowance", "Axial stock to leave · mm", t));
    }
    if !app.is_sample() && s == Strategy::Face {
        c = c.push(note("The preview supports flat end mills and face mills on the highest planar face. Continuous cutter sweeps preserve removed stock; fully cleared top layers are skipped automatically. This does not establish machining accuracy or collision clearance."));
    } else if !app.is_sample() && s == Strategy::Pocket {
        c = c.push(note(fixture_note(app, Note::Passes)));
    } else {
        c = c.push(field(p, "tolerance", "Tolerance · mm", t));
    }
    if matches!(s, Strategy::Adaptive | Strategy::Scallop) {
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
pub(crate) const SETUP_STEPS: [&str; 4] = ["Machine", "Stock", "Position", "Holding"];
/// Whether the active setup's `step` needs nothing more: its tab's ✓, else ○.
pub(crate) fn setup_step_done(app: &App, step: &str) -> bool {
    let s = app.doc.setup();
    match step {
        "Machine" => {
            s.machine.issues().is_empty()
                && !get(&s.p, "offset").is_empty()
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
        _ => matches!(get(&s.p, "holding"), "Vise" | "Clamps") && holding_lines(app).is_empty(),
    }
}
/// Whether the part reaches outside the setup's fixed stock box.
fn outside_fixed_stock(s: &Setup, resolved: &crate::stock::ResolvedStock) -> bool {
    !crate::stock::relative(s)
        && resolved.part.is_some_and(|part| {
            (0..3).any(|i| {
                part.min[i] < resolved.stock.min[i] - 0.001
                    || part.max[i] > resolved.stock.max[i] + 0.001
            })
        })
}
fn setup_editor(app: &App) -> Element<'_, Message> {
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
    let c = column![
        bold("Setup", 20),
        muted(
            "One position of the part on the machine: its machine, stock, placement and holding."
        ),
        field(p, "name", "Setup name", t),
        steps,
    ]
    .spacing(12);
    let mut c = match app.setup_step {
        "Stock" => setup_stock(app, c),
        "Position" => setup_position(app, c),
        "Holding" => setup_holding(app, c),
        _ => setup_machine(app, c),
    };
    c = c
        .push(warnings(s.issues()))
        .push(btn("Duplicate setup", Message::DuplicateSetup, false).width(Fill))
        .push(
            row![
                btn("Move up", Message::MoveSetup(-1), false),
                btn("Move down", Message::MoveSetup(1), false)
            ]
            .spacing(5),
        );
    if app.confirm_delete_setup {
        c=c.push(note("Delete this setup and its operations? Later stock will be rebuilt from the revised setup order.")).push(row![btn("Cancel",Message::ConfirmDeleteSetup(false),false),primary("Delete setup",Message::ConfirmDeleteSetup(true))].spacing(5));
    } else {
        c = c.push(btn("Delete setup…", Message::RemoveSetup, false).width(Fill));
    }
    c.into()
}
fn setup_machine<'a>(app: &'a App, c: Column<'a, Message>) -> Column<'a, Message> {
    let s = app.doc.setup();
    let p = &s.p;
    let t = Target::Setup;
    c.push(section("Machine & coordinate system"))
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
        .push(choice(
            p,
            "offset",
            "Work offset",
            &["G54", "G55", "G56", "G57", "G58", "G59"],
            t,
        ))
        .push(choice(
            p,
            "origin",
            "Work coordinate origin",
            &crate::post::ORIGINS,
            t,
        ))
        .push(zero_note(app))
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
        (Some(spots), _) => {
            format!("On the stock this setup starts from ({from}): touch off {spots}.")
        }
        (None, Some(refs)) => format!(
            "Model origin is the part file's own 0,0,0. A probe finds it from the stock this setup starts from ({from}): {}. By hand, the program header locates it from the part.",
            crate::post::probe_spots(refs, stock)
        ),
        _ => "Choose a work zero.".into(),
    }];
    if let Some(refs) = refs {
        lines.push(format!(
            "On the Control page, {} sets exactly this work zero.",
            crate::control::probe_name(refs)
        ));
    }
    let mut c = column![muted(lines.join(" "))].spacing(8);
    if let Some(zero) = app.current_ready().and_then(|r| r.zero.as_ref()) {
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
    if app.doc.index() == 0 || app.is_sample() {
        c = c.push(choice(
            p,
            "stock_type",
            "Initial stock mode",
            &["Relative size box", "Fixed size box"],
            t,
        ));
        if crate::stock::relative(s) {
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
                let mut summary = column![
                    bold("Resulting stock · X × Y × Z", 12),
                    text(crate::stock::dimensions_text(resolved.stock.dimensions()))
                        .size(14)
                        .color(BLUE)
                ]
                .spacing(6);
                if let Some(part) = resolved.part {
                    summary = summary.push(muted(format!(
                        "Part: {}",
                        crate::stock::dimensions_text(part.dimensions())
                    )));
                }
                c = c.push(
                    container(summary)
                        .padding(12)
                        .width(Fill)
                        .style(panel_style),
                );
                if outside_fixed_stock(s, &resolved) {
                    c = c.push(note("The part extends outside this fixed stock box. Increase its dimensions, adjust placement, or choose Relative size box."));
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
    let mut c = c.push(section("Part position")).push(
        row![
            btn("Flip X 180°", Message::Flip("rx"), false),
            btn("Side Y 90°", Message::Flip("ry"), false)
        ]
        .spacing(4),
    );
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
fn setup_holding<'a>(app: &'a App, c: Column<'a, Message>) -> Column<'a, Message> {
    let p = &app.doc.setup().p;
    let t = Target::Setup;
    let mut c = c.push(section("Workholding"));
    for control in holding_controls(app) {
        c = c.push(control);
    }
    c = c
        .push(field(p, "clearance", "Fixture clearance · mm", t))
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
        ))
        .push(field(p, "notes", "Setup instructions", t));
    c
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
                .text_size(13),
            Help::text(
                "Project material",
                "Material from the library. Its cutting data are used to check speeds and feeds; its name, group, hardness, notes and cutting data are sent to the AI planner."
            ),
            widget::tooltip::Position::Left
        ),
        btn(
            "Open material library",
            Message::Page(Page::Materials),
            false
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
/// The setup editor's controls for the modelled workholding, above the
/// fixture description.
fn holding_controls(app: &App) -> Vec<Element<'_, Message>> {
    let p = &app.doc.setup().p;
    let t = Target::Setup;
    let holding = match get(p, "holding") {
        "" => "None",
        value => value,
    };
    let mut c: Vec<Element<_>> = vec![
        muted("The vise or clamps that hold the stock, from the workholding library.").into(),
        choice_value(
            p,
            holding,
            "holding",
            "Modelled workholding",
            &["None", "Vise", "Clamps"],
            t,
        ),
    ];
    match holding {
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
        c.push(field(p, "holding_gap", "Clearance gap · mm", t));
        c.push(
            btn("Suggest layout", Message::SuggestLayout, false)
                .width(Fill)
                .into(),
        );
        let lines = holding_lines(app);
        if !lines.is_empty() {
            let heading = format!(
                "{} workholding issue{}",
                lines.len(),
                if lines.len() == 1 { "" } else { "s" }
            );
            c.push(caution(heading, lines));
        }
        let margins = holding_margins(app);
        if !margins.is_empty() {
            let mut m = column![muted("Against the cutting forces").size(12)].spacing(3);
            for line in margins {
                m = m.push(muted(line).size(11));
            }
            c.push(m.into());
        }
    }
    if let Some(note) = stock_note(app) {
        c.push(muted(note).into());
    }
    c.push(
        btn(
            "Open workholding library",
            Message::Page(Page::Workholding),
            false,
        )
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
                .text_size(13),
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
            line = line.push(btn("Remove", Message::SetClamps(rows_text(&rest)), false));
        }
        c = c.push(line);
    }
    if rows.len() < MAX_CLAMPS {
        let mut more = rows.clone();
        more.push([Side::ALL[rows.len() % 4].label().into(), "0".into()]);
        c = c.push(btn(ADD_CLAMP, Message::SetClamps(rows_text(&more)), false).width(Fill));
    }
    c.into()
}
/// The picker of the library's vises or clamps (`kind`) for the active setup.
fn holder_picker<'a>(app: &'a App, kind: &'static str) -> Element<'a, Message> {
    let choices: Vec<Named> = app
        .doc
        .workholding
        .iter()
        .filter(|h| h.kind() == kind)
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
        let setup = app
            .doc
            .active_setup()
            .filter(|s| app.stock && matches!(get(&s.p, "holding"), "Vise" | "Clamps"))?;
        let stock = app
            .active_simulation()
            .map(|sim| sim.plan.stock)
            .or_else(|| app.known_stock(app.doc.index()).0)?;
        let boxes = app.doc.holding_boxes(setup, held_of(app, stock)).ok()?;
        Some(crate::holding::shown(&boxes, stock))
    };
    boxes().unwrap_or_default()
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
/// What the browser shows of an operation of the active setup.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum OpMark {
    /// —
    Suppressed,
    /// ○: not generated.
    Pending,
    /// Grey ✓: generated, but the setup will not run for a reason the
    /// simulation panel lists, or its preview is on screen and the checks
    /// before a run are still being worked out.
    Generated,
    /// Amber !: those checks refuse the setup's workholding for it, or for
    /// the whole setup (its clearance plane): a jaw or clamp in the way, or
    /// one that would not hold the stock against the operation's cut.
    Collides,
    /// Green ✓: those checks pass, so the setup will run.
    Ready,
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
        return if shown {
            OpMark::Generated
        } else {
            OpMark::Pending
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
/// What the simulation panel says of the selected operation's setup above
/// its controls: a heading, the lines of the checks before a run (the Run
/// panel's, export's for a machine that is not a UC100; the selected
/// operation's workholding problems first, then the setup's), and whether
/// the setup will run. None for the example and a suppressed operation.
pub(crate) fn run_check(app: &App) -> Option<(String, Vec<String>, bool)> {
    let i = app.selected?;
    let setup = app.doc.active_setup()?;
    let uc100 = get(&setup.machine.p, "controller") == "UC100";
    let (by, verb) = if uc100 {
        ("Run", "run")
    } else {
        ("Export", "export")
    };
    let name = setup.name();
    let Some(r) = app.current_ready() else {
        // Nothing is checked for the example.
        app.holding_key()?;
        return Some((format!("Checking {name} as {verb} will…"), vec![], false));
    };
    let mark = op_mark(app, Some(r), i);
    let own = r.held.iter().filter(|h| h.operation == Some(i));
    let setup_wide = r.held.iter().filter(|h| h.operation.is_none());
    let mut lines: Vec<String> = vec![];
    for problem in own.chain(setup_wide).map(|h| &h.message).chain(&r.problems) {
        if !lines.contains(problem) {
            lines.push(problem.clone());
        }
    }
    let machine = "The machine's position and the work zero are checked on the Run panel.";
    match mark {
        OpMark::Suppressed => None,
        OpMark::Ready => Some((
            format!("Ready to {verb}: {by}'s checks of {name} pass."),
            if uc100 { vec![machine.into()] } else { vec![] },
            true,
        )),
        OpMark::Collides => Some((
            format!("Workholding problem: {by} will refuse {name}"),
            lines,
            false,
        )),
        _ => Some((format!("{by} will refuse {name}"), lines, false)),
    }
}
/// `holding_warning` of the plan of the operation `name`, worked out.
fn holding_note(
    app: &App,
    plan: &crate::simulation::Plan,
    name: &str,
    setup: &Setup,
) -> crate::HoldingNote {
    let boxes = match app.doc.holding_boxes(setup, held_of(app, plan.stock)) {
        Ok(boxes) => boxes,
        Err(error) => {
            return Some((
                "Workholding cannot be checked; export will refuse this setup until it is fixed"
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
    let lines: Vec<String> = crate::holding::hits(plan, &boxes, reach)
        .iter()
        .map(|hit| crate::holding::message(name, hit, gap, datum.unwrap_or_default()))
        .collect();
    (!lines.is_empty()).then(|| {
        let frame = if datum.is_none() {
            "; coordinates in the setup frame"
        } else {
            ""
        };
        (
            format!("Too close to the workholding (export will refuse this{frame})"),
            lines,
        )
    })
}
/// What is wrong with the active setup's workholding, as the setup editor
/// lists it under the controls (`holding_lines_in`).
pub(crate) fn holding_lines(app: &App) -> Vec<String> {
    holding_lines_in(app, app.left_boxes(), app.current_checks(), app.doc.index())
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
        btn("← Back to machining", Message::Page(Page::Work), false)
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
        primary("+ New machine profile", Message::NewMachine),
        space().height(5)
    ]
    .spacing(18);
    for m in &app.doc.machines {
        let id = m.id.clone();
        let card = column![
            row![
                column![
                    bold(m.name(), 18),
                    muted(format!(
                        "{}-axis  ·  {}  ·  Profile v{}",
                        m.axes(),
                        get(&m.p, "mode"),
                        m.version
                    ))
                ]
                .spacing(5),
                space().width(Fill),
                btn("Edit profile", Message::EditMachine(id.clone()), false),
                btn("Remove", Message::DeleteMachine(id.clone()), false),
                primary(
                    if app.doc.active_setup().is_some() {
                        "Assign to current setup"
                    } else {
                        "Create first setup"
                    },
                    Message::AssignMachine(id)
                )
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            rule::horizontal(1),
            row![
                muted(format!(
                    "Travel  {} × {} × {} {}",
                    get(&m.p, "x"),
                    get(&m.p, "y"),
                    get(&m.p, "z"),
                    get(&m.p, "units")
                )),
                space().width(35),
                muted(format!("Spindle  {} rpm", get(&m.p, "rpm"))),
                space().width(35),
                muted(format!("Controller  {}", get(&m.p, "controller"))),
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
    container(scrollable(c))
        .padding(28)
        .width(Fill)
        .height(Fill)
        .into()
}
fn wizard(app: &App) -> Element<'_, Message> {
    let m = &app.machine_draft;
    let p = &m.p;
    let t = Target::Machine;
    let steps = [
        "Identity",
        "Travel & spindle",
        "Kinematics",
        "Controller",
        "Review",
    ];
    let mut nav = Row::new().spacing(8);
    for (i, label) in steps.iter().enumerate() {
        nav = nav.push(btn(
            format!("{}  {label}", i + 1),
            Message::WizardStep(i),
            i == app.wizard_step,
        ));
    }
    let mut form = column![bold(steps[app.wizard_step], 22)].spacing(16);
    match app.wizard_step {
        0 => {
            form=form.push(field(p,"name","Machine name",t)).push(choice(p,"axes","Number of axes",&["3","4","5"],t)).push(choice(p,"units","Machine specification units",&["mm","inch"],t)).push(note("Machine profile values use these units. The CAM project and tool definitions use millimeters."));
        }
        1 => {
            for (k, l) in [
                ("x", "X travel"),
                ("y", "Y travel"),
                ("z", "Z travel"),
                ("rpm", "Maximum spindle speed · rpm"),
                ("power", "Spindle power · W"),
                ("feed", "Maximum feed · machine units/min"),
            ] {
                form = form.push(field(p, k, l, t));
            }
        }
        2 => {
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
            } else {
                form = form.push(note(
                    "Fixed spindle, linear X/Y/Z motion. No rotary kinematics are needed.",
                ));
            }
        }
        3 => {
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
                    .push(note("Setups keep a snapshot of the machine. After saving, use Assign to setup so existing setups use these settings."));
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
                                btn("Browse…", Message::BrowseUc100Profile, false)
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
                    ].spacing(8))
                    .push(warnings(crate::control::probe_settings(p).err().unwrap_or_default()))
                    .push(note("AXIS CAM drives this machine itself from the Control page, one tool run at a time with a manual tool change in between. The profile is the UC100 driver's machine file: pins, steps per mm, speeds, limits and homing. Set this machine's X, Y and Z travel to the profile's soft limits: Cycle Start checks that they agree."));
            }
        }
        _ => {
            form=form.push(bold(m.name(),18)).push(muted(format!("{}-axis · {} · {}",m.axes(),get(p,"mode"),get(p,"controller")))).push(muted(format!("Travel: {} × {} × {} {}",get(p,"x"),get(p,"y"),get(p,"z"),get(p,"units")))).push(muted(format!("Spindle: {} rpm · Feed: {} {}/min",get(p,"rpm"),get(p,"feed"),get(p,"units")))).push(muted(format!("Post: {} · {} · G64 P{} · dwell {} s", post_setting(p, "post_tool_length"), post_setting(p, "post_retract"), post_setting(p, "post_blend"), post_setting(p, "post_dwell")))).push(warnings(m.issues())).push(note("Save this profile to your local library. Assign it to a setup to capture this version of its settings."));
        }
    }
    let mut actions = row![
        btn("Cancel", Message::Page(Page::Machines), false),
        space().width(Fill)
    ]
    .spacing(8);
    if app.wizard_step > 0 {
        actions = actions.push(btn(
            "Previous",
            Message::WizardStep(app.wizard_step - 1),
            false,
        ));
    }
    actions = actions.push(if app.wizard_step < 4 {
        primary("Continue", Message::WizardStep(app.wizard_step + 1))
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
    let label = label.into();
    let b = button(text(label.clone()).size(12))
        .padding([7, 12])
        .on_press_maybe(msg.clone())
        .style(move |_, status| {
            let (background, text_color) = match status {
                _ if on => (Color::from_rgb8(223, 237, 249), BLUE),
                button::Status::Disabled => (BG, MUTED),
                button::Status::Hovered | button::Status::Pressed => {
                    (Color::from_rgb8(235, 240, 246), INK)
                }
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

const AXES: [&str; 6] = ["X", "Y", "Z", "A", "B", "C"];

/// "X", "X and Y", "X, Y and Z".
fn axis_names(axes: &[usize]) -> String {
    let names: Vec<&str> = axes.iter().map(|&a| AXES[a]).collect();
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => names.concat(),
    }
}

/// The Control page: the UC100 machine, its connection and its live state.
fn control(app: &App) -> Element<'_, Message> {
    use crate::control::Action;
    use uc100::{controller::State, gcode::Spindle};
    let target = app.control_target();
    let mut left = column![bold("Machine", 18)].spacing(10);
    let machines: Vec<&Machine> = app
        .doc
        .machines
        .iter()
        .filter(|m| get(&m.p, "controller") == "UC100")
        .collect();
    if machines.is_empty() {
        left = left
            .push(note("No UC100 machine yet. In the Machine library, set a machine's controller to UC100 and choose its UC100 profile file."))
            .push(btn("Machine library", Message::Page(Page::Machines), false));
    }
    for m in &machines {
        left = left.push(btn(
            m.name(),
            Message::ControlMachine(m.id.clone()),
            target.is_some_and(|t| t.id == m.id),
        ));
    }
    if let Some(m) = target {
        left = left.push(muted(format!(
            "Connection: {}",
            crate::control::connection_label(m)
        )));
        // The profile is read only while disconnected, not on every 33 ms tick.
        left = match &app.control {
            Some(_) => left.push(btn("Disconnect", Message::Disconnect, false)),
            None => match crate::control::profile(m) {
                Ok(_) => left.push(primary("Connect", Message::Connect)),
                Err(e) => left.push(warnings(vec![e])),
            },
        };
    }
    let Some(c) = &app.control else {
        let main: Element<_> = if app.machine_setup {
            crate::setup::view(app)
        } else {
            container(muted("Not connected."))
                .padding(16)
                .width(Fill)
                .into()
        };
        return row![
            container(left).padding(16).width(300).style(panel_style),
            main
        ]
        .spacing(12)
        .padding(12)
        .height(Fill)
        .into();
    };
    let s = &c.snapshot;
    // ponytail: the last 40 log lines, newest last; a scrollable log when someone needs older lines.
    let log = s
        .log
        .iter()
        .skip(s.log.len().saturating_sub(40))
        .fold(column![].spacing(2), |col, line| {
            col.push(text(line.clone()).size(11).font(Font::MONOSPACE))
        });
    let left = container(
        column![
            left,
            rule::horizontal(1),
            bold("Log", 14),
            scrollable(log).anchor_bottom().height(Fill)
        ]
        .spacing(10),
    )
    .padding(16)
    .width(300)
    .height(Fill)
    .style(panel_style);
    if app.machine_setup {
        return row![left, crate::setup::view(app)]
            .spacing(12)
            .padding(12)
            .height(Fill)
            .into();
    }
    let mut head = row![bold(
        s.state.as_ref().map_or("Connecting…", State::label),
        28
    )]
    .spacing(12)
    .align_y(Alignment::Center);
    if c.simulated {
        head = head.push(
            container(bold("SIMULATED", 12).color(Color::WHITE))
                .padding([4, 8])
                .style(|_| container::Style {
                    background: Some(Color::from_rgb8(196, 120, 20).into()),
                    border: Border {
                        radius: 4.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        );
    }
    if s.state == Some(State::EStop) {
        head = head.push(primary("Reset", Message::MachineReset));
    }
    // Fixed above the part of the page that scrolls: the state, the E-stop
    // and the Run strip, so Cycle Start, Hold, Resume and Stop never scroll
    // out of sight below the tool-run list or the manual controls (#71).
    let mut fixed = column![head.push(estop())].spacing(12).width(Fill);
    if s.state == Some(State::EStop)
        && let Some(reason) = &s.estop_reason
    {
        fixed = fixed.push(text(reason.clone()).size(13));
    }
    let fixed = fixed.push(run_strip(app, c));
    let mut main = column![].spacing(12).width(Fill);
    if let Some(id) = &s.identity {
        let v = |o: &Option<String>| o.clone().unwrap_or_else(|| "?".into());
        main = main.push(muted(format!(
            "UC100 firmware {} · hardware {} · bootloader {}",
            v(&id.firmware),
            v(&id.hardware_id),
            v(&id.bootloader)
        )));
    }
    let unset = c.unset_limited_axes();
    let limits: Element<_> = if !c.soft_limits_configured() {
        muted("This profile sets no soft limits: nothing stops an axis at the end of its travel.")
            .into()
    } else if !unset.is_empty() {
        let one = unset.len() == 1;
        caution(
            "Soft limits are OFF",
            vec![
                format!(
                    "{} {} no machine position (every E-stop clears it), so nothing stops {} at the end of {} travel. Jog slowly and watch the machine.",
                    axis_names(&unset),
                    if one { "has" } else { "have" },
                    if one { "it" } else { "them" },
                    if one { "its" } else { "their" },
                ),
                if c.home_switches() {
                    "Home below to turn them on."
                } else {
                    "Set machine zero below to turn them on."
                }
                .into(),
            ],
        )
    } else {
        let ranges: Vec<String> = (0..6)
            .filter_map(|a| {
                let x = c.config.axes.get(a)?;
                Some(format!("{} {}…{}", AXES[a], x.soft_min?, x.soft_max?))
            })
            .collect();
        muted(format!("Soft limits on: {} mm.", ranges.join(", ")))
            .color(Color::from_rgb8(39, 130, 93))
            .into()
    };
    main = main.push(limits);
    // The setup picked to run, beside its tool runs, with the machine's tool
    // while a tool run cuts: the Work page is out of reach while it moves.
    let job = c
        .run
        .as_ref()
        .filter(|r| {
            app.doc
                .setups
                .get(r.setup)
                .is_some_and(|x| x.id == app.doc.active)
        })
        .and_then(|_| scene(app))
        .map(|view| {
            machine_panel(
                "Job",
                column![
                    container(view).height(360),
                    muted("Drag to orbit · right-drag to pan · scroll to zoom. The machine's tool is red while a tool run cuts."),
                    uncut_line(app).map(muted)
                ],
            )
        });
    main = match (run_panel(app, c), job) {
        (Some(runs), Some(job)) => main.push(row![runs, job].spacing(12)),
        (Some(runs), None) => main.push(runs),
        (None, Some(job)) => main.push(job),
        (None, None) => main,
    };
    let enabled: Vec<usize> = (0..6).filter(|&a| s.axes_enabled[a]).collect();
    let mono = |v: String| text(v).size(14).font(Font::MONOSPACE).width(110);
    let mut table = column![row![
        bold("Axis", 12).width(50),
        bold("Machine", 12).width(110),
        bold(format!("Work (G{})", 54 + s.active_wcs), 12).width(110),
        bold("Position set", 12)
    ]]
    .spacing(4);
    for &i in &enabled {
        table = table.push(row![
            text(AXES[i]).size(14).width(50),
            mono(format!("{:.3}", s.machine_pos[i])),
            mono(format!("{:.3}", s.work_pos[i])),
            text(if s.referenced[i] { "Yes" } else { "No" }).size(14),
        ]);
    }
    let idle = |a: Action| c.idle().then_some(Message::Machine(a));
    // A declared position references only an axis without a home switch.
    let declared: Vec<usize> = enabled
        .iter()
        .copied()
        .filter(|&a| a < 3 && c.switchless(a))
        .collect();
    let mut zero = column![bold("Machine zero", 13)].spacing(10);
    if !declared.is_empty() {
        let mut here = row![act(
            "Set machine zero here",
            idle(Action::MachineZero(None)),
            false
        )]
        .spacing(6)
        .align_y(Alignment::Center)
        .push(muted("or one axis"));
        for &a in &declared {
            here = here.push(act(AXES[a], idle(Action::MachineZero(Some(a))), false));
        }
        zero = zero.push(here).push(act(
            "Keep the last position",
            idle(Action::KeepPosition),
            false,
        ));
    }
    if c.home_switches() {
        zero = zero.push(act("Home", idle(Action::Home), false));
    }
    let model = crate::machine_view::MachineView {
        pos: [
            s.machine_pos[0] as f32,
            s.machine_pos[1] as f32,
            s.machine_pos[2] as f32,
        ],
        known: (0..3).all(|a| !c.axis(a) || s.referenced[a]),
        envelope: crate::machine_view::MachineView::envelope(&c.config),
        block: c
            .block
            .map(|(lo, hi)| [lo.map(|v| v as f32), hi.map(|v| v as f32)]),
        zero: std::array::from_fn(|k| s.work_offset[k] as f32),
    };
    let position = machine_panel(
        "Position",
        column![
            table,
            zero,
            note(if declared.is_empty() {
                HOME_NOTE
            } else {
                MACHINE_ZERO_NOTE
            })
        ],
    );
    let model = container(canvas(model).width(Fill).height(400))
        .padding(1)
        .width(Fill)
        .style(panel_style);
    let can_jog = c.manual() || s.state == Some(State::Jog);
    let mut modes = row![act(
        "Continuous",
        Some(Message::Machine(Action::JogStep(None))),
        c.jog_step.is_none()
    )]
    .spacing(6);
    for step in [0.01, 0.1, 1., 10.] {
        modes = modes.push(act(
            format!("{step} mm"),
            Some(Message::Machine(Action::JogStep(Some(step)))),
            c.jog_step == Some(step),
        ));
    }
    // Hold to jog: a press starts it; a release, anywhere, or leaving the
    // button stops it.
    let jog = |axis: usize, negative: bool| -> Element<'_, Message> {
        if !c.axis(axis) {
            return space().width(64).height(44).into();
        }
        let held = c.held() == Some((axis, negative));
        let label = format!("{}{}", AXES[axis], if negative { "−" } else { "+" });
        let face = container(bold(label, 15).color(if held {
            Color::WHITE
        } else if can_jog {
            INK
        } else {
            MUTED
        }))
        .center_x(64)
        .center_y(44)
        .style(move |_| container::Style {
            background: Some(
                if held {
                    BLUE
                } else if can_jog {
                    Color::WHITE
                } else {
                    BG
                }
                .into(),
            ),
            border: Border {
                color: if held { BLUE } else { LINE },
                width: 1.,
                radius: 6.into(),
            },
            ..Default::default()
        });
        let release = Message::Machine(Action::JogRelease);
        mouse_area(face)
            .on_press(Message::Machine(Action::JogPress(axis, negative)))
            .on_release(release.clone())
            .on_exit(release)
            .into()
    };
    let gap = || space().width(64).height(44);
    let pad = row![
        column![gap(), jog(0, true), gap()].spacing(6),
        column![jog(1, false), gap(), jog(1, true)].spacing(6),
        column![gap(), jog(0, false), gap()].spacing(6),
        space().width(18),
        column![jog(2, false), gap(), jog(2, true)].spacing(6),
    ]
    .spacing(6);
    let speed = row![
        text(format!(
            "Speed {:.0} % · X {:.0} mm/min",
            c.jog_speed,
            c.jog_feed(0)
        ))
        .size(12)
        .width(190),
        slider(1.0..=100.0, c.jog_speed, |v| Message::Machine(
            Action::JogSpeed(v)
        ))
        .step(1.0_f32)
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    let jog_panel = machine_panel(
        "Jog",
        column![
            modes,
            pad,
            speed,
            muted(
                "Hold a button, an arrow key (X, Y) or Page Up/Down (Z) to jog, and let go to stop. The arrow and Page keys jog while these controls are on screen and no text field is being edited."
            )
        ],
    );
    let manual = |a: Action| c.manual().then_some(Message::Machine(a));
    let mut zeros = row![].spacing(6);
    for &a in &enabled {
        zeros = zeros.push(act(
            format!("Zero {}", AXES[a]),
            manual(Action::WorkZero(Some(a))),
            false,
        ));
    }
    zeros = zeros.push(act("Zero all", manual(Action::WorkZero(None)), false));
    let mut systems = row![].spacing(4);
    for i in 0..6 {
        systems = systems.push(act(
            format!("G{}", 54 + i),
            idle(Action::Wcs(i)),
            s.active_wcs == i,
        ));
    }
    let blocked = c.probe_z_blocked();
    // The stock top's work Z the plate lies on.
    let top = c.stock_top().unwrap_or_default();
    let mut probe = column![
        row![
            act(
                "Probe Z",
                blocked.is_none().then_some(Message::Machine(Action::ProbeZ)),
                c.probing() && !c.edging()
            ),
            muted(match (&blocked, &c.probe) {
                (Some(why), _) if !c.probing() => why.clone(),
                (_, Ok(p)) => format!(
                    "Touch plate on the stock top under the tool, probe clipped on: feeds down at {} mm/min, up to {} mm. Work Z at the plate's top becomes {} mm, the stock top Z {}, then the tool lifts {} mm.",
                    p.feed,
                    p.distance,
                    crate::post::exact_mm((top + p.plate.unwrap_or_default()) as f32),
                    crate::post::exact_mm(top as f32),
                    p.retract
                ),
                _ => String::new(),
            })
        ]
        .spacing(8)
        .align_y(Alignment::Center)
    ];
    probe = probe.push(match c.edges_setup() {
        Err(why) => Element::from(muted(why)),
        Ok((refs, p)) => {
            let blocked = c.edges_blocked();
            let name = crate::control::probe_name(refs);
            row![
                act(
                    name,
                    blocked.is_none().then_some(Message::Machine(Action::ProbeEdges)),
                    c.edging()
                ),
                muted(match (&blocked, &c.probe) {
                    (Some(why), _) if !c.edging() => why.clone(),
                    (_, Ok(pr)) => format!(
                        "Jog the probe above the stock top, over {}, and press {name}: it touches off {}, allowing for the {} mm tip, at {} mm/min, and sets the X, Y and Z work zero there.",
                        crate::control::probe_spot(refs),
                        crate::post::touch_off_spots(&p.origin, p.stock).unwrap_or_else(|| format!(
                            "{}, and puts X0 Y0 Z0 on Model origin from them",
                            crate::post::probe_spots(refs, p.stock)
                        )),
                        pr.tip.unwrap_or_default(),
                        pr.feed
                    ),
                    _ => String::new(),
                })
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        }
    });
    if let Some(note) = probe_note(c) {
        probe = probe.push(note);
    }
    if let Some((lo, hi)) = c.block {
        probe = probe.push(muted(format!(
            "Simulated: a block at machine X {:.0}…{:.0}, Y {:.0}…{:.0}, its top at Z {:.3}, stands for the stock{}.",
            lo[0],
            hi[0],
            lo[1],
            hi[1],
            hi[2],
            if c.config.inputs.probe.is_some() {
                " and trips the probe"
            } else {
                ""
            }
        )));
    }
    let work_panel = machine_panel(
        &format!("Work zero (G{})", 54 + s.active_wcs),
        column![
            zeros,
            systems,
            muted(
                "Zero makes the current position work zero in the active coordinate system. G54 to G59 each keep their own; they change only while Idle."
            ),
            probe
        ],
    );
    let start = |state| {
        (c.spindle_ok(state) && c.rpm_value().is_some())
            .then_some(Message::Machine(Action::Spindle(state)))
    };
    let mut spindle_row = row![
        text_input("RPM", &c.rpm)
            .on_input(|v| Message::Machine(Action::Rpm(v)))
            .width(90)
            .padding([7, 9])
            .size(13),
        act("CW", start(Spindle::Cw), s.spindle == Some(Spindle::Cw))
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    if c.reversible() {
        spindle_row = spindle_row.push(act(
            "CCW",
            start(Spindle::Ccw),
            s.spindle == Some(Spindle::Ccw),
        ));
    }
    spindle_row = spindle_row.push(act(
        "Stop",
        Some(Message::Machine(Action::Spindle(Spindle::Off))),
        false,
    ));
    let turning = match s.spindle {
        Some(Spindle::Cw) => format!("CW at {:.0} rpm", s.rpm),
        Some(Spindle::Ccw) => format!("CCW at {:.0} rpm", s.rpm),
        _ => "off".into(),
    };
    let mut spindle = column![spindle_row, muted(format!("Spindle {turning}."))];
    if !c.rpm.trim().is_empty() && c.rpm_value().is_none() {
        spindle = spindle.push(muted(format!(
            "Enter an RPM from 1 to {:.0}.",
            c.config.outputs.spindle_max_rpm
        )));
    }
    if !c.reversible() {
        spindle = spindle.push(muted(
            "No CCW: the profile has no spindle direction output.",
        ));
    }
    let coolant = |on: bool, label: &'static str, f: fn(bool) -> Action| {
        checkbox(on)
            .label(label)
            .on_toggle(move |b| Message::Machine(f(b)))
            .text_size(12)
    };
    if c.config.outputs.flood.is_some() {
        spindle = spindle.push(coolant(s.flood, "Flood (M8)", Action::Flood));
    }
    if c.config.outputs.mist.is_some() {
        spindle = spindle.push(coolant(s.mist, "Mist (M7)", Action::Mist));
    }
    if c.config.outputs.flood.is_none() && c.config.outputs.mist.is_none() {
        spindle = spindle.push(muted("The profile has no coolant outputs."));
    }
    let mut overrides = column![
        row![
            text(format!("Feed override {:.0} %", c.feed_override))
                .size(12)
                .width(140),
            slider(10.0..=200.0, c.feed_override, |v| Message::Machine(
                Action::FeedOverride(v)
            ))
            .step(5.0_f32)
        ]
        .spacing(10)
        .align_y(Alignment::Center),
        muted(format!("Feed now {:.0} mm/min.", s.feed_rate))
    ];
    if c.limit_inputs() {
        overrides = overrides.push(
            checkbox(s.limits_override)
                .label("Override limit switches")
                .on_toggle(|b| Message::Machine(Action::OverrideLimits(b)))
                .text_size(12),
        );
        overrides = overrides.push(muted("To drive off a limit switch that tripped. It turns itself off a second after no switch is active."));
    }
    // Jog beside the position, so both fit without scrolling; the model
    // beside the work zero and the spindle.
    main = main.push(row![position, jog_panel].spacing(12)).push(
        row![
            model,
            column![
                work_panel,
                machine_panel("Spindle and coolant", spindle),
                machine_panel("Overrides", overrides)
            ]
            .spacing(12)
        ]
        .spacing(12),
    );
    let send =
        (c.manual() && !c.mdi.trim().is_empty()).then_some(Message::Machine(Action::MdiSend));
    main = main.push(machine_panel(
        "MDI",
        column![
            row![
                text_input("One G-code line, for example G0 X10 Y10", &c.mdi)
                    .on_input(|v| Message::Machine(Action::MdiText(v)))
                    .on_submit_maybe(send.clone())
                    .padding([7, 9])
                    .size(13),
                act("Send", send, false)
            ]
            .spacing(6)
            .align_y(Alignment::Center),
            muted("Runs the line when Idle, or while a program is paused. Enter sends it too.")
        ],
    ));
    row![
        left,
        column![
            fixed.padding(4),
            scrollable(main.padding(4)).height(Fill).width(Fill)
        ]
        .spacing(4)
        .height(Fill)
        .width(Fill)
    ]
    .spacing(12)
    .padding(12)
    .height(Fill)
    .into()
}
/// The green Cycle Start button, greyed out without a message.
fn cycle_start<'a>(msg: Option<Message>) -> Element<'a, Message> {
    let on = msg.is_some();
    let b = button(bold("Cycle Start", 14).color(Color::WHITE))
        .on_press_maybe(msg.clone())
        .padding([8, 18])
        .style(move |_, status| button::Style {
            background: Some(
                if !on {
                    Color::from_rgb8(166, 186, 176)
                } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                    Color::from_rgb8(28, 108, 74)
                } else {
                    Color::from_rgb8(39, 130, 93)
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

/// The Run strip: the setup of this machine run tool run by tool run, with
/// Cycle Start, Hold, Resume and Stop, fixed above the part of the Control
/// page that scrolls, so they never scroll out of sight below the tool-run
/// list or the manual controls (#71). It shows the setup picked, the tool
/// run Cycle Start starts, the progress of the tool run being cut or what
/// holds Cycle Start back, and how the last tool run ended. The tool runs
/// themselves, the manual tool change between them and the work zero on
/// the stock are in the Tool runs panel (`run_panel`), which scrolls.
pub(crate) fn run_strip<'a>(
    app: &'a App,
    c: &'a crate::control::Connection,
) -> Element<'a, Message> {
    use crate::run::Action as R;
    use uc100::controller::State;
    let s = &c.snapshot;
    let running = c.run.as_ref().is_some_and(|r| r.running.is_some());
    let msg = |on: bool, a: R| on.then_some(Message::Run(a));
    let mut body = column![].spacing(10);
    let setups: Vec<(usize, &Setup)> = app
        .doc
        .setups
        .iter()
        .enumerate()
        .filter(|(_, x)| x.machine.id == c.machine_id)
        .collect();
    if setups.is_empty() {
        body = body.push(muted("No setup uses this machine. On the Work page, choose it as a setup's machine (Assign to setup) to run that setup here."));
    } else {
        let mut pick = row![muted("Setup")].spacing(6).align_y(Alignment::Center);
        for (si, x) in setups {
            let picked = c.run.as_ref().is_some_and(|r| r.setup == si);
            pick = pick.push(act(
                format!("{}. {}", si + 1, x.name()),
                msg(!running, R::Setup(si)),
                picked,
            ));
        }
        body = body.push(pick);
    }
    let Some(run) = &c.run else {
        return machine_panel(
            "Run",
            body.push(muted("Pick a setup: AXIS CAM checks it as export would and runs it here, one tool run at a time.")),
        );
    };
    let programs = match run.programs.as_deref() {
        None => {
            return machine_panel(
                "Run",
                body.push(muted("Checking the setup: simulating the job up to it…")),
            );
        }
        Some(Err(problems)) => {
            return machine_panel(
                "Run",
                body.push(caution("This setup cannot run", problems.clone()))
                    .push(act("Check again", msg(true, R::Check), false)),
            );
        }
        Some(Ok(programs)) => programs,
    };
    let count = programs.len();
    // The tool run Cycle Start starts: the list marks it "next" below.
    if !running && let Some(p) = programs.get(run.next) {
        body = body.push(muted(format!(
            "Next tool run: {} of {count}, T{} {}",
            run.next + 1,
            p.ops[0].tool,
            p.ops[0].tool_name
        )));
    }
    let mut problems = run.problems(c, &app.doc);
    problems.extend(app.planning_busy().map(String::from));
    let state = s.state.as_ref();
    body = body.push(
        row![
            cycle_start(problems.is_empty().then_some(Message::Run(R::CycleStart))),
            act("Hold", msg(state == Some(&State::Run), R::Hold), false),
            act(
                "Resume",
                msg(
                    matches!(state, Some(State::Hold | State::Paused(_))),
                    R::Resume
                ),
                false
            ),
            act("Stop", msg(crate::run::busy(state), R::Stop), false),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    );
    body = body.push(muted(
        "Space holds from any page while no text field is being edited, and resumes on this page.",
    ));
    if let Some((p, op, block)) = run.progress(s.current_line) {
        let i = run.running.as_ref().map_or(0, |r| r.index);
        body = body
            .push(
                text(format!(
                    "Tool run {} of {count} · operation {} of {}: {} · block {} of {}",
                    i + 1,
                    op + 1,
                    p.ops.len(),
                    p.ops[op].name,
                    block + 1,
                    p.blocks.len()
                ))
                .size(13),
            )
            .push(widget::progress_bar(
                0.0..=p.blocks.len() as f32,
                block as f32 + 1.,
            ));
    } else if !problems.is_empty() && !crate::run::busy(state) {
        body = body.push(caution("Before Cycle Start", problems));
        let off = run.coolant_off(c, &app.doc);
        if !off.is_empty() {
            let ops = &app.doc.setups[run.setup].operations;
            let names: Vec<&str> = off.iter().map(|&i| ops[i].name()).collect();
            body = body.push(act(
                format!("Set coolant to Off: {}", names.join(", ")),
                msg(!running, R::CoolantOff),
                false,
            ));
        }
    }
    if let Some((ok, message)) = &run.outcome {
        body = body.push(text(message.clone()).size(12).color(if *ok {
            Color::from_rgb8(39, 130, 93)
        } else {
            Color::from_rgb8(166, 101, 32)
        }));
    }
    machine_panel(&format!("Run · {}", run.name), body)
}

/// The Tool runs panel, in the part of the Control page that scrolls, under
/// the Run strip: the setup's tool runs (Cycle Start starts the one marked
/// next), the manual tool change between tool runs, and the work zero on
/// the stock before the first. None until the setup is checked; the Run
/// strip says what is being done or why it cannot run.
pub(crate) fn run_panel<'a>(
    app: &'a App,
    c: &'a crate::control::Connection,
) -> Option<Element<'a, Message>> {
    use crate::run::Action as R;
    let s = &c.snapshot;
    let run = c.run.as_ref()?;
    let programs = run.checked()?;
    let running = run.running.is_some();
    let msg = |on: bool, a: R| on.then_some(Message::Run(a));
    let mut body = column![].spacing(10);
    let count = programs.len();
    for (i, p) in programs.iter().enumerate() {
        let mark = if run.running.as_ref().is_some_and(|r| r.index == i) {
            "running"
        } else if run.done[i] {
            "done"
        } else if i == run.next {
            "next"
        } else {
            ""
        };
        let ops: Vec<&str> = p.ops.iter().map(|o| o.name.as_str()).collect();
        body = body.push(
            row![
                text(mark).size(12).width(52),
                act(
                    format!(
                        "Tool run {} of {count}: T{} {}",
                        i + 1,
                        p.ops[0].tool,
                        p.ops[0].tool_name
                    ),
                    msg(!running, R::Next(i)),
                    i == run.next,
                ),
                muted(ops.join(" → ")),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    }
    if run.stale {
        body = body
            .push(caution(
                "The project changed since this setup was checked",
                vec!["Check it again before the next tool run.".into()],
            ))
            .push(act("Check again", msg(!running, R::Check), false));
    }
    if let Some(p) = run.tool_change() {
        let o = &p.ops[0];
        body = body.push(note(format!(
            "Tool change for tool run {}: T{} {} (stickout {:.1} mm). The last tool run ended with the spindle and coolant off at the clearance plane. 1. If no reference is recorded, touch the old tool on a surface that stays put (a vise jaw, the table, a block) and press Record reference. 2. Jog up and change to T{}. 3. Touch the new tool on the same surface and press Set Z from reference, or Zero Z on the stock top if no tool run has cut it. With a touch plate on that surface, Probe reference and Probe Z from reference do steps 1 and 3. 4. Jog clear and press Cycle Start. X/Y zero stays as it is.",
            run.next + 1,
            o.tool,
            o.tool_name,
            o.stickout,
            o.tool
        )));
    }
    if count > 1 {
        let manual = c.manual() && !running;
        body = body.push(
            row![
                muted(match run.reference {
                    Some(z) => format!("Reference surface: Z {z:.3} (G{})", 54 + s.active_wcs),
                    None => "Reference surface: not recorded".into(),
                }),
                act("Record reference", msg(manual, R::RecordReference), false),
                act(
                    "Set Z from reference",
                    msg(manual && run.reference.is_some(), R::ZFromReference),
                    false
                ),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
        if c.config.inputs.probe.is_some() {
            let probe = c.probe_blocked().is_none() && !running;
            body = body.push(
                row![
                    act("Probe reference", msg(probe, R::ProbeReference), false),
                    act(
                        "Probe Z from reference",
                        msg(probe && run.reference.is_some(), R::ProbeFromReference),
                        false
                    ),
                    muted("The same with the touch plate on the reference surface."),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
            if let Some(note) = probe_note(c) {
                body = body.push(note);
            }
        }
    }
    if let Some(steps) = work_zero_steps(app, c) {
        use crate::control::Action;
        // The buttons act on the checked setup the steps are from, which an
        // edit may have changed: like the probes, they wait while its check
        // is out of date, and say so (#39).
        let checked = c.run_program();
        let mut buttons = row![].spacing(8).align_y(Alignment::Center);
        if c.sim_zero().is_some() {
            let ready = checked.is_ok() && c.idle() && c.unset_limited_axes().is_empty();
            buttons = buttons.push(act(
                "Move there",
                ready.then_some(Message::Machine(Action::ToSimZero)),
                false,
            ));
        }
        let manual = checked.is_ok() && c.manual();
        buttons = buttons.push(match c.touch_off() {
            Some((_, work)) => act(
                touch_label(work),
                manual.then_some(Message::Machine(Action::TouchOff)),
                false,
            ),
            None => act(
                "Zero all",
                manual.then_some(Message::Machine(Action::WorkZero(None))),
                false,
            ),
        });
        if let Err(why) = checked {
            buttons = buttons.push(muted(why));
        }
        body = body
            .push(bold("Work zero on the stock", 13))
            .push(note(steps.join("\n")))
            .push(buttons);
    }
    Some(machine_panel("Tool runs", body))
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
        "1. Home under Position.".into()
    } else {
        "1. Set machine zero under Position: jog X to its left end, Y to its front end and Z to the top first.".into()
    });
    steps.push(if p.offset == format!("G{}", 54 + s.active_wcs) {
        format!("2. {} is the active work system.", p.offset)
    } else {
        format!("2. Select {} under Work zero.", p.offset)
    });
    let checks =
        "The checks above then say whether every tool run fits the soft limits from there.";
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
    if c.config.inputs.probe.is_some() {
        steps.push(match (c.edges_setup(), c.stock_top()) {
            (Ok((refs, _)), _) => format!(
                "With the probe clipped on, {} under Work zero does 3 and 4.",
                crate::control::probe_name(refs)
            ),
            (Err(why), Ok(top)) => format!(
                "{why} With the touch plate on the stock top, Probe Z under Work zero sets Z{}.",
                crate::post::exact_mm(top as f32)
            ),
            (Err(why), Err(_)) => why,
        });
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
const HOME_NOTE: &str = "Home references every axis on its switch. Every E-stop clears the machine position: home again after one.";
const MACHINE_ZERO_NOTE: &str = "Every E-stop clears the machine position: set it again after one. Without home switches, jog X to its left end, Y to its front end (gantry toward you) and Z to the top, then Set machine zero here. Keep the last position takes the position AXIS CAM saved at the last disconnect: only if nothing moved the machine since.";
fn tools(app: &App) -> Element<'_, Message> {
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
                    "T{} · {}\n{} · Ø{} mm",
                    get(&t.p, "number"),
                    t.name(),
                    get(&t.p, "type"),
                    get(&t.p, "diameter")
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
                "Chamfer mill"
            ],
            t
        )
    ]
    .spacing(14);
    for (k, l) in [
        ("number", "Tool number · T"),
        ("diameter", "Diameter · mm"),
        ("flutes", "Flutes"),
        ("length", "Flute length · mm"),
        ("stickout", "Stickout · mm"),
        ("holder", "Holder diameter · mm"),
    ] {
        form = form.push(field(p, k, l, t));
    }
    let mut tool_issues = app.tool_draft.issues();
    tool_issues.extend(app.doc.tool_number_issues(&app.tool_draft));
    form = form.push(warnings(tool_issues));
    // Below the scrolling fields, so Save stays in sight.
    let actions = row![
        primary(
            app.resume_label().unwrap_or("Save tool".into()),
            Message::SaveTool
        ),
        btn(
            "Delete tool",
            Message::DeleteTool(app.tool_draft.id.clone()),
            false
        )
    ]
    .spacing(10);
    container(column![page_header("Tools","Cutter dimensions are stored in millimeters. Tool edits invalidate generated previews."),row![container(scrollable(list)).padding(20).width(320).height(Fill).style(panel_style),container(column![scrollable(form).height(Fill),actions].spacing(14)).padding(24).width(Fill).height(Fill).style(panel_style)].spacing(20).height(Fill)].spacing(24)).padding(28).height(Fill).into()
}
fn materials(app: &App) -> Element<'_, Message> {
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
    form = form.push(warnings(issues)).push(
        row![
            primary("Save material", Message::SaveMaterial),
            btn(
                "Delete material",
                Message::DeleteMaterial(app.material_draft.id.clone()),
                false
            )
        ]
        .spacing(10),
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
                container(scrollable(form))
                    .padding(24)
                    .width(Fill)
                    .height(Fill)
                    .style(panel_style)
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
const VISE_FIELDS: [(&str, &str); 6] = [
    ("jaw_width", "Jaw width · mm"),
    ("jaw_height", "Jaw height · mm"),
    ("jaw_thickness", "Jaw thickness · mm"),
    ("max_opening", "Maximum opening · mm"),
    ("grip", "Grip height · mm"),
    ("force", "Clamping force · N"),
];
const CLAMP_FIELDS: [(&str, &str); 5] = [
    ("length", "Length · mm"),
    ("width", "Width · mm"),
    ("height", "Height above the stock · mm"),
    ("bite", "Bite onto the stock · mm"),
    ("force", "Clamping force · N"),
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
        other => other.to_string(),
    }
}
fn workholding(app: &App) -> Element<'_, Message> {
    let mut list = column![
        bold("Workholding library", 17),
        row![
            primary("+ New vise", Message::NewHolder("Vise")),
            primary("+ New clamp", Message::NewHolder("Clamp"))
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
        _ => ("Workholding definition", &[]),
    };
    let mut form = column![bold(title, 21), field(p, "name", "Name", t)].spacing(14);
    for (key, label) in sizes {
        form = form.push(field(p, key, label, t));
    }
    let mut issues = draft.issues();
    issues.extend(app.doc.holder_name_issues(draft));
    form = form
        .push(field(p, "notes", "Notes", t))
        .push(warnings(issues))
        .push(
            row![
                primary("Save", Message::SaveHolder),
                btn("Delete", Message::DeleteHolder(draft.id.clone()), false)
            ]
            .spacing(10),
        );
    container(
        column![
            page_header(
                "Workholding",
                "Vises and toe clamps your setups can model. The starter entries have typical sizes: measure yours."
            ),
            row![
                container(scrollable(list))
                    .padding(20)
                    .width(320)
                    .height(Fill)
                    .style(panel_style),
                container(scrollable(form))
                    .padding(24)
                    .width(Fill)
                    .height(Fill)
                    .style(panel_style)
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
                .push(btn("Cancel", Message::SignInCancel, false));
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
                        btn("Manage usage", manage(), false)
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
                    c = c.push(btn("Load models", Message::LoadModels, false).width(Fill));
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
                        ]
                        .spacing(5),
                    );
                }
                if !app.settings.model.is_empty() {
                    c = if app.account_busy {
                        c.push(muted("Checking the connection…"))
                    } else {
                        c.push(btn("Check connection", Message::CheckConnection, false).width(Fill))
                    };
                }
            }
            if note_above {
                c = c.push(note(app.account_note.clone()));
            }
            c = c.push(btn("Sign out", Message::SignOut, false).width(Fill));
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
    stage_text(stage, &app.plan_model)
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
pub(crate) const SENT_TO_OPENAI: &str = "Sent to OpenAI: the part's dimensions and feature list, your tools, machine limits, the project material (its name, group, hardness, notes and cutting data), stock size, the sizes and positions of setup 1's vise jaws or clamps and their clearance gap (not their names), the sizes of every vise and clamp in your workholding library (sizes only, never their names or notes), and the app's check reports and tool answers while the model works. The CAD file, file paths, the project title and your setup names are not sent.";
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
pub(crate) const PICTURES_HELP: &str = "Ticked, the planner also sends OpenAI pictures the app draws of the part, the stock and setup 1's vise or clamps, with surface and hole labels: an overview with the first request, and the views the model asks for, its own toolpaths among them. Unticked, it sends text only. The CAD file is never sent. A run keeps the choice it started with: the box cannot be changed until the run is over.";
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
        Help::text("Pictures for the AI planner", PICTURES_HELP),
        widget::tooltip::Position::Top,
    ));
    if !ready {
        c = c.push(muted(
            "Plan becomes available when every item above is met.",
        ));
    }
    if app.undo.is_some() {
        c = c.push(btn("Undo applied plan", Message::UndoPlan, false).width(Fill));
    }
    c.into()
}
/// Model text is shown as plain text, shortened to keep the layout stable.
/// One character more than shown is read, to know whether to add "…".
fn clip(s: &str, limit: usize) -> String {
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
            push(format!("facing area {}", get(p, "boundary").to_lowercase()));
            push(stepdown);
            push(stepover);
            bottom.into_iter().for_each(&mut push);
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
        Strategy::Drill => {
            let cycle = get(p, "cycle");
            push(if ["Chip breaking", "Deep drilling"].contains(&cycle) {
                format!("{cycle}, peck {} mm", n(value("peck")))
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
    if let Some(error) = &outcome.note {
        lines.push(format!("A follow-up request failed: {}", error.message()));
    }
    lines
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
/// The NC program page's note on what export checks, for a job whose setups
/// model their vise or clamps as `held` says.
pub(crate) fn program_note(held: Held) -> &'static str {
    match held {
        Held::No => {
            "LinuxCNC 3-axis output, one .ngc program per setup. Export repeats the export checks — limits, travel, rapids into material, holder clearance — and a replay of each program against the simulated moves, and writes nothing if any setup fails. The planner's tool-axis and gouge checks covered the plan as applied, not later edits. Fixtures, clamps and the machine body are not checked: preview each program in LinuxCNC and air-cut first."
        }
        Held::Every => {
            "LinuxCNC 3-axis output, one .ngc program per setup. Export repeats the export checks — limits, travel, rapids into material, holder clearance — and a replay of each program against the simulated moves, and writes nothing if any setup fails. The planner's tool-axis and gouge checks covered the plan as applied, not later edits. Every program move, the links between operations, approaches and retracts included, is checked against each setup's vise jaws or clamps as boxes with a clearance gap; parallels, bolts, the table, the machine body and tool-change motion are not checked, and retracts to machine home are assumed to clear everything: preview each program in LinuxCNC and air-cut first."
        }
        Held::Mixed => {
            "LinuxCNC 3-axis output, one .ngc program per setup. Export repeats the export checks — limits, travel, rapids into material, holder clearance — and a replay of each program against the simulated moves, and writes nothing if any setup fails. The planner's tool-axis and gouge checks covered the plan as applied, not later edits. Every program move of a setup that models a vise or clamps, the links between operations, approaches and retracts included, is checked against its jaws or clamps as boxes with a clearance gap; parallels, bolts, the table, the machine body and tool-change motion are not checked, and neither are fixtures and clamps in the setups whose workholding is not modelled or cannot be checked; retracts to machine home are assumed to clear everything: preview each program in LinuxCNC and air-cut first."
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
        Note::Facing => {
            "Selections use connected planar regions of the imported mesh. Facing simulation uses a rectangular raster preview. "
        }
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
fn plan_preview<'a>(app: &'a App, preview: &'a crate::PlanPreview) -> Element<'a, Message> {
    let outcome = &preview.outcome;
    let (job, check, proposal) = (&outcome.job, &outcome.check, &outcome.proposal);
    let actions = || {
        row![
            primary("Apply plan", Message::ApplyPlan),
            btn("Discard", Message::DiscardPlan, false)
        ]
        .spacing(10)
    };
    let mut c = column![bold("Plan ready to review", 21)].spacing(12);
    for line in preview_totals(outcome) {
        c = c.push(muted(line));
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
        c = c.push(
            column![
                bold("The planner's summary", 15),
                text(clip(&proposal.summary, 600)).size(13)
            ]
            .spacing(6),
        );
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
            card = card.push(muted(workholding_line(si, &clip(notes, 500))));
        }
        if let Some(line) = plan_holding_line(job, si) {
            card = card.push(muted(line));
        }
        for (oi, o) in setup.operations.iter().enumerate() {
            let tool = job
                .tools
                .iter()
                .find(|t| t.id == o.tool)
                .map(|t| format!("T{} {}", get(&t.p, "number"), t.name()))
                .unwrap_or_default();
            let minutes = check
                .operations
                .iter()
                .find(|r| r.setup == si + 1 && r.operation == oi + 1)
                .map_or(0., |r| r.minutes);
            let reason = proposed
                .and_then(|s| s.operations.get(oi))
                .map_or(String::new(), |p| clip(&p.reason, 160));
            let [head, values, why] = operation_lines(o, &tool, minutes, &reason);
            card = card.push(
                row![
                    icon(app, o.strategy, 22.),
                    column![text(head).size(13), muted(values), muted(why)].spacing(2)
                ]
                .spacing(8),
            );
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
        for item in &outcome.proposal.unmachined {
            block = block.push(
                text(format!(
                    "• {}: {}",
                    clip(&item.feature, 80),
                    clip(&item.reason, 240)
                ))
                .size(13),
            );
        }
        if outcome.more_unmachined > 0 {
            block = block.push(muted(format!("• and {} more", outcome.more_unmachined)));
        }
        c = c.push(block);
    }
    if !outcome.proposal.warnings.is_empty() {
        let mut block = column![bold("Warnings from the planner", 15)].spacing(6);
        for warning in &outcome.proposal.warnings {
            block = block.push(text(format!("• {}", clip(warning, 240))).size(13));
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
        let scene: Element<viewport::Event> = canvas(Viewport {
            setup,
            operation,
            mesh,
            simulation: Some(view),
            sample: false,
            camera: app.plan_camera,
            picking: false,
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            leftover: None,
            live: None,
            uncut: &[],
        })
        .width(Fill)
        .height(380)
        .into();
        c = c
            .push(bold("Stock after the whole plan", 15))
            .push(muted("Drag to orbit, right-drag to pan, scroll to zoom."))
            .push(
                container(scene.map(Message::PlanViewport))
                    .width(Fill)
                    .style(panel_style),
            );
    }
    c.push(note(apply_note(&app.doc, job)))
        .push(actions())
        .into()
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
            let head = match step.ended {
                Some(_) => done_text(step.stage, &app.plan_model),
                None => stage_text(step.stage, &app.plan_model),
            };
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
    for row in rows {
        let mark = match row.mark {
            Mark::Open => text("•").size(13).color(MUTED),
            Mark::Done => text("✓").size(13).color(GOOD),
            Mark::Warn => text("!").size(13).color(WARN),
        };
        let mut item = column![text(row.text).size(13)].spacing(2);
        for line in row.notes {
            item = item.push(muted(line));
        }
        for look in row.looks {
            let line = if look.warn {
                text(look.line).size(12).color(WARN)
            } else {
                muted(look.line)
            };
            item = item.push(match look.thumb {
                Some(handle) => Element::from(
                    row![widget::image(handle).width(120), line]
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
/// A finished run's timeline under `heading`, in a panel: one line a step.
fn finished_timeline<'a>(app: &App, heading: &'static str) -> Option<Element<'a, Message>> {
    let rows = timeline_view(app, true)?;
    Some(
        container(column![bold(heading, 15), rows].spacing(8))
            .padding(16)
            .width(Fill)
            .style(panel_style)
            .into(),
    )
}
fn plan_result(app: &App) -> Element<'_, Message> {
    use crate::Planner;
    match &app.planner {
        Planner::Idle => container(
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
                if app.plan_checklist().iter().all(|line| line.ok) {
                    Element::from(primary("Plan this job", Message::Plan))
                } else {
                    muted("Complete the checklist on the left to plan.").into()
                }
            ]
            .spacing(16),
        )
        .padding(35)
        .width(Fill)
        .style(panel_style)
        .into(),
        Planner::Running(stage) => {
            let mut c = column![bold(running_text(app, *stage), 20)].spacing(14);
            if let Some(overview) = &app.plan_overview {
                let mut picture = column![bold("The AI starts with this picture", 15)].spacing(6);
                if let Some(handle) = &overview.thumb {
                    picture = picture.push(widget::image(handle.clone()).width(240));
                }
                c = c.push(picture.push(muted(overview.line.clone())));
            }
            if let Some(rows) = timeline_view(app, false) {
                c = c.push(rows);
            }
            container(
                c.push(muted(
                    "This can take a few minutes. Nothing changes until you apply a plan.",
                ))
                .push(btn("Cancel", Message::PlanCancel, false)),
            )
            .padding(35)
            .width(Fill)
            .style(panel_style)
            .into()
        }
        Planner::Failed(failure) => {
            let mut c = column![].spacing(12);
            if let Some(tried) = finished_timeline(app, "What the planner tried") {
                c = c.push(tried);
            }
            c = c
                .push(bold("No plan this time", 21))
                .push(text(failure.message.clone()).size(13));
            for line in problem_lines(&failure.problems) {
                c = c.push(muted(line));
            }
            c.push(
                row![
                    primary("Try again", Message::Plan),
                    btn("Dismiss", Message::DiscardPlan, false)
                ]
                .spacing(10),
            )
            .into()
        }
        Planner::Preview(preview) => match finished_timeline(app, "How the plan was made") {
            Some(made) => column![made, plan_preview(app, preview)].spacing(12).into(),
            None => plan_preview(app, preview),
        },
    }
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
                scrollable(container(plan_result(app)).padding(iced::Padding::default().right(14)))
                    .id("planner-result")
                    .height(Fill)
                    .width(Fill)
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
fn post(app: &App) -> Element<'_, Message> {
    let mut summary =
        column![bold("Program review", 20), note(program_note(held(app))),].spacing(14);
    let lines = speed_feed_lines(app);
    if !lines.is_empty() {
        summary = summary.push(caution(
            speed_feed_heading(&lines),
            lines.into_iter().map(|(_, line)| line).collect(),
        ));
    }
    let mut code = column![].spacing(18);
    match &app.post {
        None => {
            summary = summary.push(muted(if app.post_busy {
                "Checking every setup and replaying its program…"
            } else {
                "Checks run when this page opens."
            }))
        }
        Some(job) => {
            if !job.errors.is_empty() {
                summary = summary.push(warnings(job.errors.clone()));
            }
            for s in &job.setups {
                match &s.result {
                    Ok(programs) => {
                        for program in programs {
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
                            code = code.push(
                                text(crate::post::preview(&program.text, 120))
                                    .font(Font::MONOSPACE)
                                    .size(12)
                                    .color(Color::from_rgb8(183, 211, 221)),
                            );
                        }
                    }
                    Err(problems) => {
                        summary = summary
                            .push(bold(format!("!  {}", s.name), 13))
                            .push(warnings(problems.clone()));
                        if (app.doc.setups.get(s.setup))
                            .is_some_and(|x| get(&x.machine.p, "controller") == "UC100")
                        {
                            summary = summary.push(
                                primary("Run on the machine", Message::RunOnMachine(s.setup))
                                    .width(Fill),
                            );
                        }
                    }
                }
            }
        }
    }
    summary = if app.post.as_ref().is_some_and(|job| job.ready()) {
        summary.push(primary("Export LinuxCNC G-code…", Message::ExportGcode).width(Fill))
    } else {
        summary.push(muted("Export becomes available when every setup passes."))
    };
    summary = summary
        .push(btn("Export setup sheet (.md)", Message::Export("sheet"), false).width(Fill))
        .push(btn("Save complete project (.json)", Message::SaveProject, false).width(Fill));
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
            page_header(
                "NC program",
                "Check each setup, then export LinuxCNC G-code."
            ),
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

fn operation_audit(app: &App) -> Element<'_, Message> {
    let mut c=column![bold("Operation support & job checks",24),note("Available means an imported-mesh preview is implemented and regression-tested within the scope below. Their moves can be exported as LinuxCNC 3-axis programs after checks for limits, rapids into material and holder clearance, and, in a setup that models a vise or clamps, for the jaws and clamps as boxes with a clearance gap. Parallels, bolts, the table, the machine body and other fixtures are not checked. A sample animation does not establish operation support."),row![primary("Check all enabled operations",Message::GenerateAll),btn("Back to machining",Message::Page(Page::Work),false)].spacing(12)].spacing(14).max_width(1020);
    for (name, ok, detail) in &app.audit_report {
        c = c.push(
            container(
                column![
                    bold(format!("{}  {name}", if *ok { "✓" } else { "!" }), 14),
                    text(detail).size(12)
                ]
                .spacing(5),
            )
            .padding(12)
            .width(Fill)
            .style(panel_style),
        );
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
    c=c.push(note("Remaining 3D stock carries through all enabled operations and setups, including flips. Fully cleared top layers are skipped automatically. Leftover-region rest machining, collision checks beyond modelled vise jaws and clamps, multi-axis kinematics and controllers other than LinuxCNC remain unavailable. Unavailable strategies stay visible for planning, but cannot be added as imported-part machining operations."));
    container(scrollable(c))
        .padding(24)
        .width(Fill)
        .height(Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

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

    #[test]
    fn sources_and_docs_have_no_mojibake() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut hits = Vec::new();
        for path in ["src", "docs", "README.md"] {
            scan(&root.join(path), &mut hits);
        }
        assert!(hits.is_empty(), "mojibake found:\n{}", hits.join("\n"));
    }
}
