//! Explicit renderer smoke test. It uses an isolated data directory and never
//! sends native input events or changes the user's regular saved workspace.
use crate::{App, Message, Page, Target, model::Machine};
use iced::Task;
use std::fs;
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
            7 => vec![
                Message::Page(Page::Ai),
                Message::AiCoverage("Top, bottom and side".into()),
                Message::Plan,
            ],
            8 => vec![Message::ApplyPlan, Message::Page(Page::Post)],
            9 => {
                self.doc = crate::model::Document::sample();
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
                self.replace_document(crate::model::Document::default(), None);
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
        assert!(a.selected.is_none() && a.plan.is_empty() && a.undo.is_none());
        assert_eq!(a.doc.machines[0].id, machine);
        assert_eq!(a.doc.tools[0].id, tool);
        let archive = crate::storage::load(a.last_archived.as_ref().unwrap()).unwrap();
        assert_eq!(archive.title, "Unsaved custom job");
        assert_eq!(archive.setups[0].operations.len(), 4);
        let _ = a.update(Message::RecoverPrevious);
        assert_eq!(a.doc.title, "Unsaved custom job");
        assert!(a.is_sample());
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
        assert!(a.plan.is_empty());
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
    fn ai_apply_and_undo_preserve_job() {
        let mut a = app();
        let before = a.doc.setup().id.clone();
        let _ = a.update(Message::Plan);
        let _ = a.update(Message::ApplyPlan);
        assert_eq!(a.doc.setups.len(), 2);
        assert_ne!(a.doc.setup().id, before);
        let _ = a.update(Message::UndoPlan);
        assert_eq!(a.doc.setup().id, before)
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
            "Your ChatGPT plan lists no models for this app."
        );
        // The panel offers Load models; a new list replaces the note.
        assert_eq!(a.update(Message::LoadModels).units(), 1);
        let _ = a.update(Message::ModelsLoaded(epoch, Ok(both.clone())));
        assert_eq!(a.models, both);
        assert_eq!(a.settings.model, "gpt-6-luna");
        assert!(a.account_note.is_empty());
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
