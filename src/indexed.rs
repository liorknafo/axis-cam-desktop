//! Indexed setups on the rotary A axis (LIO-188). A later setup whose
//! Reposition action is "Index rotary axis" does not re-clamp the stock: A
//! turns it, in the clamping of the setup before it that is not indexed
//! itself (its base: setup 1, or a later true re-clamp). Its frame is the
//! base's frame turned about the A centreline by the setup's A angle, its
//! stock is what the setups before it leave, and its work zero, its work
//! offset, its machine and its workholding are the base's: nothing is
//! probed again. A setup may list several A angles for the same operations
//! (a pattern of index steps, 0, 90, 180, 270); the 3D view shows the first.
//!
//! The setup's frame keeps the machine's axes, as `chuck` has it: X along
//! the A axis, the A centreline through the middle of the Y and Z of the
//! stock the base starts from. Turning about it leaves that line, and a
//! work zero on it, where they are.
//!
//! THE SIGN OF A. Positive A turns the part by the right-hand rule about
//! +X, as LinuxCNC has it: a point on +Y of the centreline goes up to +Z at
//! A90, and +Z goes to -Y. Seen from the chuck looking along +X (towards
//! the tailstock), positive A turns the part clockwise. The UC100 driver's
//! simulated bar ("positive A turns +Y toward +Z"), the Control page's A
//! zero on a flat and the chuck's solids (`chuck::Rig::solids`) turn the
//! same way. A machine whose A direction is Negative turns it the other way.
use crate::mesh::{Bounds, Mesh, rotation};
use crate::model::{Document, Machine, Setup, get, num};

/// The Reposition action of an indexed setup.
pub const ACTION: &str = "Index rotary axis";
/// Why an operation that picks holes at several A angles makes nothing.
pub const NO_HOLE_UP: &str = "None of the picked holes is turned up and open from above at any of this setup's A angles: pick holes that one of the angles turns up, or add the angle that does.";
/// The setup field that lists its A angles, degrees. A setup whose action
/// is `ACTION` without it is from a project saved before indexing turned
/// the stock: the action is a label there, and the setup keeps its own
/// placement, as it did.
pub const ANGLES: &str = "a_angles";
/// What a setup indexed from the Reposition choice starts with.
pub const DEFAULT_ANGLES: &str = "90";
/// The work zeros for work on A: X at the stock's free (+X) end or at the
/// chuck's face, Y and Z on the A centreline. A0 is where the Control
/// page's rotary calibration (Find A zero on a flat) put it.
pub const ORIGINS: [&str; 2] = ["A axis at stock end", "A axis at chuck face"];
/// The fields of a setup that are its own when it is indexed: everything
/// else (placement, work zero and offset, workholding) is its base's.
const OWN: [&str; 5] = ["name", "notes", "reposition", ANGLES, "stock_source"];

