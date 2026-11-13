use super::*;
use crate::simulation::Plan;
use crate::{
    model::{Document, Strategy},
    simulation::{self, CutterShape, Simulation},
    volume::{Frame, Volume},
};
fn set(p: &mut Fields, k: &str, v: impl ToString) {
    p.insert(k.into(), v.to_string());
}
fn fixture() -> (Setup, Mesh, Tool, Operation) {
    let mesh = crate::mesh::normalize(crate::cam::arcs::tests::slot(5.)).unwrap();
    let setup = crate::corner::tests::setup("0");
    let mut tool = crate::corner::tests::end_mill("10");
    set(&mut tool.p, "center_cutting", "Yes");
    set(&mut tool.p, "machines", &setup.machine.id);
    let mut op = Operation::new(Strategy::Slot, tool.id.clone());
    for (k, v) in [
        ("slot_length", "15"),
        ("slot_width", "10"),
        ("z_bottom", "-8"),
        ("through", "Yes"),
    ] {
        set(&mut op.p, k, v);
    }
    (setup, mesh, tool, op)
}
#[test]
fn exact_centres_and_capsule_stock_never_remove_the_box_corners() {
    let (s, m, t, o) = fixture();
    let p = generate(&s, &o, &t, &m).unwrap();
    let zero = crate::post::datum(&s, p.stock, &m).unwrap();
    assert_eq!(p.levels, 16);
    assert!((p.target - zero[2] + 8.).abs() < 1e-5);
    for mv in &p.moves {
        if mv.motion == Motion::Dwell {
            continue;
        }
        assert_eq!((0..3).filter(|&k| mv.from[k] != mv.to[k]).count(), 1);
    }
    for mv in p.moves.iter().filter(|v| v.cutting_pass) {
        for (got, want) in [
            (mv.from[0] - zero[0], -2.5),
            (mv.to[0] - zero[0], 2.5),
            (mv.from[1] - zero[1], 0.),
        ] {
            assert!((got - want).abs() < 1e-5);
        }
    }
    let mut sim = Simulation::new(p.clone());
    sim.volume = Some(Volume::stock(p.stock, Frame::new(&s, &m), false));
    sim.frame = Some(Frame::new(&s, &m));
    sim.seek(1.);
    let v = sim.volume.unwrap();
    let removed = crate::rest::Removed::new(&v, Frame::new(&s, &m));
    assert!(removed.at([zero[0], zero[1], zero[2] - 7.9]));
    assert!(!removed.at([zero[0] - 7.4, zero[1] - 4.9, zero[2] - 4.]));
    // A clamp in the rectangle's uncut corner is clear of the actual sweep;
    // one crossing a rounded end is refused by that same continuous plan.
    let clamp = |min, max| crate::holding::HoldBox {
        label: "probe clamp".into(),
        bounds: Bounds { min, max },
        shape: crate::holding::Shape::Box,
    };
    let outside = clamp(
        [zero[0] - 7.5, zero[1] - 5., zero[2] - 6.],
        [zero[0] - 7.2, zero[1] - 4.7, zero[2] - 4.],
    );
    assert!(crate::holding::hits(&p, &[outside], 0.).is_empty());
    let inside = clamp(
        [zero[0] - 7.4, zero[1] - 0.1, zero[2] - 6.],
        [zero[0] - 7.2, zero[1] + 0.1, zero[2] - 4.],
    );
    assert!(!crate::holding::hits(&p, &[inside], 0.).is_empty());
}
#[test]
fn nominal_end_arcs_reject_subfacet_shifts_and_oversize() {
    let (s, m, t, o) = fixture();
    generate(&s, &o, &t, &m).unwrap();
    for (key, value) in [
        ("slot_x", "0.001"),
        ("slot_x", "-0.001"),
        ("slot_y", "0.001"),
        ("slot_y", "-0.001"),
        ("slot_length", "15.002"),
    ] {
        let mut bad = o.clone();
        set(&mut bad.p, key, value);
        let error = generate(&s, &bad, &t, &m).unwrap_err();
        assert!(error.contains("sweep"), "{key}={value}: {error}");
    }
    // Bypass the width-match refusal with both width and cutter enlarged:
    // this must still fail the nominal rounded wall, not pass by sagitta slack.
    let (mut tool, mut bad) = (t.clone(), o.clone());
    set(&mut tool.p, "diameter", "10.002");
    set(&mut bad.p, "slot_width", "10.002");
    set(&mut bad.p, "slot_length", "15.002");
    assert!(generate(&s, &bad, &tool, &m).unwrap_err().contains("sweep"));
    let arc = cam::arcs::Arc {
        centre: [-2.5, 0.],
        r: 5.,
        walls: vec![],
        span: 180.,
        middle: std::f32::consts::PI,
        top: 8.,
        bottom: 0.,
        sagitta: 0.01,
    };
    assert!(arc_overcut(&arc, [-2.5, 0.], [2.5, 0.], 5.).abs() < 1e-6);
    assert!(arc_overcut(&arc, [-2.501, 0.], [2.499, 0.], 5.) > 0.0009);
    assert!(arc_overcut(&arc, [-2.5, 0.], [2.5, 0.], 5.001) > 0.0009);
}
#[test]
fn sizes_orientation_allowances_blind_through_and_reach_are_honest() {
    let (s, m, t, o) = fixture();
    // Wider than the slot overcuts its ends; narrower walks it (LIO-455),
    // but not within a hundredth either side.
    for d in ["11", "10.002", "9.99"] {
        let mut tool = t.clone();
        set(&mut tool.p, "diameter", d);
        assert!(
            generate(&s, &o, &tool, &m)
                .unwrap_err()
                .contains("matching"),
            "{d}"
        );
    }
    let mut narrower = t.clone();
    set(&mut narrower.p, "diameter", "9");
    let plan = generate(&s, &o, &narrower, &m).unwrap();
    assert_eq!(plan.slot_passes.last().unwrap().kind, Kind::Finish);
    let mut rotated = o.clone();
    set(&mut rotated.p, "slot_axis", "45");
    assert!(
        generate(&s, &rotated, &t, &m)
            .unwrap_err()
            .contains("X or Y")
    );
    let (mut small, mut leave) = (t.clone(), o.clone());
    set(&mut small.p, "diameter", "9");
    set(&mut leave.p, "allowance", "0.5");
    let plan = generate(&s, &leave, &small, &m).unwrap();
    assert_eq!(plan.radius, 4.5);
    set(&mut leave.p, "through", "No");
    set(&mut leave.p, "axial_allowance", "0.25");
    let plan = generate(&s, &leave, &small, &m).unwrap();
    let zero = crate::post::datum(&s, plan.stock, &m).unwrap();
    assert!((plan.target - zero[2] + 7.75).abs() < 1e-5);
    let mut bad = o.clone();
    set(&mut bad.p, "z_bottom", "-8.1");
    assert!(generate(&s, &bad, &t, &m).is_err());
    set(&mut bad.p, "through", "No");
    assert!(generate(&s, &bad, &t, &m).is_err());
    let mut through = o.clone();
    set(&mut through.p, "breakthrough", "0.5");
    let p = generate(&s, &through, &t, &m).unwrap();
    assert!((p.target - zero[2] + 8.5).abs() < 1e-5);
    set(&mut through.p, "axial_allowance", "0.1");
    assert!(generate(&s, &through, &t, &m).is_err());
    let mut short = t.clone();
    set(&mut short.p, "length", "7.9");
    assert!(
        generate(&s, &o, &short, &m)
            .unwrap_err()
            .contains("flute length")
    );
    set(&mut short.p, "length", "8");
    generate(&s, &o, &short, &m).unwrap();
    let mut cnc = s.clone();
    set(&mut cnc.machine.p, "kind", "CNC");
    assert_eq!(generate(&cnc, &o, &t, &m).unwrap_err(), ON_CNC);
    // Rotate the actual setup 90 degrees; a Y slot is still exactly one axis.
    let mut ys = s.clone();
    set(&mut ys.p, "rz", "90");
    let mut yo = o.clone();
    set(&mut yo.p, "slot_axis", "Y");
    generate(&ys, &yo, &t, &m).unwrap();
}
#[test]
fn entry_requires_explicit_capability_or_a_full_cleared_cylinder() {
    let (mut s, m, mut t, o) = fixture();
    set(&mut t.p, "center_cutting", "Unknown");
    assert!(
        generate(&s, &o, &t, &m)
            .unwrap_err()
            .contains("Center cutting")
    );
    let stock = crate::stock::resolve(&s, Some(&m)).unwrap().stock;
    let zero = crate::post::datum(&s, stock, &m).unwrap();
    let f = Frame::new(&s, &m);
    let mut v = Volume::stock(stock, f, false);
    // Shallow predrill is insufficient; offset or undersized entry also fails.
    for (x, r, z) in [(-2.5, 5., -7.9), (-2.49, 5., -8.), (-2.5, 4.99, -8.)] {
        let mut shallow = v.clone();
        let at = [x + zero[0], zero[1], z + zero[2]];
        shallow.cut(
            at,
            [at[0] + 0.00001, at[1], at[2]],
            r,
            25.,
            CutterShape::Flat,
            f,
            (1, 1),
        );
        s.preview_remaining = Some(std::sync::Arc::new(crate::rest::RestStock::new(
            &shallow, f,
        )));
        assert!(
            generate(&s, &o, &t, &m)
                .unwrap_err()
                .contains("Center cutting")
        );
    }
    let at = [zero[0] - 2.5, zero[1], zero[2] - 8.];
    v.cut(
        at,
        [at[0] + 0.00001, at[1], at[2]],
        5.,
        25.,
        CutterShape::Flat,
        f,
        (1, 1),
    );
    s.preview_remaining = Some(std::sync::Arc::new(crate::rest::RestStock::new(&v, f)));
    generate(&s, &o, &t, &m).unwrap();
    // A false Z top cannot make the fresh entry disappear.
    s.preview_remaining = None;
    let mut low = o.clone();
    set(&mut low.p, "z_top", "-1");
    assert!(
        generate(&s, &low, &t, &m)
            .unwrap_err()
            .contains("remaining stock")
    );
}

