//! Bounded 2.5D pocket preview from imported planar floor triangles.
//! Offset contours preserve holes/islands. No machine postprocessor or fixture solver.
use crate::{
    mesh::Mesh,
    model::{
        Operation, Setup, Strategy, Tool, facing_setting, get, num, pocket_setting,
        pocket_setting_issues,
    },
    simulation::{Motion, Move, Plan},
    stock::{self, Placement},
    surface,
};
use clipper2::{EndType, FillRule, JoinType, Milli, Paths, intersect, union};

type Polygon = Vec<(f64, f64)>;
type Region = Paths<Milli>;
const MAX_MOVES: usize = 30_000;
fn area(p: &Polygon) -> f64 {
    p.iter()
        .zip(p.iter().cycle().skip(1))
        .take(p.len())
        .map(|(a, b)| a.0 * b.1 - b.0 * a.1)
        .sum::<f64>()
        / 2.
}
fn region(triangles: Vec<Polygon>) -> Result<Region, String> {
    union::<Milli>(triangles, Vec::<Polygon>::new(), FillRule::NonZero)
        .map_err(|e| format!("Could not join pocket floor triangles: {e}"))
}
fn number(op: &Operation, key: &str) -> Result<f32, String> {
    num(&op.p, key).ok_or_else(|| format!("Enter a finite {} value.", key.replace('_', " ")))
}
fn positive(tool: &Tool, key: &str) -> Result<f32, String> {
    num(&tool.p, key)
        .filter(|v| *v > 0.)
        .ok_or_else(|| format!("Enter a positive tool {key}."))
}
/// Clip a triangle to the half-space above the floor before projecting it.
fn above(points: &[[f32; 3]], z: f32) -> Polygon {
    let mut result = Vec::new();
    for (a, b) in points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
    {
        if a[2] >= z {
            result.push((a[0] as f64, a[1] as f64));
        }
        if (a[2] >= z) != (b[2] >= z) {
            let t = (z - a[2]) / (b[2] - a[2]);
            result.push((
                (a[0] + t * (b[0] - a[0])) as f64,
                (a[1] + t * (b[1] - a[1])) as f64,
            ));
        }
    }
    result
}
fn push(
    plan: &mut Plan,
    position: &mut [f32; 3],
    to: [f32; 3],
    motion: Motion,
    rate: f32,
    cutting: bool,
) -> Result<(), String> {
    if plan.moves.len() >= MAX_MOVES {
        return Err(
            "Pocket preview exceeds 30,000 moves. Increase stepover, stepdown or ramp angle."
                .into(),
        );
    }
    let distance = (0..3)
        .map(|i| (position[i] - to[i]).powi(2))
        .sum::<f32>()
        .sqrt();
    if distance > 0.00001 {
        let end = plan.duration + distance / rate * 60.;
        plan.moves.push(Move {
            from: *position,
            to,
            motion,
            cutting_pass: cutting,
            start: plan.duration,
            end,
        });
        plan.duration = end;
    }
    *position = to;
    Ok(())
}

