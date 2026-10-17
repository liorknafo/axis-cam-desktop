use crate::{
    mesh::{Face, Mesh},
    model::{Operation, Setup, Strategy},
    stock::{self, Placement},
};
use iced::{
    Color, Element, Fill, Length, Point, Rectangle, Renderer, Theme, mouse,
    widget::{
        Action, canvas,
        canvas::{Frame, Path, Stroke, Text},
    },
};
use std::cell::RefCell;
/// Where the 3D view looks from. The viewer is at (sin yaw · cos pitch,
/// −cos yaw · cos pitch, sin pitch) from the target: yaw 0 looks from the
/// front (−Y), a positive yaw goes round to the right (+X) and a positive
/// pitch rises above. The frame is right-handed, screen right (cos yaw,
/// sin yaw, 0) and screen up +Z projected on the screen, so a top view
/// (yaw 0, pitch π/2) shows +X to the right and +Y up, a front view +X to
/// the right and +Z up, and a right view (yaw π/2) +Y to the right.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub pan: [f32; 2],
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: 0.75,
            pitch: 0.65,
            zoom: 1.,
            pan: [0., 0.],
        }
    }
}
#[derive(Debug, Clone)]
pub enum Event {
    Orbit(f32, f32),
    Pan(f32, f32),
    Zoom(f32),
    Pick(usize),
    PickSurface(usize),
    /// Where pin 1 would stand under the pointer while placing the pins
    /// (`PinAim`), in plan mm; None once the pointer leaves the view.
    AimPins(Option<[f32; 2]>),
    /// A click, or a drag of the pins ending, while placing them: pin 1
    /// stands there (plan mm).
    PlacePins([f32; 2]),
}
#[derive(Default)]
pub struct State {
    drag: Option<(Point, Point, mouse::Button)>,
    moved: bool,
    /// While the placed pins are dragged: where the pointer took them, from
    /// pin 1 (plan mm).
    grab: Option<[f32; 2]>,
    /// Whether the pins follow the pointer, so that leaving the view says so.
    aiming: bool,
    raster: RefCell<Option<(String, RasterFrame)>>,
}

