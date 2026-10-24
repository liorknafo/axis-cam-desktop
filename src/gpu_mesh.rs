//! The simulated stock's surface meshed on the GPU: the field that
//! `stock_surface::Snapshot` describes, sampled on a uniform grid of its
//! detail's cell and contoured by dual marching cubes (`gpu_mesh.wgsl`), as
//! fidget's octree is by dual contouring on the CPU (`Snapshot::build_cpu`),
//! in milliseconds where that takes seconds. `Snapshot::build` uses it
//! whenever there is a GPU.
//!
//! The grid is cut into bricks of `BRICK`³ cells, one workgroup each. A brick
//! is meshed only when the stock's box crosses it or a cut reaches it, and it
//! only looks at the cuts that reach it. The field is clamped to `CLAMP` cells
//! either side of 0, so a cut further than `MARGIN` cells from a brick never
//! changes its samples and can be left out.
use crate::{
    mesh::Bounds,
    simulation::CutterShape,
    stock_surface::{Cut, Snapshot, SurfaceMesh},
    volume::Frame,
};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};
use wgpu::util::DeviceExt;

const BRICK: usize = 8;
const CELLS: u64 = 512;
const CLAMP: f32 = 1.5;
/// More than `CLAMP`·√3: a cut's field passes the clamp that far outside its box.
const MARGIN: f32 = 3.;
/// Bricks a dispatch: each is short, so a slow GPU's watchdog never fires.
const BATCH: usize = 4096;
const NONE: u32 = u32::MAX;
/// Vertices a cell can number (`gpu_mesh.wgsl`'s `cells`).
const MOST_VERTICES: u64 = 1 << 24;

/// The corners edge `e` joins, the lower first: edges 4·axis..4·axis+3 run
/// along that axis (as `gpu_mesh.wgsl`'s `edge_corners`).
fn edge(e: usize) -> (usize, usize) {
    let axis = e / 4;
    let j = e % 4;
    let a = (j >> axis) << (axis + 1) | (j & ((1 << axis) - 1));
    (a, a | 1 << axis)
}
/// For each of the 256 patterns of corners holding material (bit c for
/// corner c), the pieces of surface crossing a cell, each as the loop of the
/// edges it crosses. On a face whose diagonal corners hold material, those
/// corners are cut off one by one: the face alone decides, so the cells on
/// either side of it agree.
fn pieces() -> Vec<Vec<Vec<usize>>> {
    (0..256usize)
        .map(|pattern| {
            let full = |c: usize| pattern >> c & 1 == 1;
            let crosses = |e: usize| {
                let (a, b) = edge(e);
                full(a) != full(b)
            };
            let between =
                |a: usize, b: usize| (0..12).find(|&e| edge(e) == (a.min(b), a.max(b))).unwrap();
            let mut links = vec![vec![]; 12];
            for axis in 0..3 {
                let (u, w) = ((axis + 1) % 3, (axis + 2) % 3);
                for side in 0..2 {
                    let ring = [(0, 0), (1, 0), (1, 1), (0, 1)]
                        .map(|(cu, cw)| side << axis | cu << u | cw << w);
                    let edges: [usize; 4] =
                        std::array::from_fn(|i| between(ring[i], ring[(i + 1) % 4]));
                    let crossing: Vec<usize> = (0..4).filter(|&i| crosses(edges[i])).collect();
                    let pairs = match crossing.len() {
                        0 => vec![],
                        2 => vec![(edges[crossing[0]], edges[crossing[1]])],
                        _ if full(ring[0]) => vec![(edges[3], edges[0]), (edges[1], edges[2])],
                        _ => vec![(edges[0], edges[1]), (edges[2], edges[3])],
                    };
                    for (a, b) in pairs {
                        links[a].push(b);
                        links[b].push(a);
                    }
                }
            }
            let mut used = [false; 12];
            let mut loops = vec![];
            for first in 0..12 {
                if !crosses(first) || used[first] {
                    continue;
                }
                let mut piece = vec![first];
                used[first] = true;
                let (mut previous, mut at) = (first, links[first][0]);
                while at != first {
                    used[at] = true;
                    piece.push(at);
                    let next = if links[at][0] == previous {
                        links[at][1]
                    } else {
                        links[at][0]
                    };
                    (previous, at) = (at, next);
                }
                loops.push(piece);
            }
            loops
        })
        .collect()
}
/// `pieces` as the shader reads it: two words a pattern, four bits an edge
/// for its piece (15 for none), the count from bit 16 of the second.
fn piece_words() -> Vec<u32> {
    let mut words = vec![];
    for loops in pieces() {
        let mut of = [15u32; 12];
        for (i, piece) in loops.iter().enumerate() {
            for &e in piece {
                of[e] = i as u32;
            }
        }
        let nibbles =
            |edges: &[u32]| (edges.iter().enumerate()).fold(0, |w, (i, p)| w | p << (4 * i));
        words.push(nibbles(&of[..8]));
        words.push(nibbles(&of[8..]) | (loops.len() as u32) << 16);
    }
    words
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuCut {
    m: [[f32; 4]; 3],
    d: [f32; 4],
    e: [f32; 4],
    g: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    origin: [f32; 4],
    bmin: [f32; 4],
    bmax: [f32; 4],
    extra: [f32; 4],
    limits: [u32; 4],
    grid: [u32; 4],
}
/// `cut` as the shader reads it, in the stock's frame `reference`, as
/// `Cut::field` reads it.
fn pack(cut: &Cut, reference: Frame) -> GpuCut {
    let origin = reference.transfer([0.; 3], cut.frame);
    let axes =
        [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]].map(|p| reference.transfer_vector(p, cut.frame));
    let ring = cut.shape == CutterShape::Revolve;
    let code = match cut.shape {
        CutterShape::Flat => 0.,
        CutterShape::Ball => 1.,
        CutterShape::Drill | CutterShape::Chamfer(_) => 2.,
        CutterShape::Revolve => 3.,
        CutterShape::Thread { .. } => 4.,
    };
    let (pitch, teeth, cot) = cut.shape.teeth().unwrap_or((0., 0., 0.));
    GpuCut {
        m: std::array::from_fn(|k| {
            [
                axes[0][k],
                axes[1][k],
                axes[2][k],
                origin[k] - if ring { 0. } else { cut.a[k] },
            ]
        }),
        d: [
            cut.b[0] - cut.a[0],
            cut.b[1] - cut.a[1],
            cut.b[2] - cut.a[2],
            cut.radius,
        ],
        e: [cut.flute, cut.shape.rise().unwrap_or(0.), code, pitch],
        g: if teeth > 0. {
            [teeth, cot, 0., 0.]
        } else {
            [cut.a[0], cut.b[0], cut.a[1], cut.b[1]]
        },
    }
}
/// The box of the stock's frame `reference` around what `cut` can remove.
fn reach(cut: &Cut, reference: Frame) -> Bounds {
    let (below, above) = cut.shape.span();
    let local = cut.ring().unwrap_or_else(|| Bounds {
        min: [
            cut.a[0].min(cut.b[0]) - cut.radius,
            cut.a[1].min(cut.b[1]) - cut.radius,
            cut.a[2].min(cut.b[2]) - below,
        ],
        max: [
            cut.a[0].max(cut.b[0]) + cut.radius,
            cut.a[1].max(cut.b[1]) + cut.radius,
            cut.a[2].max(cut.b[2]) + cut.flute.max(above),
        ],
    });
    cut.frame.transfer_bounds(local, reference)
}

