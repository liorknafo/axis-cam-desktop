//! Add threading: the operations that make a setup's threaded holes, each
//! thread size its own pair, on the machine the setup runs on. The holes
//! are drilled to their tap drill, then threaded: thread milled on a CNC,
//! tapped by hand on the manual mill. Tools come from the library; a pair
//! whose tool is missing is still added, without it, and the status says
//! which tool to add.
use crate::{
    cam,
    mesh::Mesh,
    model::{
        DRILL_HOLES, Document, FROM_MODEL, HELICAL_MILLING, Machine, Operation, Strategy, TAPPING,
        THREAD_MILLING, Tool, get, num,
    },
};

/// What Add threading adds to a setup, and what to tell the user.
pub struct Added {
    pub ops: Vec<Operation>,
    pub status: String,
}

/// One thread size's holes opening in one face: what one pair of
/// operations makes.
struct Group {
    rim: f32,
    surfaces: Vec<usize>,
    holes: Vec<cam::Hole>,
}

/// The operations that drill and thread the threaded holes of setup `si`
/// that open upwards, or why there are none.
pub fn add(doc: &Document, si: usize, mesh: &Mesh) -> Result<Added, String> {
    let setup = doc.setups.get(si).ok_or("Choose a setup.")?;
    let machine = &setup.machine;
    if crate::lathe::is_lathe(machine) {
        return lathe(doc, setup, mesh);
    }
    let groups = groups(setup, mesh);
    if groups.is_empty() {
        return Err(if crate::thread::in_setup(setup, mesh).is_empty() {
            "The part has no modeled thread.".into()
        } else {
            "No threaded hole opens upwards in this setup: add a setup with its face up.".into()
        });
    }
    let hand = crate::manual::is_hand(machine);
    let mut ops = vec![];
    let mut done = vec![];
    let mut missing = vec![];
    for g in &groups {
        let first = &g.holes[0];
        let t = first.thread.as_ref().expect("grouped holes are threaded");
        let name = t.name();
        let tap_drill = t.spec.map_or(t.major - t.pitch, |s| s.tap_drill);
        let through = g.holes.iter().all(|h| h.through);
        let selection = |op: &mut Operation| {
            op.geometry.clear();
            op.surfaces = Some(crate::surface::Selection {
                mesh_fingerprint: mesh.fingerprint.clone(),
                ids: g.surfaces.clone(),
            });
            op.p.insert("holes".into(), DRILL_HOLES[2].into());
            op.p.insert("hole_diameter".into(), format!("{:.2}", first.diameter));
            // A coolant the machine has: its UC100 profile may have no
            // flood output, and a run refuses one it lacks.
            op.p.insert(
                "coolant".into(),
                crate::control::default_coolant(machine).into(),
            );
        };
        // The tap drill: a drill of its size, else (on the CNC) an end mill
        // milling the bore out.
        let drill = tap_drill_tool(&doc.tools, machine, tap_drill, first.widest_drill());
        let mill = (drill.is_none() && !hand)
            .then(|| bore_mill(&doc.tools, machine, first))
            .flatten();
        let mut hole = Operation::for_machine(
            Strategy::Drill,
            drill.or(mill).map(|t| t.id.clone()).unwrap_or_default(),
            machine,
        );
        selection(&mut hole);
        hole.p.insert("name".into(), format!("{name} · tap drill"));
        hole.p
            .insert("through".into(), if through { "Yes" } else { "No" }.into());
        // Through the part far enough that the drill's 118° point is past
        // what follows it: the thread mill's tooth, half a pitch past the
        // thread, or a tap's lead, up to five pitches past it.
        // Under the stock is air: the drill needs to go no further than
        // its point past the stock's bottom, nor into what holds it.
        let past = if hand { 5. * t.pitch } else { t.pitch };
        let below = crate::stock::resolve(setup, Some(mesh))
            .map_or(f32::INFINITY, |r| first.bottom - r.stock.min[2]);
        hole.p.insert(
            "breakthrough".into(),
            format!(
                "{:.1}",
                (0.3 * tap_drill + past + 0.05)
                    .min(below + 0.3 * tap_drill + 0.5)
                    .max(1.)
            ),
        );
        if mill.is_some() {
            hole.p.insert("cycle".into(), HELICAL_MILLING.into());
        } else if first.top - first.bottom > 3. * tap_drill {
            hole.p.insert("cycle".into(), "Deep drilling".into());
            hole.p.insert("peck".into(), format!("{:.1}", tap_drill));
        }
        if drill.is_none() && mill.is_none() {
            missing.push(format!("a Ø{} drill", trim(tap_drill)));
        }
        // The thread.
        let cutter = if hand {
            tools(&doc.tools, machine)
                .filter(|t| t.tap_spec().is_some_and(|s| s.name == name))
                .next()
        } else {
            thread_mill(&doc.tools, machine, &g.holes)
        };
        let mut thread = Operation::for_machine(
            Strategy::Drill,
            cutter.map(|t| t.id.clone()).unwrap_or_default(),
            machine,
        );
        selection(&mut thread);
        thread.p.insert("name".into(), format!("{name} · thread"));
        thread.p.insert(
            "cycle".into(),
            if hand { TAPPING } else { THREAD_MILLING }.into(),
        );
        thread.p.insert("thread".into(), FROM_MODEL.into());
        thread.p.insert("thread_hand".into(), FROM_MODEL.into());
        if hand {
            // Turned by hand: the slowest gear, one pitch a turn, times it.
            let slow = crate::manual::gears(machine)
                .into_iter()
                .fold(f32::INFINITY, f32::min);
            if slow.is_finite() {
                thread.p.insert("rpm".into(), slow.to_string());
                thread
                    .p
                    .insert("feed".into(), format!("{:.1}", slow * t.pitch));
            }
        }
        if cutter.is_none() {
            missing.push(if hand {
                format!("an {name} tap")
            } else {
                format!(
                    "a {}° thread mill for a {} mm pitch, under Ø{:.2}, reaching {:.1} mm",
                    t.angle,
                    trim(t.pitch),
                    first.diameter,
                    reach_needed(&g.holes)
                )
            });
        }
        done.push(format!("{}× {name}", g.holes.len()));
        let _ = g.rim;
        ops.push(hole);
        ops.push(thread);
    }
    let mut status = format!(
        "Added {} to {}: {}.",
        if hand {
            "drilling and tapping"
        } else {
            "drilling and thread milling"
        },
        setup.name(),
        done.join(", ")
    );
    if !missing.is_empty() {
        status.push_str(&format!(
            " Add to the library, then choose in the operation: {}.",
            missing.join("; ")
        ));
    }
    Ok(Added { ops, status })
}
/// On the lathe: a Turn of kind Thread, filled from the thread on the
/// part's axis, with a die of its size for an outside thread when the
/// library has one, else a threading tool whose pitches hold it.
fn lathe(doc: &Document, setup: &crate::model::Setup, mesh: &Mesh) -> Result<Added, String> {
    let machine = &setup.machine;
    let fields = crate::lathe::thread_from_part(setup, mesh)
        .ok_or("The part has no thread on the lathe's axis.")?;
    let value = |k: &str| {
        fields
            .iter()
            .find(|(key, _)| *key == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let (outside, major, pitch) = (
        value("thread_side") == "Outside",
        value("diameter").parse::<f32>().unwrap_or(0.),
        value("pitch").parse::<f32>().unwrap_or(0.),
    );
    let die = tools(&doc.tools, machine).find(|t| {
        outside
            && crate::lathe::die_spec(t)
                .is_some_and(|s| (s.major - major).abs() <= 0.3 && (s.pitch - pitch).abs() < 1e-3)
    });
    let single = tools(&doc.tools, machine).find(|t| {
        get(&t.p, "type") == "Threading tool"
            && t.thread_pitches()
                .is_some_and(|(lo, hi)| pitch >= lo - 1e-4 && pitch <= hi + 1e-4)
    });
    let tool = die.or(single);
    let mut op = Operation::for_machine(
        Strategy::Turn,
        tool.map(|t| t.id.clone()).unwrap_or_default(),
        machine,
    );
    op.set_turn_kind("Thread");
    for (k, v) in &fields {
        op.p.insert((*k).into(), v.clone());
    }
    let mut status = format!(
        "Added a thread to {}: Ø{} × {} {}, {}.",
        setup.name(),
        trim(major),
        trim(pitch),
        if outside { "outside" } else { "inside" },
        match (die, single) {
            (Some(d), _) => format!("cut with {} by hand", d.name()),
            (None, Some(t)) => format!("single point with {}", t.name()),
            _ => "no tool yet".into(),
        }
    );
    if tool.is_none() {
        status.push_str(&format!(
            " Add a threading tool for a {} mm pitch{} to the library.",
            trim(pitch),
            if outside {
                ", or a die of the size"
            } else {
                ""
            }
        ));
    }
    Ok(Added {
        ops: vec![op],
        status,
    })
}
/// The threaded holes of `setup` opening up its faces, by face and thread
/// size and hand: each face a plane of surfaces facing up, read as a Drill
/// reads a picked rim.
fn groups(setup: &crate::model::Setup, mesh: &Mesh) -> Vec<Group> {
    if crate::thread::holes_in_setup(setup, mesh).is_empty() {
        return vec![];
    }
    let placement = crate::stock::Placement::new(mesh, Some(setup));
    let (angles, _) = placement.parts();
    let scale = mesh.display_scale();
    let mut heights: Vec<f32> = mesh
        .surfaces
        .iter()
        .filter(|s| crate::mesh::rotation(s.normal, angles)[2] > 0.999)
        .map(|s| (placement.transform(s.center)[2] / scale * 100.).round() / 100.)
        .collect();
    heights.sort_by(|a, b| b.total_cmp(a));
    heights.dedup();
    let mut groups: Vec<Group> = vec![];
    let mut seen: Vec<[f32; 2]> = vec![];
    for z in heights {
        let surfaces = cam::rim_surfaces(setup, mesh, z);
        let mut probe = Operation::new(Strategy::Drill, String::new());
        probe.geometry.clear();
        probe.surfaces = Some(crate::surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: surfaces.clone(),
        });
        let Ok(holes) = cam::holes(setup, &probe, mesh) else {
            continue;
        };
        for h in holes {
            let Some(t) = &h.thread else { continue };
            if seen
                .iter()
                .any(|c| (c[0] - h.center[0]).hypot(c[1] - h.center[1]) < 0.05)
            {
                continue;
            }
            seen.push(h.center);
            let key = (t.name(), t.right_hand);
            match groups.iter_mut().find(|g| {
                g.rim == z
                    && g.holes[0]
                        .thread
                        .as_ref()
                        .is_some_and(|o| (o.name(), o.right_hand) == key)
            }) {
                Some(g) => g.holes.push(h),
                None => groups.push(Group {
                    rim: z,
                    surfaces: surfaces.clone(),
                    holes: vec![h],
                }),
            }
        }
    }
    groups
}
/// The library's tools on `machine`.
fn tools<'a>(tools: &'a [Tool], machine: &'a Machine) -> impl Iterator<Item = &'a Tool> {
    tools
        .iter()
        .filter(move |t| t.fits(machine) && t.issues().is_empty())
}
/// The drill nearest `size` from a tenth under it up to `widest`.
fn tap_drill_tool<'a>(
    tools: &'a [Tool],
    machine: &'a Machine,
    size: f32,
    widest: f32,
) -> Option<&'a Tool> {
    self::tools(tools, machine)
        .filter(|t| get(&t.p, "type") == "Drill")
        .filter_map(|t| Some((t, num(&t.p, "diameter")?)))
        .filter(|(_, d)| *d >= size - 0.1 && *d <= widest)
        .min_by(|a, b| (a.1 - size).abs().total_cmp(&(b.1 - size).abs()))
        .map(|(t, _)| t)
}
/// The widest flat end mill that mills `hole`'s bore by Helical milling:
/// narrower than the bore, its flutes as long as the hole is deep.
fn bore_mill<'a>(tools: &'a [Tool], machine: &'a Machine, hole: &cam::Hole) -> Option<&'a Tool> {
    self::tools(tools, machine)
        .filter(|t| get(&t.p, "type") == "Flat end mill")
        .filter_map(|t| Some((t, num(&t.p, "diameter")?, num(&t.p, "length")?)))
        .filter(|(_, d, l)| *d <= hole.diameter - 0.1 && *l > hole.top - hole.bottom)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(t, _, _)| t)
}
/// How deep a thread mill must reach for every hole of `holes`: its thread
/// and the half pitch its tooth reaches below it.
fn reach_needed(holes: &[cam::Hole]) -> f32 {
    holes
        .iter()
        .filter_map(|h| {
            let t = h.thread.as_ref()?;
            Some(h.top - t.from.max(h.bottom) + t.pitch / 2.)
        })
        .fold(0., f32::max)
}
/// The thread mill that cuts every hole of `holes`: their angle and pitch,
/// narrower than their bore, reaching their depth. The widest with a tenth
/// of the bore to spare going down it, else the widest that goes down.
fn thread_mill<'a>(
    tools: &'a [Tool],
    machine: &'a Machine,
    holes: &[cam::Hole],
) -> Option<&'a Tool> {
    let t = holes[0].thread.as_ref()?;
    let bore = holes
        .iter()
        .map(|h| h.diameter)
        .fold(f32::INFINITY, f32::min);
    let reach = reach_needed(holes);
    self::tools(tools, machine)
        .filter(|tool| get(&tool.p, "type") == "Thread mill")
        .filter(|tool| num(&tool.p, "thread_angle").is_some_and(|a| (a - t.angle).abs() < 0.5))
        .filter(|tool| {
            tool.thread_pitches()
                .is_some_and(|(lo, hi)| t.pitch >= lo - 1e-4 && t.pitch <= hi + 1e-4)
        })
        .filter(|tool| num(&tool.p, "reach").is_some_and(|r| r >= reach))
        .filter_map(|tool| Some((tool, num(&tool.p, "diameter")?)))
        .filter(|(_, d)| *d <= bore - 0.02)
        .max_by(|a, b| {
            let roomy = |d: f32| d <= 0.9 * bore;
            roomy(a.1).cmp(&roomy(b.1)).then(a.1.total_cmp(&b.1))
        })
        .map(|(tool, _)| tool)
}
fn trim(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plate() -> Mesh {
        crate::mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/threaded-plate.3mf"),
        )
        .unwrap()
    }
    fn doc(machine: Machine, tools: Vec<Tool>) -> Document {
        let mut setup = crate::model::Setup::new(machine.clone(), "G54");
        setup.p.insert("stock_top".into(), "1".into());
        let mut doc = Document::default();
        doc.machines = vec![machine];
        doc.tools = tools;
        doc.setups = vec![setup];
        doc
    }
    fn tool(name: &str, kind: &str, d: &str, extra: &[(&str, &str)]) -> Tool {
        let mut t = Tool::new(name, kind, d);
        t.set_type(kind);
        for (k, v) in [("length", "20"), ("stickout", "40")].iter().chain(extra) {
            t.p.insert((*k).into(), (*v).into());
        }
        t
    }

    #[test]
    fn the_cnc_drills_and_thread_mills_each_size_with_the_tools_that_fit() {
        let tools = vec![
            tool("Ø5 drill", "Drill", "5", &[]),
            tool("Ø6.8 drill", "Drill", "6.8", &[]),
            tool(
                "Thread mill Ø4",
                "Thread mill",
                "4",
                &[("pitches", "0.5-1.25"), ("reach", "16")],
            ),
            tool(
                "Thread mill Ø3.15",
                "Thread mill",
                "3.15",
                &[("pitches", "0.5-0.8"), ("reach", "10")],
            ),
        ];
        let doc = doc(Machine::new(3), tools);
        let mesh = plate();
        let added = add(&doc, 0, &mesh).unwrap();
        let names: Vec<&str> = added.ops.iter().map(|o| o.name()).collect();
        // Every size: M5 (blind), M6 ×2, M8, each a tap drill and a thread.
        assert_eq!(added.ops.len(), 6, "{names:?}");
        for o in &added.ops {
            assert!(o.threaded_holes_only(), "{}", o.name());
        }
        let tool_of = |name: &str| {
            let o = added.ops.iter().find(|o| o.name() == name).unwrap();
            doc.tools
                .iter()
                .find(|t| t.id == o.tool)
                .map(|t| t.name().to_string())
        };
        assert_eq!(tool_of("M6×1 · tap drill").as_deref(), Some("Ø5 drill"));
        assert_eq!(
            tool_of("M8×1.25 · tap drill").as_deref(),
            Some("Ø6.8 drill")
        );
        assert_eq!(tool_of("M6×1 · thread").as_deref(), Some("Thread mill Ø4"));
        // M5's Ø4.1 bore takes the Ø3.15 mill; no Ø4.2 drill: it is said.
        assert_eq!(
            tool_of("M5×0.8 · thread").as_deref(),
            Some("Thread mill Ø3.15")
        );
        assert_eq!(tool_of("M5×0.8 · tap drill"), None);
        assert!(added.status.contains("a Ø4.2 drill"), "{}", added.status);
        // The ops it adds generate: the M6 pair, in a job of its own.
        let mut job = doc.clone();
        job.setups[0].operations = added
            .ops
            .iter()
            .filter(|o| o.name().starts_with("M6×1"))
            .cloned()
            .collect();
        let mut results = vec![];
        crate::simulation::sequence(&job, &mesh, None, |_, _, r| results.push(r));
        for r in results {
            r.unwrap();
        }
    }
    /// Writes a demo workspace to `$AXIS_DEMO_HOME` (default
    /// /tmp/axis-demo-home)'s app folder: the threaded plate on the owner's
    /// CNC4030 with the LUOCUT DLC thread mills and the RUKO tap drills,
    /// Add threading applied. Launch the app with that HOME to see it. Not
    /// a check: run it with --ignored.
    #[test]
    #[ignore]
    fn write_a_demo_workspace() {
        let home = std::path::PathBuf::from(
            std::env::var("AXIS_DEMO_HOME").unwrap_or("/tmp/axis-demo-home".into()),
        );
        let dir = home.join("Library/Application Support/AXIS CAM Studio");
        std::fs::create_dir_all(&dir).unwrap();
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/threaded-plate.3mf");
        let mesh = crate::mesh::load(&path).unwrap();
        let machine = crate::control::owners_machine(&dir).unwrap();
        let mut tools = vec![];
        // The owner's 3-tooth thread mills: cutting Ø, the one pitch their
        // teeth are spaced at (a guess, M3 to M8, until read off the
        // shanks), reach (L1), shank Ø.
        for (n, (d, pitches, reach, shank)) in [
            ("2.35", "0.5", "8", "4"),
            ("3.15", "0.7", "10", "4"),
            ("4", "0.8", "12", "4"),
            ("4.8", "1", "16", "6"),
            ("6", "1.25", "16", "6"),
        ]
        .into_iter()
        .enumerate()
        {
            tools.push(tool(
                &format!("Thread mill Ø{d} DLC"),
                "Thread mill",
                d,
                &[
                    ("pitches", pitches),
                    ("reach", reach),
                    ("teeth", "3"),
                    ("shank", shank),
                    ("flutes", "4"),
                    ("length", "3"),
                    ("stickout", "35"),
                    ("number", &(10 + n).to_string()),
                ],
            ));
        }
        // RUKO 245052RO's tap drills.
        for (n, d) in ["4.2", "5", "6.8"].into_iter().enumerate() {
            tools.push(tool(
                &format!("Drill Ø{d}"),
                "Drill",
                d,
                &[
                    ("length", "40"),
                    ("stickout", "50"),
                    ("number", &(20 + n).to_string()),
                ],
            ));
        }
        let mut doc = doc(machine, tools);
        doc.title = "Threaded plate".into();
        doc.file = Some(crate::model::ModelFile {
            name: "threaded-plate.3mf".into(),
            path: path.clone(),
            format: "3mf".into(),
            bytes: std::fs::metadata(&path).unwrap().len(),
        });
        doc.adopt_materials();
        if let Some(al) = doc
            .materials
            .iter()
            .find(|m| m.name().contains("6061"))
            .map(|m| m.id.clone())
        {
            doc.select_material(&al);
        }
        doc.setups[0].p.insert("name".into(), "Top side".into());
        doc.active = doc.setups[0].id.clone();
        let added = add(&doc, 0, &mesh).unwrap();
        eprintln!("{}", added.status);
        // On the owner's machine, whose profile has no flood output, the
        // ops it adds pass the run's checks.
        let mut job = doc.clone();
        job.setups[0].operations = added.ops.clone();
        let mut sims = vec![];
        crate::simulation::sequence(&job, &mesh, None, |_, oi, r| sims.push((oi, r)));
        if let Err(problems) = crate::run::plan_problems(&job, &mesh, 0, &sims) {
            panic!("{problems:?}");
        }
        doc.setups[0].operations = added.ops;
        for o in &mut doc.setups[0].operations {
            let (rpm, feed) = if o.thread_milling() {
                ("12000", "300")
            } else {
                ("3000", "150")
            };
            o.p.insert("rpm".into(), rpm.into());
            o.p.insert("feed".into(), feed.into());
        }
        crate::storage::save(&dir.join("workspace.json"), &doc).unwrap();
        let mut results = vec![];
        crate::simulation::sequence(&doc, &mesh, None, |_, oi, r| {
            results.push((oi, r.map(|s| s.plan.duration)))
        });
        for (oi, r) in results {
            eprintln!("{} {:?}", doc.setups[0].operations[oi].name(), r);
        }
    }
    #[test]
    fn on_the_owners_cnc_the_ops_added_pass_the_runs_checks() {
        // Its UC100 profile has no flood output: the ops take its coolant.
        let dir = tempfile::tempdir().unwrap();
        let machine = crate::control::owners_machine(dir.path()).unwrap();
        let tools = vec![
            tool(
                "Ø5 drill",
                "Drill",
                "5",
                &[("length", "40"), ("stickout", "50")],
            ),
            tool(
                "Thread mill Ø4.8",
                "Thread mill",
                "4.8",
                &[
                    ("pitches", "0.5-1.25"),
                    ("reach", "15"),
                    ("length", "1"),
                    ("stickout", "35"),
                ],
            ),
        ];
        let mut doc = doc(machine, tools);
        for (n, t) in doc.tools.iter_mut().enumerate() {
            t.p.insert("number".into(), (n + 1).to_string());
        }
        let mesh = plate();
        let added = add(&doc, 0, &mesh).unwrap();
        doc.setups[0].operations = added
            .ops
            .into_iter()
            .filter(|o| o.name().starts_with("M6×1"))
            .collect();
        for o in &mut doc.setups[0].operations {
            o.p.insert("rpm".into(), "12000".into());
            o.p.insert("feed".into(), "300".into());
        }
        let mut sims = vec![];
        crate::simulation::sequence(&doc, &mesh, None, |_, oi, r| sims.push((oi, r)));
        assert_eq!(crate::run::plan_problems(&doc, &mesh, 0, &sims), Ok(()));
    }
    #[test]
    fn the_owners_tools_are_given_once_and_are_valid() {
        let mut doc = Document::default();
        doc.adopt_tools();
        // 27 taps, 27 dies and the insert, each checking clean.
        assert_eq!(doc.tools.len(), 55);
        for t in &doc.tools {
            assert!(t.issues().is_empty(), "{}: {:?}", t.name(), t.issues());
        }
        let mut manual = Machine::new(3);
        crate::manual::set_kind(&mut manual, "Manual");
        let tap = doc.tools.iter().find(|t| t.name() == "M6×1 tap").unwrap();
        assert!(tap.fits(&manual) && !tap.fits(&Machine::new(3)));
        // Deleted, a tool stays deleted.
        doc.tools.retain(|t| t.name() != "M6×1 tap");
        doc.adopt_tools();
        assert_eq!(doc.tools.len(), 54);
    }
    #[test]
    fn the_manual_mill_taps_with_the_sets_taps() {
        let mut manual = Machine::new(3);
        crate::manual::set_kind(&mut manual, "Manual");
        let mut tap = tool("M6 tap", "Tap", "6", &[("thread", "M6×1")]);
        tap.p.insert("machines".into(), manual.id.clone());
        let mut drill = tool("Ø5 drill", "Drill", "5", &[]);
        drill.p.insert("machines".into(), manual.id.clone());
        let doc = doc(manual, vec![tap, drill]);
        let added = add(&doc, 0, &plate()).unwrap();
        let m6 = added
            .ops
            .iter()
            .find(|o| o.name() == "M6×1 · thread")
            .unwrap();
        assert!(m6.tapping());
        assert!(!m6.tool.is_empty());
        assert!(added.status.contains("an M8×1.25 tap"), "{}", added.status);
    }
}
