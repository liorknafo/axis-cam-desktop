use super::*;
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
    for d in ["9", "11"] {
        let mut tool = t.clone();
        set(&mut tool.p, "diameter", d);
        assert!(
            generate(&s, &o, &tool, &m)
                .unwrap_err()
                .contains("matching")
        );
    }
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