/// What the GPU is given to mesh a snapshot.
struct Work {
    params: Params,
    cuts: Vec<GpuCut>,
    /// Per brick to mesh: its position, its first entry in `lists`, its count.
    bricks: Vec<[u32; 4]>,
    /// Each brick of the grid's index in `bricks`, or NONE.
    slots: Vec<u32>,
    lists: Vec<u32>,
    cell: f32,
}
fn work(snapshot: &Snapshot) -> Result<Work, String> {
    let bounds = snapshot.bounds;
    let size = bounds.dimensions();
    let span = size.into_iter().fold(0., f32::max) * 1.04;
    if !span.is_finite() || span <= 0. {
        return Err("Stock bounds are invalid.".into());
    }
    // The cell of fidget's octree at the same detail.
    let cell = span / (1u32 << snapshot.detail.depth()) as f32;
    // Half a cell off the box's faces, so none lies on a sample.
    let origin: [f32; 3] = std::array::from_fn(|k| bounds.min[k] - 0.5 * cell);
    let grid: [usize; 3] =
        std::array::from_fn(|k| ((size[k] / cell).ceil() as usize + 2).div_ceil(BRICK));
    if grid.iter().any(|&n| n > 1024) {
        return Err("The stock is too large for the GPU mesher's grid.".into());
    }
    let side = BRICK as f32 * cell;
    let margin = MARGIN * cell;
    let total: usize = grid.iter().product();
    let index = |b: [usize; 3]| (b[2] * grid[1] + b[1]) * grid[0] + b[0];

    // The bricks each cut reaches, clipped to the stock.
    let ranges: Vec<Option<[(usize, usize); 3]>> = (snapshot.cuts.iter())
        .map(|cut| {
            let r = reach(cut, snapshot.frame);
            let mut out = [(0, 0); 3];
            for k in 0..3 {
                let lo = r.min[k].max(bounds.min[k]) - margin;
                let hi = r.max[k].min(bounds.max[k]) + margin;
                if lo > hi {
                    return None;
                }
                let first = ((lo - origin[k]) / side).floor().max(0.) as usize;
                let last = (((hi - origin[k]) / side).floor().max(0.) as usize).min(grid[k] - 1);
                if first > last {
                    return None;
                }
                out[k] = (first, last);
            }
            Some(out)
        })
        .collect();
    let each = |range: &[(usize, usize); 3], f: &mut dyn FnMut(usize)| {
        for z in range[2].0..=range[2].1 {
            for y in range[1].0..=range[1].1 {
                for x in range[0].0..=range[0].1 {
                    f(index([x, y, z]));
                }
            }
        }
    };
    let mut counts = vec![0u32; total];
    for range in ranges.iter().flatten() {
        each(range, &mut |b| counts[b] += 1);
    }
    // The bricks the box's surface crosses, or a cut reaches.
    let (centre, radius) = crate::stock::bar(bounds);
    let mut bricks = vec![];
    let mut slots = vec![NONE; total];
    let mut entries = 0usize;
    for z in 0..grid[2] {
        for y in 0..grid[1] {
            for x in 0..grid[0] {
                let b = [x, y, z];
                let lo: [f32; 3] = std::array::from_fn(|k| origin[k] + b[k] as f32 * side);
                let hi: [f32; 3] = std::array::from_fn(|k| lo[k] + side);
                let axes = if snapshot.round { 2..3 } else { 0..3 };
                let mut outside =
                    (axes.clone()).any(|k| hi[k] < bounds.min[k] || lo[k] > bounds.max[k]);
                let mut inside =
                    (axes.clone()).all(|k| lo[k] > bounds.min[k] && hi[k] < bounds.max[k]);
                if snapshot.round {
                    let near = (0..2)
                        .map(|k| (centre[k] - centre[k].clamp(lo[k], hi[k])).powi(2))
                        .sum::<f32>()
                        .sqrt();
                    let far = (0..2)
                        .map(|k| {
                            (centre[k] - lo[k])
                                .abs()
                                .max((hi[k] - centre[k]).abs())
                                .powi(2)
                        })
                        .sum::<f32>()
                        .sqrt();
                    outside |= near > radius;
                    inside &= far < radius;
                }
                let i = index(b);
                if outside || (inside && counts[i] == 0) {
                    continue;
                }
                slots[i] = bricks.len() as u32;
                bricks.push([(x | y << 10 | z << 20) as u32, entries as u32, counts[i], 0]);
                entries += counts[i] as usize;
            }
        }
    }
    let mut lists = vec![0u32; entries.max(1)];
    let mut filled = vec![0u32; bricks.len()];
    for (c, range) in ranges.iter().enumerate() {
        if let Some(range) = range {
            each(range, &mut |b| {
                if slots[b] != NONE {
                    let s = slots[b] as usize;
                    lists[(bricks[s][1] + filled[s]) as usize] = c as u32;
                    filled[s] += 1;
                }
            });
        }
    }
    let mut cuts: Vec<GpuCut> = (snapshot.cuts.iter())
        .map(|c| pack(c, snapshot.frame))
        .collect();
    if cuts.is_empty() {
        cuts.push(bytemuck::Zeroable::zeroed());
    }
    Ok(Work {
        params: Params {
            origin: [origin[0], origin[1], origin[2], cell],
            bmin: [
                bounds.min[0],
                bounds.min[1],
                bounds.min[2],
                if snapshot.round { 1. } else { 0. },
            ],
            bmax: [bounds.max[0], bounds.max[1], bounds.max[2], radius],
            extra: [centre[0], centre[1], CLAMP * cell, 0.],
            limits: [0, 0, bricks.len() as u32, 0],
            grid: [grid[0] as u32, grid[1] as u32, grid[2] as u32, BATCH as u32],
        },
        cuts,
        bricks,
        slots,
        lists,
        cell,
    })
}

