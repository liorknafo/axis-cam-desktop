//! Face picks found again on the part a changed model file gives. A pick is
//! a surface of the mesh it was made on, by number (`surface::Selection`),
//! and a changed file numbers its surfaces anew. So where each picked
//! surface is on the part is kept with its operation, in the model file's
//! own units (`Places`, by `record`), and a reimport, or a project opened
//! after its file changed, picks the surfaces at those places on the new
//! part (`follow`).
use crate::{mesh::Mesh, model::Document, surface::Selection};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// A surface of the part, in the model file's units: its normal, centre,
/// area, box and second moments of area.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    /// The surface's number on the mesh the places were recorded on.
    pub id: usize,
    pub normal: [f32; 3],
    pub center: [f32; 3],
    pub area: f32,
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// Its second moments of area about its centre: xx, yy, zz, xy, xz,
    /// yz. Two surfaces at one place as large but of another shape (a ring
    /// and the boss top inside it) differ in them. All 0 in places kept
    /// before they were: not known, so not compared.
    #[serde(default)]
    pub moments: [f32; 6],
}
/// The places of an operation's picked surfaces, one for each, in the order
/// of its selection, on the mesh `mesh_fingerprint` names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Places {
    pub mesh_fingerprint: String,
    pub faces: Vec<Place>,
    /// The surfaces not picked on the plane of a picked one, where their
    /// boxes meet: never taken for a picked surface that changed shape.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub others: Vec<Place>,
}
impl Places {
    /// Whether these are the places of `selection`'s surfaces.
    fn describe(&self, selection: &Selection) -> bool {
        self.mesh_fingerprint == selection.mesh_fingerprint
            && self.faces.len() == selection.ids.len()
            && (self.faces.iter().zip(&selection.ids)).all(|(p, id)| p.id == *id)
    }
}
/// What `follow` did: how many picks it found again, and a note for each
/// operation it could not carry whole (also kept in `Document::pick_checks`
/// until its picks are checked).
#[derive(Debug, Default)]
pub struct Followed {
    /// The operations whose picks are now on the new part.
    pub operations: usize,
    pub notes: Vec<String>,
}

/// Every surface of `mesh` in the model file's units.
fn places(mesh: &Mesh) -> Vec<Place> {
    let scale = mesh.display_scale();
    let file =
        |p: [f32; 3]| -> [f32; 3] { std::array::from_fn(|i| (p[i] - mesh.origin[i]) / scale) };
    let mut out: Vec<Place> = (mesh.surfaces.iter().enumerate())
        .map(|(id, s)| Place {
            id,
            normal: s.normal,
            center: file(s.center),
            area: s.area / (scale * scale),
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
            moments: [0.; 6],
        })
        .collect();
    const PAIRS: [(usize, usize); 6] = [(0, 0), (1, 1), (2, 2), (0, 1), (0, 2), (1, 2)];
    for face in &mesh.faces {
        let Some(place) = face.feature.and_then(|id| out.get_mut(id)) else {
            continue;
        };
        let points: Vec<[f32; 3]> = face.points.iter().map(|p| file(*p)).collect();
        for p in &points {
            for i in 0..3 {
                place.min[i] = place.min[i].min(p[i]);
                place.max[i] = place.max[i].max(p[i]);
            }
        }
        // Each triangle's second moments about the surface's centre (a face
        // of more points as a fan): its area over 12 times (the sum of v vᵀ
        // plus (Σv)(Σv)ᵀ), v from the centre.
        for k in 2..points.len() {
            let [a, b, c] = [points[0], points[k - 1], points[k]];
            let v = [a, b, c].map(|p| sub(p, place.center));
            let cross = {
                let (e, f) = (sub(b, a), sub(c, a));
                [
                    e[1] * f[2] - e[2] * f[1],
                    e[2] * f[0] - e[0] * f[2],
                    e[0] * f[1] - e[1] * f[0],
                ]
            };
            let area = dot(cross, cross).sqrt() / 2.;
            let sum: [f32; 3] = std::array::from_fn(|i| v[0][i] + v[1][i] + v[2][i]);
            for (m, (i, j)) in place.moments.iter_mut().zip(PAIRS) {
                let vv: f32 = v.iter().map(|p| p[i] * p[j]).sum();
                *m += area / 12. * (vv + sum[i] * sum[j]);
            }
        }
    }
    out
}

