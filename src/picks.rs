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
/// area and box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    /// The surface's number on the mesh the places were recorded on.
    pub id: usize,
    pub normal: [f32; 3],
    pub center: [f32; 3],
    pub area: f32,
    pub min: [f32; 3],
    pub max: [f32; 3],
}
/// The places of an operation's picked surfaces, one for each, in the order
/// of its selection, on the mesh `mesh_fingerprint` names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Places {
    pub mesh_fingerprint: String,
    pub faces: Vec<Place>,
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
/// operation it could not carry whole.
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
        })
        .collect();
    for face in &mesh.faces {
        let Some(place) = face.feature.and_then(|id| out.get_mut(id)) else {
            continue;
        };
        for p in &face.points {
            let p = file(*p);
            for i in 0..3 {
                place.min[i] = place.min[i].min(p[i]);
                place.max[i] = place.max[i].max(p[i]);
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
            Some(faces) => kept.insert(
                o.id.clone(),
                Places {
                    mesh_fingerprint: selection.mesh_fingerprint.clone(),
                    faces,
                },
            ),
            None => kept.remove(&o.id),
        };
    }
}

const SAME_NORMAL: f32 = 1. - 1e-6;
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
/// The same surface: facing the same way, at the same place, as large.
fn same(a: &Place, b: &Place, tol: f32) -> bool {
    let d = sub(a.center, b.center);
    dot(a.normal, b.normal) >= SAME_NORMAL
        && dot(d, d).sqrt() <= tol
        && (a.area - b.area).abs() <= 1e-3 * a.area.max(b.area) + tol * tol
}
/// On the same plane, facing the same way, with boxes that meet.
fn same_plane(a: &Place, b: &Place, tol: f32) -> bool {
    dot(a.normal, b.normal) >= SAME_NORMAL
        && dot(a.normal, sub(b.center, a.center)).abs() <= tol
        && (0..3).all(|i| a.min[i] <= b.max[i] + tol && b.min[i] <= a.max[i] + tol)
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
    let extent = mesh.dimensions.into_iter().fold(0f32, f32::max);
    let tol = (extent * 1e-4).max(f32::EPSILON);
    let new = places(mesh);
    let grid = Grid::new(&new, tol);
    // The surfaces of `mesh` that `old` had too, unchanged: worked out only
    // when a surface changed shape.
    let mut kept: Option<HashSet<usize>> = None;
    let several = doc.setups.len() > 1;
    let Document {
        setups,
        places: kept_places_of,
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
                followed.notes.push(format!(
                    "{name}: its surfaces were picked before picks were kept with their places, so they are not found on the new part. Pick them again in Geometry."
                ));
                continue;
            };
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
                let there: Vec<usize> = (new.iter())
                    .filter(|n| !kept.contains(&n.id) && same_plane(place, n, tol))
                    .map(|n| n.id)
                    .collect();
                if there.is_empty() {
                    lost += 1;
                    continue;
                }
                reshaped += 1;
                for id in there {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
            }
            let total = kept_places.faces.len();
            let faces = |n: usize| if n == 1 { "surface" } else { "surfaces" };
            if ids.is_empty() {
                followed.notes.push(format!(
                    "{name}: none of its {total} picked {} is on the new part. Pick them again in Geometry.",
                    faces(total)
                ));
                continue;
            }
            o.surfaces = Some(Selection {
                mesh_fingerprint: mesh.fingerprint.clone(),
                ids,
            });
            followed.operations += 1;
            if reshaped > 0 {
                followed.notes.push(format!(
                    "{name}: {reshaped} picked {} changed shape; the surfaces now on {} plane are picked instead. Check them in Geometry.",
                    faces(reshaped),
                    if reshaped == 1 { "its" } else { "their" },
                ));
            }
            if lost > 0 {
                followed.notes.push(format!(
                    "{name}: {lost} of its {total} picked {} {} not on the new part and {} dropped. Pick {} again in Geometry if it still needs {}.",
                    faces(total),
                    if lost == 1 { "is" } else { "are" },
                    if lost == 1 { "was" } else { "were" },
                    if lost == 1 { "it" } else { "them" },
                    if lost == 1 { "it" } else { "them" },
                ));
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
        assert!(followed.notes[1].contains("1 of its 2 picked surfaces is not on the new part"));
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
            followed.notes[0].contains("none of its 1 picked surface"),
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
}
