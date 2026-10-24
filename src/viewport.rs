use crate::{
    mesh::{Face, Mesh},
    model::{Operation, Setup, Strategy},
    section::{Plane, Section, Segment},
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
impl Camera {
    /// The direction the viewer is in from the target, of length 1.
    pub fn toward(&self) -> [f32; 3] {
        let (s, c) = self.yaw.sin_cos();
        let (se, ce) = self.pitch.sin_cos();
        [s * ce, -c * ce, se]
    }
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
    /// A turn of the view (yaw, pitch, radians), and how far to move the
    /// pan with it (pixels) so the point the view turns about stays put.
    Orbit(f32, f32, [f32; 2]),
    Pan(f32, f32),
    /// A wheel step, and where the pointer is from the view's centre
    /// (`Projector::center`, pixels): the zoom keeps the point under it.
    Zoom(f32, [f32; 2]),
    Pick(usize),
    PickSurface(usize),
    /// Where pin 1 would stand under the pointer while placing the pins
    /// (`PinAim`), in plan mm; None once the pointer leaves the view.
    AimPins(Option<[f32; 2]>),
    /// A click, or a drag of the pins ending, while placing them: pin 1
    /// stands there (plan mm).
    PlacePins([f32; 2]),
    /// A click on a surface of the part while picking the face a section
    /// goes through (`Sectioning::picking`).
    SectionSurface(usize),
}
/// The section the view is cut on, and whether a click on the part picks
/// the face it goes through.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sectioning {
    pub cut: Option<Section>,
    pub picking: bool,
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
    /// While the view is turned by a drag: the point it turns about
    /// (`Viewport::pivot`), found once the drag starts turning, and the
    /// camera the turns sent so far leave, which the next turn starts from
    /// even when the app has not drawn them yet.
    pivot: Option<([f32; 3], Camera)>,
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
/// A hole of the part a Pocket or a Contour picks by its outline
/// (`pocket::part_holes`): what of it is open from above, as loops in the
/// setup's frame (mm) at the height of its rim, the walls a click on it
/// clicks, and whether the operation has it. The outline is drawn while
/// the pointer is over it or over the hole, blue once picked.
#[derive(Clone, Debug, Default)]
pub struct HoleOutline {
    pub loops: Vec<Vec<[f32; 2]>>,
    pub z: f32,
    pub walls: Vec<usize>,
    pub picked: bool,
    /// What it is labelled with (a Drill's: the diameter it drills). A
    /// labelled outline is drawn whether or not the pointer is over it, so
    /// every hole the operation can take shows, and inside it the pointer
    /// takes the hole before the faces under it.
    pub label: Option<String>,
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
    /// The part's holes a Pocket or a Contour being picked can pick by their
    /// outline; empty otherwise.
    pub hole_outlines: Vec<HoleOutline>,
    pub show_stock: bool,
    pub show_paths: bool,
    pub progress: Option<f32>,
    /// The jaws or clamps of the setup, in its frame in mm; drawn with the
    /// stock and part of the camera's fit.
    pub holding: Vec<crate::holding::HoldBox>,
    /// The jaws or clamps the camera fits (`scene_bounds`), in the same frame
    /// and units: the setup's round its own stock, drawn or not, so that
    /// hiding them or a change of the box they hold does not move the view.
    pub framed: Vec<crate::holding::HoldBox>,
    /// The box a later setup starts from, in its frame in mm, when the setup
    /// editor knows it (`App::known_stock`): drawn as the stock, and said in
    /// the label, where the setup's own box would be the original block's.
    /// None for the first setup and when it is not known.
    pub leftover: Option<crate::mesh::Bounds>,
    /// The machine's tool while a tool run of this setup cuts: its tip in
    /// plan coordinates (mm), radius and stickout. Drawn over the scene.
    pub live: Option<([f32; 3], f32, f32)>,
    /// The machine's A (degrees, as commanded) while a tool run of this
    /// setup cuts: an indexed setup's part, stock and chuck are drawn
    /// turned to it, else to the simulation's (`shown_a`).
    pub a: Option<f32>,
    /// The stock a run of this setup has carved, or starts from, in plan
    /// coordinates (`run::Run::stock`) with its version: drawn instead of the
    /// setup's stock when no simulation is shown.
    pub live_stock: Option<(&'a crate::simulation::StockGrid, u64)>,
    /// The stock the operation shown starts from, as the operations before
    /// it in the job leave it (`App::start_stock`): the job's stock, for its
    /// box and frame, and the surface of what is left of it then. Drawn
    /// instead of the setup's stock when no simulation is shown, solid as
    /// the simulation draws it, and see-through while the part is picked.
    /// None while nothing is cut before the operation or it is not known.
    pub start_stock: Option<(
        &'a crate::volume::Volume,
        &'a crate::stock_surface::SurfaceMesh,
    )>,
    /// The stock a run of this setup that turns A has carved, as a volume
    /// (`run::Run::carved_volume`) with its version: drawn, turned to the
    /// machine's A, instead of the setup's stock when no simulation is shown.
    pub live_volume: Option<(&'a crate::volume::Volume, u64)>,
    /// Per face of the part (`Mesh::faces`): whether the whole job still
    /// leaves it under stock (`uncut::Uncut::faces`); those are drawn in
    /// `UNCUT`. Empty when that is not known.
    pub uncut: &'a [bool],
    /// Whether those faces are drawn in `UNCUT` at all ("Show uncut faces").
    /// With a simulation and the stock shown the part itself is not drawn, so
    /// then just these faces are, after the stock and depth-tested against it.
    pub show_uncut: bool,
    /// How high (logical pixels) the frame the camera centres and fits the
    /// scene in is, from the view's top (`ui::scene_frame`): the area the
    /// view shares with the panels under it, less the room they take at
    /// their tallest. A panel that grows or shrinks then covers or uncovers
    /// the bottom of the view and the scene stays where it is on the
    /// screen. None: the view's own height (pictures, tests).
    pub frame: Option<f32>,
    /// The section the scene is cut on (`section`): what lies on its far
    /// side is left out and the cut faces of the part and the stock are
    /// filled and hatched.
    pub section: Sectioning,
}
/// `camera` turned by `yaw` and `pitch` (radians), as a drag turns the
/// view: the model follows the mouse, as in the machine view, and the pitch
/// stops short of looking straight up or down past the vertical.
pub(crate) fn orbited(camera: Camera, yaw: f32, pitch: f32) -> Camera {
    Camera {
        yaw: camera.yaw - yaw,
        pitch: (camera.pitch + pitch).clamp(-1.5, 1.57),
        ..camera
    }
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
    /// The point the screen point `at` shows at depth `depth` (`depth`
    /// back, as `p` with it).
    pub(crate) fn at_depth(&self, at: Point, depth: f32) -> [f32; 3] {
        let (s, c) = self.camera.yaw.sin_cos();
        let (se, ce) = self.camera.pitch.sin_cos();
        let right = [c, s, 0.];
        let up = [-s * se, c * se, ce];
        let toward = [s * ce, -c * ce, se];
        let a = (at.x - self.center.x) / self.scale;
        let b = (self.center.y - at.y) / self.scale;
        let d = depth - self.depth(self.target);
        std::array::from_fn(|i| self.target[i] + right[i] * a + up[i] * b + toward[i] * d)
    }
    /// How far toward the viewer: larger is nearer.
    pub(crate) fn depth(&self, v: [f32; 3]) -> f32 {
        let (s, c) = self.camera.yaw.sin_cos();
        let (se, ce) = self.camera.pitch.sin_cos();
        v[0] * s * ce - v[1] * c * ce + v[2] * se
    }
    /// The depth (`depth`) of `plane` at each pixel of a frame `ratio`
    /// times the projector's, (a, b, c): a·x + b·y + c at the point (x, y).
    /// None when the view looks along the plane, which then shows nothing
    /// of a cut face.
    fn plane_depth(&self, plane: &Plane, ratio: f32) -> Option<[f32; 3]> {
        let (s, c) = self.camera.yaw.sin_cos();
        let (se, ce) = self.camera.pitch.sin_cos();
        let right = [c, s, 0.];
        let up = [-s * se, c * se, ce];
        let toward = [s * ce, -c * ce, se];
        let along = |v: [f32; 3]| -> f32 { (0..3).map(|i| plane.normal[i] * v[i]).sum() };
        let k = along(toward);
        if k.abs() < 0.05 {
            return None;
        }
        // The point (x, y) shows is target + right·a + up·b + toward·t, a
        // and b its offsets from the centre, t its depth less the target's
        // (`at_depth`); on the plane where its distance is 0, linear in x
        // and y. Worked out whole, not by differences, for f32's sake.
        let pixel = self.scale * ratio;
        let at_centre = plane.distance(self.target) - along(right) * self.center.x / self.scale
            + along(up) * self.center.y / self.scale;
        Some([
            -along(right) / (k * pixel),
            along(up) / (k * pixel),
            self.depth(self.target) - at_centre / k,
        ])
    }
}
/// The cutter as the simulation view draws it: frustums from the tip up of
/// the saved cutter diameter, flute length and stickout, (radius below,
/// radius above, from, to, colour), mm above the tip: a thread mill's teeth
/// and neck, a drill's or chamfer mill's cone, the flutes, the body and the
/// holder.
fn cutter_parts(plan: &crate::simulation::Plan) -> Vec<(f32, f32, f32, f32, [f32; 3])> {
    let (cutter, body, holder) = ([221f32, 158., 50.], [137., 153., 170.], [77., 96., 118.]);
    let r = plan.radius;
    let length = plan.tool_length;
    let mut parts: Vec<(f32, f32, f32, f32, [f32; 3])> = vec![];
    if let (Some((pitch, teeth, cot)), Some([neck, reach, shank])) = (plan.shape.teeth(), plan.neck)
    {
        // A thread mill: its V teeth stacked a pitch apart over a thin
        // neck, which widens to the shank the collet holds.
        let depth = crate::simulation::THREAD_TOOTH * pitch;
        let half = depth / cot;
        for k in 0..teeth as usize {
            let at = k as f32 * pitch;
            parts.push((r - depth, r, at - half, at, cutter));
            parts.push((r, r - depth, at, at + half, cutter));
            if (k + 1) < teeth as usize && pitch > 2. * half {
                parts.push((r - depth, r - depth, at + half, at + pitch - half, cutter));
            }
        }
        let teeth_top = (teeth - 1.) * pitch + half;
        let neck_top = reach.clamp(teeth_top, length);
        parts.push((
            neck.min(r - depth),
            neck,
            teeth_top,
            teeth_top + 0.01,
            cutter,
        ));
        parts.push((neck, neck, teeth_top, neck_top, body));
        // The shoulder up to the shank, at about 30° to the axis.
        let shoulder = (neck_top + (shank - neck).abs() * 1.7).min(length);
        parts.push((neck, shank, neck_top, shoulder, body));
        parts.push((shank, shank, shoulder, length, body));
    } else if let Some(rise) = plan.shape.rise() {
        // A drill's 118° point or a chamfer mill's cone, from its tip (a
        // chamfer mill's flat) up to its diameter, then its flutes.
        let flat = plan.shape.flat().min(r);
        let flute = plan.flute_length;
        let point = ((r - flat) * rise).min(flute);
        parts.push((flat, (flat + point / rise).min(r), 0., point, cutter));
        if point < flute {
            parts.push((r, r, point, flute, cutter));
        }
        parts.push((r, r, flute, length, body));
    } else {
        let bottom = if plan.shape == crate::simulation::CutterShape::Ball {
            r
        } else {
            0.
        };
        parts.push((r, r, bottom, plan.flute_length, cutter));
        parts.push((r, r, plan.flute_length, length, body));
    }
    parts.push((
        plan.holder_radius,
        plan.holder_radius,
        length,
        length + 8.,
        holder,
    ));
    parts
}
impl Viewport<'_> {
    /// The frame the camera centres and fits the scene in, for the view's
    /// bounds `b`: as wide as the view, `frame` high from its top (its own
    /// height without one).
    fn framed_size(&self, b: Rectangle) -> (f32, f32) {
        (b.width, self.frame.unwrap_or(b.height))
    }
    fn projector(&self, b: Rectangle) -> Projector {
        Projector::fit(self.camera, self.framed_size(b), self.scene_bounds())
    }
    /// The point the view turns about: the part's nearest surface at the
    /// view's centre, so what is zoomed in on stays in view as it turns;
    /// where no surface is there, the point at the scene's centre's depth.
    fn pivot(&self, b: Rectangle) -> [f32; 3] {
        let p = self.projector(b);
        let (width, height) = self.framed_size(b);
        let centre = Point::new(width * 0.5, height * 0.53);
        let nearest = self
            .shown()
            .iter()
            .filter_map(|face| {
                let v: Vec<_> = face.points.iter().map(|q| (p.p(*q), p.depth(*q))).collect();
                (1..v.len().saturating_sub(1))
                    .filter_map(|i| triangle_depth([v[0], v[i], v[i + 1]], centre))
                    .reduce(f32::max)
            })
            .reduce(f32::max);
        p.at_depth(centre, nearest.unwrap_or_else(|| p.depth(p.target)))
    }
    /// The A (degrees, as commanded) an indexed setup's part, stock and
    /// chuck are drawn at: the machine's while a run of it cuts, else the
    /// simulation's (`Simulation::a_now`), which turns through each index
    /// move. None for a setup that is not indexed.
    pub(crate) fn shown_a(&self) -> Option<f32> {
        self.setup?.turn?;
        self.a.or_else(|| self.simulation?.a_now())
    }
    /// The turn (degrees, about +X by the right-hand rule) from where the
    /// setup's placement puts the part and its stock, at its first A angle,
    /// to `shown_a`, and the A centreline it is about (Y, Z, mm, the setup's
    /// frame). None when they are not turned.
    fn extra_turn(&self) -> Option<([f32; 2], f32)> {
        let setup = self.setup?;
        let t = setup.turn?;
        let turn = crate::indexed::sign(&setup.machine) * self.shown_a()? - t.turn;
        (turn.abs() > 1e-4).then_some((t.axis, turn))
    }
    /// A point in display units turned by `extra_turn`.
    fn turner(&self) -> impl Fn([f32; 3]) -> [f32; 3] + use<> {
        let scale = self.mesh.display_scale();
        let turn = self.extra_turn();
        move |v| match turn {
            Some((axis, t)) => crate::indexed::turn_point(v, axis.map(|n| n * scale), t),
            None => v,
        }
    }
    /// What the camera fits, in display units: the part, the setup's own
    /// stock and the jaws or clamps round it (`framed`). The same with a
    /// simulation or without, with the stock shown or hidden and before or
    /// after the box a later setup starts from is known (which lies inside
    /// its own), so none of them moves the view.
    pub(crate) fn scene_bounds(&self) -> crate::mesh::Bounds {
        let mut bounds = Placement::new(self.mesh, self.setup).bounds;
        let stock = (self.setup)
            .and_then(|s| stock::resolve(s, Some(self.mesh)).ok())
            .map(|s| s.stock.scaled(s.display_scale));
        let scale = self.mesh.display_scale();
        let holding = self.framed.iter().map(|hold| hold.bounds.scaled(scale));
        for b in stock.into_iter().chain(holding) {
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
                part_radius: None,
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
    /// What a still picture fits to, box by box: the part's first, then the
    /// stock's when shown, each jaw's or clamp's and, when simulating, the
    /// simulated stock's and the reach of every move's cutter. Pictures fit
    /// to their projected corners; the 3D view fits `scene_bounds`.
    pub(crate) fn scene_boxes(&self) -> Vec<crate::mesh::Bounds> {
        let mut boxes = vec![Placement::new(self.mesh, self.setup).bounds];
        if self.show_stock
            && let Some(stock) = self.resolved_stock()
        {
            boxes.push(stock.stock.scaled(stock.display_scale));
        }
        // An indexed setup's part and stock turn with A: the camera holds
        // still round all they sweep about the centreline.
        if let Some(t) = self.setup.and_then(|s| s.turn) {
            let [y, z] = t.axis.map(|n| n * self.mesh.display_scale());
            let swept: Vec<_> = (boxes.iter())
                .map(|b| {
                    let r = [b.min[1] - y, b.max[1] - y]
                        .into_iter()
                        .flat_map(|dy| [b.min[2] - z, b.max[2] - z].map(|dz| dy.hypot(dz)))
                        .fold(0., f32::max);
                    crate::mesh::Bounds {
                        min: [b.min[0], y - r, z - r],
                        max: [b.max[0], y + r, z + r],
                    }
                })
                .collect();
            boxes.extend(swept);
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
        let turned = self.turner();
        self.mesh
            .faces
            .iter()
            .map(|f| Face {
                points: (f.points.iter())
                    .map(|p| turned(placement.transform(*p)))
                    .collect(),
                feature: f.feature,
                tone: f.tone,
            })
            .collect()
    }
    /// The section's plane in the setup's frame (display units): None
    /// without a section or for a surface the part does not have.
    pub(crate) fn cut(&self) -> Option<Plane> {
        self.section.cut?.plane(self.mesh, self.setup)
    }
    /// The part's faces in the setup's frame (`transformed`), less what the
    /// section cuts away: a face it cuts away whole has no points left.
    fn shown(&self) -> Vec<Face> {
        let mut faces = self.transformed();
        if let Some(cut) = self.cut() {
            for face in &mut faces {
                face.points = cut.clip(&face.points);
            }
        }
        faces
    }
    /// Where the section's plane crosses the part's faces, in the setup's
    /// frame: the outline of the part's cut face. Empty without a section.
    fn part_crossings(&self) -> Vec<Segment> {
        let Some(cut) = self.cut() else {
            return vec![];
        };
        let placement = Placement::new(self.mesh, self.setup);
        (self.mesh.faces.iter())
            .filter_map(|f| {
                let points: Vec<_> = f.points.iter().map(|p| placement.transform(*p)).collect();
                cut.crossing(&points)
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
        // The face a section goes through: any surface of the part.
        if self.section.picking {
            return self
                .nearest_surface(b, point)
                .filter(|f| *f < self.mesh.surfaces.len());
        }
        if !self.picking
            || (!self.sample
                && !self.pin_holes.pick
                && !self
                    .operation
                    .is_some_and(|o| o.strategy.imported_preview()))
        {
            return None;
        }
        // A hole's outline under the pointer clicks one of its walls, and so
        // does the inside of a labelled one (a Drill's), whatever face of
        // the hole (its floor, its bore) is under the pointer there.
        if let Some(i) = self.outline_near(b, point) {
            return self.hole_outlines[i].walls.first().copied();
        }
        if let Some(i) = self.outline_around(b, point)
            && self.hole_outlines[i].label.is_some()
        {
            return self.hole_outlines[i].walls.first().copied();
        }
        let feature = self.nearest_surface(b, point);
        // Nothing of the part under the pointer: the opening of a hole
        // through it, seen from above.
        if feature.is_none()
            && let Some(i) = self.outline_around(b, point)
        {
            return self.hole_outlines[i].walls.first().copied();
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
    /// The surface of the part's face nearest the viewer at `point`, of the
    /// faces the section leaves (`shown`).
    fn nearest_surface(&self, b: Rectangle, point: Point) -> Option<usize> {
        let p = self.projector(b);
        let mut nearest = f32::NEG_INFINITY;
        let mut feature = None;
        for face in self.shown() {
            let vertices: Vec<_> = face.points.iter().map(|v| (p.p(*v), p.depth(*v))).collect();
            for i in 1..vertices.len().saturating_sub(1) {
                if let Some(z) = triangle_depth([vertices[0], vertices[i], vertices[i + 1]], point)
                    && z > nearest
                {
                    nearest = z;
                    feature = face.feature;
                }
            }
        }
        feature
    }
    /// Where this view of `bounds` puts the point `q` (display units),
    /// relative to it: for pictures with the pointer over it.
    #[cfg(test)]
    pub(crate) fn screen_point(&self, bounds: Rectangle, q: [f32; 3]) -> Point {
        self.projector(bounds).p(q)
    }
    /// The holes' outlines: the one under the pointer (by its outline, a
    /// wall, or its opening) in amber, those picked in blue, and the other
    /// labelled ones in teal, each with its label.
    fn hole_marks(&self, frame: &mut Frame, bounds: Rectangle, cursor: mouse::Cursor) {
        if self.hole_outlines.is_empty() {
            return;
        }
        let hovered = cursor
            .position_in(bounds)
            .and_then(|at| self.picked(bounds, at))
            .and_then(|id| {
                self.hole_outlines
                    .iter()
                    .position(|o| o.walls.contains(&id))
            });
        let halo = Color::from_rgba8(255, 255, 255, 0.9);
        let blue = Color::from_rgb8(37, 99, 235);
        let amber = Color::from_rgb8(217, 119, 6);
        let teal = Color::from_rgb8(13, 128, 120);
        let screen = self.outlines_on_screen(bounds);
        let color = |i: usize| {
            let o = &self.hole_outlines[i];
            if hovered == Some(i) {
                Some(amber)
            } else if o.picked {
                Some(blue)
            } else if o.label.is_some() {
                Some(teal)
            } else {
                None
            }
        };
        for (i, loops) in screen.iter().enumerate() {
            let Some(color) = color(i) else { continue };
            for l in loops {
                line(frame, l, halo, 4.5);
                line(frame, l, color, 2.);
            }
        }
        let boxes: Vec<Rectangle> = (screen.iter())
            .map(|loops| {
                let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for q in loops.iter().flatten() {
                    (x0, y0, x1, y1) = (x0.min(q.x), y0.min(q.y), x1.max(q.x), y1.max(q.y));
                }
                Rectangle::new(
                    Point::new(x0 - 2., y0 - 2.),
                    iced::Size::new(x1 - x0 + 4., y1 - y0 + 4.),
                )
            })
            .collect();
        let widths: Vec<Option<usize>> = (self.hole_outlines.iter())
            .map(|o| o.label.as_ref().map(|l| l.chars().count()))
            .collect();
        let picked: Vec<bool> = self.hole_outlines.iter().map(|o| o.picked).collect();
        let (placed, hidden) = place_labels(&boxes, &widths, &picked, hovered);
        // Text is not clipped to the view: a label off it is left out.
        let inside = |r: &Rectangle| {
            r.x >= 0.
                && r.y >= 0.
                && r.x + r.width <= bounds.width
                && r.y + r.height <= bounds.height
        };
        // The hovered hole's is moved into the view rather than left out.
        let placed = placed.into_iter().map(|(i, spot, away)| {
            if hovered == Some(i) && !inside(&spot) {
                let at = Point::new(
                    spot.x.clamp(0., (bounds.width - spot.width).max(0.)),
                    spot.y.clamp(0., (bounds.height - spot.height).max(0.)),
                );
                (i, Rectangle::new(at, spot.size()), true)
            } else {
                (i, spot, away)
            }
        });
        let labels: Vec<(usize, Rectangle, bool, Color)> = placed
            .filter(|(_, spot, _)| inside(spot))
            .filter_map(|(i, spot, away)| color(i).map(|c| (i, spot, away, c)))
            .collect();
        // In order: the hovered hole's comes last (`place_labels`), over the
        // others, which cover none of each other.
        for &(i, spot, away, color) in &labels {
            if away {
                let hole = boxes[i];
                let from = Point::new(hole.x + hole.width / 2., hole.y + hole.height / 2.);
                let to = leader_end(from, spot);
                line(frame, &[from, to], halo, 3.);
                line(frame, &[from, to], color, 1.);
            }
            frame.fill_rectangle(spot.position(), spot.size(), halo);
            frame.fill_text(Text {
                content: self.hole_outlines[i].label.clone().unwrap_or_default(),
                position: Point::new(spot.x + 3., spot.y + 1.),
                color,
                size: 11.into(),
                ..Text::default()
            });
        }
        if hidden > 0 {
            let said = hidden_note(hidden);
            let size = iced::Size::new(said.chars().count() as f32 * 6.4 + 12., 20.);
            // Under the stock's label, top left.
            let corner = Point::new(((bounds.width - size.width) / 2.).max(6.), 44.);
            frame.fill_rectangle(corner, size, halo);
            frame.fill_text(Text {
                content: said,
                position: Point::new(corner.x + 6., corner.y + 3.),
                color: teal,
                size: 12.into(),
                ..Text::default()
            });
        }
    }
    /// Each hole outline's loops on the screen.
    fn outlines_on_screen(&self, b: Rectangle) -> Vec<Vec<Vec<Point>>> {
        let p = self.projector(b);
        let scale = self.mesh.display_scale();
        (self.hole_outlines.iter())
            .map(|o| {
                (o.loops.iter())
                    .map(|l| {
                        (l.iter().chain(l.first()))
                            .map(|q| p.p([q[0] * scale, q[1] * scale, o.z * scale]))
                            .collect()
                    })
                    .collect()
            })
            .collect()
    }
    /// The hole outline nearest `point` within `OUTLINE_REACH` on the screen.
    fn outline_near(&self, b: Rectangle, point: Point) -> Option<usize> {
        if self.hole_outlines.is_empty() {
            return None;
        }
        let mut best = None;
        for (i, loops) in self.outlines_on_screen(b).iter().enumerate() {
            for l in loops {
                for w in l.windows(2) {
                    let d = segment_distance(point, w[0], w[1]);
                    if d <= OUTLINE_REACH && best.is_none_or(|(_, e)| d < e) {
                        best = Some((i, d));
                    }
                }
            }
        }
        best.map(|(i, _)| i)
    }
    /// The smallest hole outline around `point` on the screen.
    fn outline_around(&self, b: Rectangle, point: Point) -> Option<usize> {
        let mut best = None;
        for (i, loops) in self.outlines_on_screen(b).iter().enumerate() {
            for l in loops {
                let area = screen_area(l);
                if area > 0. && inside(point, l) && best.is_none_or(|(_, a)| area < a) {
                    best = Some((i, area));
                }
            }
        }
        best.map(|(i, _)| i)
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
        let key = format!("{key}|{:?}", self.shown_a());
        format!(
            "{key}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{}|{}|{:?}",
            self.holding,
            self.framed,
            self.frame,
            self.leftover,
            self.live,
            self.live_stock.map(|(g, v)| (std::ptr::from_ref(g), v)),
            self.start_stock
                .map(|(_, surface)| std::ptr::from_ref(surface)),
            self.live_volume.map(|(g, v)| (std::ptr::from_ref(g), v)),
            self.uncut.first().map(std::ptr::from_ref),
            self.uncut.len(),
            self.show_uncut,
            self.section
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
        // So is the stock a run carved, or the operation starts from, unless
        // the part is being picked: then that stock is see-through.
        let start = self.start_stock.filter(|_| self.setup.is_some());
        let carved = self.simulation.is_none()
            && self.show_stock
            && (self.live_stock.is_some()
                || self.live_volume.is_some()
                || (start.is_some() && !self.picking));
        let behind_stock = (self.simulation.is_some() && self.show_stock) || carved;
        let cut = self.cut();
        let whole = if behind_stock && !self.show_uncut {
            vec![]
        } else {
            self.transformed()
        };
        // What the section leaves of each face. Each is shaded as the whole
        // face is: what is left of one can start with three points nearly in
        // a line, whose normal says nothing.
        let clipped: Option<Vec<Face>> = cut.map(|cut| {
            (whole.iter())
                .map(|f| Face {
                    points: cut.clip(&f.points),
                    feature: f.feature,
                    tone: f.tone,
                })
                .collect()
        });
        let faces = clipped.as_ref().unwrap_or(&whole);
        // A point of the setup's frame (display units) in the frame's pixels.
        let screen = |v: [f32; 3]| {
            let q = p.p(v);
            Point::new(q.x * ratio, q.y * ratio)
        };
        let cap_depth = cut.and_then(|cut| p.plane_depth(&cut, ratio));
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
        // Where a click picks a whole smooth face (Parallel and Waterline
        // finishing), the pointer shows all of it.
        let hovered_face: std::collections::HashSet<usize> = match hovered {
            Some(h)
                if !self.sample
                    && !self.section.picking
                    && self.operation.is_some_and(crate::picks_faces) =>
            {
                self.mesh.region_of(h).into_iter().collect()
            }
            _ => Default::default(),
        };
        // A selected face still under stock is striped in the selection's
        // blue and the magenta (`hatch`).
        let hatch = hatch(ratio);
        for (fi, face) in faces.iter().enumerate() {
            if behind_stock {
                break;
            }
            let light = face_light(&whole[fi]);
            let selected = selected_by(face);
            let hover = face.feature.is_some()
                && (face.feature == hovered || hovered_face.contains(&face.feature.unwrap()));
            let kept = !hover && face.feature.is_some_and(|f| marked.contains(&f));
            let uncut = self.show_uncut && self.uncut.get(fi) == Some(&true);
            let striped = !hover && uncut && selected;
            let color = if hover {
                [242, 190, 93, 255]
            } else if uncut {
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
            for i in 1..v.len().saturating_sub(1) {
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
                                let color = if striped { hatch(color, x, y) } else { color };
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
        // The part's cut face, where the section goes through it.
        let crossings = self.part_crossings();
        if !behind_stock && let Some(plane) = cap_depth {
            let outline: Vec<_> = (crossings.iter())
                .map(|[a, b]| (screen(*a), screen(*b)))
                .collect();
            raster_cap(
                &mut rgba,
                &mut depths,
                (w, h),
                &outline,
                plane,
                section_hatch(PART_CUT, ratio),
                |index| {
                    if let Some(plain) = plain.get_mut(index) {
                        *plain = false;
                    }
                },
            );
        }
        if self.simulation.is_some() || carved {
            let outline = match (self.simulation, self.live_stock) {
                (Some(simulation), _) => {
                    self.raster_simulation(simulation, p, &mut rgba, &mut depths, w, h, ratio)
                }
                (None, Some((grid, _))) => {
                    let scale = self.mesh.display_scale();
                    let mut outline = vec![];
                    let mut face = |points: &[[f32; 3]], color: [u8; 4]| {
                        let points: Vec<_> = points.iter().map(|v| v.map(|n| n * scale)).collect();
                        let points = match cut {
                            Some(cut) => {
                                outline.extend(cut.crossing(&points));
                                cut.clip(&points)
                            }
                            None => points,
                        };
                        let v: Vec<_> =
                            (points.iter()).map(|v| (screen(*v), p.depth(*v))).collect();
                        for i in 1..v.len().saturating_sub(1) {
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
                    outline
                }
                (None, None) => {
                    if let (Some((volume, _)), Some(setup)) = (self.live_volume, self.setup) {
                        // A run that turns A: the stock it carved, seen in the
                        // setup's frame turned to the machine's A.
                        let frame = crate::volume::Frame::new(setup, self.mesh);
                        let current = match self.extra_turn() {
                            Some((axis, turn)) => frame.turned(axis, turn),
                            None => frame,
                        };
                        let scale = self.mesh.display_scale();
                        let mut outline = vec![];
                        for (points, color) in voxel_faces(volume, current) {
                            let points: Vec<_> =
                                points.iter().map(|v| v.map(|n| n * scale)).collect();
                            let points = match cut {
                                Some(cut) => {
                                    outline.extend(cut.crossing(&points));
                                    cut.clip(&points)
                                }
                                None => points,
                            };
                            let v: Vec<_> =
                                (points.iter()).map(|v| (screen(*v), p.depth(*v))).collect();
                            for i in 1..v.len().saturating_sub(1) {
                                raster_triangle(
                                    &mut rgba,
                                    &mut depths,
                                    w,
                                    h,
                                    [v[0], v[i], v[i + 1]],
                                    color,
                                );
                            }
                        }
                        outline
                    } else if let (Some((volume, surface)), Some(setup)) = (start, self.setup) {
                        let current = crate::volume::Frame::new(setup, self.mesh);
                        let (triangles, colors, outline) =
                            self.carved(volume, surface, current, p, ratio);
                        raster_triangles(&mut rgba, &mut depths, w, h, triangles.len(), |i| {
                            (triangles[i], colors[i])
                        });
                        outline
                    } else {
                        vec![]
                    }
                }
            };
            // The stock's cut face, and on it the outline of the part's.
            if let Some(plane) = cap_depth
                && !outline.is_empty()
            {
                let outline: Vec<_> = (outline.iter())
                    .map(|[a, b]| (screen(*a), screen(*b)))
                    .collect();
                raster_cap(
                    &mut rgba,
                    &mut depths,
                    (w, h),
                    &outline,
                    plane,
                    section_hatch(STOCK_CUT, ratio),
                    |_| {},
                );
                for [a, b] in &crossings {
                    raster_depth_line(
                        &mut rgba,
                        &depths,
                        w,
                        h,
                        (screen(*a), p.depth(*a)),
                        (screen(*b), p.depth(*b)),
                        PART_OUTLINE,
                        0.9 * ratio,
                    );
                }
            }
            for (fi, face) in faces.iter().enumerate() {
                if !behind_stock || self.uncut.get(fi) != Some(&true) {
                    continue;
                }
                let v = project(face);
                let color = uncut_color(face_light(&whole[fi]));
                let striped = selected_by(face);
                for i in 1..v.len().saturating_sub(1) {
                    raster_rows_with(
                        &mut rgba,
                        &mut depths,
                        w,
                        0..h,
                        [v[0], v[i], v[i + 1]],
                        |x, y| {
                            if striped { hatch(color, x, y) } else { color }
                        },
                    );
                }
            }
        } else if self.show_stock
            && let (Some((volume, surface)), Some(setup)) = (start, self.setup)
        {
            let current = crate::volume::Frame::new(setup, self.mesh);
            let (triangles, colors, _) = self.carved(volume, surface, current, p, ratio);
            raster_ghost(&mut rgba, &depths, (w, h), &triangles, &colors);
        } else if self.show_stock
            && let Some(setup) = self.setup
            && self.stock_is_known(setup)
            && let Some(stock) = self.resolved_stock()
        {
            // An indexed setup's stock is turned to the A shown. Turned by
            // A, it is not cut where the section is, which is set on it
            // unturned.
            let turned = self.turner();
            let cut = cut.filter(|_| self.extra_turn().is_none());
            let project = |v: [f32; 3]| {
                let v = turned(v);
                (screen(v), p.depth(v))
            };
            let solid_box = if let Some(bar) = stock::round(setup, stock.stock)
                .filter(|_| crate::model::get(&setup.p, "stock_source") == "fresh")
            {
                // A round bar along X: its polygon, corners outside the bar.
                let sides = 48;
                let r = bar.radius / (std::f32::consts::PI / sides as f32).cos();
                let ring = |x: f32| -> Vec<[f32; 3]> {
                    (0..sides)
                        .map(|i| {
                            let t = (i as f32 * 360. / sides as f32).to_radians();
                            [x, bar.axis[0] + r * t.cos(), bar.axis[1] + r * t.sin()]
                                .map(|n| n * stock.display_scale)
                        })
                        .collect()
                };
                let (a, b) = (ring(stock.stock.min[0]), ring(stock.stock.max[0]));
                prism_solid([&a, &b], true, cut.as_ref(), project)
            } else {
                box_solid(
                    stock.stock.scaled(stock.display_scale),
                    cut.as_ref(),
                    project,
                )
            };
            raster_stock(
                &mut rgba, &depths, w, h, &solid_box, ratio, p.scale, &mut solid,
            );
            // Only the planner's pictures mark surfaces (the app passes
            // none, so its view is unchanged): the model asked to see them,
            // so the translucent box must not dim them.
            for (index, _) in plain.iter().enumerate().filter(|(_, kept)| **kept) {
                rgba[index * 4..index * 4 + 4].copy_from_slice(&SELECTED);
            }
        }
        if !self.holding.is_empty() {
            let scale = self.mesh.display_scale();
            for hold in &self.holding {
                let project = |v: [f32; 3]| (screen(v), p.depth(v));
                let solid_box = match &hold.shape {
                    crate::holding::Shape::Hidden => continue,
                    // A chuck's turned solids.
                    crate::holding::Shape::Prism(prism) => {
                        let [a, b] = &prism.ends;
                        let (a, b): (Vec<_>, Vec<_>) = (
                            a.iter().map(|v| v.map(|n| n * scale)).collect(),
                            b.iter().map(|v| v.map(|n| n * scale)).collect(),
                        );
                        prism_solid([&a, &b], prism.smooth, cut.as_ref(), project)
                    }
                    crate::holding::Shape::Box => {
                        box_solid(hold.bounds.scaled(scale), cut.as_ref(), project)
                    }
                };
                raster_solid(
                    &HOLDING,
                    &mut rgba,
                    &depths,
                    (w, h),
                    &solid_box,
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
                    let Some((a, b)) = (match cut {
                        Some(cut) => cut.clip_segment(segment[0], segment[1]),
                        None => Some((segment[0], segment[1])),
                    }) else {
                        continue;
                    };
                    raster_line(
                        &mut rgba,
                        w,
                        h,
                        scaled(a),
                        scaled(b),
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
    /// The triangles of `surface`, what is left of the stock `volume`, moved
    /// to the setup's frame `current` and projected (each vertex once), and
    /// the colour of each (once), as the view draws them; less what the
    /// section cuts away, with where it crosses them (display units).
    fn carved(
        &self,
        volume: &crate::volume::Volume,
        surface: &crate::stock_surface::SurfaceMesh,
        current: crate::volume::Frame,
        p: &Projector,
        ratio: f32,
    ) -> (Vec<Corners>, Vec<[u8; 4]>, Vec<Segment>) {
        let scale = self.mesh.display_scale();
        let project = |v: [f32; 3]| {
            let pt = p.p(v);
            (Point::new(pt.x * ratio, pt.y * ratio), p.depth(v))
        };
        let corners = on_every_core(&surface.vertices, |&v| {
            let q = volume.frame.transfer(v, current);
            (q, project(q.map(|n| n * scale)))
        });
        let colors = on_every_core(&surface.triangles, |t| {
            stock_color(volume, surface, t, t.map(|i| corners[i].0))
        });
        let Some(cut) = self.cut() else {
            let triangles = (surface.triangles.iter())
                .map(|t| t.map(|v| corners[v].1))
                .collect();
            return (triangles, colors, vec![]);
        };
        let (mut triangles, mut kept, mut outline) = (vec![], vec![], vec![]);
        for (t, color) in surface.triangles.iter().zip(colors) {
            let points = t.map(|i| corners[i].0.map(|n| n * scale));
            match points.iter().filter(|v| cut.distance(**v) > 0.).count() {
                0 => {
                    triangles.push(t.map(|v| corners[v].1));
                    kept.push(color);
                }
                3 => {}
                _ => {
                    let left: Vec<_> = cut.clip(&points).into_iter().map(project).collect();
                    for i in 1..left.len().saturating_sub(1) {
                        triangles.push([left[0], left[i], left[i + 1]]);
                        kept.push(color);
                    }
                    outline.extend(cut.crossing(&points));
                }
            }
        }
        (triangles, kept, outline)
    }
    /// The simulation `sim` drawn: its stock, less what the section cuts
    /// away, the tool and the paths. Returns where the section crosses the
    /// stock (display units), the outline of its cut face.
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
    ) -> Vec<Segment> {
        let scale = sim.plan.display_scale;
        let shown = |v: [f32; 3]| {
            let q = p.p(v);
            (Point::new(q.x * ratio, q.y * ratio), p.depth(v))
        };
        let project = |v: [f32; 3]| shown(v.map(|n| n * scale));
        let cut = self.cut();
        let mut outline = vec![];
        // An indexed setup's stock is drawn turned to the A shown.
        let current = match (sim.frame, self.extra_turn()) {
            (Some(frame), Some((axis, turn))) => Some(frame.turned(axis, turn)),
            (frame, _) => frame,
        };
        if self.show_stock
            && let (Some(volume), Some(current)) = (&sim.volume, current)
            && let Some(surface) = &sim.stock_surface
        {
            let (triangles, colors, crossings) = self.carved(volume, surface, current, p, ratio);
            raster_triangles(rgba, depths, w, h, triangles.len(), |i| {
                (triangles[i], colors[i])
            });
            outline = crossings;
        }
        if self.show_stock && sim.volume.is_none() {
            let mut grid = |points: &[[f32; 3]], color: [u8; 4]| {
                let mut points: Vec<_> = points.iter().map(|v| v.map(|n| n * scale)).collect();
                if let Some(cut) = cut {
                    outline.extend(cut.crossing(&points));
                    points = cut.clip(&points);
                }
                let vertices: Vec<_> = points.into_iter().map(shown).collect();
                for i in 1..vertices.len().saturating_sub(1) {
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
            grid_faces(&sim.stock, &mut grid);
        }
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
        let pos = sim.position;
        let parts = cutter_parts(&sim.plan);
        for (below, above, from, to, base) in parts {
            let (bottom, top) = (pos[2] + from, pos[2] + to);
            let mut cap = Vec::new();
            let mut floor = Vec::new();
            // A flank facing down is in shade, one facing up in the light.
            let tilt = if above < below {
                1.08
            } else if above > below {
                0.82
            } else {
                1.
            };
            for i in 0..24 {
                let angle = i as f32 * std::f32::consts::TAU / 24.;
                let next = (i + 1) as f32 * std::f32::consts::TAU / 24.;
                let at =
                    |radius: f32, a: f32| [pos[0] + radius * a.cos(), pos[1] + radius * a.sin()];
                let (a0, b0, a1, b1) = (
                    at(below, angle),
                    at(below, next),
                    at(above, angle),
                    at(above, next),
                );
                let light = ((0.75 + 0.25 * (angle + 0.6).cos().abs()) * tilt).min(1.15);
                face(
                    &[
                        [a0[0], a0[1], bottom],
                        [b0[0], b0[1], bottom],
                        [b1[0], b1[1], top],
                        [a1[0], a1[1], top],
                    ],
                    [
                        (base[0] * light).min(255.) as u8,
                        (base[1] * light).min(255.) as u8,
                        (base[2] * light).min(255.) as u8,
                        255,
                    ],
                );
                cap.push([a1[0], a1[1], top]);
                floor.push([a0[0], a0[1], bottom]);
            }
            face(&cap, [base[0] as u8, base[1] as u8, base[2] as u8, 255]);
            face(
                &floor,
                [
                    (base[0] * 0.7) as u8,
                    (base[1] * 0.7) as u8,
                    (base[2] * 0.7) as u8,
                    255,
                ],
            );
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
                let (from, to) = (
                    movement.from.map(|n| n * scale),
                    movement.at(sim.seconds).map(|n| n * scale),
                );
                let Some((from, to)) = (match cut {
                    Some(cut) => cut.clip_segment(from, to),
                    None => Some((from, to)),
                }) else {
                    continue;
                };
                let (a, b) = (shown(from), shown(to));
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
        outline
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
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        vec![self.0.marks(renderer, bounds, cursor)]
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
    fn marks(
        &self,
        renderer: &Renderer,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> canvas::Geometry {
        let p = self.projector(bounds);
        let mut frame = Frame::new(renderer, bounds.size());
        self.hole_marks(&mut frame, bounds, cursor);
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
                } else if stock::round_bar(setup)
                    && crate::model::get(&setup.p, "stock_source") == "fresh"
                {
                    format!("Stock  Ø{:.1} × {:.1} mm bar", d[1].min(d[2]), d[0])
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
            // Text is not clipped to the view: off it, over the panels round
            // it, the label is left out.
            if corner.x >= 0.
                && corner.y >= 0.
                && corner.x + size.width <= bounds.width
                && corner.y + size.height <= bounds.height
            {
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
                    state.pivot = None;
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
                                    let (yaw, pitch) = (dx * 0.008, dy * 0.008);
                                    // The pan moves so the pivot stays where
                                    // it is on the screen as the view turns.
                                    // A wheel step during the drag: start again
                                    // from the camera the view has now.
                                    if state.pivot.is_some_and(|(_, c)| c.zoom != self.camera.zoom)
                                    {
                                        state.pivot = None;
                                    }
                                    let (pivot, camera) = state
                                        .pivot
                                        .get_or_insert_with(|| (self.pivot(bounds), self.camera));
                                    let size = self.framed_size(bounds);
                                    let scene = self.scene_bounds();
                                    let before = Projector::fit(*camera, size, scene).p(*pivot);
                                    let mut turned = orbited(*camera, yaw, pitch);
                                    let after = Projector::fit(turned, size, scene).p(*pivot);
                                    let shift = [before.x - after.x, before.y - after.y];
                                    turned.pan =
                                        [turned.pan[0] + shift[0], turned.pan[1] + shift[1]];
                                    *camera = turned;
                                    Event::Orbit(yaw, pitch, shift)
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
                            Action::publish(if self.section.picking {
                                Event::SectionSurface(f)
                            } else if self.sample {
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
                // From the view's centre as `Projector::new` puts it.
                let (width, height) = self.framed_size(bounds);
                let at = cursor.position_in(bounds).map_or([0., 0.], |p| {
                    [
                        p.x - (width * 0.5 + self.camera.pan[0]),
                        p.y - (height * 0.53 + self.camera.pan[1]),
                    ]
                });
                return Some(Action::publish(Event::Zoom(dy, at)).and_capture());
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
            if self.picking || self.section.picking {
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
/// Where each hole's label goes in the 3D view (`boxes`: each hole's
/// outline's box on the screen; `widths`: its label's length, None for an
/// unlabelled hole). The picked holes' first, then the rest, each where
/// `label_spot` finds room: beside its outline, or a little way out with a
/// leader line, clear of the other holes and labels. Else it is left out
/// (counted in the second value): the view then says how many are, and the
/// pointer over the hole shows it, for the hovered hole's label always goes,
/// laid out after the others so that pointing moves none of them, and last
/// in the list, so that it is drawn over them. So no label covers another
/// or another hole, or seems to be another hole's. Each placed label: its
/// hole, its place, and whether it is away from its hole.
pub(crate) fn place_labels(
    boxes: &[Rectangle],
    widths: &[Option<usize>],
    picked: &[bool],
    hovered: Option<usize>,
) -> (Vec<(usize, Rectangle, bool)>, usize) {
    // The labels without the pointer, so that pointing at a hole moves no
    // other label; then the hovered hole's, when it was left out.
    let mut order: Vec<usize> = (0..boxes.len()).filter(|&i| widths[i].is_some()).collect();
    order.sort_by_key(|&i| !picked[i]);
    let mut labels: Vec<(usize, Rectangle, bool)> = vec![];
    let mut hidden = 0;
    for &i in &order {
        let taken: Vec<Rectangle> = labels.iter().map(|l| l.1).collect();
        match label_spot(boxes, widths, i, &taken, false) {
            Some((spot, away)) => labels.push((i, spot, away)),
            None => hidden += 1,
        }
    }
    if let Some(at) = hovered.and_then(|h| labels.iter().position(|l| l.0 == h)) {
        // Shown already: last, drawn over the others.
        let l = labels.remove(at);
        labels.push(l);
    } else if let Some(h) = hovered
        && widths.get(h).is_some_and(|w| w.is_some())
    {
        let taken: Vec<Rectangle> = labels.iter().map(|l| l.1).collect();
        if let Some((spot, away)) = label_spot(boxes, widths, h, &taken, true) {
            labels.push((h, spot, away));
            hidden -= 1;
        }
    }
    (labels, hidden)
}
/// How far a label keeps from another hole's outline on the screen: so near
/// it, it would seem to be that hole's.
const LABEL_GAP: f32 = 4.;
/// Where hole `i`'s label goes, given the labels `taken` (`place_labels`),
/// and whether that is away from its hole, with a leader line. Beside it,
/// right or left, clear of its own outline, `LABEL_GAP` from every other
/// labelled hole's and off every label taken; else 20 or 32 points out in one
/// of eight directions, its leader line crossing no other labelled hole and
/// no label. None if no place is; but `anyway` (the hovered hole) takes any
/// free place whatever its line crosses, and else the place beside it, over
/// whatever is there.
fn label_spot(
    boxes: &[Rectangle],
    widths: &[Option<usize>],
    i: usize,
    taken: &[Rectangle],
    anyway: bool,
) -> Option<(Rectangle, bool)> {
    let width = widths[i]?;
    let hole = boxes[i];
    let size = iced::Size::new(width as f32 * 6.6 + 6., 15.);
    let (cx, cy) = (hole.x + hole.width / 2., hole.y + hole.height / 2.);
    let beside = [
        Point::new(hole.x + hole.width + 2., cy - size.height / 2.),
        Point::new(hole.x - 2. - size.width, cy - size.height / 2.),
    ];
    let mut spots: Vec<(Point, bool)> = beside.iter().map(|&p| (p, false)).collect();
    for reach in [20., 32.] {
        for k in 0..8 {
            let a = k as f32 * std::f32::consts::FRAC_PI_4;
            let (x, y) = (cx + reach * a.cos(), cy - reach * a.sin());
            spots.push((Point::new(x - size.width / 2., y - size.height / 2.), true));
        }
    }
    let apart = |a: &Rectangle, b: &Rectangle| {
        a.x + a.width <= b.x
            || b.x + b.width <= a.x
            || a.y + a.height <= b.y
            || b.y + b.height <= a.y
    };
    let others = (boxes.iter().enumerate())
        .filter(|&(j, _)| j != i && widths[j].is_some())
        .map(|(_, b)| *b);
    let near: Vec<Rectangle> = others
        .clone()
        .map(|b| {
            Rectangle::new(
                Point::new(b.x - LABEL_GAP, b.y - LABEL_GAP),
                iced::Size::new(b.width + 2. * LABEL_GAP, b.height + 2. * LABEL_GAP),
            )
        })
        .collect();
    let free = |at: Point| {
        let r = Rectangle::new(at, size);
        apart(&r, &hole) && near.iter().chain(taken).all(|b| apart(&r, b))
    };
    // A leader line crosses no other hole (it would seem to point at that
    // one) and no other label.
    let others: Vec<Rectangle> = others.collect();
    let clear = |at: Point| {
        let from = Point::new(cx, cy);
        let to = leader_end(from, Rectangle::new(at, size));
        (0..=20).all(|k| {
            let t = k as f32 / 20.;
            let q = Point::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
            others.iter().chain(taken).all(|b| !b.contains(q))
        })
    };
    let found = (spots.iter())
        .find(|&&(at, away)| free(at) && (!away || clear(at)))
        .or_else(|| {
            anyway
                .then(|| spots.iter().find(|&&(at, _)| free(at)))
                .flatten()
        })
        .copied()
        .or(anyway.then_some((beside[0], false)));
    found.map(|(at, away)| (Rectangle::new(at, size), away))
}
/// What the view says when `n` holes' labels are left out where holes are
/// close together.
pub(crate) fn hidden_note(n: usize) -> String {
    format!(
        "{} hidden where holes are close: zoom in, or point at a hole",
        if n == 1 {
            "1 hole's size".to_string()
        } else {
            format!("{n} holes' sizes")
        }
    )
}
/// Where a leader line from `from` meets the label `spot`: its nearest point.
fn leader_end(from: Point, spot: Rectangle) -> Point {
    Point::new(
        from.x.clamp(spot.x, spot.x + spot.width),
        from.y.clamp(spot.y, spot.y + spot.height),
    )
}
/// How near a hole's outline the pointer picks it, in screen points.
const OUTLINE_REACH: f32 = 4.;
/// How far `p` is from the segment `a`–`b`, on the screen.
fn segment_distance(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length = dx * dx + dy * dy;
    let t = if length > 0. {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / length).clamp(0., 1.)
    } else {
        0.
    };
    p.distance(Point::new(a.x + t * dx, a.y + t * dy))
}
/// Whether `p` is inside the closed loop `l`, on the screen (even-odd).
fn inside(p: Point, l: &[Point]) -> bool {
    let mut odd = false;
    for w in l.windows(2) {
        let (a, b) = (w[0], w[1]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            odd = !odd;
        }
    }
    odd
}
/// The area the closed loop `l` covers on the screen.
fn screen_area(l: &[Point]) -> f32 {
    (l.windows(2)
        .map(|w| w[0].x * w[1].y - w[1].x * w[0].y)
        .sum::<f32>()
        / 2.)
        .abs()
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
/// `shade` for a cut face of the stock, whose corners run counterclockwise
/// seen from outside it: lit by whether it faces up or down, so the faces
/// under an overhang, a thread's lower flanks, are in shade and its teeth
/// read; a floor and a wall shade as `shade` has them.
fn shade_cut(points: &[[f32; 3]]) -> [u8; 4] {
    let n = normal(points);
    let light = 0.6 + 0.28 * n[2] + 0.12 * n[0].abs();
    let [r, g, b] = CUT.map(|c| (c * light) as u8);
    [r, g, b, 255]
}
/// The faces between the voxels of `volume` that hold material and those
/// that do not (or the outside), as quads in the frame `current` (mm), each
/// with the colour the view gives stock: the block's own colour where the
/// side beyond it never held material, the cut colour where it was cut.
pub(crate) fn voxel_faces(
    volume: &crate::volume::Volume,
    current: crate::volume::Frame,
) -> Vec<([[f32; 3]; 4], [u8; 4])> {
    let n = volume.n;
    let filled = |c: [i64; 3]| {
        (0..3).all(|k| c[k] >= 0 && (c[k] as usize) < n[k])
            && volume.occupied[volume.index(c.map(|v| v as usize))] != 0
    };
    // Whether the voxel at `c` was ever material: inside the block, and
    // the bar when it is one.
    let stock = |c: [i64; 3]| {
        (0..3).all(|k| c[k] >= 0 && (c[k] as usize) < n[k])
            && volume
                .bar_x
                .is_none_or(|bar| bar.contains(volume.point(c.map(|v| v as usize), 0.5)))
    };
    let corner = |c: [i64; 3]| {
        let p = std::array::from_fn(|k| volume.bounds.min[k] + c[k] as f32 * volume.cell[k]);
        volume.frame.transfer(p, current)
    };
    let mut out = vec![];
    for z in 0..n[2] as i64 {
        for y in 0..n[1] as i64 {
            for x in 0..n[0] as i64 {
                let c = [x, y, z];
                if !filled(c) {
                    continue;
                }
                for k in 0..3 {
                    for side in [0, 1] {
                        let mut beyond = c;
                        beyond[k] += if side == 0 { -1 } else { 1 };
                        if filled(beyond) {
                            continue;
                        }
                        let (u, v) = ((k + 1) % 3, (k + 2) % 3);
                        let at = |du: i64, dv: i64| {
                            let mut q = c;
                            q[k] += side;
                            q[u] += du;
                            q[v] += dv;
                            corner(q)
                        };
                        let quad = [at(0, 0), at(1, 0), at(1, 1), at(0, 1)];
                        let base = if stock(beyond) { CUT } else { BLOCK };
                        out.push((quad, shade(&quad, base)));
                    }
                }
            }
        }
    }
    out
}
/// The triangles of `surface`, the simulated stock of `volume`, in the frame
/// `current` (mm), each with the colour the view gives it.
pub(crate) fn stock_triangles<'a>(
    volume: &'a crate::volume::Volume,
    surface: &'a crate::stock_surface::SurfaceMesh,
    current: crate::volume::Frame,
) -> impl Iterator<Item = ([[f32; 3]; 3], [u8; 4])> + 'a {
    surface.triangles.iter().map(move |triangle| {
        let points = triangle.map(|i| volume.frame.transfer(surface.vertices[i], current));
        (points, stock_color(volume, surface, triangle, points))
    })
}
/// The colour the view gives `triangle` of `surface`, whose corners are
/// `points` in the frame it is seen in: the block's where it lies on the
/// stock's own faces, else the cut's, shaded.
fn stock_color(
    volume: &crate::volume::Volume,
    surface: &crate::stock_surface::SurfaceMesh,
    triangle: &[usize; 3],
    points: [[f32; 3]; 3],
) -> [u8; 4] {
    let source = triangle.map(|i| surface.vertices[i]);
    let outer = (0..3).any(|k| {
        source
            .iter()
            .all(|p| (p[k] - volume.bounds.min[k]).abs() < 0.01)
            || source
                .iter()
                .all(|p| (p[k] - volume.bounds.max[k]).abs() < 0.01)
    }) || volume.round && on_bar(volume.bounds, &source, surface.cell_mm)
        || volume.bar_x.is_some_and(|bar| {
            // A bar along X's own surface is the block's too: its points lie
            // within half a cell of the meshing of it.
            source.iter().all(|p| {
                let r = (p[1] - bar.axis[0]).hypot(p[2] - bar.axis[1]);
                (r - bar.radius).abs() <= surface.cell_mm / 2.
            })
        });
    if outer {
        shade(&points, BLOCK)
    } else {
        shade_cut(&points)
    }
}
/// `f` of each of `items`, in order, worked out on every core.
fn on_every_core<T: Sync, U: Send>(items: &[T], f: impl Fn(&T) -> U + Sync) -> Vec<U> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    if threads == 1 || items.len() < 4096 {
        return items.iter().map(f).collect();
    }
    let f = &f;
    std::thread::scope(|scope| {
        let parts: Vec<_> = (items.chunks(items.len().div_ceil(threads)))
            .map(|part| scope.spawn(move || part.iter().map(f).collect::<Vec<_>>()))
            .collect();
        (parts.into_iter())
            .flat_map(|part| part.join().unwrap())
            .collect()
    })
}
/// Whether every vertex of `triangle` lies on the cylinder of the round bar
/// whose box is `bounds` (`stock::bar`); its end faces are the box's. Dual
/// contouring puts a curved face's vertices within a few hundredths of a
/// mesh cell (`cell`, mm) of it, so 6 % of a cell counts, at least 0.01 mm
/// as on the box's faces: under 0.05 mm at Draft, half a 0.2 mm finish pass.
fn on_bar(bounds: crate::mesh::Bounds, triangle: &[[f32; 3]; 3], cell: f32) -> bool {
    let (centre, radius) = stock::bar(bounds);
    let tolerance = (0.06 * cell).max(0.01);
    triangle
        .iter()
        .all(|p| ((p[0] - centre[0]).hypot(p[1] - centre[1]) - radius).abs() < tolerance)
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
    let projected = on_every_core(triangles, |(t, color)| {
        (t.map(|v| (p.p(v), p.depth(v))), *color)
    });
    raster_triangles(&mut rgba, &mut depths, w, h, projected.len(), |i| {
        projected[i]
    });
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
/// How wide each stripe of `hatch` is, in logical pixels.
const HATCH_STRIPE: f32 = 4.;
/// The colour at pixel (x, y) of a selected face still under stock, whose
/// magenta is `uncut`: diagonal stripes of it and the selection's blue, so
/// the face reads as both, at a frame of `ratio` pixels a logical pixel.
fn hatch(ratio: f32) -> impl Fn([u8; 4], u32, u32) -> [u8; 4] + Copy {
    let stripe = (HATCH_STRIPE * ratio).max(2.);
    move |uncut, x, y| {
        if (((x + y) as f32 / stripe) as u32).is_multiple_of(2) {
            uncut
        } else {
            SELECTED
        }
    }
}
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
    solid_box: &SolidBox,
    ratio: f32,
    scale: f32,
    buffers: &mut SolidBuffers,
) {
    raster_solid(
        &STOCK,
        pixels,
        part_depth,
        (w, h),
        solid_box,
        ratio,
        scale,
        buffers,
    );
}
/// The faces of a box by its corners (`box_vertices`): bottom, top, −Y,
/// +X, +Y and −X, each one's tone (`Look::faces`) its place here.
const BOX_FACES: [[usize; 4]; 6] = [
    [0, 1, 2, 3],
    [4, 5, 6, 7],
    [0, 1, 5, 4],
    [1, 2, 6, 5],
    [2, 3, 7, 6],
    [3, 0, 4, 7],
];
/// The edges of a box by its corners (`box_vertices`).
const BOX_EDGES: [(usize, usize); 12] = [
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
];
/// The tone of a box's cut face (`Look::faces`, as the X and Y sides').
const CUT_TONE: u8 = 6;
/// A box as `raster_solid` draws it, projected: its faces, each a convex
/// polygon with its tone, and its edges.
struct SolidBox {
    faces: Vec<(Vec<(Point, f32)>, u8)>,
    edges: Vec<((Point, f32), (Point, f32))>,
}
/// The box `b` as `raster_solid` draws it, its corners projected by
/// `project`: what `cut` leaves of it, closed by the box's cut face and that
/// face's edges.
fn box_solid(
    b: crate::mesh::Bounds,
    cut: Option<&Plane>,
    project: impl Fn([f32; 3]) -> (Point, f32),
) -> SolidBox {
    let v = box_vertices(b);
    let mut faces = vec![];
    for (tone, face) in BOX_FACES.iter().enumerate() {
        let points = face.map(|i| v[i]);
        let points = match cut {
            Some(cut) => cut.clip(&points),
            None => points.to_vec(),
        };
        if points.len() >= 3 {
            faces.push((points.into_iter().map(&project).collect(), tone as u8));
        }
    }
    let mut edges = vec![];
    for (a, b) in BOX_EDGES {
        let kept = match cut {
            Some(cut) => cut.clip_segment(v[a], v[b]),
            None => Some((v[a], v[b])),
        };
        if let Some((a, b)) = kept {
            edges.push((project(a), project(b)));
        }
    }
    if let Some(cut) = cut {
        let face = cut.box_face(b);
        for (i, a) in face.iter().enumerate() {
            edges.push((project(*a), project(face[(i + 1) % face.len()])));
        }
        if face.len() >= 3 {
            faces.push((face.into_iter().map(&project).collect(), CUT_TONE));
        }
    }
    SolidBox { faces, edges }
}
/// A turned solid (a chuck's `chuck::Prism`, a round bar) as `raster_solid`
/// draws it, its points projected by `project`: what `cut` leaves of its two
/// end polygons and its sides, with the ends' edges and, for a round one,
/// only the sides' edges on its outline. Unlike a box's, its cut face is
/// left open.
fn prism_solid(
    ends: [&[[f32; 3]]; 2],
    smooth: bool,
    cut: Option<&Plane>,
    project: impl Fn([f32; 3]) -> (Point, f32),
) -> SolidBox {
    let mut solid = SolidBox {
        faces: vec![],
        edges: vec![],
    };
    let n = ends[0].len().min(ends[1].len());
    if n < 3 {
        return solid;
    }
    let mut face = |points: &[[f32; 3]], tone: u8| {
        let points = match cut {
            Some(cut) => cut.clip(points),
            None => points.to_vec(),
        };
        if points.len() >= 3 {
            solid
                .faces
                .push((points.into_iter().map(&project).collect(), tone));
        }
    };
    face(&ends[0][..n], 0);
    face(&ends[1][..n], 1);
    // Whether each side faces the camera, by its winding on the screen.
    let mut facing = vec![];
    for i in 0..n {
        let j = (i + 1) % n;
        let quad = [ends[0][i], ends[0][j], ends[1][j], ends[1][i]];
        let area: f32 = (0..4)
            .map(|k| {
                let (a, b) = (project(quad[k]).0, project(quad[(k + 1) % 4]).0);
                a.x * b.y - b.x * a.y
            })
            .sum();
        facing.push(area > 0.);
        face(&quad, if smooth { 5 } else { 2 + (i % 2) as u8 * 2 });
    }
    let mut edge = |a: [f32; 3], b: [f32; 3]| {
        let kept = match cut {
            Some(cut) => cut.clip_segment(a, b),
            None => Some((a, b)),
        };
        if let Some((a, b)) = kept {
            solid.edges.push((project(a), project(b)));
        }
    };
    for i in 0..n {
        let j = (i + 1) % n;
        edge(ends[0][i], ends[0][j]);
        edge(ends[1][i], ends[1][j]);
        let before = (i + n - 1) % n;
        if !smooth || facing[i] != facing[before] {
            edge(ends[0][i], ends[1][i]);
        }
    }
    solid
}
/// Composite only the nearest surface of a box; use both surfaces and part
/// depth to distinguish visible edges from dashed hidden edges.
#[allow(clippy::too_many_arguments)]
fn raster_solid(
    look: &Look,
    pixels: &mut [u8],
    part_depth: &[f32],
    (w, h): (u32, u32),
    solid_box: &SolidBox,
    ratio: f32,
    scale: f32,
    buffers: &mut SolidBuffers,
) {
    buffers.reset((w * h) as usize);
    let SolidBuffers { near, tones } = buffers;
    for (face, tone) in &solid_box.faces {
        for i in 1..face.len() - 1 {
            let tri = [face[0], face[i], face[i + 1]];
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
                        tones[index] = *tone;
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
    for &((a, za), (b, zb)) in &solid_box.edges {
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
/// The projected `triangles` of a stock's surface, each in its colour in
/// `colors`, laid see-through over `pixels` as the stock's box is: only the
/// nearest surface, where it is not behind the part (`part_depth`), lighter
/// over the part than over empty space.
fn raster_ghost(
    pixels: &mut [u8],
    part_depth: &[f32],
    (w, h): (u32, u32),
    triangles: &[Corners],
    colors: &[[u8; 4]],
) {
    let mut near = vec![0u8; (w * h * 4) as usize];
    let mut depths = vec![f32::NEG_INFINITY; (w * h) as usize];
    raster_triangles(&mut near, &mut depths, w, h, triangles.len(), |i| {
        (triangles[i], colors[i])
    });
    for (index, z) in depths.iter().enumerate() {
        if z.is_finite() && *z >= part_depth[index] - 0.001 {
            let alpha = if part_depth[index].is_finite() {
                STOCK.behind
            } else {
                STOCK.open
            };
            let tint = [near[index * 4], near[index * 4 + 1], near[index * 4 + 2]];
            alpha_over(&mut pixels[index * 4..index * 4 + 4], tint, alpha);
        }
    }
}
/// The fill and the hatch lines of the part's cut face: a lighter steel
/// than its faces, hatched dark.
const PART_CUT: [[u8; 3]; 2] = [[198, 212, 228], [64, 88, 118]];
/// The fill and the hatch lines of the stock's cut face: the block's tan,
/// hatched brown.
const STOCK_CUT: [[u8; 3]; 2] = [[232, 204, 150], [150, 104, 40]];
/// The outline of the part's cut face drawn on the stock's.
const PART_OUTLINE: [u8; 4] = [40, 64, 96, 255];
/// The colour at pixel (x, y) of a cut face, whose fill and hatch lines
/// are `colors`: thin lines at 45° across the fill, as a drawing hatches a
/// section, at a frame of `ratio` pixels a logical pixel.
fn section_hatch(colors: [[u8; 3]; 2], ratio: f32) -> impl Fn(u32, u32) -> [u8; 4] + Copy {
    let spacing = (7. * ratio).max(4.);
    let width = ratio.max(1.);
    move |x, y| {
        let [r, g, b] = if (x as f32 + y as f32).rem_euclid(spacing) < width {
            colors[1]
        } else {
            colors[0]
        };
        [r, g, b, 255]
    }
}
/// The cut face of a solid the section goes through: filled even-odd
/// between the segments of `outline` (where the plane crosses the solid's
/// faces, in the frame's pixels), at the plane's depth `plane` (a·x + b·y +
/// c, `Projector::plane_depth`), wherever nothing drawn is nearer. Each
/// pixel drawn is `color`'s, and `drawn` is told its index.
fn raster_cap(
    pixels: &mut [u8],
    depths: &mut [f32],
    (w, h): (u32, u32),
    outline: &[(Point, Point)],
    plane: [f32; 3],
    color: impl Fn(u32, u32) -> [u8; 4],
    mut drawn: impl FnMut(usize),
) {
    // Where each row's centre crosses the outline: a segment takes the rows
    // whose centres are from its upper end to just short of its lower one,
    // so two segments meeting at a corner count it once.
    let mut rows: Vec<Vec<f32>> = vec![vec![]; h as usize];
    for &(a, b) in outline {
        if a.y == b.y || !(a.y.is_finite() && b.y.is_finite()) {
            continue;
        }
        let (a, b) = if a.y < b.y { (a, b) } else { (b, a) };
        let first = (a.y - 0.5).ceil().clamp(0., h as f32) as usize;
        let last = (b.y - 0.5).ceil().clamp(0., h as f32) as usize;
        for (y, row) in rows.iter_mut().enumerate().take(last).skip(first) {
            let t = (y as f32 + 0.5 - a.y) / (b.y - a.y);
            row.push(a.x + (b.x - a.x) * t);
        }
    }
    for (y, row) in rows.iter_mut().enumerate() {
        row.sort_by(f32::total_cmp);
        for &[a, b] in row.as_chunks::<2>().0 {
            let from = (a - 0.5).ceil().clamp(0., w as f32) as u32;
            let to = (b - 0.5).ceil().clamp(0., w as f32) as u32;
            let y = y as u32;
            for x in from..to {
                let index = (y * w + x) as usize;
                let z = plane[0] * (x as f32 + 0.5) + plane[1] * (y as f32 + 0.5) + plane[2];
                if z >= depths[index] {
                    depths[index] = z;
                    pixels[index * 4..index * 4 + 4].copy_from_slice(&color(x, y));
                    drawn(index);
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
    raster_rows(pixels, depths, w, 0..h, vertices, color);
}
/// A triangle's corners projected to the frame: their pixels and depths.
type Corners = [(Point, f32); 3];
/// A triangle projected to the frame, and its colour.
type Projected = (Corners, [u8; 4]);
/// `triangles` drawn as `raster_triangle` draws each in turn, on every core:
/// each draws a band of rows, taking the triangles in the same order, so the
/// pixels are the same.
fn raster_triangles(
    pixels: &mut [u8],
    depths: &mut [f32],
    w: u32,
    h: u32,
    count: usize,
    triangle: impl Fn(usize) -> Projected + Sync,
) {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()) as u32;
    if threads == 1 || count < 4096 || w == 0 {
        for i in 0..count {
            let (vertices, color) = triangle(i);
            raster_triangle(pixels, depths, w, h, vertices, color);
        }
        return;
    }
    let band = h.div_ceil(threads).max(1);
    let bands = h.div_ceil(band) as usize;
    let length = count.div_ceil(threads as usize);
    let triangle = &triangle;
    // The triangles reaching each band, sorted out a part of the list a core.
    let parts: Vec<Vec<Vec<u32>>> = std::thread::scope(|scope| {
        let sorting: Vec<_> = (0..count)
            .step_by(length)
            .map(|first| {
                scope.spawn(move || {
                    let mut reaching = vec![vec![]; bands];
                    for i in first..(first + length).min(count) {
                        let (low, high) = (triangle(i).0.iter())
                            .fold((f32::INFINITY, f32::NEG_INFINITY), |(l, h), v| {
                                (l.min(v.0.y), h.max(v.0.y))
                            });
                        // The rows `raster_rows` draws.
                        let top = low.floor().clamp(0., h as f32) as u32;
                        let bottom = high.ceil().clamp(0., h as f32) as u32;
                        if top < bottom {
                            for b in (top / band)..=((bottom - 1) / band) {
                                reaching[b as usize].push(i as u32);
                            }
                        }
                    }
                    reaching
                })
            })
            .collect();
        (sorting.into_iter()).map(|s| s.join().unwrap()).collect()
    });
    std::thread::scope(|scope| {
        let rows = (pixels.chunks_mut((band * w * 4) as usize))
            .zip(depths.chunks_mut((band * w) as usize));
        for (b, (pixels, depths)) in rows.enumerate() {
            let top = b as u32 * band;
            let rows = top..top + depths.len() as u32 / w;
            let parts = &parts;
            scope.spawn(move || {
                for i in parts.iter().flat_map(|reaching| &reaching[b]) {
                    let (vertices, color) = triangle(*i as usize);
                    raster_rows(pixels, depths, w, rows.clone(), vertices, color);
                }
            });
        }
    });
}
/// `raster_triangle` over the frame's rows `rows` alone, whose pixels and
/// depths `pixels` and `depths` hold.
fn raster_rows(
    pixels: &mut [u8],
    depths: &mut [f32],
    w: u32,
    rows: std::ops::Range<u32>,
    vertices: [(Point, f32); 3],
    color: [u8; 4],
) {
    raster_rows_with(pixels, depths, w, rows, vertices, |_, _| color);
}
/// `raster_rows` with the colour of each pixel (x, y) its own.
fn raster_rows_with(
    pixels: &mut [u8],
    depths: &mut [f32],
    w: u32,
    rows: std::ops::Range<u32>,
    vertices: [(Point, f32); 3],
    color: impl Fn(u32, u32) -> [u8; 4],
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
        .clamp(rows.start as f32, rows.end as f32) as u32;
    let maxy = vertices
        .iter()
        .map(|v| v.0.y)
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .clamp(rows.start as f32, rows.end as f32) as u32;
    for y in miny..maxy {
        for x in minx..maxx {
            let index = ((y - rows.start) * w + x) as usize;
            if let Some(z) = triangle_depth(vertices, Point::new(x as f32 + 0.5, y as f32 + 0.5))
                && z >= depths[index]
            {
                depths[index] = z;
                pixels[index * 4..index * 4 + 4].copy_from_slice(&color(x, y));
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
    let Some((t0, t1)) = in_frame(a.0, b.0, w, h, ratio + 2.) else {
        return;
    };
    let z = |t: f32| a.1 + (b.1 - a.1) * t;
    let (a, b) = ((along(a.0, b.0, t0), z(t0)), (along(a.0, b.0, t1), z(t1)));
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
/// The part of the segment `a`–`b` within the `w` × `h` frame grown by
/// `margin` on each side, as its ends' fractions of the way from `a`; None
/// when none of it is. A zoomed-in view stretches a move far past the
/// frame: only what shows is stepped along.
fn in_frame(a: Point, b: Point, w: u32, h: u32, margin: f32) -> Option<(f32, f32)> {
    let (mut t0, mut t1) = (0f32, 1f32);
    let d = [b.x - a.x, b.y - a.y];
    for (start, delta, low, high) in [
        (a.x, d[0], -margin, w as f32 + margin),
        (a.y, d[1], -margin, h as f32 + margin),
    ] {
        if delta.abs() < 1e-9 {
            if start < low || start > high {
                return None;
            }
            continue;
        }
        let (u, v) = ((low - start) / delta, (high - start) / delta);
        t0 = t0.max(u.min(v));
        t1 = t1.min(u.max(v));
    }
    (t0 <= t1).then_some((t0, t1))
}
/// The point `t` of the way from `a` to `b`.
fn along(a: Point, b: Point, t: f32) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}
fn raster_line(pixels: &mut [u8], w: u32, h: u32, a: Point, b: Point, color: [u8; 4], width: f32) {
    let Some((t0, t1)) = in_frame(a, b, w, h, width + 2.) else {
        return;
    };
    let (a, b) = (along(a, b, t0), along(a, b, t1));
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
    #[test]
    fn a_chamfer_mill_and_a_drill_are_drawn_with_their_cone() {
        use crate::simulation::{CutterShape, Motion, POINT_RISE};
        let mut plan =
            crate::post::fixture::plan([0., 0., 5.], &[(Motion::Feed, [9., 0., 5.], 600.)]);
        plan.radius = 3.;
        plan.flute_length = 10.;
        // A 60° chamfer mill with a 0.2 flat: from the flat up to Ø6, 2.9 ×
        // cot 30° tall, then its flutes, its body and its holder.
        plan.shape = CutterShape::Chamfer(600, 200);
        let parts = cutter_parts(&plan);
        let (below, above, from, to, _) = parts[0];
        let tall = 2.9 / 30f32.to_radians().tan();
        assert!(
            (below - 0.1).abs() < 1e-5 && (above - 3.).abs() < 1e-4,
            "{parts:?}"
        );
        assert!(from == 0. && (to - tall).abs() < 1e-4, "{parts:?}");
        assert_eq!((parts[1].0, parts[1].1, parts[1].3), (3., 3., 10.));
        // A drill: its 118° point under its flutes.
        plan.shape = CutterShape::Drill;
        let parts = cutter_parts(&plan);
        assert!(parts[0].0 == 0. && (parts[0].3 - 3. * POINT_RISE).abs() < 1e-5);
        // A cone longer than the flutes is drawn to the flutes' end.
        plan.shape = CutterShape::Chamfer(200, 0);
        let parts = cutter_parts(&plan);
        assert!(parts[0].3 == 10. && parts[0].1 < 3., "{parts:?}");
        // A flat end mill is still a cylinder from its tip.
        plan.shape = CutterShape::Flat;
        assert_eq!(cutter_parts(&plan)[0].0, 3.);
    }
    /// FNV-1a over the pixels, to pin the drawing across the refactor.
    fn digest(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf29ce484222325, |h, b| {
            (h ^ *b as u64).wrapping_mul(0x100000001b3)
        })
    }
    /// A section cuts a turned solid (a round bar on A, a chuck's) as it
    /// cuts a box: nothing drawn of it is left past the plane, and a round
    /// one still has its outline edges.
    #[test]
    fn a_section_cuts_a_turned_solid_as_it_cuts_a_box() {
        let ring = |x: f32| -> Vec<[f32; 3]> {
            (0..24)
                .map(|i| {
                    let t = (i as f32 * 15.).to_radians();
                    [x, 10. * t.cos(), 10. + 10. * t.sin()]
                })
                .collect()
        };
        let (a, b) = (ring(-30.), ring(30.));
        // Seen from above: screen X, Y are the solid's, its depth is Z.
        let project = |v: [f32; 3]| (Point::new(v[0], v[1]), v[2]);
        let whole = prism_solid([&a, &b], true, None, project);
        assert_eq!(whole.faces.len(), 2 + 24);
        let x = |solid: &SolidBox| {
            let points = (solid.faces.iter().flat_map(|(f, _)| f))
                .chain(solid.edges.iter().flat_map(|(p, q)| [p, q]));
            points.fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), (p, _)| {
                (lo.min(p.x), hi.max(p.x))
            })
        };
        assert_eq!(x(&whole), (-30., 30.));
        // Cut away past X 10: the +X end goes, the sides are shortened.
        let cut = Plane {
            normal: [1., 0., 0.],
            d: 10.,
        };
        let half = prism_solid([&a, &b], true, Some(&cut), project);
        assert_eq!(half.faces.len(), 1 + 24);
        let (lo, hi) = x(&half);
        assert!(lo == -30. && (hi - 10.).abs() < 1e-4, "{lo} {hi}");
        // A round solid keeps its two end rings and its outline seen from
        // here, not an edge down every side.
        assert!(half.edges.len() < 24 + 24, "{}", half.edges.len());
        assert!(half.edges.len() > 24);
        // Nothing left at all when the plane takes the whole solid.
        let gone = Plane {
            normal: [1., 0., 0.],
            d: -40.,
        };
        let none = prism_solid([&a, &b], false, Some(&gone), project);
        assert!(none.faces.is_empty() && none.edges.is_empty());
    }
    #[test]
    fn stock_drawn_on_every_core_is_the_same_as_one_triangle_at_a_time() {
        let mesh =
            crate::mesh::normalize(vec![[[0., 0., 0.], [20., 0., 0.], [0., 20., 10.]]]).unwrap();
        let frame = crate::volume::Frame::new(
            &crate::model::Setup::new(crate::model::Machine::new(3), "G54"),
            &mesh,
        );
        let bounds = crate::mesh::Bounds {
            min: [-30., -20., 0.],
            max: [30., 20., 15.],
        };
        let mut v = crate::volume::Volume::new(bounds, frame);
        for i in 0..12 {
            let y = -15. + i as f32 * 2.5;
            let z = 13. - i as f32 * 0.6;
            use crate::simulation::CutterShape;
            let shape = [CutterShape::Flat, CutterShape::Ball][i % 2];
            v.cut(
                [-25., y, z],
                [25., y + 1., z - 1.],
                3.,
                12.,
                shape,
                frame,
                (0, 0),
            );
        }
        let surface =
            crate::stock_surface::Snapshot::of(&v, crate::stock_surface::Detail::Standard)
                .build()
                .unwrap();
        let (w, h) = (900, 700);
        let p = Projector::filling(Camera::default(), (w as f32, h as f32), &[bounds], 0.9);
        let triangles: Vec<_> = stock_triangles(&v, &surface, frame)
            .map(|(t, color)| (t.map(|q| (p.p(q), p.depth(q))), color))
            .collect();
        assert!(triangles.len() > 4096, "{} triangles", triangles.len());
        let blank = || {
            (
                vec![0u8; (w * h * 4) as usize],
                vec![f32::NEG_INFINITY; (w * h) as usize],
            )
        };
        let (mut one, mut one_depths) = blank();
        for (t, color) in &triangles {
            raster_triangle(&mut one, &mut one_depths, w, h, *t, *color);
        }
        let (mut all, mut all_depths) = blank();
        raster_triangles(&mut all, &mut all_depths, w, h, triangles.len(), |i| {
            triangles[i]
        });
        assert!(one.chunks(4).filter(|px| px[3] != 0).count() > (w * h / 4) as usize);
        assert!(one == all);
        assert!((one_depths.iter().zip(&all_depths)).all(|(a, b)| a.to_bits() == b.to_bits()));
    }
    #[test]
    fn a_round_bars_cylinder_and_ends_are_block_and_its_turned_faces_cut() {
        let mesh =
            crate::mesh::normalize(vec![[[0., 0., 0.], [20., 0., 0.], [0., 20., 10.]]]).unwrap();
        let frame = crate::volume::Frame::new(
            &crate::model::Setup::new(crate::model::Machine::new(3), "G54"),
            &mesh,
        );
        let bar = crate::mesh::Bounds {
            min: [-25., -25., 0.],
            max: [25., 25., 100.],
        };
        let mut v = crate::volume::Volume::bar(bar, frame);
        v.revolve(50., 101., 20., 26., frame, (0, 0));
        let surface = crate::stock_surface::Snapshot::of(&v, crate::stock_surface::Detail::Draft)
            .build()
            .unwrap();
        let (mut block, mut cut) = (0, 0);
        for (points, color) in stock_triangles(&v, &surface, frame) {
            let radii = points.map(|p| p[0].hypot(p[1]));
            let z = points.map(|p| p[2]);
            if radii.iter().all(|r| (r - 25.).abs() < 0.05) && z.iter().all(|z| *z < 49.) {
                // The bar's own cylinder, below the turned part.
                assert_eq!(color, shade(&points, BLOCK), "{points:?}");
                block += 1;
            } else if z.iter().all(|z| z.abs() < 0.001) {
                // The end in the chuck.
                assert_eq!(color, shade(&points, BLOCK), "{points:?}");
            } else if radii.iter().all(|r| (r - 20.).abs() < 0.01)
                && z.iter().all(|z| *z > 51.)
                // A sliver along the turned diameter's end lies on the bar's
                // end too, and one along the shoulder's rim on its cylinder:
                // those are the block's.
                && !z.iter().all(|z| (z - 100.).abs() < 0.001)
                || z.iter().all(|z| (z - 50.).abs() < 0.001)
                    && !radii.iter().all(|r| (r - 25.).abs() < 0.05)
            {
                // The turned diameter and its shoulder.
                assert_eq!(color, shade_cut(&points), "{points:?}");
                cut += 1;
            }
        }
        assert!(block > 50 && cut > 50, "{block} block, {cut} cut");
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
            hole_outlines: vec![],
            pin_aim: Default::default(),
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            framed: vec![],
            leftover: None,
            live: None,
            a: None,
            live_stock: None,
            start_stock: None,
            live_volume: None,
            uncut: &[],
            show_uncut: true,
            section: Default::default(),
            frame: None,
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
    #[test]
    fn a_planes_depth_on_the_screen_is_the_depth_of_its_points() {
        let plane = Plane {
            normal: {
                let n = [0.3f32, -0.5, 0.8];
                let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                n.map(|c| c / l)
            },
            d: 4.,
        };
        let camera = Camera {
            yaw: 0.4,
            pitch: 0.7,
            zoom: 1.3,
            pan: [12., -7.],
        };
        let bounds = crate::mesh::Bounds {
            min: [-20., -10., 0.],
            max: [30., 25., 18.],
        };
        let p = Projector::fit(camera, (640., 480.), bounds);
        let ratio = 1.5;
        let [a, b, c] = p.plane_depth(&plane, ratio).unwrap();
        // Points of the plane: its point nearest the origin, moved along it.
        let base = plane.normal.map(|v| v * plane.d);
        for (u, w) in [(0., 0.), (5., -3.), (-12., 8.), (20., 14.)] {
            let along = [
                u,
                w,
                -(plane.normal[0] * u + plane.normal[1] * w) / plane.normal[2],
            ];
            let v: [f32; 3] = std::array::from_fn(|k| base[k] + along[k]);
            assert!(plane.distance(v).abs() < 1e-4);
            let q = p.p(v);
            let (x, y) = (q.x * ratio, q.y * ratio);
            let depth = a * x + b * y + c;
            assert!((depth - p.depth(v)).abs() < 1e-3, "{depth} {}", p.depth(v));
        }
    }
    /// The section of a STEP part across each axis: every face it crosses
    /// is crossed on two edges, and the segments close on themselves, so the
    /// cut face is filled without streaks.
    #[test]
    fn a_step_parts_cut_outline_closes_on_itself() {
        use crate::model::{Machine, Setup};
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/half-ball-on-plate.step");
        let mesh = crate::mesh::load(&path).unwrap();
        let setup = Setup::new(Machine::new(3), "G54");
        let placement = Placement::new(&mesh, Some(&setup));
        for k in 0..3 {
            let section = Section::across(k, &mesh, Some(&setup), Camera::default().toward());
            let cut = section.plane(&mesh, Some(&setup)).unwrap();
            let mut ends = std::collections::HashMap::new();
            for f in &mesh.faces {
                let p: Vec<_> = f.points.iter().map(|q| placement.transform(*q)).collect();
                let crossed = (0..p.len())
                    .filter(|&i| {
                        (cut.distance(p[i]) <= 0.) != (cut.distance(p[(i + 1) % p.len()]) <= 0.)
                    })
                    .count();
                assert!(
                    crossed == 0 || crossed == 2,
                    "a face crossed {crossed} times"
                );
                for end in cut.crossing(&p).into_iter().flatten() {
                    *ends.entry(end.map(f32::to_bits)).or_insert(0) += 1;
                }
            }
            assert!(ends.len() > 20, "{} ends across axis {k}", ends.len());
            assert!(
                ends.values().all(|n| n % 2 == 0),
                "an open end across axis {k}"
            );
        }
    }
    /// Writes /tmp/section-step-{x,y,z}.png: the STEP part `$AXIS_SECTION_STEP`
    /// (default the half ball on its plate) cut across each axis through its
    /// middle, the side toward the viewer cut away: for looking at.
    #[test]
    #[ignore]
    fn picture_a_step_parts_sections() {
        use crate::model::{Machine, Setup};
        let path = std::env::var("AXIS_SECTION_STEP").map_or_else(
            |_| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("examples/half-ball-on-plate.step")
            },
            std::path::PathBuf::from,
        );
        let mesh = crate::mesh::load(&path).unwrap();
        let setup = Setup::new(Machine::new(3), "G54");
        let zoom: f32 = std::env::var("AXIS_SECTION_ZOOM").map_or(1., |z| z.parse().unwrap());
        for k in 0..3 {
            let camera = Camera {
                yaw: 0.3,
                pitch: 0.35,
                zoom,
                ..Camera::default()
            };
            let view = Viewport {
                setup: Some(&setup),
                operation: None,
                mesh: &mesh,
                simulation: None,
                sample: false,
                camera,
                picking: false,
                pin_holes: Default::default(),
                hole_outlines: vec![],
                pin_aim: Default::default(),
                show_stock: false,
                show_paths: false,
                progress: None,
                holding: vec![],
                framed: vec![],
                leftover: None,
                live: None,
                live_stock: None,
                a: None,
                live_volume: None,
                start_stock: None,
                uncut: &[],
                show_uncut: true,
                section: Sectioning {
                    cut: Some(Section::across(k, &mesh, Some(&setup), camera.toward())),
                    picking: false,
                },
                frame: None,
            };
            let (w, h) = (900, 700);
            let p = Projector::filling(
                camera,
                (w as f32, h as f32),
                &view.scene_boxes(),
                0.9 * zoom,
            );
            let (rgba, _) = view.render(&p, (w, h), 1., None, &[]);
            let file = format!("/tmp/section-step-{}.png", ["x", "y", "z"][k]);
            let mut encoder = png::Encoder::new(
                std::io::BufWriter::new(std::fs::File::create(&file).unwrap()),
                w,
                h,
            );
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&rgba).unwrap();
            println!("{file}");
        }
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
    fn an_indexed_setups_part_and_stock_turn_to_the_a_shown() {
        use crate::indexed::tests::{bar_job, contour, cutter, prism};
        let mesh = prism(30.);
        let doc = bar_job(
            &mesh,
            0.,
            "80",
            "-150",
            "0, 90, 180, 270",
            contour(&cutter("12")),
        );
        let mut sim =
            crate::planner::view_after(&doc, &mesh, 1, 0, crate::stock_surface::Detail::Draft)
                .unwrap();
        let base = Viewport {
            setup: doc.setups.get(1),
            operation: None,
            mesh: &mesh,
            simulation: None,
            sample: false,
            camera: Camera::default(),
            picking: false,
            pin_holes: Default::default(),
            hole_outlines: vec![],
            pin_aim: Default::default(),
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            framed: vec![],
            leftover: None,
            live: None,
            a: None,
            live_stock: None,
            start_stock: None,
            live_volume: None,
            uncut: &[],
            show_uncut: false,
            section: Default::default(),
            frame: None,
        };
        let view = |sim, a| Viewport {
            simulation: Some(sim),
            a,
            ..base.clone()
        };
        let t = doc.setups[1].turn.unwrap();
        let scale = mesh.display_scale();
        // The job played to its end is shown at its last angle.
        let done = view(&sim, None);
        assert_eq!(done.shown_a(), Some(270.));
        assert_eq!(done.extra_turn(), Some((t.axis, 270.)));
        // At its first angle the part is where the setup's placement puts it.
        let at_start = view(&sim, Some(0.));
        assert_eq!(at_start.extra_turn(), None);
        let placed = at_start.transformed();
        // The machine's A, while a run cuts, turns it about the centreline.
        let live = view(&sim, Some(90.));
        assert_eq!(live.extra_turn(), Some((t.axis, 90.)));
        for (f, g) in placed.iter().zip(live.transformed()) {
            for (p, q) in f.points.iter().zip(&g.points) {
                let want = crate::indexed::turn_point(*p, t.axis.map(|v| v * scale), 90.);
                assert!(
                    (0..3).all(|k| (want[k] - q[k]).abs() < 1e-3),
                    "{want:?} {q:?}"
                );
            }
        }
        // The camera holds still as it turns; the picture does not.
        assert_eq!(at_start.scene_bounds(), live.scene_bounds());
        let size = (240, 180);
        let draw = |v: &Viewport| {
            let p = Projector::fit(v.camera, (240., 180.), v.scene_bounds());
            v.render(&p, size, 1., None, &[]).0
        };
        // (The square left on the bar looks the same a quarter turn on.)
        let eighth = view(&sim, Some(45.));
        assert_eq!(at_start.scene_bounds(), eighth.scene_bounds());
        assert_ne!(draw(&at_start), draw(&eighth));
        // Playing the job, A turns through the first index move and the
        // stock with it.
        let first = (sim.plan.moves.iter())
            .find(|m| m.motion == crate::simulation::Motion::Index)
            .unwrap()
            .clone();
        sim.seek((first.start + first.end) / 2. / sim.plan.duration);
        let midway = Viewport {
            simulation: Some(&sim),
            ..base.clone()
        };
        assert!((midway.shown_a().unwrap() - 45.).abs() < 1e-3);
        assert!((midway.extra_turn().unwrap().1 - 45.).abs() < 1e-3);
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
            hole_outlines: vec![],
            pin_aim: Default::default(),
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            framed: vec![],
            leftover: None,
            live: None,
            a: None,
            live_stock: None,
            start_stock: None,
            live_volume: None,
            uncut: &all,
            show_uncut: true,
            section: Default::default(),
            frame: None,
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
            section: Default::default(),
            ..view
        };
        assert_eq!(magenta(&off, size), 0);
        // The toggle is in the key the raster is cached under.
        assert_ne!(on_key, off.raster_key(bounds, None));
        // Nothing marked uncut: nothing magenta either.
        let none = Viewport {
            uncut: &[],
            show_uncut: true,
            section: Default::default(),
            ..off
        };
        assert_eq!(magenta(&none, size), 0);
    }
    /// Leaving a simulation, or showing or hiding the stock, does not move
    /// the view: the camera fits the same scene, not the cutter's reach.
    #[test]
    fn the_view_stays_put_with_a_simulation_and_without() {
        let (doc, mesh, report) = crate::proposal::tests::block();
        let good = crate::proposal::tests::good(&report);
        let job = crate::proposal::build(&doc, &mesh, &report, &good).unwrap();
        let sim =
            crate::planner::view_after(&job, &mesh, 0, 0, crate::stock_surface::Detail::Draft)
                .unwrap();
        let view = |simulation, show_stock| Viewport {
            setup: job.setups.first(),
            operation: None,
            mesh: &mesh,
            simulation,
            sample: false,
            camera: Camera {
                zoom: 3.,
                pan: [40., -25.],
                ..Camera::default()
            },
            picking: false,
            pin_holes: Default::default(),
            hole_outlines: vec![],
            pin_aim: Default::default(),
            show_stock,
            show_paths: false,
            progress: None,
            holding: vec![],
            framed: vec![],
            leftover: None,
            live: None,
            live_stock: None,
            a: None,
            live_volume: None,
            start_stock: None,
            uncut: &[],
            show_uncut: true,
            section: Default::default(),
            frame: None,
        };
        let bounds = Rectangle {
            x: 0.,
            y: 0.,
            width: 900.,
            height: 600.,
        };
        let corner = Placement::new(&mesh, job.setups.first()).bounds.max;
        let seen = |v: Viewport| {
            let p = v.projector(bounds);
            (p.p(corner), p.scale)
        };
        let regular = seen(view(None, true));
        assert_eq!(seen(view(Some(&sim), true)), regular);
        assert_eq!(seen(view(None, false)), regular);
        assert_eq!(seen(view(Some(&sim), false)), regular);
        // The panels under the view grow or shrink with the operation and
        // the simulation, and the view's height with them; in the same
        // frame the scene stays where it is, and so does what the wheel
        // zooms toward and the drag turns about.
        let framed = |height: f32| {
            let v = Viewport {
                frame: Some(450.),
                ..view(Some(&sim), true)
            };
            let b = Rectangle { height, ..bounds };
            let p = v.projector(b);
            (p.p(corner), p.scale, v.pivot(b))
        };
        assert_eq!(framed(600.), framed(380.));
        assert_eq!(framed(600.), framed(520.));
        // Drawn again in another frame, or round other jaws.
        let key = |frame, framed| {
            Viewport {
                frame,
                framed,
                ..view(None, true)
            }
            .raster_key(bounds, None)
        };
        assert_ne!(key(Some(450.), vec![]), key(Some(400.), vec![]));
        assert_ne!(key(None, vec![]), key(Some(450.), vec![]));
        let jaw = crate::holding::HoldBox {
            shape: Default::default(),
            label: "jaw".into(),
            bounds: Placement::new(&mesh, job.setups.first()).bounds,
        };
        assert_ne!(key(None, vec![]), key(None, vec![jaw]));
    }
    /// Without a simulation the view shows the stock the operation starts
    /// from as the operations before it leave it: solid, as the simulation
    /// draws it, in place of the setup's box; see-through over the part
    /// while the part is picked.
    #[test]
    fn the_stock_an_operation_starts_from_is_drawn_without_a_simulation() {
        let (doc, mesh, report) = crate::proposal::tests::block();
        let good = crate::proposal::tests::good(&report);
        let job = crate::proposal::build(&doc, &mesh, &report, &good).unwrap();
        let sim =
            crate::planner::view_after(&job, &mesh, 0, 0, crate::stock_surface::Detail::Draft)
                .unwrap();
        let (volume, surface) = (sim.volume.as_ref().unwrap(), sim.stock_surface.as_deref());
        let start = Some((volume, surface.unwrap()));
        let view = |simulation, start_stock, picking| Viewport {
            setup: job.setups.first(),
            operation: None,
            mesh: &mesh,
            simulation,
            sample: false,
            camera: Camera::default(),
            picking,
            pin_holes: Default::default(),
            hole_outlines: vec![],
            pin_aim: Default::default(),
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            framed: vec![],
            leftover: None,
            live: None,
            live_stock: None,
            a: None,
            live_volume: None,
            start_stock,
            uncut: &[],
            show_uncut: true,
            section: Default::default(),
            frame: None,
        };
        let bounds = Rectangle {
            x: 0.,
            y: 0.,
            width: 300.,
            height: 200.,
        };
        let pixels = |v: Viewport| {
            let p = v.projector(bounds);
            v.pixels(&p, bounds, None).2
        };
        // Drawn as the simulation draws its stock when it stands there: the
        // same pixels but for the simulation's cutter.
        let simulated = pixels(view(Some(&sim), None, false));
        let differ = |a: &[u8], b: &[u8]| {
            (a.chunks(4).zip(b.chunks(4)))
                .filter(|(a, b)| a != b)
                .count()
        };
        let started = pixels(view(None, start, false));
        let boxed = pixels(view(None, None, false));
        let (near, far) = (differ(&started, &simulated), differ(&boxed, &simulated));
        assert!(near * 10 < far, "{near} {far}");
        // Picking: the part is drawn, under a see-through stock.
        let picked = pixels(view(None, start, true));
        assert_ne!(picked, simulated);
        assert_ne!(picked, pixels(view(None, None, true)));
        // Hidden with the stock switched off.
        let off = Viewport {
            show_stock: false,
            ..view(None, start, false)
        };
        let bare = Viewport {
            show_stock: false,
            ..view(None, None, false)
        };
        assert_eq!(pixels(off), pixels(bare));
        // A new surface is drawn again.
        assert_ne!(
            view(None, start, false).raster_key(bounds, None),
            view(None, None, false).raster_key(bounds, None)
        );
    }
    /// Zoomed far in, turning the view keeps what is zoomed in on in place:
    /// it turns about the part's surface at the view's centre, not the
    /// scene's centre, which a 100× view would swing hundreds of pixels.
    #[test]
    fn turning_a_zoomed_in_view_keeps_the_surface_at_its_centre() {
        use canvas::Program;
        let (doc, mesh) = crate::post::fixture::job();
        let mut view = Viewport {
            setup: doc.setups.first(),
            operation: None,
            mesh: &mesh,
            simulation: None,
            sample: false,
            camera: Camera::default(),
            picking: false,
            pin_holes: Default::default(),
            hole_outlines: vec![],
            pin_aim: Default::default(),
            show_stock: false,
            show_paths: false,
            progress: None,
            holding: vec![],
            framed: vec![],
            leftover: None,
            live: None,
            live_stock: None,
            a: None,
            live_volume: None,
            start_stock: None,
            uncut: &[],
            show_uncut: false,
            section: Default::default(),
            frame: None,
        };
        let bounds = Rectangle {
            x: 0.,
            y: 0.,
            width: 900.,
            height: 600.,
        };
        let centre = Point::new(450., 600. * 0.53);
        // A point of the part off the scene's centre, brought to the view's
        // centre at 100×.
        let p = view.projector(bounds);
        let corner = view.scene_bounds().min;
        let near = Point::new(
            p.p(corner).x * 0.6 + centre.x * 0.4,
            p.p(corner).y * 0.6 + centre.y * 0.4,
        );
        let mut camera = Camera {
            zoom: 100.,
            ..view.camera
        };
        let k = 100.;
        camera.pan = [(centre.x - near.x) * k, (centre.y - near.y) * k];
        view.camera = camera;
        let pivot = view.pivot(bounds);
        let at = |c: Camera| Projector::fit(c, (900., 600.), view.scene_bounds()).p(pivot);
        let start = at(view.camera);
        assert!(start.distance(centre) < 1., "{start:?}");
        // The point found is on the part: back where the view shows it.
        let q = view.projector(bounds);
        assert!(q.p(q.at_depth(centre, q.depth(pivot))).distance(centre) < 0.01);
        // A drag that turns the view.
        let mut state = State::default();
        let cursor = |x, y| mouse::Cursor::Available(Point::new(x, y));
        let press = canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let _ = view.update(&mut state, &press, bounds, cursor(300., 300.));
        let moved = canvas::Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(310., 304.),
        });
        let action = view.update(&mut state, &moved, bounds, cursor(310., 304.));
        let Some(Event::Orbit(yaw, pitch, shift)) = action.and_then(|a| a.into_inner().0) else {
            panic!("the drag turns the view");
        };
        let mut turned = orbited(view.camera, yaw, pitch);
        let unshifted = at(turned);
        turned.pan = [turned.pan[0] + shift[0], turned.pan[1] + shift[1]];
        // Without the pan it would leave the view; with it, it stays.
        assert!(unshifted.distance(start) > 100., "{unshifted:?}");
        assert!(
            at(turned).distance(start) < 0.5,
            "{:?} {start:?}",
            at(turned)
        );
        // More moves before the app draws again (the view keeps the camera
        // it had): each turn starts from the camera the earlier ones left,
        // so applied in turn they keep the point too.
        for (i, (x, y)) in [(318., 300.), (325., 309.), (331., 305.)]
            .into_iter()
            .enumerate()
        {
            let moved = canvas::Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(x, y),
            });
            let action = view.update(&mut state, &moved, bounds, cursor(x, y));
            let Some(Event::Orbit(yaw, pitch, shift)) = action.and_then(|a| a.into_inner().0)
            else {
                panic!("move {i} turns the view");
            };
            turned = orbited(turned, yaw, pitch);
            turned.pan = [turned.pan[0] + shift[0], turned.pan[1] + shift[1]];
            assert!(
                at(turned).distance(start) < 0.5,
                "move {i}: {:?}",
                at(turned)
            );
        }
    }
    /// A move stretched far past the frame by a zoomed-in view is drawn
    /// whole where it shows, not as dashes: only its part in the frame is
    /// stepped along.
    #[test]
    fn a_long_line_through_the_frame_is_drawn_without_gaps() {
        let (w, h) = (200u32, 100u32);
        // Along the centres of row 50's pixels.
        let (a, b) = (Point::new(-500_000., 50.5), Point::new(500_000., 50.5));
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        raster_line(&mut pixels, w, h, a, b, [255; 4], 1.);
        let row = |p: &[u8]| {
            (0..w)
                .filter(|x| p[((50 * w + x) * 4) as usize] == 255)
                .count()
        };
        assert_eq!(row(&pixels), w as usize);
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        let depths = vec![f32::NEG_INFINITY; (w * h) as usize];
        raster_depth_line(&mut pixels, &depths, w, h, (a, 0.), (b, 0.), [255; 4], 1.);
        assert_eq!(row(&pixels), w as usize);
        // One that misses the frame draws nothing.
        assert_eq!(
            in_frame(Point::new(-50., -50.), Point::new(-10., -60.), w, h, 2.),
            None
        );
    }
    #[test]
    fn a_selected_face_under_stock_is_striped_in_blue_and_magenta() {
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
            hole_outlines: vec![],
            pin_aim: Default::default(),
            show_stock: false,
            show_paths: false,
            progress: None,
            holding: vec![],
            framed: vec![],
            leftover: None,
            live: None,
            a: None,
            live_stock: None,
            start_stock: None,
            live_volume: None,
            uncut: &all,
            show_uncut: true,
            section: Default::default(),
            frame: None,
        };
        let size = (300, 200);
        let p = Projector::fit(view.camera, (300., 200.), view.scene_bounds());
        let features: Vec<usize> = mesh.faces.iter().filter_map(|f| f.feature).collect();
        assert!(!features.is_empty());
        let (plain, _) = view.render(&p, size, 1., None, &[]);
        let (marked, _) = view.render(&p, size, 1., None, &features);
        let tinted = |rgba: &[u8]| rgba.chunks(4).filter(|px| **px == SELECTED).count();
        // Selected, the faces are striped: about half of what was magenta
        // is the selection's blue, the rest still magenta.
        assert_eq!(tinted(&plain), 0);
        assert!(magenta(&view, size) > 0);
        let fill = |rgba: &[u8]| {
            rgba.chunks(4)
                .filter(|px| px[0] > px[1].saturating_add(60) && px[2] > px[1].saturating_add(40))
                .count()
        };
        let (blue, kept, was) = (tinted(&marked), fill(&marked), fill(&plain));
        assert!(
            blue * 10 > was * 3 && kept * 10 > was * 3,
            "{blue} {kept} {was}"
        );
        assert!(blue + kept <= was, "{blue} {kept} {was}");
        // In stripes across the frame: along a row, blue and magenta take
        // turns many times.
        let row = (0..size.1)
            .max_by_key(|y| {
                let line = &marked[(y * size.0 * 4) as usize..((y + 1) * size.0 * 4) as usize];
                fill(line) + tinted(line)
            })
            .unwrap();
        let line = &marked[(row * size.0 * 4) as usize..((row + 1) * size.0 * 4) as usize];
        let turns = line
            .chunks(4)
            .map(|px| *px == SELECTED)
            .collect::<Vec<_>>()
            .windows(2)
            .filter(|w| w[0] != w[1])
            .count();
        assert!(turns >= 6, "{turns}");
        // Without the toggle the selection fills the face, as it did.
        let off = Viewport {
            show_uncut: false,
            section: Default::default(),
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
                hole_outlines: vec![],
                pin_aim: Default::default(),
                show_stock: true,
                show_paths: false,
                progress: None,
                holding: vec![],
                framed: vec![],
                leftover: None,
                live: None,
                a: None,
                live_stock: None,
                start_stock: None,
                live_volume: None,
                uncut,
                show_uncut: true,
                section: Default::default(),
                frame: None,
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
            hole_outlines: vec![],
            pin_aim: Default::default(),
            show_stock,
            show_paths: false,
            progress: None,
            holding: vec![],
            framed: vec![],
            leftover,
            live: None,
            a: None,
            live_stock: None,
            start_stock: None,
            live_volume: None,
            uncut: &[],
            show_uncut: true,
            section: Default::default(),
            frame: None,
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
            hole_outlines: vec![],
            pin_aim: Default::default(),
            show_stock: false,
            show_paths: false,
            progress: None,
            holding: vec![],
            framed: vec![],
            leftover: None,
            live: None,
            a: None,
            live_stock: None,
            start_stock: None,
            live_volume: None,
            uncut: &[],
            show_uncut: true,
            section: Default::default(),
            frame: None,
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
            hole_outlines: vec![],
            pin_aim: Default::default(),
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            framed: vec![],
            leftover: None,
            live: None,
            a: None,
            live_stock: None,
            start_stock: None,
            live_volume: None,
            uncut: &[],
            show_uncut: true,
            section: Default::default(),
            frame: None,
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
        let vertices = SolidBox {
            faces: (BOX_FACES.iter().enumerate())
                .map(|(tone, f)| (f.map(|i| vertices[i]).to_vec(), tone as u8))
                .collect(),
            edges: BOX_EDGES.map(|(a, b)| (vertices[a], vertices[b])).to_vec(),
        };
        let index = (5 * 12 + 5) * 4;
        for (depth, expected_unchanged) in [(3., true), (1., false)] {
            let mut pixels = [128u8, 128, 128, 255].repeat(144);
            raster_stock(
                &mut pixels,
                &vec![depth; 144],
                12,
                12,
                &vertices,
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
            &vertices,
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
            shape: Default::default(),
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
            hole_outlines: vec![],
            pin_aim: Default::default(),
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            framed: vec![],
            leftover: None,
            live: None,
            a: None,
            live_stock: None,
            start_stock: None,
            live_volume: None,
            uncut: &[],
            show_uncut: true,
            section: Default::default(),
            frame: None,
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
    fn the_camera_fit_contains_the_workholding_drawn_or_not() {
        use crate::holding::HoldBox;
        let d = crate::model::Document::sample();
        let mesh = crate::mesh::sample();
        let scale = mesh.display_scale();
        // A clamp reaching far beyond the stock's +X side.
        let clamp = HoldBox {
            shape: Default::default(),
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
            hole_outlines: vec![],
            pin_aim: Default::default(),
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            framed: vec![],
            leftover: None,
            live: None,
            a: None,
            live_stock: None,
            start_stock: None,
            live_volume: None,
            uncut: &[],
            show_uncut: true,
            section: Default::default(),
            frame: None,
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
        v.framed = vec![clamp.clone()];
        let with = v.projector(b);
        assert!(inside(&with));
        assert!(with.scale < without.scale);
        // Framed whether drawn or not: the view draws none with the stock
        // hidden, and the camera stays where it is.
        v.holding = vec![clamp];
        let drawn = v.projector(b);
        v.show_stock = false;
        v.holding.clear();
        let hidden = v.projector(b);
        assert_eq!(
            (drawn.scale, drawn.p([0.; 3])),
            (with.scale, with.p([0.; 3]))
        );
        assert_eq!(
            (hidden.scale, hidden.p([0.; 3])),
            (with.scale, with.p([0.; 3]))
        );
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
            hole_outlines: vec![],
            pin_aim: Default::default(),
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            framed: vec![],
            leftover: None,
            live: None,
            a: None,
            live_stock: None,
            start_stock: None,
            live_volume: None,
            uncut: &[],
            show_uncut: true,
            section: Default::default(),
            frame: None,
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
            hole_outlines: vec![],
            pin_aim: Default::default(),
            show_stock: false,
            show_paths: false,
            progress: None,
            holding: vec![],
            framed: vec![],
            leftover: None,
            live: None,
            a: None,
            live_stock: None,
            start_stock: None,
            live_volume: None,
            uncut: &[],
            show_uncut: true,
            section: Default::default(),
            frame: None,
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
        // An operation that cannot pick on an imported part (the sample's
        // second, an Adaptive clearing, can now).
        let tool = d.tools[0].id.clone();
        d.setup_mut()
            .operations
            .push(Operation::new(Strategy::Scallop, tool));
        let unavailable = d.setup().operations.len() - 1;
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
            hole_outlines: vec![],
            pin_aim: Default::default(),
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            framed: vec![],
            leftover: None,
            live: None,
            a: None,
            live_stock: None,
            start_stock: None,
            live_volume: None,
            uncut: &[],
            show_uncut: true,
            section: Default::default(),
            frame: None,
        };
        let bounds = Rectangle::new(Point::ORIGIN, iced::Size::new(800., 600.));
        let point = Placement::new(&mesh, v.setup).transform(mesh.surfaces[1].center);
        assert_eq!(v.picked(bounds, v.projector(bounds).p(point)), Some(1));
        v.operation = Some(&d.setup().operations[unavailable]);
        assert_eq!(v.picked(bounds, v.projector(bounds).p(point)), None);
        v.operation = Some(&d.setup().operations[0]);
        v.picking = false;
        assert_eq!(v.picked(bounds, v.projector(bounds).p(point)), None);
    }

    #[test]
    fn a_holes_outline_under_the_pointer_or_its_opening_clicks_one_of_its_walls() {
        let mesh = crate::mesh::normalize(vec![
            [[0., 0., 0.], [10., 0., 0.], [0., 10., 0.]],
            [[0., 0., 2.], [10., 0., 2.], [0., 10., 2.]],
        ])
        .unwrap();
        let d = crate::model::Document::sample();
        let scale = mesh.display_scale();
        let at = Placement::new(&mesh, Some(d.setup()))
            .transform(mesh.surfaces[1].center)
            .map(|v| v / scale);
        // A 2 mm square outline round the upper face's centre, and one 50 mm
        // off the part, where nothing of it is under the pointer.
        let square = |x: f32, walls: Vec<usize>| HoleOutline {
            loops: vec![vec![
                [x - 1., at[1] - 1.],
                [x + 1., at[1] - 1.],
                [x + 1., at[1] + 1.],
                [x - 1., at[1] + 1.],
            ]],
            z: at[2],
            walls,
            picked: false,
            label: None,
        };
        let v = Viewport {
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
            hole_outlines: vec![square(at[0], vec![7, 9]), square(at[0] + 50., vec![8])],
            pin_aim: Default::default(),
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
            framed: vec![],
            leftover: None,
            live: None,
            live_stock: None,
            a: None,
            live_volume: None,
            start_stock: None,
            uncut: &[],
            show_uncut: true,
            section: Default::default(),
            frame: None,
        };
        let b = Rectangle::new(Point::ORIGIN, iced::Size::new(800., 600.));
        let screen = |x: f32, y: f32| v.projector(b).p([x * scale, y * scale, at[2] * scale]);
        // On the outline, over the part: the outline's hole, by its first wall.
        assert_eq!(v.picked(b, screen(at[0] + 1., at[1])), Some(7));
        // Within its reach on the screen, and not beyond it.
        let edge = screen(at[0] + 1., at[1]);
        assert_eq!(v.picked(b, Point::new(edge.x + 3.5, edge.y)), Some(7));
        assert!(v.picked(b, Point::new(edge.x + 30., edge.y)) != Some(7));
        // Inside it with the part's face under the pointer: the face.
        assert_eq!(v.picked(b, screen(at[0], at[1])), Some(1));
        // Inside the outline off the part: the opening picks its hole.
        assert_eq!(v.picked(b, screen(at[0] + 50., at[1])), Some(8));
        assert_eq!(v.picked(b, screen(at[0] + 80., at[1])), None);
        // Not while picking is off.
        let off = Viewport {
            picking: false,
            ..v.clone()
        };
        assert_eq!(off.picked(b, screen(at[0] + 50., at[1])), None);
    }

    #[test]
    fn hole_labels_never_cover_each_other_or_another_hole_and_crowded_ones_are_counted() {
        let hole = |x: f32, y: f32| Rectangle::new(Point::new(x, y), iced::Size::new(8., 8.));
        let overlap = |a: &Rectangle, b: &Rectangle| {
            a.x < b.x + b.width
                && b.x < a.x + a.width
                && a.y < b.y + b.height
                && b.y < a.y + a.height
        };
        // Three holes far apart: each labelled beside it.
        let apart = [hole(0., 0.), hole(200., 0.), hole(0., 200.)];
        let (placed, hidden) = place_labels(&apart, &[Some(2); 3], &[false; 3], None);
        assert_eq!((placed.len(), hidden), (3, 0));
        assert!(placed.iter().all(|(_, _, away)| !away));
        // A 6 × 6 grid of holes 11 points apart, labels as wide as "Ø1.8".
        let grid: Vec<Rectangle> = (0..36)
            .map(|k| hole((k % 6) as f32 * 11., (k / 6) as f32 * 11.))
            .collect();
        for hovered in std::iter::once(None).chain((0..36).map(Some)) {
            let (placed, hidden) = place_labels(&grid, &[Some(4); 36], &[false; 36], hovered);
            assert_eq!(placed.len() + hidden, 36);
            assert!(hidden > 0, "{placed:?}");
            for (n, (i, spot, away)) in placed.iter().enumerate() {
                // The hovered hole's goes over the others if it must.
                if hovered == Some(*i) {
                    continue;
                }
                // No label covers another, nor a hole but its own.
                for (j, other, _) in &placed[n + 1..] {
                    assert!(
                        hovered == Some(*j) || !overlap(spot, other),
                        "{spot:?} {other:?}"
                    );
                }
                for (j, b) in grid.iter().enumerate() {
                    assert!(j == *i || !overlap(spot, b), "label of {i} on hole {j}");
                }
                // A leader line passes no other hole.
                if *away {
                    let from = Point::new(grid[*i].x + 4., grid[*i].y + 4.);
                    let to = leader_end(from, *spot);
                    for k in 0..=50 {
                        let t = k as f32 / 50.;
                        let q =
                            Point::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
                        for (j, b) in grid.iter().enumerate() {
                            assert!(j == *i || !b.contains(q), "leader of {i} through hole {j}");
                        }
                    }
                }
            }
            // The hovered hole's label always shows, last, so that it is
            // drawn over the others, and pointing at it moves no other label.
            if let Some(h) = hovered {
                assert_eq!(placed.last().map(|l| l.0), Some(h));
                let (still, _) = place_labels(&grid, &[Some(4); 36], &[false; 36], None);
                for l in &still {
                    assert!(placed.contains(l), "{l:?} moved");
                }
            }
            // Each keeps its gap from every other hole.
            for (i, spot, _) in placed.iter().filter(|l| hovered != Some(l.0)) {
                for (j, b) in grid.iter().enumerate().filter(|(j, _)| j != i) {
                    let wide = Rectangle::new(
                        Point::new(b.x - LABEL_GAP + 0.01, b.y - LABEL_GAP + 0.01),
                        iced::Size::new(
                            b.width + 2. * LABEL_GAP - 0.02,
                            b.height + 2. * LABEL_GAP - 0.02,
                        ),
                    );
                    assert!(
                        !overlap(spot, &wide),
                        "label of {i} within the gap of hole {j}"
                    );
                }
            }
        }
        assert_eq!(
            hidden_note(7),
            "7 holes' sizes hidden where holes are close: zoom in, or point at a hole"
        );
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
        let view = |holding: Vec<HoldBox>| Viewport {
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
            hole_outlines: vec![],
            pin_aim: Default::default(),
            show_stock: false,
            show_paths: false,
            progress: None,
            framed: holding.clone(),
            holding,
            leftover: None,
            live: None,
            a: None,
            live_stock: None,
            start_stock: None,
            live_volume: None,
            uncut: &[],
            show_uncut: true,
            section: Default::default(),
            frame: None,
        };
        let b = Rectangle::new(Point::ORIGIN, iced::Size::new(400., 300.));
        let hold = |min, max| HoldBox {
            shape: Default::default(),
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