/// `op` as it runs on the part `posed` at one of its setup's angles: a
/// Facing operation whose selected faces are not all on top there faces
/// those that are, so that one operation faces a square's four sides at A
/// 0, 90, 180 and 270° (LIO-211). Unchanged otherwise, and when none of its
/// faces is on top, which the facing then refuses. An operation that picks
/// holes at several angles cuts its own at each (`holes_by_angle`).
pub fn on_top<'a>(
    op: &'a crate::model::Operation,
    posed: &Setup,
    mesh: &Mesh,
) -> std::borrow::Cow<'a, crate::model::Operation> {
    use std::borrow::Cow;
    let Some(selection) = op.surfaces.as_ref() else {
        return Cow::Borrowed(op);
    };
    if op.strategy != crate::model::Strategy::Face || selection.ids.len() < 2 {
        return Cow::Borrowed(op);
    }
    let angles = crate::stock::Placement::new(mesh, Some(posed)).rotation();
    let up = (selection.ids.iter().copied())
        .filter(|&id| faces_up(mesh, angles, id))
        .collect::<Vec<_>>();
    if up.is_empty() || up.len() == selection.ids.len() {
        return Cow::Borrowed(op);
    }
    Cow::Owned(with_surfaces(op, up))
}
/// Whether surface `id` of `mesh` faces up, the part turned by `angles`.
fn faces_up(mesh: &Mesh, angles: [f32; 3], id: usize) -> bool {
    mesh.surfaces
        .get(id)
        .is_some_and(|f| rotation(f.normal, angles)[2] >= 0.5f32.to_radians().cos())
}
/// `op` cutting only the surfaces `ids` of its selection.
fn with_surfaces(op: &crate::model::Operation, ids: Vec<usize>) -> crate::model::Operation {
    let mut op = op.clone();
    if let Some(s) = op.surfaces.as_mut() {
        s.ids = ids;
    }
    op
}
/// " at A30°" for a hole read at A 30° (`cam::Hole::a`), to tell holes
/// apart that are listed at one centre in their own angles' frames; empty
/// for None.
pub fn at_angle(a: Option<f32>) -> String {
    a.map_or(String::new(), |a| format!(" at A{}°", degrees(a)))
}
/// Whether `op`, of `setup`, picks holes (a Drill, a Pocket, a Contour,
/// `crate::picks_holes`) in an indexed setup of several A angles: then each
/// angle cuts the holes it turns up (`holes_by_angle`).
pub fn holes_at_angles(op: &crate::model::Operation, setup: &Setup) -> bool {
    crate::picks_holes(op)
        && op.surfaces.is_some()
        && is_indexed(setup)
        && angles(setup).is_ok_and(|a| a.len() > 1)
}
/// The operation field that lists, at one A angle of an indexed setup of
/// several, the walls of the holes drilled just through their own wall
/// there (`holes_by_angle`, read by `cam::holes`): set on the copy of the
/// operation that angle cuts, never saved.
pub const OWN_WALL: &str = "own_wall_holes";
/// What one A angle of an operation that picks holes cuts.
pub struct AngleCut {
    pub a: f32,
    /// The setup posed at `a` (`Document::setup_at`).
    pub posed: Setup,
    /// The picked surfaces cut there: those of the holes it turns up, and
    /// the picked faces on top there. Empty when it cuts none: the angle
    /// is skipped.
    pub ids: Vec<usize>,
    /// The walls of the holes drilled there just through their own wall.
    pub own_wall: Vec<usize>,
    /// Whether a plunge there goes on across a cavity and out through the
    /// part's far side (for the flute-length message: `ACROSS`).
    pub across: bool,
}
impl AngleCut {
    /// `op` as this angle cuts it: its surfaces there, and the holes that
    /// stop through their own wall (`OWN_WALL`).
    pub fn op(&self, op: &crate::model::Operation) -> crate::model::Operation {
        let mut op = with_surfaces(op, self.ids.clone());
        if !self.own_wall.is_empty() {
            let list: Vec<String> = self.own_wall.iter().map(|w| w.to_string()).collect();
            op.p.insert(OWN_WALL.into(), list.join(" "));
        }
        op
    }
}
/// What a plunge across a cavity refused for its flute length adds: each
/// hole from its own side, with a short drill.
pub const ACROSS: &str = "This plunge goes on across the cavity under the hole and out through the far wall. To drill each hole from its own side with a short drill, pick the hole opposite too and add the angle that turns it up";
/// What each A angle of operation `op` of setup `si`, which picks holes at
/// several angles (`holes_at_angles`), cuts, in the setup's order, and how
/// many picked holes (or faces) no angle drills.
///
/// Each picked hole goes to the first angle that turns its axis along Z
/// and opens it upward (`hole_along_z`, `opens_up`), each picked face to the
/// first that turns it up. Then a hole over a cavity (`cam::into_cavity`, a
/// tube's bore) whose far side has the picked hole opposite, which another
/// angle drills, stops just through its own wall: each is drilled from its
/// own side, with a short drill. Else, where the far side lets the drill
/// out (`cam::part_under_disc`: a hole opposite at least as wide as it), it
/// goes on through as main drills it, and a picked hole opposite that no
/// angle has is drilled with it, when the plunge reaches its bottom (with
/// the operation's Bottom offset and Breakthrough); else (a solid far
/// wall) it stops through its own wall. A single-angle or 3-axis setup
/// never comes here.
pub fn holes_by_angle(
    doc: &Document,
    si: usize,
    op: &crate::model::Operation,
    mesh: &Mesh,
) -> Result<(Vec<AngleCut>, usize), String> {
    use std::collections::HashSet;
    let setup = &doc.setups[si];
    let ids: Vec<usize> = (op.surfaces.iter())
        .flat_map(|s| s.ids.iter().copied())
        .collect();
    let picked: HashSet<usize> = ids.iter().copied().collect();
    let mut done = HashSet::new();
    // The first pass: each angle's posed setup and part, the holes it is
    // the first to turn up (and the faces), and the picked holes along Z
    // there that open downward.
    let mut at = vec![];
    for a in angles(setup)? {
        let posed = doc.setup_at(setup, si, a, mesh)?;
        let g = crate::cam::Geometry::new(&posed, mesh);
        let turned = crate::stock::Placement::new(mesh, Some(&posed)).rotation();
        let (mut up, mut down, mut faces, mut tried) = (vec![], vec![], vec![], HashSet::new());
        for &id in &ids {
            if tried.contains(&id) {
                continue;
            }
            // A hole another angle drills is still one that opens downward
            // here, past the cavity under a hole this one turns up.
            if let Some(walls) = hole_along_z(&posed, mesh, id) {
                tried.extend(walls.iter().copied());
                if !walls.iter().all(|w| picked.contains(w)) {
                    continue;
                }
                if !opens_up(&g, &walls) {
                    down.push(walls);
                } else if !walls.iter().any(|w| done.contains(w)) {
                    done.extend(walls.iter().copied());
                    up.push(walls);
                }
            } else if done.contains(&id) {
                continue;
            } else if faces_up(mesh, turned, id) {
                done.insert(id);
                faces.push(id);
            }
        }
        at.push((a, posed, g, up, down, faces));
    }
    // Whether the holes whose walls are `a` and `b` are one.
    let same = |a: &[usize], b: &[usize]| a.iter().any(|w| b.contains(w));
    // The angle each assigned hole is drilled at.
    let assigned: Vec<(usize, &Vec<usize>)> = (at.iter().enumerate())
        .flat_map(|(i, (_, _, _, up, _, _))| up.iter().map(move |w| (i, w)))
        .collect();
    // The drill: its radius, less a facet's sagitta, probes the far side.
    let drill = (doc.tools.iter())
        .find(|t| t.id == op.tool)
        .and_then(|t| num(&t.p, "diameter"))
        .filter(|d| *d > 0.);
    let offset = num(&op.p, "bottom").unwrap_or(0.);
    let breakthrough = match get(&op.p, "through") {
        "Yes" => num(&op.p, "breakthrough").unwrap_or(0.).max(0.),
        _ => 0.,
    };
    // The second pass: each drilled hole's depth, and what it drills with it.
    let mut out = vec![];
    for (i, (a, posed, g, up, down, faces)) in at.iter().enumerate() {
        let (mut own_wall, mut across) = (vec![], false);
        for walls in up {
            let Some(ring) = crate::cam::wall_ring(g, walls) else {
                continue;
            };
            if !crate::cam::into_cavity(g, ring.centre, ring.r, ring.bottom) {
                continue;
            }
            let far = (down.iter())
                .filter_map(|w| Some((w, crate::cam::wall_ring(g, w)?)))
                .find(|(_, f)| ring.over(f));
            // Drilled from its own side at another angle.
            if let Some((w, _)) = far
                && assigned.iter().any(|(j, b)| *j != i && same(w, b))
            {
                own_wall.extend(walls.iter().copied());
                continue;
            }
            // The drill's radius less a facet's sagitta: a drill as wide as
            // the hole opposite goes out through it.
            let probe = drill.map_or(ring.r, |d| d / 2.) - ring.sagitta - 0.01;
            if crate::cam::part_under_disc(g, ring.centre, probe, ring.bottom) {
                own_wall.extend(walls.iter().copied());
                continue;
            }
            // On through and out, as main drills it: to the part's bottom.
            across = true;
            let target = g.bounds.min[2] + offset - breakthrough;
            if let Some((w, f)) = far
                && target <= f.bottom + 0.01
            {
                done.extend(w.iter().copied());
            }
        }
        let cut: HashSet<usize> = up.iter().flatten().chain(faces).copied().collect();
        out.push(AngleCut {
            a: *a,
            posed: posed.clone(),
            ids: (ids.iter().copied())
                .filter(|id| cut.contains(id))
                .collect(),
            own_wall,
            across,
        });
    }
    // The picked surfaces no angle cuts, counted by hole: those joined by a
    // corner of their triangles are one.
    let left: Vec<usize> = ids
        .iter()
        .copied()
        .filter(|id| !done.contains(id))
        .collect();
    let mut root: std::collections::HashMap<usize, usize> =
        left.iter().map(|&id| (id, id)).collect();
    fn find(root: &mut std::collections::HashMap<usize, usize>, id: usize) -> usize {
        let up = root[&id];
        if up == id {
            return id;
        }
        let top = find(root, up);
        root.insert(id, top);
        top
    }
    let mut corner: std::collections::HashMap<[i64; 3], usize> = Default::default();
    for f in &mesh.faces {
        let Some(id) = f.feature.filter(|id| root.contains_key(id)) else {
            continue;
        };
        for p in &f.points {
            let key = p.map(|v| (v as f64 * 1e5).round() as i64);
            match corner.get(&key) {
                Some(&other) => {
                    let (a, b) = (find(&mut root, id), find(&mut root, other));
                    root.insert(a, b);
                }
                None => {
                    corner.insert(key, id);
                }
            }
        }
    }
    let count = (left.iter())
        .filter(|&&id| find(&mut root, id) == id)
        .count();
    Ok((out, count))
}
/// Whether the hole whose walls are `walls` opens upward in `g`: nothing of
/// the part over it past a gap (`cam::part_past_gap`), as there is over a
/// through-hole facing down, across a tube's bore.
fn opens_up(g: &crate::cam::Geometry, walls: &[usize]) -> bool {
    crate::cam::wall_ring(g, walls)
        .is_some_and(|w| !crate::cam::part_past_gap(g, w.centre, w.r, w.top, true))
}
/// The mean lean of a hole's walls from Z under which its axis is along Z:
/// their triangles' normals' Z, area-weighted, is (2/π)·sin t for a hole
/// tilted by t, so this is about 4.5°.
const ALONG_Z: f32 = 0.05;
/// The walls of the hole surface `id` is a wall of (`pocket::hole_walls`),
/// when `posed` turns that hole's axis along Z; None when it does not, or
/// `id` is no wall of a hole there.
pub fn hole_along_z(posed: &Setup, mesh: &Mesh, id: usize) -> Option<Vec<usize>> {
    if !crate::pocket::is_wall(posed, mesh, id) {
        return None;
    }
    let walls = crate::pocket::hole_walls(posed, mesh, id).ok()?;
    let angles = crate::stock::Placement::new(mesh, Some(posed)).rotation();
    // By its triangles: one surface's normal says nothing of a whole
    // cylinder's (a STEP part's hole).
    let (mut lean, mut area) = (0., 0.);
    for f in (mesh.faces.iter()).filter(|f| f.feature.is_some_and(|w| walls.contains(&w))) {
        let Some([a, b, c]) = f
            .points
            .get(..3)
            .and_then(|p| <[[f32; 3]; 3]>::try_from(p).ok())
        else {
            continue;
        };
        let [a, b, c] = [a, b, c].map(|p| rotation(p, angles));
        let n = cross(sub(b, a), sub(c, a));
        lean += n[2].abs();
        area += (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    }
    (area > 0. && lean / area < ALONG_Z).then_some(walls)
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|k| a[k] - b[k])
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// The selected surfaces' problems (`surface::issues`) of operation `op` of
/// setup `si`: an indexed setup's at each of its angles, with the part
/// posed there and its faces on top there (`on_top`), each said once. An
/// operation that picks holes at several angles is checked at each angle
/// on what it cuts there (`holes_by_angle`), and refused when a picked hole
/// is cut at none.
pub fn surface_issues(
    doc: &Document,
    si: usize,
    op: &crate::model::Operation,
    mesh: Option<&Mesh>,
) -> Vec<String> {
    let setup = &doc.setups[si];
    let (Some(mesh), Ok(list)) = (mesh, angles(setup)) else {
        return crate::surface::issues(op, setup, mesh);
    };
    if !is_indexed(setup) {
        return crate::surface::issues(op, setup, Some(mesh));
    }
    let mut out: Vec<String> = vec![];
    let mut add = |issues: Vec<String>| {
        for issue in issues {
            if !out.contains(&issue) {
                out.push(issue);
            }
        }
    };
    if holes_at_angles(op, setup) {
        let (cuts, left) = match holes_by_angle(doc, si, op, mesh) {
            Ok(found) => found,
            Err(e) => return vec![e],
        };
        for c in cuts.iter().filter(|c| !c.ids.is_empty()) {
            add(crate::surface::issues(&c.op(op), &c.posed, Some(mesh)));
        }
        if left > 0 {
            add(vec![format!(
                "{left} of the picked holes {} drilled at none of this setup's A angles ({}): none turns {} up and open from above, and no plunge from the hole opposite goes out through {}. Pick holes that one of the angles turns up, or add the angle that does.",
                if left == 1 { "is" } else { "are" },
                angles_text(&list),
                if left == 1 { "it" } else { "them" },
                if left == 1 { "it" } else { "them" }
            )]);
        }
        return out;
    }
    for &a in &list {
        let Ok(posed) = doc.setup_at(setup, si, a, mesh) else {
            return crate::surface::issues(op, setup, Some(mesh));
        };
        add(crate::surface::issues(
            &on_top(op, &posed, mesh),
            &posed,
            Some(mesh),
        ));
    }
    out
}
impl Document {
    /// The setup a click on surface `id` of setup `si` picks a hole in: the
    /// setup as the view shows it, or for an indexed setup the setup posed at
    /// the first of its A angles that turns the hole of `id` along Z and
    /// opens it upward, so that one operation picks holes at each angle.
    pub fn picking_pose(&self, si: usize, mesh: &Mesh, id: usize) -> Setup {
        let setup = &self.setups[si];
        let up = |posed: &Setup| {
            hole_along_z(posed, mesh, id)
                .is_some_and(|walls| opens_up(&crate::cam::Geometry::new(posed, mesh), &walls))
        };
        if !is_indexed(setup) || up(setup) {
            return setup.clone();
        }
        (angles(setup).unwrap_or_default().into_iter())
            .filter_map(|a| self.setup_at(setup, si, a, mesh).ok())
            .find(|posed| up(posed))
            .unwrap_or_else(|| setup.clone())
    }
    /// What the Geometry tab lists of operation `op` of setup `si` that picks
    /// holes at several angles (`holes_at_angles`): `read` of what each angle
    /// cuts (`holes_by_angle`), read in the setup posed there, one after the
    /// other, each with its angle. None for another operation.
    pub fn holes_listed<T>(
        &self,
        si: usize,
        op: &crate::model::Operation,
        mesh: &Mesh,
        read: impl Fn(&Setup, &crate::model::Operation) -> Result<Vec<T>, String>,
    ) -> Option<Result<Vec<(f32, T)>, String>> {
        if !holes_at_angles(op, self.setups.get(si)?) {
            return None;
        }
        let listed = holes_by_angle(self, si, op, mesh).and_then(|(cuts, _)| {
            let mut all = vec![];
            for c in cuts.iter().filter(|c| !c.ids.is_empty()) {
                let read = read(&c.posed, &c.op(op))?;
                all.extend(read.into_iter().map(|h| (c.a, h)));
            }
            Ok(all)
        });
        Some(listed)
    }
}
/// Whether `setup` is indexed: its Reposition action is `ACTION` and it
/// lists A angles.
pub fn is_indexed(setup: &Setup) -> bool {
    get(&setup.p, "reposition") == ACTION && setup.p.contains_key(ANGLES)
}
/// Whether the Reposition action of `setup` is `ACTION` from a project
/// saved before indexing turned the stock: a label only.
pub fn is_label_only(setup: &Setup) -> bool {
    get(&setup.p, "reposition") == ACTION && !setup.p.contains_key(ANGLES)
}
/// The A angles `setup` lists, degrees, in order: separated by commas,
/// slashes, semicolons or spaces ("0, 90, 180, 270").
pub fn angles(setup: &Setup) -> Result<Vec<f32>, String> {
    let text = get(&setup.p, ANGLES);
    let mut out: Vec<f32> = vec![];
    for word in text
        .split(|c: char| c == ',' || c == '/' || c == ';' || c.is_whitespace())
        .filter(|w| !w.is_empty())
    {
        let a = word
            .trim_end_matches('°')
            .parse::<f32>()
            .ok()
            .filter(|a| a.is_finite())
            .ok_or_else(|| {
                format!("\"{word}\" is not an A angle: list them in degrees, as 0, 90, 180, -90.")
            })?;
        if out.iter().any(|b| (a - b).abs() < 1e-3) {
            return Err(format!("A {}° is listed twice.", degrees(a)));
        }
        out.push(a);
    }
    if out.is_empty() {
        return Err("List the A angles to index to, in degrees: 90, or 0, 90, 180, -90.".into());
    }
    Ok(out)
}
/// `a` degrees for a message, without a trailing ".0".
pub fn degrees(a: f32) -> String {
    let s = format!("{a:.3}");
    match s.trim_end_matches('0').trim_end_matches('.') {
        "-0" => "0".into(),
        t => t.into(),
    }
}
/// The A angles of `v` in words: "A 90°", "A 0°, 90°, 180°, 270°".
pub fn angles_text(v: &[f32]) -> String {
    let list: Vec<String> = v.iter().map(|a| format!("{}°", degrees(*a))).collect();
    format!("A {}", list.join(", "))
}
/// The A angle of `setup` for the setups list, the Job view and the Run
/// panel: its listed angles when it is indexed, A 0° for another setup on
/// a machine with an A axis, None without one.
pub fn label(setup: &Setup) -> Option<String> {
    if is_indexed(setup) {
        return Some(match angles(setup) {
            Ok(v) => angles_text(&v),
            Err(_) => "A ?".into(),
        });
    }
    setup.machine.has_a().then(|| "A 0°".into())
}
/// The A angles `v` as a setup lists them: "0, 90, 180, 270".
pub fn angles_list(v: &[f32]) -> String {
    v.iter().map(|a| degrees(*a)).collect::<Vec<_>>().join(", ")
}
/// Sets a setup's Reposition `action` in its fields `p`, as the Reposition
/// choice does: `ACTION` keeps the A angles the setup lists (`DEFAULT_ANGLES`
/// while it lists none), another action takes them away.
pub fn set_reposition(p: &mut crate::model::Fields, action: &str) {
    if action == ACTION {
        p.entry(ANGLES.into())
            .or_insert_with(|| DEFAULT_ANGLES.into());
    } else {
        p.remove(ANGLES);
    }
    p.insert("reposition".into(), action.into());
}
/// +1 when the machine's positive A turns the part by the right-hand rule
/// about +X (direction Positive), -1 for Negative.
pub fn sign(m: &Machine) -> f32 {
    if get(&m.p, "direction") == "Negative" {
        -1.
    } else {
        1.
    }
}
/// The point `p` of a setup's frame turned with the part about the line
/// along X through Y, Z `axis` by `turn` degrees, by the right-hand rule
/// about +X: +Y towards +Z for a positive turn.
pub fn turn_point(p: [f32; 3], axis: [f32; 2], turn: f32) -> [f32; 3] {
    let (s, c) = turn.to_radians().sin_cos();
    let (y, z) = (p[1] - axis[0], p[2] - axis[1]);
    [p[0], axis[0] + y * c - z * s, axis[1] + y * s + z * c]
}
/// The angles (`mesh::rotation`'s, X then Y then Z) that turn as `angles`
/// do followed by `turn` degrees about +X.
pub fn turned_angles(angles: [f32; 3], turn: f32) -> [f32; 3] {
    // The columns of the whole rotation: where it takes X, Y and Z.
    let col = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]
        .map(|e| turn_point(rotation(e, angles), [0., 0.], turn));
    let m = |r: usize, c: usize| col[c][r];
    // rotation(p, [a, b, c]) is Rz(c) Ry(b) Rx(a) p: its row 2 is
    // (-sin b, cos b sin a, cos b cos a), its column 0 (cos c cos b,
    // sin c cos b, -sin b).
    let b = (-m(2, 0)).clamp(-1., 1.).asin();
    let (a, c) = if m(2, 0).abs() < 1. - 1e-6 {
        (m(2, 1).atan2(m(2, 2)), m(1, 0).atan2(m(0, 0)))
    } else {
        // Y at ±90°: X and Z turn about the same line; Z is taken as 0.
        ((-m(1, 2)).atan2(m(1, 1)), 0.)
    };
    [a, b, c].map(f32::to_degrees)
}
/// How an indexed setup's part is turned from its base's placement, as
/// `Document::sync_turns` works it out. Not saved: it follows from the job.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Turn {
    /// The base setup: the clamping A turns the stock in.
    pub base: usize,
    /// The setup's first A angle, degrees, and what it turns the part
    /// about +X (`sign` × A).
    pub a: f32,
    pub turn: f32,
    /// The A centreline in the base's frame: Y and Z, mm.
    pub axis: [f32; 2],
    /// The box the base starts from, mm, in its frame.
    pub stock: Bounds,
}
/// The A centreline in a setup's frame for the `stock` box it starts
/// from: the middle of its Y and Z, where the chuck centres it.
pub fn axis_of(stock: Bounds) -> [f32; 2] {
    [
        (stock.min[1] + stock.max[1]) / 2.,
        (stock.min[2] + stock.max[2]) / 2.,
    ]
}
/// Where the work zero `origin` of `ORIGINS` is in a setup's frame: X at
/// the free end of `stock` (the box the clamping starts from) or the chuck's
/// face, Y and Z on `axis`. None for another work zero.
pub fn rotary_datum(
    setup: &Setup,
    origin: &str,
    stock: Bounds,
    axis: [f32; 2],
) -> Option<Result<[f32; 3], String>> {
    let x = match ORIGINS.iter().position(|o| *o == origin)? {
        0 => Ok(stock.max[0]),
        _ if !crate::chuck::holds(setup) => Err(
            "A axis at chuck face needs the stock held in the chuck: choose Chuck under Modelled workholding, or A axis at stock end.".to_string(),
        ),
        _ => crate::chuck::rig(setup, stock, false).map(|r| r.face),
    };
    Some(x.map(|x| [x, axis[0], axis[1]]))
}
/// How the operator finds a work zero of `ORIGINS`, for the program header
/// and the Work zero step. None for another work zero.
pub fn zero_words(origin: &str) -> Option<&'static str> {
    match ORIGINS.iter().position(|o| *o == origin)? {
        0 => Some(
            "Y0 Z0 on the A centreline from the rotary calibration (Rotary A on the Control page); touch off X0 on the stock's free end face; A0 where Find A zero on a flat put it.",
        ),
        _ => Some(
            "Y0 Z0 on the A centreline from the rotary calibration (Rotary A on the Control page); X0 on the chuck's face; A0 where Find A zero on a flat put it.",
        ),
    }
}
/// A setup at one A angle, for running its operations there: the angle,
/// how a point of the part comes into the setup's frame turned to it, and
/// where the work zero is, in that frame and on the machine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtAngle {
    /// The A angle, degrees, as the machine is commanded.
    pub a: f32,
    /// From the part's own frame (its file's mm) to the setup's frame with
    /// the stock turned to `a`.
    pub frame: crate::volume::Frame,
    /// The work zero in the setup's frame (mm): the same at every angle.
    pub zero: [f32; 3],
    /// The work zero in machine coordinates (mm): Y and Z from the A
    /// centreline of the machine's rotary geometry, X from its chuck face.
    /// None without that geometry or the stock held in the chuck.
    pub machine_zero: Option<[f32; 3]>,
    /// Whether that centreline was probed and has not gone stale since
    /// (`control::Rotary::measured`); else it is assumed.
    pub measured: bool,
}
impl AtAngle {
    /// The point `p` of the part (its file's mm) in work coordinates at
    /// this angle.
    pub fn work(&self, p: [f32; 3]) -> [f32; 3] {
        let q = self.frame.map(p);
        std::array::from_fn(|k| q[k] - self.zero[k])
    }
    /// The point `p` of the part in machine coordinates at this angle,
    /// when the work zero's place on the machine is known.
    pub fn machine(&self, p: [f32; 3]) -> Option<[f32; 3]> {
        let (w, z) = (self.work(p), self.machine_zero?);
        Some(std::array::from_fn(|k| w[k] + z[k]))
    }
}
impl Document {
    /// The base of setup `si` when it is indexed: the nearest setup before
    /// it that is not. None when it is not indexed, or is the first.
    pub fn index_base(&self, si: usize) -> Option<usize> {
        if !is_indexed(self.setups.get(si)?) {
            return None;
        }
        (0..si).rev().find(|&i| !is_indexed(&self.setups[i]))
    }
    /// Gives each indexed setup its base's machine and fields (`OWN`
    /// aside): its placement, work zero, work offset and workholding. Its
    /// turn (`sync_turns`) is not changed.
    pub fn sync_indexed(&mut self) {
        for si in 0..self.setups.len() {
            let Some(base) = self.index_base(si) else {
                continue;
            };
            let (machine, fields) = (
                self.setups[base].machine.clone(),
                self.setups[base].p.clone(),
            );
            let s = &mut self.setups[si];
            s.machine = machine;
            s.p.retain(|k, _| OWN.contains(&k.as_str()));
            for (k, v) in fields {
                if !OWN.contains(&k.as_str()) {
                    s.p.insert(k, v);
                }
            }
        }
    }
    /// Works out how each indexed setup's part is turned (`Setup::turn`),
    /// from its base's starting stock: with the part when its stock is
    /// sized from it, else without. Without its base's stock or its angles
    /// a setup is not turned.
    pub fn sync_turns(&mut self, mesh: Option<&Mesh>) {
        for si in 0..self.setups.len() {
            let turn = angles(&self.setups[si])
                .ok()
                .and_then(|v| self.turn_at(si, *v.first()?, mesh));
            self.setups[si].turn = turn;
        }
    }
    /// How A turns setup `si`'s part at A `a` (degrees) from its base's
    /// placement, from its base's starting stock. None when it is not
    /// indexed, or that stock cannot be told.
    pub fn turn_at(&self, si: usize, a: f32, mesh: Option<&Mesh>) -> Option<Turn> {
        let base = self.index_base(si)?;
        let stock = self.setup_stock(base, mesh)?;
        Some(Turn {
            base,
            a,
            turn: sign(&self.setups[si].machine) * a,
            axis: axis_of(stock),
            stock,
        })
    }
    /// `setup` (indexed setup `si`, or the copy a simulation runs) posed at
    /// A `a`: its turn is `a`'s, so its placement, its frame (`at_angle`'s)
    /// and its chuck are where A turns them.
    pub fn setup_at(&self, setup: &Setup, si: usize, a: f32, mesh: &Mesh) -> Result<Setup, String> {
        let mut s = setup.clone();
        s.turn = Some(self.turn_at(si, a, Some(mesh)).ok_or(STOCK_UNKNOWN)?);
        Ok(s)
    }
    /// What an index move of setup `si` clears with `clearance` (mm, the
    /// operation's), with the tool anywhere along A: the farthest the
    /// stock, the part or the chuck reaches from the A centreline, at any
    /// angle, and the height above it (`index_height` over no span).
    /// Refused when that height is above the machine's Z travel. For a
    /// setup that is not indexed, its own clamping's.
    pub fn swing(&self, si: usize, mesh: &Mesh, clearance: f32) -> Result<Swing, String> {
        self.index_height(si, mesh, clearance, None, &[])
    }
    /// `swing`, though its height is above the machine's Z travel.
    pub fn swing_unchecked(&self, si: usize, mesh: &Mesh, clearance: f32) -> Result<Swing, String> {
        self.index_height_unchecked(si, mesh, clearance, None, &[])
    }
    /// The height an index move of setup `si` turns A at, with the tool
    /// (`parts`, as `post::tool_parts` gives them; none: a point) over X
    /// span `over` of the setup's frame (`cut_span`: where the operation
    /// keeps the tool; None: anywhere along A) and `clearance` (mm, the
    /// operation's): each part of the tool, its radius and the setup's gap
    /// to either side, clears by `clearance` all that A swings under it at
    /// any angle: the stock it starts from, the part, the chuck's body and
    /// jaws and the tailstock's centre, each where it is along X. Over the
    /// bar, away from the chuck, that is the bar; over the jaws, the jaws.
    /// The export check, the run and the generated index moves all take
    /// this height. Refused when it is above the machine's Z travel: it is
    /// never cut short to fit.
    pub fn index_height(
        &self,
        si: usize,
        mesh: &Mesh,
        clearance: f32,
        over: Option<[f32; 2]>,
        parts: &[crate::holding::Part],
    ) -> Result<Swing, String> {
        let swing = self.index_height_unchecked(si, mesh, clearance, over, parts)?;
        // Machine Z is zero at the top of its travel and negative down.
        if swing.machine_z > 0. {
            let clamped = &self.setups[self.index_base(si).unwrap_or(si)];
            let unit = self.rotary_unit(&clamped.machine).ok_or(NO_GEOMETRY)?;
            let travel = crate::post_check::machine_mm(&clamped.machine, "z");
            let (place, fix) = match swing.over {
                Some([x0, x1]) => (
                    format!(
                        " with the {} over X {} to {} of the setup",
                        swing.part,
                        mm(x0),
                        mm(x1)
                    ),
                    if swing.what == JAWS {
                        "keep the cuts farther from the jaws (more stick-out), "
                    } else {
                        ""
                    },
                ),
                None => (String::new(), ""),
            };
            return Err(format!(
                "Index moves turn A with the tool above all that A swings under it: at machine Z {}{place} (the A centreline at Z {} + {} mm, as far as {} from it at any angle, + {} mm clearance). That is {} mm above the top of the machine's {} mm Z travel (Z 0), so it cannot be reached and is not cut short. Measure the A centreline (Rotary A on the Control page) or reach less far from it: {fix}a smaller stock or chuck, or less clearance.",
                mm(swing.machine_z),
                mm(unit.centre_z),
                mm(swing.radius),
                swing.what,
                mm(swing.clearance),
                mm(swing.machine_z),
                mm(travel),
            ));
        }
        Ok(swing)
    }
    /// `index_height`, though it is above the machine's Z travel.
    pub fn index_height_unchecked(
        &self,
        si: usize,
        mesh: &Mesh,
        clearance: f32,
        over: Option<[f32; 2]>,
        parts: &[crate::holding::Part],
    ) -> Result<Swing, String> {
        let base = self.index_base(si).unwrap_or(si);
        let clamped = self.setups.get(base).ok_or("There is no such setup.")?;
        let stock = self.setup_stock(base, Some(mesh)).ok_or(STOCK_UNKNOWN)?;
        let unit = self.rotary_unit(&clamped.machine).ok_or(NO_GEOMETRY)?;
        let axis = axis_of(stock);
        let [_, dy, dz] = stock.dimensions();
        // What turns with A: what reaches, how far, and from X to X.
        let along = [stock.min[0], stock.max[0]];
        let mut reach = vec![(STOCK, dy.hypot(dz) / 2., along)];
        if crate::chuck::holds(clamped) {
            let rig = self.chuck_rig(clamped, stock)?;
            reach[0].1 = rig.outer;
            for (label, prism, r) in rig.solids(0.) {
                let what = match label.as_str() {
                    "chuck body" => "the chuck's body reaches",
                    l if l.starts_with("chuck jaw") => JAWS,
                    l if l.starts_with("tailstock") => "the tailstock's centre reaches",
                    _ => continue,
                };
                let (Some(r), b) = (r, prism.bounds()) else {
                    continue;
                };
                reach.push((what, r, [b.min[0], b.max[0]]));
            }
        }
        let placed = crate::stock::Placement::new(mesh, Some(clamped));
        let scale = mesh.display_scale();
        reach.extend(mesh.faces.iter().flat_map(|f| &f.points).map(|p| {
            let q = placed.transform(*p).map(|v| v / scale);
            (PART, (q[1] - axis[0]).hypot(q[2] - axis[1]), [q[0], q[0]])
        }));
        let gap = match over {
            Some(_) => crate::holding::gap(clamped)?,
            None => 0.,
        };
        let point = [("tool", 0., 0., f32::INFINITY)];
        let parts = if parts.is_empty() { &point[..] } else { parts };
        // Each part needs the tip that far over the axis: what reaches
        // farthest under it, less how far over the tip it starts.
        let mut need = (STOCK, "tool", f32::NEG_INFINITY);
        for &(part, r, low, _) in parts {
            let under = over.map(|[x0, x1]| [x0 - r - gap, x1 + r + gap]);
            for &(what, reach, [x0, x1]) in &reach {
                if under.is_some_and(|[lo, hi]| x1 < lo || x0 > hi) {
                    continue;
                }
                if reach - low > need.2 {
                    need = (what, part, reach - low);
                }
            }
        }
        let (what, part, radius) = need;
        let radius = radius.max(0.);
        Ok(Swing {
            radius,
            what,
            part,
            over,
            clearance,
            z: axis[1] + radius + clearance,
            machine_z: unit.centre_z + radius + clearance,
        })
    }
    /// Setup `si` at A angle `a` (degrees): its clamping's frame (its base's
    /// when it is indexed, else its own) turned about the A centreline by
    /// `a`, and its work zero, which `a` does not move. Every operation of
    /// an indexed setup is run this way at each of its angles.
    pub fn at_angle(&self, si: usize, a: f32, mesh: &Mesh) -> Result<AtAngle, String> {
        let setup = self.setups.get(si).ok_or("There is no such setup.")?;
        let base = self.index_base(si).unwrap_or(si);
        let clamped = &self.setups[base];
        if !clamped.machine.has_a() {
            return Err(NO_A.into());
        }
        let stock = self.setup_stock(base, Some(mesh)).ok_or(
            "The stock the setup starts from is not known: check the first setup's stock.",
        )?;
        let axis = axis_of(stock);
        let frame = crate::volume::Frame::new(clamped, mesh).turned(axis, sign(&setup.machine) * a);
        let zero = crate::post::datum(clamped, stock, mesh)?;
        let unit = self.rotary_unit(&clamped.machine);
        let face = crate::chuck::holds(clamped)
            .then(|| crate::chuck::rig(clamped, stock, false).ok())
            .flatten()
            .map(|r| r.face);
        let machine_zero = unit.zip(face).map(|(u, face)| {
            [
                u.chuck_x + zero[0] - face,
                u.centre_y + zero[1] - axis[0],
                u.centre_z + zero[2] - axis[1],
            ]
        });
        Ok(AtAngle {
            a,
            frame,
            zero,
            machine_zero,
            measured: self.centre_mark(&clamped.machine) == crate::control::MEASURED,
        })
    }
    /// The rotary unit of machine `m`: its A centreline as the Control page
    /// measured it, in the machine library, while it is marked measured;
    /// else the setup's copy's.
    pub fn rotary_unit(&self, m: &Machine) -> Option<crate::model::RotaryUnit> {
        let mut unit = m.rotary_unit()?;
        if self.centre_mark(m) == crate::control::MEASURED
            && let Some(library) = self
                .machines
                .iter()
                .find(|x| x.id == m.id)
                .and_then(Machine::rotary_unit)
        {
            (unit.centre_y, unit.centre_z, unit.reference) =
                (library.centre_y, library.centre_z, library.reference);
        }
        Some(unit)
    }
    /// How the A centreline of machine `m` was found (`control::CENTRE_MARK`),
    /// from the machine library, which the Control page keeps it in, else
    /// from the setup's copy of the machine.
    pub fn centre_mark<'a>(&'a self, m: &'a Machine) -> &'a str {
        let library = self.machines.iter().find(|x| x.id == m.id).unwrap_or(m);
        get(&library.p, crate::control::CENTRE_MARK).trim()
    }
    /// What is wrong with setup `si` being indexed that the setup alone
    /// does not tell (`Setup::issues` tells the rest).
    pub fn index_issues(&self, si: usize) -> Vec<String> {
        match self.setups.get(si) {
            Some(s) if is_indexed(s) && self.index_base(si).is_none() => vec![
                "The first setup cannot index A: it is the clamping later setups turn. Choose Initial placement.".into(),
            ],
            _ => vec![],
        }
    }
    /// What is not ready about indexing setup `si`, said beside it: the A
    /// centreline it turns about assumed, gone stale, or not recorded.
    pub fn index_warnings(&self, si: usize) -> Vec<String> {
        let Some(s) = self
            .setups
            .get(si)
            .filter(|s| is_indexed(s) && s.machine.has_a())
        else {
            return vec![];
        };
        if s.machine.rotary_unit().is_none() {
            return vec![
                "The machine's rotary unit geometry is not recorded: edit the machine to fill it in, then probe its A centreline on the Control page (Rotary A).".into(),
            ];
        }
        match self.centre_mark(&s.machine) {
            crate::control::MEASURED => vec![],
            "" => vec![
                "The A centreline is assumed, not measured: probe it on the Control page (Rotary A) before trusting where A turns the stock.".into(),
            ],
            why => vec![format!(
                "The measured A centreline went {why}: probe it again on the Control page (Rotary A) before trusting where A turns the stock."
            )],
        }
    }
    /// The Reposition line of setup `si` for the setup sheet.
    pub fn reposition_text(&self, si: usize) -> String {
        let s = &self.setups[si];
        match self.index_base(si) {
            Some(base) => format!(
                "{ACTION}: {} in the clamping of setup {} ({}), work zero kept",
                label(s).unwrap_or_default(),
                base + 1,
                self.setups[base].name()
            ),
            None => get(&s.p, "reposition").to_string(),
        }
    }
}
/// Why a setup cannot be indexed on its machine.
pub const NO_A: &str = "Index rotary axis turns the stock about A: the setup's machine has no rotary A axis around X. Choose one, or Reclamp and probe origin.";
/// What is wrong with `setup`'s A angles on its machine, when it is indexed.
pub fn issues(setup: &Setup) -> Vec<String> {
    if !is_indexed(setup) {
        return vec![];
    }
    if !setup.machine.has_a() {
        return vec![NO_A.into()];
    }
    let v = match angles(setup) {
        Ok(v) => v,
        Err(e) => return vec![e],
    };
    let m = &setup.machine.p;
    let (lo, hi) = (num(m, "rotary_min"), num(m, "rotary_max"));
    v.iter()
        .filter(|a| lo.is_some_and(|lo| **a < lo) || hi.is_some_and(|hi| **a > hi))
        .map(|a| {
            format!(
                "A {}° is outside the machine's A range, {}° to {}°.",
                degrees(*a),
                get(m, "rotary_min"),
                get(m, "rotary_max")
            )
        })
        .collect()
}

