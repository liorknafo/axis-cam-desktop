// The stock's field (`stock_surface::Snapshot::field`) sampled over one
// brick of BRICK³ cells per workgroup and contoured by dual marching cubes:
// `vertices_main` puts a vertex on each piece of surface crossing a cell,
// where the planes of its crossings meet, so edges and corners stay sharp;
// `faces_main` makes a quad of the four pieces round each crossed edge. A
// cell's crossed edges are grouped into pieces as `gpu_mesh::pieces` does, the
// same way on each side of a face, so the surface is closed.
// Material is where the field is below 0.

struct Params {
    // xyz: the grid's first sample (mm); w: the cell (mm).
    origin: vec4f,
    // The stock's box; bmin.w is 1 for a round bar, bmax.w its radius.
    bmin: vec4f,
    bmax: vec4f,
    // xy: the bar's centre; z: the clamp, the field's limit either side of 0.
    extra: vec4f,
    // x: vertex capacity; y: triangle capacity; z: bricks to mesh; w: this
    // dispatch's first brick.
    limits: vec4u,
    // xyz: bricks an axis; w: workgroups per dispatch row.
    grid: vec4u,
}

// One cut, as `pack` lays it out: rows m0..m2 take a point of the stock's
// frame to the cut's, less its start `a` (with `a`, for a ring).
struct Cut {
    m0: vec4f,
    m1: vec4f,
    m2: vec4f,
    // xyz: b - a; w: radius.
    d: vec4f,
    // x: flute; y: the cone's rise; z: the shape (0 flat, 1 ball, 2 cone, 3 ring,
    // 4 thread mill); w: a thread mill's tooth spacing.
    e: vec4f,
    // A ring's z from, z to, inner and outer radius; a thread mill's teeth and
    // the cotangent of half its angle.
    g: vec4f,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> cuts: array<Cut>;
@group(0) @binding(2) var<storage, read> lists: array<u32>;
// x: the brick's position (10 bits an axis); y: its first entry in `lists`; z: its count.
@group(0) @binding(3) var<storage, read> bricks: array<vec4u>;
// Each brick of the grid's index in `bricks`, or NONE.
@group(0) @binding(4) var<storage, read> slots: array<u32>;
// The vertices and triangles made so far.
@group(0) @binding(5) var<storage, read_write> counters: array<atomic<u32>, 2>;
@group(0) @binding(6) var<storage, read_write> vertices: array<f32>;
// Per cell of each brick: its corners holding material (bit c for corner c),
// and from bit 8 its pieces' first vertex.
@group(0) @binding(7) var<storage, read_write> cells: array<u32>;
@group(0) @binding(8) var<storage, read_write> indices: array<u32>;
// Per pattern of corners holding material, two words: four bits an edge, the
// piece of surface crossing it (15 for none), and from bit 16 of the second
// the number of pieces.
@group(0) @binding(9) var<storage, read> pieces: array<u32>;

const BRICK: u32 = 8u;
const CELLS: u32 = 512u;
const SIDE: u32 = 9u;
const SAMPLES: u32 = 729u;
const THREADS: u32 = 64u;
const PER: u32 = 12u;
const NONE: u32 = 0xffffffffu;
const NO_PIECE: u32 = 15u;

var<workgroup> grid: array<f32, 729>;
var<workgroup> shared_cuts: array<Cut, 64>;
var<workgroup> wg_start: u32;
var<workgroup> wg_count: u32;

fn box_field(x: vec3f) -> f32 {
    let a = max(params.bmin.xyz - x, x - params.bmax.xyz);
    if params.bmin.w > 0.5 {
        return max(length(x.xy - params.extra.xy) - params.bmax.w, a.z);
    }
    return max(a.x, max(a.y, a.z));
}

fn cut_field(c: Cut, x: vec3f) -> f32 {
    let p = vec3f(
        dot(c.m0.xyz, x) + c.m0.w,
        dot(c.m1.xyz, x) + c.m1.w,
        dot(c.m2.xyz, x) + c.m2.w,
    );
    let shape = u32(c.e.z);
    if shape == 3u {
        let radial = length(p.xy);
        return max(max(c.g.x - p.z, p.z - c.g.y), max(c.g.z - radial, radial - c.g.w));
    }
    let d = c.d.xyz;
    let r = c.d.w;
    let flute = c.e.x;
    let rise = c.e.y;
    let l = length(d.xy);
    if shape == 4u {
        // As `CutterShape::thread_field`.
        var rho = length(p.xy);
        if l >= 1e-6 {
            let u = (p.x * d.x + p.y * d.y) / l;
            let v = (p.y * d.x - p.x * d.y) / l;
            let nearest = min(max(u, 0.0), l);
            rho = sqrt((u - nearest) * (u - nearest) + v * v);
        }
        let h = p.z - min(d.z, 0.0);
        let up = abs(d.z);
        let pitch = c.e.w;
        let teeth = c.g.x;
        let cot = c.g.y;
        let depth = 0.65 * pitch;
        var e = 1e9;
        for (var k = 0.0; k < teeth; k += 1.0) {
            let at = k * pitch;
            e = min(e, max(max(at - h, h - at - up), 0.0));
        }
        let ends = max(max(-h, h - (teeth - 1.0) * pitch - up), 0.0);
        return max(rho - max(r - e * cot, r - depth), ends - depth / cot);
    }
    if l < 1e-6 {
        let radial = length(p.xy) - r;
        var bottom = min(d.z, 0.0);
        if shape == 1u {
            bottom = min(d.z, 0.0) + r - sqrt(max(r * r - dot(p.xy, p.xy), 0.0));
        } else if shape == 2u {
            bottom = min(d.z, 0.0) + length(p.xy) * rise;
        }
        return max(max(radial, bottom - p.z), p.z - (max(d.z, 0.0) + flute));
    }
    let u = (p.x * d.x + p.y * d.y) / l;
    let v = (p.y * d.x - p.x * d.y) / l;
    let nearest = min(max(u, 0.0), l);
    let radial = sqrt((u - nearest) * (u - nearest) + v * v) - r;
    let reach = sqrt(max(r * r - v * v, 0.0));
    var lo = min(max((u - reach) / l, 0.0), 1.0);
    var hi = min(max((u + reach) / l, 0.0), 1.0);
    var lower: f32;
    if shape == 1u || shape == 2u {
        if d.z > 1e-7 {
            lo = max(lo, min(max((p.z - flute) / d.z, 0.0), 1.0));
        } else if d.z < -1e-7 {
            hi = min(hi, min(max((p.z - flute) / d.z, 0.0), 1.0));
        }
    }
    if shape == 1u {
        let t = min(max(u / l - reach * (d.z / (l * sqrt(l * l + d.z * d.z))), lo), hi);
        let w = u - t * l;
        lower = t * d.z + r - sqrt(max(r * r - w * w - v * v, 0.0));
    } else if shape == 2u {
        let cone = rise * rise * l * l;
        var t: f32;
        if cone > d.z * d.z {
            t = u / l - abs(v) * (d.z / (l * sqrt(cone - d.z * d.z)));
        } else if d.z < 0.0 {
            t = hi;
        } else {
            t = lo;
        }
        t = min(max(t, lo), hi);
        let w = u - t * l;
        lower = t * d.z + sqrt(w * w + v * v) * rise;
    } else if d.z < 0.0 {
        lower = hi * d.z;
    } else {
        lower = lo * d.z;
    }
    var upper: f32;
    if d.z < 0.0 {
        upper = lo * d.z + flute;
    } else {
        upper = hi * d.z + flute;
    }
    return max(max(radial, lower - p.z), p.z - upper);
}

// The two cuts whose fields are lowest at `x`, of the `count` cuts from
// `start` of `lists`; NONE where there are fewer.
fn nearest(x: vec3f, start: u32, count: u32) -> vec2u {
    var best = vec2f(1e30);
    var which = vec2u(NONE);
    for (var j = 0u; j < count; j++) {
        let i = lists[start + j];
        let f = cut_field(cuts[i], x);
        if f < best.x {
            best = vec2f(f, best.x);
            which = vec2u(i, which.x);
        } else if f < best.y {
            best.y = f;
            which.y = i;
        }
    }
    return which;
}

// The field near a point, of the box and the cuts `near` alone: what sets
// it there, so its crossing and its slope are the field's.
fn near_field(x: vec3f, near: vec2u) -> f32 {
    var low = 1e30;
    if near.x != NONE {
        low = cut_field(cuts[near.x], x);
    }
    if near.y != NONE {
        low = min(low, cut_field(cuts[near.y], x));
    }
    return max(box_field(x), -low);
}

// The least-squares solution y of `ata` y = `atb`, `ata` symmetric, leaving
// out the directions it barely constrains (a flat face's own plane, a
// rounded wall's turn), by Jacobi's eigen-decomposition.
fn solve(ata: mat3x3f, atb: vec3f) -> vec3f {
    var a = ata;
    var v = mat3x3f(vec3f(1.0, 0.0, 0.0), vec3f(0.0, 1.0, 0.0), vec3f(0.0, 0.0, 1.0));
    for (var sweep = 0u; sweep < 6u; sweep++) {
        for (var r = 0u; r < 3u; r++) {
            let p = select(0u, 1u, r == 2u);
            let q = select(1u, 2u, r > 0u);
            let apq = a[q][p];
            if abs(apq) < 1e-12 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * apq);
            let t = select(1.0, sign(theta), theta != 0.0) / (abs(theta) + sqrt(theta * theta + 1.0));
            let c = 1.0 / sqrt(t * t + 1.0);
            let s = t * c;
            var j = mat3x3f(vec3f(1.0, 0.0, 0.0), vec3f(0.0, 1.0, 0.0), vec3f(0.0, 0.0, 1.0));
            j[p][p] = c;
            j[q][q] = c;
            j[q][p] = s;
            j[p][q] = -s;
            a = transpose(j) * a * j;
            v = v * j;
        }
    }
    let most = max(a[0][0], max(a[1][1], a[2][2]));
    var y = vec3f(0.0);
    for (var i = 0u; i < 3u; i++) {
        let l = a[i][i];
        if l > 0.01 * most && l > 1e-12 {
            y += v[i] * (dot(v[i], atb) / l);
        }
    }
    return y;
}

