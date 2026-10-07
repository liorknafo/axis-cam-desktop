//! Pictures of the part for the AI planner: the overview its first request
//! carries and the views the model asks for with the `view_model` tool.
use crate::{
    analysis::{self, PartReport},
    mesh::{Bounds, Mesh},
    model::{Document, Setup},
    stock::Placement,
    viewport::{Camera, Projector, Viewport},
};
use serde::Deserialize;
use std::sync::Arc;

/// The tool's name in requests.
pub const NAME: &str = "view_model";
/// The pictures' width and height in pixels.
pub const SIZE: u32 = 768;
/// The strip along the bottom: the caption and the scale bar.
const CAPTION: u32 = 44;
/// The most surfaces one list may name.
const MAX_IDS: usize = 200;
/// Each angle: name, words, yaw and pitch of `viewport::Camera`. The
/// projector puts the viewer at (sin yaw · cos pitch, cos yaw · cos pitch,
/// sin pitch); its frame is a mirror image, so pictures are flipped.
const ANGLES: [(&str, &str, f32, f32); 9] = [
    (
        "top",
        "top",
        std::f32::consts::PI,
        std::f32::consts::FRAC_PI_2,
    ),
    ("front", "front (from -Y)", std::f32::consts::PI, 0.),
    ("back", "back (from +Y)", 0., 0.),
    ("left", "left (from -X)", -std::f32::consts::FRAC_PI_2, 0.),
    ("right", "right (from +X)", std::f32::consts::FRAC_PI_2, 0.),
    (
        "iso_front_left",
        "isometric front-left",
        -(std::f32::consts::PI - 0.75),
        0.62,
    ),
    (
        "iso_front_right",
        "isometric front-right",
        std::f32::consts::PI - 0.75,
        0.62,
    ),
    ("iso_back_left", "isometric back-left", -0.75, 0.62),
    ("iso_back_right", "isometric back-right", 0.75, 0.62),
];
/// What the pictures are drawn from.
#[derive(Clone)]
pub struct Scene {
    pub doc: Arc<Document>,
    pub mesh: Arc<Mesh>,
    pub report: Arc<PartReport>,
    /// The latest proposal's built job, for `operation` views.
    pub job: Option<Arc<Document>>,
}
/// One picture for the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub png: Arc<Vec<u8>>,
    /// What the picture shows, for the model and the timeline.
    pub description: String,
    /// The model's reason, cleaned, at most 120 characters.
    pub purpose: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpRef {
    setup: usize,
    operation: usize,
}
/// The `view_model` tool's arguments.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Args {
    orientation: Option<String>,
    angle: String,
    focus_surfaces: Vec<usize>,
    highlight_surfaces: Vec<usize>,
    show_stock: bool,
    show_workholding: bool,
    labels: bool,
    operation: Option<OpRef>,
    purpose: String,
}
impl Args {
    /// The arguments the model sent, or why they cannot be used.
    pub(crate) fn parse(arguments: &str) -> Result<Self, String> {
        let args: Self = serde_json::from_str(arguments).map_err(|e| {
            crate::proposal::clip(
                &format!("The view_model arguments could not be read: {e}"),
                300,
            )
        })?;
        let known_up = |u: &str| analysis::ORIENTATIONS.iter().any(|(up, _)| *up == u);
        if args.orientation.as_deref().is_some_and(|u| !known_up(u))
            || !ANGLES.iter().any(|(a, ..)| *a == args.angle)
        {
            return Err(
                "The view_model arguments could not be read: unknown orientation or angle.".into(),
            );
        }
        if args.focus_surfaces.len() > MAX_IDS || args.highlight_surfaces.len() > MAX_IDS {
            return Err(format!(
                "List at most {MAX_IDS} surfaces to focus on or highlight."
            ));
        }
        Ok(args)
    }
}
/// What one picture draws: the setup frame, the projector and the layers.
pub(crate) struct View {
    setup: Setup,
    /// The part axis that points up; None for a setup not square to the part.
    up: Option<&'static str>,
    projector: Projector,
    sim: Option<crate::simulation::Simulation>,
    holding: Vec<crate::holding::HoldBox>,
    angle_words: &'static str,
    focus: Vec<usize>,
    /// The operation looked at, counted from 1: setup and operation.
    operation: Option<(usize, usize)>,
}
impl View {
    /// The view `args` ask for, or one sentence on why it cannot be drawn.
    pub(crate) fn new(scene: &Scene, args: &Args) -> Result<Self, String> {
        let mesh = &*scene.mesh;
        let surfaces = mesh.surfaces.len();
        if let Some(bad) = args
            .focus_surfaces
            .iter()
            .chain(&args.highlight_surfaces)
            .find(|id| **id >= surfaces)
        {
            return Err(format!("Surface {bad} is not a surface of this part."));
        }
        let (_, angle_words, yaw, pitch) = *ANGLES
            .iter()
            .find(|(a, ..)| *a == args.angle)
            .expect("Args::parse knows the angle");
        let camera = Camera {
            yaw,
            pitch,
            zoom: 1.,
            pan: [0., 0.],
        };
        let (setup, up, sim, operation) = match &args.operation {
            Some(OpRef { setup, operation }) => {
                let job = scene.job.as_ref().ok_or(
                    "No proposal has been built yet, so there is no operation to look at.",
                )?;
                let s = job
                    .setups
                    .get(setup.wrapping_sub(1))
                    .ok_or_else(|| format!("The latest proposal has no setup {setup}."))?;
                if operation.wrapping_sub(1) >= s.operations.len() {
                    return Err(format!(
                        "Setup {setup} has no operation {operation} in the latest proposal."
                    ));
                }
                let sim = crate::planner::view_after(
                    job,
                    mesh,
                    setup - 1,
                    operation - 1,
                    crate::stock_surface::Detail::Standard,
                )
                .ok_or_else(|| {
                    format!("Setup {setup}, operation {operation} could not be simulated.")
                })?;
                (
                    s.clone(),
                    analysis::up_axis(s),
                    Some(sim),
                    Some((*setup, *operation)),
                )
            }
            None => {
                let first = scene
                    .doc
                    .setups
                    .first()
                    .ok_or("The project has no setup 1.")?;
                let up = match &args.orientation {
                    Some(u) => {
                        analysis::ORIENTATIONS
                            .iter()
                            .find(|(name, _)| *name == u.as_str())
                            .expect("Args::parse knows the orientation")
                            .0
                    }
                    None => analysis::up_axis(first).ok_or("Setup 1 is not square to the part.")?,
                };
                let mut setup =
                    analysis::oriented(first, up).ok_or("That orientation cannot be drawn.")?;
                // The stock as the report gives it in this orientation.
                if let Some(o) = scene.report.orientations.iter().find(|o| o.up == up) {
                    let scale = mesh.display_scale();
                    let model = Placement::new(mesh, Some(&setup)).bounds.scaled(1. / scale);
                    let m = o.stock_margin;
                    setup.preview_stock = Some(Bounds {
                        min: [
                            model.min[0] - m[2],
                            model.min[1] - m[4],
                            model.min[2] - m[1],
                        ],
                        max: [
                            model.max[0] + m[3],
                            model.max[1] + m[5],
                            model.max[2] + m[0],
                        ],
                    });
                    setup.p.insert("stock_source".into(), "fresh".into());
                }
                (setup, Some(up), None, None)
            }
        };
        let holding = if args.show_workholding {
            let doc = scene
                .job
                .as_deref()
                .filter(|_| operation.is_some())
                .unwrap_or(&scene.doc);
            let first_up = scene.doc.setups.first().and_then(analysis::up_axis);
            let stock = sim.as_ref().map(|s| s.plan.stock).or_else(|| {
                crate::stock::resolve(&setup, Some(mesh))
                    .ok()
                    .map(|r| r.stock)
            });
            match stock {
                // Only setup 1's orientation, or an operation's own setup,
                // has its vise or clamps placed.
                Some(stock) if operation.is_some() || up == first_up => doc
                    .holding_boxes(&setup, stock)
                    .map(|b| crate::holding::shown(&b, stock))
                    .unwrap_or_default(),
                _ => vec![],
            }
        } else {
            vec![]
        };
        let viewport = Viewport {
            setup: Some(&setup),
            operation: None,
            mesh,
            simulation: sim.as_ref(),
            sample: false,
            camera,
            picking: false,
            show_stock: args.show_stock,
            show_paths: false,
            progress: None,
            holding: holding.clone(),
        };
        let bounds = focus_bounds(mesh, &setup, &args.focus_surfaces)
            .unwrap_or_else(|| viewport.scene_bounds());
        let projector = Projector::fit(camera, (SIZE as f32, (SIZE - CAPTION) as f32), bounds);
        Ok(Self {
            setup,
            up,
            projector,
            sim,
            holding,
            angle_words,
            focus: args.focus_surfaces.clone(),
            operation,
        })
    }
    /// Where a point (display units, setup frame) lands in the picture:
    /// x, y in pixels after the flip, and its depth.
    pub(crate) fn screen(&self, v: [f32; 3]) -> (f32, f32, f32) {
        let q = self.projector.p(v);
        (SIZE as f32 - q.x, q.y, self.projector.depth(v))
    }
}
/// The display-unit bounds of `ids`' faces in `setup`'s frame, 15 % larger;
/// None without any such face.
fn focus_bounds(mesh: &Mesh, setup: &Setup, ids: &[usize]) -> Option<Bounds> {
    let placement = Placement::new(mesh, Some(setup));
    let mut b = Bounds {
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
    };
    for face in mesh
        .faces
        .iter()
        .filter(|f| f.feature.is_some_and(|id| ids.contains(&id)))
    {
        for p in &face.points {
            let q = placement.transform(*p);
            b.min = std::array::from_fn(|k| b.min[k].min(q[k]));
            b.max = std::array::from_fn(|k| b.max[k].max(q[k]));
        }
    }
    if !b.min.iter().chain(&b.max).all(|v| v.is_finite()) {
        return None;
    }
    let pad: [f32; 3] = std::array::from_fn(|k| (b.max[k] - b.min[k]).max(1.) * 0.15);
    Some(Bounds {
        min: std::array::from_fn(|k| b.min[k] - pad[k]),
        max: std::array::from_fn(|k| b.max[k] + pad[k]),
    })
}
/// The picture `args` ask for: the scene, the tags and the caption.
fn render(scene: &Scene, args: &Args) -> Result<Picture, String> {
    let view = View::new(scene, args)?;
    let mesh = &*scene.mesh;
    let job = scene.job.as_deref();
    let operation = view
        .operation
        .zip(job)
        .map(|((s, o), job)| &job.setups[s - 1].operations[o - 1]);
    let viewport = Viewport {
        setup: Some(&view.setup),
        operation,
        mesh,
        simulation: view.sim.as_ref(),
        sample: false,
        // Unused: the picture is drawn with the view's projector.
        camera: Camera::default(),
        picking: false,
        show_stock: args.show_stock,
        show_paths: view.operation.is_some(),
        progress: None,
        holding: view.holding.clone(),
    };
    let (w, h) = (SIZE, SIZE - CAPTION);
    let (rgba, depths) =
        viewport.render(&view.projector, (w, h), 1., None, &args.highlight_surfaces);
    let mut canvas = Canvas::over(&rgba, w, h, true);
    // Tags go on before the caption strip is added, so none reaches into it.
    let tags = if args.labels {
        draw_tags(&mut canvas, scene, &view, &depths)
    } else {
        vec![]
    };
    canvas.rgb.resize((SIZE * SIZE * 3) as usize, 255);
    canvas.h = SIZE;
    caption(&mut canvas, &view, mesh.display_scale());
    Ok(Picture {
        png: Arc::new(canvas.png()),
        description: describe(&view, args, &tags),
        purpose: crate::proposal::clip(&args.purpose, 120),
    })
}
/// The picture the `view_model` arguments ask for, or one sentence on why
/// it cannot be drawn.
pub fn draw(scene: &Scene, arguments: &str) -> Result<Picture, String> {
    render(scene, &Args::parse(arguments)?)
}
/// The picture the first request carries: setup 1's orientation from the
/// front left, the whole scene, stock, workholding and tags shown.
pub fn overview(scene: &Scene) -> Result<Picture, String> {
    let args = Args {
        orientation: None,
        angle: "iso_front_left".into(),
        focus_surfaces: vec![],
        highlight_surfaces: vec![],
        show_stock: true,
        show_workholding: true,
        labels: true,
        operation: None,
        purpose: "Overview".into(),
    };
    render(scene, &args)
}
/// The report's tags for the drawn orientation where their points can be
/// seen: each plane's surface IDs, largest plane first, then the diameters
/// of its holes. A tag that would overlap one already drawn is skipped.
/// Returns the texts drawn, in order.
fn draw_tags(canvas: &mut Canvas, scene: &Scene, view: &View, depths: &[f32]) -> Vec<String> {
    let Some(o) = scene
        .report
        .orientations
        .iter()
        .find(|o| Some(o.up) == view.up)
    else {
        return vec![];
    };
    let mesh = &*scene.mesh;
    let scale = mesh.display_scale();
    let model = Placement::new(mesh, Some(&view.setup)).bounds;
    // Report positions are mm from the model's -X/-Y corner and its top.
    let at = |x: f32, y: f32, depth: f32| {
        [
            model.min[0] + x * scale,
            model.min[1] + y * scale,
            model.max[2] - depth * scale,
        ]
    };
    let planes = o.planes.iter().map(|plane| {
        let ids: Vec<String> = plane
            .surfaces
            .iter()
            .take(3)
            .map(usize::to_string)
            .collect();
        let more = if plane.surfaces.len() > 3 { "…" } else { "" };
        let [x0, y0, x1, y1] = plane.bbox;
        (
            at((x0 + x1) / 2., (y0 + y1) / 2., plane.depth_below_top),
            format!("{}{more}", ids.join(",")),
        )
    });
    let holes = o.planes.iter().flat_map(|plane| {
        plane.holes.iter().map(|hole| {
            (
                at(hole.center[0], hole.center[1], plane.depth_below_top),
                format!("Ø{:.1}", hole.diameter),
            )
        })
    });
    let (w, h) = (canvas.w as usize, canvas.h as usize);
    let (mut drawn, mut texts): (Vec<[i32; 4]>, Vec<String>) = (vec![], vec![]);
    for (point, text) in planes.chain(holes) {
        let (x, y, z) = view.screen(point);
        if !(x >= 0. && y >= 0. && x < w as f32 && y < h as f32) {
            continue;
        }
        // The depths are those of the frame before it was flipped.
        let nearest = depths[y as usize * w + (w - 1 - x as usize)];
        if z + 0.5 < nearest {
            continue;
        }
        let rect = Canvas::tag_rect((x, y), &text);
        let overlaps =
            |r: &[i32; 4]| r[0] < rect[2] && rect[0] < r[2] && r[1] < rect[3] && rect[1] < r[3];
        if drawn.iter().any(overlaps) {
            continue;
        }
        if let Some(rect) = canvas.tag((x, y), &text) {
            drawn.push(rect);
            texts.push(text);
        }
    }
    texts
}
/// The words for the view's up axis.
fn up_words(view: &View) -> &'static str {
    view.up.unwrap_or("rotated setup")
}
/// The strip along the bottom (the view in words and a scale bar) and the
/// axis triad in the top-left corner.
fn caption(canvas: &mut Canvas, view: &View, scale: f32) {
    let top = (SIZE - CAPTION) as i32;
    let size = SIZE as i32;
    canvas.fill([0, top, size, size], [255, 255, 255]);
    canvas.fill([0, top, size, top + 1], [200, 205, 212]);
    let mut words = format!("{} UP · {}", up_words(view), view.angle_words);
    if let Some((s, o)) = view.operation {
        words += &format!(" · AFTER S{s} OP{o}");
    }
    // Words too long to clear the scale bar are written smaller.
    let room = SIZE - 12 - 160 - 20;
    if Canvas::text_width(&words, 2) <= room {
        canvas.text(10, top + 8, &words, 2, INK);
    } else {
        canvas.text(10, top + 12, &words, 1, INK);
    }
    // The longest round length that fits in 160 px.
    let px_per_mm = scale * view.projector.scale;
    if let Some(mm) = [1, 2, 5, 10, 20, 50, 100, 200, 500]
        .into_iter()
        .rev()
        .find(|mm| *mm as f32 * px_per_mm <= 160.)
    {
        let (right, y) = ((size - 12) as f32, (size - 12) as f32);
        canvas.line((right - mm as f32 * px_per_mm, y), (right, y), INK, 3.);
        let label = format!("{mm} MM");
        let x = size - 12 - Canvas::text_width(&label, 2) as i32;
        canvas.text(x, top + 8, &label, 2, INK);
    }
    let origin = view.screen([0.; 3]);
    for (axis, letter, color) in [
        (0, "X", [205, 81, 85]),
        (1, "Y", [76, 154, 105]),
        (2, "Z", [67, 125, 196]),
    ] {
        let mut unit = [0.; 3];
        unit[axis] = 1.;
        let q = view.screen(unit);
        // The axis as drawn, 1 when it lies in the picture's plane.
        let d = (
            (q.0 - origin.0) / view.projector.scale,
            (q.1 - origin.1) / view.projector.scale,
        );
        let length = d.0.hypot(d.1);
        // An axis pointing at the viewer or away has no direction to show.
        if length < 0.1 {
            continue;
        }
        canvas.line((40., 40.), (40. + 28. * d.0, 40. + 28. * d.1), color, 2.);
        let (lx, ly) = (
            40. + 28. * d.0 + 9. * d.0 / length,
            40. + 28. * d.1 + 9. * d.1 / length,
        );
        canvas.text(
            (lx.round() as i32 - 5).max(1),
            (ly.round() as i32 - 7).max(1),
            letter,
            2,
            color,
        );
    }
}
/// The picture in one line for the model and the timeline.
fn describe(view: &View, args: &Args, tags: &[String]) -> String {
    let scope = if view.focus.is_empty() {
        "whole part".to_string()
    } else {
        let ids: Vec<String> = view.focus.iter().map(usize::to_string).collect();
        format!("zoomed on {}", ids.join(", "))
    };
    let after = view
        .operation
        .map(|(s, o)| format!(", after setup {s}, operation {o}"))
        .unwrap_or_default();
    let stock = if args.show_stock {
        "stock shown"
    } else {
        "stock hidden"
    };
    let holding = if view.holding.is_empty() {
        ""
    } else {
        "; vise or clamps shown"
    };
    let tags = if tags.is_empty() {
        "none".to_string()
    } else {
        tags.join(", ")
    };
    format!(
        "{} up, {}, {scope}{after}; {stock}{holding}; tags: {tags}",
        up_words(view),
        view.angle_words
    )
}