/// Read only explicitly supplied private copies. Never opens an app or loads
/// the user's live workspace, and never rewrites the original project/STEP.
pub(crate) fn adapter() -> (Document, Mesh, std::path::PathBuf) {
    let dir = std::path::PathBuf::from(
        std::env::var_os("AXIS_CAPSULE_FIXTURE")
            .expect("set AXIS_CAPSULE_FIXTURE to the unique private copy directory"),
    );
    let mut d = crate::storage::load(&dir.join("adapter-plate.axis.json")).unwrap();
    let file = d.file.as_mut().unwrap();
    file.path = dir.join("adapter-plate.step");
    let mesh = crate::mesh::load_file(file).unwrap();
    let cb = d.tools.iter_mut().find(|t| t.number() == Some(69)).unwrap();
    set(&mut cb.p, "center_cutting", "Yes");
    let cb_id = cb.id.clone();
    let mut through = cb.clone();
    through.id = crate::model::id();
    set(&mut through.p, "diameter", "8");
    set(&mut through.p, "number", "70");
    set(
        &mut through.p,
        "name",
        "Acceptance Ø8 center-cutting slot drill",
    );
    let through_id = through.id.clone();
    d.tools.push(through);
    for (width, length, bottom, tool) in [(13., 18., -10., cb_id), (8., 13., -20., through_id)] {
        for y in [-15.4, 30.4] {
            for x in [-52.85, 52.85] {
                let mut o =
                    Operation::for_machine(Strategy::Slot, tool.clone(), &d.setups[2].machine);
                for (k, v) in [
                    ("slot_x", x),
                    ("slot_y", y),
                    ("slot_width", width),
                    ("slot_length", length),
                    ("z_bottom", bottom),
                    (
                        "z_top",
                        if width == 8. {
                            -10.
                        } else if y < 0. {
                            -1.5
                        } else {
                            0.
                        },
                    ),
                    ("rpm", 1600.),
                    ("feed", 140.),
                ] {
                    set(&mut o.p, k, v);
                }
                set(&mut o.p, "through", if width == 8. { "Yes" } else { "No" });
                set(
                    &mut o.p,
                    "name",
                    format!(
                        "{} {length}×{width} X{x} Y{y}",
                        if width == 13. {
                            "Counterbore"
                        } else {
                            "Through slot"
                        }
                    ),
                );
                d.setups[2].operations.push(o);
            }
        }
    }
    (d, mesh, dir)
}
#[test]
#[ignore = "requires private adapter copy; see task acceptance command"]
fn real_adapter_eight_slots_and_prior_box_failure() {
    let (d, m, dir) = adapter();
    crate::storage::save(&dir.join("capsule-acceptance.axis.json"), &d).unwrap();
    // Direct replay of the saved, suppressed Box without altering its source.
    let old = &d.setups[2].operations[6];
    let mut failed = d.clone();
    failed.setups[2].operations.truncate(7);
    failed.setups[2].operations[6].enabled = true;
    let error = simulation::prepare(&failed, &m, 2, 6).unwrap_err();
    assert!(error.contains("The box cuts into the part"), "{error}");
    let mut report = vec![format!("Prior failure: {}: {error}", old.name())];
    let mut failures = vec![];
    let mut slots = vec![];
    let mut sweeps = String::new();
    let mut picking = vec![];
    simulation::sequence_with(
        &d,
        &m,
        None,
        |si, oi, result| match result {
            Ok(mut sim) if si == 2 && oi >= 7 => {
                let op = &d.setups[si].operations[oi];
                let p = &sim.plan;
                let z = crate::post::datum(&d.setups[si], p.stock, &m).unwrap();
                let slot = Slot::read(op, z).unwrap();
                let expected = if oi < 11 {
                    if num(&op.p, "slot_y").unwrap() < 0. {
                        -1.5
                    } else {
                        0.
                    }
                } else {
                    -10.
                };
                assert!(
                    (p.rest_top.unwrap() - z[2] - expected).abs() < 0.001,
                    "{} start={:?} zero={z:?}",
                    op.name(),
                    p.rest_top
                );
                for mv in p.moves.iter().filter(|v| v.cutting_pass) {
                    for k in 0..2 {
                        assert!((mv.from[k] - slot.a[k]).abs() < 0.0001);
                        assert!((mv.to[k] - slot.b[k]).abs() < 0.0001);
                    }
                }
                let before = sim.volume.as_ref().unwrap().removed_volume();
                let p = sim.plan.clone();
                sim.seek(1.);
                let removed = sim.volume.as_ref().unwrap().removed_volume() - before;
                report.push(format!("{}: top={:.3} bottom={:.3} centres=({:.5},{:.5})→({:.5},{:.5}); levels={}; voxel removed={removed:.5} mm3",op.name(),p.rest_top.unwrap()-z[2],p.target-z[2],slot.a[0]-z[0],slot.a[1]-z[1],slot.b[0]-z[0],slot.b[1]-z[1],p.levels));
                // Export the full swept prism in original STEP coordinates,
                // undoing both setup placement and mesh normalization.
                let frame = Frame::new(&d.setups[si], &m);
                let original = |xy: [f32; 2], z: f32| {
                    let q = frame.unmap([xy[0], xy[1], z]);
                    std::array::from_fn::<f32, 3, _>(|k| q[k] - m.origin[k] / m.display_scale())
                };
                let a = original(slot.a, p.target);
                let b = original(slot.b, p.stock.max[2] + p.flute_length);
                sweeps.push_str(&format!(
                    "{:.9} {:.9} {:.9} {:.9} {:.9} {:.9} {:.9}\n",
                    a[0], a[1], b[0], b[1], p.radius, a[2], b[2]
                ));
                // Click every end facet and the floor when one exists, with
                // the actual preceding stock from this sequential simulation.
                let mut here = d.setups[si].clone();
                here.p.insert("stock_source".into(), "fresh".into());
                here.preview_stock = Some(p.stock);
                here.preview_remaining = sim
                    .initial_volume
                    .as_ref()
                    .map(|v| std::sync::Arc::new(crate::rest::RestStock::new(v, frame)));
                let g = Geometry::new(&here, &m);
                let arcs = cam::arcs::arcs_among(&here, &m, &cam::arcs::upright(&here, &m));
                let ends = arcs
                    .iter()
                    .filter(|v| {
                        (v.r - p.radius).abs() < 0.0001
                            && (v.bottom - slot.bottom).abs() < 0.0001
                            && [slot.a, slot.b]
                                .iter()
                                .any(|c| (v.centre[0] - c[0]).hypot(v.centre[1] - c[1]) < 0.0001)
                    })
                    .collect::<Vec<_>>();
                assert_eq!(ends.len(), 2, "{}", op.name());
                let mut ids = ends
                    .iter()
                    .flat_map(|v| v.walls.iter().copied())
                    .collect::<Vec<_>>();
                if oi < 11 {
                    let floor = g
                        .triangles
                        .iter()
                        .find(|tri| {
                            tri.p.iter().all(|q| {
                                (q[2] - slot.bottom).abs() < 0.0001
                                    && q[0] >= slot.a[0] - p.radius - 0.0001
                                    && q[0] <= slot.b[0] + p.radius + 0.0001
                                    && (q[1] - slot.a[1]).abs() <= p.radius + 0.0001
                            })
                        })
                        .and_then(|tri| tri.feature)
                        .expect("counterbore floor");
                    ids.push(floor);
                }
                // One representative facet from each end plus the floor:
                // all facets of synthetic slots are covered by the regression.
                let representatives = if oi < 11 {
                    vec![ends[0].walls[0], ends[1].walls[0], *ids.last().unwrap()]
                } else {
                    vec![ends[0].walls[0], ends[1].walls[0]]
                };
                for id in representatives {
                    let pick = Selection {
                        mesh_fingerprint: m.fingerprint.clone(),
                        ids: vec![id],
                    };
                    let got = from_face(&here, &m, &pick, p.stock).unwrap();
                    for k in ["slot_x", "slot_y", "slot_length", "slot_width", "z_bottom"] {
                        assert!(
                            (num(&got, k).unwrap() - num(&op.p, k).unwrap()).abs() < 0.001,
                            "{} pick {id} {k}: {got:?}",
                            op.name()
                        );
                    }
                    assert!(
                        (num(&got, "z_top").unwrap() - expected).abs() < 0.001,
                        "{} picked top {got:?}",
                        op.name()
                    );
                    picking.push(format!("{}: face {id} picked correctly", op.name()));
                }
                slots.push(p);
            }
            Ok(_) => {}
            Err(e) => failures.push(format!("{si}/{oi}: {e}")),
        },
        |_, _, _| {},
    );
    assert!(failures.is_empty(), "{failures:#?}");
    assert_eq!(slots.len(), 8);
    std::fs::write(dir.join("sweeps.tsv"), sweeps).unwrap();
    std::fs::write(dir.join("picking.txt"), picking.join("\n") + "\n").unwrap();
    // The same full project must produce operator Steps, including its modeled vise.
    let guide = crate::guide::build(&d, &m, 2).unwrap();
    assert_eq!(
        guide
            .done
            .iter()
            .filter(|v| v.plan.strategy == Strategy::Slot)
            .count(),
        8
    );
    assert_eq!(
        guide
            .steps
            .iter()
            .filter(|v| matches!(v, crate::guide::Step::Slot { .. }))
            .count(),
        154
    );
    report.push(format!(
        "Guide: {} steps; {} operations; zero {:?}",
        guide.steps.len(),
        guide.done.len(),
        guide.zero
    ));
    std::fs::write(dir.join("acceptance.txt"), report.join("\n") + "\n").unwrap();
}

