//! Explicit renderer smoke test. It uses an isolated data directory and never
//! sends native input events or changes the user's regular saved workspace.
use crate::{
    App, Message, Page, Target,
    model::Machine,
    planner::{Progress, StepResult},
};
use iced::Task;
use std::fs;
/// Timeline steps for the renderer smoke test: the stage, when it began and
/// ended in seconds before now (None while it runs), and its result.
fn smoke_steps(steps: Vec<(Progress, u64, Option<u64>, Option<StepResult>)>) -> Vec<crate::Step> {
    let now = std::time::Instant::now();
    let ago = |seconds| now.checked_sub(std::time::Duration::from_secs(seconds));
    steps
        .into_iter()
        .map(|(stage, began, ended, result)| crate::Step {
            stage,
            started: ago(began).unwrap_or(now),
            ended: ended.and_then(ago),
            result,
        })
        .collect()
}
/// A failed check's result: two problem lines and `more` others.
fn smoke_problems(more: usize) -> StepResult {
    StepResult::Problems {
        shown: vec![
            "Setup 1, operation 2: Pocket depth must be less than tool stickout. Choose a tool with adequate reach.".into(),
            "Setup 1, operation 3: Contour stepdown must be at most the flute length.".into(),
        ],
        more,
    }
}
impl App {
    pub fn capture_smoke(&mut self, image: iced::window::Screenshot) -> Task<Message> {
        let Some(dir) = self.smoke_dir.clone() else {
            return Task::none();
        };
        fs::create_dir_all(&dir).expect("create smoke output");
        let names = [
            "01-workspace",
            "02-geometry",
            "03-simulation",
            "04-setups",
            "05-machines",
            "06-wizard",
            "07-tools",
            "08-ai",
            "09-nc",
            "10-minimum-window",
            "11-step-import",
            "12-step-flipped",
            "13-3mf-import",
            "14-operation-help",
            "15-field-help",
            "16-new-blank-project",
            "17-start-screen",
            "18-import-before-setup",
            "19-first-machine",
            "20-first-empty-setup",
            "21-last-setup-removed",
            "22-relative-stock",
            "23-rotated-relative-stock",
            "24-fixed-stock",
            "25-invalid-stock-offset",
            "26-stock-offset-help",
        ];
        let name = names
            .get(self.smoke_step)
            .map(|s| s.to_string())
            .unwrap_or_else(|| match self.smoke_step {
                50 => "27-imported-facing-selected".into(),
                51 => "28-imported-facing-finished".into(),
                52 => "29-imported-unsupported-strategy".into(),
                53 => "30-facing-stock-start".into(),
                54 => "31-facing-stock-partial".into(),
                55 => "32-facing-stock-finished".into(),
                56 => "33-facing-stock-rewind".into(),
                57 => "34-facing-linking".into(),
                58 => "35-facing-one-way".into(),
                59 => "36-facing-conventional".into(),
                60 => "37-facing-ccw".into(),
                61 => "38-spindle-rotation".into(),
                62 => "39-milling-direction-help".into(),
                63 => "40-pocket-floor-selected".into(),
                64 => "41-pocket-start".into(),
                65 => "42-pocket-partial".into(),
                66 => "43-pocket-finished".into(),
                67 => "44-pocket-rewind".into(),
                68 => "45-pocket-conventional".into(),
                69 => "46-pocket-ccw".into(),
                70 => "47-pocket-linking".into(),
                71 => "48-pocket-help".into(),
                72 => "49-contour-partial".into(),
                73 => "50-contour-finished".into(),
                74 => "51-drilling-detection".into(),
                75 => "52-drilling-finished".into(),
                76 => "53-parallel-partial".into(),
                77 => "54-parallel-finished".into(),
                78 => "55-waterline-partial".into(),
                79 => "56-waterline-finished".into(),
                80 => "57-operation-audit".into(),
                81 => "58-inside-contour-finished".into(),
                82 => "59-flipped-setup-inherited-stock".into(),
                83 => "60-flipped-setup-partial".into(),
                84 => "61-flipped-setup-finished".into(),
                85 => "62-flipped-setup-restart".into(),
                86 => "63-cumulative-job-report".into(),
                87 => "64-adaptive-pocket-fine".into(),
                88 => "65-adaptive-pocket-orbit".into(),
                89 => "66-adaptive-pocket-draft".into(),
                90 => "67-adaptive-pocket-restart".into(),
                91 => "68-selected-facing-help".into(),
                92 => "69-selected-milling-help".into(),
                93 => "70-selected-entry-help".into(),
                94 => "71-selected-containment-help".into(),
                95 => "72-selected-stock-help".into(),
                96 => "73-selected-cycle-help".into(),
                97 => "74-selected-spindle-help".into(),
                98 => "75-program-blocked".into(),
                99 => "76-program-ready".into(),
                100 => "77-back-to-machining".into(),
                101 => "78-chatgpt-notice".into(),
                102 => "79-chatgpt-signed-in".into(),
                103 => "80-chatgpt-not-granted".into(),
                104 => "81-planner-preview".into(),
                105 => "82-planner-preview-stock".into(),
                106 => "83-planner-failed".into(),
                107 => "84-planner-running".into(),
                108 => "85-planner-ready".into(),
                109 => "86-materials".into(),
                110 => "87-planner-timeline".into(),
                _ => format!("motion-{:02}", self.smoke_step - 26),
            });
        if (26..50).contains(&self.smoke_step) {
            // Inspect the model region, excluding sidebars, labels and controls.
            // A blank async-upload frame contains only the pale background/grid.
            let scale = image.scale_factor;
            let mut model_pixels = 0usize;
            for y in (280. * scale) as u32..image.size.height - (235. * scale) as u32 {
                for x in (240. * scale) as u32..image.size.width - (360. * scale) as u32 {
                    let index = ((y * image.size.width + x) * 4) as usize;
                    let [r, g, b, _] = image.rgba[index..index + 4].try_into().unwrap();
                    if r > 65 && r < 215 && g >= r && b > r.saturating_add(5) {
                        model_pixels += 1;
                    }
                }
            }
            assert!(
                model_pixels > 2000,
                "Missing model in motion frame {}: {model_pixels} pixels",
                self.smoke_step - 26
            );
            eprintln!(
                "Motion frame {}: {model_pixels} visible model pixels",
                self.smoke_step - 26
            );
        }
        let path = dir.join(format!("{name}.png"));
        let mut encoder = png::Encoder::new(
            std::io::BufWriter::new(fs::File::create(&path).unwrap()),
            image.size.width,
            image.size.height,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&image.rgba).unwrap();
        eprintln!("Rendered {}", path.display());
        if std::env::args().any(|arg| arg == "--help-only") {
            return iced::window::latest().and_then(iced::window::close);
        }
        self.smoke_step += 1;
        self.smoke_pending = false;
        let actions = match self.smoke_step {
            1 => vec![Message::SelectOp(1), Message::Tab("Geometry".into())],
            2 => vec![
                Message::Tab("Passes".into()),
                Message::Generate,
                Message::Scrub(0.55),
            ],
            3 => vec![Message::DuplicateSetup, Message::Flip("rx")],
            4 => vec![Message::Page(Page::Machines)],
            5 => {
                self.machine_draft = Machine::new(5);
                vec![Message::Page(Page::Wizard), Message::WizardStep(2)]
            }
            6 => vec![Message::Page(Page::Tools)],
            7 => vec![Message::Page(Page::Ai)],
            8 => vec![Message::Page(Page::Post)],
            9 => {
                self.doc = crate::model::Document::sample();
                self.doc.adopt_materials();
                let _ = self.update(Message::SelectOp(3));
                let _ = self.update(Message::Edit(
                    Target::Operation,
                    "cycle",
                    "Deep drilling".into(),
                ));
                let _ = self.update(Message::Tab("Cycle".into()));
                return iced::window::latest()
                    .and_then(|id| iced::window::resize(id, iced::Size::new(1120., 720.)));
            }
            10 | 12 => {
                let format = if self.smoke_step == 10 { "step" } else { "3mf" };
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("examples/four-setup-block.{format}"));
                let mesh = crate::mesh::load(&path).expect("real CAD fixture import");
                self.doc = crate::model::Document::sample();
                self.doc.adopt_materials();
                let info = crate::model::ModelFile {
                    name: format!("four-setup-block.{format}"),
                    path: path.to_string_lossy().into_owned(),
                    format: format.into(),
                    bytes: std::fs::metadata(path).unwrap().len(),
                };
                let _ = self.update(Message::FileImported(Some(Ok((info, Some(mesh))))));
                self.stock = false;
                vec![Message::SetupEditor]
            }
            11 => vec![Message::Flip("rx")],
            13 | 14 => {
                self.help_animating = true;
                vec![]
            }
            15 => vec![Message::NewProject],
            16 => vec![Message::Page(Page::Home)],
            17 => {
                let mut doc = crate::model::Document::default();
                doc.adopt_materials();
                self.replace_document(doc, None);
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("examples/four-setup-block.3mf");
                let mesh = crate::mesh::load(&path).unwrap();
                vec![Message::FileImported(Some(Ok((
                    crate::model::ModelFile {
                        name: "four-setup-block.3mf".into(),
                        path: path.to_string_lossy().into_owned(),
                        format: "3mf".into(),
                        bytes: std::fs::metadata(path).unwrap().len(),
                    },
                    Some(mesh),
                ))))]
            }
            18 => vec![Message::AddSetup],
            19 => {
                let id = self.machine_draft.id.clone();
                vec![Message::SaveMachine, Message::AssignMachine(id)]
            }
            20 => vec![Message::ConfirmDeleteSetup(true)],
            21 => {
                self.stock = true;
                vec![
                    Message::AssignMachine(self.doc.machines[0].id.clone()),
                    Message::Edit(Target::Setup, "stock_top", "10".into()),
                    Message::Edit(Target::Setup, "stock_right", "2".into()),
                ]
            }
            22 => vec![Message::Flip("ry")],
            23 => vec![
                Message::Edit(Target::Setup, "stock_type", "Fixed size box".into()),
                Message::Edit(Target::Setup, "stock_x", "60".into()),
                Message::Edit(Target::Setup, "stock_y", "100".into()),
                Message::Edit(Target::Setup, "stock_z", "140".into()),
            ],
            24 => vec![
                Message::Edit(Target::Setup, "stock_type", "Relative size box".into()),
                Message::Edit(Target::Setup, "stock_top", "-1".into()),
            ],
            25 => {
                self.help_animating = true;
                vec![]
            }
            26 => {
                self.doc
                    .setup_mut()
                    .p
                    .insert("stock_top".into(), "10".into());
                self.doc.setup_mut().p.insert("ry".into(), "0".into());
                self.camera = crate::viewport::Camera::default();
                self.page = Page::Work;
                self.stock = true;
                // Exercise frequently replaced images above the former 2 MiB
                // upload boundary, at the same update cadence as animated help.
                self.help_animating = true;
                return iced::window::latest()
                    .and_then(|id| iced::window::resize(id, iced::Size::new(1440., 930.)));
            }
            27..=49 => {
                let i = (self.smoke_step - 26) as f32;
                self.camera.yaw = 0.75 + i * 0.23;
                self.camera.pitch = 0.65 + 0.25 * (i * 0.35).sin();
                self.camera.pan = [(i * 0.4).sin() * 20., (i * 0.3).cos() * 12.];
                self.camera.zoom = 0.95 + 0.15 * (i * 0.2).sin();
                self.stock = self.smoke_step < 38 || self.smoke_step > 43;
                vec![]
            }
            50 => {
                self.camera = crate::viewport::Camera::default();
                self.help_animating = false;
                self.stock = true;
                let mesh = self.imported_mesh.as_ref().unwrap();
                let surface = mesh
                    .surfaces
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.normal[2].abs() > 0.999999)
                    .max_by(|(_, a), (_, b)| a.center[2].total_cmp(&b.center[2]))
                    .unwrap()
                    .0;
                vec![
                    Message::NewTool,
                    Message::SaveTool,
                    Message::AddOp(crate::model::Strategy::Face),
                    Message::Tab("Geometry".into()),
                    Message::Surface(surface),
                ]
            }
            51 => vec![Message::PickMode, Message::Generate],
            52 => vec![
                Message::AddOp(crate::model::Strategy::Adaptive),
                Message::Tab("Geometry".into()),
            ],
            53 => {
                self.doc.setup_mut().operations[0]
                    .p
                    .insert("boundary".into(), "Stock silhouette".into());
                self.doc.setup_mut().operations[0]
                    .p
                    .insert("containment".into(), "Tool center on boundary".into());
                vec![
                    Message::SelectOp(0),
                    Message::Tab("Passes".into()),
                    Message::Generate,
                    Message::Scrub(0.),
                ]
            }
            54 => {
                assert!(self.active_simulation().is_some(), "{}", self.status);
                vec![Message::Scrub(0.45)]
            }
            55 => vec![Message::Scrub(1.)],
            56 => vec![Message::Scrub(0.2)],
            57 => vec![Message::Tab("Linking".into())],
            58 => vec![
                Message::Edit(Target::Operation, "facing_pattern", "One way".into()),
                Message::Tab("Passes".into()),
                Message::Generate,
                Message::Scrub(0.2),
            ],
            59 => vec![
                Message::Edit(Target::Operation, "facing_milling", "Conventional".into()),
                Message::Generate,
                Message::Scrub(0.2),
            ],
            60 => vec![
                Message::Edit(
                    Target::Operation,
                    "spindle_rotation",
                    "Counterclockwise (CCW)".into(),
                ),
                Message::Generate,
                Message::Scrub(0.2),
            ],
            61 => vec![Message::Tab("Tool".into())],
            62 => {
                self.help_animating = true;
                vec![]
            }
            63 => {
                self.help_animating = false;
                let mesh = self.imported_mesh.as_ref().unwrap();
                let placement = crate::stock::Placement::new(mesh, self.doc.active_setup());
                let floor = mesh
                    .surfaces
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| {
                        s.normal[2].abs() > 0.999999
                            && placement.transform(s.center)[2] < placement.bounds.max[2] - 0.1
                            && placement.transform(s.center)[2] > placement.bounds.min[2] + 0.1
                    })
                    .max_by(|(_, a), (_, b)| a.area.total_cmp(&b.area))
                    .unwrap()
                    .0;
                vec![
                    Message::AddOp(crate::model::Strategy::Pocket),
                    Message::Tab("Geometry".into()),
                    Message::Surface(floor),
                ]
            }
            64 => vec![
                Message::PickMode,
                Message::Tab("Passes".into()),
                Message::Generate,
                Message::Scrub(0.),
            ],
            65 => {
                assert!(self.active_simulation().is_some(), "{}", self.status);
                vec![Message::Scrub(0.45)]
            }
            66 => vec![Message::Scrub(1.)],
            67 => vec![Message::Scrub(0.25)],
            68 => vec![
                Message::Edit(Target::Operation, "pocket_milling", "Conventional".into()),
                Message::Generate,
                Message::Scrub(0.35),
            ],
            69 => vec![
                Message::Edit(
                    Target::Operation,
                    "spindle_rotation",
                    "Counterclockwise (CCW)".into(),
                ),
                Message::Generate,
                Message::Scrub(0.35),
                Message::Tab("Tool".into()),
            ],
            70 => vec![Message::Tab("Linking".into())],
            71 => {
                self.help_animating = true;
                vec![]
            }
            72 => {
                self.help_animating = false;
                for tool in &mut self.doc.tools {
                    tool.p.insert("length".into(), "60".into());
                    tool.p.insert("stickout".into(), "65".into());
                }
                self.doc.tools.push(crate::model::Tool::new(
                    "Audit ball Ø6",
                    "Ball end mill",
                    "6",
                ));
                self.doc
                    .tools
                    .push(crate::model::Tool::new("Audit drill Ø4", "Drill", "4"));
                for tool in &mut self.doc.tools {
                    tool.p.insert("length".into(), "60".into());
                    tool.p.insert("stickout".into(), "65".into());
                }
                vec![
                    Message::AddOp(crate::model::Strategy::Contour),
                    Message::Tab("Passes".into()),
                    Message::Generate,
                    Message::Scrub(0.5),
                ]
            }
            73 => {
                assert!(self.active_simulation().is_some(), "{}", self.status);
                vec![Message::Scrub(1.)]
            }
            74 => {
                let mesh = self.imported_mesh.as_ref().unwrap();
                let top = mesh
                    .surfaces
                    .iter()
                    .filter(|s| s.normal[2].abs() > 0.999999)
                    .map(|s| s.center[2])
                    .fold(f32::NEG_INFINITY, f32::max);
                let ids: Vec<_> = mesh
                    .surfaces
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| {
                        s.normal[2].abs() > 0.999999 && (s.center[2] - top).abs() < 0.001
                    })
                    .map(|(id, _)| id)
                    .collect();
                let mut actions = vec![
                    Message::AddOp(crate::model::Strategy::Drill),
                    Message::Tab("Geometry".into()),
                ];
                actions.extend(ids.into_iter().map(Message::Surface));
                actions
            }
            75 => vec![
                Message::Tab("Cycle".into()),
                Message::Edit(Target::Operation, "cycle", "Deep drilling".into()),
                Message::Generate,
                Message::Scrub(1.),
            ],
            76 => {
                assert!(self.active_simulation().is_some(), "{}", self.status);
                vec![
                    Message::AddOp(crate::model::Strategy::Parallel),
                    Message::Edit(Target::Operation, "surface_stepover", "2".into()),
                    Message::Edit(Target::Operation, "tolerance", "0.1".into()),
                    Message::Tab("Passes".into()),
                    Message::Generate,
                    Message::Scrub(0.5),
                ]
            }
            77 => {
                assert!(self.active_simulation().is_some(), "{}", self.status);
                vec![Message::Scrub(1.)]
            }
            78 => vec![
                Message::AddOp(crate::model::Strategy::Waterline),
                Message::Tab("Passes".into()),
                Message::Generate,
                Message::Scrub(0.5),
            ],
            79 => {
                assert!(self.active_simulation().is_some(), "{}", self.status);
                vec![Message::Scrub(1.)]
            }
            80 => vec![Message::GenerateAll],
            81 => {
                let selection = self
                    .doc
                    .setup()
                    .operations
                    .iter()
                    .find(|o| o.strategy == crate::model::Strategy::Pocket)
                    .unwrap()
                    .surfaces
                    .clone()
                    .unwrap();
                let mut actions = vec![
                    Message::AddOp(crate::model::Strategy::Contour),
                    Message::Edit(Target::Operation, "boundary", "Selected geometry".into()),
                    Message::Edit(Target::Operation, "side", "Inside".into()),
                ];
                actions.extend(selection.ids.into_iter().map(Message::Surface));
                actions.extend([
                    Message::Tab("Passes".into()),
                    Message::Generate,
                    Message::Scrub(1.),
                ]);
                actions
            }
            82 => {
                use crate::model::{Setup, Strategy};
                self.doc.setups[0]
                    .p
                    .insert("stock_bottom".into(), "3".into());
                let mut setup = Setup::new(self.doc.setups[0].machine.clone(), "G55");
                setup.p.insert("name".into(), "Flipped underside".into());
                setup.p.insert("rx".into(), "180".into());
                setup.p.insert("tx".into(), "12".into());
                let mesh = self.imported_mesh.as_ref().unwrap();
                let place = crate::stock::Placement::new(mesh, Some(&setup));
                let id = mesh
                    .surfaces
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| {
                        (place.transform(s.center)[2] - place.bounds.max[2]).abs() < 0.001
                    })
                    .max_by(|(_, a), (_, b)| a.area.total_cmp(&b.area))
                    .unwrap()
                    .0;
                let mut face =
                    crate::model::Operation::new(Strategy::Face, self.doc.tools[0].id.clone());
                face.geometry.clear();
                face.surfaces = Some(crate::surface::Selection {
                    mesh_fingerprint: mesh.fingerprint.clone(),
                    ids: vec![id],
                });
                face.p.insert("boundary".into(), "Stock silhouette".into());
                face.p
                    .insert("containment".into(), "Tool outside boundary".into());
                setup.operations.push(face);
                self.doc.setups.push(setup);
                self.doc.sync_stock_sources();
                vec![
                    Message::SelectSetup(1),
                    Message::SelectOp(0),
                    Message::Tab("Passes".into()),
                    Message::Generate,
                    Message::Scrub(0.),
                ]
            }
            83 => {
                assert!(self.active_simulation().is_some(), "{}", self.status);
                assert_eq!(self.active_simulation().unwrap().preceding, 7);
                vec![Message::Scrub(0.5)]
            }
            84 => vec![Message::Scrub(1.)],
            85 => vec![Message::Scrub(0.)],
            86 => {
                let sim = self.active_simulation().unwrap();
                assert_eq!(
                    sim.volume.as_ref().unwrap().occupied,
                    sim.initial_volume.as_ref().unwrap().occupied
                );
                vec![Message::GenerateAll]
            }
            87 => {
                self.paths = false;
                self.camera.zoom = 2.2;
                vec![
                    Message::SelectSetup(0),
                    Message::SelectOp(1),
                    Message::Generate,
                    Message::Scrub(1.),
                    Message::StockDetail(crate::stock_surface::Detail::Fine),
                ]
            }
            88 => {
                self.paths = false;
                self.camera.yaw += 0.8;
                self.camera.pitch = 0.85;
                vec![]
            }
            89 => vec![Message::StockDetail(crate::stock_surface::Detail::Draft)],
            90 => vec![Message::Scrub(0.)],
            91..=97 => vec![],
            98 => {
                self.help_animating = false;
                self.invalidate_post();
                vec![Message::Page(Page::Post)]
            }
            99 => {
                use crate::model::Strategy;
                self.doc.number_tools();
                self.doc.setups.truncate(1);
                self.doc.setups[0].machine = Machine::new(3);
                self.doc.setups[0]
                    .operations
                    .retain(|o| matches!(o.strategy, Strategy::Face | Strategy::Pocket));
                self.doc.active = self.doc.setups[0].id.clone();
                self.doc.sync_stock_sources();
                self.invalidate_post();
                vec![Message::Page(Page::Post)]
            }
            100 => {
                let job = self.post.as_ref().expect("smoke post result");
                assert!(
                    job.ready(),
                    "{:?}",
                    job.setups
                        .iter()
                        .filter_map(|s| s.result.as_ref().err())
                        .collect::<Vec<_>>()
                );
                vec![
                    Message::Page(Page::Work),
                    Message::SelectOp(0),
                    Message::Generate,
                ]
            }
            101 => {
                // A signed-in account has a client, so every planner screen's
                // checklist agrees with the account panel. This one never
                // sends a request.
                let placeholder = crate::session::Session {
                    access_token: "smoke".into(),
                    refresh_token: "smoke".into(),
                    id_token: "smoke".into(),
                    expires_at: crate::session::now() + 3600,
                    scope: crate::session::PLAN_SCOPE.into(),
                    subject: "smoke".into(),
                    email: "lior@example.com".into(),
                    name: "Lior".into(),
                };
                self.client = Some(std::sync::Arc::new(crate::openai::Client::new(
                    self.endpoints.clone(),
                    String::new(),
                    self.store.clone(),
                    placeholder,
                )));
                self.account = crate::Account::SignedIn {
                    name: "Lior".into(),
                    email: "lior@example.com".into(),
                    granted: true,
                };
                self.models = vec!["gpt-6.1-sol".into(), "gpt-6-luna".into()];
                self.settings.model = "gpt-6.1-sol".into();
                self.settings.plan_notice_shown = false;
                vec![Message::Page(Page::Ai)]
            }
            102 => vec![Message::PlanNoticeDismissed],
            103 => {
                self.account = crate::Account::SignedIn {
                    name: "Lior".into(),
                    email: "lior@example.com".into(),
                    granted: false,
                };
                self.account_note = crate::NOT_GRANTED.into();
                // The once-only notice must stay hidden without plan usage.
                self.settings.plan_notice_shown = false;
                vec![]
            }
            104 => {
                // A checked plan for the imported block, as the planner shows it.
                self.account = crate::Account::SignedIn {
                    name: "Lior".into(),
                    email: "lior@example.com".into(),
                    granted: true,
                };
                self.account_note.clear();
                self.settings.plan_notice_shown = true;
                // Planning needs a library material with cutting data.
                self.doc.adopt_materials();
                let starter = self
                    .doc
                    .materials
                    .iter()
                    .find(|m| m.name() == "Aluminium 6061-T6")
                    .expect("starter material")
                    .id
                    .clone();
                assert!(self.doc.select_material(&starter));
                self.planner = crate::Planner::Preview(self.smoke_plan_preview());
                self.page = Page::Ai;
                vec![]
            }
            105 => {
                self.smoke_pending = false;
                return iced::widget::operation::scroll_to(
                    "planner-result",
                    iced::widget::scrollable::AbsoluteOffset { x: 0., y: 5000. },
                );
            }
            106 => {
                self.planner = crate::Planner::Failed(crate::planner::Failure {
                    message: "The planner could not produce a plan that passes every check in 4 attempts.".into(),
                    problems: vec![crate::proposal::Problem {
                        setup: 1,
                        operation: Some(2),
                        stage: "generation",
                        message: "Pocket depth must be less than tool stickout, with stickout at least the flute length. Choose a tool with adequate reach.".into(),
                    }],
                    api: None,
                    requests: 4,
                });
                vec![]
            }
            107 => {
                // A run started with the picked model, asking for a correction.
                use Progress::*;
                self.plan_model = self.settings.model.clone();
                self.planner = crate::Planner::Running(Repairing(2));
                self.plan_timeline = smoke_steps(vec![
                    (Analyzing, 68, Some(66), None),
                    (Asking(1), 66, Some(29), None),
                    (Checking(1), 29, Some(22), Some(smoke_problems(2))),
                    (Repairing(2), 22, None, None),
                ]);
                self.plan_received
                    .store(1840, std::sync::atomic::Ordering::Relaxed);
                vec![]
            }
            108 => {
                self.planner = crate::Planner::Idle;
                self.plan_timeline.clear();
                let unmet: Vec<_> = self
                    .plan_checklist()
                    .into_iter()
                    .filter(|line| !line.ok)
                    .map(|line| line.label)
                    .collect();
                assert!(unmet.is_empty(), "the ready screen's checklist: {unmet:?}");
                vec![]
            }
            109 => vec![Message::Page(Page::Materials)],
            110 => {
                // The finished run above its preview.
                use Progress::*;
                self.plan_model = self.settings.model.clone();
                self.planner = crate::Planner::Preview(self.smoke_plan_preview());
                self.plan_timeline = smoke_steps(vec![
                    (Analyzing, 91, Some(89), None),
                    (Asking(1), 89, Some(52), None),
                    (Checking(1), 52, Some(43), Some(smoke_problems(0))),
                    (Repairing(2), 43, Some(19), None),
                    (Checking(2), 19, Some(7), Some(StepResult::Passed)),
                    (Previewing, 7, Some(4), None),
                ]);
                self.page = Page::Ai;
                vec![]
            }
            _ => {
                assert!(self.active_simulation().is_some(), "{}", self.status);
                assert!(self.doc.validate_structure().is_ok());
                self.persist();
                return iced::window::latest().and_then(iced::window::close);
            }
        };
        for action in actions {
            let _ = self.update(action);
        }
        if (87..=90).contains(&self.smoke_step) {
            self.paths = false;
        }
        if self.smoke_step >= 50 {
            return iced::widget::operation::scroll_to(
                "inspector",
                iced::widget::scrollable::AbsoluteOffset { x: 0., y: 0. },
            );
        }
        if (21..=24).contains(&self.smoke_step) {
            return iced::widget::operation::scroll_to(
                "inspector",
                iced::widget::scrollable::AbsoluteOffset { x: 0., y: 390. },
            );
        }
        Task::none()
    }
    /// A checked plan for the imported block, as the planner shows it.
    fn smoke_plan_preview(&self) -> std::sync::Arc<crate::PlanPreview> {
        use crate::proposal::{Proposal, ProposedOp, ProposedSetup, Unmachined};
        let mesh = self.imported_mesh.clone().expect("imported part");
        let report = crate::analysis::report(&self.doc, &mesh).expect("part report");
        let first = &report.orientations[0];
        let plane = |depth: f32| {
            let found = first
                .planes
                .iter()
                .find(|p| (p.depth_below_top - depth).abs() < 0.01);
            found.expect("plane").surfaces.clone()
        };
        let flat = report
            .tools
            .iter()
            .find(|t| t.kind == "Flat end mill")
            .expect("flat tool");
        let op = |strategy: &str, surfaces: Vec<usize>| ProposedOp {
            strategy: strategy.into(),
            name: strategy.into(),
            tool: flat.number,
            surfaces,
            rpm: 8000.,
            feed: 800.,
            coolant: "Flood".into(),
            reason: "smoke".into(),
            ..Default::default()
        };
        let proposal = Proposal {
            summary: "Face the top, clear the main pocket and cut the outline from the top. The holes and the side features need tools and setups this plan does not have.".into(),
            setups: vec![ProposedSetup {
                orientation: first.up.into(),
                name: "Top".into(),
                origin: "Stock top center".into(),
                workholding: "Vise on the long sides.".into(),
                operations: vec![
                    op("Face", plane(0.)),
                    op("Pocket", plane(12.)),
                    op("Contour", vec![]),
                ],
            }],
            unmachined: vec![Unmachined {
                feature: "Four counterbored through holes".into(),
                reason: "The library has no drill small enough for the 6.6 mm holes.".into(),
            }],
            warnings: vec!["The outline is cut to full depth; hold the part below the cut.".into()],
        };
        let (check, job) = crate::plan_check::check(&self.doc, &mesh, &report, &proposal);
        assert!(check.passed, "smoke plan: {:?}", check.problems);
        let job = job.expect("built job");
        let view = crate::planner::final_view(&job, &mesh);
        assert!(view.is_some(), "smoke plan view");
        std::sync::Arc::new(crate::PlanPreview {
            outcome: crate::planner::Outcome {
                proposal,
                check,
                job,
                report,
                requests: 2,
                chosen: 2,
                more_warnings: 0,
                more_unmachined: 0,
                note: None,
            },
            view,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    #[test]
    fn first_launch_and_empty_project_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.json");
        let mut a = App::from_workspace(path.clone(), None);
        assert_eq!(a.page, Page::Home);
        assert!(!a.is_sample());
        assert!(
            a.doc.file.is_none()
                && a.doc.setups.is_empty()
                && a.doc.tools.is_empty()
                && a.doc.machines.is_empty()
        );
        assert!(a.doc.validate_structure().is_ok());
        for page in [
            Page::Home,
            Page::Work,
            Page::Machines,
            Page::Tools,
            Page::Materials,
            Page::Ai,
            Page::Post,
            Page::Wizard,
        ] {
            a.page = page;
            let _view = a.view();
        }
        a.dirty = true;
        a.persist();
        let reopened = App::from_workspace(path, None);
        assert_eq!(reopened.page, Page::Home);
        assert!(!reopened.is_sample() && reopened.doc.setups.is_empty());
    }
    #[test]
    fn new_project_archives_unsaved_work_and_preserves_libraries() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        a.replace_document(Document::sample(), None);
        a.doc.title = "Unsaved custom job".into();
        let machine = a.doc.machines[0].id.clone();
        let tool = a.doc.tools[0].id.clone();
        let _ = a.update(Message::NewProject);
        assert!(!a.is_sample() && !a.doc.has_model() && a.doc.setups.is_empty());
        assert!(a.selected.is_none() && a.undo.is_none());
        assert!(matches!(a.planner, crate::Planner::Idle));
        assert_eq!(a.doc.machines[0].id, machine);
        assert_eq!(a.doc.tools[0].id, tool);
        let archive = crate::storage::load(a.last_archived.as_ref().unwrap()).unwrap();
        assert_eq!(archive.title, "Unsaved custom job");
        assert_eq!(archive.setups[0].operations.len(), 4);
        let _ = a.update(Message::RecoverPrevious);
        assert_eq!(a.doc.title, "Unsaved custom job");
        assert!(a.is_sample());
    }
    fn tool_numbers(doc: &Document) -> Vec<(String, u32)> {
        doc.tools
            .iter()
            .map(|t| (t.name().to_string(), t.number().unwrap_or(0)))
            .collect()
    }
    #[test]
    fn explore_example_merges_a_legacy_library_without_duplicates() {
        // A workspace saved before tool numbers and post settings: the user's
        // own tool, then the example's tools from an earlier Explore example,
        // and the example's machines without the post keys.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.json");
        let sample = Document::sample();
        let mut legacy = Document::default();
        legacy.tools.push(Tool::new("My spot drill", "Drill", "6"));
        legacy.tools.extend(sample.tools.iter().cloned());
        for t in &mut legacy.tools {
            t.p.remove("number");
        }
        legacy.machines = sample.machines.clone();
        for m in &mut legacy.machines {
            for key in POST_KEYS {
                m.p.remove(key);
            }
        }
        crate::storage::save(&path, &legacy).unwrap();
        let mut a = App::from_workspace(path, None);
        assert_eq!(
            a.doc.tools.iter().map(|t| t.number()).collect::<Vec<_>>(),
            [Some(1), Some(2), Some(3), Some(4)]
        );
        let _ = a.update(Message::OpenSample);
        assert!(a.is_sample(), "{}", a.status);
        // The example's tools take the numbers the user's copies had, and
        // the user's own tool is added once.
        let mut numbers = tool_numbers(&a.doc);
        numbers.sort_by_key(|t| t.1);
        assert_eq!(
            numbers,
            [
                ("My spot drill".to_string(), 1),
                ("Flat end mill · Ø10".to_string(), 2),
                ("Ball end mill · Ø6".to_string(), 3),
                ("Twist drill · Ø8".to_string(), 4),
            ]
        );
        assert!(
            a.doc
                .tools
                .iter()
                .all(|t| a.doc.tool_number_issues(t).is_empty())
        );
        assert_eq!(a.doc.machines.len(), 3);
        assert!(a.doc.validate_structure().is_ok());
        // The example's operations still reference its tools.
        assert!(
            a.doc.setups[0]
                .operations
                .iter()
                .all(|o| a.doc.tools.iter().any(|t| t.id == o.tool))
        );
        assert!(a.status.contains("My spot drill"), "{}", a.status);
        assert!(!a.status.contains("Flat end mill"), "{}", a.status);
        // New project keeps the merged library as it is.
        let _ = a.update(Message::NewProject);
        assert_eq!(a.doc.tools.len(), 4);
        assert_eq!(a.doc.machines.len(), 3);
    }
    #[test]
    fn explore_example_gives_its_tools_numbers_the_user_library_does_not_use() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        for (n, name) in ["My flat 6", "My ball 4", "My drill 5"].iter().enumerate() {
            let mut tool = Tool::new(name, "Flat end mill", "6");
            tool.p.insert("number".into(), (n + 1).to_string());
            a.doc.tools.push(tool);
        }
        let mut machine = Machine::new(3);
        machine.p.insert("name".into(), "Shop router".into());
        a.doc.machines.push(machine);
        let _ = a.update(Message::OpenSample);
        assert_eq!(
            tool_numbers(&a.doc),
            [
                ("Flat end mill · Ø10".to_string(), 4),
                ("Ball end mill · Ø6".to_string(), 5),
                ("Twist drill · Ø8".to_string(), 6),
                ("My flat 6".to_string(), 1),
                ("My ball 4".to_string(), 2),
                ("My drill 5".to_string(), 3),
            ]
        );
        assert!(
            a.doc
                .tools
                .iter()
                .all(|t| a.doc.tool_number_issues(t).is_empty())
        );
        assert_eq!(a.doc.machines.len(), 4);
        for name in ["My flat 6", "My ball 4", "My drill 5", "Shop router"] {
            assert!(a.status.contains(name), "{}", a.status);
        }
        // Opening it again adds nothing more.
        let _ = a.update(Message::OpenSample);
        assert_eq!(a.doc.tools.len(), 6);
        assert_eq!(a.doc.machines.len(), 4);
        assert!(
            a.doc
                .tools
                .iter()
                .all(|t| a.doc.tool_number_issues(t).is_empty())
        );
    }
    #[test]
    fn tools_numbered_on_load_are_announced() {
        let note = "Tools in this project had no tool numbers and were numbered T1–Tn in library order. Check each one against your LinuxCNC tool table before exporting.";
        assert_eq!(crate::storage::NUMBERED_TOOLS, note);
        let dir = tempfile::tempdir().unwrap();
        let mut legacy = Document::sample();
        for t in &mut legacy.tools {
            t.p.remove("number");
        }
        // Workspace start.
        let path = dir.path().join("workspace.json");
        crate::storage::save(&path, &legacy).unwrap();
        let mut a = App::from_workspace(path.clone(), None);
        assert!(a.status.contains(note), "{}", a.status);
        // Open, with the notes from the load.
        let project = dir.path().join("old.axis.json");
        crate::storage::save(&project, &legacy).unwrap();
        let (doc, notes) = crate::storage::load_reporting(&project).unwrap();
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert_eq!(notes[0], note);
        let all = notes.join(" ");
        let _ = a.update(Message::ProjectOpened(Some(Ok((
            doc,
            project.clone(),
            notes,
        )))));
        assert!(
            a.status.starts_with("Project opened.") && a.status.ends_with(&all),
            "{}",
            a.status
        );
        // A project that already had numbers says nothing about them.
        let numbered_doc = crate::storage::load(&project).unwrap();
        let _ = a.update(Message::ProjectOpened(Some(Ok((
            numbered_doc,
            project,
            vec![],
        )))));
        assert!(!a.status.contains("LinuxCNC tool table"), "{}", a.status);
        // Recover previous loads the archived project the same way.
        let archived = a.last_archived.clone().unwrap();
        crate::storage::save(&archived, &legacy).unwrap();
        let _ = a.update(Message::RecoverPrevious);
        assert!(a.status.contains(note), "{}", a.status);
        // A workspace with numbers starts without the note.
        let fresh = tempfile::tempdir().unwrap();
        let path = fresh.path().join("workspace.json");
        crate::storage::save(&path, &Document::sample()).unwrap();
        let a = App::from_workspace(path, None);
        assert!(!a.status.contains("LinuxCNC tool table"), "{}", a.status);
    }
    #[test]
    fn loading_a_project_with_a_legacy_material_links_it_and_says_so() {
        let linked = "The project material \"Brass C360\" was linked to the library.";
        let dir = tempfile::tempdir().unwrap();
        let legacy = Document {
            material: "Brass C360".into(),
            ..Document::default()
        };
        let project = dir.path().join("brass.axis.json");
        crate::storage::save(&project, &legacy).unwrap();
        let (doc, notes) = crate::storage::load_reporting(&project).unwrap();
        assert_eq!(notes, [linked]);
        assert_eq!(doc.materials.len(), 9);
        assert_eq!(doc.selected_material().unwrap().name(), "Brass C360");
        // Open: the status carries the note.
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        let _ = a.update(Message::ProjectOpened(Some(Ok((
            doc,
            project.clone(),
            notes,
        )))));
        assert!(
            a.status.starts_with("Project opened.") && a.status.contains(linked),
            "{}",
            a.status
        );
        // Saved with its link, it loads without the note.
        crate::storage::save(&project, &a.doc).unwrap();
        assert!(
            crate::storage::load_reporting(&project)
                .unwrap()
                .1
                .is_empty()
        );
        // Workspace start and Recover previous say it too.
        let path = dir.path().join("start").join("workspace.json");
        crate::storage::save(&path, &legacy).unwrap();
        let mut a = App::from_workspace(path, None);
        assert!(a.status.contains(linked), "{}", a.status);
        let _ = a.update(Message::NewProject);
        let archived = a.last_archived.clone().unwrap();
        crate::storage::save(&archived, &legacy).unwrap();
        let _ = a.update(Message::RecoverPrevious);
        assert!(a.status.contains(linked), "{}", a.status);
        assert_eq!(a.doc.selected_material().unwrap().name(), "Brass C360");
    }
    #[test]
    fn new_project_and_explore_example_carry_materials_without_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        // A blank workspace starts with the starter materials.
        assert_eq!(a.doc.materials.len(), 9);
        let user = crate::material::Material::new("Titanium Gr5");
        assert!(user.issues().is_empty());
        let titanium = user.id.clone();
        a.doc.materials.push(user);
        let _ = a.update(Message::NewProject);
        assert_eq!(a.doc.materials.len(), 10);
        assert!(a.doc.material_id.is_empty() && a.doc.material.is_empty());
        // The user's own copy of a starter, renamed in another case and
        // edited, is kept; no starter is added next to it.
        a.doc.materials[0]
            .p
            .insert("name".into(), "aluminium 6061-t6".into());
        a.doc.materials[0]
            .p
            .insert("hardness".into(), "mine".into());
        let mine = a.doc.materials[0].id.clone();
        let _ = a.update(Message::OpenSample);
        assert!(a.is_sample(), "{}", a.status);
        let names: Vec<_> = a
            .doc
            .materials
            .iter()
            .map(|m| m.name().to_lowercase())
            .collect();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(names.len(), 10);
        assert_eq!(unique.len(), 10, "{names:?}");
        assert!(a.doc.materials.iter().any(|m| m.id == titanium));
        assert_eq!(a.doc.material_id, mine);
        assert_eq!(
            a.doc.selected_material().unwrap().name(),
            "aluminium 6061-t6"
        );
        assert_eq!(a.doc.material, "aluminium 6061-t6");
        assert!(a.status.contains("Titanium Gr5"), "{}", a.status);
        assert!(!a.status.contains("Brass C360"), "{}", a.status);
        assert!(
            a.status
                .ends_with("The project material \"aluminium 6061-t6\" was linked to the library."),
            "{}",
            a.status
        );
        // Opening it again, and a new project, add nothing more.
        let _ = a.update(Message::OpenSample);
        assert_eq!(a.doc.materials.len(), 10);
        let _ = a.update(Message::NewProject);
        assert_eq!(a.doc.materials.len(), 10);
        assert!(a.doc.material_id.is_empty());
        // An example opened from a library without the user's edits links
        // its material to the starter.
        let mut fresh = App::from_workspace(dir.path().join("other").join("workspace.json"), None);
        let _ = fresh.update(Message::OpenSample);
        assert_eq!(fresh.doc.materials.len(), 9);
        assert_eq!(
            fresh.doc.selected_material().unwrap().name(),
            "Aluminium 6061-T6"
        );
        assert!(
            fresh
                .status
                .ends_with("The project material \"Aluminium 6061-T6\" was linked to the library."),
            "{}",
            fresh.status
        );
    }
    #[test]
    fn explore_example_adds_its_material_without_data_when_the_user_deleted_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        a.doc.materials.retain(|m| m.name() != "Aluminium 6061-T6");
        assert_eq!(a.doc.materials.len(), 8);
        let _ = a.update(Message::OpenSample);
        // The deleted starter is not brought back with its data: the example's
        // material is added to the library without cutting data, and says so.
        assert_eq!(a.doc.materials.len(), 9);
        let selected = a.doc.selected_material().unwrap();
        assert_eq!(selected.name(), "Aluminium 6061-T6");
        assert!(selected.cutting_data().is_none());
        assert!(
            a.status.ends_with("\"Aluminium 6061-T6\" was added to the material library without cutting data; add it on the Materials page."),
            "{}",
            a.status
        );
        // New project and a second Explore example keep that entry once.
        let _ = a.update(Message::NewProject);
        let _ = a.update(Message::OpenSample);
        assert_eq!(a.doc.materials.len(), 9);
        assert!(a.status.contains("linked to the library"), "{}", a.status);
    }
    #[test]
    fn the_first_save_on_an_empty_library_gets_a_tool_number() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        // Tool library from the start screen, without New tool.
        let _ = a.update(Message::Page(Page::Tools));
        let _ = a.update(Message::Edit(Target::Tool, "name", "Flat 6".into()));
        let _ = a.update(Message::SaveTool);
        assert_eq!(a.doc.tools.len(), 1, "{}", a.status);
        assert_eq!(a.doc.tools[0].number(), Some(1));
        // After the last tool is deleted, and after New project on an empty library.
        let id = a.doc.tools[0].id.clone();
        let _ = a.update(Message::DeleteTool(id));
        assert_eq!(a.tool_draft.number(), Some(1));
        a.doc.tools.clear();
        a.replace_document(Document::default(), None);
        let _ = a.update(Message::Edit(Target::Tool, "name", "Drill 3".into()));
        let _ = a.update(Message::SaveTool);
        assert_eq!(a.doc.tools.len(), 1, "{}", a.status);
        assert_eq!(a.doc.tools[0].number(), Some(1));
    }
    #[test]
    fn materials_are_created_edited_saved_and_deleted_like_tools() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        let before = a.doc.materials.len();
        let _ = a.update(Message::NewMaterial);
        assert_eq!(a.page, Page::Materials);
        let _ = a.update(Message::Edit(
            Target::Material,
            "name",
            "Titanium Gr5".into(),
        ));
        let _ = a.update(Message::SaveMaterial);
        assert_eq!(a.doc.materials.len(), before + 1);
        // A duplicate name is refused.
        let _ = a.update(Message::NewMaterial);
        let _ = a.update(Message::Edit(
            Target::Material,
            "name",
            "titanium gr5".into(),
        ));
        let _ = a.update(Message::SaveMaterial);
        assert_eq!(a.doc.materials.len(), before + 1);
        assert!(a.status.contains("already"), "{}", a.status);
        // Partial cutting data is refused with the material's issue.
        let id = a.doc.materials.last().unwrap().id.clone();
        let _ = a.update(Message::EditMaterial(id.clone()));
        let _ = a.update(Message::Edit(Target::Material, "vc_min", "40".into()));
        let _ = a.update(Message::SaveMaterial);
        assert!(
            a.status.contains("every cutting-data field"),
            "{}",
            a.status
        );
        // The project material cannot be deleted while selected.
        let _ = a.update(Message::PickMaterial(id.clone()));
        assert_eq!(a.doc.material, "Titanium Gr5");
        let _ = a.update(Message::DeleteMaterial(id.clone()));
        assert!(a.doc.materials.iter().any(|m| m.id == id));
        assert!(a.status.contains("project material"), "{}", a.status);
        let other = a.doc.materials[0].id.clone();
        let _ = a.update(Message::PickMaterial(other));
        let _ = a.update(Message::DeleteMaterial(id.clone()));
        assert!(!a.doc.materials.iter().any(|m| m.id == id));
        a.page = Page::Materials;
        let _ = a.view();
    }
    #[test]
    fn saving_a_material_keeps_names_unique_trimmed_and_the_project_material_in_step() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        // The editor starts on the first library material, like the tool editor.
        assert_eq!(a.material_draft.id, a.doc.materials[0].id);
        let _ = a.update(Message::NewMaterial);
        let _ = a.update(Message::Edit(
            Target::Material,
            "name",
            "  Titanium Gr5  ".into(),
        ));
        let _ = a.update(Message::SaveMaterial);
        assert_eq!(a.status, "Material saved to the library.");
        let id = a.doc.materials.last().unwrap().id.clone();
        assert_eq!(a.doc.materials.last().unwrap().name(), "Titanium Gr5");
        // Renaming the project material renames the project's material too,
        // and keeping the same name in another case is not a clash with itself.
        let _ = a.update(Message::PickMaterial(id.clone()));
        let _ = a.update(Message::EditMaterial(id.clone()));
        let _ = a.update(Message::Edit(
            Target::Material,
            "name",
            "TITANIUM GR5".into(),
        ));
        let _ = a.update(Message::SaveMaterial);
        assert_eq!(a.doc.material, "TITANIUM GR5");
        assert_eq!(a.doc.selected_material().unwrap().name(), "TITANIUM GR5");
        assert_eq!(a.doc.materials.iter().filter(|m| m.id == id).count(), 1);
        // A rename onto another material's name is refused, in any case.
        let _ = a.update(Message::Edit(
            Target::Material,
            "name",
            "mild STEEL 1018".into(),
        ));
        assert!(
            a.doc
                .material_name_issues(&a.material_draft)
                .iter()
                .any(|e| e.contains("already")),
        );
        let _ = a.update(Message::SaveMaterial);
        assert!(a.status.contains("already"), "{}", a.status);
        assert_eq!(a.doc.material, "TITANIUM GR5");
        assert_eq!(a.doc.material_id, id);
        // Another material's save leaves the project material alone.
        let first = a.doc.materials[0].id.clone();
        let _ = a.update(Message::EditMaterial(first));
        let _ = a.update(Message::Edit(Target::Material, "notes", "Checked.".into()));
        let _ = a.update(Message::SaveMaterial);
        assert_eq!(a.doc.material, "TITANIUM GR5");
        assert_eq!(get(&a.doc.materials[0].p, "notes"), "Checked.");
        assert!(a.doc.validate_structure().is_ok());
        // Deleting a material puts the editor on the first one that is left.
        let _ = a.update(Message::DeleteMaterial(first_but(&a, &id)));
        assert_eq!(a.material_draft.id, a.doc.materials[0].id);
        assert!(a.status.contains("deleted"), "{}", a.status);
        // Deleting a draft that was never saved only discards it.
        let count = a.doc.materials.len();
        let _ = a.update(Message::NewMaterial);
        let draft = a.material_draft.id.clone();
        let _ = a.update(Message::DeleteMaterial(draft));
        assert_eq!(a.doc.materials.len(), count);
        assert_eq!(a.material_draft.id, a.doc.materials[0].id);
        assert!(a.status.contains("discarded"), "{}", a.status);
    }
    fn first_but(a: &App, id: &str) -> String {
        a.doc
            .materials
            .iter()
            .find(|m| m.id != id)
            .unwrap()
            .id
            .clone()
    }
    #[test]
    fn the_material_picker_selects_by_id_and_every_state_renders() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        a.replace_document(Document::sample(), None);
        a.doc.adopt_materials();
        a.page = Page::Work;
        let _ = a.update(Message::SetupEditor);
        let _ = a.view();
        let steel = a.doc.materials[2].id.clone();
        a.project_dirty = false;
        let _ = a.update(Message::PickMaterial(steel.clone()));
        assert_eq!(
            (a.doc.material.as_str(), &a.doc.material_id),
            ("Mild steel 1018", &steel)
        );
        assert!(a.project_dirty);
        // An id that is not in the library changes nothing.
        a.project_dirty = false;
        let _ = a.update(Message::PickMaterial("missing".into()));
        assert_eq!(a.doc.material_id, steel);
        assert!(!a.project_dirty);
        // A material without cutting data, and none at all, still render.
        let mut bare = crate::material::Material::new("Inconel 718");
        bare.p.insert("notes".into(), "Hard.".into());
        let bare_id = bare.id.clone();
        a.doc.materials.push(bare);
        let _ = a.update(Message::PickMaterial(bare_id));
        let _ = a.view();
        a.doc.material_id.clear();
        let _ = a.view();
        for page in [Page::Materials, Page::Work] {
            a.page = page;
            let _ = a.view();
        }
    }
    fn feed_lines(a: &App) -> Vec<String> {
        crate::ui::speed_feed_lines(a)
            .into_iter()
            .map(|(_, line)| line)
            .collect()
    }
    #[test]
    fn picking_the_selected_material_again_changes_nothing() {
        // A pick_list sends the message again for the item already chosen;
        // that must not drop a running plan.
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let same = a.doc.material_id.clone();
        let _ = a.update(Message::Plan);
        assert!(matches!(a.planner, crate::Planner::Running(_)));
        a.project_dirty = false;
        a.status = "Kept.".into();
        let _ = a.update(Message::PickMaterial(same.clone()));
        assert!(matches!(a.planner, crate::Planner::Running(_)));
        assert!(!a.project_dirty);
        assert_eq!(a.status, "Kept.");
        // Another material is a change to the job.
        let other = first_but(&a, &same);
        let _ = a.update(Message::PickMaterial(other.clone()));
        assert_eq!(a.doc.material_id, other);
        assert!(matches!(a.planner, crate::Planner::Idle));
        assert!(a.project_dirty);
        assert_eq!(a.status, "Planning stopped because the job changed.");
    }
    #[test]
    fn the_material_note_has_three_outcomes() {
        use crate::ui::material_note;
        let mut doc = Document::sample();
        doc.adopt_materials();
        assert_eq!(doc.selected_material().unwrap().name(), "Aluminium 6061-T6");
        assert_eq!(material_note(&doc), None);
        let mut bare = crate::material::Material::new("Inconel 718");
        bare.p.insert("notes".into(), "Hard.".into());
        let id = bare.id.clone();
        doc.materials.push(bare);
        assert!(doc.select_material(&id));
        assert_eq!(
            material_note(&doc).as_deref(),
            Some("Inconel 718 has no cutting data: speeds and feeds are not checked.")
        );
        doc.material_id.clear();
        assert_eq!(
            material_note(&doc).as_deref(),
            Some("No material is chosen: speeds and feeds are not checked.")
        );
    }
    #[test]
    fn speeds_and_feeds_outside_the_materials_data_warn_and_never_block_export() {
        use crate::material::SpeedFeed;
        let dir = tempfile::tempdir().unwrap();
        let mut a = planning_app(&dir);
        let mesh = a.imported_mesh.clone().unwrap();
        // The fixture's tools run in range, the drill once it turns faster
        // and feeds more.
        let drill = &mut a.doc.setups[0].operations[3];
        drill.p.insert("rpm".into(), "12000".into());
        drill.p.insert("feed".into(), "1000".into());
        assert_eq!(feed_lines(&a), Vec::<String>::new());
        let pocket_check = |a: &App| {
            let setup = &a.doc.setups[0];
            a.doc.speed_feed(setup, &setup.operations[1])
        };
        assert_eq!(a.doc.setups[0].operations[1].name(), "Pocket");
        // The check uses the operation's own values, whatever the machine
        // allows (its limit blocks such an export on its own).
        for (rpm, feed) in [("30000", "3000"), ("12000", "2500")] {
            let pocket = &mut a.doc.setups[0].operations[1];
            pocket.p.insert("rpm".into(), rpm.into());
            pocket.p.insert("feed".into(), feed.into());
            let Some(SpeedFeed::Out(problems)) = pocket_check(&a) else {
                panic!("{:?}", pocket_check(&a))
            };
            assert_eq!(problems.len(), 1);
            assert_eq!(
                feed_lines(&a),
                vec![format!("Setup 1 · Pocket: {}", problems[0])]
            );
            let vc = if rpm == "30000" { "942" } else { "377" };
            assert_eq!(
                problems[0],
                format!("Surface speed {vc} m/min is above Aluminium 6061-T6's 150–300 m/min.")
            );
        }
        let setup = &a.doc.setups[0];
        assert!(matches!(
            a.doc.speed_feed(setup, &setup.operations[0]),
            Some(SpeedFeed::InRange { .. })
        ));
        // At the machine's own maximum the program exports: the warning is
        // advice and never blocks.
        assert!(crate::post::post_job(&a.doc, &mesh).ready());
        a.smoke_dir = Some(dir.path().to_path_buf());
        let _ = a.update(Message::Page(Page::Post));
        assert!(a.post.as_ref().is_some_and(|j| j.ready()));
        let _ = a.view();
        for tab in ["Tool", "Passes"] {
            let _ = a.update(Message::SelectOp(1));
            let _ = a.update(Message::Tab(tab.into()));
            assert_eq!(a.page, Page::Work);
            let _ = a.view();
        }
        // A suppressed operation is not exported, so it is not listed.
        a.doc.setups[0].operations[1].enabled = false;
        assert_eq!(feed_lines(&a), Vec::<String>::new());
        a.doc.setups[0].operations[1].enabled = true;
        // A material without cutting data: nothing is checked, and the
        // lines say so for each operation. No material: no lines.
        let bare = crate::material::Material::new("Unknown");
        let id = bare.id.clone();
        a.doc.materials.push(bare);
        assert!(a.doc.select_material(&id));
        let setup = &a.doc.setups[0];
        assert_eq!(
            a.doc.speed_feed(setup, &setup.operations[1]),
            Some(SpeedFeed::Unchecked("Unknown has no cutting data.".into()))
        );
        let lines = feed_lines(&a);
        assert_eq!(lines.len(), 6, "{lines:?}");
        assert_eq!(
            lines[1],
            "Setup 1 · Pocket: Not checked. Unknown has no cutting data."
        );
        let _ = a.update(Message::Page(Page::Post));
        let _ = a.view();
        let _ = a.update(Message::SelectOp(1));
        let _ = a.update(Message::Tab("Tool".into()));
        let _ = a.view();
        a.doc.material_id.clear();
        let setup = &a.doc.setups[0];
        assert_eq!(a.doc.speed_feed(setup, &setup.operations[1]), None);
        assert!(feed_lines(&a).is_empty());
        // An operation whose tool is not in the library is not checked, and
        // says why, in the editor and on the NC page.
        let _ = a.doc.select_material(&a.doc.materials[0].id.clone());
        a.doc.setups[0].operations[1].tool = "gone".into();
        let setup = &a.doc.setups[0];
        let missing = "The operation has no tool in the library.";
        assert_eq!(
            a.doc.speed_feed(setup, &setup.operations[1]),
            Some(SpeedFeed::Unchecked(missing.into()))
        );
        assert_eq!(
            crate::ui::speed_feed_lines(&a),
            vec![(false, format!("Setup 1 · Pocket: Not checked. {missing}"))]
        );
        let _ = a.update(Message::Page(Page::Post));
        let _ = a.view();
        let _ = a.update(Message::SelectOp(1));
        let _ = a.update(Message::Tab("Tool".into()));
        let _ = a.view();
        // With no material there is still nothing to say about the tool.
        a.doc.material_id.clear();
        let setup = &a.doc.setups[0];
        assert_eq!(a.doc.speed_feed(setup, &setup.operations[1]), None);
    }
    #[test]
    fn the_nc_heading_follows_the_lines() {
        use crate::ui::{speed_feed_heading, speed_feed_lines};
        let outside = "Speeds and feeds outside the material's cutting data (export still allowed)";
        let unchecked = "Speeds and feeds not checked (export still allowed)";
        let dir = tempfile::tempdir().unwrap();
        let mut a = planning_app(&dir);
        let drill = &mut a.doc.setups[0].operations[3];
        drill.p.insert("rpm".into(), "12000".into());
        drill.p.insert("feed".into(), "1000".into());
        assert_eq!(speed_feed_lines(&a), vec![]);
        // Valid cutting data, one operation out of range, another without a
        // spindle speed: the list is mixed, so it is headed as a problem.
        let pocket = &mut a.doc.setups[0].operations[1];
        pocket.p.insert("rpm".into(), "12000".into());
        pocket.p.insert("feed".into(), "2500".into());
        a.doc.setups[0].operations[2]
            .p
            .insert("rpm".into(), " ".into());
        let lines = speed_feed_lines(&a);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines[0].0
                && lines[0]
                    .1
                    .starts_with("Setup 1 · Pocket: Surface speed 377")
        );
        assert_eq!(
            lines[1],
            (
                false,
                "Setup 1 · Contour: Not checked. The operation has no spindle speed or feed."
                    .into()
            )
        );
        assert_eq!(speed_feed_heading(&lines), outside);
        // Only the operation that could not be checked: not a problem list.
        let pocket = &mut a.doc.setups[0].operations[1];
        pocket.p.insert("rpm".into(), "8000".into());
        pocket.p.insert("feed".into(), "1000".into());
        let lines = speed_feed_lines(&a);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(!lines[0].0);
        assert_eq!(speed_feed_heading(&lines), unchecked);
        // A tool without flutes is the same, and so is a material without data.
        a.doc.setups[0].operations[2]
            .p
            .insert("rpm".into(), "8000".into());
        a.doc.tools[0].p.insert("flutes".into(), String::new());
        let lines = speed_feed_lines(&a);
        assert!(!lines.is_empty() && lines.iter().all(|l| !l.0), "{lines:?}");
        assert!(lines[0].1.ends_with("The tool has no diameter or flutes."));
        assert_eq!(speed_feed_heading(&lines), unchecked);
        let bare = crate::material::Material::new("Unknown");
        let id = bare.id.clone();
        a.doc.materials.push(bare);
        assert!(a.doc.select_material(&id));
        let lines = speed_feed_lines(&a);
        assert_eq!(lines.len(), 6, "{lines:?}");
        assert_eq!(speed_feed_heading(&lines), unchecked);
        // The page builds with each list.
        a.page = Page::Post;
        let _ = a.view();
    }
    #[test]
    fn closing_waits_for_a_sign_out_and_exits_past_blocking_work() {
        use crate::{CloseAction::*, close_action};
        use std::time::Duration;
        let s = Duration::from_secs;
        // The workspace could not be saved: the window stays.
        assert_eq!(close_action(false, None, false, false), Stay);
        assert_eq!(close_action(false, Some((s(20), true)), true, true), Stay);
        // Nothing pending: close; blocking work would keep a windowless
        // process alive, so the process exits.
        assert_eq!(close_action(true, None, false, false), Close);
        assert_eq!(close_action(true, None, true, false), Exit);
        // A G-code export is never cut short: the window closes and the
        // runtime finishes it.
        assert_eq!(close_action(true, None, true, true), Close);
        assert_eq!(close_action(true, None, false, true), Close);
        // A sign-out or sign-in save is waited for, at most 10 seconds.
        for sign_out in [true, false] {
            assert_eq!(
                close_action(true, Some((s(0), sign_out)), true, false),
                Wait
            );
            assert_eq!(
                close_action(
                    true,
                    Some((Duration::from_millis(9_900), sign_out)),
                    false,
                    false
                ),
                Wait
            );
        }
        // Then the window closes, never an exit while it is still pending;
        // a sign-out first starts deleting the saved sign-in.
        assert_eq!(
            close_action(true, Some((s(10), false)), false, false),
            Close
        );
        assert_eq!(close_action(true, Some((s(10), false)), true, false), Close);
        assert_eq!(
            close_action(true, Some((s(10), true)), true, false),
            ClearAndClose
        );
    }
    #[test]
    fn close_waits_for_a_sign_in_save_then_closes() {
        let dir = tempfile::tempdir().unwrap();
        let session = crate::session::sample();
        for saved in [
            Ok(session.clone()),
            Err("Keychain: save denied".to_string()),
        ] {
            let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
            let id = iced::window::Id::unique();
            let _ = a.update(Message::SignIn);
            let _ = a.update(Message::SignedIn(Ok((session.clone(), "oaiapp_1".into()))));
            assert!(a.saving_sign_in());
            let _ = a.update(Message::Close(id));
            assert!(a.closing.is_some());
            assert_eq!(
                a.status,
                "Closing when the sign-in has been saved (at most 10 seconds)…"
            );
            let ok = saved.is_ok();
            let _ = a.update(Message::SessionSaved(saved));
            assert!(a.closing.is_none(), "{ok}");
            assert!(!a.saving_sign_in());
        }
    }
    /// A client for `a` that never sends a request.
    fn idle_client(a: &App) -> std::sync::Arc<crate::openai::Client> {
        std::sync::Arc::new(crate::openai::Client::new(
            a.endpoints.clone(),
            String::new(),
            a.store.clone(),
            crate::session::sample(),
        ))
    }
    #[test]
    fn the_close_decision_follows_what_is_still_running() {
        use crate::CloseAction::*;
        use std::time::{Duration, Instant};
        let dir = tempfile::tempdir().unwrap();
        let fresh = || App::from_workspace(dir.path().join("workspace.json"), None);
        let id = iced::window::Id::unique();
        let start = Instant::now();
        let after = |seconds: f32| start + Duration::from_secs_f32(seconds);
        // A sign-out still pending after 11 s: delete the saved sign-in,
        // then close.
        let mut a = fresh();
        a.sign_out_pending = true;
        assert_eq!(a.close_decision(id, start), Wait);
        assert_eq!(a.close_decision(id, after(9.9)), Wait);
        assert_eq!(a.close_decision(id, after(11.)), ClearAndClose);
        assert!(a.closing.is_none());
        // A sign-in save still pending after 11 s: close.
        let mut a = fresh();
        let _ = a.update(Message::SignIn);
        let _ = a.update(Message::SignedIn(Ok((
            crate::session::sample(),
            "oaiapp_1".into(),
        ))));
        assert!(a.saving_sign_in());
        assert_eq!(a.close_decision(id, start), Wait);
        assert_eq!(a.close_decision(id, after(11.)), Close);
        // Blocking work alone ends the process; with a G-code export
        // running the window closes instead, so the export finishes.
        let mut a = fresh();
        a.post_busy = true;
        assert_eq!(a.close_decision(id, start), Exit);
        a.gcode_exports = 1;
        assert_eq!(a.close_decision(id, start), Close);
        a.post_busy = false;
        assert_eq!(a.close_decision(id, start), Close);
        // A refresh of the sign-in in flight is waited for, at most 10 s.
        let mut a = fresh();
        let client = idle_client(&a);
        a.client = Some(client.clone());
        let refresh = client.hold_refresh();
        assert_eq!(a.close_decision(id, after(1.)), Wait);
        assert_eq!(a.close_decision(id, after(10.5)), Wait);
        assert_eq!(a.close_decision(id, after(11.5)), Close);
        // After the wait, a refresh still waiting for OpenAI is not waited
        // for: with blocking work the process ends.
        a.post_busy = true;
        assert_eq!(a.close_decision(id, after(12.)), Wait);
        assert_eq!(a.close_decision(id, after(22.5)), Exit);
        assert!(a.closing.is_none());
        drop(refresh);
        // A refresh saving its tokens is waited for too, and after the wait
        // the window closes, so that the save finishes.
        let save = client.hold_refresh_save();
        assert_eq!(a.close_decision(id, after(23.)), Wait);
        assert_eq!(a.close_decision(id, after(33.5)), Close);
        assert!(a.closing.is_none());
        drop(save);
        a.post_busy = false;
        // Nothing pending: close.
        assert_eq!(a.close_decision(id, after(34.)), Close);
        assert!(a.closing.is_none());
    }
    #[tokio::test]
    async fn a_close_that_stops_waiting_deletes_through_the_signing_out_client() {
        use crate::fake_openai::{Fake, Req, Resp};
        use std::time::Duration;
        // OpenAI answers the refresh after 300 ms.
        let fake = Fake::start(|req: &Req| match req.path.as_str() {
            "/token" => Resp {
                delay_ms: 300,
                ..Resp::json(
                    200,
                    serde_json::json!({"access_token": "access-2", "refresh_token": "refresh-2", "expires_in": 3600}),
                )
            },
            _ => Resp::answer("{}"),
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        let soon = crate::session::Session {
            expires_at: crate::session::now() + 60,
            ..crate::session::sample()
        };
        a.store.save(&soon).unwrap();
        let client = std::sync::Arc::new(crate::openai::Client::new(
            crate::oauth::Endpoints::local(&fake.base),
            "oaiapp_1".into(),
            a.store.clone(),
            soon,
        ));
        a.signing_out = Some(client.clone());
        a.sign_out_pending = true;
        // A request that first refreshes the sign-in; its /token is out.
        let respond = {
            let client = client.clone();
            tokio::spawn(async move {
                client
                    .respond(&crate::openai::Request {
                        model: "gpt-6.1-sol".into(),
                        messages: vec![(crate::openai::Role::User, "Part.".into())],
                        schema_name: "plan",
                        schema: serde_json::json!({"type": "object"}),
                    })
                    .await
            })
        };
        let mut waited = 0;
        while fake.seen("/token").is_empty() && waited < 200 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            waited += 1;
        }
        assert!(client.refreshing() && !client.saving_refresh());
        // The close stops waiting and starts the delete.
        a.start_close_clear();
        assert!(a.signing_out.is_none());
        // The refresh finishes after the delete; its save is not made.
        respond.await.unwrap().unwrap();
        assert!(!client.refreshing());
        let mut waited = 0;
        while a.store.load().unwrap().is_some() && waited < 200 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            waited += 1;
        }
        assert_eq!(a.store.load().unwrap(), None);
    }
    #[test]
    fn close_waits_for_a_refresh_in_flight_then_closes() {
        use std::time::{Duration, Instant};
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        let client = idle_client(&a);
        a.client = Some(client.clone());
        let id = iced::window::Id::unique();
        let refresh = client.hold_refresh();
        let _ = a.update(Message::Close(id));
        assert!(a.closing.is_some());
        assert_eq!(
            a.status,
            "Closing when the sign-in refresh has finished (at most 10 seconds)…"
        );
        // Tick checks again: still refreshing, still waiting.
        let _ = a.update(Message::Tick);
        assert!(a.closing.is_some());
        drop(refresh);
        let _ = a.update(Message::Tick);
        assert!(a.closing.is_none());
        // A sign-out still pending after the wait hands its client to the
        // delete, which then blocks that client's late refresh save. The
        // delete starts on the blocking pool, as within the app's runtime.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let within = runtime.enter();
        a.store.save(&crate::session::sample()).unwrap();
        a.signing_out = Some(client);
        a.sign_out_pending = true;
        let _ = a.update(Message::Close(id));
        a.closing = Some((id, Instant::now() - Duration::from_secs(11)));
        let _ = a.update(Message::Tick);
        assert!(a.closing.is_none());
        assert!(a.signing_out.is_none());
        // Ending the runtime, as the app does when it ends, joins the delete
        // the client started: the saved sign-in is gone.
        drop(within);
        drop(runtime);
        assert_eq!(a.store.load().unwrap(), None);
    }
    #[test]
    fn a_gcode_export_counts_as_work_that_must_finish() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = planning_app(&dir);
        assert!(!a.finishing_work());
        let _ = a.update(Message::ExportGcode);
        assert!(a.finishing_work());
        // No folder chosen.
        let _ = a.update(Message::GcodeExported(None));
        assert!(!a.finishing_work());
        let _ = a.update(Message::ExportGcode);
        let _ = a.update(Message::ExportGcode);
        let _ = a.update(Message::GcodeExported(Some(Err("disk full".into()))));
        assert!(a.finishing_work());
        let _ = a.update(Message::GcodeExported(Some(Ok((dir.path().into(), 2)))));
        assert!(!a.finishing_work());
        assert!(
            a.status.starts_with("Exported 2 LinuxCNC programs"),
            "{}",
            a.status
        );
    }
    #[test]
    fn close_waits_for_signing_out_then_closes() {
        use std::time::{Duration, Instant};
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        let id = iced::window::Id::unique();
        a.sign_out_pending = true;
        let _ = a.update(Message::Close(id));
        assert!(a.closing.is_some());
        assert_eq!(
            a.status,
            "Closing when signing out has finished (at most 10 seconds)…"
        );
        // A second request keeps the first one's time.
        let first = a.closing.unwrap().1;
        let _ = a.update(Message::Close(id));
        assert_eq!(a.closing.unwrap().1, first);
        let _ = a.update(Message::SignedOut(Ok(())));
        assert!(a.closing.is_none());
        // Bounded: after 10 seconds the window closes anyway, once the
        // saved sign-in's delete has started on the blocking pool, as within
        // the app's runtime.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let within = runtime.enter();
        a.store.save(&crate::session::sample()).unwrap();
        a.sign_out_pending = true;
        let _ = a.update(Message::Close(id));
        a.closing = Some((id, Instant::now() - Duration::from_secs(11)));
        let _ = a.update(Message::Tick);
        assert!(a.closing.is_none());
        // Ending the runtime, as the app does when it ends, joins the
        // delete: the saved sign-in is gone.
        drop(within);
        drop(runtime);
        assert_eq!(a.store.load().unwrap(), None);
        // Nothing pending: no waiting.
        a.sign_out_pending = false;
        let _ = a.update(Message::Close(id));
        assert!(a.closing.is_none());
    }
    #[test]
    fn failed_archive_and_pending_file_actions_do_not_reset_work() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        a.replace_document(Document::sample(), None);
        std::fs::write(dir.path().join("history"), "blocked by file").unwrap();
        let _ = a.update(Message::NewProject);
        assert_eq!(a.doc.title, "Bearing housing");
        assert!(a.status.contains("Could not preserve"));
        a.import_busy = true;
        let _ = a.update(Message::NewProject);
        assert_eq!(a.doc.setups.len(), 1);
        a.import_busy = false;
        a.save_busy = true;
        let _ = a.update(Message::OpenSample);
        assert!(a.status.contains("current file action"));
    }
    #[test]
    fn part_then_machine_then_setup_without_hidden_operations() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        let file = ModelFile {
            name: "part.obj".into(),
            path: "part.obj".into(),
            format: "obj".into(),
            bytes: 10,
        };
        let _ = a.update(Message::FileImported(Some(Ok((
            file,
            Some(crate::mesh::sample()),
        )))));
        assert!(a.doc.setups.is_empty() && !a.is_sample());
        let _view = a.view();
        drop(_view);
        let _ = a.update(Message::AddSetup);
        assert_eq!(a.page, Page::Wizard);
        let machine = a.machine_draft.id.clone();
        let _ = a.update(Message::SaveMachine);
        assert!(a.doc.setups.is_empty());
        let _ = a.update(Message::AssignMachine(machine));
        assert_eq!(a.doc.setups.len(), 1);
        assert!(a.doc.setup().operations.is_empty());
        assert_eq!(get(&a.doc.setup().p, "stock_x"), "");
        let _ = a.update(Message::AddOp(Strategy::Face));
        assert!(a.doc.setup().operations.is_empty());
        assert_eq!(a.page, Page::Tools);
        let _ = a.update(Message::NewTool);
        let _ = a.update(Message::SaveTool);
        let _ = a.update(Message::AddOp(Strategy::Face));
        assert_eq!(a.doc.setup().operations.len(), 1);
        assert!(a.operation().unwrap().geometry.is_empty());
        let _ = a.update(Message::ConfirmDeleteSetup(true));
        assert!(a.doc.setups.is_empty() && a.doc.active.is_empty());
        assert!(a.doc.validate_structure().is_ok());
        let _view = a.view();
    }
    #[test]
    fn setup_actions_on_empty_document_are_safe() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        for message in [
            Message::AddOp(Strategy::Face),
            Message::GenerateAll,
            Message::Generate,
            Message::Flip("rx"),
            Message::Plan,
            Message::SelectOp(0),
            Message::ConfirmDeleteSetup(true),
            Message::DuplicateSetup,
        ] {
            let _ = a.update(message);
            assert!(a.doc.setups.is_empty());
            assert!(a.operation().is_none());
        }
    }
    fn app() -> App {
        // Root the app in its own folder, not the real workspace folder:
        // settings and the saved sign-in live beside the workspace file.
        let dir = std::env::temp_dir().join(format!("axis-test-{}", id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut a = App::from_workspace(dir.join("workspace.json"), None);
        a.doc = Document::sample();
        a.selected = Some(0);
        a.page = Page::Work;
        a
    }
    fn imported_app() -> App {
        let mut a = app();
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/four-setup-block.3mf");
        let mesh = crate::mesh::load(&path).unwrap();
        let file = ModelFile {
            name: "part.3mf".into(),
            path: path.to_string_lossy().into_owned(),
            format: "3mf".into(),
            bytes: 1,
        };
        let _ = a.update(Message::FileImported(Some(Ok((file, Some(mesh))))));
        a
    }
    #[test]
    fn imported_play_generates_animates_pauses_rewinds_and_edits_invalidate_it() {
        let mut a = imported_app();
        let mesh = a.imported_mesh.as_ref().unwrap();
        let top = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| s.normal[2].abs() > 0.99999)
            .max_by(|(_, a), (_, b)| a.center[2].total_cmp(&b.center[2]))
            .unwrap()
            .0;
        let _ = a.update(Message::Surface(top));
        let _ = a.update(Message::Edit(Target::Setup, "stock_top", "10".into()));
        let _ = a.update(Message::Play);
        assert!(a.playing && a.playback_ready(), "{}", a.status);
        a.last_tick = std::time::Instant::now() - std::time::Duration::from_millis(100);
        let _ = a.update(Message::Tick);
        assert!(a.progress > 0. && a.simulation.as_ref().unwrap().seconds > 0.);
        let _ = a.update(Message::Play);
        assert!(!a.playing);
        let _ = a.update(Message::Scrub(0.7));
        assert!(a.simulation.as_ref().unwrap().stock.removed_volume() > 0.);
        let _ = a.update(Message::Scrub(0.));
        assert_eq!(a.simulation.as_ref().unwrap().stock.removed_volume(), 0.);
        let _ = a.update(Message::Scrub(1.));
        let _ = a.update(Message::Play);
        assert!(a.playing && a.progress == 0.);
        let _ = a.update(Message::Edit(Target::Operation, "stepover", "30".into()));
        assert!(!a.playing && !a.playback_ready() && a.simulation.is_none());
        for (key, value) in [
            ("facing_pattern", "One way"),
            ("facing_milling", "Conventional"),
            ("spindle_rotation", "Counterclockwise (CCW)"),
        ] {
            let _ = a.update(Message::Play);
            assert!(a.playing && a.simulation.is_some());
            let _ = a.update(Message::Edit(Target::Operation, key, value.into()));
            assert!(!a.playing && !a.playback_ready() && a.simulation.is_none());
        }
        let _ = a.update(Message::Play);
        assert!(a.playing);
        let _ = a.update(Message::Tab("Geometry".into()));
        assert!(!a.playing && a.simulation.is_none());
        let _ = a.update(Message::AddOp(Strategy::Adaptive));
        let _ = a.update(Message::Play);
        assert!(!a.playing && !a.playback_ready() && a.selected.is_none());
        assert_eq!(a.page, Page::Audit);
    }
    #[test]
    fn stock_mesher_rejects_stale_results_after_rewind_edit_and_detail_change() {
        let mut a = imported_app();
        let mesh = a.imported_mesh.as_ref().unwrap();
        let top = mesh
            .surfaces
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.center[2].total_cmp(&b.center[2]))
            .unwrap()
            .0;
        let _ = a.update(Message::Surface(top));
        let _ = a.update(Message::Generate);
        assert!(a.active_simulation().is_some(), "{}", a.status);
        let initial = a.stock_mesh_key().unwrap();
        let mesh = std::sync::Arc::new(crate::stock_surface::SurfaceMesh {
            vertices: vec![],
            triangles: vec![],
            cell_mm: 0.5,
            milliseconds: 0,
        });
        let _ = a.update(Message::StockMeshed {
            key: initial.clone(),
            seconds: 0.,
            result: Ok(mesh.clone()),
        });
        assert!(a.stock_mesh_key().is_none());
        let _ = a.update(Message::Scrub(0.5));
        let old = a.stock_mesh_key().unwrap();
        let _ = a.update(Message::Scrub(0.));
        let _ = a.update(Message::StockMeshed {
            key: old.clone(),
            seconds: 100.,
            result: Ok(mesh.clone()),
        });
        assert!(a.active_simulation().unwrap().stock_surface.is_none());
        let current = a.stock_mesh_key().unwrap();
        let _ = a.update(Message::StockDetail(crate::stock_surface::Detail::Fine));
        let _ = a.update(Message::StockMeshed {
            key: current,
            seconds: 0.,
            result: Ok(mesh.clone()),
        });
        assert!(a.active_simulation().unwrap().stock_surface.is_none());
        let _ = a.update(Message::Edit(Target::Operation, "feed", "1200".into()));
        let _ = a.update(Message::StockMeshed {
            key: old,
            seconds: 100.,
            result: Ok(mesh),
        });
        assert!(a.simulation.is_none());
    }
    #[test]
    fn imported_pocket_selection_playback_direction_and_reopen_work_end_to_end() {
        let mut a = imported_app();
        a.doc.setup_mut().operations.clear();
        a.selected = None;
        let _ = a.update(Message::AddOp(Strategy::Pocket));
        let _ = a.update(Message::Tab("Geometry".into()));
        assert!(a.can_pick() && a.picking && !a.is_sample());
        let mesh = a.imported_mesh.as_ref().unwrap();
        let placement = crate::stock::Placement::new(mesh, a.doc.active_setup());
        let id = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.normal[2].abs() > 0.999999
                    && placement.transform(s.center)[2] < placement.bounds.max[2] - 0.1
                    && placement.transform(s.center)[2] > placement.bounds.min[2] + 0.1
            })
            .max_by(|(_, a), (_, b)| a.area.total_cmp(&b.area))
            .unwrap()
            .0;
        let _ = a.update(Message::Viewport(crate::viewport::Event::PickSurface(id)));
        assert_eq!(
            a.operation().unwrap().surfaces.as_ref().unwrap().ids,
            vec![id]
        );
        assert!(a.operation().unwrap().geometry.is_empty());
        let _ = a.update(Message::PickMode);
        let _ = a.update(Message::Play);
        assert!(a.playing && a.playback_ready(), "{}", a.status);
        assert_eq!(
            a.active_simulation().unwrap().plan.strategy,
            Strategy::Pocket
        );
        let _ = a.update(Message::Scrub(0.65));
        let removed = a.active_simulation().unwrap().stock.removed_volume();
        assert!(removed > 1000.);
        let _ = a.update(Message::Scrub(0.1));
        assert!(a.active_simulation().unwrap().stock.removed_volume() < removed);
        for (key, value) in [
            ("pocket_milling", "Conventional"),
            ("spindle_rotation", "Counterclockwise (CCW)"),
            ("pocket_entry", "Plunge"),
        ] {
            let _ = a.update(Message::Edit(Target::Operation, key, value.into()));
            assert!(!a.playing && a.simulation.is_none());
            let _ = a.update(Message::Play);
            assert!(a.playing && a.playback_ready(), "{}", a.status);
        }
        assert_eq!(
            a.active_simulation().unwrap().plan.milling_direction,
            "Conventional"
        );
        assert!(!a.active_simulation().unwrap().plan.spindle_clockwise);
        let restored: Document =
            serde_json::from_str(&serde_json::to_string(&a.doc).unwrap()).unwrap();
        let op = restored.setup().operations.last().unwrap();
        assert_eq!(op.surfaces.as_ref().unwrap().ids, vec![id]);
        assert_eq!(pocket_setting(&op.p, "pocket_milling"), "Conventional");
        assert_eq!(pocket_setting(&op.p, "pocket_entry"), "Plunge");
    }
    #[test]
    fn imported_facing_never_switches_to_sample_and_selections_survive_reopen() {
        let mut a = imported_app();
        let _ = a.update(Message::Tab("Geometry".into()));
        assert!(a.picking && !a.is_sample());
        let file_path = a.doc.file.as_ref().unwrap().path.clone();
        let mesh = a.imported_mesh.as_ref().unwrap();
        let top = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| s.normal[2].abs() > 0.99999)
            .max_by(|(_, a), (_, b)| a.center[2].total_cmp(&b.center[2]))
            .map(|(id, _)| id)
            .unwrap();
        let _ = a.update(Message::Viewport(crate::viewport::Event::PickSurface(top)));
        assert_eq!(
            a.operation().unwrap().surfaces.as_ref().unwrap().ids,
            vec![top]
        );
        assert!(a.operation().unwrap().geometry.is_empty());
        let _ = a.update(Message::Feature(0));
        let _ = a.update(Message::Viewport(crate::viewport::Event::Pick(0)));
        assert!(a.operation().unwrap().geometry.is_empty());
        let _ = a.update(Message::PickMode);
        assert!(!a.picking && !a.is_sample());
        let _ = a.update(Message::Edit(Target::Setup, "stock_top", "10".into()));
        let _ = a.update(Message::Generate);
        assert!(!a.operation().unwrap().preview);
        assert!(a.status.contains("preview ready"), "{}", a.status);
        let _ = a.update(Message::Plan);
        assert!(matches!(a.planner, crate::Planner::Idle));
        let document = serde_json::from_str(&serde_json::to_string(&a.doc).unwrap()).unwrap();
        a.replace_document(document, None);
        let _ = a.update(Message::ModelRestored(crate::mesh::load(
            std::path::Path::new(&file_path),
        )));
        let _ = a.update(Message::SelectOp(0));
        assert_eq!(
            a.operation().unwrap().surfaces.as_ref().unwrap().ids,
            vec![top]
        );
        assert!(a.op_issues().is_empty(), "{:?}", a.op_issues());
        let _ = a.update(Message::Tab("Geometry".into()));
        let _ = a.update(Message::Surface(top));
        assert!(
            a.operation()
                .unwrap()
                .surfaces
                .as_ref()
                .unwrap()
                .ids
                .is_empty()
        );
        assert!(a.op_issues().iter().any(|i| i.contains("Select at least")));
    }
    #[test]
    fn imported_replacement_invalidates_selections_and_unsupported_strategies_keep_part() {
        let mut a = imported_app();
        let _ = a.update(Message::Surface(0));
        let file = a.doc.file.clone().unwrap();
        let mesh = a.imported_mesh.clone().unwrap();
        let _ = a.update(Message::FileImported(Some(Ok((file, Some(mesh))))));
        assert!(
            a.doc
                .setups
                .iter()
                .flat_map(|s| &s.operations)
                .all(|o| o.geometry.is_empty() && o.surfaces.is_none())
        );
        let count = a.doc.setup().operations.len();
        let _ = a.update(Message::AddOp(Strategy::Adaptive));
        assert_eq!(a.doc.setup().operations.len(), count);
        assert!(!a.picking && !a.is_sample());
        assert!(a.selected.is_none());
        assert_eq!(a.page, Page::Audit);
        assert!(a.status.contains("not implemented"));
        let _view = a.view();
    }
    #[test]
    fn changed_mesh_and_repositioned_selection_are_reported_and_legacy_sample_ids_removed() {
        let mut a = imported_app();
        let mesh = a.imported_mesh.as_ref().unwrap();
        let top = mesh
            .surfaces
            .iter()
            .position(|s| s.normal[2].abs() > 0.99999)
            .unwrap();
        let _ = a.update(Message::Surface(top));
        let mut changed = a.imported_mesh.clone().unwrap();
        changed.fingerprint = "0".repeat(64);
        a.operation_mut().unwrap().geometry = vec![0];
        let _ = a.update(Message::ModelRestored(Ok(changed)));
        assert!(a.operation().unwrap().geometry.is_empty());
        assert!(a.op_issues().iter().any(|i| i.contains("geometry changed")));
        assert!(
            a.job_issues()
                .iter()
                .any(|i| i.contains("geometry changed"))
        );
        let _ = a.update(Message::Surface(top));
        assert!(a.op_issues().is_empty());
        let _ = a.update(Message::Edit(Target::Setup, "ry", "90".into()));
        assert!(
            a.op_issues()
                .iter()
                .any(|i| i.contains("not perpendicular"))
        );
        let _ = a.update(Message::ClearSurfaces);
        assert!(a.operation().unwrap().surfaces.is_none());
    }
    #[test]
    fn all_new_operations_complete_the_application_flow_and_job_audit_collects_failures() {
        let mut a = imported_app();
        a.doc.setup_mut().operations.clear();
        a.selected = None;
        for tool in &mut a.doc.tools {
            tool.p.insert("length".into(), "60".into());
            tool.p.insert("stickout".into(), "65".into());
        }
        a.doc
            .tools
            .iter_mut()
            .find(|t| get(&t.p, "type") == "Drill")
            .unwrap()
            .p
            .insert("diameter".into(), "4".into());
        for strategy in [
            Strategy::Contour,
            Strategy::Drill,
            Strategy::Parallel,
            Strategy::Waterline,
        ] {
            let _ = a.update(Message::AddOp(strategy));
            let _ = a.update(Message::Edit(
                Target::Operation,
                "surface_stepover",
                "2".into(),
            ));
            let _ = a.update(Message::Edit(Target::Operation, "tolerance", "0.1".into()));
            if strategy == Strategy::Drill {
                let mesh = a.imported_mesh.as_ref().unwrap();
                let top = mesh
                    .surfaces
                    .iter()
                    .filter(|s| s.normal[2].abs() > 0.999999)
                    .map(|s| s.center[2])
                    .fold(f32::NEG_INFINITY, f32::max);
                let ids: Vec<_> = mesh
                    .surfaces
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| {
                        s.normal[2].abs() > 0.999999 && (s.center[2] - top).abs() < 0.001
                    })
                    .map(|(id, _)| id)
                    .collect();
                for id in ids {
                    let _ = a.update(Message::Surface(id));
                }
            }
            assert!(
                a.op_issues().is_empty(),
                "{strategy:?}: {:?}",
                a.op_issues()
            );
            let _ = a.update(Message::Play);
            assert!(
                a.playing && a.active_simulation().is_some(),
                "{strategy:?}: {}",
                a.status
            );
            let _ = a.update(Message::Scrub(1.));
            assert!(a.active_simulation().unwrap().stock.removed_volume() > 0.);
            let _ = a.update(Message::Scrub(0.));
            let sim = a.active_simulation().unwrap();
            assert_eq!(sim.removed_volume(), sim.inherited_volume());
            for tab in strategy.tabs() {
                let _ = a.update(Message::Tab(tab.into()));
                let _view = a.view();
            }
            let _ = a.update(Message::Edit(Target::Operation, "feed", "850".into()));
            assert!(!a.playback_ready());
            let _ = a.update(Message::Generate);
            assert!(a.active_simulation().is_some(), "{}", a.status);
        }
        let _ = a.update(Message::GenerateAll);
        assert_eq!(a.audit_report.len(), 4);
        assert!(a.audit_report.iter().all(|r| r.1), "{:?}", a.audit_report);
        assert_eq!(a.page, Page::Audit);
        drop(a.view());
        let saved: Document =
            serde_json::from_str(&serde_json::to_string(&a.doc).unwrap()).unwrap();
        assert!(saved.validate_structure().is_ok());
        a.doc = saved;
        let mut second = a.doc.setup().duplicate("G55");
        second.p.insert("stock_source".into(), "fresh".into());
        second.id = id();
        second.p.insert("name".into(), "Second placement".into());
        second.operations[0].p.insert("feed".into(), "0".into());
        let mut unavailable = Operation::new(Strategy::FiveAxis, a.doc.tools[0].id.clone());
        unavailable.geometry.clear();
        second.operations.push(unavailable);
        second.operations[1].enabled = false;
        a.doc.setups.push(second);
        let _ = a.update(Message::GenerateAll);
        assert_eq!(a.audit_report.len(), 8);
        assert_eq!(a.audit_report.iter().filter(|r| !r.1).count(), 4);
        let _ = a.update(Message::SelectOp(0));
        let _ = a.update(Message::Edit(Target::Operation, "feed", "900".into()));
        assert!(a.audit_report.is_empty());
    }
    #[test]
    fn inside_contour_geometry_selection_and_profile_playback_work_on_import() {
        let mut a = imported_app();
        a.doc.setup_mut().operations.clear();
        a.selected = None;
        let _ = a.update(Message::AddOp(Strategy::Contour));
        let _ = a.update(Message::Edit(
            Target::Operation,
            "boundary",
            "Selected geometry".into(),
        ));
        let _ = a.update(Message::Edit(Target::Operation, "side", "Inside".into()));
        let _ = a.update(Message::Tab("Geometry".into()));
        assert!(a.can_pick() && a.picking);
        let mesh = a.imported_mesh.as_ref().unwrap();
        let place = crate::stock::Placement::new(mesh, a.doc.active_setup());
        let id = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.normal[2].abs() > 0.999999
                    && place.transform(s.center)[2] < place.bounds.max[2] - 0.1
                    && place.transform(s.center)[2] > place.bounds.min[2] + 0.1
            })
            .max_by(|(_, a), (_, b)| a.area.total_cmp(&b.area))
            .unwrap()
            .0;
        let _ = a.update(Message::Surface(id));
        let _ = a.update(Message::Play);
        assert!(a.playing && a.active_simulation().is_some(), "{}", a.status);
        assert_eq!(
            a.active_simulation().unwrap().plan.strategy,
            Strategy::Contour
        );
        let _ = a.update(Message::Scrub(1.));
        assert!(a.active_simulation().unwrap().stock.removed_volume() > 0.);
        for tab in ["Geometry", "Passes", "Linking"] {
            let _ = a.update(Message::Tab(tab.into()));
            drop(a.view());
        }
    }
    #[test]
    fn every_unavailable_strategy_is_blocked_without_mutating_the_job() {
        let mut a = imported_app();
        let before = serde_json::to_string(&a.doc).unwrap();
        for s in Strategy::ALL.into_iter().filter(|s| !s.imported_preview()) {
            let _ = a.update(Message::AddOp(s));
            assert_eq!(a.page, Page::Audit);
            assert_eq!(serde_json::to_string(&a.doc).unwrap(), before);
            assert!(a.status.contains("Unavailable"));
        }
    }
    #[test]
    fn stock_mode_switch_keeps_offsets_and_dimensions_and_invalidates_preview() {
        let mut a = app();
        let _ = a.update(Message::Generate);
        assert!(a.operation().unwrap().preview);
        let _ = a.update(Message::Edit(Target::Setup, "stock_top", "10".into()));
        assert!(!a.operation().unwrap().preview);
        let _ = a.update(Message::Edit(
            Target::Setup,
            "stock_type",
            "Fixed size box".into(),
        ));
        let _ = a.update(Message::Edit(Target::Setup, "stock_x", "200".into()));
        let _ = a.update(Message::Edit(
            Target::Setup,
            "stock_type",
            "Relative size box".into(),
        ));
        assert_eq!(get(&a.doc.setup().p, "stock_top"), "10");
        assert_eq!(get(&a.doc.setup().p, "stock_x"), "200");
        let duplicate = a.doc.setup().duplicate("G55");
        assert_eq!(get(&duplicate.p, "stock_top"), "10");
        a.doc.setup_mut().p.insert("stock_top".into(), "3".into());
        assert_eq!(get(&duplicate.p, "stock_top"), "10");
    }
    #[test]
    fn edit_invalidates_generated_preview() {
        let mut a = app();
        let _ = a.update(Message::Generate);
        assert!(a.operation().unwrap().preview);
        let _ = a.update(Message::Edit(Target::Operation, "feed", "1200".into()));
        assert!(!a.operation().unwrap().preview);
        assert!(a.dirty)
    }
    #[test]
    fn assignment_is_an_independent_snapshot() {
        let mut a = app();
        let machine = a.doc.machines[1].id.clone();
        let _ = a.update(Message::AssignMachine(machine));
        a.doc.machines[1].p.insert("rpm".into(), "500".into());
        assert_ne!(get(&a.doc.setup().machine.p, "rpm"), "500")
    }
    #[test]
    fn tool_in_use_cannot_be_deleted() {
        let mut a = app();
        let tool = a.doc.tools[0].id.clone();
        let _ = a.update(Message::DeleteTool(tool));
        assert_eq!(a.doc.tools.len(), 3)
    }
    #[test]
    fn new_strategy_gets_correct_tabs_and_tool() {
        let mut a = app();
        let _ = a.update(Message::AddOp(Strategy::Drill));
        assert_eq!(a.operation().unwrap().tool, a.doc.tools[2].id);
        assert!(a.operation().unwrap().strategy.tabs().contains(&"Cycle"));
        assert!(a.op_issues().is_empty())
    }
    #[test]
    fn program_page_rechecks_after_edits_and_ignores_stale_results() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        let (doc, mesh) = crate::post::fixture::job();
        a.replace_document(doc, None);
        a.imported_mesh = Some(mesh);
        a.smoke_dir = Some(dir.path().to_path_buf());
        let _ = a.update(Message::Page(Page::Post));
        assert!(a.post.as_ref().is_some_and(|j| j.ready()));
        let _ = a.view();
        let stale = a.post_epoch;
        let _ = a.update(Message::Edit(Target::Setup, "offset", "G92".into()));
        assert!(a.post.is_none() && a.post_epoch != stale);
        let _ = a.update(Message::PostChecked(
            stale,
            std::sync::Arc::new(crate::post::JobPost::default()),
        ));
        assert!(a.post.as_ref().is_none_or(|j| !j.setups.is_empty()));
        let _ = a.update(Message::NewTool);
        assert_eq!(a.tool_draft.number(), Some(4));
    }
    fn fresh_app(dir: &tempfile::TempDir) -> App {
        App::from_workspace(dir.path().join("workspace.json"), None)
    }
    /// Plays a whole sign-in: the browser result, then the save step's result.
    /// The save step writes the session before it reports, and unit tests do
    /// not run tasks, so the write happens here.
    fn sign_in_with(a: &mut App, dir: &tempfile::TempDir, session: crate::session::Session) {
        crate::session::default_store(dir.path(), true)
            .save(&session)
            .unwrap();
        let _ = a.update(Message::SignIn);
        let _ = a.update(Message::SignedIn(Ok((session.clone(), "oaiapp_1".into()))));
        let _ = a.update(Message::SessionSaved(Ok(session)));
    }
    /// What a message that must be ignored may not change.
    fn snapshot(
        a: &App,
    ) -> (
        crate::Account,
        Option<usize>,
        Vec<String>,
        String,
        String,
        u64,
    ) {
        (
            a.account.clone(),
            a.client
                .as_ref()
                .map(|c| std::sync::Arc::as_ptr(c) as usize),
            a.models.clone(),
            a.settings.model.clone(),
            a.account_note.clone(),
            a.account_epoch,
        )
    }
    /// The fixture block, imported, with a material: ready to plan once signed in.
    fn planning_app(dir: &tempfile::TempDir) -> App {
        let mut a = fresh_app(dir);
        let (mut doc, mesh) = crate::post::fixture::job();
        doc.adopt_materials();
        let starter = doc
            .materials
            .iter()
            .find(|m| m.name() == "Aluminium 6061-T6")
            .unwrap()
            .id
            .clone();
        assert!(doc.select_material(&starter));
        a.replace_document(doc, None);
        a.imported_mesh = Some(mesh);
        a
    }
    fn signed_in_planning_app(dir: &tempfile::TempDir) -> App {
        let mut a = planning_app(dir);
        sign_in_with(&mut a, dir, crate::session::sample());
        let epoch = a.account_epoch;
        let _ = a.update(Message::ModelsLoaded(epoch, Ok(vec!["gpt-6.1-sol".into()])));
        a
    }
    fn checked_preview(a: &App, view: bool) -> std::sync::Arc<crate::PlanPreview> {
        let mesh = a.imported_mesh.as_ref().unwrap();
        let report = crate::analysis::report(&a.doc, mesh).unwrap();
        let proposal = crate::proposal::tests::good(&report);
        let (check, job) = crate::plan_check::check(&a.doc, mesh, &report, &proposal);
        assert!(check.passed, "{:?}", check.problems);
        let job = job.unwrap();
        // Draft is enough to build the page; the app meshes at Standard.
        let draft = crate::stock_surface::Detail::Draft;
        let view = view.then(|| crate::planner::final_view_at(&job, mesh, draft).unwrap());
        std::sync::Arc::new(crate::PlanPreview {
            outcome: crate::planner::Outcome {
                proposal,
                check,
                job,
                report,
                requests: 2,
                chosen: 2,
                more_warnings: 0,
                more_unmachined: 0,
                note: None,
            },
            view,
        })
    }
    #[test]
    fn plan_starts_only_when_the_checklist_is_met() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = planning_app(&dir);
        let unmet = |a: &App| -> Vec<&'static str> {
            a.plan_checklist()
                .into_iter()
                .filter(|l| !l.ok)
                .map(|l| l.label)
                .collect()
        };
        assert_eq!(unmet(&a), vec!["ChatGPT sign-in"]);
        let _ = a.update(Message::Plan);
        assert!(matches!(a.planner, crate::Planner::Idle));
        assert!(
            a.status.starts_with("ChatGPT sign-in: Sign in"),
            "{}",
            a.status
        );
        let mut a = signed_in_planning_app(&dir);
        assert!(unmet(&a).is_empty());
        let starter = a.doc.material_id.clone();
        a.doc.material_id.clear();
        assert_eq!(unmet(&a), vec!["Material"]);
        let _ = a.update(Message::Plan);
        assert!(matches!(a.planner, crate::Planner::Idle));
        assert_eq!(a.status, "Material: Choose a material on the setup page.");
        // A material without cutting data cannot be planned with either.
        let bare = crate::material::Material::new("Inconel 718");
        let bare_id = bare.id.clone();
        a.doc.materials.push(bare);
        assert!(a.doc.select_material(&bare_id));
        assert_eq!(unmet(&a), vec!["Material"]);
        let _ = a.update(Message::Plan);
        assert!(matches!(a.planner, crate::Planner::Idle));
        assert_eq!(
            a.status,
            "Material: Add cutting data to Inconel 718 on the Materials page."
        );
        assert!(a.doc.select_material(&starter));
        let _ = a.update(Message::Plan);
        assert!(matches!(
            a.planner,
            crate::Planner::Running(crate::planner::Progress::Analyzing)
        ));
        assert!(a.plan_run.is_some());
        let epoch = a.plan_epoch;
        let _ = a.update(Message::Plan);
        assert_eq!(
            a.plan_epoch, epoch,
            "a second click while running does nothing"
        );
    }
    #[test]
    fn setup_1_must_be_valid_as_the_user_has_it() {
        // The checks run on a copy where setup 1 is named "Setup 1", so a
        // blank name would pass them and then fail in the applied job.
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        a.doc.setups[0].p.insert("name".into(), "  ".into());
        let unmet: Vec<_> = a.plan_checklist().into_iter().filter(|l| !l.ok).collect();
        assert_eq!(
            unmet.len(),
            1,
            "{:?}",
            unmet.iter().map(|l| l.label).collect::<Vec<_>>()
        );
        assert!(unmet[0].label.starts_with("Setup 1"), "{}", unmet[0].label);
        assert_eq!(unmet[0].hint, "Give the setup a name.");
        let _ = a.update(Message::Plan);
        assert!(matches!(a.planner, crate::Planner::Idle));
        assert!(
            a.status.ends_with(": Give the setup a name."),
            "{}",
            a.status
        );
        // Any of setup 1's own issues counts, here a negative clearance.
        a.doc.setups[0].p.insert("name".into(), "Top".into());
        assert!(a.plan_checklist().iter().all(|l| l.ok));
        a.doc.setups[0].p.insert("clearance".into(), "-1".into());
        let unmet: Vec<_> = a.plan_checklist().into_iter().filter(|l| !l.ok).collect();
        assert_eq!(unmet.len(), 1);
        assert_eq!(unmet[0].hint, a.doc.setups[0].issues().join(" "));
    }
    #[test]
    fn progress_and_results_belong_to_the_current_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let preview = checked_preview(&a, false);
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let asking = crate::planner::Event::Stage(crate::planner::Progress::Asking(2));
        let _ = a.update(Message::PlanProgress(epoch.wrapping_sub(1), asking.clone()));
        assert!(matches!(
            a.planner,
            crate::Planner::Running(crate::planner::Progress::Analyzing)
        ));
        let _ = a.update(Message::PlanProgress(epoch, asking));
        assert!(matches!(
            a.planner,
            crate::Planner::Running(crate::planner::Progress::Asking(2))
        ));
        let _ = a.update(Message::PlanFinished(
            epoch.wrapping_sub(1),
            Ok(preview.clone()),
        ));
        assert!(matches!(a.planner, crate::Planner::Running(_)));
        let _ = a.update(Message::PlanFinished(epoch, Ok(preview)));
        assert!(matches!(a.planner, crate::Planner::Preview(_)));
        assert_eq!(a.status, "Plan ready to review: 2 setups, 6 operations.");
        assert!(a.plan_run.is_none());
    }
    fn send(a: &mut App, epoch: u64, event: crate::planner::Event) {
        let _ = a.update(Message::PlanProgress(epoch, event));
    }
    #[test]
    fn the_timeline_follows_the_runs_events_and_stays_until_the_plan_goes() {
        use crate::planner::{Event::*, Progress::*, StepResult};
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        a.page = Page::Ai;
        let preview = checked_preview(&a, false);
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        assert!(a.plan_timeline.is_empty());
        let _ = a.view();
        // An event of an older run changes nothing.
        send(&mut a, epoch.wrapping_sub(1), Stage(Asking(1)));
        assert!(a.plan_timeline.is_empty());
        send(&mut a, epoch, Stage(Asking(1)));
        assert_eq!(a.plan_timeline.len(), 1);
        assert_eq!(a.plan_timeline[0].stage, Asking(1));
        assert!(a.plan_timeline[0].ended.is_none() && a.plan_timeline[0].result.is_none());
        assert!(matches!(a.planner, crate::Planner::Running(Asking(1))));
        // The next stage ends the open step; its check's result joins it.
        send(&mut a, epoch, Stage(Checking(1)));
        assert_eq!(a.plan_timeline.len(), 2);
        let first = &a.plan_timeline[0];
        assert!(first.ended.is_some_and(|ended| ended >= first.started));
        assert!(a.plan_timeline[1].ended.is_none());
        let problems = StepResult::Problems {
            shown: vec!["Setup 1, operation 1: T9 is not in the tool library.".into()],
            more: 0,
        };
        send(&mut a, epoch, Checked(1, problems.clone()));
        assert_eq!(a.plan_timeline[1].result, Some(problems.clone()));
        assert!(a.plan_timeline[1].ended.is_some());
        // A result for a request with no step of its kind goes nowhere.
        send(&mut a, epoch, Checked(9, StepResult::Passed));
        send(
            &mut a,
            epoch,
            Checked(2, StepResult::Unreadable("x".into())),
        );
        assert_eq!(a.plan_timeline.len(), 2);
        assert!(
            a.plan_timeline
                .iter()
                .all(|s| s.result != Some(StepResult::Passed))
        );
        // An unreadable reply joins its request's step, and ends it.
        send(&mut a, epoch, Stage(Repairing(2)));
        send(
            &mut a,
            epoch,
            Checked(2, StepResult::Unreadable("not valid JSON".into())),
        );
        assert_eq!(
            a.plan_timeline[2].result,
            Some(StepResult::Unreadable("not valid JSON".into()))
        );
        assert!(a.plan_timeline[2].ended.is_some());
        for stage in [Repairing(3), Checking(3)] {
            send(&mut a, epoch, Stage(stage));
        }
        let _ = a.view();
        send(&mut a, epoch.wrapping_sub(1), Checked(3, problems.clone()));
        send(&mut a, epoch, Checked(3, StepResult::Passed));
        send(&mut a, epoch, Stage(Previewing));
        assert_eq!(a.plan_timeline.len(), 6);
        assert_eq!(a.plan_timeline[4].result, Some(StepResult::Passed));
        assert!(a.plan_timeline[5].ended.is_none());
        // The finished run keeps its timeline, every step ended.
        let _ = a.update(Message::PlanFinished(epoch, Ok(preview)));
        assert!(matches!(a.planner, crate::Planner::Preview(_)));
        assert_eq!(a.plan_timeline.len(), 6);
        assert!(a.plan_timeline.iter().all(|s| s.ended.is_some()));
        let _ = a.view();
        // A late event does not touch it.
        send(&mut a, epoch, Stage(Asking(1)));
        assert_eq!(a.plan_timeline.len(), 6);
        // Discard, and an edit, clear it.
        let _ = a.update(Message::DiscardPlan);
        assert!(a.plan_timeline.is_empty());
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        send(&mut a, epoch, Stage(Analyzing));
        assert_eq!(a.plan_timeline.len(), 1);
        let _ = a.update(Message::Edit(Target::Setup, "stock_top", "6".into()));
        assert!(a.plan_timeline.is_empty());
        // A new run starts a new timeline; Cancel clears the old one.
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        send(&mut a, epoch, Stage(Analyzing));
        let _ = a.update(Message::PlanCancel);
        assert!(a.plan_timeline.is_empty());
    }
    #[test]
    fn a_failed_run_keeps_its_timeline_and_the_next_run_starts_clean() {
        use crate::planner::{Event::*, Progress::*, StepResult};
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        a.page = Page::Ai;
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        send(&mut a, epoch, Stage(Analyzing));
        send(&mut a, epoch, Stage(Asking(1)));
        send(
            &mut a,
            epoch,
            Checked(1, StepResult::Unreadable("oops".into())),
        );
        send(&mut a, epoch, Stage(Repairing(2)));
        let failure = crate::planner::Failure {
            message: "No plan.".into(),
            problems: vec![],
            api: None,
            requests: 2,
        };
        let _ = a.update(Message::PlanFinished(epoch, Err(failure)));
        assert!(matches!(a.planner, crate::Planner::Failed(_)));
        assert_eq!(a.plan_timeline.len(), 3);
        assert!(a.plan_timeline.iter().all(|s| s.ended.is_some()));
        assert_eq!(
            a.plan_timeline[1].result,
            Some(StepResult::Unreadable("oops".into()))
        );
        let _ = a.view();
        // Try again: the old timeline and count are gone.
        let counted = a.plan_received.clone();
        counted.store(900, std::sync::atomic::Ordering::Relaxed);
        let _ = a.update(Message::Plan);
        assert!(a.plan_timeline.is_empty());
        assert!(!std::sync::Arc::ptr_eq(&counted, &a.plan_received));
        assert_eq!(
            a.plan_received.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }
    /// Timeline steps `(stage, began, ended, result)`, the times in seconds
    /// before `now`; None for an open step.
    fn steps_ago(
        now: std::time::Instant,
        steps: Vec<(
            crate::planner::Progress,
            u64,
            Option<u64>,
            Option<crate::planner::StepResult>,
        )>,
    ) -> Vec<crate::Step> {
        let ago = |seconds| {
            now.checked_sub(std::time::Duration::from_secs(seconds))
                .unwrap()
        };
        steps
            .into_iter()
            .map(|(stage, began, ended, result)| crate::Step {
                stage,
                started: ago(began),
                ended: ended.map(ago),
                result,
            })
            .collect()
    }
    #[test]
    fn running_rows_list_each_steps_problems_and_stream_count_under_it() {
        use crate::planner::{Progress::*, StepResult};
        use crate::ui::Mark::*;
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        a.plan_model = "gpt-6.1-sol".into();
        let now = std::time::Instant::now();
        let problems = StepResult::Problems {
            shown: vec!["Setup 1, operation 2: Pocket too deep.".into()],
            more: 2,
        };
        a.plan_timeline = steps_ago(
            now,
            vec![
                (Analyzing, 60, Some(58), None),
                (Asking(1), 58, Some(20), None),
                (Checking(1), 20, Some(11), Some(problems)),
                (
                    Repairing(2),
                    11,
                    Some(10),
                    Some(StepResult::Unreadable("not valid JSON".into())),
                ),
                (Checking(2), 10, Some(8), Some(StepResult::Passed)),
                (
                    Checking(3),
                    8,
                    Some(7),
                    Some(StepResult::Problems {
                        shown: vec![],
                        more: 0,
                    }),
                ),
                (Extending(3), 7, None, None),
            ],
        );
        a.plan_received
            .store(1, std::sync::atomic::Ordering::Relaxed);
        let rows = crate::ui::timeline_rows(&a, now, false);
        let text: Vec<_> = rows.iter().map(|r| r.text.as_str()).collect();
        // A step that ended reads in the past tense; the open one as it runs.
        assert_eq!(
            text,
            [
                "Analyzed the part · 2 s",
                "Asked gpt-6.1-sol for a plan (1 of 4) · 38 s",
                "Checked the plan (1 of 4) · 9 s",
                "Asked gpt-6.1-sol for a correction (2 of 4) · 1 s",
                "Checked the plan (2 of 4) · 2 s",
                "Checked the plan (3 of 4) · 1 s",
                "The plan passes; asking gpt-6.1-sol about faces still covered (3 of 4)… · 7 s",
            ]
        );
        // A tick only for steps that went well; a check with problems and a
        // reply that could not be read get the warning mark.
        let marks: Vec<_> = rows.iter().map(|r| r.mark).collect();
        assert_eq!(marks, [Done, Done, Warn, Warn, Done, Warn, Open]);
        assert!(rows[0].notes.is_empty() && rows[1].notes.is_empty());
        assert_eq!(
            rows[2].notes,
            ["• Setup 1, operation 2: Pocket too deep.", "• and 2 more"]
        );
        assert_eq!(
            rows[3].notes,
            ["The reply could not be read: not valid JSON"]
        );
        assert_eq!(rows[4].notes, ["passed"]);
        assert_eq!(rows[5].notes, ["did not pass"]);
        // Only the open request shows what has streamed in so far.
        assert_eq!(rows[6].notes, ["received 1 character"]);
        a.plan_received
            .store(1840, std::sync::atomic::Ordering::Relaxed);
        let rows = crate::ui::timeline_rows(&a, now, false);
        assert_eq!(rows[6].notes, ["received 1840 characters"]);
        assert!(
            rows[..6]
                .iter()
                .all(|r| r.notes.iter().all(|n| !n.starts_with("received")))
        );
        // An open check says nothing yet.
        a.plan_timeline[6].stage = Checking(3);
        assert!(crate::ui::timeline_rows(&a, now, false)[6].notes.is_empty());
    }
    #[test]
    fn a_finished_timeline_is_one_line_a_step_with_a_short_summary() {
        use crate::planner::{Progress::*, StepResult};
        use crate::ui::Mark::*;
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        a.plan_model = "gpt-6.1-sol".into();
        let now = std::time::Instant::now();
        let many = StepResult::Problems {
            shown: (1..=5)
                .map(|i| format!("Setup 1, operation {i}: Too deep."))
                .collect(),
            more: 2,
        };
        let one = StepResult::Problems {
            shown: vec!["Setup 1, operation 2: Too deep.".into()],
            more: 0,
        };
        // A run that passed on its second proposal.
        a.plan_timeline = steps_ago(
            now,
            vec![
                (Analyzing, 91, Some(90), None),
                (Asking(1), 90, Some(52), None),
                (Checking(1), 52, Some(40), Some(many)),
                (Repairing(2), 40, Some(19), None),
                (Checking(2), 19, Some(7), Some(StepResult::Passed)),
                (Previewing, 7, Some(3), None),
            ],
        );
        let rows = crate::ui::timeline_rows(&a, now, true);
        let text: Vec<_> = rows.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(
            text,
            [
                "Analyzed the part · 1 s",
                "Asked gpt-6.1-sol for a plan (1 of 4) · 38 s",
                "Checked the plan (1 of 4) · 12 s · 7 problems",
                "Asked gpt-6.1-sol for a correction (2 of 4) · 21 s",
                "Checked the plan (2 of 4) · 12 s · passed",
                "Built the 3D view · 4 s",
            ]
        );
        assert!(
            rows.iter().all(|r| r.notes.is_empty()),
            "no lines under a row"
        );
        let marks: Vec<_> = rows.iter().map(|r| r.mark).collect();
        assert_eq!(marks, [Done, Done, Warn, Done, Done, Done]);
        // A run that never passed: a reply that could not be read, and
        // checks with one problem and with several.
        a.plan_timeline = steps_ago(
            now,
            vec![
                (Analyzing, 80, Some(79), None),
                (
                    Asking(1),
                    79,
                    Some(50),
                    Some(StepResult::Unreadable("not valid JSON".into())),
                ),
                (Repairing(2), 50, Some(30), None),
                (Checking(2), 30, Some(20), Some(one)),
                (Repairing(3), 20, Some(5), None),
                (
                    Checking(3),
                    5,
                    Some(1),
                    Some(StepResult::Problems {
                        shown: vec![],
                        more: 6,
                    }),
                ),
            ],
        );
        let rows = crate::ui::timeline_rows(&a, now, true);
        let text: Vec<_> = rows.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(
            text,
            [
                "Analyzed the part · 1 s",
                "Asked gpt-6.1-sol for a plan (1 of 4) · 29 s · reply could not be read",
                "Asked gpt-6.1-sol for a correction (2 of 4) · 20 s",
                "Checked the plan (2 of 4) · 10 s · 1 problem",
                "Asked gpt-6.1-sol for a correction (3 of 4) · 15 s",
                "Checked the plan (3 of 4) · 4 s · 6 problems",
            ]
        );
        assert!(
            rows.iter()
                .all(|r| r.notes.is_empty() && !r.text.contains(['…', '\n']))
        );
        let marks: Vec<_> = rows.iter().map(|r| r.mark).collect();
        assert_eq!(marks, [Done, Warn, Done, Warn, Done, Warn]);
        // Both finished views build: above a failure, and above a preview.
        a.page = Page::Ai;
        let failure = crate::planner::Failure {
            message: "No plan.".into(),
            problems: vec![],
            api: None,
            requests: 3,
        };
        a.planner = crate::Planner::Failed(failure);
        let _ = a.view();
        a.planner = crate::Planner::Preview(checked_preview(&a, false));
        let _ = a.view();
    }
    #[test]
    fn a_new_request_starts_its_row_at_zero_characters() {
        use crate::planner::{Event::*, Progress::*};
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let count = |a: &App| a.plan_received.load(std::sync::atomic::Ordering::Relaxed);
        let stream = |a: &App, n| {
            a.plan_received
                .store(n, std::sync::atomic::Ordering::Relaxed)
        };
        send(&mut a, epoch, Stage(Analyzing));
        send(&mut a, epoch, Stage(Asking(1)));
        stream(&a, 1200);
        // A check does not touch the count; the next request starts again.
        send(&mut a, epoch, Stage(Checking(1)));
        assert_eq!(count(&a), 1200);
        for request in [Repairing(2), Extending(3), Asking(4)] {
            send(&mut a, epoch, Stage(request));
            assert_eq!(count(&a), 0, "{request:?}");
            let rows = crate::ui::timeline_rows(&a, std::time::Instant::now(), false);
            assert_eq!(rows.last().unwrap().notes, ["received 0 characters"]);
            stream(&a, 900);
            send(&mut a, epoch, Stage(Checking(2)));
        }
        // An event of an older run leaves the count alone.
        send(&mut a, epoch.wrapping_sub(1), Stage(Repairing(2)));
        assert_eq!(count(&a), 900);
    }
    #[test]
    fn applying_a_plan_replaces_the_job_and_undo_restores_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let before = serde_json::to_string(&a.doc).unwrap();
        let first = a.doc.setups[0].id.clone();
        let _ = a.update(Message::ApplyPlan);
        assert_eq!(
            serde_json::to_string(&a.doc).unwrap(),
            before,
            "nothing to apply yet"
        );
        let preview = checked_preview(&a, false);
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let _ = a.update(Message::PlanFinished(epoch, Ok(preview)));
        assert_eq!(
            serde_json::to_string(&a.doc).unwrap(),
            before,
            "a preview changes nothing"
        );
        let _ = a.update(Message::ApplyPlan);
        assert_eq!(a.doc.setups.len(), 2);
        assert_eq!(a.doc.setups[0].id, first);
        assert_eq!(a.doc.active, first);
        assert_eq!(a.doc.setups[0].operations.len(), 4);
        assert_eq!(get(&a.doc.setups[1].p, "offset"), "G55");
        assert_eq!(a.page, Page::Work);
        assert!(matches!(a.planner, crate::Planner::Idle));
        assert!(a.undo.is_some() && a.dirty);
        assert!(a.doc.validate_structure().is_ok());
        let _ = a.update(Message::UndoPlan);
        assert_eq!(serde_json::to_string(&a.doc).unwrap(), before);
        assert!(a.undo.is_none());
    }
    /// Plan, show the checked preview and apply it.
    fn apply_checked_plan(a: &mut App) {
        let preview = checked_preview(a, false);
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let _ = a.update(Message::PlanFinished(epoch, Ok(preview)));
        assert!(matches!(a.planner, crate::Planner::Preview(_)));
        let _ = a.update(Message::ApplyPlan);
        assert!(a.undo.is_some());
    }
    fn json<T: serde::Serialize>(value: &T) -> String {
        serde_json::to_string(value).unwrap()
    }
    /// A closed box mesh, `size` in mm.
    fn box_mesh(size: [f32; 3]) -> crate::mesh::Mesh {
        let v = |i: usize| -> [f32; 3] {
            std::array::from_fn(|k| if i >> k & 1 == 0 { 0. } else { size[k] })
        };
        let mut triangles = vec![];
        for q in [
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ] {
            triangles.push([v(q[0]), v(q[1]), v(q[2])]);
            triangles.push([v(q[0]), v(q[2]), v(q[3])]);
        }
        crate::mesh::normalize(triangles).unwrap()
    }
    #[test]
    fn apply_keeps_the_previous_job_in_history_and_undo_restores_only_its_setups() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let before = a.doc.clone();
        apply_checked_plan(&mut a);
        // The copy is in History; Recover previous still means the project
        // before the last switch (there was none here).
        let history: Vec<_> = std::fs::read_dir(dir.path().join("history"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(history.len(), 1);
        let archived = crate::storage::load(&history[0]).unwrap();
        assert_eq!(json(&archived.setups), json(&before.setups));
        assert!(a.last_archived.is_none());
        assert!(a.status.contains("History"), "{}", a.status);
        // Work done after Apply: a tool, a machine, the title and the material.
        let _ = a.update(Message::NewTool);
        let _ = a.update(Message::Edit(Target::Tool, "name", "Spot drill".into()));
        let _ = a.update(Message::SaveTool);
        let _ = a.update(Message::NewMachine);
        let _ = a.update(Message::SaveMachine);
        let _ = a.update(Message::Title("Renamed after apply".into()));
        let brass = a
            .doc
            .materials
            .iter()
            .find(|m| m.name() == "Brass C360")
            .unwrap()
            .id
            .clone();
        let _ = a.update(Message::PickMaterial(brass));
        let (tools, machines) = (json(&a.doc.tools), json(&a.doc.machines));
        assert_eq!(a.doc.tools.len(), before.tools.len() + 1);
        assert_eq!(a.doc.machines.len(), before.machines.len() + 1);
        let applied = json(&a.doc);
        let _ = a.update(Message::UndoPlan);
        // Undo first keeps the applied job, with the edits since, in History.
        let history: Vec<String> = std::fs::read_dir(dir.path().join("history"))
            .unwrap()
            .map(|entry| json(&crate::storage::load(&entry.unwrap().path()).unwrap()))
            .collect();
        assert_eq!(history.len(), 2);
        assert!(history.contains(&applied), "the applied job is in History");
        assert!(
            a.status.starts_with(
                "The setups from before the plan are restored. The applied job is in History."
            ),
            "{}",
            a.status
        );
        assert_eq!(json(&a.doc.setups), json(&before.setups));
        assert_eq!(a.doc.active, before.active);
        assert_eq!(json(&a.doc.tools), tools);
        assert_eq!(json(&a.doc.machines), machines);
        assert_eq!(a.doc.title, "Renamed after apply");
        assert_eq!(a.doc.material, "Brass C360");
        assert!(a.undo.is_none());
        assert!(a.doc.validate_structure().is_ok());
    }
    /// Plans again with setup 1's workholding note set to `note`, and applies.
    /// `base` is the preview checked once for `a`'s job. A built job's setup 1
    /// is a copy of the user's with the plan's operations, so it is copied
    /// from `a` again here (Apply reads its notes) instead of checking the
    /// same plan again; only setup 1's notes may have changed since.
    fn apply_with_setup_1_note(a: &mut App, base: &crate::PlanPreview, note: &str) {
        let mut outcome = base.outcome.clone();
        let built = &mut outcome.job.setups[0];
        let mut setup = a.doc.setups[0].clone();
        let without_notes = |p: &crate::model::Fields| {
            let mut p = p.clone();
            p.remove("notes");
            p.remove(crate::planner::PLANNER_NOTE_KEY);
            p
        };
        assert_eq!(without_notes(&setup.p), without_notes(&built.p));
        assert_eq!(
            (&setup.id, json(&setup.machine)),
            (&built.id, json(&built.machine))
        );
        setup.operations = std::mem::take(&mut built.operations);
        *built = setup;
        outcome.proposal.setups[0].workholding = note.into();
        let preview = crate::PlanPreview {
            outcome,
            view: None,
        };
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let _ = a.update(Message::PlanFinished(
            epoch,
            Ok(std::sync::Arc::new(preview)),
        ));
        let _ = a.update(Message::ApplyPlan);
        assert!(a.undo.is_some(), "{}", a.status);
    }
    #[test]
    fn apply_adds_the_planners_setup_1_note_on_one_line_and_undo_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        a.doc.setups[0]
            .p
            .insert("notes".into(), "Soft jaws.".into());
        let before = json(&a.doc.setups);
        // The plan is checked once per job; every Apply below reuses it.
        let base = checked_preview(&a, false);
        let vise = "Vise on the long sides.";
        assert_eq!(base.outcome.proposal.setups[0].workholding, vise);
        apply_with_setup_1_note(&mut a, &base, vise);
        assert_eq!(
            get(&a.doc.setups[0].p, "notes"),
            "Soft jaws. · Planner: Vise on the long sides."
        );
        // Setup 1 keeps the note Apply added, saved with the project.
        assert_eq!(
            get(&a.doc.setups[0].p, "planner_note"),
            "Vise on the long sides."
        );
        let saved = dir.path().join("planner-note.axis.json");
        crate::storage::save(&saved, &a.doc).unwrap();
        let loaded = crate::storage::load(&saved).unwrap();
        assert_eq!(
            get(&loaded.setups[0].p, "planner_note"),
            "Vise on the long sides."
        );
        let _ = a.update(Message::UndoPlan);
        assert_eq!(json(&a.doc.setups), before);
        assert!(!a.doc.setups[0].p.contains_key("planner_note"));
        // A later Apply replaces the part an earlier Apply added, and the
        // planner's line breaks become spaces.
        apply_with_setup_1_note(&mut a, &base, vise);
        apply_with_setup_1_note(
            &mut a,
            &base,
            "Clamp the\nlong sides.\r\n\nKeep the top clear.",
        );
        let notes = get(&a.doc.setups[0].p, "notes");
        assert_eq!(
            notes,
            "Soft jaws. · Planner: Clamp the long sides. Keep the top clear."
        );
        // Without notes of its own, setup 1 gets the planner's note alone,
        // within 500 characters, and a later Apply replaces it too.
        let mut a = signed_in_planning_app(&dir);
        let base = checked_preview(&a, false);
        apply_with_setup_1_note(&mut a, &base, &"w".repeat(900));
        let notes = get(&a.doc.setups[0].p, "notes");
        assert!(notes.starts_with("Planner: www"), "{notes}");
        assert_eq!(notes.chars().count(), 500);
        apply_with_setup_1_note(&mut a, &base, "Vise.");
        assert_eq!(get(&a.doc.setups[0].p, "notes"), "Planner: Vise.");
        // Text the user typed after the planner's note survives a second
        // Apply; only the note an earlier Apply added is replaced.
        a.doc.setups[0].p.insert(
            "notes".into(),
            "Soft jaws. · Planner: Vise. Torque 40 Nm.".into(),
        );
        apply_with_setup_1_note(&mut a, &base, "Clamps.");
        assert_eq!(
            get(&a.doc.setups[0].p, "notes"),
            "Soft jaws. Torque 40 Nm. · Planner: Clamps."
        );
        assert_eq!(get(&a.doc.setups[0].p, "planner_note"), "Clamps.");
        // Notes the user starts with "Planner: " survive too.
        a.doc.setups[0]
            .p
            .insert("notes".into(), "Planner: Alice checks jaws.".into());
        apply_with_setup_1_note(&mut a, &base, "Vise.");
        assert_eq!(
            get(&a.doc.setups[0].p, "notes"),
            "Planner: Alice checks jaws. · Planner: Vise."
        );
        // A plan without a note for setup 1 removes its earlier one and
        // keeps nothing to remove later.
        apply_with_setup_1_note(&mut a, &base, "");
        assert_eq!(
            get(&a.doc.setups[0].p, "notes"),
            "Planner: Alice checks jaws."
        );
        assert!(!a.doc.setups[0].p.contains_key("planner_note"));
        // The card says where the note goes.
        assert_eq!(
            crate::ui::workholding_line(0, "Vise."),
            "Workholding (from the planner; on Apply it goes at the end of setup 1's notes, replacing an earlier planner note if it is unchanged): Vise."
        );
        assert_eq!(
            crate::ui::workholding_line(1, "Flip."),
            "Workholding (from the planner): Flip."
        );
    }
    #[test]
    fn apply_changes_nothing_when_the_job_cannot_be_kept_in_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        // A file where the history folder belongs makes archiving fail.
        std::fs::write(dir.path().join("history"), "").unwrap();
        let before = json(&a.doc);
        let preview = checked_preview(&a, false);
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let _ = a.update(Message::PlanFinished(epoch, Ok(preview)));
        let _ = a.update(Message::ApplyPlan);
        assert_eq!(json(&a.doc), before);
        assert!(a.undo.is_none());
        assert!(matches!(a.planner, crate::Planner::Preview(_)));
        assert!(
            a.status.starts_with("Could not preserve your project"),
            "{}",
            a.status
        );
    }
    #[test]
    fn undo_brings_back_a_tool_deleted_after_apply() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let ball = a.doc.tools[1].clone();
        assert_eq!((ball.name(), ball.number()), ("Ball 6", Some(2)));
        // Two operations of the job use the ball mill; it comes back once.
        let mut second = Operation::new(Strategy::Parallel, ball.id.clone());
        second.p.insert("surface_stepover".into(), "2".into());
        a.doc.setups[0].operations.push(second);
        let uses = |a: &App| {
            a.doc.setups[0]
                .operations
                .iter()
                .filter(|o| o.tool == ball.id)
                .count()
        };
        assert_eq!(uses(&a), 2);
        apply_checked_plan(&mut a);
        // The plan does not use the ball mill, so it can be deleted now.
        let _ = a.update(Message::DeleteTool(ball.id.clone()));
        assert!(!a.doc.tools.iter().any(|t| t.id == ball.id), "{}", a.status);
        let _ = a.update(Message::UndoPlan);
        assert_eq!(
            a.doc.tools.iter().filter(|t| t.id == ball.id).count(),
            1,
            "the ball mill is back once"
        );
        assert_eq!(uses(&a), 2, "both restored operations use it");
        assert!(a.doc.validate_structure().is_ok());
        assert!(
            a.status.ends_with(" Ball 6 is back in the library as T2."),
            "{}",
            a.status
        );
        assert_eq!(a.status.matches("Ball 6").count(), 1, "{}", a.status);
        // A new tool took T2 after the delete: the ball mill comes back as
        // the next free number, and the status names it.
        let mut a = signed_in_planning_app(&dir);
        let ball = a.doc.tools[1].clone();
        apply_checked_plan(&mut a);
        let _ = a.update(Message::DeleteTool(ball.id.clone()));
        let _ = a.update(Message::NewTool);
        let _ = a.update(Message::SaveTool);
        assert!(
            a.doc
                .tools
                .iter()
                .any(|t| t.number() == Some(2) && t.id != ball.id)
        );
        let _ = a.update(Message::UndoPlan);
        let back = a.doc.tools.iter().find(|t| t.id == ball.id).unwrap();
        assert_eq!(back.number(), Some(4));
        assert!(
            a.status.ends_with(" Ball 6 is back in the library as T4."),
            "{}",
            a.status
        );
        for tool in &a.doc.tools {
            assert_eq!(a.doc.tool_number_issues(tool), Vec::<String>::new());
        }
    }
    #[test]
    fn undo_keeps_every_free_tool_number_and_renumbers_only_taken_ones() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        // A fourth tool, used after the ball mill's Parallel in the job.
        let mut flat = Tool::new("Flat 4", "Flat end mill", "4");
        flat.p.insert("number".into(), "4".into());
        a.doc.tools.push(flat.clone());
        a.doc.setups[0]
            .operations
            .push(Operation::new(Strategy::Contour, flat.id.clone()));
        let ball = a.doc.tools[1].clone();
        assert_eq!(ball.number(), Some(2));
        apply_checked_plan(&mut a);
        // Both are deleted, and a new tool takes T2, the ball mill's number.
        for id in [&ball.id, &flat.id] {
            let _ = a.update(Message::DeleteTool(id.clone()));
            assert!(!a.doc.tools.iter().any(|t| &t.id == id), "{}", a.status);
        }
        let _ = a.update(Message::NewTool);
        let _ = a.update(Message::SaveTool);
        let numbers =
            |a: &App| -> Vec<Option<u32>> { a.doc.tools.iter().map(|t| t.number()).collect() };
        assert_eq!(numbers(&a), [Some(1), Some(3), Some(2)]);
        let _ = a.update(Message::UndoPlan);
        // Flat 4 keeps T4, which is free. The ball mill's T2 is taken, so it
        // gets a number that is neither used nor kept: T5, not T4.
        let number = |id: &str| a.doc.tools.iter().find(|t| t.id == id).unwrap().number();
        assert_eq!(number(&flat.id), Some(4));
        assert_eq!(number(&ball.id), Some(5));
        assert!(
            a.status.ends_with(
                " Ball 6 is back in the library as T5. Flat 4 is back in the library as T4."
            ),
            "{}",
            a.status
        );
        for tool in &a.doc.tools {
            assert_eq!(a.doc.tool_number_issues(tool), Vec::<String>::new());
        }
    }
    #[test]
    fn undo_changes_nothing_when_the_applied_job_cannot_be_kept_in_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        apply_checked_plan(&mut a);
        // A file where the history folder belongs makes archiving fail.
        std::fs::remove_dir_all(dir.path().join("history")).unwrap();
        std::fs::write(dir.path().join("history"), "").unwrap();
        let applied = json(&a.doc);
        let _ = a.update(Message::UndoPlan);
        assert!(json(&a.doc) == applied, "the job is unchanged");
        assert!(a.undo.is_some(), "Undo is still offered");
        assert!(
            a.status.starts_with("Could not preserve your project"),
            "{}",
            a.status
        );
    }
    #[test]
    fn a_new_part_drops_the_undo_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        apply_checked_plan(&mut a);
        let file = ModelFile {
            name: "other.stl".into(),
            path: dir.path().join("other.stl").to_string_lossy().into_owned(),
            format: "stl".into(),
            bytes: 1,
        };
        let other = box_mesh([30., 20., 10.]);
        let _ = a.update(Message::FileImported(Some(Ok((file, Some(other))))));
        assert!(a.undo.is_none());
        let after = json(&a.doc);
        let _ = a.update(Message::UndoPlan);
        assert_eq!(json(&a.doc), after, "Undo changes nothing for the new part");
        // Restoring the model also drops it.
        let mut a = signed_in_planning_app(&dir);
        apply_checked_plan(&mut a);
        let mesh = a.imported_mesh.clone().unwrap();
        let _ = a.update(Message::ModelRestored(Ok(mesh)));
        assert!(a.undo.is_none());
    }
    #[test]
    fn new_project_and_open_drop_the_preview_the_run_and_the_undo() {
        let dir = tempfile::tempdir().unwrap();
        let other = || {
            (
                Document::default(),
                dir.path().join("project-b.json"),
                vec![],
            )
        };
        for open in [false, true] {
            let switch = |a: &mut App| {
                let message = if open {
                    Message::ProjectOpened(Some(Ok(other())))
                } else {
                    Message::NewProject
                };
                let _ = a.update(message);
            };
            // A preview of project A cannot be applied to project B.
            let mut a = signed_in_planning_app(&dir);
            let preview = checked_preview(&a, false);
            let _ = a.update(Message::Plan);
            let epoch = a.plan_epoch;
            let _ = a.update(Message::PlanFinished(epoch, Ok(preview.clone())));
            switch(&mut a);
            assert!(matches!(a.planner, crate::Planner::Idle));
            let b = json(&a.doc);
            let _ = a.update(Message::ApplyPlan);
            assert_eq!(json(&a.doc), b);
            // A running plan stops, and its late result is ignored.
            let mut a = signed_in_planning_app(&dir);
            let _ = a.update(Message::Plan);
            let epoch = a.plan_epoch;
            switch(&mut a);
            assert!(a.plan_run.is_none() && matches!(a.planner, crate::Planner::Idle));
            let _ = a.update(Message::PlanFinished(epoch, Ok(preview)));
            assert!(matches!(a.planner, crate::Planner::Idle));
            // Undo of project A is gone.
            let mut a = signed_in_planning_app(&dir);
            apply_checked_plan(&mut a);
            switch(&mut a);
            assert!(a.undo.is_none());
            let b = json(&a.doc);
            let _ = a.update(Message::UndoPlan);
            assert_eq!(json(&a.doc), b);
        }
    }
    #[test]
    fn cancel_discard_edits_and_sign_out_drop_the_plan() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let before = json(&a.doc);
        let preview = checked_preview(&a, false);
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let _ = a.update(Message::PlanCancel);
        assert!(matches!(a.planner, crate::Planner::Idle));
        assert!(a.plan_run.is_none());
        assert_eq!(a.status, "Planning cancelled. The job is unchanged.");
        let _ = a.update(Message::PlanFinished(epoch, Ok(preview.clone())));
        assert!(
            matches!(a.planner, crate::Planner::Idle),
            "a cancelled run's result is ignored"
        );
        assert_eq!(json(&a.doc), before, "Cancel leaves the job unchanged");
        // Discard
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let _ = a.update(Message::PlanFinished(epoch, Ok(preview.clone())));
        let _ = a.update(Message::DiscardPlan);
        assert!(matches!(a.planner, crate::Planner::Idle));
        assert_eq!(json(&a.doc), before, "Discard leaves the job unchanged");
        // An edit makes a preview stale.
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let _ = a.update(Message::PlanFinished(epoch, Ok(preview)));
        let _ = a.update(Message::Edit(Target::Setup, "stock_top", "6".into()));
        assert!(matches!(a.planner, crate::Planner::Idle));
        let _ = a.update(Message::ApplyPlan);
        assert_eq!(a.doc.setups.len(), 1);
        // Signing out stops a running plan.
        let _ = a.update(Message::Plan);
        assert!(matches!(a.planner, crate::Planner::Running(_)));
        let edited = json(&a.doc);
        let _ = a.update(Message::SignOut);
        assert!(matches!(a.planner, crate::Planner::Idle));
        assert_eq!(a.status, "Planning stopped because you are signed out.");
        assert_eq!(json(&a.doc), edited, "Sign out leaves the job unchanged");
    }
    /// The open box imported and signed in: one Face and Contour plan covers it.
    fn signed_in_box_app(dir: &tempfile::TempDir) -> (App, crate::analysis::PartReport) {
        let (doc, mesh, report) = crate::planner::tests::open_box();
        let mut a = fresh_app(dir);
        a.replace_document(doc, None);
        a.imported_mesh = Some(mesh);
        sign_in_with(&mut a, dir, crate::session::sample());
        let epoch = a.account_epoch;
        let _ = a.update(Message::ModelsLoaded(epoch, Ok(vec!["gpt-6.1-sol".into()])));
        (a, report)
    }
    /// Runs the Plan task body with `script` and collects what it sends.
    async fn run_task(a: &App, script: crate::planner::tests::Script) -> Vec<Message> {
        use iced::futures::StreamExt;
        let (sender, receiver) = iced::futures::channel::mpsc::channel(16);
        let mesh = a.imported_mesh.clone().unwrap();
        let busy = a.plan_busy.clone();
        crate::run_plan(&script, a.doc.clone(), mesh, busy, a.plan_epoch, sender).await;
        receiver.collect().await
    }
    #[tokio::test]
    async fn the_plan_task_sends_its_stages_and_its_result_with_the_runs_epoch() {
        use crate::planner::{Event, Progress, StepResult, tests::Script};
        let dir = tempfile::tempdir().unwrap();
        let (mut a, report) = signed_in_box_app(&dir);
        let _ = a.update(Message::Plan);
        assert!(
            matches!(a.planner, crate::Planner::Running(_)),
            "{}",
            a.status
        );
        let epoch = a.plan_epoch;
        let plan = crate::planner::tests::complete(&report);
        let script = Script::new(vec![Ok(serde_json::to_string(&plan).unwrap())]);
        let messages = run_task(&a, script).await;
        let mut events = vec![];
        for message in &messages[..messages.len() - 1] {
            let Message::PlanProgress(e, event) = message else {
                panic!("{message:?}")
            };
            assert_eq!(*e, epoch);
            events.push(event.clone());
        }
        assert_eq!(
            events,
            [
                Event::Stage(Progress::Analyzing),
                Event::Stage(Progress::Asking(1)),
                Event::Stage(Progress::Checking(1)),
                Event::Checked(1, StepResult::Passed),
                Event::Stage(Progress::Previewing)
            ]
        );
        let Some(Message::PlanFinished(e, Ok(preview))) = messages.last() else {
            panic!("{:?}", messages.last())
        };
        assert_eq!(*e, epoch);
        assert!(preview.view.is_some(), "the 3D view is built");
        assert_eq!(preview.outcome.requests, 1);
        for message in messages {
            let _ = a.update(message);
        }
        assert!(matches!(a.planner, crate::Planner::Preview(_)));
        assert_eq!(a.status, "Plan ready to review: 1 setup, 2 operations.");
        assert!(a.plan_run.is_none());
        assert_eq!(a.plan_busy.running(), 0);
        // The timeline of the run stays with its preview, every step ended.
        let kept: Vec<_> = a
            .plan_timeline
            .iter()
            .map(|s| (s.stage, s.ended.is_some(), s.result.clone()))
            .collect();
        assert_eq!(
            kept,
            [
                (Progress::Analyzing, true, None),
                (Progress::Asking(1), true, None),
                (Progress::Checking(1), true, Some(StepResult::Passed)),
                (Progress::Previewing, true, None),
            ]
        );
        // A run that fails ends with PlanFinished(Err), with the same epoch.
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let script = Script::new(vec![Err(crate::openai::ApiError::UsageLimit)]);
        let messages = run_task(&a, script).await;
        assert!(matches!(
            messages.last(),
            Some(Message::PlanFinished(e, Err(f))) if *e == epoch
                && f.api == Some(crate::openai::ApiError::UsageLimit)
        ));
        assert!(!messages.iter().any(|m| matches!(
            m,
            Message::PlanProgress(_, Event::Stage(Progress::Previewing))
        )));
        for message in messages {
            let _ = a.update(message);
        }
        assert!(matches!(a.planner, crate::Planner::Failed(_)) && a.usage_limit);
    }
    #[tokio::test]
    async fn a_dropped_plan_task_counts_its_3d_view_until_it_ends() {
        use crate::planner::tests::{Gate, Script, complete, drop_while_held};
        let dir = tempfile::tempdir().unwrap();
        let (a, report) = signed_in_box_app(&dir);
        let gate = Gate::default();
        let held = gate.clone();
        let script = Script::new(vec![Ok(serde_json::to_string(&complete(&report)).unwrap())]);
        let (sender, _receiver) = iced::futures::channel::mpsc::channel(16);
        let busy = a.plan_busy.clone();
        let run = crate::run_plan_with(
            &script,
            a.doc.clone(),
            a.imported_mesh.clone().unwrap(),
            busy.clone(),
            a.plan_epoch,
            sender,
            move |_: &Document, _: &crate::mesh::Mesh| {
                held.hold();
                None
            },
        );
        drop_while_held(run, &gate, &busy).await;
    }
    #[test]
    fn cancel_and_edits_abort_the_task_and_a_late_result_is_not_the_next_runs() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let preview = checked_preview(&a, false);
        let stop: [fn(&mut App); 2] = [
            |a| {
                let _ = a.update(Message::PlanCancel);
            },
            |a| {
                let _ = a.update(Message::Edit(Target::Setup, "stock_top", "6".into()));
            },
        ];
        for stop in stop {
            let _ = a.update(Message::Plan);
            let first = a.plan_epoch;
            let handle = a.plan_run.clone().expect("the run's handle");
            assert!(!handle.is_aborted());
            stop(&mut a);
            assert!(handle.is_aborted(), "the task is aborted");
            assert!(a.plan_run.is_none());
            assert_ne!(a.plan_epoch, first, "the epoch moves on");
            // The next run is not fooled by the first run's late messages.
            let _ = a.update(Message::Plan);
            assert!(matches!(a.planner, crate::Planner::Running(_)));
            let asking = crate::planner::Event::Stage(crate::planner::Progress::Asking(3));
            let _ = a.update(Message::PlanProgress(first, asking));
            let _ = a.update(Message::PlanFinished(first, Ok(preview.clone())));
            assert!(matches!(
                a.planner,
                crate::Planner::Running(crate::planner::Progress::Analyzing)
            ));
            let _ = a.update(Message::PlanCancel);
        }
    }
    #[test]
    fn apply_from_a_later_active_setup_removes_it_and_activates_setup_1() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let first = a.doc.setups[0].id.clone();
        let _ = a.update(Message::DuplicateSetup);
        let _ = a.update(Message::Flip("rx"));
        let later = a.doc.setups[1].id.clone();
        assert_eq!((a.doc.setups.len(), &a.doc.active), (2, &later));
        let preview = checked_preview(&a, false);
        let planned: Vec<String> = preview
            .outcome
            .job
            .setups
            .iter()
            .map(|s| s.id.clone())
            .collect();
        assert_eq!(planned[0], first);
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let _ = a.update(Message::PlanFinished(epoch, Ok(preview)));
        let _ = a.update(Message::ApplyPlan);
        let ids: Vec<String> = a.doc.setups.iter().map(|s| s.id.clone()).collect();
        assert_eq!(ids, planned);
        assert!(!ids.contains(&later), "the later setup is gone");
        assert_eq!(a.doc.active, first);
        assert!(a.doc.validate_structure().is_ok());
        // Undo puts the later setup back, still active.
        let _ = a.update(Message::UndoPlan);
        assert_eq!(a.doc.setups.len(), 2);
        assert_eq!(a.doc.active, later);
    }
    #[test]
    fn a_failed_run_shows_its_problems_and_account_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let failure = |api: Option<crate::openai::ApiError>| crate::planner::Failure {
            message: api
                .as_ref()
                .map_or("No plan passed.".into(), |e| e.message()),
            problems: vec![],
            api,
            requests: 1,
        };
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let limit = failure(Some(crate::openai::ApiError::UsageLimit));
        let _ = a.update(Message::PlanFinished(epoch, Err(limit)));
        assert!(matches!(a.planner, crate::Planner::Failed(_)));
        assert!(a.usage_limit);
        assert!(a.status.contains("Usage limit reached"));
        let _ = a.update(Message::Plan);
        assert!(
            matches!(a.planner, crate::Planner::Running(_)),
            "Try again starts a new run"
        );
        let epoch = a.plan_epoch;
        let gone = failure(Some(crate::openai::ApiError::SignedOut(
            "Sign in again.".into(),
        )));
        let _ = a.update(Message::PlanFinished(epoch, Err(gone)));
        assert_eq!(a.account, crate::Account::SignedOut);
        assert!(matches!(a.planner, crate::Planner::Failed(_)));
        let _ = a.update(Message::Plan);
        assert!(
            matches!(a.planner, crate::Planner::Failed(_)),
            "not signed in: no new run"
        );
        let _ = a.update(Message::DiscardPlan);
        assert!(matches!(a.planner, crate::Planner::Idle));
        // Every API error reaches the failed panel, the status and the account.
        use crate::openai::ApiError;
        for error in [
            ApiError::SignedOut("Sign in again.".into()),
            ApiError::UsageLimit,
            ApiError::NotEligible("Upgrade your plan.".into()),
            ApiError::Unavailable("OpenAI is unavailable.".into()),
            ApiError::Refused("No.".into()),
            ApiError::Unsupported("Not on this plan.".into()),
            ApiError::Other("Something else.".into()),
        ] {
            let mut a = signed_in_planning_app(&dir);
            let _ = a.update(Message::Plan);
            let epoch = a.plan_epoch;
            let _ = a.update(Message::PlanFinished(
                epoch,
                Err(failure(Some(error.clone()))),
            ));
            let crate::Planner::Failed(shown) = &a.planner else {
                panic!("{error:?}: no failed panel")
            };
            assert_eq!(shown.message, error.message());
            assert_eq!(a.status, error.message());
            assert_eq!(a.account_note, error.message());
            assert_eq!(a.usage_limit, error == ApiError::UsageLimit);
            let signed_out = matches!(error, ApiError::SignedOut(_));
            assert_eq!(a.account == crate::Account::SignedOut, signed_out);
        }
        // A failed check's problems are listed on the failed panel.
        let mut a = signed_in_planning_app(&dir);
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let problem = crate::proposal::Problem {
            setup: 1,
            operation: Some(2),
            stage: "generation",
            message: "Pocket depth must be less than tool stickout.".into(),
        };
        let mut no_plan = failure(None);
        no_plan.problems = vec![problem];
        let _ = a.update(Message::PlanFinished(epoch, Err(no_plan)));
        let crate::Planner::Failed(shown) = &a.planner else {
            panic!("no failed panel")
        };
        assert_eq!(
            crate::ui::problem_lines(&shown.problems),
            ["• Setup 1, operation 2: Pocket depth must be less than tool stickout."]
        );
        let _ = a.update(Message::ApplyPlan);
        assert_eq!(a.doc.setups.len(), 1, "a failed run has nothing to apply");
    }
    #[test]
    fn a_dead_clients_run_never_signs_out_a_new_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let _ = a.update(Message::Plan);
        let first_run = a.plan_epoch;
        assert!(matches!(a.planner, crate::Planner::Running(_)));
        // A connection check reports that the sign-in is gone: the run,
        // which uses that client, stops as it does on Sign out.
        let _ = a.update(Message::CheckConnection);
        let gone = || crate::openai::ApiError::SignedOut("Sign in again.".into());
        let _ = a.update(Message::ConnectionChecked(a.account_epoch, Err(gone())));
        assert_eq!(a.account, crate::Account::SignedOut);
        assert!(matches!(a.planner, crate::Planner::Idle));
        assert_eq!(a.status, "Planning stopped because you are signed out.");
        assert!(a.plan_run.is_none());
        assert_ne!(a.plan_epoch, first_run);
        // The user signs in again; the old run's failure arrives late.
        sign_in_with(&mut a, &dir, crate::session::sample());
        let _ = a.update(Message::ModelsLoaded(
            a.account_epoch,
            Ok(vec!["gpt-6.1-sol".into()]),
        ));
        let before = snapshot(&a);
        let failure = crate::planner::Failure {
            message: "Sign in with ChatGPT to continue.".into(),
            problems: vec![],
            api: Some(gone()),
            requests: 2,
        };
        let _ = a.update(Message::PlanFinished(first_run, Err(failure)));
        assert_eq!(snapshot(&a), before, "the new sign-in is kept");
        assert!(matches!(
            a.account,
            crate::Account::SignedIn { granted: true, .. }
        ));
        assert!(matches!(a.planner, crate::Planner::Idle));
    }
    #[tokio::test]
    async fn a_refusal_reaches_the_status_the_account_and_the_page_clean_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let hostile = format!("No\nway \u{202e}\u{7}{}", "x".repeat(5000));
        let script = crate::planner::tests::Script::new(vec![Err(
            crate::openai::ApiError::Refused(hostile),
        )]);
        let (doc, mesh) = (a.doc.clone(), a.imported_mesh.clone().unwrap());
        let busy = crate::planner::Busy::default();
        let failure = crate::planner::plan(&script, doc, mesh, &busy, |_| {})
            .await
            .unwrap_err();
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let _ = a.update(Message::PlanFinished(epoch, Err(failure)));
        let crate::Planner::Failed(shown) = &a.planner else {
            panic!("no failure shown")
        };
        for text in [&a.status, &a.account_note, &shown.message] {
            assert!(text.starts_with("The model declined the request: No way "));
            assert!(!text.contains(['\n', '\u{202e}', '\u{7}']), "{text}");
            assert!(text.chars().count() <= 340);
        }
        a.page = Page::Ai;
        let _ = a.view();
    }
    #[test]
    fn a_failed_run_lists_twelve_problems_and_how_many_more() {
        let problems: Vec<crate::proposal::Problem> = (0..15)
            .map(|i| crate::proposal::Problem {
                setup: 1,
                operation: Some(i + 1),
                stage: "structure",
                message: format!("problem {i}"),
            })
            .collect();
        let lines = crate::ui::problem_lines(&problems);
        assert_eq!(lines.len(), 13);
        assert_eq!(lines[0], "• Setup 1, operation 1: problem 0");
        assert_eq!(lines[12], "• and 3 more problems");
        assert_eq!(crate::ui::problem_lines(&problems[..2]).len(), 2);
        assert_eq!(
            crate::ui::problem_lines(&problems[..13])[12],
            "• and 1 more problem"
        );
    }
    #[test]
    fn preview_rows_show_the_strategy_and_the_values_that_will_run() {
        use crate::proposal::tests::{block, good, op};
        let (doc, mesh, report) = block();
        let mut proposal = good(&report);
        {
            let ops = &mut proposal.setups[0].operations;
            ops[0].bottom_offset = Some(0.5);
            let pocket = &mut ops[1];
            pocket.rpm = 8000.5;
            pocket.stepover_percent = Some(35.);
            pocket.axial_allowance = Some(0.1);
            pocket.ramp_angle = Some(3.);
            pocket.finish_passes = Some(2);
            ops[2].radial_allowance = Some(0.25);
            ops[3].hole_diameter = Some(6.6);
            ops[3].through = Some(true);
            ops[3].breakthrough = Some(1.5);
            let mut parallel = op("Parallel", 2, vec![]);
            parallel.surface_stepover = Some(0.8);
            parallel.tolerance = Some(0.02);
            let mut waterline = op("Waterline", 1, vec![]);
            waterline.stepdown = Some(3.);
            ops.extend([parallel, waterline]);
        }
        let job = crate::proposal::build(&doc, &mesh, &report, &proposal).unwrap();
        let ops = &job.setups[0].operations;
        let lines = |i: usize| crate::ui::operation_lines(&ops[i], "T1 Flat 10", 1.3, "Because.");
        assert_eq!(
            lines(0),
            [
                "Facing · “Face by planner” · T1 Flat 10 · 1.3 min",
                "8000 rpm · 1000 mm/min · coolant Flood · facing area stock silhouette · stepdown 2 mm · stepover 40% · bottom offset 0.5 mm",
                "The planner's reason: Because.",
            ]
        );
        assert_eq!(
            lines(1)[1],
            "8001 rpm · 1000 mm/min · coolant Flood · stepdown 2 mm · stepover 35% · axial allowance 0.1 mm · entry Profile ramp at 3° · 2 finish passes"
        );
        assert_eq!(
            lines(1)[0],
            "Pocket · “Pocket by planner” · T1 Flat 10 · 1.3 min"
        );
        assert_eq!(
            lines(2)[..2],
            [
                "Contour, outside · “Contour by planner” · T1 Flat 10 · 1.3 min",
                "8000 rpm · 1000 mm/min · coolant Flood · stepdown 2 mm · radial allowance 0.25 mm · 1 finish pass",
            ]
        );
        assert_eq!(
            lines(3)[1],
            "12000 rpm · 1200 mm/min · coolant Flood · Deep drilling, peck 3 mm · through, breakthrough 1.5 mm · holes of Ø6.6 mm only"
        );
        assert_eq!(
            lines(4)[..2],
            [
                "Parallel finishing · “Parallel by planner” · T1 Flat 10 · 1.3 min",
                "8000 rpm · 1000 mm/min · coolant Flood · surface stepover 0.8 mm · tolerance 0.02 mm",
            ]
        );
        assert_eq!(
            lines(5)[1],
            "8000 rpm · 1000 mm/min · coolant Flood · stepdown 3 mm"
        );
        // Face says which area it faces; coolant shows the value that runs.
        let mut face = ops[0].clone();
        face.p.insert("boundary".into(), "Model silhouette".into());
        face.p.insert("coolant".into(), "Off".into());
        assert!(
            crate::ui::operation_lines(&face, "T1", 1., "")[1].starts_with(
                "8000 rpm · 1000 mm/min · coolant Off · facing area model silhouette · "
            )
        );
        // An inside contour, and a drill that stops in the hole.
        let mut inside = ops[2].clone();
        inside.p.insert("side".into(), "Inside".into());
        inside
            .p
            .insert("boundary".into(), "Selected geometry".into());
        assert!(
            crate::ui::operation_lines(&inside, "T1", 1., "")[0]
                .starts_with("Contour, inside floor · ")
        );
        // It runs as a floor profile: Pocket's entry and ramp, a fixed stepover.
        assert_eq!(
            crate::ui::operation_lines(&inside, "T1", 1., "")[1],
            "8000 rpm · 1000 mm/min · coolant Flood · stepdown 2 mm · stepover 40% (fixed) · radial allowance 0.25 mm · entry Profile ramp at 2° · 1 finish pass"
        );
        inside.p.insert("pocket_entry".into(), "Plunge".into());
        assert!(
            crate::ui::operation_lines(&inside, "T1", 1., "")[1].ends_with(
                " · stepover 40% (fixed) · radial allowance 0.25 mm · entry Plunge · 1 finish pass"
            )
        );
        let mut blind = ops[3].clone();
        blind.p.insert("through".into(), "No".into());
        blind.p.insert("cycle".into(), "Drilling".into());
        blind.p.insert("hole_diameter".into(), "0".into());
        assert_eq!(
            crate::ui::operation_lines(&blind, "T3", 1., "")[1],
            "12000 rpm · 1200 mm/min · coolant Flood · Drilling · not through"
        );
        assert_eq!(
            crate::ui::operation_lines(&blind, "T3", 1., "")[2],
            "The planner gave no reason."
        );
    }
    #[test]
    fn the_preview_and_the_program_page_say_what_their_checks_leave_out() {
        use crate::ui::{plan_check_lines, plan_not_checked, plan_work_zero_line, program_note};
        // The preview's check lines; the second names the machine limits
        // that speeds and feeds are held to.
        assert_eq!(
            plan_check_lines(),
            [
                "Every operation generates with the stock left by the ones before it",
                "LinuxCNC export checks pass: machine rpm and feed limits, travel, rapids into material, holder clearance",
                "No tool centre line cuts into the part, and nothing goes below the stock",
                "No gouging detected on the stock grid (coarse check)",
            ]
        );
        // The note under the lines says what is not checked.
        assert_eq!(
            plan_not_checked(),
            "Not checked: cutting loads, tool deflection, fixtures, clamps and the machine body. Speeds and feeds are checked against the material's cutting data, which are starting points."
        );
        // The page beside the Plan button says the cutting data are sent.
        assert!(
            crate::ui::SENT_TO_OPENAI.contains(
                "your tools, machine limits, the project material with its cutting data,"
            )
        );
        // The program page says what export repeats, and that the planner's
        // own checks covered the plan as applied, not later edits.
        assert_eq!(
            program_note(),
            "LinuxCNC 3-axis output, one .ngc program per setup. Export repeats the export checks — limits, travel, rapids into material, holder clearance — and a replay of each program against the simulated moves, and writes nothing if any setup fails. The planner's tool-axis and gouge checks covered the plan as applied, not later edits. Fixtures, clamps and the machine body are not checked: preview each program in LinuxCNC and air-cut first."
        );
        // The good plan's work-zero lines, with the values its check found:
        // setup 2's Model origin is located on the part and the stock top.
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let preview = checked_preview(&a, false);
        assert_eq!(
            plan_work_zero_line(&preview.outcome, 0),
            "Rotation [0, 0, 0]° · work zero: Stock top center"
        );
        assert_eq!(
            plan_work_zero_line(&preview.outcome, 1),
            "Rotation [180, 0, 0]° · work zero: Model origin — X0 Y0 at X+60 Y+40 mm from the part's −X −Y corner (part 120 × 80 mm), Z0 3 mm below the stock top"
        );
        // Both pages build with these lines.
        a.planner = crate::Planner::Preview(preview);
        a.page = Page::Ai;
        let _ = a.view();
        a.page = Page::Post;
        let _ = a.view();
    }
    #[test]
    fn the_preview_locates_a_model_origin_zero_on_the_part_and_the_stock_top() {
        let mut setup = Setup::new(Machine::new(3), "G55");
        setup.p.insert("rx".into(), "180".into());
        let part = crate::mesh::Bounds {
            min: [-60., -40., -40.],
            max: [60., 40., 0.],
        };
        assert_eq!(
            crate::ui::work_zero_line(&setup, Some((part, Some(3.)))),
            "Rotation [180, 0, 0]° · work zero: Stock top center"
        );
        setup.p.insert("origin".into(), "Model origin".into());
        assert_eq!(
            crate::ui::work_zero_line(&setup, Some((part, Some(3.)))),
            "Rotation [180, 0, 0]° · work zero: Model origin — X0 Y0 at X+60 Y+40 mm from the part's −X −Y corner (part 120 × 80 mm), Z0 3 mm below the stock top"
        );
        // A file origin far from the part is visibly off it.
        let far = crate::mesh::Bounds {
            min: [-360., -40., -40.],
            max: [-240., 40., 0.],
        };
        assert_eq!(
            crate::ui::work_zero_line(&setup, Some((far, Some(-2.5)))),
            "Rotation [180, 0, 0]° · work zero: Model origin — X0 Y0 at X+360 Y+40 mm from the part's −X −Y corner (part 120 × 80 mm), Z0 2.5 mm above the stock top"
        );
        assert_eq!(
            crate::ui::work_zero_line(&setup, None),
            "Rotation [180, 0, 0]° · work zero: Model origin"
        );
        // An earlier setup cut the stock top: Z0 from the part's top, and
        // touch off on a part face.
        let side = crate::mesh::Bounds {
            min: [-60., 0., -40.],
            max: [60., 40., 40.],
        };
        assert_eq!(
            crate::ui::work_zero_line(&setup, Some((side, None))),
            "Rotation [180, 0, 0]° · work zero: Model origin — X0 Y0 at X+60 Y0 mm from the part's −X −Y corner (part 120 × 40 mm), Z0 40 mm below the part's top. The stock top was cut by an earlier setup; touch off on a part face."
        );
    }
    #[test]
    fn later_setup_names_are_labelled_as_the_planners() {
        // Setup 1 keeps the user's name; a later setup's name is the planner's.
        assert_eq!(crate::ui::setup_title(0, "Top"), "Setup 1: Top");
        assert_eq!(
            crate::ui::setup_title(1, "Underside"),
            "Setup 2 · named by the planner: “Underside”"
        );
    }
    #[test]
    fn the_preview_lists_covered_walls_and_how_coverage_is_measured() {
        let dir = tempfile::tempdir().unwrap();
        let a = signed_in_planning_app(&dir);
        let preview = checked_preview(&a, false);
        let cover = preview.outcome.check.coverage.as_ref().unwrap();
        assert!(!cover.covered_other.is_empty());
        let lines = crate::ui::coverage_lines(cover);
        assert!(
            lines.iter().any(|l| l
                .starts_with("• Still covered: walls and curved surface facing down in setup 1 (")),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l.contains("facing +X")));
        assert_eq!(lines.last().unwrap(), cover.note);
    }
    #[test]
    fn a_follow_up_error_reaches_the_account_and_the_preview() {
        use crate::openai::ApiError;
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        for (note, signed_out) in [
            (ApiError::UsageLimit, false),
            (ApiError::SignedOut("Sign in again.".into()), true),
        ] {
            let mut a = signed_in_planning_app(&dir);
            let mut preview = Arc::try_unwrap(checked_preview(&a, false)).unwrap();
            preview.outcome.note = Some(note.clone());
            let lines = crate::ui::preview_totals(&preview.outcome);
            assert!(lines[0].ends_with("· 2 model requests"), "{}", lines[0]);
            assert_eq!(
                lines[1],
                format!("A follow-up request failed: {}", note.message())
            );
            let _ = a.update(Message::Plan);
            let epoch = a.plan_epoch;
            let _ = a.update(Message::PlanFinished(epoch, Ok(Arc::new(preview))));
            assert!(matches!(a.planner, crate::Planner::Preview(_)));
            assert_eq!(a.account == crate::Account::SignedOut, signed_out);
            assert_eq!(a.usage_limit, !signed_out);
            assert_eq!(a.account_note, note.message());
        }
        // A note from a run whose client was replaced changes nothing.
        let mut a = signed_in_planning_app(&dir);
        let mut preview = Arc::try_unwrap(checked_preview(&a, false)).unwrap();
        preview.outcome.note = Some(ApiError::SignedOut("Sign in again.".into()));
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        a.account_epoch = a.account_epoch.wrapping_add(1);
        let _ = a.update(Message::PlanFinished(epoch, Ok(Arc::new(preview))));
        assert!(matches!(a.planner, crate::Planner::Preview(_)));
        assert!(matches!(a.account, crate::Account::SignedIn { .. }));
    }
    #[test]
    fn the_preview_view_has_its_own_camera() {
        use crate::viewport::Event;
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let _ = a.update(Message::Viewport(Event::Orbit(0.3, 0.1)));
        let workspace = format!("{:?}", a.camera);
        let framed = format!("{:?}", crate::viewport::Camera::default());
        assert_ne!(workspace, framed);
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        // The camera is steered without the view, so none is built here.
        let _ = a.update(Message::PlanFinished(epoch, Ok(checked_preview(&a, false))));
        assert_eq!(format!("{:?}", a.plan_camera), framed, "starts framed");
        for event in [
            Event::Orbit(0.5, -0.2),
            Event::Pan(4., 2.),
            Event::Zoom(0.2),
        ] {
            let _ = a.update(Message::PlanViewport(event));
        }
        let moved = a.plan_camera;
        assert!(moved.yaw != 0.75 && moved.pan == [4., 2.] && moved.zoom > 1.);
        assert_eq!(
            format!("{:?}", a.camera),
            workspace,
            "the workspace camera stays"
        );
        a.page = Page::Ai;
        let _ = a.view();
        // The next preview starts framed again.
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let _ = a.update(Message::PlanFinished(epoch, Ok(checked_preview(&a, false))));
        assert_eq!(format!("{:?}", a.plan_camera), framed);
    }
    #[test]
    fn every_planner_stage_has_its_own_page_text() {
        use crate::planner::Progress::*;
        let texts: Vec<String> = [
            Analyzing,
            Asking(1),
            Repairing(2),
            Extending(2),
            Checking(2),
            Previewing,
        ]
        .into_iter()
        .map(|stage| crate::ui::stage_text(stage, "gpt-6.1-sol"))
        .collect();
        for (i, text) in texts.iter().enumerate() {
            assert!(!texts[..i].contains(text), "{text}");
        }
        assert!(texts[1].contains("gpt-6.1-sol") && texts[1].contains("1 of 4"));
        assert!(texts[2].contains("correction") && texts[2].contains("2 of 4"));
        assert!(texts[3].contains("still covered") && texts[3].contains("2 of 4"));
        assert!(texts[4].contains("2 of 4"));
        assert!(texts[5].contains("3D view"));
    }
    #[test]
    fn a_plan_dropped_by_an_edit_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let stopped = "Planning stopped because the job changed.";
        let discarded = "The plan preview was discarded because the job changed.";
        let mut a = signed_in_planning_app(&dir);
        let _ = a.update(Message::Plan);
        let _ = a.update(Message::Edit(Target::Setup, "stock_top", "6".into()));
        assert!(matches!(a.planner, crate::Planner::Idle));
        assert_eq!(a.status, stopped);
        // An edit elsewhere with a status of its own, even one that changes
        // nothing, says both.
        let _ = a.update(Message::SaveTool);
        let own = a.status.clone();
        let _ = a.update(Message::Plan);
        let _ = a.update(Message::SaveTool);
        assert!(matches!(a.planner, crate::Planner::Idle));
        assert_eq!(a.status, format!("{own} {stopped}"));
        let preview = checked_preview(&a, false);
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let _ = a.update(Message::PlanFinished(epoch, Ok(preview)));
        let _ = a.update(Message::Title("Renamed".into()));
        assert!(matches!(a.planner, crate::Planner::Idle));
        assert_eq!(a.status, discarded);
        // Nothing to drop: the edit's own status stays.
        let _ = a.update(Message::SaveTool);
        assert_eq!(a.status, own);
        // Undo with a preview on screen says what it restored, then why the
        // preview went.
        apply_checked_plan(&mut a);
        let preview = checked_preview(&a, false);
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let _ = a.update(Message::PlanFinished(epoch, Ok(preview)));
        let _ = a.update(Message::UndoPlan);
        assert!(a.undo.is_none());
        assert!(
            a.status.starts_with(
                "The setups from before the plan are restored. The applied job is in History."
            ) && a.status.ends_with(&format!(" {discarded}")),
            "{}",
            a.status
        );
    }
    #[test]
    fn the_running_stage_names_the_model_the_run_uses() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let epoch = a.account_epoch;
        let both = vec!["gpt-6.1-sol".to_string(), "gpt-6-luna".into()];
        let _ = a.update(Message::ModelsLoaded(epoch, Ok(both)));
        let _ = a.update(Message::Plan);
        let _ = a.update(Message::PickModel("gpt-6-luna".into()));
        assert_eq!(a.settings.model, "gpt-6-luna");
        let text = crate::ui::running_text(&a, crate::planner::Progress::Asking(1));
        assert!(
            text.contains("gpt-6.1-sol") && !text.contains("gpt-6-luna"),
            "{text}"
        );
    }
    #[test]
    fn plan_waits_for_the_blocking_work_of_an_earlier_run() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let _ = a.update(Message::Plan);
        let _ = a.update(Message::PlanCancel);
        // The cancelled run's check is still running on the blocking pool.
        let check = a.plan_busy.enter();
        let _ = a.update(Message::Plan);
        assert!(matches!(a.planner, crate::Planner::Idle));
        assert_eq!(
            a.status,
            "The previous plan check is still finishing; try again in a moment."
        );
        drop(check);
        let _ = a.update(Message::Plan);
        assert!(matches!(a.planner, crate::Planner::Running(_)));
    }
    #[test]
    fn the_planner_page_builds_in_every_planner_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        a.page = Page::Ai;
        let _ = a.view();
        a.planner = crate::Planner::Running(crate::planner::Progress::Checking(3));
        let _ = a.view();
        a.planner = crate::Planner::Failed(crate::planner::Failure {
            message: "No plan passed.".into(),
            problems: vec![crate::proposal::Problem {
                setup: 1,
                operation: Some(2),
                stage: "generation",
                message: "x".repeat(900),
            }],
            api: None,
            requests: 4,
        });
        let _ = a.view();
        let with_view = checked_preview(&a, true);
        a.planner = crate::Planner::Preview(std::sync::Arc::new(crate::PlanPreview {
            outcome: with_view.outcome.clone(),
            view: None,
        }));
        let _ = a.view();
        a.planner = crate::Planner::Preview(with_view);
        let _ = a.view();
        a.undo = Some(a.doc.clone());
        let _ = a.view();
    }
    fn signed_in_app(dir: &tempfile::TempDir) -> App {
        let mut a = fresh_app(dir);
        sign_in_with(&mut a, dir, crate::session::sample());
        a
    }
    fn without_plan_usage() -> crate::session::Session {
        crate::session::Session {
            scope: "openid profile email".into(),
            ..crate::session::sample()
        }
    }
    #[test]
    fn signing_in_saves_the_client_and_session_but_never_tokens_in_settings() {
        let dir = tempfile::tempdir().unwrap();
        let a = signed_in_app(&dir);
        assert_eq!(
            a.account,
            crate::Account::SignedIn {
                name: "Lior".into(),
                email: "lior@example.com".into(),
                granted: true
            }
        );
        let settings = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(settings.contains("oaiapp_1"));
        assert!(!settings.contains("access-1") && !settings.contains("refresh-1"));
        let saved = std::fs::read_to_string(dir.path().join("chatgpt-session.json")).unwrap();
        assert!(saved.contains("refresh-1"));
        let reopened = App::from_workspace(dir.path().join("workspace.json"), None);
        assert_eq!(reopened.settings.client_id, "oaiapp_1");
        assert_eq!(reopened.account, crate::Account::SignedOut);
    }
    #[test]
    fn account_messages_drive_the_panel_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        let _ = a.update(Message::SignIn);
        assert_eq!(a.account, crate::Account::SigningIn);
        let _ = a.update(Message::SignInCancel);
        assert_eq!(a.account, crate::Account::SignedOut);
        assert_eq!(a.account_note, "Sign-in cancelled.");
        let _ = a.update(Message::SignIn);
        let _ = a.update(Message::SignedIn(Err(
            "Sign-in timed out. Try again.".into()
        )));
        assert_eq!(a.account, crate::Account::SignedOut);
        assert_eq!(a.account_note, "Sign-in timed out. Try again.");
        let mut a = signed_in_app(&dir);
        let epoch = a.account_epoch;
        let listed = vec!["gpt-6-luna".to_string(), "gpt-6.1-sol".to_string()];
        let _ = a.update(Message::ModelsLoaded(epoch, Ok(listed)));
        assert_eq!(a.settings.model, "gpt-6.1-sol");
        let _ = a.update(Message::PickModel("gpt-6-luna".into()));
        let _ = a.update(Message::PickModel("not-listed".into()));
        assert_eq!(a.settings.model, "gpt-6-luna");
        let _ = a.update(Message::ModelsLoaded(epoch, Ok(vec!["gpt-6-luna".into()])));
        assert_eq!(a.settings.model, "gpt-6-luna");
        let _ = a.update(Message::CheckConnection);
        assert!(a.account_busy);
        let limit = crate::openai::ApiError::UsageLimit;
        let _ = a.update(Message::ConnectionChecked(epoch, Err(limit)));
        assert!(a.usage_limit && !a.account_busy);
        assert!(a.account_note.contains("Usage limit reached"));
        let ok = Ok("Connection works.".into());
        let _ = a.update(Message::ConnectionChecked(epoch, ok));
        assert!(!a.usage_limit);
        assert_eq!(a.account_note, "Connection works.");
        assert!(!a.settings.plan_notice_shown);
        let _ = a.update(Message::PlanNoticeDismissed);
        assert!(crate::settings::load(&dir.path().join("settings.json")).plan_notice_shown);
        let gone = crate::openai::ApiError::SignedOut("Sign in again.".into());
        let _ = a.update(Message::ModelsLoaded(epoch, Err(gone)));
        assert_eq!(a.account, crate::Account::SignedOut);
        assert!(a.models.is_empty());
        assert_eq!(a.account_note, "Sign in again.");
    }
    #[test]
    fn a_session_without_plan_usage_is_signed_in_but_not_granted() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        sign_in_with(&mut a, &dir, without_plan_usage());
        assert!(matches!(
            a.account,
            crate::Account::SignedIn { granted: false, .. }
        ));
        assert!(a.account_note.contains("not granted"));
        assert!(a.models.is_empty());
        let _ = a.update(Message::SignOut);
        assert_eq!(a.account, crate::Account::SignedOut);
        // The panel says "Signing out…" while the old sign-in is deleted.
        assert!(a.account_note.is_empty());
        // Signing in again waits until the old sign-in has been deleted.
        assert!(a.sign_out_pending && !a.account_busy);
        let _ = a.update(Message::SignIn);
        assert_eq!(a.account, crate::Account::SignedOut);
        let failed = "Signed out on this computer. Could not reach OpenAI.".to_string();
        let _ = a.update(Message::SignedOut(Err(failed.clone())));
        assert!(!a.sign_out_pending && !a.account_busy);
        assert_eq!(a.account_note, failed);
        let _ = a.update(Message::SignIn);
        assert_eq!(a.account, crate::Account::SigningIn);
    }
    #[test]
    fn the_saved_session_is_adopted_when_the_planner_page_restores_it() {
        let dir = tempfile::tempdir().unwrap();
        let _ = signed_in_app(&dir);
        let mut a = fresh_app(&dir);
        assert!(!a.restoring);
        let _ = a.update(Message::Page(Page::Ai));
        assert_eq!(a.account, crate::Account::SignedOut);
        assert!(a.restoring);
        // Unit tests do not run tasks; deliver the restore result directly.
        let saved = crate::session::default_store(dir.path(), true).load();
        let _ = a.update(Message::AccountRestored(saved));
        assert!(matches!(
            a.account,
            crate::Account::SignedIn { granted: true, .. }
        ));
        assert!(!a.restoring);
        // A read that fails says why and leaves the account signed out.
        let mut a = fresh_app(&dir);
        let _ = a.update(Message::Page(Page::Ai));
        let _ = a.update(Message::AccountRestored(Err("Keychain: denied".into())));
        assert_eq!(a.account, crate::Account::SignedOut);
        assert_eq!(a.account_note, "Keychain: denied");
        assert!(!a.restoring);
    }
    #[test]
    fn a_restore_result_is_ignored_when_no_restore_is_pending() {
        let dir = tempfile::tempdir().unwrap();
        // Signed out, and signed in: neither is waiting for a saved sign-in.
        for mut a in [fresh_app(&dir), signed_in_app(&dir)] {
            let before = snapshot(&a);
            for result in [
                Ok(Some(crate::session::sample())),
                Ok(None),
                Err("Keychain: denied".to_string()),
            ] {
                assert_eq!(a.update(Message::AccountRestored(result)).units(), 0);
                assert_eq!(snapshot(&a), before);
                assert!(!a.restoring);
            }
        }
    }
    #[test]
    fn signing_in_waits_until_the_saved_sign_in_has_been_read() {
        let dir = tempfile::tempdir().unwrap();
        let _ = signed_in_app(&dir);
        for result in [Ok(None), Err("Keychain: denied".to_string())] {
            let mut a = fresh_app(&dir);
            assert_eq!(a.update(Message::Page(Page::Ai)).units(), 1);
            assert!(a.restoring);
            let before = snapshot(&a);
            assert_eq!(a.update(Message::SignIn).units(), 0);
            assert_eq!(snapshot(&a), before);
            assert!(a.restoring && a.sign_in.is_none());
            // The read found nothing, or failed: now the user can sign in.
            let _ = a.update(Message::AccountRestored(result));
            assert!(!a.restoring);
            assert_eq!(a.update(Message::SignIn).units(), 1);
            assert_eq!(a.account, crate::Account::SigningIn);
        }
    }
    #[test]
    fn a_second_sign_out_keeps_the_gate_and_sign_in_waits_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_app(&dir);
        assert_eq!(a.update(Message::SignOut).units(), 1);
        assert!(a.sign_out_pending && !a.account_busy);
        assert_eq!(a.update(Message::SignOut).units(), 0);
        assert!(a.sign_out_pending);
        assert_eq!(a.update(Message::SignIn).units(), 0);
        assert_eq!(a.account, crate::Account::SignedOut);
        let _ = a.update(Message::SignedOut(Ok(())));
        assert!(!a.sign_out_pending);
        assert_eq!(a.update(Message::SignIn).units(), 1);
        assert_eq!(a.account, crate::Account::SigningIn);
        // Signing in is only possible from signed-out.
        assert_eq!(a.update(Message::SignIn).units(), 0);
    }
    #[test]
    fn late_results_from_a_signed_out_client_change_nothing() {
        use crate::openai::ApiError;
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_app(&dir);
        let old = a.account_epoch;
        let _ = a.update(Message::ModelsLoaded(old, Ok(vec!["gpt-6.1-sol".into()])));
        let _ = a.update(Message::CheckConnection);
        assert!(a.account_busy);
        let _ = a.update(Message::SignOut);
        assert!(a.account_epoch != old);
        assert!(a.account_note.is_empty());
        assert!(!a.account_busy && !a.usage_limit);
        let late = [
            Message::ConnectionChecked(old, Err(ApiError::UsageLimit)),
            Message::ConnectionChecked(old, Ok("Connection works.".into())),
            Message::ConnectionChecked(old, Err(ApiError::Unavailable("Offline.".into()))),
            Message::ModelsLoaded(old, Ok(vec!["gpt-6-luna".into()])),
            Message::ModelsLoaded(old, Err(ApiError::UsageLimit)),
            Message::ModelsLoaded(old, Err(ApiError::SignedOut("Gone.".into()))),
        ];
        for message in late {
            assert_eq!(a.update(message).units(), 0);
            assert_eq!(a.account, crate::Account::SignedOut);
            assert!(a.account_note.is_empty());
            assert!(!a.account_busy && !a.usage_limit && a.models.is_empty());
            assert!(a.sign_out_pending);
        }
        assert_eq!(a.settings.model, "gpt-6.1-sol");
    }
    #[test]
    fn late_results_from_an_old_client_do_not_touch_a_new_sign_in() {
        use crate::openai::ApiError;
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_app(&dir);
        let old = a.account_epoch;
        let _ = a.update(Message::SignOut);
        let _ = a.update(Message::SignedOut(Ok(())));
        sign_in_with(&mut a, &dir, crate::session::sample());
        let new = a.account_epoch;
        assert!(new != old);
        let _ = a.update(Message::ModelsLoaded(new, Ok(vec!["gpt-6-luna".into()])));
        let signed_in = a.account.clone();
        let gone = ApiError::SignedOut("Old sign-in gone.".into());
        let _ = a.update(Message::ModelsLoaded(old, Err(gone)));
        assert_eq!(a.account, signed_in);
        assert_eq!(a.models, vec!["gpt-6-luna".to_string()]);
        assert!(a.account_note.is_empty());
        let _ = a.update(Message::ModelsLoaded(old, Ok(vec!["gpt-6.1-sol".into()])));
        assert_eq!(a.models, vec!["gpt-6-luna".to_string()]);
        assert_eq!(a.settings.model, "gpt-6-luna");
        // The new client still works with its own results.
        let gone = ApiError::SignedOut("Now gone.".into());
        let _ = a.update(Message::ModelsLoaded(new, Err(gone)));
        assert_eq!(a.account, crate::Account::SignedOut);
        assert!(a.account_epoch != new);
    }
    #[test]
    fn a_cancelled_sign_in_ignores_a_late_result() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        let _ = a.update(Message::SignIn);
        let _ = a.update(Message::SignInCancel);
        // The result is not used, but its refresh token is revoked: one task.
        let late = Message::SignedIn(Ok((crate::session::sample(), "oaiapp_1".into())));
        assert_eq!(a.update(late).units(), 1);
        assert_eq!(a.account, crate::Account::SignedOut);
        assert_eq!(a.account_note, "Sign-in cancelled.");
        assert!(a.settings.client_id.is_empty() && a.models.is_empty());
        assert!(a.client.is_none());
        let late = Message::SignedIn(Err("Sign-in timed out. Try again.".into()));
        assert_eq!(a.update(late).units(), 0);
        assert_eq!(a.account_note, "Sign-in cancelled.");
    }
    #[test]
    fn signing_out_while_signing_in_stops_the_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        let _ = a.update(Message::SignIn);
        let handle = a.sign_in.clone().expect("the sign-in is running");
        assert!(!handle.is_aborted());
        let _ = a.update(Message::SignOut);
        assert!(handle.is_aborted() && a.sign_in.is_none());
        assert_eq!(a.account, crate::Account::SignedOut);
        assert!(!a.sign_out_pending, "there was no client to sign out");
        let late = Message::SignedIn(Ok((crate::session::sample(), "oaiapp_1".into())));
        assert_eq!(a.update(late).units(), 1, "revoked, not adopted");
        assert_eq!(a.account, crate::Account::SignedOut);
        assert!(a.account_note.is_empty());
        assert!(a.settings.client_id.is_empty() && a.client.is_none());
    }
    #[test]
    fn a_restored_session_cannot_replace_a_current_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_app(&dir);
        let account = a.account.clone();
        let client = a.client.clone().unwrap();
        let other = crate::session::Session {
            name: "Other".into(),
            ..crate::session::sample()
        };
        let restored = Message::AccountRestored(Ok(Some(other.clone())));
        assert_eq!(a.update(restored).units(), 0);
        assert_eq!(a.account, account);
        assert!(std::sync::Arc::ptr_eq(a.client.as_ref().unwrap(), &client));
        // Not while a sign-out is still deleting the old sign-in, nor while
        // a new sign-in is running.
        let _ = a.update(Message::SignOut);
        let restored = Message::AccountRestored(Ok(Some(other.clone())));
        assert_eq!(a.update(restored).units(), 0);
        assert!(a.client.is_none() && a.account == crate::Account::SignedOut);
        let _ = a.update(Message::SignedOut(Ok(())));
        let _ = a.update(Message::SignIn);
        let restored = Message::AccountRestored(Ok(Some(other.clone())));
        assert_eq!(a.update(restored).units(), 0);
        assert!(a.client.is_none() && a.account == crate::Account::SigningIn);
        // Nor once the account is signed out again: no read is pending, so
        // an old session does not come back.
        let _ = a.update(Message::SignInCancel);
        let restored = Message::AccountRestored(Ok(Some(other.clone())));
        assert_eq!(a.update(restored).units(), 0);
        assert!(a.client.is_none() && a.account == crate::Account::SignedOut);
        assert_eq!(a.account_note, "Sign-in cancelled.");
        // Even a pending read only fills an empty account.
        let mut a = signed_in_app(&dir);
        let before = snapshot(&a);
        assert_eq!(a.update(Message::Page(Page::Ai)).units(), 1);
        assert!(a.restoring);
        let restored = Message::AccountRestored(Ok(Some(other)));
        assert_eq!(a.update(restored).units(), 0);
        assert_eq!(snapshot(&a), before);
        assert!(!a.restoring);
    }
    #[test]
    fn the_planner_page_restores_only_once() {
        let dir = tempfile::tempdir().unwrap();
        let _ = signed_in_app(&dir);
        let mut a = fresh_app(&dir);
        assert_eq!(a.update(Message::Page(Page::Home)).units(), 0);
        assert_eq!(a.update(Message::Page(Page::Ai)).units(), 1);
        let saved = crate::session::default_store(dir.path(), true).load();
        let _ = a.update(Message::AccountRestored(saved));
        let _ = a.update(Message::Page(Page::Home));
        assert_eq!(a.update(Message::Page(Page::Ai)).units(), 0);
        // A visit before any sign-in uses up the restore too, so signing in
        // and out afterwards never reads the saved session behind it.
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        assert_eq!(a.update(Message::Page(Page::Ai)).units(), 0);
        assert!(!a.restoring, "nothing is being read");
        sign_in_with(&mut a, &dir, crate::session::sample());
        assert!(matches!(a.account, crate::Account::SignedIn { .. }));
        let _ = a.update(Message::SignOut);
        let _ = a.update(Message::SignedOut(Ok(())));
        let _ = a.update(Message::Page(Page::Home));
        assert_eq!(a.update(Message::Page(Page::Ai)).units(), 0);
    }
    #[test]
    fn checking_the_connection_needs_granted_plan_usage() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        a.settings.model = "gpt-6.1-sol".into();
        assert_eq!(a.update(Message::CheckConnection).units(), 0);
        sign_in_with(&mut a, &dir, without_plan_usage());
        let note = a.account_note.clone();
        assert!(a.client.is_some());
        assert_eq!(a.update(Message::CheckConnection).units(), 0);
        assert!(!a.account_busy);
        assert_eq!(a.account_note, note);
        // Granted: the check starts, and a second one waits for it.
        let _ = a.update(Message::SignOut);
        let _ = a.update(Message::SignedOut(Ok(())));
        sign_in_with(&mut a, &dir, crate::session::sample());
        assert_eq!(a.update(Message::CheckConnection).units(), 1);
        assert!(a.account_busy);
        assert_eq!(a.update(Message::CheckConnection).units(), 0);
    }
    #[test]
    fn an_empty_model_list_keeps_the_saved_model_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_app(&dir);
        let epoch = a.account_epoch;
        let both = vec!["gpt-6-luna".to_string(), "gpt-6.1-sol".to_string()];
        let _ = a.update(Message::ModelsLoaded(epoch, Ok(both.clone())));
        let _ = a.update(Message::PickModel("gpt-6-luna".into()));
        let _ = a.update(Message::ModelsLoaded(epoch, Ok(vec![])));
        assert!(a.models.is_empty());
        assert_eq!(a.settings.model, "gpt-6-luna");
        assert_eq!(
            crate::settings::load(&dir.path().join("settings.json")).model,
            "gpt-6-luna"
        );
        assert_eq!(
            a.account_note,
            "Your ChatGPT plan lists no models for this app. The app will use gpt-6-luna; Check connection shows whether it answers."
        );
        // The saved model can still be checked without a list.
        assert_eq!(a.update(Message::CheckConnection).units(), 1);
        let _ = a.update(Message::ConnectionChecked(a.account_epoch, Ok("ok".into())));
        let _ = a.update(Message::ModelsLoaded(epoch, Ok(vec![])));
        // The panel offers Load models; a new list replaces the note.
        assert_eq!(a.update(Message::LoadModels).units(), 1);
        let _ = a.update(Message::ModelsLoaded(epoch, Ok(both.clone())));
        assert_eq!(a.models, both);
        assert_eq!(a.settings.model, "gpt-6-luna");
        assert!(a.account_note.is_empty());
    }
    #[test]
    fn without_a_model_list_the_planner_falls_back_to_the_default_model() {
        for result in [
            Ok(vec![]),
            Err(crate::openai::ApiError::Unavailable(
                "OpenAI is unavailable.".into(),
            )),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut a = signed_in_app(&dir);
            assert!(a.settings.model.is_empty());
            let failed = result.is_err();
            let _ = a.update(Message::ModelsLoaded(a.account_epoch, result));
            assert_eq!(a.settings.model, "gpt-6.1-sol");
            assert_eq!(
                crate::settings::load(&dir.path().join("settings.json")).model,
                "gpt-6.1-sol"
            );
            assert!(a.models.is_empty());
            assert!(
                a.account_note.ends_with(
                    "The app will use gpt-6.1-sol; Check connection shows whether it answers."
                ),
                "{}",
                a.account_note
            );
            assert_eq!(a.account_note.contains("OpenAI is unavailable."), failed);
            a.page = Page::Ai;
            let _ = a.view();
        }
        // A sign-in that is gone, or has no plan usage, gets no model.
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_app(&dir);
        let gone = crate::openai::ApiError::SignedOut("Sign in again.".into());
        let _ = a.update(Message::ModelsLoaded(a.account_epoch, Err(gone)));
        assert!(a.settings.model.is_empty());
        assert_eq!(a.account_note, "Sign in again.");
    }
    #[test]
    fn a_model_list_clears_the_fallback_note_whatever_its_first_words() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_app(&dir);
        let unavailable = crate::openai::ApiError::Unavailable("OpenAI is unavailable.".into());
        let _ = a.update(Message::ModelsLoaded(a.account_epoch, Err(unavailable)));
        assert!(
            a.account_note.starts_with("OpenAI is unavailable.")
                && a.account_note.contains("The app will use gpt-6.1-sol"),
            "{}",
            a.account_note
        );
        let _ = a.update(Message::LoadModels);
        let luna = vec!["gpt-6-luna".to_string()];
        let _ = a.update(Message::ModelsLoaded(a.account_epoch, Ok(luna)));
        assert_eq!(a.settings.model, "gpt-6-luna");
        assert!(a.account_note.is_empty(), "{}", a.account_note);
        // A note that is not about the fallback stays.
        a.account_note = "ok".into();
        let both = vec!["gpt-6-luna".to_string(), "gpt-6.1-sol".into()];
        let _ = a.update(Message::ModelsLoaded(a.account_epoch, Ok(both)));
        assert_eq!(a.account_note, "ok");
    }
    #[test]
    fn a_successful_plan_clears_the_usage_limit_banner() {
        use crate::openai::ApiError;
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let limited = |a: &mut App| {
            let _ = a.update(Message::Plan);
            let epoch = a.plan_epoch;
            let failure = crate::planner::Failure {
                message: ApiError::UsageLimit.message(),
                problems: vec![],
                api: Some(ApiError::UsageLimit),
                requests: 1,
            };
            let _ = a.update(Message::PlanFinished(epoch, Err(failure)));
            assert!(a.usage_limit);
        };
        // The job does not change here, so one checked plan serves every run.
        let checked = checked_preview(&a, false);
        limited(&mut a);
        assert_eq!(a.account_note, ApiError::UsageLimit.message());
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let _ = a.update(Message::PlanFinished(epoch, Ok(checked.clone())));
        assert!(matches!(a.planner, crate::Planner::Preview(_)));
        assert!(!a.usage_limit);
        assert_eq!(
            a.account_note, "",
            "the panel no longer says the limit is reached"
        );
        // So does one that carries the fallback sentence after the limit,
        // as a model list that hit the limit leaves it.
        let epoch = a.account_epoch;
        let _ = a.update(Message::ModelsLoaded(epoch, Err(ApiError::UsageLimit)));
        assert!(a.usage_limit);
        assert!(
            a.account_note.starts_with(&format!(
                "{} The app will use",
                ApiError::UsageLimit.message()
            )),
            "{}",
            a.account_note
        );
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let _ = a.update(Message::PlanFinished(epoch, Ok(checked.clone())));
        assert!(!a.usage_limit);
        assert_eq!(a.account_note, "", "the fallback sentence goes with it");
        // Another note stays.
        a.account_note = "Connection works: gpt-6.1-sol answered.".into();
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let _ = a.update(Message::PlanFinished(epoch, Ok(checked.clone())));
        assert_eq!(a.account_note, "Connection works: gpt-6.1-sol answered.");
        // A follow-up request that hit the limit keeps the banner.
        limited(&mut a);
        let mut preview = crate::PlanPreview {
            outcome: checked.outcome.clone(),
            view: None,
        };
        preview.outcome.note = Some(ApiError::UsageLimit);
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let _ = a.update(Message::PlanFinished(
            epoch,
            Ok(std::sync::Arc::new(preview)),
        ));
        assert!(a.usage_limit);
        assert_eq!(a.account_note, ApiError::UsageLimit.message());
    }
    #[test]
    fn loading_models_needs_a_granted_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        assert_eq!(a.update(Message::LoadModels).units(), 0);
        let _ = a.update(Message::SignIn);
        assert_eq!(a.update(Message::LoadModels).units(), 0);
        let _ = a.update(Message::SignInCancel);
        sign_in_with(&mut a, &dir, without_plan_usage());
        assert_eq!(a.update(Message::LoadModels).units(), 0);
        let _ = a.update(Message::SignOut);
        let _ = a.update(Message::SignedOut(Ok(())));
        sign_in_with(&mut a, &dir, crate::session::sample());
        assert_eq!(a.update(Message::LoadModels).units(), 1);
        let _ = a.update(Message::SignOut);
        assert_eq!(a.update(Message::LoadModels).units(), 0);
    }
    #[test]
    fn unsaved_settings_do_not_stop_a_sign_in_and_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        // A folder where the settings file belongs makes every save fail.
        std::fs::create_dir(dir.path().join("settings.json")).unwrap();
        let mut a = fresh_app(&dir);
        sign_in_with(&mut a, &dir, crate::session::sample());
        assert!(matches!(
            a.account,
            crate::Account::SignedIn { granted: true, .. }
        ));
        assert!(a.client.is_some());
        assert!(
            a.account_note
                .starts_with("Signed in for this session, but settings could not be saved: "),
            "{}",
            a.account_note
        );
    }
    #[test]
    fn a_finished_sign_in_is_saved_in_its_own_step_before_the_account_shows() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        let _ = a.update(Message::SignIn);
        let session = crate::session::sample();
        let result = Message::SignedIn(Ok((session.clone(), "oaiapp_1".into())));
        // The save step is the one returned task; the account waits for it.
        assert_eq!(a.update(result).units(), 1);
        assert_eq!(a.account, crate::Account::SigningIn);
        assert!(a.sign_in.is_none() && a.client.is_none());
        let saved = crate::settings::load(&dir.path().join("settings.json"));
        assert_eq!(saved.client_id, "oaiapp_1");
        // Its result shows the account and starts loading the models.
        assert_eq!(a.update(Message::SessionSaved(Ok(session))).units(), 1);
        assert!(matches!(
            a.account,
            crate::Account::SignedIn { granted: true, .. }
        ));
        assert!(a.client.is_some() && a.account_note.is_empty());
    }
    #[test]
    fn a_session_that_could_not_be_saved_returns_to_signed_out_with_the_message() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        let _ = a.update(Message::SignIn);
        let result = Message::SignedIn(Ok((crate::session::sample(), "oaiapp_1".into())));
        let _ = a.update(result);
        let failed = "Could not save the ChatGPT sign-in: Keychain: save denied".to_string();
        assert_eq!(
            a.update(Message::SessionSaved(Err(failed.clone()))).units(),
            0
        );
        assert_eq!(a.account, crate::Account::SignedOut);
        assert_eq!(a.account_note, failed);
        assert!(a.client.is_none() && a.models.is_empty());
        // The user can try again.
        assert_eq!(a.update(Message::SignIn).units(), 1);
        assert_eq!(a.account, crate::Account::SigningIn);
    }
    #[test]
    fn cancelling_during_the_save_step_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        let _ = a.update(Message::SignIn);
        let session = crate::session::sample();
        let result = Message::SignedIn(Ok((session.clone(), "oaiapp_1".into())));
        let _ = a.update(result);
        // The save cannot be aborted, so there is nothing to cancel.
        assert_eq!(a.update(Message::SignInCancel).units(), 0);
        assert_eq!(a.account, crate::Account::SigningIn);
        assert!(a.account_note.is_empty());
        // The save finishes and the session is adopted.
        let _ = a.update(Message::SessionSaved(Ok(session)));
        assert!(matches!(
            a.account,
            crate::Account::SignedIn { granted: true, .. }
        ));
        assert!(a.client.is_some());
    }
    #[test]
    fn the_sign_out_note_says_nothing_while_waiting_and_confirms_afterwards() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_app(&dir);
        a.account_note = "An earlier note.".into();
        // The panel shows "Signing out…" by itself while the delete runs.
        assert_eq!(a.update(Message::SignOut).units(), 1);
        assert!(a.sign_out_pending && a.account_note.is_empty());
        let _ = a.update(Message::SignedOut(Ok(())));
        assert!(!a.sign_out_pending);
        assert_eq!(a.account_note, "Signed out.");
        // A failed delete says why instead.
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_app(&dir);
        let _ = a.update(Message::SignOut);
        assert!(a.account_note.is_empty());
        let failed = "Signed out on this computer. Could not reach OpenAI.".to_string();
        let _ = a.update(Message::SignedOut(Err(failed.clone())));
        assert!(!a.sign_out_pending);
        assert_eq!(a.account_note, failed);
    }
    #[test]
    fn a_connection_check_marks_the_account_busy_and_leaves_the_note_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_app(&dir);
        let epoch = a.account_epoch;
        let _ = a.update(Message::ModelsLoaded(epoch, Ok(vec!["gpt-6.1-sol".into()])));
        a.account_note = "An earlier note.".into();
        // The panel shows "Checking the connection…" itself; the note does not repeat it.
        assert_eq!(a.update(Message::CheckConnection).units(), 1);
        assert!(a.account_busy);
        assert_eq!(a.account_note, "An earlier note.");
        // A second press while the check runs does nothing.
        assert_eq!(a.update(Message::CheckConnection).units(), 0);
        let _ = a.update(Message::ConnectionChecked(
            epoch,
            Ok("Connection works.".into()),
        ));
        assert!(!a.account_busy);
        assert_eq!(a.account_note, "Connection works.");
    }
    #[test]
    fn the_planner_page_builds_in_every_account_state() {
        use crate::openai::ApiError;
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        a.page = Page::Ai;
        let _ = a.view(); // signed out
        let _ = a.update(Message::SignIn);
        let _ = a.view(); // browser step
        let result = Message::SignedIn(Ok((crate::session::sample(), "oaiapp_1".into())));
        let _ = a.update(result);
        assert!(a.saving_sign_in());
        let _ = a.view(); // save step
        let _ = a.update(Message::SessionSaved(Ok(crate::session::sample())));
        assert!(!a.settings.plan_notice_shown);
        let epoch = a.account_epoch;
        let _ = a.view(); // granted, models not loaded yet, notice showing
        let listed = vec!["gpt-6-luna".to_string(), "gpt-6.1-sol".to_string()];
        let _ = a.update(Message::ModelsLoaded(epoch, Ok(listed)));
        let _ = a.view(); // granted with the notice and the model list
        let _ = a.update(Message::PlanNoticeDismissed);
        let _ = a.view(); // granted after the notice
        let _ = a.update(Message::CheckConnection);
        assert!(a.account_busy);
        let _ = a.view(); // checking
        let _ = a.update(Message::ConnectionChecked(epoch, Err(ApiError::UsageLimit)));
        assert!(a.usage_limit);
        let _ = a.view(); // usage limit
        a.account = crate::Account::SignedIn {
            name: String::new(),
            email: "lior@example.com".into(),
            granted: true,
        };
        let _ = a.view(); // no name: the email is the title
        let _ = a.update(Message::SignOut);
        assert!(a.sign_out_pending);
        let _ = a.view(); // signing out
        let _ = a.update(Message::SignedOut(Ok(())));
        let _ = a.view(); // signed out again, with the note
        let dir = tempfile::tempdir().unwrap();
        let mut b = fresh_app(&dir);
        b.page = Page::Ai;
        sign_in_with(&mut b, &dir, without_plan_usage());
        assert!(matches!(
            b.account,
            crate::Account::SignedIn { granted: false, .. }
        ));
        let _ = b.view(); // plan usage not granted
        // `b` left a client ID behind, so the planner page of a new app
        // looks for its saved sign-in.
        let mut c = fresh_app(&dir);
        let _ = c.update(Message::Page(Page::Ai));
        assert!(c.restoring && c.account == crate::Account::SignedOut);
        let _ = c.view(); // looking for a saved sign-in
    }
    #[test]
    fn signing_out_during_the_save_step_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        let _ = a.update(Message::SignIn);
        let session = crate::session::sample();
        let result = Message::SignedIn(Ok((session.clone(), "oaiapp_1".into())));
        let _ = a.update(result);
        assert!(a.saving_sign_in());
        let before = snapshot(&a);
        // The save cannot be aborted, so there is nothing to sign out of yet.
        assert_eq!(a.update(Message::SignOut).units(), 0);
        assert_eq!(snapshot(&a), before);
        assert_eq!(a.account, crate::Account::SigningIn);
        assert!(a.saving_sign_in() && !a.sign_out_pending);
        // The save finishes and the session is adopted as usual.
        let _ = a.update(Message::SessionSaved(Ok(session)));
        assert!(matches!(
            a.account,
            crate::Account::SignedIn { granted: true, .. }
        ));
        assert!(a.client.is_some() && !a.sign_out_pending);
    }
    #[test]
    fn a_second_sign_in_result_during_the_save_step_is_revoked_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        let _ = a.update(Message::SignIn);
        let session = crate::session::sample();
        let first = Message::SignedIn(Ok((session.clone(), "oaiapp_1".into())));
        let _ = a.update(first);
        assert!(a.saving_sign_in());
        let before = snapshot(&a);
        let other = crate::session::Session {
            name: "Other".into(),
            refresh_token: "refresh-other".into(),
            ..crate::session::sample()
        };
        // The only task is the revoke of the unused session.
        let late = Message::SignedIn(Ok((other, "oaiapp_other".into())));
        assert_eq!(a.update(late).units(), 1);
        assert_eq!(snapshot(&a), before);
        assert_eq!(a.settings.client_id, "oaiapp_1");
        let saved = crate::settings::load(&dir.path().join("settings.json"));
        assert_eq!(saved.client_id, "oaiapp_1");
        assert!(a.saving_sign_in());
        // A late failure neither ends the save step nor shows a message.
        let late = Message::SignedIn(Err("The browser sign-in failed.".into()));
        assert_eq!(a.update(late).units(), 0);
        assert_eq!(snapshot(&a), before);
        assert!(a.saving_sign_in());
        // The first save still signs in.
        let _ = a.update(Message::SessionSaved(Ok(session)));
        assert!(matches!(
            a.account,
            crate::Account::SignedIn { granted: true, .. }
        ));
        assert_eq!(a.settings.client_id, "oaiapp_1");
    }
    #[test]
    fn a_save_result_is_ignored_unless_the_save_step_is_running() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_app(&dir);
        let _ = a.update(Message::SignOut);
        let _ = a.update(Message::SignedOut(Ok(())));
        let signed_out = snapshot(&a);
        let late = Message::SessionSaved(Ok(crate::session::sample()));
        assert_eq!(a.update(late).units(), 0);
        let late = Message::SessionSaved(Err("Could not save it.".into()));
        assert_eq!(a.update(late).units(), 0);
        assert_eq!(snapshot(&a), signed_out);
        assert!(a.client.is_none() && !a.sign_out_pending);
        // While the browser step is still running there is nothing to save yet.
        let _ = a.update(Message::SignIn);
        let late = Message::SessionSaved(Ok(crate::session::sample()));
        assert_eq!(a.update(late).units(), 0);
        assert_eq!(a.account, crate::Account::SigningIn);
        assert!(a.client.is_none() && a.sign_in.is_some());
        let late = Message::SessionSaved(Err("Could not save it.".into()));
        assert_eq!(a.update(late).units(), 0);
        assert_eq!(a.account, crate::Account::SigningIn);
        assert!(a.account_note.is_empty());
    }
    #[test]
    fn signing_in_again_while_signed_in_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_app(&dir);
        let epoch = a.account_epoch;
        let listed = vec!["gpt-6-luna".to_string(), "gpt-6.1-sol".to_string()];
        let _ = a.update(Message::ModelsLoaded(epoch, Ok(listed)));
        let _ = a.update(Message::PickModel("gpt-6-luna".into()));
        let before = snapshot(&a);
        assert_eq!(a.update(Message::SignIn).units(), 0);
        assert_eq!(snapshot(&a), before);
        assert!(a.sign_in.is_none());
    }
    #[test]
    fn a_sign_in_result_while_signed_in_is_revoked_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_app(&dir);
        let epoch = a.account_epoch;
        let listed = vec!["gpt-6-luna".to_string(), "gpt-6.1-sol".to_string()];
        let _ = a.update(Message::ModelsLoaded(epoch, Ok(listed)));
        let before = snapshot(&a);
        let other = crate::session::Session {
            name: "Other".into(),
            refresh_token: "refresh-other".into(),
            ..crate::session::sample()
        };
        // The only task is the revoke of the unused session.
        let late = Message::SignedIn(Ok((other.clone(), "oaiapp_other".into())));
        assert_eq!(a.update(late).units(), 1);
        assert_eq!(snapshot(&a), before);
        assert_eq!(a.settings.client_id, "oaiapp_1");
        let saved = crate::settings::load(&dir.path().join("settings.json"));
        assert_eq!(saved.client_id, "oaiapp_1");
        assert_eq!(a.update(Message::SessionSaved(Ok(other))).units(), 0);
        assert_eq!(snapshot(&a), before);
    }
    #[test]
    fn both_the_missing_plan_usage_and_the_unsaved_settings_are_shown() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("settings.json")).unwrap();
        let mut a = fresh_app(&dir);
        sign_in_with(&mut a, &dir, without_plan_usage());
        assert!(matches!(
            a.account,
            crate::Account::SignedIn { granted: false, .. }
        ));
        let note = a.account_note.as_str();
        let first = note.find("not granted").expect(note);
        let second = note
            .find("Signed in for this session, but settings could not be saved: ")
            .expect(note);
        assert!(first < second, "{note}");
    }
    #[test]
    #[should_panic(expected = "tests must use an isolated workspace")]
    fn tests_cannot_open_the_real_workspace() {
        let _ = App::from_workspace(crate::storage::state_path(), None);
    }
}