/// The viewport's background, under everything a picture draws.
const BACKGROUND: [u8; 3] = [239, 243, 247];
const INK: [u8; 3] = [40, 46, 54];
/// A 5 × 7 glyph per character, one byte a row, bit 4 the leftmost pixel.
fn glyph(c: char) -> Option<[u8; 7]> {
    Some(match c {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        ' ' => [0; 7],
        '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x0C, 0x04, 0x08],
        ':' => [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00],
        '/' => [0x00, 0x01, 0x02, 0x04, 0x08, 0x10, 0x00],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        'Ø' => [0x0D, 0x12, 0x15, 0x15, 0x15, 0x09, 0x16],
        '×' => [0x00, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x00],
        '%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
        '…' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x15, 0x15],
        '·' => [0x00, 0x00, 0x00, 0x0C, 0x0C, 0x00, 0x00],
        '?' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
        _ => return None,
    })
}
/// An opaque RGB picture being drawn.
pub(crate) struct Canvas {
    pub w: u32,
    pub h: u32,
    pub rgb: Vec<u8>,
}
impl Canvas {
    #[cfg(test)]
    fn blank(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            rgb: BACKGROUND.repeat((w * h) as usize),
        }
    }
    #[cfg(test)]
    fn at(&self, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * self.w + x) * 3) as usize;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }
    /// `rgba` laid over the background; mirrored left to right when `flip`.
    pub(crate) fn over(rgba: &[u8], w: u32, h: u32, flip: bool) -> Self {
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                let sx = if flip { w - 1 - x } else { x };
                let i = ((y * w + sx) * 4) as usize;
                let a = rgba[i + 3] as f32 / 255.;
                for k in 0..3 {
                    rgb.push(
                        (rgba[i + k] as f32 * a + BACKGROUND[k] as f32 * (1. - a)).round() as u8,
                    );
                }
            }
        }
        Self { w, h, rgb }
    }
    fn put(&mut self, x: i32, y: i32, color: [u8; 3]) {
        if x >= 0 && y >= 0 && (x as u32) < self.w && (y as u32) < self.h {
            let i = ((y as u32 * self.w + x as u32) * 3) as usize;
            self.rgb[i..i + 3].copy_from_slice(&color);
        }
    }
    pub(crate) fn fill(&mut self, [x0, y0, x1, y1]: [i32; 4], color: [u8; 3]) {
        for y in y0..y1 {
            for x in x0..x1 {
                self.put(x, y, color);
            }
        }
    }
    pub(crate) fn line(&mut self, a: (f32, f32), b: (f32, f32), color: [u8; 3], width: f32) {
        let steps = ((b.0 - a.0).hypot(b.1 - a.1).ceil() as usize).clamp(1, 4000);
        // Floored so a hairline's disc still covers a pixel centre on each step.
        let r = (width / 2.).max(0.75);
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            for yy in (y - r).floor() as i32..=(y + r).ceil() as i32 {
                for xx in (x - r).floor() as i32..=(x + r).ceil() as i32 {
                    if (xx as f32 + 0.5 - x).powi(2) + (yy as f32 + 0.5 - y).powi(2) <= r * r {
                        self.put(xx, yy, color);
                    }
                }
            }
        }
    }
    pub(crate) fn text_width(text: &str, scale: u32) -> u32 {
        (text.chars().count() as u32 * 6).saturating_sub(1) * scale
    }
    /// `text` in capitals from its top-left corner; unknown characters as "?".
    pub(crate) fn text(&mut self, x: i32, y: i32, text: &str, scale: u32, color: [u8; 3]) {
        let s = scale as i32;
        for (n, c) in text.chars().flat_map(char::to_uppercase).enumerate() {
            let rows = glyph(c).unwrap_or_else(|| glyph('?').unwrap());
            for (row, bits) in rows.iter().enumerate() {
                for col in 0..5 {
                    if bits & (0x10 >> col) != 0 {
                        let (px, py) = (x + (n as i32 * 6 + col) * s, y + row as i32 * s);
                        self.fill([px, py, px + s, py + s], color);
                    }
                }
            }
        }
    }
    /// Where `tag` puts the box for `text` centred on `center`.
    fn tag_rect(center: (f32, f32), text: &str) -> [i32; 4] {
        let (w, h) = (Self::text_width(text, 2) as i32 + 8, 14 + 8);
        let x0 = center.0.round() as i32 - w / 2;
        let y0 = center.1.round() as i32 - h / 2;
        [x0, y0, x0 + w, y0 + h]
    }
    /// A label box centred on `center`; None when it would leave the picture.
    pub(crate) fn tag(&mut self, center: (f32, f32), text: &str) -> Option<[i32; 4]> {
        let rect = Self::tag_rect(center, text);
        let [x0, y0, ..] = rect;
        if x0 < 0 || y0 < 0 || rect[2] > self.w as i32 || rect[3] > self.h as i32 {
            return None;
        }
        self.fill(rect, INK);
        self.fill([x0 + 1, y0 + 1, rect[2] - 1, rect[3] - 1], [255, 255, 255]);
        self.text(x0 + 4, y0 + 4, text, 2, INK);
        Some(rect)
    }
    pub(crate) fn png(&self) -> Vec<u8> {
        let mut out = vec![];
        let mut encoder = png::Encoder::new(&mut out, self.w, self.h);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("PNG header");
        writer.write_image_data(&self.rgb).expect("PNG data");
        drop(writer);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_character_a_tag_or_caption_uses_has_a_glyph() {
        for c in "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ +-.,:/()Ø×%…·".chars() {
            assert!(glyph(c).is_some(), "{c}");
        }
        assert!(glyph('ä').is_none());
        // Lower case is drawn as capitals; anything else as "?".
        let mut a = Canvas::blank(40, 20);
        let mut b = Canvas::blank(40, 20);
        a.text(0, 0, "ab", 1, [0, 0, 0]);
        b.text(0, 0, "AB", 1, [0, 0, 0]);
        assert_eq!(a.rgb, b.rgb);
    }
    #[test]
    fn a_tag_is_drawn_centred_and_refused_at_the_edge() {
        let mut c = Canvas::blank(200, 100);
        let rect = c.tag((100., 50.), "12").unwrap();
        assert!(rect[0] < 100 && rect[2] > 100 && rect[1] < 50 && rect[3] > 50);
        // Ink strictly inside the 1 px border can only be the text.
        let inked = (rect[0] + 2..rect[2] - 2)
            .any(|x| (rect[1] + 2..rect[3] - 2).any(|y| c.at(x as u32, y as u32) == INK));
        assert!(inked, "the text is drawn inside the box");
        assert!(c.tag((2., 50.), "12").is_none());
    }
    #[test]
    fn text_draws_the_glyph_pixels() {
        let mut c = Canvas::blank(40, 20);
        c.text(0, 0, "1", 1, INK);
        // '1' is [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E], bit 4 leftmost.
        assert_eq!(c.at(2, 0), INK);
        assert_eq!(c.at(0, 0), BACKGROUND);
        for x in 1..=3 {
            assert_eq!(c.at(x, 6), INK);
        }
        let mut t = Canvas::blank(40, 20);
        t.text(0, 0, "AB", 1, INK);
        assert_ne!(t.rgb, Canvas::blank(40, 20).rgb);
        let mut unknown = Canvas::blank(40, 20);
        unknown.text(0, 0, "ä", 1, INK);
        let mut question = Canvas::blank(40, 20);
        question.text(0, 0, "?", 1, INK);
        assert_ne!(question.rgb, Canvas::blank(40, 20).rgb);
        assert_eq!(unknown.rgb, question.rgb);
    }
    #[test]
    fn a_hairline_is_drawn_without_gaps() {
        let mut h = Canvas::blank(80, 70);
        h.line((10., 10.), (60., 10.), INK, 1.);
        let row = (0..80).filter(|&x| h.at(x, 10) == INK).count();
        assert!(row >= 45, "{row} pixels in row 10");
        let mut d = Canvas::blank(80, 70);
        d.line((10., 10.), (60., 40.), INK, 1.);
        for x in 10..=60 {
            assert!((0..70).any(|y| d.at(x, y) == INK), "column {x} has a gap");
        }
    }
    #[test]
    fn composite_flips_and_fills_the_background() {
        // A 2 × 1 frame: left pixel red and opaque, right pixel empty.
        let rgba = [255, 0, 0, 255, 0, 0, 0, 0];
        let c = Canvas::over(&rgba, 2, 1, true);
        assert_eq!(c.at(0, 0), [239, 243, 247]);
        assert_eq!(c.at(1, 0), [255, 0, 0]);
        let png = c.png();
        assert_eq!(&png[1..4], b"PNG");
    }
    use crate::proposal::tests::{block, good};
    use std::sync::Arc;
    fn scene() -> Scene {
        let (doc, mesh, report) = block();
        Scene {
            doc: Arc::new(doc),
            mesh: Arc::new(mesh),
            report: Arc::new(report),
            job: None,
        }
    }
    fn args(json: serde_json::Value) -> String {
        let mut all = serde_json::json!({
            "orientation": null, "angle": "iso_front_left", "focus_surfaces": [],
            "highlight_surfaces": [], "show_stock": true, "show_workholding": true,
            "labels": true, "operation": null, "purpose": "look"
        });
        for (k, v) in json.as_object().unwrap() {
            all[k] = v.clone();
        }
        all.to_string()
    }
    fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).unwrap();
        (info.width, info.height, buf)
    }
    #[test]
    fn the_overview_is_a_labelled_768_pixel_png_of_setup_1() {
        let s = scene();
        let picture = overview(&s).unwrap();
        let (w, h, _) = decode(&picture.png);
        assert_eq!((w, h), (768, 768));
        assert!(
            picture
                .description
                .starts_with("+Z up, isometric front-left, whole part"),
            "{}",
            picture.description
        );
        assert!(
            picture.description.contains("stock shown"),
            "{}",
            picture.description
        );
        let top = &s.report.orientations[0].planes[0];
        assert!(
            picture.description.contains(&top.surfaces[0].to_string()),
            "{}",
            picture.description
        );
    }
    #[test]
    fn a_top_view_keeps_x_right_and_y_up() {
        let s = scene();
        let view = View::new(
            &s,
            &Args::parse(&args(serde_json::json!({"angle": "top"}))).unwrap(),
        )
        .unwrap();
        let (a, b, c) = (
            view.screen([0., 0., 0.]),
            view.screen([10., 0., 0.]),
            view.screen([0., 10., 0.]),
        );
        assert!(b.0 > a.0 + 1., "+X is to the right: {a:?} {b:?}");
        assert!(c.1 < a.1 - 1., "+Y is up: {a:?} {c:?}");
        let front = View::new(
            &s,
            &Args::parse(&args(serde_json::json!({"angle": "front"}))).unwrap(),
        )
        .unwrap();
        let (a, b) = (front.screen([0., 0., 0.]), front.screen([10., 0., 0.]));
        assert!(b.0 > a.0 + 1., "seen from -Y, +X is to the right");
    }
    #[test]
    fn highlighting_and_focus_change_the_picture() {
        let s = scene();
        let pocket = s.report.orientations[0]
            .planes
            .iter()
            .find(|p| p.kind == "floor")
            .unwrap()
            .surfaces[0];
        let plain = draw(
            &s,
            &args(serde_json::json!({"angle": "top", "labels": false})),
        )
        .unwrap();
        let marked = draw(
            &s,
            &args(serde_json::json!({"angle": "top", "labels": false, "highlight_surfaces": [pocket]})),
        )
        .unwrap();
        let tinted = |png: &[u8]| {
            decode(png)
                .2
                .chunks(3)
                .filter(|p| p == &[92, 168, 223])
                .count()
        };
        assert!(tinted(&marked.png) > tinted(&plain.png) + 100);
        let near = draw(
            &s,
            &args(
                serde_json::json!({"angle": "top", "labels": false, "highlight_surfaces": [pocket], "focus_surfaces": [pocket]}),
            ),
        )
        .unwrap();
        assert!(
            tinted(&near.png) > tinted(&marked.png) * 2,
            "the focused surface fills more of the picture"
        );
        assert!(
            near.description.contains("zoomed on "),
            "{}",
            near.description
        );
    }
    #[test]
    fn bad_arguments_get_one_sentence_errors() {
        let s = scene();
        for (bad, says) in [
            ("not json".to_string(), "could not be read"),
            (
                args(serde_json::json!({"angle": "sideways"})),
                "could not be read",
            ),
            (
                args(serde_json::json!({"focus_surfaces": [99999]})),
                "Surface 99999",
            ),
            (
                args(serde_json::json!({"highlight_surfaces": (0..10_000).collect::<Vec<_>>()})),
                "at most 200",
            ),
            (
                args(serde_json::json!({"operation": {"setup": 1, "operation": 1}})),
                "No proposal has been built yet",
            ),
            (
                args(serde_json::json!({"orientation": "+Q"})),
                "could not be read",
            ),
        ] {
            let error = draw(&s, &bad).unwrap_err();
            assert!(error.contains(says), "{bad}: {error}");
            assert!(
                !error.contains('\n') && error.chars().count() <= 300,
                "{error}"
            );
        }
        let mut extra: serde_json::Value =
            serde_json::from_str(&args(serde_json::json!({}))).unwrap();
        extra["zoom"] = serde_json::json!(2);
        assert!(
            draw(&s, &extra.to_string())
                .unwrap_err()
                .contains("could not be read")
        );
    }
    #[test]
    fn an_operation_of_the_latest_proposal_can_be_looked_at() {
        let mut s = scene();
        let (_, job) = crate::plan_check::evaluate(&s.doc, &s.mesh, &s.report, &good(&s.report));
        s.job = job.map(Arc::new);
        let picture = draw(
            &s,
            &args(serde_json::json!({"operation": {"setup": 1, "operation": 1}})),
        )
        .unwrap();
        assert!(
            picture.description.contains("after setup 1, operation 1"),
            "{}",
            picture.description
        );
        let out = draw(
            &s,
            &args(serde_json::json!({"operation": {"setup": 1, "operation": 99}})),
        )
        .unwrap_err();
        assert!(out.contains("Setup 1 has no operation 99"), "{out}");
    }
    #[test]
    fn the_purpose_is_cleaned_and_clipped() {
        let s = scene();
        let long = format!("why\n\u{202e}{}", "x".repeat(500));
        let p = draw(
            &s,
            &args(serde_json::json!({"purpose": long, "labels": false})),
        )
        .unwrap();
        assert!(p.purpose.starts_with("why x") && p.purpose.chars().count() <= 120);
    }
}