#[test]
fn picking_two_same_row_slots_never_pairs_across_the_part_after_reordering() {
    let outer = [
        (-20., -15., 0.),
        (20., -15., 0.),
        (20., 15., 0.),
        (-20., 15., 0.),
    ];
    let left = [
        (-12.5, -2.5, 0.),
        (-7.5, -2.5, 1.),
        (-7.5, 2.5, 0.),
        (-12.5, 2.5, 1.),
    ];
    let right = [
        (7.5, -2.5, 0.),
        (12.5, -2.5, 1.),
        (12.5, 2.5, 0.),
        (7.5, 2.5, 1.),
    ];
    let tris = crate::cam::arcs::tests::slab(&[&outer, &left, &right], 8., 0., true, true);
    for order in 0..3 {
        let mut t = tris.clone();
        if order == 1 {
            t.reverse();
        }
        if order == 2 {
            t.rotate_left(23);
        }
        let mesh = crate::mesh::normalize(t).unwrap();
        let setup = crate::corner::tests::setup("0");
        let stock = crate::stock::resolve(&setup, Some(&mesh)).unwrap().stock;
        let zero = crate::post::datum(&setup, stock, &mesh).unwrap();
        let arcs = cam::arcs::arcs_among(&setup, &mesh, &cam::arcs::upright(&setup, &mesh));
        assert_eq!(arcs.len(), 4);
        for arc in &arcs {
            for &id in &arc.walls {
                let pick = Selection {
                    mesh_fingerprint: mesh.fingerprint.clone(),
                    ids: vec![id],
                };
                let p = from_face(&setup, &mesh, &pick, stock).unwrap();
                let expected = if arc.centre[0] - zero[0] < 0. {
                    -10.
                } else {
                    10.
                };
                assert!((num(&p, "slot_x").unwrap() - expected).abs() < 0.001);
                assert_eq!(get(&p, "slot_length"), "10.000");
                assert_eq!(get(&p, "slot_width"), "5.000");
            }
        }
    }
}