/// Keeps the places of every operation's picks made on `mesh` with it, for
/// a later `follow`; those already kept are left.
pub fn record(doc: &mut Document, mesh: &Mesh) {
    if mesh.fingerprint.is_empty() {
        return;
    }
    let Document {
        setups,
        places: kept,
        pick_checks,
        ..
    } = doc;
    // Those of operations deleted since go.
    let ids: HashSet<&str> = (setups.iter().flat_map(|s| &s.operations))
        .map(|o| o.id.as_str())
        .collect();
    kept.retain(|id, _| ids.contains(id.as_str()));
    let mut all = None;
    for o in setups.iter().flat_map(|s| &s.operations) {
        let Some(selection) = o.surfaces.as_ref().filter(|s| s.matches(mesh)) else {
            continue;
        };
        if kept.get(&o.id).is_some_and(|p| p.describe(selection)) {
            continue;
        }
        let all = all.get_or_insert_with(|| places(mesh));
        let faces: Option<Vec<Place>> = (selection.ids.iter())
            .map(|&id| all.get(id).cloned())
            .collect();
        match faces {
            Some(faces) => {
                let tol = tolerance(mesh);
                let others = (all.iter())
                    .filter(|p| {
                        !selection.ids.contains(&p.id)
                            && faces.iter().any(|f| same_plane(f, p, tol))
                    })
                    .cloned()
                    .collect();
                kept.insert(
                    o.id.clone(),
                    Places {
                        mesh_fingerprint: selection.mesh_fingerprint.clone(),
                        faces,
                        others,
                    },
                )
            }
            None => kept.remove(&o.id),
        };
    }
    pick_checks.retain(|id, _| ids.contains(id.as_str()));
}
/// How near two places are the same, in the model file's units: a ten
/// thousandth of the part's largest dimension.
fn tolerance(mesh: &Mesh) -> f32 {
    let extent = mesh.dimensions.into_iter().fold(0f32, f32::max);
    (extent * 1e-4).max(f32::EPSILON)
}

