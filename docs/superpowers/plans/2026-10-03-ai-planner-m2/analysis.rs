//! Deterministic summary of the imported part for the AI planner: what can be
//! reached from each of the six axis-aligned orientations. Lengths are mm.
use crate::{
    cam::{self, Geometry, Polygon},
    mesh::{Bounds, Mesh, rotation},
    model::{Document, Operation, Setup, Strategy, get, num},
    surface::Selection,
    volume::{Frame, Volume},
};
use clipper2::{EndType, JoinType};
use serde::Serialize;

/// Orientation name (the part axis that points up) and its setup rotation.
pub const ORIENTATIONS: [(&str, [f32; 3]); 6] = [
    ("+Z", [0., 0., 0.]),
    ("-Z", [180., 0., 0.]),
    ("+X", [0., -90., 0.]),
    ("-X", [0., 90., 0.]),
    ("+Y", [90., 0., 0.]),
    ("-Y", [-90., 0., 0.]),
];
const MAX_PLANES: usize = 40;
const MAX_HOLES: usize = 100;
/// cos 0.5°: a face this close to horizontal counts as planar-horizontal.
const FLAT: f32 = 0.99996;
/// cos 89.5°: steeper faces count as vertical walls.
const WALL: f32 = 0.0087;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PartReport {
    pub units: &'static str,
    pub part_size: [f32; 3],
    pub material: String,
    pub machine: MachineReport,
    pub tools: Vec<ToolReport>,
    /// The first entry is setup 1's orientation.
    pub orientations: Vec<OrientationReport>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MachineReport {
    pub max_rpm: f32,
    pub max_feed: f32,
    pub travel: [f32; 3],
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolReport {
    pub number: u32,
    pub name: String,
    pub kind: String,
    pub diameter: f32,
    pub flute_length: f32,
    pub stickout: f32,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OrientationReport {
    pub up: &'static str,
    pub is_setup1: bool,
    /// Model extent in this orientation: X, Y and height.
    pub model_size: [f32; 3],
    /// Stock beyond the model before any machining: top, bottom, -X, +X, -Y, +Y.
    pub stock_margin: [f32; 6],
    /// Visible horizontal faces, largest first. Positions are measured from
    /// the model's -X/-Y corner in this orientation.
    pub planes: Vec<PlaneReport>,
    /// Visible area that is neither horizontal nor vertical (mm²).
    pub freeform_area: f32,
    pub truncated: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlaneReport {
    pub surfaces: Vec<usize>,
    /// "top" for the highest plane, otherwise "floor".
    pub kind: &'static str,
    pub depth_below_top: f32,
    pub area: f32,
    /// x0, y0, x1, y1
    pub bbox: [f32; 4],
    /// Largest flat cutter that fits inside the floor outline (floors only).
    pub max_cutter_diameter: Option<f32>,
    pub holes: Vec<HoleReport>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HoleReport {
    pub center: [f32; 2],
    /// Narrowest diameter down to the bottom: the largest drill that fits.
    pub diameter: f32,
    pub opening_diameter: f32,
    pub depth: f32,
    pub through: bool,
}
fn r2(v: f32) -> f32 {
    (v * 100.).round() / 100.
}
/// The part axis that points up in this setup, when the setup is square to
/// the part (every rotation a multiple of 90°).
pub fn up_axis(setup: &Setup) -> Option<&'static str> {
    let angles = ["rx", "ry", "rz"].map(|k| num(&setup.p, k).unwrap_or(f32::NAN));
    let axes = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]].map(|a| rotation(a, angles));
    if axes
        .iter()
        .flatten()
        .any(|v| !v.is_finite() || (v.abs() > 0.001 && v.abs() < 0.999))
    {
        return None;
    }
    let (axis, z) = (0..3)
        .map(|k| (k, axes[k][2]))
        .find(|(_, z)| z.abs() > 0.5)?;
    Some(match (axis, z > 0.) {
        (0, true) => "+X",
        (0, false) => "-X",
        (1, true) => "+Y",
        (1, false) => "-Y",
        (_, true) => "+Z",
        (_, false) => "-Z",
    })
}
/// The setup rotation for an orientation: setup 1's own rotation for its up
/// axis, the canonical rotation otherwise.
pub fn rotation_for(setup1: &Setup, up: &str) -> Option<[f32; 3]> {
    if up_axis(setup1) == Some(up) {
        return Some(["rx", "ry", "rz"].map(|k| num(&setup1.p, k).unwrap_or(0.)));
    }
    ORIENTATIONS
        .iter()
        .find(|(name, _)| *name == up)
        .map(|(_, r)| *r)
}
/// A copy of setup 1 turned to `up`. Later setups carry no translation.
pub fn oriented(setup1: &Setup, up: &str) -> Option<Setup> {
    let rotation = rotation_for(setup1, up)?;
    let mut setup = setup1.clone();
    let first = up_axis(setup1) == Some(up);
    for (key, value) in ["rx", "ry", "rz"].iter().zip(rotation) {
        setup.p.insert((*key).into(), value.to_string());
    }
    if !first {
        for key in ["tx", "ty", "tz"] {
            setup.p.insert(key.into(), "0".into());
        }
    }
    Some(setup)
}
/// XY bucket index over the placed triangles, for height queries.
pub(crate) struct Index<'a> {
    geometry: &'a Geometry,
    cell: f32,
    n: [usize; 2],
    cells: Vec<Vec<u32>>,
}
impl<'a> Index<'a> {
    pub(crate) fn new(geometry: &'a Geometry) -> Self {
        let d = geometry.bounds.dimensions();
        let cell = (d[0].max(d[1]) / 128.).max(0.01);
        let n = [0, 1].map(|k| (d[k] / cell).ceil() as usize + 1);
        let mut cells = vec![vec![]; n[0] * n[1]];
        let mut index = Self {
            geometry,
            cell,
            n,
            cells: vec![],
        };
        for (i, t) in geometry.triangles.iter().enumerate() {
            let lo = [0, 1].map(|k| t.p.iter().map(|p| p[k]).fold(f32::INFINITY, f32::min));
            let hi = [0, 1].map(|k| t.p.iter().map(|p| p[k]).fold(f32::NEG_INFINITY, f32::max));
            let (a, b) = (index.cell_of(lo), index.cell_of(hi));
            for y in a[1]..=b[1] {
                for x in a[0]..=b[0] {
                    cells[y * n[0] + x].push(i as u32);
                }
            }
        }
        index.cells = cells;
        index
    }
    fn cell_of(&self, xy: [f32; 2]) -> [usize; 2] {
        [0, 1].map(|k| {
            (((xy[k] - self.geometry.bounds.min[k]) / self.cell)
                .floor()
                .max(0.) as usize)
                .min(self.n[k] - 1)
        })
    }
    /// Triangles whose XY bucket contains `xy`.
    pub(crate) fn near(&self, xy: [f32; 2]) -> impl Iterator<Item = &'a cam::Triangle> + '_ {
        let c = self.cell_of(xy);
        self.cells[c[1] * self.n[0] + c[0]]
            .iter()
            .map(|i| &self.geometry.triangles[*i as usize])
    }
    /// Highest mesh point above `xy`.
    fn height(&self, xy: [f32; 2]) -> Option<f32> {
        self.near(xy)
            .filter_map(|t| cam::plane_height(&t.p, xy))
            .max_by(f32::total_cmp)
    }
}
/// Unnormalized normal; its length is twice the triangle's area.
pub(crate) fn normal(p: &[[f32; 3]; 3]) -> [f32; 3] {
    let a: [f32; 3] = std::array::from_fn(|i| p[1][i] - p[0][i]);
    let b: [f32; 3] = std::array::from_fn(|i| p[2][i] - p[0][i]);
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(crate) fn centroid(p: &[[f32; 3]; 3]) -> [f32; 3] {
    std::array::from_fn(|i| (p[0][i] + p[1][i] + p[2][i]) / 3.)
}
#[derive(Default)]
struct Face {
    z: f32,
    area: f32,
    visible: f32,
    flat: bool,
    bbox: [f32; 4],
}
/// The largest cutter diameter that still fits inside the plane's outline.
fn fitting_diameter(geometry: &Geometry, surfaces: &[usize], bbox: [f32; 4]) -> Option<f32> {
    let outline: Vec<Polygon> = geometry
        .triangles
        .iter()
        .filter(|t| t.feature.is_some_and(|id| surfaces.contains(&id)))
        .map(|t| t.p.iter().map(|v| (v[0] as f64, v[1] as f64)).collect())
        .collect();
    let region = cam::region(outline).ok()?;
    let (mut lo, mut hi) = (
        0f32,
        ((bbox[2] - bbox[0]).min(bbox[3] - bbox[1]) / 2.).max(0.),
    );
    for _ in 0..20 {
        let mid = (lo + hi) / 2.;
        let shrunk: Vec<Polygon> = region
            .inflate(-(mid as f64), JoinType::Round, EndType::Polygon, 2.)
            .into();
        if shrunk.iter().any(|p| cam::area(p).abs() > 1e-6) {
            lo = mid
        } else {
            hi = mid
        }
    }
    Some(lo * 2.)
}
fn orientation(
    setup1: &Setup,
    stock: &Volume,
    mesh: &Mesh,
    up: &'static str,
) -> Option<OrientationReport> {
    let setup = oriented(setup1, up)?;
    let geometry = Geometry::new(&setup, mesh);
    let index = Index::new(&geometry);
    let model = geometry.bounds;
    let mut faces: std::collections::BTreeMap<usize, Face> = Default::default();
    let mut freeform = 0.;
    for t in &geometry.triangles {
        let n = normal(&t.p);
        let twice_area = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if twice_area < 1e-9 {
            continue;
        }
        let (c, slope) = (centroid(&t.p), n[2] / twice_area);
        let visible = index.height([c[0], c[1]]).is_none_or(|h| h <= c[2] + 0.01);
        if slope.abs() < FLAT {
            if slope > WALL && visible {
                freeform += twice_area / 2.;
            }
            continue;
        }
        let Some(id) = t.feature else { continue };
        let face = faces.entry(id).or_insert(Face {
            z: c[2],
            flat: true,
            bbox: [
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ],
            ..Default::default()
        });
        face.area += twice_area / 2.;
        if visible {
            face.visible += twice_area / 2.;
        }
        for p in &t.p {
            face.flat &= (p[2] - face.z).abs() <= 0.002;
            face.bbox = [
                face.bbox[0].min(p[0]),
                face.bbox[1].min(p[1]),
                face.bbox[2].max(p[0]),
                face.bbox[3].max(p[1]),
            ];
        }
    }
    // Coplanar visible faces at one height form one plane.
    let mut planes: std::collections::BTreeMap<i64, PlaneReport> = Default::default();
    for (id, face) in &faces {
        if !face.flat || face.visible < face.area * 0.5 {
            continue;
        }
        let plane = planes
            .entry((face.z * 500.).round() as i64)
            .or_insert(PlaneReport {
                surfaces: vec![],
                kind: "floor",
                depth_below_top: model.max[2] - face.z,
                area: 0.,
                bbox: face.bbox,
                max_cutter_diameter: None,
                holes: vec![],
            });
        plane.surfaces.push(*id);
        plane.area += face.area;
        plane.bbox = [
            plane.bbox[0].min(face.bbox[0]),
            plane.bbox[1].min(face.bbox[1]),
            plane.bbox[2].max(face.bbox[2]),
            plane.bbox[3].max(face.bbox[3]),
        ];
    }
    let mut planes: Vec<PlaneReport> = planes.into_values().filter(|p| p.area >= 1.).collect();
    planes.sort_by(|a, b| b.area.total_cmp(&a.area));
    let mut truncated = planes.len() > MAX_PLANES;
    planes.truncate(MAX_PLANES);
    let mut holes_left = MAX_HOLES;
    for plane in &mut planes {
        if plane.depth_below_top < 0.01 {
            plane.kind = "top";
        } else {
            plane.max_cutter_diameter =
                fitting_diameter(&geometry, &plane.surfaces, plane.bbox).map(r2);
        }
        let mut probe = Operation::new(Strategy::Drill, String::new());
        probe.geometry.clear();
        probe.surfaces = Some(Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: plane.surfaces.clone(),
        });
        let holes = cam::holes(&setup, &probe, mesh).unwrap_or_default();
        truncated |= holes.len() > holes_left;
        plane.holes = holes
            .iter()
            .take(holes_left)
            .map(|h| HoleReport {
                center: [
                    r2(h.center[0] - model.min[0]),
                    r2(h.center[1] - model.min[1]),
                ],
                diameter: r2(h.diameter),
                opening_diameter: r2(h.opening),
                depth: r2(h.top - h.bottom),
                through: h.bottom <= model.min[2] + 0.01,
            })
            .collect();
        holes_left -= plane.holes.len();
        plane.depth_below_top = r2(plane.depth_below_top);
        plane.area = r2(plane.area);
        plane.bbox = [
            r2(plane.bbox[0] - model.min[0]),
            r2(plane.bbox[1] - model.min[1]),
            r2(plane.bbox[2] - model.min[0]),
            r2(plane.bbox[3] - model.min[1]),
        ];
    }
    let stock: Bounds = stock.transformed_bounds(Frame::new(&setup, mesh));
    Some(OrientationReport {
        up,
        is_setup1: up_axis(setup1) == Some(up),
        model_size: model.dimensions().map(r2),
        stock_margin: [
            stock.max[2] - model.max[2],
            model.min[2] - stock.min[2],
            model.min[0] - stock.min[0],
            stock.max[0] - model.max[0],
            model.min[1] - stock.min[1],
            stock.max[1] - model.max[1],
        ]
        .map(r2),
        planes,
        freeform_area: r2(freeform),
        truncated,
    })
}
/// One line of the planner's readiness checklist.
#[derive(Debug, Clone, PartialEq)]
pub struct Precondition {
    pub label: &'static str,
    pub ok: bool,
    /// What to do when it is not met.
    pub hint: String,
}
/// What must be in place before a plan can be requested or checked.
pub fn preconditions(doc: &Document, mesh: Option<&Mesh>) -> Vec<Precondition> {
    let line = |label, problem: Option<String>| Precondition {
        label,
        ok: problem.is_none(),
        hint: problem.unwrap_or_default(),
    };
    let part = (doc.file.is_none() || mesh.is_none()).then(|| {
        "Import a part and wait for it to load. The built-in example cannot be planned.".to_string()
    });
    let setup = match doc.setups.first() {
        None => Some("Create setup 1 with your machine and stock.".to_string()),
        Some(setup1) => crate::post::PostSettings::from_machine(&setup1.machine)
            .err()
            .and_then(|e| e.into_iter().next())
            .or_else(|| crate::stock::resolve(setup1, mesh).err())
            .or_else(|| {
                up_axis(setup1).is_none().then(|| {
                    "Setup 1 must be square to the part: use rotations that are multiples of 90°.".into()
                })
            }),
    };
    let material = doc
        .material
        .trim()
        .is_empty()
        .then(|| "Enter the material.".to_string());
    let tools = if doc.tools.is_empty() {
        Some("Add at least one tool to the library.".to_string())
    } else {
        doc.tools
            .iter()
            .find_map(|t| doc.tool_number_issues(t).into_iter().next())
    };
    vec![
        line("Imported part", part),
        line("Setup 1: 3-axis LinuxCNC machine and stock", setup),
        line("Material", material),
        line("Tool library", tools),
    ]
}
/// The whole report, or the first reason planning cannot start.
pub fn report(doc: &Document, mesh: &Mesh) -> Result<PartReport, String> {
    let setup1 = doc
        .setups
        .first()
        .ok_or("Create a machining setup first.")?;
    let first = up_axis(setup1)
        .ok_or("Setup 1 must be square to the part: use rotations that are multiples of 90°.")?;
    let stock = crate::stock::resolve(setup1, Some(mesh))?.stock;
    let stock = Volume::new(stock, Frame::new(setup1, mesh));
    let mut order: Vec<&'static str> = vec![first];
    order.extend(
        ORIENTATIONS
            .iter()
            .map(|(up, _)| *up)
            .filter(|up| *up != first),
    );
    let machine = &setup1.machine;
    let inch = if get(&machine.p, "units") == "inch" {
        25.4
    } else {
        1.
    };
    Ok(PartReport {
        units: "mm",
        part_size: mesh.dimensions.map(r2),
        material: doc.material.clone(),
        machine: MachineReport {
            max_rpm: num(&machine.p, "rpm").unwrap_or(0.),
            max_feed: r2(num(&machine.p, "feed").unwrap_or(0.) * inch),
            travel: ["x", "y", "z"].map(|k| r2(num(&machine.p, k).unwrap_or(0.) * inch)),
        },
        tools: doc
            .tools
            .iter()
            .filter_map(|t| {
                Some(ToolReport {
                    number: t.number()?,
                    name: t.name().to_string(),
                    kind: get(&t.p, "type").to_string(),
                    diameter: num(&t.p, "diameter")?,
                    flute_length: num(&t.p, "length")?,
                    stickout: num(&t.p, "stickout")?,
                })
            })
            .collect(),
        orientations: order
            .into_iter()
            .filter_map(|up| orientation(setup1, &stock, mesh, up))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::post::fixture;
    fn block() -> (Document, Mesh) {
        fixture::job()
    }
    fn plane<'a>(o: &'a OrientationReport, depth: f32) -> &'a PlaneReport {
        o.planes
            .iter()
            .find(|p| (p.depth_below_top - depth).abs() < 0.01)
            .unwrap_or_else(|| panic!("no plane at depth {depth} in {}", o.up))
    }
    #[test]
    fn the_example_block_reports_its_top_pocket_and_holes() {
        let (doc, mesh) = block();
        let report = report(&doc, &mesh).unwrap();
        assert_eq!(report.part_size, [120., 80., 40.]);
        assert_eq!(report.units, "mm");
        let ups: Vec<_> = report.orientations.iter().map(|o| o.up).collect();
        assert_eq!(ups, vec!["+Z", "-Z", "+X", "-X", "+Y", "-Y"]);
        let top = &report.orientations[0];
        assert!(top.is_setup1 && !report.orientations[1].is_setup1);
        assert_eq!(top.model_size, [120., 80., 40.]);
        // fixture stock: top 4, bottom 3, sides 2
        assert_eq!(top.stock_margin, [4., 3., 2., 2., 2., 2.]);
        let face = plane(top, 0.);
        assert_eq!(face.kind, "top");
        assert_eq!(face.max_cutter_diameter, None);
        assert_eq!(face.holes.len(), 5);
        let through: Vec<_> = face.holes.iter().filter(|h| h.through).collect();
        assert_eq!(through.len(), 4);
        assert!(
            through
                .iter()
                .all(|h| h.diameter == 6.6 && h.opening_diameter == 11. && h.depth == 40.)
        );
        let blind = face.holes.iter().find(|h| !h.through).unwrap();
        assert_eq!((blind.diameter, blind.depth), (6., 8.));
        assert_eq!(blind.center, [42., 40.]);
        let pocket = plane(top, 12.);
        assert_eq!(pocket.kind, "floor");
        assert_eq!(pocket.bbox, [16., 20., 88., 60.]);
        assert!((pocket.max_cutter_diameter.unwrap() - 37.05).abs() < 0.2);
        let counterbores = plane(top, 6.);
        assert_eq!(counterbores.surfaces.len(), 4);
        assert!(counterbores.max_cutter_diameter.unwrap() < 2.3);
        assert!(top.freeform_area > 100.);
        assert!(!top.truncated);
    }
    #[test]
    fn each_orientation_lists_only_what_is_visible_from_above() {
        let (doc, mesh) = block();
        let report = report(&doc, &mesh).unwrap();
        let by_up = |up: &str| report.orientations.iter().find(|o| o.up == up).unwrap();
        let bottom = by_up("-Z");
        assert_eq!(bottom.stock_margin, [3., 4., 2., 2., 2., 2.]);
        assert!(plane(bottom, 6.).max_cutter_diameter.unwrap() > 37.);
        assert_eq!(plane(bottom, 0.).holes.len(), 6);
        // The top pocket floor is not reachable from below or from the sides.
        let top_pocket = plane(by_up("+Z"), 12.).surfaces.clone();
        for up in ["-Z", "+X", "-X", "+Y", "-Y"] {
            assert!(
                by_up(up).planes.iter().all(|p| p.surfaces != top_pocket),
                "{up}"
            );
        }
        assert_eq!(by_up("+X").model_size, [40., 80., 120.]);
        assert_eq!(plane(by_up("+X"), 0.).holes.len(), 2);
        assert_eq!(by_up("-X").planes.len(), 1);
        let side = by_up("-Y");
        assert_eq!(side.model_size, [120., 40., 80.]);
        let bore = plane(side, 0.)
            .holes
            .iter()
            .find(|h| h.opening_diameter == 24.)
            .unwrap();
        assert!((bore.diameter - 16.).abs() < 0.02 && bore.depth == 14. && !bore.through);
        // every surface is listed in at most one orientation
        let mut seen = std::collections::HashSet::new();
        for id in report
            .orientations
            .iter()
            .flat_map(|o| &o.planes)
            .flat_map(|p| &p.surfaces)
        {
            assert!(seen.insert(*id));
        }
    }
    #[test]
    fn setup_one_must_be_square_and_keeps_its_own_rotation() {
        let (mut doc, mesh) = block();
        doc.setups[0].p.insert("rz".into(), "90".into());
        assert_eq!(up_axis(&doc.setups[0]), Some("+Z"));
        assert_eq!(rotation_for(&doc.setups[0], "+Z"), Some([0., 0., 90.]));
        assert_eq!(rotation_for(&doc.setups[0], "-Z"), Some([180., 0., 0.]));
        let turned = report(&doc, &mesh).unwrap();
        assert_eq!(turned.orientations[0].model_size, [80., 120., 40.]);
        doc.setups[0].p.insert("rx".into(), "180".into());
        doc.setups[0].p.insert("rz".into(), "0".into());
        assert_eq!(report(&doc, &mesh).unwrap().orientations[0].up, "-Z");
        doc.setups[0].p.insert("rx".into(), "30".into());
        assert!(report(&doc, &mesh).unwrap_err().contains("multiples of 90"));
        doc.setups.clear();
        assert!(report(&doc, &mesh).is_err());
        for (up, rotation) in ORIENTATIONS {
            let mut setup = Setup::new(crate::model::Machine::new(3), "G54");
            for (k, v) in ["rx", "ry", "rz"].iter().zip(rotation) {
                setup.p.insert((*k).into(), v.to_string());
            }
            assert_eq!(up_axis(&setup), Some(up));
        }
    }
    #[test]
    fn preconditions_name_what_is_missing() {
        let (mut doc, mesh) = block();
        let unmet = |doc: &Document, mesh: Option<&Mesh>| -> Vec<&'static str> {
            preconditions(doc, mesh)
                .into_iter()
                .filter(|p| !p.ok)
                .map(|p| p.label)
                .collect()
        };
        assert_eq!(unmet(&doc, Some(&mesh)), vec!["Material"]);
        doc.material = "Aluminum 6061".into();
        assert!(unmet(&doc, Some(&mesh)).is_empty());
        assert_eq!(
            unmet(&doc, None),
            vec![
                "Imported part",
                "Setup 1: 3-axis LinuxCNC machine and stock"
            ]
        );
        let mut other = doc.clone();
        other.setups[0]
            .machine
            .p
            .insert("controller".into(), "GRBL".into());
        other.tools[1].p.insert("number".into(), "1".into());
        other.file = None;
        let lines = preconditions(&other, Some(&mesh));
        assert_eq!(lines.iter().filter(|p| !p.ok).count(), 3);
        assert!(lines[1].hint.contains("LinuxCNC") && lines[3].hint.contains("also used"));
        other.setups[0].machine = doc.setups[0].machine.clone();
        other.setups[0].p.insert("ry".into(), "45".into());
        assert!(
            preconditions(&other, Some(&mesh))[1]
                .hint
                .contains("multiples of 90")
        );
        other.setups.clear();
        other.tools.clear();
        let lines = preconditions(&other, Some(&mesh));
        assert!(
            lines[1].hint.contains("Create setup 1") && lines[3].hint.contains("at least one tool")
        );
    }
    #[test]
    fn the_report_lists_tools_machine_limits_and_serializes() {
        let (mut doc, mesh) = block();
        doc.material = "Aluminum 6061".into();
        doc.tools[1].p.remove("number");
        let report = report(&doc, &mesh).unwrap();
        assert_eq!(
            report.tools.iter().map(|t| t.number).collect::<Vec<_>>(),
            vec![1, 3]
        );
        assert_eq!(report.tools[0].kind, "Flat end mill");
        assert_eq!(
            (report.machine.max_rpm, report.machine.max_feed),
            (12000., 5000.)
        );
        let json = serde_json::to_string(&report).unwrap();
        assert!(
            json.contains("\"material\":\"Aluminum 6061\"")
                && json.contains("\"max_cutter_diameter\"")
        );
        assert!(json.len() < 20_000, "{}", json.len());
    }
}