#[test]
fn drilled_entry_needs_its_full_diameter_past_the_floor() {
    let (mut s, m, mut t, o) = fixture();
    set(&mut t.p, "center_cutting", "No");
    let stock = crate::stock::resolve(&s, Some(&m)).unwrap().stock;
    let zero = crate::post::datum(&s, stock, &m).unwrap();
    let f = Frame::new(&s, &m);
    for (extra, valid) in [
        (0., false),
        (5. * crate::simulation::POINT_RISE - 0.01, false),
        (5. * crate::simulation::POINT_RISE + 0.01, true),
    ] {
        let mut v = Volume::stock(stock, f, false);
        v.cut(
            [zero[0] - 2.5, zero[1], zero[2] + 1.],
            [zero[0] - 2.5, zero[1], zero[2] - 8. - extra],
            5.,
            25.,
            CutterShape::Drill,
            f,
            (1, 1),
        );
        s.preview_remaining = Some(std::sync::Arc::new(crate::rest::RestStock::new(&v, f)));
        assert_eq!(generate(&s, &o, &t, &m).is_ok(), valid, "extra {extra}");
    }
}

#[test]
#[ignore = "requires private adapter copy; headless App dispatch only"]
fn toolbar_add_pick_and_edit_use_the_first_class_slot() {
    let (mut doc, mesh, dir) = adapter();
    doc.active = doc.setups[2].id.clone();
    doc.setups[2].operations.truncate(7);
    let tiles = crate::ui::ribbon_tiles(Some(&doc.setups[2].machine));
    assert!(
        tiles
            .iter()
            .flat_map(|(_, t)| t)
            .any(|t| { matches!(t, crate::ui::Tile::Op(Strategy::Slot)) })
    );
    let mut app = crate::App::from_workspace(dir.join("pick-state.axis.json"), None);
    app.doc = doc;
    app.imported_mesh = Some(mesh);
    let _ = app.update(crate::Message::AddOp(Strategy::Slot));
    assert_eq!(app.operation().unwrap().strategy, Strategy::Slot);
    assert_eq!(app.tab, "Slot");
    assert!(app.picking);
    let mesh = app.imported_mesh.as_ref().unwrap();
    let setup = app.doc.setup();
    let stock = app.known_stock(2).0.unwrap();
    let zero = crate::post::datum(setup, stock, mesh).unwrap();
    let arc = cam::arcs::arcs_among(setup, mesh, &cam::arcs::upright(setup, mesh))
        .into_iter()
        .find(|a| {
            (a.r - 6.5).abs() < 0.0001
                && (a.centre[0] - zero[0] + 55.35).abs() < 0.0001
                && (a.centre[1] - zero[1] + 15.4).abs() < 0.0001
        })
        .unwrap();
    let _ = app.update(crate::Message::Surface(arc.walls[0]));
    assert!(app.status.starts_with("Picked Slot"), "{}", app.status);
    let op = app.operation().unwrap();
    for (k, want) in [
        ("slot_x", -52.85),
        ("slot_y", -15.4),
        ("slot_length", 18.),
        ("slot_width", 13.),
        ("z_top", -1.5),
        ("z_bottom", -10.),
    ] {
        assert!((num(&op.p, k).unwrap() - want).abs() < 0.001, "{k}");
    }
    let _ = app.update(crate::Message::Edit(
        crate::Target::Operation,
        "slot_x",
        "52.85".into(),
    ));
    assert_eq!(get(&app.operation().unwrap().p, "slot_x"), "52.85");
    let texts = crate::ui::probe::texts(app.view(), iced::Size::new(1440., 1000.));
    assert!(
        texts.iter().any(|s| s.contains("Overall length")),
        "{texts:?}"
    );
    std::fs::write(dir.join("toolbar-picking.txt"), texts.join("\n")).unwrap();
}