/// What the ribbon's Indexed tile says now: indexing is a setup's, not an
/// operation's.
pub const POINTER: &str = "Indexed work is a setup now: add a setup, choose Reposition → Index rotary axis and list its A angles. Every operation in it runs at each angle.";
/// The Indexed strategy's availability, in the words of the others'.
pub const AVAILABILITY: &str = "Unavailable · indexed work is a setup now: add a setup, choose Reposition → Index rotary axis and list its A angles, and every operation in it runs at each angle";
/// Why an Indexed operation of an older project that could not be moved
/// into an indexed setup (`Document::migrate_indexed_operations`) is not
/// ready.
pub const LEGACY: &str = "This Indexed operation is from an older project and was not moved into an indexed setup: it was disabled at an angle, its machine has no A axis around X, or it sets a B angle or an A angle that is not a number. Indexing is a setup now: add a setup on a machine with A, choose Reposition → Index rotary axis at the angle, and add a Facing operation there.";
impl Document {
    /// Moves each Indexed operation of an older project (an Indexed face,
    /// which only recorded the A angle `angle_a`) into the setups that index
    /// now: it becomes the Facing operation it was, in a new indexed setup at
    /// its angle right after its own, in that setup's clamping; the
    /// operations after it follow in an indexed setup at A 0°, so that
    /// everything still runs in its order. At A 0°, or in a setup that is
    /// indexed already, it stays where it is. One that cannot be moved (no A
    /// axis around X, a B angle, an angle that is not a number) stays an
    /// Indexed operation, which `LEGACY` refuses. Returns a note for each.
    pub fn migrate_indexed_operations(&mut self) -> Vec<String> {
        use crate::model::Strategy;
        let mut notes = vec![];
        let mut out = vec![];
        let mut added = false;
        for mut setup in std::mem::take(&mut self.setups) {
            if !setup
                .operations
                .iter()
                .any(|o| o.strategy == Strategy::Indexed)
            {
                out.push(setup);
                continue;
            }
            let own = is_indexed(&setup);
            // The operations by the A angle they now run at: the setup's own
            // first, then each indexed setup's.
            let mut groups: Vec<(Option<f32>, Vec<crate::model::Operation>)> = vec![(None, vec![])];
            for mut op in std::mem::take(&mut setup.operations) {
                let mut at = None;
                if op.strategy == Strategy::Indexed {
                    let a = match get(&op.p, "angle_a").trim() {
                        "" => Some(0.),
                        _ => num(&op.p, "angle_a"),
                    };
                    let b = num(&op.p, "angle_b").unwrap_or(0.);
                    // A disabled one at an angle would split the setup for
                    // nothing it cuts: it stays, as it did.
                    let moved = |a: &f32| {
                        setup.machine.has_a() && b.abs() < 1e-3 && (op.enabled || a.abs() < 1e-3)
                    };
                    match a.filter(moved) {
                        Some(a) => {
                            op.strategy = Strategy::Face;
                            op.p.remove("angle_a");
                            op.p.remove("angle_b");
                            at = (!own && a.abs() > 1e-3).then_some(a);
                            notes.push(match at {
                                Some(a) => format!(
                                    "{}: the Indexed operation at A {}° is a Facing operation in a new indexed setup at that angle, after {}.",
                                    op.name(),
                                    degrees(a),
                                    setup.name()
                                ),
                                None if own => format!(
                                    "{}: the Indexed operation is a Facing operation in {}, at that setup's A angles.",
                                    op.name(),
                                    setup.name()
                                ),
                                None => format!(
                                    "{}: the Indexed operation at A 0° is a Facing operation in {}.",
                                    op.name(),
                                    setup.name()
                                ),
                            });
                        }
                        None => notes.push(format!("{}: {LEGACY}", op.name())),
                    }
                }
                // After an indexed group, the setup's own operations run at A 0°.
                let at = at.or((groups.len() > 1).then_some(0.));
                match groups.last_mut() {
                    Some((key, ops)) if *key == at => ops.push(op),
                    _ => groups.push((at, vec![op])),
                }
            }
            let mut groups = groups.into_iter();
            let (_, first) = groups.next().unwrap_or_default();
            setup.operations = first;
            let template = setup.clone();
            out.push(setup);
            for (a, ops) in groups {
                let a = a.unwrap_or(0.);
                let mut s = template.clone();
                s.id = crate::model::id();
                s.turn = None;
                s.p.insert(
                    "name".into(),
                    format!("{} · A {}°", template.name(), degrees(a)),
                );
                s.p.insert("reposition".into(), ACTION.into());
                s.p.insert(ANGLES.into(), degrees(a));
                s.p.remove("notes");
                s.operations = ops;
                out.push(s);
                added = true;
            }
        }
        self.setups = out;
        if added {
            self.sync_stock_sources();
        }
        notes
    }
}
/// `v` mm for a message, to the hundredth, without trailing zeros.
fn mm(v: f32) -> String {
    let s = format!("{v:.2}");
    match s.trim_end_matches('0').trim_end_matches('.') {
        "-0" => "0".into(),
        t => t.into(),
    }
}
/// What reaches from the A centreline, as the index height says it.
const STOCK: &str = "the stock reaches";
const PART: &str = "the part reaches";
const JAWS: &str = "the chuck's jaws reach";
/// The X span (the setup's frame) the tool tip is kept in from the first
/// feed of each of `plans` on: where an operation's index moves turn A
/// (`Document::index_height`). The moves before the first feed, the
/// kernels' lead-in or an index move's entry, are over the cuts or left
/// out. None without a feed.
pub fn cut_span<'a>(
    plans: impl IntoIterator<Item = &'a [crate::simulation::Move]>,
) -> Option<[f32; 2]> {
    use crate::simulation::Motion;
    let mut span: Option<[f32; 2]> = None;
    for moves in plans {
        let Some(first) = moves.iter().position(|m| m.motion == Motion::Feed) else {
            continue;
        };
        for x in moves[first..].iter().flat_map(|m| [m.from[0], m.to[0]]) {
            let [lo, hi] = span.unwrap_or([x, x]);
            span = Some([lo.min(x), hi.max(x)]);
        }
    }
    span
}
/// Why an index move cannot be planned without the base's stock.
const STOCK_UNKNOWN: &str =
    "The stock the setup starts from is not known: check the first setup's stock.";
