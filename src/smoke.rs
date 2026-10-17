//! Explicit renderer smoke test. It uses an isolated data directory and never
//! sends native input events or changes the user's regular saved workspace.
//!
//! Each screen is captured from a frame the window drew after the step that
//! set it up (`frames`, `App::smoke_frame`), and checked against the screen
//! before it (`App::smoke_check`): Iced's screenshot renders what the window
//! last drew, and a window that draws no frame (on Wayland, one that is
//! covered) would otherwise give the previous screen again, with its labels
//! gone (#90).
use crate::{
    App, Message, Page, Target,
    model::Machine,
    planner::{Progress, StepResult},
};
use iced::Task;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
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
            looks: vec![],
        })
        .collect()
}
/// The start of a reply being written, as the running screens show it.
const SMOKE_REPLY: &str = "{\"summary\":\"Face the top, rough the pocket with T3, then turn the part over.\",\"setups\":[{\"orientation\":\"+Z\",\"name\":\"Top\",\"origin\":\"Stock top center\",\"workholding\":\"Vise\",\"operations\":[{\"strategy\":\"Face\",\"name\":\"Face top\",\"tool\":1";
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
/// Whether the smoke run is waiting for frames: the window's frames become
/// messages only then, since each message has Iced draw the window again.
static WAITING: AtomicBool = AtomicBool::new(false);
/// The frames drawn after a step before its screen is captured. The first
/// shows the step; pictures of 2 MiB and more are uploaded after it.
const FRAMES: u32 = 2;
/// How long a step's frame may take to come before the run fails.
const FRAME_WAIT: Duration = Duration::from_secs(15);
/// The sharp edges per 1,000 drawn pixels, at the window's scale, under
/// which a screen has no text: labelled screens measure 15.9 (the Tools page,
/// screen 07) to 120, the stale frames of #90 3.5 to 3.8, and one that kept
/// some of its labels 8.7.
const TEXT_EDGES: f64 = 12.;
/// The window's frames and sizes, for the smoke run: a message for each
/// frame drawn while the run waits for one, and for each size the window
/// takes.
pub fn frames() -> iced::Subscription<Message> {
    use iced::window::Event;
    iced::event::listen_raw(|event, _status, _window| match event {
        iced::Event::Window(Event::RedrawRequested(at)) if WAITING.load(Ordering::Relaxed) => {
            Some(Message::Frame(at))
        }
        iced::Event::Window(Event::Opened { size, .. } | Event::Resized(size)) => {
            Some(Message::WindowSized(size))
        }
        _ => None,
    })
}
/// The marks of a drawn screen in a capture: the sharp edges (a luminance
/// step over 80 between neighbouring pixels), which text makes in numbers,
/// per 1,000 drawn pixels, and the share of pixels that are drawn (not the
/// window's background). Every other row is read. A glyph's edges grow with
/// the window's scale factor and its pixels with the square of it, so the
/// edge count is taken at the scale.
fn smoke_marks(image: &iced::window::Screenshot) -> (f64, f64) {
    let (w, h) = (image.size.width as usize, image.size.height as usize);
    let (mut edges, mut drawn, mut seen) = (0usize, 0usize, 0usize);
    for y in (0..h).step_by(2) {
        let mut previous = None;
        for x in 0..w {
            let [r, g, b] = <[u8; 3]>::try_from(&image.rgba[(y * w + x) * 4..][..3]).unwrap();
            let luminance = (i32::from(r) * 299 + i32::from(g) * 587 + i32::from(b) * 114) / 1000;
            if previous.is_some_and(|p: i32| (luminance - p).abs() > 80) {
                edges += 1;
            }
            previous = Some(luminance);
            if (i32::from(r) - 242).abs() > 12
                || (i32::from(g) - 245).abs() > 12
                || (i32::from(b) - 249).abs() > 12
            {
                drawn += 1;
            }
            seen += 1;
        }
    }
    let scale = f64::from(image.scale_factor);
    (
        edges as f64 * scale * 1000. / drawn.max(1) as f64,
        drawn as f64 / seen.max(1) as f64,
    )
}
impl App {
    /// The name of the screen captured at `smoke_step`.
    fn smoke_name(&self) -> String {
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
        names
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
                111 => "88-workholding-vise".into(),
                112 => "89-workholding-clamps".into(),
                113 => "90-workholding-page".into(),
                // Iced draws the pictures of this screen from the second frame
                // on (it uploads images of 2 MiB and more after the first),
                // so it is captured twice: the second file replaces the first.
                114 | 115 => "91-planner-pictures".into(),
                116 => "92-flipped-setup-leftover-stock".into(),
                117 => "93-planner-tool-lines".into(),
                118 => "94-machine-disconnected".into(),
                119 => "95-machine-estop".into(),
                120 => "96-machine-idle".into(),
                121 => "97-machine-run-panel".into(),
                122 => "98-machine-setup".into(),
                123 => "99-machine-work-zero".into(),
                124 => "100-manual-steps".into(),
                _ => format!("motion-{:02}", self.smoke_step.saturating_sub(26)),
            })
    }
    /// Starts the wait for the frames that show the step just taken.
    pub fn smoke_arm(&mut self) {
        self.smoke_frames = 0;
        self.smoke_changed = Instant::now();
        WAITING.store(true, Ordering::Relaxed);
    }
    /// The window drew a frame at `at`. The `FRAMES`th one drawn after the
    /// last step is captured. Frames drawn before the step, while the
    /// window takes a new size or while the stock surface is still being
    /// meshed count for nothing: the screen is not settled yet.
    pub fn smoke_frame(&mut self, at: Instant) -> Task<Message> {
        if self.smoke_dir.is_none() || self.smoke_pending || !WAITING.load(Ordering::Relaxed) {
            return Task::none();
        }
        if at < self.smoke_changed {
            return Task::none();
        }
        if self.smoke_resize || self.stock_mesh_key().is_some() {
            self.smoke_frames = 0;
            self.smoke_changed = Instant::now();
            return Task::none();
        }
        self.smoke_frames += 1;
        if self.smoke_frames < FRAMES {
            return Task::none();
        }
        self.smoke_pending = true;
        WAITING.store(false, Ordering::Relaxed);
        iced::window::latest()
            .and_then(iced::window::screenshot)
            .map(Message::Captured)
    }
    /// The window took a logical size.
    pub fn smoke_sized(&mut self, size: iced::Size) {
        if self.smoke_dir.is_none() {
            return;
        }
        eprintln!("Window {} × {}", size.width, size.height);
        self.smoke_window = Some(size);
        if self.smoke_resize {
            self.smoke_resize = false;
            self.smoke_frames = 0;
            self.smoke_changed = Instant::now();
        }
    }
    /// Fails the run when the frame a step waits for has not come in
    /// `FRAME_WAIT`, naming the screen; the tick calls it, before it notes
    /// its time.
    pub fn smoke_overdue(&mut self) {
        if self.smoke_pending || !WAITING.load(Ordering::Relaxed) {
            return;
        }
        // A tick long after the one before it says the main thread was busy
        // with work a tick does in line in the smoke run (the stock the
        // setups leave, the setup's checks), during which no frame could be
        // drawn: the wait starts again.
        if self.last_tick.elapsed() > Duration::from_secs(2) {
            self.smoke_changed = Instant::now();
            return;
        }
        let waited = self.smoke_changed.elapsed();
        if waited <= FRAME_WAIT {
            return;
        }
        let what = if self.smoke_resize {
            "the window has not taken its new size".to_string()
        } else {
            format!(
                "the window drew {} frame{} after its step",
                self.smoke_frames,
                if self.smoke_frames == 1 { "" } else { "s" }
            )
        };
        panic!(
            "Screen {}: {what} in {:.0} s. The smoke run captures each screen from a frame drawn after its step, so its window must stay visible and unoccluded on screen (on Wayland a covered window draws no frames).",
            self.smoke_name(),
            waited.as_secs_f64()
        );
    }
    /// Refreshes the connection until the machine's snapshot satisfies `ok`,
    /// or `secs` have passed: whether it did.
    fn smoke_machine(
        &mut self,
        secs: f64,
        ok: impl Fn(&uc100::controller::Snapshot) -> bool,
    ) -> bool {
        let end = Instant::now() + Duration::from_secs_f64(secs);
        loop {
            let c = self.control.as_mut().expect("connected");
            c.refresh();
            if ok(&c.snapshot) {
                return true;
            }
            if Instant::now() > end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    /// The machine's state and its last log lines, for a failure.
    fn smoke_machine_state(&self) -> String {
        let s = &self.control.as_ref().expect("connected").snapshot;
        format!(
            "{:?}, log: {:?}",
            s.state,
            s.log.iter().rev().take(5).collect::<Vec<_>>()
        )
    }
    /// The checks on a captured screen: it is not the frame the screen before
    /// it was captured from, it is drawn, and it has text. A capture made
    /// before the window drew the step's frame renders the previous screen,
    /// and most of its labels are gone from it: their paragraphs belong to
    /// the widget tree, which each message rebuilds (#90).
    fn smoke_check(&mut self, name: &str, image: &iced::window::Screenshot) {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (image.size.width, image.size.height, &image.rgba).hash(&mut hasher);
        let hash = hasher.finish();
        // Screen 91 is captured twice by design, the second file replacing
        // the first; the second capture may equal the first.
        assert!(
            self.smoke_step == 115 || self.smoke_last != Some(hash),
            "Screen {name} is the same frame as the screen before it: a stale capture (the window drew no new frame)"
        );
        self.smoke_last = Some(hash);
        let (edges, drawn) = smoke_marks(image);
        assert!(
            drawn > 0.02,
            "Screen {name} is blank: {:.1} % of its pixels differ from the background",
            drawn * 100.
        );
        assert!(
            edges > TEXT_EDGES,
            "Screen {name} has no text: {edges:.1} sharp edges per 1,000 drawn pixels, labelled screens have over {TEXT_EDGES}"
        );
        eprintln!(
            "Screen {name}: {} × {} px, {edges:.1} sharp edges per 1,000 drawn pixels, {:.0} % drawn, {} frame{} after the step",
            image.size.width,
            image.size.height,
            drawn * 100.,
            self.smoke_frames,
            if self.smoke_frames == 1 { "" } else { "s" }
        );
    }
    pub fn capture_smoke(&mut self, image: iced::window::Screenshot) -> Task<Message> {
        let Some(dir) = self.smoke_dir.clone() else {
            return Task::none();
        };
        fs::create_dir_all(&dir).expect("create smoke output");
        let name = self.smoke_name();
        self.smoke_check(&name, &image);
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
        self.smoke_pending = false;
        let task = self.smoke_advance(&dir);
        self.smoke_arm();
        task
    }
    /// Takes the next step: sets up the next screen, in `dir`, the smoke's
    /// workspace. The screen is captured once the window has drawn it.
    fn smoke_advance(&mut self, dir: &std::path::Path) -> Task<Message> {
        self.smoke_step += 1;
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
                self.smoke_resize = true;
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
                    path: path.clone(),
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
                        path: path.clone(),
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
                    Message::SetupStep("Stock"),
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
                self.smoke_resize = true;
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
                *self.plan_stream.lock().unwrap() = SMOKE_REPLY.into();
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
                // The first request looked at two pictures: its row ends
                // "· 2 looks".
                self.plan_timeline[1].looks = vec![
                    self.smoke_look("top", "Which face is the pocket floor"),
                    self.smoke_look("iso_back_right", "The far side of the part"),
                ];
                self.page = Page::Ai;
                vec![]
            }
            111 => {
                // The vise: setup 1 held by the starter vise, drawn round the
                // stock with its controls in the setup editor. The layout is
                // suggested from the operations' toolpaths.
                self.planner = crate::Planner::Idle;
                self.plan_timeline.clear();
                self.stock = true;
                self.camera = crate::viewport::Camera::default();
                // The example's document starts with an empty library.
                self.doc.adopt_holders();
                self.holder_draft = crate::holder_draft(&self.doc);
                vec![
                    Message::Page(Page::Work),
                    Message::SelectSetup(0),
                    Message::SelectOp(0),
                    Message::Edit(Target::Setup, "holding", "Vise".into()),
                    Message::SetupEditor,
                    Message::SetupStep("Holding"),
                ]
            }
            112 => {
                // Clamps, drawn the same way; the facing preview's toolpath
                // comes within the gap of them, which its warning says in the
                // inspector.
                self.camera = crate::viewport::Camera::default();
                vec![
                    Message::Edit(Target::Setup, "holding", "Clamps".into()),
                    Message::SelectOp(0),
                    Message::Tab("Passes".into()),
                    Message::Generate,
                    Message::Scrub(0.5),
                ]
            }
            113 => vec![Message::Page(Page::Workholding)],
            114 => {
                // The first request still going: the overview the run started
                // from, the model's two looks (a picture and a view it could
                // not get), then OpenAI refusing the tool, and the plan as far
                // as it is written.
                use Progress::*;
                self.plan_model = self.settings.model.clone();
                self.planner = crate::Planner::Running(Asking(1));
                self.plan_timeline = smoke_steps(vec![
                    (Analyzing, 41, Some(39), None),
                    (Asking(1), 39, None, None),
                ]);
                self.plan_timeline[1].looks = vec![
                    self.smoke_look("top", "Which face is the pocket floor"),
                    self.smoke_failed_look(),
                    crate::Shown {
                        line: crate::ui::refusal(crate::planner::Dropped::Tool).0.into(),
                        warn: true,
                        thumb: None,
                    },
                ];
                self.plan_overview = Some(self.smoke_overview());
                *self.plan_stream.lock().unwrap() = SMOKE_REPLY.into();
                self.page = Page::Ai;
                vec![]
            }
            // The same screen again, with its pictures uploaded.
            115 => vec![],
            116 => {
                // Setup 1, on stock 4 mm wider than the part all round, faces
                // the 10 mm above the part and contours its walls down to its
                // underside; setup 2 turns the part over and is held in the
                // starter vise. The setup editor shows the stock setup 2
                // starts from, in the panel and drawn: as wide as the block,
                // for the 3 mm left under the part, now its top. The jaws
                // start at its bottom and close on the walls the Contour
                // finished, under that rim.
                use crate::model::{Setup, Strategy};
                // The run's last check of the earlier screens: the operation
                // of the Workholding screens is still previewed.
                assert!(self.active_simulation().is_some(), "{}", self.status);
                self.planner = crate::Planner::Idle;
                self.plan_timeline.clear();
                self.plan_overview = None;
                self.stock = true;
                self.paths = false;
                self.camera = crate::viewport::Camera::default();
                self.doc.setups.truncate(1);
                let mut contour =
                    crate::model::Operation::new(Strategy::Contour, self.doc.tools[0].id.clone());
                contour.geometry.clear();
                let first = &mut self.doc.setups[0];
                first.operations.retain(|o| o.strategy == Strategy::Face);
                first.operations.push(contour);
                for key in ["stock_left", "stock_right", "stock_front", "stock_back"] {
                    first.p.insert(key.into(), "4".into());
                }
                let mut setup = Setup::new(self.doc.setups[0].machine.clone(), "G55");
                setup.p.insert("name".into(), "Flipped underside".into());
                setup.p.insert("rx".into(), "180".into());
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
                // The smoke run's ticks only take screenshots, so the stock the
                // earlier setups leave is worked out here, in line.
                let _ = self.refresh_left_stock();
                vec![
                    Message::SelectSetup(1),
                    Message::Edit(Target::Setup, "holding", "Vise".into()),
                    Message::SetupEditor,
                    Message::SetupStep("Stock"),
                ]
            }
            117 => {
                // The planner at its first request, going back to the plan:
                // the tools the model has called, one line each, as the
                // answers of the tools themselves read.
                use Progress::*;
                self.doc.setups.truncate(1);
                self.doc.active = self.doc.setups[0].id.clone();
                self.plan_model = self.settings.model.clone();
                self.planner = crate::Planner::Running(Asking(1));
                self.plan_timeline = smoke_steps(vec![
                    (Analyzing, 52, Some(50), None),
                    (Asking(1), 50, None, None),
                ]);
                self.plan_timeline[1].looks = self
                    .smoke_tool_lines()
                    .into_iter()
                    .map(|line| crate::Shown {
                        line: crate::ui::look_line(&crate::planner::Look {
                            description: line,
                            purpose: String::new(),
                            png: None,
                        }),
                        warn: false,
                        thumb: None,
                    })
                    .collect();
                *self.plan_stream.lock().unwrap() = SMOKE_REPLY.into();
                self.page = Page::Ai;
                vec![]
            }
            118 => {
                // MACHINE: the Control page before Connect. The owner's
                // machine, with the driver's JY5800 profile written to the
                // smoke's workspace, is simulated and assigned to the setup.
                self.planner = crate::Planner::Idle;
                self.plan_timeline.clear();
                let mut m = crate::control::owners_machine(dir).expect("the owner's machine");
                m.p.insert("uc100_connection".into(), "Simulated".into());
                self.doc.setups[0].machine = m.clone();
                self.doc.machines.push(m);
                vec![Message::Page(Page::Control)]
            }
            119 => {
                // Connected: the simulated machine starts in E-stop. The tick
                // puts the state in the status line.
                use uc100::controller::State;
                let _ = self.update(Message::Connect);
                assert!(self.control.is_some(), "Connect: {}", self.status);
                assert!(
                    self.smoke_machine(10., |s| s.state == Some(State::EStop)),
                    "E-stop after connecting: {}",
                    self.smoke_machine_state()
                );
                vec![Message::Tick]
            }
            120 => {
                // Reset to Idle, with machine zero set where the machine
                // starts: the model is drawn with its travel and the
                // simulator's block.
                use uc100::controller::State;
                let _ = self.update(Message::MachineReset);
                assert!(
                    self.smoke_machine(10., |s| s.state == Some(State::Idle)),
                    "Idle after Reset: {}",
                    self.smoke_machine_state()
                );
                // Pressed again while an axis stays unset: the driver can
                // drop a declaration (LIO-163), as the tests allow for.
                let set = |s: &uc100::controller::Snapshot| s.referenced[..3].iter().all(|r| *r);
                for _ in 0..5 {
                    let _ =
                        self.update(Message::Machine(crate::control::Action::MachineZero(None)));
                    if self.smoke_machine(1., set) {
                        break;
                    }
                }
                assert!(
                    self.smoke_machine(1., set),
                    "machine zero: {}",
                    self.smoke_machine_state()
                );
                vec![Message::Tick]
            }
            121 => {
                // The setup picked in the Run panel, checked here (the smoke
                // run leaves the tasks its steps return alone), with the Run
                // strip above it.
                let _ = self.update(Message::Run(crate::run::Action::Setup(0)));
                let epoch = self
                    .control
                    .as_ref()
                    .and_then(|c| c.run.as_ref())
                    .expect("the setup picked to run")
                    .epoch;
                let mesh = self.part_mesh().expect("the imported part");
                let programs = crate::run::programs(&self.doc, mesh, 0);
                eprintln!(
                    "Screen 97: {}",
                    match &programs {
                        Ok(runs) => format!("{} tool runs", runs.len()),
                        Err(problems) => problems.join(" | "),
                    }
                );
                let _ = self.update(Message::RunChecked(epoch, std::sync::Arc::new(programs)));
                vec![]
            }
            // The Setup view: the profile of the connected machine.
            122 => vec![Message::ControlSetup(true)],
            // Back on the Control page, the work zero under the jog: each
            // axis's DRO line with Zero, ½ and Set, a value typed for X, and
            // the probes. No setup picked: the one of screen 97 cannot run,
            // and the fixed Run strip listing why leaves little of the page.
            123 => {
                if let Some(c) = &mut self.control {
                    c.run = None;
                }
                vec![
                    Message::ControlSetup(false),
                    Message::Machine(crate::control::Action::WorkText(0, "-5".into())),
                ]
            }
            // MACHINE on a setup of the manual mill put first: its Steps
            // page, with the CNC still connected.
            124 => {
                self.smoke_manual_setup();
                vec![Message::Page(Page::Control)]
            }
            _ => {
                assert!(self.doc.validate_structure().is_ok());
                self.disconnect_machine();
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
        match self.smoke_step {
            124 => {
                let line = self.smoke_manual_check();
                eprintln!("Screen 100: {line}");
            }
            // The machine drawn under the work zero, in view.
            120 => {
                return iced::widget::operation::scroll_to(
                    "control",
                    iced::widget::scrollable::AbsoluteOffset { x: 0., y: 970. },
                );
            }
            // The work zero under the position table and the jog.
            123 => {
                return iced::widget::operation::scroll_to(
                    "control",
                    iced::widget::scrollable::AbsoluteOffset { x: 0., y: 420. },
                );
            }
            116 => {
                // The stock setup 2 starts from is the one worked out, and the
                // jaws are drawn round it.
                let _ = self.refresh_left_stock();
                let (stock, current) = self.known_stock(1);
                let stock = stock.expect("setup 2's stock");
                assert!(current, "the stock before setup 2 was not worked out");
                let original = self.doc.setup_stock(1, self.part_mesh()).unwrap();
                assert!(
                    stock.dimensions()[2] < original.dimensions()[2] - 1.,
                    "{stock:?} against {original:?}"
                );
                eprintln!(
                    "Screen 92: setup 2 starts from {}, its original box is {}",
                    crate::stock::dimensions_text(stock.dimensions()),
                    crate::stock::dimensions_text(original.dimensions())
                );
                assert_eq!(crate::model::get(&self.doc.setup().p, "holding"), "Vise");
                assert!(crate::ui::stock_note(self).is_none());
                // The 3D view draws that box as the stock.
                assert_eq!(crate::ui::leftover_view(self), Some(stock));
                // The jaws close on the walls: the band they grip is
                // narrower across them than the box, and they hold it.
                let held = self.known_held(1).expect("setup 2's holding");
                let axis = usize::from(crate::model::get(&self.doc.setup().p, "vise_axis") == "Y");
                let (band, whole) = (held.band.dimensions()[axis], stock.dimensions()[axis]);
                assert!(band < whole - 7., "{band} across the jaws of {whole}");
                let jaws = crate::ui::holding_boxes(self);
                assert_eq!(jaws.len(), 2, "{}", self.status);
                assert!(
                    jaws.iter()
                        .all(|j| (j.bounds.max[axis] - held.band.min[axis]).abs() < 1e-3
                            || (j.bounds.min[axis] - held.band.max[axis]).abs() < 1e-3),
                    "{jaws:?} {held:?}"
                );
                assert_eq!(crate::ui::holding_lines(self), Vec::<String>::new());
                eprintln!(
                    "Screen 92: the jaws close on {band:.3} mm of the box's {whole:.3} mm across them. {}",
                    self.status
                );
            }
            // The overview's picture at the top of the visible part, so the
            // lines of the open request fit under it.
            115 => {
                return iced::widget::operation::scroll_to(
                    "planner-result",
                    iced::widget::scrollable::AbsoluteOffset { x: 0., y: 90. },
                );
            }
            111 => {
                assert_eq!(crate::model::get(&self.doc.setup().p, "holding"), "Vise");
                assert_eq!(crate::ui::holding_boxes(self).len(), 2, "{}", self.status);
                self.paths = false;
            }
            112 => {
                assert_eq!(crate::model::get(&self.doc.setup().p, "holding"), "Clamps");
                assert!(
                    !crate::ui::holding_boxes(self).is_empty(),
                    "{}",
                    self.status
                );
                assert!(
                    crate::ui::holding_warning(self).is_some(),
                    "no warning on screen 89: {}",
                    self.status
                );
                // The warning is under the operation's fields: scroll to it.
                return iced::widget::operation::scroll_to(
                    "inspector",
                    iced::widget::scrollable::AbsoluteOffset { x: 0., y: 5000. },
                );
            }
            _ => {}
        }
        if self.smoke_step >= 50 {
            // Screen 82 scrolls the planner's result to its end; a scrollable
            // keeps that offset in the pages that follow (the Materials form
            // shares its place in the tree), so each screen starts at the top.
            let top = |id| {
                iced::widget::operation::scroll_to(
                    id,
                    iced::widget::scrollable::AbsoluteOffset { x: 0., y: 0. },
                )
            };
            return Task::batch([top("inspector"), top("planner-result"), top("control")]);
        }
        Task::none()
    }
    /// Screen 100's job: a manual mill in the library with a Ø25 end mill
    /// that fits it, and a setup on it put first that faces 2 mm off the
    /// whole top, made the active setup.
    pub(crate) fn smoke_manual_setup(&mut self) {
        use crate::model::{BOX_FIELDS, Operation, Strategy, Tool};
        let machine = crate::manual::defaults();
        let mut mill = Tool::new("End mill 25", "Flat end mill", "25");
        for (k, v) in [
            ("length", "60"),
            ("stickout", "65"),
            ("machines", &machine.id),
        ] {
            mill.p.insert(k.into(), v.into());
        }
        let mut face = Operation::new(Strategy::Box, mill.id.clone());
        face.p.insert("name".into(), "Face".into());
        for (k, v) in BOX_FIELDS
            .iter()
            .zip(["-500", "500", "-500", "500", "0", "-2"])
        {
            face.p.insert((*k).into(), v.into());
        }
        face.p.insert("allowance".into(), "0".into());
        crate::manual::snap(&machine, &mut face.p);
        self.doc.machines.push(machine);
        self.doc.tools.push(mill);
        self.doc.add_manual_setup().expect("a manual setup");
        self.doc.setups[0].operations = vec![face];
        self.doc.active = self.doc.setups[0].id.clone();
    }
    /// What screen 100 shows: the Steps page on the manual setup, its
    /// guide built (set up, the face, the hand-over) at its first step.
    pub(crate) fn smoke_manual_check(&self) -> String {
        use crate::guide::Step;
        assert!(self.steps_page(), "the Steps page");
        let (shown, at) = self.guide_at().unwrap_or_else(|| {
            panic!(
                "the guide: {:?}",
                self.current_guide().map(|r| r.as_ref().err())
            )
        });
        let g = &shown.guide;
        let names: Vec<&str> = g.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Set up", "Face", "Hand over"]);
        assert_eq!(at, 0);
        assert!(matches!(g.steps[0], Step::Hold { .. }), "{:?}", g.steps[0]);
        let Some(Step::Face { to, remove, .. }) =
            g.steps.iter().find(|s| matches!(s, Step::Face { .. }))
        else {
            panic!("no Face step: {:?}", g.steps)
        };
        assert!((to + 2.).abs() < 1e-3 && *remove > 0., "{to} {remove}");
        format!(
            "{} steps, face to Z {to:.2}, {remove:.2} mm off, ~{:.0} min",
            g.steps.len(),
            g.minutes.ceil()
        )
    }
    /// What the planner's pictures are drawn from: the imported part as the
    /// planner reads it.
    fn smoke_scene(&self) -> crate::look::Scene {
        let mesh = self.imported_mesh.clone().expect("imported part");
        let report = crate::analysis::report(&self.doc, &mesh).expect("part report");
        crate::look::Scene {
            doc: std::sync::Arc::new(self.doc.clone()),
            mesh: std::sync::Arc::new(mesh),
            report: std::sync::Arc::new(report),
            job: None,
        }
    }
    /// A timeline picture as the run makes it: the PNG's handle, once.
    fn smoke_shown(picture: &crate::look::Picture, line: String) -> crate::Shown {
        crate::Shown {
            line,
            warn: false,
            thumb: Some(iced::widget::image::Handle::from_bytes(
                picture.png.to_vec(),
            )),
        }
    }
    /// The picture the first request carries, as the planner page shows it.
    fn smoke_overview(&self) -> crate::Shown {
        let picture = crate::look::overview(&self.smoke_scene()).expect("overview");
        Self::smoke_shown(&picture, picture.description.clone())
    }
    /// The arguments of a `view_model` call that looks at the whole part
    /// (or at `focus`, surface IDs) from `angle`.
    fn smoke_arguments(angle: &str, purpose: &str, focus: &[usize]) -> String {
        serde_json::json!({
            "orientation": null,
            "angle": angle,
            "focus_surfaces": focus,
            "highlight_surfaces": [],
            "show_stock": true,
            "show_workholding": true,
            "labels": true,
            "operation": null,
            "purpose": purpose,
        })
        .to_string()
    }
    /// A look the model asked for with the `view_model` tool, as the
    /// timeline shows it.
    fn smoke_look(&self, angle: &str, purpose: &str) -> crate::Shown {
        let arguments = Self::smoke_arguments(angle, purpose, &[]);
        let picture = crate::look::draw(&self.smoke_scene(), &arguments)
            .expect("a look at the imported part");
        let line = crate::ui::look_line(&crate::planner::Look {
            description: picture.description.clone(),
            purpose: picture.purpose.clone(),
            png: Some(picture.png.clone()),
        });
        Self::smoke_shown(&picture, line)
    }
    /// A look that cannot be drawn (a surface the part does not have), as
    /// the timeline shows the sentence the model is told.
    fn smoke_failed_look(&self) -> crate::Shown {
        let arguments = Self::smoke_arguments("top", "Zoom to the slot", &[999_999]);
        let message = crate::look::draw(&self.smoke_scene(), &arguments)
            .expect_err("surface 999999 is not on the imported part");
        crate::Shown {
            line: crate::ui::look_line(&crate::planner::Look {
                description: message,
                purpose: String::new(),
                png: None,
            }),
            warn: false,
            thumb: None,
        }
    }
    /// The timeline's lines for tool calls made on the imported part, in the
    /// order a model might make them: it checks a draft of two setups, asks for
    /// the stock before the second and for the starter vise on it, then looks
    /// up the cutting data of its tool and a surface. Each is the line the
    /// tool itself gives (`planner_tools::answer`).
    fn smoke_tool_lines(&self) -> Vec<String> {
        use crate::planner_tools::{self as tools, ToolState};
        use crate::proposal::{Proposal, ProposedOp, ProposedSetup};
        let scene = self.smoke_scene();
        let report = scene.report.clone();
        let plane = |up: &str, kind: &str| {
            let orientation = report.orientations.iter().find(|o| o.up == up);
            let found = orientation.and_then(|o| o.planes.iter().find(|p| p.kind == kind));
            found.expect("plane").surfaces.clone()
        };
        let flat = report
            .tools
            .iter()
            .find(|t| t.kind == "Flat end mill")
            .expect("flat tool")
            .number;
        let op = |strategy: &str, surfaces: Vec<usize>| ProposedOp {
            strategy: strategy.into(),
            name: strategy.into(),
            tool: flat,
            surfaces,
            rpm: 8000.,
            feed: 800.,
            coolant: "Flood".into(),
            reason: "smoke".into(),
            ..Default::default()
        };
        let up = report.orientations[0].up;
        let floor = plane(up, "floor");
        let proposal = Proposal {
            summary:
                "Face the top and clear the pocket, then turn the part over and face the underside."
                    .into(),
            setups: vec![
                ProposedSetup {
                    orientation: up.into(),
                    name: "Top".into(),
                    origin: "Stock top center".into(),
                    workholding: "Vise on the long sides.".into(),
                    holder: None,
                    operations: vec![
                        op("Face", plane(up, "top")),
                        op("Pocket", floor.clone()),
                        op("Contour", vec![]),
                    ],
                },
                ProposedSetup {
                    orientation: "-Z".into(),
                    name: "Underside".into(),
                    origin: "Model origin".into(),
                    workholding: "Turn over; the vise on the long sides.".into(),
                    holder: Some(1),
                    operations: vec![op("Face", plane("-Z", "top"))],
                },
            ],
            unmachined: vec![],
            warnings: vec![],
        };
        let mut state = ToolState::new(scene);
        [
            (
                tools::CHECK_PLAN,
                serde_json::json!({ "proposal": proposal }),
            ),
            (tools::STOCK_AFTER, serde_json::json!({ "setup": 2 })),
            (
                tools::CHECK_HOLDING,
                serde_json::json!({ "setup": 2, "holder": 1 }),
            ),
            (tools::CUTTING_DATA, serde_json::json!({ "tool": flat })),
            (
                tools::SURFACE_INFO,
                serde_json::json!({ "surface": floor[0] }),
            ),
        ]
        .into_iter()
        .map(|(name, arguments)| tools::answer(name, &arguments.to_string(), &mut state).line)
        .collect()
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
                holder: None,
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
    fn a_uc100_machine_needs_its_profile_file() {
        let mut m = Machine::new(3);
        m.p.insert("controller".into(), "UC100".into());
        assert!(m.issues().iter().any(|e| e.contains("UC100 profile")));
        m.p.insert("uc100_profile".into(), "C:/machines/jy5800.toml".into());
        assert!(m.issues().is_empty());
        assert!(Machine::new(3).issues().is_empty());
    }

    #[test]
    fn the_wizard_shows_the_uc100_fields_and_takes_a_picked_profile() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        let _ = a.update(Message::NewMachine);
        let _ = a.update(Message::WizardStep(3));
        let _ = a.update(Message::Edit(Target::Machine, "controller", "UC100".into()));
        let _ = a.view();
        let profile = dir.path().join("jy5800.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let _ = a.update(Message::Uc100ProfilePicked(Some(profile.clone())));
        assert_eq!(
            get(&a.machine_draft.p, "uc100_profile"),
            profile.display().to_string()
        );
        let _ = a.view();
        // Cancelling the file dialog keeps the chosen profile.
        let _ = a.update(Message::Uc100ProfilePicked(None));
        assert_eq!(
            get(&a.machine_draft.p, "uc100_profile"),
            profile.display().to_string()
        );
    }

    /// An app with one simulated UC100 machine whose profile is the driver's template.
    fn uc100_app(dir: &std::path::Path) -> App {
        profile_app(dir, uc100::config::TEMPLATE)
    }

    /// Like the JY5800: X, Y, Z with soft limits, no switches, a PWM spindle
    /// without a direction output, flood but no mist.
    const LIMITED: &str = r#"
[controller]
step_rate = "100kHz"
[axes.x]
steps_per_unit = 200.0
max_velocity = 2000.0
acceleration = 300.0
step = { pin = 2 }
dir = { pin = 3 }
soft_min = 0.0
soft_max = 390.0
[axes.y]
steps_per_unit = 200.0
max_velocity = 2000.0
acceleration = 300.0
step = { pin = 4 }
dir = { pin = 5 }
soft_min = 0.0
soft_max = 280.0
[axes.z]
steps_per_unit = 400.0
max_velocity = 1000.0
acceleration = 200.0
step = { pin = 6 }
dir = { pin = 7 }
soft_min = -55.0
soft_max = 0.0
[outputs]
spindle_pwm = { pin = 16 }
flood = { pin = 8 }
"#;

    /// X, Y and Z without soft limits (and without switches).
    const UNLIMITED: &str = r#"
[controller]
step_rate = "100kHz"
[axes.x]
steps_per_unit = 200.0
max_velocity = 2000.0
acceleration = 300.0
step = { pin = 2 }
dir = { pin = 3 }
[axes.y]
steps_per_unit = 200.0
max_velocity = 2000.0
acceleration = 300.0
step = { pin = 4 }
dir = { pin = 5 }
[axes.z]
steps_per_unit = 400.0
max_velocity = 1000.0
acceleration = 200.0
step = { pin = 6 }
dir = { pin = 7 }
"#;

    /// Home switches on every axis, a limit switch on X, a reversible
    /// spindle and mist.
    const SWITCHED: &str = r#"
[controller]
step_rate = "100kHz"
[axes.x]
steps_per_unit = 200.0
max_velocity = 2000.0
acceleration = 300.0
step = { pin = 2 }
dir = { pin = 3 }
home = { pin = 11 }
limit_neg = { pin = 15 }
[axes.y]
steps_per_unit = 200.0
max_velocity = 2000.0
acceleration = 300.0
step = { pin = 4 }
dir = { pin = 5 }
home = { pin = 12 }
[axes.z]
steps_per_unit = 400.0
max_velocity = 1000.0
acceleration = 200.0
step = { pin = 6 }
dir = { pin = 7 }
home = { pin = 13 }
home_negative = false
[outputs]
spindle_cw = { pin = 1 }
spindle_ccw = { pin = 14 }
mist = { pin = 9 }
"#;

    /// The connection of a connected test app.
    fn conn(a: &App) -> &crate::control::Connection {
        a.control.as_ref().expect("connected")
    }

    /// Connects, resets and waits for Idle.
    fn idle(a: &mut App) {
        use uc100::controller::State;
        let _ = a.update(Message::Connect);
        tick_until(a, 3.0, "E-stop after connecting", |s| {
            s.state == Some(State::EStop)
        });
        let _ = a.update(Message::MachineReset);
        tick_until(a, 3.0, "Idle after Reset", |s| s.state == Some(State::Idle));
    }

    /// An MDI line the driver refuses in every state, and how: busy while a
    /// line runs, unsupported otherwise.
    const MDI_MARK: &str = "G999";
    const MDI_MARK_REFUSED: [&str; 2] = ["MDI: machine is busy", "MDI: unsupported G999"];

    /// Runs one MDI line and waits until the machine is Idle again.
    fn mdi(a: &mut App, line: &str) {
        use uc100::controller::{Command, State};
        let seq = conn(a).snapshot.log_seq;
        conn(a).send(Command::Mdi(line.into()));
        // A line that runs may leave Idle for a single 4 ms driver loop, and
        // a refused one never leaves it. The driver takes commands in order,
        // so the refusal of a line sent after it says it has been taken up,
        // and Idle from then on says it is done.
        conn(a).send(Command::Mdi(MDI_MARK.into()));
        tick_until(a, 5.0, line, |s| {
            let new = usize::try_from(s.log_seq - seq).unwrap_or(usize::MAX);
            let taken = s
                .log
                .iter()
                .rev()
                .take(new)
                .any(|l| MDI_MARK_REFUSED.contains(&l.as_str()));
            taken && s.state == Some(State::Idle)
        });
    }

    /// An app with one simulated UC100 machine with the given profile.
    fn profile_app(dir: &std::path::Path, profile_text: &str) -> App {
        let mut a = App::from_workspace(dir.join("workspace.json"), None);
        let profile = dir.join("uc100.toml");
        std::fs::write(&profile, profile_text).unwrap();
        let mut m = Machine::new(3);
        m.p.insert("name".into(), "JY5800".into());
        m.p.insert("controller".into(), "UC100".into());
        m.p.insert("uc100_profile".into(), profile.display().to_string());
        m.p.insert("uc100_connection".into(), "Simulated".into());
        a.doc.machines.push(m);
        a
    }

    /// A wait's limit: `secs` of the driver's time. The simulated
    /// controller's timer counts the driver's 4 ms loops, which a loaded PC
    /// runs late while the simulated machine still moves in real time, so a
    /// limit in loops leaves a wait the time the load takes (#9). Wall time
    /// bounds it at ten times that, and a minute at least, should the driver
    /// stop.
    struct Deadline {
        loops: u64,
        seen: u64,
        last: Option<u16>,
        cap: std::time::Instant,
    }

    impl Deadline {
        fn new(secs: f64) -> Self {
            let wall = std::time::Duration::from_secs_f64(secs * 10.)
                .max(std::time::Duration::from_secs(60));
            Self {
                loops: (secs * 250.) as u64,
                seen: 0,
                last: None,
                cap: std::time::Instant::now() + wall,
            }
        }

        /// Counts the loops up to `s`; false once the limit has passed.
        fn within(&mut self, s: &uc100::controller::Snapshot) -> bool {
            if let Some(timer) = s.status.map(|st| st.timer) {
                if let Some(last) = self.last {
                    self.seen += u64::from(timer.wrapping_sub(last));
                }
                self.last = Some(timer);
            }
            self.seen <= self.loops && std::time::Instant::now() < self.cap
        }
    }

    /// Ticks until the machine snapshot satisfies `ok`; fails after `secs`
    /// of the driver's time (see `Deadline`).
    fn tick_until(
        a: &mut App,
        secs: f64,
        what: &str,
        ok: impl Fn(&uc100::controller::Snapshot) -> bool,
    ) {
        let mut deadline = Deadline::new(secs);
        loop {
            let _ = a.update(Message::Tick);
            let s = &a.control.as_ref().expect("connected").snapshot;
            if ok(s) {
                return;
            }
            assert!(
                deadline.within(s),
                "timed out waiting for {what}: {:?} {:?}",
                s.state,
                s.log
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn the_connected_machine_is_not_removed_and_disconnect_stays_on_the_control_page() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = uc100_app(dir.path());
        a.page = Page::Control;
        let id = a.doc.machines[0].id.clone();
        let _ = a.update(Message::Connect);
        assert!(a.control.is_some());
        let _ = a.update(Message::DeleteMachine(id.clone()));
        assert_eq!(a.doc.machines.len(), 1);
        assert_eq!(
            a.status,
            "Disconnect before removing the connected machine."
        );
        assert!(page_texts(&a).iter().any(|t| t == "Disconnect"));
        // A connection whose machine is gone from the library still ends
        // from the Control page.
        a.doc.machines.clear();
        assert!(a.control_target().is_none());
        assert!(page_texts(&a).iter().any(|t| t == "Disconnect"));
        let _ = a.update(Message::Disconnect);
        assert!(a.control.is_none());
        assert!(!page_texts(&a).iter().any(|t| t == "Disconnect"));
        // Disconnected, the machine can be removed again.
        a.doc.machines.push(Machine::new(3));
        let other = a.doc.machines[0].id.clone();
        let _ = a.update(Message::DeleteMachine(other));
        assert!(a.doc.machines.is_empty());
    }

    #[test]
    fn connects_to_the_simulated_uc100_resets_and_e_stops() {
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = uc100_app(dir.path());
        let _ = a.update(Message::Connect);
        assert!(a.control.as_ref().is_some_and(|c| c.simulated));
        tick_until(&mut a, 3.0, "E-stop after connecting", |s| {
            s.state == Some(State::EStop)
        });
        let identity = a
            .control
            .as_ref()
            .unwrap()
            .snapshot
            .identity
            .clone()
            .unwrap();
        assert_eq!(identity.hardware_id.as_deref(), Some("SIMULATED"));
        let _ = a.update(Message::MachineReset);
        tick_until(&mut a, 3.0, "Idle after Reset", |s| {
            s.state == Some(State::Idle)
        });
        let _ = a.update(Message::MachineEStop);
        tick_until(&mut a, 2.0, "E-stop", |s| s.state == Some(State::EStop));
        let _ = a.update(Message::Disconnect);
        assert!(a.control.is_none());
        // The simulator keeps its own saved state; the real machine's file is untouched.
        let id = a.doc.machines[0].id.clone();
        let state = dir.path().join("uc100");
        assert!(state.join(format!("{id}-simulated.toml")).exists());
        assert!(!state.join(format!("{id}.toml")).exists());
    }

    /// Like the JY5800 (X 0…390, Y 0…280, no switches, a PWM spindle without
    /// direction) but fast, with Z −80…0 for the fixture's drill, flood and
    /// no mist.
    const FAST: &str = r#"
[controller]
step_rate = "100kHz"
[motion]
junction_deviation = 0.02
arc_tolerance = 0.002
buffer_ms = 400
default_feed = 500
[axes.x]
steps_per_unit = 20.0
max_velocity = 60000.0
acceleration = 20000.0
step = { pin = 2 }
dir = { pin = 3 }
soft_min = 0.0
soft_max = 390.0
[axes.y]
steps_per_unit = 20.0
max_velocity = 60000.0
acceleration = 20000.0
step = { pin = 4 }
dir = { pin = 5 }
soft_min = 0.0
soft_max = 280.0
[axes.z]
steps_per_unit = 20.0
max_velocity = 30000.0
acceleration = 20000.0
step = { pin = 6 }
dir = { pin = 7 }
soft_min = -80.0
soft_max = 0.0
[outputs]
spindle_pwm = { pin = 16 }
flood = { pin = 8 }
"#;

    /// The fixture block's top setup on a simulated FAST machine whose AXIS
    /// CAM travel matches the profile: Facing (T1) then Drilling (T3), two
    /// tool runs.
    fn run_app(dir: &std::path::Path) -> App {
        run_app_on(dir, FAST)
    }

    fn run_app_on(dir: &std::path::Path, profile: &str) -> App {
        run_app_of(dir, profile, &[0, 3])
    }

    /// `run_app_on` with the fixture job's operations `ops` (by index):
    /// Face and Drill there.
    fn run_app_of(dir: &std::path::Path, profile: &str, ops: &[usize]) -> App {
        use crate::model::Strategy;
        let mut a = profile_app(dir, profile);
        let mut m = a.doc.machines.pop().unwrap();
        for (k, v) in [
            ("x", "390"),
            ("y", "280"),
            ("z", "80"),
            ("feed", "60000"),
            ("post_dwell", "0.2"),
        ] {
            m.p.insert(k.into(), v.into());
        }
        let (mut doc, mesh) = crate::post::fixture::job();
        let setup = &mut doc.setups[0];
        setup.operations = ops.iter().map(|&i| setup.operations[i].clone()).collect();
        for op in &mut setup.operations {
            op.p.insert("feed".into(), "30000".into());
            // A few seconds each: wide facing passes, holes without pecks.
            match op.strategy {
                Strategy::Face => {
                    op.p.insert("stepover".into(), "90".into());
                    op.p.insert("stepdown".into(), "5".into());
                }
                Strategy::Pocket | Strategy::Contour => {
                    op.p.insert("stepover".into(), "50".into());
                    op.p.insert("stepdown".into(), "10".into());
                }
                Strategy::Drill => {
                    op.p.insert("cycle".into(), "Drilling".into());
                    op.p.insert("coolant".into(), "Off".into());
                }
                _ => {}
            }
        }
        setup.machine = m.clone();
        doc.machines = vec![m];
        a.replace_document(doc, None);
        a.imported_mesh = Some(mesh);
        a.page = Page::Control;
        a
    }

    #[test]
    fn new_operations_take_the_coolant_the_machine_has() {
        for (profile, coolant) in [
            (FAST.to_string(), "Flood"),
            (FAST.replace("flood", "mist"), "Mist"),
            // The owner's JY5800 has neither output.
            (include_str!("../assets/jy5800.toml").to_string(), "Off"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut a = run_app_on(dir.path(), &profile);
            let _ = a.update(Message::AddOp(Strategy::Face));
            let op = &a.doc.setup().operations[a.selected.expect("added")];
            assert_eq!(get(&op.p, "coolant"), coolant, "{}", a.status);
        }
    }

    /// Idle with machine zero set where the machine starts and the work
    /// zero (the stock top centre) at machine X195 Y140 Z-20.
    fn zeroed(a: &mut App) {
        idle(a);
        zero(a);
    }

    /// Machine zero where the machine is, work zero at machine X195 Y140 Z-20.
    fn zero(a: &mut App) {
        let _ = a.update(Message::Machine(crate::control::Action::MachineZero(None)));
        tick_until(a, 3.0, "machine zero", |s| {
            s.referenced[..3].iter().all(|r| *r)
        });
        mdi(a, "G0 X195 Y140 Z-20");
        let _ = a.update(Message::Machine(crate::control::Action::WorkZero(None)));
        tick_until(a, 2.0, "work zero", |s| {
            s.work_pos[..3].iter().all(|v| v.abs() < 1e-6)
        });
    }

    /// Picks setup `si` to run and checks it here: tests do not run tasks.
    fn check_run(a: &mut App, si: usize) {
        let _ = a.update(Message::Run(crate::run::Action::Setup(si)));
        let epoch = run_of(a).epoch;
        let programs = crate::run::programs(&a.doc, a.part_mesh().unwrap(), si);
        let _ = a.update(Message::RunChecked(epoch, std::sync::Arc::new(programs)));
    }

    fn run_of(a: &App) -> &crate::run::Run {
        conn(a).run.as_ref().expect("a setup to run")
    }

    fn problems(a: &App) -> Vec<String> {
        run_of(a).problems(conn(a), &a.doc)
    }

    /// Ticks until the run satisfies `ok`; fails after `secs`.
    fn run_until(a: &mut App, secs: f64, what: &str, ok: impl Fn(&crate::run::Run) -> bool) {
        let mut deadline = Deadline::new(secs);
        loop {
            let _ = a.update(Message::Tick);
            let _ = a.view();
            if ok(run_of(a)) {
                return;
            }
            let s = &conn(a).snapshot;
            assert!(
                deadline.within(s),
                "timed out waiting for {what}: {:?} line {:?} at {:?} feed {} {:?} {:?}",
                s.state,
                s.current_line,
                s.machine_pos,
                s.feed_rate,
                run_of(a).outcome,
                s.log.iter().rev().take(5).collect::<Vec<_>>()
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn a_setup_runs_tool_run_by_tool_run_with_a_manual_tool_change() {
        use crate::run::Action as R;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        zeroed(&mut a);
        let _ = a.view();
        check_run(&mut a, 0);
        let _ = a.view();
        let programs = run_of(&a).checked().unwrap_or_else(|| {
            panic!(
                "{:?}",
                run_of(&a).programs.as_deref().map(|p| p.as_ref().err())
            )
        });
        assert_eq!(programs.len(), 2);
        assert_eq!((programs[0].ops[0].tool, programs[1].ops[0].tool), (1, 3));
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
        // Tool run 1: progress names the operation and block being cut.
        let _ = a.update(Message::Run(R::CycleStart));
        tick_until(&mut a, 2.0, "the job", |s| s.state == Some(State::Run));
        let _ = a.view();
        let (p, op, block) = run_of(&a).progress(conn(&a).snapshot.current_line).unwrap();
        assert!(op == 0 && block < p.blocks.len());
        run_until(&mut a, 30.0, "tool run 1", |r| r.done[0]);
        tick_until(&mut a, 2.0, "Idle", |s| s.state == Some(State::Idle));
        assert!(run_of(&a).outcome.as_ref().unwrap().0);
        assert_eq!(run_of(&a).next, 1);
        // It ended at the clearance plane with the spindle and coolant off.
        let s = conn(&a).snapshot.clone();
        assert_eq!(
            (s.spindle, s.flood),
            (Some(uc100::gcode::Spindle::Off), false)
        );
        let p = &run_of(&a).checked().unwrap()[0];
        let plane = crate::post::clearance_plane(&p.blocks) - p.datum[2];
        assert!(
            (s.work_pos[2] - plane as f64).abs() < 1e-3,
            "{}",
            s.work_pos[2]
        );
        // The drill is another tool: Cycle Start waits for its Z touch-off.
        assert!(run_of(&a).tool_change().is_some());
        let _ = a.view();
        assert!(
            problems(&a)
                .iter()
                .any(|p| p.contains("touch off its Z first")),
            "{:?}",
            problems(&a)
        );
        let _ = a.update(Message::Run(R::CycleStart));
        assert!(run_of(&a).running.is_none());
        // The old tool on the reference surface (here, where it stands), then
        // the new one 2 mm lower on the same surface: it is 2 mm longer.
        let _ = a.update(Message::Run(R::RecordReference));
        let reference = run_of(&a).reference.unwrap();
        assert!((reference - s.work_pos[2]).abs() < 1e-9);
        mdi(&mut a, &format!("G0 Z{:.3}", reference - 2.));
        let _ = a.update(Message::Run(R::ZFromReference));
        tick_until(&mut a, 2.0, "the touch-off", |s| {
            (s.work_offset[2] - (-22.)).abs() < 1e-6
        });
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
        // X/Y zero must stay: moving it holds Cycle Start back.
        let _ = a.update(Message::Machine(crate::control::Action::WorkZero(Some(0))));
        tick_until(&mut a, 2.0, "X zero moved", |s| s.work_pos[0].abs() < 1e-6);
        assert!(
            problems(&a)
                .iter()
                .any(|p| p.contains("X/Y work zero changed"))
        );
        mdi(&mut a, "G10 L2 P1 X195");
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
        let _ = a.update(Message::Run(R::CycleStart));
        run_until(&mut a, 30.0, "tool run 2", |r| r.done[1]);
        let s = &conn(&a).snapshot;
        assert!(run_of(&a).outcome.as_ref().unwrap().0, "{:?}", s.log);
        // Lowered with the new Z offset: it ends at the plane in work Z.
        let p = &run_of(&a).checked().unwrap()[1];
        let plane = crate::post::clearance_plane(&p.blocks) - p.datum[2];
        assert!(
            (s.work_pos[2] - plane as f64).abs() < 1e-3,
            "{}",
            s.work_pos[2]
        );
        assert!(problems(&a).iter().any(|p| p.contains("Every tool run")));
    }

    /// The fixture job's Face, Pocket and Contour with the 10 mm flat, then
    /// its Drill: two tool runs, the first of three operations.
    fn ops_app(dir: &std::path::Path) -> App {
        run_app_of(dir, FAST, &[0, 1, 2, 3])
    }

    /// `check_run`, with the stock the check leaves (`run::check`),
    /// pictured in line, as in the smoke run.
    fn check_run_pictured(a: &mut App, dir: &std::path::Path, si: usize) {
        let _ = a.update(Message::Run(crate::run::Action::Setup(si)));
        answer_with_stock(a, dir, si);
    }

    /// Answers the run's check of setup `si` with the stock it leaves, as
    /// `App::check_run` does, pictures made in line.
    fn answer_with_stock(a: &mut App, dir: &std::path::Path, si: usize) {
        use std::sync::Arc;
        let epoch = run_of(a).epoch;
        let (programs, left) = crate::run::check(&a.doc, a.part_mesh().unwrap(), si);
        a.smoke_dir = Some(dir.to_path_buf());
        let _ = a.update(Message::RunCheckedStock(
            epoch,
            Arc::new(programs),
            Arc::new(left),
        ));
        a.smoke_dir = None;
    }

    /// The Tool runs panel shows each operation with a picture of the stock
    /// it leaves, each different, and where it stands: the first marked
    /// next, then the one being cut and those done before it while its tool
    /// run cuts, all done after (#229).
    #[test]
    fn each_operation_of_a_run_is_pictured_with_where_it_stands() {
        use crate::run::{Action as R, Stage::*};
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = ops_app(dir.path());
        zeroed(&mut a);
        check_run_pictured(&mut a, dir.path(), 0);
        let run = run_of(&a);
        let programs = run.checked().unwrap();
        assert_eq!(
            programs
                .iter()
                .map(|p| p.indexes.clone())
                .collect::<Vec<_>>(),
            vec![vec![0, 1, 2], vec![3]]
        );
        let pixels = |op| {
            let Some(iced::widget::image::Handle::Rgba { pixels, .. }) = run_of(&a).picture(op)
            else {
                panic!(
                    "no picture of operation {op}: {:?}",
                    run_of(&a).pictures.len()
                )
            };
            pixels.clone()
        };
        let drawn = run_of(&a).pictures.clone();
        let pictures = move |_: &App| drawn.clone();
        let pixels: Vec<_> = (0..4).map(pixels).collect();
        for k in 1..4 {
            assert_ne!(pixels[k - 1], pixels[k], "operation {k}");
        }
        let line = |a: &App| conn(a).snapshot.current_line;
        let stages = |a: &App, i| run_of(a).stages(i, line(a));
        assert_eq!(stages(&a, 0), [Next, ToCome, ToCome]);
        assert_eq!(stages(&a, 1), [ToCome]);
        let _ = a.view();
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
        let _ = a.update(Message::Run(R::CycleStart));
        tick_until(&mut a, 2.0, "the job", |s| s.state == Some(State::Run));
        let mut deadline = Deadline::new(30.);
        while stages(&a, 0) != [Done, Cutting, ToCome] {
            let _ = a.update(Message::Tick);
            assert!(deadline.within(&conn(&a).snapshot), "{:?}", stages(&a, 0));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // The pictures stay through the run.
        assert_eq!(run_of(&a).pictures.len(), 4);
        // A check answered while the machine cuts is not pictured then: the
        // meshing would compete with the driver feeding it (#167).
        answer_with_stock(&mut a, dir.path(), 0);
        let _ = a.update(Message::Tick);
        assert!(run_of(&a).pictures.is_empty() && run_of(&a).left.is_some());
        let _ = a.update(Message::MachineEStop);
        tick_until(&mut a, 2.0, "E-stop", |s| s.state == Some(State::EStop));
        // Once tool run 1 is done, as when it ends.
        let r = a.control.as_mut().unwrap().run.as_mut().unwrap();
        r.running = None;
        (r.done[0], r.next) = (true, 1);
        assert_eq!(stages(&a, 0), [Done, Done, Done]);
        assert_eq!(stages(&a, 1), [Next]);
        // Stopped: Cycle Start waits for the pictures, which the Tick makes.
        assert_eq!(
            a.planning_busy(),
            Some("The stock after each operation is still being pictured: wait a moment.")
        );
        let epoch = run_of(&a).epoch;
        let _ = a.update(Message::Tick);
        assert!(a.picture_job == Some(epoch) && run_of(&a).left.is_none());
        assert!(a.planning_busy().is_some());
        let _ = a.update(Message::RunPictures(epoch, pictures(&a)));
        assert_eq!(run_of(&a).pictures.len(), 4);
        assert_eq!(a.planning_busy(), None);
    }

    /// Renders the Control page's Tool runs panel to PNGs in
    /// `$AXIS_RENDER_DIR` (default /tmp/op-cards): before the run, while the
    /// Pocket of tool run 1 is cut, and at the tool change after it. Not a
    /// check: run it with --ignored.
    #[test]
    #[ignore]
    fn render_the_operation_cards() {
        use crate::run::{Action as R, Stage::*};
        use uc100::controller::State;
        let out = std::path::PathBuf::from(
            std::env::var("AXIS_RENDER_DIR").unwrap_or("/tmp/op-cards".into()),
        );
        std::fs::create_dir_all(&out).unwrap();
        let size = iced::Size::new(1600., 1500.);
        let dir = tempfile::tempdir().unwrap();
        let mut a = ops_app(dir.path());
        zeroed(&mut a);
        check_run_pictured(&mut a, dir.path(), 0);
        let _ = a.view();
        render_png(&a, size, &out.join("1-before-the-run.png"));
        let _ = a.update(Message::Run(R::CycleStart));
        tick_until(&mut a, 2.0, "the job", |s| s.state == Some(State::Run));
        let mut deadline = Deadline::new(30.);
        while run_of(&a).stages(0, conn(&a).snapshot.current_line) != [Done, Cutting, ToCome] {
            let _ = a.update(Message::Tick);
            assert!(deadline.within(&conn(&a).snapshot));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let _ = a.view();
        render_png(&a, size, &out.join("2-cutting-the-pocket.png"));
        run_until(&mut a, 30.0, "tool run 1", |r| r.done[0]);
        tick_until(&mut a, 2.0, "Idle", |s| s.state == Some(State::Idle));
        let _ = a.view();
        render_png(&a, size, &out.join("3-tool-change.png"));
    }

    /// The operation of a block of a program, if it names one.
    fn op_of(b: &crate::post::Block) -> Option<usize> {
        use crate::post::Block;
        match b {
            Block::Start { op, .. }
            | Block::Run { op, .. }
            | Block::Approach { op, .. }
            | Block::Descend { op, .. }
            | Block::Move { op, .. } => Some(*op),
            _ => None,
        }
    }

    /// A run started at the Pocket, the second operation of tool run 1,
    /// after the Facing was cut: nothing of the Facing is sent, the program
    /// starts safely (Z up alone, the Pocket's spindle speed and coolant,
    /// the dwell, across at the clearance plane and down), the cards say
    /// Skipped and Next, the checks before Cycle Start cover only what runs,
    /// Z is touched off on the floor the Facing left, and Cycle Start cuts
    /// from the Pocket on the simulated UC100 (LIO-177).
    #[test]
    fn a_run_starts_at_a_chosen_operation_with_a_safe_start_and_nothing_before_it() {
        use crate::post::Block;
        use crate::run::{Action as R, Stage::*};
        use crate::ui::work_zero_steps;
        use uc100::controller::State;
        use uc100::gcode::{Action as Do, Spindle};
        let dir = tempfile::tempdir().unwrap();
        let mut a = ops_app(dir.path());
        // The Facing asks for mist, which the profile has no output for.
        a.doc.setups[0].operations[0]
            .p
            .insert("coolant".into(), "Mist".into());
        zeroed(&mut a);
        check_run(&mut a, 0);
        let has = |a: &App, text: &str| problems(a).iter().any(|p| p.contains(text));
        assert!(has(&a, "Facing: uses mist coolant"), "{:?}", problems(&a));
        let p = run_of(&a).checked().unwrap()[0].clone();
        let names: Vec<&str> = p.ops.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["Facing", "Pocket", "Contour"]);
        assert!(p.starts.iter().all(Result::is_ok), "{:?}", p.starts);
        // The program from the Pocket: the tool change's safe start, then
        // the same blocks as the whole program, with nothing of the Facing.
        let from = p.from(1).unwrap();
        assert!(from.iter().all(|b| op_of(b) != Some(0)));
        assert!(
            matches!(
                from[..8],
                [
                    Block::Stop,
                    Block::Retract,
                    Block::Start {
                        op: 1,
                        change: true
                    },
                    Block::Retract,
                    Block::Run {
                        op: 1,
                        spindle: true,
                        coolant: true
                    },
                    Block::Wait { .. },
                    Block::Approach { op: 1, .. },
                    Block::Descend { op: 1, .. },
                ]
            ),
            "{:?}",
            &from[..8]
        );
        let of = |blocks: &[Block], op| {
            let at = blocks
                .iter()
                .position(|b| matches!(b, Block::Start { op: k, .. } if *k == op))
                .unwrap();
            blocks[at..].to_vec()
        };
        // The Pocket's own moves are all there; the whole program's link
        // from the Facing (up, across and down) is not.
        let pocket = |blocks: &[Block]| -> Vec<Block> {
            blocks
                .iter()
                .filter(|b| matches!(b, Block::Move { op: 1, .. }))
                .cloned()
                .collect()
        };
        let (own, whole) = (pocket(from), pocket(&p.blocks));
        assert!(whole.ends_with(&own) && whole.len() - own.len() <= 3);
        assert_eq!(of(from, 2), of(&p.blocks, 2));
        // Lowered where the machine stands: Z up alone to the clearance
        // plane, the Pocket's spindle and coolant, the dwell, across, down.
        let s = conn(&a).snapshot.clone();
        let actions = crate::run::lower(&p, 1, s.work_offset, s.machine_pos).unwrap();
        let o = &p.ops[1];
        let high = crate::post::clearance_plane(from) as f64 - p.datum[2] as f64 + s.work_offset[2];
        let Do::Move {
            target: up,
            feed: None,
        } = actions[0].action
        else {
            panic!("{:?}", actions[0])
        };
        assert_eq!(up[..2], s.machine_pos[..2]);
        assert!((up[2] - high).abs() < 1e-9);
        // Plan times are f32: a dwell is its seconds to a few µs.
        let rounded = |l: &uc100::gcode::Located| match l.action {
            Do::Dwell { seconds } => Do::Dwell {
                seconds: (seconds * 1e3).round() / 1e3,
            },
            ref a => a.clone(),
        };
        assert_eq!(
            actions[1..4].iter().map(rounded).collect::<Vec<_>>(),
            [
                Do::Spindle {
                    state: Spindle::Cw,
                    rpm: o.rpm as f64
                },
                Do::Coolant {
                    flood: o.coolant.1,
                    mist: o.coolant.0
                },
                Do::Dwell { seconds: 0.2 },
            ]
        );
        assert!(o.coolant.1 && !o.coolant.0);
        let Do::Move {
            target: over,
            feed: None,
        } = actions[4].action
        else {
            panic!("{:?}", actions[4])
        };
        let Block::Descend { xy, z, .. } = from[7] else {
            unreachable!()
        };
        // Across at the clearance plane to over the Pocket's first point,
        // then down to it unless it is at the plane.
        assert!((over[2] - high).abs() < 1e-9);
        for k in 0..2 {
            assert!((over[k] - (xy[k] - p.datum[k]) as f64 - s.work_offset[k]).abs() < 1e-4);
        }
        let first = (z - p.datum[2]) as f64 + s.work_offset[2];
        assert!(first <= high + 1e-9);
        if first < high - 1e-9 {
            let Do::Move {
                target: down,
                feed: None,
            } = actions[5].action
            else {
                panic!("{:?}", actions[5])
            };
            assert_eq!(over[..2], down[..2]);
            assert!((down[2] - first).abs() < 1e-4);
        }
        assert!(
            actions
                .iter()
                .all(|l| op_of(&from[l.line as usize - 1]) != Some(0))
        );
        // The cards: the Facing skipped, the Pocket next.
        let line = |a: &App| conn(a).snapshot.current_line;
        let stages = |a: &App, i| run_of(a).stages(i, line(a));
        assert_eq!(stages(&a, 0), [Next, ToCome, ToCome]);
        let _ = a.update(Message::Run(R::From(1, 0)));
        assert_eq!(stages(&a, 0), [Skipped, Skipped, Skipped]);
        assert_eq!(stages(&a, 1), [Next]);
        let _ = a.update(Message::Run(R::From(0, 1)));
        assert_eq!(stages(&a, 0), [Skipped, Next, ToCome]);
        assert_eq!(stages(&a, 1), [ToCome]);
        let _ = a.view();
        // The Facing's mist is not sent, so it is not refused; Cycle Start
        // waits for the work zero on the stock the Facing left.
        assert!(!has(&a, "mist"), "{:?}", problems(&a));
        assert!(
            has(&a, "after operations taken as cut"),
            "{:?}",
            problems(&a)
        );
        let _ = a.update(Message::Run(R::CycleStart));
        assert!(run_of(&a).running.is_none());
        // The Facing took the top down to the part's, 4 mm: Z is touched
        // off on that floor at the stock top's centre.
        let partly = run_of(&a).partly().unwrap();
        assert!(
            partly.cut.iter().any(|c| c == "top face"),
            "{:?}",
            partly.cut
        );
        let floor = partly.surface.unwrap() - p.datum[2];
        assert!((floor + 4.).abs() < 1e-3, "{floor}");
        let (_, work) = conn(&a).touch_off().unwrap();
        assert!((work[2] - floor as f64).abs() < 1e-9);
        // A plate laid anywhere on the top might lie on the Pocket's floor:
        // Probe Z is off.
        assert!(
            conn(&a)
                .stock_top()
                .unwrap_err()
                .contains("Probe Z cannot set Z")
        );
        let steps = work_zero_steps(&a, conn(&a)).unwrap().join("\n");
        assert!(
            steps.contains("This run starts at Pocket (tool run 1), taking Facing as cut")
                && steps.contains("at Z-4 (the stock top was Z0)")
                && steps.contains("pinches a sheet of paper on the floor there")
                && steps.contains("4. Set Z-4: that point becomes Z-4. X and Y stay as they are"),
            "{steps}"
        );
        // The stock top was at machine Z-20 (`zeroed`): the tip 4 mm below
        // it, on the Facing's floor, and over the centre within a few mm by
        // eye. Set Z-4 there sets Z alone, which stays: X and Y stay too.
        // (The simulator's block, 47 mm tall with its top at Z-40, would put
        // the Contour below the soft limits, which Cycle Start refuses.)
        let offset = conn(&a).snapshot.work_offset;
        mdi(&mut a, "G0 X3 Y-2 Z-4");
        let _ = a.update(Message::Machine(crate::control::Action::TouchOff));
        tick_for(&mut a, 100);
        assert_eq!(conn(&a).snapshot.work_offset, offset);
        assert!(run_of(&a).zeroed);
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
        // Cycle Start cuts from the Pocket: the job is the program from it.
        let _ = a.update(Message::Run(R::CycleStart));
        tick_until(&mut a, 2.0, "the job", |s| s.state == Some(State::Run));
        assert_eq!(run_of(&a).running_blocks(), Some(from));
        let mut deadline = Deadline::new(30.);
        while stages(&a, 0) != [Skipped, Cutting, ToCome] {
            let _ = a.update(Message::Tick);
            assert!(deadline.within(&conn(&a).snapshot), "{:?}", stages(&a, 0));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let (_, op, _) = run_of(&a).progress(line(&a)).unwrap();
        assert_eq!(op, 1);
        run_until(&mut a, 30.0, "tool run 1 from the Pocket", |r| r.done[0]);
        let (ok, message) = run_of(&a).outcome.clone().unwrap();
        assert!(
            ok && message.contains("tool run 1 of 2 from Pocket"),
            "{message}"
        );
        assert_eq!((run_of(&a).next, run_of(&a).from), (1, 0));
        assert_eq!(stages(&a, 0), [Done, Done, Done]);
        assert_eq!(stages(&a, 1), [Next]);
        // Started: the work zero is kept from here, as after any first tool run.
        assert!(!run_of(&a).partway() && work_zero_steps(&a, conn(&a)).is_none());
        // The Contour picked again in the tool run done: cut again from it.
        let _ = a.update(Message::Run(R::From(0, 2)));
        assert_eq!(stages(&a, 0), [Done, Done, Next]);
        assert_eq!(stages(&a, 1), [ToCome]);
    }

    /// A run started at the Drilling with the work zero on the stock top's
    /// front-left corner, which the Contour cut down in part: the check
    /// cannot tell one surface there, so Zero Z on the stock top and Probe Z
    /// are off and touch off nothing, and Cycle Start waits until the work
    /// zero is kept or set from a reference, for each start chosen (LIO-177).
    #[test]
    fn a_partway_start_whose_surface_is_not_known_waits_for_the_work_zero() {
        use crate::run::Action as R;
        use crate::ui::work_zero_steps;
        let dir = tempfile::tempdir().unwrap();
        let mut a = ops_app(dir.path());
        a.doc.setups[0]
            .p
            .insert("origin".into(), "Stock top front-left".into());
        zeroed(&mut a);
        check_run(&mut a, 0);
        let has = |a: &App, text: &str| problems(a).iter().any(|p| p.contains(text));
        let waits = "after operations taken as cut";
        assert!(!has(&a, waits) && conn(&a).partway_blocked().is_none());
        let _ = a.update(Message::Run(R::Next(1)));
        assert!(run_of(&a).partway());
        let blocked = conn(&a).partway_blocked().unwrap();
        assert!(
            blocked.contains("cannot tell the surface they left"),
            "{blocked}"
        );
        assert!(conn(&a).touch_off().is_none());
        assert!(
            conn(&a)
                .stock_top()
                .unwrap_err()
                .contains("Probe Z cannot set it")
        );
        assert!(conn(&a).edges_setup().is_err());
        assert!(conn(&a).probe_spots().iter().all(|(_, ok)| ok.is_err()));
        let steps = work_zero_steps(&a, conn(&a)).unwrap().join("\n");
        assert!(
            steps.contains("taking Facing, Pocket, Contour as cut")
                && steps.contains("Load T3")
                && steps.contains("Zero Z on the stock top and Probe Z are off")
                && !steps.contains("Zero all"),
            "{steps}"
        );
        // Touch off does nothing; Cycle Start waits.
        let offset = conn(&a).snapshot.work_offset;
        let _ = a.update(Message::Machine(crate::control::Action::TouchOff));
        tick_for(&mut a, 100);
        assert_eq!(conn(&a).snapshot.work_offset, offset);
        assert!(has(&a, waits), "{:?}", problems(&a));
        let _ = a.view();
        let _ = a.update(Message::Run(R::CycleStart));
        assert!(run_of(&a).running.is_none());
        // Kept: Cycle Start may start.
        let _ = a.update(Message::Run(R::KeepZero));
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
        // Another start asks again.
        let _ = a.update(Message::Run(R::From(0, 2)));
        assert!(has(&a, waits));
        let _ = a.update(Message::Run(R::KeepZero));
        assert!(!has(&a, waits));
        // A check of the setup again asks again, and keeps the start.
        let epoch = run_of(&a).epoch;
        let programs = crate::run::programs(&a.doc, a.part_mesh().unwrap(), 0);
        let _ = a.update(Message::RunChecked(epoch, std::sync::Arc::new(programs)));
        assert_eq!((run_of(&a).next, run_of(&a).from), (0, 2));
        assert!(has(&a, waits));
        // From the setup's first operation the work zero is found as ever.
        let _ = a.update(Message::Run(R::Next(0)));
        assert!(!run_of(&a).partway() && !has(&a, waits));
    }

    /// Renders the Tool runs panel with an operation chosen to PNGs in
    /// `$AXIS_RENDER_DIR` (default /tmp/run-from-op): the Pocket chosen
    /// before Z is touched off on the floor the Facing left, the Pocket
    /// being cut, and the Drilling chosen with the work zero on a corner
    /// whose surface the check cannot tell. Not a check: run it with
    /// --ignored.
    #[test]
    #[ignore]
    fn render_a_run_started_at_a_chosen_operation() {
        use crate::run::{Action as R, Stage::*};
        use uc100::controller::State;
        let out = std::path::PathBuf::from(
            std::env::var("AXIS_RENDER_DIR").unwrap_or("/tmp/run-from-op".into()),
        );
        std::fs::create_dir_all(&out).unwrap();
        let size = iced::Size::new(1600., 1700.);
        let dir = tempfile::tempdir().unwrap();
        let mut a = ops_app(dir.path());
        zeroed(&mut a);
        check_run_pictured(&mut a, dir.path(), 0);
        let _ = a.update(Message::Run(R::From(0, 1)));
        let _ = a.view();
        render_png(&a, size, &out.join("1-pocket-chosen.png"));
        mdi(&mut a, "G0 X0 Y0 Z-4");
        let _ = a.update(Message::Machine(crate::control::Action::TouchOff));
        tick_for(&mut a, 100);
        let _ = a.update(Message::Run(R::CycleStart));
        tick_until(&mut a, 2.0, "the job", |s| s.state == Some(State::Run));
        let mut deadline = Deadline::new(30.);
        while run_of(&a).stages(0, conn(&a).snapshot.current_line) != [Skipped, Cutting, ToCome] {
            let _ = a.update(Message::Tick);
            assert!(deadline.within(&conn(&a).snapshot));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let _ = a.view();
        render_png(&a, size, &out.join("2-cutting-from-the-pocket.png"));
        let dir = tempfile::tempdir().unwrap();
        let mut a = ops_app(dir.path());
        a.doc.setups[0]
            .p
            .insert("origin".into(), "Stock top front-left".into());
        zeroed(&mut a);
        check_run_pictured(&mut a, dir.path(), 0);
        let _ = a.update(Message::Run(R::Next(1)));
        let _ = a.view();
        render_png(&a, size, &out.join("3-drilling-chosen-surface-unknown.png"));
    }

    /// The driver refuses a job sent while it is not idle for it: for a few
    /// milliseconds after a work or machine zero it re-anchors its position,
    /// while the app's snapshot still says Idle. Such a Cycle Start ran
    /// nothing, so it does not count as the setup's first tool run: the
    /// work-zero steps stay and the X/Y work zero is not held yet (#168).
    #[test]
    fn a_cycle_start_the_driver_refuses_has_not_started() {
        use crate::run::Action as R;
        use crate::ui::work_zero_steps;
        use uc100::controller::{Command, State};
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        zeroed(&mut a);
        check_run(&mut a, 0);
        tick_for(&mut a, 100);
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
        assert!(!run_of(&a).started());
        assert!(work_zero_steps(&a, conn(&a)).is_some());
        let has = |a: &App, text: &str| problems(a).iter().any(|p| p.contains(text));
        // The machine starts a slow move; the app, with no tick since, still
        // sees it Idle and sends the job, which the driver refuses.
        conn(&a).send(Command::Mdi("G1 X5 F600".into()));
        let _ = a.update(Message::Run(R::CycleStart));
        assert!(run_of(&a).running.is_some());
        run_until(&mut a, 5.0, "the refusal", |r| r.running.is_none());
        let (ok, message) = run_of(&a).outcome.clone().unwrap();
        assert!(
            !ok && message.contains("not started, the machine is not idle"),
            "{message}"
        );
        assert!(!run_of(&a).done[0] && run_of(&a).next == 0);
        // Nothing ran: the setup has not started, and the steps stay.
        assert!(!run_of(&a).started());
        assert!(work_zero_steps(&a, conn(&a)).is_some());
        let _ = a.view();
        tick_until(&mut a, 5.0, "Idle after the move", |s| {
            s.state == Some(State::Idle) && (s.work_pos[0] - 5.).abs() < 1e-3
        });
        // The X/Y work zero may still be moved: no tool run was cut from it.
        conn(&a).send(Command::SetWork { axis: 0, value: 0. });
        tick_until(&mut a, 2.0, "X zeroed here", |s| s.work_pos[0].abs() < 1e-6);
        tick_for(&mut a, 100);
        assert!(!has(&a, "X/Y work zero changed"), "{:?}", problems(&a));
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
        // A Cycle Start the driver takes starts the setup: the steps go. A
        // Stop ends the tool run not ok too, but it did cut, so the work
        // zero stays held.
        let _ = a.update(Message::Run(R::CycleStart));
        tick_until(&mut a, 2.0, "the job", |s| s.state == Some(State::Run));
        assert!(run_of(&a).started());
        assert!(work_zero_steps(&a, conn(&a)).is_none());
        let _ = a.update(Message::Run(R::Stop));
        run_until(&mut a, 5.0, "the stop", |r| r.running.is_none());
        tick_until(&mut a, 2.0, "Idle", |s| s.state == Some(State::Idle));
        let (ok, message) = run_of(&a).outcome.clone().unwrap();
        assert!(!ok && !message.contains("not started"), "{message}");
        assert!(run_of(&a).started());
        assert!(work_zero_steps(&a, conn(&a)).is_none());
    }

    /// The Run panel's check simulates the whole job up to the setup, minutes
    /// on a big part, and would starve the driver of the processor while it
    /// feeds the machine (#167): it waits for the machine to stop.
    #[test]
    fn the_setup_check_waits_until_the_machine_stops() {
        use crate::control::Action;
        use crate::run::Action as R;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        idle(&mut a);
        check_run(&mut a, 0);
        assert!(run_of(&a).checked().is_some());
        a.last_edit -= crate::EDIT_PAUSE * 2;
        // A held jog: Check again marks the check wanted and starts none.
        let _ = a.update(Message::Machine(Action::JogPress(0, false)));
        tick_until(&mut a, 2.0, "the jog", |s| s.state == Some(State::Jog));
        assert!(a.locked());
        let epoch = run_of(&a).epoch;
        let task = a.update(Message::Run(R::Check));
        assert!(
            iced_runtime::task::into_stream(task).is_none(),
            "a check started while jogging"
        );
        assert!(run_of(&a).stale && run_of(&a).checked().is_some());
        assert_eq!(run_of(&a).epoch, epoch);
        // Nor does the tick start one, however long the jog is held.
        tick_for(&mut a, 300);
        assert_eq!(conn(&a).snapshot.state, Some(State::Jog));
        assert!(run_of(&a).stale && run_of(&a).checked().is_some());
        assert_eq!(run_of(&a).epoch, epoch);
        // Picking the setup again while jogging: the panel says the check waits.
        let task = a.update(Message::Run(R::Setup(0)));
        assert!(
            iced_runtime::task::into_stream(task).is_none(),
            "a check started while jogging"
        );
        assert!(run_of(&a).stale && run_of(&a).programs.is_none());
        assert_eq!(problems(&a), vec![crate::run::CHECK_WAITS.to_string()]);
        let _ = a.view();
        // Released, the tick that finds the machine Idle checks it.
        let held = run_of(&a).epoch;
        let _ = a.update(Message::Machine(Action::JogRelease));
        tick_until(&mut a, 3.0, "Idle after the release", |s| {
            s.state == Some(State::Idle)
        });
        assert!(!a.locked());
        a.last_edit -= crate::EDIT_PAUSE * 2;
        let _ = a.update(Message::Tick);
        assert_ne!(run_of(&a).epoch, held);
        assert!(!run_of(&a).stale && run_of(&a).programs.is_none());
        assert_eq!(problems(&a), vec!["Checking the setup…".to_string()]);
        let epoch = run_of(&a).epoch;
        let programs = crate::run::programs(&a.doc, a.part_mesh().unwrap(), 0);
        let _ = a.update(Message::RunChecked(epoch, std::sync::Arc::new(programs)));
        assert_eq!(run_of(&a).checked().map(<[_]>::len), Some(2));
    }

    /// The setup's work zero moved in the project between tool runs (its
    /// origin picked anew, or the stock moved under the same origin), but the
    /// machine's work offset was touched off for the old one: a later tool
    /// run would cut shifted by the difference. Cycle Start refuses it once
    /// the setup is checked again, until it is put back or picked again.
    #[test]
    fn a_work_zero_moved_in_the_project_between_tool_runs_is_refused() {
        use crate::run::Action as R;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        zeroed(&mut a);
        check_run(&mut a, 0);
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
        let has = |a: &App, text: &str| problems(a).iter().any(|p| p.contains(text));
        // Tool run 1 starts, which keeps the work zero from then on, and is
        // stopped: the machine is Idle again with the same tool next.
        let _ = a.update(Message::Run(R::CycleStart));
        tick_until(&mut a, 2.0, "the job", |s| s.state == Some(State::Run));
        let _ = a.update(Message::Run(R::Stop));
        run_until(&mut a, 5.0, "the stop", |r| r.running.is_none());
        tick_until(&mut a, 2.0, "Idle", |s| s.state == Some(State::Idle));
        assert!(run_of(&a).started());
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
        // An edit, then the Control page's own check once the edits pause.
        let recheck = |a: &mut App| {
            let _ = a.update(Message::Title("Edited".into()));
            assert!(has(a, "project changed"), "{:?}", problems(a));
            a.last_edit -= crate::EDIT_PAUSE * 2;
            let _ = a.update(Message::Tick);
            let epoch = run_of(a).epoch;
            let programs = crate::run::programs(&a.doc, a.part_mesh().unwrap(), 0);
            assert!(programs.is_ok(), "{:?}", programs.err());
            let _ = a.update(Message::RunChecked(epoch, std::sync::Arc::new(programs)));
            assert!(!run_of(a).stale);
        };
        // The work zero moves to a corner: refused, with both zeros and the
        // distance, and Cycle Start does nothing.
        a.doc.setups[0]
            .p
            .insert("origin".into(), "Stock top front-left".into());
        recheck(&mut a);
        let moved: Vec<String> = problems(&a)
            .into_iter()
            .filter(|p| p.contains("work zero moved since the first tool run started"))
            .collect();
        assert_eq!(moved.len(), 1, "{:?}", problems(&a));
        for text in [
            "Stock top center then, Stock top front-left now",
            "pick it again to start over and touch off the new work zero",
        ] {
            assert!(moved[0].contains(text), "{text}: {}", moved[0]);
        }
        let _ = a.update(Message::Run(R::CycleStart));
        assert!(run_of(&a).running.is_none());
        // Put back, it runs again.
        a.doc.setups[0]
            .p
            .insert("origin".into(), "Stock top center".into());
        recheck(&mut a);
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
        // The same origin on a stock grown 10 mm to the right: its centre,
        // the zero, moved 5 mm in X.
        a.doc.setups[0].p.insert("stock_right".into(), "12".into());
        recheck(&mut a);
        assert!(
            has(&a, "Stock top center moved X 5.000 mm"),
            "{:?}",
            problems(&a)
        );
        let _ = a.update(Message::Run(R::CycleStart));
        assert!(run_of(&a).running.is_none());
        // Picking the setup again starts over: the new zero is touched off.
        check_run(&mut a, 0);
        assert!(!run_of(&a).started());
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
    }

    /// The Run strip, fixed above the part of the Control page that scrolls,
    /// holds Cycle Start, Hold, Resume and Stop however long the tool-run
    /// list is; the list is under it, in the Tool runs panel (#71).
    #[test]
    fn the_run_strip_keeps_cycle_start_and_stop_above_a_long_tool_run_list() {
        use iced::advanced::widget::{Tree, tree::Tag};
        /// The buttons in a widget tree: the nodes that keep a button's state.
        fn buttons(t: &Tree, button: Tag) -> usize {
            usize::from(t.tag == button)
                + t.children.iter().map(|c| buttons(c, button)).sum::<usize>()
        }
        let button = Tree::new(
            iced::Element::<Message>::from(iced::widget::button("Cycle Start")).as_widget(),
        )
        .tag;
        let counts = |a: &App| {
            let strip = crate::ui::run_strip(a, conn(a));
            let runs = crate::ui::run_panel(a, conn(a)).expect("the tool runs");
            (
                buttons(&Tree::new(strip.as_widget()), button),
                buttons(&Tree::new(runs.as_widget()), button),
            )
        };
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        idle(&mut a);
        check_run(&mut a, 0);
        let two = run_of(&a).checked().expect("checked").to_vec();
        assert_eq!(two.len(), 2);
        let _ = a.view();
        let (strip, runs) = counts(&a);
        // The setup's button, then Cycle Start, Hold, Resume and Stop.
        let setups = a
            .doc
            .setups
            .iter()
            .filter(|x| x.machine.id == conn(&a).machine_id)
            .count();
        assert_eq!(strip, setups + 4);
        assert!(runs >= 2, "{runs}");
        // Twelve tool runs: the two, six times over.
        let epoch = run_of(&a).epoch;
        let many: Vec<_> = (0..6).flat_map(|_| two.clone()).collect();
        let _ = a.update(Message::RunChecked(epoch, std::sync::Arc::new(Ok(many))));
        assert_eq!(run_of(&a).checked().map(<[_]>::len), Some(12));
        let _ = a.view();
        // The strip is the same; the ten more tool runs are in the list under
        // it, each of one operation: its button and the card's Start here.
        assert_eq!(counts(&a), (strip, runs + 20));
    }

    #[test]
    fn cycle_start_waits_for_what_the_run_panel_lists() {
        use crate::run::Action as R;
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        idle(&mut a);
        check_run(&mut a, 0);
        let has = |a: &App, text: &str| problems(a).iter().any(|p| p.contains(text));
        // Machine zero is not set yet.
        assert!(
            has(&a, "no machine position (every E-stop clears it)"),
            "{:?}",
            problems(&a)
        );
        let _ = a.update(Message::Run(R::CycleStart));
        assert!(run_of(&a).running.is_none());
        let _ = a.view();
        let _ = a.update(Message::Run(R::Next(9)));
        assert_eq!(run_of(&a).next, 0);
        zero(&mut a);
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
        // The work zero puts the clearance plane above the top of Z.
        mdi(&mut a, "G53 G0 Z-5");
        let _ = a.update(Message::Machine(crate::control::Action::WorkZero(Some(2))));
        tick_until(&mut a, 2.0, "work Z", |s| s.work_pos[2].abs() < 1e-6);
        assert!(has(&a, "outside the soft limits"), "{:?}", problems(&a));
        mdi(&mut a, "G53 G0 Z-20");
        let _ = a.update(Message::Machine(crate::control::Action::WorkZero(Some(2))));
        tick_until(&mut a, 2.0, "work Z", |s| s.work_pos[2].abs() < 1e-6);
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
        // Another work system is active.
        let _ = a.update(Message::Machine(crate::control::Action::Wcs(1)));
        tick_until(&mut a, 2.0, "G55", |s| s.active_wcs == 1);
        assert!(has(&a, "active work system is G55"), "{:?}", problems(&a));
        let _ = a.update(Message::Machine(crate::control::Action::Wcs(0)));
        tick_until(&mut a, 2.0, "G54", |s| s.active_wcs == 0);
        // The AXIS CAM machine's travel disagrees with the profile, the
        // spindle runs CCW without a direction output, and mist is missing:
        // edits, so the setup is checked again.
        let setup = &mut a.doc.setups[0];
        setup.machine.p.insert("y".into(), "300".into());
        setup.operations[0]
            .p
            .insert("spindle_rotation".into(), "Counterclockwise (CCW)".into());
        setup.operations[1]
            .p
            .insert("coolant".into(), "Mist".into());
        let _ = a.update(Message::Title("Edited".into()));
        assert!(has(&a, "project changed"), "{:?}", problems(&a));
        let _ = a.view();
        // Once the edits pause, the Control page checks it again on its own
        // (LIO-150), as Check again does.
        let epoch = run_of(&a).epoch;
        let _ = a.update(Message::Tick);
        assert_eq!(run_of(&a).epoch, epoch, "rechecked mid-edit");
        a.last_edit -= crate::EDIT_PAUSE * 2;
        let _ = a.update(Message::Tick);
        assert_ne!(run_of(&a).epoch, epoch);
        assert!(!run_of(&a).stale && run_of(&a).programs.is_none());
        let epoch = run_of(&a).epoch;
        let _ = a.update(Message::Run(R::Check));
        assert_ne!(run_of(&a).epoch, epoch);
        let epoch = run_of(&a).epoch;
        let programs = crate::run::programs(&a.doc, a.part_mesh().unwrap(), 0);
        let _ = a.update(Message::RunChecked(epoch, std::sync::Arc::new(programs)));
        for text in [
            "Y travel in AXIS CAM is 300.000 mm",
            "no spindle direction output",
        ] {
            assert!(has(&a, text), "{text}: {:?}", problems(&a));
        }
        let _ = a.update(Message::Run(R::Next(1)));
        assert!(has(&a, "no mist output"), "{:?}", problems(&a));
        // Set coolant to Off turns off only the coolant the profile lacks,
        // in any tool run, and checks again.
        let _ = a.view();
        assert_eq!(run_of(&a).coolant_off(conn(&a), &a.doc), [1]);
        let _ = a.update(Message::Run(R::CoolantOff));
        let ops = &a.doc.setups[0].operations;
        assert_eq!(
            (get(&ops[0].p, "coolant"), get(&ops[1].p, "coolant")),
            ("Flood", "Off")
        );
        assert!(run_of(&a).programs.is_none());
        let epoch = run_of(&a).epoch;
        let programs = crate::run::programs(&a.doc, a.part_mesh().unwrap(), 0);
        let _ = a.update(Message::RunChecked(epoch, std::sync::Arc::new(programs)));
        assert!(!has(&a, "no mist output"), "{:?}", problems(&a));
        // An E-stop clears the machine position: Cycle Start waits for it.
        let _ = a.update(Message::MachineEStop);
        tick_until(&mut a, 2.0, "E-stop", |s| !s.referenced[0]);
        let _ = a.view();
        assert!(has(&a, "Set machine zero again"), "{:?}", problems(&a));
    }

    /// A run names its setup by id: moving a setup in MANUFACTURE moves the
    /// run with it, and the check is of the setup where it is now. Deleting
    /// it leaves the run with nothing to check, which it says.
    #[test]
    fn the_run_keeps_to_its_setup_when_setups_are_moved_or_removed() {
        use crate::run::Action as R;
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        idle(&mut a);
        // A copy of the first setup without its operations, after it; the
        // run is for the first setup, two tool runs.
        let _ = a.update(Message::DuplicateSetup);
        a.doc.setups[1].operations.clear();
        a.page = Page::Control;
        check_run(&mut a, 0);
        let id = a.doc.setups[0].id.clone();
        let count = |a: &App| run_of(a).checked().map(|p| p.len());
        let refusal = |a: &App| {
            run_of(a)
                .programs
                .as_deref()
                .and_then(|p| p.as_ref().err().cloned())
        };
        assert_eq!(count(&a), Some(2), "{:?}", refusal(&a));
        // MANUFACTURE: the copy is moved before it.
        let _ = a.update(Message::SelectSetup(1));
        let _ = a.update(Message::MoveSetup(-1));
        assert_eq!(a.doc.setups[1].id, id);
        assert!(run_of(&a).stale);
        assert_eq!(run_of(&a).setup, 1, "the run keeps to its setup");
        assert!(
            !problems(&a).iter().any(|p| p.contains("removed")),
            "{:?}",
            problems(&a)
        );
        // Back on the Control page, once the edits pause, it is checked again
        // where it is now: its two tool runs, not the copy's refusal.
        a.page = Page::Control;
        a.last_edit -= crate::EDIT_PAUSE * 2;
        let _ = a.update(Message::Tick);
        assert!(!run_of(&a).stale && run_of(&a).programs.is_none());
        assert_eq!(run_of(&a).setup, 1);
        let epoch = run_of(&a).epoch;
        let programs = crate::run::programs(&a.doc, a.part_mesh().unwrap(), run_of(&a).setup);
        let _ = a.update(Message::RunChecked(epoch, std::sync::Arc::new(programs)));
        assert_eq!(count(&a), Some(2), "{:?}", refusal(&a));
        // Deleted: the run has nothing to check and says so, and nothing is
        // touched off or probed from it.
        let removed = "This setup was removed from the project.";
        let _ = a.update(Message::SelectSetup(1));
        let _ = a.update(Message::ConfirmDeleteSetup(true));
        assert!(a.doc.setups.iter().all(|s| s.id != id));
        assert_eq!(refusal(&a), Some(vec![removed.into()]));
        assert_eq!(problems(&a), vec![removed.to_string()]);
        assert!(conn(&a).touch_off().is_none());
        assert!(conn(&a).stock_top().is_err());
        assert!(conn(&a).edges_setup().is_err());
        assert!(crate::ui::work_zero_steps(&a, conn(&a)).is_none());
        // The check once the edits pause, and Check again, find the same.
        a.page = Page::Control;
        a.last_edit -= crate::EDIT_PAUSE * 2;
        let epoch = run_of(&a).epoch;
        let _ = a.update(Message::Tick);
        assert_ne!(run_of(&a).epoch, epoch);
        assert!(!run_of(&a).stale);
        assert_eq!(refusal(&a), Some(vec![removed.into()]));
        let _ = a.update(Message::Run(R::Check));
        assert_eq!(refusal(&a), Some(vec![removed.into()]));
        let _ = a.view();
        // The check refuses an index past the end rather than failing on it.
        let past = crate::run::programs(&a.doc, a.part_mesh().unwrap(), 1);
        assert_eq!(past.err(), Some(vec![removed.into()]));
    }

    /// A spindle the operator left running by hand the other way (issue
    /// #41): Cycle Start refuses it before anything moves, saying to stop
    /// it, since the driver E-stops a reversal after the first retract.
    /// Stopped, or running the tool run's way, it starts.
    #[test]
    fn cycle_start_refuses_a_spindle_running_the_other_way() {
        use crate::control::Action;
        use crate::run::Action as R;
        use uc100::{controller::State, gcode::Spindle};
        let dir = tempfile::tempdir().unwrap();
        // FAST with a reverse relay: the spindle can run CCW.
        let profile = format!("{FAST}spindle_ccw = {{ pin = 14 }}\n");
        let mut a = run_app_on(dir.path(), &profile);
        zeroed(&mut a);
        check_run(&mut a, 0);
        assert!(conn(&a).reversible());
        assert!(run_of(&a).checked().unwrap()[0].ops[0].clockwise);
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
        let _ = a.update(Message::Machine(Action::Rpm("8000".into())));
        let _ = a.update(Message::Machine(Action::Spindle(Spindle::Ccw)));
        tick_until(&mut a, 2.0, "CCW", |s| s.spindle == Some(Spindle::Ccw));
        let _ = a.view();
        let _ = a.update(Message::Run(R::CycleStart));
        tick_for(&mut a, 300);
        // Nothing ran: no E-stop, so the machine position is kept.
        let s = conn(&a).snapshot.clone();
        assert_eq!(
            (s.state.clone(), s.spindle, s.referenced[..3].to_vec()),
            (Some(State::Idle), Some(Spindle::Ccw), vec![true; 3]),
            "{:?} {:?}",
            s.estop_reason,
            s.log
        );
        assert!(run_of(&a).running.is_none() && run_of(&a).outcome.is_none());
        assert!(
            problems(&a).iter().any(|p| {
                p.contains(
                    "The spindle is running counterclockwise, but tool run 1 starts it clockwise",
                ) && p.contains("Stop it first")
            }),
            "{:?}",
            problems(&a)
        );
        // Stopped, or already running clockwise (the program sets its rpm),
        // it starts.
        let _ = a.update(Message::Machine(Action::Spindle(Spindle::Off)));
        tick_until(&mut a, 2.0, "off", |s| s.spindle == Some(Spindle::Off));
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
        let _ = a.update(Message::Machine(Action::Spindle(Spindle::Cw)));
        tick_until(&mut a, 2.0, "CW", |s| s.spindle == Some(Spindle::Cw));
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
        let _ = a.update(Message::Run(R::CycleStart));
        assert!(run_of(&a).running.is_some());
        tick_until(&mut a, 2.0, "the job", |s| s.state == Some(State::Run));
        let _ = a.update(Message::Run(R::Stop));
        tick_until(&mut a, 5.0, "Idle", |s| s.state == Some(State::Idle));
        assert_eq!(conn(&a).snapshot.estop_reason, None);
    }

    /// A manual mill's setup has steps, not a run: the Run strip does not list
    /// it, even on a machine it shares with the connected one, and picking it
    /// by message does nothing.
    #[test]
    fn the_run_strip_lists_no_manual_setup() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        idle(&mut a);
        let name = a.doc.setups[0].name().to_string();
        let mut hand = crate::manual::defaults();
        hand.id = conn(&a).machine_id.clone();
        let mut setup = crate::model::Setup::new(hand, "G55");
        setup.p.insert("name".into(), "Hand mill".into());
        a.doc.setups.push(setup);
        let texts = page_texts(&a);
        assert!(texts.contains(&format!("1. {name}")), "{texts:?}");
        assert!(!texts.iter().any(|t| t.contains("Hand mill")), "{texts:?}");
        let _ = a.update(Message::Run(crate::run::Action::Setup(1)));
        assert!(conn(&a).run.is_none());
        let _ = a.update(Message::Run(crate::run::Action::Setup(0)));
        assert_eq!(run_of(&a).setup, 0);
    }

    /// The blind review's dead end (LIO-140): machine zero where the machine
    /// starts puts every tool run outside the soft limits. The errors point to
    /// Work zero on the stock, which says where the simulator's block is;
    /// Move there and Zero all put the work zero on it and Cycle Start can go.
    #[test]
    fn the_soft_limits_errors_lead_to_a_work_zero_on_the_simulated_block() {
        use crate::control::Action;
        use crate::ui::work_zero_steps;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        // No probe input.
        let mut a = run_app(dir.path());
        idle(&mut a);
        check_run(&mut a, 0);
        let steps = |a: &App| work_zero_steps(a, conn(a)).expect("the guide").join("\n");
        assert!(
            steps(&a).contains("1. Set machine zero under Position"),
            "{}",
            steps(&a)
        );
        // ponytail: pressed again while an axis stays unset, since the driver
        // can drop a declaration (LIO-163); drop the loop once that is fixed.
        let set = |a: &App| conn(a).snapshot.referenced[..3].iter().all(|r| *r);
        for _ in 0..5 {
            let _ = a.update(Message::Machine(Action::MachineZero(None)));
            let end = std::time::Instant::now() + std::time::Duration::from_secs(1);
            while !set(&a) && std::time::Instant::now() < end {
                let _ = a.update(Message::Tick);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if set(&a) {
                break;
            }
        }
        assert!(set(&a), "{:?}", conn(&a).snapshot.log);
        let off: Vec<String> = (problems(&a).into_iter())
            .filter(|p| p.contains("outside the soft limits"))
            .collect();
        assert!(
            off.len() == 3
                && off
                    .iter()
                    .all(|p| p.contains("see Work zero on the stock below")),
            "{:?}",
            problems(&a)
        );
        let _ = a.view();
        let zero = conn(&a).sim_zero().unwrap();
        assert!(
            (0..3).all(|k| (zero[k] - [195., 140., -40.][k]).abs() < 1e-3),
            "{zero:?}"
        );
        let text = steps(&a);
        let on_block = format!(
            "work zero on it is at machine X {:.3} Y {:.3} Z {:.3}: Move there",
            zero[0], zero[1], zero[2]
        );
        for line in [
            "work zero, Stock top center in G54",
            "1. X, Y and Z have a machine position.",
            "3. Jog the tool tip over the centre of the stock top",
            "4. Zero all",
            on_block.as_str(),
        ] {
            assert!(text.contains(line), "{line}: {text}");
        }
        assert!(!text.contains("probe"), "{text}");
        let _ = a.update(Message::Machine(Action::ToSimZero));
        tick_until(&mut a, 10.0, "the move onto the block", |s| {
            s.state == Some(State::Idle)
                && (0..3).all(|k| (s.machine_pos[k] - zero[k]).abs() < 0.06)
        });
        let _ = a.update(Message::Machine(Action::WorkZero(None)));
        tick_until(&mut a, 2.0, "work zero", |s| {
            s.work_pos[..3].iter().all(|v| v.abs() < 1e-6)
        });
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
        let _ = a.view();
        // Once a tool run starts, the guide gives way.
        let _ = a.update(Message::Run(crate::run::Action::CycleStart));
        assert!(run_of(&a).running.is_some());
        assert!(work_zero_steps(&a, conn(&a)).is_none());
        let _ = a.update(Message::MachineEStop);
        tick_until(&mut a, 2.0, "E-stop", |s| s.state == Some(State::EStop));
    }

    /// `post::fixture::pinned_flip("rx", true)` on a simulated FAST machine
    /// with a probe input and the setups' fixture pins (Ø4, 6 mm tall, 80 mm
    /// apart along X): setup 1 drills the block's two dowel holes, setup 2
    /// ("Flipped", G55) sits on the pins by them, flipped about X, so that
    /// the part on the simulator's pins is on the table. A Ø2 probe tip and a
    /// touch plate of no thickness.
    fn pins_app(dir: &std::path::Path) -> App {
        let mut a = profile_app(dir, &probe_profile());
        let mut m = a.doc.machines.pop().unwrap();
        for (k, v) in [
            ("x", "390"),
            ("y", "280"),
            ("z", "80"),
            ("feed", "60000"),
            ("post_dwell", "0.2"),
            ("pins", "0 0; 80 0"),
            ("pin_diameter", "4"),
            ("pin_height", "6"),
            ("probe_tip", "2"),
            ("probe_plate", "0"),
            ("probe_feed", "1000"),
        ] {
            m.p.insert(k.into(), v.into());
        }
        let (mut doc, mesh) = crate::post::fixture::pinned_flip("rx", true);
        for s in &mut doc.setups {
            s.machine = m.clone();
        }
        doc.machines = vec![m];
        a.replace_document(doc, None);
        a.imported_mesh = Some(mesh);
        a.page = Page::Control;
        a
    }

    /// Ticks until Record the pins' place or Reference X Y from pin 1 has
    /// ended and declared what it does; how it ended.
    fn pinned(a: &mut App) -> (bool, String) {
        use uc100::controller::State;
        let end = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            let _ = a.update(Message::Tick);
            let _ = a.view();
            let c = conn(a);
            if c.pinning().is_none()
                && c.snapshot.state == Some(State::Idle)
                && let Some(said) = c.pin_said.clone()
                && (said.0 || !c.pin_referenced)
            {
                // A Reference is said once X and Y are declared from the pin.
                if !said.1.starts_with("X and Y referenced") || c.pin_referenced {
                    return said;
                }
            }
            assert!(
                std::time::Instant::now() < end,
                "timed out waiting for the pins: {:?} at {:?} {:?}",
                c.snapshot.state,
                c.snapshot.machine_pos,
                c.snapshot.log.iter().rev().take(5).collect::<Vec<_>>()
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// Sets machine zero where the machine is, pressed again while an axis
    /// stays unset (the driver can drop a declaration, LIO-163).
    fn machine_zero(a: &mut App) {
        use crate::control::Action;
        let set = |a: &App| conn(a).snapshot.referenced[..3].iter().all(|r| *r);
        for _ in 0..5 {
            let _ = a.update(Message::Machine(Action::MachineZero(None)));
            let end = std::time::Instant::now() + std::time::Duration::from_secs(1);
            while !set(a) && std::time::Instant::now() < end {
                let _ = a.update(Message::Tick);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if set(a) {
                break;
            }
        }
        assert!(set(a), "{:?}", conn(a).snapshot.log);
    }

    /// The fixture pins on the simulated UC100, end to end: the pins' place
    /// is recorded; after an E-stop and a machine zero 1.3 mm and 0.7 mm off,
    /// Cycle Start waits until Reference X Y from pin 1 declares X and Y back
    /// by exactly that; the flipped setup on the pins then gets X and Y from
    /// them, Z from Probe Z, and every pin of it lands where the pins stand.
    #[test]
    fn a_flipped_setup_on_the_pins_runs_with_x_y_from_them_after_an_e_stop() {
        use crate::control::Action;
        use crate::ui::work_zero_steps;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = pins_app(dir.path());
        idle(&mut a);
        machine_zero(&mut a);
        // The simulator's pins stand in front of its block, 6 mm tall on
        // the bottom of Z.
        let pin1 = conn(&a).sim_pin1().expect("simulated pins");
        assert_eq!(pin1, [155., 45.]);
        mdi(
            &mut a,
            &format!("G53 G0 X{} Y{} Z-71", pin1[0] + 0.4, pin1[1] - 0.3),
        );
        assert_eq!(conn(&a).record_blocked(), None);
        let _ = a.update(Message::Machine(Action::RecordPins));
        let (ok, said) = pinned(&mut a);
        assert!(ok, "{said}");
        assert!(said.contains("over 80.0 mm"), "{said}");
        // The app keeps pin 1's place in the machine's profile.
        let _ = a.update(Message::Tick);
        let kept = ["pin_ref_x", "pin_ref_y"]
            .map(|k| get(&a.doc.machines[0].p, k).parse::<f64>().unwrap());
        assert!((0..2).all(|k| (kept[k] - pin1[k]).abs() < 0.06), "{kept:?}");

        // Off to near the ends, E-stop, and a machine zero 1.3 mm and 0.7 mm
        // from where the first one was.
        mdi(&mut a, "G53 G0 Z0");
        mdi(&mut a, "G53 G0 X1.3 Y0.7");
        let _ = a.update(Message::MachineEStop);
        tick_until(&mut a, 2.0, "E-stop", |s| s.state == Some(State::EStop));
        let _ = a.update(Message::MachineReset);
        tick_until(&mut a, 3.0, "Idle after Reset", |s| {
            s.state == Some(State::Idle)
        });
        machine_zero(&mut a);
        assert!(!conn(&a).pin_referenced);
        let off = conn(&a).sim_pin1().unwrap();
        assert!(
            (off[0] - (pin1[0] - 1.3)).abs() < 0.06 && (off[1] - (pin1[1] - 0.7)).abs() < 0.06,
            "{off:?}"
        );
        check_run(&mut a, 1);
        let _ = a.view();
        let steps = |a: &App| work_zero_steps(a, conn(a)).expect("the guide").join("\n");
        assert!(
            steps(&a).contains("Reference X Y from pin 1 under Position"),
            "{}",
            steps(&a)
        );
        assert!(
            problems(&a)
                .iter()
                .any(|p| p.starts_with("X and Y are not referenced from pin 1")),
            "{:?}",
            problems(&a)
        );
        mdi(
            &mut a,
            &format!("G53 G0 X{} Y{}", off[0] + 0.5, off[1] + 0.4),
        );
        mdi(&mut a, "G53 G0 Z-71");
        let _ = a.update(Message::Machine(Action::ReferencePins));
        let (ok, said) = pinned(&mut a);
        assert!(ok, "{said}");
        assert!(
            said.contains("declared moved by +1.30") && said.contains("and +0.70"),
            "{said}"
        );
        assert!(conn(&a).pin_referenced);
        let back = conn(&a).sim_pin1().unwrap();
        assert!(
            (0..2).all(|k| (back[k] - kept[k]).abs() < 0.06),
            "{back:?} {kept:?}"
        );

        // G55, X Y from the pins: the offset is pin 1's place less where pin
        // 1 stands in work coordinates, in its hole.
        // Pressed again while the driver re-anchors after the declarations.
        for _ in 0..10 {
            if conn(&a).snapshot.active_wcs == 1 {
                break;
            }
            let _ = a.update(Message::Machine(Action::Wcs(1)));
            let end = std::time::Instant::now() + std::time::Duration::from_millis(500);
            while conn(&a).snapshot.active_wcs != 1 && std::time::Instant::now() < end {
                let _ = a.update(Message::Tick);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        assert_eq!(
            conn(&a).snapshot.active_wcs,
            1,
            "{:?}",
            conn(&a).snapshot.log
        );
        let want = conn(&a).pin_offset().unwrap();
        let pin1 = conn(&a).run_program().unwrap().pins.clone().unwrap().pin1;
        assert!(
            (0..2).all(|k| (want[k] - (kept[k] - pin1[k] as f64)).abs() < 1e-6),
            "{want:?} {kept:?} {pin1:?}"
        );
        assert!(
            problems(&a)
                .iter()
                .any(|p| p.contains("press Set X Y from the pins")),
            "{:?}",
            problems(&a)
        );
        // Pressed again should the driver still be taking the MDI line.
        let set = |a: &App| {
            let s = &conn(a).snapshot;
            (0..2).all(|k| (s.work_offset[k] - want[k]).abs() < 1e-3)
        };
        for _ in 0..10 {
            if set(&a) {
                break;
            }
            let _ = a.update(Message::Machine(Action::PinZero));
            let end = std::time::Instant::now() + std::time::Duration::from_millis(500);
            while !set(&a) && std::time::Instant::now() < end {
                let _ = a.update(Message::Tick);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        assert!(set(&a), "{:?}", conn(&a).snapshot.work_offset);
        let _ = a.view();
        assert!(
            steps(&a).contains("3. X and Y are set from the pins"),
            "{}",
            steps(&a)
        );
        // Z: the part is on the pins now; the touch plate on the stock top.
        let _ = a.update(Message::Machine(Action::ToSimZero));
        let at = conn(&a).sim_touch().unwrap();
        tick_until(&mut a, 10.0, "the move onto the block", |s| {
            s.state == Some(State::Idle) && (0..3).all(|k| (s.machine_pos[k] - at[k]).abs() < 0.06)
        });
        mdi(&mut a, &format!("G53 G0 Z{}", at[2] + 5.));
        let _ = a.update(Message::Machine(Action::ProbeZ));
        let (ok, said) = probed(&mut a);
        assert!(ok, "{said}");
        // Work Z on the stock top, at machine Z -40.
        tick_until(&mut a, 5.0, "work Z and the retract", |s| {
            (s.work_offset[2] - -40.).abs() < 0.06 && s.state == Some(State::Idle)
        });
        let offset = conn(&a).snapshot.work_offset;
        assert!(
            (0..2).all(|k| (offset[k] - want[k]).abs() < 1e-3),
            "Probe Z kept X Y: {offset:?}"
        );
        assert!(problems(&a).is_empty(), "{:?}", problems(&a));
        // Every pin of the setup, in its work coordinates, lands on the
        // machine where the simulator's pins stand.
        let p = conn(&a).run_program().unwrap().clone();
        let on = p.pins.clone().unwrap();
        for (i, c) in on.places.iter().enumerate() {
            let machine: Vec<f64> = (0..2)
                .map(|k| (c[k] - p.datum[k]) as f64 + offset[k])
                .collect();
            let stands = [back[0] + 80. * i as f64, back[1]];
            assert!(
                (0..2).all(|k| (machine[k] - stands[k]).abs() < 0.06),
                "pin {}: {machine:?} {stands:?}",
                i + 1
            );
        }
        let _ = a.update(Message::Run(crate::run::Action::CycleStart));
        assert!(run_of(&a).running.is_some());
        let _ = a.update(Message::MachineEStop);
        tick_until(&mut a, 2.0, "E-stop", |s| s.state == Some(State::EStop));
    }

    /// Draws `a`'s whole window, `size` px, headless (tiny-skia) to `path`.
    fn render_png(a: &App, size: iced::Size, path: &std::path::Path) {
        use futures_util::FutureExt;
        use iced::advanced::renderer::{self, Headless};
        let mut renderer = <iced::Renderer as Headless>::new(
            iced::Font::DEFAULT,
            iced::Pixels(16.),
            Some("tiny-skia"),
        )
        .now_or_never()
        .flatten()
        .expect("tiny-skia renderer");
        let mut ui = iced_runtime::UserInterface::build(
            a.view(),
            size,
            iced_runtime::user_interface::Cache::new(),
            &mut renderer,
        );
        ui.draw(
            &mut renderer,
            &iced::Theme::Light,
            &renderer::Style {
                text_color: iced::Color::BLACK,
            },
            iced::mouse::Cursor::Unavailable,
        );
        let physical = iced::Size::new(size.width as u32, size.height as u32);
        let rgba = renderer.screenshot(physical, 1., iced::Color::WHITE);
        let mut encoder = png::Encoder::new(
            std::io::BufWriter::new(std::fs::File::create(path).unwrap()),
            physical.width,
            physical.height,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&rgba)
            .unwrap();
        eprintln!("Rendered {}", path.display());
    }

    #[test]
    fn the_work_zero_step_picks_the_parts_holes_for_the_pins_in_the_3d_view() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = pins_app(dir.path());
        a.page = Page::Work;
        let _ = a.update(Message::SelectSetup(1));
        let _ = a.update(Message::SetupStep("Work zero"));
        let _ = a.update(Message::Edit(Target::Setup, "pin_holes", String::new()));
        let mesh = a.part_mesh().unwrap().clone();
        let setup = a.doc.setups[1].clone();
        let holes = crate::cam::bottom_holes(&setup, &mesh).unwrap();
        assert_eq!(holes.len(), 2);
        // A facet of the −X hole's wall, the +X hole's end and the face
        // they open on.
        let placement = crate::stock::Placement::new(&mesh, Some(&setup));
        let normal_z = |i: usize| {
            placement.transform(mesh.surfaces[i].normal)[2] - placement.transform([0.; 3])[2]
        };
        let of = |i| crate::cam::surface_hole(&setup, &mesh, &holes, i);
        let wall = (0..mesh.surfaces.len())
            .find(|&i| of(i) == Some(0) && normal_z(i).abs() < 0.01)
            .unwrap();
        let end = (0..mesh.surfaces.len())
            .find(|&i| of(i) == Some(1) && normal_z(i).abs() > 0.99)
            .unwrap();
        let face = (0..mesh.surfaces.len())
            .filter(|&i| of(i).is_none() && normal_z(i).abs() > 0.99)
            .find(|&i| {
                let z = placement.transform(mesh.surfaces[i].center)[2] / mesh.display_scale();
                (z - holes[0].z).abs() < 0.01
            })
            .unwrap();
        let pick = |a: &mut App, n: usize| {
            let _ = a.update(Message::Viewport(crate::viewport::Event::PickSurface(n)));
            crate::post::chosen_pin_holes(&a.doc.setups[1]).len()
        };
        // Not picking: a click chooses nothing.
        assert_eq!(pick(&mut a, wall), 0);
        let _ = a.update(Message::PickPinHoles);
        assert!(a.picking_pin_holes());
        assert_eq!(pick(&mut a, wall), 1);
        // The face the holes open on is no hole.
        assert_eq!(pick(&mut a, face), 1);
        assert!(
            a.status.starts_with("That is not in a round hole"),
            "{}",
            a.status
        );
        assert_eq!(pick(&mut a, end), 2);
        let stock = a.known_stock(1).0.unwrap();
        let datum = crate::post::datum(&a.doc.setups[1], stock, &mesh).unwrap();
        let on = crate::post::placed_on_pins(&a.doc.setups[1], &mesh, stock, datum).unwrap();
        assert!(
            (on.places[0][0] - holes[0].center[0]).abs() < 0.01,
            "{on:?}"
        );
        // A second click on a chosen hole removes it.
        assert_eq!(pick(&mut a, wall), 1);
        // Away from the Work zero step the clicks pick nothing.
        let _ = a.update(Message::SetupStep("Stock"));
        assert!(!a.picking_pin_holes());
        assert_eq!(pick(&mut a, wall), 1);
    }

    /// `post::fixture::leftover_flip("rx", tool)` in the app: the cube
    /// flipped onto pins in its leftover, on the flipped setup's Work zero
    /// step.
    fn leftover_pins_app(tool: Option<&str>) -> App {
        let mut a = app();
        let (doc, mesh) = crate::post::fixture::leftover_flip("rx", tool);
        a.replace_document(doc, None);
        a.imported_mesh = Some(mesh);
        a.page = Page::Work;
        let _ = a.update(Message::SelectSetup(1));
        let _ = a.update(Message::SetupStep("Work zero"));
        a
    }

    #[test]
    fn the_work_zero_step_places_the_pins_on_the_leftover_in_the_3d_view() {
        use crate::viewport::Event;
        let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) < 0.01;
        let mut a = leftover_pins_app(None);
        let mesh = a.part_mesh().unwrap().clone();
        let part = crate::simulation::part_box(&a.doc.setups[1], &mesh);
        let placed = crate::post::leftover_places(&a.doc, &mesh, 1).unwrap();
        // Not placing: the pointer and a click move nothing.
        let _ = a.update(Message::Viewport(Event::AimPins(Some([0., 0.]))));
        let _ = a.update(Message::Viewport(Event::PlacePins([0., 0.])));
        assert_eq!(a.pin_aim, None);
        assert!(near(
            crate::post::leftover_places(&a.doc, &mesh, 1).unwrap()[0],
            placed[0]
        ));
        assert!(!crate::ui::pin_aim(&a).on);
        let _ = a.update(Message::PlacePins);
        assert!(a.placing_leftover_pins());
        assert!(!a.picking_pin_holes());
        // Over the part they are red, with why.
        let y = (part.min[1] + part.max[1]) / 2.;
        let _ = a.update(Message::Viewport(Event::AimPins(Some([
            part.min[0] + 5.,
            y,
        ]))));
        let aim = crate::ui::pin_aim(&a);
        assert!(aim.on && aim.offsets.len() == 2, "{aim:?}");
        assert!(near(aim.placed.unwrap(), placed[0]), "{aim:?}");
        assert_eq!(
            aim.ghost.unwrap().1,
            Err(
                "Pin 1 would stand under the part: place the pins on the leftover beside it."
                    .into()
            )
        );
        // A click there leaves them where they were, and says why.
        let _ = a.update(Message::Viewport(Event::PlacePins([part.min[0] + 5., y])));
        assert!(near(
            crate::post::leftover_places(&a.doc, &mesh, 1).unwrap()[0],
            placed[0]
        ));
        assert_eq!(
            a.status,
            "The pins do not fit there: Pin 1 would stand under the part: place the pins on the leftover beside it."
        );
        // On the leftover they fit, 2 mm further from the part, and a click
        // puts pin 1 there.
        let to = [placed[0][0] - 2., placed[0][1]];
        let _ = a.update(Message::Viewport(Event::AimPins(Some(to))));
        let (_, fit) = crate::ui::pin_aim(&a).ghost.unwrap();
        let (gap, _) = fit.unwrap();
        assert!((gap - 11.95).abs() < 0.01, "{gap}");
        let _ = a.view();
        let _ = a.update(Message::Viewport(Event::PlacePins(to)));
        assert!(near(
            crate::post::leftover_places(&a.doc, &mesh, 1).unwrap()[0],
            to
        ));
        // Add the pin holes to Top: a Drill of them with the Ø4.1 drill,
        // its holes where the pins are.
        let _ = a.update(Message::AddPinDrilling);
        assert!(
            a.status
                .starts_with("Added Pin holes to Top, with Drill 4.1."),
            "{}",
            a.status
        );
        let op = a.doc.setups[0].operations.last().unwrap().clone();
        assert!(op.drills_pin_holes());
        assert_eq!(
            a.doc.tools.iter().find(|t| t.id == op.tool).unwrap().name(),
            "Drill 4.1"
        );
        assert_eq!(crate::post::pin_drillings(&a.doc, 1), vec![(0, 1)]);
        let points = crate::post::drill_pin_points(&op);
        assert_eq!(points.len(), 2);
        let _ = a.view();
        // Moving the pins moves their holes.
        let _ = a.update(Message::Viewport(Event::PlacePins([to[0], to[1] + 1.])));
        let moved = crate::post::drill_pin_points(&a.doc.setups[0].operations[1]);
        assert!(moved.len() == 2 && moved != points, "{moved:?} {points:?}");
        // Away from the Work zero step the pointer places nothing.
        let _ = a.update(Message::SetupStep("Stock"));
        assert!(!a.placing_leftover_pins());
        assert!(!crate::ui::pin_aim(&a).on);
    }

    /// Renders the pins placed in the stock's leftover to PNGs in
    /// `$AXIS_RENDER_DIR` (default /tmp/leftover-pins): the flipped setup's
    /// Work zero step with the pins under the pointer where they fit and
    /// where they do not, then placed; setup 1's Drill of their holes, its
    /// drilled and its milled holes; the pins seen from below. Not a check:
    /// run it with --ignored.
    #[test]
    #[ignore]
    fn render_the_pins_in_the_leftover() {
        use crate::viewport::Event;
        let out = std::path::PathBuf::from(
            std::env::var("AXIS_RENDER_DIR").unwrap_or("/tmp/leftover-pins".into()),
        );
        std::fs::create_dir_all(&out).unwrap();
        let tall = iced::Size::new(1600., 1300.);
        let dir = tempfile::tempdir().unwrap();
        let mut a = leftover_pins_app(None);
        a.smoke_dir = Some(dir.path().to_path_buf());
        let _ = a.refresh_left_stock();
        a.smoke_dir = None;
        let mesh = a.part_mesh().unwrap().clone();
        let part = crate::simulation::part_box(&a.doc.setups[1], &mesh);
        let placed = crate::post::leftover_places(&a.doc, &mesh, 1).unwrap();
        let _ = a.update(Message::PlacePins);
        let _ = a.update(Message::Viewport(Event::AimPins(Some([
            placed[0][0] - 1.,
            placed[0][1] + 3.,
        ]))));
        render_png(&a, tall, &out.join("1-aim-fits.png"));
        let y = (part.min[1] + part.max[1]) / 2.;
        let _ = a.update(Message::Viewport(Event::AimPins(Some([
            part.min[0] - 5.,
            y,
        ]))));
        render_png(&a, tall, &out.join("2-aim-too-near-the-part.png"));
        let _ = a.update(Message::PlacePins);
        render_png(&a, tall, &out.join("3-placed.png"));
        a.camera.pitch = -0.6;
        render_png(&a, tall, &out.join("4-placed-from-below.png"));
        a.camera = Default::default();
        let _ = a.update(Message::AddPinDrilling);
        let _ = a.view();
        render_png(&a, tall, &out.join("5-add-the-pin-holes.png"));
        let _ = a.update(Message::SelectSetup(0));
        let _ = a.update(Message::SelectOp(1));
        let _ = a.update(Message::Tab("Geometry".into()));
        // At the end of each, with the stock it leaves meshed in line.
        let ended = |a: &mut App| {
            let _ = a.update(Message::Generate);
            let _ = a.update(Message::Scrub(1.));
            let key = a.stock_mesh_key().unwrap();
            let sim = a.active_simulation().unwrap();
            let volume = sim.volume.as_ref().unwrap();
            let result = crate::stock_surface::Snapshot {
                bounds: volume.bounds,
                frame: volume.frame,
                cuts: volume.cuts.clone(),
                detail: crate::stock_surface::Detail::Fine,
            }
            .build()
            .map(std::sync::Arc::new);
            let seconds = sim.seconds;
            a.stock_mesh_job = None;
            let _ = a.update(Message::StockMeshed {
                key,
                seconds,
                result,
            });
        };
        ended(&mut a);
        render_png(&a, tall, &out.join("6-pin-holes-drilled.png"));
        a.camera.zoom = 2.5;
        render_png(&a, tall, &out.join("7-pin-holes-drilled-close.png"));
        a.camera = Default::default();
        let flat = (a.doc.tools.iter())
            .find(|t| t.name() == "Flat 3")
            .unwrap()
            .id
            .clone();
        let _ = a.update(Message::SetOpTool(flat));
        ended(&mut a);
        a.camera.zoom = 2.5;
        render_png(&a, tall, &out.join("8-pin-holes-milled-close.png"));
    }

    /// Renders the fixture pins to PNGs in `$AXIS_RENDER_DIR` (default
    /// /tmp/fp-renders) for a look: the Machine library's pins, the flipped
    /// setup's Work zero step before and after its holes are chosen for the
    /// pins, setup 1's Drill of those holes, and the Control page's Run steps
    /// before and after X Y from the pins. Not a check: run it with
    /// --ignored.
    #[test]
    #[ignore]
    fn render_the_fixture_pins() {
        use crate::control::Action;
        use uc100::controller::State;
        let out = std::path::PathBuf::from(
            std::env::var("AXIS_RENDER_DIR").unwrap_or("/tmp/fp-renders".into()),
        );
        std::fs::create_dir_all(&out).unwrap();
        let size = iced::Size::new(1600., 1000.);
        let dir = tempfile::tempdir().unwrap();
        let mut a = pins_app(dir.path());
        // The Machine library's UC100 step.
        a.page = Page::Machines;
        let id = a.doc.machines[0].id.clone();
        let _ = a.update(Message::EditMachine(id));
        let _ = a.update(Message::WizardStep(3));
        render_png(
            &a,
            iced::Size::new(1600., 2600.),
            &out.join("1-machine-pins.png"),
        );
        // The flipped setup's Work zero step, the stock setup 1 leaves.
        a.page = Page::Work;
        let _ = a.update(Message::SelectSetup(1));
        let _ = a.update(Message::SetupStep("Work zero"));
        // Worked out in line, as in the smoke run.
        a.smoke_dir = Some(dir.path().to_path_buf());
        let _ = a.refresh_left_stock();
        a.smoke_dir = None;
        let key = a.holding_key().unwrap();
        a.ready = crate::run::ready(&a.doc, a.part_mesh().unwrap(), 1, key);
        let _ = a.view();
        let tall = iced::Size::new(1600., 1700.);
        // Before the part's holes are chosen for the pins, then with them.
        let chosen = get(&a.doc.setups[1].p, "pin_holes").to_string();
        let edit = |a: &mut App, v: String| {
            let _ = a.update(Message::Edit(Target::Setup, "pin_holes", v));
            a.ready =
                crate::run::ready(&a.doc, a.part_mesh().unwrap(), 1, a.holding_key().unwrap());
            let _ = a.view();
        };
        edit(&mut a, String::new());
        render_png(&a, tall, &out.join("2a-work-zero-choose-holes.png"));
        edit(&mut a, chosen);
        render_png(&a, tall, &out.join("2-work-zero-step.png"));
        // Setup 1's Drill of the part's dowel holes, found in its top face.
        let _ = a.update(Message::SelectSetup(0));
        let drill = a.doc.setups[0].operations.len() - 1;
        let _ = a.update(Message::SelectOp(drill));
        let _ = a.update(Message::Tab("Geometry".into()));
        let _ = a.view();
        render_png(&a, tall, &out.join("3-drill-dowel-holes.png"));
        let _ = a.update(Message::SelectSetup(1));
        // Control: pins recorded, then an E-stop: the Run steps for the
        // flipped setup on the pins.
        a.page = Page::Control;
        idle(&mut a);
        machine_zero(&mut a);
        let pin1 = conn(&a).sim_pin1().unwrap();
        mdi(
            &mut a,
            &format!("G53 G0 X{} Y{} Z-71", pin1[0] + 0.4, pin1[1] - 0.3),
        );
        let _ = a.update(Message::Machine(Action::RecordPins));
        assert!(pinned(&mut a).0);
        let _ = a.update(Message::Tick);
        mdi(&mut a, "G53 G0 Z0");
        mdi(&mut a, "G53 G0 X1.3 Y0.7");
        let _ = a.update(Message::MachineEStop);
        tick_until(&mut a, 2.0, "E-stop", |s| s.state == Some(State::EStop));
        let _ = a.update(Message::MachineReset);
        tick_until(&mut a, 3.0, "Idle after Reset", |s| {
            s.state == Some(State::Idle)
        });
        machine_zero(&mut a);
        check_run(&mut a, 1);
        let _ = a.view();
        render_png(&a, size, &out.join("4-run-steps-after-e-stop.png"));
        let at = conn(&a).sim_pin1().unwrap();
        mdi(
            &mut a,
            &format!("G53 G0 X{} Y{} Z-71", at[0] + 0.5, at[1] + 0.4),
        );
        let _ = a.update(Message::Machine(Action::ReferencePins));
        let (ok, said) = pinned(&mut a);
        assert!(ok, "{said}");
        for _ in 0..10 {
            if conn(&a).snapshot.active_wcs == 1 {
                break;
            }
            let _ = a.update(Message::Machine(Action::Wcs(1)));
            for _ in 0..25 {
                let _ = a.update(Message::Tick);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        let _ = a.view();
        render_png(&a, size, &out.join("5-run-steps-referenced.png"));
        let want = conn(&a).pin_offset().unwrap();
        for _ in 0..10 {
            if (0..2).all(|k| (conn(&a).snapshot.work_offset[k] - want[k]).abs() < 1e-3) {
                break;
            }
            let _ = a.update(Message::Machine(Action::PinZero));
            for _ in 0..25 {
                let _ = a.update(Message::Tick);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        let _ = a.update(Message::Machine(Action::ToSimZero));
        let touch = conn(&a).sim_touch().unwrap();
        tick_until(&mut a, 10.0, "onto the block", |s| {
            s.state == Some(State::Idle)
                && (0..3).all(|k| (s.machine_pos[k] - touch[k]).abs() < 0.06)
        });
        let _ = a.update(Message::Machine(Action::TouchOff));
        for _ in 0..25 {
            let _ = a.update(Message::Tick);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = a.view();
        render_png(&a, size, &out.join("6-run-steps-ready.png"));
        let _ = a.update(Message::MachineEStop);
    }

    /// FAST with a probe input. Its simulated block (`control::sim_block`)
    /// is X 145…245, Y 90…190, its top at machine Z -40.
    fn probe_profile() -> String {
        format!("{FAST}[inputs]\nprobe = {{ pin = 13, active_low = true }}\n")
    }

    /// A 10 mm touch plate probed at `feed` mm/min, set before connecting.
    fn plate(a: &mut App, feed: &str) {
        let m = a.doc.machines.last_mut().unwrap();
        m.p.insert("probe_plate".into(), "10".into());
        m.p.insert("probe_feed".into(), feed.into());
        for s in &mut a.doc.setups {
            s.machine = m.clone();
        }
    }

    /// The project's material is the first starter of `group`: on a metal
    /// the plate's lead goes on the stock, which the probes then touch.
    fn material(a: &mut App, group: &str) {
        let m = crate::material::starters()
            .into_iter()
            .find(|m| get(&m.p, "group") == group)
            .expect("a starter of the group");
        let id = match a.doc.materials.iter().find(|x| x.name() == m.name()) {
            Some(x) => x.id.clone(),
            None => {
                let id = m.id.clone();
                a.doc.materials.push(m);
                id
            }
        };
        assert!(a.doc.select_material(&id));
    }

    /// Ticks until the probe or probing routine has ended; how it ended.
    fn probed(a: &mut App) -> (bool, String) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            let _ = a.update(Message::Tick);
            let _ = a.view();
            let c = conn(a);
            if let (false, Some(p)) = (c.probing(), &c.probed) {
                return p.clone();
            }
            assert!(
                std::time::Instant::now() < end,
                "timed out waiting for the probe: {:?} at {:?} {:?}",
                c.snapshot.state,
                c.snapshot.machine_pos,
                c.snapshot.log.iter().rev().take(5).collect::<Vec<_>>()
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    fn blocked(a: &App) -> String {
        conn(a).probe_blocked().unwrap_or_default()
    }

    #[test]
    fn probe_z_sets_work_z_at_the_plate_and_lifts() {
        use crate::control::Action;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = profile_app(dir.path(), &probe_profile());
        plate(&mut a, "1000");
        a.page = Page::Control;
        idle(&mut a);
        assert!(
            blocked(&a).contains("Z has no machine position"),
            "{}",
            blocked(&a)
        );
        let _ = a.update(Message::Machine(Action::ProbeZ));
        assert!(!conn(&a).probing());
        let _ = a.update(Message::Machine(Action::MachineZero(None)));
        tick_until(&mut a, 3.0, "machine zero", |s| {
            s.referenced[..3].iter().all(|r| *r)
        });
        // From Z -10 the 20 mm probe ends at Z -30, above the block: a miss
        // sets nothing.
        mdi(&mut a, "G0 X195 Y140 Z-10");
        assert_eq!(blocked(&a), "");
        let _ = a.update(Message::Machine(Action::ProbeZ));
        assert!(a.locked());
        let _ = a.view();
        let (ok, message) = probed(&mut a);
        assert!(
            !ok && message.contains("No contact down to machine Z -30.000"),
            "{message}"
        );
        assert!(conn(&a).snapshot.work_offset[2].abs() < 1e-9);
        assert!(!a.locked());
        // From Z -30: contact at the block's top, Z -40, which becomes the
        // plate's 10 mm; then the tool lifts 5 mm.
        mdi(&mut a, "G0 Z-30");
        let _ = a.update(Message::Machine(Action::ProbeZ));
        let (ok, message) = probed(&mut a);
        assert!(ok, "{message}");
        tick_until(&mut a, 3.0, "the retract", |s| {
            s.state == Some(State::Idle) && (s.machine_pos[2] - -35.).abs() < 0.06
        });
        let s = &conn(&a).snapshot;
        assert!(
            (s.work_offset[2] - -50.).abs() < 0.06,
            "{:?}",
            s.work_offset
        );
        assert!((s.work_pos[2] - 15.).abs() < 0.06, "{:?}", s.work_pos);
        assert!(!a.locked());
    }

    /// On a metal the plate's lead goes on the stock and the tool touches
    /// it: Probe Z needs no plate and adds none. On wood it waits for one.
    #[test]
    fn probe_z_on_metal_touches_the_stock_without_the_plate() {
        use crate::control::Action;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = profile_app(dir.path(), &probe_profile());
        let m = a.doc.machines.last_mut().unwrap();
        m.p.insert("probe_feed".into(), "1000".into());
        a.page = Page::Control;
        material(&mut a, "Wood and composite");
        idle(&mut a);
        let _ = a.update(Message::Machine(Action::MachineZero(None)));
        tick_until(&mut a, 3.0, "machine zero", |s| {
            s.referenced[..3].iter().all(|r| *r)
        });
        assert!(
            blocked(&a).starts_with("Touch plate thickness is not set"),
            "{}",
            blocked(&a)
        );
        material(&mut a, "Aluminium");
        let _ = a.update(Message::Tick);
        assert_eq!(blocked(&a), "");
        let _ = a.view();
        // From Z -30: contact at the block's top, Z -40, which becomes the
        // stock top's Z 0 with no plate; then the tool lifts 5 mm.
        mdi(&mut a, "G0 X195 Y140 Z-30");
        let _ = a.update(Message::Machine(Action::ProbeZ));
        let (ok, message) = probed(&mut a);
        assert!(
            ok && message.ends_with("Work Z set: the surface touched is Z 0.000."),
            "{message}"
        );
        tick_until(&mut a, 3.0, "the retract", |s| {
            s.state == Some(State::Idle) && (s.machine_pos[2] - -35.).abs() < 0.06
        });
        let s = &conn(&a).snapshot;
        assert!(
            (s.work_offset[2] - -40.).abs() < 0.06,
            "{:?}",
            s.work_offset
        );
        assert!((s.work_pos[2] - 5.).abs() < 0.06, "{:?}", s.work_pos);
    }

    /// Pictures of the Control page's probes on wood and on aluminium, for
    /// a look: `cargo test render_the_probes_by_material -- --ignored`,
    /// written to `AXIS_RENDER_DIR` (default /tmp/probe-renders).
    #[test]
    #[ignore]
    fn render_the_probes_by_material() {
        let out = std::path::PathBuf::from(
            std::env::var("AXIS_RENDER_DIR").unwrap_or("/tmp/probe-renders".into()),
        );
        std::fs::create_dir_all(&out).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut a = edges_app(dir.path(), Some("Stock top front-left"));
        plate(&mut a, "100");
        let id = a.doc.machines[0].id.clone();
        if let Some(c) = a.control.as_mut().filter(|c| c.machine_id == id) {
            c.probe = crate::control::probe_settings(&a.doc.machines[0].p);
        }
        for (group, file) in [
            ("Wood and composite", "1-wood.png"),
            ("Aluminium", "2-aluminium.png"),
        ] {
            material(&mut a, group);
            let _ = a.update(Message::Tick);
            render_png(&a, iced::Size::new(1600., 2400.), &out.join(file));
        }
    }

    #[test]
    fn probe_z_waits_for_a_probe_input_a_plate_and_a_free_input() {
        use crate::control::Action;
        // No probe input in the profile.
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        zeroed(&mut a);
        assert!(blocked(&a).contains("no probe input"), "{}", blocked(&a));
        let dir = tempfile::tempdir().unwrap();
        let mut a = profile_app(dir.path(), &probe_profile());
        plate(&mut a, "1000");
        zeroed(&mut a);
        assert_eq!(blocked(&a), "");
        // The tool inside the block: the probe input is active.
        mdi(&mut a, "G53 G0 Z-45");
        tick_until(&mut a, 2.0, "the probe input", |_| true);
        assert!(
            blocked(&a).contains("probe input is active"),
            "{}",
            blocked(&a)
        );
        let _ = a.update(Message::Machine(Action::ProbeZ));
        assert!(!conn(&a).probing());
        mdi(&mut a, "G53 G0 Z-30");
        assert_eq!(blocked(&a), "");
        // Beside the block at the bottom of Z nothing is left to probe: the
        // driver refuses and says why.
        mdi(&mut a, "G53 G0 X10 Z-80");
        let _ = a.update(Message::Machine(Action::ProbeZ));
        let (ok, message) = probed(&mut a);
        assert!(
            !ok && message.contains("not started, the target is where the machine already is"),
            "{message}"
        );
        assert!(!a.locked());
        // A plate without a thickness, saved while connected.
        a.machine_draft = a.doc.machines[0].clone();
        a.machine_draft.p.remove("probe_plate");
        let _ = a.update(Message::SaveMachine);
        assert!(
            blocked(&a).contains("Touch plate thickness"),
            "{}",
            blocked(&a)
        );
        // E-stop.
        a.machine_draft.p.insert("probe_plate".into(), "10".into());
        let _ = a.update(Message::SaveMachine);
        let _ = a.update(Message::MachineEStop);
        tick_until(&mut a, 2.0, "E-stop", |s| {
            s.state == Some(uc100::controller::State::EStop)
        });
        assert!(blocked(&a).contains("not Idle"), "{}", blocked(&a));
        let _ = a.view();
    }

    #[test]
    fn e_stop_stops_a_probe() {
        use crate::control::Action;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = profile_app(dir.path(), &probe_profile());
        plate(&mut a, "60");
        zeroed(&mut a);
        mdi(&mut a, "G53 G0 Z-25");
        let _ = a.update(Message::Machine(Action::ProbeZ));
        tick_until(&mut a, 2.0, "the probe", |s| s.state != Some(State::Idle));
        let _ = a.update(Message::MachineEStop);
        let (ok, message) = probed(&mut a);
        assert!(!ok && message.contains("E-stop"), "{message}");
        assert!(!a.locked());
        assert!((conn(&a).snapshot.work_offset[2] - -20.).abs() < 1e-9);
    }

    #[test]
    fn a_tool_change_probes_the_reference_surface() {
        use crate::run::Action as R;
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app_on(dir.path(), &probe_profile());
        plate(&mut a, "1000");
        zeroed(&mut a);
        check_run(&mut a, 0);
        let _ = a.view();
        // The plate on the block (top at machine Z -40, work Z -20): the
        // surface under it is work Z -30.
        mdi(&mut a, "G53 G0 Z-30");
        let _ = a.update(Message::Run(R::ProbeReference));
        let (ok, message) = probed(&mut a);
        assert!(ok, "{message}");
        let reference = run_of(&a).reference.unwrap();
        assert!((reference - -30.).abs() < 0.06, "{reference}");
        // Z zeroed wrong, as a new tool would be: probing the reference
        // puts it back and counts as the touch-off.
        mdi(&mut a, "G53 G0 Z-30");
        let _ = a.update(Message::Machine(crate::control::Action::WorkZero(Some(2))));
        tick_until(&mut a, 2.0, "work Z", |s| s.work_pos[2].abs() < 1e-6);
        let _ = a.update(Message::Run(R::ProbeFromReference));
        let (ok, message) = probed(&mut a);
        assert!(ok, "{message}");
        tick_until(&mut a, 2.0, "work Z set", |s| {
            (s.work_offset[2] - -20.).abs() < 0.06
        });
        assert!(run_of(&a).touched);
        let _ = a.view();
    }

    /// The fixture's top setup (Stock top center, or `origin`) checked on a
    /// zeroed FAST machine with a probe input and a 4 mm probe tip probed at
    /// 1000 mm/min. The simulator's block takes the stock's X/Y size round
    /// X195 Y140, its top at machine Z -40.
    fn edges_app(dir: &std::path::Path, origin: Option<&str>) -> App {
        let mut a = run_app_on(dir, &probe_profile());
        let m = a.doc.machines.last_mut().unwrap();
        m.p.insert("probe_tip".into(), "4".into());
        m.p.insert("probe_feed".into(), "1000".into());
        for s in &mut a.doc.setups {
            s.machine = m.clone();
        }
        material(&mut a, "Aluminium");
        if let Some(o) = origin {
            a.doc.setups[0].p.insert("origin".into(), o.into());
        }
        zeroed(&mut a);
        check_run(&mut a, 0);
        a
    }

    fn edges_blocked(a: &App) -> String {
        conn(a).edges_blocked().unwrap_or_default()
    }

    /// Asserts the work offset X, Y and Z within 0.15 mm: the simulator's
    /// probe trips at the first status inside the block, up to 4 ms late.
    fn work_offset(a: &App, expected: [f64; 3]) {
        let s = &conn(a).snapshot;
        assert!(
            (0..3).all(|k| (s.work_offset[k] - expected[k]).abs() < 0.15),
            "{:?}, expected {expected:?}",
            s.work_offset
        );
    }

    #[test]
    fn probe_centre_sets_the_work_zero_on_the_middle_of_the_block() {
        use crate::control::Action;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = edges_app(dir.path(), None);
        let (lo, hi) = conn(&a).block.unwrap();
        let stock = run_of(&a).checked().unwrap()[0].stock.dimensions();
        assert!(
            (hi[0] - lo[0] - stock[0] as f64).abs() < 1e-3,
            "{lo:?} {hi:?}"
        );
        assert!(
            (hi[1] - lo[1] - stock[1] as f64).abs() < 1e-3,
            "{lo:?} {hi:?}"
        );
        assert_eq!(
            ((lo[0] + hi[0]) / 2., (lo[1] + hi[1]) / 2., hi[2]),
            (195., 140., -40.)
        );
        // 3 mm right of the middle and 2 mm in front of it, 10 mm above the
        // top: inside the top's spot.
        mdi(&mut a, "G53 G0 X198 Y138 Z-30");
        assert_eq!(edges_blocked(&a), "");
        let _ = a.view();
        let _ = a.update(Message::Machine(Action::ProbeEdges));
        assert!(a.locked() && conn(&a).edging());
        let (ok, message) = probed(&mut a);
        assert!(ok, "{message}");
        assert!(
            message.contains("on the centre of the stock top")
                && message.contains("from 5 touches"),
            "{message}"
        );
        // Backed off the +Y side and up 10 mm above the top.
        tick_until(&mut a, 5.0, "backing off", |s| {
            s.state == Some(State::Idle) && (s.machine_pos[2] - -30.).abs() < 0.2
        });
        work_offset(&a, [195., 140., -40.]);
        assert!(conn(&a).snapshot.machine_pos[1] > hi[1]);
        assert!(!a.locked() && run_of(&a).touched);
        let _ = a.view();
    }

    #[test]
    fn probe_corner_sets_the_work_zero_on_the_front_left_corner() {
        use crate::control::Action;
        let dir = tempfile::tempdir().unwrap();
        let mut a = edges_app(dir.path(), Some("Stock top front-left"));
        // The simulator's probe is a point: allowing for a 4 mm tip puts the
        // sides it finds, and so the corner, 2 mm inside the block's.
        let (lo, _) = conn(&a).block.unwrap();
        let corner = [lo[0] + 2., lo[1] + 2., -40.];
        // 2 mm in from both edges: on the top's spot (1 to 11 mm), short of
        // the -X side's (5% to 15% of the stock's depth): -X is touched
        // there again once -Y has fixed Y.
        mdi(
            &mut a,
            &format!("G53 G0 X{} Y{} Z-30", corner[0] + 2., corner[1] + 2.),
        );
        let _ = a.update(Message::Machine(Action::ProbeEdges));
        let (ok, message) = probed(&mut a);
        assert!(ok, "{message}");
        assert!(
            message.contains("on the front-left corner") && message.contains("from 4 touches"),
            "{message}"
        );
        tick_until(&mut a, 5.0, "work zero", |s| s.work_offset[2] < -39.);
        work_offset(&a, corner);
    }

    /// A front-left zero probed from the back-right corner, as when a jaw
    /// stands at the front-left: the probe touches the top and the +X and +Y
    /// sides there and puts the zero on the front-left corner by the
    /// setup's stock size, which the simulator's block has.
    #[test]
    fn the_setups_zero_is_probed_from_the_corner_picked() {
        use crate::control::Action;
        use crate::post::At::{Max, Min};
        let dir = tempfile::tempdir().unwrap();
        let mut a = edges_app(dir.path(), Some("Stock top front-left"));
        let spots = run_of(&a).checked().unwrap()[0].probe_from.clone();
        assert_eq!(spots.len(), 9);
        assert!(spots.iter().all(|(_, ok)| ok.is_ok()), "{spots:?}");
        // The zero's own spot is the default; picking it again is None.
        assert_eq!(conn(&a).edges_setup().unwrap().0, [Min, Min]);
        let _ = a.update(Message::Machine(Action::ProbeFrom(Some([Max, Max]))));
        assert_eq!(conn(&a).edges_setup().unwrap().0, [Max, Max]);
        let _ = a.view();
        let (lo, hi) = conn(&a).block.unwrap();
        // The simulator's probe is a point: allowing for the 4 mm tip puts
        // each side it finds 2 mm inside the block's.
        let corner = [lo[0] + 2., lo[1] + 2., -40.];
        mdi(
            &mut a,
            &format!("G53 G0 X{} Y{} Z-30", hi[0] - 4., hi[1] - 4.),
        );
        let _ = a.update(Message::Machine(Action::ProbeEdges));
        let (ok, message) = probed(&mut a);
        assert!(ok, "{message}");
        assert!(
            message.contains("Work zero set on Stock top front-left")
                && message.contains("placed from the back-right corner"),
            "{message}"
        );
        tick_until(&mut a, 5.0, "work zero", |s| s.work_offset[2] < -39.);
        // The back-right corner, also 2 mm inside, less the stock's size.
        work_offset(&a, [corner[0] - 4., corner[1] - 4., -40.]);
        // A new setup picked to run starts from its own spot again.
        check_run(&mut a, 0);
        assert_eq!(conn(&a).probe_spot, None);
    }

    /// The probing screen's features on the simulator's block, from where
    /// the tip is put: the top, a side and an outside corner, each setting
    /// work zero on what it found and leaving the other axes alone.
    #[test]
    fn a_feature_is_probed_where_the_tip_is_and_becomes_work_zero() {
        use crate::control::{Action, Feature};
        use crate::post::At::Min;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = edges_app(dir.path(), None);
        let (lo, hi) = conn(&a).block.unwrap();
        let mid = [(lo[0] + hi[0]) / 2., (lo[1] + hi[1]) / 2.];
        let probe = |a: &mut App, f: Feature| {
            assert_eq!(conn(a).feature_blocked(f), None);
            let _ = a.update(Message::Machine(Action::ProbeFeature(f)));
            assert!(a.locked() && conn(a).feature() == Some(f));
            let _ = a.view();
            let (ok, message) = probed(a);
            assert!(ok, "{message}");
            // The work offset is set, then the tip moves back.
            tick_for(a, 100);
            tick_until(a, 5.0, "back where it started", |s| {
                s.state == Some(State::Idle)
            });
            message
        };
        // Z top: Z0 on the block's top, X and Y as they were.
        let before = conn(&a).snapshot.work_offset;
        mdi(&mut a, &format!("G53 G0 X{} Y{} Z-30", mid[0], mid[1]));
        let message = probe(&mut a, Feature::Top);
        assert!(message.contains("Z0 set on the surface"), "{message}");
        work_offset(&a, [before[0], before[1], -40.]);
        // X+ from left of the block, 5 mm below its top.
        mdi(&mut a, &format!("G53 G0 X{} Y{}", lo[0] - 8., mid[1]));
        mdi(&mut a, "G53 G0 Z-45");
        let message = probe(&mut a, Feature::Edge(0, true));
        assert!(message.contains("X0 set on the left side"), "{message}");
        work_offset(&a, [lo[0] + 2., before[1], -40.]);
        assert!((conn(&a).snapshot.machine_pos[0] - (lo[0] - 8.)).abs() < 0.01);
        // X− from right of the block: X0 on its right side.
        // Over the block, not through it.
        let over = |a: &mut App, x: f64, y: f64| {
            mdi(a, "G53 G0 Z-30");
            mdi(a, &format!("G53 G0 X{x} Y{y}"));
            mdi(a, "G53 G0 Z-45");
        };
        over(&mut a, hi[0] + 8., mid[1]);
        let message = probe(&mut a, Feature::Edge(0, false));
        assert!(message.contains("X0 set on the right side"), "{message}");
        work_offset(&a, [hi[0] - 2., before[1], -40.]);
        // Outside the front-left corner, 5 mm off both sides.
        over(&mut a, lo[0] - 5., lo[1] - 5.);
        let message = probe(&mut a, Feature::Corner([Min, Min]));
        assert!(message.contains("on the front-left corner"), "{message}");
        work_offset(&a, [lo[0] + 2., lo[1] + 2., -40.]);
        let s = &conn(&a).snapshot;
        assert!(
            (s.machine_pos[0] - (lo[0] - 5.)).abs() < 0.01,
            "{:?}",
            s.machine_pos
        );
        assert!(
            (s.machine_pos[1] - (lo[1] - 5.)).abs() < 0.01,
            "{:?}",
            s.machine_pos
        );
        // Started in front of the block but not left of it, the corner's
        // step along Y runs into it: nothing is set.
        over(&mut a, lo[0] + 5., lo[1] - 5.);
        let before = conn(&a).snapshot.work_offset;
        let _ = a.update(Message::Machine(Action::ProbeFeature(Feature::Corner([
            Min, Min,
        ]))));
        let (ok, message) = probed(&mut a);
        assert!(
            !ok && message.contains("stepping along the side"),
            "{message}"
        );
        assert_eq!(conn(&a).snapshot.work_offset, before);
    }

    /// A milling machine DRO's ½, Set and Go to X0 Y0.
    #[test]
    fn half_set_and_go_to_x0_y0_work_as_on_a_dro() {
        use crate::control::Action;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        zeroed(&mut a);
        // Zeroed at machine X195: 40 mm to the right reads X40, ½ makes it X20.
        mdi(&mut a, "G0 X40 Y-12");
        let _ = a.update(Message::Machine(Action::WorkHalf(0)));
        tick_until(&mut a, 2.0, "X halved", |s| {
            (s.work_pos[0] - 20.).abs() < 1e-3
        });
        assert!((conn(&a).snapshot.work_pos[1] - -12.).abs() < 1e-3);
        assert!((conn(&a).snapshot.machine_pos[0] - 235.).abs() < 1e-3);
        // Set Y to −5 where the tool is; a field that is no number sets nothing.
        let _ = a.update(Message::Machine(Action::WorkText(1, "abc".into())));
        let _ = a.update(Message::Machine(Action::WorkSet(1)));
        tick_for(&mut a, 20);
        assert!((conn(&a).snapshot.work_pos[1] - -12.).abs() < 1e-3);
        let _ = a.update(Message::Machine(Action::WorkText(1, " -5 ".into())));
        let _ = a.update(Message::Machine(Action::WorkSet(1)));
        tick_until(&mut a, 2.0, "Y set", |s| (s.work_pos[1] - -5.).abs() < 1e-3);
        let _ = a.view();
        // Go to X0 Y0: X and Y move, Z stays.
        let z = conn(&a).snapshot.machine_pos[2];
        assert_eq!(conn(&a).to_work_xy(), Ok(()));
        let _ = a.update(Message::Machine(Action::ToWorkXY));
        tick_until(&mut a, 5.0, "at X0 Y0", |s| {
            s.state == Some(State::Idle) && s.work_pos[..2].iter().all(|v| v.abs() < 1e-3)
        });
        assert!((conn(&a).snapshot.machine_pos[2] - z).abs() < 1e-6);
        // In E-stop none of them does anything.
        let _ = a.update(Message::MachineEStop);
        tick_until(&mut a, 2.0, "E-stop", |s| s.state == Some(State::EStop));
        let _ = a.update(Message::Machine(Action::WorkText(0, "7".into())));
        let _ = a.update(Message::Machine(Action::WorkSet(0)));
        let _ = a.update(Message::Machine(Action::WorkHalf(1)));
        tick_for(&mut a, 50);
        assert!(conn(&a).snapshot.work_pos[0].abs() < 1e-3);
        assert!(conn(&a).to_work_xy().is_err());
    }

    /// Picking another setup in the Run strip while a probe is under way
    /// is refused, in the handler and on the buttons (`setup_locked`): the
    /// probe sets the work zero of the setup it started with, on that
    /// setup's panel (#165).
    #[test]
    fn picking_another_setup_in_the_run_strip_waits_for_the_probe() {
        use crate::control::Action;
        use crate::run::Action as R;
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app_on(dir.path(), &probe_profile());
        let m = a.doc.machines.last_mut().unwrap();
        m.p.insert("probe_tip".into(), "4".into());
        m.p.insert("probe_feed".into(), "1000".into());
        for s in &mut a.doc.setups {
            s.machine = m.clone();
        }
        a.doc.setups[0]
            .p
            .insert("origin".into(), "Stock top front-left".into());
        material(&mut a, "Aluminium");
        // A second setup on the same machine: a copy of the first.
        let _ = a.update(Message::DuplicateSetup);
        assert_eq!(a.doc.setups.len(), 2);
        a.page = Page::Control;
        zeroed(&mut a);
        check_run(&mut a, 0);
        assert!(!conn(&a).setup_locked());
        let (lo, _) = conn(&a).block.unwrap();
        mdi(
            &mut a,
            &format!("G53 G0 X{} Y{} Z-30", lo[0] + 6., lo[1] + 6.),
        );
        let _ = a.update(Message::Machine(Action::ProbeEdges));
        assert!(conn(&a).edging() && conn(&a).setup_locked());
        let _ = a.view();
        // The second setup, picked while it probes: nothing happens.
        let _ = a.update(Message::Run(R::Setup(1)));
        assert_eq!(run_of(&a).setup, 0);
        assert!(conn(&a).edging());
        let (ok, message) = probed(&mut a);
        assert!(ok, "{message}");
        // The work zero set is the first setup's, on its panel.
        assert_eq!(run_of(&a).setup, 0);
        assert!(run_of(&a).touched);
        tick_until(&mut a, 5.0, "work zero", |s| s.work_offset[2] < -39.);
        work_offset(&a, [lo[0] + 2., lo[1] + 2., -40.]);
        // Ended, the second setup can be picked, and starts untouched.
        assert!(!conn(&a).setup_locked());
        let _ = a.update(Message::Run(R::Setup(1)));
        assert_eq!(run_of(&a).setup, 1);
        assert!(!run_of(&a).touched);
        let _ = a.view();
    }

    /// Stock bottom back on the simulated UC100: the Control page offers
    /// Probe edge, which sets the work zero the program has, the stock's
    /// height below the block's top under the middle of its back edge (Y a
    /// tip radius inside the block's, the simulator's probe being a point);
    /// by hand, Move there and Set X0 Y0 Z… give it too.
    #[test]
    fn probe_edge_and_the_touch_off_set_a_zero_on_the_stock_bottom() {
        use crate::control::Action;
        let dir = tempfile::tempdir().unwrap();
        let mut a = edges_app(dir.path(), Some("Stock bottom back"));
        let (lo, hi) = conn(&a).block.unwrap();
        let height = run_of(&a).checked().unwrap()[0].stock.dimensions()[2] as f64;
        let zero = [(lo[0] + hi[0]) / 2., hi[1], hi[2] - height];
        let sim = conn(&a).sim_zero().unwrap();
        assert!(
            (0..3).all(|k| (sim[k] - zero[k]).abs() < 1e-3),
            "{sim:?} {zero:?}"
        );
        let (refs, _) = conn(&a).edges_setup().unwrap();
        assert_eq!(crate::control::probe_name(refs), "Probe edge");
        // Over the top's spot, 6 mm in from the middle of the back edge.
        mdi(&mut a, &format!("G53 G0 X{} Y{} Z-30", zero[0], hi[1] - 6.));
        let _ = a.update(Message::Machine(Action::ProbeEdges));
        let (ok, message) = probed(&mut a);
        assert!(ok, "{message}");
        assert!(
            message.contains("on the stock bottom under the middle of the back edge"),
            "{message}"
        );
        tick_until(&mut a, 5.0, "backing off", |s| {
            s.state == Some(uc100::controller::State::Idle) && s.work_offset[2] < -40.
        });
        work_offset(&a, [zero[0], zero[1] - 2., zero[2]]);
        // By hand: onto the top over the zero, then the program's position there.
        let steps = crate::ui::work_zero_steps(&a, conn(&a)).unwrap().join("\n");
        let set = format!("4. Set X0 Y0 Z{}", crate::post::exact_mm(height as f32));
        assert!(steps.contains(&set), "{steps}");
        let _ = a.update(Message::Machine(Action::ToSimZero));
        tick_until(&mut a, 10.0, "the move onto the block", |s| {
            s.state == Some(uc100::controller::State::Idle)
                && (s.machine_pos[2] - hi[2]).abs() < 0.06
                && (s.machine_pos[1] - hi[1]).abs() < 0.06
        });
        // The probe set Y a tip radius inside; the touch-off puts it on the block's side.
        let _ = a.update(Message::Machine(Action::TouchOff));
        tick_until(&mut a, 2.0, "the touch-off", |s| {
            s.work_pos[1].abs() < 1e-3 && (s.work_pos[2] - height).abs() < 1e-3
        });
        work_offset(&a, zero);
        let _ = a.view();
    }

    /// After an edit, until the setup is checked again, Move there and the
    /// touch-off button wait as Probe Z does: both act on the checked setup
    /// the steps are from, which the edit may have changed (#39).
    #[test]
    fn the_touch_off_and_move_there_wait_for_the_check_as_probe_z_does() {
        use crate::control::Action;
        use crate::run::Action as R;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        // A plate for Probe Z and a tip for Probe edge, set before connecting.
        let mut a = run_app_on(dir.path(), &probe_profile());
        plate(&mut a, "1000");
        let m = a.doc.machines.last_mut().unwrap();
        m.p.insert("probe_tip".into(), "4".into());
        for s in &mut a.doc.setups {
            s.machine = m.clone();
        }
        a.doc.setups[0]
            .p
            .insert("origin".into(), "Stock bottom back".into());
        material(&mut a, "Aluminium");
        zeroed(&mut a);
        check_run(&mut a, 0);
        let (lo, hi) = conn(&a).block.unwrap();
        let height = run_of(&a).checked().unwrap()[0].stock.dimensions()[2] as f64;
        let zero = [(lo[0] + hi[0]) / 2., hi[1], hi[2] - height];
        let set = format!("4. Set X0 Y0 Z{}", crate::post::exact_mm(height as f32));
        let steps = |a: &App| {
            crate::ui::work_zero_steps(a, conn(a))
                .expect("the guide")
                .join("\n")
        };
        assert!(steps(&a).contains(&set), "{}", steps(&a));
        assert_eq!(conn(&a).probe_z_blocked(), None);
        assert_eq!(edges_blocked(&a), "");
        // An edit: the setup is out of date until it is checked again. The
        // edits never pause here, so the Control page does not check it
        // again on its own (LIO-150) under the presses.
        let _ = a.update(Message::Title("Edited".into()));
        a.last_edit = std::time::Instant::now() + std::time::Duration::from_secs(3600);
        assert!(run_of(&a).stale);
        let why = conn(&a).run_program().expect_err("out of date");
        assert!(why.contains("check it again"), "{why}");
        assert_eq!(conn(&a).probe_z_blocked().as_deref(), Some(why.as_str()));
        assert_eq!(edges_blocked(&a), why);
        // The steps still end with the old button; it and Move there wait.
        assert!(steps(&a).contains(&set), "{}", steps(&a));
        let _ = a.view();
        let at = conn(&a).snapshot.machine_pos;
        let _ = a.update(Message::Machine(Action::ToSimZero));
        // A line sent after the press is taken once the press has been.
        mdi(&mut a, "G54");
        let now = conn(&a).snapshot.machine_pos;
        assert!(
            (0..3).all(|k| (now[k] - at[k]).abs() < 1e-6),
            "Move there moved the tool from {at:?} to {now:?} while out of date"
        );
        // Away from the work zero, the touch-off would set the offset here.
        mdi(&mut a, "G53 G0 X100 Y100 Z-10");
        let _ = a.update(Message::Machine(Action::TouchOff));
        mdi(&mut a, "G54");
        work_offset(&a, [195., 140., -20.]);
        assert!(!run_of(&a).touched);
        // Checked again, as Check again does, both work.
        let _ = a.update(Message::Run(R::Check));
        let epoch = run_of(&a).epoch;
        let programs = crate::run::programs(&a.doc, a.part_mesh().unwrap(), 0);
        let _ = a.update(Message::RunChecked(epoch, std::sync::Arc::new(programs)));
        assert!(!run_of(&a).stale && conn(&a).run_program().is_ok());
        assert_eq!(conn(&a).probe_z_blocked(), None);
        assert!(steps(&a).contains(&set), "{}", steps(&a));
        let _ = a.update(Message::Machine(Action::ToSimZero));
        tick_until(&mut a, 10.0, "the move onto the block", |s| {
            s.state == Some(State::Idle)
                && (s.machine_pos[2] - hi[2]).abs() < 0.06
                && (s.machine_pos[1] - hi[1]).abs() < 0.06
        });
        let _ = a.update(Message::Machine(Action::TouchOff));
        tick_until(&mut a, 2.0, "the touch-off", |s| {
            s.work_pos[1].abs() < 1e-3 && (s.work_pos[2] - height).abs() < 1e-3
        });
        work_offset(&a, zero);
        assert!(run_of(&a).touched);
        let _ = a.view();
    }

    #[test]
    fn probing_the_edges_needs_a_stock_zero_a_tip_and_its_work_system() {
        use crate::control::Action;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app_on(dir.path(), &probe_profile());
        zeroed(&mut a);
        let why = edges_blocked(&a);
        assert!(why.contains("picked in the Run panel"), "{why}");
        // Model origin is found from the front-left corner of the stock top.
        a.doc.setups[0]
            .p
            .insert("origin".into(), "Model origin".into());
        check_run(&mut a, 0);
        let refs = conn(&a).edges_setup().map(|(refs, _)| refs);
        assert_eq!(refs, Ok([crate::post::At::Min; 2]));
        // Without a material, and on wood, the plate's lead has no stock to
        // go on: X and Y are zeroed by hand, and every feature says so too.
        let why = edges_blocked(&a);
        assert!(why.starts_with("The project has no material"), "{why}");
        material(&mut a, "Wood and composite");
        let _ = a.update(Message::Tick);
        let why = edges_blocked(&a);
        assert!(
            why.ends_with("does not conduct, so the probe cannot touch the stock: probe Z on the touch plate and zero X and Y by hand."),
            "{why}"
        );
        for f in [crate::control::Feature::Top, crate::control::Feature::Bore] {
            assert_eq!(conn(&a).feature_blocked(f).as_deref(), Some(why.as_str()));
        }
        let _ = a.view();
        material(&mut a, "Aluminium");
        let _ = a.update(Message::Tick);
        let why = edges_blocked(&a);
        assert!(why.contains("Probe tip diameter is not set"), "{why}");
        let _ = a.view();
        a.doc.setups[0]
            .p
            .insert("origin".into(), "Stock top center".into());
        check_run(&mut a, 0);
        let why = edges_blocked(&a);
        assert!(why.contains("Probe tip diameter is not set"), "{why}");
        // A tip saved while connected; saving changes the project.
        a.machine_draft = a.doc.machines[0].clone();
        a.machine_draft.p.insert("probe_tip".into(), "4".into());
        a.machine_draft.p.insert("probe_feed".into(), "1000".into());
        let _ = a.update(Message::SaveMachine);
        let why = edges_blocked(&a);
        assert!(why.contains("check it again"), "{why}");
        check_run(&mut a, 0);
        assert_eq!(edges_blocked(&a), "");
        mdi(&mut a, "G55");
        let why = edges_blocked(&a);
        assert!(
            why.contains("This setup uses G54, but the active work system is G55"),
            "{why}"
        );
        mdi(&mut a, "G54");
        // Started beside the block: the top is missed.
        mdi(&mut a, "G53 G0 X20 Y20 Z-30");
        let _ = a.update(Message::Machine(Action::ProbeEdges));
        let (ok, message) = probed(&mut a);
        assert!(
            !ok && message.contains("No contact down to machine Z -50.000"),
            "{message}"
        );
        // 20 mm right of the middle: going down beside the -X side lands on
        // the block.
        mdi(&mut a, "G53 G0 X215 Y140 Z-30");
        let _ = a.update(Message::Machine(Action::ProbeEdges));
        let (ok, message) = probed(&mut a);
        assert!(
            !ok && message.contains("touched something going down beside the -X side"),
            "{message}"
        );
        // E-stop stops it.
        tick_until(&mut a, 5.0, "Idle", |s| s.state == Some(State::Idle));
        mdi(&mut a, "G53 G0 X195 Y140 Z-30");
        let _ = a.update(Message::Machine(Action::ProbeEdges));
        tick_until(&mut a, 5.0, "a side", |s| {
            s.machine_pos[0] < 150. && s.state == Some(State::Run)
        });
        let _ = a.update(Message::MachineEStop);
        let (ok, message) = probed(&mut a);
        assert!(!ok && message.contains("E-stop"), "{message}");
        assert!(!a.locked());
        // Nothing was set.
        work_offset(&a, [195., 140., -20.]);
    }

    /// The Control page's 3D view shows the stock a run cuts, where the
    /// machine's tool has gone and not ahead of it; MANUFACTURE keeps the
    /// planned view, and picking the start again drops the carve (LIO-181).
    #[test]
    fn a_run_cuts_the_stock_where_the_machines_tool_goes() {
        use crate::run::Action as R;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        zeroed(&mut a);
        check_run_pictured(&mut a, dir.path(), 0);
        a.page = Page::Control;
        // Nothing is shown before the tool moves: the view keeps the part.
        assert!(a.live_stock().is_none());
        let start = (run_of(&a).before.iter())
            .find(|b| b.op == 0)
            .expect("the stock before the facing")
            .stock
            .clone();
        let top = start.bounds.max[2];
        assert!(
            start.heights.iter().all(|h| (h - top).abs() < 1e-3),
            "uncut"
        );
        let _ = a.update(Message::Run(R::CycleStart));
        tick_until(&mut a, 2.0, "the job", |s| s.state == Some(State::Run));
        run_until(&mut a, 20.0, "a cut into the stock", |r| {
            r.carving
                .as_ref()
                .is_some_and(|c| c.stock.heights.iter().any(|h| *h < top - 0.05))
        });
        // The passes go along X, stepping over in +Y: nothing past the tool
        // in Y is cut while it faces, though the driver's line runs ahead.
        let mut seen = 0;
        let end = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while run_of(&a).done.first() != Some(&true) {
            assert!(run_of(&a).running.is_some(), "{:?}", run_of(&a).outcome);
            assert!(
                std::time::Instant::now() < end,
                "the facing tool run did not end"
            );
            let _ = a.update(Message::Tick);
            let (Some((tip, radius, _)), Some((g, _))) = (a.live_tool(), a.live_stock()) else {
                continue;
            };
            if tip[2] > top {
                continue;
            }
            seen += 1;
            for y in 0..g.ny {
                let cy = g.bounds.min[1] + (y as f32 + 0.5) * g.dy;
                if cy > tip[1] + radius + 2. * g.dy {
                    let row = &g.heights[y * g.nx..(y + 1) * g.nx];
                    assert!(
                        row.iter().all(|h| (h - top).abs() < 1e-3),
                        "Y {cy} cut, tool at {tip:?}"
                    );
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(seen > 3, "{seen}");
        // MANUFACTURE keeps its planned view.
        a.page = Page::Work;
        assert!(a.live_stock().is_none());
        a.page = Page::Control;
        let _ = a.view();
        // The facing tool run done, its passes have cut the whole top, every
        // pass however late the ticks came.
        tick_until(&mut a, 2.0, "Idle", |s| s.state == Some(State::Idle));
        let faced = a.live_stock().unwrap().0;
        assert!(
            faced.heights.iter().all(|h| *h < top - 0.05),
            "{:?}",
            faced.heights
        );
        // Jogging between tool runs cuts nothing; the carve is kept.
        let kept = faced.heights.clone();
        mdi(&mut a, "G0 X5 Y5 Z-5");
        assert_eq!(a.live_stock().unwrap().0.heights, kept);
        let _ = a.update(Message::Run(R::From(0, 0)));
        assert!(run_of(&a).carving.is_none() && a.live_stock().is_none());
    }

    #[test]
    fn the_3d_view_shows_the_machine_and_planning_waits_for_it() {
        use crate::run::Action as R;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        zeroed(&mut a);
        check_run(&mut a, 0);
        // MANUFACTURE shows the facing operation's simulation.
        a.selected = Some(0);
        let sim = crate::simulation::prepare(&a.doc, a.part_mesh().unwrap(), 0, 0).unwrap();
        a.simulation = Some(sim);
        assert!(a.live_tool().is_none());
        // Cycle Start waits for planning work under way in the background.
        a.post_busy = true;
        let _ = a.update(Message::Run(R::CycleStart));
        assert!(run_of(&a).running.is_none());
        a.post_busy = false;
        let _ = a.update(Message::Run(R::CycleStart));
        tick_until(&mut a, 2.0, "the job", |s| s.state == Some(State::Run));
        run_until(&mut a, 5.0, "a cutting move", |r| {
            r.running.is_some() && r.done.iter().all(|d| !d)
        });
        let datum = run_of(&a).checked().unwrap()[0].datum;
        let first = a.live_tool().expect("the live tool").0;
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let _ = a.update(Message::Tick);
            let s = conn(&a).snapshot.clone();
            let (tip, radius, stickout) = a.live_tool().expect("the live tool");
            // The tip in plan coordinates: machine - work offset + datum.
            for k in 0..3 {
                let expected = s.machine_pos[k] - s.work_offset[k] + datum[k] as f64;
                assert!((tip[k] as f64 - expected).abs() < 1e-3, "{k}: {tip:?}");
            }
            assert_eq!((radius, stickout), (5., 65.));
            if tip != first {
                break;
            }
            assert!(std::time::Instant::now() < end, "the tool stays at {tip:?}");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // Planning is locked while it runs: no edit, preview or playback.
        assert!(a.locked());
        let title = a.doc.title.clone();
        for m in [
            Message::Play,
            Message::Generate,
            Message::Title("Changed".into()),
            Message::Plan,
        ] {
            let _ = a.update(m);
            assert_eq!(a.status, crate::LOCKED);
        }
        assert!(!a.playing && a.progress == 0. && a.doc.title == title);
        // The Manufacture workspace does not open while it runs; the Machine
        // workspace shows the job with the machine's tool.
        for m in [
            Message::Page(Page::Work),
            Message::Page(Page::Tools),
            Message::SetupEditor,
            Message::SelectSetup(0),
            Message::SelectOp(1),
        ] {
            let _ = a.update(m);
            assert_eq!((a.page, a.status.as_str()), (Page::Control, crate::LOCKED));
        }
        assert_eq!(a.selected, Some(0));
        let _ = a.view();
        let _ = a.update(Message::Run(R::Stop));
        run_until(&mut a, 5.0, "the stop", |r| r.running.is_none());
        tick_until(&mut a, 2.0, "Idle", |s| s.state == Some(State::Idle));
        assert!(a.live_tool().is_none() && !a.locked() && a.status.is_empty());
        let _ = a.update(Message::Play);
        assert!(a.playing);
    }

    #[test]
    fn screen_100_shows_the_steps_of_a_manual_setup_put_first() {
        let mut a = imported_app();
        a.smoke_manual_setup();
        let _ = a.update(Message::Page(Page::Control));
        crate::steps::tests::build(&mut a);
        a.smoke_manual_check();
        let texts = page_texts(&a);
        for want in ["Big mill", "Set up", "Face", "Hold"] {
            assert!(texts.iter().any(|t| t == want), "{want} in {texts:?}");
        }
    }

    #[test]
    fn the_steps_page_works_while_the_cnc_jogs() {
        use crate::control::Action;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = uc100_app(dir.path());
        idle(&mut a);
        let _ = a.update(Message::Machine(Action::JogPress(0, false)));
        tick_until(&mut a, 2.0, "the jog", |s| s.state == Some(State::Jog));
        assert!(a.locked());
        // The manual setup of the guide's job, its steps built.
        let manual = crate::steps::tests::app("Yes");
        a.doc = manual.doc.clone();
        a.imported_mesh = manual.imported_mesh.clone();
        a.guide = manual.guide.clone();
        a.status.clear();
        let _ = a.update(Message::Page(Page::Control));
        assert!(a.steps_page() && a.guide_at().is_some());
        let _ = a.update(Message::Guide(crate::GuideAction::Next));
        let _ = a.update(Message::Guide(crate::GuideAction::Section(2)));
        assert_ne!(a.status, crate::LOCKED);
        assert_eq!(
            a.guide_at().unwrap().1,
            a.guide_at().unwrap().0.guide.sections[2].steps.start
        );
        let texts = page_texts(&a);
        assert!(texts.iter().any(|t| t == "Tool"), "{texts:?}");
        // The way back to the CNC works too.
        let _ = a.update(Message::Guide(crate::GuideAction::Open(1)));
        assert!(!a.steps_page() && a.doc.active == a.doc.setups[1].id);
        let _ = a.update(Message::Machine(Action::JogRelease));
        tick_until(&mut a, 3.0, "Idle", |s| s.state == Some(State::Idle));
    }

    #[test]
    fn tab_moves_the_focus_while_the_machine_moves() {
        use crate::run::Action as R;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        zeroed(&mut a);
        check_run(&mut a, 0);
        let _ = a.update(Message::Run(R::CycleStart));
        tick_until(&mut a, 2.0, "the job", |s| s.state == Some(State::Run));
        assert!(a.locked());
        // The Control page's fields stay editable, so Tab and Shift+Tab move
        // between them: the focus task is started and the status stays clear.
        a.status.clear();
        for back in [false, true] {
            let task = a.update(Message::TabKey(back));
            assert_eq!(task.units(), 1, "back: {back}");
            assert_ne!(a.status, crate::LOCKED, "back: {back}");
        }
        assert!(a.status.is_empty());
        let _ = a.update(Message::Run(R::Stop));
        run_until(&mut a, 5.0, "the stop", |r| r.running.is_none());
        tick_until(&mut a, 2.0, "Idle", |s| s.state == Some(State::Idle));
    }

    #[test]
    fn a_tool_run_holds_resumes_and_stops() {
        use crate::run::Action as R;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        zeroed(&mut a);
        check_run(&mut a, 0);
        let _ = a.update(Message::Run(R::CycleStart));
        tick_until(&mut a, 2.0, "the job", |s| s.state == Some(State::Run));
        let _ = a.update(Message::Run(R::Hold));
        tick_until(&mut a, 2.0, "Hold", |s| s.state == Some(State::Hold));
        let _ = a.view();
        let _ = a.update(Message::Run(R::Resume));
        tick_until(&mut a, 2.0, "Run", |s| s.state == Some(State::Run));
        // Space holds from any page and resumes only on the Control page.
        a.page = Page::Work;
        let _ = a.update(Message::HoldKey);
        tick_until(&mut a, 2.0, "Hold by Space", |s| {
            s.state == Some(State::Hold)
        });
        let _ = a.update(Message::HoldKey);
        tick_for(&mut a, 100);
        assert_eq!(conn(&a).snapshot.state, Some(State::Hold));
        a.page = Page::Control;
        let _ = a.update(Message::HoldKey);
        tick_until(&mut a, 2.0, "Run by Space", |s| s.state == Some(State::Run));
        let _ = a.update(Message::Run(R::Stop));
        run_until(&mut a, 5.0, "the stop", |r| r.running.is_none());
        let (ok, message) = run_of(&a).outcome.clone().unwrap();
        assert!(
            !ok && message.contains("stopped by the operator"),
            "{message}"
        );
        assert_eq!((run_of(&a).next, run_of(&a).done[0]), (0, false));
        tick_until(&mut a, 2.0, "Idle", |s| s.state == Some(State::Idle));
        // An E-stop ends the tool run and says why.
        let _ = a.update(Message::Run(R::CycleStart));
        tick_until(&mut a, 2.0, "the job", |s| s.state == Some(State::Run));
        let _ = a.update(Message::MachineEStop);
        run_until(&mut a, 2.0, "the E-stop", |r| r.running.is_none());
        let (ok, message) = run_of(&a).outcome.clone().unwrap();
        assert!(!ok && message.contains("E-stop pressed"), "{message}");
        let _ = a.view();
    }

    #[test]
    fn the_test_profiles_are_valid() {
        let dir = tempfile::tempdir().unwrap();
        for text in [LIMITED, SWITCHED, FAST, UNLIMITED] {
            let a = profile_app(dir.path(), text);
            let checked = crate::control::profile(&a.doc.machines[0]);
            assert!(checked.is_ok(), "{checked:?}");
        }
    }

    #[test]
    fn a_profile_with_no_soft_limits_gets_the_same_amber_warning_and_an_assumed_box() {
        use crate::machine_view::{Caption, MachineView};
        use crate::ui::limits_off_warning;
        let dir = tempfile::tempdir().unwrap();
        let config = |text: &str| {
            let a = profile_app(dir.path(), text);
            crate::control::profile(&a.doc.machines[0]).unwrap()
        };
        let view = |c: &uc100::config::MachineConfig, known: bool| {
            let (envelope, assumed) = MachineView::envelope(c);
            MachineView {
                pos: [0.; 3],
                known,
                envelope,
                assumed,
                block: None,
                zero: [0.; 3],
            }
        };
        // No soft limits: the warning of the unreferenced machine, with its
        // own sentence (add them to the profile), at any position.
        let none = config(UNLIMITED);
        assert!(!none.soft_limits_configured());
        let (heading, lines) = limits_off_warning(&[0, 1, 2], &[], false).unwrap();
        assert_eq!(heading, "Soft limits are OFF");
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines[0].contains("This profile sets no soft limits"),
            "{lines:?}"
        );
        assert!(lines[1].contains("soft_min and soft_max"), "{lines:?}");
        // Configured but not referenced: the same heading, other sentences.
        let limited = config(LIMITED);
        assert!(limited.soft_limits_configured());
        let (h, lines) = limits_off_warning(&[], &[0, 1, 2], false).unwrap();
        assert_eq!(h, heading);
        assert!(
            lines[0].starts_with("X, Y and Z have no machine position"),
            "{lines:?}"
        );
        assert_eq!(lines[1], "Set machine zero below to turn them on.");
        let (_, lines) = limits_off_warning(&[], &[2], true).unwrap();
        assert!(
            lines[0].starts_with("Z has no machine position"),
            "{lines:?}"
        );
        assert_eq!(lines[1], "Home below to turn them on.");
        // Configured and referenced: no warning.
        assert!(limits_off_warning(&[], &[], true).is_none());
        // Soft limits on X and Y only (#164): the driver counts the profile
        // as limited, the page does not, since Z runs into its ends, and the
        // box names the axis without them and says to add them.
        let partial = LIMITED.replace("soft_min = -55.0\nsoft_max = 0.0\n", "");
        let some = config(&partial);
        assert!(
            some.soft_limits_configured(),
            "the driver's any-axis meaning"
        );
        assert!(MachineView::envelope(&some).1, "the assumed travel");
        let other = tempfile::tempdir().unwrap();
        let mut a = profile_app(other.path(), &partial);
        idle(&mut a);
        assert!(!conn(&a).soft_limits_configured());
        assert_eq!(conn(&a).unlimited_axes(), vec![2]);
        let (h, lines) = limits_off_warning(&conn(&a).unlimited_axes(), &[], false).unwrap();
        assert_eq!(h, heading);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines[0].starts_with(
                "Z has no soft limits in this profile, so nothing stops it at the end of its travel."
            ),
            "{lines:?}"
        );
        assert_eq!(
            lines[1],
            "Add soft_min and soft_max for Z to the profile to turn them on."
        );
        // With X and Y waiting for a position too, both are said; Z is not
        // waiting for one.
        let (_, lines) = limits_off_warning(&[2], &[0, 1, 2], false).unwrap();
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert!(
            lines[2].starts_with("X and Y have no machine position"),
            "{lines:?}"
        );
        assert_eq!(lines[3], "Set machine zero below to turn them on.");
        let (_, lines) = limits_off_warning(&[1, 2], &[], false).unwrap();
        assert!(
            lines[0].starts_with(
                "Y and Z have no soft limits in this profile, so nothing stops them at the end of their travel."
            ),
            "{lines:?}"
        );
        assert_eq!(
            lines[1],
            "Add soft_min and soft_max for Y and Z to the profile to turn them on."
        );
        // The page's green line needs all three: machine zero on X and Y
        // alone leaves the amber box.
        let _ = a.update(Message::Machine(crate::control::Action::MachineZero(None)));
        tick_until(&mut a, 3.0, "X and Y set", |s| {
            s.referenced[0] && s.referenced[1]
        });
        assert!(conn(&a).unset_limited_axes().is_empty());
        assert!(!conn(&a).soft_limits_configured());
        a.page = Page::Control;
        let _ = a.view();
        // The drawing: the profile without limits gets the assumed box, said
        // so whether or not the position is set; never "where the tool tip
        // can travel".
        for known in [false, true] {
            let v = view(&none, known);
            assert!(v.assumed);
            let lines = v.captions();
            assert!(
                lines.contains(&(
                    Caption::Assumed,
                    "Dashed box: assumed travel, travel not configured in the profile"
                )),
                "{lines:?}"
            );
            assert!(
                lines.iter().all(|(_, t)| !t.contains("can travel")),
                "{lines:?}"
            );
        }
        // The real soft-limit box keeps its caption once the position is set.
        let real = view(&limited, true);
        assert!(!real.assumed);
        assert_eq!(real.envelope, [[0., 0., -55.], [390., 280., 0.]]);
        assert!(
            real.captions()
                .contains(&(Caption::Travel, "Box: where the tool tip can travel"))
        );
        assert!(
            view(&limited, false)
                .captions()
                .iter()
                .all(|(k, _)| *k != Caption::Assumed)
        );
    }

    #[test]
    fn the_status_line_shows_the_machine_state_once_connected() {
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = uc100_app(dir.path());
        let _ = a.update(Message::Connect);
        assert!(a.status.starts_with("Connecting to JY5800"), "{}", a.status);
        tick_until(&mut a, 3.0, "E-stop after connecting", |s| {
            s.state == Some(State::EStop)
        });
        assert_eq!(a.status, "Connected to JY5800: E-STOP");
        // It follows the state: Reset reads Idle (LIO-150).
        let _ = a.update(Message::MachineReset);
        tick_until(&mut a, 3.0, "Idle", |s| s.state == Some(State::Idle));
        assert_eq!(a.status, "Connected to JY5800: Idle");
        // Later states do not overwrite other messages.
        a.status = "Another message: Idle".into();
        let _ = a.update(Message::MachineEStop);
        tick_until(&mut a, 3.0, "E-stop", |s| s.state == Some(State::EStop));
        assert_eq!(a.status, "Another message: Idle");
        assert!(conn(&a).config.axis_enabled(2));
    }

    /// The machine position of a connected test app.
    fn at(a: &App) -> [f64; 6] {
        conn(a).snapshot.machine_pos
    }

    #[test]
    fn machine_zero_turns_the_soft_limits_on() {
        use crate::control::Action;
        let dir = tempfile::tempdir().unwrap();
        let mut a = profile_app(dir.path(), LIMITED);
        idle(&mut a);
        assert!(conn(&a).soft_limits_configured());
        assert_eq!(conn(&a).unset_limited_axes(), vec![0, 1, 2]);
        // Until then nothing stops a move past the travel.
        mdi(&mut a, "G0 X20 Y10 Z-5");
        assert!((at(&a)[0] - 20.).abs() < 1e-3 && (at(&a)[2] + 5.).abs() < 1e-3);
        let _ = a.update(Message::Machine(Action::MachineZero(None)));
        tick_until(&mut a, 3.0, "X, Y and Z set to 0", |s| {
            s.referenced[..3].iter().all(|r| *r)
                && s.machine_pos[..3].iter().all(|v| v.abs() < 1e-3)
        });
        assert!(conn(&a).unset_limited_axes().is_empty());
        // The soft limits now refuse a move below X 0.
        mdi(&mut a, "G0 X-5");
        assert!(at(&a)[0].abs() < 1e-3);
        assert!(conn(&a).snapshot.log.iter().any(|l| l.starts_with("MDI:")));
        // One axis at a time.
        mdi(&mut a, "G0 X7 Y3");
        let _ = a.update(Message::Machine(Action::MachineZero(Some(0))));
        tick_until(&mut a, 3.0, "X set to 0", |s| s.machine_pos[0].abs() < 1e-3);
        assert!((at(&a)[1] - 3.).abs() < 1e-3);
        // Nothing is declared outside Idle.
        let _ = a.update(Message::MachineEStop);
        tick_until(&mut a, 2.0, "E-stop", |s| {
            s.state == Some(uc100::controller::State::EStop)
        });
        let _ = a.update(Message::Machine(Action::MachineZero(Some(1))));
        let _ = a.update(Message::Tick);
        assert!((at(&a)[1] - 3.).abs() < 1e-3);
    }

    #[test]
    fn keep_the_last_position_declares_where_the_machine_was_left() {
        use crate::control::Action;
        let dir = tempfile::tempdir().unwrap();
        let mut a = profile_app(dir.path(), LIMITED);
        idle(&mut a);
        mdi(&mut a, "G0 X12 Y7");
        let _ = a.update(Message::Disconnect);
        idle(&mut a);
        assert!((at(&a)[0] - 12.).abs() < 1e-3 && (at(&a)[1] - 7.).abs() < 1e-3);
        assert_eq!(conn(&a).unset_limited_axes(), vec![0, 1, 2]);
        let _ = a.update(Message::Machine(Action::KeepPosition));
        tick_until(&mut a, 3.0, "X, Y and Z set", |s| {
            s.referenced[..3].iter().all(|r| *r)
        });
        assert!((at(&a)[0] - 12.).abs() < 1e-3 && (at(&a)[1] - 7.).abs() < 1e-3);
        assert!(conn(&a).unset_limited_axes().is_empty());
    }

    #[test]
    fn home_needs_home_switches_on_every_axis() {
        use crate::control::Action;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = profile_app(dir.path(), LIMITED);
        idle(&mut a);
        assert!(!conn(&a).home_switches());
        let _ = a.update(Message::Machine(Action::Home));
        std::thread::sleep(std::time::Duration::from_millis(100));
        let _ = a.update(Message::Tick);
        let s = &conn(&a).snapshot;
        assert_eq!(s.state, Some(State::Idle));
        assert!(
            !s.log.iter().any(|l| l.contains("no home switch")),
            "{:?}",
            s.log
        );
        let _ = a.update(Message::Disconnect);
        let dir = tempfile::tempdir().unwrap();
        let mut a = profile_app(dir.path(), SWITCHED);
        idle(&mut a);
        assert!(conn(&a).home_switches());
        mdi(&mut a, "G0 X4 Y3 Z-2");
        let _ = a.update(Message::Machine(Action::Home));
        tick_until(&mut a, 3.0, "homing", |s| s.state == Some(State::Homing));
        tick_until(&mut a, 10.0, "homed", |s| {
            s.state == Some(State::Idle) && s.referenced[..3].iter().all(|r| *r)
        });
        assert!(at(&a)[..3].iter().all(|v| v.abs() < 1e-2), "{:?}", at(&a));
    }

    /// Ticks for `ms` milliseconds.
    fn tick_for(a: &mut App, ms: u64) {
        let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
        while std::time::Instant::now() < end {
            let _ = a.update(Message::Tick);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn a_held_jog_keeps_moving_and_stops_on_release() {
        use crate::control::Action;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = uc100_app(dir.path());
        idle(&mut a);
        let _ = a.update(Message::Machine(Action::JogPress(0, false)));
        assert_eq!(conn(&a).held(), Some((0, false)));
        // Past the driver's 400 ms timeout: the tick keeps the jog going.
        tick_for(&mut a, 700);
        assert_eq!(conn(&a).snapshot.state, Some(State::Jog));
        assert!(at(&a)[0] > 5., "{:?}", at(&a));
        let _ = a.update(Message::Machine(Action::JogRelease));
        assert_eq!(conn(&a).held(), None);
        tick_until(&mut a, 3.0, "Idle after the release", |s| {
            s.state == Some(State::Idle)
        });
        let x = at(&a)[0];
        tick_for(&mut a, 100);
        assert_eq!(at(&a)[0], x);
        // Nothing jogs in E-stop, and an axis the profile lacks is never sent.
        let _ = a.update(Message::MachineEStop);
        tick_until(&mut a, 2.0, "E-stop", |s| s.state == Some(State::EStop));
        let _ = a.update(Message::Machine(Action::JogPress(0, false)));
        let _ = a.update(Message::Machine(Action::JogPress(9, false)));
        assert_eq!(conn(&a).held(), None);
    }

    #[test]
    fn a_jog_step_moves_one_step() {
        use crate::control::Action;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = uc100_app(dir.path());
        idle(&mut a);
        let _ = a.update(Message::Machine(Action::JogStep(Some(1.))));
        let _ = a.update(Message::Machine(Action::JogPress(1, true)));
        assert_eq!(conn(&a).held(), None);
        tick_until(&mut a, 3.0, "Y at -1", |s| {
            s.state == Some(State::Idle) && (s.machine_pos[1] + 1.).abs() < 1e-3
        });
        // A press for an axis the profile lacks sends nothing.
        let _ = a.update(Message::Machine(Action::JogPress(9, true)));
        tick_for(&mut a, 100);
        assert_eq!(conn(&a).snapshot.state, Some(State::Idle));
    }

    #[test]
    fn a_jog_into_a_soft_limit_ends_at_the_limit() {
        use crate::control::Action;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = profile_app(dir.path(), LIMITED);
        idle(&mut a);
        mdi(&mut a, "G0 X3");
        let _ = a.update(Message::Machine(Action::MachineZero(None)));
        tick_until(&mut a, 3.0, "X, Y and Z set", |s| {
            s.referenced[..3].iter().all(|r| *r)
        });
        mdi(&mut a, "G0 X3");
        let _ = a.update(Message::Machine(Action::JogPress(0, true)));
        let end = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while conn(&a).held().is_some() {
            assert!(std::time::Instant::now() < end, "the jog did not end");
            tick_for(&mut a, 20);
        }
        tick_until(&mut a, 3.0, "Idle", |s| s.state == Some(State::Idle));
        assert!(at(&a)[0].abs() < 0.05, "{:?}", at(&a));
        // A step past the limit is refused before it moves.
        let x = at(&a)[0];
        let _ = a.update(Message::Machine(Action::JogStep(Some(10.))));
        let _ = a.update(Message::Machine(Action::JogPress(0, true)));
        tick_for(&mut a, 100);
        assert_eq!(at(&a)[0], x);
        assert!(
            conn(&a)
                .snapshot
                .log
                .iter()
                .any(|l| l.starts_with("Jog refused"))
        );
    }

    #[test]
    fn jog_keys_only_on_the_control_page_and_not_while_typing() {
        use iced::keyboard::key::Named;
        use uc100::controller::State;
        let keys = [
            (Named::ArrowLeft, (0, true)),
            (Named::ArrowRight, (0, false)),
            (Named::ArrowUp, (1, false)),
            (Named::ArrowDown, (1, true)),
            (Named::PageUp, (2, false)),
            (Named::PageDown, (2, true)),
        ];
        for (key, jog) in keys {
            assert_eq!(crate::jog_key(key), Some(jog));
        }
        assert_eq!(crate::jog_key(Named::Enter), None);
        let dir = tempfile::tempdir().unwrap();
        let mut a = uc100_app(dir.path());
        idle(&mut a);
        let held = |a: &App| conn(a).held();
        let _ = a.update(Message::JogKey(0, false, false));
        assert_eq!(a.jog_key, None, "not on the Control page");
        a.page = Page::Control;
        let _ = a.update(Message::JogKeyChecked(0, false, false));
        assert_eq!(held(&a), None, "no key is down");
        let _ = a.update(Message::JogKey(0, false, false));
        let _ = a.update(Message::JogKeyChecked(0, false, true));
        assert_eq!(held(&a), None, "a text field has the focus");
        let _ = a.update(Message::JogKeyUp);
        let _ = a.update(Message::JogKey(0, false, false));
        let _ = a.update(Message::JogKeyUp);
        let _ = a.update(Message::JogKeyChecked(0, false, false));
        assert_eq!(held(&a), None, "released before the answer");
        let _ = a.update(Message::JogKey(0, false, false));
        let _ = a.update(Message::JogKeyChecked(0, false, false));
        assert_eq!(held(&a), Some((0, false)));
        let _ = a.update(Message::JogKey(0, false, true));
        assert_eq!(held(&a), Some((0, false)));
        let _ = a.update(Message::JogKeyUp);
        assert_eq!((held(&a), a.jog_key), (None, None));
        tick_until(&mut a, 3.0, "Idle", |s| s.state == Some(State::Idle));
    }

    /// The Controller configuration view hides the jog controls and the run
    /// panel, so no key may jog or resume while it is open.
    #[test]
    fn keys_do_not_jog_or_resume_while_the_setup_view_hides_the_jog_controls() {
        use crate::run::Action as R;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        zeroed(&mut a);
        a.page = Page::Control;
        check_run(&mut a, 0);
        let _ = a.update(Message::Run(R::CycleStart));
        tick_until(&mut a, 2.0, "the job", |s| s.state == Some(State::Run));
        let _ = a.update(Message::HoldKey);
        tick_until(&mut a, 2.0, "Hold by Space", |s| {
            s.state == Some(State::Hold)
        });
        // Space still holds with the Setup view open, but does not resume.
        let _ = a.update(Message::ControlSetup(true));
        let _ = a.view();
        let _ = a.update(Message::HoldKey);
        tick_for(&mut a, 100);
        assert_eq!(conn(&a).snapshot.state, Some(State::Hold), "Setup is open");
        let _ = a.update(Message::ControlSetup(false));
        let _ = a.update(Message::HoldKey);
        tick_until(&mut a, 2.0, "Run by Space", |s| s.state == Some(State::Run));
        let _ = a.update(Message::HoldKey);
        tick_until(&mut a, 2.0, "Hold again", |s| s.state == Some(State::Hold));
        let _ = a.update(Message::ControlSetup(true));
        let _ = a.update(Message::HoldKey);
        tick_for(&mut a, 100);
        assert_eq!(conn(&a).snapshot.state, Some(State::Hold), "Setup is open");
        let _ = a.update(Message::ControlSetup(false));
        let _ = a.update(Message::Run(R::Stop));
        run_until(&mut a, 5.0, "the stop", |r| r.running.is_none());
        tick_until(&mut a, 2.0, "Idle", |s| s.state == Some(State::Idle));
        let held = |a: &App| conn(a).held();
        let _ = a.update(Message::ControlSetup(true));
        let _ = a.view();
        let _ = a.update(Message::JogKey(0, false, false));
        assert_eq!(a.jog_key, None, "Setup is open");
        let _ = a.update(Message::JogKeyChecked(0, false, false));
        assert_eq!(held(&a), None, "Setup is open");
        // Setup opened between the key going down and the focus answer.
        let _ = a.update(Message::ControlSetup(false));
        let _ = a.update(Message::JogKey(0, false, false));
        let _ = a.update(Message::ControlSetup(true));
        let _ = a.update(Message::JogKeyChecked(0, false, false));
        assert_eq!(held(&a), None, "Setup opened before the answer");
        let _ = a.update(Message::JogKeyUp);
        let _ = a.update(Message::ControlSetup(false));
        let _ = a.view();
        let _ = a.update(Message::JogKey(0, false, false));
        let _ = a.update(Message::JogKeyChecked(0, false, false));
        assert_eq!(held(&a), Some((0, false)), "the jog controls are back");
        let _ = a.update(Message::JogKeyUp);
        assert_eq!((held(&a), a.jog_key), (None, None));
        tick_until(&mut a, 3.0, "Idle", |s| s.state == Some(State::Idle));
    }

    #[test]
    fn tab_and_shift_tab_move_between_fields_even_from_inside_one() {
        use iced::keyboard::{Event, Key, Location, Modifiers, key};
        let press = |named, modifiers| {
            iced::Event::Keyboard(Event::KeyPressed {
                key: Key::Named(named),
                modified_key: Key::Named(named),
                physical_key: key::Physical::Code(key::Code::Tab),
                location: Location::Standard,
                modifiers,
                text: None,
                repeat: false,
            })
        };
        let window = iced::window::Id::unique();
        for status in [iced::event::Status::Ignored, iced::event::Status::Captured] {
            let tab = crate::on_event(press(key::Named::Tab, Modifiers::empty()), status, window);
            assert!(
                matches!(tab, Some(Message::TabKey(false))),
                "{status:?}: {tab:?}"
            );
            let back = crate::on_event(press(key::Named::Tab, Modifiers::SHIFT), status, window);
            assert!(
                matches!(back, Some(Message::TabKey(true))),
                "{status:?}: {back:?}"
            );
        }
        let enter = crate::on_event(
            press(key::Named::Enter, Modifiers::empty()),
            iced::event::Status::Ignored,
            window,
        );
        assert!(enter.is_none(), "{enter:?}");
    }

    /// A key press as the window sends it, for `on_event`.
    fn key_press(key: iced::keyboard::Key, modifiers: iced::keyboard::Modifiers) -> iced::Event {
        use iced::keyboard::{Event, Location, key};
        iced::Event::Keyboard(Event::KeyPressed {
            modified_key: key.clone(),
            key,
            physical_key: key::Physical::Code(key::Code::KeyA),
            location: Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })
    }

    #[test]
    fn escape_sends_the_estop_from_anywhere_even_while_typing() {
        use iced::keyboard::{Key, Modifiers, key::Named};
        let window = iced::window::Id::unique();
        // Captured is a text field with the focus; Ignored is nothing typing.
        for status in [iced::event::Status::Captured, iced::event::Status::Ignored] {
            let esc = crate::on_event(
                key_press(Key::Named(Named::Escape), Modifiers::empty()),
                status,
                window,
            );
            assert!(
                matches!(esc, Some(Message::EscapeKey)),
                "{status:?}: {esc:?}"
            );
            // F1 pins the focused field's help, even while typing.
            let f1 = crate::on_event(
                key_press(Key::Named(Named::F1), Modifiers::empty()),
                status,
                window,
            );
            assert!(matches!(f1, Some(Message::HelpKey)), "{status:?}: {f1:?}");
        }
    }

    #[test]
    fn escape_is_the_estop_while_a_machine_is_connected() {
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        zeroed(&mut a);
        // Off the Control page too, and the pinned help is not what it closes.
        a.page = Page::Work;
        let tag = crate::help::FieldTag {
            target: Target::Setup,
            key: "stock_top".into(),
            label: "Stock top".into(),
        };
        let _ = a.update(Message::HelpFor(Some(tag.clone())));
        let _ = a.update(Message::EscapeKey);
        tick_until(&mut a, 2.0, "E-stop", |s| s.state == Some(State::EStop));
        assert_eq!(a.pinned_help, Some(tag));
        // The pinned card's hint says so while the machine is connected,
        // and says Esc closes it once it is not (issue #185).
        assert!(!a.esc_closes_pinned_help());
        let texts = page_texts(&a);
        assert!(
            texts
                .iter()
                .any(|t| t == crate::help::PINNED_HINT_ESC_IS_ESTOP)
                && !texts
                    .iter()
                    .any(|t| t == crate::help::PINNED_HINT_ESC_CLOSES),
            "{texts:?}"
        );
        let _ = a.update(Message::Disconnect);
        assert!(a.control.is_none());
        assert!(a.esc_closes_pinned_help());
        assert!(
            page_texts(&a)
                .iter()
                .any(|t| t == crate::help::PINNED_HINT_ESC_CLOSES)
        );
    }

    /// A view laid out without a window, as `ui::probe` lays one out, for
    /// widget operations over its tree: what the keys find there.
    struct Laid<'a> {
        ui: iced_runtime::user_interface::UserInterface<'a, Message, iced::Theme, iced::Renderer>,
        renderer: iced::Renderer,
    }
    fn laid(a: &App) -> Laid<'_> {
        let mut renderer = iced_renderer::fallback::Renderer::Secondary(
            iced_tiny_skia::Renderer::new(iced::Font::DEFAULT, iced::Pixels(16.)),
        );
        let ui = iced_runtime::user_interface::UserInterface::build(
            a.view(),
            WINDOW,
            iced_runtime::user_interface::Cache::default(),
            &mut renderer,
        );
        Laid { ui, renderer }
    }
    /// The window the views are laid out in.
    const WINDOW: iced::Size = iced::Size::new(1440., 900.);
    impl Laid<'_> {
        /// Runs `op` over the tree, each pass of a chain, for its outcome.
        fn run<T: Send + 'static>(
            &mut self,
            op: impl iced::advanced::widget::Operation<T> + 'static,
        ) -> Option<T> {
            use iced::advanced::widget::{Operation, operation};
            let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
            let put = slot.clone();
            let mut op: Box<dyn Operation> =
                Box::new(operation::map(op, move |x| *put.lock().unwrap() = Some(x)));
            loop {
                self.ui.operate(&self.renderer, &mut op);
                match op.finish() {
                    operation::Outcome::Chain(next) => op = next,
                    _ => break,
                }
            }
            slot.lock().unwrap().take()
        }
    }
    /// The texts of the app's view, in tree order.
    fn page_texts(a: &App) -> Vec<String> {
        crate::ui::probe::texts(a.view(), WINDOW)
    }

    /// What a view offers to widget operations: the form fields' tags and
    /// the titles of the hover cards (`help::HoverHelp`).
    #[derive(Default)]
    struct Helps {
        tags: Vec<crate::help::FieldTag>,
        cards: Vec<crate::help::Help>,
    }
    impl iced::advanced::widget::Operation<(Vec<crate::help::FieldTag>, Vec<crate::help::Help>)>
        for Helps
    {
        fn traverse(
            &mut self,
            operate: &mut dyn FnMut(
                &mut dyn iced::advanced::widget::Operation<(
                    Vec<crate::help::FieldTag>,
                    Vec<crate::help::Help>,
                )>,
            ),
        ) {
            operate(self)
        }
        fn custom(
            &mut self,
            _: Option<&iced::advanced::widget::Id>,
            _: iced::Rectangle,
            state: &mut dyn std::any::Any,
        ) {
            if let Some(tag) = state.downcast_ref::<crate::help::FieldTag>() {
                self.tags.push(tag.clone());
            }
            if let Some(card) = state.downcast_ref::<crate::help::Help>() {
                self.cards.push(card.clone());
            }
        }
        fn finish(
            &self,
        ) -> iced::advanced::widget::operation::Outcome<(
            Vec<crate::help::FieldTag>,
            Vec<crate::help::Help>,
        )> {
            iced::advanced::widget::operation::Outcome::Some((
                self.tags.clone(),
                self.cards.clone(),
            ))
        }
    }

    /// The Control page's RPM, MDI, jog speed, feed override and calibration
    /// fields have their help (#195): the typed fields are form fields that
    /// F1 pins and the sliders have hover cards, each with the text of its
    /// command.
    #[test]
    fn the_control_pages_typed_fields_and_sliders_have_help() {
        use crate::control::Action as Ctl;
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        zeroed(&mut a);
        let (tags, cards) = laid(&a).run(Helps::default()).expect("the Control page");
        for (key, command) in [
            ("control_rpm", Ctl::Rpm(String::new())),
            ("mdi_line", Ctl::MdiText(String::new())),
            ("work_set", Ctl::WorkText(0, String::new())),
        ] {
            let tag = tags
                .iter()
                .find(|t| t.key == key)
                .unwrap_or_else(|| panic!("{key} is no form field: {tags:?}"));
            let card = tag.help();
            let command = crate::help::command(&Message::Machine(command), tag.label.clone());
            assert_eq!(card.body, command.body, "{key}");
            assert!(!card.body.is_empty(), "{key}");
        }
        for title in ["Jog speed", "Feed override"] {
            let card = (cards.iter().find(|c| c.title == title))
                .unwrap_or_else(|| panic!("no card {title}"));
            assert!(!card.body.is_empty(), "{title}");
        }
        // The calibration fields are in the Controller configuration view.
        let _ = a.update(Message::ControlSetup(true));
        let (tags, _) = laid(&a).run(Helps::default()).expect("the Setup view");
        for key in ["cal_distance", "cal_measured"] {
            let tag = (tags.iter().find(|t| t.key == key))
                .unwrap_or_else(|| panic!("{key} is no form field: {tags:?}"));
            assert!(!tag.help().body.is_empty(), "{key}");
        }
    }

    #[test]
    fn a_pocket_stepover_over_50_percent_is_shown_under_the_stepover_field() {
        let mut a = app();
        let _ = a.update(Message::AddOp(Strategy::Pocket));
        let pocket = (a.doc.setup().operations.iter())
            .position(|o| o.strategy == Strategy::Pocket)
            .unwrap();
        let _ = a.update(Message::SelectOp(pocket));
        let _ = a.update(Message::Edit(Target::Operation, "stepover", "60".into()));
        let _ = a.update(Message::Tab("Passes".into()));
        let checks = crate::ui::editor_checks(&a);
        let stepover = (checks.list.iter())
            .find(|c| c.text.contains("Pocket stepover"))
            .unwrap_or_else(|| panic!("{:?}", checks.list))
            .clone();
        assert_eq!(
            (stepover.error, stepover.key, stepover.label()),
            (true, Some("stepover"), "Error")
        );
        assert!(checks.drawn.contains(&"stepover"), "{:?}", checks.drawn);
        assert!(!checks.rest().contains(&&stepover));
        // In the widget tree the message follows the field's label, labelled.
        let texts = page_texts(&a);
        let at = (texts.iter())
            .position(|t| t == "Stepover · % tool diameter")
            .unwrap_or_else(|| panic!("{texts:?}"));
        assert_eq!(
            (texts[at + 1].as_str(), texts[at + 2].as_str()),
            ("Error", stepover.text.as_str())
        );
        // On Tool, which has no stepover field, the list under the form keeps it.
        let _ = a.update(Message::Tab("Tool".into()));
        let checks = crate::ui::editor_checks(&a);
        assert!(!checks.drawn.contains(&"stepover"));
        assert!(checks.rest().contains(&&stepover), "{:?}", checks.rest());
        let texts = page_texts(&a);
        assert!(texts.contains(&stepover.text), "{texts:?}");
    }

    #[test]
    fn a_warning_and_an_error_have_different_labels() {
        let mut a = app();
        if a.doc.selected_material().is_none() {
            a.doc.materials = crate::material::starters();
            let id = a.doc.materials[0].id.clone();
            let _ = a.update(Message::PickMaterial(id));
        }
        assert!(a.doc.selected_material().is_some());
        // The Face operation: a feed of 1 mm/min is a chip load under the
        // material's band, a warning; a stepover of 0 is an error.
        let _ = a.update(Message::Edit(Target::Operation, "feed", "1".into()));
        let _ = a.update(Message::Edit(Target::Operation, "stepover", "0".into()));
        let checks = crate::ui::op_checks(&a);
        let warning = (checks.iter())
            .find(|c| c.text.starts_with("Chip load"))
            .unwrap_or_else(|| panic!("{checks:?}"));
        let error = (checks.iter())
            .find(|c| c.text.starts_with("stepover"))
            .unwrap_or_else(|| panic!("{checks:?}"));
        assert_eq!(
            (warning.label(), warning.error, warning.key),
            ("Warning", false, Some("feed"))
        );
        assert_eq!(
            (error.label(), error.error, error.key),
            ("Error", true, Some("stepover"))
        );
        // Each at its field: the warning on Tool, where the feed is, with the
        // error listed under the form.
        let _ = a.update(Message::Tab("Tool".into()));
        let texts = page_texts(&a);
        let at = (texts.iter())
            .position(|t| t == "Cutting feed · mm/min")
            .unwrap_or_else(|| panic!("{texts:?}"));
        assert_eq!(
            (texts[at + 1].as_str(), texts[at + 2].as_str()),
            ("Warning", warning.text.as_str())
        );
        assert!(
            texts.contains(&"1 error stops Generate and export".to_string())
                && texts.contains(&error.text),
            "{texts:?}"
        );
        let _ = a.update(Message::Edit(Target::Operation, "feed", "1200".into()));
        let _ = a.update(Message::Edit(Target::Operation, "stepover", "40".into()));
        assert!(
            crate::ui::op_checks(&a).iter().all(|c| !c.error),
            "{:?}",
            crate::ui::op_checks(&a)
        );
    }

    #[test]
    fn an_audit_row_opens_its_operation_at_the_tab_its_line_names() {
        let mut a = imported_app();
        let mesh = a.imported_mesh.as_ref().unwrap();
        let top = (mesh.surfaces.iter().enumerate())
            .filter(|(_, s)| s.normal[2].abs() > 0.99999)
            .max_by(|(_, a), (_, b)| a.center[2].total_cmp(&b.center[2]))
            .unwrap()
            .0;
        let _ = a.update(Message::Surface(top));
        let _ = a.update(Message::Edit(Target::Setup, "stock_top", "10".into()));
        let _ = a.update(Message::Edit(Target::Operation, "rpm", "0".into()));
        assert!(
            a.doc.setup().issues().is_empty(),
            "{:?}",
            a.doc.setup().issues()
        );
        assert_eq!(
            a.op_issues().first().map(String::as_str),
            Some("rpm must be greater than zero.")
        );
        let _ = a.update(Message::Tab("Passes".into()));
        let _ = a.update(Message::GenerateAll);
        assert_eq!(a.page, Page::Audit);
        let (i, row) = (a.audit_report.iter().enumerate())
            .find(|(_, r)| !r.ok)
            .map(|(i, r)| (i, r.clone()))
            .unwrap_or_else(|| panic!("{:?}", a.audit_report));
        assert_eq!((row.setup, row.operation), (0, 0));
        assert!(row.detail.starts_with("rpm must"), "{row:?}");
        let _ = a.view();
        let _ = a.update(Message::AuditRow(i));
        assert_eq!(
            (a.page, a.selected, a.tab.as_str()),
            (Page::Work, Some(0), "Tool")
        );
        assert_eq!(a.doc.active, a.doc.setups[0].id);
        for (issue, tab) in [
            ("rpm must be greater than zero.", Some("Tool")),
            (
                "Pocket stepover must be between 0.01 and 50% for overlapping offset contours.",
                Some("Passes"),
            ),
            ("Clearance must be above retract height.", Some("Heights")),
            (
                "Select at least one machining feature or surface.",
                Some("Geometry"),
            ),
            ("Choose a tool from the library.", Some("Tool")),
            ("Give this operation a name.", None),
            (
                "Generated 12 moves · 0.3 min · inherits stock after 0 operations",
                None,
            ),
        ] {
            assert_eq!(crate::ui::issue_tab(issue), tab, "{issue}");
        }
    }

    #[test]
    fn tab_with_nothing_focused_enters_the_form_unless_the_keys_jog() {
        use crate::{TabMove, tab_move};
        assert_eq!(tab_move(false, false, false), TabMove::First);
        assert_eq!(tab_move(false, true, false), TabMove::First);
        assert_eq!(tab_move(false, false, true), TabMove::Stay);
        assert_eq!(tab_move(true, false, true), TabMove::Next);
        assert_eq!(tab_move(true, true, true), TabMove::Previous);
        // The key's message starts the focus task.
        let mut a = app();
        for back in [false, true] {
            assert_eq!(a.update(Message::TabKey(back)).units(), 1, "back: {back}");
        }
        // Over the Work page's widget tree: nothing is focused, so the first
        // Tab lands in the inspector's first field, the operation's name,
        // which F1 then finds.
        let mut laid = laid(&a);
        assert_eq!(laid.run(crate::control::typing()), Some(false));
        assert_eq!(laid.run(crate::help::focused_field()), Some(None));
        laid.run(crate::control::focus_first());
        assert_eq!(laid.run(crate::control::typing()), Some(true));
        let tag = (laid.run(crate::help::focused_field()).flatten()).expect("a focused form field");
        assert_eq!(
            (tag.target, tag.key.as_str(), tag.label.as_str()),
            (Target::Operation, "name", "Operation name")
        );
    }

    #[test]
    fn f1_pins_the_focused_fields_help_and_esc_puts_it_away_where_it_is_not_the_estop() {
        let mut a = app();
        // The spindle speed: a card with a More… text.
        let tag = crate::help::FieldTag {
            target: Target::Operation,
            key: "rpm".into(),
            label: "Spindle speed · rpm".into(),
        };
        let help = tag.help();
        assert!(
            help.body.contains("revolutions") || !help.body.is_empty(),
            "{help:?}"
        );
        let shows_help = |a: &App| page_texts(a).contains(&help.body);
        assert!(!shows_help(&a));
        let _ = a.update(Message::HelpFor(Some(tag.clone())));
        assert_eq!(a.pinned_help.as_ref(), Some(&tag));
        assert!(
            shows_help(&a),
            "the pinned help shows the field's hover text"
        );
        // Open to its More… text; the card's Less and More… toggles, which
        // no hover overlay takes here, close and open it.
        assert!(!help.more.is_empty(), "{help:?}");
        assert!(page_texts(&a).contains(&help.more), "and its More… text");
        let _ = a.update(Message::HelpMore(false));
        assert!(!page_texts(&a).contains(&help.more) && shows_help(&a));
        let _ = a.update(Message::HelpMore(true));
        assert!(page_texts(&a).contains(&help.more));
        // F1 on the same field puts it away; on another, switches to it; with
        // no field focused, puts it away.
        let _ = a.update(Message::HelpFor(Some(tag.clone())));
        assert!(a.pinned_help.is_none() && !shows_help(&a));
        let _ = a.update(Message::HelpFor(Some(tag.clone())));
        let other = crate::help::FieldTag {
            target: Target::Operation,
            key: "stepover".into(),
            label: "Stepover · % tool diameter".into(),
        };
        let _ = a.update(Message::HelpFor(Some(other.clone())));
        assert_eq!(a.pinned_help.as_ref(), Some(&other));
        let _ = a.update(Message::HelpFor(None));
        assert!(a.pinned_help.is_none());
        // Esc puts it away on the Work page, where nothing is connected, and
        // the card's hint says so; on the Control page Esc is kept for the
        // E-stop and leaves it, and the hint says that instead (issue #185).
        let _ = a.update(Message::HelpFor(Some(tag.clone())));
        assert!(
            page_texts(&a)
                .iter()
                .any(|t| t == crate::help::PINNED_HINT_ESC_CLOSES)
        );
        let _ = a.update(Message::EscapeKey);
        assert!(a.pinned_help.is_none());
        let _ = a.update(Message::HelpFor(Some(tag.clone())));
        a.page = Page::Control;
        let texts = page_texts(&a);
        assert!(
            texts
                .iter()
                .any(|t| t == crate::help::PINNED_HINT_ESC_IS_ESTOP)
                && !texts
                    .iter()
                    .any(|t| t == crate::help::PINNED_HINT_ESC_CLOSES),
            "{texts:?}"
        );
        let _ = a.update(Message::EscapeKey);
        assert_eq!(a.pinned_help.as_ref(), Some(&tag));
        // Leaving for another page puts it away.
        let _ = a.update(Message::Page(Page::Work));
        assert!(a.pinned_help.is_none());
    }

    #[test]
    fn a_pinned_card_with_an_animation_ticks_fast_until_paused() {
        let mut a = app();
        assert!(!a.ticks_fast(), "idle, the app ticks every 500 ms");
        // A text-only card: no animation to drive.
        let rpm = crate::help::FieldTag {
            target: Target::Machine,
            key: "rpm".into(),
            label: "Maximum spindle speed · rpm".into(),
        };
        assert!(rpm.help().diagram.is_none());
        let _ = a.update(Message::HelpFor(Some(rpm)));
        assert!(!a.ticks_fast());
        // The requested fixture clearance's animated card (issue #186):
        // fast while it plays, slow while it is paused, fast again on Play.
        let clearance = crate::help::FieldTag {
            target: Target::Setup,
            key: "clearance".into(),
            label: "Requested fixture clearance · mm".into(),
        };
        assert!(clearance.help().diagram.is_some());
        let _ = a.update(Message::HelpFor(Some(clearance.clone())));
        assert!(a.ticks_fast());
        let _ = a.update(Message::HelpPause(true));
        assert!(!a.ticks_fast());
        let _ = a.update(Message::HelpPause(false));
        assert!(a.ticks_fast());
        // Pinned again after Pause, it plays; put away, the ticks slow.
        let _ = a.update(Message::HelpPause(true));
        let _ = a.update(Message::HelpFor(None));
        assert!(!a.ticks_fast());
        let _ = a.update(Message::HelpFor(Some(clearance)));
        assert!(a.ticks_fast(), "a card pinned anew is not paused");
        let _ = a.update(Message::HelpFor(None));
        assert!(!a.ticks_fast());
    }

    #[test]
    fn space_holds_the_job_only_when_no_text_field_takes_it() {
        use iced::keyboard::{Key, Modifiers, key::Named};
        let window = iced::window::Id::unique();
        let space = || key_press(Key::Named(Named::Space), Modifiers::empty());
        let typed = crate::on_event(space(), iced::event::Status::Captured, window);
        assert!(
            typed.is_none(),
            "a focused field takes Space as a character: {typed:?}"
        );
        let held = crate::on_event(space(), iced::event::Status::Ignored, window);
        assert!(matches!(held, Some(Message::HoldKey)), "{held:?}");
    }

    #[test]
    fn command_n_s_and_o_start_save_and_open_a_project() {
        use iced::keyboard::{Key, Modifiers};
        let window = iced::window::Id::unique();
        let status = iced::event::Status::Ignored;
        // Cmd on macOS, Ctrl elsewhere; the other one is not the shortcut key.
        let (command, other) = if cfg!(target_os = "macos") {
            (Modifiers::LOGO, Modifiers::CTRL)
        } else {
            (Modifiers::CTRL, Modifiers::LOGO)
        };
        let press = |letter: &str, modifiers| {
            crate::on_event(
                key_press(Key::Character(letter.into()), modifiers),
                status,
                window,
            )
        };
        let save = press("s", command);
        assert!(matches!(save, Some(Message::SaveProject)), "{save:?}");
        let new = press("n", command);
        assert!(matches!(new, Some(Message::NewProject)), "{new:?}");
        let open = press("o", command);
        assert!(matches!(open, Some(Message::OpenProject)), "{open:?}");
        // Caps Lock or Shift gives an upper-case letter; it is the same shortcut.
        let shifted = press("S", command | Modifiers::SHIFT);
        assert!(matches!(shifted, Some(Message::SaveProject)), "{shifted:?}");
        // Without the command key, or with the wrong one, a letter does nothing.
        for (letter, modifiers) in [
            ("s", Modifiers::empty()),
            ("n", other),
            ("o", Modifiers::ALT),
        ] {
            let none = press(letter, modifiers);
            assert!(none.is_none(), "{letter} with {modifiers:?}: {none:?}");
        }
        let other_letter = press("q", command);
        assert!(other_letter.is_none(), "{other_letter:?}");
    }

    #[test]
    fn work_zero_and_the_coordinate_systems() {
        use crate::control::Action;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = uc100_app(dir.path());
        idle(&mut a);
        mdi(&mut a, "G0 X5 Y6");
        let _ = a.update(Message::Machine(Action::WorkZero(Some(0))));
        tick_until(&mut a, 2.0, "work X 0", |s| s.work_pos[0].abs() < 1e-3);
        assert!((conn(&a).snapshot.work_pos[1] - 6.).abs() < 1e-3);
        let _ = a.update(Message::Machine(Action::WorkZero(None)));
        tick_until(&mut a, 2.0, "work X, Y, Z 0", |s| {
            s.work_pos[..3].iter().all(|v| v.abs() < 1e-3)
        });
        // G55 has its own offsets, still unset.
        let _ = a.update(Message::Machine(Action::Wcs(1)));
        tick_until(&mut a, 2.0, "G55", |s| {
            s.active_wcs == 1 && s.state == Some(State::Idle)
        });
        assert!((conn(&a).snapshot.work_pos[0] - 5.).abs() < 1e-3);
        let _ = a.update(Message::Machine(Action::Wcs(0)));
        tick_until(&mut a, 2.0, "G54", |s| {
            s.active_wcs == 0 && s.state == Some(State::Idle)
        });
        assert!(conn(&a).snapshot.work_pos[0].abs() < 1e-3);
        // Nothing changes in E-stop, and G60 does not exist.
        let _ = a.update(Message::Machine(Action::Wcs(6)));
        let _ = a.update(Message::MachineEStop);
        tick_until(&mut a, 2.0, "E-stop", |s| s.state == Some(State::EStop));
        let _ = a.update(Message::Machine(Action::WorkZero(Some(1))));
        tick_for(&mut a, 100);
        assert!((conn(&a).snapshot.work_pos[1]).abs() < 1e-3);
        assert_eq!(conn(&a).snapshot.active_wcs, 0);
    }

    #[test]
    fn spindle_and_coolant_follow_the_profile() {
        use crate::control::Action;
        use uc100::{controller::State, gcode::Spindle};
        let dir = tempfile::tempdir().unwrap();
        let mut a = profile_app(dir.path(), LIMITED);
        idle(&mut a);
        assert!(!conn(&a).reversible());
        let _ = a.update(Message::Machine(Action::Rpm("12000".into())));
        let _ = a.update(Message::Machine(Action::Spindle(Spindle::Cw)));
        tick_until(&mut a, 2.0, "CW", |s| {
            s.spindle == Some(Spindle::Cw) && s.rpm == 12000.
        });
        // Reversing, or CCW without a direction output, would be an E-stop:
        // neither is sent.
        let _ = a.update(Message::Machine(Action::Spindle(Spindle::Ccw)));
        tick_for(&mut a, 100);
        let s = &conn(&a).snapshot;
        assert_eq!(
            (s.state.clone(), s.spindle),
            (Some(State::Idle), Some(Spindle::Cw))
        );
        let _ = a.update(Message::Machine(Action::Spindle(Spindle::Off)));
        tick_until(&mut a, 2.0, "off", |s| s.spindle == Some(Spindle::Off));
        let _ = a.update(Message::Machine(Action::Spindle(Spindle::Ccw)));
        tick_for(&mut a, 100);
        let s = &conn(&a).snapshot;
        assert_eq!(
            (s.state.clone(), s.spindle),
            (Some(State::Idle), Some(Spindle::Off))
        );
        // Flood is wired, mist is not.
        let _ = a.update(Message::Machine(Action::Flood(true)));
        tick_until(&mut a, 2.0, "flood", |s| s.flood);
        let _ = a.update(Message::Machine(Action::Mist(true)));
        tick_for(&mut a, 100);
        assert!(!conn(&a).snapshot.mist);
        // An RPM that is not a number, or above the profile's maximum, starts nothing.
        for rpm in ["abc", "30000", "0"] {
            let _ = a.update(Message::Machine(Action::Rpm(rpm.into())));
            assert_eq!(conn(&a).rpm_value(), None, "{rpm}");
            let _ = a.update(Message::Machine(Action::Spindle(Spindle::Cw)));
        }
        tick_for(&mut a, 100);
        assert_eq!(conn(&a).snapshot.spindle, Some(Spindle::Off));
        let _ = a.update(Message::Disconnect);
        let dir = tempfile::tempdir().unwrap();
        let mut a = profile_app(dir.path(), SWITCHED);
        idle(&mut a);
        assert!(conn(&a).reversible());
        let _ = a.update(Message::Machine(Action::Rpm("8000".into())));
        let _ = a.update(Message::Machine(Action::Spindle(Spindle::Ccw)));
        tick_until(&mut a, 2.0, "CCW", |s| {
            s.spindle == Some(Spindle::Ccw) && s.rpm == 8000.
        });
    }

    #[test]
    fn feed_and_limit_override() {
        use crate::control::Action;
        let dir = tempfile::tempdir().unwrap();
        let mut a = uc100_app(dir.path());
        idle(&mut a);
        let _ = a.update(Message::Machine(Action::FeedOverride(150.)));
        tick_until(&mut a, 2.0, "150 %", |s| {
            (s.feed_override - 1.5).abs() < 1e-9
        });
        let _ = a.update(Message::Machine(Action::FeedOverride(500.)));
        tick_until(&mut a, 2.0, "200 %", |s| {
            (s.feed_override - 2.).abs() < 1e-9
        });
        assert_eq!(conn(&a).feed_override, 200.);
        // Without limit switches there is nothing to override.
        assert!(!conn(&a).limit_inputs());
        let _ = a.update(Message::Machine(Action::OverrideLimits(true)));
        tick_for(&mut a, 100);
        assert!(!conn(&a).snapshot.limits_override);
        let _ = a.update(Message::Disconnect);
        let dir = tempfile::tempdir().unwrap();
        let mut a = profile_app(dir.path(), SWITCHED);
        idle(&mut a);
        assert!(conn(&a).limit_inputs());
        let _ = a.update(Message::Machine(Action::OverrideLimits(true)));
        // It turns itself off a second after no switch is active.
        tick_until(&mut a, 0.8, "limit override", |s| s.limits_override);
    }

    #[test]
    fn an_mdi_line_runs_and_the_field_clears() {
        use crate::control::Action;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = uc100_app(dir.path());
        idle(&mut a);
        let _ = a.update(Message::Machine(Action::MdiText("G0 X5".into())));
        assert_eq!(conn(&a).mdi, "G0 X5");
        let _ = a.update(Message::Machine(Action::MdiSend));
        assert_eq!(conn(&a).mdi, "");
        tick_until(&mut a, 3.0, "X at 5", |s| {
            s.state == Some(State::Idle) && (s.machine_pos[0] - 5.).abs() < 1e-3
        });
        // An empty line sends nothing; a line in E-stop stays in the field.
        let lines = conn(&a).snapshot.log.len();
        let _ = a.update(Message::Machine(Action::MdiSend));
        let _ = a.update(Message::MachineEStop);
        tick_until(&mut a, 2.0, "E-stop", |s| s.state == Some(State::EStop));
        let _ = a.update(Message::Machine(Action::MdiText("G0 X0".into())));
        let _ = a.update(Message::Machine(Action::MdiSend));
        tick_for(&mut a, 100);
        assert_eq!(conn(&a).mdi, "G0 X0");
        assert!(
            !conn(&a).snapshot.log[lines..]
                .iter()
                .any(|l| l.starts_with("MDI"))
        );
    }

    /// The profile open on Setup.
    fn draft(a: &App) -> &crate::setup::Draft {
        a.profile_draft
            .as_ref()
            .and_then(|(_, d)| d.as_ref().ok())
            .expect("profile open")
    }

    fn setup_edit(a: &mut App, e: crate::setup::Edit) {
        let _ = a.update(Message::Setup(crate::setup::Msg::Edit(e)));
    }

    #[test]
    fn setup_edits_and_saves_the_profile() {
        use crate::setup::{Edit, Msg};
        let dir = tempfile::tempdir().unwrap();
        let mut a = profile_app(dir.path(), LIMITED);
        a.page = Page::Control;
        let _ = a.update(Message::ControlSetup(true));
        assert!(draft(&a).checked.is_ok());
        let _ = a.view();
        setup_edit(&mut a, Edit::Number("axes.x.soft_max".into(), "380".into()));
        setup_edit(&mut a, Edit::Pin("inputs.estop".into(), Some(10)));
        // Back to Control and to Setup again: the changes stay.
        let _ = a.update(Message::ControlSetup(false));
        let _ = a.update(Message::ControlSetup(true));
        assert_eq!(draft(&a).number("axes.x.soft_max"), "380");
        let _ = a.view();
        let _ = a.update(Message::Setup(Msg::Save));
        assert_eq!(draft(&a).note.as_deref(), Some("Saved."));
        let cfg = crate::control::profile(&a.doc.machines[0]).unwrap();
        assert_eq!(cfg.axes.x.unwrap().soft_max, Some(380.));
        assert_eq!(cfg.inputs.estop.unwrap().pin, 10);
        // Revert throws changes away.
        setup_edit(&mut a, Edit::Number("axes.x.soft_max".into(), "1".into()));
        let _ = a.update(Message::Setup(Msg::Revert));
        assert_eq!(draft(&a).number("axes.x.soft_max"), "380");
        // A profile the driver refuses is shown and not saved.
        setup_edit(&mut a, Edit::Pin("axes.y.step".into(), Some(2)));
        assert!(draft(&a).blocker().unwrap().contains("already used"));
        let _ = a.view();
        let _ = a.update(Message::Setup(Msg::Save));
        let cfg = crate::control::profile(&a.doc.machines[0]).unwrap();
        assert_eq!(cfg.axes.y.unwrap().step.pin, 4);
        // A file that is not TOML is shown as such.
        std::fs::write(dir.path().join("uc100.toml"), "[axes.x\n").unwrap();
        let _ = a.update(Message::Setup(Msg::Revert));
        assert!(matches!(&a.profile_draft, Some((_, Err(e))) if e.contains("not valid TOML")));
        let _ = a.view();
    }

    #[test]
    fn outputs_are_forced_only_while_idle_and_on_free_pins() {
        use crate::control::Action;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = profile_app(dir.path(), LIMITED);
        a.page = Page::Control;
        let _ = a.update(Message::ControlSetup(true));
        idle(&mut a);
        let level = |a: &App, pin: u8| {
            let st = conn(a).snapshot.status.unwrap();
            uc100::protocol::OutputWord(st.outputs).level(pin)
        };
        assert_eq!(level(&a, 14), Some(false));
        let _ = a.update(Message::Machine(Action::TestOutput(14, Some(true))));
        let _ = a.view();
        tick_until(&mut a, 2.0, "pin 14 HIGH", |s| {
            uc100::protocol::OutputWord(s.status.unwrap().outputs).level(14) == Some(true)
        });
        assert_eq!(conn(&a).forced.get(&14), Some(&true));
        let _ = a.update(Message::Machine(Action::TestOutput(14, None)));
        tick_until(&mut a, 2.0, "pin 14 released", |s| {
            uc100::protocol::OutputWord(s.status.unwrap().outputs).level(14) == Some(false)
        });
        assert!(conn(&a).forced.is_empty());
        // A step pin and the spindle PWM are the controller's.
        for pin in [2, 16] {
            let _ = a.update(Message::Machine(Action::TestOutput(pin, Some(true))));
        }
        assert!(conn(&a).forced.is_empty());
        // E-stop releases a forced pin, and nothing is forced in E-stop.
        let _ = a.update(Message::Machine(Action::TestOutput(17, Some(true))));
        tick_until(&mut a, 2.0, "pin 17 HIGH", |s| {
            uc100::protocol::OutputWord(s.status.unwrap().outputs).level(17) == Some(true)
        });
        let _ = a.update(Message::MachineEStop);
        tick_until(&mut a, 2.0, "E-stop", |s| s.state == Some(State::EStop));
        assert!(conn(&a).forced.is_empty());
        tick_until(&mut a, 2.0, "pin 17 released", |s| {
            uc100::protocol::OutputWord(s.status.unwrap().outputs).level(17) == Some(false)
        });
        let _ = a.update(Message::Machine(Action::TestOutput(17, Some(true))));
        assert!(conn(&a).forced.is_empty());
        let _ = a.view();
        // Input pins are not outputs.
        assert!(level(&a, 10).is_none());
    }

    #[test]
    fn calibration_measures_steps_per_mm_and_fills_the_profile() {
        use crate::control::Action;
        use crate::setup::Msg;
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = profile_app(dir.path(), LIMITED);
        a.page = Page::Control;
        let _ = a.update(Message::ControlSetup(true));
        idle(&mut a);
        let _ = a.update(Message::Machine(Action::MachineZero(None)));
        tick_until(&mut a, 3.0, "X, Y and Z set", |s| {
            s.referenced[..3].iter().all(|r| *r)
        });
        for action in [
            Action::CalAxis(0),
            Action::CalDistance("10".into()),
            Action::CalMove,
        ] {
            let _ = a.update(Message::Machine(action));
        }
        let _ = a.view();
        std::thread::sleep(std::time::Duration::from_millis(60));
        tick_until(&mut a, 5.0, "X at 10", |s| {
            s.state == Some(State::Idle) && (s.machine_pos[0] - 10.).abs() < 1e-3
        });
        // 10 mm at the profile's 200 steps per mm.
        assert_eq!(conn(&a).cal_pulses(), Some(2000));
        let _ = a.update(Message::Machine(Action::CalMeasured("9.95".into())));
        let v = conn(&a).calibrated().unwrap().unwrap();
        assert!((v - 201.005_025).abs() < 1e-5, "{v}");
        let _ = a.view();
        let _ = a.update(Message::Setup(Msg::UseCalibration));
        assert_eq!(draft(&a).number("axes.x.steps_per_unit"), "201.005");
        let _ = a.update(Message::Setup(Msg::Save));
        assert_eq!(
            conn(&a).config.axes.x.as_ref().unwrap().steps_per_unit,
            201.005
        );
        // A move that is not a number, or one in E-stop, is not made.
        tick_until(&mut a, 3.0, "E-stop after reconnecting", |s| {
            s.state == Some(State::EStop)
        });
        let _ = a.update(Message::Machine(Action::CalMove));
        assert_eq!(conn(&a).cal_pulses(), None);
        let _ = a.update(Message::Machine(Action::CalDistance("far".into())));
        assert_eq!(conn(&a).cal_distance(), None);
        let _ = a.view();
    }

    #[test]
    fn saving_while_connected_reconnects_with_the_new_profile() {
        use crate::control::Action;
        use crate::setup::{Edit, Msg};
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = profile_app(dir.path(), LIMITED);
        a.page = Page::Control;
        idle(&mut a);
        let _ = a.update(Message::Machine(Action::MachineZero(None)));
        tick_until(&mut a, 3.0, "X, Y and Z set", |s| {
            s.referenced[..3].iter().all(|r| *r)
        });
        let _ = a.update(Message::ControlSetup(true));
        setup_edit(
            &mut a,
            Edit::Number("axes.x.steps_per_unit".into(), "250".into()),
        );
        // Not while the machine moves.
        let _ = a.update(Message::Machine(Action::JogPress(1, false)));
        tick_until(&mut a, 3.0, "jogging", |s| s.state == Some(State::Jog));
        let _ = a.view();
        let _ = a.update(Message::Setup(Msg::Save));
        assert_eq!(
            draft(&a).note.as_deref(),
            Some("Stop the machine before saving the profile.")
        );
        let _ = a.update(Message::Machine(Action::JogRelease));
        tick_until(&mut a, 3.0, "Idle", |s| s.state == Some(State::Idle));
        let _ = a.update(Message::Setup(Msg::Save));
        assert!(
            draft(&a)
                .note
                .as_deref()
                .unwrap()
                .starts_with("Saved, and the machine was reconnected")
        );
        assert_eq!(
            conn(&a).config.axes.x.as_ref().unwrap().steps_per_unit,
            250.
        );
        assert!(conn(&a).simulated);
        tick_until(&mut a, 3.0, "E-stop after reconnecting", |s| {
            s.state == Some(State::EStop)
        });
        // The machine position counted with 200 steps per mm is gone.
        assert!(conn(&a).snapshot.referenced.iter().all(|r| !r));
        let _ = a.view();
    }

    #[test]
    fn picking_another_machine_while_connected_asks_then_switches() {
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = uc100_app(dir.path());
        let mut other = a.doc.machines[0].clone();
        other.id = "other".into();
        other.p.insert("name".into(), "Second router".into());
        a.doc.machines.push(other);
        let first = a.doc.machines[0].id.clone();
        a.page = Page::Control;
        let _ = a.update(Message::Connect);
        tick_until(&mut a, 3.0, "E-stop after connecting", |s| {
            s.state == Some(State::EStop)
        });
        // The connected machine itself asks nothing.
        let _ = a.update(Message::ControlMachine(first.clone()));
        assert!(a.switch_machine.is_none());
        let _ = a.update(Message::ControlMachine("other".into()));
        assert_eq!(conn(&a).machine_id, first);
        shows(&a, &["Switch to Second router?", "Disconnect and switch"]);
        let _ = a.update(Message::SwitchMachine(false));
        assert!(a.switch_machine.is_none() && conn(&a).machine_id == first);
        lacks(&a, &["Switch to Second router?"]);
        let _ = a.update(Message::ControlMachine("other".into()));
        let _ = a.update(Message::SwitchMachine(true));
        assert_eq!(conn(&a).machine_id, "other");
        assert_eq!(a.control_target().unwrap().id, "other");
        tick_until(&mut a, 3.0, "E-stop after switching", |s| {
            s.state == Some(State::EStop)
        });
        // A Disconnect drops a question still open.
        let _ = a.update(Message::ControlMachine(first));
        let _ = a.update(Message::Disconnect);
        assert!(a.switch_machine.is_none() && a.control.is_none());
    }

    #[test]
    fn the_real_or_simulated_switch_saves_the_machines_connection() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = uc100_app(dir.path());
        a.page = Page::Control;
        a.machine_draft = a.doc.machines[0].clone();
        shows(&a, &["Real (USB)", "Simulated"]);
        let _ = a.update(Message::MachineConnection(false));
        assert!(!crate::control::simulated(&a.doc.machines[0]));
        assert!(!crate::control::simulated(&a.machine_draft));
        assert_eq!(a.doc.machines[0].version, 1, "not a planning change");
        assert!(a.control.is_none(), "choosing does not connect");
        let _ = a.update(Message::MachineConnection(true));
        assert!(crate::control::simulated(&a.doc.machines[0]));
        let _ = a.update(Message::Connect);
        assert!(conn(&a).simulated);
        // The way it is connected already keeps the connection.
        a.control.as_mut().unwrap().mdi = "G0 X1".into();
        let _ = a.update(Message::MachineConnection(true));
        assert_eq!(conn(&a).mdi, "G0 X1");
        shows(
            &a,
            &["Real (USB) disconnects the simulator and connects to the machine itself."],
        );
    }

    #[test]
    fn connect_reports_a_missing_profile_and_needs_a_uc100_machine() {
        let dir = tempfile::tempdir().unwrap();
        let mut empty = App::from_workspace(dir.path().join("empty.json"), None);
        let _ = empty.update(Message::Connect);
        assert!(empty.control.is_none() && empty.status.contains("UC100 machine"));
        // Esc without a connection does nothing.
        let _ = empty.update(Message::MachineEStop);
        assert!(empty.control.is_none());
        let mut a = uc100_app(dir.path());
        std::fs::remove_file(dir.path().join("uc100.toml")).unwrap();
        let _ = a.update(Message::Connect);
        assert!(a.control.is_none());
        assert!(a.status.contains("cannot read"), "{}", a.status);
    }

    #[test]
    fn closing_the_app_disconnects_the_machine() {
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = uc100_app(dir.path());
        let _ = a.update(Message::Connect);
        tick_until(&mut a, 3.0, "E-stop after connecting", |s| {
            s.state == Some(State::EStop)
        });
        let _ = a.request_close(iced::window::Id::unique());
        assert!(a.control.is_none());
    }

    #[test]
    fn the_control_page_renders_in_every_state() {
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut empty = App::from_workspace(dir.path().join("empty.json"), None);
        empty.page = Page::Control;
        let _view = empty.view();
        let mut a = uc100_app(dir.path());
        a.page = Page::Control;
        let _ = a.view();
        let _ = a.update(Message::Connect);
        let _ = a.view();
        tick_until(&mut a, 3.0, "E-stop after connecting", |s| {
            s.state == Some(State::EStop)
        });
        let _ = a.view();
        let _ = a.update(Message::MachineReset);
        tick_until(&mut a, 3.0, "Idle after Reset", |s| {
            s.state == Some(State::Idle)
        });
        for page in [Page::Home, Page::Machines, Page::Control] {
            a.page = page;
            let _ = a.view();
        }
        // The connected machine's UC100 fields are locked in the wizard.
        let id = a.doc.machines[0].id.clone();
        let _ = a.update(Message::EditMachine(id));
        let _ = a.update(Message::WizardStep(3));
        let _ = a.view();
        let _ = a.update(Message::Disconnect);
        // A JY5800-like machine before and after machine zero, and one with switches.
        for profile in [LIMITED, SWITCHED] {
            let dir = tempfile::tempdir().unwrap();
            let mut a = profile_app(dir.path(), profile);
            a.page = Page::Control;
            idle(&mut a);
            let _ = a.view();
            // An axis with a home switch is referenced by homing only.
            let zero = if profile == SWITCHED {
                crate::control::Action::Home
            } else {
                crate::control::Action::MachineZero(None)
            };
            let _ = a.update(Message::Machine(zero));
            // Homing marks its last axis referenced two driver loops before
            // it is back in Idle, and a jog pressed while the driver still
            // reports Homing is rightly dropped (#9).
            tick_until(&mut a, 10.0, "X, Y and Z set, Idle", |s| {
                s.state == Some(State::Idle) && s.referenced[..3].iter().all(|r| *r)
            });
            let _ = a.update(Message::Machine(crate::control::Action::Rpm("abc".into())));
            let _ = a.update(Message::Machine(crate::control::Action::MdiText(
                "G0 X1".into(),
            )));
            let _ = a.view();
            let _ = a.update(Message::Machine(crate::control::Action::JogPress(1, false)));
            tick_until(&mut a, 3.0, "jogging", |s| {
                s.state == Some(uc100::controller::State::Jog)
            });
            let _ = a.view();
            let _ = a.update(Message::Machine(crate::control::Action::JogRelease));
        }
    }

    /// The operation editor's Delete asks first, as a setup's does: Cancel
    /// keeps the operation, another selection closes the question, and
    /// Delete operation removes it. The question shows in place of the
    /// editor's commands, and the footer knows the page its status is from.
    #[test]
    fn deleting_an_operation_asks_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("w.json"), None);
        let _ = a.update(Message::OpenSample);
        let _ = a.update(Message::SelectOp(0));
        let count = a.doc.setup().operations.len();
        let _ = a.update(Message::AskRemoveOp);
        assert!(a.confirm_delete_op);
        let _ = a.view();
        let _ = a.update(Message::ConfirmDeleteOp(false));
        assert!(!a.confirm_delete_op);
        assert_eq!(a.doc.setup().operations.len(), count);
        let _ = a.update(Message::AskRemoveOp);
        let _ = a.update(Message::SelectOp(1));
        assert!(
            !a.confirm_delete_op,
            "another operation closes the question"
        );
        let _ = a.update(Message::SelectOp(0));
        let _ = a.update(Message::AskRemoveOp);
        let _ = a.update(Message::ConfirmDeleteOp(true));
        assert!(!a.confirm_delete_op);
        assert_eq!(a.doc.setup().operations.len(), count - 1);
        // A status belongs to the page it was set on: one that a page change
        // leaves as it was is an earlier page's.
        let _ = a.update(Message::Page(Page::Work));
        assert_eq!(a.status_page, Page::Work);
        let before = a.status.clone();
        let _ = a.update(Message::Page(Page::Tools));
        let expected = if a.status == before {
            Page::Work
        } else {
            Page::Tools
        };
        assert_eq!(a.status_page, expected);
        let _ = a.view();
    }

    #[test]
    fn the_machine_workspace_goes_back_to_the_manufacture_page_left() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("w.json"), None);
        let _ = a.update(Message::Page(Page::Tools));
        let _ = a.update(Message::Page(Page::Control));
        let _ = a.update(Message::Page(Page::Control));
        assert_eq!((a.page, a.cam_page), (Page::Control, Page::Tools));
        let _ = a.view();
    }

    /// NC review hands a UC100 setup on to MACHINE, picked in the Run panel
    /// once Connect opens the machine (LIO-142).
    #[test]
    fn nc_review_runs_a_uc100_setup_on_the_machine() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        let refused = crate::post::PostSettings::from_machine(&a.doc.setups[0].machine);
        assert!(
            refused
                .as_ref()
                .is_err_and(|e| e[0].starts_with("G-code export is LinuxCNC only")
                    && e[0].contains("from the MACHINE workspace")),
            "{refused:?}"
        );
        let _ = a.update(Message::SelectSetup(1));
        let _ = a.update(Message::Page(Page::Post));
        a.post = Some(std::sync::Arc::new(crate::post::JobPost {
            errors: vec![],
            setups: vec![crate::post::SetupPost {
                setup: 0,
                name: a.doc.setups[0].name().into(),
                result: Err(refused.unwrap_err()),
                run: Some(Ok(())),
            }],
        }));
        let _ = a.view();
        let _ = a.update(Message::RunOnMachine(0));
        assert_eq!((a.page, a.cam_page), (Page::Control, Page::Post));
        assert!(
            a.control.is_none(),
            "Run on the machine never connects by itself"
        );
        let _ = a.update(Message::Connect);
        assert_eq!(run_of(&a).setup, 0);
        assert_eq!(a.doc.active, a.doc.setups[0].id);
        let _ = a.view();
    }

    /// The NC page words itself for the controller: a UC100 setup is checked
    /// for a run on the machine, with its prerequisites and the Run button
    /// and no error for the export file it does not have; a LinuxCNC setup
    /// keeps the export page (#79).
    #[test]
    fn the_nc_page_is_worded_for_the_machines_controller() {
        use crate::ui::{NO_EXPORT_FILE, NcCard, NcMachine, RUN_PREREQUISITES, nc_page};
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app(dir.path());
        let mesh = a.part_mesh().cloned().unwrap();
        a.post = Some(std::sync::Arc::new(crate::post::post_job(&a.doc, &mesh)));
        let page = nc_page(&a);
        assert_eq!(page.machine, NcMachine::Uc100);
        assert_eq!(page.title, "Run on the machine");
        assert!(page.intro.contains("run on the UC100") && page.intro.contains("no G-code file"));
        assert!(page.note.contains("takes no G-code file"));
        assert!(!page.ready, "a UC100 job has no export");
        // The run checks pass: the Run button, no error text.
        assert!(
            page.problems().is_empty(),
            "no error shown for a passing UC100 setup: {:?}",
            page.problems()
        );
        assert_eq!(page.run_buttons(), vec![0]);
        assert!(matches!(page.cards[0].1, NcCard::Run { .. }));
        assert!(
            RUN_PREREQUISITES.starts_with("To run:") && NO_EXPORT_FILE.contains("no G-code file")
        );
        // The export check still refuses it, as before; only the page words it.
        assert!(
            a.post.as_ref().unwrap().setups[0]
                .result
                .as_ref()
                .is_err_and(|e| e[0].starts_with("G-code export is LinuxCNC only"))
        );
        let _ = a.view();
        // A run check that fails is listed, and Run on the machine stays.
        a.doc.setups[0].operations[0]
            .p
            .insert("rpm".into(), "99999999".into());
        a.post = Some(std::sync::Arc::new(crate::post::post_job(&a.doc, &mesh)));
        let failed = nc_page(&a);
        assert!(!failed.problems().is_empty());
        assert_eq!(failed.run_buttons(), vec![0]);
        let _ = a.view();

        // A LinuxCNC setup keeps the export page.
        let other = tempfile::tempdir().unwrap();
        let mut b = App::from_workspace(other.path().join("workspace.json"), None);
        let (doc, mesh) = crate::post::fixture::job();
        b.replace_document(doc, None);
        b.imported_mesh = Some(mesh);
        b.smoke_dir = Some(other.path().to_path_buf());
        let _ = b.update(Message::Page(Page::Post));
        let page = nc_page(&b);
        assert_eq!(page.machine, NcMachine::LinuxCnc);
        assert_eq!(page.title, "NC review");
        assert_eq!(page.intro, "Check each setup, then export LinuxCNC G-code.");
        assert!(page.note.starts_with("LinuxCNC 3-axis output"));
        assert!(page.ready && page.run_buttons().is_empty() && page.problems().is_empty());
        assert!(matches!(page.cards[0].1, NcCard::Programs(_)));
        let _ = b.view();
        // Failing its export checks, a LinuxCNC setup is an error as before.
        b.doc.setups[0].p.insert("offset".into(), "G92".into());
        b.post = Some(std::sync::Arc::new(crate::post::JobPost {
            errors: vec![],
            setups: vec![crate::post::SetupPost {
                setup: 0,
                name: "Setup".into(),
                result: Err(vec![
                    "Work offset \"G92\" is not a LinuxCNC register.".into(),
                ]),
                run: None,
            }],
        }));
        let refused = nc_page(&b);
        assert_eq!(refused.problems().len(), 1);
        assert!(refused.run_buttons().is_empty());
        let _ = b.view();
    }
    /// The NC page lists a manual mill's setup with the line that says it has
    /// no G-code, and exports the CNC setup's program beside it.
    #[test]
    fn the_nc_page_says_a_manual_setup_has_no_gcode() {
        use crate::ui::{NcCard, nc_page};
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        let (mut doc, mesh) = crate::post::fixture::job();
        crate::post::fixture::with_manual_setup(&mut doc, [-500., 34., -500., 34., 0., -2.]);
        a.replace_document(doc, None);
        a.imported_mesh = Some(mesh);
        a.smoke_dir = Some(dir.path().to_path_buf());
        let _ = a.update(Message::Page(Page::Post));
        let page = nc_page(&a);
        assert!(page.ready && page.problems().is_empty());
        assert!(matches!(page.cards[0].1, NcCard::Manual));
        assert!(matches!(page.cards[1].1, NcCard::Programs(_)));
        let line = "Big mill: manual mill, no G-code — follow the steps on the MACHINE page.";
        assert_eq!(crate::manual::no_gcode("Big mill"), line);
        assert!(page_texts(&a).iter().any(|t| t == line));
    }
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
            Page::Control,
            Page::Tools,
            Page::Materials,
            Page::Workholding,
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
        assert_eq!(doc.materials.len(), 22);
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
    fn a_workspace_migrated_on_start_is_saved_once_and_its_note_does_not_repeat() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.json");
        let legacy = Document {
            material: "Inconel 718".into(),
            ..Document::default()
        };
        crate::storage::save(&path, &legacy).unwrap();
        let note = "The project material \"Inconel 718\" was added to the material library without cutting data; add it on the Materials page.";
        let mut a = App::from_workspace(path.clone(), None);
        assert!(a.status.contains(note), "{}", a.status);
        assert!(a.dirty, "the migrated workspace is to be saved");
        let id = a.doc.material_id.clone();
        assert_eq!(a.doc.selected_material().unwrap().name(), "Inconel 718");
        // The autosave writes it once the edit delay has passed.
        let _ = a.update(Message::Tick);
        assert!(a.dirty, "not before the delay");
        a.last_edit = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let _ = a.update(Message::Tick);
        assert!(!a.dirty);
        // The next start has nothing to say, keeps the material's id and
        // does not write the workspace again.
        let b = App::from_workspace(path.clone(), None);
        assert!(!b.status.contains("material library"), "{}", b.status);
        assert_eq!(
            b.status,
            "Choose New project, Open, or Continue your previous project."
        );
        assert_eq!((b.doc.material_id, b.doc.materials.len()), (id, 23));
        assert!(!b.dirty);
        // A workspace that needed no migration starts clean, and so does a blank one.
        let c = App::from_workspace(path, None);
        assert!(!c.dirty);
        let other = tempfile::tempdir().unwrap();
        assert!(!fresh_app(&other).dirty);
        // Tools numbered on load are saved too.
        let mut unnumbered = Document::sample();
        for tool in &mut unnumbered.tools {
            tool.p.remove("number");
        }
        let path = other.path().join("numbered").join("workspace.json");
        crate::storage::save(&path, &unnumbered).unwrap();
        let d = App::from_workspace(path, None);
        assert!(d.status.contains("numbered T1"), "{}", d.status);
        assert!(d.dirty);
    }
    #[test]
    fn an_unreadable_workspace_is_set_aside_and_the_recovery_workspace_is_resumed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.json");
        let broken = b"{ not a project";
        std::fs::write(&path, broken).unwrap();
        let mut a = App::from_workspace(path.clone(), None);
        a.doc.title = "Recovered edits".into();
        a.dirty = true;
        a.persist();
        assert!(!a.dirty, "{}", a.status);
        // The unreadable file is kept, aside, and the status says where.
        let aside: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("workspace-unreadable-")
            })
            .collect();
        assert_eq!(aside.len(), 1, "{aside:?}");
        assert_eq!(std::fs::read(&aside[0]).unwrap(), broken);
        assert!(
            a.status.contains(&aside[0].display().to_string()),
            "{}",
            a.status
        );
        // The next launch resumes what the first one saved, with no new file.
        let b = App::from_workspace(path.clone(), None);
        assert_eq!(b.doc.title, "Recovered edits", "{}", b.status);
        assert_eq!(b.state_path, path);
        assert!(!b.status.contains("unreadable"), "{}", b.status);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.iter().all(|n| !n.starts_with("workspace-recovery-")),
            "{names:?}"
        );
        assert_eq!(
            names
                .iter()
                .filter(|n| n.starts_with("workspace-unreadable-"))
                .count(),
            1,
            "{names:?}"
        );
    }
    #[test]
    fn new_project_and_explore_example_carry_materials_without_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        // A blank workspace starts with the starter materials.
        assert_eq!(a.doc.materials.len(), 22);
        let user = crate::material::Material::new("Titanium Gr5");
        assert!(user.issues().is_empty());
        let titanium = user.id.clone();
        a.doc.materials.push(user);
        let _ = a.update(Message::NewProject);
        assert_eq!(a.doc.materials.len(), 23);
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
        assert_eq!(names.len(), 23);
        assert_eq!(unique.len(), 23, "{names:?}");
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
        assert_eq!(a.doc.materials.len(), 23);
        let _ = a.update(Message::NewProject);
        assert_eq!(a.doc.materials.len(), 23);
        assert!(a.doc.material_id.is_empty());
        // An example opened from a library without the user's edits links
        // its material to the starter.
        let mut fresh = App::from_workspace(dir.path().join("other").join("workspace.json"), None);
        let _ = fresh.update(Message::OpenSample);
        assert_eq!(fresh.doc.materials.len(), 22);
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
    fn explore_matches_material_names_like_the_library_does() {
        use crate::material::Material;
        let user = Document {
            materials: vec![
                Material::new("Édelstahl 304"),
                Material::new(" édelstahl 304 "),
                Material::new(" aluminium 6061-t6 "),
            ],
            starter_materials: crate::material::STARTER_SET,
            ..Document::default()
        };
        let (doc, status) = crate::merge_sample(&user);
        // The second spelling is the first entry again; the third is the
        // starter's, so it is not announced as the user's own addition.
        let names: Vec<_> = doc.materials.iter().map(|m| m.name()).collect();
        assert_eq!(names, ["Édelstahl 304", " aluminium 6061-t6 "]);
        assert!(
            status.contains("Added from your library: material Édelstahl 304. Use New project"),
            "{status}"
        );
        assert!(status.contains("linked to the library"), "{status}");
    }
    #[test]
    fn explore_example_adds_its_material_without_data_when_the_user_deleted_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        a.doc.materials.retain(|m| m.name() != "Aluminium 6061-T6");
        assert_eq!(a.doc.materials.len(), 21);
        let _ = a.update(Message::OpenSample);
        // The deleted starter is not brought back with its data: the example's
        // material is added to the library without cutting data, and says so.
        assert_eq!(a.doc.materials.len(), 22);
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
        assert_eq!(a.doc.materials.len(), 22);
        assert!(a.status.contains("linked to the library"), "{}", a.status);
    }
    #[test]
    fn a_deleted_new_alloy_does_not_come_back_in_a_new_project_or_the_example() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        a.doc.materials.retain(|m| m.name() != "Aluminium 5052-H32");
        let _ = a.update(Message::NewProject);
        assert_eq!(a.doc.materials.len(), 21);
        let _ = a.update(Message::OpenSample);
        assert_eq!(a.doc.materials.len(), 21);
        assert!(
            !a.doc
                .materials
                .iter()
                .any(|m| m.name() == "Aluminium 5052-H32")
        );
        assert_eq!(a.doc.starter_materials, crate::material::STARTER_SET);
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
    fn the_material_picker_selects_by_id_and_every_state_builds() {
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
        // What the new material did to the speeds and feeds comes first.
        assert!(
            a.status
                .starts_with("Speeds and feeds set for Aluminium 7075-T6: ")
                && a.status
                    .ends_with(" Planning stopped because the job changed."),
            "{}",
            a.status
        );
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
    fn picking_a_material_sets_every_operations_speed_and_feed_and_says_so() {
        use crate::material::SpeedFeed;
        let dir = tempfile::tempdir().unwrap();
        let mut a = planning_app(&dir);
        let pine = a
            .doc
            .materials
            .iter()
            .find(|m| m.name() == "Pine")
            .unwrap()
            .id
            .clone();
        let before: Vec<_> = a.doc.setups[0]
            .operations
            .iter()
            .map(|o| get(&o.p, "rpm").to_string())
            .collect();
        let _ = a.update(Message::PickMaterial(pine.clone()));
        assert_eq!(a.doc.material, "Pine");
        assert!(a.dirty);
        assert!(
            a.status
                .starts_with("Speeds and feeds set for Pine: Top / "),
            "{}",
            a.status
        );
        // Every operation but the drilling now suits pine, and the line names
        // each one changed. The drilling keeps its own, and the line says so.
        assert!(
            a.status.ends_with(" A drilling operation keeps its speed and feed outside the data: set a drill's by hand, the library has no drill data."),
            "{}",
            a.status
        );
        for setup in &a.doc.setups {
            for (o, old) in setup.operations.iter().zip(&before) {
                if o.strategy == crate::model::Strategy::Drill {
                    assert_eq!(get(&o.p, "rpm"), old);
                    continue;
                }
                assert!(
                    matches!(a.doc.speed_feed(setup, o), Some(SpeedFeed::InRange { .. })),
                    "{}: {:?}",
                    o.name(),
                    a.doc.speed_feed(setup, o)
                );
                let rpm = get(&o.p, "rpm");
                if rpm != old {
                    assert!(
                        a.status.contains(&format!(
                            "{} / {} {old} → {rpm} rpm",
                            setup.name(),
                            o.name()
                        )),
                        "{}",
                        a.status
                    );
                }
            }
        }
        // Picking it again is no change and says nothing new.
        a.status = "unchanged".into();
        let _ = a.update(Message::PickMaterial(pine));
        assert_eq!(a.status, "unchanged");
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
    fn holders_named(doc: &Document) -> Vec<String> {
        doc.workholding
            .iter()
            .map(|h| h.name().to_string())
            .collect()
    }
    fn edit_holder(a: &mut App, pairs: &[(&'static str, &str)]) {
        for (key, value) in pairs {
            let _ = a.update(Message::Edit(Target::Holder, key, (*value).into()));
        }
    }
    fn edit_setup(a: &mut App, pairs: &[(&'static str, &str)]) {
        for (key, value) in pairs {
            let _ = a.update(Message::Edit(Target::Setup, key, (*value).into()));
        }
    }
    /// The example job and the starter library, in an app of its own.
    fn holding_app(dir: &tempfile::TempDir) -> App {
        let mut a = fresh_app(dir);
        a.replace_document(Document::sample(), None);
        a.doc.adopt_holders();
        a
    }
    fn first_of(a: &App, kind: &str) -> String {
        a.doc
            .workholding
            .iter()
            .find(|h| h.kind() == kind)
            .unwrap()
            .id
            .clone()
    }
    #[test]
    fn the_workholding_library_starts_with_the_starters_and_keeps_a_users_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.json");
        let mut a = App::from_workspace(path.clone(), None);
        assert_eq!(
            holders_named(&a.doc),
            ["100 mm machine vise", "Toe clamp 60 mm"]
        );
        assert_eq!(a.holder_draft.id, a.doc.workholding[0].id);
        assert!(a.doc.validate_structure().is_ok());
        // Edit the vise, delete the clamp, and save the workspace.
        let vise = first_of(&a, "Vise");
        let clamp = first_of(&a, "Clamp");
        let _ = a.update(Message::EditHolder(vise.clone()));
        edit_holder(&mut a, &[("jaw_width", "125")]);
        let _ = a.update(Message::SaveHolder);
        let _ = a.update(Message::DeleteHolder(clamp));
        assert_eq!(holders_named(&a.doc), ["100 mm machine vise"]);
        a.dirty = true;
        a.persist();
        // A library with entries gets no starters when it is opened again.
        let reopened = App::from_workspace(path, None);
        assert_eq!(holders_named(&reopened.doc), ["100 mm machine vise"]);
        assert_eq!(reopened.doc.workholding[0].id, vise);
        assert_eq!(get(&reopened.doc.workholding[0].p, "jaw_width"), "125");
    }
    #[test]
    fn new_project_carries_the_workholding_library() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        let mut mine = crate::holding::Holder::new("Clamp", "Strap clamp");
        for (k, v) in [
            ("length", "80"),
            ("width", "25"),
            ("height", "30"),
            ("bite", "20"),
        ] {
            mine.p.insert(k.into(), v.into());
        }
        a.doc.workholding.push(mine.clone());
        let ids: Vec<_> = a.doc.workholding.iter().map(|h| h.id.clone()).collect();
        let _ = a.update(Message::NewProject);
        assert!(a.doc.setups.is_empty());
        assert_eq!(
            a.doc
                .workholding
                .iter()
                .map(|h| h.id.clone())
                .collect::<Vec<_>>(),
            ids
        );
        assert_eq!(a.holder_draft.id, ids[0]);
        // An empty library is given the starters again.
        a.doc.workholding.clear();
        let _ = a.update(Message::NewProject);
        assert_eq!(a.doc.workholding.len(), 2);
        assert!(
            a.status.contains("materials and workholding are available"),
            "{}",
            a.status
        );
    }
    #[test]
    fn explore_merges_workholding_without_duplicate_names() {
        use crate::holding::Holder;
        let user = Document {
            workholding: vec![
                Holder::new("Vise", " 100 MM machine VISE "),
                Holder::new("Clamp", "Strap clamp"),
                Holder::new("Clamp", " strap CLAMP"),
            ],
            ..Document::default()
        };
        let (doc, status) = crate::merge_sample(&user);
        // The user's library is kept as it is, without starters next to it; the
        // second spelling is the first entry again, and the starter's name is
        // not announced as an addition of the user's.
        assert_eq!(
            holders_named(&doc),
            [" 100 MM machine VISE ", "Strap clamp"]
        );
        assert!(status.contains("workholding entry Strap clamp"), "{status}");
        assert!(!status.contains("VISE"), "{status}");
        assert!(doc.validate_structure().is_ok());
        // A user library of the starters adds nothing and says nothing.
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        let ids: Vec<_> = a.doc.workholding.iter().map(|h| h.id.clone()).collect();
        let _ = a.update(Message::OpenSample);
        assert_eq!(
            a.doc
                .workholding
                .iter()
                .map(|h| h.id.clone())
                .collect::<Vec<_>>(),
            ids
        );
        assert!(!a.status.contains("workholding"), "{}", a.status);
        // Several of the user's own are listed, and an empty library gets the starters.
        let several = Document {
            workholding: vec![Holder::new("Vise", "Kurt"), Holder::new("Clamp", "Strap")],
            ..Document::default()
        };
        let (_, status) = crate::merge_sample(&several);
        assert!(
            status.contains("workholding entries Kurt, Strap"),
            "{status}"
        );
        let (doc, _) = crate::merge_sample(&Document::default());
        assert_eq!(holders_named(&doc).len(), 2);
    }
    #[test]
    fn holders_are_created_edited_saved_refused_and_deleted_like_materials() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        let before = a.doc.workholding.len();
        let _ = a.update(Message::NewHolder("Vise"));
        assert_eq!(a.page, Page::Workholding);
        assert_eq!(
            (a.holder_draft.kind(), a.holder_draft.name()),
            ("Vise", "New vise")
        );
        // Nothing measured yet: refused with the entry's issues.
        let _ = a.update(Message::SaveHolder);
        assert_eq!(a.doc.workholding.len(), before);
        assert!(
            a.status.contains("jaw width must be a positive number."),
            "{}",
            a.status
        );
        edit_holder(
            &mut a,
            &[
                ("name", "  Kurt 150  "),
                ("jaw_width", "150"),
                ("jaw_height", "40"),
                ("jaw_thickness", "25"),
                ("max_opening", "180"),
                ("grip", "10"),
            ],
        );
        a.project_dirty = false;
        let _ = a.update(Message::SaveHolder);
        assert_eq!(a.status, "Workholding saved to the library.");
        assert!(a.project_dirty);
        assert_eq!(a.doc.workholding.len(), before + 1);
        let kurt = a.doc.workholding.last().unwrap().clone();
        assert_eq!(kurt.name(), "Kurt 150");
        assert_eq!(get(&kurt.p, "max_opening"), "180");
        // The same name in another case is refused.
        let _ = a.update(Message::NewHolder("Vise"));
        edit_holder(
            &mut a,
            &[
                ("name", "kurt 150"),
                ("jaw_width", "100"),
                ("jaw_height", "30"),
                ("jaw_thickness", "20"),
                ("max_opening", "120"),
                ("grip", "8"),
            ],
        );
        let _ = a.update(Message::SaveHolder);
        assert!(a.status.contains("already in the library"), "{}", a.status);
        assert_eq!(a.doc.workholding.len(), before + 1);
        // A clamp whose bite is not shorter than its length is refused.
        let _ = a.update(Message::NewHolder("Clamp"));
        assert_eq!(
            (a.holder_draft.kind(), a.holder_draft.name()),
            ("Clamp", "New clamp")
        );
        edit_holder(
            &mut a,
            &[
                ("name", "Strap 80"),
                ("length", "80"),
                ("width", "25"),
                ("height", "30"),
                ("bite", "80"),
            ],
        );
        let _ = a.update(Message::SaveHolder);
        assert!(
            a.status.contains("bite must be shorter than its length"),
            "{}",
            a.status
        );
        assert_eq!(a.doc.workholding.len(), before + 1);
        edit_holder(&mut a, &[("bite", "20")]);
        let _ = a.update(Message::SaveHolder);
        assert_eq!(a.doc.workholding.len(), before + 2);
        // Editing an entry replaces it, keeping its identifier.
        let _ = a.update(Message::EditHolder(kurt.id.clone()));
        assert_eq!(a.holder_draft, kurt);
        edit_holder(&mut a, &[("grip", "12")]);
        let _ = a.update(Message::SaveHolder);
        assert_eq!(a.doc.workholding.len(), before + 2);
        let edited = a.doc.workholding.iter().find(|h| h.id == kurt.id).unwrap();
        assert_eq!(get(&edited.p, "grip"), "12");
        assert!(a.doc.validate_structure().is_ok());
        // Deleting puts the editor on the first entry; a draft that was never
        // saved is only discarded.
        let _ = a.update(Message::DeleteHolder(kurt.id.clone()));
        assert!(!a.doc.workholding.iter().any(|h| h.id == kurt.id));
        assert!(a.status.contains("deleted"), "{}", a.status);
        assert_eq!(a.holder_draft.id, a.doc.workholding[0].id);
        let count = a.doc.workholding.len();
        let _ = a.update(Message::NewHolder("Clamp"));
        let draft = a.holder_draft.id.clone();
        let _ = a.update(Message::DeleteHolder(draft));
        assert_eq!(a.doc.workholding.len(), count);
        assert!(a.status.contains("discarded"), "{}", a.status);
        assert_eq!(a.holder_draft.id, a.doc.workholding[0].id);
        a.page = Page::Workholding;
        let _ = a.view();
    }
    #[test]
    fn a_holder_a_setup_uses_cannot_be_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        let (vise, clamp) = (first_of(&a, "Vise"), first_of(&a, "Clamp"));
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Clamps".into()));
        let _ = a.update(Message::DuplicateSetup);
        edit_setup(&mut a, &[("name", "Flipped side")]);
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        assert_eq!(a.doc.setups.len(), 2);
        // Each is refused, naming the entry and the setup that uses it.
        let _ = a.update(Message::DeleteHolder(vise.clone()));
        assert!(a.doc.workholding.iter().any(|h| h.id == vise));
        assert!(a.status.contains("100 mm machine vise"), "{}", a.status);
        assert!(a.status.contains("Flipped side"), "{}", a.status);
        let _ = a.update(Message::DeleteHolder(clamp.clone()));
        assert!(a.doc.workholding.iter().any(|h| h.id == clamp));
        assert!(a.status.contains("Toe clamp 60 mm"), "{}", a.status);
        assert!(a.status.contains("Top side"), "{}", a.status);
        // The first setup that uses it is named when two do.
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Clamps".into()));
        let _ = a.update(Message::DeleteHolder(clamp.clone()));
        assert!(a.status.contains("Top side"), "{}", a.status);
        assert!(!a.status.contains("Flipped side"), "{}", a.status);
        // A setup that no longer models workholding does not hold the entry.
        let _ = a.update(Message::Edit(Target::Setup, "holding", "None".into()));
        let _ = a.update(Message::SelectSetup(0));
        let _ = a.update(Message::Edit(Target::Setup, "holding", "None".into()));
        let _ = a.update(Message::DeleteHolder(clamp.clone()));
        assert!(!a.doc.workholding.iter().any(|h| h.id == clamp));
        assert!(a.status.contains("deleted"), "{}", a.status);
    }
    #[test]
    fn choosing_a_vise_writes_the_setup_keys_and_the_layout_follows() {
        use crate::holding::{Layout, layout};
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        let vise = first_of(&a, "Vise");
        assert_eq!(layout(a.doc.setup()), Ok(Layout::None));
        a.project_dirty = false;
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        assert!(a.project_dirty);
        let p = &a.doc.setup().p;
        assert_eq!(get(p, "holding"), "Vise");
        assert_eq!(get(p, "holder"), vise);
        assert_eq!(get(p, "vise_grip"), "8");
        assert_eq!(get(p, "holding_gap"), "2");
        // The first Vise also places the jaws (the example's stock is 150 x 104 mm: only the Y sides fit).
        assert_eq!(
            layout(a.doc.setup()),
            Ok(Layout::Vise {
                holder: vise.clone(),
                axis_y: true,
                offset: 0.,
                grip: 8.,
                gap: 2.
            })
        );
        edit_setup(&mut a, &[("vise_axis", "Y"), ("vise_offset", "5")]);
        assert_eq!(
            layout(a.doc.setup()),
            Ok(Layout::Vise {
                holder: vise.clone(),
                axis_y: true,
                offset: 5.,
                grip: 8.,
                gap: 2.
            })
        );
        // Another vise from the library: its id is written and the jaws are
        // placed again from its own grip (nothing in the example reaches them).
        let mut other = crate::holding::starters().remove(0);
        other.p.insert("name".into(), "Kurt".into());
        other.p.insert("grip".into(), "15".into());
        let other_id = other.id.clone();
        a.doc.workholding.push(other);
        let _ = a.update(Message::Edit(Target::Setup, "holder", other_id.clone()));
        assert_eq!(get(&a.doc.setup().p, "holder"), other_id);
        assert_eq!(get(&a.doc.setup().p, "vise_grip"), "15");
        assert_eq!(get(&a.doc.setup().p, "vise_offset"), "0");
        // Choosing a vise again places the jaws from its own grip, whatever grip the setup had (settle_holding's
        // default for an empty grip has its own model test).
        edit_setup(&mut a, &[("vise_grip", "")]);
        let _ = a.update(Message::Edit(Target::Setup, "holder", vise.clone()));
        assert_eq!(get(&a.doc.setup().p, "vise_grip"), "8");
        let _ = a.update(Message::Edit(Target::Setup, "vise_grip", String::new()));
        let _ = a.update(Message::Edit(Target::Setup, "holder", other_id.clone()));
        assert_eq!(get(&a.doc.setup().p, "vise_grip"), "15");
        // None keeps the entry but models nothing; Clamps picks a clamp.
        let _ = a.update(Message::Edit(Target::Setup, "holding", "None".into()));
        assert_eq!(layout(a.doc.setup()), Ok(Layout::None));
        assert!(a.doc.holding_issues(0, None).is_empty());
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Clamps".into()));
        assert_eq!(get(&a.doc.setup().p, "holder"), first_of(&a, "Clamp"));
        // The first Clamps places them; without positions the setup says so.
        assert_eq!(get(&a.doc.setup().p, "clamps"), "-X:0;+X:0;-Y:0;+Y:0");
        assert!(a.doc.holding_issues(0, None).is_empty());
        a.doc.setup_mut().p.insert("clamps".into(), String::new());
        assert_eq!(
            a.doc.holding_issues(0, None),
            ["Place 1 to 8 clamps."],
            "the positions are still to be written"
        );
    }
    #[test]
    fn picking_the_chosen_holder_again_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        // A setup with no value shows None and X sides: picking those is no change.
        a.project_dirty = false;
        a.status = "Kept.".into();
        let _ = a.update(Message::Edit(Target::Setup, "holding", "None".into()));
        let _ = a.update(Message::Edit(Target::Setup, "vise_axis", "X".into()));
        assert!(!a.project_dirty);
        assert_eq!(a.status, "Kept.");
        assert!(!a.doc.setup().p.contains_key("holding"));
        assert!(!a.doc.setup().p.contains_key("vise_axis"));
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        let _ = a.update(Message::Edit(Target::Setup, "vise_axis", "Y".into()));
        let vise = first_of(&a, "Vise");
        a.project_dirty = false;
        a.status = "Kept.".into();
        for (key, value) in [
            ("holding", "Vise"),
            ("holder", vise.as_str()),
            ("vise_axis", "Y"),
        ] {
            let _ = a.update(Message::Edit(Target::Setup, key, value.into()));
        }
        assert!(!a.project_dirty);
        assert_eq!(a.status, "Kept.");
        let _ = a.update(Message::Edit(Target::Setup, "vise_axis", "X".into()));
        assert!(a.project_dirty);
    }
    #[test]
    fn clamp_positions_are_written_only_when_they_read_correctly() {
        use crate::holding::{Layout, Side, layout};
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        let clamp = first_of(&a, "Clamp");
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Clamps".into()));
        // The first Clamps also places them: all four, at the middles.
        assert_eq!(a.clamps_field(), ("-X:0;+X:0;-Y:0;+Y:0".into(), None));
        a.project_dirty = false;
        let _ = a.update(Message::SetClamps("-X:0;+X:0".into()));
        assert!(a.project_dirty);
        assert_eq!(get(&a.doc.setup().p, "clamps"), "-X:0;+X:0");
        assert_eq!(
            layout(a.doc.setup()),
            Ok(Layout::Clamps {
                holder: clamp,
                clamps: vec![(Side::MinusX, 0.), (Side::PlusX, 0.)],
                gap: 2.
            })
        );
        assert_eq!(a.clamps_field(), ("-X:0;+X:0".into(), None));
        assert!(a.doc.holding_issues(0, None).is_empty());
        // Text that does not read: the setup keeps what it had, the field keeps
        // what was typed, and the error is in the status and under the field.
        a.project_dirty = false;
        let _ = a.update(Message::SetClamps("-X:0;+Q:3".into()));
        assert!(!a.project_dirty);
        assert_eq!(get(&a.doc.setup().p, "clamps"), "-X:0;+X:0");
        let error = "A clamp side must be -X, +X, -Y or +Y.";
        assert_eq!(a.status, error);
        assert_eq!(a.clamps_field(), ("-X:0;+Q:3".into(), Some(error.into())));
        for (text, error) in [
            ("-X", "Write each clamp as side:offset, for example -X:0."),
            ("-X:", "A clamp offset must be a number."),
            ("", "Place 1 to 8 clamps."),
        ] {
            let _ = a.update(Message::SetClamps(text.into()));
            assert_eq!(a.status, error);
            assert_eq!(get(&a.doc.setup().p, "clamps"), "-X:0;+X:0");
            assert_eq!(a.clamps_field(), (text.into(), Some(error.into())));
        }
        // Another setup shows its own text. Only one draft is kept: this
        // setup's is back on return unless a text that does not read was typed
        // in another setup meanwhile, which takes its place.
        let _ = a.update(Message::SetClamps("-X:".into()));
        let _ = a.update(Message::DuplicateSetup);
        assert_eq!(a.clamps_field(), ("-X:0;+X:0".into(), None));
        let _ = a.update(Message::SelectSetup(0));
        assert_eq!(a.clamps_field().0, "-X:");
        let _ = a.update(Message::SelectSetup(1));
        let _ = a.update(Message::SetClamps("-Q".into()));
        assert_eq!(a.clamps_field().0, "-Q");
        let _ = a.update(Message::SelectSetup(0));
        assert_eq!(a.clamps_field(), ("-X:0;+X:0".into(), None));
        let _ = a.update(Message::SetClamps("-X:".into()));
        // A valid text clears the draft and is kept as typed; a valid text from
        // another writer drops a draft typed over the old one.
        let _ = a.update(Message::SetClamps("+Y: 4".into()));
        assert_eq!(a.status, "Clamp positions saved with the setup.");
        assert_eq!(a.clamps_field(), ("+Y: 4".into(), None));
        let _ = a.update(Message::SetClamps("+Y".into()));
        a.doc.setup_mut().p.insert("clamps".into(), "-Y:1".into());
        assert_eq!(a.clamps_field(), ("-Y:1".into(), None));
        // The text is read as clamp positions whatever else the setup says: a
        // bad gap does not refuse it.
        edit_setup(&mut a, &[("holding_gap", "abc")]);
        let _ = a.update(Message::SetClamps("-X:0;+X:5".into()));
        assert_eq!(get(&a.doc.setup().p, "clamps"), "-X:0;+X:5");
        assert_eq!(
            a.doc.holding_issues(0, None),
            ["The clearance gap must be 0 or more."]
        );
    }
    #[test]
    fn the_setup_header_follows_the_modelled_holding_and_clamps_are_edited_by_row() {
        use crate::holding::{clamp_rows, rows_text};
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        a.page = Page::Work;
        a.selected = None;
        let _ = a.update(Message::SetupStep("Holding"));
        assert!(a.operation().is_none(), "the setup editor is shown");
        let label = |a: &App| a.doc.holding_label(a.doc.setup());
        // A new setup claims nothing holds it; a fixture note is said to be
        // not modelled.
        assert_eq!(label(&a), "No workholding modelled");
        edit_setup(&mut a, &[("fixture", "Low profile clamps")]);
        assert_eq!(label(&a), "Low profile clamps, not modelled");
        let _ = a.view();
        // A modelled vise or clamps say what holds the stock, in place of the
        // note, in the header and on the setup sheet.
        edit_setup(&mut a, &[("holding", "Vise")]);
        assert_eq!(label(&a), "100 mm machine vise");
        assert!(!a.doc.setup_sheet(None).contains("Low profile clamps"));
        let _ = a.view();
        edit_setup(&mut a, &[("holding", "Clamps")]);
        assert_eq!(label(&a), "4 × Toe clamp 60 mm");
        // A row per clamp: a side and an offset changed, a clamp removed.
        let (typed, error) = a.clamps_field();
        assert_eq!(error, None);
        let mut rows = clamp_rows(&typed);
        assert_eq!(rows.len(), 4);
        rows[1] = ["-Y".into(), "12".into()];
        rows.remove(3);
        let _ = a.update(Message::SetClamps(rows_text(&rows)));
        assert_eq!(get(&a.doc.setup().p, "clamps"), "-X:0;-Y:12;-Y:0");
        assert_eq!(label(&a), "3 × Toe clamp 60 mm");
        // An offset being typed stays in its row, unsaved, with what is wrong;
        // a separator typed into it never adds a clamp.
        rows[0][1] = "-".into();
        let _ = a.update(Message::SetClamps(rows_text(&rows)));
        let (typed, error) = a.clamps_field();
        assert_eq!(clamp_rows(&typed), rows);
        assert_eq!(error.as_deref(), Some("A clamp offset must be a number."));
        assert_eq!(get(&a.doc.setup().p, "clamps"), "-X:0;-Y:12;-Y:0");
        let _ = a.view();
        rows[0][1] = "5;+X:9".into();
        assert_eq!(clamp_rows(&rows_text(&rows)).len(), 3);
        // Back to None, the note is what the header says again.
        edit_setup(&mut a, &[("holding", "None")]);
        assert_eq!(label(&a), "Low profile clamps, not modelled");
        let _ = a.view();
    }
    /// The plans Suggest layout works from: those of setup `si` that generate.
    fn setup_plans(a: &App, si: usize) -> Vec<crate::simulation::Plan> {
        let mesh = a.imported_mesh.as_ref().unwrap();
        let last = a.doc.setups[si]
            .operations
            .iter()
            .rposition(|o| o.enabled)
            .unwrap();
        let mut plans = vec![];
        crate::simulation::sequence(&a.doc, mesh, Some((si, last)), |s, _, result| {
            if s == si
                && let Ok(sim) = result
            {
                plans.push(sim.plan);
            }
        });
        plans
    }
    const STOCK_ONLY: &str =
        "No toolpaths were available to check, so only the stock was considered.";
    #[test]
    fn suggest_layout_needs_a_setup_workholding_and_a_usable_entry() {
        let dir = tempfile::tempdir().unwrap();
        let mut blank = fresh_app(&dir);
        let _ = blank.update(Message::SuggestLayout);
        assert!(blank.status.contains("Create a setup"), "{}", blank.status);
        let mut a = holding_app(&dir);
        let refused = |a: &mut App, status: &str| {
            let before = a.doc.setup().p.clone();
            let _ = a.update(Message::SuggestLayout);
            assert_eq!(a.status, status);
            assert_eq!(a.doc.setup().p, before, "{status}");
        };
        refused(&mut a, "Choose a vise or clamps for this setup first.");
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        let vise = first_of(&a, "Vise");
        let clamp = first_of(&a, "Clamp");
        // The wrong kind of entry, a bad gap, an entry that is gone, none chosen.
        a.doc.setup_mut().p.insert("holder".into(), clamp);
        refused(
            &mut a,
            "The chosen workholding entry is a clamp, not a vise.",
        );
        a.doc.setup_mut().p.insert("holder".into(), vise.clone());
        a.doc
            .setup_mut()
            .p
            .insert("holding_gap".into(), "abc".into());
        refused(&mut a, "The clearance gap must be 0 or more.");
        a.doc.setup_mut().p.insert("holding_gap".into(), "2".into());
        a.doc.setup_mut().p.insert("holder".into(), String::new());
        refused(&mut a, "Choose a vise from the library first.");
        a.doc.setup_mut().p.insert("holder".into(), vise);
        a.doc.workholding.retain(|h| h.kind() != "Vise");
        refused(&mut a, "The workholding entry was deleted; choose another.");
    }
    #[test]
    fn choosing_a_vise_the_first_time_suggests_it_and_later_edits_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        a.project_dirty = false;
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        // The example's stock is 150 x 104 mm: only the Y sides fit the 120 mm opening. It has no toolpaths.
        let p = &a.doc.setup().p;
        assert_eq!(get(p, "vise_axis"), "Y");
        assert_eq!(get(p, "vise_offset"), "0");
        assert_eq!(get(p, "vise_grip"), "8");
        assert_eq!(
            a.status,
            format!("Suggested: vise jaws on ±Y, grip 8 mm. {STOCK_ONLY}")
        );
        assert!(a.project_dirty);
        // Edits to the offset, grip, gap and axis move nothing else and say nothing.
        a.status = "Kept.".into();
        edit_setup(
            &mut a,
            &[
                ("vise_offset", "5"),
                ("vise_grip", "6"),
                ("holding_gap", "3"),
                ("vise_axis", "X"),
            ],
        );
        let p = &a.doc.setup().p;
        assert_eq!(
            [
                get(p, "vise_axis"),
                get(p, "vise_offset"),
                get(p, "vise_grip"),
                get(p, "holding_gap")
            ],
            ["X", "5", "6", "3"]
        );
        assert_eq!(a.status, "Kept.");
        // None and Vise again: the setup has its placement, so it stays.
        let _ = a.update(Message::Edit(Target::Setup, "holding", "None".into()));
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        assert_eq!(get(&a.doc.setup().p, "vise_axis"), "X");
        assert_eq!(get(&a.doc.setup().p, "vise_offset"), "5");
        assert_eq!(a.status, "Kept.");
        // Suggest layout always suggests: offset 0, the holder's grip, the clearer axis.
        let _ = a.update(Message::SuggestLayout);
        let p = &a.doc.setup().p;
        assert_eq!(
            [
                get(p, "vise_axis"),
                get(p, "vise_offset"),
                get(p, "vise_grip")
            ],
            ["Y", "0", "8"]
        );
        assert_eq!(get(p, "holding_gap"), "3");
        assert!(
            a.status
                .starts_with("Suggested: vise jaws on ±Y, grip 8 mm.")
        );
        // Another library vise places the jaws again, from its own grip.
        let mut other = crate::holding::starters().remove(0);
        other.p.insert("name".into(), "Kurt".into());
        other.p.insert("grip".into(), "15".into());
        let other_id = other.id.clone();
        a.doc.workholding.push(other);
        edit_setup(&mut a, &[("vise_offset", "9")]);
        a.status = "Kept.".into();
        let _ = a.update(Message::Edit(Target::Setup, "holder", other_id.clone()));
        let p = &a.doc.setup().p;
        assert_eq!(get(p, "holder"), other_id);
        assert_eq!(
            [
                get(p, "vise_axis"),
                get(p, "vise_offset"),
                get(p, "vise_grip")
            ],
            ["Y", "0", "15"]
        );
        assert!(
            a.status
                .starts_with("Suggested: vise jaws on ±Y, grip 15 mm.")
        );
        // Choosing the same entry again changes nothing.
        edit_setup(&mut a, &[("vise_offset", "9")]);
        a.status = "Kept.".into();
        let _ = a.update(Message::Edit(Target::Setup, "holder", other_id));
        assert_eq!(get(&a.doc.setup().p, "vise_offset"), "9");
        assert_eq!(a.status, "Kept.");
    }
    #[test]
    fn choosing_clamps_the_first_time_suggests_them_and_the_typed_text_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Clamps".into()));
        assert_eq!(get(&a.doc.setup().p, "clamps"), "-X:0;+X:0;-Y:0;+Y:0");
        assert_eq!(
            a.status,
            format!("Suggested: 4 clamps on -X at 0, +X at 0, -Y at 0, +Y at 0. {STOCK_ONLY}")
        );
        assert!(a.doc.holding_issues(0, None).is_empty());
        // A text that does not read is dropped when the suggestion writes the positions.
        let _ = a.update(Message::SetClamps("-X".into()));
        assert!(a.clamps_draft.is_some());
        let _ = a.update(Message::SuggestLayout);
        assert!(a.clamps_draft.is_none());
        assert_eq!(a.clamps_field(), ("-X:0;+X:0;-Y:0;+Y:0".to_string(), None));
        // Typed positions are kept through None and Clamps and any other edit.
        let _ = a.update(Message::SetClamps("+Y:5".into()));
        a.status = "Kept.".into();
        edit_setup(&mut a, &[("holding_gap", "3")]);
        let _ = a.update(Message::Edit(Target::Setup, "holding", "None".into()));
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Clamps".into()));
        assert_eq!(get(&a.doc.setup().p, "clamps"), "+Y:5");
        assert_eq!(a.status, "Kept.");
        // A different clamp from the library suggests again.
        let mut other = crate::holding::starters().remove(1);
        other.p.insert("name".into(), "Strap".into());
        other.p.insert("width".into(), "30".into());
        let other_id = other.id.clone();
        a.doc.workholding.push(other);
        let _ = a.update(Message::Edit(Target::Setup, "holder", other_id.clone()));
        assert_eq!(get(&a.doc.setup().p, "holder"), other_id);
        assert_eq!(get(&a.doc.setup().p, "clamps"), "-X:0;+X:0;-Y:0;+Y:0");
        assert!(
            a.status.starts_with("Suggested: 4 clamps on "),
            "{}",
            a.status
        );
    }
    #[test]
    fn suggest_layout_reads_the_generated_toolpaths_and_changes_none() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = planning_app(&dir);
        let vise = first_of(&a, "Vise");
        let operations = json(&a.doc.setups[0].operations);
        let plans = setup_plans(&a, 0);
        assert_eq!(plans.len(), 6);
        let refs: Vec<&crate::simulation::Plan> = plans.iter().collect();
        // The example block's stock is 124 x 84 mm: only the Y sides fit the vise.
        let stock = plans[0].stock;
        let holder = a.doc.holder(&vise).unwrap().clone();
        let (axis_y, grip) = crate::holding::suggest_vise(stock, None, &holder, &refs, 2.);
        assert!(axis_y);
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        let p = &a.doc.setup().p;
        assert_eq!(get(p, "vise_axis"), "Y");
        assert_eq!(get(p, "vise_offset"), "0");
        assert_eq!(get(p, "vise_grip"), grip.to_string());
        // The contour and the drilling run at the jaws' height beside the stock, whatever the grip.
        let jaws = crate::holding::boxes(
            &crate::holding::layout(a.doc.setup()).unwrap(),
            &holder,
            stock,
        )
        .unwrap();
        let clashing = crate::holding::clashes(&refs, &jaws, 2.);
        assert!(clashing > 0);
        assert_eq!(
            a.status,
            format!(
                "Suggested: vise jaws on ±Y, grip {grip} mm. {clashing} operations come within the gap; see the warnings."
            )
        );
        assert_eq!(json(&a.doc.setups[0].operations), operations);
        // With only the pocket, which stays above the jaws, every operation is clear.
        for i in [0, 2, 3, 4, 5] {
            a.doc.setups[0].operations[i].enabled = false;
        }
        let _ = a.update(Message::SuggestLayout);
        assert_eq!(
            a.status,
            "Suggested: vise jaws on ±Y, grip 8 mm. Every operation is clear of them."
        );
        assert_eq!(get(&a.doc.setup().p, "vise_grip"), "8");
        // The gap is the setup's: a 100 mm gap reaches the pocket too, which is the one operation.
        a.doc
            .setup_mut()
            .p
            .insert("holding_gap".into(), "100".into());
        let _ = a.update(Message::SuggestLayout);
        assert!(
            a.status
                .ends_with("1 operation comes within the gap; see the warnings."),
            "{}",
            a.status
        );
    }
    #[test]
    fn operations_that_cannot_be_generated_are_left_out_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let break_op = |a: &mut App, i: usize| {
            a.doc.setups[0].operations[i].tool = "gone".into();
        };
        // The drilling cannot, so neither can what follows it, which needs its stock.
        let mut a = planning_app(&dir);
        break_op(&mut a, 3);
        let vise = first_of(&a, "Vise");
        let plans = setup_plans(&a, 0);
        assert_eq!(plans.len(), 3);
        let refs: Vec<&crate::simulation::Plan> = plans.iter().collect();
        let holder = a.doc.holder(&vise).unwrap().clone();
        let (axis_y, grip) = crate::holding::suggest_vise(plans[0].stock, None, &holder, &refs, 2.);
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        let p = &a.doc.setup().p;
        assert_eq!(
            [get(p, "vise_axis"), get(p, "vise_grip")],
            [if axis_y { "Y" } else { "X" }, grip.to_string().as_str()]
        );
        assert!(
            a.status.ends_with(
                " 3 operations were not considered because they could not be generated."
            ),
            "{}",
            a.status
        );
        // Only the last operation: one is left out.
        let mut a = planning_app(&dir);
        break_op(&mut a, 5);
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        assert!(
            a.status
                .ends_with(" 1 operation was not considered because it could not be generated."),
            "{}",
            a.status
        );
        assert!(
            a.status.starts_with("Suggested: vise jaws on ±Y, grip "),
            "{}",
            a.status
        );
        // The same operation disabled is not one that was left out.
        a.doc.setups[0].operations[5].enabled = false;
        let _ = a.update(Message::SuggestLayout);
        assert!(!a.status.contains("not considered"), "{}", a.status);
        // None generates: the stock alone decides, and the status says so.
        let mut a = planning_app(&dir);
        break_op(&mut a, 0);
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        assert_eq!(
            a.status,
            format!(
                "Suggested: vise jaws on ±Y, grip 8 mm. {STOCK_ONLY} 6 operations were not considered because they could not be generated."
            )
        );
    }
    #[test]
    fn suggest_layout_looks_at_the_active_setups_own_toolpaths() {
        let dir = tempfile::tempdir().unwrap();
        let (mut doc, mesh) = crate::post::fixture::flipped_after(3);
        doc.adopt_holders();
        let mut a = planning_app(&dir);
        a.replace_document(doc, None);
        a.imported_mesh = Some(mesh);
        // Setup 1 only faces the flipped part high above the jaws; setup 0 also contours it beside them.
        for (si, clear) in [(1, true), (0, false)] {
            let _ = a.update(Message::SelectSetup(si));
            let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
            let plans = setup_plans(&a, si);
            let refs: Vec<&crate::simulation::Plan> = plans.iter().collect();
            let holder = a
                .doc
                .holder(get(&a.doc.setup().p, "holder"))
                .unwrap()
                .clone();
            // The flipped setup's vise closes on the band setup 0 left it.
            let mesh = a.imported_mesh.as_ref().unwrap();
            let first = crate::simulation::prepare(&a.doc, mesh, si, 0).unwrap();
            let bands = crate::post::bands(&a.doc.setups[si], mesh, &first);
            assert_eq!(bands.is_some(), si == 1);
            let (axis_y, grip) =
                crate::holding::suggest_vise(plans[0].stock, bands.as_ref(), &holder, &refs, 2.);
            let p = &a.doc.setup().p;
            assert_eq!(get(p, "vise_axis"), if axis_y { "Y" } else { "X" });
            assert_eq!(get(p, "vise_grip"), grip.to_string());
            assert_eq!(
                a.status.ends_with("Every operation is clear of them."),
                clear,
                "setup {si}: {}",
                a.status
            );
        }
    }
    #[test]
    fn without_a_stock_nothing_is_suggested_or_written() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        // No part and a stock sized from it: the stock cannot be told.
        a.doc.sample_model = false;
        a.project_dirty = false;
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        assert!(a.project_dirty, "the edit itself still counts");
        let none = "No layout suggested: this setup's stock is not known yet. Load the part and check the stock settings.";
        assert_eq!(a.status, none);
        assert!(!a.doc.setup().p.contains_key("vise_axis"));
        assert!(!a.doc.setup().p.contains_key("vise_offset"));
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Clamps".into()));
        assert_eq!(a.status, none);
        assert!(!a.doc.setup().p.contains_key("clamps"));
        let _ = a.update(Message::SuggestLayout);
        assert_eq!(a.status, none);
        // A box of the setup's own needs no part: 100 x 60 fits both ways, and the narrower is across Y.
        edit_setup(
            &mut a,
            &[
                ("stock_type", "Fixed size box"),
                ("stock_x", "100"),
                ("stock_y", "60"),
                ("stock_z", "30"),
                ("holding", "Vise"),
            ],
        );
        let _ = a.update(Message::SuggestLayout);
        assert_eq!(get(&a.doc.setup().p, "vise_axis"), "Y");
        assert_eq!(
            a.status,
            format!("Suggested: vise jaws on ±Y, grip 8 mm. {STOCK_ONLY}")
        );
    }
    #[test]
    fn a_stock_too_wide_for_every_vise_axis_is_said_in_the_status() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        edit_setup(
            &mut a,
            &[
                ("stock_type", "Fixed size box"),
                ("stock_x", "140"),
                ("stock_y", "130"),
                ("stock_z", "30"),
            ],
        );
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        assert_eq!(
            a.status,
            format!(
                "Suggested: vise jaws on ±Y, grip 8 mm. {STOCK_ONLY} The stock is 130 mm across the jaws; this vise opens to 120 mm."
            )
        );
        assert_eq!(
            crate::ui::holding_lines(&a),
            ["The stock is 130 mm across the jaws; this vise opens to 120 mm."]
        );
    }
    #[test]
    fn suggest_layout_writes_and_touches_only_what_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        // Pressing it again suggests the same: nothing is written and the project is not marked changed,
        // but the status says what was suggested.
        a.project_dirty = false;
        a.status = "Kept.".into();
        let _ = a.update(Message::SuggestLayout);
        assert!(!a.project_dirty);
        assert_eq!(
            a.status,
            format!("Suggested: vise jaws on ±Y, grip 8 mm. {STOCK_ONLY}")
        );
        // Once the offset differs it is written again.
        edit_setup(&mut a, &[("vise_offset", "5")]);
        a.project_dirty = false;
        let _ = a.update(Message::SuggestLayout);
        assert!(a.project_dirty);
        assert_eq!(get(&a.doc.setup().p, "vise_offset"), "0");
    }
    #[test]
    fn suggesting_clamps_leaves_another_setups_typed_text_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Clamps".into()));
        let _ = a.update(Message::SetClamps("-X".into()));
        assert!(a.clamps_draft.is_some());
        // The duplicate is the active setup, with the same clamps; suggesting for it is not about the draft.
        let _ = a.update(Message::DuplicateSetup);
        let _ = a.update(Message::SuggestLayout);
        assert!(a.status.starts_with("Suggested: 4 clamps"), "{}", a.status);
        assert!(a.clamps_draft.is_some());
        let _ = a.update(Message::SelectSetup(0));
        assert_eq!(a.clamps_field().0, "-X");
        // Suggesting for the setup the text was typed in drops it, even when the positions stay as they were.
        let _ = a.update(Message::SuggestLayout);
        assert!(a.clamps_draft.is_none());
        assert_eq!(a.clamps_field().0, "-X:0;+X:0;-Y:0;+Y:0");
    }
    #[test]
    fn clamp_positions_need_a_setup() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        let _ = a.update(Message::SetClamps("-X:0".into()));
        assert!(a.doc.setups.is_empty());
        assert!(a.status.contains("Create a setup"), "{}", a.status);
    }
    #[test]
    fn the_setup_editor_shows_what_is_wrong_with_the_workholding() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        edit_setup(
            &mut a,
            &[
                ("stock_type", "Fixed size box"),
                ("stock_x", "140"),
                ("stock_y", "100"),
                ("stock_z", "40"),
            ],
        );
        let _ = a.update(Message::SetupEditor);
        assert!(crate::ui::holding_lines(&a).is_empty());
        let _ = a.view();
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        // The suggestion puts the jaws on the Y sides, the only ones the vise opens wide enough for.
        assert_eq!(get(&a.doc.setup().p, "vise_axis"), "Y");
        assert!(crate::ui::holding_lines(&a).is_empty());
        let _ = a.view();
        // The jaws facing the X sides see 140 mm.
        let _ = a.update(Message::Edit(Target::Setup, "vise_axis", "X".into()));
        assert_eq!(
            crate::ui::holding_lines(&a),
            ["The stock is 140 mm across the jaws; this vise opens to 120 mm."]
        );
        let _ = a.view();
        // The jaws facing the Y sides see 100 mm.
        let _ = a.update(Message::Edit(Target::Setup, "vise_axis", "Y".into()));
        assert!(crate::ui::holding_lines(&a).is_empty());
        // An entry deleted from the library.
        a.doc.workholding.retain(|h| h.kind() != "Vise");
        assert_eq!(
            crate::ui::holding_lines(&a),
            ["The workholding entry was deleted; choose another."]
        );
        let _ = a.view();
        // A bad number is the layout's error, after the entry's.
        edit_setup(&mut a, &[("vise_offset", "far")]);
        assert_eq!(
            crate::ui::holding_lines(&a),
            [
                "The workholding entry was deleted; choose another.",
                "The vise offset must be a number."
            ]
        );
        let vise = crate::holding::starters().remove(0);
        let id = vise.id.clone();
        a.doc.workholding.push(vise);
        // Choosing another entry places the jaws again, replacing the offset that was no number.
        edit_setup(&mut a, &[("holder", &id)]);
        assert_eq!(get(&a.doc.setup().p, "vise_offset"), "0");
        assert!(crate::ui::holding_lines(&a).is_empty());
    }
    #[test]
    fn the_setup_editor_says_a_later_setup_not_square_to_setup_1_cannot_model_its_workholding() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Clamps".into()));
        let _ = a.update(Message::DuplicateSetup);
        assert_eq!(a.doc.index(), 1);
        let _ = a.update(Message::SetupEditor);
        for rz in ["90", "180"] {
            edit_setup(&mut a, &[("rz", rz)]);
            assert!(crate::ui::holding_lines(&a).is_empty(), "rz {rz}");
            let _ = a.view();
        }
        edit_setup(&mut a, &[("rz", "30")]);
        assert_eq!(crate::ui::holding_lines(&a), [crate::holding::NOT_SQUARE]);
        let _ = a.view();
        // The 3D view draws no boxes for it, and setup 1 is as it was.
        assert!(crate::ui::holding_boxes(&a).is_empty());
        let _ = a.update(Message::SelectSetup(0));
        assert!(crate::ui::holding_lines(&a).is_empty());
        let _ = a.view();
    }
    #[test]
    fn the_setup_editor_opens_each_step_and_marks_what_is_done() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        a.doc.adopt_materials();
        a.doc.material_id.clear();
        let _ = a.update(Message::SetupEditor);
        // Holding is one click from the top, like every step, and each builds.
        assert_eq!(a.setup_step, "Machine");
        for step in crate::ui::SETUP_STEPS {
            let _ = a.update(Message::SetupStep(step));
            assert_eq!(a.setup_step, step);
            let _ = a.view();
        }
        let done = |a: &App, step| crate::ui::setup_step_done(a, step);
        assert!(done(&a, "Machine") && done(&a, "Position"));
        // Holding: a modelled vise with nothing wrong, not None or a vise too narrow.
        assert!(!done(&a, "Holding"));
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        assert!(done(&a, "Holding"));
        let _ = a.update(Message::Edit(Target::Setup, "vise_axis", "X".into()));
        assert!(!done(&a, "Holding"));
        // Stock: a material, and a box that holds the part.
        assert!(!done(&a, "Stock"));
        let id = a.doc.materials[0].id.clone();
        let _ = a.update(Message::PickMaterial(id));
        assert!(done(&a, "Stock"));
        edit_setup(
            &mut a,
            &[("stock_type", "Fixed size box"), ("stock_x", "10")],
        );
        assert!(!done(&a, "Stock"));
        // An operation opened in between keeps the step for the way back.
        let _ = a.update(Message::SetupStep("Holding"));
        let _ = a.update(Message::SelectOp(0));
        let _ = a.update(Message::SetupEditor);
        assert_eq!(a.setup_step, "Holding");
        let _ = a.view();
        // The tool editor, its Save under the scrolling fields, builds too.
        let _ = a.update(Message::NewTool);
        let _ = a.view();
    }
    /// The work zero has a step of its own: the heading's link opens it
    /// from an operation, the faces and the grid of the stock seen from
    /// above set the origin, and the work offset is chosen there too.
    #[test]
    fn the_work_zero_step_picks_the_zero_on_a_grid_of_the_stock() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        let origin = |a: &App| get(&a.doc.setup().p, "origin").to_string();
        let _ = a.update(Message::SelectOp(0));
        assert!(a.selected.is_some());
        // The heading's "· Change" link.
        let _ = a.update(Message::SetupStep("Work zero"));
        assert_eq!((a.setup_step, a.selected), ("Work zero", None));
        let _ = a.view();
        assert!(
            crate::ui::setup_step_done(&a, "Work zero"),
            "{}",
            origin(&a)
        );
        // What the grid's back-right button and the Stock bottom button send.
        let _ = a.update(Message::Edit(
            Target::Setup,
            "origin",
            "Stock top back-right".into(),
        ));
        let _ = a.view();
        let _ = a.update(Message::Edit(
            Target::Setup,
            "origin",
            "Stock bottom back-right".into(),
        ));
        assert_eq!(origin(&a), "Stock bottom back-right");
        let _ = a.update(Message::Edit(
            Target::Setup,
            "origin",
            "Model origin".into(),
        ));
        let _ = a.view();
        // A zero that is no work zero leaves the step undone.
        a.doc
            .setup_mut()
            .p
            .insert("origin".into(), "Somewhere".into());
        assert!(!crate::ui::setup_step_done(&a, "Work zero"));
        // The buttons' cards say what each zero is.
        let card = crate::help::command(
            &Message::Edit(Target::Setup, "origin", "Stock top front-left".into()),
            "Front-left".into(),
        );
        assert!(!card.body.is_empty());
    }
    #[test]
    fn the_workholding_page_and_the_setup_editor_build_in_each_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        let build = |a: &mut App| {
            let _ = a.update(Message::SetupEditor);
            for page in [Page::Workholding, Page::Work] {
                a.page = page;
                let _ = a.view();
            }
        };
        build(&mut a);
        for holding in ["Vise", "Clamps", "None"] {
            let _ = a.update(Message::Edit(Target::Setup, "holding", holding.into()));
            build(&mut a);
        }
        // Clamps with a typed text that does not read, and one that does.
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Clamps".into()));
        let _ = a.update(Message::SetClamps("-X".into()));
        build(&mut a);
        let _ = a.update(Message::SetClamps("-X:0;+X:0".into()));
        build(&mut a);
        // The library page with a clamp, a vise, an unsaved one and an empty library.
        for id in a
            .doc
            .workholding
            .iter()
            .map(|h| h.id.clone())
            .collect::<Vec<_>>()
        {
            let _ = a.update(Message::EditHolder(id));
            build(&mut a);
        }
        let _ = a.update(Message::NewHolder("Clamp"));
        build(&mut a);
        // A vise setup with no vise in the library and with a deleted one.
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        a.doc.workholding.clear();
        build(&mut a);
        let _ = a.update(Message::DeleteHolder("none".into()));
        build(&mut a);
        // Without a setup, and with no document at all.
        let mut blank = fresh_app(&dir);
        for page in [Page::Workholding, Page::Work, Page::Home] {
            blank.page = page;
            let _ = blank.view();
        }
    }
    #[test]
    fn a_setup_with_no_entry_of_its_kind_says_so_before_any_number_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        a.doc.workholding.retain(|h| h.kind() == "Clamp");
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        assert_eq!(get(&a.doc.setup().p, "holder"), "");
        assert_eq!(
            crate::ui::holding_lines(&a),
            [
                "Choose a vise from the library. Add one on the Workholding page.",
                "The vise grip must be a positive number."
            ]
        );
        let _ = a.update(Message::SetupEditor);
        let _ = a.view();
        // Adding a vise on the Workholding page and choosing it settles both.
        let _ = a.update(Message::NewHolder("Vise"));
        edit_holder(
            &mut a,
            &[
                ("name", "Kurt"),
                ("jaw_width", "150"),
                ("jaw_height", "40"),
                ("jaw_thickness", "25"),
                ("max_opening", "180"),
                ("grip", "10"),
            ],
        );
        let _ = a.update(Message::SaveHolder);
        let kurt = first_of(&a, "Vise");
        let _ = a.update(Message::Edit(Target::Setup, "holder", kurt));
        assert_eq!(get(&a.doc.setup().p, "vise_grip"), "10");
        assert!(
            crate::ui::holding_lines(&a).is_empty(),
            "{:?}",
            crate::ui::holding_lines(&a)
        );
    }
    /// The fixture block faced 4 mm off the top on setup 1 and flipped onto that
    /// face on setup 2, which the starter vise holds on ±Y, 8 mm up (written
    /// on the setup directly, nothing is suggested). Background jobs run in
    /// line, as in the smoke run. Setup 2 is the active one.
    fn leftover_app(dir: &tempfile::TempDir) -> App {
        let (doc, mesh) = crate::post::fixture::flipped_after(1);
        vise_app(dir, doc, mesh)
    }
    /// `doc`, on `mesh`, its second setup held in the starter vise on ±Y at
    /// grip 8, in the setup editor of that setup.
    fn vise_app(
        dir: &tempfile::TempDir,
        mut doc: crate::model::Document,
        mesh: crate::mesh::Mesh,
    ) -> App {
        let mut a = fresh_app(dir);
        doc.adopt_holders();
        a.replace_document(doc, None);
        a.imported_mesh = Some(mesh);
        a.smoke_dir = Some(dir.path().to_path_buf());
        let id = first_of(&a, "Vise");
        let p = &mut a.doc.setups[1].p;
        for (k, v) in [
            ("holding", "Vise"),
            ("holder", id.as_str()),
            ("vise_axis", "Y"),
            ("vise_offset", "0"),
            ("vise_grip", "8"),
            ("holding_gap", "2"),
        ] {
            p.insert(k.into(), v.into());
        }
        let _ = a.update(Message::SelectSetup(1));
        let _ = a.update(Message::SetupEditor);
        a
    }
    /// The box setup 2 of `leftover_app` starts from, as simulating setup 1
    /// makes it.
    fn simulated_stock(a: &App) -> crate::mesh::Bounds {
        let mesh = a.imported_mesh.as_ref().unwrap();
        crate::simulation::prepare(&a.doc, mesh, 1, 0)
            .unwrap()
            .plan
            .stock
    }
    /// `flipped_after(3)` (setup 1 faces, pockets and contours the walls,
    /// but only down to `bottom` mm above the part's underside) in
    /// `vise_app`, setup 1 held in the starter vise too, on ±Y at grip 8.
    fn contoured_app(dir: &tempfile::TempDir, bottom: &str) -> App {
        let (mut doc, mesh) = crate::post::fixture::flipped_after(3);
        doc.setups[0].operations[2]
            .p
            .insert("bottom".into(), bottom.into());
        let mut a = vise_app(dir, doc, mesh);
        let id = first_of(&a, "Vise");
        let p = &mut a.doc.setups[0].p;
        for (k, v) in [
            ("holding", "Vise"),
            ("holder", id.as_str()),
            ("vise_axis", "Y"),
            ("vise_offset", "0"),
            ("vise_grip", "8"),
        ] {
            p.insert(k.into(), v.into());
        }
        a
    }
    #[test]
    fn the_setup_editor_shows_what_the_earlier_setups_left_the_vise_to_grip() {
        use crate::holding::Held;
        let dir = tempfile::tempdir().unwrap();
        // Setup 1's Contour takes only the top 5 mm of the walls. Turned
        // over, setup 2's vise grips 8 mm of what was that top, cut in part.
        let mut a = contoured_app(&dir, "35");
        let mesh = a.imported_mesh.clone().unwrap();
        // Before the job has run, only what needs no simulation is said, and
        // both setups count as checked.
        assert!(crate::ui::holding_lines(&a).is_empty());
        assert_eq!(crate::ui::held(&a), Held::Every);
        let _ = a.refresh_left_stock();
        assert!(a.known_stock(1).1);
        // Worked out: the editor says what the export says, in its words,
        // and the NC review page no longer counts setup 2 as checked.
        let lines = crate::ui::holding_lines(&a);
        assert!(
            lines[0].starts_with("Setup 2 cannot be held in the vise: setup 1, operation 3 removed the material the jaws grip on ±Y from 0 to 8 mm above the stock's bottom. Leave at least 8 mm of those sides uncut in setup 1, "),
            "{lines:?}"
        );
        let mut doc = a.doc.clone();
        doc.setups[1]
            .p
            .insert("origin".into(), "Model origin".into());
        let exported = crate::post::post_job(&doc, &mesh);
        let refused = exported.setups[1].result.as_ref().unwrap_err();
        let said: Vec<String> = lines.iter().map(|l| format!("Flipped: {l}")).collect();
        assert_eq!(refused, &said);
        assert_eq!(crate::ui::held(&a), Held::Mixed);
        assert!(!crate::ui::modelled(&a));
        let _ = a.view();
        // Gripping 3 mm, all of it cut through: the jaws close on the walls.
        // A change of the holding alone checks it again on the stock worked
        // out, without simulating the earlier setups again.
        let before = a.left_stock.before[1].clone().unwrap();
        let _ = a.update(Message::Edit(Target::Setup, "vise_grip", "3".into()));
        assert!(a.known_stock(1).1);
        assert!(a.current_checks().is_none());
        assert!(crate::ui::holding_lines(&a).is_empty());
        let _ = a.refresh_left_stock();
        assert!(std::sync::Arc::ptr_eq(
            &before,
            a.left_stock.before[1].as_ref().unwrap()
        ));
        assert!(a.current_checks().is_some());
        // They hold the stock there, but 3 mm of grip does not hold it
        // against setup 2's facing, 39 mm higher, with the vise's typical
        // 5000 N (LIO-167). Tightened to 10 kN, it does.
        let lines = crate::ui::holding_lines(&a);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("Facing: the cut could tip the stock out of the vise: about 191 N, 39 mm above the top of the jaws' contact, which is 2.9 mm high"),
            "{lines:?}"
        );
        assert_eq!(crate::ui::held(&a), Held::Mixed);
        let vise = a
            .doc
            .workholding
            .iter_mut()
            .find(|h| h.kind() == "Vise")
            .unwrap();
        vise.p.insert("force".into(), "10000".into());
        let _ = a.refresh_left_stock();
        assert_eq!(crate::ui::holding_lines(&a), Vec::<String>::new());
        assert_eq!(crate::ui::held(&a), Held::Every);
        // The jaws are drawn on those walls, the band 80.7 mm across them
        // (the Contour's 0.3 mm allowance), inside the 84 mm box.
        let held = a.known_held(1).unwrap();
        let across = held.band.dimensions()[1];
        assert!(across > 80.6 && across < 80.8, "{held:?}");
        assert!(held.stock.dimensions()[1] > 83.9);
        let jaws = crate::ui::holding_boxes(&a);
        assert_eq!(jaws.len(), 2);
        assert!(
            jaws.iter()
                .any(|j| (j.bounds.max[1] - held.band.min[1]).abs() < 1e-4),
            "{jaws:?}"
        );
        let _ = a.view();
    }
    #[test]
    fn the_browser_draws_the_stock_each_setup_starts_from_and_leaves_in_its_jaws() {
        let dir = tempfile::tempdir().unwrap();
        // Both setups are held in a vise. Setup 1 faces, pockets and
        // contours the top 5 mm of the walls; setup 2 has the part turned
        // over, so that cut band is at the bottom of its walls.
        let mut a = contoured_app(&dir, "35");
        // The drawn pixels that are green, a cut face's colour, and grey,
        // the jaws'.
        let count = |picture: iced::widget::image::Handle| {
            let iced::widget::image::Handle::Rgba { pixels, .. } = picture else {
                unreachable!()
            };
            let drawn = || pixels.chunks(4).filter(|px| px[3] > 0);
            let green = drawn()
                .filter(|px| px[1] > px[0].saturating_add(30))
                .count();
            let grey = drawn()
                .filter(|px| px[0].abs_diff(px[1]) < 4 && px[1].abs_diff(px[2]) < 4)
                .count();
            (green, grey)
        };
        let start = |si, a: &App| count(crate::ui::stock_picture(a, si, false).unwrap());
        let end = |si, a: &App| crate::ui::stock_picture(a, si, true).map(count);
        // Not simulated yet: each setup starts from its block in its jaws,
        // and nothing is drawn as what it leaves. Each is drawn once.
        for si in 0..2 {
            let (green, grey) = start(si, &a);
            assert!(green == 0 && grey > 100, "{green} {grey}");
            assert_eq!(end(si, &a), None);
        }
        let _ = start(1, &a);
        assert_eq!(a.stock_pictures.borrow().len(), 2);
        let _ = a.view();
        // Simulated: setup 1 still starts from its block; setup 2 starts
        // from what setup 1 left it, the walls' cut band green where its
        // jaws, which grip that band, leave it in sight.
        let _ = a.refresh_left_stock();
        assert_eq!(start(0, &a).0, 0);
        let (green, grey) = start(1, &a);
        assert!(green > 20 && grey > 100, "{green} {grey}");
        // What each leaves, in its jaws: setup 1's cut top, and setup 2's
        // faced side, from the very stock setup 2's start is drawn from.
        for si in 0..2 {
            let (green, grey) = end(si, &a).unwrap();
            assert!(green > 1000 && grey > 100, "{si}: {green} {grey}");
        }
        assert!(std::sync::Arc::ptr_eq(
            a.left_stock.after[0].as_ref().unwrap(),
            a.left_stock.before[1].as_ref().unwrap()
        ));
        assert_eq!(a.left_stock.surfaces.len(), 2);
        let _ = a.view();
        // A job of one setup: what it leaves is drawn too.
        let mut single = leftover_app(&dir);
        single.doc.setups.pop();
        let _ = single.refresh_left_stock();
        assert!(end(0, &single).unwrap().0 > 1000);
        // The example is not simulated: its setup is drawn as its block.
        let mut sample = app();
        sample.doc = Document::sample();
        assert_eq!(start(0, &sample).0, 0);
        assert_eq!(end(0, &sample), None);
    }
    #[test]
    fn a_new_left_stock_result_does_not_reuse_the_picture_of_an_earlier_job() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = contoured_app(&dir, "35");
        let _ = a.refresh_left_stock();
        let first = crate::ui::stock_picture(&a, 0, true).unwrap();
        let drawn = a.stock_pictures.borrow().len();
        // The job is edited and its new result holds a stock at the address
        // of the old one, as one allocated after the old is freed can.
        let old = a.left_stock.clone();
        a.doc.setups[0].p.insert("note".into(), "edited".into());
        let key = a.left_key().unwrap();
        assert_ne!(key, old.key);
        a.left_stock = crate::LeftStock { key, ..old };
        let second = crate::ui::stock_picture(&a, 0, true).unwrap();
        assert_ne!(first.id(), second.id(), "the old picture was shown");
        assert_eq!(a.stock_pictures.borrow().len(), drawn + 1);
        // The same job asks for the same picture again.
        let again = crate::ui::stock_picture(&a, 0, true).unwrap();
        assert_eq!(second.id(), again.id());
    }
    #[test]
    fn the_job_says_what_it_leaves_under_stock_once_it_is_simulated() {
        let dir = tempfile::tempdir().unwrap();
        // The top faced on setup 1 and the underside on setup 2: the 2 mm of
        // stock on the sides and the top's pocket and holes are left.
        let mut a = leftover_app(&dir);
        let line = |a: &App| crate::ui::uncut_line(a).unwrap();
        assert_eq!(line(&a), "Completeness not checked yet.");
        let _ = a.refresh_left_stock();
        assert!(
            line(&a).starts_with("Magenta faces are still under stock after the whole job: "),
            "{}",
            line(&a)
        );
        let found = a.left_stock.uncut.clone().unwrap().unwrap();
        assert!(found.regions.len() > 1 && found.faces.iter().any(|f| *f));
        let _ = a.view();
        // An operation that cannot be generated: not checked, and why.
        a.doc.setups[1].operations[0].tool = "gone".into();
        let _ = a.refresh_left_stock();
        assert_eq!(
            line(&a),
            "Completeness not checked: Flipped / Facing could not be generated."
        );
        let _ = a.view();
        // The example is not simulated: nothing is said.
        let mut sample = app();
        sample.doc = Document::sample();
        assert_eq!(crate::ui::uncut_line(&sample), None);
    }
    #[test]
    fn the_3d_view_has_a_show_uncut_faces_toggle_and_a_legend_that_match_the_line() {
        use iced::advanced::widget::Tree;
        /// The nodes of a widget tree.
        fn nodes(t: &Tree) -> usize {
            1 + t.children.iter().map(nodes).sum::<usize>()
        }
        let dir = tempfile::tempdir().unwrap();
        let mut a = leftover_app(&dir);
        // Not checked yet: nothing is magenta, so no toggle and no magenta key.
        assert!(crate::ui::uncut_toggle(&a).is_none());
        let legend = |a: &App| crate::ui::uncut_legend(a).map(|l| nodes(&Tree::new(l.as_widget())));
        let toggle = |a: &App| crate::ui::uncut_toggle(a).map(|t| nodes(&Tree::new(t.as_widget())));
        let without = legend(&a).expect("tan and green");
        let _ = a.refresh_left_stock();
        // The job leaves faces under stock: the checkbox is there, on or off.
        assert!(toggle(&a).is_some());
        // On by default: the line promises magenta and the legend names it.
        assert!(a.show_uncut);
        assert!(
            crate::ui::uncut_line(&a)
                .unwrap()
                .starts_with("Magenta faces")
        );
        assert!(legend(&a) > Some(without));
        let _ = a.view();
        // Off: the line no longer promises magenta, and the legend drops it.
        let _ = a.update(Message::ToggleUncut);
        assert!(!a.show_uncut);
        let line = crate::ui::uncut_line(&a).unwrap();
        assert!(
            !line.contains("Magenta") && line.contains("Show uncut faces is off"),
            "{line}"
        );
        assert_eq!(legend(&a), Some(without));
        assert!(toggle(&a).is_some());
        let _ = a.view();
        // Stock hidden and the faces off: nothing to name.
        let _ = a.update(Message::ToggleStock);
        assert!(crate::ui::uncut_legend(&a).is_none());
        // The example is not simulated: no toggle.
        let mut sample = app();
        sample.doc = Document::sample();
        assert!(crate::ui::uncut_toggle(&sample).is_none());
    }
    #[test]
    fn the_editor_draws_the_stock_a_later_setup_starts_from_only_when_it_is_known() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = leftover_app(&dir);
        let mesh = a.imported_mesh.clone().unwrap();
        let nominal = a.doc.setup_stock(1, Some(&mesh)).unwrap();
        // Not worked out yet: the 3D view is given no box for setup 2.
        assert_eq!(crate::ui::leftover_view(&a), None);
        let _ = a.view();
        // Worked out: the box setup 1's facing left, 4 mm shorter, which is
        // what the jaws are drawn on and what the stock box is drawn as.
        let _ = a.refresh_left_stock();
        let left = simulated_stock(&a);
        assert!((left.dimensions()[2] - (nominal.dimensions()[2] - 4.)).abs() < 0.01);
        assert_eq!(crate::ui::leftover_view(&a), Some(left));
        let jaws = crate::ui::holding_boxes(&a);
        assert!(
            jaws.iter()
                .all(|b| (b.bounds.max[2] - (left.min[2] + 8.)).abs() < 0.01)
        );
        let _ = a.view();
        // An edit of an earlier operation makes it out of date: no box again.
        let _ = a.update(Message::SelectSetup(0));
        let _ = a.update(Message::SelectOp(0));
        let _ = a.update(Message::Edit(Target::Operation, "bottom", "1".into()));
        let _ = a.update(Message::SelectSetup(1));
        let _ = a.update(Message::SetupEditor);
        assert_eq!(crate::ui::leftover_view(&a), None);
        let _ = a.view();
        // Editing the vise keeps it (and the box) current once it is worked
        // out again; setup 1 never gets one, as it draws its own stock.
        let _ = a.refresh_left_stock();
        assert!(crate::ui::leftover_view(&a).is_some());
        let _ = a.update(Message::Edit(Target::Setup, "vise_grip", "6".into()));
        assert!(crate::ui::leftover_view(&a).is_some());
        let _ = a.update(Message::SelectSetup(0));
        assert_eq!(crate::ui::leftover_view(&a), None);
        let _ = a.view();
        // The example is not simulated and draws its own stock.
        let mut sample = app();
        sample.doc = Document::sample();
        assert!(sample.is_sample());
        assert_eq!(crate::ui::leftover_view(&sample), None);
    }
    #[test]
    fn the_setup_editor_says_the_stock_is_the_original_until_the_earlier_setups_are_simulated() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = leftover_app(&dir);
        let mesh = a.imported_mesh.clone().unwrap();
        let nominal = a.doc.setup_stock(1, Some(&mesh)).unwrap();
        const NOTE: &str = "Original stock size until the earlier setups are simulated.";
        // Setup 1's stock is its own, whatever has been simulated.
        assert_eq!(a.known_stock(0), (a.doc.setup_stock(0, Some(&mesh)), true));
        assert_eq!(crate::ui::stock_note(&a), Some(NOTE));
        assert_eq!(a.known_stock(1), (Some(nominal), false));
        let _ = a.view();
        // Simulated: the box setup 1 left, and no note.
        let _ = a.refresh_left_stock();
        let left = simulated_stock(&a);
        assert!((left.dimensions()[2] - (nominal.dimensions()[2] - 4.)).abs() < 0.01);
        assert_eq!(a.known_stock(1), (Some(left), true));
        assert_eq!(crate::ui::stock_note(&a), None);
        let _ = a.view();
        // Setup 1 is not a later setup: no note for it.
        let _ = a.update(Message::SelectSetup(0));
        assert_eq!(crate::ui::stock_note(&a), None);
        let _ = a.view();
        // Nothing to simulate without the part; one setup is simulated for
        // the browser's picture of what it leaves.
        let mut single = leftover_app(&dir);
        single.doc.setups.pop();
        assert!(single.left_key().is_some());
        let mut no_part = leftover_app(&dir);
        no_part.imported_mesh = None;
        assert_eq!(no_part.left_key(), None);
        assert_eq!(no_part.known_stock(1), (None, false));
        assert_eq!(crate::ui::stock_note(&no_part), None);
    }
    #[test]
    fn the_workholding_checks_and_the_jaws_use_the_stock_the_earlier_setups_left() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = leftover_app(&dir);
        let mesh = a.imported_mesh.clone().unwrap();
        let nominal = a.doc.setup_stock(1, Some(&mesh)).unwrap();
        let top_of_jaws = |a: &App| -> Vec<f32> {
            crate::ui::holding_boxes(a)
                .iter()
                .map(|b| b.bounds.max[2])
                .collect()
        };
        // The jaws are drawn from the original box's bottom, then from the leftover's.
        assert!(crate::ui::holding_lines(&a).is_empty());
        assert_eq!(top_of_jaws(&a).len(), 2);
        assert!(
            top_of_jaws(&a)
                .iter()
                .all(|z| (z - (nominal.min[2] + 8.)).abs() < 0.01)
        );
        let _ = a.refresh_left_stock();
        let left = simulated_stock(&a);
        assert!(left.min[2] > nominal.min[2] + 3.9);
        assert!(crate::ui::holding_lines(&a).is_empty());
        assert_eq!(top_of_jaws(&a).len(), 2);
        assert!(
            top_of_jaws(&a)
                .iter()
                .all(|z| (z - (left.min[2] + 8.)).abs() < 0.01)
        );
        // A grip the original box is tall enough for and the leftover is not:
        // the editor says so once the box is known.
        let grip = (nominal.dimensions()[2] + left.dimensions()[2]) / 2.;
        let _ = a.update(Message::Edit(Target::Setup, "vise_grip", grip.to_string()));
        assert!(!crate::ui::holding_lines(&a).is_empty());
        assert_eq!(
            crate::ui::holding_lines(&a),
            a.doc.holding_issues_with(1, Some(left.into()))
        );
        assert!(crate::ui::holding_boxes(&a).is_empty());
        let _ = a.view();
        assert_eq!(
            a.doc.holding_issues_with(1, Some(nominal.into())),
            Vec::<String>::new()
        );
        // The same grip on the original box is fine.
        assert_eq!(a.doc.holding_issues(1, Some(&mesh)), Vec::<String>::new());
        // The export-side height rule agrees with the editor.
        assert_eq!(
            a.doc.holding_issues_with(1, Some(left.into())),
            [crate::holding::height_issue(
                &crate::holding::layout(&a.doc.setups[1]).unwrap(),
                left
            )
            .unwrap()]
        );
    }
    #[test]
    fn an_edit_makes_the_known_stock_stale_until_the_simulation_is_done_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = leftover_app(&dir);
        let mesh = a.imported_mesh.clone().unwrap();
        let nominal = a.doc.setup_stock(1, Some(&mesh)).unwrap();
        let _ = a.refresh_left_stock();
        let first = simulated_stock(&a);
        assert_eq!(a.known_stock(1), (Some(first), true));
        // Facing 1 mm less off setup 1's top leaves a taller block.
        let _ = a.update(Message::SelectSetup(0));
        let _ = a.update(Message::SelectOp(0));
        let _ = a.update(Message::Edit(Target::Operation, "bottom", "1".into()));
        let _ = a.update(Message::SelectSetup(1));
        let _ = a.update(Message::SetupEditor);
        assert_eq!(a.known_stock(1), (Some(nominal), false));
        assert!(crate::ui::stock_note(&a).is_some());
        let _ = a.view();
        let _ = a.refresh_left_stock();
        let second = simulated_stock(&a);
        assert!(second.dimensions()[2] > first.dimensions()[2] + 0.5);
        assert_eq!(a.known_stock(1), (Some(second), true));
        assert_eq!(crate::ui::stock_note(&a), None);
        // A tool change, a placement change and another part do too.
        let key = a.left_key();
        a.doc.tools[0].p.insert("diameter".into(), "8".into());
        assert_ne!(a.left_key(), key);
        assert!(!a.known_stock(1).1);
        a.doc.tools[0].p.insert("diameter".into(), "10".into());
        assert_eq!(a.left_key(), key);
        assert!(a.known_stock(1).1);
        a.doc.setups[1].p.insert("tx".into(), "5".into());
        assert_ne!(a.left_key(), key);
        a.doc.setups[1].p.insert("tx".into(), "0".into());
        a.imported_mesh.as_mut().unwrap().fingerprint = "another part".into();
        assert_ne!(a.left_key(), key);
        assert!(!a.known_stock(1).1);
    }
    #[test]
    fn editing_the_workholding_keeps_the_known_stock() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = leftover_app(&dir);
        let _ = a.refresh_left_stock();
        let (left, key) = (a.known_stock(1), a.left_key());
        assert!(left.1);
        for (k, v) in [
            ("vise_grip", "6"),
            ("vise_offset", "2"),
            ("vise_axis", "X"),
            ("holding_gap", "3"),
        ] {
            let _ = a.update(Message::Edit(Target::Setup, k, v.into()));
            assert_eq!(a.left_key(), key, "{k}");
            assert_eq!(a.known_stock(1), left, "{k}");
        }
        let clamp = first_of(&a, "Clamp");
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Clamps".into()));
        let _ = a.update(Message::Edit(Target::Setup, "holder", clamp));
        let _ = a.update(Message::SetClamps("-X:0".into()));
        assert_eq!(a.left_key(), key);
        assert_eq!(a.known_stock(1), left);
        // The name of the setup and its other settings are the document's, though.
        let _ = a.update(Message::Edit(Target::Setup, "rz", "90".into()));
        assert_ne!(a.left_key(), key);
    }
    /// What a job that worked out `boxes` for the job whose fingerprints are
    /// `key` (the stock's) and `held` (its holding checks') answers, with no
    /// checks to show.
    fn worked_out(key: u64, held: u64, boxes: Vec<Option<crate::mesh::Bounds>>) -> Message {
        let left = crate::LeftStock {
            key,
            boxes,
            before: vec![],
            after: vec![],
            surfaces: vec![],
            bites: vec![],
            uncut: None,
        };
        let checks = crate::HoldingChecks {
            key: held,
            setups: vec![],
        };
        Message::LeftStock(Box::new((left, checks)))
    }
    #[test]
    fn a_result_for_a_document_that_was_edited_while_it_ran_is_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = leftover_app(&dir);
        // Not in line: the job is on the blocking pool, its result a message.
        a.smoke_dir = None;
        let (started, held) = (a.left_key().unwrap(), a.holding_key().unwrap());
        let _ = a.refresh_left_stock();
        assert_eq!(a.left_job, Some(started));
        // One job at a time.
        assert_eq!(a.refresh_left_stock().units(), 0);
        let nominal = a.known_stock(1);
        let boxes = vec![None, Some(simulated_stock(&a))];
        // An edit while it runs.
        let _ = a.update(Message::SelectSetup(0));
        let _ = a.update(Message::SelectOp(0));
        let _ = a.update(Message::Edit(Target::Operation, "bottom", "1".into()));
        let _ = a.update(Message::SelectSetup(1));
        assert_ne!(a.left_key(), Some(started));
        let _ = a.update(worked_out(started, held, boxes.clone()));
        assert_eq!(a.left_job, None);
        assert_eq!((a.left_stock.key, a.left_stock.boxes.len()), (0, 0));
        assert_eq!(a.holding_checks, crate::HoldingChecks::default());
        assert_eq!(a.known_stock(1), (nominal.0, false));
        // The next job is for the edited document, and its result is kept.
        let now = a.left_key().unwrap();
        assert_eq!(a.refresh_left_stock().units(), 1);
        assert_eq!(a.left_job, Some(now));
        let _ = a.update(worked_out(now, a.holding_key().unwrap(), boxes.clone()));
        assert_eq!(a.left_job, None);
        assert_eq!(a.known_stock(1), (boxes[1], true));
        // Nothing more to do for the document as it is.
        assert_eq!(a.refresh_left_stock().units(), 0);
        assert_eq!(a.left_job, None);
    }
    #[test]
    fn a_result_whose_holding_was_edited_while_it_ran_keeps_the_stock_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = leftover_app(&dir);
        a.smoke_dir = None;
        let (key, held) = (a.left_key().unwrap(), a.holding_key().unwrap());
        let _ = a.refresh_left_stock();
        assert_eq!(a.left_job, Some(key));
        let boxes = vec![None, Some(simulated_stock(&a))];
        // The grip changes while it runs: the stock is still the job's, its
        // holding checks are not.
        let _ = a.update(Message::Edit(Target::Setup, "vise_grip", "6".into()));
        assert_eq!(a.left_key(), Some(key));
        assert_ne!(a.holding_key(), Some(held));
        let _ = a.update(worked_out(key, held, boxes.clone()));
        assert_eq!(a.left_job, None);
        assert_eq!(a.known_stock(1), (boxes[1], true));
        assert!(a.current_checks().is_none());
        // The next run is for the checks.
        assert_eq!(a.refresh_left_stock().units(), 1);
        assert_eq!(a.left_job, Some(key));
    }
    #[test]
    fn the_simulation_waits_for_the_edits_to_pause_and_runs_one_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = leftover_app(&dir);
        a.smoke_dir = None;
        let _ = a.update(Message::Tick);
        // Just edited: not yet.
        a.last_edit = std::time::Instant::now();
        let _ = a.update(Message::Tick);
        assert_eq!((a.left_job, a.ready_job), (None, None));
        // The pause is as long as the project's save waits. The checks
        // before a run of the active setup start with the stock's run.
        a.last_edit = std::time::Instant::now() - std::time::Duration::from_millis(800);
        let _ = a.update(Message::Tick);
        assert_eq!(a.left_job, a.left_key());
        assert!(a.left_job.is_some());
        let held = a.holding_key().unwrap();
        assert_eq!(a.ready_job, Some(held));
        // Running: the ticks start no other.
        let running = a.left_job;
        let _ = a.update(Message::Tick);
        assert_eq!((a.left_job, a.ready_job), (running, Some(held)));
        assert!(a.blocking_work());
        let _ = a.update(worked_out(running.unwrap(), held, vec![None, None]));
        assert!(a.blocking_work());
        let setup = a.doc.index();
        let ready = crate::run::Ready {
            key: held,
            setup,
            ..Default::default()
        };
        let _ = a.update(Message::Ready(Box::new(ready)));
        assert!(!a.blocking_work());
        // Done for this document: the ticks start none.
        let _ = a.update(Message::Tick);
        assert_eq!((a.left_job, a.ready_job), (None, None));
    }
    #[test]
    fn a_setup_whose_earlier_setups_failed_keeps_the_original_stock_and_the_note() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = leftover_app(&dir);
        let mesh = a.imported_mesh.clone().unwrap();
        let nominal = a.doc.setup_stock(1, Some(&mesh)).unwrap();
        // Setup 1's facing has no tool to cut with.
        a.doc.setups[0].operations[0].tool = "gone".into();
        let _ = a.refresh_left_stock();
        assert_eq!(a.left_stock.key, a.left_key().unwrap());
        assert_eq!(a.left_stock.boxes, [None, None]);
        assert_eq!(a.known_stock(1), (Some(nominal), false));
        assert!(crate::ui::stock_note(&a).is_some());
        let _ = a.view();
        // The job is not done again for the same document.
        assert_eq!(a.refresh_left_stock().units(), 0);
    }
    #[test]
    fn suggest_layout_uses_the_stock_the_earlier_setups_left_when_it_is_known() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = leftover_app(&dir);
        let mesh = a.imported_mesh.clone().unwrap();
        // The box setup 1's facing leaves setup 2, worked out from its toolpaths.
        let left = simulated_stock(&a);
        let nominal = a.doc.setup_stock(1, Some(&mesh)).unwrap();
        assert!((left.dimensions()[2] - (nominal.dimensions()[2] - 4.)).abs() < 0.01);
        // Setup 2 has no toolpaths: the stock alone decides. The vise is one
        // that grips as much as the original box allows, and is not cut below.
        a.doc.setups[1].operations.clear();
        let id = first_of(&a, "Vise");
        let vise = a.doc.workholding.iter_mut().find(|h| h.id == id).unwrap();
        for (k, v) in [("grip", "45"), ("jaw_height", "50")] {
            vise.p.insert(k.into(), v.into());
        }
        // The original box is 124 mm across X, which the 120 mm vise does not
        // open to: the jaws go on ±Y. Before the earlier setups are simulated
        // the original box is all there is: it is 47 mm tall, 45 mm of grip.
        assert!(nominal.dimensions()[0] > 120. && nominal.dimensions()[2] > 46.);
        assert_eq!(a.known_stock(1), (Some(nominal), false));
        let _ = a.update(Message::SuggestLayout);
        assert_eq!(get(&a.doc.setup().p, "vise_axis"), "Y");
        assert_eq!(get(&a.doc.setup().p, "vise_grip"), "45");
        // The job's own run, as it is done after a pause: setup 2 has no
        // operation to simulate and still gets its box.
        let _ = a.refresh_left_stock();
        assert_eq!(a.left_stock.boxes, [None, Some(left)]);
        assert_eq!(a.known_stock(1), (Some(left), true));
        // The box is 43 mm tall: the grip leaves 1 mm above the jaws.
        let _ = a.update(Message::SuggestLayout);
        assert_eq!(get(&a.doc.setup().p, "vise_axis"), "Y");
        assert_eq!(get(&a.doc.setup().p, "vise_grip"), "42");
        assert_eq!(
            a.status,
            format!("Suggested: vise jaws on ±Y, grip 42 mm. {STOCK_ONLY}")
        );
        // The editor agrees: nothing wrong with what was suggested, and the
        // jaws are drawn on the box that was left.
        assert!(crate::ui::holding_lines(&a).is_empty());
        let jaws = crate::ui::holding_boxes(&a);
        assert_eq!(jaws.len(), 2);
        assert!(
            jaws.iter()
                .all(|j| (j.bounds.max[2] - (left.min[2] + 42.)).abs() < 0.01)
        );
        // Not worked out for this job (an edit since): the original box again.
        let _ = a.update(Message::SelectSetup(0));
        let _ = a.update(Message::SelectOp(0));
        let _ = a.update(Message::Edit(Target::Operation, "bottom", "1".into()));
        let _ = a.update(Message::SelectSetup(1));
        assert_eq!(a.known_stock(1), (Some(nominal), false));
        let _ = a.update(Message::SuggestLayout);
        assert_eq!(get(&a.doc.setup().p, "vise_grip"), "45");
        // Clamps are placed on the same box.
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Clamps".into()));
        assert!(a.status.starts_with("Suggested: "), "{}", a.status);
        assert!(crate::ui::holding_lines(&a).is_empty());
    }
    #[test]
    fn a_later_setup_without_operations_gets_its_box_from_the_job() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = leftover_app(&dir);
        let mesh = a.imported_mesh.clone().unwrap();
        let nominal = a.doc.setup_stock(1, Some(&mesh)).unwrap();
        let left = simulated_stock(&a);
        // A third setup, a copy of the second, and neither has an operation.
        let mut third = a.doc.setups[1].clone();
        third.id = crate::model::id();
        a.doc.setups.push(third);
        for setup in &mut a.doc.setups[1..] {
            setup.operations.clear();
        }
        let _ = a.refresh_left_stock();
        assert_eq!(a.left_stock.boxes, [None, Some(left), Some(left)]);
        for si in [1, 2] {
            assert_eq!(a.known_stock(si), (Some(left), true));
        }
        assert_eq!(crate::ui::stock_note(&a), None);
        let _ = a.update(Message::SelectSetup(2));
        assert_eq!(crate::ui::stock_note(&a), None);
        let _ = a.update(Message::SetupEditor);
        let _ = a.view();
        // Nothing cut by the setup before them: the original block in their frame.
        let mut b = leftover_app(&dir);
        b.doc.setups[0].operations.clear();
        let _ = b.refresh_left_stock();
        let (stock, current) = b.known_stock(1);
        let stock = stock.unwrap();
        assert!(current);
        assert!(
            (0..3).all(|k| (stock.min[k] - nominal.min[k]).abs() < 0.001
                && (stock.max[k] - nominal.max[k]).abs() < 0.001),
            "{stock:?} {nominal:?}"
        );
        assert_eq!(crate::ui::stock_note(&b), None);
        // An earlier setup that fails leaves no box for any later one, those
        // without operations included.
        let mut c = leftover_app(&dir);
        c.doc.setups[0].operations[0].tool = "gone".into();
        c.doc.setups[1].operations.clear();
        let _ = c.refresh_left_stock();
        assert_eq!(c.left_stock.boxes, [None, None]);
        assert!(crate::ui::stock_note(&c).is_some());
    }
    /// The fixture job with its first setup held by the starter clamp, on -X
    /// at `bite` mm onto the stock (12 reaches only the stock's edge, 40 the
    /// pocket), written on the setup directly so that nothing is suggested.
    fn clamped_app(dir: &tempfile::TempDir, bite: &str) -> App {
        let mut a = planning_app(dir);
        let id = first_of(&a, "Clamp");
        a.doc
            .workholding
            .iter_mut()
            .find(|h| h.id == id)
            .unwrap()
            .p
            .insert("bite".into(), bite.into());
        let p = &mut a.doc.setups[0].p;
        for (k, v) in [("holding", "Clamps"), ("holder", &id), ("clamps", "-X:0")] {
            p.insert(k.into(), v.into());
        }
        a
    }
    /// What the preview of the shown operation should say about its own plan.
    fn expected_hits(a: &App) -> Vec<String> {
        let sim = a.active_simulation().unwrap();
        let (setup, mesh) = (a.doc.setup(), a.imported_mesh.as_ref().unwrap());
        let boxes = a.doc.holding_boxes(setup, sim.plan.stock).unwrap();
        let datum = crate::post::datum(setup, sim.plan.stock, mesh).unwrap();
        crate::holding::hits(&sim.plan, &boxes, 2.)
            .iter()
            .map(|hit| crate::holding::message(a.operation().unwrap().name(), hit, 2., datum))
            .collect()
    }
    #[test]
    fn the_status_and_the_notes_say_what_is_checked_only_where_workholding_is_modelled() {
        use crate::ui::{Note, fixture_note, fixtures};
        const NOTES: [Note; 8] = [
            Note::Preview,
            Note::Facing,
            Note::Pocket,
            Note::Entry,
            Note::Separate,
            Note::Panel,
            Note::Passes,
            Note::Playback,
        ];
        // What each note says before the words of `fixtures`, and in full for
        // a setup with no checked workholding.
        let old_notes = [
            (
                Note::Facing,
                "Selections use connected planar regions of the imported mesh. Facing simulation uses a rectangular raster preview. Exact CAD faces and fixture or machine collision checks are not implemented; the moves can be exported as LinuxCNC G-code.",
            ),
            (
                Note::Pocket,
                "Offset contours run from the interior toward the walls at each depth. Entry/full-width cuts can contain both milling modes. No fixture or holder collision solver is connected.",
            ),
            (
                Note::Entry,
                "Entry and depth changes feed down at 25% of cutting feed. Keep-down links use cutting feed within the rectangular preview boundary. Lead-in curves, fixture avoidance and collision checking are not implemented.",
            ),
            (
                Note::Separate,
                "Separate paths retract vertically to clearance, move in XY, then approach at retract height. Entry feed is 25% of cutting feed. No lead arcs, stay-down links, fixture avoidance or holder collision checks are available.",
            ),
            (Note::Panel, "Adaptive stock · no collision checking"),
            (
                Note::Passes,
                "Use stepover up to 50%. Finishing passes repeat the wall/island contours at the requested allowance. Earlier cuts determine the cutting start. A small additional margin (max of 0.02 mm or 1% of cutter radius) covers polygon rounding; exact CAD accuracy and collision verification are not available.",
            ),
            (
                Note::Playback,
                "Generate the operation preview, or press Generate & play. Stock uses adaptive surface meshing and inherits earlier cuts; collision checking is not implemented.",
            ),
        ];
        let dir = tempfile::tempdir().unwrap();
        let mut a = clamped_app(&dir, "40");
        let _ = a.update(Message::SelectOp(1));
        // Modelled and usable: every note the app builds (the views call
        // `fixture_note`, so these are the texts they show) names the vise
        // jaws or clamps, and the strip under the viewport too.
        for note in NOTES.into_iter().filter(|n| *n != Note::Preview) {
            let text = fixture_note(&a, note);
            assert!(text.ends_with(fixtures(note, true)), "{note:?}: {text}");
            assert!(
                old_notes.iter().all(|(n, old)| *n != note || text != *old),
                "{note:?}: {text}"
            );
        }
        assert_eq!(
            a.playback_reason(),
            "Generate the operation preview, or press Generate & play. Stock uses adaptive surface meshing and inherits earlier cuts; collision checks beyond the modelled vise jaws and clamps are not implemented."
        );
        assert_eq!(
            fixture_note(&a, Note::Passes),
            "Use stepover up to 50%. Finishing passes repeat the wall/island contours at the requested allowance. Earlier cuts determine the cutting start. A small additional margin (max of 0.02 mm or 1% of cutter radius) covers polygon rounding; exact CAD accuracy and collision verification beyond the modelled vise jaws and clamps are not available."
        );
        assert_eq!(
            fixture_note(&a, Note::Panel),
            "Adaptive stock · jaws and clamps checked as boxes"
        );
        // The views build with them, on the tabs that carry them.
        for op in [0, 1] {
            let _ = a.update(Message::SelectOp(op));
            for tab in ["Passes", "Linking", "Geometry", "Tool"] {
                let _ = a.update(Message::Tab(tab.into()));
                let _ = a.view();
            }
        }
        let _ = a.update(Message::SelectOp(1));
        let _ = a.update(Message::Generate);
        assert!(a.playback_ready(), "{}", a.status);
        assert!(
            a.status.contains(
                "Sampled removal; this operation's own toolpath is checked against the vise jaws or clamps as boxes with a clearance gap (a warning shows if it comes too close); parallels, bolts, the table and the machine body are not. Export G-code from the NC review page."
            ),
            "{}",
            a.status
        );
        assert!(
            !a.status.contains("fixture and machine collisions"),
            "{}",
            a.status
        );
        // Workholding that cannot be checked is not claimed: the old wording
        // everywhere, as for None.
        let old = |a: &App| {
            for (note, text) in &old_notes {
                assert_eq!(fixture_note(a, *note), *text, "{note:?}");
            }
        };
        let id = get(&a.doc.setups[0].p, "holder").to_string();
        a.doc.workholding.retain(|h| h.id != id);
        old(&a);
        let _ = a.update(Message::Generate);
        assert!(
            a.status.contains(
                "Sampled removal; fixture and machine collisions are not checked. Export G-code"
            ),
            "{}",
            a.status
        );
        assert_eq!(
            a.playback_reason(),
            "Generate the operation preview, or press Generate & play. Stock uses adaptive surface meshing and inherits earlier cuts; collision checking is not implemented."
        );
        // None keeps the old wording too.
        a.doc.setups[0].p.insert("holding".into(), "None".into());
        old(&a);
        let _ = a.update(Message::Generate);
        assert!(
            a.status.contains("Sampled removal; fixture and machine collisions are not checked. Export G-code from the NC review page."),
            "{}",
            a.status
        );
        for tab in ["Passes", "Linking"] {
            let _ = a.update(Message::Tab(tab.into()));
            let _ = a.view();
        }
        // The words of each note: the old ones for no modelled workholding,
        // the others name the vise jaws or clamps, no more.
        assert_eq!(
            fixtures(Note::Facing, false),
            "Exact CAD faces and fixture or machine collision checks are not implemented; the moves can be exported as LinuxCNC G-code."
        );
        assert_eq!(
            fixtures(Note::Pocket, false),
            "No fixture or holder collision solver is connected."
        );
        assert_eq!(
            fixtures(Note::Entry, false),
            "Lead-in curves, fixture avoidance and collision checking are not implemented."
        );
        assert_eq!(
            fixtures(Note::Separate, false),
            "No lead arcs, stay-down links, fixture avoidance or holder collision checks are available."
        );
        for note in NOTES {
            let held = fixtures(note, true);
            assert_ne!(held, fixtures(note, false), "{note:?}");
            assert!(
                held.contains("vise jaws or clamps") || held.contains("jaws and clamps"),
                "{note:?}: {held}"
            );
            for claim in ["safe", "no collision", "all fixtures"] {
                assert!(!held.contains(claim), "{note:?}: {held}");
            }
        }
        assert!(
            fixtures(Note::Facing, true)
                .contains("parallels, bolts, the table and the machine body are not")
        );
    }
    #[test]
    fn the_nc_page_note_counts_only_workholding_that_can_be_checked() {
        use crate::holding::Held;
        use crate::ui::{held, program_note};
        let dir = tempfile::tempdir().unwrap();
        let mut a = clamped_app(&dir, "12");
        assert_eq!(held(&a), Held::Every);
        assert!(program_note(held(&a)).contains("Every program move"));
        // A second setup with the same clamps: still every setup.
        let _ = a.update(Message::DuplicateSetup);
        assert_eq!(a.doc.setups.len(), 2);
        assert_eq!(held(&a), Held::Every);
        // One whose entry is gone is not checked: it counts with those that
        // model nothing, and the note says so.
        a.doc.setups[1].p.insert("holder".into(), "gone".into());
        assert_eq!(held(&a), Held::Mixed);
        let mixed = program_note(held(&a));
        assert!(
            mixed.contains("the setups whose workholding is not modelled or cannot be checked"),
            "{mixed}"
        );
        // None of them can be checked: the old wording, which claims nothing.
        let id = get(&a.doc.setups[0].p, "holder").to_string();
        a.doc.workholding.retain(|h| h.id != id);
        assert_eq!(held(&a), Held::No);
        let old = program_note(held(&a));
        assert!(
            old.ends_with("Fixtures, clamps and the machine body are not checked: preview each program in LinuxCNC and air-cut first."),
            "{old}"
        );
        // A clamp text that does not read cannot be checked either.
        let other = tempfile::tempdir().unwrap();
        let mut b = clamped_app(&other, "12");
        b.doc.setups[0].p.insert("clamps".into(), "-X".into());
        assert_eq!(held(&b), Held::No);
    }
    #[test]
    fn the_operation_preview_warns_when_the_toolpath_comes_within_the_gap() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = clamped_app(&dir, "40");
        assert_eq!(a.doc.setups[0].operations[1].name(), "Pocket");
        let _ = a.update(Message::SelectOp(1));
        // Nothing is said before the operation is generated.
        assert_eq!(crate::ui::holding_warning(&a), None);
        let _ = a.view();
        let _ = a.update(Message::Generate);
        assert!(a.playback_ready(), "{}", a.status);
        let expected = expected_hits(&a);
        assert_eq!(expected.len(), 1);
        assert!(
            expected[0].starts_with("Pocket: the tool hits the clamp on -X (offset 0) at X-22.0 Y"),
            "{expected:?}"
        );
        assert_eq!(
            crate::ui::holding_warning(&a),
            Some((
                "Too close to the workholding (export will refuse this)".to_string(),
                expected.clone()
            ))
        );
        // The lines are worked out once for a preview, its plan and the
        // workholding, then served from the app's cache; any of them changing
        // (the gap, the library entry) works them out again.
        let key = a.holding_note.borrow().clone().unwrap().0;
        let cached = Some(("from the cache".to_string(), vec![]));
        *a.holding_note.borrow_mut() = Some((key, cached.clone()));
        assert_eq!(crate::ui::holding_warning(&a), cached);
        assert_eq!(crate::ui::holding_warning(&a), cached);
        a.doc.setups[0].p.insert("holding_gap".into(), "3".into());
        let (_, lines) = crate::ui::holding_warning(&a).unwrap();
        assert!(lines[0].ends_with("(within the 3 mm gap)."), "{lines:?}");
        a.doc.setups[0].p.insert("holding_gap".into(), "2".into());
        assert_eq!(crate::ui::holding_warning(&a).unwrap().1, expected);
        let key = a.holding_note.borrow().clone().unwrap().0;
        *a.holding_note.borrow_mut() = Some((key, cached.clone()));
        let holder = get(&a.doc.setups[0].p, "holder").to_string();
        a.doc
            .workholding
            .iter_mut()
            .find(|h| h.id == holder)
            .unwrap()
            .p
            .insert("bite".into(), "41".into());
        assert_ne!(crate::ui::holding_warning(&a), cached);
        a.doc
            .workholding
            .iter_mut()
            .find(|h| h.id == holder)
            .unwrap()
            .p
            .insert("bite".into(), "40".into());
        // When the work zero cannot be told the lines are in the setup frame,
        // and the heading says so.
        let origin = get(&a.doc.setups[0].p, "origin").to_string();
        a.doc.setups[0]
            .p
            .insert("origin".into(), "Selected point (planned)".into());
        let (heading, lines) = crate::ui::holding_warning(&a).unwrap();
        assert_eq!(
            heading,
            "Too close to the workholding (export will refuse this; coordinates in the setup frame)"
        );
        let plan = &a.active_simulation().unwrap().plan;
        let boxes = a.doc.holding_boxes(a.doc.setup(), plan.stock).unwrap();
        let in_frame: Vec<String> = crate::holding::hits(plan, &boxes, 2.)
            .iter()
            .map(|hit| crate::holding::message("Pocket", hit, 2., [0.; 3]))
            .collect();
        assert_eq!(lines, in_frame);
        assert_ne!(lines, expected);
        let _ = a.view();
        a.doc.setups[0].p.insert("origin".into(), origin);
        assert_eq!(crate::ui::holding_warning(&a).unwrap().1, expected);
        for tab in ["Tool", "Heights", "Passes", "Linking"] {
            let _ = a.update(Message::Tab(tab.into()));
            assert!(crate::ui::holding_warning(&a).is_some(), "{tab}");
            let _ = a.view();
        }
        // The warning never blocks: the preview is still there to play.
        assert!(a.playback_ready());
        // Picking geometry drops the preview, and the warning with it.
        let _ = a.update(Message::Tab("Geometry".into()));
        assert_eq!(crate::ui::holding_warning(&a), None);
        let _ = a.view();
        // Another operation shown, or the operation edited, takes it away.
        let _ = a.update(Message::SelectOp(0));
        assert_eq!(crate::ui::holding_warning(&a), None);
        let _ = a.update(Message::SelectOp(1));
        let _ = a.update(Message::Generate);
        assert!(crate::ui::holding_warning(&a).is_some());
        let _ = a.update(Message::Edit(Target::Operation, "rpm", "7000".into()));
        assert_eq!(crate::ui::holding_warning(&a), None);
        // Moved clear of the pocket, the clamp is not in the way.
        let _ = a.update(Message::Generate);
        a.doc.setups[0].p.insert("clamps".into(), "-X:42".into());
        assert_eq!(crate::ui::holding_warning(&a), None);
        let _ = a.view();
        // A setup that cannot be modelled says the check did not run, with the editor's line.
        a.doc.setups[0].p.insert("holder".into(), "gone".into());
        assert_eq!(
            crate::ui::holding_warning(&a),
            Some((
                "Workholding cannot be checked; export will refuse this setup until it is fixed"
                    .to_string(),
                vec![crate::holding::DELETED.to_string()]
            ))
        );
        let _ = a.view();
        // Without workholding there is nothing to say.
        a.doc.setups[0].p.insert("holding".into(), "None".into());
        assert_eq!(crate::ui::holding_warning(&a), None);
        let _ = a.view();
    }
    /// LIO-143: a generated operation showed a green check that Run then
    /// refused for the clamps. Green now means the Run panel's own checks pass.
    #[test]
    fn the_browser_marks_an_operation_ready_only_when_the_run_checks_pass() {
        use crate::ui::{OpMark, op_mark, run_check};
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app_on(dir.path(), LIMITED);
        a.doc.adopt_holders();
        a.smoke_dir = Some(dir.path().to_path_buf());
        a.page = Page::Work;
        let marks =
            |a: &App| -> Vec<OpMark> { (0..2).map(|i| op_mark(a, a.current_ready(), i)).collect() };
        // The suggested toe clamps, as the reviewer had them.
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Clamps".into()));
        let _ = a.update(Message::SelectOp(0));
        let _ = a.update(Message::Generate);
        assert!(a.playback_ready(), "{}", a.status);
        // Generated and not yet checked: grey, not the green of ready.
        assert_eq!(marks(&a), [OpMark::Checking, OpMark::Pending]);
        assert_eq!(run_check(&a).unwrap().0, "Checking Top as run will…");
        let _ = a.refresh_ready();
        let refused = crate::run::programs(&a.doc, a.part_mesh().unwrap(), 0).unwrap_err();
        assert_eq!(a.current_ready().unwrap().problems, refused);
        assert_eq!(marks(&a), [OpMark::Collides, OpMark::Collides]);
        let (heading, lines, ok) = run_check(&a).unwrap();
        assert_eq!(heading, "Workholding problem: Run will refuse Top");
        assert!(!ok);
        let hit = format!(
            "{}: the tool hits the clamp on",
            a.doc.setups[0].operations[0].name()
        );
        assert!(lines[0].starts_with(&hit), "{lines:?}");
        assert!(
            refused.iter().all(|r| lines.contains(r)),
            "{lines:?} {refused:?}"
        );
        let _ = a.view();
        // The suggested vise instead: out of date until checked again, then
        // green, and Run agrees.
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        assert!(a.current_ready().is_none());
        let _ = a.refresh_ready();
        assert!(crate::run::programs(&a.doc, a.part_mesh().unwrap(), 0).is_ok());
        assert_eq!(marks(&a), [OpMark::Ready, OpMark::Ready]);
        let (heading, _, ok) = run_check(&a).unwrap();
        assert_eq!(heading, "Ready to run: Run's checks of Top pass.");
        assert!(ok);
        let _ = a.view();
        // Coolant the profile has no output for: generated, not ready.
        let _ = a.update(Message::Edit(Target::Operation, "coolant", "Mist".into()));
        let _ = a.refresh_ready();
        assert_eq!(marks(&a), [OpMark::Generated, OpMark::Generated]);
        let (heading, lines, ok) = run_check(&a).unwrap();
        assert_eq!(heading, "Run will refuse Top");
        assert!(!ok);
        assert!(
            lines[0].contains("uses mist coolant, but the UC100 profile has no mist output"),
            "{lines:?}"
        );
    }
    /// #74: a grey and a green ✓ differed by colour only, and the hover help
    /// did not explain the mark.
    #[test]
    fn each_browser_mark_has_its_own_glyph_and_hover_help_that_names_the_state() {
        use crate::ui::{OpMark, mark_help, op_mark};
        let mut glyphs: Vec<&str> = OpMark::ALL.iter().map(|m| m.glyph()).collect();
        glyphs.sort();
        glyphs.dedup();
        assert_eq!(glyphs.len(), OpMark::ALL.len(), "{glyphs:?}");
        let mut states: Vec<&str> = OpMark::ALL.iter().map(|m| m.state()).collect();
        states.sort();
        states.dedup();
        assert_eq!(states.len(), OpMark::ALL.len(), "{states:?}");
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app_on(dir.path(), LIMITED);
        a.doc.adopt_holders();
        a.smoke_dir = Some(dir.path().to_path_buf());
        a.page = Page::Work;
        let help = |a: &App, i: usize| {
            let mark = op_mark(a, a.current_ready(), i);
            (mark, mark_help(a, a.current_ready(), i, mark))
        };
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Clamps".into()));
        let _ = a.update(Message::SelectOp(0));
        let _ = a.update(Message::Generate);
        // Generated, checks not in yet.
        let (mark, text) = help(&a, 0);
        assert_eq!(mark, OpMark::Checking);
        assert!(
            text.contains("…") && text.contains("Generated, being checked"),
            "{text}"
        );
        let (mark, text) = help(&a, 1);
        assert_eq!(mark, OpMark::Pending);
        assert!(
            text.contains("○") && text.contains("Not generated"),
            "{text}"
        );
        // Clamps in the way: the Run panel's own lines.
        let _ = a.refresh_ready();
        let refused = crate::run::programs(&a.doc, a.part_mesh().unwrap(), 0).unwrap_err();
        let (mark, text) = help(&a, 0);
        assert_eq!(mark, OpMark::Collides);
        assert!(
            text.contains("!") && text.contains("Workholding refused"),
            "{text}"
        );
        assert!(text.contains("Run will refuse Top"), "{text}");
        assert!(
            text.contains(&refused[0]) || text.contains("tool hits the clamp"),
            "{text}"
        );
        // Vise: ready.
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        let _ = a.refresh_ready();
        let (mark, text) = help(&a, 0);
        assert_eq!(mark, OpMark::Ready);
        assert!(
            text.contains("✓") && text.contains("Ready to run"),
            "{text}"
        );
        // Mist coolant: generated, but Run refuses, with the reason.
        let _ = a.update(Message::Edit(Target::Operation, "coolant", "Mist".into()));
        let _ = a.refresh_ready();
        let (mark, text) = help(&a, 0);
        assert_eq!(mark, OpMark::Generated);
        assert!(text.contains("Generated, will not run"), "{text}");
        assert!(
            text.contains("uses mist coolant, but the UC100 profile has no mist output"),
            "{text}"
        );
        // Suppressed.
        a.doc.active_setup_mut().unwrap().operations[1].enabled = false;
        let (mark, text) = help(&a, 1);
        assert_eq!(mark, OpMark::Suppressed);
        assert!(text.contains("—") && text.contains("Suppressed"), "{text}");
        let _ = a.view();
    }
    #[test]
    fn the_setup_editor_and_the_run_checks_say_whether_the_vise_holds_against_the_cut() {
        use crate::ui::{OpMark, holding_lines, holding_margins, op_mark};
        let dir = tempfile::tempdir().unwrap();
        let mut a = run_app_on(dir.path(), LIMITED);
        a.doc.adopt_holders();
        a.smoke_dir = Some(dir.path().to_path_buf());
        a.page = Page::Work;
        let checked = |a: &mut App| {
            let _ = a.refresh_left_stock();
            let _ = a.refresh_ready();
            let _ = a.view();
        };
        // The suggested vise holds: a margin for each operation, no issue.
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        checked(&mut a);
        assert!(holding_lines(&a).is_empty(), "{:?}", holding_lines(&a));
        let margins = holding_margins(&a);
        assert_eq!(margins.len(), 2, "{margins:?}");
        assert!(margins[0].starts_with("Facing: holds ×"), "{margins:?}");
        assert_eq!(
            margins[1],
            "Drilling: no cutting force sideways or up to hold against."
        );
        assert_eq!(op_mark(&a, a.current_ready(), 0), OpMark::Ready);
        // Gripped 1 mm deep, the facing cut tips the stock out of the jaws.
        let _ = a.update(Message::Edit(Target::Setup, "vise_grip", "1".into()));
        checked(&mut a);
        let lines = holding_lines(&a);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("Facing: the cut could tip the stock out of the vise"),
            "{lines:?}"
        );
        assert!(holding_margins(&a)[0].starts_with("Facing: only ×"));
        assert_eq!(op_mark(&a, a.current_ready(), 0), OpMark::Collides);
        assert_eq!(op_mark(&a, a.current_ready(), 1), OpMark::Generated);
    }
    #[test]
    fn the_3d_view_gets_the_boxes_only_when_the_stock_is_shown() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = planning_app(&dir);
        assert!(a.stock);
        assert!(crate::ui::holding_boxes(&a).is_empty());
        let vise = first_of(&a, "Vise");
        let p = &mut a.doc.setups[0].p;
        for (k, v) in [
            ("holding", "Vise"),
            ("holder", &vise),
            ("vise_axis", "Y"),
            ("vise_grip", "8"),
        ] {
            p.insert(k.into(), v.into());
        }
        let boxes = crate::ui::holding_boxes(&a);
        let labels: Vec<_> = boxes.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(labels, ["vise jaw on -Y", "vise jaw on +Y"]);
        // They are the jaws round the setup's stock, without the gap, drawn
        // down to the stock's bottom (the check models them far deeper).
        let stock = a.doc.setup_stock(0, a.part_mesh()).unwrap();
        let layout = crate::holding::layout(a.doc.setup()).unwrap();
        let holder = a.doc.holder(&vise).unwrap();
        let whole = crate::holding::boxes(&layout, holder, stock).unwrap();
        assert_eq!(boxes, crate::holding::shown(&whole, stock));
        assert_eq!(boxes[1].bounds.min[1], stock.max[1]);
        assert!(whole.iter().all(|b| b.bounds.min[2] < stock.min[2] - 50.));
        assert!(boxes.iter().all(|b| b.bounds.min[2] == stock.min[2]));
        let _ = a.view();
        // With a preview shown the plan's stock is the one used.
        let _ = a.update(Message::SelectOp(1));
        let _ = a.update(Message::Generate);
        assert!(a.active_simulation().is_some(), "{}", a.status);
        assert_eq!(crate::ui::holding_boxes(&a), boxes);
        let _ = a.view();
        // Hidden with the stock; not drawn when the setup cannot be modelled.
        let _ = a.update(Message::ToggleStock);
        assert!(!a.stock);
        assert!(crate::ui::holding_boxes(&a).is_empty());
        let _ = a.view();
        let _ = a.update(Message::ToggleStock);
        assert_eq!(crate::ui::holding_boxes(&a), boxes);
        a.doc.setups[0].p.insert("holder".into(), "gone".into());
        assert!(crate::ui::holding_boxes(&a).is_empty());
        let _ = a.view();
        a.doc.setups[0].p.insert("holding".into(), "None".into());
        assert!(crate::ui::holding_boxes(&a).is_empty());
        // Clamps: one box each, on the sides asked for.
        let clamp = first_of(&a, "Clamp");
        for (k, v) in [
            ("holding", "Clamps"),
            ("holder", clamp.as_str()),
            ("clamps", "-X:0;+Y:10"),
        ] {
            a.doc.setups[0].p.insert(k.into(), v.into());
        }
        let labels: Vec<_> = crate::ui::holding_boxes(&a)
            .into_iter()
            .map(|b| b.label)
            .collect();
        assert_eq!(
            labels,
            ["clamp on -X (offset 0)", "clamp on +Y (offset 10)"]
        );
        let _ = a.view();
    }
    #[test]
    fn the_nc_page_lists_what_a_modelled_setup_hits_and_leaves_other_setups_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = clamped_app(&dir, "40");
        a.smoke_dir = Some(dir.path().to_path_buf());
        let _ = a.update(Message::Page(Page::Post));
        let job = a.post.clone().unwrap();
        assert!(!job.ready());
        let problems = job.setups[0].result.as_ref().unwrap_err();
        // The operations that come within the gap are named, at most the tool
        // and the holder of each.
        let named: Vec<_> = problems
            .iter()
            .map(|p| p.split(':').next().unwrap())
            .collect();
        assert!(
            named.contains(&"Pocket") && named.contains(&"Facing"),
            "{problems:?}"
        );
        assert!(
            problems
                .iter()
                .all(|p| p.ends_with("(within the 2 mm gap)."))
        );
        for name in &named {
            assert!(
                named.iter().filter(|n| *n == name).count() <= 2,
                "{problems:?}"
            );
        }
        let _ = a.view();
        // The same job with no workholding exports.
        a.doc.setups[0].p.insert("holding".into(), "None".into());
        a.invalidate_post();
        let _ = a.update(Message::Page(Page::Post));
        assert!(a.post.as_ref().is_some_and(|job| job.ready()));
        let _ = a.view();
    }
    #[test]
    fn undo_brings_back_a_workholding_entry_deleted_after_apply() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let vise = first_of(&a, "Vise");
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        assert_eq!(get(&a.doc.setups[0].p, "holder"), vise);
        apply_checked_plan_of(&mut a, held_in_a_vise);
        // The applied job's setups all keep the vise (the planner copies
        // setup 1's workholding to the later ones); once none of them models
        // one, the entry can be deleted.
        let _ = a.update(Message::DeleteHolder(vise.clone()));
        assert!(a.doc.holder(&vise).is_some(), "{}", a.status);
        for setup in &mut a.doc.setups {
            setup.p.insert("holding".into(), "None".into());
        }
        let _ = a.update(Message::DeleteHolder(vise.clone()));
        assert!(a.doc.holder(&vise).is_none(), "{}", a.status);
        let _ = a.update(Message::UndoPlan);
        assert_eq!(
            a.doc.workholding.iter().filter(|h| h.id == vise).count(),
            1,
            "the vise is back once"
        );
        assert_eq!(get(&a.doc.setups[0].p, "holder"), vise);
        assert!(a.doc.holding_issues(0, None).is_empty());
        assert!(a.doc.validate_structure().is_ok());
        assert!(
            a.status
                .ends_with(" 100 mm machine vise is back in the workholding library."),
            "{}",
            a.status
        );
        // An entry that was never deleted is not added again.
        let mut a = signed_in_planning_app(&dir);
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        let count = a.doc.workholding.len();
        apply_checked_plan_of(&mut a, held_in_a_vise);
        let _ = a.update(Message::UndoPlan);
        assert_eq!(a.doc.workholding.len(), count);
        assert!(!a.status.contains("workholding library"), "{}", a.status);
        // A name another entry has taken since is made new.
        let mut a = signed_in_planning_app(&dir);
        let vise = first_of(&a, "Vise");
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        apply_checked_plan_of(&mut a, held_in_a_vise);
        for setup in &mut a.doc.setups {
            setup.p.insert("holding".into(), "None".into());
        }
        let _ = a.update(Message::DeleteHolder(vise.clone()));
        let _ = a.update(Message::NewHolder("Vise"));
        edit_holder(
            &mut a,
            &[
                ("name", "100 MM machine vise"),
                ("jaw_width", "100"),
                ("jaw_height", "30"),
                ("jaw_thickness", "20"),
                ("max_opening", "120"),
                ("grip", "8"),
            ],
        );
        let _ = a.update(Message::SaveHolder);
        let _ = a.update(Message::UndoPlan);
        let back = a.doc.holder(&vise).unwrap();
        assert_eq!(back.name(), "100 mm machine vise (restored)");
        assert!(back.issues().is_empty());
        assert!(a.doc.holder_name_issues(back).is_empty());
        assert!(
            a.status
                .ends_with(" 100 mm machine vise (restored) is back in the workholding library."),
            "{}",
            a.status
        );
    }
    #[test]
    fn apply_brings_back_a_workholding_entry_deleted_after_the_preview() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let vise = first_of(&a, "Vise");
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        let show = |a: &mut App| {
            let preview = checked_preview_of(a, false, held_in_a_vise);
            let _ = a.update(Message::Plan);
            let epoch = a.plan_epoch;
            let _ = a.update(Message::PlanFinished(epoch, Ok(preview)));
            assert!(matches!(a.planner, crate::Planner::Preview(_)));
        };
        show(&mut a);
        // The entry is gone from the library when the plan is applied. (The
        // Workholding page touches the job when it deletes one, which
        // discards the preview; this is an entry lost some other way.)
        a.doc.setups[0].p.insert("holding".into(), "None".into());
        a.doc.workholding.retain(|h| h.id != vise);
        assert!(a.doc.holder(&vise).is_none());
        let _ = a.update(Message::ApplyPlan);
        assert!(a.undo.is_some(), "{}", a.status);
        assert_eq!(
            a.doc.workholding.iter().filter(|h| h.id == vise).count(),
            1,
            "the vise is back once, with its own id"
        );
        assert!(a.doc.setups.iter().all(|s| get(&s.p, "holder") == vise));
        for si in 0..a.doc.setups.len() {
            assert!(a.doc.holding_issues(si, None).is_empty(), "setup {si}");
        }
        assert!(a.doc.validate_structure().is_ok());
        assert!(
            a.status
                .ends_with(" 100 mm machine vise is back in the workholding library."),
            "{}",
            a.status
        );
        // Undo keeps what Apply brought back, and says nothing more of it.
        let _ = a.update(Message::UndoPlan);
        assert_eq!(a.doc.workholding.iter().filter(|h| h.id == vise).count(), 1);
        // An entry that was not deleted is not added again, and a name that
        // another entry has taken since is made new.
        let mut a = signed_in_planning_app(&dir);
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        let count = a.doc.workholding.len();
        show(&mut a);
        let _ = a.update(Message::ApplyPlan);
        assert_eq!(a.doc.workholding.len(), count);
        assert!(!a.status.contains("workholding library"), "{}", a.status);
        let mut a = signed_in_planning_app(&dir);
        let vise = first_of(&a, "Vise");
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Vise".into()));
        show(&mut a);
        a.doc.setups[0].p.insert("holding".into(), "None".into());
        a.doc.workholding.retain(|h| h.id != vise);
        let mut taken = crate::holding::starters().remove(0);
        taken.p.insert("name".into(), "100 MM machine vise".into());
        a.doc.workholding.push(taken);
        let _ = a.update(Message::ApplyPlan);
        let back = a.doc.holder(&vise).unwrap();
        assert_eq!(back.name(), "100 mm machine vise (restored)");
        assert!(a.doc.holder_name_issues(back).is_empty());
        assert!(a.doc.validate_structure().is_ok());
        assert!(
            a.status
                .ends_with(" 100 mm machine vise (restored) is back in the workholding library."),
            "{}",
            a.status
        );
    }
    #[test]
    fn a_replaced_project_forgets_the_typed_clamp_text() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        let _ = a.update(Message::Edit(Target::Setup, "holding", "Clamps".into()));
        let _ = a.update(Message::SetClamps("-X".into()));
        assert!(a.clamps_draft.is_some());
        a.replace_document(Document::sample(), None);
        assert!(a.clamps_draft.is_none());
        assert_eq!(a.holder_draft.name(), "New vise");
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
            let _ = a.update(Message::SignedIn(
                a.sign_in_attempt,
                Ok((session.clone(), "oaiapp_1".into())),
            ));
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
        let _ = a.update(Message::SignedIn(
            a.sign_in_attempt,
            Ok((crate::session::sample(), "oaiapp_1".into())),
        ));
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
    #[test]
    fn a_model_import_in_flight_ends_the_app_on_close() {
        use crate::CloseAction::*;
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        let id = iced::window::Id::unique();
        let now = std::time::Instant::now();
        assert_eq!(a.close_decision(id, now), Close);
        // The conversion runs under spawn_blocking, which the runtime would
        // wait for (up to two minutes for a STEP file) after the window is gone.
        a.import_busy = true;
        assert!(a.blocking_work());
        assert_eq!(a.close_decision(id, now), Exit);
        // A G-code export still finishes first.
        a.gcode_exports = 1;
        assert_eq!(a.close_decision(id, now), Close);
    }
    #[test]
    fn importing_from_the_start_screen_keeps_the_project_until_a_file_is_chosen() {
        let dir = tempfile::tempdir().unwrap();
        let history = || std::fs::read_dir(dir.path().join("history")).map_or(0, |d| d.count());
        let mut a = fresh_app(&dir);
        a.doc.title = "Resumable job".into();
        a.page = Page::Home;
        let _ = a.update(Message::Import);
        assert!(a.import_busy);
        assert_eq!((a.page, history()), (Page::Home, 0));
        // The dialog is cancelled: the job is still there to continue.
        let _ = a.update(Message::FileImported(None));
        assert!(!a.import_busy);
        assert_eq!(
            (a.page, a.doc.title.as_str(), history()),
            (Page::Home, "Resumable job", 0)
        );
        let _ = a.update(Message::ResumeProject);
        assert_eq!(
            (a.page, a.doc.title.as_str()),
            (Page::Work, "Resumable job")
        );
        // A chosen file starts the new project, the old one going to History.
        a.page = Page::Home;
        let _ = a.update(Message::Import);
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/preview-cube.obj");
        let mesh = crate::mesh::load(&path).unwrap();
        let info = crate::model::ModelFile {
            name: "preview-cube.obj".into(),
            path: path.clone(),
            format: "obj".into(),
            bytes: std::fs::metadata(&path).unwrap().len(),
        };
        let _ = a.update(Message::FileImported(Some(Ok((info, Some(mesh))))));
        assert!(!a.import_busy);
        assert_eq!(
            (a.page, a.doc.title.as_str(), history()),
            (Page::Work, "preview-cube", 1)
        );
        assert_eq!(a.doc.file.as_ref().unwrap().path, path);
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
                        messages: vec![crate::openai::Item::text(
                            crate::openai::Role::User,
                            "Part.",
                        )],
                        schema_name: "plan",
                        schema: serde_json::json!({"type": "object"}),
                        ..Default::default()
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
        // Each detour to make what an action needs comes back and finishes
        // it (LIO-147): saving the machine creates the setup, saving the
        // cutter adds Facing.
        let _ = a.update(Message::AddSetup);
        assert_eq!(a.page, Page::Wizard);
        assert_eq!(
            a.resume_label().as_deref(),
            Some("Save and create first setup")
        );
        let _ = a.update(Message::SaveMachine);
        assert_eq!(a.doc.setups.len(), 1);
        assert_eq!(a.page, Page::Work);
        assert!(a.doc.setup().operations.is_empty());
        assert_eq!(get(&a.doc.setup().p, "stock_x"), "");
        let _ = a.update(Message::AddOp(Strategy::Face));
        assert!(a.doc.setup().operations.is_empty());
        assert_eq!(a.page, Page::Tools);
        let _ = a.update(Message::NewTool);
        assert_eq!(
            a.resume_label().as_deref(),
            Some("Save and continue to Facing")
        );
        let _view = a.view();
        drop(_view);
        let _ = a.update(Message::SaveTool);
        assert_eq!(a.doc.setup().operations.len(), 1);
        assert_eq!(a.doc.setup().operations[0].strategy, Strategy::Face);
        assert_eq!(a.page, Page::Work);
        assert!(a.pending.is_none());
        assert!(a.operation().unwrap().geometry.is_empty());
        // Leaving the page drops the action: a later save does not add it.
        let _ = a.update(Message::RemoveOp);
        a.doc.tools.clear();
        let _ = a.update(Message::AddOp(Strategy::Face));
        let _ = a.update(Message::Page(Page::Work));
        let _ = a.update(Message::Page(Page::Tools));
        let _ = a.update(Message::NewTool);
        let _ = a.update(Message::SaveTool);
        assert!(a.doc.setup().operations.is_empty());
        assert_eq!(a.page, Page::Tools);
        // A library with one machine creates a further first setup with it.
        let _ = a.update(Message::ConfirmDeleteSetup(true));
        let _ = a.update(Message::AddSetup);
        assert_eq!(a.doc.setups.len(), 1);
        assert_eq!(a.page, Page::Work);
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
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        a.test_dir = Some(dir);
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
            path: path.clone(),
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
    fn a_contour_through_the_stock_bottom_warns_until_it_leaves_tabs() {
        let mut a = imported_app();
        // The Contour alone, on fresh stock.
        for o in &mut a.doc.setup_mut().operations {
            o.enabled = false;
        }
        let _ = a.update(Message::AddOp(Strategy::Contour));
        let contour = (a.doc.setup().operations.iter())
            .rposition(|o| o.strategy == Strategy::Contour)
            .unwrap();
        let _ = a.update(Message::SelectOp(contour));
        let _ = a.update(Message::Edit(Target::Setup, "stock_bottom", "0".into()));
        // A tool that reaches down the whole part.
        let tool = a.operation().unwrap().tool.clone();
        let tool = a.doc.tools.iter_mut().find(|t| t.id == tool).unwrap();
        tool.p.insert("length".into(), "80".into());
        tool.p.insert("stickout".into(), "90".into());
        // A project saved before tabs: none of their settings.
        for key in ["tabs", "tab_count", "tab_width", "tab_height"] {
            a.operation_mut().unwrap().p.remove(key);
        }
        let _ = a.update(Message::Generate);
        assert!(a.active_simulation().is_some(), "{}", a.status);
        let loose = |a: &App| {
            crate::ui::op_checks(a)
                .into_iter()
                .find(|c| c.text.contains("the part comes loose"))
        };
        let warning = loose(&a).unwrap_or_else(|| panic!("{:?}", crate::ui::op_checks(&a)));
        assert_eq!((warning.error, warning.key), (false, Some("bottom")));
        let _ = a.update(Message::Tab("Heights".into()));
        let texts = page_texts(&a);
        assert!(texts.contains(&"Holding tabs".to_string()), "{texts:?}");
        assert!(!texts.contains(&"Tab width · mm".to_string()), "{texts:?}");
        // Turning tabs on fills in their sizes and drops the old plan.
        let _ = a.update(Message::Edit(Target::Operation, "tabs", "Yes".into()));
        assert!(a.simulation.is_none());
        let p = &a.operation().unwrap().p;
        for (key, value) in crate::model::TAB_DEFAULTS {
            assert_eq!(get(p, key), value);
        }
        let texts = page_texts(&a);
        for label in ["Tabs on each outline", "Tab width · mm", "Tab height · mm"] {
            assert!(texts.contains(&label.to_string()), "{label}: {texts:?}");
        }
        // The import leaves 5 mm of stock beside the part, less than the
        // Ø10 cut: the tabs would join the part to nothing.
        let _ = a.update(Message::Generate);
        assert!(a.active_simulation().is_none());
        assert!(
            a.status.contains("4 of 4 tabs join the part to no stock")
                && a.status.contains("at least 12.4 mm"),
            "{}",
            a.status
        );
        for key in ["stock_left", "stock_right", "stock_front", "stock_back"] {
            let _ = a.update(Message::Edit(Target::Setup, key, "15".into()));
        }
        let _ = a.update(Message::Generate);
        let plan = &a
            .active_simulation()
            .unwrap_or_else(|| panic!("{}", a.status))
            .plan;
        let rises = (plan.moves.iter())
            .filter(|m| m.cutting_pass && m.from[..2] == m.to[..2] && m.to[2] > m.from[2])
            .count();
        assert!(rises >= 4, "{rises}");
        assert!(loose(&a).is_none());
        // A bad size is an error at its field, on Heights.
        let _ = a.update(Message::Edit(Target::Operation, "tab_count", "0".into()));
        let error = (crate::ui::op_checks(&a).into_iter())
            .find(|c| c.key == Some("tab_count"))
            .unwrap_or_else(|| panic!("{:?}", crate::ui::op_checks(&a)));
        assert!(error.error);
        assert_eq!(crate::ui::issue_tab(&error.text), Some("Heights"));
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
    /// The strip names the wait at normal size, with the time it has taken,
    /// while the mesher runs, and keeps the cell size and triangle count in
    /// a details line (#83).
    #[test]
    fn the_simulation_strip_says_the_stock_surface_is_updating_while_the_mesher_runs() {
        let mut a = imported_app();
        let top = a
            .imported_mesh
            .as_ref()
            .unwrap()
            .surfaces
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.center[2].total_cmp(&b.center[2]))
            .unwrap()
            .0;
        let _ = a.update(Message::Surface(top));
        let _ = a.update(Message::Generate);
        assert!(a.active_simulation().is_some(), "{}", a.status);
        // No surface and no job yet: nothing is updating, the details say it is awaited.
        assert_eq!(crate::ui::stock_surface_notice(&a), None);
        assert_eq!(
            crate::ui::stock_surface_details(&a).as_deref(),
            Some("Building stock surface…")
        );
        let mesh = std::sync::Arc::new(crate::stock_surface::SurfaceMesh {
            vertices: vec![],
            triangles: vec![],
            cell_mm: 0.5,
            milliseconds: 0,
        });
        let first = a.stock_mesh_key().unwrap();
        let _ = a.update(Message::StockMeshed {
            key: first,
            seconds: 0.,
            result: Ok(mesh.clone()),
        });
        assert_eq!(crate::ui::stock_surface_notice(&a), None);
        assert_eq!(
            crate::ui::stock_surface_details(&a).as_deref(),
            Some("0.50 mm finest cell · 0 triangles")
        );
        // The timeline moves on and a newer surface is being meshed.
        let _ = a.update(Message::Scrub(0.5));
        let key = a.stock_mesh_key().unwrap();
        a.stock_mesh_job = Some(key.clone());
        a.stock_mesh_started = Some(std::time::Instant::now() - std::time::Duration::from_secs(12));
        assert_eq!(
            crate::ui::stock_surface_notice(&a).as_deref(),
            Some("Updating the stock surface, showing the last finished one · 12 s")
        );
        // The cell size and triangle count are in the details line, not the notice.
        assert_eq!(
            crate::ui::stock_surface_details(&a).as_deref(),
            Some("0.50 mm finest cell · 0 triangles")
        );
        let _ = a.view();
        // The result is in: the line is gone, the surface is the new one.
        let _ = a.update(Message::StockMeshed {
            key,
            seconds: 0.5,
            result: Ok(mesh),
        });
        assert_eq!(crate::ui::stock_surface_notice(&a), None);
        assert!(a.stock_mesh_elapsed().is_none());
        let _ = a.view();
    }
    /// The footer's 02 and 03 steps only navigate: 02 keeps the operation
    /// selected, and 03 shows the preview strip without generating (#86).
    #[test]
    fn the_footers_operations_and_preview_steps_navigate_without_selecting_or_generating() {
        let mut a = imported_app();
        let top = a
            .imported_mesh
            .as_ref()
            .unwrap()
            .surfaces
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.center[2].total_cmp(&b.center[2]))
            .unwrap()
            .0;
        let _ = a.update(Message::Surface(top));
        let _ = a.update(Message::SelectOp(0));
        let _ = a.update(Message::DuplicateOp);
        assert_eq!(a.selected, Some(1));
        // 02 Operations from another page keeps operation 1.
        a.page = Page::Audit;
        let _ = a.update(Message::OperationsStep);
        assert_eq!((a.selected, a.page), (Some(1), Page::Work));
        // 03 Preview shows the strip of the selected operation and does not generate.
        let status = a.status.clone();
        let _ = a.update(Message::PreviewStep);
        assert_eq!((a.selected, a.page), (Some(1), Page::Work));
        assert!(a.simulation.is_none() && !a.playback_ready());
        assert_eq!(a.status, status);
        // The strip offers it: Generate works from there.
        let _ = a.update(Message::Generate);
        assert!(a.playback_ready(), "{}", a.status);
        // With the simulation up, 02 and 03 leave it alone.
        let _ = a.update(Message::OperationsStep);
        let _ = a.update(Message::PreviewStep);
        assert!(a.playback_ready() && a.selected == Some(1));
        // Nothing selected: 02 selects the first operation, 03 does not generate either.
        let _ = a.update(Message::SetupEditor);
        assert_eq!(a.selected, None);
        let _ = a.update(Message::OperationsStep);
        assert_eq!(a.selected, Some(0));
        let _ = a.update(Message::SetupEditor);
        let _ = a.update(Message::PreviewStep);
        assert_eq!(a.selected, Some(0));
        assert!(a.simulation.is_none());
        let _ = a.view();
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
    /// Issue #156: the Drill editor's hole list is worked out once for the
    /// same part, placement and selected surfaces, not on every redraw.
    #[test]
    fn the_drill_editor_works_out_its_holes_once_per_part_placement_and_selection() {
        let mut a = imported_app();
        a.doc.setup_mut().operations.clear();
        a.selected = None;
        let _ = a.update(Message::AddOp(Strategy::Drill));
        let _ = a.update(Message::Tab("Geometry".into()));
        let faces: Vec<_> = a
            .imported_mesh
            .as_ref()
            .unwrap()
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| s.normal[2] > 0.99999)
            .map(|(id, _)| id)
            .collect();
        assert!(faces.len() >= 2, "{faces:?}");
        let runs = || crate::DRILL_HOLE_RUNS.with(|n| n.get());
        // Nothing selected is an answer too, kept like any other.
        let none = runs();
        for _ in 0..3 {
            let _ = a.view();
        }
        assert_eq!(runs() - none, 1);
        let _ = a.update(Message::Surface(faces[0]));
        let before = runs();
        // The planner page and the playback redraw many times a second.
        for _ in 0..5 {
            let _ = a.view();
        }
        assert_eq!(runs() - before, 1);
        // Another selection is another list; the same list again is kept.
        let _ = a.update(Message::Surface(faces[1]));
        for _ in 0..3 {
            let _ = a.view();
        }
        assert_eq!(runs() - before, 2);
        // So is the setup's placement.
        let _ = a.update(Message::Edit(Target::Setup, "tx", "1".into()));
        for _ in 0..3 {
            let _ = a.view();
        }
        assert_eq!(runs() - before, 3);
        // An edit that does not move the part does not.
        let _ = a.update(Message::Edit(Target::Operation, "feed", "851".into()));
        let _ = a.view();
        assert_eq!(runs() - before, 3);
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
        assert!(a.audit_report.iter().all(|r| r.ok), "{:?}", a.audit_report);
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
        assert_eq!(a.audit_report.iter().filter(|r| !r.ok).count(), 4);
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
        let users = a.doc.tool_users(&tool);
        assert!(!users.is_empty());
        let _ = a.update(Message::DeleteTool(tool.clone()));
        assert_eq!(a.doc.tools.len(), 3);
        // The refusal and the tool editor name the operations that use it.
        let note = a.doc.tool_in_use_note(&tool).unwrap();
        for u in &users {
            assert!(note.contains(u.as_str()), "{note}");
            assert!(a.status.contains(u.as_str()), "{}", a.status);
        }
        let unused = a
            .doc
            .tools
            .iter()
            .find(|t| a.doc.tool_users(&t.id).is_empty())
            .map(|t| t.id.clone());
        if let Some(unused) = unused {
            assert_eq!(a.doc.tool_in_use_note(&unused), None);
        }
        // A tool on many operations names the first five and counts the rest.
        let op = a.doc.setups[0].operations[0].clone();
        for _ in 0..7 {
            let mut o = op.clone();
            o.tool = tool.clone();
            a.doc.setups[0].operations.push(o);
        }
        let n = a.doc.tool_users(&tool).len();
        let note = a.doc.tool_in_use_note(&tool).unwrap();
        assert!(note.contains(&format!("{n} operations: ")), "{note}");
        assert!(note.contains(&format!(" and {} more.", n - 5)), "{note}");
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
    /// What a widget tree has outside its scrollables: buttons, and how many
    /// scrollables there are.
    fn outside_scrolls(e: iced::Element<'_, Message>) -> (usize, usize) {
        use iced::advanced::widget::{Tree, tree::Tag};
        fn walk(t: &Tree, button: Tag, scroll: Tag) -> (usize, usize) {
            if t.tag == scroll {
                return (0, 1);
            }
            t.children
                .iter()
                .fold((usize::from(t.tag == button), 0), |(b, s), c| {
                    let (cb, cs) = walk(c, button, scroll);
                    (b + cb, s + cs)
                })
        }
        let button =
            Tree::new(iced::Element::<Message>::from(iced::widget::button("x")).as_widget()).tag;
        let scroll = Tree::new(
            iced::Element::<Message>::from(iced::widget::scrollable(iced::widget::text("x")))
                .as_widget(),
        )
        .tag;
        walk(&Tree::new(e.as_widget()), button, scroll)
    }

    /// The three library pages share one editor layout (#87): the fields
    /// scroll, and Save and Delete, named for the entry, sit under them.
    #[test]
    fn each_library_page_builds_with_save_and_delete_fixed_under_the_scrolling_fields() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        // The list and the fields scroll; outside both are the page's Back
        // button and the editor's Save and Delete (before, Materials and
        // Workholding had only Back outside: their Save scrolled).
        let pages: [fn(&App) -> iced::Element<'_, Message>; 3] = [
            crate::ui::tools,
            crate::ui::materials,
            crate::ui::workholding,
        ];
        for page in pages {
            assert_eq!(outside_scrolls(page(&a)), (3, 2));
        }
        // Still so for a draft that is new, and one with a clamp, a vise and
        // another kind of entry.
        let _ = a.update(Message::NewTool);
        let _ = a.update(Message::NewMaterial);
        for kind in ["Vise", "Clamp"] {
            let _ = a.update(Message::NewHolder(kind));
            for page in pages {
                assert_eq!(outside_scrolls(page(&a)), (3, 2), "{kind}");
            }
        }
        a.holder_draft = crate::holding::Holder::new("Fixture plate", "Plate");
        assert_eq!(outside_scrolls(crate::ui::workholding(&a)), (3, 2));
    }

    #[test]
    fn a_workholding_entry_saves_and_deletes_under_its_own_name() {
        use crate::holding::Holder;
        let labels = |kind| crate::ui::holder_labels(&Holder::new(kind, "x"));
        assert_eq!(labels("Vise"), ("Save vise".into(), "Delete vise".into()));
        assert_eq!(
            labels("Clamp"),
            ("Save clamp".into(), "Delete clamp".into())
        );
        assert_eq!(
            labels("Plate"),
            ("Save entry".into(), "Delete entry".into())
        );
    }

    #[test]
    fn a_library_editor_says_when_its_draft_is_not_what_the_library_holds() {
        use crate::ui::{holder_unsaved, material_unsaved, tool_unsaved};
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        a.doc.adopt_materials();
        a.material_draft = a.doc.materials[0].clone();
        a.holder_draft = a.doc.workholding[0].clone();
        // Each draft is the library's first entry: nothing to say.
        assert_eq!(tool_unsaved(&a), None);
        assert_eq!(material_unsaved(&a), None);
        assert_eq!(holder_unsaved(&a), None);
        // An edit makes it unsaved, and saving it clears the note.
        let _ = a.update(Message::Edit(Target::Tool, "flutes", "5".into()));
        let _ = a.update(Message::Edit(Target::Material, "hardness", "63 HRC".into()));
        let _ = a.update(Message::Edit(Target::Holder, "notes", "my vise".into()));
        assert!(
            tool_unsaved(&a)
                .unwrap()
                .starts_with("Unsaved changes to this tool")
        );
        assert!(
            material_unsaved(&a)
                .unwrap()
                .starts_with("Unsaved changes to this material")
        );
        assert!(
            holder_unsaved(&a)
                .unwrap()
                .starts_with("Unsaved changes to this vise")
        );
        let _ = a.update(Message::SaveTool);
        let _ = a.update(Message::SaveMaterial);
        let _ = a.update(Message::SaveHolder);
        assert_eq!(a.doc.tools[0].p["flutes"], "5", "{}", a.status);
        assert_eq!(tool_unsaved(&a), None, "{}", a.status);
        assert_eq!(material_unsaved(&a), None, "{}", a.status);
        assert_eq!(holder_unsaved(&a), None, "{}", a.status);
        // A new entry is not in the library until it is saved.
        let _ = a.update(Message::NewMaterial);
        let _ = a.update(Message::NewHolder("Clamp"));
        assert!(
            material_unsaved(&a)
                .unwrap()
                .contains("not in the library yet")
        );
        assert!(
            holder_unsaved(&a)
                .unwrap()
                .starts_with("This clamp is not in the library")
        );
    }

    /// The browser keeps its setup count and rule outside the scrolling tree,
    /// puts every setup and operation in it, and cuts a long name (#78).
    #[test]
    fn the_browser_builds_with_a_long_operation_and_setup_name_and_keeps_its_count_fixed() {
        use crate::ui::ellipsize;
        let long =
            "Rough the whole outer wall of the bearing housing down to the finishing allowance";
        assert_eq!(ellipsize("Facing", 20), "Facing");
        assert_eq!(
            ellipsize("12345678901234567890", 20),
            "12345678901234567890"
        );
        let cut = ellipsize(long, 20);
        assert_eq!(cut.chars().count(), 20);
        assert!(cut.ends_with('…') && long.starts_with(&cut[..cut.len() - '…'.len_utf8()]));
        // A multi-byte name is cut on a character, and the space before the
        // ellipsis goes.
        assert_eq!(ellipsize("Ø6 flat   mill", 8), "Ø6 flat…");
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        a.page = Page::Work;
        let before = crate::ui::browser(&a);
        let (outside, scrolls) = outside_scrolls(before);
        // The browser holds one button outside its scroll, Hide (#76): the
        // count and the footer are text.
        assert_eq!((outside, scrolls), (1, 1));
        let ops = a.doc.setups[0].operations.len();
        assert!(ops > 0);
        a.doc.setups[0].operations[0]
            .p
            .insert("name".into(), long.into());
        a.doc.setups[0].p.insert("name".into(), long.into());
        let _ = a.view();
        assert_eq!(outside_scrolls(crate::ui::browser(&a)), (1, 1));
    }

    fn fresh_app(dir: &tempfile::TempDir) -> App {
        App::from_workspace(dir.path().join("workspace.json"), None)
    }
    #[test]
    fn an_edit_made_while_the_save_dialog_is_open_is_in_the_saved_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        a.replace_document(Document::sample(), None);
        let _ = a.update(Message::SaveProject);
        assert!(a.save_busy);
        // The dialog does not block the window: the job changes meanwhile.
        a.doc.title = "Edited while the dialog was open".into();
        a.project_dirty = true;
        let file = dir.path().join("job.axis.json");
        let _ = a.update(Message::Saved(Some(file.clone())));
        assert!(!a.save_busy && !a.project_dirty);
        assert_eq!(a.project_path.as_deref(), Some(file.as_path()));
        assert_eq!(a.status, format!("Saved {}", file.display()));
        let saved = crate::storage::load(&file).unwrap();
        assert_eq!(saved.title, "Edited while the dialog was open");
        // A path that cannot be written is reported, and the job stays unsaved.
        a.project_dirty = true;
        a.project_path = None;
        let _ = a.update(Message::SaveProject);
        let blocked = file.join("job.axis.json");
        let _ = a.update(Message::Saved(Some(blocked)));
        assert!(!a.save_busy && a.project_dirty && a.project_path.is_none());
        assert!(a.status.starts_with("Save failed: "), "{}", a.status);
        // Cancelling the dialog changes nothing.
        let _ = a.update(Message::SaveProject);
        let _ = a.update(Message::Saved(None));
        assert!(!a.save_busy && a.project_dirty);
    }
    #[test]
    fn a_failing_workspace_save_is_said_once_and_shown_in_the_header_until_it_works() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        let good = a.state_path.clone();
        // A file where the directory should be: the write cannot succeed.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "a file").unwrap();
        a.state_path = blocker.join("workspace.json");
        a.dirty = true;
        a.persist();
        assert!(a.dirty);
        let error = a.save_error.clone().expect("the failure is kept");
        assert_eq!(a.status, format!("Could not save the workspace: {error}"));
        // Another message is not replaced by the retry with the same error.
        a.status = "Preview ready.".into();
        a.persist();
        a.persist();
        assert_eq!(a.status, "Preview ready.");
        assert_eq!(a.save_error.as_deref(), Some(error.as_str()));
        assert_eq!(
            crate::ui::workspace_note(&a),
            format!("Workspace not saved: {error}")
        );
        let _ = a.view();
        // A different error is a new thing to say.
        std::fs::remove_file(&blocker).unwrap();
        std::fs::create_dir(&blocker).unwrap();
        a.state_path = blocker.clone();
        a.persist();
        let second = a.save_error.clone().unwrap();
        assert_ne!(second, error);
        assert_eq!(a.status, format!("Could not save the workspace: {second}"));
        // A save that works clears it.
        a.state_path = good.clone();
        a.persist();
        assert!(!a.dirty && a.save_error.is_none());
        assert!(good.exists());
        assert_eq!(crate::ui::workspace_note(&a), "Workspace saved locally");
        a.dirty = true;
        assert_eq!(crate::ui::workspace_note(&a), "Saving workspace…");
        let _ = a.view();
    }
    /// Plays a whole sign-in: the browser result, then the save step's result.
    /// The save step writes the session before it reports, and unit tests do
    /// not run tasks, so the write happens here.
    fn sign_in_with(a: &mut App, dir: &tempfile::TempDir, session: crate::session::Session) {
        crate::session::default_store(dir.path(), true, false)
            .save(&session)
            .unwrap();
        let _ = a.update(Message::SignIn);
        let _ = a.update(Message::SignedIn(
            a.sign_in_attempt,
            Ok((session.clone(), "oaiapp_1".into())),
        ));
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
        doc.adopt_holders();
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
        checked_preview_of(a, view, |_| {})
    }
    /// `checked_preview` of the good plan as `edit` changes it.
    fn checked_preview_of(
        a: &App,
        view: bool,
        edit: impl FnOnce(&mut crate::proposal::Proposal),
    ) -> std::sync::Arc<crate::PlanPreview> {
        let mesh = a.imported_mesh.as_ref().unwrap();
        let report = crate::analysis::report(&a.doc, mesh).unwrap();
        let mut proposal = crate::proposal::tests::good(&report);
        edit(&mut proposal);
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
        // Try again: the old timeline and text are gone.
        let streamed = a.plan_stream.clone();
        *streamed.lock().unwrap() = "x".repeat(900);
        let _ = a.update(Message::Plan);
        assert!(a.plan_timeline.is_empty());
        assert!(!std::sync::Arc::ptr_eq(&streamed, &a.plan_stream));
        assert_eq!(*a.plan_stream.lock().unwrap(), "");
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
                looks: vec![],
            })
            .collect()
    }
    #[test]
    fn running_rows_list_each_steps_problems_and_the_draft_under_it() {
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
        *a.plan_stream.lock().unwrap() = "x".into();
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
        // Only the open request shows what has streamed in so far; text that
        // is no JSON yet is still thinking.
        assert_eq!(
            rows[6].notes,
            ["Thinking: the model has not started writing yet."]
        );
        *a.plan_stream.lock().unwrap() =
            r#"{"summary":"Face it","setups":[{"orientation":"+Z","operations":[{"strategy":"Face","name":"Face top","tool":1,"rpm":8000,"feed":12"#
                .into();
        let rows = crate::ui::timeline_rows(&a, now, false);
        assert_eq!(
            rows[6].notes,
            [
                "Plan: Face it",
                "Setup 1 · +Z up · writing: Face \"Face top\" T1, 8000 rpm"
            ]
        );
        assert!(rows[..6].iter().all(|r| {
            r.notes
                .iter()
                .all(|n| !n.starts_with("Plan:") && !n.starts_with("Thinking"))
        }));
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
    fn a_step_that_ended_without_a_result_is_not_ticked_and_says_why() {
        use crate::planner::{Progress::*, StepResult};
        use crate::ui::Mark::*;
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        a.plan_model = "gpt-6.1-sol".into();
        let now = std::time::Instant::now();
        let problems = StepResult::Problems {
            shown: vec!["Setup 1, operation 2: Pocket too deep.".into()],
            more: 0,
        };
        let rows = |a: &App, finished| crate::ui::timeline_rows(a, now, finished);
        let texts =
            |a: &App| -> Vec<String> { rows(a, true).into_iter().map(|r| r.text).collect() };
        let marks =
            |a: &App, finished| -> Vec<_> { rows(a, finished).iter().map(|r| r.mark).collect() };
        // A request that errored after a check found problems: its row is last.
        a.plan_timeline = steps_ago(
            now,
            vec![
                (Analyzing, 50, Some(49), None),
                (Asking(1), 49, Some(30), None),
                (Checking(1), 30, Some(20), Some(problems.clone())),
                (Repairing(2), 20, Some(18), None),
            ],
        );
        assert_eq!(
            texts(&a),
            [
                "Analyzed the part · 1 s",
                "Asked gpt-6.1-sol for a plan (1 of 4) · 19 s",
                "Checked the plan (1 of 4) · 10 s · 1 problem",
                "Asked gpt-6.1-sol for a correction (2 of 4) · 2 s · no reply",
            ]
        );
        assert_eq!(marks(&a, true), [Done, Done, Warn, Warn]);
        // A request whose reply was read, and then checked, is ticked.
        a.plan_timeline = steps_ago(
            now,
            vec![
                (Analyzing, 50, Some(49), None),
                (Asking(1), 49, Some(30), None),
                (Checking(1), 30, Some(20), Some(StepResult::Passed)),
                (Extending(2), 20, Some(18), None),
                (Checking(2), 18, Some(10), None),
            ],
        );
        // A check that failed to run: its row is last.
        assert_eq!(
            texts(&a),
            [
                "Analyzed the part · 1 s",
                "Asked gpt-6.1-sol for a plan (1 of 4) · 19 s",
                "Checked the plan (1 of 4) · 10 s · passed",
                "Asked gpt-6.1-sol about faces still covered (2 of 4) · 2 s",
                "Checked the plan (2 of 4) · 8 s · did not finish",
            ]
        );
        assert_eq!(marks(&a, true), [Done, Done, Done, Done, Warn]);
        // The first check that failed to run, and an analysis that did.
        a.plan_timeline = steps_ago(
            now,
            vec![
                (Analyzing, 50, Some(49), None),
                (Asking(1), 49, Some(30), None),
                (Checking(1), 30, Some(20), None),
            ],
        );
        assert!(texts(&a)[2].ends_with(" · did not finish"));
        a.plan_timeline = steps_ago(now, vec![(Analyzing, 50, Some(49), None)]);
        assert_eq!(texts(&a), ["Analyzed the part · 1 s · did not finish"]);
        assert_eq!(marks(&a, true), [Warn]);
        // While the run goes on (the 3D view is being built): the same marks,
        // the reason under the row, and the problems of the others as before.
        a.plan_timeline = steps_ago(
            now,
            vec![
                (Analyzing, 60, Some(59), None),
                (Asking(1), 59, Some(40), None),
                (Checking(1), 40, Some(35), Some(problems)),
                (Repairing(2), 35, Some(30), None),
                (Checking(2), 30, Some(20), Some(StepResult::Passed)),
                (Extending(3), 20, Some(18), None),
                (Previewing, 18, None, None),
            ],
        );
        assert_eq!(marks(&a, false), [Done, Done, Warn, Done, Done, Warn, Open]);
        let running = rows(&a, false);
        assert_eq!(running[5].notes, ["no reply"]);
        assert_eq!(
            running[2].notes,
            ["• Setup 1, operation 2: Pocket too deep."]
        );
        assert!(running[3].notes.is_empty() && running[6].notes.is_empty());
        assert_eq!(
            running[5].text,
            "Asked gpt-6.1-sol about faces still covered (3 of 4) · 2 s"
        );
        // The same run, its follow-up's check failing to run.
        a.plan_timeline = steps_ago(
            now,
            vec![
                (Analyzing, 60, Some(59), None),
                (Asking(1), 59, Some(40), None),
                (Checking(1), 40, Some(35), Some(StepResult::Passed)),
                (Extending(2), 35, Some(30), None),
                (Checking(2), 30, Some(20), None),
                (Previewing, 20, None, None),
            ],
        );
        assert_eq!(marks(&a, false), [Done, Done, Done, Done, Warn, Open]);
        assert_eq!(rows(&a, false)[4].notes, ["did not finish"]);
        // Both finished views build.
        a.page = Page::Ai;
        a.planner = crate::Planner::Preview(checked_preview(&a, false));
        let _ = a.view();
    }
    #[test]
    fn the_draft_reads_as_plan_and_setup_lines() {
        use crate::proposal::{Draft, DraftOp, DraftSetup};
        let op = |s: &str, n: &str, t, rpm, feed| DraftOp {
            strategy: Some(s.into()),
            name: Some(n.into()),
            tool: Some(t),
            rpm,
            feed,
        };
        let draft = Draft {
            summary: Some("Face the top,\nthen pocket.".into()),
            setups: vec![
                DraftSetup {
                    name: Some("Top".into()),
                    orientation: Some("+Z".into()),
                    operations: vec![
                        op("Face", "Face top", 1, Some(8000.), Some(1200.)),
                        op("Pocket", "Main pocket", 3, Some(9000.), None),
                    ],
                },
                DraftSetup {
                    name: Some("Bottom".into()),
                    orientation: Some("-Z".into()),
                    operations: vec![op("Contour", "Outside", 2, Some(9000.), Some(1200.))],
                },
            ],
            writing: true,
        };
        assert_eq!(
            crate::ui::draft_lines(&draft),
            vec![
                "Plan: Face the top, then pocket.".to_string(),
                "Setup 1 · +Z up · Face \"Face top\" T1 · Pocket \"Main pocket\" T3".into(),
                "Setup 2 · -Z up · writing: Contour \"Outside\" T2, 9000 rpm, 1200 mm/min".into(),
            ]
        );
        assert_eq!(
            crate::ui::draft_lines(&Draft {
                writing: true,
                ..Draft::default()
            }),
            vec!["Thinking: the model has not started writing yet.".to_string()]
        );
    }
    #[test]
    fn a_summary_not_yet_written_is_still_thinking() {
        use crate::proposal::partial;
        let thinking = vec!["Thinking: the model has not started writing yet.".to_string()];
        // The summary's opening quote has arrived and nothing more.
        assert_eq!(partial("{\"summary\":\"").summary.as_deref(), Some(""));
        assert_eq!(
            crate::ui::draft_lines(&partial("{\"summary\":\"")),
            thinking
        );
        assert_eq!(
            crate::ui::draft_lines(&partial("{\"summary\":\" \\n  ")),
            thinking
        );
        // Once a word has, it is the plan; a setup shows without an empty plan.
        assert_eq!(
            crate::ui::draft_lines(&partial("{\"summary\":\"Fa")),
            ["Plan: Fa"]
        );
        assert_eq!(
            crate::ui::draft_lines(&partial("{\"summary\":\"\",\"setups\":[{")),
            ["Setup 1"]
        );
    }
    #[test]
    fn a_draft_shows_six_setups_and_counts_the_rest() {
        use crate::proposal::{Draft, DraftSetup};
        let setups = |n: usize| -> Vec<DraftSetup> {
            (1..=n)
                .map(|i| DraftSetup {
                    name: Some(format!("S{i}")),
                    ..DraftSetup::default()
                })
                .collect()
        };
        let lines = |n| {
            crate::ui::draft_lines(&Draft {
                summary: Some("Plan.".into()),
                setups: setups(n),
                writing: true,
            })
        };
        let six = lines(6);
        assert_eq!(six.len(), 7, "the plan and six setups: {six:?}");
        assert_eq!(six[6], "Setup 6");
        let ten = lines(10);
        assert_eq!(ten.len(), 8, "{ten:?}");
        assert_eq!(ten[6], "Setup 6");
        assert_eq!(ten[7], "and 4 more setups");
        assert_eq!(lines(7)[7], "and 1 more setup");
    }
    #[test]
    fn a_look_reads_as_one_line() {
        let look = crate::planner::Look {
            description: "+Z up, top, zoomed on 12, 14; stock shown; tags: 12".into(),
            purpose: "is the pocket open on −Y?".into(),
            png: Some(std::sync::Arc::new(vec![])),
        };
        assert_eq!(
            crate::ui::look_line(&look),
            "Looked: +Z up, top, zoomed on 12, 14; stock shown; tags: 12 — \"is the pocket open on −Y?\""
        );
        let bare = crate::planner::Look {
            purpose: String::new(),
            ..look
        };
        assert_eq!(
            crate::ui::look_line(&bare),
            "Looked: +Z up, top, zoomed on 12, 14; stock shown; tags: 12"
        );
        let error = crate::planner::Look {
            description: "Surface 99 is not a surface of this part.".into(),
            purpose: String::new(),
            png: None,
        };
        assert_eq!(
            crate::ui::look_line(&error),
            "Surface 99 is not a surface of this part."
        );
    }
    #[test]
    fn the_running_request_shows_the_draft_looks_and_drops() {
        use crate::planner::{Dropped, Event, Look, Progress};
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        a.page = Page::Ai;
        a.planner = crate::Planner::Running(Progress::Asking(1));
        a.record_plan_event(Event::Overview(Look {
            description: "+Z up, isometric; stock shown; tags: none".into(),
            purpose: String::new(),
            png: Some(std::sync::Arc::new(vec![1])),
        }));
        let overview = a.plan_overview.as_ref().unwrap();
        assert!(overview.thumb.is_some() && !overview.warn);
        a.record_plan_event(Event::Stage(Progress::Asking(1)));
        a.record_plan_event(Event::Looked(
            1,
            Look {
                description: "+Z up, top, whole part; stock shown; tags: none".into(),
                purpose: "overview".into(),
                png: Some(std::sync::Arc::new(vec![1])),
            },
        ));
        a.record_plan_event(Event::Looked(
            1,
            Look {
                description: "Surface 99 is not a surface of this part.".into(),
                purpose: String::new(),
                png: None,
            },
        ));
        a.record_plan_event(Event::Dropped(1, Dropped::Pictures));
        *a.plan_stream.lock().unwrap() = "{\"summary\":\"Face it\",\"setups\":[".into();
        let _ = a.view();
        let rows = crate::ui::timeline_rows(&a, std::time::Instant::now(), false);
        let row = rows.last().unwrap();
        assert_eq!(row.notes, vec!["Plan: Face it".to_string()]);
        assert_eq!(row.looks.len(), 3);
        assert!(row.looks[0].line.starts_with("Looked: "));
        assert!(row.looks[0].thumb.is_some() && !row.looks[0].warn);
        // An error sentence is shown bare, without a picture.
        assert_eq!(
            row.looks[1].line,
            "Surface 99 is not a surface of this part."
        );
        assert!(row.looks[1].thumb.is_none() && !row.looks[1].warn);
        assert!(row.looks[2].warn && row.looks[2].thumb.is_none());
        assert_eq!(
            row.looks[2].line,
            "OpenAI refused the request; sending it again without pictures."
        );
        // A refused tool says so too.
        a.record_plan_event(Event::Dropped(1, Dropped::Tool));
        let rows = crate::ui::timeline_rows(&a, std::time::Instant::now(), false);
        assert_eq!(
            rows.last().unwrap().looks[3].line,
            "OpenAI refused the request; sending it again without its tools."
        );
        // An event for a request that is not on the timeline joins nothing.
        a.record_plan_event(Event::Dropped(3, Dropped::Tool));
        assert_eq!(a.plan_timeline[0].looks.len(), 4);
        // Finished: one line, ending in the pictures looked at (not errors)
        // and in what was refused, and no look lines under it; the request is
        // marked for the refusal it met.
        a.record_plan_event(Event::Stage(Progress::Checking(1)));
        let _ = a.view();
        let rows = crate::ui::timeline_rows(&a, std::time::Instant::now(), true);
        assert!(rows[0].text.ends_with("· 1 look"), "{}", rows[0].text);
        assert!(
            rows[0]
                .text
                .ends_with("· sent again without pictures · sent again without tools · 1 look"),
            "{}",
            rows[0].text
        );
        assert_eq!(rows[0].mark, crate::ui::Mark::Warn);
        assert!(
            rows.iter()
                .all(|r| r.looks.is_empty() && r.notes.is_empty())
        );
        // The run going on lists them still.
        let rows = crate::ui::timeline_rows(&a, std::time::Instant::now(), false);
        assert_eq!(rows[0].looks.len(), 4);
        assert!(!rows[0].text.contains("look"), "{}", rows[0].text);
        // A new run starts without them.
        a.drop_plan();
        assert!(a.plan_overview.is_none() && a.plan_timeline.is_empty());
    }
    /// A grey PNG, as the planner's pictures are.
    fn picture(width: u32, height: u32) -> std::sync::Arc<Vec<u8>> {
        let mut out = vec![];
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![200; (width * height * 3) as usize])
            .unwrap();
        drop(writer);
        std::sync::Arc::new(out)
    }
    const PAGE: iced::Size = iced::Size::new(1000., 800.);
    /// A run on its first request that has sent the overview, `steps` more
    /// stages, and looked at one picture in the first request.
    fn running_plan_app(dir: &tempfile::TempDir, steps: usize) -> App {
        use crate::planner::{Event, Look, Progress};
        let mut a = signed_in_planning_app(dir);
        a.page = Page::Ai;
        a.planner = crate::Planner::Running(Progress::Asking(1));
        a.record_plan_event(Event::Overview(Look {
            description: "+Z up, isometric; stock shown; tags: none".into(),
            purpose: String::new(),
            png: Some(picture(300, 200)),
        }));
        a.record_plan_event(Event::Stage(Progress::Analyzing));
        a.record_plan_event(Event::Stage(Progress::Asking(1)));
        a.record_plan_event(Event::Looked(
            1,
            Look {
                description: "+Z up, top, whole part; stock shown; tags: none".into(),
                purpose: "overview".into(),
                png: Some(picture(300, 200)),
            },
        ));
        for n in 0..steps {
            a.record_plan_event(Event::Stage(Progress::Checking(n % 4 + 1)));
        }
        a
    }
    fn index_of(texts: &[String], start: &str) -> usize {
        texts
            .iter()
            .position(|t| t.starts_with(start))
            .unwrap_or_else(|| panic!("no text starts with {start:?} in {texts:?}"))
    }
    #[test]
    fn the_running_page_has_its_stage_time_and_cancel_before_the_scrolling_timeline() {
        use crate::ui::probe::{Seen, id, seen};
        let dir = tempfile::tempdir().unwrap();
        let before = |a: &App| {
            let items = seen(crate::ui::plan_side(a), PAGE);
            let text = |start: &str| {
                items
                    .iter()
                    .position(|i| matches!(i, Seen::Text(t) if t.starts_with(start)))
                    .unwrap_or_else(|| panic!("no text starts with {start:?}"))
            };
            let timeline = items
                .iter()
                .position(
                    |i| matches!(i, Seen::Scroll(scroll, _) if *scroll == id("plan-timeline")),
                )
                .expect("the timeline scrolls on its own");
            let crate::Planner::Running(progress) = a.planner else {
                panic!("running")
            };
            let running = crate::ui::running_text(a, progress);
            let (stage, since, cancel) = (
                text(&running),
                text("0 s since the run started"),
                text("Cancel"),
            );
            // The stage, the time and Cancel come first, in the pinned part.
            assert!(stage < cancel && since < cancel && cancel < timeline);
            // What scrolls: the overview picture and the timeline rows, the
            // open request's among them (the stage above it says the same).
            for later in [
                "The AI starts with this picture",
                "Analyzed the part",
                &running,
            ] {
                assert!(
                    items.iter().enumerate().any(|(at, i)| at > timeline
                        && matches!(i, Seen::Text(t) if t.starts_with(later))),
                    "{later} is in the scrolling part"
                );
            }
            // The waiting note stays with Cancel.
            assert!(text("This can take a few minutes") < timeline);
            (cancel, timeline)
        };
        // However long the timeline grows, Cancel keeps its place in the tree
        // and in front of the rows.
        let short = before(&running_plan_app(&dir, 0));
        let long = before(&running_plan_app(&dir, 40));
        assert_eq!(short, long);
    }
    #[test]
    fn a_finished_page_has_the_outcome_and_its_actions_before_the_timeline() {
        use crate::ui::probe::{Seen, id, seen, texts};
        let dir = tempfile::tempdir().unwrap();
        let mut a = running_plan_app(&dir, 3);
        // The failure: its message, then Try again and Dismiss, then the steps.
        a.planner = crate::Planner::Failed(crate::planner::Failure {
            message: "The model never gave a plan that passed.".into(),
            problems: vec![],
            api: None,
            requests: 4,
        });
        let lines = texts(crate::ui::plan_side(&a), PAGE);
        let (head, again, dismiss, tried, row) = (
            index_of(&lines, "No plan this time"),
            index_of(&lines, "Try again"),
            index_of(&lines, "Dismiss"),
            index_of(&lines, "What the planner tried"),
            index_of(&lines, "Analyzed the part"),
        );
        assert!(head < again && again < dismiss && dismiss < tried && tried < row);
        // The preview: the outcome, the actions and the 3D heading, then the
        // timeline in a scrollable of its own.
        a.planner = crate::Planner::Preview(checked_preview(&a, false));
        let items = seen(crate::ui::plan_side(&a), PAGE);
        let at = |start: &str| {
            items
                .iter()
                .position(|i| matches!(i, Seen::Text(t) if t.starts_with(start)))
                .unwrap_or_else(|| panic!("no text starts with {start:?}"))
        };
        let timeline = items
            .iter()
            .position(|i| matches!(i, Seen::Scroll(scroll, _) if *scroll == id("plan-timeline")))
            .expect("the finished timeline scrolls on its own");
        let (outcome, apply, made, row) = (
            at("Plan ready to review"),
            at("Apply plan"),
            at("How the plan was made"),
            at("Analyzed the part"),
        );
        assert!(outcome < apply && apply < made && made < timeline && timeline < row);
        // Every action is still on the page: Apply and Discard twice.
        let texts = texts(crate::ui::plan_side(&a), PAGE);
        assert_eq!(texts.iter().filter(|t| *t == "Apply plan").count(), 2);
        assert_eq!(texts.iter().filter(|t| *t == "Discard").count(), 2);
    }
    #[test]
    fn a_thumbnail_click_opens_the_picture_at_full_size_and_a_second_click_closes_it() {
        use crate::planner::Progress;
        use crate::ui::PlanPicture;
        use crate::ui::probe::{Seen, clicks, seen, texts};
        let dir = tempfile::tempdir().unwrap();
        let mut a = running_plan_app(&dir, 0);
        let enlarged = |a: &App| {
            texts(crate::ui::plan_side(a), PAGE)
                .iter()
                .any(|t| t == "The picture at full size")
        };
        assert!(!enlarged(&a));
        // Clicking the overview and the look sends each its own toggle.
        let sent = clicks(crate::ui::plan_side(&a), PAGE);
        let picked = |picture| {
            sent.iter()
                .any(|m| matches!(m, Message::PlanPicture(p) if *p == picture))
        };
        let look = PlanPicture::Look { step: 1, look: 0 };
        assert!(picked(PlanPicture::Overview) && picked(look));
        let _ = a.update(Message::PlanPicture(look));
        assert_eq!(a.plan_enlarged, Some(look));
        assert!(enlarged(&a));
        // The picture is in the page at its natural size, in a scrollable.
        let items = seen(crate::ui::plan_side(&a), PAGE);
        let sizes: Vec<iced::Size> = items
            .iter()
            .filter_map(|i| match i {
                Seen::Scroll(None, size) => Some(*size),
                _ => None,
            })
            .collect();
        assert!(sizes.contains(&iced::Size::new(300., 200.)), "{sizes:?}");
        // The stage and Cancel stay above it, and a Close button sends the
        // same toggle as the thumbnail.
        let lines = texts(crate::ui::plan_side(&a), PAGE);
        assert!(index_of(&lines, "Cancel") < index_of(&lines, "The picture at full size"));
        assert!(index_of(&lines, "Close") > index_of(&lines, "The picture at full size"));
        let closing = clicks(crate::ui::plan_side(&a), PAGE);
        assert!(
            closing
                .iter()
                .any(|m| matches!(m, Message::PlanPicture(p) if *p == look))
        );
        // Another picture replaces it; the same one again closes it.
        let _ = a.update(Message::PlanPicture(PlanPicture::Overview));
        assert_eq!(a.plan_enlarged, Some(PlanPicture::Overview));
        let _ = a.update(Message::PlanPicture(PlanPicture::Overview));
        assert_eq!(a.plan_enlarged, None);
        assert!(!enlarged(&a));
        // A picture the timeline no longer has shows nothing, and a new run
        // starts with none open.
        let _ = a.update(Message::PlanPicture(PlanPicture::Look { step: 9, look: 0 }));
        assert!(!enlarged(&a));
        let _ = a.update(Message::PlanPicture(look));
        a.drop_plan();
        assert_eq!(a.plan_enlarged, None);
        a.planner = crate::Planner::Running(Progress::Analyzing);
        assert!(!enlarged(&a));
    }
    #[test]
    fn show_full_text_opens_a_clipped_note_reason_and_summary() {
        use crate::ui::Expand;
        use crate::ui::probe::{clicks, texts};
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        a.page = Page::Ai;
        let long = |tail: &str| format!("word{} {tail}", " word".repeat(120));
        let preview = checked_preview_of(&a, false, |p| {
            p.summary = long("SUMMARYEND");
            p.setups[0].workholding = long("NOTEEND");
            p.setups[0].operations[0].reason = long("REASONEND");
            p.warnings = vec![long("WARNINGEND")];
        });
        a.planner = crate::Planner::Preview(preview);
        let window = iced::Size::new(1000., 3000.);
        let page = |a: &App| texts(crate::ui::plan_side(a), window);
        let has = |lines: &[String], end: &str| lines.iter().any(|t| t.contains(end));
        // Clipped: the ends are missing, and each text has its link.
        let lines = page(&a);
        for end in ["SUMMARYEND", "NOTEEND", "REASONEND", "WARNINGEND"] {
            assert!(!has(&lines, end), "{end} is clipped");
        }
        assert_eq!(lines.iter().filter(|t| *t == "Show full text").count(), 4);
        assert!(lines.iter().all(|t| t != "Show less"));
        // The link is in the page and sends its text's toggle.
        let sent = clicks(crate::ui::plan_side(&a), window);
        assert!(
            sent.iter()
                .any(|m| matches!(m, Message::PlanExpand(Expand::Reason(0, 0))))
        );
        // Each opens on its own, ends in its last word and offers Show less;
        // the others stay clipped.
        for (key, end) in [
            (Expand::Summary, "SUMMARYEND"),
            (Expand::Workholding(0), "NOTEEND"),
            (Expand::Reason(0, 0), "REASONEND"),
            (Expand::Warning(0), "WARNINGEND"),
        ] {
            let _ = a.update(Message::PlanExpand(key));
            let lines = page(&a);
            assert!(has(&lines, end), "{end} is shown in full");
            assert_eq!(lines.iter().filter(|t| *t == "Show less").count(), 1);
            assert_eq!(lines.iter().filter(|t| *t == "Show full text").count(), 3);
            let _ = a.update(Message::PlanExpand(key));
            assert!(!has(&page(&a), end), "{end} is clipped again");
        }
        // A short text has no link, and a discarded plan forgets what was open.
        let preview = checked_preview(&a, false);
        a.planner = crate::Planner::Preview(preview);
        assert!(page(&a).iter().all(|t| t != "Show full text"));
        let _ = a.update(Message::PlanExpand(Expand::Summary));
        let _ = a.update(Message::DiscardPlan);
        assert!(a.plan_expanded.is_empty());
    }
    #[test]
    fn the_tool_lines_of_the_smoke_timeline_are_the_tools_own() {
        let dir = tempfile::tempdir().unwrap();
        let a = planning_app(&dir);
        let lines = a.smoke_tool_lines();
        // One line per call, each the sentence its tool gives; the stock
        // before setup 2 is the one setup 1's facing left.
        let starts = [
            "Checked a draft: ",
            "Stock before setup 2: ",
            "Vise 1 for setup 2: ",
            "Cutting data for T1: ",
            "Surface ",
        ];
        assert_eq!(lines.len(), starts.len(), "{lines:?}");
        for (line, start) in lines.iter().zip(starts) {
            assert!(line.starts_with(start), "{line}");
        }
        assert!(lines[1].contains("; bottom cut "), "{}", lines[1]);
        assert!(lines[4].contains(": floor "), "{}", lines[4]);
    }
    #[test]
    fn a_finished_line_says_what_was_refused_and_how_many_pictures_were_seen() {
        use crate::planner::{Dropped, Event, Look, Progress, StepResult};
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        a.page = Page::Ai;
        a.plan_model = "gpt-6.1-sol".into();
        let picture = |text: &str| Look {
            description: text.into(),
            purpose: String::new(),
            png: Some(std::sync::Arc::new(vec![1])),
        };
        a.record_plan_event(Event::Stage(Progress::Asking(1)));
        a.record_plan_event(Event::Dropped(1, Dropped::Tool));
        a.record_plan_event(Event::Stage(Progress::Checking(1)));
        a.record_plan_event(Event::Checked(1, StepResult::Passed));
        a.record_plan_event(Event::Stage(Progress::Repairing(2)));
        a.record_plan_event(Event::Looked(2, picture("+Z up, top")));
        a.record_plan_event(Event::Looked(2, picture("+Z up, front")));
        a.record_plan_event(Event::Stage(Progress::Checking(2)));
        a.record_plan_event(Event::Checked(2, StepResult::Passed));
        a.record_plan_event(Event::Stage(Progress::Extending(3)));
        a.record_plan_event(Event::Dropped(3, Dropped::Pictures));
        a.record_plan_event(Event::Looked(3, picture("+Z up, side")));
        a.record_plan_event(Event::Stage(Progress::Checking(3)));
        a.record_plan_event(Event::Checked(3, StepResult::Passed));
        let now = std::time::Instant::now();
        let rows = crate::ui::timeline_rows(&a, now, true);
        let text: Vec<_> = rows.iter().map(|r| r.text.as_str()).collect();
        // What follows the step and its time.
        let tail = |i: usize| text[i].split(" · ").skip(2).collect::<Vec<_>>();
        assert_eq!(tail(0), ["sent again without tools"]);
        assert_eq!(text.len(), 6);
        assert!(rows.iter().all(|r| r.looks.is_empty()));
        assert_eq!(tail(1), ["passed"]);
        assert_eq!(tail(2), ["2 looks"]);
        assert_eq!(tail(3), ["passed"]);
        // The count is last, after what was refused.
        assert_eq!(tail(4), ["sent again without pictures", "1 look"]);
        // The same rows while the run goes: the lines, not the summary.
        let rows = crate::ui::timeline_rows(&a, now, false);
        assert_eq!(rows[0].looks.len(), 1);
        assert!(rows[0].looks[0].warn);
        assert_eq!(rows[2].looks.len(), 2);
        assert!(rows.iter().all(|r| !r.text.contains("sent again")));
        assert!(rows.iter().all(|r| !r.text.contains(" look")));
        // Both finished views build.
        a.planner = crate::Planner::Failed(crate::planner::Failure {
            message: "No plan.".into(),
            problems: vec![],
            api: None,
            requests: 3,
        });
        let _ = a.view();
    }
    #[tokio::test]
    async fn a_new_requests_row_starts_at_zero_because_the_client_resets_the_count() {
        use crate::fake_openai::{Fake, Resp};
        use crate::openai::{Item, Role};
        use crate::planner::{Backend, Event::*, OpenAiBackend, Progress::*};
        use crate::session::SessionStore;
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let _ = a.update(Message::Plan);
        let epoch = a.plan_epoch;
        let count = |a: &App| a.plan_stream.lock().unwrap().chars().count();
        // The run's backend streams into the App's own text, as Plan sets it up.
        let fake = Fake::start(|_| Resp::answer("{}")).await;
        let store = std::sync::Arc::new(crate::session::FileStore(dir.path().join("s.json")));
        let session = crate::session::sample();
        store.save(&session).unwrap();
        let client = crate::openai::Client::new(
            crate::oauth::Endpoints::local(&fake.base),
            "oaiapp_1".into(),
            store,
            session,
        );
        let backend = OpenAiBackend::new(
            std::sync::Arc::new(client),
            "gpt-6.1-sol".into(),
            a.plan_stream.clone(),
            true,
        );
        let conversation = [Item::text(Role::User, "Plan.")];
        send(&mut a, epoch, Stage(Analyzing));
        send(&mut a, epoch, Stage(Asking(1)));
        *a.plan_stream.lock().unwrap() = "x".repeat(1200);
        // The App does not touch the count when a stage starts, for a check
        // or for the next request: only the client's attempts reset it.
        send(&mut a, epoch, Stage(Checking(1)));
        for request in [Repairing(2), Extending(3), Asking(4)] {
            send(&mut a, epoch, Stage(request));
            assert_eq!(count(&a), 1200, "{request:?}");
        }
        // The next request's attempt starts at zero and counts its own reply.
        assert_eq!(
            backend
                .reply(&conversation, &[], crate::openai::ToolChoice::Auto)
                .await
                .unwrap()
                .text,
            "{}"
        );
        assert_eq!(count(&a), 2);
        let rows = crate::ui::timeline_rows(&a, std::time::Instant::now(), false);
        assert_eq!(
            rows.last().unwrap().notes,
            ["Thinking: the model has not started writing yet."]
        );
        // An event of an older run leaves the count alone.
        send(&mut a, epoch.wrapping_sub(1), Stage(Repairing(2)));
        assert_eq!(count(&a), 2);
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
        apply_checked_plan_of(a, |_| {});
    }
    /// The good plan for a job held in a vise: setup 2, flipped, gets the
    /// same vise, whose jaws grip the faced top. Setup 1's Contour would cut
    /// the stock's sides there from the top down, so it is left out.
    fn held_in_a_vise(proposal: &mut crate::proposal::Proposal) {
        let contour = proposal.setups[0].operations.remove(2);
        assert_eq!(contour.strategy, "Contour");
        // The pocket's first loop past the boss slots the narrow corridor
        // beside it at the full feed: 2 mm deep, that could tip the stock
        // out of the vise's shallow grip (×1.8 against ×2).
        let pocket = &mut proposal.setups[0].operations[1];
        assert_eq!(pocket.strategy, "Pocket");
        pocket.stepdown = Some(1.);
    }
    /// `apply_checked_plan` of the good plan as `edit` changes it.
    fn apply_checked_plan_of(a: &mut App, edit: impl FnOnce(&mut crate::proposal::Proposal)) {
        let preview = checked_preview_of(a, false, edit);
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
            path: dir.path().join("other.stl"),
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
        // A raised travel height is listed after the other values, whatever
        // the strategy; the default, set or not, is not.
        let mut raised = ops[0].clone();
        raised.p.insert("clearance".into(), "30".into());
        let line = crate::ui::operation_lines(&raised, "T1", 1., "")[1].clone();
        assert!(
            line.ends_with(" · bottom offset 0.5 mm · clearance 30 mm"),
            "{line}"
        );
        raised.p.insert("clearance".into(), "22.5".into());
        assert!(
            crate::ui::operation_lines(&raised, "T1", 1., "")[1].ends_with(" · clearance 22.5 mm")
        );
        raised.p.insert("clearance".into(), "15".into());
        assert_eq!(
            crate::ui::operation_lines(&raised, "T1", 1., "")[1],
            lines(0)[1]
        );
        let mut drilled = ops[3].clone();
        drilled.p.insert("clearance".into(), "40".into());
        assert!(
            crate::ui::operation_lines(&drilled, "T3", 1., "")[1].ends_with(" · clearance 40 mm")
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
        use crate::holding::Held;
        use crate::ui::{plan_check_lines, plan_not_checked, plan_work_zero_line, program_note};
        // The preview's check lines; the second names the machine limits
        // that speeds and feeds are held to.
        assert_eq!(
            plan_check_lines(false),
            [
                "Every operation generates with the stock left by the ones before it",
                "LinuxCNC export checks pass: machine rpm and feed limits, travel, rapids into material, holder clearance",
                "No tool centre line cuts into the part, and nothing goes below the stock",
                "No gouging detected on the stock grid (coarse check)",
            ]
        );
        // The note under the lines says what is not checked: fixtures and
        // clamps for a plan without modelled workholding, parallels, bolts,
        // the table and the machine body when every setup has a vise or clamps.
        assert_eq!(
            plan_not_checked(Held::No, false),
            "Not checked: cutting loads, tool deflection, fixtures, clamps and the machine body. Speeds and feeds are checked against the material's cutting data, which are starting points."
        );
        assert_eq!(
            plan_not_checked(Held::Every, false),
            "Not checked: cutting loads, tool deflection, parallels, bolts, the table, the machine body and tool-change motion. Vise jaws and clamps are checked as boxes with a clearance gap in every setup; retracts to machine home are assumed to clear everything. Speeds and feeds are checked against the material's cutting data, which are starting points."
        );
        assert_eq!(
            plan_not_checked(Held::Mixed, false),
            plan_not_checked(Held::No, false)
        );
        // A UC100 plan is checked as a run checks it, whose retracts go to
        // the clearance plane, not to machine home.
        assert_eq!(
            plan_check_lines(true)[1],
            "UC100 run checks pass: machine rpm and feed limits, travel, rapids into material, holder clearance, the profile's spindle and coolant outputs"
        );
        assert!(
            plan_not_checked(Held::Every, true)
                .contains("in every setup, retracts to the clearance plane included.")
        );
        assert_eq!(
            plan_not_checked(Held::No, true),
            plan_not_checked(Held::No, false)
        );
        // The page beside the Plan button says the cutting data are sent.
        assert!(
            crate::ui::SENT_TO_OPENAI.contains(
                "your tools, machine limits, the project material (its name, group, hardness, notes and cutting data),"
            )
        );
        // The program page says what export repeats, and that the planner's
        // own checks covered the plan as applied, not later edits.
        assert_eq!(
            program_note(Held::No),
            "LinuxCNC 3-axis output, one .ngc program per setup. Export repeats the export checks — limits, travel, rapids into material, holder clearance — and a replay of each program against the simulated moves, and writes nothing if any setup fails. The planner's tool-axis and gouge checks covered the plan as applied, not later edits. Fixtures, clamps and the machine body are not checked: preview each program in LinuxCNC and air-cut first."
        );
        let base = "LinuxCNC 3-axis output, one .ngc program per setup. Export repeats the export checks — limits, travel, rapids into material, holder clearance — and a replay of each program against the simulated moves, and writes nothing if any setup fails. The planner's tool-axis and gouge checks covered the plan as applied, not later edits. ";
        assert_eq!(
            program_note(Held::Every),
            format!(
                "{base}Every program move, the links between operations, approaches and retracts included, is checked against each setup's vise jaws or clamps as boxes with a clearance gap; parallels, bolts, the table, the machine body and tool-change motion are not checked, and retracts to machine home are assumed to clear everything: preview each program in LinuxCNC and air-cut first."
            )
        );
        assert_eq!(
            program_note(Held::Mixed),
            format!(
                "{base}Every program move of a setup that models a vise or clamps, the links between operations, approaches and retracts included, is checked against its jaws or clamps as boxes with a clearance gap; parallels, bolts, the table, the machine body and tool-change motion are not checked, and neither are fixtures and clamps in the setups whose workholding is not modelled or cannot be checked; retracts to machine home are assumed to clear everything: preview each program in LinuxCNC and air-cut first."
            )
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
    fn the_plan_card_shows_each_setups_modelled_workholding_and_what_is_checked_of_it() {
        use crate::ui::plan_holding_line;
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        // Without modelled workholding the cards say nothing of it.
        let preview = checked_preview(&a, false);
        assert_eq!(plan_holding_line(&preview.outcome.job, 0), None);
        assert_eq!(plan_holding_line(&preview.outcome.job, 1), None);
        assert_eq!(plan_holding_line(&preview.outcome.job, 2), None);
        // Its note under the checks keeps the old wording.
        assert_eq!(
            crate::holding::held(&preview.outcome.job.setups),
            crate::holding::Held::No
        );
        // Setup 1 held by clamps with room round the part: its card names
        // them, and setup 2's the layout the plan check suggested.
        for key in ["stock_left", "stock_right", "stock_front", "stock_back"] {
            a.doc.setups[0].p.insert(key.into(), "36".into());
        }
        crate::proposal::tests::hold(&mut a.doc, "Clamp", &[("clamps", "-X:0;+X:0")]);
        let preview = checked_preview_of(&a, false, |proposal| {
            for setup in &mut proposal.setups {
                setup.operations[0].facing_area = Some("model".into());
            }
        });
        let job = &preview.outcome.job;
        // Every setup of the plan models its clamps, so the note says so.
        assert_eq!(
            crate::holding::held(&job.setups),
            crate::holding::Held::Every
        );
        let checked = "Jaws and clamps are checked as boxes with the gap; parallels, bolts, the table and the machine body are not.";
        assert_eq!(
            plan_holding_line(job, 0).unwrap(),
            format!(
                "Modelled workholding: 2 \u{d7} Toe clamp 60 mm at -X 0, +X 0, gap 2 mm. {checked}"
            )
        );
        let later = plan_holding_line(job, 1).unwrap();
        assert!(
            later.starts_with(
                "Modelled workholding, laid out by the app from this setup's toolpaths: "
            ) && later.contains(" \u{d7} Toe clamp 60 mm at ")
                && later.ends_with(checked),
            "{later}"
        );
        // The page builds with the lines.
        a.planner = crate::Planner::Preview(preview);
        a.page = Page::Ai;
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
    fn the_disclosure_names_pictures_only_when_they_are_on() {
        use crate::ui::sent_to_openai;
        assert!(sent_to_openai(true).contains("pictures the app draws of the part"));
        assert!(!sent_to_openai(false).contains("pictures"));
        assert!(sent_to_openai(true).contains("The CAD file"));
        assert_eq!(sent_to_openai(false), crate::ui::SENT_TO_OPENAI);
        // The library's sizes go whatever the pictures choice, and only sizes.
        // So do the check reports and the tools' answers, which the model
        // gets whenever it asks, not only after an answer that failed.
        for pictures in [true, false] {
            let text = sent_to_openai(pictures);
            assert!(text.contains(
                "the sizes of every vise and clamp in your workholding library (sizes only, never their names or notes), and the app's check reports and tool answers while the model works."
            ));
            assert!(!text.contains("correction rounds"), "{text}");
        }
        assert_eq!(
            sent_to_openai(true),
            format!("{}{}", crate::ui::SENT_TO_OPENAI, crate::ui::PICTURES_SENT)
        );
    }
    #[test]
    fn the_pictures_choice_is_saved_and_shown_on_the_planner_page() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        let file = dir.path().join("settings.json");
        a.page = Page::Ai;
        let _ = a.view(); // pictures on, the default
        assert!(!a.settings.plan_pictures_off);
        let _ = a.update(Message::PlanPictures(false));
        assert!(a.settings.plan_pictures_off);
        assert!(crate::settings::load(&file).plan_pictures_off);
        let _ = a.view(); // pictures off
        let _ = a.update(Message::PlanPictures(true));
        assert!(!a.settings.plan_pictures_off);
        assert!(!crate::settings::load(&file).plan_pictures_off);
        let _ = a.update(Message::PlanPictures(false));
        let reopened = App::from_workspace(dir.path().join("workspace.json"), None);
        assert!(reopened.settings.plan_pictures_off);
    }
    #[test]
    fn the_pictures_choice_is_fixed_while_a_run_goes() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let file = dir.path().join("settings.json");
        a.page = Page::Ai;
        let _ = a.update(Message::Plan);
        assert!(matches!(a.planner, crate::Planner::Running(_)));
        let _ = a.view(); // the box disabled
        let _ = a.update(Message::PlanPictures(false));
        assert!(!a.settings.plan_pictures_off);
        assert!(!crate::settings::load(&file).plan_pictures_off);
        // Once the run is over the box is the user's again.
        let _ = a.update(Message::PlanCancel);
        let _ = a.update(Message::PlanPictures(false));
        assert!(a.settings.plan_pictures_off);
        assert!(crate::settings::load(&file).plan_pictures_off);
        // Its hover help says what is sent, what never is, and that a run
        // keeps its choice.
        let help = crate::ui::PICTURES_HELP;
        assert!(
            help.contains("pictures the app draws of the part"),
            "{help}"
        );
        assert!(help.contains("The CAD file is never sent."), "{help}");
        let more = crate::ui::PICTURES_HELP_MORE;
        assert!(
            more.contains("A run keeps the choice it started with"),
            "{more}"
        );
    }
    #[tokio::test]
    async fn a_run_sends_pictures_and_the_tool_only_as_the_box_was_when_it_started() {
        use crate::fake_openai::{Fake, Resp};
        use crate::session::SessionStore;
        let dir = tempfile::tempdir().unwrap();
        let mut a = signed_in_planning_app(&dir);
        let mesh = a.imported_mesh.clone().unwrap();
        let report = crate::analysis::report(&a.doc, &mesh).unwrap();
        let plan = serde_json::to_string(&crate::proposal::tests::good(&report)).unwrap();
        let fake = Fake::start(move |_| Resp::answer(&plan)).await;
        let store = std::sync::Arc::new(crate::session::FileStore(dir.path().join("s.json")));
        let session = crate::session::sample();
        store.save(&session).unwrap();
        let client = std::sync::Arc::new(crate::openai::Client::new(
            crate::oauth::Endpoints::local(&fake.base),
            "oaiapp_1".into(),
            store,
            session,
        ));
        for off in [true, false] {
            let _ = a.update(Message::PlanPictures(!off));
            assert_eq!(a.settings.plan_pictures_off, off);
            // The backend `Plan` starts the run with; the box changed after
            // that does not reach the run.
            let backend = a.plan_backend(client.clone());
            let _ = a.update(Message::PlanPictures(off));
            fake.requests.lock().unwrap().clear();
            let outcome = crate::planner::plan(
                &backend,
                a.doc.clone(),
                mesh.clone(),
                &crate::planner::Busy::default(),
                |_| {},
            )
            .await;
            assert!(outcome.is_ok(), "{:?}", outcome.err().map(|f| f.message));
            let sent = fake.seen("/v1/responses");
            assert!(!sent.is_empty());
            let tools = sent.iter().filter(|r| !r.json()["tools"].is_null()).count();
            let images = sent
                .iter()
                .filter(|r| r.body.contains("\"input_image\""))
                .count();
            // The tools every request offers, by name.
            let names = |r: &crate::fake_openai::Req| -> Vec<String> {
                r.json()["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|t| t["name"].as_str().unwrap().to_string())
                    .collect()
            };
            let part = [
                "check_plan",
                "stock_after",
                "check_holding",
                "cutting_data",
                "surface_info",
            ];
            if off {
                // The part tools need no pictures: offered, without view_model.
                assert_eq!(
                    (tools, images),
                    (sent.len(), 0),
                    "pictures off: the tools, no picture"
                );
                assert!(sent.iter().all(|r| names(r) == part));
                assert!(sent.iter().all(|r| !r.body.contains("PICTURES")));
                assert!(sent.iter().all(|r| !r.body.contains("view_model")));
            } else {
                assert_eq!(
                    (tools, images),
                    (sent.len(), sent.len()),
                    "the overview and the tools in every request"
                );
                assert!(
                    sent.iter()
                        .all(|r| names(r)[0] == "view_model" && names(r)[1..] == part)
                );
            }
        }
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
        let _ = a.update(Message::SignedIn(
            a.sign_in_attempt,
            Err("Sign-in timed out. Try again.".into()),
        ));
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
        let saved = crate::session::default_store(dir.path(), true, false).load();
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
    /// The files beside a workspace that are an unreadable settings file kept aside.
    fn settings_kept_aside(dir: &tempfile::TempDir) -> Vec<std::path::PathBuf> {
        let mut found: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                let name = p.file_name().unwrap().to_string_lossy();
                name.starts_with("settings-unreadable-") && name.ends_with(".json")
            })
            .collect();
        found.sort();
        found
    }
    #[test]
    fn an_unreadable_settings_file_is_kept_aside_and_the_saved_sign_in_still_restores() {
        let dir = tempfile::tempdir().unwrap();
        let _ = signed_in_app(&dir);
        // One wrong-typed field: serde rejects the whole document.
        let file = dir.path().join("settings.json");
        let mut doc: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        doc["model"] = serde_json::json!(5);
        std::fs::write(&file, doc.to_string()).unwrap();
        let mut a = fresh_app(&dir);
        let kept = settings_kept_aside(&dir);
        assert_eq!(
            kept.len(),
            1,
            "the unreadable file is renamed, not replaced"
        );
        assert_eq!(
            std::fs::read(&kept[0]).unwrap(),
            doc.to_string().into_bytes()
        );
        let name = kept[0].file_name().unwrap().to_string_lossy().to_string();
        assert!(a.account_note.contains("could not be read") && a.account_note.contains(&name));
        // The client ID was read from the broken file, so the sign-in is used.
        assert_eq!(a.settings.client_id, "oaiapp_1");
        let _ = a.update(Message::Page(Page::Ai));
        let saved = crate::session::default_store(dir.path(), true, false).load();
        let _ = a.update(Message::AccountRestored(saved));
        assert!(matches!(
            a.account,
            crate::Account::SignedIn { granted: true, .. }
        ));
        // The new settings file was written, and a second start keeps it.
        let _ = fresh_app(&dir);
        assert_eq!(settings_kept_aside(&dir).len(), 1);
    }
    #[test]
    fn a_saved_sign_in_with_no_client_id_is_reported_and_deleted_not_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let _ = signed_in_app(&dir);
        // The client ID itself is wrong-typed, so nothing of it can be kept.
        let file = dir.path().join("settings.json");
        let mut doc: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        doc["client_id"] = serde_json::json!(5);
        std::fs::write(&file, doc.to_string()).unwrap();
        let mut a = fresh_app(&dir);
        assert_eq!(settings_kept_aside(&dir).len(), 1);
        assert!(a.settings.client_id.is_empty());
        assert_eq!(
            a.update(Message::Page(Page::Ai)).units(),
            1,
            "the store is read"
        );
        assert!(a.restoring);
        let store = crate::session::default_store(dir.path(), true, false);
        let result = crate::read_saved_session(&*store, true);
        let _ = a.update(Message::AccountRestored(result));
        assert!(!a.restoring && a.account == crate::Account::SignedOut && a.client.is_none());
        assert!(
            a.account_note.contains("could not be read"),
            "{}",
            a.account_note
        );
        assert!(
            a.account_note.contains("deleted from this computer"),
            "{}",
            a.account_note
        );
        assert_eq!(store.load().unwrap(), None);
        assert_eq!(a.update(Message::SignIn).units(), 1);
        // With a client ID the saved sign-in is only read.
        let _ = signed_in_app(&dir);
        assert!(crate::read_saved_session(&*store, false).unwrap().is_some());
        assert!(store.load().unwrap().is_some());
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
        let late = Message::SignedIn(
            a.sign_in_attempt,
            Ok((crate::session::sample(), "oaiapp_1".into())),
        );
        assert_eq!(a.update(late).units(), 1);
        assert_eq!(a.account, crate::Account::SignedOut);
        assert_eq!(a.account_note, "Sign-in cancelled.");
        assert!(a.settings.client_id.is_empty() && a.models.is_empty());
        assert!(a.client.is_none());
        let late = Message::SignedIn(
            a.sign_in_attempt,
            Err("Sign-in timed out. Try again.".into()),
        );
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
        let late = Message::SignedIn(
            a.sign_in_attempt,
            Ok((crate::session::sample(), "oaiapp_1".into())),
        );
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
        let saved = crate::session::default_store(dir.path(), true, false).load();
        let _ = a.update(Message::AccountRestored(saved));
        let _ = a.update(Message::Page(Page::Home));
        assert_eq!(a.update(Message::Page(Page::Ai)).units(), 0);
        // A visit before any sign-in uses up the restore too, so signing in
        // and out afterwards never reads the saved session behind it.
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        // With no client ID the store is still read, in case a sign-in is
        // saved that the app can no longer use; here there is none.
        assert_eq!(a.update(Message::Page(Page::Ai)).units(), 1);
        let _ = a.update(Message::AccountRestored(Ok(None)));
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
        let result = Message::SignedIn(a.sign_in_attempt, Ok((session.clone(), "oaiapp_1".into())));
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
        let result = Message::SignedIn(
            a.sign_in_attempt,
            Ok((crate::session::sample(), "oaiapp_1".into())),
        );
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
        let result = Message::SignedIn(a.sign_in_attempt, Ok((session.clone(), "oaiapp_1".into())));
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
    /// Runs `task` on the current runtime, handing each message it produces
    /// to `tx`; `pending` counts the tasks that have not ended.
    fn pump_task(
        task: Task<Message>,
        tx: tokio::sync::mpsc::UnboundedSender<Message>,
        pending: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        use futures_util::StreamExt;
        use std::sync::atomic::Ordering::SeqCst;
        let Some(mut stream) = iced_runtime::task::into_stream(task) else {
            return;
        };
        pending.fetch_add(1, SeqCst);
        tokio::spawn(async move {
            while let Some(action) = stream.next().await {
                if let iced_runtime::Action::Output(message) = action {
                    let _ = tx.send(message);
                }
            }
            pending.fetch_sub(1, SeqCst);
        });
    }
    #[test]
    fn a_cancel_or_sign_out_after_the_token_endpoint_answered_loses_no_tokens() {
        use crate::fake_openai::{Fake, Resp, claims, id_token, jwks, tokens};
        use std::sync::{Arc, Mutex, atomic::AtomicUsize, atomic::Ordering::SeqCst};
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        for sign_out in [false, true] {
            runtime.block_on(async {
                let dir = tempfile::tempdir().unwrap();
                let mut a = fresh_app(&dir);
                let id_claims = Arc::new(Mutex::new(serde_json::Value::Null));
                let served = id_claims.clone();
                // The token endpoint answers at once; the signing keys stall,
                // so the exchange is still running when the user acts.
                let fake = Fake::start(move |r| match r.path.as_str() {
                    "/jwks" => Resp {
                        delay_ms: 700,
                        ..Resp::json(200, jwks())
                    },
                    "/token" => Resp::json(
                        200,
                        tokens(
                            "access-1",
                            "refresh-1",
                            &id_token(served.lock().unwrap().clone()),
                            &crate::session::sample().scope,
                        ),
                    ),
                    _ => Resp::json(404, serde_json::json!({})),
                })
                .await;
                a.endpoints = crate::oauth::Endpoints::local(&fake.base);
                a.open_browser = Arc::new(move |url: &str| {
                    let url = reqwest::Url::parse(url).unwrap();
                    let get = |key: &str| {
                        url.query_pairs()
                            .find(|(k, _)| k == key)
                            .map(|(_, v)| v.into_owned())
                            .unwrap_or_default()
                    };
                    *id_claims.lock().unwrap() = claims("oaiapp_1", &get("nonce"));
                    let callback = format!(
                        "{}?code=code-1&state={}&client_id=oaiapp_1",
                        get("redirect_uri"),
                        get("state")
                    );
                    tokio::spawn(async move { reqwest::get(callback).await });
                    Ok(())
                });
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                let pending = Arc::new(AtomicUsize::new(0));
                pump_task(a.update(Message::SignIn), tx.clone(), pending.clone());
                // The token endpoint has answered once the keys are asked for.
                let mut waited = 0;
                while fake.seen("/jwks").is_empty() {
                    assert!(waited < 400, "the sign-in never reached the keys");
                    waited += 1;
                    while let Ok(message) = rx.try_recv() {
                        pump_task(a.update(message), tx.clone(), pending.clone());
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
                assert_eq!(fake.seen("/token").len(), 1);
                let act = if sign_out {
                    Message::SignOut
                } else {
                    Message::SignInCancel
                };
                pump_task(a.update(act), tx.clone(), pending.clone());
                loop {
                    if let Ok(message) = rx.try_recv() {
                        pump_task(a.update(message), tx.clone(), pending.clone());
                    } else if pending.load(SeqCst) == 0 {
                        break;
                    } else {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                }
                // The issued refresh token is either kept or revoked.
                let saved = a.store.load().unwrap().is_some();
                let revoked = !fake.seen("/revoke").is_empty();
                assert!(saved || revoked, "sign_out={sign_out}: tokens were lost");
                assert!(
                    saved != matches!(a.account, crate::Account::SignedOut),
                    "sign_out={sign_out}: the account must match the saved sign-in"
                );
            });
        }
    }
    #[test]
    fn a_result_of_a_superseded_sign_in_changes_nothing_and_is_revoked() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = fresh_app(&dir);
        let _ = a.update(Message::SignIn);
        let first = a.sign_in_attempt;
        let _ = a.update(Message::SignInCancel);
        let _ = a.update(Message::SignIn);
        assert!(a.sign_in_attempt != first);
        let handle = a.sign_in.clone().expect("the second sign-in is running");
        let before = snapshot(&a);
        // The first attempt's exchange ends: revoked, not adopted, and the
        // second attempt is left running.
        let late = Message::SignedIn(first, Ok((crate::session::sample(), "oaiapp_1".into())));
        assert_eq!(a.update(late).units(), 1);
        assert_eq!(snapshot(&a), before);
        assert!(a.sign_in.is_some() && !handle.is_aborted() && !a.saving_sign_in());
        assert!(a.settings.client_id.is_empty() && a.client.is_none());
        // Neither its failure nor its browser step touches the second one.
        let late = Message::SignedIn(first, Err("The ID token is not valid.".into()));
        assert_eq!(a.update(late).units(), 0);
        let late = Message::Approved(first, Err("Sign-in timed out. Try again.".into()));
        assert_eq!(a.update(late).units(), 0);
        assert_eq!(snapshot(&a), before);
        assert!(a.sign_in.is_some() && a.account_note.is_empty());
        // Starting over while a handle is left behind aborts it.
        a.account = crate::Account::SignedOut;
        let _ = a.update(Message::SignIn);
        assert!(handle.is_aborted());
        assert!(a.sign_in.is_some());
    }
    #[test]
    fn cancelling_during_the_exchange_and_signing_in_again_adopts_the_second_account() {
        use crate::fake_openai::{Fake, Resp, claims, id_token, jwks, tokens};
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex, atomic::AtomicUsize, atomic::Ordering::SeqCst};
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let mut a = fresh_app(&dir);
            // The claims of each code, filled in when its browser opens.
            let id_claims = Arc::new(Mutex::new(HashMap::<String, serde_json::Value>::new()));
            let served = id_claims.clone();
            let keys = Arc::new(AtomicUsize::new(0));
            let fake = Fake::start(move |r| match r.path.as_str() {
                // The first attempt's keys stall, so its exchange is still
                // running when the user cancels and starts over.
                "/jwks" if keys.fetch_add(1, SeqCst) == 0 => Resp {
                    delay_ms: 700,
                    ..Resp::json(200, jwks())
                },
                "/jwks" => Resp::json(200, jwks()),
                "/revoke" => Resp::json(200, serde_json::json!({})),
                "/token" => {
                    let code = r.field("code");
                    let claims = served.lock().unwrap()[&code].clone();
                    let refresh = format!("refresh-{code}");
                    let scope = crate::session::sample().scope;
                    Resp::json(200, tokens("access", &refresh, &id_token(claims), &scope))
                }
                _ => Resp::json(404, serde_json::json!({})),
            })
            .await;
            a.endpoints = crate::oauth::Endpoints::local(&fake.base);
            // The second browser waits for the test before it approves.
            let callbacks = Arc::new(Mutex::new(Vec::<String>::new()));
            let opened = Arc::new(AtomicUsize::new(0));
            let (stored, count) = (callbacks.clone(), opened.clone());
            a.open_browser = Arc::new(move |url: &str| {
                let url = reqwest::Url::parse(url).unwrap();
                let get = |key: &str| {
                    url.query_pairs()
                        .find(|(k, _)| k == key)
                        .map(|(_, v)| v.into_owned())
                        .unwrap_or_default()
                };
                let n = count.fetch_add(1, SeqCst) + 1;
                let mut c = claims("oaiapp_1", &get("nonce"));
                c["sub"] = format!("user-{n}").into();
                c["name"] = format!("Account {n}").into();
                id_claims.lock().unwrap().insert(format!("code-{n}"), c);
                let callback = format!(
                    "{}?code=code-{n}&state={}&client_id=oaiapp_1",
                    get("redirect_uri"),
                    get("state")
                );
                if n == 1 {
                    tokio::spawn(async move { reqwest::get(callback).await });
                } else {
                    stored.lock().unwrap().push(callback);
                }
                Ok(())
            });
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let pending = Arc::new(AtomicUsize::new(0));
            pump_task(a.update(Message::SignIn), tx.clone(), pending.clone());
            let mut waited = 0;
            while fake.seen("/jwks").is_empty() {
                assert!(waited < 400, "the sign-in never reached the keys");
                waited += 1;
                while let Ok(message) = rx.try_recv() {
                    pump_task(a.update(message), tx.clone(), pending.clone());
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            // Cancel during the exchange, then sign in as the other account.
            pump_task(a.update(Message::SignInCancel), tx.clone(), pending.clone());
            pump_task(a.update(Message::SignIn), tx.clone(), pending.clone());
            assert_eq!(a.account, crate::Account::SigningIn);
            // The first exchange ends while the second browser step still
            // waits: its tokens must not be adopted.
            let mut waited = 0;
            let mut ended = false;
            while !ended {
                assert!(waited < 400, "the first exchange never ended");
                waited += 1;
                match rx.try_recv() {
                    Ok(message) => {
                        ended = matches!(message, Message::SignedIn(..));
                        pump_task(a.update(message), tx.clone(), pending.clone());
                    }
                    Err(_) => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
                }
            }
            assert_eq!(a.account, crate::Account::SigningIn);
            assert!(a.sign_in.is_some() && !a.saving_sign_in());
            // Now the second browser approves.
            let callback = callbacks.lock().unwrap().pop().expect("a second browser");
            tokio::spawn(async move { reqwest::get(callback).await });
            loop {
                if let Ok(message) = rx.try_recv() {
                    pump_task(a.update(message), tx.clone(), pending.clone());
                } else if pending.load(SeqCst) == 0 {
                    break;
                } else {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }
            match &a.account {
                crate::Account::SignedIn { name, .. } => assert_eq!(name, "Account 2"),
                other => panic!("the second account was not adopted: {other:?}"),
            }
            assert_eq!(
                a.store.load().unwrap().unwrap().refresh_token,
                "refresh-code-2"
            );
            let revoked: Vec<String> = fake
                .seen("/revoke")
                .iter()
                .map(|r| r.field("token"))
                .collect();
            assert_eq!(revoked, ["refresh-code-1"]);
            assert!(a.sign_in.is_none());
        });
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
        let result = Message::SignedIn(
            a.sign_in_attempt,
            Ok((crate::session::sample(), "oaiapp_1".into())),
        );
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
        let result = Message::SignedIn(a.sign_in_attempt, Ok((session.clone(), "oaiapp_1".into())));
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
        let first = Message::SignedIn(a.sign_in_attempt, Ok((session.clone(), "oaiapp_1".into())));
        let _ = a.update(first);
        assert!(a.saving_sign_in());
        let before = snapshot(&a);
        let other = crate::session::Session {
            name: "Other".into(),
            refresh_token: "refresh-other".into(),
            ..crate::session::sample()
        };
        // The only task is the revoke of the unused session.
        let late = Message::SignedIn(a.sign_in_attempt, Ok((other, "oaiapp_other".into())));
        assert_eq!(a.update(late).units(), 1);
        assert_eq!(snapshot(&a), before);
        assert_eq!(a.settings.client_id, "oaiapp_1");
        let saved = crate::settings::load(&dir.path().join("settings.json"));
        assert_eq!(saved.client_id, "oaiapp_1");
        assert!(a.saving_sign_in());
        // A late failure neither ends the save step nor shows a message.
        let late = Message::SignedIn(a.sign_in_attempt, Err("The browser sign-in failed.".into()));
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
        let late = Message::SignedIn(
            a.sign_in_attempt,
            Ok((other.clone(), "oaiapp_other".into())),
        );
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
    #[test]
    fn the_holding_step_builds_with_the_gap_fields_named_for_what_they_do() {
        use crate::ui::{FIXTURE_CLEARANCE_LABEL, HOLDING_GAP_LABEL};
        let dir = tempfile::tempdir().unwrap();
        let mut a = holding_app(&dir);
        a.page = Page::Work;
        a.selected = None;
        a.setup_step = "Holding";
        // The step builds with None, a vise and clamps: the checked gap only
        // with a vise or clamps, the requested clearance always (issue #72).
        for holding in ["None", "Vise", "Clamps"] {
            edit_setup(&mut a, &[("holding", holding)]);
            let _ = a.view();
        }
        assert!(
            HOLDING_GAP_LABEL.starts_with("Checked gap")
                && FIXTURE_CLEARANCE_LABEL.ends_with("not checked")
                && HOLDING_GAP_LABEL != FIXTURE_CLEARANCE_LABEL
        );
        // The requested clearance is printed on the setup sheet after the
        // reposition, as a note that is not checked, "not set" when blank.
        edit_setup(&mut a, &[("clearance", "15")]);
        let reposition = get(&a.doc.setup().p, "reposition").to_string();
        let sheet = a.doc.setup_sheet(None);
        assert!(
            sheet.contains(&format!(
                "Reposition: {reposition}\n\nRequested fixture clearance (not checked): 15 mm\n\n"
            )),
            "{sheet}"
        );
        edit_setup(&mut a, &[("clearance", " ")]);
        assert!(
            a.doc
                .setup_sheet(None)
                .contains("Requested fixture clearance (not checked): not set\n\n")
        );
    }
    /// The example on the Work page, in its own folder, so that the settings
    /// saved beside it can be read back.
    fn sample_in(dir: &tempfile::TempDir) -> App {
        let mut a = fresh_app(dir);
        a.doc = Document::sample();
        a.selected = Some(0);
        a.page = Page::Work;
        a
    }
    /// Beside the 3D view the browser and the inspector fold to a strip and
    /// resize by their handles; the 3D view keeps its least width in the
    /// window at its least; the layout is saved with the settings and a new
    /// app starts with it (#76).
    #[test]
    fn the_sidebars_fold_and_resize_and_the_layout_is_saved() {
        use crate::settings::{self, Side};
        use iced::advanced::widget::Tree;
        fn nodes(t: &Tree) -> usize {
            1 + t.children.iter().map(nodes).sum::<usize>()
        }
        let dir = tempfile::tempdir().unwrap();
        let settings_file = settings::path(&dir.path().join("workspace.json"));
        let mut a = sample_in(&dir);
        let layout = |a: &App| a.settings.layout.clone();
        assert_eq!(layout(&a), settings::Layout::default());
        let browser = |a: &App| nodes(&Tree::new(crate::ui::browser(a).as_widget()));
        let inspector = |a: &App| nodes(&Tree::new(crate::ui::inspector(a).as_widget()));
        let (open_browser, open_inspector) = (browser(&a), inspector(&a));
        // Hide: each sidebar becomes its strip, with the Show button only.
        let _ = a.update(Message::ToggleSidebar(Side::Browser));
        assert!(!layout(&a).browser_open && layout(&a).inspector_open);
        assert!(browser(&a) < open_browser);
        assert_eq!(inspector(&a), open_inspector);
        let _ = a.update(Message::ToggleSidebar(Side::Inspector));
        assert!(!layout(&a).inspector_open);
        assert!(inspector(&a) < open_inspector);
        let _ = a.view();
        // Both choices are saved at once.
        assert_eq!(settings::load(&settings_file).layout, layout(&a));
        // Show again.
        let _ = a.update(Message::ToggleSidebar(Side::Browser));
        let _ = a.update(Message::ToggleSidebar(Side::Inspector));
        assert_eq!(layout(&a), settings::Layout::default());
        assert_eq!((browser(&a), inspector(&a)), (open_browser, open_inspector));
        // A drag moves the width while it goes and saves it when it ends.
        let _ = a.update(Message::SidebarWidth(Side::Browser, 300., false));
        assert_eq!(layout(&a).browser_width, 300.);
        assert_eq!(
            settings::load(&settings_file).layout.browser_width,
            crate::ui::BROWSER.default
        );
        let _ = a.update(Message::SidebarWidth(Side::Browser, 310., true));
        assert_eq!(settings::load(&settings_file).layout.browser_width, 310.);
        // Dragged past its range, a sidebar stops at the range's end.
        let _ = a.update(Message::SidebarWidth(Side::Inspector, 9999., true));
        assert_eq!(layout(&a).inspector_width, crate::ui::INSPECTOR.max);
        let _ = a.update(Message::SidebarWidth(Side::Inspector, 1., true));
        assert_eq!(layout(&a).inspector_width, crate::ui::INSPECTOR.min);
        let _ = a.update(Message::SidebarWidth(Side::Browser, 9999., true));
        assert_eq!(layout(&a).browser_width, crate::ui::BROWSER.max);
        // In the window at its least width the 3D view keeps its least: the
        // browser, as drawn, gives way first.
        let least = crate::MIN_WINDOW.width;
        let _ = a.update(Message::Resized(least));
        assert_eq!(a.window_width, least);
        let l = layout(&a);
        let (b, i) = crate::ui::fitted(&l, least);
        assert!(b < l.browser_width && i == l.inspector_width, "{b} {i}");
        assert!(crate::ui::view_width(&l, least) >= crate::ui::VIEW_MIN);
        // The inspector dragged wide there pushes the browser to its least
        // and stops where the view keeps its least; the browser's saved
        // width is kept for a wider window.
        let _ = a.update(Message::SidebarWidth(Side::Inspector, 9999., true));
        let l = layout(&a);
        assert!(
            l.inspector_width > crate::ui::INSPECTOR.min
                && l.inspector_width < crate::ui::INSPECTOR.max,
            "{}",
            l.inspector_width
        );
        assert_eq!(
            crate::ui::fitted(&l, least),
            (crate::ui::BROWSER.min, l.inspector_width)
        );
        assert_eq!(crate::ui::view_width(&l, least), crate::ui::VIEW_MIN);
        assert_eq!(l.browser_width, crate::ui::BROWSER.max);
        assert_eq!(
            crate::ui::fitted(&l, 1920.),
            (l.browser_width, l.inspector_width)
        );
        // The page builds at both window widths.
        let _ = a.view();
        let _ = a.update(Message::Resized(1920.));
        let _ = a.view();
        // A new app on the same workspace starts with the saved layout.
        assert_eq!(fresh_app(&dir).settings.layout, l);
    }
    /// Fit beside the camera views puts the scene back in the 3D view from
    /// where it looks: the zoom and the pan go back, the direction stays
    /// (#76).
    #[test]
    fn fit_refits_the_scene_and_keeps_the_views_direction() {
        use crate::viewport::Event;
        let mut a = app();
        let start = a.camera;
        for e in [
            Event::Orbit(0.3, 0.2),
            Event::Zoom(0.5),
            Event::Pan(30., -20.),
        ] {
            let _ = a.update(Message::Viewport(e));
        }
        let moved = a.camera;
        assert!(moved.zoom != 1. && moved.pan != [0., 0.] && moved.yaw != start.yaw);
        let _ = a.update(Message::Camera("FIT"));
        assert_eq!((a.camera.zoom, a.camera.pan), (1., [0., 0.]));
        assert_eq!((a.camera.yaw, a.camera.pitch), (moved.yaw, moved.pitch));
        // The named views still set the direction.
        let _ = a.update(Message::Camera("TOP"));
        assert_eq!(a.camera.pitch, std::f32::consts::FRAC_PI_2);
        // The button has its own help, and the page builds with it.
        let help = crate::help::command(&Message::Camera("FIT"), "Fit".into()).body;
        assert!(
            help.contains("zoom") && help.contains("direction stays"),
            "{help}"
        );
        let _ = a.view();
    }
    /// The operation inspector pins the operation's identity and one row of
    /// tabs above a scrolling body of parameters, and the checks, Generate
    /// and the operation's actions under it; the strategy's explanation is
    /// behind a disclosure, closed unless opened, and the choice is saved.
    /// The setup editor pins its title and steps and its actions the same
    /// way (#77).
    #[test]
    fn the_inspector_pins_the_identity_tabs_and_generate_and_scrolls_the_parameters() {
        use iced::advanced::widget::{Tree, tree::Tag};
        /// The buttons in a widget tree, by the state a button keeps.
        fn buttons(t: &Tree, button: Tag) -> usize {
            usize::from(t.tag == button)
                + t.children.iter().map(|c| buttons(c, button)).sum::<usize>()
        }
        /// The subtrees that are scrollables.
        fn scrollables<'t>(t: &'t Tree, scrollable: Tag, found: &mut Vec<&'t Tree>) {
            if t.tag == scrollable {
                found.push(t);
            }
            for c in &t.children {
                scrollables(c, scrollable, found);
            }
        }
        fn nodes(t: &Tree) -> usize {
            1 + t.children.iter().map(nodes).sum::<usize>()
        }
        let button =
            Tree::new(iced::Element::<Message>::from(iced::widget::button("x")).as_widget()).tag;
        let scrollable = Tree::new(
            iced::Element::<Message>::from(iced::widget::scrollable(iced::widget::text("x")))
                .as_widget(),
        )
        .tag;
        let dir = tempfile::tempdir().unwrap();
        let settings_file = crate::settings::path(&dir.path().join("workspace.json"));
        let mut a = sample_in(&dir);
        // The buttons pinned outside the one scrolling body, and those in it.
        let split = |a: &App| {
            let tree = Tree::new(crate::ui::inspector(a).as_widget());
            let mut scrolls = Vec::new();
            scrollables(&tree, scrollable, &mut scrolls);
            assert_eq!(scrolls.len(), 1, "one scrolling body");
            let inside = buttons(scrolls[0], button);
            (buttons(&tree, button) - inside, inside)
        };
        let strategy = a.operation().unwrap().strategy;
        assert_eq!(strategy, Strategy::Face);
        let tabs = strategy.tabs();
        assert_eq!(tabs.len(), 5);
        // Pinned: Hide, the five tabs, Generate, Duplicate, Suppress, Delete,
        // Move up and Move down; scrolling: the About disclosure, the tools
        // and the tool library at least.
        let (pinned, scrolled) = split(&a);
        assert_eq!(pinned, 1 + 5 + 1 + 3 + 2);
        assert!(scrolled >= 2 + a.doc.tools.len(), "{scrolled}");
        for t in &tabs {
            let _ = a.update(Message::Tab(t.to_string()));
            assert_eq!(split(&a).0, 12, "{t}");
            let _ = a.view();
        }
        let _ = a.update(Message::Tab("Tool".into()));
        // The strategy's explanation: behind the disclosure, closed at first,
        // and the choice is saved.
        assert!(!a.settings.layout.strategy_help_open);
        let closed = nodes(&Tree::new(crate::ui::inspector(&a).as_widget()));
        let _ = a.update(Message::ToggleStrategyHelp);
        assert!(a.settings.layout.strategy_help_open);
        assert!(nodes(&Tree::new(crate::ui::inspector(&a).as_widget())) > closed);
        assert_eq!(split(&a), (pinned, scrolled));
        assert!(
            crate::settings::load(&settings_file)
                .layout
                .strategy_help_open
        );
        assert!(fresh_app(&dir).settings.layout.strategy_help_open);
        let _ = a.view();
        let _ = a.update(Message::ToggleStrategyHelp);
        assert_eq!(
            nodes(&Tree::new(crate::ui::inspector(&a).as_widget())),
            closed
        );
        assert!(
            !crate::settings::load(&settings_file)
                .layout
                .strategy_help_open
        );
        // The setup editor: Hide, the four steps, Duplicate setup, Move up,
        // Move down and Delete setup… pinned; the step's fields scroll.
        let _ = a.update(Message::SetupEditor);
        let (pinned, scrolled) = split(&a);
        assert_eq!(pinned, 1 + crate::ui::SETUP_STEPS.len() + 1 + 2 + 1);
        assert!(scrolled >= 1, "{scrolled}");
        let _ = a.view();
        // With no setup, the inspector still has its one scrolling body.
        a.doc = Document::default();
        a.doc.sample_model = true;
        a.selected = None;
        let (pinned, scrolled) = split(&a);
        assert_eq!((pinned, scrolled), (1, 2));
        let _ = a.view();
    }
    fn with_manual(a: &mut App) {
        a.doc.machines.push(crate::manual::defaults());
    }
    /// A 10 mm end mill on the manual machine added by `with_manual`.
    fn with_big_cutter(a: &mut App) -> crate::model::Tool {
        with_manual(a);
        let mut tool = crate::model::Tool::new("Big cutter", "Flat end mill", "10");
        let id = a.doc.machines.last().unwrap().id.clone();
        tool.p.insert("machines".into(), id);
        a.doc.tools.insert(1, tool.clone());
        tool
    }
    fn lacks(a: &App, texts: &[&str]) {
        let seen = page_texts(a);
        for text in texts {
            assert!(!seen.iter().any(|t| t == text), "{text}");
        }
    }
    fn shows(a: &App, texts: &[&str]) {
        let seen = page_texts(a);
        for text in texts {
            assert!(seen.iter().any(|t| t == text), "{text}: {seen:?}");
        }
    }
    #[test]
    fn the_wizard_shows_the_manual_fields_and_hides_the_post_and_probe_ones() {
        let mut a = app();
        let _ = a.update(Message::NewMachine);
        let _ = a.update(Message::Edit(Target::Machine, "kind", "Manual".into()));
        assert!(crate::manual::is_manual(&a.machine_draft));
        shows(&a, &["Machine kind", "3  Manual mill"]);
        lacks(&a, &["Number of axes"]);
        let _ = a.update(Message::WizardStep(1));
        shows(
            &a,
            &[
                "X travel",
                "Spindle power · W",
                "Spindle speeds · rpm, or a range",
                "Hand feed · mm/min",
                "Hand rapid · mm/min",
                "X per handwheel turn · mm",
                "Y per handwheel turn · mm",
                "Z per handwheel turn · mm",
            ],
        );
        lacks(
            &a,
            &[
                "Maximum spindle speed · rpm",
                "Maximum feed · machine units/min",
            ],
        );
        // Steps 2 and 3 are one step; the last is the review.
        let _ = a.update(Message::WizardStep(2));
        shows(
            &a,
            &[
                "DRO tool radius compensation",
                "Time per pass · s",
                "Time per tool change · min",
                "Time to move the stock to another machine · min",
                "Manual mill — no G-code; MACHINE shows the steps.",
            ],
        );
        lacks(
            &a,
            &["Motion capability", "Controller / intended postprocessor"],
        );
        let _ = a.update(Message::WizardStep(9));
        assert_eq!(a.wizard_step, 3);
        shows(&a, &["Review", "Save machine profile"]);
        let clicks = crate::ui::probe::clicks(a.view(), WINDOW);
        assert!(clicks.iter().any(|m| matches!(m, Message::SaveMachine)));
        // The post and probe fields of a UC100 machine are on no step.
        let mut cnc = app();
        let _ = cnc.update(Message::NewMachine);
        cnc.machine_draft
            .p
            .insert("controller".into(), "UC100".into());
        let _ = cnc.update(Message::WizardStep(3));
        shows(
            &cnc,
            &["Spindle start dwell · s", "Touch plate thickness · mm"],
        );
        for step in 0..4 {
            let _ = a.update(Message::WizardStep(step));
            lacks(
                &a,
                &[
                    "Spindle start dwell · s",
                    "Touch plate thickness · mm",
                    "Probe feed · mm/min",
                    "Safe retract",
                    "Tool length",
                ],
            );
        }
    }
    #[test]
    fn choosing_the_manual_kind_keeps_the_name_travel_and_power() {
        let mut a = app();
        let _ = a.update(Message::NewMachine);
        for (k, v) in [("name", "Bridgeport"), ("x", "800"), ("power", "2200")] {
            let _ = a.update(Message::Edit(Target::Machine, k, v.into()));
        }
        let _ = a.update(Message::Edit(Target::Machine, "kind", "Manual".into()));
        let p = &a.machine_draft.p;
        assert_eq!(
            [
                get(p, "name"),
                get(p, "x"),
                get(p, "power"),
                get(p, "speeds")
            ],
            ["Bridgeport", "800", "2200", "90,150,250,400,640,1000,1600"]
        );
        let _ = a.update(Message::Edit(Target::Machine, "kind", "CNC".into()));
        assert!(!crate::manual::is_manual(&a.machine_draft));
        assert_eq!(crate::ui::wizard_steps(&a.machine_draft).len(), 5);
    }
    #[test]
    fn saving_a_manual_machine_settles_its_rpm_and_feed() {
        let mut a = app();
        let _ = a.update(Message::NewMachine);
        let _ = a.update(Message::Edit(Target::Machine, "kind", "Manual".into()));
        for (k, v) in [("speeds", "100, 400,2500"), ("hand_feed", "120")] {
            let _ = a.update(Message::Edit(Target::Machine, k, v.into()));
        }
        let id = a.machine_draft.id.clone();
        let _ = a.update(Message::SaveMachine);
        let saved = a.doc.machines.iter().find(|m| m.id == id).expect("saved");
        assert_eq!(
            (get(&saved.p, "rpm"), get(&saved.p, "feed")),
            ("2500", "120")
        );
        assert_eq!(a.page, Page::Machines);
        // A manual machine without gears is refused.
        let _ = a.update(Message::NewMachine);
        let _ = a.update(Message::Edit(Target::Machine, "kind", "Manual".into()));
        let _ = a.update(Message::Edit(Target::Machine, "speeds", "".into()));
        let count = a.doc.machines.len();
        let _ = a.update(Message::SaveMachine);
        assert_eq!(a.doc.machines.len(), count);
    }
    #[test]
    fn the_tool_editor_has_a_check_box_per_machine_that_edits_machines() {
        let mut a = app();
        with_manual(&mut a);
        let id = a.doc.machines.last().unwrap().id.clone();
        a.tool_draft = a.doc.tools[0].clone();
        a.page = Page::Tools;
        shows(&a, &["Machines", "Manual Mill"]);
        // Tall enough for the whole form, which scrolls in the real window.
        let sent = |a: &App| crate::ui::probe::clicks(a.view(), iced::Size::new(1440., 2400.));
        let mid = id.clone();
        let on = move |m: &Message, want: bool| matches!(m, Message::ToolMachine(i, o) if *i == mid && *o == want);
        assert!(sent(&a).iter().any(|m| on(m, true)));
        assert!(!sent(&a).iter().any(|m| on(m, false)));
        let _ = a.update(Message::ToolMachine(id.clone(), true));
        assert_eq!(get(&a.tool_draft.p, "machines"), id);
        assert!(sent(&a).iter().any(|m| on(m, false)));
        let other = a.doc.machines[0].id.clone();
        let _ = a.update(Message::ToolMachine(other.clone(), true));
        assert_eq!(get(&a.tool_draft.p, "machines"), format!("{id},{other}"));
        let _ = a.update(Message::ToolMachine(id, false));
        let _ = a.update(Message::ToolMachine(other, false));
        assert_eq!(get(&a.tool_draft.p, "machines"), "");
    }
    #[test]
    fn the_holder_editor_has_a_check_box_per_machine_that_edits_machines() {
        let mut a = app();
        with_manual(&mut a);
        let id = a.doc.machines.last().unwrap().id.clone();
        a.holder_draft = crate::holding::starters().remove(0);
        a.page = Page::Workholding;
        shows(&a, &["Machines", "Manual Mill"]);
        let _ = a.update(Message::HolderMachine(id.clone(), true));
        assert_eq!(get(&a.holder_draft.p, "machines"), id);
        let _ = a.update(Message::HolderMachine(id, false));
        assert_eq!(get(&a.holder_draft.p, "machines"), "");
    }
    #[test]
    fn a_holder_that_does_not_fit_the_setups_machine_is_missing_from_the_picker() {
        let mut a = app();
        with_manual(&mut a);
        let manual_id = a.doc.machines.last().unwrap().id.clone();
        a.doc.workholding = crate::holding::starters();
        let mut big = crate::holding::Holder::new("Vise", "Big vise");
        big.p.insert("machines".into(), manual_id);
        a.doc.workholding.push(big);
        let names = |a: &App| -> Vec<String> {
            crate::ui::fitting_holders(a, "Vise")
                .iter()
                .map(|h| h.name().to_string())
                .collect()
        };
        // A CNC setup sees the vises with no machines, not the big one.
        assert!(names(&a).contains(&"100 mm machine vise".to_string()));
        assert!(!names(&a).contains(&"Big vise".to_string()));
        // The manual setup sees only the big one.
        a.doc.add_manual_setup().unwrap();
        a.doc.active = a.doc.setups[0].id.clone();
        assert_eq!(names(&a), ["Big vise"]);
        assert_eq!(crate::ui::fitting_holders(&a, "Clamp").len(), 0);
    }
    #[test]
    fn add_manual_setup_shows_with_a_manual_machine_and_inserts_the_setup() {
        let mut a = app();
        lacks(&a, &["+ Add manual setup"]);
        with_manual(&mut a);
        shows(&a, &["+ Add manual setup"]);
        let clicks = crate::ui::probe::clicks(a.view(), WINDOW);
        assert!(clicks.iter().any(|m| matches!(m, Message::AddManualSetup)));
        let old = a.doc.active.clone();
        let _ = a.update(Message::AddManualSetup);
        assert_eq!(a.doc.setups.len(), 2);
        assert_eq!(a.doc.setups[0].name(), "Big mill");
        assert_eq!(a.doc.setups[1].id, old);
        assert_eq!(a.doc.active, old);
        assert_eq!(a.page, Page::Work);
        assert_eq!(a.selected, None);
        // Without a manual machine the press says so and changes nothing.
        let mut b = app();
        let _ = b.update(Message::AddManualSetup);
        assert_eq!(b.doc.setups.len(), 1);
        assert!(b.status.contains("manual machine"), "{}", b.status);
    }
    #[test]
    fn the_ribbon_dims_all_but_box_and_drill_on_a_manual_setup() {
        let mut a = app();
        with_manual(&mut a);
        let sent = |a: &App| crate::ui::probe::clicks(a.view(), WINDOW);
        let adds = |a: &App, s: Strategy| {
            sent(a)
                .iter()
                .any(|m| matches!(m, Message::AddOp(x) if *x == s))
        };
        let reasons = |a: &App, reason: &str| {
            let (_, cards) = laid(a).run(Helps::default()).expect("the view");
            cards.iter().filter(|c| c.body == reason).count()
        };
        let (one_axis, box_on_cnc) = (crate::manual::ONE_AXIS, crate::manual::BOX_ON_CNC);
        assert!(adds(&a, Strategy::Face) && adds(&a, Strategy::Drill));
        assert!(!adds(&a, Strategy::Box));
        assert_eq!(reasons(&a, one_axis), 0);
        assert_eq!(reasons(&a, box_on_cnc), 1);
        let _ = a.update(Message::AddManualSetup);
        a.doc.active = a.doc.setups[0].id.clone();
        assert!(!adds(&a, Strategy::Face));
        assert!(!adds(&a, Strategy::FiveAxis));
        assert!(adds(&a, Strategy::Drill) && adds(&a, Strategy::Box));
        // The ten dimmed tiles each say why; Box and Drill are not dimmed.
        assert_eq!(reasons(&a, one_axis), Strategy::ALL.len() - 2);
        assert_eq!(reasons(&a, box_on_cnc), 0);
        // Back on the CNC setup only Box is dimmed.
        a.doc.active = a.doc.setups[1].id.clone();
        assert!(adds(&a, Strategy::Face) && !adds(&a, Strategy::Box));
        assert_eq!(reasons(&a, one_axis), 0);
        assert_eq!(reasons(&a, box_on_cnc), 1);
    }
    #[test]
    fn adding_a_box_on_the_big_mill_takes_a_fitting_tool_and_a_gear() {
        let mut a = app();
        let tool = with_big_cutter(&mut a);
        let _ = a.update(Message::AddManualSetup);
        a.doc.active = a.doc.setups[0].id.clone();
        let _ = a.update(Message::AddOp(Strategy::Box));
        let op = a.operation().unwrap().clone();
        assert_eq!(op.strategy, Strategy::Box);
        assert_eq!(op.tool, tool.id);
        assert_eq!((get(&op.p, "rpm"), get(&op.p, "feed")), ("640", "120"));
        assert_eq!(a.op_issues(), Vec::<String>::new());
        // Drill's own defaults are above the machine: they come down to it.
        let _ = a.update(Message::AddOp(Strategy::Drill));
        let drill = a.operation().unwrap();
        assert_eq!(
            (get(&drill.p, "rpm"), get(&drill.p, "feed")),
            ("1600", "150")
        );
        // On the CNC setup nothing changes.
        a.doc.active = a.doc.setups[1].id.clone();
        let _ = a.update(Message::AddOp(Strategy::Drill));
        let drill = a.operation().unwrap();
        assert_eq!(
            (get(&drill.p, "rpm"), get(&drill.p, "feed")),
            ("2400", "280")
        );
    }
    #[test]
    fn the_box_editor_has_the_six_fields_in_x_y_z_rows_and_the_passes() {
        let mut a = app();
        with_big_cutter(&mut a);
        let _ = a.update(Message::AddManualSetup);
        a.doc.active = a.doc.setups[0].id.clone();
        let _ = a.update(Message::AddOp(Strategy::Box));
        let tabs = Strategy::Box.tabs();
        for tab in &tabs {
            let _ = a.update(Message::Tab(tab.to_string()));
            let _view = a.view();
        }
        let _ = a.update(Message::Tab("Box".into()));
        let texts = page_texts(&a);
        let at = |label: &str| {
            (texts.iter().position(|t| t == label)).unwrap_or_else(|| panic!("{label}: {texts:?}"))
        };
        let order = [
            "X min · mm",
            "X max · mm",
            "Y min · mm",
            "Y max · mm",
            "Z top · mm",
            "Z bottom · mm",
        ]
        .map(at);
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
        let _ = a.update(Message::Tab("Passes".into()));
        shows(
            &a,
            &[
                "Stock to leave · mm",
                "Stepover · % tool diameter",
                "Maximum stepdown · mm",
            ],
        );
        lacks(&a, &["Pass direction · degrees", "Tolerance · mm"]);
        let _ = a.update(Message::Edit(Target::Operation, "x_max", "-5".into()));
        let _ = a.update(Message::Tab("Box".into()));
        let checks = crate::ui::op_checks(&a);
        assert_eq!(
            checks.len(),
            1,
            "{:?}",
            checks.iter().map(|c| &c.text).collect::<Vec<_>>()
        );
        assert_eq!(checks[0].key, Some("x_max"));
    }
    #[test]
    fn a_box_message_opens_the_tab_with_its_field() {
        for (issue, tab) in [
            ("x min must be a finite number.", Some("Box")),
            ("x max must be greater than x min.", Some("Box")),
            ("y max must be greater than y min.", Some("Box")),
            ("z top must be greater than z bottom.", Some("Box")),
            ("z bottom must be a finite number.", Some("Box")),
            (
                "The box must be at least as wide as the tool across its passes.",
                Some("Box"),
            ),
            ("stepover must be between 10 and 100.", Some("Passes")),
            ("stepdown must be greater than zero.", Some("Passes")),
            ("allowance cannot be negative or blank.", Some("Passes")),
            ("End mill 10 is not on Manual Mill.", Some("Tool")),
            (
                "Spindle speed must be one of the machine's gears: 90, 150.",
                Some("Tool"),
            ),
            (
                "The big mill cuts one axis at a time: use Box or Drill.",
                None,
            ),
            ("Box runs on a manual mill.", None),
        ] {
            assert_eq!(crate::ui::issue_tab(issue), tab, "{issue}");
        }
    }
    #[test]
    fn the_box_tab_fills_its_fields_from_the_picked_face() {
        let mut a = imported_app();
        with_big_cutter(&mut a);
        let _ = a.update(Message::AddManualSetup);
        a.doc.active = a.doc.setups[0].id.clone();
        let _ = a.update(Message::Edit(Target::Setup, "stock_top", "10".into()));
        let _ = a.update(Message::AddOp(Strategy::Box));
        let _ = a.update(Message::Tab("Box".into()));
        shows(
            &a,
            &["From the part", "From selected face", "0 faces selected"],
        );
        assert!(
            crate::ui::probe::clicks(a.view(), iced::Size::new(WINDOW.width, 2400.))
                .iter()
                .any(|m| matches!(m, Message::BoxFromFace))
        );
        let fields = |a: &App| -> Vec<String> {
            crate::model::BOX_FIELDS
                .iter()
                .map(|k| get(&a.operation().unwrap().p, k).to_string())
                .collect()
        };
        let before = fields(&a);
        // No face yet: the message, and the fields stay.
        let _ = a.update(Message::BoxFromFace);
        assert_eq!(a.status, crate::manual::ONE_FACE);
        assert_eq!(fields(&a), before);
        // Picking stays on the Box tab, and a click replaces the face before it.
        let _ = a.update(Message::PickMode);
        assert!(a.picking && a.tab == "Box");
        let mesh = a.imported_mesh.as_ref().unwrap();
        let top = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| s.normal[2] > 0.99999)
            .max_by(|(_, a), (_, b)| a.center[2].total_cmp(&b.center[2]))
            .unwrap()
            .0;
        let chamfer = mesh
            .surfaces
            .iter()
            .position(|s| s.normal[2] > 0.5 && s.normal[2] < 0.9)
            .unwrap();
        let picked = |a: &App| {
            a.operation()
                .unwrap()
                .surfaces
                .as_ref()
                .unwrap()
                .ids
                .clone()
        };
        let _ = a.update(Message::Surface(chamfer));
        let _ = a.update(Message::Surface(top));
        assert_eq!(picked(&a), vec![top]);
        shows(&a, &["1 face selected"]);
        // The block's top, under 10 mm of stock: the whole stock above it.
        let _ = a.update(Message::BoxFromFace);
        let stock = a.known_stock(0).0.unwrap();
        let zero = crate::post::datum(a.doc.setup(), stock, a.part_mesh().unwrap()).unwrap();
        let whole: Vec<String> = [
            stock.min[0] - zero[0],
            stock.max[0] - zero[0],
            stock.min[1] - zero[1],
            stock.max[1] - zero[1],
            0.,
            -10.,
        ]
        .map(|v| format!("{}", (v * 1000.).round() / 1000.))
        .to_vec();
        assert_eq!(fields(&a), whole);
        assert_eq!(a.status, "Box filled from the selected face.");
        assert_eq!(a.op_issues(), Vec::<String>::new());
        // A slanted face is refused, and the fields stay.
        let _ = a.update(Message::Surface(chamfer));
        let _ = a.update(Message::BoxFromFace);
        assert_eq!(a.status, crate::manual::ONE_FACE);
        assert_eq!(fields(&a), whole);
    }
}