const SAME_NORMAL: f32 = 1. - 1e-6;
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
/// The same surface: facing the same way, at the same place, as large, in
/// the same box and of the same shape (its second moments).
fn same(a: &Place, b: &Place, tol: f32) -> bool {
    let d = sub(a.center, b.center);
    let area = a.area.max(b.area);
    // A moment is an area times a length squared: its scale here.
    let reach = (0..3)
        .map(|i| (a.max[i] - a.min[i]).max(b.max[i] - b.min[i]))
        .fold(0f32, f32::max);
    let moment = area * reach * reach;
    dot(a.normal, b.normal) >= SAME_NORMAL
        && dot(d, d).sqrt() <= tol
        && (a.area - b.area).abs() <= 1e-3 * area + tol * tol
        && (0..3).all(|i| (a.min[i] - b.min[i]).abs() <= tol && (a.max[i] - b.max[i]).abs() <= tol)
        && (a.moments == [0.; 6]
            || b.moments == [0.; 6]
            || (a.moments.iter().zip(&b.moments))
                .all(|(x, y)| (x - y).abs() <= 1e-3 * moment + tol * tol * area))
}
/// On the same plane, facing the same way, with boxes that meet.
fn same_plane(a: &Place, b: &Place, tol: f32) -> bool {
    dot(a.normal, b.normal) >= SAME_NORMAL
        && dot(a.normal, sub(b.center, a.center)).abs() <= tol
        && (0..3).all(|i| a.min[i] <= b.max[i] + tol && b.min[i] <= a.max[i] + tol)
}
/// How much of the boxes `a` and `b` of surfaces on a plane facing `normal`
/// is common to both, from 0 to 1: their overlap over their union, seen
/// along the axis nearest the normal.
fn overlap(normal: [f32; 3], a: ([f32; 3], [f32; 3]), b: ([f32; 3], [f32; 3])) -> f32 {
    let along = (0..3)
        .max_by(|&i, &j| normal[i].abs().total_cmp(&normal[j].abs()))
        .unwrap_or(2);
    let (mut both, mut area_a, mut area_b) = (1f32, 1f32, 1f32);
    for i in (0..3).filter(|&i| i != along) {
        both *= (a.1[i].min(b.1[i]) - a.0[i].max(b.0[i])).max(0.);
        area_a *= (a.1[i] - a.0[i]).max(0.);
        area_b *= (b.1[i] - b.0[i]).max(0.);
    }
    let union = area_a + area_b - both;
    if union > 0. { both / union } else { 0. }
}
/// The surfaces of a part by where their centres are, in cells `tol` wide,
/// to find the one `same` as a place without trying them all.
struct Grid<'a> {
    places: &'a [Place],
    cells: HashMap<[i64; 3], Vec<usize>>,
    tol: f32,
}
impl<'a> Grid<'a> {
    fn new(places: &'a [Place], tol: f32) -> Self {
        let mut cells: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
        for (i, p) in places.iter().enumerate() {
            cells.entry(Self::cell(p.center, tol)).or_default().push(i);
        }
        Self { places, cells, tol }
    }
    fn cell(p: [f32; 3], tol: f32) -> [i64; 3] {
        p.map(|v| (v / tol).floor() as i64)
    }
    /// The surface `same` as `place`, the nearest when several are.
    fn same(&self, place: &Place) -> Option<usize> {
        let [x, y, z] = Self::cell(place.center, self.tol);
        let mut best: Option<(f32, usize)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &i in self
                        .cells
                        .get(&[x + dx, y + dy, z + dz])
                        .into_iter()
                        .flatten()
                    {
                        let p = &self.places[i];
                        if !same(place, p, self.tol) {
                            continue;
                        }
                        let d = sub(place.center, p.center);
                        let d = dot(d, d);
                        if best.is_none_or(|(b, _)| d < b) {
                            best = Some((d, i));
                        }
                    }
                }
            }
        }
        best.map(|(_, i)| i)
    }
}

