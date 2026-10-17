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
use std::{collections::HashMap, sync::Arc};

mod canvas;
use canvas::{Canvas, INK};

/// The tool's name in requests.
pub const NAME: &str = "view_model";
/// The pictures' width and height in pixels.
pub const SIZE: u32 = 768;
/// The strip along the bottom: the caption and the scale bar.
const CAPTION: u32 = 44;
/// The share of the picture's width or height, above the caption strip,
/// that the box around what is framed fills.
const FILL: f32 = 0.9;
/// The corner box of the axis triad: no tag may cover it.
const TRIAD: [i32; 4] = [0, 0, 86, 86];
/// The most surfaces one list may name.
const MAX_IDS: usize = 200;
/// Each angle: name, words, yaw and pitch of `viewport::Camera`, whose
/// yaw 0 looks from the front (-Y), a positive yaw round to the right (+X)
/// and a positive pitch from above.
const ANGLES: [(&str, &str, f32, f32); 9] = [
    ("top", "top", 0., std::f32::consts::FRAC_PI_2),
    ("front", "front (from -Y)", 0., 0.),
    ("back", "back (from +Y)", std::f32::consts::PI, 0.),
    ("left", "left (from -X)", -std::f32::consts::FRAC_PI_2, 0.),
    ("right", "right (from +X)", std::f32::consts::FRAC_PI_2, 0.),
    ("iso_front_left", "isometric front-left", -0.75, 0.62),
    ("iso_front_right", "isometric front-right", 0.75, 0.62),
    (
        "iso_back_left",
        "isometric back-left",
        -(std::f32::consts::PI - 0.75),
        0.62,
    ),
    (
        "iso_back_right",
        "isometric back-right",
        std::f32::consts::PI - 0.75,
        0.62,
    ),
];
/// What the pictures are drawn from.
#[derive(Clone)]
pub struct Scene {
    pub doc: Arc<Document>,
    /// Shared: a copy of the scene, as each planner tool call takes, does
    /// not copy the part.
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
        let mesh = &scene.mesh;
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
                // has its vise or clamps placed: a vise round the band it
                // closes on, which the cuts of the setups before it tell.
                Some(stock) if operation.is_some() || up == first_up => {
                    let bands = sim.as_ref().zip(operation).and_then(|(sim, (si, _))| {
                        let (volume, frame) = sim.initial_volume.as_ref().zip(sim.frame)?;
                        let mut before = volume.clone();
                        before.cuts = std::sync::Arc::new(
                            volume
                                .cuts
                                .iter()
                                .filter(|c| usize::from(c.source.0) + 1 < si)
                                .cloned()
                                .collect(),
                        );
                        (!before.cuts.is_empty()).then(|| {
                            crate::holding::Bands::new(
                                &before,
                                frame,
                                crate::simulation::part_box(&setup, mesh),
                            )
                        })
                    });
                    let held = crate::holding::held_by(doc, &setup, stock, bands.as_ref());
                    let mut boxes = doc
                        .holding_boxes(&setup, held)
                        .map(|b| crate::holding::shown(&b, stock))
                        .unwrap_or_default();
                    let on = crate::post::datum(&setup, stock, mesh)
                        .ok()
                        .and_then(|datum| crate::post::on_pins(&setup, mesh, datum));
                    boxes.extend(crate::holding::pin_boxes(on.as_ref(), stock));
                    boxes
                }
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
            pin_holes: Default::default(),
            show_stock: args.show_stock,
            show_paths: false,
            progress: None,
            holding: holding.clone(),
            leftover: None,
            live: None,
            live_stock: None,
            uncut: &[],
            show_uncut: true,
        };
        let boxes = focus_bounds(mesh, &setup, &args.focus_surfaces)
            .map(|b| vec![b])
            .unwrap_or_else(|| viewport.scene_boxes());
        let frame = (SIZE as f32, (SIZE - CAPTION) as f32);
        let projector = Projector::filling(camera, frame, &boxes, FILL);
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
    /// x, y in pixels, and its depth.
    pub(crate) fn screen(&self, v: [f32; 3]) -> (f32, f32, f32) {
        let q = self.projector.p(v);
        (q.x, q.y, self.projector.depth(v))
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
    Ok(paint(scene, args, &View::new(scene, args)?))
}
/// `view` drawn as `args` ask: the scene, the tags and the caption.
fn paint(scene: &Scene, args: &Args, view: &View) -> Picture {
    let mesh = &scene.mesh;
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
        pin_holes: Default::default(),
        show_stock: args.show_stock,
        show_paths: view.operation.is_some(),
        progress: None,
        holding: view.holding.clone(),
        leftover: None,
        live: None,
        live_stock: None,
        uncut: &[],
        show_uncut: true,
    };
    let (w, h) = (SIZE, SIZE - CAPTION);
    let (rgba, depths) =
        viewport.render(&view.projector, (w, h), 1., None, &args.highlight_surfaces);
    let mut canvas = Canvas::over(&rgba, w, h);
    draw_toolpaths(&mut canvas, view, &depths);
    // Tags go on before the caption strip is added, so none reaches into it.
    let tags = if args.labels {
        draw_tags(&mut canvas, scene, view, &depths)
    } else {
        vec![]
    };
    canvas.rgb.resize((SIZE * SIZE * 3) as usize, 255);
    canvas.h = SIZE;
    caption(&mut canvas, view, mesh.display_scale());
    Picture {
        png: Arc::new(canvas.png()),
        description: describe(view, args, &tags),
        purpose: crate::proposal::clip(&args.purpose, 120),
    }
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
/// The `view_model` function as the model is offered it: every property is
/// required and nullable ones may be null, as strict schemas demand.
pub fn tool() -> crate::openai::Tool {
    crate::openai::Tool {
        name: NAME,
        description: "Draws a picture of the part as the program sees it and returns it with a short description. Use it to check which faces the report's surface IDs are, where holes and pockets are, how the stock and the vise or clamps sit, and, after a proposal has been checked, what an operation's toolpaths and the stock left after it look like.".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "orientation": {"type": ["string", "null"], "enum": ["+Z", "-Z", "+X", "-X", "+Y", "-Y", null],
                    "description": "The report's orientation to draw the part in; null for setup 1's. Ignored when `operation` is set: an operation is drawn in its own setup."},
                "angle": {"type": "string", "enum": ANGLES.map(|a| a.0)},
                "focus_surfaces": {"type": "array", "items": {"type": "integer"}, "description": "Zoom to these surface IDs; empty for the whole part."},
                "highlight_surfaces": {"type": "array", "items": {"type": "integer"}, "description": "Surface IDs drawn in blue. Not drawn in an `operation` view with `show_stock` true (the stock left after the operation hides the part's surfaces); the description then says so. Set `show_stock` to false to see them."},
                "show_stock": {"type": "boolean"},
                "show_workholding": {"type": "boolean"},
                "labels": {"type": "boolean", "description": "Tags with each listed plane's surface IDs and each hole's diameter."},
                "operation": {"type": ["object", "null"], "properties": {"setup": {"type": "integer"}, "operation": {"type": "integer"}},
                    "required": ["setup", "operation"], "additionalProperties": false,
                    "description": "An operation of your latest checked proposal, counted from 1: its toolpaths and the stock left after it."},
                "purpose": {"type": "string", "description": "A few words on why you look; shown to the user."}
            },
            "required": ["orientation", "angle", "focus_surfaces", "highlight_surfaces", "show_stock", "show_workholding", "labels", "operation", "purpose"],
            "additionalProperties": false
        }),
    }
}
/// The most feed moves an operation view draws; more are sampled evenly.
const MAX_SEGMENTS: usize = 20_000;
/// The feed moves of the operation looked at, as 1.5 px lines wherever
/// nothing drawn is nearer (the viewport itself shows only the last ones).
fn draw_toolpaths(canvas: &mut Canvas, view: &View, depths: &[f32]) {
    let Some(sim) = &view.sim else {
        return;
    };
    let feeds: Vec<_> = sim
        .plan
        .moves
        .iter()
        .filter(|m| m.motion == crate::simulation::Motion::Feed)
        .collect();
    let n = feeds.len().min(MAX_SEGMENTS);
    let scale = sim.plan.display_scale;
    let (w, h) = (canvas.w as usize, canvas.h as usize);
    for m in (0..n).map(|i| feeds[i * feeds.len() / n]) {
        let a = view.screen(m.from.map(|v| v * scale));
        let b = view.screen(m.to.map(|v| v * scale));
        let steps = ((b.0 - a.0).hypot(b.1 - a.1).ceil() as usize).clamp(1, 4000);
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            if !(x >= 0. && y >= 0. && x < w as f32 && y < h as f32) {
                continue;
            }
            // The viewport's tolerance for its depth-tested lines.
            let z = a.2 + (b.2 - a.2) * t;
            if z + 0.05 >= depths[y as usize * w + x as usize] {
                canvas.disc((x, y), 0.75, [34, 114, 176]);
            }
        }
    }
}
/// The report's tags for the drawn orientation where their points can be
/// seen: each plane's surface IDs, largest plane first, then the diameters
/// of its holes at their centres. A tag that would leave the picture or
/// overlap the triad's corner or a tag already drawn is skipped. Returns
/// the texts drawn, in order.
///
/// Unlike the design's "centre of its bbox", a plane's tag sits on the
/// plane itself (`plane_anchors`): the centre of a plane around a pocket
/// lies over the pocket, where its tag would float and hide the floor's.
/// It goes at the first of its anchors where it is seen, inside the
/// picture and clear of the triad and the tags already drawn (`spot`).
fn draw_tags(canvas: &mut Canvas, scene: &Scene, view: &View, depths: &[f32]) -> Vec<String> {
    let Some(o) = scene
        .report
        .orientations
        .iter()
        .find(|o| Some(o.up) == view.up)
    else {
        return vec![];
    };
    let mesh = &scene.mesh;
    let scale = mesh.display_scale();
    let placement = Placement::new(mesh, Some(&view.setup));
    let model = placement.bounds;
    // Report positions are mm from the model's -X/-Y corner and its top.
    let at = |x: f32, y: f32, depth: f32| {
        [
            model.min[0] + x * scale,
            model.min[1] + y * scale,
            model.max[2] - depth * scale,
        ]
    };
    let anchors = plane_anchors(mesh, &placement, &o.planes);
    let planes = o.planes.iter().zip(anchors).map(|(plane, anchors)| {
        let ids: Vec<String> = plane
            .surfaces
            .iter()
            .take(3)
            .map(usize::to_string)
            .collect();
        let more = if plane.surfaces.len() > 3 { "…" } else { "" };
        let z = model.max[2] - plane.depth_below_top * scale;
        let mut points: Vec<[f32; 3]> = anchors.iter().map(|[x, y]| [*x, *y, z]).collect();
        if points.is_empty() {
            let [x0, y0, x1, y1] = plane.bbox;
            points.push(at((x0 + x1) / 2., (y0 + y1) / 2., plane.depth_below_top));
        }
        (points, format!("{}{more}", ids.join(",")))
    });
    let holes = o.planes.iter().flat_map(|plane| {
        plane.holes.iter().map(|hole| {
            (
                vec![at(hole.center[0], hole.center[1], plane.depth_below_top)],
                format!("Ø{:.1}", hole.diameter),
            )
        })
    });
    let frame = (canvas.w as usize, canvas.h as usize);
    // The triad is drawn later, over whatever is in its corner.
    let mut drawn = vec![TRIAD];
    let mut texts = vec![];
    for (points, text) in planes.chain(holes) {
        let points = points.iter().map(|p| view.screen(*p));
        let Some(at) = spot(points, &text, &drawn, frame, depths) else {
            continue;
        };
        if let Some(rect) = canvas.tag(at, &text) {
            drawn.push(rect);
            texts.push(text);
        }
    }
    texts
}
/// Where the tag reading `text` goes: the first of `points` (picture x, y
/// and depth) that is in the `w` × `h` picture and not hidden behind what
/// was drawn (`depths`), and whose box
/// lies inside the picture and clear of the `taken` boxes. None when no
/// point will do.
fn spot(
    points: impl IntoIterator<Item = (f32, f32, f32)>,
    text: &str,
    taken: &[[i32; 4]],
    (w, h): (usize, usize),
    depths: &[f32],
) -> Option<(f32, f32)> {
    points
        .into_iter()
        .find(|&(x, y, z)| {
            if !(x >= 0. && y >= 0. && x < w as f32 && y < h as f32) {
                return false;
            }
            if z + 0.5 < depths[y as usize * w + x as usize] {
                return false;
            }
            let rect = Canvas::tag_rect((x, y), text);
            let inside = rect[0] >= 0 && rect[1] >= 0 && rect[2] <= w as i32 && rect[3] <= h as i32;
            inside
                && !taken
                    .iter()
                    .any(|r| r[0] < rect[2] && rect[0] < r[2] && r[1] < rect[3] && rect[1] < r[3])
        })
        .map(|(x, y, _)| (x, y))
}
/// The most anchors a plane's tag tries.
const ANCHORS: usize = 8;
/// Where each plane's tag may go, best first: the centroids (x, y in display
/// units, in `placement`'s frame) of the largest triangles of the plane's
/// faces, largest first; empty for a plane without any.
fn plane_anchors(
    mesh: &Mesh,
    placement: &Placement,
    planes: &[analysis::PlaneReport],
) -> Vec<Vec<[f32; 2]>> {
    let owner: HashMap<usize, usize> = planes
        .iter()
        .enumerate()
        .flat_map(|(i, p)| p.surfaces.iter().map(move |id| (*id, i)))
        .collect();
    let mut triangles: Vec<Vec<(f32, [f32; 2])>> = vec![vec![]; planes.len()];
    for face in &mesh.faces {
        let Some(&i) = face.feature.and_then(|id| owner.get(&id)) else {
            continue;
        };
        let p: Vec<[f32; 3]> = face
            .points
            .iter()
            .map(|q| placement.transform(*q))
            .collect();
        // The face as a fan of triangles; planes are flat, so the area seen
        // from above is the area.
        for (b, c) in p.iter().skip(1).zip(p.iter().skip(2)) {
            let a = p[0];
            let area = ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() / 2.;
            let centroid = [(a[0] + b[0] + c[0]) / 3., (a[1] + b[1] + c[1]) / 3.];
            triangles[i].push((area, centroid));
        }
    }
    triangles
        .into_iter()
        .map(|mut t| {
            t.sort_by(|a, b| b.0.total_cmp(&a.0));
            t.into_iter().take(ANCHORS).map(|(_, c)| c).collect()
        })
        .collect()
}
/// The words for the view's up axis.
fn up_words(view: &View) -> &'static str {
    view.up.unwrap_or("rotated setup")
}
/// The strip along the bottom (the view in words and a scale bar) and the
/// axis triad in the top-left corner, inside `TRIAD`: 28 px axes from
/// (40, 40) and their letters 9 px beyond.
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
    // The viewport leaves the part's faces out when it shows the stock left
    // after an operation, so nothing is there to tint.
    let highlights = if view.sim.is_some() && args.show_stock && !args.highlight_surfaces.is_empty()
    {
        let ids: Vec<String> = args
            .highlight_surfaces
            .iter()
            .map(usize::to_string)
            .collect();
        format!(
            "; highlight of surfaces {} not drawn (stock after the operation shown; hide the stock to see them)",
            ids.join(", ")
        )
    } else {
        String::new()
    };
    let tags = if tags.is_empty() {
        "none".to_string()
    } else {
        tags.join(", ")
    };
    format!(
        "{} up, {}, {scope}{after}; {stock}{holding}{highlights}; tags: {tags}",
        up_words(view),
        view.angle_words
    )
}