// The grid position, in cells, of sample `s` of the brick at `base`.
fn sample_at(base: vec3u, s: u32) -> vec3f {
    return vec3f(base + vec3u(s % SIDE, (s / SIDE) % SIDE, s / (SIDE * SIDE)));
}

// The corners an edge joins, the lower first: edges 4·axis..4·axis+3 run
// along that axis.
fn edge_corners(e: u32) -> vec2u {
    let axis = e / 4u;
    let j = e % 4u;
    let a = ((j >> axis) << (axis + 1u)) | (j & ((1u << axis) - 1u));
    return vec2u(a, a | (1u << axis));
}

fn corner_offset(c: u32) -> vec3f {
    return vec3f(f32(c & 1u), f32((c >> 1u) & 1u), f32((c >> 2u) & 1u));
}

fn brick_base(brick: vec4u) -> vec3u {
    return vec3u(brick.x & 1023u, (brick.x >> 10u) & 1023u, brick.x >> 20u) * BRICK;
}

// The piece of surface the pattern `pattern` has crossing edge `e`, or NO_PIECE.
fn piece(pattern: u32, e: u32) -> u32 {
    return (pieces[pattern * 2u + e / 8u] >> ((e % 8u) * 4u)) & 15u;
}

// The vertex of piece `which` of the cell at `cell` (grid cells), whose
// corners' fields are `v` and pattern `pattern`, in mm: where the planes of
// the piece's crossings meet, kept in the cell.
fn place(cell: vec3f, v: array<f32, 8>, pattern: u32, which: u32, start: u32, count: u32) -> vec3f {
    let size = params.origin.w;
    var ata = mat3x3f(vec3f(0.0), vec3f(0.0), vec3f(0.0));
    var atb = vec3f(0.0);
    var mass = vec3f(0.0);
    var crossings = 0.0;
    for (var e = 0u; e < 12u; e++) {
        if piece(pattern, e) != which {
            continue;
        }
        let ends = edge_corners(e);
        let a = corner_offset(ends.x);
        let b = corner_offset(ends.y);
        let va = v[ends.x];
        let vb = v[ends.y];
        // Where the samples put the crossing, then where the field near
        // there does (Illinois' false position): the samples are clamped and
        // a crease between them bends the field.
        var t = va / (va - vb);
        let wa = params.origin.xyz + (cell + a) * size;
        let wb = params.origin.xyz + (cell + b) * size;
        let near = nearest(wa + (wb - wa) * t, start, count);
        var lo = 0.0;
        var hi = 1.0;
        var flo = near_field(wa, near);
        var fhi = near_field(wb, near);
        if (flo < 0.0) != (fhi < 0.0) {
            var side = 0;
            for (var i = 0u; i < 12u; i++) {
                t = lo + (hi - lo) * (flo / (flo - fhi));
                let f = near_field(wa + (wb - wa) * t, near);
                if f == 0.0 || hi - lo < 1e-6 {
                    break;
                }
                if (f < 0.0) == (flo < 0.0) {
                    lo = t;
                    flo = f;
                    if side == -1 {
                        fhi *= 0.5;
                    }
                    side = -1;
                } else {
                    hi = t;
                    fhi = f;
                    if side == 1 {
                        flo *= 0.5;
                    }
                    side = 1;
                }
            }
        }
        let p = a + (b - a) * t;
        // The field's slope there, toward air.
        let x = params.origin.xyz + (cell + p) * size;
        let h = size * 0.02;
        let k0 = vec3f(1.0, -1.0, -1.0);
        let k1 = vec3f(-1.0, -1.0, 1.0);
        let k2 = vec3f(-1.0, 1.0, -1.0);
        let k3 = vec3f(1.0, 1.0, 1.0);
        let g = k0 * near_field(x + k0 * h, near) + k1 * near_field(x + k1 * h, near)
            + k2 * near_field(x + k2 * h, near) + k3 * near_field(x + k3 * h, near);
        if length(g) > 1e-12 {
            let n = normalize(g);
            ata += mat3x3f(n * n.x, n * n.y, n * n.z);
            atb += n * dot(n, p);
        }
        mass += p;
        crossings += 1.0;
    }
    mass /= crossings;
    let x = clamp(mass + solve(ata, atb - ata * mass), vec3f(0.0), vec3f(1.0));
    return params.origin.xyz + (cell + x) * size;
}