/// A `w` × `l` plate `t` thick (X −w/2..w/2, Y −l/2..l/2, Z 0..t) with a
/// round-ended counterbore `width` wide and `length` long along X (Y when
/// `along_y`), `depth` deep from its top, round X0 Y0 (LIO-455).
pub(crate) fn counterbore(
    (w, l, t): (f64, f64, f32),
    length: f64,
    width: f64,
    depth: f32,
    along_y: bool,
) -> Vec<[[f32; 3]; 3]> {
    use crate::cam::arcs::tests::slab;
    let (h, r) = ((length - width) / 2., width / 2.);
    let capsule: Vec<(f64, f64, f64)> = if along_y {
        vec![(r, -h, 0.), (r, h, 1.), (-r, h, 0.), (-r, -h, 1.)]
    } else {
        vec![(-h, -r, 0.), (h, -r, 1.), (h, r, 0.), (-h, r, 1.)]
    };
    let outer = [
        (-w / 2., -l / 2., 0.),
        (w / 2., -l / 2., 0.),
        (w / 2., l / 2., 0.),
        (-w / 2., l / 2., 0.),
    ];
    let floor = t - depth;
    let mut tris = slab(&[&outer, &capsule], depth, floor, true, false);
    tris.extend(slab(&[&outer], floor, 0., false, true));
    // The floor: the capsule's own top face, its edge the walls' facets.
    tris.extend(
        slab(&[&capsule], 1., floor - 1., true, false)
            .into_iter()
            .filter(|t| t.iter().all(|p| (p[2] - floor).abs() < 1e-4)),
    );
    tris
}
/// The manual mill's job on `mesh`: the stock the part's own box, its zero
/// on the stock top's centre, the vise across Y; one Slot with `tool`
/// (center cutting) of `fields`.
pub(crate) fn job(mesh: &Mesh, diameter: &str, fields: &[(&str, &str)]) -> Document {
    let (mut doc, _) = crate::corner::tests::plate_job("Y");
    let tool = &mut doc.tools[0];
    for (k, v) in [
        ("diameter", diameter),
        ("center_cutting", "Yes"),
        ("length", "25"),
        ("stickout", "40"),
    ] {
        set(&mut tool.p, k, v);
    }
    set(&mut tool.p, "name", "Centre-cutting end mill");
    let mut op = Operation::new(Strategy::Slot, tool.id.clone());
    for (k, v) in fields {
        set(&mut op.p, k, v);
    }
    let s = &mut doc.setups[0];
    for side in ["left", "right", "front", "back"] {
        s.p.insert(format!("stock_{side}"), "0".into());
    }
    s.p.insert("vise_grip".into(), "4".into());
    s.operations = vec![op];
    let _ = mesh;
    doc
}
/// The adapter's counterbore (LIO-455): 21.5 long along X, 16.5 wide, 8.9
/// deep, in a 60 × 50 × 20 plate.
fn adapter_counterbore(along_y: bool) -> (Document, Mesh) {
    let mesh =
        crate::mesh::normalize(counterbore((60., 50., 20.), 21.5, 16.5, 8.9, along_y)).unwrap();
    let doc = job(
        &mesh,
        "12",
        &[
            ("name", "Counterbore 21.5×16.5"),
            ("slot_axis", if along_y { "Y" } else { "X" }),
            ("slot_length", "21.5"),
            ("slot_width", "16.5"),
            ("z_top", "0"),
            ("z_bottom", "-8.9"),
            ("through", "No"),
            ("stepdown", "0.5"),
            ("rpm", "1000"),
            ("feed", "100"),
        ],
    );
    (doc, mesh)
}
/// The plan of `doc`'s one Slot, and the stock after it.
fn cut(doc: &Document, mesh: &Mesh) -> (Plan, Volume) {
    let (mut plan, mut stock) = (None, None);
    simulation::sequence_with(
        doc,
        mesh,
        None,
        |_, _, r| plan = Some(r.unwrap().plan),
        |_, _, v| stock = Some(v.clone()),
    );
    (plan.unwrap(), stock.unwrap())
}
/// Every point within `inside` of the slot's centre line (`a`–`b`, plan
/// frame), on a 0.05 mm grid, is within the cutter's radius of a feed of
/// `plan` at Z `z`: nothing is left there, whatever the voxels.
fn covered(plan: &Plan, a: [f32; 2], b: [f32; 2], inside: f32, z: f32) -> Result<(), [f32; 2]> {
    let feeds: Vec<([f32; 2], [f32; 2])> = (plan.moves.iter())
        .filter(|m| {
            m.motion == Motion::Feed && (m.from[2] - z).abs() < 1e-4 && (m.to[2] - z).abs() < 1e-4
        })
        .map(|m| ([m.from[0], m.from[1]], [m.to[0], m.to[1]]))
        .collect();
    let lo = [a[0].min(b[0]) - inside, a[1].min(b[1]) - inside];
    let hi = [a[0].max(b[0]) + inside, a[1].max(b[1]) + inside];
    let n = [
        ((hi[0] - lo[0]) / 0.05) as usize,
        ((hi[1] - lo[1]) / 0.05) as usize,
    ];
    for i in 0..=n[0] {
        for j in 0..=n[1] {
            let p = [lo[0] + i as f32 * 0.05, lo[1] + j as f32 * 0.05];
            if off_line(p, a, b) > inside {
                continue;
            }
            if !feeds
                .iter()
                .any(|(u, v)| off_line(p, *u, *v) <= plan.radius + 1e-5)
            {
                return Err(p);
            }
        }
    }
    Ok(())
}
#[test]
fn a_twelve_mill_cuts_the_adapters_16_5_counterbore_to_size() {
    for along_y in [false, true] {
        let (doc, mesh) = adapter_counterbore(along_y);
        let (plan, stock) = cut(&doc, &mesh);
        let setup = &doc.setups[0];
        let zero = crate::post::datum(setup, plan.stock, &mesh).unwrap();
        let slot = Slot::read(&setup.operations[0], zero).unwrap();
        assert_eq!(plan.radius, 6.);
        // 8.9 deep in half millimetres: 18 levels, each the centre sweep and
        // one walk round (2.25 − 0.2 out, one step), then the finish.
        assert_eq!(plan.levels, 18);
        let kinds: Vec<Kind> = plan.slot_passes.iter().map(|p| p.kind).collect();
        assert_eq!(kinds.len(), 18 * 2 + 1);
        for l in 0..18 {
            assert_eq!(kinds[2 * l], Kind::Sweep);
            assert_eq!(kinds[2 * l + 1], Kind::Outline { index: 1, of: 1 });
        }
        assert_eq!(kinds[36], Kind::Finish);
        // The passes cover the plan's moves in order, all of them.
        let mut next = 0;
        for p in &plan.slot_passes {
            assert_eq!(p.moves.start, next, "{:?}", p.kind);
            next = p.moves.end;
        }
        assert_eq!(next, plan.moves.len());
        // One axis at a time; every walk's reading on the DRO's hundredths.
        for mv in plan.moves.iter().filter(|m| m.motion != Motion::Dwell) {
            assert_eq!((0..3).filter(|&k| mv.from[k] != mv.to[k]).count(), 1);
        }
        for p in plan.slot_passes.iter().filter(|p| p.kind != Kind::Sweep) {
            for q in &p.points {
                for v in q {
                    assert!(((v / 0.01).round() * 0.01 - v).abs() < 2e-4, "{v}");
                }
            }
        }
        // The finish: 2.25 out, at size, the steps leaving at most 0.05.
        let finish = plan.slot_passes.last().unwrap();
        assert!((finish.offset - 2.25).abs() < 1e-5);
        assert!(finish.scallop <= 0.05 + 1e-6 && finish.scallop > 0.);
        assert!((finish.z + 8.9).abs() < 1e-4);
        // Never past the walls: every tool centre within 2.25 of the centre
        // line, so the Ø12's edge within R8.25.
        let reach = (plan.moves.iter())
            .filter(|m| m.motion == Motion::Feed)
            .flat_map(|m| [m.from, m.to])
            .map(|p| off_line([p[0], p[1]], slot.a, slot.b))
            .fold(0f32, f32::max);
        assert!(reach <= 2.25 + 1e-5, "{reach}");
        assert!(reach >= 2.25 - 0.01, "{reach}");
        // Nothing left on the floor up to the scallop off the walls, at the
        // floor and at every level up to the finish band.
        covered(&plan, slot.a, slot.b, 8.25 - 0.05, plan.target).unwrap();
        for p in plan.slot_passes.iter().filter(|p| p.kind == Kind::Sweep) {
            let z = p.z + zero[2];
            covered(&plan, slot.a, slot.b, 8.25 - 0.2 - 0.1, z)
                .unwrap_or_else(|q| panic!("level {} at {q:?}", p.level));
        }
        // The simulation: the part all cut, 0% left; the floor gone and the
        // wall just past R8.25 still there.
        let left = crate::uncut::uncut(&stock, &doc.setups, &mesh);
        assert!(!left.any(), "{:?}", left.regions);
        assert_eq!(crate::uncut::amount(&left.regions, left.share), "0%");
        let frame = Frame::new(setup, &mesh);
        let removed = crate::rest::Removed::new(&stock, frame);
        let k = usize::from(along_y);
        let at = |along: f32, across: f32, z: f32| {
            let mut q = [0., 0., zero[2] + z];
            q[k] = zero[k] + along;
            q[1 - k] = zero[1 - k] + across;
            q
        };
        for (along, across) in [(0., 0.), (10.6, 0.), (-10.6, 0.), (0., 8.1), (2.5, -8.1)] {
            assert!(removed.at(at(along, across, -8.85)), "{along} {across}");
        }
        for (along, across) in [(11.05, 0.), (-11.05, 0.), (0., 8.5), (-2.5, -8.5)] {
            assert!(!removed.at(at(along, across, -4.)), "{along} {across}");
        }
    }
}
#[test]
fn a_wide_slot_steps_out_no_more_than_0_9_of_the_cutter_and_a_ccw_spindle_walks_back() {
    let mesh = crate::mesh::normalize(counterbore((80., 60., 20.), 40., 30., 4., false)).unwrap();
    for spindle in ["CW", "CCW"] {
        let mut doc = job(
            &mesh,
            "6",
            &[
                ("slot_length", "40"),
                ("slot_width", "30"),
                ("z_top", "0"),
                ("z_bottom", "-4"),
                ("through", "No"),
                ("stepdown", "1"),
                ("scallop", "0.03"),
            ],
        );
        if spindle == "CCW" {
            let op = &mut doc.setups[0].operations[0];
            set(&mut op.p, "spindle_rotation", "Counterclockwise (CCW)");
        }
        let (plan, stock) = cut(&doc, &mesh);
        let zero = crate::post::datum(&doc.setups[0], plan.stock, &mesh).unwrap();
        let slot = Slot::read(&doc.setups[0].operations[0], zero).unwrap();
        // 15 − 3 = 12 out; 11.8 in steps of at most 5.4: three walks a level.
        let level: Vec<&Pass> = plan.slot_passes.iter().filter(|p| p.level == 1).collect();
        assert_eq!(level.len(), 4);
        let mut last = 0.;
        for (i, p) in level[1..].iter().enumerate() {
            assert_eq!(
                p.kind,
                Kind::Outline {
                    index: i + 1,
                    of: 3
                }
            );
            assert!(p.offset - last <= 0.9 * 6. + 1e-4, "{} {last}", p.offset);
            assert!(p.scallop <= 0.6 + 1e-6);
            last = p.offset;
        }
        assert!((last - 11.8).abs() < 1e-4);
        let finish = plan.slot_passes.last().unwrap();
        assert!(finish.scallop <= 0.03 + 1e-6);
        // Which way round: clockwise seen from above with the spindle
        // clockwise (conventional inside), counter-clockwise with it CCW.
        let pts = &finish.points;
        let area: f32 = (0..pts.len())
            .map(|i| {
                let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
                p[0] * q[1] - q[0] * p[1]
            })
            .sum();
        assert_eq!(area < 0., plan.spindle_clockwise, "{spindle}: {area}");
        for z in [-1., -2., -3., -4.] {
            covered(&plan, slot.a, slot.b, 15. - 0.2 - 0.1, z + zero[2]).unwrap();
        }
        covered(&plan, slot.a, slot.b, 15. - 0.05, plan.target).unwrap();
        let left = crate::uncut::uncut(&stock, &doc.setups, &mesh);
        assert!(!left.any(), "{spindle}: {:?}", left.regions);
    }
}
#[test]
fn a_narrower_cutters_steps_give_each_pass_its_dro_numbers_and_the_check_the_round() {
    let (doc, mesh) = adapter_counterbore(false);
    let shown = crate::steps::shown(&doc, &mesh, 0).unwrap();
    let g = &shown.guide;
    let slots: Vec<&crate::guide::Step> = (g.steps.iter())
        .filter(|s| matches!(s, crate::guide::Step::Slot { .. }))
        .collect();
    assert_eq!(slots.len(), 37);
    // Where the tool is, from step to step.
    let mut at = [f32::NAN; 2];
    for s in &slots {
        let crate::guide::Step::Slot {
            targets,
            pass,
            points,
            ..
        } = s
        else {
            unreachable!()
        };
        assert!(!targets.is_empty());
        if *pass != Kind::Sweep {
            // Each of its readings is a number on the page, in order.
            let feeds: Vec<[f32; 2]> = {
                let mut out = vec![];
                for t in targets.iter().filter(|t| t.axis < 2) {
                    at[t.axis] = t.centre;
                    if t.feed {
                        out.push(at);
                    }
                }
                out
            };
            for p in points {
                assert!(
                    feeds
                        .iter()
                        .any(|q| (q[0] - p[0]).abs() < 1e-4 && (q[1] - p[1]).abs() < 1e-4),
                    "{pass:?}: {p:?}"
                );
            }
            // The walk is closed: it ends where it starts, at the end's tip.
            assert_eq!(points.first(), points.last());
            // Out to the walk's tip on the DRO's hundredths: 2.5 + 2.05 for
            // the outline, + 2.25 for the finish.
            let tip = if *pass == Kind::Finish { 4.75 } else { 4.55 };
            assert!(
                (points[0][0] - tip).abs() < 1e-4,
                "{pass:?} {:?}",
                points[0]
            );
        } else {
            for t in targets.iter().filter(|t| t.axis < 2) {
                at[t.axis] = t.centre;
            }
        }
    }
    let top = shown.boxes[0].max[2] + g.zero[2];
    let m = crate::steps::measures(&doc, g, &g.done[0], top);
    let find = |axis: &str, v: f32| {
        m.iter()
            .find(|x| x.axis == axis && (x.value - v).abs() < 0.001)
            .unwrap_or_else(|| panic!("{axis} {v} in {m:?}"))
    };
    find("X", 21.5);
    find("Y", 16.5);
    find("Z", 8.9);
    let r = find("R", 8.25);
    assert!(r.words[1].contains("0.05"), "{:?}", r.words);
}