#[cfg(test)]
mod tests {
    use super::*;
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
    /// The block with 36 mm of stock around it and a clamp on each side
    /// in X, as setup 1's workholding.
    fn clamped() -> Scene {
        let (mut doc, mesh, _) = block();
        for key in ["stock_left", "stock_right", "stock_front", "stock_back"] {
            doc.setups[0].p.insert(key.into(), "36".into());
        }
        crate::proposal::tests::hold(&mut doc, "Clamp", &[("clamps", "-X:0;+X:0")]);
        let report = crate::analysis::report(&doc, &mesh).unwrap();
        Scene {
            doc: Arc::new(doc),
            mesh: Arc::new(mesh),
            report: Arc::new(report),
            job: None,
        }
    }
    /// The colour of a decoded picture's pixel at `(x, y)`.
    fn pixel(rgb: &[u8], x: usize, y: usize) -> [u8; 3] {
        let i = (y * SIZE as usize + x) * 3;
        [rgb[i], rgb[i + 1], rgb[i + 2]]
    }
    #[test]
    fn the_overview_fills_the_picture() {
        for (name, s) in [("block", scene()), ("clamped", clamped())] {
            // The overview's view without its tags, so only the scene counts;
            // the triad's corner is left out too.
            let picture = draw(&s, &args(serde_json::json!({"labels": false}))).unwrap();
            assert!(
                picture.description.contains("vise or clamps shown") == (name == "clamped"),
                "{}",
                picture.description
            );
            let (_, _, rgb) = decode(&picture.png);
            let (w, h) = (SIZE as usize, (SIZE - CAPTION) as usize);
            let (mut lo, mut hi) = ([usize::MAX; 2], [0; 2]);
            for y in 0..h {
                for x in 0..w {
                    let triad = x < TRIAD[2] as usize && y < TRIAD[3] as usize;
                    if !triad && pixel(&rgb, x, y) != [239, 243, 247] {
                        lo = [lo[0].min(x), lo[1].min(y)];
                        hi = [hi[0].max(x), hi[1].max(y)];
                    }
                }
            }
            let across = (hi[0] + 1 - lo[0]) as f32 / w as f32;
            let down = (hi[1] + 1 - lo[1]) as f32 / h as f32;
            assert!(
                across >= 0.8 || down >= 0.8,
                "{name}: the scene spans {across:.2} of the width and {down:.2} of the height"
            );
            // Inside the picture, with a margin on every side.
            assert!(
                lo[0] > 10 && lo[1] > 10 && hi[0] < w - 10 && hi[1] < h - 10,
                "{name}: {lo:?} {hi:?}"
            );
        }
    }
    #[test]
    fn no_tag_covers_the_triad() {
        let s = scene();
        let args = Args::parse(&args(serde_json::json!({}))).unwrap();
        let mut view = View::new(&s, &args).unwrap();
        // The part drawn small around the triad's centre, so that every
        // tag's point is near it. The overview's angle, isometric front-left.
        let (_, _, yaw, pitch) = ANGLES[5];
        let camera = Camera {
            yaw,
            pitch,
            zoom: 1.,
            pan: [0., 0.],
        };
        let part = Placement::new(&s.mesh, Some(&view.setup)).bounds;
        let corner = (80., 80.);
        view.projector = Projector::filling(camera, corner, &[part], 0.9);
        let (x, y, _) = view.screen(std::array::from_fn(|k| (part.min[k] + part.max[k]) / 2.));
        assert!(
            (35. ..45.).contains(&x) && (35. ..45.).contains(&y),
            "{x} {y}"
        );
        let picture = paint(&s, &args, &view);
        // Tags are white boxes; nothing else above the caption strip is
        // pure white.
        let (_, _, rgb) = decode(&picture.png);
        let covered = (0..TRIAD[3] as usize)
            .flat_map(|y| (0..TRIAD[2] as usize).map(move |x| (x, y)))
            .find(|&(x, y)| pixel(&rgb, x, y) == [255, 255, 255]);
        assert_eq!(covered, None, "{}", picture.description);
    }
    #[test]
    fn a_tag_goes_to_its_next_point_rather_than_cover_the_triad_or_leave_the_picture() {
        let frame = (SIZE as usize, (SIZE - CAPTION) as usize);
        let depths = vec![f32::NEG_INFINITY; frame.0 * frame.1];
        let spot = |points: &[(f32, f32, f32)], taken: &[[i32; 4]]| {
            spot(
                points.iter().copied(),
                "2562,2563,2566…",
                taken,
                frame,
                &depths,
            )
        };
        let inside = (400., 300., 0.);
        // Inside the picture, but over the triad's corner.
        let corner = (120., 60., 0.);
        assert_eq!(spot(&[corner], &[]), Some((120., 60.)));
        assert_eq!(spot(&[corner, inside], &[TRIAD]), Some((400., 300.)));
        // Its point in the picture, its box not: at each edge.
        for edge in [
            (20., 300., 0.),
            (750., 300., 0.),
            (400., 5., 0.),
            (400., 720., 0.),
        ] {
            assert_eq!(spot(&[edge, inside], &[]), Some((400., 300.)), "{edge:?}");
        }
        // Over a tag already drawn, or with no point left.
        let drawn = Canvas::tag_rect((400., 300.), "12");
        assert_eq!(spot(&[inside], &[drawn]), None);
        assert_eq!(spot(&[(600., 500., 0.)], &[drawn]), Some((600., 500.)));
        // Hidden behind what was drawn nearer.
        let mut near = depths.clone();
        near[300 * frame.0 + 400] = 10.;
        assert_eq!(
            super::spot([inside, (600., 500., 0.)], "12", &[], frame, &near),
            Some((600., 500.))
        );
    }
    #[test]
    fn the_tool_requires_every_argument_the_views_take() {
        let tool = tool();
        assert_eq!(tool.name, NAME);
        let schema = &tool.parameters;
        let keys = |v: &serde_json::Value| -> Vec<String> {
            let mut keys: Vec<String> = match v {
                serde_json::Value::Object(map) => map.keys().cloned().collect(),
                list => list
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|k| k.as_str().unwrap().into())
                    .collect(),
            };
            keys.sort();
            keys
        };
        let sent: serde_json::Value = serde_json::from_str(&args(serde_json::json!({}))).unwrap();
        assert_eq!(keys(&schema["properties"]), keys(&sent));
        assert_eq!(keys(&schema["required"]), keys(&sent));
        let angles: Vec<&str> = schema["properties"]["angle"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect();
        assert_eq!(angles, ANGLES.map(|a| a.0));
    }
    #[test]
    fn a_scene_can_move_between_threads() {
        fn assert_send<T: Send + 'static>() {}
        assert_send::<Scene>();
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
    fn the_pocket_floor_is_tagged_where_it_can_be_seen() {
        let s = scene();
        let floor = s.report.orientations[0]
            .planes
            .iter()
            .find(|p| p.kind == "floor")
            .unwrap();
        let id = floor.surfaces[0].to_string();
        let tagged = |p: &Picture| {
            let (_, tags) = p.description.split_once("tags: ").unwrap();
            tags.split(", ")
                .any(|t| t.split(',').any(|n| n.trim_end_matches('…') == id))
        };
        let picture = overview(&s).unwrap();
        assert!(tagged(&picture), "{}", picture.description);
        // From the front the floor is behind the pocket's front wall.
        let front = draw(
            &s,
            &args(serde_json::json!({"angle": "front", "show_stock": false})),
        )
        .unwrap();
        assert!(!tagged(&front), "{}", front.description);
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
        // The whole part fills the picture already and the floor is most of
        // its top, so it gains about 1.8 times the pixels.
        let (near_px, whole_px) = (tinted(&near.png), tinted(&marked.png));
        assert!(
            near_px * 2 > whole_px * 3,
            "the focused surface fills more of the picture: {near_px} against {whole_px}"
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
    fn an_operation_of_the_latest_proposal_is_drawn_with_its_toolpaths() {
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
        let feed = decode(&picture.png)
            .2
            .chunks(3)
            .filter(|p| p == &[34, 114, 176])
            .count();
        // Every pass, not only the last one the viewport draws (~600 px).
        assert!(feed >= 2000, "{feed} pixels of toolpath");
        let out = draw(
            &s,
            &args(serde_json::json!({"operation": {"setup": 1, "operation": 99}})),
        )
        .unwrap_err();
        assert!(out.contains("Setup 1 has no operation 99"), "{out}");
    }
    #[test]
    fn highlights_are_reported_as_not_drawn_when_the_stock_after_an_operation_is_shown() {
        let mut s = scene();
        let (_, job) = crate::plan_check::evaluate(&s.doc, &s.mesh, &s.report, &good(&s.report));
        s.job = job.map(Arc::new);
        let pocket = s.report.orientations[0]
            .planes
            .iter()
            .find(|p| p.kind == "floor")
            .unwrap()
            .surfaces[0];
        let look = |stock: bool, highlight: Vec<usize>| {
            draw(
                &s,
                &args(serde_json::json!({
                    "operation": {"setup": 1, "operation": 1},
                    "show_stock": stock,
                    "highlight_surfaces": highlight,
                })),
            )
            .unwrap()
            .description
        };
        // The part's faces are not drawn then, so the model is told.
        let hidden = look(true, vec![pocket]);
        assert!(
            hidden.contains(&format!("highlight of surfaces {pocket} not drawn")),
            "{hidden}"
        );
        assert!(look(true, vec![]).find("not drawn").is_none());
        // With the stock hidden the part is drawn and tinted, nothing to report.
        let shown = look(false, vec![pocket]);
        assert!(shown.find("not drawn").is_none(), "{shown}");
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