@compute @workgroup_size(64)
fn vertices_main(@builtin(workgroup_id) wid: vec3u, @builtin(local_invocation_index) li: u32) {
    let slot = params.limits.w + wid.x + wid.y * params.grid.w;
    if slot >= params.limits.z {
        return;
    }
    let brick = bricks[slot];
    if li == 0u {
        wg_start = brick.y;
        wg_count = brick.z;
    }
    let start = workgroupUniformLoad(&wg_start);
    let count = workgroupUniformLoad(&wg_count);
    let base = brick_base(brick);
    let cell = params.origin.w;
    let limit = params.extra.z;

    var outer: array<f32, 12>;
    var low: array<f32, 12>;
    for (var k = 0u; k < PER; k++) {
        let s = li + k * THREADS;
        low[k] = 1e30;
        outer[k] = 1e30;
        if s < SAMPLES {
            outer[k] = box_field(params.origin.xyz + sample_at(base, s) * cell);
        }
    }
    for (var chunk = 0u; chunk < count; chunk += THREADS) {
        if chunk + li < count {
            shared_cuts[li] = cuts[lists[start + chunk + li]];
        }
        workgroupBarrier();
        let n = min(THREADS, count - chunk);
        for (var k = 0u; k < PER; k++) {
            let s = li + k * THREADS;
            // Past the clamp either way, no further cut changes the sample.
            if s >= SAMPLES || outer[k] > limit || low[k] < -limit {
                continue;
            }
            let x = params.origin.xyz + sample_at(base, s) * cell;
            var m = low[k];
            for (var j = 0u; j < n; j++) {
                m = min(m, cut_field(shared_cuts[j], x));
                if m < -limit {
                    break;
                }
            }
            low[k] = m;
        }
        workgroupBarrier();
    }
    for (var k = 0u; k < PER; k++) {
        let s = li + k * THREADS;
        if s < SAMPLES {
            grid[s] = clamp(max(outer[k], -low[k]), -limit, limit);
        }
    }
    workgroupBarrier();

    for (var k = 0u; k < CELLS / THREADS; k++) {
        let c = li + k * THREADS;
        let at = vec3u(c % BRICK, (c / BRICK) % BRICK, c / (BRICK * BRICK));
        var v: array<f32, 8>;
        var pattern = 0u;
        for (var i = 0u; i < 8u; i++) {
            let o = vec3u(i & 1u, (i >> 1u) & 1u, (i >> 2u) & 1u) + at;
            v[i] = grid[o.x + o.y * SIDE + o.z * SIDE * SIDE];
            if v[i] < 0.0 {
                pattern |= 1u << i;
            }
        }
        var entry = pattern;
        if pattern != 0u && pattern != 255u {
            let count_pieces = (pieces[pattern * 2u + 1u] >> 16u) & 15u;
            let first = atomicAdd(&counters[0], count_pieces);
            if first + count_pieces <= params.limits.x {
                for (var which = 0u; which < count_pieces; which++) {
                    let p = place(vec3f(base + at), v, pattern, which, start, count);
                    vertices[(first + which) * 3u] = p.x;
                    vertices[(first + which) * 3u + 1u] = p.y;
                    vertices[(first + which) * 3u + 2u] = p.z;
                }
            }
            entry |= min(first, 0xffffffu) << 8u;
        }
        cells[slot * CELLS + c] = entry;
    }
}