// Iced 0.14 uploads RGBA images >= 2 MiB asynchronously and omits them from
// the first frame. Orbiting replaces every image, so that path causes blinking.
// Each full-resolution tile (including its filtering gutter) is under 1.1 MiB.
const TILE_SIDE: u32 = 512;
struct RasterTile {
    image: iced::widget::image::Handle,
    image_bounds: Rectangle,
    clip_bounds: Rectangle,
}
struct RasterFrame {
    width: u32,
    height: u32,
    tiles: Vec<RasterTile>,
}
impl RasterFrame {
    fn from_rgba(width: u32, height: u32, pixels: &[u8]) -> Self {
        let mut tiles = Vec::new();
        for y in (0..height).step_by(TILE_SIDE as usize) {
            for x in (0..width).step_by(TILE_SIDE as usize) {
                let right = (x + TILE_SIDE).min(width);
                let bottom = (y + TILE_SIDE).min(height);
                // Keep one neighboring pixel for seamless linear filtering;
                // clip the extra pixels when compositing adjacent tiles.
                let left = x.saturating_sub(1);
                let top = y.saturating_sub(1);
                let image_right = (right + 1).min(width);
                let image_bottom = (bottom + 1).min(height);
                let w = image_right - left;
                let h = image_bottom - top;
                let mut rgba = Vec::with_capacity((w * h * 4) as usize);
                for row in top..image_bottom {
                    let begin = ((row * width + left) * 4) as usize;
                    rgba.extend_from_slice(&pixels[begin..begin + (w * 4) as usize]);
                }
                tiles.push(RasterTile {
                    image: iced::widget::image::Handle::from_rgba(w, h, rgba),
                    image_bounds: Rectangle {
                        x: left as f32,
                        y: top as f32,
                        width: w as f32,
                        height: h as f32,
                    },
                    clip_bounds: Rectangle {
                        x: x as f32,
                        y: y as f32,
                        width: (right - x) as f32,
                        height: (bottom - y) as f32,
                    },
                });
            }
        }
        Self {
            width,
            height,
            tiles,
        }
    }
    fn draw(&self, frame: &mut Frame, size: iced::Size) {
        let map = |r: Rectangle| Rectangle {
            x: r.x * size.width / self.width as f32,
            y: r.y * size.height / self.height as f32,
            width: r.width * size.width / self.width as f32,
            height: r.height * size.height / self.height as f32,
        };
        for tile in &self.tiles {
            frame.with_clip(map(tile.clip_bounds), |f| {
                f.draw_image(
                    map(tile.image_bounds),
                    canvas::Image::new(tile.image.clone()),
                );
            });
        }
    }
}
/// The part's holes the setup editor offers for the fixture pins and has
/// not chosen: each one's number in its list (from 1), its opening's centre
/// in the setup's frame and radius (mm), drawn as a ring labelled "Hole N";
/// and whether a click on the part picks one (`picking` then picks
/// surfaces with no operation).
#[derive(Clone, Debug, Default)]
pub struct PinHolePick {
    pub pick: bool,
    pub holes: Vec<(usize, [f32; 3], f32)>,
}
/// The fixture pins being placed in the stock's leftover ("Place the pins
/// on the leftover", the Work zero step): the plate's pins follow the
/// pointer on the stock's bottom, and a click, or a drag of the placed
/// pins, puts pin 1 there.
#[derive(Clone, Debug, Default)]
pub struct PinAim {
    /// Whether the pointer places them.
    pub on: bool,
    /// The height of the stock's bottom they stand on (plan mm).
    pub z: f32,
    /// Each pin from pin 1, as the plate has them (mm).
    pub offsets: Vec<[f32; 2]>,
    pub diameter: f32,
    pub height: f32,
    /// Where pin 1 stands, placed (plan mm).
    pub placed: Option<[f32; 2]>,
    pub ghost: Option<Ghost>,
}
/// The pins under the pointer while placing them (`PinAim`): pin 1 (plan
/// mm), and how far they keep from the part and the stock's edge, or why
/// they do not fit there.
pub type Ghost = ([f32; 2], Result<(f32, f32), String>);
#[derive(Clone)]
pub struct Viewport<'a> {
    pub setup: Option<&'a Setup>,
    pub operation: Option<&'a Operation>,
    pub mesh: &'a Mesh,
    pub simulation: Option<&'a crate::simulation::Simulation>,
    pub sample: bool,
    pub camera: Camera,
    pub picking: bool,
    /// The part's holes offered for the fixture pins (the Work zero step).
    pub pin_holes: PinHolePick,
    /// The pins being placed in the stock's leftover (the Work zero step).
    pub pin_aim: PinAim,
    pub show_stock: bool,
    pub show_paths: bool,
    pub progress: Option<f32>,
    /// The jaws or clamps of the setup, in its frame in mm; drawn with the
    /// stock and part of the camera's fit.
    pub holding: Vec<crate::holding::HoldBox>,
    /// The box a later setup starts from, in its frame in mm, when the setup
    /// editor knows it (`App::known_stock`): drawn as the stock, and said in
    /// the label, where the setup's own box would be the original block's.
    /// None for the first setup and when it is not known.
    pub leftover: Option<crate::mesh::Bounds>,
    /// The machine's tool while a tool run of this setup cuts: its tip in
    /// plan coordinates (mm), radius and stickout. Drawn over the scene.
    pub live: Option<([f32; 3], f32, f32)>,
    /// The stock a run of this setup has carved, or starts from, in plan
    /// coordinates (`run::Run::stock`) with its version: drawn instead of the
    /// setup's stock when no simulation is shown.
    pub live_stock: Option<(&'a crate::simulation::StockGrid, u64)>,
    /// Per face of the part (`Mesh::faces`): whether the whole job still
    /// leaves it under stock (`uncut::Uncut::faces`); those are drawn in
    /// `UNCUT`. Empty when that is not known.
    pub uncut: &'a [bool],
    /// Whether those faces are drawn in `UNCUT` at all ("Show uncut faces").
    /// With a simulation and the stock shown the part itself is not drawn, so
    /// then just these faces are, after the stock and depth-tested against it.
    pub show_uncut: bool,
}
pub(crate) struct Projector {
    camera: Camera,
    center: Point,
    pub(crate) scale: f32,
    target: [f32; 3],
}
impl Projector {
    fn new(c: Camera, b: Rectangle) -> Self {
        Self {
            camera: c,
            center: Point::new(b.width * 0.5 + c.pan[0], b.height * 0.53 + c.pan[1]),
            scale: (b.width / 235.).min(b.height / 170.) * c.zoom,
            target: [0., 0., 15.],
        }
    }
    /// The camera fitted to `bounds` (display units) in a `size` frame: the
    /// bounds' centre where the 3D view puts its target (centred across, a
    /// touch below the middle), their bounding sphere filling the frame.
    pub(crate) fn fit(camera: Camera, size: (f32, f32), bounds: crate::mesh::Bounds) -> Self {
        let frame = Rectangle {
            x: 0.,
            y: 0.,
            width: size.0,
            height: size.1,
        };
        let mut p = Projector::new(camera, frame);
        p.target = std::array::from_fn(|i| bounds.min[i] + (bounds.max[i] - bounds.min[i]) / 2.);
        // Fit the scene's bounding sphere once per scene/size/zoom. Its diameter
        // is independent of yaw and pitch, so orbiting never changes zoom.
        let diameter = bounds
            .dimensions()
            .iter()
            .map(|v| v * v)
            .sum::<f32>()
            .sqrt()
            .max(1.);
        p.scale = (size.0 * 0.86).min(size.1 * 0.82) / diameter * camera.zoom;
        p
    }
    /// `fit` for a still picture: scaled and moved so that the box around
    /// the projected corners of `boxes` is centred and fills `share` of the
    /// frame's width or height. The 3D view keeps `fit`, so that orbiting
    /// never changes its zoom.
    pub(crate) fn filling(
        camera: Camera,
        size: (f32, f32),
        boxes: &[crate::mesh::Bounds],
        share: f32,
    ) -> Self {
        let mut all = boxes[0];
        for b in &boxes[1..] {
            all.min = std::array::from_fn(|k| all.min[k].min(b.min[k]));
            all.max = std::array::from_fn(|k| all.max[k].max(b.max[k]));
        }
        let mut p = Self::fit(camera, size, all);
        let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
        for b in boxes {
            for corner in 0..8 {
                let q = p.p(std::array::from_fn(|k| {
                    if corner >> k & 1 == 1 {
                        b.max[k]
                    } else {
                        b.min[k]
                    }
                }));
                lo = [lo[0].min(q.x), lo[1].min(q.y)];
                hi = [hi[0].max(q.x), hi[1].max(q.y)];
            }
        }
        let k = (size.0 * share / (hi[0] - lo[0]).max(1.))
            .min(size.1 * share / (hi[1] - lo[1]).max(1.));
        // The box's middle goes to the frame's; every point keeps its offset
        // from it, k times larger.
        let middle = Point::new((lo[0] + hi[0]) / 2., (lo[1] + hi[1]) / 2.);
        p.center = Point::new(
            size.0 / 2. + (p.center.x - middle.x) * k,
            size.1 / 2. + (p.center.y - middle.y) * k,
        );
        p.scale *= k;
        p
    }
    pub(crate) fn p(&self, v: [f32; 3]) -> Point {
        let [x, y, z] = std::array::from_fn(|i| v[i] - self.target[i]);
        let (s, c) = self.camera.yaw.sin_cos();
        let (se, ce) = self.camera.pitch.sin_cos();
        Point::new(
            self.center.x + (x * c + y * s) * self.scale,
            self.center.y - (-x * s * se + y * c * se + z * ce) * self.scale,
        )
    }
    /// The point of the plane at height `z` the screen point `at` shows
    /// (`p` back): None when the view looks along that plane.
    pub(crate) fn unproject(&self, at: Point, z: f32) -> Option<[f32; 3]> {
        let (s, c) = self.camera.yaw.sin_cos();
        let (se, ce) = self.camera.pitch.sin_cos();
        if se.abs() < 0.05 {
            return None;
        }
        let a = (at.x - self.center.x) / self.scale;
        let b = ((self.center.y - at.y) / self.scale - (z - self.target[2]) * ce) / se;
        Some([
            self.target[0] + a * c - b * s,
            self.target[1] + a * s + b * c,
            z,
        ])
    }
    /// How far toward the viewer: larger is nearer.
    pub(crate) fn depth(&self, v: [f32; 3]) -> f32 {
        let (s, c) = self.camera.yaw.sin_cos();
        let (se, ce) = self.camera.pitch.sin_cos();
        v[0] * s * ce - v[1] * c * ce + v[2] * se
    }
}
impl Viewport<'_> {
    fn projector(&self, b: Rectangle) -> Projector {
        Projector::fit(self.camera, (b.width, b.height), self.scene_bounds())
    }
    /// What the camera must contain, in display units: the part, the stock,
    /// the jaws or clamps and, when simulating, the stock and every move.
    pub(crate) fn scene_bounds(&self) -> crate::mesh::Bounds {
        let mut boxes = self.scene_boxes().into_iter();
        let mut bounds = boxes.next().expect("the part is always framed");
        for b in boxes {
            for i in 0..3 {
                bounds.min[i] = bounds.min[i].min(b.min[i]);
                bounds.max[i] = bounds.max[i].max(b.max[i]);
            }
        }
        bounds
    }
    /// The stock the view shows for its setup: the box a later setup starts
    /// from when that is known (`leftover`), else the setup's own.
    fn resolved_stock(&self) -> Option<stock::ResolvedStock> {
        let setup = self.setup?;
        match self.leftover {
            Some(stock) => Some(stock::ResolvedStock {
                part: None,
                stock,
                display_scale: self.mesh.display_scale(),
            }),
            None => stock::resolve(setup, Some(self.mesh)).ok(),
        }
    }
    /// The setup's work zero in plan coordinates (mm), as the program and a
    /// run put it (`post::datum`) on the stock the simulation starts from,
    /// or the stock the view shows when that is the real one. None without
    /// a setup or a known stock, and for the example.
    pub(crate) fn zero(&self) -> Option<[f32; 3]> {
        let setup = self.setup.filter(|_| !self.sample)?;
        let stock = match self.simulation {
            Some(sim) => sim.plan.stock,
            None => {
                let resolved = self
                    .resolved_stock()
                    .filter(|_| self.stock_is_known(setup))?;
                resolved.stock
            }
        };
        crate::post::datum(setup, stock, self.mesh).ok()
    }
    /// Whether `setup`'s stock is the real one, to be drawn without a
    /// simulation: the first setup's, the example's and a later setup's whose
    /// box is known. Otherwise it is the original block, which is not drawn.
    fn stock_is_known(&self, setup: &Setup) -> bool {
        self.sample
            || self.leftover.is_some()
            || crate::model::get(&setup.p, "stock_source") == "fresh"
    }
    /// `scene_bounds` box by box: the part's first, then the stock's, each
    /// jaw's or clamp's and, when simulating, the simulated stock's and the
    /// reach of every move's cutter. Pictures fit to their projected corners.
    pub(crate) fn scene_boxes(&self) -> Vec<crate::mesh::Bounds> {
        let mut boxes = vec![Placement::new(self.mesh, self.setup).bounds];
        if self.show_stock
            && let Some(stock) = self.resolved_stock()
        {
            boxes.push(stock.stock.scaled(stock.display_scale));
        }
        let scale = self.mesh.display_scale();
        boxes.extend(self.holding.iter().map(|hold| hold.bounds.scaled(scale)));
        if let Some(sim) = self.simulation {
            boxes.push(sim.plan.stock.scaled(sim.plan.display_scale));
            // Keep the whole cutter and motion inside a stable camera fit.
            let mut reach = crate::mesh::Bounds {
                min: [f32::INFINITY; 3],
                max: [f32::NEG_INFINITY; 3],
            };
            for point in sim.plan.moves.iter().flat_map(|m| [m.from, m.to]) {
                for (k, &pk) in point.iter().enumerate() {
                    reach.min[k] = reach.min[k].min(
                        (pk - if k == 2 {
                            0.
                        } else {
                            sim.plan.radius.max(sim.plan.holder_radius)
                        }) * sim.plan.display_scale,
                    );
                    reach.max[k] = reach.max[k].max(
                        (pk + if k == 2 {
                            sim.plan.tool_length + 8.
                        } else {
                            sim.plan.radius.max(sim.plan.holder_radius)
                        }) * sim.plan.display_scale,
                    );
                }
            }
            if !sim.plan.moves.is_empty() {
                boxes.push(reach);
            }
        }
        boxes
    }
    fn transformed(&self) -> Vec<Face> {
        let placement = Placement::new(self.mesh, self.setup);
        self.mesh
            .faces
            .iter()
            .map(|f| Face {
                points: f.points.iter().map(|p| placement.transform(*p)).collect(),
                feature: f.feature,
                tone: f.tone,
            })
            .collect()
    }
    /// The point of the stock's bottom under the screen point `at` while
    /// placing the pins (plan mm).
    fn aimed(&self, b: Rectangle, at: Point) -> Option<[f32; 2]> {
        let scale = self.mesh.display_scale();
        let q = self.projector(b).unproject(at, self.pin_aim.z * scale)?;
        Some([q[0] / scale, q[1] / scale])
    }
    /// Where the pointer took the placed pins at the screen point `at`,
    /// from pin 1 (plan mm): None off them.
    fn grabbed(&self, b: Rectangle, at: Point) -> Option<[f32; 2]> {
        let (aim, placed) = (&self.pin_aim, self.pin_aim.placed?);
        let q = self.aimed(b, at)?;
        let p = self.projector(b);
        let scale = self.mesh.display_scale();
        // Near a pin on the screen, or within its hole.
        let reach = (aim.diameter / 2. * scale * p.scale).max(10.);
        aim.offsets
            .iter()
            .any(|d| {
                let c = [placed[0] + d[0], placed[1] + d[1]];
                p.p([c[0] * scale, c[1] * scale, aim.z * scale])
                    .distance(at)
                    <= reach
            })
            .then(|| [q[0] - placed[0], q[1] - placed[1]])
    }
    fn picked(&self, b: Rectangle, point: Point) -> Option<usize> {
        if !self.picking
            || (!self.sample
                && !self.pin_holes.pick
                && !self
                    .operation
                    .is_some_and(|o| o.strategy.imported_preview()))
        {
            return None;
        }
        let p = self.projector(b);
        let mut nearest = f32::NEG_INFINITY;
        let mut feature = None;
        for face in self.transformed() {
            let vertices: Vec<_> = face.points.iter().map(|v| (p.p(*v), p.depth(*v))).collect();
            for i in 1..vertices.len() - 1 {
                if let Some(z) = triangle_depth([vertices[0], vertices[i], vertices[i + 1]], point)
                    && z > nearest
                {
                    nearest = z;
                    feature = face.feature;
                }
            }
        }
        feature.filter(|f| {
            if self.sample {
                self.operation
                    .is_some_and(|o| o.strategy.features().contains(f))
            } else {
                *f < self.mesh.surfaces.len()
            }
        })
    }
    fn raster(&self, p: &Projector, bounds: Rectangle, hovered: Option<usize>) -> RasterFrame {
        let (w, h, rgba) = self.pixels(p, bounds, hovered);
        RasterFrame::from_rgba(w, h, &rgba)
    }
    /// What the cached raster (`State::raster`) was drawn from: `draw` keeps
    /// it while this stays the same and draws it again when it changes, so
    /// everything the pixels depend on is in it. The mesh and the uncut faces
    /// are in it by identity, as each is replaced whole (the faces by
    /// `Message::LeftStock`, in the background) and never edited in place.
    fn raster_key(&self, bounds: Rectangle, hovered: Option<usize>) -> String {
        let key = format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{}|{:p}",
            self.camera,
            bounds,
            self.setup.map(|s| &s.p),
            self.operation.map(|o| &o.geometry),
            hovered,
            self.picking,
            (self.mesh.faces.as_ptr())
        );
        let key = format!(
            "{key}|{}|{:?}|{:?}|{}|{:?}|{}|{:?}",
            self.show_paths,
            self.progress,
            self.operation.map(|o| o.strategy),
            self.show_stock,
            self.operation.map(|o| &o.surfaces),
            self.sample,
            self.simulation.map(|s| (s.id.as_str(), s.revision))
        );
        format!(
            "{key}|{:?}|{:?}|{:?}|{:?}|{:?}|{}|{}",
            self.holding,
            self.leftover,
            self.live,
            self.live_stock.map(|(g, v)| (std::ptr::from_ref(g), v)),
            self.uncut.first().map(std::ptr::from_ref),
            self.uncut.len(),
            self.show_uncut
        )
    }
    /// The scene as RGBA pixels, `w` by `h`.
    fn pixels(
        &self,
        p: &Projector,
        bounds: Rectangle,
        hovered: Option<usize>,
    ) -> (u32, u32, Vec<u8>) {
        let ratio = (1600. / bounds.width.max(1.))
            .min(1200. / bounds.height.max(1.))
            .min(1.5);
        let w = (bounds.width * ratio).ceil().max(1.) as u32;
        let h = (bounds.height * ratio).ceil().max(1.) as u32;
        let (rgba, _) = self.render(p, (w, h), ratio, hovered, &[]);
        (w, h, rgba)
    }
    /// The scene drawn into a `w` × `h` frame whose points are the
    /// projector's times `ratio`: RGBA (alpha 0 where nothing was drawn) and
    /// the depth buffer of the opaque things drawn (the part, or the
    /// simulated stock and the tool). `marked` surfaces take the selection
    /// tint and keep it under the stock's box, so a picture shows them
    /// plainly.
    pub(crate) fn render(
        &self,
        p: &Projector,
        (w, h): (u32, u32),
        ratio: f32,
        hovered: Option<usize>,
        marked: &[usize],
    ) -> (Vec<u8>, Vec<f32>) {
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        let mut depths = vec![f32::NEG_INFINITY; (w * h) as usize];
        // The pixels where a marked surface is nearest; empty without any.
        let cells = if marked.is_empty() { 0 } else { w * h };
        let mut plain = vec![false; cells as usize];
        // The nearest-surface buffers of the boxes, shared by all of them.
        let mut solid = SolidBuffers::default();
        // Remaining stock is the simulated solid. Do not overlay the finished
        // reference mesh on its coincident walls; Stock off reveals that model.
        // Only the faces still under stock are drawn then, after the stock.
        let carved = self.simulation.is_none() && self.show_stock && self.live_stock.is_some();
        let behind_stock = (self.simulation.is_some() && self.show_stock) || carved;
        let faces = if behind_stock && !self.show_uncut {
            vec![]
        } else {
            self.transformed()
        };
        let selected_by = |face: &Face| {
            face.feature.is_some_and(|f| {
                self.operation.is_some_and(|o| {
                    if self.sample {
                        self.picking && o.geometry.contains(&f)
                    } else {
                        o.surfaces
                            .as_ref()
                            .is_some_and(|s| s.matches(self.mesh) && s.ids.contains(&f))
                    }
                })
            }) || face.feature.is_some_and(|f| marked.contains(&f))
        };
        let project = |face: &Face| -> Vec<(Point, f32)> {
            face.points
                .iter()
                .map(|v| {
                    let pt = p.p(*v);
                    (Point::new(pt.x * ratio, pt.y * ratio), p.depth(*v))
                })
                .collect()
        };
        // A selected face still under stock keeps its magenta and gets its
        // outline in the selection colour, drawn once the faces are.
        let mut outlines: Vec<Vec<(Point, f32)>> = vec![];
        for (fi, face) in faces.iter().enumerate() {
            if behind_stock {
                break;
            }
            let light = face_light(face);
            let selected = selected_by(face);
            let hover = face.feature.is_some() && face.feature == hovered;
            let kept = !hover && face.feature.is_some_and(|f| marked.contains(&f));
            let uncut = self.show_uncut && self.uncut.get(fi) == Some(&true);
            let color = if hover {
                [242, 190, 93, 255]
            } else if uncut {
                if selected {
                    outlines.push(project(face));
                }
                uncut_color(light)
            } else if selected {
                SELECTED
            } else {
                [
                    (light * 0.80 * 255.) as u8,
                    (light * 0.91 * 255.) as u8,
                    (light * 255.) as u8,
                    255,
                ]
            };
            let v = project(face);
            for i in 1..v.len() - 1 {
                let tri = [v[0], v[i], v[i + 1]];
                let minx = tri
                    .iter()
                    .map(|(p, _)| p.x)
                    .fold(f32::INFINITY, f32::min)
                    .floor()
                    .max(0.) as u32;
                let maxx = tri
                    .iter()
                    .map(|(p, _)| p.x)
                    .fold(f32::NEG_INFINITY, f32::max)
                    .ceil()
                    .clamp(0., w as f32) as u32;
                let miny = tri
                    .iter()
                    .map(|(p, _)| p.y)
                    .fold(f32::INFINITY, f32::min)
                    .floor()
                    .max(0.) as u32;
                let maxy = tri
                    .iter()
                    .map(|(p, _)| p.y)
                    .fold(f32::NEG_INFINITY, f32::max)
                    .ceil()
                    .clamp(0., h as f32) as u32;
                for y in miny..maxy {
                    for x in minx..maxx {
                        if let Some(z) =
                            triangle_depth(tri, Point::new(x as f32 + 0.5, y as f32 + 0.5))
                        {
                            let index = (y * w + x) as usize;
                            if z > depths[index] {
                                depths[index] = z;
                                rgba[index * 4..index * 4 + 4].copy_from_slice(&color);
                                if let Some(plain) = plain.get_mut(index) {
                                    *plain = kept;
                                }
                            }
                        }
                    }
                }
            }
        }
        if self.simulation.is_some() || carved {
            match (self.simulation, self.live_stock) {
                (Some(simulation), _) => {
                    self.raster_simulation(simulation, p, &mut rgba, &mut depths, w, h, ratio)
                }
                (None, Some((grid, _))) => {
                    let scale = self.mesh.display_scale();
                    let mut face = |points: &[[f32; 3]], color: [u8; 4]| {
                        let v: Vec<_> = (points.iter())
                            .map(|v| {
                                let v = v.map(|n| n * scale);
                                let q = p.p(v);
                                (Point::new(q.x * ratio, q.y * ratio), p.depth(v))
                            })
                            .collect();
                        for i in 1..v.len() - 1 {
                            raster_triangle(
                                &mut rgba,
                                &mut depths,
                                w,
                                h,
                                [v[0], v[i], v[i + 1]],
                                color,
                            );
                        }
                    };
                    grid_faces(grid, &mut face);
                }
                (None, None) => {}
            }
            for (fi, face) in faces.iter().enumerate() {
                if !behind_stock || self.uncut.get(fi) != Some(&true) {
                    continue;
                }
                let v = project(face);
                let light = face_light(face);
                for i in 1..v.len() - 1 {
                    raster_triangle(
                        &mut rgba,
                        &mut depths,
                        w,
                        h,
                        [v[0], v[i], v[i + 1]],
                        uncut_color(light),
                    );
                }
                if selected_by(face) {
                    outlines.push(v);
                }
            }
        } else if self.show_stock
            && let Some(setup) = self.setup
            && self.stock_is_known(setup)
            && let Some(stock) = self.resolved_stock()
        {
            let projected = box_vertices(stock.stock.scaled(stock.display_scale)).map(|v| {
                let q = p.p(v);
                (Point::new(q.x * ratio, q.y * ratio), p.depth(v))
            });
            raster_stock(
                &mut rgba, &depths, w, h, projected, ratio, p.scale, &mut solid,
            );
            // Only the planner's pictures mark surfaces (the app passes
            // none, so its view is unchanged): the model asked to see them,
            // so the translucent box must not dim them.
            for (index, _) in plain.iter().enumerate().filter(|(_, kept)| **kept) {
                rgba[index * 4..index * 4 + 4].copy_from_slice(&SELECTED);
            }
        }
        for outline in &outlines {
            for (i, a) in outline.iter().enumerate() {
                let b = outline[(i + 1) % outline.len()];
                raster_depth_line(&mut rgba, &depths, w, h, *a, b, SELECTED, ratio);
            }
        }
        if !self.holding.is_empty() {
            let scale = self.mesh.display_scale();
            for hold in &self.holding {
                let projected = box_vertices(hold.bounds.scaled(scale)).map(|v| {
                    let q = p.p(v);
                    (Point::new(q.x * ratio, q.y * ratio), p.depth(v))
                });
                raster_solid(
                    &HOLDING,
                    &mut rgba,
                    &depths,
                    (w, h),
                    projected,
                    ratio,
                    p.scale,
                    &mut solid,
                );
            }
        }
        if let Some((tip, radius, stickout)) = self.live {
            // The machine's tool, in red, depth-tested like the simulated one.
            // A little wider, so that it shows over the simulated tool when
            // the simulation follows the machine.
            let radius = radius + 0.5;
            let scale = self.mesh.display_scale();
            let project = |v: [f32; 3]| {
                let v = v.map(|n| n * scale);
                let q = p.p(v);
                (Point::new(q.x * ratio, q.y * ratio), p.depth(v))
            };
            let ring = |a: f32, up: f32| {
                project([
                    tip[0] + radius * a.cos(),
                    tip[1] + radius * a.sin(),
                    tip[2] + up,
                ])
            };
            let step = std::f32::consts::TAU / 24.;
            let top = ring(0., stickout);
            for i in 0..24 {
                let (a, b) = (i as f32 * step, (i + 1) as f32 * step);
                let light = 0.75 + 0.25 * (a + 0.6).cos().abs();
                let side = [
                    (214. * light) as u8,
                    (40. * light) as u8,
                    (40. * light) as u8,
                    255,
                ];
                let quad = [
                    ring(a, 0.),
                    ring(b, 0.),
                    ring(b, stickout),
                    ring(a, stickout),
                ];
                for t in [[quad[0], quad[1], quad[2]], [quad[0], quad[2], quad[3]]] {
                    raster_triangle(&mut rgba, &mut depths, w, h, t, side);
                }
                let cap = [top, ring(a, stickout), ring(b, stickout)];
                raster_triangle(&mut rgba, &mut depths, w, h, cap, [214, 40, 40, 255]);
            }
        }
        if self.sample && self.show_paths && self.setup.is_some_and(|s| !s.is_repositioned()) {
            let strategy = self
                .operation
                .map(|o| o.strategy)
                .unwrap_or(Strategy::Adaptive);
            let paths = paths(strategy);
            let mut all = vec![];
            let scaled = |v| {
                let pt = p.p(v);
                Point::new(pt.x * ratio, pt.y * ratio)
            };
            for (i, path) in paths.iter().enumerate() {
                for segment in path.windows(2) {
                    raster_line(
                        &mut rgba,
                        w,
                        h,
                        scaled(segment[0]),
                        scaled(segment[1]),
                        if i % 2 == 0 {
                            [35, 139, 199, 255]
                        } else {
                            [30, 172, 142, 255]
                        },
                        1.6 * ratio,
                    );
                }
                all.extend(path.iter().copied());
            }
            if let Some(progress) = self.progress
                && !all.is_empty()
            {
                let point = all[((all.len() - 1) as f32 * progress.clamp(0., 1.)) as usize];
                let top = [point[0], point[1], point[2] + 30.];
                let cap = [point[0], point[1], point[2] + 45.];
                raster_line(
                    &mut rgba,
                    w,
                    h,
                    scaled(point),
                    scaled(top),
                    [209, 158, 57, 255],
                    9. * ratio,
                );
                raster_line(
                    &mut rgba,
                    w,
                    h,
                    scaled(top),
                    scaled(cap),
                    [80, 98, 115, 255],
                    16. * ratio,
                );
            }
        }
        (rgba, depths)
    }
    #[allow(clippy::too_many_arguments)]
    fn raster_simulation(
        &self,
        sim: &crate::simulation::Simulation,
        p: &Projector,
        rgba: &mut [u8],
        depths: &mut [f32],
        w: u32,
        h: u32,
        ratio: f32,
    ) {
        let scale = sim.plan.display_scale;
        let project = |v: [f32; 3]| {
            let v = v.map(|n| n * scale);
            let q = p.p(v);
            (Point::new(q.x * ratio, q.y * ratio), p.depth(v))
        };
        let mut face = |points: &[[f32; 3]], color: [u8; 4]| {
            let vertices: Vec<_> = points.iter().copied().map(project).collect();
            for i in 1..vertices.len() - 1 {
                raster_triangle(
                    rgba,
                    depths,
                    w,
                    h,
                    [vertices[0], vertices[i], vertices[i + 1]],
                    color,
                );
            }
        };
        if self.show_stock
            && let (Some(volume), Some(current)) = (&sim.volume, sim.frame)
            && let Some(surface) = &sim.stock_surface
        {
            for (points, color) in stock_triangles(volume, surface, current) {
                face(&points, color);
            }
        }
        if self.show_stock && sim.volume.is_none() {
            grid_faces(&sim.stock, &mut face);
        }
        let pos = sim.position;
        // Cylinders use the saved cutter diameter, flute length and stickout.
        for (radius, bottom, top, base) in [
            (
                sim.plan.radius,
                pos[2]
                    + if sim.plan.shape == crate::simulation::CutterShape::Ball {
                        sim.plan.radius
                    } else {
                        0.
                    },
                pos[2] + sim.plan.flute_length,
                [221f32, 158., 50.],
            ),
            (
                sim.plan.radius,
                pos[2] + sim.plan.flute_length,
                pos[2] + sim.plan.tool_length,
                [137., 153., 170.],
            ),
            (
                sim.plan.holder_radius,
                pos[2] + sim.plan.tool_length,
                pos[2] + sim.plan.tool_length + 8.,
                [77., 96., 118.],
            ),
        ] {
            let mut cap = Vec::new();
            for i in 0..24 {
                let angle = i as f32 * std::f32::consts::TAU / 24.;
                let next = (i + 1) as f32 * std::f32::consts::TAU / 24.;
                let a = [pos[0] + radius * angle.cos(), pos[1] + radius * angle.sin()];
                let b = [pos[0] + radius * next.cos(), pos[1] + radius * next.sin()];
                let light = 0.75 + 0.25 * (angle + 0.6).cos().abs();
                face(
                    &[
                        [a[0], a[1], bottom],
                        [b[0], b[1], bottom],
                        [b[0], b[1], top],
                        [a[0], a[1], top],
                    ],
                    [
                        (base[0] * light) as u8,
                        (base[1] * light) as u8,
                        (base[2] * light) as u8,
                        255,
                    ],
                );
                cap.push([a[0], a[1], top]);
            }
            face(&cap, [base[0] as u8, base[1] as u8, base[2] as u8, 255]);
        }
        if sim.plan.shape == crate::simulation::CutterShape::Ball {
            let radius = sim.plan.radius;
            for ring in 0..12 {
                let latitude = ring as f32 * std::f32::consts::FRAC_PI_2 / 12.;
                let next = (ring + 1) as f32 * std::f32::consts::FRAC_PI_2 / 12.;
                let point = |lat: f32, a: f32| {
                    [
                        pos[0] + radius * lat.sin() * a.cos(),
                        pos[1] + radius * lat.sin() * a.sin(),
                        pos[2] + radius * (1. - lat.cos()),
                    ]
                };
                for i in 0..24 {
                    let a = i as f32 * std::f32::consts::TAU / 24.;
                    let b = (i + 1) as f32 * std::f32::consts::TAU / 24.;
                    let light = 0.72 + 0.28 * (a + 0.6).cos().abs();
                    face(
                        &[
                            point(latitude, a),
                            point(latitude, b),
                            point(next, b),
                            point(next, a),
                        ],
                        [
                            (221. * light) as u8,
                            (158. * light) as u8,
                            (50. * light) as u8,
                            255,
                        ],
                    );
                }
            }
        }
        if self.show_paths {
            let last_cut = sim
                .plan
                .moves
                .iter()
                .take(sim.current + 1)
                .rposition(|m| m.cutting_pass && m.start <= sim.seconds);
            for (index, movement) in sim.plan.moves.iter().enumerate().take(sim.current + 1) {
                if movement.start > sim.seconds {
                    break;
                }
                let a = project(movement.from);
                let b = project(movement.at(sim.seconds));
                let feed = movement.motion == crate::simulation::Motion::Feed;
                let pocket_layer = sim.plan.strategy != Strategy::Face
                    && movement.cutting_pass
                    && (sim.plan.strategy == Strategy::Parallel
                        || ((movement.to[2] - pos[2]).abs() < 0.001
                            && (movement.from[2] - pos[2]).abs() < 0.001));
                if feed && !pocket_layer && Some(index) != last_cut && index != sim.current {
                    continue;
                }
                if !feed && sim.seconds >= movement.end {
                    continue;
                }
                raster_depth_line(
                    rgba,
                    depths,
                    w,
                    h,
                    a,
                    b,
                    if feed {
                        [34, 114, 176, 255]
                    } else {
                        [226, 149, 38, 255]
                    },
                    ratio * 0.5,
                );
            }
        }
    }
}
/// The marks drawn over the 3D view: the stock's label, the machine's
/// tool's label, the work zero and the axes. They are a layer of their own
/// above the scene (`view`): within one layer the renderer paints lines
/// before images, so the scene's raster would cover them.
struct Marks<'a>(Viewport<'a>);
impl canvas::Program<Event> for Marks<'_> {
    type State = ();
    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        vec![self.0.marks(renderer, bounds)]
    }
}
impl<'a> Viewport<'a> {
    /// The 3D view, `height` tall: the scene, with its marks above it.
    pub(crate) fn view(self, height: impl Into<Length> + Copy) -> Element<'a, Event> {
        let marks = Marks(self.clone());
        iced::widget::stack![
            canvas(self).width(Fill).height(height),
            canvas(marks).width(Fill).height(height),
        ]
        .width(Fill)
        .height(height)
        .into()
    }
    fn marks(&self, renderer: &Renderer, bounds: Rectangle) -> canvas::Geometry {
        let p = self.projector(bounds);
        let mut frame = Frame::new(renderer, bounds.size());
        if self.show_stock
            && let Some(setup) = self.setup
            && let Some(stock) = self.resolved_stock()
        {
            let d = self
                .simulation
                .map_or(stock.stock, |s| s.plan.stock)
                .dimensions();
            frame.fill_rectangle(
                Point::new(10., 10.),
                iced::Size::new(250., 28.),
                Color::from_rgba8(255, 255, 255, 0.90),
            );
            frame.fill_rectangle(
                Point::new(19., 19.),
                iced::Size::new(9., 9.),
                Color::from_rgb8(185, 128, 41),
            );
            frame.fill_text(Text {
                content: if self.simulation.is_none() && !self.stock_is_known(setup) {
                    "Remaining stock · generate preview".into()
                } else {
                    format!("Stock  {:.1} × {:.1} × {:.1} mm", d[0], d[1], d[2])
                },
                position: Point::new(35., 16.),
                color: Color::from_rgb8(123, 84, 27),
                size: 12.into(),
                ..Text::default()
            });
        }
        // The live tool is in the raster; text is drawn above it.
        if let Some((tip, _, stickout)) = self.live {
            let scale = self.mesh.display_scale();
            let top = p.p([tip[0] * scale, tip[1] * scale, (tip[2] + stickout) * scale]);
            frame.fill_text(Text {
                content: "Machine".into(),
                position: Point::new(top.x + 10., top.y - 14.),
                color: Color::from_rgb8(214, 40, 40),
                size: 12.into(),
                ..Text::default()
            });
        }
        // The fixture pins the setup sits on: they stand in the part's holes,
        // under it, so the scene hides them. Each is drawn over it as its
        // cylinder's outline (the rings at its foot and top and the sides
        // between them), outlined in white and labelled.
        let scale = self.mesh.display_scale();
        let halo = Color::from_rgba8(255, 255, 255, 0.92);
        let steel = Color::from_rgb8(16, 104, 138);
        for (n, pin) in (self.holding.iter())
            .filter_map(|h| Some((h.label.strip_prefix("fixture pin ")?, h.bounds)))
        {
            let c = [
                (pin.min[0] + pin.max[0]) / 2.,
                (pin.min[1] + pin.max[1]) / 2.,
            ];
            let r = (pin.max[0] - pin.min[0]) / 2.;
            let z = [pin.min[2], pin.max[2]].map(|z| z * scale);
            let c = c.map(|v| v * scale);
            pin_outline(
                &mut frame,
                &p,
                c,
                r * scale,
                z,
                steel,
                &format!("Pin {n}"),
                false,
            );
        }
        // The pins being placed in the stock's leftover, under the pointer:
        // green where they fit, red where they do not, with how far they
        // keep from the part and the stock's edge or why not.
        if let Some((pin1, fit)) = &self.pin_aim.ghost {
            let aim = &self.pin_aim;
            let color = match fit {
                Ok(_) => Color::from_rgb8(22, 132, 64),
                Err(_) => Color::from_rgb8(200, 36, 36),
            };
            let z = [aim.z, aim.z + aim.height].map(|z| z * scale);
            let mut label_at = None;
            for (i, d) in aim.offsets.iter().enumerate() {
                let c = [(pin1[0] + d[0]) * scale, (pin1[1] + d[1]) * scale];
                let top = pin_outline(
                    &mut frame,
                    &p,
                    c,
                    aim.diameter / 2. * scale,
                    z,
                    color,
                    &format!("Pin {}", i + 1),
                    true,
                );
                label_at.get_or_insert(top);
            }
            let said = match fit {
                Ok((part, edge)) => format!(
                    "{} mm to the part · {} mm to the edge · click to place",
                    crate::post::exact_mm((part * 10.).floor() / 10.),
                    crate::post::exact_mm((edge * 10.).floor() / 10.)
                ),
                Err(why) => why.clone(),
            };
            if let Some(at) = label_at {
                let width = (said.chars().count() as f32 * 6.2 + 8.).min(bounds.width - 12.);
                let size = iced::Size::new(width, 18.);
                let corner = Point::new(
                    (at.x - width / 2.).clamp(6., (bounds.width - width - 6.).max(6.)),
                    (at.y - 44.).max(6.),
                );
                frame.fill_rectangle(corner, size, halo);
                frame.fill_text(Text {
                    content: said,
                    position: Point::new(corner.x + 4., corner.y + 2.),
                    color,
                    size: 11.into(),
                    ..Text::default()
                });
            }
        }
        // The part's holes offered for the pins, a ring at each opening.
        let amber = Color::from_rgb8(176, 96, 8);
        for (n, c, r) in &self.pin_holes.holes {
            let ring: Vec<Point> = (0..=32)
                .map(|k| {
                    let a = k as f32 * std::f32::consts::TAU / 32.;
                    p.p([
                        (c[0] + r * a.cos()) * scale,
                        (c[1] + r * a.sin()) * scale,
                        c[2] * scale,
                    ])
                })
                .collect();
            for (width, color) in [(4.5, halo), (2., amber)] {
                line(&mut frame, &ring, color, width);
            }
            let right = ring
                .iter()
                .fold(ring[0], |m, q| if q.x > m.x { *q } else { m });
            let size = iced::Size::new(50., 16.);
            let corner = Point::new(right.x + 6., right.y - size.height / 2.);
            frame.fill_rectangle(corner, size, halo);
            frame.fill_text(Text {
                content: format!("Hole {n}"),
                position: Point::new(corner.x + 3., corner.y + 1.),
                color: amber,
                size: 11.into(),
                ..Text::default()
            });
        }
        // The work zero: an X/Y/Z cross where the program's X0 Y0 Z0 is, the
        // same size on the screen at any zoom and outlined in white, so it
        // stands out on the part, the stock and the background alike.
        if let Some(zero) = self.zero() {
            let scale = self.mesh.display_scale();
            let at = |d: [f32; 3]| p.p(std::array::from_fn(|k| (zero[k] + d[k]) * scale));
            let o = at([0.; 3]);
            let halo = Color::from_rgba8(255, 255, 255, 0.92);
            let arms = [
                ([1., 0., 0.], Color::from_rgb8(205, 81, 85)),
                ([0., 1., 0.], Color::from_rgb8(76, 154, 105)),
                ([0., 0., 1.], Color::from_rgb8(67, 125, 196)),
            ]
            .map(|(d, c)| {
                // A unit along the axis, drawn 22 px long when it lies flat
                // on the screen and shorter as it turns towards the viewer.
                let end = at(d.map(|v| v / scale));
                let k = 22. / p.scale;
                (
                    Point::new(o.x + (end.x - o.x) * k, o.y + (end.y - o.y) * k),
                    c,
                )
            });
            for (end, _) in arms {
                line(&mut frame, &[o, end], halo, 5.);
            }
            for (end, c) in arms {
                line(&mut frame, &[o, end], c, 2.5);
            }
            frame.fill(&canvas::Path::circle(o, 5.5), halo);
            frame.fill(&canvas::Path::circle(o, 3.5), Color::from_rgb8(30, 30, 30));
            // The label goes on the side away from the arms, so it covers
            // none of them.
            let (mut ax, mut ay) = arms.iter().fold((0., 0.), |(x, y), (end, _)| {
                (x + end.x - o.x, y + end.y - o.y)
            });
            let n = ax.hypot(ay);
            (ax, ay) = if n < 1. { (0., -1.) } else { (ax / n, ay / n) };
            let size = iced::Size::new(56., 16.);
            let centre = Point::new(o.x - ax * 40., o.y - ay * 22.);
            let corner = Point::new(centre.x - size.width / 2., centre.y - size.height / 2.);
            frame.fill_rectangle(corner, size, halo);
            let label = Point::new(corner.x + 3., corner.y + 1.);
            frame.fill_text(Text {
                content: "X0 Y0 Z0".into(),
                position: label,
                color: Color::from_rgb8(30, 30, 30),
                size: 11.into(),
                ..Text::default()
            });
        }
        let origin = Point::new(45., bounds.height - 45.);
        let o = p.p([0., 0., 0.]);
        for (v, c, label) in [
            ([24., 0., 0.], Color::from_rgb8(205, 81, 85), "X"),
            ([0., 24., 0.], Color::from_rgb8(76, 154, 105), "Y"),
            ([0., 0., 24.], Color::from_rgb8(67, 125, 196), "Z"),
        ] {
            let end = p.p(v);
            let end = Point::new(
                origin.x + (end.x - o.x) / p.scale * 1.1,
                origin.y + (end.y - o.y) / p.scale * 1.1,
            );
            line(&mut frame, &[origin, end], c, 2.);
            frame.fill_text(Text {
                content: label.into(),
                position: Point::new(end.x + 3., end.y - 7.),
                color: c,
                size: 11.into(),
                ..Text::default()
            });
        }
        frame.into_geometry()
    }
}
impl canvas::Program<Event> for Viewport<'_> {
    type State = State;
    fn update(
        &self,
        state: &mut State,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Event>> {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(b))
                if matches!(
                    b,
                    mouse::Button::Left | mouse::Button::Right | mouse::Button::Middle
                ) =>
            {
                if let Some(p) = cursor.position_in(bounds) {
                    if self.pin_aim.on
                        && *b == mouse::Button::Left
                        && let Some(grab) = self.grabbed(bounds, p)
                    {
                        state.grab = Some(grab);
                        return Some(Action::capture());
                    }
                    state.drag = Some((p, p, *b));
                    state.moved = false;
                    return Some(Action::capture());
                }
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. })
                if self.pin_aim.on && state.drag.is_none() =>
            {
                let at = cursor
                    .position_in(bounds)
                    .and_then(|p| self.aimed(bounds, p));
                let grab = state.grab.unwrap_or_default();
                if at.is_some() || state.aiming {
                    state.aiming = at.is_some();
                    let pin1 = at.map(|q| [q[0] - grab[0], q[1] - grab[1]]);
                    return Some(Action::publish(Event::AimPins(pin1)).and_capture());
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(_)) if state.grab.is_some() => {
                let grab = state.grab.take().unwrap_or_default();
                if let Some(q) = cursor
                    .position_in(bounds)
                    .and_then(|p| self.aimed(bounds, p))
                {
                    let pin1 = [q[0] - grab[0], q[1] - grab[1]];
                    return Some(Action::publish(Event::PlacePins(pin1)).and_capture());
                }
                return Some(Action::capture());
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let Some((start, last, b)) = state.drag {
                    if let Some(p) = cursor
                        .position()
                        .map(|p| Point::new(p.x - bounds.x, p.y - bounds.y))
                    {
                        let dx = p.x - last.x;
                        let dy = p.y - last.y;
                        state.moved |= p.distance(start) > 4.;
                        state.drag = Some((start, p, b));
                        if state.moved {
                            return Some(
                                Action::publish(if b == mouse::Button::Left {
                                    Event::Orbit(dx * 0.008, dy * 0.008)
                                } else {
                                    Event::Pan(dx, dy)
                                })
                                .and_capture(),
                            );
                        }
                    }
                } else if cursor.is_over(bounds) {
                    return Some(Action::request_redraw());
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(_)) => {
                if let Some((_, _, button)) = state.drag.take() {
                    if !state.moved
                        && button == mouse::Button::Left
                        && self.pin_aim.on
                        && let Some(q) = cursor
                            .position_in(bounds)
                            .and_then(|p| self.aimed(bounds, p))
                    {
                        return Some(Action::publish(Event::PlacePins(q)).and_capture());
                    }
                    if !state.moved
                        && button == mouse::Button::Left
                        && let Some(p) = cursor.position_in(bounds)
                        && let Some(f) = self.picked(bounds, p)
                    {
                        return Some(
                            Action::publish(if self.sample {
                                Event::Pick(f)
                            } else {
                                Event::PickSurface(f)
                            })
                            .and_capture(),
                        );
                    }
                    return Some(Action::capture());
                }
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta })
                if cursor.is_over(bounds) =>
            {
                let dy = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y * 0.12,
                    mouse::ScrollDelta::Pixels { y, .. } => *y * 0.004,
                };
                return Some(Action::publish(Event::Zoom(dy)).and_capture());
            }
            _ => {}
        }
        None
    }
    fn draw(
        &self,
        state: &State,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let p = self.projector(bounds);
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            Color::from_rgb8(239, 243, 247),
        );
        for i in -10..=10 {
            let n = i as f32 * 15.;
            line(
                &mut frame,
                &[p.p([n, -150., 0.]), p.p([n, 150., 0.])],
                Color::from_rgba8(133, 154, 171, 0.18),
                0.6,
            );
            line(
                &mut frame,
                &[p.p([-150., n, 0.]), p.p([150., n, 0.])],
                Color::from_rgba8(133, 154, 171, 0.18),
                0.6,
            );
        }
        let shadow = polygon(
            &(0..48)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::TAU / 48.;
                    let c = p.p([0., 0., 0.]);
                    Point::new(c.x + a.cos() * 76. * p.scale, c.y + a.sin() * 26. * p.scale)
                })
                .collect::<Vec<_>>(),
        );
        frame.fill(&shadow, Color::from_rgba8(68, 88, 107, 0.09));
        let hovered = cursor
            .position_in(bounds)
            .filter(|_| state.drag.is_none())
            .and_then(|pt| self.picked(bounds, pt));
        let background = frame.into_geometry();
        let key = self.raster_key(bounds, hovered);
        let mut cached = state.raster.borrow_mut();
        if cached.as_ref().is_none_or(|(previous, _)| *previous != key) {
            *cached = Some((key, self.raster(&p, bounds, hovered)));
        }
        let mut model_frame = Frame::new(renderer, bounds.size());
        cached
            .as_ref()
            .unwrap()
            .1
            .draw(&mut model_frame, bounds.size());
        vec![background, model_frame.into_geometry()]
    }
    fn mouse_interaction(
        &self,
        state: &State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if state.drag.is_some() {
            mouse::Interaction::Grabbing
        } else if cursor.is_over(bounds) {
            if self.picking {
                mouse::Interaction::Crosshair
            } else {
                mouse::Interaction::Grab
            }
        } else {
            mouse::Interaction::default()
        }
    }
}
fn normal(p: &[[f32; 3]]) -> [f32; 3] {
    if p.len() < 3 {
        return [0., 0., 1.];
    }
    let a = std::array::from_fn::<_, 3, _>(|i| p[1][i] - p[0][i]);
    let b = std::array::from_fn::<_, 3, _>(|i| p[2][i] - p[0][i]);
    let n = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let l = (n.iter().map(|x| x * x).sum::<f32>()).sqrt().max(0.001);
    n.map(|x| x / l)
}
fn polygon(points: &[Point]) -> Path {
    Path::new(|b| {
        if let Some(p) = points.first() {
            b.move_to(*p);
            for p in &points[1..] {
                b.line_to(*p)
            }
            b.close()
        }
    })
}
/// A fixture pin standing at `c` (display units) from `z[0]` up to `z[1]`,
/// `r` round, drawn over the scene as its cylinder's outline (the rings at
/// its foot and top and the sides between them) in `color`, outlined in
/// white and labelled `label` on its right, or on its `left`: where its
/// top's centre is on the screen.
#[allow(clippy::too_many_arguments)]
fn pin_outline(
    frame: &mut Frame,
    p: &Projector,
    c: [f32; 2],
    r: f32,
    z: [f32; 2],
    color: Color,
    label: &str,
    left: bool,
) -> Point {
    let halo = Color::from_rgba8(255, 255, 255, 0.92);
    let ring = |z: f32| -> Vec<Point> {
        (0..=32)
            .map(|i| {
                let a = i as f32 * std::f32::consts::TAU / 32.;
                p.p([c[0] + r * a.cos(), c[1] + r * a.sin(), z])
            })
            .collect()
    };
    let (foot, top) = (ring(z[0]), ring(z[1]));
    // The sides: the ring points furthest left and right on screen.
    let (left_most, right) = (0..foot.len()).fold((0, 0), |(l, r), i| {
        (
            if foot[i].x < foot[l].x { i } else { l },
            if foot[i].x > foot[r].x { i } else { r },
        )
    });
    let sides = [[foot[left_most], top[left_most]], [foot[right], top[right]]];
    for (width, color) in [(4.5, halo), (2., color)] {
        line(frame, &foot, color, width);
        line(frame, &top, color, width);
        for side in &sides {
            line(frame, side, color, width);
        }
    }
    let size = iced::Size::new(40., 16.);
    let corner = if left {
        let at = top[left_most];
        Point::new(at.x - 6. - size.width, at.y - size.height / 2.)
    } else {
        let at = top[right];
        Point::new(at.x + 6., at.y - size.height / 2.)
    };
    frame.fill_rectangle(corner, size, halo);
    frame.fill_text(Text {
        content: label.into(),
        position: Point::new(corner.x + 3., corner.y + 1.),
        color,
        size: 11.into(),
        ..Text::default()
    });
    p.p([c[0], c[1], z[1]])
}
fn line(frame: &mut Frame, points: &[Point], c: Color, w: f32) {
    let path = Path::new(|b| {
        if let Some(p) = points.first() {
            b.move_to(*p);
            for p in &points[1..] {
                b.line_to(*p)
            }
        }
    });
    frame.stroke(&path, Stroke::default().with_color(c).with_width(w))
}
/// The colour of the original block's faces of simulated stock.
pub(crate) const BLOCK: [f32; 3] = [219., 183., 113.];
/// The colour of the faces of simulated stock that were cut.
const CUT: [f32; 3] = [103., 181., 151.];
/// The colour of jaws and clamps in still pictures.
pub(crate) const JAWS: [f32; 3] = [176., 176., 176.];
/// `base` shaded as the view shades a face of simulated stock: by its normal.
fn shade(points: &[[f32; 3]], base: [f32; 3]) -> [u8; 4] {
    let n = normal(points);
    let light = 0.58 + 0.29 * n[2].abs() + 0.13 * n[0].abs();
    [
        (base[0] * light) as u8,
        (base[1] * light) as u8,
        (base[2] * light) as u8,
        255,
    ]
}
/// The triangles of `surface`, the simulated stock of `volume`, in the frame
/// `current` (mm), each with the colour the view gives it.
pub(crate) fn stock_triangles<'a>(
    volume: &'a crate::volume::Volume,
    surface: &'a crate::stock_surface::SurfaceMesh,
    current: crate::volume::Frame,
) -> impl Iterator<Item = ([[f32; 3]; 3], [u8; 4])> + 'a {
    surface.triangles.iter().map(move |triangle| {
        let source = triangle.map(|i| surface.vertices[i]);
        let points = source.map(|p| volume.frame.transfer(p, current));
        let outer = (0..3).any(|k| {
            source
                .iter()
                .all(|p| (p[k] - volume.bounds.min[k]).abs() < 0.01)
                || source
                    .iter()
                    .all(|p| (p[k] - volume.bounds.max[k]).abs() < 0.01)
        });
        (points, shade(&points, if outer { BLOCK } else { CUT }))
    })
}
/// The faces of a box `b` (mm), two triangles each, in `base` shaded as the
/// view shades stock.
pub(crate) fn box_triangles(
    b: crate::mesh::Bounds,
    base: [f32; 3],
) -> Vec<([[f32; 3]; 3], [u8; 4])> {
    let v = box_vertices(b);
    [
        [0, 1, 2, 3],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 0, 4, 7],
    ]
    .into_iter()
    .flat_map(|[a, b, c, d]| [[v[a], v[b], v[c]], [v[a], v[c], v[d]]])
    .map(|t| (t, shade(&t, base)))
    .collect()
}
/// The pixel size of the setup browser's stock pictures: twice the size they
/// are shown at, for smooth edges.
pub(crate) const PICTURE: (u32, u32) = (128, 96);
/// The pixel size of the Tool runs panel's pictures of the stock after each
/// operation, twice the size they are shown at.
pub(crate) const OP_PICTURE: (u32, u32) = (320, 240);
/// A still picture of stock, `triangles` in a setup's frame, seen as the 3D
/// view first shows a setup and filling the frame; clear around it; (w, h)
/// px. None without a triangle.
pub(crate) fn stock_picture(
    triangles: &[([[f32; 3]; 3], [u8; 4])],
    (w, h): (u32, u32),
) -> Option<iced::widget::image::Handle> {
    let mut all = crate::mesh::Bounds {
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
    };
    for p in triangles.iter().flat_map(|(t, _)| t) {
        all.min = std::array::from_fn(|k| all.min[k].min(p[k]));
        all.max = std::array::from_fn(|k| all.max[k].max(p[k]));
    }
    if !all.min.iter().chain(&all.max).all(|v| v.is_finite()) {
        return None;
    }
    let p = Projector::filling(Camera::default(), (w as f32, h as f32), &[all], 0.9);
    let mut rgba = vec![0; (w * h * 4) as usize];
    let mut depths = vec![f32::NEG_INFINITY; (w * h) as usize];
    for (t, color) in triangles {
        let vertices = t.map(|v| (p.p(v), p.depth(v)));
        raster_triangle(&mut rgba, &mut depths, w, h, vertices, *color);
    }
    Some(iced::widget::image::Handle::from_rgba(w, h, rgba))
}
fn box_vertices(bounds: crate::mesh::Bounds) -> [[f32; 3]; 8] {
    let [x, y, z] = bounds.min;
    let [xx, yy, zz] = bounds.max;
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
}
fn alpha_over(pixel: &mut [u8], color: [u8; 3], alpha: f32) {
    let destination = pixel[3] as f32 / 255.;
    let output = alpha + destination * (1. - alpha);
    if output <= 0. {
        return;
    }
    for i in 0..3 {
        pixel[i] = ((color[i] as f32 * alpha + pixel[i] as f32 * destination * (1. - alpha))
            / output)
            .round() as u8;
    }
    pixel[3] = (output * 255.).round() as u8;
}
/// The tint of an operation's surfaces and of marked ones.
const SELECTED: [u8; 4] = [92, 168, 223, 255];
/// The colour of the part's faces the whole job leaves under stock.
const UNCUT: [u8; 3] = [214, 72, 166];
/// `UNCUT` as a face of brightness `light` is drawn.
fn uncut_color(light: f32) -> [u8; 4] {
    let shade = |c: u8| (c as f32 * (light + 0.1).min(1.)) as u8;
    [shade(UNCUT[0]), shade(UNCUT[1]), shade(UNCUT[2]), 255]
}
/// How bright the part's `face` is drawn, from its normal and tone.
fn face_light(face: &Face) -> f32 {
    let normal = normal(&face.points);
    (0.42 + normal[2].abs() * 0.27 + normal[0].abs() * 0.12 + face.tone * 0.13).clamp(0.2, 0.94)
}
/// How a translucent box is painted: the tint of its top and bottom, of its
/// X sides and of its Y sides, its edges (seen, and behind something), and
/// how opaque it is over empty space and over the part.
struct Look {
    faces: [[u8; 3]; 3],
    edge: [u8; 3],
    hidden_edge: [u8; 3],
    open: f32,
    behind: f32,
}
const STOCK: Look = Look {
    faces: [[222, 177, 91], [194, 141, 58], [210, 157, 65]],
    edge: [163, 108, 25],
    hidden_edge: [162, 129, 77],
    open: 0.30,
    behind: 0.12,
};
/// Jaws and clamps: a neutral grey, more opaque than the stock.
const HOLDING: Look = Look {
    faces: [[176, 176, 176], [138, 138, 138], [158, 158, 158]],
    edge: [84, 84, 84],
    hidden_edge: [118, 118, 118],
    open: 0.5,
    behind: 0.3,
};
/// What `raster_solid` finds nearest at each pixel: the depth and the face's
/// tone. One is kept for all the boxes of a frame and reset for each.
#[derive(Default)]
struct SolidBuffers {
    near: Vec<f32>,
    tones: Vec<u8>,
}
impl SolidBuffers {
    fn reset(&mut self, cells: usize) {
        self.near.clear();
        self.near.resize(cells, f32::NEG_INFINITY);
        self.tones.clear();
        self.tones.resize(cells, 0);
    }
}
/// The stock's box: `raster_solid` in the stock's colors.
#[allow(clippy::too_many_arguments)]
fn raster_stock(
    pixels: &mut [u8],
    part_depth: &[f32],
    w: u32,
    h: u32,
    vertices: [(Point, f32); 8],
    ratio: f32,
    scale: f32,
    buffers: &mut SolidBuffers,
) {
    raster_solid(
        &STOCK,
        pixels,
        part_depth,
        (w, h),
        vertices,
        ratio,
        scale,
        buffers,
    );
}
/// Composite only the nearest surface of a box; use both surfaces and part
/// depth to distinguish visible edges from dashed hidden edges.
#[allow(clippy::too_many_arguments)]
fn raster_solid(
    look: &Look,
    pixels: &mut [u8],
    part_depth: &[f32],
    (w, h): (u32, u32),
    vertices: [(Point, f32); 8],
    ratio: f32,
    scale: f32,
    buffers: &mut SolidBuffers,
) {
    buffers.reset((w * h) as usize);
    let SolidBuffers { near, tones } = buffers;
    for (tone, face) in [
        [0, 1, 2, 3],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 0, 4, 7],
    ]
    .iter()
    .enumerate()
    {
        for tri in [[face[0], face[1], face[2]], [face[0], face[2], face[3]]] {
            let tri = tri.map(|i| vertices[i]);
            let minx = tri
                .iter()
                .map(|(p, _)| p.x)
                .fold(f32::INFINITY, f32::min)
                .floor()
                .clamp(0., w as f32) as u32;
            let maxx = tri
                .iter()
                .map(|(p, _)| p.x)
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil()
                .clamp(0., w as f32) as u32;
            let miny = tri
                .iter()
                .map(|(p, _)| p.y)
                .fold(f32::INFINITY, f32::min)
                .floor()
                .clamp(0., h as f32) as u32;
            let maxy = tri
                .iter()
                .map(|(p, _)| p.y)
                .fold(f32::NEG_INFINITY, f32::max)
                .ceil()
                .clamp(0., h as f32) as u32;
            for y in miny..maxy {
                for x in minx..maxx {
                    let index = (y * w + x) as usize;
                    if let Some(z) = triangle_depth(tri, Point::new(x as f32 + 0.5, y as f32 + 0.5))
                        && z > near[index]
                    {
                        near[index] = z;
                        tones[index] = tone as u8;
                    }
                }
            }
        }
    }
    let tolerance = 1.5 / (scale * ratio).max(0.01);
    for (index, z) in near.iter().enumerate() {
        if z.is_finite() && *z >= part_depth[index] - 0.001 {
            let tint = match tones[index] {
                0 | 1 => look.faces[0],
                2 | 4 => look.faces[1],
                _ => look.faces[2],
            };
            let alpha = if part_depth[index].is_finite() {
                look.behind
            } else {
                look.open
            };
            alpha_over(&mut pixels[index * 4..index * 4 + 4], tint, alpha);
        }
    }
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
        let (a, za) = vertices[a];
        let (b, zb) = vertices[b];
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let length_sq = dx * dx + dy * dy;
        if length_sq < 0.001 {
            continue;
        }
        let length = length_sq.sqrt();
        let radius = 0.95 * ratio;
        let minx = (a.x.min(b.x) - radius - 1.).floor().clamp(0., w as f32) as u32;
        let maxx = (a.x.max(b.x) + radius + 1.).ceil().clamp(0., w as f32) as u32;
        let miny = (a.y.min(b.y) - radius - 1.).floor().clamp(0., h as f32) as u32;
        let maxy = (a.y.max(b.y) + radius + 1.).ceil().clamp(0., h as f32) as u32;
        for y in miny..maxy {
            for x in minx..maxx {
                let q = Point::new(x as f32 + 0.5, y as f32 + 0.5);
                let t = (((q.x - a.x) * dx + (q.y - a.y) * dy) / length_sq).clamp(0., 1.);
                let closest = Point::new(a.x + t * dx, a.y + t * dy);
                let index = (y * w + x) as usize;
                let z = za + t * (zb - za);
                let hidden = z + tolerance < near[index].max(part_depth[index]);
                if hidden && (t * length) % (10. * ratio) > 4. * ratio {
                    continue;
                }
                let thickness = if hidden { radius * 0.55 } else { radius };
                let coverage = (thickness + 0.5 - q.distance(closest)).clamp(0., 1.);
                if coverage > 0. {
                    alpha_over(
                        &mut pixels[index * 4..index * 4 + 4],
                        if hidden { look.hidden_edge } else { look.edge },
                        coverage * if hidden { 0.34 } else { 0.95 },
                    );
                }
            }
        }
    }
}
fn raster_triangle(
    pixels: &mut [u8],
    depths: &mut [f32],
    w: u32,
    h: u32,
    vertices: [(Point, f32); 3],
    color: [u8; 4],
) {
    let minx = vertices
        .iter()
        .map(|v| v.0.x)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .clamp(0., w as f32) as u32;
    let maxx = vertices
        .iter()
        .map(|v| v.0.x)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .clamp(0., w as f32) as u32;
    let miny = vertices
        .iter()
        .map(|v| v.0.y)
        .fold(f32::INFINITY, f32::min)
        .floor()
        .clamp(0., h as f32) as u32;
    let maxy = vertices
        .iter()
        .map(|v| v.0.y)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .clamp(0., h as f32) as u32;
    for y in miny..maxy {
        for x in minx..maxx {
            let index = (y * w + x) as usize;
            if let Some(z) = triangle_depth(vertices, Point::new(x as f32 + 0.5, y as f32 + 0.5))
                && z >= depths[index]
            {
                depths[index] = z;
                pixels[index * 4..index * 4 + 4].copy_from_slice(&color);
            }
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn raster_depth_line(
    pixels: &mut [u8],
    depths: &[f32],
    w: u32,
    h: u32,
    a: (Point, f32),
    b: (Point, f32),
    color: [u8; 4],
    ratio: f32,
) {
    let steps = (a.0.distance(b.0).ceil() as usize).clamp(1, 10000);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = a.0.x + (b.0.x - a.0.x) * t;
        let y = a.0.y + (b.0.y - a.0.y) * t;
        let z = a.1 + (b.1 - a.1) * t;
        for yy in (y - ratio).floor().clamp(0., h as f32) as u32
            ..(y + ratio).ceil().clamp(0., h as f32) as u32
        {
            for xx in (x - ratio).floor().clamp(0., w as f32) as u32
                ..(x + ratio).ceil().clamp(0., w as f32) as u32
            {
                let index = (yy * w + xx) as usize;
                if z + 0.05 >= depths[index] {
                    pixels[index * 4..index * 4 + 4].copy_from_slice(&color);
                }
            }
        }
    }
}
fn raster_line(pixels: &mut [u8], w: u32, h: u32, a: Point, b: Point, color: [u8; 4], width: f32) {
    let steps = (a.distance(b).ceil() as usize).clamp(1, 10000);
    let radius = (width / 2.).max(0.7);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = a.x + (b.x - a.x) * t;
        let y = a.y + (b.y - a.y) * t;
        for yy in
            ((y - radius).floor().max(0.) as u32)..((y + radius).ceil().clamp(0., h as f32) as u32)
        {
            for xx in ((x - radius).floor().max(0.) as u32)
                ..((x + radius).ceil().clamp(0., w as f32) as u32)
            {
                if (xx as f32 + 0.5 - x).powi(2) + (yy as f32 + 0.5 - y).powi(2) <= radius * radius
                {
                    let index = ((yy * w + xx) * 4) as usize;
                    pixels[index..index + 4].copy_from_slice(&color);
                }
            }
        }
    }
}
fn triangle_depth(v: [(Point, f32); 3], p: Point) -> Option<f32> {
    let [a, b, c] = v;
    let denom = (b.0.y - c.0.y) * (a.0.x - c.0.x) + (c.0.x - b.0.x) * (a.0.y - c.0.y);
    if denom.abs() < 0.00001 {
        return None;
    }
    let u = ((b.0.y - c.0.y) * (p.x - c.0.x) + (c.0.x - b.0.x) * (p.y - c.0.y)) / denom;
    let v = ((c.0.y - a.0.y) * (p.x - c.0.x) + (a.0.x - c.0.x) * (p.y - c.0.y)) / denom;
    let w = 1. - u - v;
    if u >= -0.00001 && v >= -0.00001 && w >= -0.00001 {
        Some(u * a.1 + v * b.1 + w * c.1)
    } else {
        None
    }
}
fn paths(s: Strategy) -> Vec<Vec<[f32; 3]>> {
    match s {
        Strategy::Face | Strategy::Indexed | Strategy::Parallel => (-4..=4)
            .map(|i| vec![[-65., i as f32 * 10., 33.], [65., i as f32 * 10., 33.]])
            .collect(),
        Strategy::Drill => [(-57., -34.), (57., -34.), (57., 34.), (-57., 34.)]
            .iter()
            .map(|(x, y)| vec![[*x, *y, 40.], [*x, *y, 8.], [*x, *y, 40.]])
            .collect(),
        _ => (0..7)
            .map(|i| {
                let w = 42. - i as f32 * 4.;
                let h = 21. - i as f32 * 2.;
                let z = 14. + i as f32 * 0.3;
                vec![[-w, -h, z], [w, -h, z], [w, h, z], [-w, h, z], [-w, -h, z]]
            })
            .collect(),
    }
}
/// The faces of the stock grid `g` (plan mm): its top, cut cells green; the
/// steps between cells of different heights, green as they were cut; its
/// sides and bottom in the stock's colours.
fn grid_faces(g: &crate::simulation::StockGrid, face: &mut impl FnMut(&[[f32; 3]], [u8; 4])) {
    for y in 0..g.ny {
        for x in 0..g.nx {
            let z = g.heights[y * g.nx + x];
            let a = g.bounds.min[0] + x as f32 * g.dx;
            let b = a + g.dx;
            let c = g.bounds.min[1] + y as f32 * g.dy;
            let d = c + g.dy;
            let cut = z < g.bounds.max[2] - 0.0001;
            face(
                &[[a, c, z], [b, c, z], [b, d, z], [a, d, z]],
                if cut {
                    [103, 181, 151, 255]
                } else {
                    [219, 183, 113, 255]
                },
            );
            for (adjacent, edge, tone, outer) in [
                (
                    if x == 0 {
                        g.bounds.min[2]
                    } else {
                        g.heights[y * g.nx + x - 1]
                    },
                    [[a, c, z], [a, d, z]],
                    0,
                    x == 0,
                ),
                (
                    if x + 1 == g.nx {
                        g.bounds.min[2]
                    } else {
                        g.heights[y * g.nx + x + 1]
                    },
                    [[b, c, z], [b, d, z]],
                    1,
                    x + 1 == g.nx,
                ),
                (
                    if y == 0 {
                        g.bounds.min[2]
                    } else {
                        g.heights[(y - 1) * g.nx + x]
                    },
                    [[a, c, z], [b, c, z]],
                    1,
                    y == 0,
                ),
                (
                    if y + 1 == g.ny {
                        g.bounds.min[2]
                    } else {
                        g.heights[(y + 1) * g.nx + x]
                    },
                    [[a, d, z], [b, d, z]],
                    0,
                    y + 1 == g.ny,
                ),
            ] {
                if adjacent < z - 0.00001 {
                    face(
                        &[
                            edge[0],
                            edge[1],
                            [edge[1][0], edge[1][1], adjacent],
                            [edge[0][0], edge[0][1], adjacent],
                        ],
                        // A step between cells was cut; the block's own
                        // sides are stock under any cut on top.
                        if !outer {
                            if tone == 0 {
                                [70, 135, 115, 255]
                            } else {
                                [86, 155, 129, 255]
                            }
                        } else if tone == 0 {
                            [157, 127, 78, 255]
                        } else {
                            [183, 149, 88, 255]
                        },
                    );
                }
            }
        }
    }
    let z = g.bounds.min[2];
    face(
        &[
            [g.bounds.min[0], g.bounds.min[1], z],
            [g.bounds.max[0], g.bounds.min[1], z],
            [g.bounds.max[0], g.bounds.max[1], z],
            [g.bounds.min[0], g.bounds.max[1], z],
        ],
        [148, 121, 76, 255],
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    /// FNV-1a over the pixels, to pin the drawing across the refactor.
    fn digest(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf29ce484222325, |h, b| {
            (h ^ *b as u64).wrapping_mul(0x100000001b3)
        })
    }
    #[test]
    fn the_viewport_draws_the_same_pixels_after_the_split() {
        let (doc, mesh) = crate::post::fixture::job();
        let setup = doc.setups.first();
        let view = Viewport {
            setup,
            operation: None,
            mesh: &mesh,
            simulation: None,
            sample: false,
            camera: Camera::default(),
            picking: false,
            pin_holes: Default::default(),
            pin_aim: Default::default(),
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            leftover: None,
            live: None,
            live_stock: None,
            uncut: &[],
            show_uncut: true,
        };
        let bounds = Rectangle {
            x: 0.,
            y: 0.,
            width: 400.,
            height: 300.,
        };
        let p = view.projector(bounds);
        let (w, h, rgba) = view.pixels(&p, bounds, None);
        assert_eq!((w, h), (600, 450));
        assert_eq!(digest(&rgba), 8741738974532346260);
        // Faces the job leaves under stock are magenta.
        let magenta = |rgba: &[u8]| {
            rgba.chunks(4)
                .filter(|px| px[0] > px[1].saturating_add(60) && px[2] > px[1].saturating_add(40))
                .count()
        };
        let bare = Viewport {
            show_stock: false,
            ..view
        };
        assert_eq!(magenta(&bare.pixels(&p, bounds, None).2), 0);
        let all = vec![true; mesh.faces.len()];
        let uncut = Viewport {
            uncut: &all,
            ..bare
        };
        let (_, _, rgba) = uncut.pixels(&p, bounds, None);
        assert!(magenta(&rgba) > 10_000, "{}", magenta(&rgba));
    }
    /// The pixels of `view` that are magenta, as a face under stock is drawn.
    fn magenta(view: &Viewport, size: (u32, u32)) -> usize {
        let p = Projector::fit(
            view.camera,
            (size.0 as f32, size.1 as f32),
            view.scene_bounds(),
        );
        let (rgba, _) = view.render(&p, size, 1., None, &[]);
        rgba.chunks(4)
            .filter(|px| px[0] > px[1].saturating_add(60) && px[2] > px[1].saturating_add(40))
            .count()
    }
    #[test]
    fn faces_under_stock_are_magenta_over_the_simulated_stock_when_shown() {
        let (doc, mesh, report) = crate::proposal::tests::block();
        let good = crate::proposal::tests::good(&report);
        let job = crate::proposal::build(&doc, &mesh, &report, &good).unwrap();
        let sim =
            crate::planner::view_after(&job, &mesh, 0, 0, crate::stock_surface::Detail::Draft)
                .unwrap();
        let all = vec![true; mesh.faces.len()];
        let view = Viewport {
            setup: job.setups.first(),
            operation: None,
            mesh: &mesh,
            simulation: Some(&sim),
            sample: false,
            camera: Camera::default(),
            picking: false,
            pin_holes: Default::default(),
            pin_aim: Default::default(),
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            leftover: None,
            live: None,
            live_stock: None,
            uncut: &all,
            show_uncut: true,
        };
        let size = (300, 200);
        // The part is not drawn over the simulated stock, but its faces still
        // under stock are, where the stock does not hide them.
        let shown = magenta(&view, size);
        assert!(shown > 0, "{shown}");
        let bounds = Rectangle {
            x: 0.,
            y: 0.,
            width: 300.,
            height: 200.,
        };
        let on_key = view.raster_key(bounds, None);
        let off = Viewport {
            show_uncut: false,
            ..view
        };
        assert_eq!(magenta(&off, size), 0);
        // The toggle is in the key the raster is cached under.
        assert_ne!(on_key, off.raster_key(bounds, None));
        // Nothing marked uncut: nothing magenta either.
        let none = Viewport {
            uncut: &[],
            show_uncut: true,
            ..off
        };
        assert_eq!(magenta(&none, size), 0);
    }
    #[test]
    fn a_selected_face_under_stock_keeps_its_magenta_and_is_outlined() {
        let (doc, mesh) = crate::post::fixture::job();
        let all = vec![true; mesh.faces.len()];
        let view = Viewport {
            setup: doc.setups.first(),
            operation: None,
            mesh: &mesh,
            simulation: None,
            sample: false,
            camera: Camera::default(),
            picking: false,
            pin_holes: Default::default(),
            pin_aim: Default::default(),
            show_stock: false,
            show_paths: false,
            progress: None,
            holding: vec![],
            leftover: None,
            live: None,
            live_stock: None,
            uncut: &all,
            show_uncut: true,
        };
        let size = (300, 200);
        let p = Projector::fit(view.camera, (300., 200.), view.scene_bounds());
        let features: Vec<usize> = mesh.faces.iter().filter_map(|f| f.feature).collect();
        assert!(!features.is_empty());
        let (plain, _) = view.render(&p, size, 1., None, &[]);
        let (marked, _) = view.render(&p, size, 1., None, &features);
        let tinted = |rgba: &[u8]| rgba.chunks(4).filter(|px| **px == SELECTED).count();
        // Selected, the faces are still magenta, with a line round each in the
        // selection colour; they are no longer filled with it.
        assert_eq!(tinted(&plain), 0);
        assert!(tinted(&marked) > 0);
        assert!(magenta(&view, size) > 0);
        let fill = |rgba: &[u8]| {
            rgba.chunks(4)
                .filter(|px| px[0] > px[1].saturating_add(60) && px[2] > px[1].saturating_add(40))
                .count()
        };
        // The lines take some of it; most is still magenta.
        assert!(
            fill(&marked) * 4 > fill(&plain),
            "{} {}",
            fill(&marked),
            fill(&plain)
        );
        // Without the toggle the selection fills the face, as it did.
        let off = Viewport {
            show_uncut: false,
            ..view
        };
        let (filled, _) = off.render(&p, size, 1., None, &features);
        assert!(tinted(&filled) > tinted(&marked));
    }
    #[test]
    fn the_raster_is_drawn_again_when_the_uncut_result_changes() {
        let (doc, mesh) = crate::post::fixture::job();
        let bounds = Rectangle {
            x: 0.,
            y: 0.,
            width: 400.,
            height: 300.,
        };
        let key = |uncut: &[bool]| {
            Viewport {
                setup: doc.setups.first(),
                operation: None,
                mesh: &mesh,
                simulation: None,
                sample: false,
                camera: Camera::default(),
                picking: false,
                pin_holes: Default::default(),
                pin_aim: Default::default(),
                show_stock: true,
                show_paths: false,
                progress: None,
                holding: vec![],
                leftover: None,
                live: None,
                live_stock: None,
                uncut,
                show_uncut: true,
            }
            .raster_key(bounds, None)
        };
        let all = vec![true; mesh.faces.len()];
        // The same inputs keep the raster.
        assert_eq!(key(&[]), key(&[]));
        assert_eq!(key(&all), key(&all));
        // The check finishing (`Message::LeftStock`) changes nothing but the
        // uncut faces: the raster is drawn again, so the magenta appears
        // without a camera move.
        assert_ne!(key(&[]), key(&all));
        // The job edited, so the result no longer counts, and the result
        // worked out again, a new allocation whether or not the same faces.
        assert_ne!(key(&all), key(&[]));
        assert_ne!(key(&all), key(&all.clone()));
    }
    #[test]
    fn a_later_setups_known_box_is_drawn_as_its_stock_and_otherwise_none_is() {
        let (doc, mesh) = crate::post::fixture::flipped_after(1);
        let setup = &doc.setups[1];
        assert_ne!(crate::model::get(&setup.p, "stock_source"), "fresh");
        let own = stock::resolve(setup, Some(&mesh)).unwrap().stock;
        // A box 4 mm shorter than the original block, as a facing leaves it.
        let mut left = own;
        left.min[2] += 4.;
        let view = |show_stock, leftover| Viewport {
            setup: Some(setup),
            operation: None,
            mesh: &mesh,
            simulation: None,
            sample: false,
            camera: Camera::default(),
            picking: false,
            pin_holes: Default::default(),
            pin_aim: Default::default(),
            show_stock,
            show_paths: false,
            progress: None,
            holding: vec![],
            leftover,
            live: None,
            live_stock: None,
            uncut: &[],
            show_uncut: true,
        };
        let bounds = Rectangle {
            x: 0.,
            y: 0.,
            width: 400.,
            height: 300.,
        };
        let p = Projector::fit(
            Camera::default(),
            (400., 300.),
            view(true, Some(left)).scene_bounds(),
        );
        let pixels = |v: &Viewport| v.pixels(&p, bounds, None).2;
        // Not known: the original block is not drawn, as before.
        assert_eq!(pixels(&view(true, None)), pixels(&view(false, None)));
        // Known: the box is drawn, and is the one the camera fits.
        let drawn = pixels(&view(true, Some(left)));
        assert_ne!(drawn, pixels(&view(false, Some(left))));
        assert_ne!(drawn, pixels(&view(true, None)));
        let scale = mesh.display_scale();
        assert_eq!(
            view(true, Some(left)).scene_boxes().last(),
            Some(&left.scaled(scale))
        );
        assert_eq!(
            view(true, None).scene_boxes().last(),
            Some(&own.scaled(scale))
        );
        // Hidden with the stock switched off.
        assert!(
            !view(false, Some(left))
                .scene_boxes()
                .contains(&left.scaled(scale))
        );
        // The work zero marker is the program's, on the known box; on the
        // original block it would not be, so none is drawn.
        let zero = crate::post::datum(setup, left, &mesh).unwrap();
        assert_eq!(view(true, Some(left)).zero(), Some(zero));
        assert_eq!(view(false, Some(left)).zero(), Some(zero));
        assert_eq!(view(true, None).zero(), None);
        // The first setup draws its own stock whether or not a box is passed
        // for it, since its source is "fresh".
        let first = Viewport {
            setup: Some(&doc.setups[0]),
            ..view(true, None)
        };
        assert_eq!(crate::model::get(&doc.setups[0].p, "stock_source"), "fresh");
        assert_ne!(
            pixels(&first),
            pixels(&Viewport {
                show_stock: false,
                ..view(true, None)
            })
        );
    }
    #[test]
    fn render_tints_marked_surfaces_and_returns_the_depths() {
        let (doc, mesh) = crate::post::fixture::job();
        let view = Viewport {
            setup: doc.setups.first(),
            operation: None,
            mesh: &mesh,
            simulation: None,
            sample: false,
            camera: Camera::default(),
            picking: false,
            pin_holes: Default::default(),
            pin_aim: Default::default(),
            show_stock: false,
            show_paths: false,
            progress: None,
            holding: vec![],
            leftover: None,
            live: None,
            live_stock: None,
            uncut: &[],
            show_uncut: true,
        };
        let p = Projector::fit(view.camera, (300., 200.), view.scene_bounds());
        let tint = [92, 168, 223, 255];
        let tinted = |rgba: &[u8]| rgba.chunks(4).filter(|px| **px == tint).count();
        let (plain, depths) = view.render(&p, (300, 200), 1., None, &[]);
        assert_eq!(tinted(&plain), 0);
        let features: Vec<usize> = mesh.faces.iter().filter_map(|f| f.feature).collect();
        let (marked, _) = view.render(&p, (300, 200), 1., None, &features);
        assert!(tinted(&marked) > 0);
        // Under the stock's box they keep the plain tint.
        let stocked = Viewport {
            show_stock: true,
            holding: vec![],
            ..view.clone()
        };
        let (under, _) = stocked.render(&p, (300, 200), 1., None, &features);
        assert_eq!(tinted(&under), tinted(&marked));
        // The same pixels are drawn, and the depths are the part's: finite
        // exactly where a pixel was drawn.
        assert_eq!(plain.len(), marked.len());
        assert_eq!(depths.len(), 300 * 200);
        let drawn = |rgba: &[u8]| rgba.chunks(4).map(|px| px[3] > 0).collect::<Vec<_>>();
        assert_eq!(drawn(&plain), drawn(&marked));
        assert_eq!(
            depths.iter().map(|d| d.is_finite()).collect::<Vec<_>>(),
            drawn(&plain)
        );
        // With the simulated stock shown the part is not drawn: the depths
        // are the stock's and the tool's.
        let (doc, mesh, report) = crate::proposal::tests::block();
        let good = crate::proposal::tests::good(&report);
        let job = crate::proposal::build(&doc, &mesh, &report, &good).unwrap();
        let sim =
            crate::planner::view_after(&job, &mesh, 0, 0, crate::stock_surface::Detail::Draft)
                .unwrap();
        let view = Viewport {
            setup: job.setups.first(),
            simulation: Some(&sim),
            show_stock: true,
            mesh: &mesh,
            ..view
        };
        let p = Projector::fit(view.camera, (300., 200.), view.scene_bounds());
        let (rgba, depths) = view.render(&p, (300, 200), 1., None, &[]);
        assert!(depths.iter().any(|d| d.is_finite()));
        assert_eq!(
            depths.iter().map(|d| d.is_finite()).collect::<Vec<_>>(),
            drawn(&rgba)
        );
    }
    #[test]
    fn full_resolution_tiles_stay_below_async_upload_limit_and_preserve_pixels() {
        for (width, height) in [(1, 1), (512, 512), (513, 517), (1600, 1200)] {
            let pixels: Vec<u8> = (0..width * height)
                .flat_map(|i| {
                    [
                        (i % 251) as u8,
                        ((i / width) % 253) as u8,
                        ((i / 7) % 255) as u8,
                        255,
                    ]
                })
                .collect();
            let tiled = RasterFrame::from_rgba(width, height, &pixels);
            let mut reconstructed = vec![0; pixels.len()];
            let mut coverage = vec![0u8; (width * height) as usize];
            for tile in tiled.tiles {
                let iced::widget::image::Handle::Rgba {
                    width: w,
                    height: h,
                    pixels: data,
                    ..
                } = tile.image
                else {
                    panic!("RGBA tiles required");
                };
                assert!(
                    (w * h * 4) < 2 * 1024 * 1024,
                    "Iced skips the first draw of larger uploads"
                );
                let x0 = tile.image_bounds.x as u32;
                let y0 = tile.image_bounds.y as u32;
                // Every gutter pixel must be a true neighbor from the full frame.
                for y in 0..h {
                    let source = (((y0 + y) * width + x0) * 4) as usize;
                    let local = (y * w * 4) as usize;
                    assert_eq!(
                        &data[local..local + (w * 4) as usize],
                        &pixels[source..source + (w * 4) as usize]
                    );
                }
                for y in
                    tile.clip_bounds.y as u32..(tile.clip_bounds.y + tile.clip_bounds.height) as u32
                {
                    for x in tile.clip_bounds.x as u32
                        ..(tile.clip_bounds.x + tile.clip_bounds.width) as u32
                    {
                        let destination = ((y * width + x) * 4) as usize;
                        let source = (((y - y0) * w + (x - x0)) * 4) as usize;
                        reconstructed[destination..destination + 4]
                            .copy_from_slice(&data[source..source + 4]);
                        coverage[(y * width + x) as usize] += 1;
                    }
                }
            }
            assert_eq!(pixels, reconstructed);
            assert!(
                coverage.iter().all(|v| *v == 1),
                "Each pixel is composited exactly once"
            );
        }
    }
    #[test]
    fn orbit_and_pan_never_change_fit_scale_or_target() {
        let d = crate::model::Document::sample();
        let mesh = crate::mesh::sample();
        let mut v = Viewport {
            setup: Some(d.setup()),
            operation: None,
            mesh: &mesh,
            simulation: None,
            sample: true,
            camera: Camera::default(),
            picking: false,
            pin_holes: Default::default(),
            pin_aim: Default::default(),
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            leftover: None,
            live: None,
            live_stock: None,
            uncut: &[],
            show_uncut: true,
        };
        let b = Rectangle::new(Point::ORIGIN, iced::Size::new(860., 540.));
        let reference = v.projector(b);
        for i in 0..72 {
            v.camera.yaw = i as f32 * std::f32::consts::TAU / 72.;
            v.camera.pitch = -1.3 + 2.6 * i as f32 / 71.;
            v.camera.pan = [i as f32, -(i as f32)];
            let p = v.projector(b);
            assert_eq!(p.scale, reference.scale);
            assert_eq!(p.target, reference.target);
        }
        v.camera.zoom = 2.;
        assert_eq!(v.projector(b).scale, reference.scale * 2.);
    }
    #[test]
    fn transparent_stock_preserves_part_occlusion_and_alpha() {
        let vertices = [
            (Point::new(2., 2.), 0.),
            (Point::new(9., 2.), 0.),
            (Point::new(9., 9.), 0.),
            (Point::new(2., 9.), 0.),
            (Point::new(2., 2.), 2.),
            (Point::new(9., 2.), 2.),
            (Point::new(9., 9.), 2.),
            (Point::new(2., 9.), 2.),
        ];
        let index = (5 * 12 + 5) * 4;
        for (depth, expected_unchanged) in [(3., true), (1., false)] {
            let mut pixels = [128u8, 128, 128, 255].repeat(144);
            raster_stock(
                &mut pixels,
                &vec![depth; 144],
                12,
                12,
                vertices,
                1.,
                1.,
                &mut SolidBuffers::default(),
            );
            assert_eq!(
                pixels[index..index + 4] == [128, 128, 128, 255],
                expected_unchanged
            );
            assert_eq!(pixels[index + 3], 255);
        }
        let mut transparent = vec![0; 144 * 4];
        raster_stock(
            &mut transparent,
            &vec![f32::NEG_INFINITY; 144],
            12,
            12,
            vertices,
            1.,
            1.,
            &mut SolidBuffers::default(),
        );
        assert!(transparent[index + 3] > 0 && transparent[index + 3] < 128);
    }
    #[test]
    fn workholding_is_drawn_as_grey_translucent_boxes() {
        use crate::holding::HoldBox;
        let d = crate::model::Document::sample();
        let mesh = crate::mesh::sample();
        let scale = mesh.display_scale();
        // A jaw beyond the stock's +Y side, seen from above.
        let jaw = HoldBox {
            label: "vise jaw on +Y".into(),
            bounds: crate::mesh::Bounds {
                min: [-60., 60., -20.],
                max: [60., 80., 30.],
            },
        };
        let mut v = Viewport {
            setup: Some(d.setup()),
            operation: None,
            mesh: &mesh,
            simulation: None,
            sample: true,
            camera: Camera {
                yaw: 0.,
                pitch: std::f32::consts::FRAC_PI_2,
                ..Default::default()
            },
            picking: false,
            pin_holes: Default::default(),
            pin_aim: Default::default(),
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            leftover: None,
            live: None,
            live_stock: None,
            uncut: &[],
            show_uncut: true,
        };
        let b = Rectangle::new(Point::ORIGIN, iced::Size::new(400., 300.));
        let p = v.projector(b);
        let (w, h, plain) = v.pixels(&p, b, None);
        v.holding = vec![jaw];
        let (_, _, drawn) = v.pixels(&p, b, None);
        assert_eq!((w, h), (600, 450));
        let at = |x: f32, y: f32| ((y * 1.5) as u32 * w + (x * 1.5) as u32) as usize * 4;
        let top = p.p([0., 70. * scale, 30. * scale]);
        let i = at(top.x, top.y);
        // Nothing was there; now there is neutral grey, not opaque.
        assert_eq!(plain[i + 3], 0);
        let [r, g, bl, a] = [drawn[i], drawn[i + 1], drawn[i + 2], drawn[i + 3]];
        assert!(a > 0 && a < 255, "{a}");
        assert!(r == g && g == bl, "{r} {g} {bl}");
        // Only the jaw's own pixels changed.
        let changed = (0..plain.len() / 4)
            .filter(|k| plain[k * 4..k * 4 + 4] != drawn[k * 4..k * 4 + 4])
            .count();
        assert!(changed > 0 && changed < (w * h) as usize / 4, "{changed}");
        assert_eq!(plain[..4], drawn[..4]);
        // The list alone decides: the view passes none when the stock is hidden.
        v.show_stock = false;
        let (_, _, listed) = v.pixels(&p, b, None);
        v.holding.clear();
        assert_ne!(v.pixels(&p, b, None).2, listed);
    }
    #[test]
    fn the_camera_fit_contains_the_drawn_workholding() {
        use crate::holding::HoldBox;
        let d = crate::model::Document::sample();
        let mesh = crate::mesh::sample();
        let scale = mesh.display_scale();
        // A clamp reaching far beyond the stock's +X side.
        let clamp = HoldBox {
            label: "clamp on +X (offset 0)".into(),
            bounds: crate::mesh::Bounds {
                min: [200., -10., 45.],
                max: [600., 10., 70.],
            },
        };
        let mut v = Viewport {
            setup: Some(d.setup()),
            operation: None,
            mesh: &mesh,
            simulation: None,
            sample: true,
            camera: Camera::default(),
            picking: false,
            pin_holes: Default::default(),
            pin_aim: Default::default(),
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            leftover: None,
            live: None,
            live_stock: None,
            uncut: &[],
            show_uncut: true,
        };
        let b = Rectangle::new(Point::ORIGIN, iced::Size::new(500., 300.));
        let bounds = clamp.bounds;
        let inside = |p: &Projector| {
            box_vertices(bounds.scaled(scale)).iter().all(|point| {
                let q = p.p(*point);
                q.x >= 0. && q.x <= b.width && q.y >= 0. && q.y <= b.height
            })
        };
        let without = v.projector(b);
        assert!(!inside(&without));
        v.holding = vec![clamp];
        let with = v.projector(b);
        assert!(inside(&with));
        assert!(with.scale < without.scale);
        // The list alone decides: the view passes none when the stock is hidden.
        v.show_stock = false;
        let listed = v.projector(b);
        v.holding.clear();
        let none = v.projector(b);
        assert!(listed.scale < none.scale);
    }
    #[test]
    fn camera_fit_contains_rotated_part_and_asymmetric_stock() {
        let mut d = crate::model::Document::sample();
        d.setup_mut().p.insert("ry".into(), "90".into());
        d.setup_mut().p.insert("stock_top".into(), "100".into());
        let mesh = crate::mesh::sample();
        let v = Viewport {
            setup: Some(d.setup()),
            operation: None,
            mesh: &mesh,
            simulation: None,
            sample: true,
            camera: Camera::default(),
            picking: false,
            pin_holes: Default::default(),
            pin_aim: Default::default(),
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            leftover: None,
            live: None,
            live_stock: None,
            uncut: &[],
            show_uncut: true,
        };
        let b = Rectangle::new(Point::ORIGIN, iced::Size::new(500., 240.));
        let p = v.projector(b);
        let stock = stock::resolve(d.setup(), Some(&mesh)).unwrap();
        for point in box_vertices(stock.stock.scaled(stock.display_scale)) {
            let q = p.p(point);
            assert!(q.x >= 0. && q.x <= b.width && q.y >= 0. && q.y <= b.height);
        }
    }
    #[test]
    fn transform_rotation() {
        let p = crate::mesh::rotation([0., 1., 0.], [180., 0., 0.]);
        assert!((p[1] + 1.).abs() < 0.001)
    }
    #[test]
    fn depth_interpolation() {
        let t = [
            (Point::new(0., 0.), 1.),
            (Point::new(10., 0.), 2.),
            (Point::new(0., 10.), 3.),
        ];
        assert!((triangle_depth(t, Point::new(2., 2.)).unwrap() - 1.6).abs() < 0.001);
        assert!(triangle_depth(t, Point::new(20., 20.)).is_none());
    }
    #[test]
    fn picks_visible_floor() {
        let d = crate::model::Document::sample();
        let mesh = crate::mesh::sample();
        let camera = Camera::default();
        let v = Viewport {
            setup: Some(d.setup()),
            operation: Some(&d.setup().operations[1]),
            mesh: &mesh,
            simulation: None,
            sample: true,
            camera,
            picking: true,
            pin_holes: Default::default(),
            pin_aim: Default::default(),
            show_stock: false,
            show_paths: false,
            progress: None,
            holding: vec![],
            leftover: None,
            live: None,
            live_stock: None,
            uncut: &[],
            show_uncut: true,
        };
        let b = Rectangle::new(Point::ORIGIN, iced::Size::new(800., 600.));
        let p = v.projector(b);
        assert_eq!(v.picked(b, p.p([20., 0., 13.1])), Some(1));
    }
    #[test]
    fn imported_picker_uses_visible_surface_and_setup_transform() {
        // Overlapping screen projections: the upper triangle occludes the lower.
        let mesh = crate::mesh::normalize(vec![
            [[0., 0., 0.], [10., 0., 0.], [0., 10., 0.]],
            [[0., 0., 2.], [10., 0., 2.], [0., 10., 2.]],
        ])
        .unwrap();
        let mut d = crate::model::Document::sample();
        d.setup_mut().p.insert("rz".into(), "90".into());
        d.setup_mut().p.insert("tx".into(), "15".into());
        let mut v = Viewport {
            setup: Some(d.setup()),
            operation: Some(&d.setup().operations[0]),
            mesh: &mesh,
            simulation: None,
            sample: false,
            camera: Camera {
                yaw: 0.,
                pitch: std::f32::consts::FRAC_PI_2,
                ..Default::default()
            },
            picking: true,
            pin_holes: Default::default(),
            pin_aim: Default::default(),
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            leftover: None,
            live: None,
            live_stock: None,
            uncut: &[],
            show_uncut: true,
        };
        let bounds = Rectangle::new(Point::ORIGIN, iced::Size::new(800., 600.));
        let point = Placement::new(&mesh, v.setup).transform(mesh.surfaces[1].center);
        assert_eq!(v.picked(bounds, v.projector(bounds).p(point)), Some(1));
        v.operation = Some(&d.setup().operations[1]);
        assert_eq!(v.picked(bounds, v.projector(bounds).p(point)), None);
        v.operation = Some(&d.setup().operations[0]);
        v.picking = false;
        assert_eq!(v.picked(bounds, v.projector(bounds).p(point)), None);
    }

    /// The frame is right-handed: in each of the camera buttons' views
    /// (TOP, FRONT, RIGHT and ISO, the default) the axes go where a part
    /// seen from there has them.
    #[test]
    fn the_named_views_project_the_axes_as_a_right_handed_frame() {
        use std::f32::consts::FRAC_PI_2;
        let view = |yaw, pitch| {
            Projector::fit(
                Camera {
                    yaw,
                    pitch,
                    ..Default::default()
                },
                (400., 300.),
                crate::mesh::Bounds {
                    min: [-1.; 3],
                    max: [1.; 3],
                },
            )
        };
        // Each unit axis from the origin: the sign of its screen motion
        // (x to the right, y down) and of its depth (toward the viewer).
        let axes = |p: &Projector| {
            let o = p.p([0.; 3]);
            let sign = |d: f32| if d.abs() < 1e-4 { 0 } else { d.signum() as i8 };
            [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]].map(|v| {
                let q = p.p(v);
                (
                    sign((q.x - o.x) / p.scale),
                    sign((q.y - o.y) / p.scale),
                    sign(p.depth(v)),
                )
            })
        };
        // TOP: +X right, +Y up, +Z toward the viewer.
        assert_eq!(
            axes(&view(0., FRAC_PI_2)),
            [(1, 0, 0), (0, -1, 0), (0, 0, 1)]
        );
        // FRONT, from -Y: +X right, +Y away, +Z up.
        assert_eq!(axes(&view(0., 0.)), [(1, 0, 0), (0, 0, -1), (0, -1, 0)]);
        // RIGHT, from +X: +X toward the viewer, +Y right, +Z up.
        assert_eq!(
            axes(&view(FRAC_PI_2, 0.)),
            [(0, 0, 1), (1, 0, 0), (0, -1, 0)]
        );
        // ISO, from the front right and above: +X right and down, toward
        // the viewer; +Y right and up, away; +Z up.
        let iso = Camera::default();
        assert_eq!(
            axes(&view(iso.yaw, iso.pitch)),
            [(1, 1, 1), (1, -1, -1), (0, -1, 1)]
        );
    }
    /// The pixels agree with the projector: a box beyond the part's +X
    /// side is drawn on the right of a top view and one beyond +Y above.
    #[test]
    fn a_top_view_draws_what_is_beyond_x_on_the_right_and_beyond_y_above() {
        use crate::holding::HoldBox;
        let d = crate::model::Document::sample();
        let mesh = crate::mesh::sample();
        let view = |holding| Viewport {
            setup: Some(d.setup()),
            operation: None,
            mesh: &mesh,
            simulation: None,
            sample: true,
            camera: Camera {
                yaw: 0.,
                pitch: std::f32::consts::FRAC_PI_2,
                ..Default::default()
            },
            picking: false,
            pin_holes: Default::default(),
            pin_aim: Default::default(),
            show_stock: false,
            show_paths: false,
            progress: None,
            holding,
            leftover: None,
            live: None,
            live_stock: None,
            uncut: &[],
            show_uncut: true,
        };
        let b = Rectangle::new(Point::ORIGIN, iced::Size::new(400., 300.));
        let hold = |min, max| HoldBox {
            label: "box".into(),
            bounds: crate::mesh::Bounds { min, max },
        };
        for (hold, beyond_x) in [
            (hold([150., -10., 0.], [180., 10., 20.]), true),
            (hold([-10., 120., 0.], [10., 150., 20.]), false),
        ] {
            let with = view(vec![hold]);
            let p = with.projector(b);
            let (w, h, plain) = view(vec![]).pixels(&p, b, None);
            let (_, _, drawn) = with.pixels(&p, b, None);
            let changed: Vec<(u32, u32)> = (0..w * h)
                .filter(|k| {
                    let k = *k as usize * 4;
                    plain[k..k + 4] != drawn[k..k + 4]
                })
                .map(|k| (k % w, k / w))
                .collect();
            assert!(changed.len() > 100, "{}", changed.len());
            assert!(
                changed
                    .iter()
                    .all(|&(x, y)| if beyond_x { x > w / 2 } else { y < h / 2 }),
                "{:?}",
                changed.iter().take(4).collect::<Vec<_>>()
            );
        }
    }
}