/// Picks each operation's surfaces made on another mesh again on `mesh`,
/// from their places (`record`). A surface still where it was, as large,
/// is picked as it was. One whose shape changed on its plane is picked as
/// the surfaces of `mesh` there, other than those `old` (the part as it was,
/// when it is still loaded) already had unchanged, which were not it. One
/// found nowhere is dropped. An operation none of whose surfaces is found
/// keeps its selection as it was, refused as made on another part, so that
/// a later version of the file may find them.
pub fn follow(doc: &mut Document, mesh: &Mesh, old: Option<&Mesh>) -> Followed {
    let mut followed = Followed::default();
    if mesh.fingerprint.is_empty() {
        return followed;
    }
    let tol = tolerance(mesh);
    let new = places(mesh);
    let grid = Grid::new(&new, tol);
    // The surfaces of `mesh` that `old` had too, unchanged: worked out only
    // when a surface changed shape.
    let mut kept: Option<HashSet<usize>> = None;
    let several = doc.setups.len() > 1;
    let Document {
        setups,
        places: kept_places_of,
        pick_checks,
        ..
    } = doc;
    for (si, setup) in setups.iter_mut().enumerate() {
        for o in &mut setup.operations {
            let Some(selection) = o.surfaces.as_ref() else {
                continue;
            };
            if selection.matches(mesh) || selection.ids.is_empty() {
                continue;
            }
            let name = if several {
                format!("Setup {} · {}", si + 1, o.name())
            } else {
                o.name().to_string()
            };
            let Some(kept_places) = kept_places_of.get(&o.id).filter(|p| p.describe(selection))
            else {
                let note = "Its surfaces were picked before picks were kept with their places, so they are not found on the new part. Pick them again in Geometry.";
                followed.notes.push(format!("{name}: {note}"));
                pick_checks.insert(o.id.clone(), note.into());
                continue;
            };
            // The surfaces not picked beside the picks before: never taken.
            let others: HashSet<usize> = (kept_places.others.iter())
                .filter_map(|p| grid.same(p))
                .collect();
            let mut ids: Vec<usize> = vec![];
            let (mut reshaped, mut lost) = (0, 0);
            for place in &kept_places.faces {
                if let Some(id) = grid.same(place) {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                    continue;
                }
                let kept = kept.get_or_insert_with(|| {
                    old.filter(|o| !o.fingerprint.is_empty())
                        .map(|old| places(old).iter().filter_map(|p| grid.same(p)).collect())
                        .unwrap_or_default()
                });
                let there: Vec<&Place> = (new.iter())
                    .filter(|n| {
                        !kept.contains(&n.id)
                            && !others.contains(&n.id)
                            && same_plane(place, n, tol)
                    })
                    .collect();
                // Taken for the surface only when they are about as large
                // and cover about the same part of the plane: not a small
                // boss top left where a deepened floor was.
                // ponytail: boxes, not outlines: a ring's plane taken by the
                // boss inside it, as large, passes, said as changed shape;
                // keep outlines with the places if that needs telling apart.
                let area: f32 = there.iter().map(|n| n.area).sum();
                let mut span = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
                for n in &there {
                    for i in 0..3 {
                        span.0[i] = span.0[i].min(n.min[i]);
                        span.1[i] = span.1[i].max(n.max[i]);
                    }
                }
                if there.is_empty()
                    || !(0.5..=2.).contains(&(area / place.area))
                    || overlap(place.normal, (place.min, place.max), span) < 0.5
                {
                    lost += 1;
                    continue;
                }
                let there: Vec<usize> = there.iter().map(|n| n.id).collect();
                reshaped += 1;
                for id in there {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
            }
            let total = kept_places.faces.len();
            let faces = |n: usize| if n == 1 { "surface" } else { "surfaces" };
            let mut notes = vec![];
            if ids.is_empty() {
                notes.push(format!(
                    "None of its {total} picked {} is on the new part. Pick them again in Geometry.",
                    faces(total)
                ));
            } else {
                o.surfaces = Some(Selection {
                    mesh_fingerprint: mesh.fingerprint.clone(),
                    ids,
                });
                followed.operations += 1;
            }
            if reshaped > 0 {
                notes.push(format!(
                    "{reshaped} picked {} changed shape; the surfaces now on {} plane are picked instead. Check them in Geometry.",
                    faces(reshaped),
                    if reshaped == 1 { "its" } else { "their" },
                ));
            }
            if lost > 0 && lost < total {
                notes.push(format!(
                    "{lost} of its {total} picked {} {} not found on the new part and {} dropped. Pick {} again in Geometry if it still needs {}.",
                    faces(total),
                    if lost == 1 { "is" } else { "are" },
                    if lost == 1 { "was" } else { "were" },
                    if lost == 1 { "it" } else { "them" },
                    if lost == 1 { "it" } else { "them" },
                ));
            }
            if !notes.is_empty() {
                followed
                    .notes
                    .extend(notes.iter().map(|n| format!("{name}: {n}")));
                pick_checks.insert(o.id.clone(), notes.join(" "));
            }
        }
    }
    record(doc, mesh);
    followed
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A box `[0, x1] × [0, 10] × [0, h]` as triangles in mm; `shift` lists
    /// them in another order.
    pub(crate) fn block(x1: f32, h: f32, shift: bool) -> Vec<[[f32; 3]; 3]> {
        let quad = |a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]| [[a, b, c], [a, c, d]];
        let (x0, y1) = (0., 10.);
        let mut t = vec![];
        t.extend(quad([x0, 0., h], [x1, 0., h], [x1, y1, h], [x0, y1, h]));
        t.extend(quad([x0, 0., 0.], [x0, y1, 0.], [x1, y1, 0.], [x1, 0., 0.]));
        t.extend(quad([x0, 0., 0.], [x1, 0., 0.], [x1, 0., h], [x0, 0., h]));
        t.extend(quad([x0, y1, 0.], [x0, y1, h], [x1, y1, h], [x1, y1, 0.]));
        t.extend(quad([x0, 0., 0.], [x0, 0., h], [x0, y1, h], [x0, y1, 0.]));
        t.extend(quad([x1, 0., 0.], [x1, y1, 0.], [x1, y1, h], [x1, 0., h]));
        if shift {
            // The same faces in another order: numbered anew.
            t.rotate_left(3);
        }
        t
    }
    fn surface(mesh: &Mesh, normal: [f32; 3]) -> usize {
        (mesh.surfaces.iter())
            .position(|s| dot(s.normal, normal) > 0.999)
            .unwrap()
    }
    fn doc_picking(mesh: &Mesh, ids: Vec<usize>) -> Document {
        let mut doc = Document::sample();
        let op = &mut doc.setup_mut().operations[0];
        op.surfaces = Some(Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids,
        });
        doc
    }

    /// The same part with its surfaces numbered anew: each pick is found as
    /// it was, and the places are kept for the new part.
    #[test]
    fn picks_follow_surfaces_numbered_anew() {
        let before = crate::mesh::normalize(block(40., 8., false)).unwrap();
        let after = crate::mesh::normalize(block(40., 8., true)).unwrap();
        assert_ne!(before.fingerprint, after.fingerprint);
        let (top, side) = (
            surface(&before, [0., 0., 1.]),
            surface(&before, [1., 0., 0.]),
        );
        let mut doc = doc_picking(&before, vec![top, side]);
        record(&mut doc, &before);
        // As kept before places had their moments: read back, and found as
        // they were.
        let mut old = serde_json::to_value(&doc).unwrap();
        for p in old["places"].as_object_mut().unwrap().values_mut() {
            for f in p["faces"].as_array_mut().unwrap() {
                f.as_object_mut().unwrap().remove("moments");
            }
        }
        let mut old: Document = serde_json::from_value(old).unwrap();
        assert!((old.places.values().flat_map(|p| &p.faces)).all(|f| f.moments == [0.; 6]));
        let followed = follow(&mut old, &after, None);
        assert!(followed.notes.is_empty(), "{:?}", followed.notes);
        assert_eq!(followed.operations, 1);
        let followed = follow(&mut doc, &after, None);
        assert_eq!(followed.operations, 1);
        assert!(followed.notes.is_empty(), "{:?}", followed.notes);
        let op = &doc.setup().operations[0];
        let s = op.surfaces.as_ref().unwrap();
        assert!(s.matches(&after));
        assert_eq!(
            s.ids,
            vec![surface(&after, [0., 0., 1.]), surface(&after, [1., 0., 0.])]
        );
        assert!(doc.places[&op.id].describe(s));
        // Kept in the project file and read back.
        let text = serde_json::to_string(&doc).unwrap();
        let back: Document = serde_json::from_str(&text).unwrap();
        assert_eq!(back.places, doc.places);
        // Those of an operation deleted go.
        let id = op.id.clone();
        doc.setup_mut().operations.remove(0);
        record(&mut doc, &after);
        assert!(!doc.places.contains_key(&id));
    }

    /// A longer part: its top changed shape on its plane and is picked
    /// again, saying so; its far end moved and is dropped, saying so.
    #[test]
    fn a_reshaped_surface_is_picked_on_its_plane_and_a_moved_one_dropped() {
        let before = crate::mesh::normalize(block(40., 8., false)).unwrap();
        let after = crate::mesh::normalize(block(60., 8., false)).unwrap();
        let ids = vec![
            surface(&before, [0., 0., 1.]),
            surface(&before, [1., 0., 0.]),
        ];
        let mut doc = doc_picking(&before, ids);
        record(&mut doc, &before);
        let followed = follow(&mut doc, &after, Some(&before));
        let s = doc.setup().operations[0].surfaces.as_ref().unwrap();
        assert_eq!(s.ids, vec![surface(&after, [0., 0., 1.])]);
        assert_eq!(followed.notes.len(), 2, "{:?}", followed.notes);
        assert!(followed.notes[0].contains("1 picked surface changed shape"));
        assert!(
            followed.notes[1].contains("1 of its 2 picked surfaces is not found on the new part")
        );
        let id = &doc.setup().operations[0].id;
        assert!(doc.pick_checks[id].contains("changed shape"));
    }

    /// Nothing found: the selection is left as it was, with its places, for
    /// a later version of the file. A selection with no places says it
    /// cannot be followed.
    #[test]
    fn picks_found_nowhere_are_kept_for_a_later_file() {
        let before = crate::mesh::normalize(block(40., 8., false)).unwrap();
        let after = crate::mesh::normalize(block(40., 12., false)).unwrap();
        let top = surface(&before, [0., 0., 1.]);
        let mut doc = doc_picking(&before, vec![top]);
        record(&mut doc, &before);
        let places = doc.places.clone();
        let followed = follow(&mut doc, &after, Some(&before));
        assert_eq!(followed.operations, 0);
        assert!(
            followed.notes[0].contains("None of its 1 picked surface"),
            "{:?}",
            followed.notes
        );
        let op = &doc.setup().operations[0];
        assert!(!op.surfaces.as_ref().unwrap().matches(&after));
        assert_eq!(doc.places, places);

        let mut unrecorded = doc_picking(&before, vec![top]);
        let followed = follow(&mut unrecorded, &after, None);
        assert!(
            followed.notes[0].contains("Pick them again"),
            "{:?}",
            followed.notes
        );
    }

    /// A rectangle `[x0, x1] × [y0, y1]` at height `z` facing up (or down).
    fn rect(x0: f32, y0: f32, x1: f32, y1: f32, z: f32, up: bool) -> Vec<[[f32; 3]; 3]> {
        let (a, b, c, d) = ([x0, y0, z], [x1, y0, z], [x1, y1, z], [x0, y1, z]);
        if up {
            vec![[a, b, c], [a, c, d]]
        } else {
            vec![[a, c, b], [a, d, c]]
        }
    }
    fn up_at(mesh: &Mesh, x: f32, y: f32, z: f32) -> usize {
        let scale = mesh.display_scale();
        (mesh.surfaces.iter().enumerate())
            .filter(|(_, s)| s.normal[2] > 0.999)
            .map(|(i, s)| {
                (
                    i,
                    (0..3)
                        .map(|k| (s.center[k] - mesh.origin[k]) / scale)
                        .collect::<Vec<_>>(),
                )
            })
            .min_by(|a, b| {
                let d = |c: &Vec<f32>| (c[0] - x).hypot(c[1] - y) + (c[2] - z).abs();
                d(&a.1).total_cmp(&d(&b.1))
            })
            .unwrap()
            .0
    }

    /// Review finding: a pocket floor deepened, with a small boss left at
    /// its old height, is not taken for the boss's top: it is not found,
    /// and the operation says so until checked.
    #[test]
    fn a_deepened_floor_is_not_taken_for_a_boss_left_at_its_height() {
        let mut before = rect(0., 0., 40., 40., 0., false);
        before.extend(rect(12., 12., 28., 28., 5., true));
        let mut after = rect(0., 0., 40., 40., 0., false);
        after.extend(rect(12., 12., 28., 28., 3., true));
        after.extend(rect(18., 18., 22., 22., 5., true));
        let before = crate::mesh::normalize(before).unwrap();
        let after = crate::mesh::normalize(after).unwrap();
        let mut doc = doc_picking(&before, vec![up_at(&before, 20., 20., 5.)]);
        record(&mut doc, &before);
        for old in [Some(&before), None] {
            let mut doc = doc.clone();
            let followed = follow(&mut doc, &after, old);
            assert_eq!(followed.operations, 0);
            assert!(
                followed.notes[0].contains("None of its 1 picked surface"),
                "{:?}",
                followed.notes
            );
            let op = &doc.setup().operations[0];
            assert!(!op.surfaces.as_ref().unwrap().matches(&after));
            assert!(doc.pick_checks[&op.id].contains("Pick them again"));
        }
    }

    /// Review finding: an L-shaped floor shortened, beside a small pocket
    /// floor on its plane inside its box that was not picked, picks the
    /// floor alone, the part as it was loaded or not (a project opened after
    /// its file changed).
    #[test]
    fn a_reshaped_floor_does_not_take_a_surface_beside_it_not_picked() {
        let part = |x: f32| {
            let mut t = rect(0., 0., 40., 40., 0., false);
            t.extend(rect(0., 0., 6., 6., 5., true));
            t.extend(rect(6., 0., x, 6., 5., true));
            t.extend(rect(0., 6., 6., 20., 5., true));
            t.extend(rect(12., 12., 16., 16., 5., true));
            crate::mesh::normalize(t).unwrap()
        };
        let (before, after) = (part(20.), part(16.));
        let l = before.region_of(up_at(&before, 1., 1., 5.));
        let floor: Vec<usize> = (0..before.surfaces.len())
            .filter(|&i| before.surfaces[i].normal[2] > 0.999 && i != up_at(&before, 14., 14., 5.))
            .collect();
        assert!(!l.is_empty());
        let mut doc = doc_picking(&before, floor);
        record(&mut doc, &before);
        let small = up_at(&after, 14., 14., 5.);
        for old in [Some(&before), None] {
            let mut doc = doc.clone();
            let followed = follow(&mut doc, &after, old);
            let ids = &doc.setup().operations[0].surfaces.as_ref().unwrap().ids;
            assert!(!ids.contains(&small), "{ids:?} {:?}", followed.notes);
            assert!(!ids.is_empty());
            assert!(
                followed.notes.iter().any(|n| n.contains("changed shape")),
                "{:?}",
                followed.notes
            );
        }
    }

    /// Review finding: a square ring floor deepened, with the boss inside it
    /// grown to the ring's area at the ring's old height, is not taken for
    /// the boss top unchanged: same centre and area are not the same
    /// surface. Taken as the ring changed shape, it says so until checked.
    #[test]
    fn a_ring_is_not_taken_for_a_boss_top_of_its_centre_and_area() {
        // A 20 × 20 ring round a 12 × 12 hole: 256 mm², at `z`, in four
        // trapezoids sharing whole edges, so one surface.
        let ring = |z: f32| {
            let o = [[0., 0.], [20., 0.], [20., 20.], [0., 20.]];
            let i = [[4., 4.], [16., 4.], [16., 16.], [4., 16.]];
            let p = |[x, y]: [f32; 2]| [x, y, z];
            (0..4)
                .flat_map(|k| {
                    let n = (k + 1) % 4;
                    [[p(o[k]), p(o[n]), p(i[n])], [p(o[k]), p(i[n]), p(i[k])]]
                })
                .collect::<Vec<_>>()
        };
        let mut before = rect(-10., -10., 30., 30., 0., false);
        before.extend(ring(5.));
        let mut after = rect(-10., -10., 30., 30., 0., false);
        after.extend(ring(3.));
        after.extend(rect(2., 2., 18., 18., 5., true));
        let before = crate::mesh::normalize(before).unwrap();
        let after = crate::mesh::normalize(after).unwrap();
        let ring_ids: Vec<usize> = (0..before.surfaces.len())
            .filter(|&i| before.surfaces[i].normal[2] > 0.999)
            .collect();
        assert_eq!(ring_ids.len(), 1);
        let mut doc = doc_picking(&before, ring_ids);
        record(&mut doc, &before);
        let boss = up_at(&after, 10., 10., 5.);
        for old in [Some(&before), None] {
            let mut doc = doc.clone();
            let followed = follow(&mut doc, &after, old);
            let op = &doc.setup().operations[0];
            let s = op.surfaces.as_ref().unwrap();
            let note = &doc.pick_checks[&op.id];
            assert!(
                !(s.matches(&after) && s.ids.contains(&boss)) || note.contains("changed shape"),
                "{:?} {note}",
                s.ids
            );
        }
    }
}
