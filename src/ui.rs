use crate::{
    App, Message, Page, Target,
    help::{self, Help, HelpButton},
    holding::Held,
    material::SpeedFeed,
    model::*,
    viewport::{self, Viewport},
};
use iced::{
    Alignment, Border, Color, Element, Font,
    Length::{self, Fill},
    Theme,
    widget::{
        self, Column, Row, button, canvas, checkbox, column, container, pick_list, row, rule,
        scrollable, slider, space, svg, text, text_input,
    },
};
const INK: Color = Color::from_rgb(0.13, 0.20, 0.28);
const MUTED: Color = Color::from_rgb(0.43, 0.50, 0.57);
const BLUE: Color = Color::from_rgb(0.13, 0.42, 0.71);
const LINE: Color = Color::from_rgb(0.85, 0.88, 0.91);
const BG: Color = Color::from_rgb(0.95, 0.96, 0.975);
fn bold<'a>(s: impl Into<String>, size: u32) -> widget::Text<'a> {
    text(s.into()).size(size).font(Font {
        weight: iced::font::Weight::Semibold,
        ..Font::DEFAULT
    })
}
fn muted<'a>(s: impl Into<String>) -> widget::Text<'a> {
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
fn btn<'a>(label: impl Into<String>, msg: Message, active: bool) -> HelpButton<'a> {
    let label = label.into();
    let action = msg.clone();
    let b = button(text(label.clone()).size(12))
        .on_press(msg)
        .padding([7, 10])
        .style(move |_, status| {
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
        });
    HelpButton::new(b, &action, label)
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
fn note<'a>(s: impl Into<String>) -> Element<'a, Message> {
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
fn caution<'a>(heading: impl Into<String>, lines: Vec<String>) -> Element<'a, Message> {
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
    let brand = row![
        bold("AXIS", 23).color(Color::WHITE),
        text("CAM STUDIO")
            .size(11)
            .color(Color::from_rgb8(164, 190, 213)),
        space().width(18),
        text("MANUFACTURE")
            .size(11)
            .color(Color::from_rgb8(184, 215, 241)),
        space().width(Fill),
        text(if app.dirty {
            "Saving workspace…"
        } else {
            "Workspace saved locally"
        })
        .size(11)
        .color(Color::from_rgb8(183, 204, 222)),
        space().width(10),
        text("NATIVE PROTOTYPE")
            .size(10)
            .color(Color::from_rgb8(228, 193, 122))
    ]
    .spacing(7)
    .align_y(Alignment::Center);
    let brand = container(brand)
        .padding([12, 18])
        .style(|_| container::Style {
            background: Some(Color::from_rgb8(29, 46, 63).into()),
            ..Default::default()
        });
    let docbar = container(
        row![
            btn("Start", Message::Page(Page::Home), app.page == Page::Home),
            btn("New project", Message::NewProject, false),
            help::wrap(
                text_input("Project name", &app.doc.title)
                    .on_input(Message::Title)
                    .width(140)
                    .size(14)
                    .padding(7),
                Help::text(
                    "Project name",
                    "The name shown in the browser and used as the suggested project filename."
                ),
                widget::tooltip::Position::Bottom
            ),
            space().width(Fill),
            btn("Import model", Message::Import, false),
            btn("Open", Message::OpenProject, false),
            btn("Save project", Message::SaveProject, false),
            rule::vertical(1),
            btn(
                "Machine library",
                Message::Page(Page::Machines),
                app.page == Page::Machines
            ),
            btn("Tools", Message::Page(Page::Tools), app.page == Page::Tools),
            btn(
                "Materials",
                Message::Page(Page::Materials),
                app.page == Page::Materials
            ),
            btn(
                "Workholding",
                Message::Page(Page::Workholding),
                app.page == Page::Workholding
            ),
            btn("AI planner", Message::Page(Page::Ai), app.page == Page::Ai),
            btn(
                "Operation support",
                Message::Page(Page::Audit),
                app.page == Page::Audit
            )
        ]
        .spacing(5)
        .align_y(Alignment::Center),
    )
    .height(48)
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
    if app.page != Page::Home && !app.doc.setups.is_empty() {
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
        row![btn("Set up a machine",Message::NewMachine,false),btn("Saved machines",Message::Page(Page::Machines),false),btn("Tool library",Message::Page(Page::Tools),false)].spacing(8),
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
        btn(if app.doc.machines.is_empty() {"Set up a machine"} else {"Open machine library"},if app.doc.machines.is_empty(){Message::NewMachine}else{Message::Page(Page::Machines)},false).width(Fill),
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
        ]
        .spacing(6);
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
            for (i, o) in s.operations.iter().enumerate() {
                let selected = app.selected == Some(i) && app.page == Page::Work;
                let ready = o.preview
                    || app
                        .active_simulation()
                        .is_some_and(|sim| sim.plan.operation == o.id);
                let mark = if !o.enabled {
                    "—"
                } else if ready {
                    "✓"
                } else {
                    "○"
                };
                let content = row![
                    icon(app, o.strategy, 23.),
                    text(o.name()).size(12).width(Fill),
                    text(mark).size(13).color(if ready {
                        Color::from_rgb8(38, 146, 111)
                    } else {
                        MUTED
                    })
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
        }
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
                        get(&s.p, "fixture"),
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
    let scene: Element<_> = if let Some(mesh) = app.part_mesh() {
        let canvas: Element<viewport::Event> = canvas(Viewport {
            setup: s,
            operation: app.operation(),
            mesh,
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
        })
        .width(Fill)
        .height(Fill)
        .into();
        canvas.map(Message::Viewport)
    } else {
        container(column![bold(if app.import_busy { "Loading geometry…" } else if !app.doc.has_model() { "Import your part" } else { "Model preview unavailable" },25),muted(app.doc.file.as_ref().map(|f|f.name.clone()).unwrap_or_default()),note(app.import_error.clone().unwrap_or_else(|| "Import STEP, STP, 3MF, STL or OBJ to view your model. Geometry loads in the background; a STEP conversion may take up to two minutes.".into())),primary("Import model",Message::Import),btn("Start screen",Message::Page(Page::Home),false)].spacing(20).max_width(380)).center(Fill).into()
    };
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
    let workflow = container(
        row![
            btn("01  Setup", Message::SetupEditor, app.selected.is_none()),
            btn(
                "02  Operations",
                Message::SelectOp(0),
                app.selected.is_some()
            ),
            btn("03  Preview", Message::Generate, false),
            btn("04  NC review", Message::Page(Page::Post), false)
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
                                            "Bore Ø{:.2} / opening Ø{:.2} · X{:.2} Y{:.2} · depth {:.2} mm",
                                            h.diameter, h.opening,
                                            h.center[0],
                                            h.center[1],
                                            h.top - h.bottom
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
fn setup_editor(app: &App) -> Element<'_, Message> {
    let s = app.doc.setup();
    let p = &s.p;
    let t = Target::Setup;
    let mut c = column![
        bold("Setup", 20),
        muted("Define placement, stock and workholding."),
        field(p, "name", "Setup name", t),
        section("Machine & coordinate system"),
        btn(
            format!(
                "{} · {}-axis · v{}",
                s.machine.name(),
                s.machine.axes(),
                s.machine.version
            ),
            Message::Page(Page::Machines),
            true
        )
        .width(Fill),
        choice(
            p,
            "offset",
            "Work offset",
            &["G54", "G55", "G56", "G57", "G58", "G59"],
            t
        ),
        choice(
            p,
            "origin",
            "Work coordinate origin",
            &[
                "Stock top center",
                "Stock top front-left",
                "Model origin",
                "Selected point (planned)"
            ],
            t
        ),
        material_picker(app),
        section("Stock")
    ]
    .spacing(12);
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
                if !crate::stock::relative(s)
                    && resolved.part.is_some_and(|part| {
                        (0..3).any(|i| {
                            part.min[i] < resolved.stock.min[i] - 0.001
                                || part.max[i] > resolved.stock.max[i] + 0.001
                        })
                    })
                {
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
    }
    c = c.push(section("Part position")).push(
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
    c = c.push(section("Workholding"));
    for control in holding_controls(app) {
        c = c.push(control);
    }
    c = c
        .push(choice(
            p,
            "fixture",
            "Fixture",
            &[
                "Low profile clamps",
                "Vise",
                "Soft jaws",
                "Chuck",
                "Custom fixture",
            ],
            t,
        ))
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
        .push(field(p, "notes", "Setup instructions", t))
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
            c.push(help::field(
                column![
                    muted("Clamp positions"),
                    text_input("-X:0; +X:0", &typed)
                        .on_input(Message::SetClamps)
                        .padding([7, 9])
                        .size(13)
                ]
                .spacing(5),
                "clamps",
                "Clamp positions",
                t,
            ));
            if let Some(error) = error {
                c.push(text(error).size(12).color(WARN).into());
            }
        }
        _ => {}
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
/// round the stock of the preview shown or else the setup's own, from the
/// stock's bottom up (`holding::shown`). None when the stock is hidden, the
/// setup has no workholding or it cannot be modelled (the setup editor says
/// why).
pub(crate) fn holding_boxes(app: &App) -> Vec<crate::holding::HoldBox> {
    let boxes = || {
        let setup = app
            .doc
            .active_setup()
            .filter(|s| app.stock && matches!(get(&s.p, "holding"), "Vise" | "Clamps"))?;
        let stock = app
            .active_simulation()
            .map(|sim| sim.plan.stock)
            .or_else(|| app.doc.setup_stock(app.doc.index(), app.part_mesh()))?;
        let boxes = app.doc.holding_boxes(setup, stock).ok()?;
        Some(crate::holding::shown(&boxes, stock))
    };
    boxes().unwrap_or_default()
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
/// `holding_warning` of the plan of the operation `name`, worked out.
fn holding_note(
    app: &App,
    plan: &crate::simulation::Plan,
    name: &str,
    setup: &Setup,
) -> crate::HoldingNote {
    let boxes = match app.doc.holding_boxes(setup, plan.stock) {
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
    let datum = app
        .part_mesh()
        .and_then(|mesh| crate::post::datum(setup, plan.stock, mesh).ok());
    let lines: Vec<String> = crate::holding::hits(plan, &boxes, gap)
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
/// lists it under the controls.
pub(crate) fn holding_lines(app: &App) -> Vec<String> {
    app.doc.holding_issues(app.doc.index(), app.part_mesh())
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
        primary("+ Set up a machine", Message::NewMachine),
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
            "Your machine library is empty. Use Set up a machine to add your own specification.",
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
            form=form.push(choice(p,"controller","Controller / intended postprocessor",&["LinuxCNC","GRBL","Mach3 / Mach4","Fanuc","Haas","Siemens","Heidenhain","Custom"],t)).push(choice(p,"tool_change","Tool change",&["Manual","Automatic"],t)).push(choice(p,"tcp","Tool center point control",&["No","Yes"],t)).push(note("LinuxCNC has a 3-axis postprocessor. Other controllers are saved for planning; exporting G-code for them is refused."));
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
        primary("Save machine profile", Message::SaveMachine)
    });
    let body = column![
        page_header(
            "Machine setup wizard",
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
fn tools(app: &App) -> Element<'_, Message> {
    if app.doc.tools.is_empty() && app.tool_draft.name().is_empty() {
        return container(
            column![
                page_header("Tool library", "Save cutters you actually use."),
                space().height(40),
                bold("No tools yet", 28),
                muted("Create your first cutter, then select it when adding an operation."),
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
    form = form.push(warnings(tool_issues)).push(
        row![
            primary("Save tool", Message::SaveTool),
            btn(
                "Delete tool",
                Message::DeleteTool(app.tool_draft.id.clone()),
                false
            )
        ]
        .spacing(10),
    );
    container(column![page_header("Tools","Cutter dimensions are stored in millimeters. Tool edits invalidate generated previews."),row![container(scrollable(list)).padding(20).width(320).height(Fill).style(panel_style),container(scrollable(form)).padding(24).width(Fill).height(Fill).style(panel_style)].spacing(20).height(Fill)].spacing(24)).padding(28).height(Fill).into()
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
const VISE_FIELDS: [(&str, &str); 5] = [
    ("jaw_width", "Jaw width · mm"),
    ("jaw_height", "Jaw height · mm"),
    ("jaw_thickness", "Jaw thickness · mm"),
    ("max_opening", "Maximum opening · mm"),
    ("grip", "Grip height · mm"),
];
const CLAMP_FIELDS: [(&str, &str); 4] = [
    ("length", "Length · mm"),
    ("width", "Width · mm"),
    ("height", "Height above the stock · mm"),
    ("bite", "Bite onto the stock · mm"),
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
pub(crate) const SENT_TO_OPENAI: &str = "Sent to OpenAI: the part's dimensions and feature list, your tools, machine limits, the project material (its name, group, hardness, notes and cutting data), stock size, the sizes and positions of setup 1's vise jaws or clamps and their clearance gap (not their names), and on correction rounds the app's check report. The CAD file, file paths, the project title and your setup names are not sent.";
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
    c = c.push(muted(SENT_TO_OPENAI));
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
/// The checks a plan passed before it is shown, as the preview lists them.
pub(crate) fn plan_check_lines() -> [&'static str; 4] {
    [
        "Every operation generates with the stock left by the ones before it",
        "LinuxCNC export checks pass: machine rpm and feed limits, travel, rapids into material, holder clearance",
        "No tool centre line cuts into the part, and nothing goes below the stock",
        "No gouging detected on the stock grid (coarse check)",
    ]
}
/// What the preview's checks leave out, under the check lines. A plan's
/// setups all hold their stock the way setup 1 does, so they model their
/// vise or clamps together or not at all.
pub(crate) fn plan_not_checked(held: Held) -> &'static str {
    match held {
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
/// model them and have nothing wrong with how (`holding_issues`).
pub(crate) fn held(app: &App) -> Held {
    crate::holding::held_of(app.doc.setups.iter().enumerate().map(|(i, setup)| {
        crate::holding::modelled(setup) && app.doc.holding_issues(i, app.part_mesh()).is_empty()
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
    for line in plan_check_lines() {
        checks = checks.push(row![text("✓").size(13).color(GOOD), text(line).size(13)].spacing(8));
    }
    checks = checks.push(muted(plan_not_checked(crate::holding::held(&job.setups))));
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
    /// Under the notes: the pictures the request's model looked at, and the
    /// refusals it met.
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
    if draft.summary.is_none() && draft.setups.is_empty() {
        return vec!["Thinking: the model has not started writing yet.".into()];
    }
    let mut lines = vec![];
    if let Some(summary) = &draft.summary {
        lines.push(format!("Plan: {}", clip(summary, 300)));
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
/// run's rows are one line each, ending in a short summary and the number of
/// pictures looked at. A step that ended without the result it should have
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
                looks: step.looks.clone(),
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
                    "3. The app generates and simulates every operation and runs the export checks"
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