@compute @workgroup_size(64)
fn faces_main(@builtin(workgroup_id) wid: vec3u, @builtin(local_invocation_index) li: u32) {
    let slot = params.limits.w + wid.x + wid.y * params.grid.w;
    if slot >= params.limits.z {
        return;
    }
    let base = vec3i(brick_base(bricks[slot]));
    for (var k = 0u; k < CELLS / THREADS; k++) {
        let c = li + k * THREADS;
        let pattern = cells[slot * CELLS + c] & 255u;
        let at = base + vec3i(vec3u(c % BRICK, (c / BRICK) % BRICK, c / (BRICK * BRICK)));
        let full = (pattern & 1u) == 1u;
        // The cell's edges from its lowest corner, each made a quad of the
        // four cells round it.
        for (var a = 0u; a < 3u; a++) {
            if full == (((pattern >> (1u << a)) & 1u) == 1u) {
                continue;
            }
            let u = (a + 1u) % 3u;
            let w = (a + 2u) % 3u;
            // The four cells round the edge, turning counter-clockwise seen
            // from its far end, and the piece of each that crosses it.
            var quad: array<u32, 4>;
            var whole = true;
            for (var i = 0u; i < 4u; i++) {
                let du = select(1u, 0u, i == 1u || i == 2u);
                let dw = select(1u, 0u, i >= 2u);
                var g = at;
                g[u] -= i32(du);
                g[w] -= i32(dw);
                if any(g < vec3i(0)) {
                    whole = false;
                    break;
                }
                let b = vec3u(g) / BRICK;
                if any(b >= params.grid.xyz) {
                    whole = false;
                    break;
                }
                let s = slots[b.x + params.grid.x * (b.y + params.grid.y * b.z)];
                if s == NONE {
                    whole = false;
                    break;
                }
                let o = vec3u(g) % BRICK;
                let entry = cells[s * CELLS + o.x + BRICK * (o.y + BRICK * o.z)];
                // The edge is that cell's from its corner `du` along u and
                // `dw` along w, numbered as `edge_corners` numbers them.
                let corner = (du << u) | (dw << w);
                let e = 4u * a + (((corner >> (a + 1u)) << a) | (corner & ((1u << a) - 1u)));
                let which = piece(entry & 255u, e);
                if which == NO_PIECE {
                    whole = false;
                    break;
                }
                quad[i] = (entry >> 8u) + which;
            }
            if !whole {
                continue;
            }
            // Material at the near end: the surface faces the far end, so
            // the quad keeps its turn; else it turns the other way.
            if !full {
                let second = quad[1];
                quad[1] = quad[3];
                quad[3] = second;
            }
            let t = atomicAdd(&counters[1], 2u);
            if t + 2u > params.limits.y {
                continue;
            }
            indices[t * 3u] = quad[0];
            indices[t * 3u + 1u] = quad[1];
            indices[t * 3u + 2u] = quad[2];
            indices[t * 3u + 3u] = quad[0];
            indices[t * 3u + 4u] = quad[2];
            indices[t * 3u + 5u] = quad[3];
        }
    }
}