/// The redesigned adapter plate (LIO-455) from a private copy of its STEP
/// in `AXIS_LIO455_FIXTURE`, face up on the manual mill in the vise, its
/// zero on the stock top's centre; its four 16.5 × 21.5 counterbores
/// picked off the part and cut with a Ø12 end mill, then the four 8 × 13
/// through slots in them with a Ø8 slot drill. Never the user's files.
pub(crate) fn redesigned() -> (Document, Mesh, std::path::PathBuf) {
    let dir = std::path::PathBuf::from(
        std::env::var_os("AXIS_LIO455_FIXTURE")
            .expect("set AXIS_LIO455_FIXTURE to the private copy's directory"),
    );
    let path = dir.join("adapter-plate.step");
    let file = crate::model::ModelFile {
        name: "adapter-plate.step".into(),
        bytes: std::fs::metadata(&path).unwrap().len(),
        path,
        format: "step".into(),
        thickness: None,
    };
    let mesh = crate::mesh::load_file(&file).unwrap();
    let mut doc = job(&mesh, "12", &[]);
    doc.file = Some(file);
    doc.setups[0].operations.clear();
    // Too wide for the vise: two toe clamps on its front and back edges,
    // as the user's own project holds it.
    let manual = doc.machines[0].id.clone();
    let clamp = doc
        .workholding
        .iter_mut()
        .find(|h| get(&h.p, "kind") == "Clamp")
        .unwrap();
    set(&mut clamp.p, "machines", manual);
    let clamp = clamp.id.clone();
    for (k, v) in [
        ("holding", "Clamps"),
        ("holder", clamp.as_str()),
        ("clamps", "-Y:0;+Y:0"),
    ] {
        set(&mut doc.setups[0].p, k, v);
    }
    let twelve = doc.tools[0].id.clone();
    let mut eight = doc.tools[0].clone();
    eight.id = crate::model::id();
    for (k, v) in [
        ("diameter", "8"),
        ("number", "2"),
        ("name", "Centre-cutting slot drill"),
    ] {
        set(&mut eight.p, k, v);
    }
    let eight_id = eight.id.clone();
    doc.tools.push(eight);
    let setup = doc.setups[0].clone();
    let stock = crate::stock::resolve(&setup, Some(&mesh)).unwrap().stock;
    let arcs = cam::arcs::arcs_among(&setup, &mesh, &cam::arcs::upright(&setup, &mesh));
    let zero = crate::post::datum(&setup, stock, &mesh).unwrap();
    // One end of each capsule of radius `r`, in order round the plate.
    let ends = |r: f32| {
        let mut found: Vec<&cam::arcs::Arc> = arcs
            .iter()
            .filter(|a| (a.r - r).abs() < 0.01 && (a.span - 180.).abs() < 1.)
            .filter(|a| a.middle.cos() > 0.5)
            .collect();
        found.sort_by(|a, b| {
            (a.centre[1].total_cmp(&b.centre[1])).then(a.centre[0].total_cmp(&b.centre[0]))
        });
        found.into_iter().cloned().collect::<Vec<_>>()
    };
    let bores = ends(8.25);
    assert_eq!(bores.len(), 4, "{arcs:#?}");
    let mut floors = vec![];
    for (i, a) in bores.iter().enumerate() {
        let pick = Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: vec![a.walls[0]],
        };
        let fields = from_face(&setup, &mesh, &pick, stock).unwrap();
        let mut o = Operation::for_machine(Strategy::Slot, twelve.clone(), &setup.machine);
        o.p.extend(fields);
        for (k, v) in [
            ("rpm", "1000"),
            ("feed", "100"),
            ("stepdown", "0.5"),
            ("scallop", "0.05"),
            ("clearance", "40"),
        ] {
            set(&mut o.p, k, v);
        }
        set(
            &mut o.p,
            "name",
            format!(
                "Counterbore 21.5×16.5 #{} X{:.2} Y{:.2}",
                i + 1,
                a.centre[0] - zero[0] - 2.5,
                a.centre[1] - zero[1]
            ),
        );
        floors.push(num(&o.p, "z_bottom").unwrap());
        doc.setups[0].operations.push(o);
    }
    let slots = ends(4.);
    assert_eq!(slots.len(), 4, "{arcs:#?}");
    for (i, a) in slots.iter().enumerate() {
        let pick = Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: vec![a.walls[0]],
        };
        let fields = from_face(&setup, &mesh, &pick, stock).unwrap();
        let mut o = Operation::for_machine(Strategy::Slot, eight_id.clone(), &setup.machine);
        o.p.extend(fields);
        for (k, v) in [
            ("rpm", "1600"),
            ("feed", "100"),
            ("stepdown", "0.5"),
            ("breakthrough", "0.5"),
            ("clearance", "40"),
        ] {
            set(&mut o.p, k, v);
        }
        set(&mut o.p, "z_top", floors[i]);
        set(&mut o.p, "name", format!("Through slot 13×8 #{}", i + 1));
        doc.setups[0].operations.push(o);
    }
    (doc, mesh, dir)
}
#[test]
#[ignore = "requires the private copy of the redesigned adapter STEP (AXIS_LIO455_FIXTURE)"]
fn the_redesigned_adapters_counterbores_cut_with_a_twelve() {
    let (d, m, dir) = redesigned();
    let mut report = vec![];
    for o in &d.setups[0].operations {
        report.push(format!(
            "{}: X{} Y{} L{} W{} axis {} Z{}..{} through {}",
            o.name(),
            get(&o.p, "slot_x"),
            get(&o.p, "slot_y"),
            get(&o.p, "slot_length"),
            get(&o.p, "slot_width"),
            get(&o.p, "slot_axis"),
            get(&o.p, "z_top"),
            get(&o.p, "z_bottom"),
            get(&o.p, "through"),
        ));
    }
    let mut failures = vec![];
    let mut plans = vec![];
    let mut last = None;
    simulation::sequence_with(
        &d,
        &m,
        None,
        |si, oi, r| match r {
            Ok(sim) => plans.push((oi, sim.plan)),
            Err(e) => failures.push(format!("{si}/{oi}: {e}")),
        },
        |_, _, v| last = Some(v.clone()),
    );
    assert!(failures.is_empty(), "{failures:#?}\n{}", report.join("\n"));
    let stock = last.unwrap();
    let setup = &d.setups[0];
    let frame = Frame::new(setup, &m);
    let removed = crate::rest::Removed::new(&stock, frame);
    for (oi, p) in plans.iter().filter(|(oi, _)| *oi < 4) {
        let o = &setup.operations[*oi];
        let zero = crate::post::datum(setup, p.stock, &m).unwrap();
        let slot = Slot::read(o, zero).unwrap();
        let finish = p.slot_passes.last().unwrap();
        assert_eq!(finish.kind, Kind::Finish);
        covered(p, slot.a, slot.b, 8.25 - 0.05, p.target).unwrap();
        let reach = (p.moves.iter())
            .filter(|m| m.motion == Motion::Feed)
            .flat_map(|m| [m.from, m.to])
            .map(|q| off_line([q[0], q[1]], slot.a, slot.b))
            .fold(0f32, f32::max);
        assert!(reach <= 2.25 + 1e-5, "{reach}");
        let k = slot.axis();
        let c = [(slot.a[0] + slot.b[0]) / 2., (slot.a[1] + slot.b[1]) / 2.];
        let at = |along: f32, across: f32, z: f32| {
            let mut q = [0., 0., z];
            q[k] = c[k] + along;
            q[1 - k] = c[1 - k] + across;
            q
        };
        // The floor all cut, round the slot opening; the walls just past
        // R8.25 not.
        for (along, across) in [
            (7.5, 0.),
            (-7.5, 0.),
            (10.6, 0.),
            (-10.6, 0.),
            (0., 8.1),
            (2.5, -8.1),
        ] {
            assert!(
                removed.at(at(along, across, p.target + 0.05)),
                "{} {along} {across}",
                o.name()
            );
        }
        for (along, across) in [(11.05, 0.), (-11.05, 0.), (0., 8.5), (-2.5, -8.5)] {
            assert!(
                !removed.at(at(along, across, p.target + 4.)),
                "{} {along} {across}",
                o.name()
            );
        }
        report.push(format!(
            "{}: {} levels, {} passes, {} moves; finish scallop {:.4} mm; tool centres within {:.4} of the centre line; {:.1} min",
            o.name(),
            p.levels,
            p.slot_passes.len(),
            p.moves.len(),
            finish.scallop,
            reach,
            p.duration / 60.
        ));
    }
    // What the part has left under stock: nothing in or round the
    // counterbores and slots (the rest of the plate is not machined here).
    let left = crate::uncut::uncut(&stock, &d.setups, &m);
    for r in &left.regions {
        report.push(format!("Uncut elsewhere: {}", r.line()));
    }
    let bounds = Geometry::new(setup, &m).bounds;
    for (oi, p) in plans.iter().filter(|(oi, _)| *oi < 4) {
        let o = &setup.operations[*oi];
        let zero = crate::post::datum(setup, p.stock, &m).unwrap();
        let slot = Slot::read(o, zero).unwrap();
        // The counterbore's box from the part's −X/−Y/bottom corner.
        let lo = [
            slot.a[0] - 8.25 - bounds.min[0],
            slot.a[1] - 8.25 - bounds.min[1],
        ];
        let hi = [
            slot.b[0] + 8.25 - bounds.min[0],
            slot.b[1] + 8.25 - bounds.min[1],
        ];
        for r in &left.regions {
            let b = r.bounds;
            let overlaps = (0..2).all(|k| b.min[k] < hi[k] - 0.1 && b.max[k] > lo[k] + 0.1);
            assert!(!overlaps, "{} left: {}", o.name(), r.line());
        }
    }
    report.push(format!(
        "Uncut share of the whole plate: {}",
        crate::uncut::amount(&left.regions, left.share)
    ));
    std::fs::write(dir.join("acceptance.txt"), report.join("\n") + "\n").unwrap();
    println!("{}", report.join("\n"));
}