pub fn generate(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    if !op.enabled {
        return Err("Enable this Pocket operation before simulation.".into());
    }
    if let Some(issue) = pocket_setting_issues(&op.p).first() {
        return Err(issue.clone());
    }
    if let Some(issue) = surface::issues(op, setup, Some(mesh)).first() {
        return Err(issue.clone());
    }
    let selection = op
        .surfaces
        .as_ref()
        .filter(|s| !s.ids.is_empty())
        .ok_or("Select a recessed planar pocket floor in Geometry first.")?;
    if get(&tool.p, "type") != "Flat end mill" {
        return Err("Pocket preview requires a flat end mill.".into());
    }
    if get(&setup.p, "stock_source") != "fresh" {
        return Err("Pocket preview currently uses fresh stock. Stock transfer between setups is not implemented.".into());
    }
    if get(&op.p, "boundary") != "Selected geometry"
        || get(&op.p, "containment") != "Tool inside boundary"
    {
        return Err("Pocket preview requires Selected geometry with Tool inside boundary. Its walls and islands come from the selected floor.".into());
    }
    let stock = stock::resolve(setup, Some(mesh))?;
    let placement = Placement::new(mesh, Some(setup));
    let scale = stock.display_scale;
    let ids: std::collections::HashSet<_> = selection.ids.iter().copied().collect();
    let reference = mesh
        .faces
        .iter()
        .find(|f| f.feature.is_some_and(|id| ids.contains(&id)))
        .and_then(|f| f.points.first())
        .ok_or("The selected floor contains no triangles.")?;
    let plane = placement.transform(*reference)[2] / scale;
    if plane >= placement.bounds.max[2] / scale - 0.001 {
        return Err("Select a recessed pocket floor. Use Facing for the top surface.".into());
    }
    let axial = number(op, "axial_allowance")?;
    let radial = number(op, "allowance")?;
    let target = plane + number(op, "bottom")? + axial;
    let top = stock.stock.max[2].max(stock.stock.max[2] + number(op, "top")?);
    if axial < 0. || radial < 0. || target < plane - 0.0001 {
        return Err("Pocket target cannot cut below the selected floor; use nonnegative stock allowances and review Bottom offset.".into());
    }
    if target < stock.stock.min[2] || target >= stock.stock.max[2] - 0.0001 {
        return Err("No pocket stock exists above the requested target. Review stock and bottom/axial offsets.".into());
    }
    let radius = positive(tool, "diameter")? / 2.;
    let flute_length = positive(tool, "length")?;
    let tool_length = positive(tool, "stickout")?;
    let holder_radius = positive(tool, "holder")? / 2.;
    if tool_length < flute_length || top - target >= tool_length {
        return Err("Pocket depth must be less than tool stickout, with stickout at least the flute length. Choose a tool with adequate reach.".into());
    }
    let feed = number(op, "feed")?;
    let stepdown = number(op, "stepdown")?;
    let stepover = number(op, "stepover")? / 100. * radius * 2.;
    if feed <= 0.
        || stepdown <= 0.
        || stepdown > flute_length
        || stepover <= 0.
        || stepover > radius
    {
        return Err("Use positive feed/step sizes, stepdown within flute length, and Pocket stepover up to 50% for overlapping offset contours.".into());
    }
    let clearance = top + number(op, "clearance")?;
    let retract = top + number(op, "retract")?;
    if retract <= top || clearance < retract {
        return Err("Set positive retract and clearance at or above retract.".into());
    }
    let finish = number(op, "finish_passes")?;
    if !(0. ..=20.).contains(&finish) || finish.fract() != 0. {
        return Err("Finishing passes must be a whole number from 0 to 20.".into());
    }
    let ramp = pocket_setting(&op.p, "pocket_entry") == "Profile ramp";
    let ramp_slope = if ramp {
        let angle = number(op, "ramp_angle")?;
        if !(0.1..=15.).contains(&angle) {
            return Err("Pocket profile ramp angle must be between 0.1° and 15°.".into());
        }
        angle.to_radians().tan()
    } else {
        0.
    };

    let mut floors = Vec::new();
    let mut obstacles = Vec::new();
    for face in &mesh.faces {
        let points: Vec<_> = face
            .points
            .iter()
            .map(|p| placement.transform(*p).map(|v| v / scale))
            .collect();
        let selected = face.feature.is_some_and(|id| ids.contains(&id));
        if selected && points.iter().any(|p| (p[2] - plane).abs() > 0.002) {
            return Err("Pocket preview needs a flat floor perpendicular to setup Z. Reposition the part or select another floor.".into());
        }
        let mut p: Polygon = if selected {
            points.iter().map(|p| (p[0] as f64, p[1] as f64)).collect()
        } else {
            above(&points, plane + 0.01)
        };
        let a = area(&p);
        if a.abs() < 0.000001 {
            continue;
        }
        if a < 0. {
            p.reverse();
        }
        if selected {
            floors.push(p);
        } else {
            obstacles.push(p);
        }
    }
    let floor = region(floors)?;
    if !floor.contains_points() {
        return Err("The selected floor has no usable XY area.".into());
    }
    // Reject overhanging/tapered geometry rather than assuming vertical access.
    let blocked = intersect::<Milli>(floor.clone(), region(obstacles)?, FillRule::NonZero)
        .map_err(|e| e.to_string())?;
    if blocked.signed_area().abs() > 0.05 {
        return Err("Geometry above this floor obstructs vertical access. Pocket preview supports open, vertical-wall 2.5D pockets; choose another floor or setup.".into());
    }
    // A small conservative margin covers integer rounding and offset arc chords.
    let margin = (radius * 0.01).max(0.02);
    let inset = (radius + radial + margin) as f64;
    let boundary = floor
        .inflate(-inset, JoinType::Round, EndType::Polygon, 2.)
        .simplify(0.001, false);
    if !boundary.contains_points() {
        return Err("The selected pocket is too narrow for this tool and radial allowance. Choose a smaller cutter.".into());
    }
    let profile_only = op.strategy == Strategy::Contour;
    let mut rings: Vec<Vec<Polygon>> = Vec::new();
    for ring in 0..=if profile_only { 0 } else { 256 } {
        let paths = floor
            .inflate(
                -(inset + ring as f64 * stepover as f64),
                JoinType::Round,
                EndType::Polygon,
                2.,
            )
            .simplify(0.001, false);
        if !paths.contains_points() {
            break;
        }
        if ring == 256 {
            return Err(
                "Pocket preview exceeds 256 offset rings. Increase stepover or reduce pocket size."
                    .into(),
            );
        }
        rings.push(paths.into());
    }
    let mut envelope = crate::mesh::Bounds {
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
    };
    for point in rings.iter().flatten().flatten() {
        for (k, v) in [point.0 as f32, point.1 as f32].into_iter().enumerate() {
            envelope.min[k] = envelope.min[k].min(v - radius);
            envelope.max[k] = envelope.max[k].max(v + radius);
        }
    }
    let cutting_top = crate::rest::start(setup, envelope, target, top, number(op, "top")?);
    let approach = (cutting_top + number(op, "retract")?).min(retract);
    let levels = ((cutting_top - target - 0.0001) / stepdown).ceil().max(1.) as usize;
    let segments: usize = rings.iter().flatten().map(|p| p.len() + 5).sum();
    if levels > 1000 || segments.saturating_mul(levels) > MAX_MOVES {
        return Err("Pocket preview exceeds its move limit. Increase stepdown or stepover.".into());
    }
    let clockwise = facing_setting(&op.p, "spindle_rotation") == "Clockwise (CW)";
    let climb = pocket_setting(&op.p, "pocket_milling") == "Climb";
    // Clipper outer boundaries are CCW and islands CW (free area on left).
    // CW climb keeps remaining material on the right of cutter travel.
    if clockwise != climb {
        for p in rings.iter_mut().flatten() {
            p.reverse();
        }
    }
    let finishing = rings[0].clone();
    let loops: Vec<_> = rings
        .into_iter()
        .rev()
        .flatten()
        .chain((0..finish as usize).flat_map(|_| finishing.clone()))
        .collect();
    let mut plan = Plan {
        shape: crate::simulation::CutterShape::Flat,
        strategy: op.strategy,
        operation: op.id.clone(),
        setup: setup.id.clone(),
        moves: vec![],
        stock: stock.stock,
        radius,
        flute_length,
        tool_length,
        holder_radius,
        display_scale: scale,
        duration: 0.,
        target,
        levels,
        rest_top: (cutting_top < top - 0.0001).then_some(cutting_top),
        spindle_clockwise: clockwise,
        milling_direction: if climb { "Climb" } else { "Conventional" },
    };
    let mut position = [loops[0][0].0 as f32, loops[0][0].1 as f32, clearance];
    for level in 1..=levels {
        let z = (cutting_top - level as f32 * stepdown).max(target);
        let previous_z = (cutting_top - (level - 1) as f32 * stepdown).max(target);
        for path in &loops {
            let start = [path[0].0 as f32, path[0].1 as f32];
            push(
                &mut plan,
                &mut position,
                [start[0], start[1], clearance],
                Motion::Rapid,
                6000.,
                false,
            )?;
            push(
                &mut plan,
                &mut position,
                [start[0], start[1], approach],
                Motion::Rapid,
                6000.,
                false,
            )?;
            push(
                &mut plan,
                &mut position,
                [start[0], start[1], if ramp { previous_z } else { z }],
                Motion::Feed,
                feed * 0.25,
                false,
            )?;
            if ramp {
                while position[2] > z + 0.00001 {
                    for point in path.iter().skip(1).chain(std::iter::once(&path[0])) {
                        let xy = [point.0 as f32, point.1 as f32];
                        let distance =
                            ((xy[0] - position[0]).powi(2) + (xy[1] - position[1]).powi(2)).sqrt();
                        let next_z = (position[2] - distance * ramp_slope).max(z);
                        push(
                            &mut plan,
                            &mut position,
                            [xy[0], xy[1], next_z],
                            Motion::Feed,
                            feed * 0.25,
                            true,
                        )?;
                    }
                }
            }
            for point in path.iter().skip(1).chain(std::iter::once(&path[0])) {
                push(
                    &mut plan,
                    &mut position,
                    [point.0 as f32, point.1 as f32, z],
                    Motion::Feed,
                    feed,
                    true,
                )?;
            }
            push(
                &mut plan,
                &mut position,
                [start[0], start[1], clearance],
                Motion::Rapid,
                6000.,
                false,
            )?;
        }
    }
    if plan.moves.is_empty() || !plan.duration.is_finite() {
        return Err("No finite pocket motion could be generated.".into());
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mesh, model::Machine, simulation::Simulation};
    fn quad(tris: &mut Vec<[[f32; 3]; 3]>, x: f32, y: f32, xx: f32, yy: f32, z: f32) {
        tris.push([[x, y, z], [xx, y, z], [xx, yy, z]]);
        tris.push([[x, y, z], [xx, yy, z], [x, yy, z]]);
    }
    fn fixture() -> (Setup, Operation, Tool, Mesh) {
        let mut tris = Vec::new();
        // Floor ring, central island, enclosing rim and lower model bound.
        for (x, y, xx, yy) in [
            (-10., -8., 10., -2.),
            (-10., 2., 10., 8.),
            (-10., -2., -2., 2.),
            (2., -2., 10., 2.),
        ] {
            quad(&mut tris, x, y, xx, yy, 0.);
        }
        for (x, y, xx, yy) in [
            (-15., -12., 15., -8.),
            (-15., 8., 15., 12.),
            (-15., -8., -10., 8.),
            (10., -8., 15., 8.),
            (-2., -2., 2., 2.),
        ] {
            quad(&mut tris, x, y, xx, yy, 6.);
        }
        quad(&mut tris, -15., -12., 15., 12., -4.);
        let mesh = mesh::normalize(tris).unwrap();
        let mut setup = Setup::new(Machine::new(3), "G54");
        setup
            .p
            .insert("stock_type".into(), "Relative size box".into());
        for (k, _) in stock::OFFSETS {
            setup.p.insert(k.into(), "0".into());
        }
        let mut tool = Tool::new("Pocket test cutter", "Flat end mill", "2");
        tool.p.insert("diameter".into(), "2".into());
        let mut op = Operation::new(Strategy::Pocket, tool.id.clone());
        op.geometry.clear();
        let center_z = mesh.surfaces[0].center[2];
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: mesh
                .surfaces
                .iter()
                .enumerate()
                .filter(|(_, s)| (s.center[2] - center_z).abs() < 0.001)
                .map(|(i, _)| i)
                .collect(),
        });
        for (k, v) in [
            ("allowance", "0"),
            ("axial_allowance", "0"),
            ("stepover", "40"),
            ("pocket_entry", "Plunge"),
            ("finish_passes", "0"),
        ] {
            op.p.insert(k.into(), v.into());
        }
        (setup, op, tool, mesh)
    }
    fn loops(plan: &Plan) -> Vec<Polygon> {
        let mut loops = Vec::new();
        let mut current = Vec::new();
        for m in &plan.moves {
            if m.cutting_pass {
                if current.is_empty() {
                    current.push((m.from[0] as f64, m.from[1] as f64));
                }
                current.push((m.to[0] as f64, m.to[1] as f64));
            } else if !current.is_empty() {
                loops.push(std::mem::take(&mut current));
            }
        }
        loops
    }
    #[test]
    fn inside_contour_follows_walls_without_clearing_the_pocket_interior() {
        let (setup, mut op, tool, mesh) = fixture();
        let pocket = generate(&setup, &op, &tool, &mesh).unwrap();
        op.strategy = Strategy::Contour;
        op.p.insert("side".into(), "Inside".into());
        let plan = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(plan.strategy, Strategy::Contour);
        assert!(plan.moves.len() < pocket.moves.len());
        assert_eq!(plan.target, pocket.target);
        let mut contour_sim = Simulation::new(plan.clone());
        contour_sim.seek(1.);
        let mut pocket_sim = Simulation::new(pocket);
        pocket_sim.seek(1.);
        assert!(contour_sim.stock.removed_volume() < pocket_sim.stock.removed_volume());
        let paths = loops(&plan);
        assert!(paths.iter().any(|p| area(p) > 0.) && paths.iter().any(|p| area(p) < 0.));
        op.p.insert("pocket_milling".into(), "Conventional".into());
        let reversed = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
        for (a, b) in paths.iter().zip(loops(&reversed)) {
            assert!((area(a) + area(&b)).abs() < 0.001);
        }
        op.surfaces = None;
        assert!(crate::simulation::generate(&setup, &op, &tool, &mesh).is_err());
    }
    #[test]
    fn pocket_preserves_walls_island_and_floor_and_removes_known_volume() {
        let (setup, op, tool, mesh) = fixture();
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(plan.levels, 3);
        for pair in plan.moves.windows(2) {
            assert_eq!(pair[0].to, pair[1].from);
            assert_eq!(pair[0].end, pair[1].start);
        }
        for m in &plan.moves {
            if m.motion == Motion::Rapid && m.from[..2] != m.to[..2] {
                assert!(m.from[2] > plan.stock.max[2] && m.to[2] > plan.stock.max[2]);
            }
            if m.motion == Motion::Feed {
                for i in 0..=10 {
                    let p = m.at(m.start + (m.end - m.start) * i as f32 / 10.);
                    assert!(
                        p[0].abs() + plan.radius <= 10. + 0.001
                            && p[1].abs() + plan.radius <= 8. + 0.001
                    );
                    let dx = (p[0].abs() - 2.).max(0.);
                    let dy = (p[1].abs() - 2.).max(0.);
                    assert!(dx * dx + dy * dy >= plan.radius * plan.radius);
                    assert!(p[2] >= plan.target - 0.001);
                }
            }
        }
        let mut sim = Simulation::new(plan);
        sim.seek(1.);
        let expected = (20. * 16. - 4. * 4.) * 6.;
        assert!(
            (sim.stock.removed_volume() - expected).abs() / expected < 0.04,
            "{} vs {expected}",
            sim.stock.removed_volume()
        );
        for y in 0..sim.stock.ny {
            for x in 0..sim.stock.nx {
                let px = sim.stock.bounds.min[0] + (x as f32 + 0.5) * sim.stock.dx;
                let py = sim.stock.bounds.min[1] + (y as f32 + 0.5) * sim.stock.dy;
                if px.abs() > 10. || py.abs() > 8. || (px.abs() < 2. && py.abs() < 2.) {
                    assert_eq!(
                        sim.stock.heights[y * sim.stock.nx + x],
                        sim.stock.bounds.max[2]
                    );
                }
            }
        }
    }
    #[test]
    fn pocket_direction_and_spindle_reverse_loops_and_keep_final_stock_equal() {
        let (setup, mut op, tool, mesh) = fixture();
        let cw = generate(&setup, &op, &tool, &mesh).unwrap();
        let paths = loops(&cw);
        assert!(paths.iter().any(|p| area(p) > 0.) && paths.iter().any(|p| area(p) < 0.));
        op.p.insert("pocket_milling".into(), "Conventional".into());
        let conventional = generate(&setup, &op, &tool, &mesh).unwrap();
        for (a, b) in paths.iter().zip(loops(&conventional)) {
            assert!((area(a) + area(&b)).abs() < 0.001);
        }
        op.p.insert("spindle_rotation".into(), "Counterclockwise (CCW)".into());
        let ccw = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(loops(&ccw), paths);
        let mut expected = None;
        for plan in [cw, conventional, ccw] {
            let mut sim = Simulation::new(plan);
            sim.seek(1.);
            if let Some(ref stock) = expected {
                assert_eq!(&sim.stock.heights, stock);
            } else {
                expected = Some(sim.stock.heights.clone());
            }
        }
    }
    #[test]
    fn profile_ramp_obeys_angle_and_rewind_matches_incremental_playback() {
        let (setup, mut op, tool, mesh) = fixture();
        op.p.insert("pocket_entry".into(), "Profile ramp".into());
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!(
            plan.moves
                .iter()
                .any(|m| m.cutting_pass && m.to[2] < m.from[2])
        );
        for m in plan.moves.iter().filter(|m| m.cutting_pass) {
            let xy = ((m.to[0] - m.from[0]).powi(2) + (m.to[1] - m.from[1]).powi(2)).sqrt();
            assert!((m.to[2] - m.from[2]).abs() <= xy * 2_f32.to_radians().tan() + 0.0001);
        }
        let mut a = Simulation::new(plan.clone());
        let mut b = Simulation::new(plan);
        a.seek(0.5);
        for i in 0..=50 {
            b.seek(i as f32 / 100.);
        }
        for (x, y) in a.stock.heights.iter().zip(&b.stock.heights) {
            assert!((x - y).abs() < 0.001);
        }
        let partial = a.stock.heights.clone();
        a.seek(1.);
        a.seek(0.5);
        assert_eq!(partial, a.stock.heights);
        a.seek(0.);
        assert_eq!(a.stock.removed_volume(), 0.);
    }
    #[test]
    fn pocket_rejects_stale_geometry_bad_targets_tools_and_parameters() {
        let (setup, op, tool, mesh) = fixture();
        for (key, value, expected) in [
            ("bottom", "-1", "below"),
            ("stepover", "100", "50%"),
            ("stepdown", "0.000001", "limit"),
            ("pocket_milling", "unknown", "valid"),
            ("pocket_entry", "Helix", "valid"),
            ("containment", "Tool outside boundary", "inside"),
            ("boundary", "Stock silhouette", "Selected"),
            ("finish_passes", "0.5", "whole"),
        ] {
            let mut bad = op.clone();
            bad.p.insert(key.into(), value.into());
            assert!(
                generate(&setup, &bad, &tool, &mesh)
                    .unwrap_err()
                    .contains(expected),
                "{key}"
            );
        }
        let mut bad = op.clone();
        bad.surfaces.as_mut().unwrap().mesh_fingerprint = "0".repeat(64);
        assert!(
            generate(&setup, &bad, &tool, &mesh)
                .unwrap_err()
                .contains("changed")
        );
        let mut huge = tool.clone();
        huge.p.insert("diameter".into(), "40".into());
        assert!(
            generate(&setup, &op, &huge, &mesh)
                .unwrap_err()
                .contains("narrow")
        );
        huge.p.insert("type".into(), "Ball end mill".into());
        assert!(
            generate(&setup, &op, &huge, &mesh)
                .unwrap_err()
                .contains("flat end")
        );
    }
    #[test]
    fn pocket_rotation_translation_allowances_and_preferences_persist() {
        let (mut setup, mut op, tool, mesh) = fixture();
        let original = generate(&setup, &op, &tool, &mesh).unwrap();
        setup.p.insert("rz".into(), "90".into());
        setup.p.insert("tx".into(), "40".into());
        op.p.insert("allowance".into(), "0.5".into());
        op.p.insert("axial_allowance".into(), "0.4".into());
        let restored: Operation =
            serde_json::from_str(&serde_json::to_string(&op).unwrap()).unwrap();
        assert_eq!(pocket_setting(&restored.p, "pocket_entry"), "Plunge");
        let plan = generate(&setup, &restored, &tool, &mesh).unwrap();
        assert!((plan.target - original.target - 0.4).abs() < 0.001);
        assert!(
            plan.moves
                .iter()
                .filter(|m| m.cutting_pass)
                .all(|m| m.to[0] > 32. && m.to[0] < 48.)
        );
        let mut a = Simulation::new(original);
        let mut b = Simulation::new(plan);
        a.seek(1.);
        b.seek(1.);
        assert!(b.stock.removed_volume() < a.stock.removed_volume());
    }
    #[test]
    fn pocket_rejects_overhangs_and_floors_that_are_not_horizontal() {
        let (mut setup, mut op, tool, mesh) = fixture();
        let scale = mesh.display_scale();
        let mut triangles: Vec<[[f32; 3]; 3]> = mesh
            .faces
            .iter()
            .map(|f| std::array::from_fn(|i| f.points[i].map(|v| v / scale)))
            .collect();
        let floor_z = mesh.surfaces[0].center[2] / scale;
        quad(&mut triangles, 4., -1., 7., 1., floor_z + 2.);
        let covered = mesh::normalize(triangles).unwrap();
        op.surfaces.as_mut().unwrap().mesh_fingerprint = covered.fingerprint.clone();
        assert!(
            generate(&setup, &op, &tool, &covered)
                .unwrap_err()
                .contains("obstructs")
        );
        op.surfaces.as_mut().unwrap().mesh_fingerprint = mesh.fingerprint.clone();
        setup.p.insert("rx".into(), "10".into());
        assert!(
            generate(&setup, &op, &tool, &mesh)
                .unwrap_err()
                .contains("perpendicular")
        );
    }
    #[test]
    fn actual_step_and_3mf_pocket_with_boss_generate_and_leave_the_boss() {
        for extension in ["step", "3mf"] {
            let mesh = mesh::load(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("examples/four-setup-block.{extension}")),
            )
            .unwrap();
            let (mut setup, mut op, mut tool, _) = fixture();
            setup.p.insert("stock_top".into(), "10".into());
            tool.p.insert("diameter".into(), "10".into());
            let placement = Placement::new(&mesh, Some(&setup));
            let top = placement.bounds.max[2];
            let bottom = placement.bounds.min[2];
            let (id, floor) = mesh
                .surfaces
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    s.normal[2].abs() > 0.999999
                        && placement.transform(s.center)[2] < top - 0.1
                        && placement.transform(s.center)[2] > bottom + 0.1
                })
                .max_by(|(_, a), (_, b)| a.area.total_cmp(&b.area))
                .unwrap();
            eprintln!(
                "{extension}: floor {} area {:.2} depth {:.2} mm",
                id + 1,
                floor.area / mesh.display_scale().powi(2),
                (top - placement.transform(floor.center)[2]) / mesh.display_scale()
            );
            op.surfaces = Some(surface::Selection {
                mesh_fingerprint: mesh.fingerprint.clone(),
                ids: vec![id],
            });
            let plan = generate(&setup, &op, &tool, &mesh).unwrap();
            eprintln!(
                "{extension}: {} moves, {} levels, {:.2} min",
                plan.moves.len(),
                plan.levels,
                plan.duration / 60.
            );
            assert!(plan.moves.len() > 100 && plan.levels > 1);
            let mut sim = Simulation::new(plan);
            sim.seek(1.);
            assert!(sim.stock.removed_volume() > 10_000.);
            // The fixture boss is centered at X=-18, Y=0 (not the part origin).
            let x = ((-18. - sim.stock.bounds.min[0]) / sim.stock.dx) as usize;
            let y = ((0. - sim.stock.bounds.min[1]) / sim.stock.dy) as usize;
            assert_eq!(
                sim.stock.heights[y * sim.stock.nx + x],
                sim.stock.bounds.max[2]
            );
        }
    }
}
