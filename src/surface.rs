//! Connected planar regions of an imported tessellation, not exact CAD faces.
use crate::{
    mesh::{Face, Mesh, rotation},
    model::{Operation, Setup, num},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Surface {
    pub normal: [f32; 3],
    pub center: [f32; 3],
    /// Area in normalized display units squared.
    pub area: f32,
    pub triangles: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Selection {
    pub mesh_fingerprint: String,
    pub ids: Vec<usize>,
}
impl Selection {
    pub fn matches(&self, mesh: &Mesh) -> bool {
        !mesh.fingerprint.is_empty() && self.mesh_fingerprint == mesh.fingerprint
    }
    pub fn valid_structure(&self) -> bool {
        self.mesh_fingerprint.len() == 64
            && self.mesh_fingerprint.bytes().all(|c| c.is_ascii_hexdigit())
            && self.ids.len() <= 200_000
            && self.ids.iter().all(|id| *id < 200_000)
            && self
                .ids
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                == self.ids.len()
    }
}

type Vertex = [i64; 3];
type Edge = [Vertex; 2];
fn edge(a: [f32; 3], b: [f32; 3]) -> Edge {
    let mut points = [a, b].map(|p| p.map(|v| (v as f64 * 100_000.).round() as i64));
    points.sort_unstable();
    points
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn triangle(face: &Face) -> Option<Surface> {
    if face.points.len() != 3 {
        return None;
    }
    let a: [f32; 3] = std::array::from_fn(|i| face.points[1][i] - face.points[0][i]);
    let b: [f32; 3] = std::array::from_fn(|i| face.points[2][i] - face.points[0][i]);
    let cross = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    let length = dot(cross, cross).sqrt();
    if length < 1e-8 || !length.is_finite() {
        return None;
    }
    Some(Surface {
        normal: cross.map(|v| v / length),
        center: std::array::from_fn(|i| face.points.iter().map(|p| p[i]).sum::<f32>() / 3.),
        area: length / 2.,
        triangles: 1,
    })
}

pub fn group(faces: &mut [Face]) -> Vec<Surface> {
    let triangles: Vec<_> = faces.iter().map(triangle).collect();
    let mut edges: HashMap<Edge, Vec<usize>> = HashMap::new();
    for (i, face) in faces.iter_mut().enumerate() {
        face.feature = None;
        if triangles[i].is_none() {
            continue;
        }
        for k in 0..3 {
            let adjacent = edges
                .entry(edge(face.points[k], face.points[(k + 1) % 3]))
                .or_default();
            // Three entries mark a non-manifold edge. Do not cross it, and keep
            // malformed meshes from growing an unbounded adjacency list.
            if adjacent.len() < 3 {
                adjacent.push(i);
            }
        }
    }
    let mut neighbors = vec![Vec::new(); faces.len()];
    for adjacent in edges.values().filter(|a| a.len() == 2) {
        neighbors[adjacent[0]].push(adjacent[1]);
        neighbors[adjacent[1]].push(adjacent[0]);
    }
    let mut surfaces = Vec::new();
    for seed in 0..faces.len() {
        let Some(seed_plane) = triangles[seed].as_ref() else {
            continue;
        };
        if faces[seed].feature.is_some() {
            continue;
        }
        let id = surfaces.len();
        let mut surface = seed_plane.clone();
        let mut weighted_center = seed_plane.center.map(|v| v * seed_plane.area);
        let distance = dot(seed_plane.normal, faces[seed].points[0]);
        let mut stack = vec![seed];
        faces[seed].feature = Some(id);
        while let Some(i) = stack.pop() {
            for &next in &neighbors[i] {
                if faces[next].feature.is_some() {
                    continue;
                }
                let Some(t) = triangles[next].as_ref() else {
                    continue;
                };
                // Compare every candidate to the original plane. Comparing
                // only neighbors would slowly merge curved tessellations.
                if dot(t.normal, seed_plane.normal).abs() < 0.999999
                    || faces[next]
                        .points
                        .iter()
                        .any(|p| (dot(seed_plane.normal, *p) - distance).abs() > 0.0002)
                {
                    continue;
                }
                faces[next].feature = Some(id);
                surface.area += t.area;
                surface.triangles += 1;
                for (k, sum) in weighted_center.iter_mut().enumerate() {
                    *sum += t.center[k] * t.area;
                }
                stack.push(next);
            }
        }
        surface.center = weighted_center.map(|v| v / surface.area);
        surfaces.push(surface);
    }
    surfaces
}

pub fn issues(operation: &Operation, setup: &Setup, mesh: Option<&Mesh>) -> Vec<String> {
    if !operation.strategy.imported_preview() {
        return vec![operation.strategy.availability().into()];
    }
    // A Box is a block of numbers: it selects no surfaces.
    if operation.strategy == crate::model::Strategy::Box {
        return vec![];
    }
    let Some(mesh) = mesh else {
        return vec!["Load the imported model before selecting its surfaces.".into()];
    };
    if !operation.needs_surfaces() {
        return vec![];
    }
    let Some(selection) = &operation.surfaces else {
        return vec![];
    };
    if !selection.matches(mesh) {
        return vec!["The model geometry changed. Clear the saved surface selection and select surfaces on the current part.".into()];
    }
    let angles = ["rx", "ry", "rz"].map(|k| num(&setup.p, k).unwrap_or(0.));
    let mut result = Vec::new();
    let mut heights = Vec::new();
    for &id in &selection.ids {
        let Some(surface) = mesh.surfaces.get(id) else {
            result.push(
                "A selected surface is missing. Clear the selection and select again.".into(),
            );
            break;
        };
        if rotation(surface.normal, angles)[2].abs() < 0.5f32.to_radians().cos() {
            result.push(format!(
                "Surface {} is not perpendicular to setup Z. Reposition the part in another setup.",
                id + 1
            ));
        }
        heights.push(rotation(surface.center, angles)[2]);
    }
    // A Pocket or Adaptive clearing clears each selected floor to its own
    // height.
    if !matches!(
        operation.strategy,
        crate::model::Strategy::Pocket | crate::model::Strategy::Adaptive
    ) && heights.iter().copied().fold(f32::NEG_INFINITY, f32::max)
        - heights.iter().copied().fold(f32::INFINITY, f32::min)
        > 0.001 * mesh.display_scale()
    {
        result.push(
            "Selected surfaces are at different heights. Use a separate operation for each plane."
                .into(),
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn face(p: [[f32; 3]; 3]) -> Face {
        Face {
            points: p.to_vec(),
            feature: None,
            tone: 1.,
        }
    }
    #[test]
    fn connected_coplanar_triangles_merge_but_sharp_and_disconnected_faces_do_not() {
        let mut faces = vec![
            face([[0., 0., 0.], [2., 0., 0.], [0., 2., 0.]]),
            face([[2., 0., 0.], [2., 2., 0.], [0., 2., 0.]]),
            face([[0., 0., 0.], [2., 0., 0.], [0., 0., 2.]]),
            face([[5., 0., 0.], [7., 0., 0.], [5., 2., 0.]]),
            face([[0.; 3]; 3]),
        ];
        let surfaces = group(&mut faces);
        assert_eq!(surfaces.len(), 3);
        assert_eq!(surfaces[0].triangles, 2);
        assert_eq!(surfaces[0].area, 4.);
        assert_eq!(surfaces[0].center, [1., 1., 0.]);
        assert_eq!(faces[0].feature, faces[1].feature);
        assert_ne!(faces[0].feature, faces[2].feature);
        assert_ne!(faces[0].feature, faces[3].feature);
        assert_eq!(faces[4].feature, None);
    }
    #[test]
    fn slight_curvature_does_not_merge_into_a_plane() {
        let mut faces = vec![
            face([[0., 0., 0.], [10., 0., 0.], [0., 10., 0.]]),
            face([[10., 0., 0.], [10., 10., 0.01], [0., 10., 0.]]),
        ];
        assert_eq!(group(&mut faces).len(), 2);
    }
    #[test]
    fn tessellation_fingerprint_is_stable_and_order_sensitive() {
        let triangles = vec![
            [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]],
            [[1., 0., 0.], [1., 1., 0.], [0., 1., 0.]],
        ];
        let first = crate::mesh::normalize(triangles.clone()).unwrap();
        let second = crate::mesh::normalize(triangles.clone()).unwrap();
        assert_eq!(first.fingerprint, second.fingerprint);
        assert_eq!(
            first.faces.iter().map(|f| f.feature).collect::<Vec<_>>(),
            second.faces.iter().map(|f| f.feature).collect::<Vec<_>>()
        );
        let mut reordered = triangles.clone();
        reordered.reverse();
        assert_ne!(
            first.fingerprint,
            crate::mesh::normalize(reordered).unwrap().fingerprint
        );
        let mut modified = triangles;
        modified[0][0][2] = 0.1;
        assert_ne!(
            first.fingerprint,
            crate::mesh::normalize(modified).unwrap().fingerprint
        );
    }
    #[test]
    fn a_box_selects_no_surfaces_and_needs_no_model_for_them() {
        let setup = Setup::new(crate::manual::defaults(), "G54");
        let op = Operation::new(crate::model::Strategy::Box, "t".into());
        assert!(issues(&op, &setup, None).is_empty());
        let drill = Operation::new(crate::model::Strategy::Drill, "t".into());
        assert!(!issues(&drill, &setup, None).is_empty());
    }
    #[test]
    fn facing_rejects_different_heights_and_missing_surfaces() {
        let mesh = crate::mesh::normalize(vec![
            [[0., 0., 0.], [10., 0., 0.], [0., 10., 0.]],
            [[0., 0., 2.], [10., 0., 2.], [0., 10., 2.]],
        ])
        .unwrap();
        let mut document = crate::model::Document::sample();
        document.setup_mut().operations[0].geometry.clear();
        document.setup_mut().operations[0].surfaces = Some(Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: vec![0, 1],
        });
        let op = &document.setup().operations[0];
        assert!(
            issues(op, document.setup(), Some(&mesh))
                .iter()
                .any(|e| e.contains("different heights"))
        );
        document.setup_mut().operations[0]
            .surfaces
            .as_mut()
            .unwrap()
            .ids = vec![999];
        assert!(
            issues(
                &document.setup().operations[0],
                document.setup(),
                Some(&mesh)
            )
            .iter()
            .any(|e| e.contains("missing"))
        );
    }
}