/// The walks before the finish overlap whatever scallop the finish is
/// given (LIO-455 review: a Ø2 with a 1 mm scallop left floor standing
/// between its step-outs, 4.4 mm in from the wall).
#[test]
fn a_coarse_finish_scallop_leaves_no_floor_between_the_walks() {
    for (width, length, diameter) in [(30., 40., "2"), (38., 48., "6")] {
        let mesh =
            crate::mesh::normalize(counterbore((90., 80., 20.), length, width, 1., false)).unwrap();
        let doc = job(
            &mesh,
            diameter,
            &[
                ("slot_width", &width.to_string()),
                ("slot_length", &length.to_string()),
                ("z_top", "0"),
                ("z_bottom", "-1"),
                ("through", "No"),
                ("stepdown", "1"),
                ("scallop", "1"),
            ],
        );
        let (plan, stock) = cut(&doc, &mesh);
        let zero = crate::post::datum(&doc.setups[0], plan.stock, &mesh).unwrap();
        let slot = Slot::read(&doc.setups[0].operations[0], zero).unwrap();
        let rc = plan.radius;
        for p in plan.slot_passes.iter().filter(|p| p.kind != Kind::Finish) {
            assert!(p.scallop <= 0.1 * rc + 1e-6, "{diameter}: {:?}", p.kind);
        }
        // Every walk before the finish leaves its band covered: up to the
        // finish's 0.2 and the walks' own steps off the walls.
        let half = width as f32 / 2.;
        covered(
            &plan,
            slot.a,
            slot.b,
            half - 0.2 - 0.1 * rc - 0.01,
            plan.target,
        )
        .unwrap_or_else(|q| panic!("Ø{diameter}: floor left at {q:?}"));
        let left = crate::uncut::uncut(&stock, &doc.setups, &mesh);
        // The walls' 1 mm scallops are what was asked for; the floor is cut.
        for r in &left.regions {
            assert!(
                r.bounds.max[2] - r.bounds.min[2] > 0.5 || r.bounds.min[2] > 18.9,
                "Ø{diameter}: {}",
                r.line()
            );
        }
    }
}

