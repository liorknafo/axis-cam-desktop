use crate::{
    App, Message, Page, Target,
    help::{self, Help, HelpButton},
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
    let mut c = column![
        bold(
            format!(
                "{} configuration issue{}",
                issues.len(),
                if issues.len() == 1 { "" } else { "s" }
            ),
            12
        )
        .color(Color::from_rgb8(166, 101, 32))
    ]
    .spacing(5);
    for issue in issues {
        c = c.push(
            text(format!("• {issue}"))
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
                    .width(185)
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
        note("No sample geometry, setups or operations have been added. Saved machines and tools are available when you need them.")
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
                "Illustrative motion"
            } else {
                "Adaptive stock · no collision checking"
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
                .push(field(p, "rpm", "Spindle speed · rpm (saved)", t));
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
                    "Coolant (saved for postprocessing)",
                    &["Flood", "Mist", "Air", "Off"],
                    t,
                ));
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
                    c = c.push(note(if o.strategy == Strategy::Pocket { "Pocket uses offset contours of the selected floor triangles, including islands. Supports open, vertical-wall pockets with a flat floor; overhanging geometry is rejected. Simulation inherits the stock left by all earlier enabled operations and setups." } else if o.strategy == Strategy::Contour { "Inside contour offsets only the walls and islands of this floor. It does not clear the interior. The floor must be flat and accessible along setup Z; overhangs are rejected." } else if o.strategy == Strategy::Drill { "Hole detection uses tessellated circular openings and vertical rays. Noncircular openings and obstructions are excluded. Confirm the detected centers and depths before previewing." } else { "Selections use connected planar regions of the imported mesh. Facing simulation uses a rectangular raster preview. Exact CAD faces and fixture or machine collision checks are not implemented; the moves can be exported as LinuxCNC G-code." }));
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
            c = c.push(note("Offset contours run from the interior toward the walls at each depth. Entry/full-width cuts can contain both milling modes. No fixture or holder collision solver is connected."));
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
            c = c.push(note("Entry and depth changes feed down at 25% of cutting feed. Keep-down links use cutting feed within the rectangular preview boundary. Lead-in curves, fixture avoidance and collision checking are not implemented."));
        }
        "Linking" if !app.is_sample() && o.strategy.imported_preview() => {
            c=c.push(note("Separate paths retract vertically to clearance, move in XY, then approach at retract height. Entry feed is 25% of cutting feed. No lead arcs, stay-down links, fixture avoidance or holder collision checks are available."));
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
    c = c
        .push(rule::horizontal(1))
        .push(warnings(app.op_issues()))
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
        c = c.push(note("Use stepover up to 50%. Finishing passes repeat the wall/island contours at the requested allowance. Earlier cuts determine the cutting start. A small additional margin (max of 0.02 mm or 1% of cutter radius) covers polygon rounding; exact CAD accuracy and collision verification are not available."));
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
        column![
            muted("Project material"),
            help::wrap(text_input("Material", &app.doc.material)
                .on_input(Message::Material).padding(8).size(13),
                Help::text("Project material", "Material to machine, such as Aluminum 6061. This is recorded in the setup sheet; automatic feed and speed recommendations are not connected yet."), widget::tooltip::Position::Left)
        ]
        .spacing(5),
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
    c = c
        .push(section("Workholding"))
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
fn ai(app: &App) -> Element<'_, Message> {
    let controls=column![bold("Planning brief",18),help::wrap(text_input("Describe the intended machining job",&app.ai_brief).on_input(Message::AiBrief).padding(12).size(13),Help::text("Planning brief", "Describe the result you want. The brief is saved but does not influence the current example planner; OpenAI planning will be connected later."),widget::tooltip::Position::Right),muted("Example coverage"),help::wrap(pick_list(vec!["Top only".to_string(),"Top and bottom".into(),"Top, bottom and side".into()],Some(app.ai_coverage.clone()),Message::AiCoverage).width(Fill).padding(9).text_size(13),crate::choice_help::selected("ai_coverage","Example coverage",&app.ai_coverage,Target::Setup,&Fields::new()),widget::tooltip::Position::Right),muted("Priority"),help::wrap(pick_list(vec!["Balanced".to_string(),"Surface finish".into(),"Fewer tool changes".into()],Some(app.ai_priority.clone()),Message::AiPriority).width(Fill).padding(9).text_size(13),crate::choice_help::selected("ai_priority","Planning priority",&app.ai_priority,Target::Setup,&Fields::new()),widget::tooltip::Position::Right),note("This is a deterministic example planner for the sample housing. Coverage and priority drive the example; the brief is a UI placeholder. No OpenAI API is connected and no files leave this app."),primary("Create example plan",Message::Plan).width(Fill),btn("Undo last applied plan",Message::UndoPlan,false).width(Fill)].spacing(14);
    let mut review = column![
        bold("Review before applying", 21),
        muted("An applied plan replaces the current setups. You can undo it here.")
    ]
    .spacing(14);
    if app.plan.is_empty() {
        review = review.push(
            container(
                column![
                    bold("From part to machining plan", 22),
                    muted("1. Choose setup coverage"),
                    muted("2. Review part placements and operations"),
                    muted("3. Apply the plan and tune each operation"),
                    muted("4. Generate previews and review the NC program")
                ]
                .spacing(18),
            )
            .padding(35)
            .width(Fill)
            .style(panel_style),
        );
    } else {
        for s in &app.plan {
            let mut card = column![
                row![
                    bold(s.name(), 17),
                    space().width(Fill),
                    muted(get(&s.p, "offset"))
                ],
                muted(format!(
                    "Rotation [{}, {}, {}]° · {}",
                    get(&s.p, "rx"),
                    get(&s.p, "ry"),
                    get(&s.p, "rz"),
                    get(&s.p, "reposition")
                ))
            ]
            .spacing(10);
            for o in &s.operations {
                card = card.push(
                    row![icon(app, o.strategy, 22.), text(o.name()).size(13)]
                        .spacing(8)
                        .align_y(Alignment::Center),
                );
            }
            review = review.push(container(card).padding(18).width(Fill).style(panel_style));
        }
        review = review.push(primary("Apply example plan", Message::ApplyPlan));
    }
    container(
        column![
            page_header(
                "AI machining planner",
                "Sign in with ChatGPT. The planner below is still the local example for the sample housing."
            ),
            row![
                container(scrollable(
                    column![account_panel(app), rule::horizontal(1), controls]
                        .spacing(18)
                        // The scrollbar is drawn over the content: keep it off the
                        // right edge of full-width buttons, boxes and lists.
                        .padding(iced::Padding::ZERO.right(14))
                ))
                    .padding(22)
                    .width(370)
                    .height(Fill)
                    .style(panel_style),
                scrollable(review).height(Fill).width(Fill)
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
    let mut summary = column![
        bold("Program review", 20),
        note("LinuxCNC 3-axis output, one .ngc program per setup. Export repeats every check, replays each program against the simulated moves and writes nothing if any setup fails. Fixtures, clamps and the machine body are not checked: preview each program in LinuxCNC and air-cut first."),
    ]
    .spacing(14);
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
    let mut c=column![bold("Operation support & job checks",24),note("Available means an imported-mesh preview is implemented and regression-tested within the scope below. Their moves can be exported as LinuxCNC 3-axis programs after checks for limits, rapids into material and holder clearance. Fixture and machine collisions are not checked. A sample animation does not establish operation support."),row![primary("Check all enabled operations",Message::GenerateAll),btn("Back to machining",Message::Page(Page::Work),false)].spacing(12)].spacing(14).max_width(1020);
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
    c=c.push(note("Remaining 3D stock carries through all enabled operations and setups, including flips. Fully cleared top layers are skipped automatically. Leftover-region rest machining, fixture and machine collision checks, multi-axis kinematics and controllers other than LinuxCNC remain unavailable. Unavailable strategies stay visible for planning, but cannot be added as imported-part machining operations."));
    container(scrollable(c))
        .padding(24)
        .width(Fill)
        .height(Fill)
        .into()
}
