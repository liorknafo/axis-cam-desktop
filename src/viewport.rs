use crate::{
    mesh::{Face, Mesh},
    model::{Operation, Setup, Strategy},
    stock::{self, Placement},
};
use iced::{
    Color, Point, Rectangle, Renderer, Theme, mouse,
    widget::{
        Action,
        canvas::{self, Frame, Path, Stroke, Text},
    },
};
use std::cell::RefCell;
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
}
#[derive(Default)]
pub struct State {
    drag: Option<(Point, Point, mouse::Button)>,
    moved: bool,
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
pub struct Viewport<'a> {
    pub setup: Option<&'a Setup>,
    pub operation: Option<&'a Operation>,
    pub mesh: &'a Mesh,
    pub simulation: Option<&'a crate::simulation::Simulation>,
    pub sample: bool,
    pub camera: Camera,
    pub picking: bool,
    pub show_stock: bool,
    pub show_paths: bool,
    pub progress: Option<f32>,
    /// The jaws or clamps of the setup, in its frame in mm; drawn with the
    /// stock and part of the camera's fit.
    pub holding: Vec<crate::holding::HoldBox>,
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
    pub(crate) fn p(&self, v: [f32; 3]) -> Point {
        let [x, y, z] = std::array::from_fn(|i| v[i] - self.target[i]);
        let (s, c) = self.camera.yaw.sin_cos();
        let (se, ce) = self.camera.pitch.sin_cos();
        Point::new(
            self.center.x + (x * c - y * s) * self.scale,
            self.center.y - (-x * s * se - y * c * se + z * ce) * self.scale,
        )
    }
    pub(crate) fn depth(&self, v: [f32; 3]) -> f32 {
        let (s, c) = self.camera.yaw.sin_cos();
        let (se, ce) = self.camera.pitch.sin_cos();
        v[0] * s * ce + v[1] * c * ce + v[2] * se
    }
}
impl Viewport<'_> {
    fn projector(&self, b: Rectangle) -> Projector {
        Projector::fit(self.camera, (b.width, b.height), self.scene_bounds())
    }
    /// What the camera must contain, in display units: the part, the stock,
    /// the jaws or clamps and, when simulating, the stock and every move.
    pub(crate) fn scene_bounds(&self) -> crate::mesh::Bounds {
        let mut bounds = Placement::new(self.mesh, self.setup).bounds;
        if self.show_stock
            && let Some(setup) = self.setup
            && let Ok(stock) = stock::resolve(setup, Some(self.mesh))
        {
            let stock = stock.stock.scaled(stock.display_scale);
            for i in 0..3 {
                bounds.min[i] = bounds.min[i].min(stock.min[i]);
                bounds.max[i] = bounds.max[i].max(stock.max[i]);
            }
        }
        if !self.holding.is_empty() {
            let scale = self.mesh.display_scale();
            for hold in &self.holding {
                let hold = hold.bounds.scaled(scale);
                for i in 0..3 {
                    bounds.min[i] = bounds.min[i].min(hold.min[i]);
                    bounds.max[i] = bounds.max[i].max(hold.max[i]);
                }
            }
        }
        if let Some(sim) = self.simulation {
            for k in 0..3 {
                bounds.min[k] = bounds.min[k].min(sim.plan.stock.min[k] * sim.plan.display_scale);
                bounds.max[k] = bounds.max[k].max(sim.plan.stock.max[k] * sim.plan.display_scale);
            }
            // Keep the whole cutter and motion inside a stable camera fit.
            for point in sim.plan.moves.iter().flat_map(|m| [m.from, m.to]) {
                for k in 0..3 {
                    bounds.min[k] = bounds.min[k].min(
                        (point[k]
                            - if k == 2 {
                                0.
                            } else {
                                sim.plan.radius.max(sim.plan.holder_radius)
                            })
                            * sim.plan.display_scale,
                    );
                    bounds.max[k] = bounds.max[k].max(
                        (point[k]
                            + if k == 2 {
                                sim.plan.tool_length + 8.
                            } else {
                                sim.plan.radius.max(sim.plan.holder_radius)
                            })
                            * sim.plan.display_scale,
                    );
                }
            }
        }
        bounds
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
    fn picked(&self, b: Rectangle, point: Point) -> Option<usize> {
        if !self.picking
            || (!self.sample
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
                {
                    if z > nearest {
                        nearest = z;
                        feature = face.feature;
                    }
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
    /// the part's depth buffer. `marked` surfaces take the selection tint.
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
        // Remaining stock is the simulated solid. Do not overlay the finished
        // reference mesh on its coincident walls; Stock off reveals that model.
        for face in if self.simulation.is_some() && self.show_stock {
            vec![]
        } else {
            self.transformed()
        } {
            let normal = normal(&face.points);
            let light = (0.42 + normal[2].abs() * 0.27 + normal[0].abs() * 0.12 + face.tone * 0.13)
                .clamp(0.2, 0.94);
            let selected = face.feature.is_some_and(|f| {
                self.operation.is_some_and(|o| {
                    if self.sample {
                        self.picking && o.geometry.contains(&f)
                    } else {
                        o.surfaces
                            .as_ref()
                            .is_some_and(|s| s.matches(self.mesh) && s.ids.contains(&f))
                    }
                })
            }) || face.feature.is_some_and(|f| marked.contains(&f));
            let color = if face.feature.is_some() && face.feature == hovered {
                [242, 190, 93, 255]
            } else if selected {
                [92, 168, 223, 255]
            } else {
                [
                    (light * 0.80 * 255.) as u8,
                    (light * 0.91 * 255.) as u8,
                    (light * 255.) as u8,
                    255,
                ]
            };
            let v: Vec<_> = face
                .points
                .iter()
                .map(|v| {
                    let pt = p.p(*v);
                    (Point::new(pt.x * ratio, pt.y * ratio), p.depth(*v))
                })
                .collect();
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
                            }
                        }
                    }
                }
            }
        }
        let part_depths = depths.clone();
        if let Some(simulation) = self.simulation {
            self.raster_simulation(simulation, p, &mut rgba, &mut depths, w, h, ratio);
        } else if self.show_stock
            && let Some(setup) = self.setup
            && (self.sample || crate::model::get(&setup.p, "stock_source") == "fresh")
            && let Ok(stock) = stock::resolve(setup, Some(self.mesh))
        {
            let projected = box_vertices(stock.stock.scaled(stock.display_scale)).map(|v| {
                let q = p.p(v);
                (Point::new(q.x * ratio, q.y * ratio), p.depth(v))
            });
            raster_stock(&mut rgba, &depths, w, h, projected, ratio, p.scale);
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
                );
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
            if let Some(progress) = self.progress {
                if !all.is_empty() {
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
        }
        (rgba, part_depths)
    }
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
        if self.show_stock {
            if let (Some(volume), Some(current)) = (&sim.volume, sim.frame) {
                if let Some(surface) = &sim.stock_surface {
                    for triangle in &surface.triangles {
                        let source = triangle.map(|i| surface.vertices[i]);
                        let points = source.map(|p| volume.frame.transfer(p, current));
                        let n = normal(&points);
                        let outer = (0..3).any(|k| {
                            source
                                .iter()
                                .all(|p| (p[k] - volume.bounds.min[k]).abs() < 0.01)
                                || source
                                    .iter()
                                    .all(|p| (p[k] - volume.bounds.max[k]).abs() < 0.01)
                        });
                        let light = 0.58 + 0.29 * n[2].abs() + 0.13 * n[0].abs();
                        let base = if outer {
                            [219., 183., 113.]
                        } else {
                            [103., 181., 151.]
                        };
                        face(
                            &points,
                            [
                                (base[0] * light) as u8,
                                (base[1] * light) as u8,
                                (base[2] * light) as u8,
                                255,
                            ],
                        );
                    }
                }
            }
        }
        if self.show_stock && sim.volume.is_none() {
            let g = &sim.stock;
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
                    for (adjacent, edge, tone) in [
                        (
                            if x == 0 {
                                g.bounds.min[2]
                            } else {
                                g.heights[y * g.nx + x - 1]
                            },
                            [[a, c, z], [a, d, z]],
                            0,
                        ),
                        (
                            if x + 1 == g.nx {
                                g.bounds.min[2]
                            } else {
                                g.heights[y * g.nx + x + 1]
                            },
                            [[b, c, z], [b, d, z]],
                            1,
                        ),
                        (
                            if y == 0 {
                                g.bounds.min[2]
                            } else {
                                g.heights[(y - 1) * g.nx + x]
                            },
                            [[a, c, z], [b, c, z]],
                            1,
                        ),
                        (
                            if y + 1 == g.ny {
                                g.bounds.min[2]
                            } else {
                                g.heights[(y + 1) * g.nx + x]
                            },
                            [[a, d, z], [b, d, z]],
                            0,
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
                                if cut {
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
                    state.drag = Some((p, p, *b));
                    state.moved = false;
                    return Some(Action::capture());
                }
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
                    if !state.moved && button == mouse::Button::Left {
                        if let Some(p) = cursor.position_in(bounds) {
                            if let Some(f) = self.picked(bounds, p) {
                                return Some(
                                    Action::publish(if self.sample {
                                        Event::Pick(f)
                                    } else {
                                        Event::PickSurface(f)
                                    })
                                    .and_capture(),
                                );
                            }
                        }
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
        let key = format!("{key}|{:?}", self.holding);
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
        let model = model_frame.into_geometry();
        let mut frame = Frame::new(renderer, bounds.size());
        if self.show_stock
            && let Some(setup) = self.setup
            && let Ok(stock) = stock::resolve(setup, Some(self.mesh))
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
                content: if !self.sample
                    && self.simulation.is_none()
                    && crate::model::get(&setup.p, "stock_source") != "fresh"
                {
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
        vec![background, model, frame.into_geometry()]
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
/// The stock's box: `raster_solid` in the stock's colors.
fn raster_stock(
    pixels: &mut [u8],
    part_depth: &[f32],
    w: u32,
    h: u32,
    vertices: [(Point, f32); 8],
    ratio: f32,
    scale: f32,
) {
    raster_solid(&STOCK, pixels, part_depth, (w, h), vertices, ratio, scale);
}
/// Composite only the nearest surface of a box; use both surfaces and part
/// depth to distinguish visible edges from dashed hidden edges.
fn raster_solid(
    look: &Look,
    pixels: &mut [u8],
    part_depth: &[f32],
    (w, h): (u32, u32),
    vertices: [(Point, f32); 8],
    ratio: f32,
    scale: f32,
) {
    let mut near = vec![f32::NEG_INFINITY; (w * h) as usize];
    let mut tones = vec![0usize; (w * h) as usize];
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
                        tones[index] = tone;
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
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
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
        assert_eq!(digest(&rgba), 11316679358304618149);
    }
    #[test]
    fn render_tints_marked_surfaces_and_returns_the_part_depths() {
        let (doc, mesh) = crate::post::fixture::job();
        let view = Viewport {
            setup: doc.setups.first(),
            operation: None,
            mesh: &mesh,
            simulation: None,
            sample: false,
            camera: Camera::default(),
            picking: false,
            show_stock: false,
            show_paths: false,
            progress: None,
            holding: vec![],
        };
        let p = Projector::fit(view.camera, (300., 200.), view.scene_bounds());
        let tint = [92, 168, 223, 255];
        let tinted = |rgba: &[u8]| rgba.chunks(4).filter(|px| **px == tint).count();
        let (plain, depths) = view.render(&p, (300, 200), 1., None, &[]);
        assert_eq!(tinted(&plain), 0);
        let features: Vec<usize> = mesh.faces.iter().filter_map(|f| f.feature).collect();
        let (marked, _) = view.render(&p, (300, 200), 1., None, &features);
        assert!(tinted(&marked) > 0);
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
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
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
            raster_stock(&mut pixels, &vec![depth; 144], 12, 12, vertices, 1., 1.);
            assert_eq!(
                &pixels[index..index + 4] == [128, 128, 128, 255],
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
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
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
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
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
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
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
            show_stock: false,
            show_paths: false,
            progress: None,
            holding: vec![],
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
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
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
}