/// The adapter's counterbore with the part placed far from the setup's
/// origin, as the user's fourth setup places the plate (tx 21, ty 179):
/// the part check must not read single-precision slivers along the round
/// ends' chords as part (LIO-455: the two upper counterbores were refused
/// as cutting into the part, the two lower ones with the same sizes not).
#[test]
fn a_counterbore_far_from_the_setups_origin_is_planned_as_near_it() {
    let (doc, mesh) = adapter_counterbore(false);
    let near = generate(
        &doc.setups[0],
        &doc.setups[0].operations[0],
        &doc.tools[0],
        &mesh,
    )
    .unwrap()
    .slot_passes
    .len();
    // The user's placement, and placements a part check in single
    // precision refused (it refused 22 of 60 between X −300 Y 120 and
    // X 308 Y 515 as cutting into the part, and X −260 Y 233.7 as crossing
    // a round end: one step of single precision there).
    for (tx, ty) in [
        ("21", "179"),
        ("-289.7", "126.7"),
        ("-73.4", "267.4"),
        ("60.5", "354.5"),
        ("194.4", "441.6"),
        ("-260", "233.7"),
    ] {
        let mut far = doc.clone();
        set(&mut far.setups[0].p, "tx", tx);
        set(&mut far.setups[0].p, "ty", ty);
        let s = &far.setups[0];
        let plan = generate(s, &s.operations[0], &far.tools[0], &mesh)
            .unwrap_or_else(|e| panic!("tx {tx} ty {ty}: {e}"));
        assert_eq!(plan.slot_passes.len(), near, "tx {tx} ty {ty}");
    }
    // A real overcut is still seen out there, as near the origin: the
    // slot a thousandth off its place or long; a hundredth too wide for the
    // walked Ø12 (its readings are on the DRO's hundredths, rounded in, so
    // a thousandth wider still reads 2.25 out and cuts R8.25); a thousandth
    // too wide with a cutter as wide, which sweeps it.
    for (tx, ty) in [
        ("0", "0"),
        ("21", "179"),
        ("-289.7", "126.7"),
        ("194.4", "441.6"),
    ] {
        for (key, value, tool) in [
            ("slot_x", "0.001", "12"),
            ("slot_x", "-0.001", "12"),
            ("slot_y", "0.001", "12"),
            ("slot_y", "-0.001", "12"),
            ("slot_length", "21.502", "12"),
            ("slot_width", "16.52", "12"),
            ("slot_width", "16.502", "16.502"),
        ] {
            let mut far = doc.clone();
            set(&mut far.setups[0].p, "tx", tx);
            set(&mut far.setups[0].p, "ty", ty);
            set(&mut far.setups[0].operations[0].p, key, value);
            if key == "slot_width" {
                let length = 21.5 + (value.parse::<f32>().unwrap() - 16.5);
                set(&mut far.setups[0].operations[0].p, "slot_length", length);
            }
            set(&mut far.tools[0].p, "diameter", tool);
            let s = &far.setups[0];
            let e = generate(s, &s.operations[0], &far.tools[0], &mesh)
                .err()
                .unwrap_or_else(|| panic!("tx {tx} ty {ty} {key} {value} Ø{tool}: planned"));
            assert!(
                e.contains("sweep") || e.contains("walk reaches past"),
                "tx {tx} ty {ty} {key} {value} Ø{tool}: {e}"
            );
        }
    }
}

/// The user's own adapter project, a private copy in `AXIS_ADAPTER_COPY`
/// (adapter-plate.axis.json and the STEP), never the original: every
/// operation of every setup plans and simulates, the Slots among them
/// (LIO-455: setup 4's two upper counterbores, far from its origin, were
/// refused), and every setup's Steps build.
#[test]
#[ignore = "requires a private copy of the user's adapter project (AXIS_ADAPTER_COPY)"]
fn the_users_adapter_project_plans_every_slot() {
    let dir = std::path::PathBuf::from(
        std::env::var_os("AXIS_ADAPTER_COPY").expect("set AXIS_ADAPTER_COPY to the private copy"),
    );
    let mut doc = crate::storage::load(&dir.join("adapter-plate.axis.json")).unwrap();
    doc.file.as_mut().unwrap().path = dir.join("adapter-plate.step");
    let mesh = crate::mesh::load_file(doc.file.as_ref().unwrap()).unwrap();
    let mut failures = vec![];
    let mut slots = 0;
    simulation::sequence_with(
        &doc,
        &mesh,
        None,
        |si, oi, r| {
            let op = &doc.setups[si].operations[oi];
            match r {
                Ok(_) if op.strategy == Strategy::Slot => slots += 1,
                Ok(_) => {}
                Err(e) => failures.push(format!("{}/{} {}: {e}", si + 1, oi, op.name())),
            }
        },
        |_, _, _| {},
    );
    assert!(failures.is_empty(), "{failures:#?}");
    assert_eq!(slots, 8);
    for si in 0..doc.setups.len() {
        if let Err(e) = crate::steps::shown(&doc, &mesh, si) {
            panic!("setup {}: {e}", si + 1);
        }
    }
}
