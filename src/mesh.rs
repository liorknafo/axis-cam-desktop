use sha2::{Digest, Sha256};
use std::{cell::RefCell, fs, path::Path};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}
impl Bounds {
    pub fn dimensions(self) -> [f32; 3] {
        std::array::from_fn(|i| self.max[i] - self.min[i])
    }
    pub fn scaled(self, scale: f32) -> Self {
        Self {
            min: self.min.map(|v| v * scale),
            max: self.max.map(|v| v * scale),
        }
    }
    pub fn translated(self, delta: [f32; 3]) -> Self {
        Self {
            min: std::array::from_fn(|i| self.min[i] + delta[i]),
            max: std::array::from_fn(|i| self.max[i] + delta[i]),
        }
    }
}
pub fn rotation(mut p: [f32; 3], angles: [f32; 3]) -> [f32; 3] {
    let (s, c) = angles[0].to_radians().sin_cos();
    p = [p[0], p[1] * c - p[2] * s, p[1] * s + p[2] * c];
    let (s, c) = angles[1].to_radians().sin_cos();
    p = [p[0] * c + p[2] * s, p[1], -p[0] * s + p[2] * c];
    let (s, c) = angles[2].to_radians().sin_cos();
    [p[0] * c - p[1] * s, p[0] * s + p[1] * c, p[2]]
}
#[derive(Debug, Clone)]
pub struct Face {
    pub points: Vec<[f32; 3]>,
    pub feature: Option<usize>,
    pub tone: f32,
}
#[derive(Debug, Clone)]
pub struct Mesh {
    pub faces: Vec<Face>,
    pub surfaces: Vec<crate::surface::Surface>,
    pub fingerprint: String,
    pub dimensions: [f32; 3],
    pub triangles: usize,
    pub sampled: bool,
    pub units: &'static str,
    /// Display-space position of the source file's (0,0,0).
    pub origin: [f32; 3],
    rotated_bounds: RefCell<Option<([f32; 3], Bounds)>>,
}
impl Mesh {
    /// The preview normalization is uniform; use the same factor for mm stock.
    pub fn display_scale(&self) -> f32 {
        140. / self.dimensions.into_iter().fold(0., f32::max)
    }
    pub fn rotated_bounds(&self, angles: [f32; 3]) -> Bounds {
        if let Some((previous, bounds)) = *self.rotated_bounds.borrow()
            && previous == angles
        {
            return bounds;
        }
        let mut bounds = Bounds {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        };
        for p in self.faces.iter().flat_map(|f| &f.points) {
            let p = rotation(*p, angles);
            for (i, v) in p.into_iter().enumerate() {
                bounds.min[i] = bounds.min[i].min(v);
                bounds.max[i] = bounds.max[i].max(v);
            }
        }
        *self.rotated_bounds.borrow_mut() = Some((angles, bounds));
        bounds
    }
}
pub fn load(path: &Path) -> Result<Mesh, String> {
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > 50_000_000 {
        return Err("Mesh preview accepts files up to 50 MB.".into());
    }
    let ext = path
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_lowercase();
    let (triangles, units) = if matches!(ext.as_str(), "step" | "stp") {
        (crate::step::load(path)?, "mm")
    } else {
        let b = fs::read(path).map_err(|e| e.to_string())?;
        match ext.as_str() {
            "stl" => (stl(&b)?, "source units"),
            "obj" => (
                obj(std::str::from_utf8(&b).map_err(|_| "OBJ must be UTF-8 text")?)?,
                "source units",
            ),
            "3mf" => (crate::three_mf::load(&b)?, "mm"),
            _ => return Err("Choose STEP, STP, 3MF, STL or OBJ.".into()),
        }
    };
    let mut mesh = normalize(triangles)?;
    mesh.units = units;
    Ok(mesh)
}
pub(crate) fn stl(b: &[u8]) -> Result<Vec<[[f32; 3]; 3]>, String> {
    if b.len() >= 84 {
        let n = u32::from_le_bytes(b[80..84].try_into().unwrap()) as usize;
        if 84usize.checked_add(n.checked_mul(50).ok_or("Invalid STL size")?) == Some(b.len()) {
            let mut out = Vec::with_capacity(n);
            for chunk in b[84..].chunks_exact(50) {
                let mut tri = [[0.; 3]; 3];
                for (v, p) in tri.iter_mut().enumerate() {
                    for (c, x) in p.iter_mut().enumerate() {
                        let k = 12 + v * 12 + c * 4;
                        *x = f32::from_le_bytes(chunk[k..k + 4].try_into().unwrap());
                    }
                }
                out.push(tri)
            }
            return Ok(out);
        }
    }
    let text = std::str::from_utf8(b).map_err(|_| "Invalid binary or ASCII STL")?;
    let mut vertices = vec![];
    for line in text.lines() {
        let mut t = line.split_whitespace();
        if t.next() == Some("vertex") {
            let mut v = [0.; 3];
            for x in &mut v {
                *x = t
                    .next()
                    .ok_or("Incomplete STL vertex")?
                    .parse()
                    .map_err(|_| "Invalid STL number")?;
            }
            vertices.push(v)
        }
    }
    if vertices.is_empty() || vertices.len() % 3 != 0 {
        return Err("STL must contain complete triangles.".into());
    }
    Ok(vertices
        .chunks_exact(3)
        .map(|v| [v[0], v[1], v[2]])
        .collect())
}
fn obj(text: &str) -> Result<Vec<[[f32; 3]; 3]>, String> {
    let mut vertices: Vec<[f32; 3]> = vec![];
    let mut out = vec![];
    for line in text.lines() {
        let mut t = line.split_whitespace();
        match t.next() {
            Some("v") => {
                let mut v = [0.; 3];
                for x in &mut v {
                    *x = t
                        .next()
                        .ok_or("Incomplete OBJ vertex")?
                        .parse()
                        .map_err(|_| "Invalid OBJ number")?;
                }
                vertices.push(v)
            }
            Some("f") => {
                let mut face = vec![];
                for index in t.take_while(|s| !s.starts_with('#')) {
                    let n: i64 = index
                        .split('/')
                        .next()
                        .unwrap_or("")
                        .parse()
                        .map_err(|_| "Invalid OBJ face index")?;
                    let i = if n < 0 {
                        vertices.len() as i64 + n
                    } else {
                        n - 1
                    };
                    if i < 0 {
                        return Err("OBJ face index is out of range.".into());
                    }
                    face.push(
                        *vertices
                            .get(i as usize)
                            .ok_or("OBJ face references a missing vertex")?,
                    )
                }
                if face.len() < 3 {
                    return Err("OBJ face needs 3 vertices.".into());
                }
                for i in 1..face.len() - 1 {
                    out.push([face[0], face[i], face[i + 1]])
                }
            }
            _ => {}
        }
    }
    Ok(out)
}
pub(crate) fn normalize(tris: Vec<[[f32; 3]; 3]>) -> Result<Mesh, String> {
    if tris.is_empty() {
        return Err("No mesh triangles were found.".into());
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for p in tris.iter().flatten() {
        for k in 0..3 {
            if !p[k].is_finite() {
                return Err("Mesh coordinates must be finite.".into());
            }
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    let dimensions = std::array::from_fn(|i| max[i] - min[i]);
    let extent = dimensions.iter().copied().fold(0f32, f32::max);
    if extent <= f32::EPSILON || !extent.is_finite() {
        return Err("Mesh has invalid or zero dimensions.".into());
    }
    let scale = 140. / extent;
    let origin = [
        -(min[0] + dimensions[0] / 2.) * scale,
        -(min[1] + dimensions[1] / 2.) * scale,
        -min[2] * scale + 4.,
    ];
    if tris.len() > 200_000 {
        return Err("Preview exceeds 200,000 triangles. Export a coarser mesh.".into());
    }
    // IDs are tied to both coordinates and triangle ordering. A changed CAD
    // tessellation must never silently redirect a saved machining selection.
    let mut fingerprint = Sha256::new();
    fingerprint.update(b"AXIS planar regions v1");
    for value in tris.iter().flatten().flatten() {
        fingerprint.update(value.to_le_bytes());
    }
    let mut faces: Vec<_> = tris
        .iter()
        .map(|t| Face {
            points: t
                .iter()
                .map(|p| {
                    [
                        (p[0] - (min[0] + dimensions[0] / 2.)) * scale,
                        (p[1] - (min[1] + dimensions[1] / 2.)) * scale,
                        (p[2] - min[2]) * scale + 4.,
                    ]
                })
                .collect(),
            feature: None,
            tone: 0.75,
        })
        .collect();
    let surfaces = crate::surface::group(&mut faces);
    Ok(Mesh {
        faces,
        surfaces,
        fingerprint: format!("{:x}", fingerprint.finalize()),
        dimensions,
        triangles: tris.len(),
        sampled: false,
        units: "source units",
        origin,
        rotated_bounds: RefCell::new(None),
    })
}
pub fn sample() -> Mesh {
    let mut faces = vec![];
    box_faces(&mut faces, [-70., -47., 4.], [70., 47., 13.], Some(3), 0.65);
    ring(&mut faces, 53., 32., 45., 24., 13., 32.);
    cylinder(&mut faces, [-20., 0.], 12., 13., 25., Some(4), 0.78);
    disk(&mut faces, [-20., 0., 25.1], 5., None, 0.23);
    for (i, (x, y)) in [(-57., -34.), (57., -34.), (57., 34.), (-57., 34.)]
        .iter()
        .enumerate()
    {
        disk(&mut faces, [*x, *y, 13.1], 4., Some(5 + i), 0.25)
    }
    faces.push(Face {
        points: vec![
            [-45., -24., 13.1],
            [45., -24., 13.1],
            [45., 24., 13.1],
            [-45., 24., 13.1],
        ],
        feature: Some(1),
        tone: 0.76,
    });
    Mesh {
        faces,
        surfaces: vec![],
        fingerprint: String::new(),
        dimensions: [140., 94., 28.],
        triangles: 0,
        sampled: false,
        units: "mm",
        origin: [0., 0., 0.],
        rotated_bounds: RefCell::new(None),
    }
}
fn box_faces(out: &mut Vec<Face>, a: [f32; 3], b: [f32; 3], feature: Option<usize>, tone: f32) {
    let [x, y, z] = a;
    let [xx, yy, zz] = b;
    let vertices = [
        [x, y, z],
        [xx, y, z],
        [xx, yy, z],
        [x, yy, z],
        [x, y, zz],
        [xx, y, zz],
        [xx, yy, zz],
        [x, yy, zz],
    ];
    for inds in [
        [0, 1, 5, 4],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 0, 4, 7],
        [4, 5, 6, 7],
        [3, 2, 1, 0],
    ] {
        out.push(Face {
            points: inds.map(|i| vertices[i]).to_vec(),
            feature,
            tone,
        });
    }
}
fn rounded(w: f32, h: f32, r: f32) -> Vec<[f32; 2]> {
    let mut p = vec![];
    for (x, y, start) in [
        (w - r, h - r, 0.),
        (-w + r, h - r, 90.),
        (-w + r, -h + r, 180.),
        (w - r, -h + r, 270.),
    ] {
        for j in 0..=8 {
            let a = (start + j as f32 * 90. / 8.).to_radians();
            p.push([x + r * a.cos(), y + r * a.sin()])
        }
    }
    p
}
fn ring(out: &mut Vec<Face>, w: f32, h: f32, iw: f32, ih: f32, z: f32, zz: f32) {
    let outer = rounded(w, h, 9.);
    let inner = rounded(iw, ih, 8.);
    for i in 0..outer.len() {
        let j = (i + 1) % outer.len();
        let p = |v: [f32; 2], z| [v[0], v[1], z];
        out.push(Face {
            points: vec![
                p(outer[i], zz),
                p(outer[j], zz),
                p(inner[j], zz),
                p(inner[i], zz),
            ],
            feature: Some(0),
            tone: 0.85,
        });
        out.push(Face {
            points: vec![
                p(outer[i], z),
                p(outer[j], z),
                p(outer[j], zz),
                p(outer[i], zz),
            ],
            feature: Some(2),
            tone: 0.66,
        });
        out.push(Face {
            points: vec![
                p(inner[j], z),
                p(inner[i], z),
                p(inner[i], zz),
                p(inner[j], zz),
            ],
            feature: Some(2),
            tone: 0.57,
        });
    }
}
fn cylinder(
    out: &mut Vec<Face>,
    c: [f32; 2],
    r: f32,
    z: f32,
    zz: f32,
    f: Option<usize>,
    tone: f32,
) {
    for i in 0..32 {
        let a = i as f32 * std::f32::consts::TAU / 32.;
        let b = (i + 1) as f32 * std::f32::consts::TAU / 32.;
        out.push(Face {
            points: vec![
                [c[0] + r * a.cos(), c[1] + r * a.sin(), z],
                [c[0] + r * b.cos(), c[1] + r * b.sin(), z],
                [c[0] + r * b.cos(), c[1] + r * b.sin(), zz],
                [c[0] + r * a.cos(), c[1] + r * a.sin(), zz],
            ],
            feature: f,
            tone,
        });
    }
    disk(out, [c[0], c[1], zz], r, f, tone + 0.08)
}
fn disk(out: &mut Vec<Face>, c: [f32; 3], r: f32, f: Option<usize>, tone: f32) {
    let p = (0..32)
        .map(|i| {
            let a = i as f32 * std::f32::consts::TAU / 32.;
            [c[0] + r * a.cos(), c[1] + r * a.sin(), c[2]]
        })
        .collect();
    out.push(Face {
        points: p,
        feature: f,
        tone,
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn obj_negative_indices() {
        let t = obj("v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf -4 -3 -2 -1").unwrap();
        assert_eq!(t.len(), 2);
        assert!(normalize(t).is_ok())
    }
    #[test]
    fn bad_obj_rejected() {
        assert!(obj("v 0 0 0\nf 1 2 3").is_err())
    }
    #[test]
    fn ascii_stl() {
        let t = stl(b"solid demo\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendsolid").unwrap();
        assert_eq!(t.len(), 1)
    }
    #[test]
    fn binary_stl() {
        let mut b = vec![0u8; 134];
        b[80..84].copy_from_slice(&1u32.to_le_bytes());
        b[108..112].copy_from_slice(&1f32.to_le_bytes());
        b[124..128].copy_from_slice(&1f32.to_le_bytes());
        assert_eq!(stl(&b).unwrap().len(), 1)
    }
    #[test]
    fn nonfinite_rejected() {
        assert!(normalize(vec![[[f32::NAN, 0., 0.]; 3]]).is_err())
    }
    #[test]
    fn full_resolution_preview_preserves_faces() {
        let t = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        let m = normalize(vec![t; 12_001]).unwrap();
        assert_eq!(m.faces.len(), 12_001);
        assert!(!m.sampled);
    }
    #[test]
    fn step_and_3mf_test_block_have_same_mm_bounds() {
        for format in ["step", "3mf"] {
            if format == "step" && option_env!("AXIS_BUILT_STEP_HELPER").is_none() {
                continue;
            }
            let p = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("examples/four-setup-block.{format}"));
            let m = load(&p).unwrap();
            assert_eq!(m.units, "mm");
            for (actual, expected) in m.dimensions.iter().zip([120., 80., 40.]) {
                assert!(
                    (actual - expected).abs() < 0.02,
                    "{format}: {actual} vs {expected}"
                );
            }
            assert!(m.triangles > 1000);
            assert_eq!(m.faces.len(), m.triangles);
        }
    }
    #[test]
    fn step_converts_inches_and_places_assembly_components() {
        if option_env!("AXIS_BUILT_STEP_HELPER").is_none() {
            return;
        }
        for (file, expected) in [
            ("inch-block.step", [25.4, 50.8, 12.7]),
            ("placed-assembly.step", [47.5, 10., 3.]),
        ] {
            let p = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("examples")
                .join(file);
            let m = load(&p).unwrap();
            for (a, b) in m.dimensions.iter().zip(expected) {
                assert!((a - b).abs() < 0.01, "{file}: {a} vs {b}");
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.step");
        std::fs::write(&bad, "this is not STEP").unwrap();
        assert!(load(&bad).is_err());
    }
    #[test]
    fn normalize_remembers_the_source_origin() {
        let mesh = normalize(vec![[[10., 20., 30.], [20., 20., 30.], [10., 40., 35.]]]).unwrap();
        let s = mesh.display_scale();
        let expected = [-15. * s, -30. * s, -30. * s + 4.];
        for k in 0..3 {
            assert!(
                (mesh.origin[k] - expected[k]).abs() < 1e-3,
                "{:?}",
                mesh.origin
            );
        }
        let sources = [[10., 20., 30.], [20., 20., 30.], [10., 40., 35.]];
        for p in &mesh.faces[0].points {
            assert!(
                sources
                    .iter()
                    .any(|q| (0..3).all(|k| (p[k] - (mesh.origin[k] + q[k] * s)).abs() < 1e-3))
            );
        }
    }
}