/// A GPU device with the mesher's two passes ready.
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    vertices: wgpu::ComputePipeline,
    faces: wgpu::ComputePipeline,
    pieces: wgpu::Buffer,
    /// One mesh at a time: wgpu's error scopes are the device's, not a thread's.
    busy: Mutex<()>,
    /// Set when the device is lost or reports an error outside a mesh: the
    /// CPU meshes from then on.
    broken: Arc<AtomicBool>,
    /// The adapter's name, for the timing bench.
    #[cfg_attr(not(test), allow(dead_code))]
    pub adapter: String,
}
/// Where `mesh_timed`'s time went (ms), and how much work it found, for the
/// timing bench.
#[derive(Debug, Default)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct Timing {
    pub bin: f64,
    pub gpu: f64,
    pub read: f64,
    pub total: f64,
    pub bricks: usize,
    pub meshed: usize,
    pub entries: usize,
}
/// The GPU every stock mesh shares, made the first time one is wanted; None
/// without a usable one.
pub fn shared() -> Option<&'static Gpu> {
    static SHARED: OnceLock<Option<Gpu>> = OnceLock::new();
    SHARED
        .get_or_init(Gpu::new)
        .as_ref()
        .filter(|gpu| !gpu.broken.load(Ordering::Relaxed))
}
impl Gpu {
    fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok()?;
        if adapter.get_info().device_type == wgpu::DeviceType::Cpu {
            return None;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("stock mesher"),
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .ok()?;
        let broken = Arc::new(AtomicBool::new(false));
        let lost = broken.clone();
        device.set_device_lost_callback(move |_, _| lost.store(true, Ordering::Relaxed));
        let failed = broken.clone();
        device.on_uncaptured_error(Arc::new(move |e| {
            eprintln!("GPU stock mesher: {e}");
            failed.store(true, Ordering::Relaxed);
        }));
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("stock mesher"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu_mesh.wgsl").into()),
        });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let (vertices, faces) = (pipeline("vertices_main"), pipeline("faces_main"));
        let pieces = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pieces"),
            contents: bytemuck::cast_slice(&piece_words()),
            usage: wgpu::BufferUsages::STORAGE,
        });
        if let Some(e) = pollster::block_on(device.pop_error_scope()) {
            eprintln!("GPU stock mesher: {e}");
            return None;
        }
        Some(Self {
            adapter: adapter.get_info().name,
            device,
            queue,
            vertices,
            faces,
            pieces,
            busy: Mutex::new(()),
            broken,
        })
    }
    /// The surface of the stock `snapshot` holds, at its detail's cell.
    pub fn mesh(&self, snapshot: &Snapshot) -> Result<SurfaceMesh, String> {
        self.mesh_timed(snapshot).map(|(mesh, _)| mesh)
    }
    pub fn mesh_timed(&self, snapshot: &Snapshot) -> Result<(SurfaceMesh, Timing), String> {
        let _busy = self.busy.lock().unwrap_or_else(|e| e.into_inner());
        let started = std::time::Instant::now();
        let work = work(snapshot)?;
        let mut timing = Timing {
            bin: started.elapsed().as_secs_f64() * 1e3,
            bricks: work.slots.len(),
            meshed: work.bricks.len(),
            entries: work.bricks.iter().map(|b| b[2] as usize).sum(),
            ..Default::default()
        };
        let (vertices, triangles) = if work.bricks.is_empty() {
            (vec![], vec![])
        } else {
            self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            self.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let result = self.run(&work, &mut timing);
            let invalid = pollster::block_on(self.device.pop_error_scope());
            let memory = pollster::block_on(self.device.pop_error_scope());
            if let Some(e) = invalid.or(memory) {
                return Err(format!("{e}"));
            }
            if self.broken.load(Ordering::Relaxed) {
                return Err("The GPU was lost.".into());
            }
            result?
        };
        timing.total = started.elapsed().as_secs_f64() * 1e3;
        Ok((
            SurfaceMesh {
                vertices,
                triangles,
                cell_mm: work.cell,
                milliseconds: timing.total as u128,
            },
            timing,
        ))
    }
    #[allow(clippy::type_complexity)]
    fn run(
        &self,
        work: &Work,
        timing: &mut Timing,
    ) -> Result<(Vec<[f32; 3]>, Vec<[usize; 3]>), String> {
        let gpu_started = std::time::Instant::now();
        let limits = self.device.limits();
        let most = (limits.max_storage_buffer_binding_size as u64).min(limits.max_buffer_size);
        let meshed = work.bricks.len() as u64;
        if meshed * CELLS * 4 > most {
            return Err("The stock has too many cells for one GPU buffer.".into());
        }
        let storage = |label: &str, contents: &[u8]| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        let cuts = storage("cuts", bytemuck::cast_slice(&work.cuts));
        let lists = storage("lists", bytemuck::cast_slice(&work.lists));
        let bricks = storage("bricks", bytemuck::cast_slice(&work.bricks));
        let slots = storage("slots", bytemuck::cast_slice(&work.slots));
        let cells = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cells"),
            size: meshed * CELLS * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        // A guess of the surface's size; measured, and run again, when short.
        let mut capacity = [(meshed * 192).clamp(1024, most / 12), 0];
        capacity[1] = (capacity[0] * 3).min(most / 12);
        let (counts, vertices, indices) = loop {
            let counters = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("counters"),
                    contents: bytemuck::cast_slice(&[0u32; 2]),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                });
            let output = |label, count: u64| {
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: count * 12,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                })
            };
            let vertices = output("vertices", capacity[0]);
            let indices = output("indices", capacity[1]);
            for (pipeline, bindings) in [
                (
                    &self.vertices,
                    vec![
                        (1, &cuts),
                        (2, &lists),
                        (3, &bricks),
                        (5, &counters),
                        (6, &vertices),
                        (7, &cells),
                        (9, &self.pieces),
                    ],
                ),
                (
                    &self.faces,
                    vec![
                        (3, &bricks),
                        (4, &slots),
                        (5, &counters),
                        (7, &cells),
                        (8, &indices),
                        (9, &self.pieces),
                    ],
                ),
            ] {
                for first in (0..work.bricks.len()).step_by(BATCH) {
                    let mut params = work.params;
                    params.limits = [
                        capacity[0] as u32,
                        capacity[1] as u32,
                        work.bricks.len() as u32,
                        first as u32,
                    ];
                    let params =
                        self.device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("params"),
                                contents: bytemuck::bytes_of(&params),
                                usage: wgpu::BufferUsages::UNIFORM,
                            });
                    let entries: Vec<_> = std::iter::once((0, &params))
                        .chain(bindings.iter().copied())
                        .map(|(binding, buffer)| wgpu::BindGroupEntry {
                            binding,
                            resource: buffer.as_entire_binding(),
                        })
                        .collect();
                    let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: None,
                        layout: &pipeline.get_bind_group_layout(0),
                        entries: &entries,
                    });
                    let mut encoder = self.device.create_command_encoder(&Default::default());
                    {
                        let mut pass = encoder.begin_compute_pass(&Default::default());
                        pass.set_pipeline(pipeline);
                        pass.set_bind_group(0, &group, &[]);
                        pass.dispatch_workgroups(BATCH.min(work.bricks.len() - first) as u32, 1, 1);
                    }
                    self.queue.submit([encoder.finish()]);
                }
            }
            let counts = self.read(&[(&counters, 8)])?;
            let counts: [u64; 2] = std::array::from_fn(|k| {
                u32::from_le_bytes(counts[0][k * 4..k * 4 + 4].try_into().unwrap()) as u64
            });
            if counts[0] <= capacity[0] && counts[1] <= capacity[1] {
                break (counts, vertices, indices);
            }
            if counts[0] >= MOST_VERTICES || counts.iter().any(|&c| c * 12 > most) {
                return Err("The stock's surface does not fit the GPU's buffers.".into());
            }
            capacity = [capacity[0].max(counts[0]), capacity[1].max(counts[1])];
        };
        timing.gpu = gpu_started.elapsed().as_secs_f64() * 1e3;
        let read_started = std::time::Instant::now();
        let data = self.read(&[(&vertices, counts[0] * 12), (&indices, counts[1] * 12)])?;
        let vertices = (bytemuck::cast_slice::<u8, f32>(&data[0]).chunks_exact(3))
            .map(|v| [v[0], v[1], v[2]])
            .collect();
        let triangles = (bytemuck::cast_slice::<u8, u32>(&data[1]).chunks_exact(3))
            .map(|t| [t[0] as usize, t[1] as usize, t[2] as usize])
            .collect();
        timing.read = read_started.elapsed().as_secs_f64() * 1e3;
        Ok((vertices, triangles))
    }
    /// The first `size` bytes of each buffer, once the work queued is done.
    fn read(&self, buffers: &[(&wgpu::Buffer, u64)]) -> Result<Vec<Vec<u8>>, String> {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let staging: Vec<_> = (buffers.iter())
            .map(|&(buffer, size)| {
                let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("read"),
                    size: size.max(4),
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                if size > 0 {
                    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, size);
                }
                staging
            })
            .collect();
        self.queue.submit([encoder.finish()]);
        for s in &staging {
            s.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        }
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| format!("{e}"))?;
        Ok(staging
            .iter()
            .zip(buffers)
            .map(|(s, &(_, size))| s.slice(..).get_mapped_range()[..size as usize].to_vec())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stock_surface::Detail;
    use fidget::{shape::EzShape, vm::VmShape};
    use std::sync::Arc;

    /// Corner `c` of a cell, in cells from its lowest corner: bit k is axis k.
    fn corner(c: usize) -> [usize; 3] {
        [c & 1, c >> 1 & 1, c >> 2 & 1]
    }
    #[test]
    fn every_crossed_edge_is_on_one_piece_of_a_closed_loop() {
        let all = pieces();
        assert!(all[0].is_empty() && all[255].is_empty());
        for (pattern, loops) in all.iter().enumerate() {
            for e in 0..12 {
                let (a, b) = edge(e);
                let crosses = (pattern >> a & 1) != (pattern >> b & 1);
                let on = loops.iter().filter(|l| l.contains(&e)).count();
                assert_eq!(on, usize::from(crosses), "pattern {pattern}, edge {e}");
            }
            // Each loop's edges follow one another round the cell's faces.
            for l in loops {
                assert!(l.len() >= 3, "pattern {pattern}: {l:?}");
                for (i, &e) in l.iter().enumerate() {
                    let next = l[(i + 1) % l.len()];
                    let face = |e: usize| {
                        let (a, b) = edge(e);
                        let (a, b) = (corner(a), corner(b));
                        (0..3)
                            .filter(move |&k| a[k] == b[k])
                            .map(move |k| (k, a[k]))
                    };
                    assert!(
                        face(e).any(|f| face(next).any(|g| g == f)),
                        "pattern {pattern}: {l:?}"
                    );
                }
            }
        }
    }

    /// How many sides of `mesh`'s triangles do not border exactly one other
    /// triangle: 0 when the surface is closed.
    fn open_sides(mesh: &SurfaceMesh) -> usize {
        let mut sides = std::collections::HashMap::new();
        for t in &mesh.triangles {
            for i in 0..3 {
                let (a, b) = (t[i], t[(i + 1) % 3]);
                *sides.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        sides.values().filter(|&&n| n != 2).count()
    }
    /// The largest, 99th-percentile and median distance of the mesh's
    /// vertices from the surface, in mm: |field| over its slope.
    fn off_surface(snapshot: &Snapshot, mesh: &SurfaceMesh) -> (f32, f32, f32) {
        let vm = VmShape::from(snapshot.field());
        let tape = vm.ez_point_tape();
        let mut eval = VmShape::new_point_eval();
        let mut f = |p: [f32; 3]| eval.eval(&tape, p[0], p[1], p[2]).unwrap().0;
        let step = (mesh.vertices.len() / 20_000).max(1);
        let mut errors: Vec<f32> = (mesh.vertices.iter().step_by(step))
            .map(|&v| {
                let h = 1e-3;
                let slope = (0..3)
                    .map(|k| {
                        let (mut a, mut b) = (v, v);
                        a[k] -= h;
                        b[k] += h;
                        ((f(b) - f(a)) / (2. * h)).powi(2)
                    })
                    .sum::<f32>()
                    .sqrt();
                f(v).abs() / slope.max(1.)
            })
            .collect();
        errors.sort_by(f32::total_cmp);
        let at = |q: f32| {
            (errors.get((errors.len() as f32 * q) as usize))
                .copied()
                .unwrap_or(0.)
        };
        (errors.last().copied().unwrap_or(0.), at(0.99), at(0.5))
    }
    fn frame(rotation: Option<(&str, &str)>) -> Frame {
        let mesh =
            crate::mesh::normalize(vec![[[0., 0., 0.], [20., 0., 0.], [0., 20., 10.]]]).unwrap();
        let mut setup = crate::model::Setup::new(crate::model::Machine::new(3), "G54");
        if let Some((k, v)) = rotation {
            setup.p.insert(k.into(), v.into());
        }
        Frame::new(&setup, &mesh)
    }
    fn cut(a: [f32; 3], b: [f32; 3], shape: CutterShape, frame: Frame) -> Cut {
        Cut {
            a,
            b,
            radius: 3.,
            flute: 12.,
            shape,
            frame,
            source: (0, 0),
        }
    }
    fn block(cuts: Vec<Cut>, detail: Detail) -> Snapshot {
        Snapshot {
            bounds: Bounds {
                min: [-10., -10., 0.],
                max: [10., 10., 10.],
            },
            frame: frame(None),
            cuts: Arc::new(cuts),
            detail,
            round: false,
            bar_x: None,
        }
    }
    fn gpu() -> Option<&'static Gpu> {
        let gpu = shared();
        if gpu.is_none() {
            eprintln!("No GPU adapter: skipped.");
        }
        gpu
    }

    #[test]
    fn the_gpu_mesh_follows_every_cutter_and_is_closed() {
        let Some(gpu) = gpu() else { return };
        let plain = frame(None);
        let turned = frame(Some(("rx", "90")));
        let mut cases: Vec<(String, Vec<Cut>, bool)> = vec![];
        for shape in [
            CutterShape::Flat,
            CutterShape::Ball,
            CutterShape::Drill,
            CutterShape::Chamfer(900),
            CutterShape::Thread {
                pitch: 1250,
                teeth: 3,
                angle: 60,
            },
        ] {
            cases.push((
                format!("{shape:?} ramps and plunges"),
                vec![
                    cut([-8., -4., 6.], [8., -3., 3.], shape, plain),
                    cut([-6., 5., 2.], [7., 2., 8.], shape, plain),
                    cut([0., 0., 9.], [0., 0., 1.], shape, plain),
                ],
                false,
            ));
        }
        cases.push((
            "a cut in a turned setup".into(),
            vec![cut([-4., 2., 1.], [5., 2., 4.], CutterShape::Ball, turned)],
            false,
        ));
        cases.push((
            "a lathe ring on a round bar".into(),
            vec![Cut {
                a: [3., 4., 0.],
                b: [7., 20., 0.],
                radius: 20.,
                flute: 0.,
                shape: CutterShape::Revolve,
                frame: plain,
                source: (0, 0),
            }],
            true,
        ));
        for (name, cuts, round) in cases {
            let snapshot = Snapshot {
                round,
                ..block(cuts, Detail::Standard)
            };
            let mesh = gpu.mesh(&snapshot).unwrap();
            assert!(!mesh.triangles.is_empty(), "{name}");
            assert_eq!(open_sides(&mesh), 0, "{name}");
            // On the surface but where a sliver of stock between two cuts is
            // thinner than a cell, as fidget's mesh is (0.1 mm off there).
            let (worst, p99, median) = off_surface(&snapshot, &mesh);
            assert!(
                median < 0.001 && p99 < 0.01 && worst < 3. * mesh.cell_mm,
                "{name}: worst {worst} 99% {p99} median {median} mm"
            );
        }
    }
    #[test]
    fn the_gpu_mesh_keeps_a_flat_end_mills_radius_and_exact_flat_faces() {
        let Some(gpu) = gpu() else { return };
        let c = cut([0., 0., 12.], [0., 0., 3.], CutterShape::Flat, frame(None));
        let mesh = gpu.mesh(&block(vec![c], Detail::Standard)).unwrap();
        assert_eq!(open_sides(&mesh), 0);
        let wall: Vec<_> = (mesh.vertices.iter())
            .filter(|p| p[2] > 3.2 && p[2] < 9.8 && p[0].abs() < 4. && p[1].abs() < 4.)
            .collect();
        assert!(wall.len() > 30);
        for p in wall {
            assert!((p[0].hypot(p[1]) - 3.).abs() < 0.01, "wall {p:?}");
        }
        // The hole's floor and the block's underside are flat, and the
        // block's edges and corners sharp: on its faces, not cut across.
        for p in (mesh.vertices.iter()).filter(|p| p[0].abs() < 2. && p[1].abs() < 2.) {
            assert!(p[2].abs() < 0.001 || (p[2] - 3.).abs() < 0.001, "{p:?}");
        }
        for p in &mesh.vertices {
            let on = (0..3)
                .filter(|&k| {
                    let [lo, hi] = [[-10., -10., 0.][k], [10., 10., 10.][k]];
                    (p[k] - lo).abs() < 0.001 || (p[k] - hi).abs() < 0.001
                })
                .count();
            let hole = p[0].hypot(p[1]) < 3.01;
            assert!(on > 0 || hole, "{p:?} is on no face");
        }
        for corner in [[-10., -10., 0.], [10., 10., 10.], [-10., 10., 10.]] {
            assert!(
                (mesh.vertices.iter()).any(|p| (0..3).all(|k| (p[k] - corner[k]).abs() < 0.001)),
                "no vertex at the corner {corner:?}"
            );
        }
    }
    #[test]
    fn the_gpu_mesh_keeps_ball_drill_and_ramp_surfaces() {
        let Some(gpu) = gpu() else { return };
        let f = frame(None);
        let ball = gpu
            .mesh(&block(
                vec![cut([0., 0., 12.], [0., 0., 3.], CutterShape::Ball, f)],
                Detail::Draft,
            ))
            .unwrap();
        assert_eq!(open_sides(&ball), 0);
        let mut count = 0;
        for p in &ball.vertices {
            if p[2] > 3.05 && p[2] < 5.7 && p[0].abs() < 3.1 && p[1].abs() < 3.1 {
                let radius = (p[0] * p[0] + p[1] * p[1] + (p[2] - 6.).powi(2)).sqrt();
                assert!((radius - 3.).abs() < 0.03, "ball {p:?}, radius {radius}");
                count += 1;
            }
        }
        assert!(count > 30);
        let drill = gpu
            .mesh(&block(
                vec![cut([0., 0., 12.], [0., 0., 3.], CutterShape::Drill, f)],
                Detail::Standard,
            ))
            .unwrap();
        assert_eq!(open_sides(&drill), 0);
        let (mut cone, mut wall) = (0, 0);
        for p in &drill.vertices {
            let distance = p[0].hypot(p[1]);
            if distance > 0.3 && distance < 2.8 && p[2] > 2.5 && p[2] < 5. {
                let expected = 3. + crate::simulation::POINT_RISE * distance;
                assert!((p[2] - expected).abs() < 0.03, "cone {p:?}");
                cone += 1;
            } else if p[2] > 5.2 && p[2] < 9.8 && p[0].abs() < 4. && p[1].abs() < 4. {
                assert!((distance - 3.).abs() < 0.01, "wall {p:?}");
                wall += 1;
            }
        }
        assert!(
            cone > 30 && wall > 30,
            "{cone} cone and {wall} wall vertices"
        );
        // A ramp's floor is its plane, without voxel steps.
        let ramp = gpu
            .mesh(&block(
                vec![cut([-12., 0., 2.], [12., 0., 7.], CutterShape::Flat, f)],
                Detail::Standard,
            ))
            .unwrap();
        assert_eq!(open_sides(&ramp), 0);
        let mut checked = 0;
        for p in &ramp.vertices {
            if p[0].abs() < 8. && p[1].abs() < 2. && p[2] > 0.5 && p[2] < 8. {
                let half = (9. - p[1] * p[1]).sqrt();
                let expected = 2. + (p[0] + 12. - half) / 24. * 5.;
                assert!((p[2] - expected).abs() < 0.02, "{p:?} expected {expected}");
                checked += 1;
            }
        }
        assert!(checked > 20);
    }
    #[test]
    fn the_gpu_mesh_turns_a_round_bar() {
        let Some(gpu) = gpu() else { return };
        let bar = Bounds {
            min: [-25., -25., 0.],
            max: [25., 25., 100.],
        };
        let mut v = crate::volume::Volume::bar(bar, frame(None));
        v.revolve(50., 101., 20., 26., frame(None), (0, 0));
        let mesh = gpu.mesh(&Snapshot::of(&v, Detail::Draft)).unwrap();
        assert_eq!(open_sides(&mesh), 0);
        let cell = mesh.cell_mm;
        for p in &mesh.vertices {
            let r = p[0].hypot(p[1]);
            // As fidget's: within a cell where faces meet, and on the
            // cylinders' own faces where `viewport::on_bar` takes them for
            // the bar's.
            assert!(r <= 25. + cell, "{p:?}");
            if p[2] > 51. {
                assert!(r <= 20. + cell, "{p:?}");
            }
            if p[2] > 1. && p[2] < 49. {
                assert!((r - 25.).abs() < 0.06 * cell, "{p:?}");
            } else if p[2] > 51. && p[2] < 99. && r > 10. {
                assert!((r - 20.).abs() < 0.06 * cell, "{p:?}");
            }
        }
    }

    /// The zig-zag pocket of `rows` passes on each of `levels` levels.
    fn pocket(rows: usize, levels: usize) -> crate::volume::Volume {
        let f = frame(None);
        let bounds = Bounds {
            min: [-50., -50., 0.],
            max: [50., 50., 20.],
        };
        let mut v = crate::volume::Volume::new(bounds, f);
        for l in 0..levels {
            let z = 19. - l as f32 * 18. / levels as f32;
            for r in 0..rows {
                let y = -40. + 80. * r as f32 / rows as f32;
                let (x0, x1) = if r % 2 == 0 { (-40., 40.) } else { (40., -40.) };
                v.cut(
                    [x0, y, z],
                    [x1, y, z],
                    3.,
                    12.,
                    CutterShape::Flat,
                    f,
                    (0, 0),
                );
                let y2 = -40. + 80. * (r + 1) as f32 / rows as f32;
                v.cut(
                    [x1, y, z],
                    [x1, y2, z],
                    3.,
                    12.,
                    CutterShape::Flat,
                    f,
                    (0, 0),
                );
            }
        }
        v
    }
    fn write_png(path: &std::path::Path, volume: &crate::volume::Volume, mesh: &SurfaceMesh) {
        let triangles: Vec<_> =
            crate::viewport::stock_triangles(volume, mesh, volume.frame).collect();
        let Some(iced::widget::image::Handle::Rgba {
            width,
            height,
            pixels,
            ..
        }) = crate::viewport::stock_picture(&triangles, (800, 600))
        else {
            return;
        };
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
    }

    /// Times fidget against the GPU at each detail, on the example block's
    /// whole job and on two zig-zag pockets, and draws both meshes to
    /// `target/gpu_mesh/`. `cargo test --release gpu_mesh_timing -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn gpu_mesh_timing() {
        let Some(gpu) = gpu() else { return };
        eprintln!("GPU: {}", gpu.adapter);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/gpu_mesh");
        std::fs::create_dir_all(&dir).unwrap();
        let (doc, mesh, report) = crate::proposal::tests::block();
        let job =
            crate::proposal::build(&doc, &mesh, &report, &crate::proposal::tests::good(&report))
                .unwrap();
        let si = job.setups.len() - 1;
        let oi = (job.setups[si].operations.iter())
            .rposition(|o| o.enabled)
            .unwrap();
        let mut sim = crate::simulation::prepare(&job, &mesh, si, oi).unwrap();
        sim.seek(1.);
        let cases = [
            ("block", sim.volume.unwrap()),
            ("pocket-1280", pocket(80, 8)),
            ("pocket-10000", pocket(250, 20)),
        ];
        eprintln!(
            "{:<13} {:>6} {:<8} | {:>8} {:>8} | {:>8} {:>6} {:>6} {:>6} {:>8} {:>13} {:>9} | {:>5} {:>6} {:>6} | {:>6} {:>6}",
            "case",
            "cuts",
            "detail",
            "fidget",
            "tris",
            "gpu",
            "bin",
            "run",
            "read",
            "tris",
            "bricks",
            "lists",
            "open",
            "worst",
            "median",
            "draw",
            "draw"
        );
        for (name, volume) in &cases {
            for detail in Detail::ALL {
                let snapshot = Snapshot::of(volume, detail);
                let cpu = snapshot.build_cpu().unwrap();
                // The first run warms the GPU's caches; time the second.
                gpu.mesh(&snapshot).unwrap();
                let (surface, t) = gpu.mesh_timed(&snapshot).unwrap();
                let (worst, _, median) = off_surface(&snapshot, &surface);
                // What the 3D view pays a frame for each, at a window's size.
                let draw = |surface: &SurfaceMesh| {
                    let triangles: Vec<_> =
                        crate::viewport::stock_triangles(volume, surface, volume.frame).collect();
                    let started = std::time::Instant::now();
                    crate::viewport::stock_picture(&triangles, (1600, 1000));
                    started.elapsed().as_secs_f64() * 1e3
                };
                eprintln!(
                    "{:<13} {:>6} {:<8} | {:>6}ms {:>8} | {:>6.0}ms {:>4.0}ms {:>4.0}ms {:>4.0}ms {:>8} {:>6}/{:<6} {:>9} | {:>5} {:>6.3} {:>6.4} | {:>4.0}ms {:>4.0}ms",
                    name,
                    volume.cuts.len(),
                    detail.to_string(),
                    cpu.milliseconds,
                    cpu.triangles.len(),
                    t.total,
                    t.bin,
                    t.gpu,
                    t.read,
                    surface.triangles.len(),
                    t.meshed,
                    t.bricks,
                    t.entries,
                    open_sides(&surface),
                    worst,
                    median,
                    draw(&cpu),
                    draw(&surface),
                );
                if detail == Detail::Standard {
                    write_png(&dir.join(format!("{name}-fidget.png")), volume, &cpu);
                    write_png(&dir.join(format!("{name}-gpu.png")), volume, &surface);
                }
            }
        }
    }
}