const NOT_INDEXED: &str = "The setup does not index A.";
/// Why an index move cannot be checked without the rotary unit's geometry.
pub const NO_GEOMETRY: &str = "The machine's rotary unit geometry is not recorded, so the index moves' retract above what A swings cannot be checked against the Z travel: edit the machine to fill it in.";
/// The rate A turns at in an index move, degrees per minute, for the time
/// it takes: an assumed 60°/s.
pub const INDEX_RATE: f32 = 3600.;
/// The retract of an indexed setup's index moves (`Document::swing`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Swing {
    /// How far over the A centreline the tool tip must be for what A swings
    /// to clear the tool, at any angle: what reaches farthest under it,
    /// less how far the part of the tool over it (`part`) is over the tip;
    /// what reaches that far, the X span of the setup the tool is over
    /// (None: anywhere along A), and the clearance kept above it, mm.
    pub radius: f32,
    pub what: &'static str,
    pub part: &'static str,
    pub over: Option<[f32; 2]>,
    pub clearance: f32,
    /// The tool tip's height while A turns: in the setup's frame, and in
    /// machine coordinates (zero at the top of the Z travel).
    pub z: f32,
    pub machine_z: f32,
}
/// An indexed setup's operation, generated at each of its A angles.
pub struct Posed {
    /// The moves at every angle, in the order of the angles, each carrying
    /// its A, joined by index moves.
    pub plan: crate::simulation::Plan,
    /// The frame the part is posed in at each angle (`AtAngle::frame`).
    pub turns: Vec<(f32, crate::volume::Frame)>,
}
/// What `generate` starts from: the stock left when the setup began and
/// now (None when it is not known), the A the machine is at, and the
/// source the operation's cuts are stamped with.
pub struct Start<'a> {
    pub begin: Option<&'a crate::volume::Volume>,
    pub now: Option<&'a crate::volume::Volume>,
    pub a: f32,
    pub source: (u16, u16),
}
/// Generates `op` of indexed setup `si` with `tool`, as an indexed setup's
/// operations all are: once at each of its A angles, in order. Indexing is
/// a frame change, not other toolpaths: at each angle the 3-axis kernel
/// runs on `setup` (the copy a simulation runs, fresh stock) posed as A
/// turns the part and its stock about the centreline (`Document::setup_at`,
/// whose frame is `Document::at_angle`'s), the stock cut at the angles
/// before. Its moves stay in the setup's frame, which keeps the machine's
/// axes, so they are machine X Y Z less the work offset, which A does not
/// move; each carries its A. Between two angles, and from the A the
/// machine is at to the first, the tool retracts above all A swings
/// (`Document::swing`), A turns, and the tool comes over the next first cut
/// and down to it. The operation enters that way even when A is already at
/// its first angle (LIO-205): the kernels' own lead-in starts at the stock
/// box's -X -Y corner at their clearance plane, which on a bar in the chuck
/// is over the jaws and below what they reach. The chuck is checked along
/// each of those moves and the whole turn (`chuck::Rig::hits_turning`).
pub fn generate(
    doc: &Document,
    si: usize,
    setup: &Setup,
    op: &crate::model::Operation,
    tool: &crate::model::Tool,
    mesh: &Mesh,
    start: Start,
) -> Result<Posed, String> {
    use crate::simulation::Simulation;
    if op.boring() {
        return Err(crate::model::BORE_IN_CHUCK.into());
    }
    // An operation that picks holes at several angles cuts at each only the
    // holes it turns up, and skips an angle that turns none up.
    let by_angle = holes_at_angles(op, setup)
        .then(|| holes_by_angle(doc, si, op, mesh))
        .transpose()?
        .map(|(cuts, _)| cuts);
    let list: Vec<f32> = match &by_angle {
        Some(cuts) => (cuts.iter())
            .filter(|c| !c.ids.is_empty())
            .map(|c| c.a)
            .collect(),
        None => angles(setup)?,
    };
    if list.is_empty() {
        return Err(NO_HOLE_UP.into());
    }
    let begin = start.begin.ok_or(STOCK_UNKNOWN)?;
    let mut now = start.now.cloned();
    let mut parts = vec![];
    let mut turns = vec![];
    for (k, &a) in list.iter().enumerate() {
        let at = doc.at_angle(si, a, mesh)?;
        let mut posed = doc.setup_at(setup, si, a, mesh)?;
        posed.preview_stock = Some(crate::simulation::leftover_box(begin, &posed, mesh));
        posed.preview_remaining = now
            .as_ref()
            .filter(|v| !v.cuts.is_empty())
            .map(|v| std::sync::Arc::new(crate::rest::RestStock::new(v, at.frame)));
        let cut = by_angle.as_ref().and_then(|c| c.iter().find(|c| c.a == a));
        let here = match cut {
            Some(c) => std::borrow::Cow::Owned(c.op(op)),
            None => on_top(op, &posed, mesh),
        };
        let mut plan = crate::simulation::generate(&posed, &here, tool, mesh).map_err(|e| {
            // A plunge across a cavity too deep for the drill: each hole
            // from its own side instead.
            let across = cut.is_some_and(|c| c.across) && e.starts_with(crate::cam::TOO_DEEP);
            match across {
                true => format!(
                    "At A {}°: {e} {ACROSS} (A {}°).",
                    degrees(a),
                    degrees((a + 180.).rem_euclid(360.))
                ),
                false => format!("At A {}°: {e}", degrees(a)),
            }
        })?;
        for m in &mut plan.moves {
            m.a = Some(a);
        }
        // The next angle starts from the stock this one leaves.
        if k + 1 < list.len()
            && let Some(v) = now.take()
        {
            let mut sim = Simulation::new(plan.clone());
            (sim.volume, sim.frame, sim.source) = (Some(v), Some(at.frame), start.source);
            sim.seek(1.);
            now = sim.volume;
        }
        parts.push((a, plan));
        turns.push((a, at.frame));
    }
    let clearance = crate::cam::number(op, "clearance")?;
    let bi = doc.index_base(si).ok_or(NOT_INDEXED)?;
    let base = &doc.setups[bi];
    // The height over what A swings under the tool where this operation
    // keeps it, its holder and the spindle nose included, as the export
    // check and the run take it.
    let (parts_of, _) = crate::post::tool_parts(doc, setup, &tool.id, &parts[0].1);
    let over = cut_span(parts.iter().map(|(_, p)| &p.moves[..]));
    let swing = if list.len() > 1 || (list[0] - start.a).abs() > 1e-4 {
        doc.index_height(si, mesh, clearance, over, &parts_of)?
    } else {
        doc.index_height_unchecked(si, mesh, clearance, over, &parts_of)?
    };
    let rig = if crate::chuck::holds(base) {
        let stock = doc.setup_stock(bi, Some(mesh)).ok_or(STOCK_UNKNOWN)?;
        Some((doc.chuck_rig(base, stock)?, crate::holding::gap(base)?))
    } else {
        None
    };
    // Where A does not turn, the entry comes down from the top of the Z
    // travel when the swing is above it. That is not checked against the
    // chuck here: the program's first turn of A, below the swing, refuses
    // it on export and on a run (`post::index_problems`), and the rotary
    // checks say why.
    let top = swing.z - swing.machine_z.max(0.);
    let rig = rig.filter(|_| swing.machine_z <= 0.);
    let mut plan = parts[0].1.clone();
    plan.moves.clear();
    plan.keep_air.clear();
    plan.duration = 0.;
    let (mut a_now, mut at): (f32, Option<[f32; 3]>) = (start.a, None);
    for (k, (a, part)) in parts.iter().enumerate() {
        let mut moves = &part.moves[..];
        let turning = (a - a_now).abs() > 1e-4;
        if k == 0 || turning {
            // The plan's own approach is replaced: over its first cut at the
            // swing height, straight down to where that cut starts.
            let entry = Entry::new(moves, at, top);
            let (from, up, over, to) = (entry.from, entry.up, entry.over, entry.to);
            if let Some((rig, gap)) = &rig {
                let tool = (part.radius, part.tool_length, part.holder_radius);
                let hits = [
                    rig.hits_at(a_now, &[(from, up)], tool, *gap),
                    rig.hits_turning(a_now, *a, &[(up, up)], tool, *gap),
                    rig.hits_at(*a, &[(up, over), (over, to)], tool, *gap),
                ];
                if let Some(hit) = hits.iter().flatten().next() {
                    let what = if turning {
                        format!(
                            "index move from A {}° to A {}°",
                            degrees(a_now),
                            degrees(*a)
                        )
                    } else {
                        format!("approach at A {}°", degrees(*a))
                    };
                    return Err(format!(
                        "The {what} brings the {} within the {} mm gap of the {} at X {} Y {} Z {}.",
                        hit.part,
                        mm(*gap),
                        hit.label,
                        mm(hit.at[0]),
                        mm(hit.at[1]),
                        mm(hit.at[2]),
                    ));
                }
            }
            entry.push(&mut plan, Some(a_now), Some(*a));
            moves = &moves[entry.first..];
        }
        let first = part.moves.len() - moves.len();
        for (i, m) in moves.iter().enumerate() {
            push(&mut plan, m.clone());
            *plan.keep_air.last_mut().unwrap() =
                part.keep_air.get(first + i).copied().unwrap_or(false);
        }
        at = plan.moves.last().map(|m| m.to);
        a_now = *a;
    }
    Ok(Posed { plan, turns })
}
/// `plan`, made by the 3-axis kernel for setup `si`, which is not indexed:
/// held in the chuck, it enters as an indexed operation does (LIO-206). The
/// kernels' own lead-in starts at the stock box's -X -Y corner at their
/// clearance plane, which on a bar is over the jaws and below what they
/// reach; it is replaced by a rapid over the first cut from the swing
/// height of the setup's own clamping (`Document::swing`), or from the top
/// of the Z travel when that is higher, straight down to where the cut
/// starts. The moves keep no A. The workholding checks check it with the
/// rest. Another setup's plan, or one whose swing cannot be told (the
/// setup's issues say why), is as made.
pub fn enter_held(
    doc: &Document,
    si: usize,
    op: &crate::model::Operation,
    mesh: &Mesh,
    plan: crate::simulation::Plan,
) -> Result<crate::simulation::Plan, String> {
    let setup = &doc.setups[si];
    if !crate::chuck::holds(setup) || plan.moves.is_empty() {
        return Ok(plan);
    }
    if !plan.stops.is_empty() {
        return Err(crate::model::BORE_IN_CHUCK.into());
    }
    let Ok(swing) = doc.swing_unchecked(si, mesh, crate::cam::number(op, "clearance")?) else {
        return Ok(plan);
    };
    let entry = Entry::new(&plan.moves, None, swing.z - swing.machine_z.max(0.));
    let mut entered = plan.clone();
    entered.moves.clear();
    entered.keep_air.clear();
    entered.duration = 0.;
    entry.push(&mut entered, None, None);
    for (i, m) in plan.moves[entry.first..].iter().enumerate() {
        push(&mut entered, m.clone());
        *entered.keep_air.last_mut().unwrap() =
            plan.keep_air.get(entry.first + i).copied().unwrap_or(false);
    }
    Ok(entered)
}
/// How an operation enters from above what its clamping swings: from
/// `from` (over the first cut when the tool is not yet anywhere) straight
/// up to the height `top`, across there over the first cut, and straight
/// down to where it starts (`to`, the start of the first feed, `first`).
struct Entry {
    first: usize,
    from: [f32; 3],
    up: [f32; 3],
    over: [f32; 3],
    to: [f32; 3],
}
impl Entry {
    fn new(moves: &[crate::simulation::Move], at: Option<[f32; 3]>, top: f32) -> Self {
        use crate::simulation::Motion;
        let first = moves
            .iter()
            .position(|m| m.motion == Motion::Feed)
            .unwrap_or(0);
        let to = moves[first].from;
        // Before the first angle the tool waits over its first cut.
        let from = at.unwrap_or([to[0], to[1], top]);
        Self {
            first,
            from,
            up: [from[0], from[1], top],
            over: [to[0], to[1], top],
            to,
        }
    }
    /// Its rapids, onto `plan`: at A `a_now` up, A turned to `a` at the top
    /// when they differ (`Motion::Index`), at `a` across and down. None:
    /// moves of a setup that is not indexed, which keep no A.
    fn push(&self, plan: &mut crate::simulation::Plan, a_now: Option<f32>, a: Option<f32>) {
        use crate::simulation::{Motion, Move};
        rapid(plan, self.from, self.up, a_now);
        if let (Some(from), Some(to)) = (a_now, a)
            && (to - from).abs() > 1e-4
        {
            push(
                plan,
                Move {
                    from: self.up,
                    to: self.up,
                    motion: Motion::Index,
                    cutting_pass: false,
                    feed: INDEX_RATE,
                    start: 0.,
                    end: (to - from).abs() / INDEX_RATE * 60.,
                    a,
                },
            );
        }
        rapid(plan, self.up, self.over, a);
        rapid(plan, self.over, self.to, a);
    }
}
/// `m` onto the end of `plan`, timed from where it ends.
fn push(plan: &mut crate::simulation::Plan, m: crate::simulation::Move) {
    plan.keep_air.push(false);
    let seconds = m.end - m.start;
    plan.moves.push(crate::simulation::Move {
        start: plan.duration,
        end: plan.duration + seconds,
        ..m
    });
    plan.duration += seconds;
}
/// A rapid at the rate the 3-axis kernels rapid at, none when it does not move.
fn rapid(plan: &mut crate::simulation::Plan, from: [f32; 3], to: [f32; 3], a: Option<f32>) {
    let d = (0..3)
        .map(|k| (to[k] - from[k]).powi(2))
        .sum::<f32>()
        .sqrt();
    if d > 1e-5 {
        push(
            plan,
            crate::simulation::Move {
                from,
                to,
                motion: crate::simulation::Motion::Rapid,
                cutting_pass: false,
                feed: RAPID,
                start: 0.,
                end: d / RAPID * 60.,
                a,
            },
        );
    }
}
/// The rapid rate of the index moves, mm/min: the 3-axis kernels'.
const RAPID: f32 = 6000.;
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{Document, Machine, Setup};

    /// A machine with A and its rotary geometry.
    pub(crate) fn rotary() -> Machine {
        let mut m = Machine::new(4);
        m.fill_rotary_defaults();
        m
    }
    /// A job on a Ø40 × 120 bar in the chuck, A axis at stock end, and a
    /// second setup indexed to `angles`.
    pub(crate) fn job(angles: &str) -> (Document, Mesh) {
        let mesh = crate::mesh::sample();
        let mut doc = Document::sample();
        doc.file = None;
        let m = rotary();
        let mut first = Setup::new(m.clone(), "G54");
        for (k, v) in [
            ("stock_type", "Round bar"),
            ("stock_diameter", "40"),
            ("stock_length", "120"),
            ("holding", "Chuck"),
            ("origin", ORIGINS[0]),
        ] {
            first.p.insert(k.into(), v.into());
        }
        for (k, v) in crate::chuck::DEFAULTS {
            first.p.insert(k.into(), v.into());
        }
        let mut second = Setup::new(m, "G55");
        second.p.insert("name".into(), "Flats".into());
        second.p.insert("reposition".into(), ACTION.into());
        second.p.insert(ANGLES.into(), angles.into());
        doc.setups = vec![first, second];
        doc.sync_stock_sources();
        doc.sync_turns(Some(&mesh));
        (doc, mesh)
    }
    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-3)
    }

    #[test]
    fn positive_a_turns_the_part_by_the_right_hand_rule_about_x() {
        // +Y goes up to +Z at A90 and +Z to -Y: seen from the chuck looking
        // along +X (screen right is -Y, up is +Z), +Y on the left goes up,
        // clockwise. The driver's bar and the chuck's jaws turn this way.
        assert!(close(turn_point([5., 1., 0.], [0., 0.], 90.), [5., 0., 1.]));
        assert!(close(
            turn_point([5., 0., 1.], [0., 0.], 90.),
            [5., -1., 0.]
        ));
        // About the centreline, which stays put.
        assert!(close(
            turn_point([0., 12., 20.], [2., 20.], 90.),
            [0., 2., 30.]
        ));
        assert!(close(
            turn_point([0., 2., 20.], [2., 20.], 37.),
            [0., 2., 20.]
        ));
        // The same sense as the chuck's: jaw 1 up at A0 is towards -Y at A90.
        assert!(close(
            turn_point([0., 0., 1.], [0., 0.], 90.),
            [0., -1., 0.]
        ));
        let mut m = rotary();
        assert_eq!(sign(&m), 1.);
        m.p.insert("direction".into(), "Negative".into());
        assert_eq!(sign(&m), -1.);
    }

    #[test]
    fn turned_angles_turn_as_the_placement_then_a_does() {
        for angles in [
            [0., 0., 0.],
            [180., 0., 0.],
            [0., 90., 0.],
            [12., -33., 71.],
            [0., -90., 25.],
        ] {
            for turn in [0., 90., -90., 180., 37.5] {
                let e = turned_angles(angles, turn);
                for p in [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [3., -2., 5.]] {
                    let want = turn_point(rotation(p, angles), [0., 0.], turn);
                    assert!(close(rotation(p, e), want), "{angles:?} {turn}: {p:?}");
                }
            }
        }
    }

    #[test]
    fn angles_are_listed_in_degrees() {
        let mut s = Setup::new(rotary(), "G54");
        s.p.insert("reposition".into(), ACTION.into());
        for (text, want) in [
            ("90", vec![90.]),
            ("0, 90, 180, 270", vec![0., 90., 180., 270.]),
            ("0/90/180/270", vec![0., 90., 180., 270.]),
            (" -45° 45 ", vec![-45., 45.]),
        ] {
            s.p.insert(ANGLES.into(), text.into());
            assert_eq!(angles(&s).unwrap(), want, "{text}");
        }
        for bad in ["", "x", "90, 90", "nan"] {
            s.p.insert(ANGLES.into(), bad.into());
            assert!(angles(&s).is_err(), "{bad}");
        }
        s.p.insert(ANGLES.into(), "0, 90, 180, 270".into());
        assert_eq!(label(&s).unwrap(), "A 0°, 90°, 180°, 270°");
        // Outside the machine's range of ±180°.
        assert!(issues(&s)[0].contains("A 270° is outside the machine's A range, -180° to 180°."));
        s.p.insert(ANGLES.into(), "90".into());
        assert!(issues(&s).is_empty());
        // Not on a machine without A.
        s.machine = Machine::new(3);
        assert_eq!(issues(&s), [NO_A]);
        assert_eq!(label(&Setup::new(Machine::new(3), "G54")), None);
        assert_eq!(label(&Setup::new(rotary(), "G54")).unwrap(), "A 0°");
    }

    #[test]
    fn an_indexed_setup_turns_setup_1s_frame_about_the_centreline_and_keeps_work_zero() {
        let (doc, mesh) = job("90, 180");
        let (first, second) = (&doc.setups[0], &doc.setups[1]);
        assert!(is_indexed(second) && !is_indexed(first));
        assert_eq!(doc.index_base(1), Some(0));
        // In setup 1's clamping: its work zero, offset and chuck, no probing.
        for k in ["origin", "offset", "holding", "chuck_stickout", "rx", "tz"] {
            assert_eq!(get(&second.p, k), get(&first.p, k), "{k}");
        }
        assert_eq!(get(&second.p, "name"), "Flats");
        // The bar's axis: Y0, Z20 (Ø40 standing on Z0).
        let turn = second.turn.unwrap();
        assert_eq!(
            (turn.base, turn.a, turn.turn, turn.axis),
            (0, 90., 90., [0., 20.])
        );
        let at0 = doc.at_angle(1, 0., &mesh).unwrap();
        let at90 = doc.at_angle(1, 90., &mesh).unwrap();
        // Work zero on the axis at the bar's free end, the same at each angle.
        assert!(close(at0.zero, [60., 0., 20.]) && at90.zero == at0.zero);
        // Every point of the part turns +Y towards +Z about the axis.
        let base = crate::volume::Frame::new(first, &mesh);
        for p in [[1., 2., 3.], [-20., 15., -4.], mesh.origin] {
            assert!(close(at0.frame.map(p), base.map(p)));
            assert!(close(
                at90.frame.map(p),
                turn_point(base.map(p), [0., 20.], 90.)
            ));
            let (w0, w90) = (at0.work(p), at90.work(p));
            assert!(close(w90, [w0[0], -w0[2], w0[1]]), "{w0:?} {w90:?}");
        }
        // The setup's own frame is its first angle's: the 3D view and the
        // stock it inherits.
        let own = crate::volume::Frame::new(second, &mesh);
        for p in [[1., 2., 3.], [-20., 15., -4.]] {
            assert!(close(own.map(p), at90.frame.map(p)));
        }
        let placed = crate::stock::Placement::new(&mesh, Some(second));
        let s = mesh.display_scale();
        let p = mesh.faces[0].points[0];
        let q = own.map(p.map(|v| v / s));
        assert!(close(placed.transform(p).map(|v| v / s), q));
        // Its work zero is setup 1's, though its stock is turned.
        let stock = doc.setup_stock(1, Some(&mesh)).unwrap();
        assert!(close(
            crate::post::datum(second, stock, &mesh).unwrap(),
            at0.zero
        ));
        // On the machine: the work zero on the rotary unit's centreline;
        // the bar's free end 40 mm out of the jaws, 25 mm long, from the face.
        let m = at90.machine_zero.unwrap();
        assert!(close(m, [20. + 40. + 25., 140., -40.]), "{m:?}");
        assert!(close(at90.machine([0.; 3]).unwrap(), {
            let w = at90.work([0.; 3]);
            [w[0] + m[0], w[1] + m[1], w[2] + m[2]]
        }));
        // A machine whose A turns the other way turns the part back.
        let (mut doc, mesh) = job("90");
        for s in &mut doc.setups {
            s.machine.p.insert("direction".into(), "Negative".into());
        }
        let at = doc.at_angle(1, 90., &mesh).unwrap();
        let base = crate::volume::Frame::new(&doc.setups[0], &mesh);
        assert!(close(
            at.frame.map([1., 2., 3.]),
            turn_point(base.map([1., 2., 3.]), [0., 20.], -90.)
        ));
    }

    #[test]
    fn a_re_clamp_after_indexed_setups_keeps_its_own_placement() {
        let (mut doc, mesh) = job("90");
        let mut third = doc.setups[0].clone();
        third.id = "third".into();
        for (k, v) in [
            ("reposition", "Reclamp and probe origin"),
            ("rx", "180"),
            ("origin", "Stock top center"),
        ] {
            third.p.insert(k.into(), v.into());
        }
        let mut fourth = doc.setups[1].clone();
        fourth.id = "fourth".into();
        doc.setups.extend([third, fourth]);
        doc.sync_stock_sources();
        doc.sync_turns(Some(&mesh));
        assert_eq!(doc.index_base(2), None);
        assert_eq!(doc.index_base(3), Some(2));
        assert_eq!(get(&doc.setups[2].p, "rx"), "180");
        assert_eq!(doc.setups[2].turn, None);
        // The fourth turns the re-clamp's frame and takes its work zero.
        assert_eq!(get(&doc.setups[3].p, "rx"), "180");
        assert_eq!(get(&doc.setups[3].p, "origin"), "Stock top center");
        assert_eq!(doc.setups[3].turn.unwrap().base, 2);
        let base = crate::volume::Frame::new(&doc.setups[2], &mesh);
        let at = doc.at_angle(3, 90., &mesh).unwrap();
        let axis = doc.setups[3].turn.unwrap().axis;
        assert!(close(
            at.frame.map([1., 2., 3.]),
            turn_point(base.map([1., 2., 3.]), axis, 90.)
        ));
        // Indexing the first setup is refused.
        doc.setups[0].p.insert("reposition".into(), ACTION.into());
        doc.setups[0].p.insert(ANGLES.into(), "90".into());
        assert!(doc.index_issues(0)[0].contains("first setup cannot index A"));
    }

    #[test]
    fn indexed_setups_warn_while_the_centreline_is_not_measured() {
        let (mut doc, _) = job("90");
        let assumed = doc.index_warnings(1);
        assert!(assumed[0].contains("assumed, not measured"), "{assumed:?}");
        assert!(doc.index_warnings(0).is_empty());
        let id = doc.setups[0].machine.id.clone();
        let mut library = doc.setups[0].machine.clone();
        library.p.insert(
            crate::control::CENTRE_MARK.into(),
            crate::control::MEASURED.into(),
        );
        doc.machines = vec![library];
        assert!(doc.index_warnings(1).is_empty());
        let mesh = crate::mesh::sample();
        doc.sync_turns(Some(&mesh));
        assert!(doc.at_angle(1, 90., &mesh).unwrap().measured);
        doc.machines[0].p.insert(
            crate::control::CENTRE_MARK.into(),
            "stale after an E-stop".into(),
        );
        assert_eq!(doc.machines[0].id, id);
        assert!(doc.index_warnings(1)[0].contains("went stale after an E-stop"));
        assert!(!doc.at_angle(1, 90., &mesh).unwrap().measured);
    }

    #[test]
    fn a_label_only_index_from_an_older_project_keeps_its_own_placement() {
        let (mut doc, mesh) = job("90");
        doc.setups[1].p.remove(ANGLES);
        for (k, v) in [("rx", "90"), ("ty", "5"), ("origin", "Stock top center")] {
            doc.setups[1].p.insert(k.into(), v.into());
        }
        let before = doc.setups[1].p.clone();
        doc.sync_stock_sources();
        doc.sync_turns(Some(&mesh));
        let s = &doc.setups[1];
        assert!(is_label_only(s) && !is_indexed(s));
        assert_eq!(s.p, before);
        assert_eq!(s.turn, None);
        assert_eq!(label(s).unwrap(), "A 0°");
        assert_eq!(doc.reposition_text(1), ACTION);
    }

    /// A square bar `len` mm long, 20 × 20, as the part: the frames' part
    /// coordinates (`volume::Frame::map`) put it about X with its middle at
    /// Y0, Z `centre_z`.
    pub(crate) fn prism(len: f32) -> Mesh {
        block(len, 20., 20.)
    }
    /// A block `l` × `w` (Y) × `h` (Z) as the part.
    pub(crate) fn block(l: f32, w: f32, h: f32) -> Mesh {
        let mut t = vec![];
        let mut quad = |p: [[f32; 3]; 4]| {
            t.push([p[0], p[1], p[2]]);
            t.push([p[0], p[2], p[3]]);
        };
        quad([[0., 0., h], [l, 0., h], [l, w, h], [0., w, h]]);
        quad([[0., 0., 0.], [0., w, 0.], [l, w, 0.], [l, 0., 0.]]);
        quad([[0., 0., 0.], [l, 0., 0.], [l, 0., h], [0., 0., h]]);
        quad([[l, w, 0.], [0., w, 0.], [0., w, h], [l, w, h]]);
        quad([[0., w, 0.], [0., 0., 0.], [0., 0., h], [0., w, h]]);
        quad([[l, 0., 0.], [l, w, 0.], [l, w, h], [l, 0., h]]);
        crate::mesh::normalize(t).unwrap()
    }
    fn centre_z(mesh: &Mesh) -> f32 {
        4. / mesh.display_scale() + mesh.dimensions[2] / 2.
    }
    /// A flat end mill: Ø`d`, 35 mm of flute, 50 mm out of its holder.
    pub(crate) fn cutter(d: &str) -> crate::model::Tool {
        let mut t = crate::model::Tool::new("Flat", "Flat end mill", d);
        t.p.insert("length".into(), "35".into());
        t.p.insert("stickout".into(), "50".into());
        t
    }
    /// The part `mesh` in a Ø40 × 120 bar `stickout` mm out of a 3-jaw
    /// chuck, its square on the bar's axis (Y0 Z20) from X `x0`, on a rotary
    /// unit whose centreline is at machine Z `centre_z`; setup 2 is indexed
    /// to `angles` with the one operation `op` (a Ø12 flat end mill's).
    pub(crate) fn bar_job(
        mesh: &Mesh,
        x0: f32,
        stickout: &str,
        centre_z: &str,
        angles: &str,
        op: crate::model::Operation,
    ) -> Document {
        let (mut doc, _) = job(angles);
        let s = mesh.display_scale();
        let len = mesh.dimensions[0];
        // Setup 2 takes setup 1's machine (`sync_indexed`).
        for (k, v) in [
            ("a_centre_z", centre_z),
            ("rotary_min", "-360"),
            ("rotary_max", "360"),
        ] {
            doc.setups[0].machine.p.insert(k.into(), v.into());
        }
        let p = &mut doc.setups[0].p;
        p.insert("chuck_stickout".into(), stickout.into());
        p.insert("tx".into(), (x0 + len / 2.).to_string());
        p.insert(
            "tz".into(),
            (20. - mesh.dimensions[2] / 2. - 4. / s).to_string(),
        );
        doc.tools = vec![cutter("12")];
        let mut op = op;
        op.tool = doc.tools[0].id.clone();
        doc.setups[1].operations = vec![op];
        doc.sync_stock_sources();
        doc.sync_turns(Some(mesh));
        doc
    }
    /// An outside contour round the part from the stock's top to its bottom.
    pub(crate) fn contour(tool: &crate::model::Tool) -> crate::model::Operation {
        let mut op = crate::model::Operation::new(crate::model::Strategy::Contour, tool.id.clone());
        op.geometry.clear();
        op.p.insert("allowance".into(), "0".into());
        op
    }
    /// `contour` down to 7 mm over the A axis instead, the part's bottom
    /// being 10 mm under it: at angles 180° apart its passes across the
    /// bar's ends leave a 14 mm web on the axis, which holds the part on the
    /// bar in the chuck against the cut (LIO-208, LIO-211: a 4 mm one does
    /// not).
    pub(crate) fn on_the_bar(tool: &crate::model::Tool) -> crate::model::Operation {
        let mut op = contour(tool);
        op.p.insert("bottom".into(), "17".into());
        op
    }
    /// Facing the four sides of `prism`'s square, its ±Y and ±Z faces, each
    /// at the angle that brings it to the top (`on_top`), the tool's centre
    /// out to the face's edges: at A 90, 180, 270 and 0° the bar comes out
    /// square over the part, joined to the bar at a shoulder towards the
    /// jaws (LIO-211). The demo 4-angle job.
    pub(crate) fn faced_square(tool: &crate::model::Tool, mesh: &Mesh) -> crate::model::Operation {
        use crate::model::{Operation, Strategy};
        let mut op = Operation::new(Strategy::Face, tool.id.clone());
        let ids = (mesh.surfaces.iter().enumerate())
            .filter(|(_, f)| f.normal[0].abs() < 0.01)
            .map(|(i, _)| i)
            .collect();
        op.surfaces = Some(crate::surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids,
        });
        op.p.insert("containment".into(), "Tool center on boundary".into());
        op
    }
    /// The angles the moves of `plan` are made at, each once, in order.
    fn angle_order(plan: &crate::simulation::Plan) -> Vec<f32> {
        let mut v: Vec<f32> = vec![];
        for a in plan.moves.iter().map(|m| m.a.unwrap()) {
            if v.last() != Some(&a) {
                v.push(a);
            }
        }
        v
    }

    /// The height `plan` of setup `si` turns A at with `clearance`: over
    /// what A swings under its tool where it keeps it (`index_height`).
    pub(crate) fn height(
        doc: &Document,
        mesh: &Mesh,
        si: usize,
        plan: &crate::simulation::Plan,
        clearance: f32,
    ) -> Swing {
        let (parts, _) = crate::post::tool_parts(doc, &doc.setups[si], &doc.tools[0].id, plan);
        let over = cut_span([&plan.moves[..]]);
        doc.index_height_unchecked(si, mesh, clearance, over, &parts)
            .unwrap()
    }

    #[test]
    fn a_flat_on_the_parts_plus_y_face_is_cut_on_top_at_a_plus_90() {
        use crate::model::{Operation, Strategy};
        use crate::simulation::{Motion, prepare};
        let mesh = prism(30.);
        let tool = cutter("12");
        // Facing the part's +Y face, which only A90 brings to the top.
        let mut op = Operation::new(Strategy::Face, tool.id.clone());
        let plus_y = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, f)| f.normal[1].abs() > 0.999)
            .max_by(|a, b| a.1.center[1].total_cmp(&b.1.center[1]))
            .unwrap()
            .0;
        op.surfaces = Some(crate::surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: vec![plus_y],
        });
        let doc = bar_job(&mesh, 0., "80", "-150", "90", op.clone());
        let sim = prepare(&doc, &mesh, 1, 0).unwrap();
        let plan = &sim.plan;
        // From A0, where setup 1 left it: the index comes first.
        assert_eq!(plan.moves[0].motion, Motion::Index);
        assert!(plan.moves.iter().all(|m| m.a == Some(90.)));
        // Every cut, taken back to the part, is on its +Y face or above it
        // (Y10 of the part's middle), across the face.
        let at = doc.at_angle(1, 90., &mesh).unwrap();
        let cuts: Vec<[f32; 3]> = plan
            .moves
            .iter()
            .filter(|m| m.motion == Motion::Feed && m.cutting_pass)
            .flat_map(|m| [m.from, m.to])
            .map(|p| at.frame.unmap(p))
            .collect();
        assert!(!cuts.is_empty());
        assert!(cuts.iter().all(|q| q[1] > 10. - 0.01), "{cuts:?}");
        assert!(cuts.iter().any(|q| q[1] < 10. + 0.01));
        // On the machine those tips are on top: Z above the A centreline.
        let axis = doc.setups[1].turn.unwrap().axis;
        assert!(
            plan.moves
                .iter()
                .filter(|m| m.cutting_pass)
                .all(|m| m.to[2] > axis[1] + 9.99)
        );
        // The stock is cut over the +Y face, and not over the -Y face.
        let mut done = sim.clone();
        done.seek(1.);
        let (volume, base) = (
            done.volume.unwrap(),
            crate::volume::Frame::new(&doc.setups[0], &mesh),
        );
        let c = centre_z(&mesh);
        let filled = |q: [f32; 3]| volume.filled(base.map(q), base);
        assert!(!filled([0., 11.5, c]), "over the +Y face");
        assert!(filled([0., -11.5, c]), "over the -Y face");
        // At A-90 the +Y face is underneath: as main faces a selected
        // surface that faces down, the Facing faces the part's top there
        // (#290), and the editor warns that it does (`facing_faces_down`, A's
        // turn included).
        assert!(!crate::simulation::facing_faces_down(
            &doc.setups[1],
            &op,
            &mesh
        ));
        let doc = bar_job(&mesh, 0., "80", "-150", "-90", op.clone());
        assert!(crate::simulation::facing_faces_down(
            &doc.setups[1],
            &op,
            &mesh
        ));
        assert!(prepare(&doc, &mesh, 1, 0).is_ok());
    }

    #[test]
    fn a_square_milled_at_0_90_180_270_comes_out_square_on_the_axis() {
        use crate::simulation::prepare;
        let mesh = prism(30.);
        let doc = bar_job(
            &mesh,
            0.,
            "80",
            "-150",
            "0, 90, 180, 270",
            contour(&cutter("12")),
        );
        let mut sim = prepare(&doc, &mesh, 1, 0).unwrap();
        assert_eq!(angle_order(&sim.plan), [0., 90., 180., 270.]);
        sim.seek(1.);
        let (volume, base) = (
            sim.volume.unwrap(),
            crate::volume::Frame::new(&doc.setups[0], &mesh),
        );
        let c = centre_z(&mesh);
        let filled = |dy: f32, dz: f32| volume.filled(base.map([0., dy, c + dz]), base);
        // Halfway along the part: the square's 20 × 20 left on the bar's
        // axis, the bar round it (Ø40) cut away on all four sides.
        for (dy, dz) in [
            (8., 0.),
            (-8., 0.),
            (0., 8.),
            (0., -8.),
            (8., 8.),
            (-8., -8.),
            (8., -8.),
            (-8., 8.),
        ] {
            assert!(filled(dy, dz), "inside the square at {dy} {dz}");
        }
        for (dy, dz) in [
            (13., 0.),
            (-13., 0.),
            (0., 13.),
            (0., -13.),
            (12., 12.),
            (-12., 12.),
            (12., -12.),
            (-12., -12.),
        ] {
            assert!(!filled(dy, dz), "outside the square at {dy} {dz}");
        }
        // The same faces on each side: as far from the axis either way.
        let edge = |dir: [f32; 2]| {
            (0..200)
                .map(|i| i as f32 * 0.1)
                .find(|r| !filled(dir[0] * r, dir[1] * r))
                .unwrap()
        };
        let sides = [[1., 0.], [-1., 0.], [0., 1.], [0., -1.]].map(edge);
        for r in sides {
            assert!(
                (r - sides[0]).abs() <= volume.cell[1].max(volume.cell[2]) + 0.01,
                "{sides:?}"
            );
        }
    }

    #[test]
    fn the_rest_analysis_at_the_last_angle_sees_what_the_earlier_angles_removed() {
        use crate::simulation::{Motion, prepare};
        let mesh = prism(30.);
        let doc = bar_job(
            &mesh,
            0.,
            "80",
            "-150",
            "0, 90, 180, 270",
            contour(&cutter("12")),
        );
        let mut sim = prepare(&doc, &mesh, 1, 0).unwrap();
        // The stock as A turns to 270°: cut at 0, 90 and 180 only.
        let last = sim
            .plan
            .moves
            .iter()
            .rfind(|m| m.motion == Motion::Index)
            .unwrap()
            .clone();
        assert_eq!(last.a, Some(270.));
        sim.seek(last.start / sim.plan.duration);
        let volume = sim.volume.clone().unwrap();
        let at = doc.at_angle(1, 270., &mesh).unwrap();
        // Halfway along the part, on the A centreline.
        let [x, y, z] = at.frame.map([0., 0., centre_z(&mesh)]);
        let axis = doc.setups[1].turn.unwrap().axis;
        assert!((y - axis[0]).abs() < 1e-3 && (z - axis[1]).abs() < 1e-3);
        // In the frame at 270° the bar round the square is gone on every
        // side, the side 270° itself is to cut too (180° cut it as well),
        // and the square is still there.
        let removed = crate::rest::Removed::new(&volume, at.frame);
        for (dy, dz) in [(15., 0.), (-15., 0.), (0., 15.), (0., -15.)] {
            assert!(removed.at([x, y + dy, z + dz]), "removed at {dy} {dz}");
            assert!(!volume.filled([x, y + dy, z + dz], at.frame));
        }
        for (dy, dz) in [(5., 5.), (-5., -5.), (0., 0.)] {
            assert!(!removed.at([x, y + dy, z + dz]), "kept at {dy} {dz}");
            assert!(volume.filled([x, y + dy, z + dz], at.frame));
        }
        // The stock the setup starts 270° from is told it: its start height
        // over the square's +Z side at 270° is the square's top, not the
        // bar's, since 90° cut there before.
        let rest = crate::rest::RestStock::new(&volume, at.frame);
        let over = crate::mesh::Bounds {
            min: [x - 5., y - 4., z + 10.],
            max: [x + 5., y + 4., z + 20.],
        };
        let top = rest.top(over, z + 10.5, z + 20.);
        assert!(top < z + 12., "{top}");
        // The whole job leaves none of the square's four sides under stock.
        sim.seek(1.);
        let left = crate::uncut::uncut(sim.volume.as_ref().unwrap(), &doc.setups[0], &mesh);
        for (i, f) in mesh.faces.iter().enumerate() {
            let [a, b, c] = [f.points[0], f.points[1], f.points[2]];
            let (u, v) = ([b[1] - a[1], b[2] - a[2]], [c[1] - a[1], c[2] - a[2]]);
            // A side's normal lies across X.
            if (u[0] * v[1] - u[1] * v[0]).abs() < 1e-6 {
                assert!(!left.faces[i], "side face {i} under stock");
            }
        }
    }

    #[test]
    fn the_simulated_a_turns_through_each_index_move() {
        use crate::simulation::{Motion, prepare};
        let mesh = prism(30.);
        let doc = bar_job(
            &mesh,
            0.,
            "80",
            "-150",
            "0, 90, 180, 270",
            contour(&cutter("12")),
        );
        let mut sim = prepare(&doc, &mesh, 1, 0).unwrap();
        let duration = sim.plan.duration;
        assert_eq!(sim.a_now(), Some(0.));
        let index: Vec<_> = (sim.plan.moves.iter())
            .filter(|m| m.motion == Motion::Index)
            .cloned()
            .collect();
        for (k, m) in index.iter().enumerate() {
            let from = 90. * k as f32;
            sim.seek((m.start + 0.25 * (m.end - m.start)) / duration);
            let a = sim.a_now().unwrap();
            assert!((a - (from + 22.5)).abs() < 0.05, "{a} turning from {from}");
            sim.seek(m.end / duration + 1e-6);
            assert_eq!(sim.a_now(), Some(from + 90.));
        }
        sim.seek(1.);
        assert_eq!(sim.a_now(), Some(270.));
        // Back to the start, A is where it began.
        sim.seek(0.);
        assert_eq!(sim.a_now(), Some(0.));
        // A 3-axis operation has no A.
        let flat = crate::simulation::prepare(
            &crate::model::Document::sample(),
            &crate::mesh::sample(),
            0,
            0,
        );
        if let Ok(flat) = flat {
            assert_eq!(flat.a_now(), None);
        }
    }

    #[test]
    fn an_index_move_retracts_above_the_swing_before_a_turns() {
        use crate::simulation::{Motion, prepare};
        let mesh = prism(30.);
        let doc = bar_job(
            &mesh,
            0.,
            "80",
            "-150",
            "0, 90, 180, 270",
            contour(&cutter("12")),
        );
        let plan = prepare(&doc, &mesh, 1, 0).unwrap().plan;
        // Along all of A the jaws reach farthest: the Ø40 bar's radius and
        // 35 mm of jaw, 20 mm wide; 15 mm of clearance over them.
        let all = doc.swing(1, &mesh, 15.).unwrap();
        assert!((all.radius - 55f32.hypot(10.)).abs() < 1e-3, "{all:?}");
        assert!((all.z - (20. + all.radius + 15.)).abs() < 1e-3);
        assert!((all.machine_z - (-150. + all.radius + 15.)).abs() < 1e-3);
        // The part is 80 mm out of the jaws: under the tool, its holder and
        // the spindle nose A swings only the Ø40 bar, so the tool turns A
        // 15 mm over the bar's top (LIO-209).
        let swing = height(&doc, &mesh, 1, &plan, 15.);
        assert_eq!((swing.what, swing.radius), ("the stock reaches", 20.));
        assert!((swing.z - (20. + 20. + 15.)).abs() < 1e-3, "{swing:?}");
        let index: Vec<usize> = (0..plan.moves.len())
            .filter(|&i| plan.moves[i].motion == Motion::Index)
            .collect();
        assert_eq!(index.len(), 3);
        let mut before = 0.;
        for i in index {
            let (up, turn, over, down) = (
                &plan.moves[i - 1],
                &plan.moves[i],
                &plan.moves[i + 1],
                &plan.moves[i + 2],
            );
            // Straight up to the swing height at the angle it cut at...
            assert_eq!((up.motion, up.a), (Motion::Rapid, Some(before)));
            assert!(up.from[2] < up.to[2] && up.from[..2] == up.to[..2]);
            assert!((up.to[2] - swing.z).abs() < 1e-4);
            // ...then A turns, the tool still there...
            assert_eq!((turn.from, turn.to), (up.to, up.to));
            let a = turn.a.unwrap();
            assert_eq!(a, before + 90.);
            assert!((turn.end - turn.start - 90. / INDEX_RATE * 60.).abs() < 1e-4);
            // ...then over the next first cut at that height (unless it is
            // there already) and down to it.
            let (down, next) = if over.from[..2] == over.to[..2] {
                (over, i + 2)
            } else {
                assert!(
                    (over.to[2] - swing.z).abs() < 1e-4 && (over.from[2] - swing.z).abs() < 1e-4
                );
                assert_eq!((over.motion, over.a), (Motion::Rapid, Some(a)));
                (down, i + 3)
            };
            assert_eq!((down.motion, down.a), (Motion::Rapid, Some(a)));
            assert!(down.to[2] < down.from[2] && down.from[..2] == down.to[..2]);
            assert_eq!(plan.moves[next].motion, Motion::Feed);
            before = a;
        }
        // The moves are one path, in time.
        for pair in plan.moves.windows(2) {
            assert_eq!(pair[0].to, pair[1].from);
            assert!((pair[0].end - pair[1].start).abs() < 1e-3);
        }
    }

    #[test]
    fn an_index_retract_above_the_z_travel_is_refused_with_its_numbers() {
        // The assumed centreline, Z -40, and a 10 mm part over the jaws:
        // the jaws' 55.9 mm and 15 mm of clearance over them end 30.9 mm
        // above the top of the Z travel.
        let mesh = prism(10.);
        let doc = bar_job(&mesh, -17., "60", "-40", "0, 90", contour(&cutter("12")));
        let refused = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap_err();
        for words in [
            "at machine Z 30.9 with the tool over X",
            "(the A centreline at Z -40 + 55.9 mm, as far as the chuck's jaws reach from it at any angle, + 15 mm clearance)",
            "30.9 mm above the top of the machine's 300 mm Z travel (Z 0)",
            "not cut short",
            "keep the cuts farther from the jaws",
        ] {
            assert!(refused.contains(words), "{refused}");
        }
        // Even one angle away from A0.
        let doc = bar_job(&mesh, -17., "60", "-40", "90", contour(&cutter("12")));
        assert!(
            crate::simulation::prepare(&doc, &mesh, 1, 0)
                .unwrap_err()
                .contains("machine Z 30.9")
        );
        // At A0 alone A does not turn: nothing to retract for.
        let doc = bar_job(&mesh, -17., "60", "-40", "0", contour(&cutter("12")));
        assert!(crate::simulation::prepare(&doc, &mesh, 1, 0).is_ok());
        // Away from the jaws the tool turns A over the Ø40 bar: 20 mm and
        // 15 mm of clearance end at machine Z -5, inside the travel.
        let mesh = prism(30.);
        let doc = bar_job(&mesh, 0., "80", "-40", "0, 90", contour(&cutter("12")));
        let plan = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap().plan;
        let swing = height(&doc, &mesh, 1, &plan, 15.);
        assert!((swing.machine_z - -5.).abs() < 1e-3, "{swing:?}");
    }

    #[test]
    fn a_tool_over_the_jaws_lifts_above_them_and_one_over_the_bar_above_the_bar() {
        use crate::simulation::{Motion, prepare};
        // A 10 mm part 3 mm from the jaws (LIO-209): the Ø12 tool round it
        // is over them, and under the tool A swings the jaws, 55.9 mm out,
        // so A turns 15 mm above them.
        let mesh = prism(10.);
        let doc = bar_job(&mesh, 3., "60", "-150", "90, 180", contour(&cutter("12")));
        let plan = prepare(&doc, &mesh, 1, 0).unwrap().plan;
        let swing = height(&doc, &mesh, 1, &plan, 15.);
        assert_eq!(swing.what, "the chuck's jaws reach");
        assert!((swing.radius - 55f32.hypot(10.)).abs() < 1e-3, "{swing:?}");
        let all = doc.swing(1, &mesh, 15.).unwrap();
        assert_eq!((swing.what, swing.z), (all.what, all.z));
        let index: Vec<_> = (plan.moves.iter())
            .filter(|m| m.motion == Motion::Index)
            .collect();
        assert_eq!(index.len(), 2);
        for m in index {
            assert!(
                (m.from[2] - (20. + swing.radius + 15.)).abs() < 1e-3,
                "{m:?}"
            );
        }
        // The same part 60 mm along the bar is over the bar alone: A turns
        // 15 mm over its 20 mm, 30.9 mm lower, whatever the jaws reach.
        let far = bar_job(&mesh, 43., "60", "-150", "90, 180", contour(&cutter("12")));
        let plan = prepare(&far, &mesh, 1, 0).unwrap().plan;
        let bar = height(&far, &mesh, 1, &plan, 15.);
        assert_eq!((bar.what, bar.radius), ("the stock reaches", 20.));
        for m in plan.moves.iter().filter(|m| m.motion == Motion::Index) {
            assert!((m.from[2] - (20. + 20. + 15.)).abs() < 1e-3, "{m:?}");
        }
    }

    #[test]
    fn an_index_move_nearer_the_jaws_than_the_gap_is_refused() {
        // A 10 mm long part over the jaws, 1 mm of clearance: waiting 1 mm
        // over the jaws' reach is inside the chuck's 2 mm gap.
        let mesh = prism(10.);
        let mut op = contour(&cutter("12"));
        op.p.insert("clearance".into(), "1".into());
        op.p.insert("retract".into(), "1".into());
        let doc = bar_job(&mesh, -17., "60", "-150", "90, 180", op.clone());
        let refused = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap_err();
        assert!(
            refused.starts_with("The index move from A 0° to A 90° brings the tool within the 2 mm gap of the chuck jaw"),
            "{refused}"
        );
        // From A0 at A0 the approach waits there too, though A does not turn.
        let doc = bar_job(&mesh, -17., "60", "-150", "0, 90", op);
        let refused = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap_err();
        assert!(
            refused.starts_with(
                "The approach at A 0° brings the tool within the 2 mm gap of the chuck jaw"
            ),
            "{refused}"
        );
    }

    #[test]
    fn three_axis_moves_carry_no_a_and_no_index() {
        use crate::simulation::{Motion, prepare};
        let mesh = prism(30.);
        let mut doc = bar_job(&mesh, 0., "80", "-150", "90", contour(&cutter("12")));
        // Setup 1's own contour is a 3-axis operation, as it was.
        let op = contour(&doc.tools[0]);
        doc.setups[0].operations = vec![op.clone()];
        let sim = prepare(&doc, &mesh, 0, 0).unwrap();
        assert!(
            sim.plan
                .moves
                .iter()
                .all(|m| m.a.is_none() && m.motion != Motion::Index)
        );
        assert!(sim.turns.is_empty());
        let mut fresh = doc.setups[0].clone();
        fresh.p.insert("stock_source".into(), "fresh".into());
        let kernel = crate::simulation::generate(&fresh, &op, &doc.tools[0], &mesh).unwrap();
        // `plan` with its air taken out as the sequence takes it
        // (`air::trim`), held as `held` holds it.
        let trimmed = |plan: crate::simulation::Plan,
                       held: &crate::model::Document,
                       sim: &crate::simulation::Simulation| {
            let mut setup = held.setups[0].clone();
            setup.p.insert("stock_source".into(), "fresh".into());
            let (volume, frame) = (sim.initial_volume.as_ref().unwrap(), sim.frame.unwrap());
            let holding = crate::air::workholding(held, &setup, &mesh, Some(volume), frame);
            crate::air::trim(plan, &setup, &op, volume, frame, sim.source, &holding)
                .unwrap()
                .0
        };
        // `b` is `a` move for move, timed alike.
        let same = |a: &[crate::simulation::Move], b: &[crate::simulation::Move]| {
            assert_eq!(a.len(), b.len());
            let shift = b[0].start - a[0].start;
            for (a, b) in a.iter().zip(b) {
                assert_eq!((a.from, a.to, a.motion, a.a), (b.from, b.to, b.motion, b.a));
                assert!((a.start + shift - b.start).abs() < 1e-3);
                assert!((a.end + shift - b.end).abs() < 1e-3);
            }
        };
        // Held in the chuck it enters from above the jaws (LIO-206); from
        // its first feed on it is the kernel's, move for move. The sequence
        // takes the air out of that.
        let entered = enter_held(&doc, 0, &op, &mesh, kernel.clone()).unwrap();
        let feed = |p: &crate::simulation::Plan| {
            (p.moves.iter())
                .position(|m| m.motion == Motion::Feed)
                .unwrap()
        };
        same(
            &kernel.moves[feed(&kernel)..],
            &entered.moves[feed(&entered)..],
        );
        same(&trimmed(entered, &doc, &sim).moves, &sim.plan.moves);
        // In a vise or clamps it is the kernel's plan itself, its air
        // taken out.
        for holding in ["Vise", "Clamps"] {
            let mut held = doc.clone();
            held.setups[0].p.insert("holding".into(), holding.into());
            let sim = prepare(&held, &mesh, 0, 0).unwrap();
            let direct = trimmed(kernel.clone(), &held, &sim);
            same(&direct.moves, &sim.plan.moves);
            assert_eq!(direct.moves[0].start, sim.plan.moves[0].start, "{holding}");
            assert_eq!(direct.duration, sim.plan.duration, "{holding}");
        }
        // The indexed setup after it starts with A turning from 0 to 90.
        let indexed = prepare(&doc, &mesh, 1, 0).unwrap();
        assert_eq!(indexed.plan.moves[0].motion, Motion::Index);
        assert_eq!(indexed.turns.len(), 1);
        assert_eq!(
            indexed.turns[0].1,
            doc.at_angle(1, 90., &mesh).unwrap().frame
        );
    }

    #[test]
    fn a_setup_posed_at_an_angle_has_the_frame_at_angle_gives() {
        let (doc, mesh) = job("0, 90, 180, -90");
        for a in [0., 90., 180., -90., 37.] {
            let posed = doc.setup_at(&doc.setups[1], 1, a, &mesh).unwrap();
            let frame = crate::volume::Frame::new(&posed, &mesh);
            let at = doc.at_angle(1, a, &mesh).unwrap().frame;
            for p in [[1., 2., 3.], [-20., 15., -4.], mesh.origin] {
                assert!(close(frame.map(p), at.map(p)), "A{a}");
            }
        }
    }

    #[test]
    fn indexed_operations_of_an_older_project_move_into_indexed_setups() {
        use crate::model::{Operation, Strategy};
        let (mut doc, _) = job("90");
        doc.setups.truncate(1);
        let tool = doc.tools[0].id.clone();
        let old = |name: &str, a: &str| {
            let mut op = Operation::new(Strategy::Indexed, tool.clone());
            op.p.insert("name".into(), name.into());
            op.p.insert("angle_a".into(), a.into());
            op.p.insert("angle_b".into(), "0".into());
            op
        };
        let face = Operation::new(Strategy::Face, tool.clone());
        let after = Operation::new(Strategy::Contour, tool.clone());
        let ids = [face.id.clone(), after.id.clone()];
        doc.setups[0].operations = vec![face, old("At zero", "0"), old("Side", "90"), after];
        let notes = doc.migrate_indexed_operations();
        let names: Vec<&str> = doc.setups.iter().map(|s| s.name()).collect();
        let first = doc.setups[0].name().to_string();
        assert_eq!(
            names,
            [
                first.clone(),
                format!("{first} · A 90°"),
                format!("{first} · A 0°")
            ]
        );
        let ops = |i: usize| -> Vec<(Strategy, String)> {
            doc.setups[i]
                .operations
                .iter()
                .map(|o| (o.strategy, o.name().to_string()))
                .collect()
        };
        assert_eq!(ops(0)[1], (Strategy::Face, "At zero".to_string()));
        assert_eq!(doc.setups[0].operations[0].id, ids[0]);
        assert_eq!(ops(1), [(Strategy::Face, "Side".to_string())]);
        assert_eq!(doc.setups[2].operations[0].id, ids[1]);
        // The new setups index A in setup 1's clamping, in its order.
        for (i, a) in [(1, "90"), (2, "0")] {
            assert!(is_indexed(&doc.setups[i]));
            assert_eq!(get(&doc.setups[i].p, ANGLES), a);
            assert_eq!(doc.index_base(i), Some(0));
            assert_eq!(
                get(&doc.setups[i].p, "origin"),
                get(&doc.setups[0].p, "origin")
            );
        }
        assert!(!doc.setups[1].operations[0].p.contains_key("angle_a"));
        assert!(notes[0].contains("Facing operation in"), "{notes:?}");
        assert!(
            notes[1].contains("new indexed setup at that angle"),
            "{notes:?}"
        );
        // On a machine without A around X it stays, and says why.
        let (mut doc, _) = job("90");
        doc.setups.truncate(1);
        doc.setups[0].machine = Machine::new(5);
        doc.setups[0].operations = vec![old("Tilted", "30")];
        let notes = doc.migrate_indexed_operations();
        assert_eq!(doc.setups.len(), 1);
        let op = &doc.setups[0].operations[0];
        assert_eq!(op.strategy, Strategy::Indexed);
        assert!(notes[0].ends_with(LEGACY));
        assert_eq!(
            op.issues(&doc.setups[0].machine, &doc.tools, &[])[0],
            LEGACY
        );
    }

    #[test]
    fn the_plan_check_checks_each_angles_moves_against_the_part_posed_there() {
        // 30 wide in Y and 10 high at A0, 10 wide at A90: the contour at
        // A90 runs 11 mm from the axis, inside where the part is at A0.
        let mesh = block(30., 30., 10.);
        let doc = bar_job(&mesh, 0., "80", "-150", "0, 90", contour(&cutter("12")));
        let sim = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap();
        let first = crate::cam::Geometry::new(&doc.setups[1], &mesh);
        let index = crate::analysis::Index::new(&first);
        let found = crate::plan_check::angle_problems(&doc, &mesh, 1, &sim, &first, &index);
        assert!(found.is_empty(), "{found:?}");
        // Against the part at A0 alone, the A90 moves would go through it.
        let wrong = crate::plan_check::axis_problems(&sim.plan, &first, &index, 1);
        assert!(wrong[0].contains("passes through"), "{wrong:?}");
    }

    #[test]
    fn the_swing_takes_the_centreline_the_control_page_measured() {
        let mesh = prism(30.);
        let mut doc = bar_job(&mesh, 0., "80", "-40", "0, 90", contour(&cutter("12")));
        // Along all of A the jaws reach above the Z travel at Z -40.
        assert!(doc.swing(1, &mesh, 15.).is_err());
        // Measured at Z -150 in the machine library; the setup's copy still
        // says -40.
        let mut library = doc.setups[0].machine.clone();
        library.p.insert("a_centre_z".into(), "-150".into());
        library.p.insert(
            crate::control::CENTRE_MARK.into(),
            crate::control::MEASURED.into(),
        );
        doc.machines = vec![library];
        assert_eq!(get(&doc.setups[1].machine.p, "a_centre_z"), "-40");
        let swing = doc.swing(1, &mesh, 15.).unwrap();
        assert!((swing.machine_z - (-150. + swing.radius + 15.)).abs() < 1e-3);
        let plan = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap().plan;
        let over = height(&doc, &mesh, 1, &plan, 15.);
        assert!((over.machine_z - (-150. + 20. + 15.)).abs() < 1e-3);
        // Gone stale, it is not trusted: the setup's copy is used again.
        doc.machines[0].p.insert(
            crate::control::CENTRE_MARK.into(),
            "stale after an E-stop".into(),
        );
        assert!(doc.swing(1, &mesh, 15.).is_err());
        let over = height(&doc, &mesh, 1, &plan, 15.);
        assert!((over.machine_z - (-40. + 20. + 15.)).abs() < 1e-3);
    }

    #[test]
    fn an_indexed_setups_cuts_are_checked_against_the_chuck_at_each_angle() {
        // 90 mm out of the jaws, with 40 mm of clearance over the bar: clear
        // of the chuck at both angles, so nothing is said, not even that the
        // holding was not checked (LIO-191). The Ø65 spindle nose is kept
        // over the jaws by a 40 mm holder (LIO-204): with the assumed 10 mm
        // one it comes down beside jaw 1.
        let mesh = prism(30.);
        let mut op = contour(&cutter("12"));
        op.p.insert("clearance".into(), "40".into());
        let mut doc = bar_job(&mesh, 0., "90", "-150", "0, 90", op);
        let sim = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap();
        let held = crate::post::held(&doc, &mesh, 1, &[(0, Ok(sim))]);
        assert!(
            held.iter().any(|h| h.message.starts_with(
                "Flats at A 0°, Contour: the spindle nose hits the chuck jaw 1"
            ) && h.message.ends_with(
                "Assumed: the holder's 10 mm length (the tool gives none: enter it in the Tool library)."
            )),
            "{held:?}"
        );
        doc.tools[0].p.insert("holder_length".into(), "40".into());
        let sim = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap();
        let held = crate::post::held(&doc, &mesh, 1, &[(0, Ok(sim))]);
        assert!(held.is_empty(), "{held:?}");
    }

    #[test]
    fn an_indexed_setup_exports_g0_a_alone_after_each_retract_and_replays() {
        // The part 20 mm along the bar and 100 mm of it out of the jaws:
        // the holder stays clear of them round the part at each angle.
        let mesh = prism(30.);
        let mut doc = bar_job(
            &mesh,
            20.,
            "100",
            "-150",
            "90, 180",
            on_the_bar(&cutter("12")),
        );
        doc.tools[0].p.insert("number".into(), "1".into());
        doc.file = Some(crate::model::ModelFile {
            name: "bar.stl".into(),
            path: "bar.stl".into(),
            format: "STL".into(),
            bytes: 1,
            thickness: None,
        });
        let posted = crate::post::post_job(&doc, &mesh);
        // `post_job` writes a program only once its replay matches the
        // simulated moves, A included.
        let programs = posted.setups[1].result.as_ref().unwrap();
        let text = &programs[0].text;
        assert!(text.starts_with("(AXIS CAM Studio - LinuxCNC 4-axis indexed program)\n"));
        assert!(
            text.contains("The A centreline is at machine Y140 Z-150 mm, assumed, not measured"),
            "{text}"
        );
        let code: Vec<&str> = text.lines().filter(|l| !l.starts_with('(')).collect();
        // A90 after the tool change's retract and the rapid across over the
        // first cut, A180 at the index move's height, A0 after the last
        // retract; no other line has an A word.
        let turns: Vec<usize> = (0..code.len()).filter(|&i| code[i].contains('A')).collect();
        assert_eq!(turns.len(), 3, "{code:?}");
        assert_eq!(code[turns[0]], "G0 A90.000");
        assert!(code[turns[0] - 1].starts_with("G0 X"), "{code:?}");
        assert!(code[..turns[0]].contains(&"G53 G0 Z0"), "{code:?}");
        assert_eq!(code[turns[1]], "G0 A180.000");
        assert_eq!(code[turns[2] - 1..], ["G53 G0 Z0", "G0 A0.000", "M2"]);
        // The tool is above all that A swings under it when A turns: the
        // bar's reach from the axis and 15 mm clearance.
        let plan = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap().plan;
        let swing = height(&doc, &mesh, 1, &plan, 15.);
        let z = |line: &str| -> f32 {
            let w = line.split(' ').find(|w| w.starts_with('Z')).unwrap();
            w[1..].parse().unwrap()
        };
        let before = code[..turns[1]]
            .iter()
            .rev()
            .find(|l| l.contains('Z'))
            .unwrap();
        // Work Z0 is on the centreline.
        let axis = doc.setups[1].turn.unwrap().axis;
        assert!(
            (z(before) - (swing.z - axis[1])).abs() < 1e-3,
            "{before} {swing:?}"
        );
        // The checks see the chuck's jaws at each angle: nothing says the
        // holding was not checked.
        let sim = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap();
        let held = crate::post::held(&doc, &mesh, 1, &[(0, Ok(sim))]);
        assert!(held.is_empty(), "{held:?}");
    }

    #[test]
    fn a_disabled_indexed_operation_at_an_angle_stays_where_it_is() {
        use crate::model::{Operation, Strategy};
        let (mut doc, _) = job("90");
        doc.setups.truncate(1);
        let tool = doc.tools[0].id.clone();
        let mut old = Operation::new(Strategy::Indexed, tool.clone());
        old.p.insert("angle_a".into(), "90".into());
        old.enabled = false;
        let after = Operation::new(Strategy::Contour, tool);
        doc.setups[0].operations = vec![old, after];
        let notes = doc.migrate_indexed_operations();
        assert_eq!(doc.setups.len(), 1);
        let ops = &doc.setups[0].operations;
        assert_eq!(
            (ops[0].strategy, ops[1].strategy),
            (Strategy::Indexed, Strategy::Contour)
        );
        assert!(notes[0].ends_with(LEGACY), "{notes:?}");
    }

    /// `bar_job` with a numbered tool and a part file, so it posts, its
    /// contour leaving a web on the A axis (`on_the_bar`).
    pub(crate) fn posting_bar_job(x0: f32, stickout: &str, angles: &str) -> (Document, Mesh) {
        let mesh = prism(30.);
        let mut doc = bar_job(
            &mesh,
            x0,
            stickout,
            "-150",
            angles,
            on_the_bar(&cutter("12")),
        );
        doc.tools[0].p.insert("number".into(), "1".into());
        doc.file = Some(crate::model::ModelFile {
            name: "bar.stl".into(),
            path: "bar.stl".into(),
            format: "STL".into(),
            bytes: 1,
            thickness: None,
        });
        (doc, mesh)
    }
    #[test]
    fn a_turn_of_a_below_the_swing_height_is_refused() {
        use crate::model::POST_RETRACT;
        // Setup 3 indexes to A90 as setup 2 left it: its plan has no index
        // move, so the program turns A to A90 where its retract leaves the
        // tool, since the A the machine is at when it starts is not known.
        let (mut doc, mesh) = posting_bar_job(20., "100", "90");
        let mut third = doc.setups[1].clone();
        third.id = crate::model::id();
        doc.setups.push(third);
        doc.sync_stock_sources();
        doc.sync_turns(Some(&mesh));
        // Setup 3 starts at the angle setup 2 left, and comes down from
        // above what A swings under the tool as an index move does
        // (LIO-205): its program turns A at machine Z home over the first
        // cut, and is exported. At the clearance plane, now over the bar
        // and not the jaws (LIO-209), a program that starts with the tool
        // wherever it was is refused, as on a 3-axis setup: it could arrive
        // over the jaws below their reach.
        for (mode, exported) in POST_RETRACT.iter().zip([true, false]) {
            doc.setups[0]
                .machine
                .p
                .insert("post_retract".into(), (*mode).into());
            doc.sync_indexed();
            let posted = crate::post::post_job(&doc, &mesh);
            for k in [1, 2] {
                let result = &posted.setups[k].result;
                match exported {
                    true => assert!(result.is_ok(), "{mode}: {result:?}"),
                    false => assert!(
                        result.as_ref().is_err_and(|e| e
                            .iter()
                            .any(|e| e.contains("is not above the chuck jaw"))),
                        "{mode}: {result:?}"
                    ),
                }
            }
        }
        // Its plans checked once the operation's clearance is raised to
        // 40 mm put the swing above where its program turns A, the
        // clearance plane: that turn is refused. At machine Z home it is not.
        let sims = crate::run::simulated_for_tests(&doc, &mesh, 2);
        let mut raised = doc.clone();
        (raised.setups[2].operations[0].p).insert("clearance".into(), "40".into());
        let below = "A turns to A 90° with the tool tip at Z";
        for (mode, refused) in POST_RETRACT.iter().zip([false, true]) {
            raised.setups[0]
                .machine
                .p
                .insert("post_retract".into(), (*mode).into());
            raised.sync_indexed();
            let found = crate::post::checked(
                &raised,
                &mesh,
                2,
                &sims,
                crate::post::PostSettings::from_machine,
            )
            .err()
            .unwrap_or_default();
            assert_eq!(
                found
                    .iter()
                    .any(|e| e.contains(below) && e.contains("where nothing A swings reaches it")),
                refused,
                "{mode}: {found:?}"
            );
        }
    }
    #[test]
    fn each_angles_cuts_are_checked_against_the_jaws_at_that_angle() {
        // The part 20 mm from the jaws, cut to 2 mm over the axis: round it
        // the holder comes within their gap at the angles it is cut at.
        let (mut doc, mesh) = posting_bar_job(0., "80", "90, 180");
        (doc.setups[1].operations[0].p).insert("bottom".into(), "12".into());
        let posted = crate::post::post_job(&doc, &mesh);
        let refused = posted.setups[1].result.as_ref().unwrap_err();
        assert!(
            refused
                .iter()
                .any(|e| e.contains("the holder hits the chuck jaw")),
            "{refused:?}"
        );
        // The workholding check says the same.
        let sim = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap();
        let held = crate::post::held(&doc, &mesh, 1, &[(0, Ok(sim))]);
        assert!(
            held.iter().any(|h| h.message.contains("chuck jaw")),
            "{held:?}"
        );
    }
    #[test]
    fn an_indexed_setup_that_the_rotary_checks_refuse_is_not_exported() {
        // Exported as it is; with 150 mm of Y travel the bar, on the A
        // centreline at machine Y140, is cut beyond it at Y156: only the
        // rotary checks, which put each move on the machine, see that.
        let (mut doc, mesh) = posting_bar_job(20., "100", "90, 180");
        assert!(crate::post::post_job(&doc, &mesh).setups[1].result.is_ok());
        doc.setups[0].machine.p.insert("y".into(), "150".into());
        doc.sync_indexed();
        let sim = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap();
        let found = crate::rotary_check::problems(&doc, &mesh, 1, &[(0, Ok(sim))]);
        assert!(!found.is_empty());
        let posted = crate::post::post_job(&doc, &mesh);
        let refused = posted.setups[1].result.as_ref().unwrap_err();
        for (_, message) in &found {
            assert!(refused.contains(message), "{message} not in {refused:?}");
        }
    }
    /// That `plan` enters straight down over its first cut from the swing
    /// height `z`, at A `a`, with no index move first, clear of the chuck.
    fn enters_from_above(
        doc: &Document,
        mesh: &Mesh,
        plan: &crate::simulation::Plan,
        z: f32,
        a: f32,
    ) {
        use crate::simulation::Motion;
        let first = plan
            .moves
            .iter()
            .position(|m| m.motion == Motion::Feed)
            .unwrap();
        let (down, over) = (&plan.moves[..first], plan.moves[first].from);
        assert!(
            !down.is_empty() && (down[0].from[2] - z).abs() < 1e-3,
            "{down:?} {z}"
        );
        for m in down {
            assert_eq!((m.motion, m.a), (Motion::Rapid, Some(a)), "{m:?}");
            for p in [m.from, m.to] {
                assert!(close([p[0], p[1], 0.], [over[0], over[1], 0.]), "{m:?}");
            }
        }
        let stock = doc.setup_stock(0, Some(mesh)).unwrap();
        let rig = doc.chuck_rig(&doc.setups[0], stock).unwrap();
        let gap = crate::holding::gap(&doc.setups[0]).unwrap();
        let segments: Vec<_> = down.iter().map(|m| (m.from, m.to)).collect();
        let tool = (plan.radius, plan.tool_length, plan.holder_radius);
        assert!(rig.hits_at(a, &segments, tool, gap).is_empty());
    }
    #[test]
    fn an_indexed_setup_at_the_angle_a_is_at_comes_down_from_above_the_swing() {
        use crate::simulation::{Motion, prepare};
        // Setup 1 leaves A at 0, the first angle: no index move comes first,
        // and the kernel's own lead-in, from the stock's -X corner in the
        // jaws at its clearance plane, is replaced all the same (LIO-205).
        let (doc, mesh) = posting_bar_job(20., "100", "0, 90, 180, 270");
        let sim = prepare(&doc, &mesh, 1, 0).unwrap();
        assert_eq!(angle_order(&sim.plan), [0., 90., 180., 270.]);
        let swing = height(&doc, &mesh, 1, &sim.plan, 15.);
        enters_from_above(&doc, &mesh, &sim.plan, swing.z, 0.);
        let index = sim.plan.moves.iter().filter(|m| m.motion == Motion::Index);
        assert_eq!(index.count(), 3);
        // The checks find nothing, and it exports: A0 after the tool
        // change's retract and the rapid across over the first cut, then
        // down to it.
        let held = crate::post::held(&doc, &mesh, 1, &[(0, Ok(sim.clone()))]);
        assert!(held.is_empty(), "{held:?}");
        let found = crate::rotary_check::problems(&doc, &mesh, 1, &[(0, Ok(sim))]);
        assert!(found.is_empty(), "{found:?}");
        let posted = crate::post::post_job(&doc, &mesh);
        let programs = posted.setups[1].result.as_ref().unwrap();
        let code: Vec<&str> = programs[0]
            .text
            .lines()
            .filter(|l| !l.starts_with('('))
            .collect();
        let turns: Vec<usize> = (0..code.len()).filter(|&i| code[i].contains('A')).collect();
        assert_eq!(code[turns[0]], "G0 A0.000");
        assert!(code[turns[0] - 1].starts_with("G0 X"), "{code:?}");
        assert!(code[turns[0] + 1].starts_with("G0 Z"), "{code:?}");
        assert!(code[..turns[0]].contains(&"G53 G0 Z0"), "{code:?}");
        assert_eq!(turns.len(), 5, "{code:?}");
    }
    #[test]
    fn a_later_operation_at_the_same_angle_comes_down_from_above_the_swing() {
        use crate::simulation::{Motion, prepare};
        // Two contours at A90: the second starts where the first left A.
        let (mut doc, mesh) = posting_bar_job(20., "100", "90");
        let ops = &mut doc.setups[1].operations;
        ops[0].p.insert("allowance".into(), "1".into());
        let mut finish = ops[0].clone();
        finish.id = crate::model::id();
        finish.p.insert("allowance".into(), "0".into());
        ops.push(finish);
        let sim = prepare(&doc, &mesh, 1, 1).unwrap();
        assert!(sim.plan.moves.iter().all(|m| m.motion != Motion::Index));
        let swing = height(&doc, &mesh, 1, &sim.plan, 15.);
        enters_from_above(&doc, &mesh, &sim.plan, swing.z, 90.);
        let posted = crate::post::post_job(&doc, &mesh);
        assert!(
            posted.setups[1].result.is_ok(),
            "{:?}",
            posted.setups[1].result
        );
    }
    /// `posting_bar_job` with the contour in setup 1 too, then a finishing
    /// contour after it there: setup 1 is a plain setup on the bar in the
    /// chuck.
    fn chuck_setup_1_job(angles: &str) -> (Document, Mesh) {
        let (mut doc, mesh) = posting_bar_job(20., "100", angles);
        let mut rough = doc.setups[1].operations[0].clone();
        rough.id = crate::model::id();
        rough.p.insert("allowance".into(), "1".into());
        let mut finish = rough.clone();
        finish.id = crate::model::id();
        finish.p.insert("allowance".into(), "0".into());
        doc.setups[0].operations = vec![rough, finish];
        doc.sync_stock_sources();
        doc.sync_turns(Some(&mesh));
        (doc, mesh)
    }
    #[test]
    fn a_plain_setup_in_the_chuck_comes_down_from_above_the_jaws() {
        use crate::simulation::{Motion, prepare};
        let (doc, mesh) = chuck_setup_1_job("90, 180");
        // The kernel starts at the stock's -X -Y corner, in the jaws, at its
        // clearance plane, below what they reach.
        let mut fresh = doc.setups[0].clone();
        fresh.p.insert("stock_source".into(), "fresh".into());
        let op = &doc.setups[0].operations[0];
        let kernel = crate::simulation::generate(&fresh, op, &doc.tools[0], &mesh).unwrap();
        let stock = doc.setup_stock(0, Some(&mesh)).unwrap();
        let rig = doc.chuck_rig(&doc.setups[0], stock).unwrap();
        let corner = kernel.moves[0].from;
        assert!(
            corner[0] < rig.front && (corner[0] - stock.min[0]).abs() < 1.,
            "{corner:?}"
        );
        // Both operations instead come down over their first cut from the
        // swing height of setup 1's own clamping, with no A, clear of the
        // jaws (LIO-206).
        let swing = doc.swing(0, &mesh, 15.).unwrap();
        let all = doc.swing(1, &mesh, 15.).unwrap();
        assert_eq!((swing.what, swing.z), (all.what, all.z));
        assert!(corner[2] < swing.z - 30., "{corner:?} {swing:?}");
        let gap = crate::holding::gap(&doc.setups[0]).unwrap();
        for oi in 0..2 {
            let plan = prepare(&doc, &mesh, 0, oi).unwrap().plan;
            let first = (plan.moves.iter())
                .position(|m| m.motion == Motion::Feed)
                .unwrap();
            let (down, over) = (&plan.moves[..first], plan.moves[first].from);
            assert_eq!(down.len(), 1, "{down:?}");
            assert_eq!(down[0].motion, Motion::Rapid);
            assert!(close(down[0].from, [over[0], over[1], swing.z]), "{down:?}");
            assert!(close(down[0].to, over));
            assert!(plan.moves.iter().all(|m| m.a.is_none()));
            let tool = (plan.radius, plan.tool_length, plan.holder_radius);
            assert!(
                rig.hits_at(0., &[(down[0].from, over)], tool, gap)
                    .is_empty()
            );
            assert!((plan.duration - plan.moves.last().unwrap().end).abs() < 1e-3);
        }
        // Its workholding check finds nothing, and the job exports: setup
        // 1 at A0 with no A word, and the indexed setup after it.
        let sims = crate::run::simulated_for_tests(&doc, &mesh, 0);
        let held = crate::post::held(&doc, &mesh, 0, &sims);
        assert!(held.is_empty(), "{held:?}");
        let posted = crate::post::post_job(&doc, &mesh);
        for k in [0, 1] {
            assert!(
                posted.setups[k].result.is_ok(),
                "{k}: {:?}",
                posted.setups[k].result
            );
        }
        let program = &posted.setups[0].result.as_ref().unwrap()[0].text;
        let code = program.lines().filter(|l| !l.starts_with('('));
        assert!(code.clone().all(|l| !l.contains('A')), "{program}");
        // Both headers say the chuck was checked, not a vise (LIO-210).
        for k in [0, 1] {
            let program = &posted.setups[k].result.as_ref().unwrap()[0].text;
            assert!(
                program.contains("(Not checked: the table, the machine body and tool-change motion. The chuck, its jaws, the rotary head,")
                    && !program.contains("Vise jaws"),
                "{program}"
            );
        }
    }
    #[test]
    fn a_plain_setup_in_the_chuck_at_the_assumed_centreline_enters_from_the_top_of_the_z_travel() {
        use crate::simulation::{Motion, prepare};
        // With the assumed centreline (Z -40) all that A swings reaches
        // above the Z travel: the entry comes down from its top (machine Z0).
        let (mut doc, mesh) = chuck_setup_1_job("90");
        doc.setups[0]
            .machine
            .p
            .insert("a_centre_z".into(), "-40".into());
        doc.sync_indexed();
        let swing = doc.swing_unchecked(0, &mesh, 15.).unwrap();
        assert!(swing.machine_z > 0., "{swing:?}");
        let plan = prepare(&doc, &mesh, 0, 0).unwrap().plan;
        assert_eq!(plan.moves[0].motion, Motion::Rapid);
        let top = swing.z - swing.machine_z;
        assert!(
            (plan.moves[0].from[2] - top).abs() < 1e-3,
            "{:?} {top}",
            plan.moves[0]
        );
    }

    /// The X (setup frame, mm) of radial hole `k` of `examples/radial-holes.3mf`
    /// (`scripts/make-radial-holes.py`) as `radial_job` places the part, and
    /// the A angle that turns it up (60k° round X from +Y towards +Z).
    fn radial_hole(k: usize) -> (f32, f32) {
        (
            20. - 12.5 + 5. * k as f32,
            (90. - 60. * k as f32).rem_euclid(360.),
        )
    }
    /// A Ø40 × 40 part with six blind radial Ø4 holes (`radial_hole`) in a
    /// Ø44 bar in the chuck, its -X end at X 0, and setup 2 indexed to
    /// `angles` with one Drill of a Ø4 drill on the walls of holes `holes`.
    pub(crate) fn radial_job(angles: &str, holes: &[usize]) -> (Document, Mesh) {
        radial_doc("radial-holes", angles, ("4", "20"), 12.3, |x, _| {
            holes.iter().any(|&k| (x - radial_hole(k).0).abs() < 2.2)
        })
    }
    /// The radial-hole part `file` (`scripts/make-radial-holes.py`) in a
    /// Ø44 bar in the chuck, its -X end at X 0, setup 2 indexed to `angles`
    /// with one Drill of a drill of (diameter, flute length) `drill` on the
    /// walls of the holes `pick` takes by their X and their angle round X
    /// (degrees from +Y towards +Z at A 0), between `inner` from the bar's
    /// axis (the holes' bottom, or the tube's bore) and the skin (20).
    pub(crate) fn radial_doc(
        file: &str,
        angles: &str,
        drill: (&str, &str),
        inner: f32,
        pick: impl Fn(f32, f32) -> bool,
    ) -> (Document, Mesh) {
        use crate::model::{Operation, Strategy, Tool};
        let mesh = crate::mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("examples/{file}.3mf")),
        )
        .unwrap();
        let mut doc = bar_job(&mesh, 0., "80", "-150", angles, contour(&cutter("12")));
        let s = mesh.display_scale();
        let p = &mut doc.setups[0].p;
        p.insert("stock_diameter".into(), "44".into());
        p.insert(
            "tz".into(),
            (22. - mesh.dimensions[2] / 2. - 4. / s).to_string(),
        );
        let mut tool = Tool::new(&format!("Drill {}", drill.0), "Drill", drill.0);
        // Out of the holder by 10 more than its flutes, 30 at least.
        let stickout = drill.1.parse::<f32>().map_or(30., |f| (f + 10.).max(30.));
        for (k, v) in [
            ("length", drill.1.to_string()),
            ("stickout", stickout.to_string()),
        ] {
            tool.p.insert(k.into(), v);
        }
        // The holes' walls, as setup 1 places the part, and not a blind
        // hole's bottom, whose normal is radial.
        let placed = crate::stock::Placement::new(&mesh, Some(&doc.setups[0]));
        let stock = crate::stock::resolve(&doc.setups[0], Some(&mesh))
            .unwrap()
            .stock;
        let bar = crate::stock::round(&doc.setups[0], stock).unwrap();
        let ids = (0..mesh.surfaces.len())
            .filter(|&id| {
                let f = &mesh.surfaces[id];
                let c = placed.transform(f.center).map(|v| v / s);
                let (y, z) = (c[1] - bar.axis[0], c[2] - bar.axis[1]);
                let r = y.hypot(z);
                let radial = (f.normal[1] * y + f.normal[2] * z).abs() / r.max(1e-6);
                (inner..19.7).contains(&r)
                    && radial < 0.5
                    && pick(c[0], z.atan2(y).to_degrees().rem_euclid(360.))
            })
            .collect::<Vec<_>>();
        assert!(ids.len() >= 4, "{} walls", ids.len());
        let mut op = Operation::new(Strategy::Drill, tool.id.clone());
        op.geometry.clear();
        op.surfaces = Some(crate::surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids,
        });
        doc.tools = vec![tool];
        doc.setups[1].operations = vec![op];
        doc.sync_stock_sources();
        doc.sync_turns(Some(&mesh));
        (doc, mesh)
    }
    /// The X (setup frame) of each A angle's drilling moves.
    fn drilled_at(plan: &crate::simulation::Plan) -> Vec<(f32, f32, f32)> {
        let mut out: Vec<(f32, f32, f32)> = vec![];
        for m in plan.moves.iter().filter(|m| m.cutting_pass) {
            let a = m.a.unwrap();
            match out.iter_mut().find(|(b, _, _)| (a - b).abs() < 1e-3) {
                Some(o) => (o.1, o.2) = (o.1.min(m.to[0]), o.2.max(m.to[0])),
                None => out.push((a, m.to[0], m.to[0])),
            }
        }
        out
    }
    /// A radial hole picked by its walls is drilled in an indexed setup at
    /// the angle that turns it up, 90° or 30° (LIO-246): its walls are read
    /// as the setup turns the part, A included.
    #[test]
    fn a_radial_hole_picked_by_its_walls_is_drilled_at_its_angle() {
        for k in [0, 1] {
            let (x, a) = radial_hole(k);
            let (doc, mesh) = radial_job(&crate::indexed::degrees(a), &[k]);
            assert_eq!(doc.setups[1].turn.unwrap().a, a);
            let sim = crate::simulation::prepare(&doc, &mesh, 1, 0)
                .unwrap_or_else(|e| panic!("hole {k} at A {a}: {e}"));
            let drilled = drilled_at(&sim.plan);
            assert_eq!(drilled.len(), 1, "{drilled:?}");
            assert!(
                (drilled[0].1 - x).abs() < 0.01 && (drilled[0].2 - x).abs() < 0.01,
                "hole {k} at X {x}: {drilled:?}"
            );
            // A click on one of its walls picks the hole at that angle.
            let wall = doc.setups[1].operations[0].surfaces.as_ref().unwrap().ids[0];
            let posed = doc.picking_pose(1, &mesh, wall);
            assert!(crate::pocket::is_wall(&posed, &mesh, wall));
            assert!(crate::pocket::hole_walls(&posed, &mesh, wall).is_ok());
        }
    }
    /// One Drill drills six radial holes 60° apart in one indexed setup at
    /// A 30, 90, 150, 210, 270 and 330: each hole at the angle that turns it
    /// up, none left out (the clutch cover's holes, LIO-246). A hole that no
    /// angle turns up is refused, since nothing would drill it.
    #[test]
    fn one_drill_drills_six_radial_holes_each_at_its_own_angle() {
        let all = [0, 1, 2, 3, 4, 5];
        let (doc, mesh) = radial_job("30, 90, 150, 210, 270, 330", &all);
        let op = &doc.setups[1].operations[0];
        assert_eq!(
            surface_issues(&doc, 1, op, Some(&mesh)),
            Vec::<String>::new()
        );
        let sim = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap();
        let drilled = drilled_at(&sim.plan);
        assert_eq!(drilled.len(), 6, "{drilled:?}");
        for k in all {
            let (x, a) = radial_hole(k);
            let (_, lo, hi) = drilled
                .iter()
                .find(|(b, _, _)| (a - b).abs() < 1e-3)
                .unwrap_or_else(|| panic!("nothing drilled at A {a}: {drilled:?}"));
            assert!(
                (lo - x).abs() < 0.01 && (hi - x).abs() < 0.01,
                "hole {k} at A {a}: X {lo} to {hi}, not {x}"
            );
        }
        // A click on a hole's wall picks it at its own angle.
        for k in all {
            let (_, a) = radial_hole(k);
            let (one, _) = radial_job("30, 90, 150, 210, 270, 330", &[k]);
            let wall = one.setups[1].operations[0].surfaces.as_ref().unwrap().ids[0];
            let posed = doc.picking_pose(1, &mesh, wall);
            assert_eq!(posed.turn.unwrap().a, a, "hole {k}");
        }
        // Without A 330, hole 2 is turned up at no angle: refused.
        let (short, _) = radial_job("30, 90, 150, 210, 270", &all);
        let issues = surface_issues(&short, 1, &short.setups[1].operations[0], Some(&mesh));
        assert!(
            issues
                .iter()
                .any(|i| i.starts_with("1 of the picked holes is drilled at none of this setup's A angles (A 30°, 90°, 150°, 210°, 270°)")),
            "{issues:?}"
        );
    }
    /// What `tube_drill` finds: each angle and how many picked surfaces it
    /// cuts, how many picked holes none drills, and each drilling angle and
    /// its drill tip over the A centreline.
    type TubeDrill = (Vec<(f32, usize)>, usize, Vec<(f32, f32)>);
    /// The tube `file` with a Drill of a Ø3.3 drill with `flutes` mm of
    /// flutes on the holes `pick` takes (by their angle round X at A 0), at
    /// `angles`, the Drill edited by `edit`.
    fn tube_job(
        file: &str,
        angles: &str,
        flutes: &str,
        pick: impl Fn(f32) -> bool,
        edit: impl Fn(&mut crate::model::Operation),
    ) -> (Document, Mesh) {
        let (mut doc, mesh) = radial_doc(file, angles, ("3.3", flutes), 14.3, |x, t| {
            (x - 20.).abs() < 2.2 && pick(t)
        });
        edit(&mut doc.setups[1].operations[0]);
        (doc, mesh)
    }
    /// `tube_job`'s Drill, unedited: what each angle cuts, how many picked
    /// holes none drills, and each angle's drill tip under the A centreline
    /// (mm, over it when positive). Its surface issues must be none.
    fn tube_drill(file: &str, angles: &str, flutes: &str, pick: impl Fn(f32) -> bool) -> TubeDrill {
        let (doc, mesh) = tube_job(file, angles, flutes, pick, |_| {});
        let op = &doc.setups[1].operations[0];
        assert_eq!(
            surface_issues(&doc, 1, op, Some(&mesh)),
            Vec::<String>::new()
        );
        let (cuts, left) = holes_by_angle(&doc, 1, op, &mesh).unwrap();
        let sim = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap();
        (
            cuts.iter().map(|c| (c.a, c.ids.len())).collect(),
            left,
            tips(&doc, &sim.plan),
        )
    }
    /// Each angle that drills in `plan` (setup 2 of `doc`) and its drill's
    /// lowest point over the A centreline, at the holes' X.
    fn tips(doc: &Document, plan: &crate::simulation::Plan) -> Vec<(f32, f32)> {
        let axis = doc.setups[1].turn.unwrap().axis;
        (drilled_at(plan).into_iter())
            .map(|(a, lo, hi)| {
                assert!((lo - 20.).abs() < 0.01 && (hi - 20.).abs() < 0.01, "A {a}");
                let low = (plan.moves.iter())
                    .filter(|m| m.cutting_pass && m.a == Some(a))
                    .map(|m| m.to[2])
                    .fold(f32::INFINITY, f32::min);
                (a, low - axis[1])
            })
            .collect()
    }
    /// Whether picked angle round X `t` (degrees, at A 0) is within 15° of
    /// one of `holes`'.
    fn near(t: f32, holes: &[f32]) -> bool {
        holes
            .iter()
            .any(|h| ((t - h + 540.).rem_euclid(360.) - 180.).abs() < 15.)
    }
    /// (a) At A 30 and 90, the A 30 hole alone picked of a tube whose holes
    /// face each other across its bore (a long drill, 60 mm of flutes, for
    /// the stock's top to under both walls): drilled through both walls, out
    /// of the hole opposite, which no angle of the Drill has, as at A 30
    /// alone and as main drills it; A 90 cuts nothing for it (LIO-246).
    #[test]
    fn a_tube_hole_over_a_hole_opposite_is_drilled_through_both_walls() {
        let (cuts, left, tips) = tube_drill("radial-tube", "30, 90", "60", |t| near(t, &[60.]));
        assert_eq!(left, 0);
        assert!(cuts[0].1 > 0 && cuts[1].1 == 0, "{cuts:?}");
        let (_, _, alone) = tube_drill("radial-tube", "30", "60", |t| near(t, &[60.]));
        assert_eq!(tips.len(), 1);
        assert!((tips[0].1 - alone[0].1).abs() < 0.01, "{tips:?} {alone:?}");
        // Out of the far wall, 20 under the axis.
        assert!(tips[0].1 < -19.99, "{tips:?}");
    }
    /// (b) At A 30, 90 and 150, all six of the tube's holes picked: three
    /// plunges, each through a hole and the one opposite (which none of the
    /// three angles turns up), drilling that one too: nothing refused (a
    /// long drill; LIO-246, review 5).
    #[test]
    fn three_angles_drill_a_tubes_six_holes_in_pairs_through_both_walls() {
        let (cuts, left, tips) = tube_drill("radial-tube", "30, 90, 150", "60", |_| true);
        assert_eq!(left, 0);
        assert!(cuts.iter().all(|(_, n)| *n > 0), "{cuts:?}");
        assert_eq!(tips.len(), 3, "{tips:?}");
        assert!(tips.iter().all(|(_, t)| *t < -19.99), "{tips:?}");
    }
    /// (e) (b) with a Bottom offset of +25, which stops the plunges short of
    /// the far holes: those are not drilled, and the Drill says so instead
    /// of leaving them undrilled unsaid (LIO-246, review 5).
    #[test]
    fn a_plunge_short_of_the_hole_opposite_does_not_drill_it() {
        let (doc, mesh) = tube_job(
            "radial-tube",
            "30, 90, 150",
            "60",
            |_| true,
            |o| {
                o.p.insert("bottom".into(), "25".into());
            },
        );
        let op = &doc.setups[1].operations[0];
        let (_, left) = holes_by_angle(&doc, 1, op, &mesh).unwrap();
        assert_eq!(left, 3);
        let issues = surface_issues(&doc, 1, op, Some(&mesh));
        assert!(
            issues.iter().any(|i| i.starts_with(
                "3 of the picked holes are drilled at none of this setup's A angles (A 30°, 90°, 150°)"
            )),
            "{issues:?}"
        );
    }
    /// The far side is open to a drill no wider than the hole opposite (Ø4)
    /// and not to a wider one: it is probed at the drill's radius, not the
    /// hole's, so a Ø4.4 drill over the Ø4 hole opposite stops through its
    /// own wall (LIO-246, review 5).
    #[test]
    fn the_far_side_is_open_to_a_drill_no_wider_than_the_hole_opposite() {
        for (drill, open) in [("3.3", true), ("4", true), ("4.4", false)] {
            let (doc, mesh) = radial_doc("radial-tube", "30, 90", (drill, "60"), 14.3, |x, t| {
                (x - 20.).abs() < 2.2 && near(t, &[60.])
            });
            let op = &doc.setups[1].operations[0];
            let (cuts, _) = holes_by_angle(&doc, 1, op, &mesh).unwrap();
            assert_eq!(cuts[0].own_wall.is_empty(), open, "Ø{drill}");
            assert_eq!(cuts[0].across, open, "Ø{drill}");
        }
    }
    /// (c) At all six angles, all six of the tube's holes picked: each hole
    /// at its own angle, drilled from its own side just through its own
    /// wall, as the hole opposite is drilled from its side, so that a short
    /// drill (20 mm of flutes) does: six plunges, none twice (LIO-246,
    /// review 5).
    #[test]
    fn six_angles_drill_each_of_a_tubes_holes_from_its_own_side() {
        let (cuts, left, tips) =
            tube_drill("radial-tube", "30, 90, 150, 210, 270, 330", "20", |_| true);
        assert_eq!(left, 0);
        assert!(cuts.iter().all(|(_, n)| *n > 0), "{cuts:?}");
        assert_eq!(tips.len(), 6, "{tips:?}");
        // Through the wall's inner face (14 over the axis), well over the
        // far wall.
        assert!(tips.iter().all(|(_, t)| *t > 9. && *t < 13.9), "{tips:?}");
        // At three of the angles a plunge across the bore would want 60 mm
        // of flutes; with a 20 mm drill the Drill says to drill each hole
        // from its own side.
        let (doc, mesh) = tube_job("radial-tube", "30, 90, 150", "20", |_| true, |_| {});
        let e = crate::simulation::prepare(&doc, &mesh, 1, 0).err().unwrap();
        assert!(e.contains(ACROSS) && e.contains("(A 210°)"), "{e}");
    }
    /// (d) A tube whose holes face solid wall across its bore: each drilled
    /// through its own wall only, not into the far wall (LIO-246).
    #[test]
    fn a_tube_hole_over_solid_wall_is_drilled_through_its_own_wall_only() {
        let (cuts, left, tips) = tube_drill("radial-tube-half", "30, 90, 330", "20", |t| {
            near(t, &[0., 60., 120.])
        });
        assert_eq!(left, 0);
        assert!(cuts.iter().all(|(_, n)| *n > 0), "{cuts:?}");
        assert_eq!(tips.len(), 3, "{tips:?}");
        // Through the wall's inner face (14 over the axis), well over the
        // far wall's (14 under it).
        assert!(tips.iter().all(|(_, t)| *t > 9. && *t < 13.9), "{tips:?}");
    }
    /// An indexed setup's angles serve its other operations too: a Drill
    /// whose holes one angle turns up skips the angles that turn none up,
    /// instead of being refused (LIO-246, review 2).
    #[test]
    fn a_drill_skips_the_angles_that_turn_none_of_its_holes_up() {
        let (doc, mesh) = radial_job("0, 90", &[0]);
        let op = &doc.setups[1].operations[0];
        assert_eq!(
            surface_issues(&doc, 1, op, Some(&mesh)),
            Vec::<String>::new()
        );
        let sim = crate::simulation::prepare(&doc, &mesh, 1, 0).unwrap();
        let drilled = drilled_at(&sim.plan);
        assert_eq!(drilled.len(), 1, "{drilled:?}");
        assert_eq!(drilled[0].0, 90.);
        assert!(sim.plan.moves.iter().all(|m| m.a != Some(0.)));
        // With no angle that turns its hole up it is refused.
        let (none, _) = radial_job("0, 180", &[0]);
        let op = &none.setups[1].operations[0];
        let issues = surface_issues(&none, 1, op, Some(&mesh));
        assert!(
            issues[0].starts_with("1 of the picked holes is drilled at none"),
            "{issues:?}"
        );
        let e = crate::simulation::prepare(&none, &mesh, 1, 0)
            .err()
            .unwrap();
        assert!(e.contains(NO_HOLE_UP), "{e}");
    }
    /// The Geometry tab lists a Drill at several angles as each angle reads
    /// its holes: all six of the six-hole job, which the setup as shown
    /// (its first angle) cannot read (LIO-246, review 2).
    #[test]
    fn a_drill_at_several_angles_lists_each_hole_as_its_angle_reads_it() {
        let (doc, mesh) = radial_job("30, 90, 150, 210, 270, 330", &[0, 1, 2, 3, 4, 5]);
        let op = &doc.setups[1].operations[0];
        assert!(crate::cam::holes(&doc.setups[1], op, &mesh).is_err());
        let listed = doc
            .holes_listed(1, op, &mesh, |posed, op| {
                crate::cam::holes(posed, op, &mesh)
            })
            .unwrap()
            .unwrap();
        let mut xs: Vec<f32> = listed.iter().map(|(_, h)| h.center[0]).collect();
        xs.sort_by(f32::total_cmp);
        assert_eq!(xs.len(), 6, "{xs:?}");
        for (k, x) in xs.iter().enumerate() {
            assert!((x - radial_hole(k).0).abs() < 0.01, "{xs:?}");
        }
        let walls = doc
            .holes_listed(1, op, &mesh, |posed, op| {
                let ids = (op.surfaces.iter()).flat_map(|s| s.ids.iter().copied());
                crate::pocket::holes(posed, &ids.collect(), &mesh)
            })
            .unwrap()
            .unwrap();
        assert_eq!(walls.len(), 6);
        // Not for a setup of one angle: the setup as it is reads it.
        let (one, _) = radial_job("90", &[0]);
        let op = &one.setups[1].operations[0];
        assert!(
            one.holes_listed(1, op, &mesh, |_, _| Ok(vec![()]))
                .is_none()
        );
    }
    /// The threaded plate, a box in a 4-jaw chuck on the rotary unit: drilled
    /// Ø4.5 and then thread milled by an indexed setup at A0, the thread
    /// milling planning its cutting moves at that angle.
    #[test]
    fn thread_milling_runs_in_an_indexed_setup() {
        use crate::model::{Operation, Strategy, Tool};
        let mesh = crate::mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/threaded-plate.3mf"),
        )
        .unwrap();
        let (mut doc, _) = job("0");
        let machine = {
            let m = &mut doc.setups[0].machine;
            for (k, v) in [
                ("a_centre_z", "-150"),
                ("rotary_min", "-360"),
                ("rotary_max", "360"),
            ] {
                m.p.insert(k.into(), v.into());
            }
            m.clone()
        };
        let s = mesh.display_scale();
        let len = mesh.dimensions[0];
        let p = &mut doc.setups[0].p;
        for (k, v) in [
            ("stock_type", "Fixed size box"),
            ("stock_x", "72"),
            ("stock_y", "42"),
            ("stock_z", "14"),
            ("chuck_jaws", "4-jaw"),
            ("chuck_stickout", "60"),
        ] {
            p.insert(k.into(), v.into());
        }
        p.insert("tx".into(), (1. + len / 2.).to_string());
        p.insert(
            "tz".into(),
            (20. - mesh.dimensions[2] / 2. - 4. / s).to_string(),
        );
        // The Ø4.5 drill, then the Ø3.15 thread mill, on the top's holes.
        let mut drill_tool = Tool::new("Tap drill", "Drill", "4.5");
        let mut thread_tool = Tool::new("Thread mill", "Thread mill", "3.15");
        thread_tool.set_type("Thread mill");
        for (k, v) in [
            ("pitches", "0.5-1.25"),
            ("reach", "16"),
            ("length", "1"),
            ("stickout", "40"),
        ] {
            thread_tool.p.insert(k.into(), v.into());
        }
        drill_tool.p.insert("length".into(), "40".into());
        drill_tool.p.insert("stickout".into(), "45".into());
        for t in [&mut drill_tool, &mut thread_tool] {
            t.p.insert("machines".into(), machine.id.clone());
        }
        let top = (mesh.surfaces.iter())
            .filter(|f| f.normal[2].abs() > 0.999999)
            .map(|f| f.center[2])
            .fold(f32::NEG_INFINITY, f32::max);
        let ids: Vec<usize> = (mesh.surfaces.iter().enumerate())
            .filter(|(_, f)| f.normal[2].abs() > 0.999999 && (f.center[2] - top).abs() < 0.001)
            .map(|(i, _)| i)
            .collect();
        let mut drill = Operation::new(Strategy::Drill, drill_tool.id.clone());
        drill.geometry.clear();
        drill.surfaces = Some(crate::surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids,
        });
        for (k, v) in [
            ("hole_diameter", "4.9"),
            ("through", "Yes"),
            ("breakthrough", "1.5"),
            ("rpm", "640"),
            ("feed", "60"),
        ] {
            drill.p.insert(k.into(), v.into());
        }
        let mut thread = drill.clone();
        thread.id = crate::model::id();
        thread.tool = thread_tool.id.clone();
        for k in ["through", "breakthrough"] {
            thread.p.remove(k);
        }
        thread
            .p
            .insert("cycle".into(), crate::model::THREAD_MILLING.into());
        let thread_id = thread.id.clone();
        doc.tools = vec![drill_tool, thread_tool];
        doc.machines = vec![machine];
        doc.setups[1].operations = vec![drill, thread];
        doc.sync_stock_sources();
        doc.sync_turns(Some(&mesh));
        let mut runs = vec![];
        crate::simulation::sequence_with(
            &doc,
            &mesh,
            None,
            |si, oi, r| runs.push((si, oi, r)),
            |_, _, _| {},
        );
        // The last operation of setup 2 is the thread milling.
        let (_, oi, threading) = runs.iter().rfind(|(si, _, _)| *si == 1).unwrap();
        assert_eq!(doc.setups[1].operations[*oi].id, thread_id);
        let plan = &threading
            .as_ref()
            .unwrap_or_else(|e| panic!("thread milling in the indexed setup: {e}"))
            .plan;
        let cuts = (plan.moves.iter())
            .filter(|m| m.cutting_pass && m.a == Some(0.))
            .count();
        assert!(cuts > 100, "{cuts} cutting moves at A0");
    }
}
