//! An agent plans whole jobs through `axis-cam --mcp`, end to end (LIO-470):
//! the real binary over a pipe, driven only with MCP tool calls, as Claude
//! Code or Codex would drive it. Each test's data folder is a temporary one
//! (`AXIS_CAM_DATA_DIR`), so it starts from the app's starter library and
//! the user's workspace is never touched.
//!
//! - The STEP fixture on the CNC: two setups (the second turned over) with
//!   Face, Pocket, Contour and Drill, checked, exported and compared with
//!   `--export-gcode`, saved, opened again, and its session resumed.
//! - A manual mill setup and a lathe setup, each built with commands only,
//!   and their steps read without moving the machinist's place.
//!
//! `docs/verification/mcp-m1-e2e.md` describes the job. With
//! `MCP_E2E_PICTURE=<file.png>` the STEP test writes its picture of the
//! finished job there.
use base64::Engine;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_axis-cam");
/// How long one call may take: the server answers within its `wait_s` (at
/// most 50 s) and a still-running call is waited for again.
const CALL: Duration = Duration::from_secs(240);

/// `axis-cam --mcp` on a pipe, as an MCP client starts it.
struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: mpsc::Receiver<String>,
    next: u64,
}

impl Server {
    /// A server on data folder `data`, in a new session or in `session`,
    /// initialized.
    fn start(data: &Path, session: Option<&str>) -> Server {
        let mut command = Command::new(BIN);
        command.arg("--mcp");
        if let Some(id) = session {
            command.args(["--session", id]);
        }
        let mut child = command
            .env("AXIS_CAM_DATA_DIR", data)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let out = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let stdin = child.stdin.take();
        let mut server = Server {
            child,
            stdin,
            lines,
            next: 0,
        };
        let init = server.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "mcp_e2e", "version": "1"},
            }),
        );
        assert_eq!(init["serverInfo"]["name"], "axis-cam", "{init}");
        server.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        server
    }
    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }
    /// A JSON-RPC request's result.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let line = (self.lines.recv_timeout(CALL))
                .unwrap_or_else(|_| panic!("no answer to {method} {params} in {CALL:?}"));
            let v: Value = serde_json::from_str(&line).expect("a JSON line");
            if v["id"] == id {
                assert!(v["error"].is_null(), "{method} {params}: {v}");
                return v["result"].clone();
            }
        }
    }
    /// A tool's whole result (content and structured content); a tool
    /// error fails the test. A call still working is waited for, as an
    /// agent does.
    fn raw(&mut self, name: &str, args: Value) -> Value {
        let mut r = self.request("tools/call", json!({"name": name, "arguments": args}));
        assert_ne!(r["isError"], true, "{name} {args}: {r}");
        while r["structuredContent"]["outcome"] == "still_running" {
            let id = r["structuredContent"]["command_id"].clone();
            assert!(id.is_string(), "{r}");
            r = self.request(
                "tools/call",
                json!({"name": "wait", "arguments": {"command_id": id, "wait_s": 50}}),
            );
            assert_ne!(r["isError"], true, "wait for {name} {args}: {r}");
        }
        r
    }
    /// A tool's structured result.
    fn tool(&mut self, name: &str, args: Value) -> Value {
        self.raw(name, args)["structuredContent"].clone()
    }
    /// Runs a command, which must come to done or no_change.
    fn run(&mut self, command: &str, args: Value) -> Value {
        let r = self.tool(
            "execute",
            json!({"command": command, "args": args, "wait_s": 50}),
        );
        assert!(
            matches!(r["outcome"].as_str(), Some("done" | "no_change")),
            "{command} {args}: {r}"
        );
        r
    }
    /// Runs `steps` as batches: a step still working when a batch's time is
    /// up is waited for and the steps not run are sent again, as an agent
    /// does. Every step must come to done or no_change.
    fn batch(&mut self, steps: Vec<Value>) {
        let mut rest = steps;
        while !rest.is_empty() {
            let r = self.request(
                "tools/call",
                json!({"name": "batch", "arguments": {"steps": rest, "wait_s": 50}}),
            );
            assert_ne!(r["isError"], true, "batch {rest:?}: {r}");
            let mut went = 0;
            for (step, sent) in r["structuredContent"]["steps"]
                .as_array()
                .unwrap()
                .iter()
                .zip(&rest)
            {
                let outcome = match step["outcome"].as_str() {
                    Some("still_running") => {
                        let id = step["command_id"].clone();
                        let mut w = json!({"outcome": "still_running"});
                        while w["outcome"] == "still_running" {
                            w = self.tool("wait", json!({"command_id": id, "wait_s": 50}));
                        }
                        w["outcome"].as_str().unwrap_or_default().to_string()
                    }
                    Some("not_run") => break,
                    other => other.unwrap_or_default().to_string(),
                };
                assert!(
                    matches!(outcome.as_str(), "done" | "no_change"),
                    "{sent}: {step}"
                );
                went += 1;
            }
            assert!(went > 0, "nothing ran: {r}");
            rest.drain(..went);
        }
    }
    /// Ends the client (stdin closed): the server ends, its session kept.
    fn end(mut self) {
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(120);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "the server did not end");
            std::thread::sleep(Duration::from_millis(100));
        };
        assert!(status.success(), "{status}");
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if self.stdin.is_some() {
            let _ = self.child.kill();
        }
    }
}

/// An `edit_field` step.
fn field(target: &str, key: &str, value: &str) -> Value {
    json!({"command": "edit_field", "args": {"target": target, "key": key, "value": value}})
}
/// An `edit_field` step on operation `op`.
fn op_field(op: &str, key: &str, value: &str) -> Value {
    json!({"command": "edit_field", "args": {"target": "operation", "operation": op, "key": key, "value": value}})
}
fn step(command: &str, args: Value) -> Value {
    json!({"command": command, "args": args})
}

/// The id of library entry `name` among `inspect {what: "libraries"}`'s
/// `kind` (machines, tools, materials, holders).
fn library_id(s: &mut Server, kind: &str, name: &str) -> String {
    let libraries = s.tool("inspect", json!({"what": "libraries"}));
    (libraries[kind].as_array().unwrap().iter())
        .find(|e| e["fields"]["name"] == name)
        .unwrap_or_else(|| panic!("no {name} in the {kind}: {libraries}"))["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Makes a machine in the wizard: its kind and fields, then saved. Its id.
fn make_machine(s: &mut Server, kind: &str, fields: &[(&str, &str)]) -> String {
    let mut steps = vec![
        step("new_machine", json!({})),
        field("machine", "kind", kind),
    ];
    steps.extend(fields.iter().map(|(k, v)| field("machine", k, v)));
    steps.push(step("save_machine", json!({})));
    s.batch(steps);
    let name = fields.iter().find(|(k, _)| *k == "name").unwrap().1;
    library_id(s, "machines", name)
}

/// Makes a tool in the tool editor: its type, then its fields, on
/// `machine` when it is for a hand machine. Its id.
fn make_tool(s: &mut Server, kind: &str, fields: &[(&str, &str)], machine: Option<&str>) -> String {
    let mut steps = vec![step("new_tool", json!({})), field("tool", "type", kind)];
    steps.extend(fields.iter().map(|(k, v)| field("tool", k, v)));
    if let Some(id) = machine {
        steps.push(step("tool_machine", json!({"id": id, "checked": true})));
    }
    steps.push(step("save_tool", json!({})));
    s.batch(steps);
    let name = fields.iter().find(|(k, _)| *k == "name").unwrap().1;
    library_id(s, "tools", name)
}

/// Adds an operation to the active setup with `tool`, its picks (replace),
/// recommended speeds when it mills, and `fields`. Its id.
fn add_op(
    s: &mut Server,
    strategy: &str,
    tool: &str,
    picks: &[u64],
    fields: &[(&str, &str)],
) -> String {
    let added = s.run("add_operation", json!({"strategy": strategy}));
    let op = added["selection"]["operation"]
        .as_str()
        .unwrap()
        .to_string();
    let mut steps = vec![step(
        "set_operation_tool",
        json!({"operation": op, "id": tool}),
    )];
    if !picks.is_empty() {
        steps.push(step(
            "pick_surfaces",
            json!({"operation": op, "ids": picks, "mode": "replace"}),
        ));
    }
    if strategy != "Drill" {
        steps.push(step("recommend_speeds", json!({"operation": op})));
    }
    steps.extend(fields.iter().map(|(k, v)| op_field(&op, k, v)));
    s.batch(steps);
    op
}

fn plane<'a>(planes: &'a [Value], kind: &str, depth: f64) -> &'a Value {
    (planes.iter())
        .find(|p| {
            p["kind"] == kind && (p["depth_below_top"].as_f64().unwrap() - depth).abs() < 0.01
        })
        .unwrap_or_else(|| panic!("no {kind} at {depth} mm: {planes:?}"))
}
fn ids(plane: &Value) -> Vec<u64> {
    (plane["surfaces"].as_array().unwrap().iter())
        .map(|v| v.as_u64().unwrap())
        .collect()
}
fn f(v: &Value) -> f64 {
    v.as_f64().unwrap()
}

/// For each hole of `holes` (part report holes: centre from the part's
/// -X/-Y corner, diameter), one surface of its wall, from `surfaces`
/// (list_surfaces, in the part's own frame) below `below` mm.
fn hole_walls(surfaces: &[Value], holes: &[&Value], below: f64) -> Vec<u64> {
    // The part's -X and -Y sides, where the report's corner is.
    let side = |axis: usize| {
        (surfaces.iter())
            .filter(|s| f(&s["normal"][axis]) < -0.99)
            .map(|s| f(&s["centre"][axis]))
            .fold(f64::MAX, f64::min)
    };
    let (x0, y0) = (side(0), side(1));
    holes
        .iter()
        .map(|h| {
            let (x, y, r) = (
                f(&h["center"][0]) + x0,
                f(&h["center"][1]) + y0,
                f(&h["diameter"]) / 2.,
            );
            (surfaces.iter())
                .find(|s| {
                    let c = &s["centre"];
                    f(&s["normal"][2]).abs() < 0.1
                        && f(&c[2]) < below
                        && ((f(&c[0]) - x).hypot(f(&c[1]) - y) - r).abs() < 0.2
                })
                .unwrap_or_else(|| panic!("no wall for the hole at {x}, {y}"))["id"]
                .as_u64()
                .unwrap()
        })
        .collect()
}

/// The job as inspect lists it, without what the selection decides.
fn job(state: &Value) -> Value {
    let mut setups = state["setups"].clone();
    for s in setups.as_array_mut().unwrap() {
        s.as_object_mut().unwrap().remove("active");
    }
    setups
}

/// Every operation's fields, as list_fields gives them, by operation id.
fn all_fields(s: &mut Server, setups: &Value) -> BTreeMap<String, Value> {
    let mut all = BTreeMap::new();
    for setup in setups.as_array().unwrap() {
        for op in setup["operations"].as_array().unwrap() {
            let listed = s.tool(
                "list_fields",
                json!({"target": "operation", "setup": setup["id"], "operation": op["id"]}),
            );
            let fields: BTreeMap<String, Value> = (listed["fields"].as_array().unwrap().iter())
                .map(|f| (f["key"].as_str().unwrap().to_string(), f["value"].clone()))
                .collect();
            all.insert(op["id"].as_str().unwrap().to_string(), json!(fields));
        }
    }
    all
}

/// The programs in `folder`, by file name.
fn programs(folder: &Path) -> BTreeMap<String, String> {
    (std::fs::read_dir(folder).unwrap())
        .map(|e| e.unwrap().path())
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(&p).unwrap(),
            )
        })
        .collect()
}

#[test]
fn an_agent_plans_the_step_fixture_end_to_end() {
    // The STEP reader is the OCCT helper build.rs builds; a build without it
    // (AXIS_SKIP_STEP) cannot import the fixture.
    if option_env!("AXIS_BUILT_STEP_HELPER").is_none() {
        eprintln!("skipped: built without the STEP helper (AXIS_SKIP_STEP)");
        return;
    }
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/four-setup-block.step");
    let mut s = Server::start(data.path(), None);

    // The part. The first call of a session is held until the session has
    // started, and still waits for the load.
    let imported = s.run("import_model", json!({"path": fixture}));
    assert_eq!(imported["outcome"], "done", "{imported}");
    assert!(
        imported["pending"].as_array().unwrap().is_empty(),
        "{imported}"
    );

    // The user's CNC, the Jingyan CNC4030, made in the wizard: a fresh data
    // folder's library has none. Its UC100 profile is the user's to choose,
    // so it posts LinuxCNC programs here.
    let cnc = make_machine(
        &mut s,
        "CNC",
        &[
            ("name", "Jingyan CNC4030"),
            ("x", "305"),
            ("y", "405"),
            ("z", "100"),
            ("x_source", "Measured"),
            ("y_source", "Measured"),
            ("z_source", "Measured"),
            ("rpm", "24000"),
            ("power", "800"),
            ("feed", "1000"),
            ("nose_to_table", "160"),
            ("gantry_height", "120"),
            ("gantry_width", "390"),
        ],
    );
    s.run("add_setup", json!({"machine": cnc}));
    let aluminium = library_id(&mut s, "materials", "Aluminium 5052-H32");
    s.run("set_project_material", json!({"id": aluminium}));

    // The cutters: none in a fresh library.
    let end_mill_10 = make_tool(
        &mut s,
        "Flat end mill",
        &[
            ("name", "Flat end mill 10mm"),
            ("diameter", "10"),
            ("flutes", "3"),
            ("length", "25"),
            ("stickout", "35"),
            ("center_cutting", "Yes"),
        ],
        None,
    );
    let end_mill_3 = make_tool(
        &mut s,
        "Flat end mill",
        &[
            ("name", "Flat end mill 3mm"),
            ("diameter", "3"),
            ("flutes", "3"),
            ("length", "12"),
            ("stickout", "20"),
            ("center_cutting", "Yes"),
        ],
        None,
    );
    // Ø6.6, an M6 clearance hole, 40 mm through the part and into the 3 mm
    // of stock under it: 48 mm of flute, and short enough to fit under the
    // spindle over the vise (160 mm nose to table).
    let drill = make_tool(
        &mut s,
        "Drill",
        &[
            ("name", "Drill 6.6mm"),
            ("diameter", "6.6"),
            ("flutes", "2"),
            ("length", "48"),
            ("stickout", "52"),
        ],
        None,
    );

    // Setup 1, top up: 3 mm of stock round the 120 x 80 x 40 block, 2 mm
    // on top and 3 mm under it (room for the drill's point past the
    // through holes' exit), in the vise.
    let stock = [
        ("stock_top", "2"),
        ("stock_bottom", "3"),
        ("stock_left", "3"),
        ("stock_right", "3"),
        ("stock_front", "3"),
        ("stock_back", "3"),
        ("holding", "Vise"),
    ];
    s.batch(stock.iter().map(|(k, v)| field("setup", k, v)).collect());
    let listed = s.tool("list_fields", json!({"target": "setup"}));
    let value = |key: &str| {
        (listed["fields"].as_array().unwrap().iter())
            .find(|f| f["key"] == key)
            .unwrap()["value"]
            .clone()
    };
    assert_eq!(value("stock_left"), "3");
    assert_eq!(value("holding"), "Vise");

    // The part as the checks read it, and its surfaces.
    let report = s.tool("part_report", json!({}));
    let top = &report["report"]["orientations"][0];
    assert_eq!(top["is_setup1"], true, "{top}");
    assert_eq!(top["model_size"], json!([120.0, 80.0, 40.0]));
    let planes = top["planes"].as_array().unwrap();
    let face_top = ids(plane(planes, "top", 0.));
    let pocket_floor = ids(plane(planes, "floor", 12.));
    let counterbores = ids(plane(planes, "floor", 6.));
    let blind_hole = ids(plane(planes, "floor", 8.));
    let through: Vec<&Value> = (plane(planes, "top", 0.)["holes"].as_array().unwrap().iter())
        .filter(|h| h["through"] == true)
        .collect();
    assert_eq!(through.len(), 4, "{through:?}");
    assert!(
        through
            .iter()
            .all(|h| (f(&h["diameter"]) - 6.6).abs() < 0.01)
    );
    let listed = s.tool("list_surfaces", json!({"limit": 5000}));
    let surfaces = listed["surfaces"].as_array().unwrap();
    // Below the Ø11 counterbores, 6 mm down from the 40 mm top.
    let walls = hole_walls(surfaces, &through, 33.);

    // Face the whole stock top (setup 2 rests on it), pocket with the
    // 10 mm cutter, the counterbores and the flat-bottomed Ø6 hole with the
    // 3 mm one, the outside down to 18 mm above the part's bottom (the
    // 35 mm stick-out and the jaws), then drill the through holes, pecking.
    let mut ops1 = vec![];
    ops1.push(add_op(
        &mut s,
        "Face",
        &end_mill_10,
        &face_top,
        &[
            ("boundary", "Stock silhouette"),
            ("containment", "Tool outside boundary"),
        ],
    ));
    ops1.push(add_op(
        &mut s,
        "Pocket",
        &end_mill_10,
        &pocket_floor,
        &[
            ("stepdown", "1.5"),
            ("allowance", "0"),
            ("axial_allowance", "0"),
        ],
    ));
    let mut small: Vec<u64> = counterbores.clone();
    small.extend(&blind_hole);
    ops1.push(add_op(
        &mut s,
        "Pocket",
        &end_mill_3,
        &small,
        &[
            ("stepdown", "1"),
            ("allowance", "0"),
            ("axial_allowance", "0"),
            ("name", "Counterbores and Ø6 hole"),
        ],
    ));
    ops1.push(add_op(
        &mut s,
        "Contour",
        &end_mill_10,
        &[],
        &[("bottom", "18"), ("stepdown", "3"), ("allowance", "0")],
    ));
    // Drilling has no cutting data in the library: 6000 rpm is 124 m/min
    // at Ø6.6, 0.08 mm a turn. The tip goes 2.5 mm past the part's bottom,
    // so the full diameter (behind the 2 mm point) breaks out, still in
    // the stock.
    ops1.push(add_op(
        &mut s,
        "Drill",
        &drill,
        &walls,
        &[
            ("rpm", "6000"),
            ("feed", "480"),
            ("cycle", "Deep drilling"),
            ("through", "Yes"),
            ("breakthrough", "2.5"),
            ("clearance", "10"),
        ],
    ));
    let state = s.tool("inspect", json!({}));
    let listed: Vec<Value> = (state["setups"][0]["operations"].as_array().unwrap().iter())
        .map(|o| o["id"].clone())
        .collect();
    assert_eq!(listed, ops1.iter().map(|id| json!(id)).collect::<Vec<_>>());
    let drilled = &state["setups"][0]["operations"][4];
    assert_eq!(drilled["name"], "Drill Ø6.6 · 4 holes", "{drilled}");

    // Setup 2, turned over about X: the bottom, faced, its pockets and Ø5 holes,
    // and the outside down to meet setup 1's.
    s.run("add_setup", json!({"machine": cnc}));
    s.run("flip_setup", json!({"axis": "rx"}));
    s.batch(vec![field("setup", "holding", "Vise")]);
    let listed = s.tool("list_surfaces", json!({"limit": 20}));
    assert_eq!(listed["up"], "-Z", "{listed}");
    let planes = listed["planes"].as_array().unwrap();
    let face_bottom = ids(plane(planes, "top", 0.));
    let bottom_pocket = ids(plane(planes, "floor", 6.));
    let small_holes = ids(plane(planes, "floor", 8.));
    add_op(
        &mut s,
        "Face",
        &end_mill_10,
        &face_bottom,
        &[
            ("boundary", "Stock silhouette"),
            ("containment", "Tool outside boundary"),
        ],
    );
    add_op(
        &mut s,
        "Pocket",
        &end_mill_10,
        &bottom_pocket,
        &[
            ("stepdown", "1.5"),
            ("allowance", "0"),
            ("axial_allowance", "0"),
        ],
    );
    add_op(
        &mut s,
        "Pocket",
        &end_mill_3,
        &small_holes,
        &[
            ("stepdown", "1"),
            ("allowance", "0"),
            ("axial_allowance", "0"),
            ("name", "Ø5 holes"),
        ],
    );
    let contour2 = add_op(
        &mut s,
        "Contour",
        &end_mill_10,
        &[],
        &[("bottom", "16"), ("stepdown", "3"), ("allowance", "0")],
    );

    // The vise placed for each setup's toolpaths.
    s.run("suggest_layout", json!({"setup": 0}));
    s.run("suggest_layout", json!({"setup": 1}));

    // Generated and checked.
    let generated = s.run("generate_all", json!({}));
    assert!(
        generated["status"]
            .as_str()
            .unwrap()
            .contains("0 need attention"),
        "{generated}"
    );
    let check = s.tool("check_job", json!({"wait_s": 50}));
    assert_eq!(check["outcome"], "done", "{check}");
    assert_eq!(check["problems"], json!([]), "{check}");
    assert_eq!(check["passed"], true, "{check}");
    assert_eq!(check["operations"].as_array().unwrap().len(), 9, "{check}");
    // Setup 2 starts from the faced block, turned over: 2 mm less at its
    // bottom, in its own axes.
    let stocks = check["stocks"].as_array().unwrap();
    assert_eq!(stocks[1]["removed"]["bottom"], 2.0, "{stocks:?}");
    assert_eq!(stocks[1]["size"], json!([126.0, 86.0, 43.0]), "{stocks:?}");
    eprintln!(
        "check_job: passed {}, {} min; {}",
        check["passed"], check["total_minutes"], check["operations"]
    );
    let holding = s.tool("check_holding", json!({"setup": 1, "wait_s": 50}));
    assert_eq!(holding["holds"], true, "{holding}");

    // The finished job: setup 2's last operation, the stock it leaves.
    let picture = s.raw(
        "view_model",
        json!({
            "operation": contour2,
            "labels": false,
            "purpose": "the finished job",
            "wait_s": 50,
        }),
    );
    let image = (picture["content"].as_array().unwrap().iter())
        .find(|c| c["type"] == "image")
        .unwrap_or_else(|| panic!("no picture: {picture}"));
    let png = base64::engine::general_purpose::STANDARD
        .decode(image["data"].as_str().unwrap())
        .unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    if let Some(path) = std::env::var_os("MCP_E2E_PICTURE") {
        std::fs::write(path, &png).unwrap();
    }

    // Exported, and saved.
    let exported = work.path().join("exported");
    std::fs::create_dir(&exported).unwrap();
    let export = s.run("export_gcode", json!({"folder": exported}));
    assert_eq!(export["outcome"], "done", "{export}");
    let project = work.path().join("four-setup-block.axis.json");
    let saved = s.run("save_project", json!({"path": project}));
    assert_eq!(saved["outcome"], "done", "{saved}");
    s.tool("checkpoint", json!({"name": "planned"}));
    let planned = s.tool("inspect", json!({}));
    assert_eq!(planned["project"]["saved"], true, "{planned}");
    let session = planned["session"]["id"].as_str().unwrap().to_string();
    let revision = planned["revision"].as_u64().unwrap();
    let fields = all_fields(&mut s, &planned["setups"]);
    s.end();

    // The same programs as the command line writes from the saved project.
    let ours = programs(&exported);
    assert_eq!(ours.len(), 2, "one program a setup: {:?}", ours.keys());
    assert!(
        ours.keys().all(|n| n.ends_with(".ngc")),
        "{:?}",
        ours.keys()
    );
    let theirs_folder = work.path().join("command-line");
    std::fs::create_dir(&theirs_folder).unwrap();
    let out = Command::new(BIN)
        .arg("--export-gcode")
        .arg(&project)
        .arg(&theirs_folder)
        .env("AXIS_CAM_DATA_DIR", data.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let theirs = programs(&theirs_folder);
    assert_eq!(
        ours.keys().collect::<Vec<_>>(),
        theirs.keys().collect::<Vec<_>>()
    );
    for (name, text) in &ours {
        let other = &theirs[name];
        let differs: Vec<(&str, &str)> = (text.lines().zip(other.lines()))
            .filter(|(a, b)| a != b)
            .collect();
        assert!(
            differs.is_empty() && text.lines().count() == other.lines().count(),
            "{name} differs: {differs:?}"
        );
        eprintln!("{name}: {} lines, the same", text.lines().count());
    }

    // Opened in a fresh session: the same job.
    let mut fresh = Server::start(data.path(), None);
    let opened = fresh.run("open_project", json!({"path": project}));
    assert_eq!(opened["outcome"], "done", "{opened}");
    let reopened = fresh.tool("inspect", json!({}));
    assert_ne!(reopened["session"]["id"], session.as_str());
    assert_eq!(reopened["project"]["saved"], true, "{reopened}");
    assert_eq!(
        reopened["project"]["material"],
        planned["project"]["material"]
    );
    assert_eq!(job(&reopened), job(&planned));
    assert_eq!(all_fields(&mut fresh, &reopened["setups"]), fields);
    fresh.end();

    // The first session resumed after its client ended: the job, its
    // revision, its change log and its checkpoint are back.
    let mut resumed = Server::start(data.path(), Some(&session));
    let state = resumed.tool("inspect", json!({}));
    assert_eq!(state["session"]["id"], session.as_str());
    assert_eq!(state["revision"], revision, "{state}");
    assert_eq!(state["user_changed"], json!([]), "{state}");
    assert_eq!(state["project"]["file"], json!(project), "{state}");
    assert_eq!(state["project"]["saved"], true, "{state}");
    assert_eq!(job(&state), job(&planned));
    let kept = &state["session"]["last_saves_and_exports"];
    assert_eq!(kept.as_array().unwrap().len(), 2, "{kept}");
    let checkpoints = resumed.tool("list_checkpoints", json!({}));
    assert_eq!(
        checkpoints["checkpoints"][0]["name"], "planned",
        "{checkpoints}"
    );
    // The revision the agent last saw still holds.
    let titled = resumed.tool(
        "execute",
        json!({
            "command": "set_project_title",
            "args": {"title": "Four-setup block, setups 1 and 2"},
            "if_revision": revision,
        }),
    );
    assert_eq!(titled["outcome"], "done", "{titled}");
    assert_eq!(titled["revision"], revision + 1, "{titled}");
    resumed.end();
}

/// A stepped shaft, a solid of revolution about Z, as an OBJ: Ø30 for 30
/// mm, then a Ø16 pin 20 mm long.
fn shaft_obj() -> String {
    let profile = [
        (0., 0.),
        (15., 0.),
        (15., 30.),
        (8., 30.),
        (8., 50.),
        (0., 50.),
    ];
    let n = 48;
    let mut index: BTreeMap<(i64, i64, usize), usize> = BTreeMap::new();
    let mut vertices: Vec<[f64; 3]> = vec![];
    let mut vertex = |r: f64, z: f64, k: usize| -> usize {
        let k = if r == 0. { 0 } else { k % n };
        let key = ((r * 1000.) as i64, (z * 1000.) as i64, k);
        *index.entry(key).or_insert_with(|| {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            vertices.push([r * a.cos(), r * a.sin(), z]);
            vertices.len()
        })
    };
    let mut faces = vec![];
    for w in profile.windows(2) {
        let ((r0, z0), (r1, z1)) = (w[0], w[1]);
        for k in 0..n {
            let (a, b) = (vertex(r0, z0, k), vertex(r0, z0, k + 1));
            let (c, d) = (vertex(r1, z1, k + 1), vertex(r1, z1, k));
            for t in [[a, b, c], [a, c, d]] {
                if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
                    faces.push(t);
                }
            }
        }
    }
    let mut obj = String::new();
    for v in &vertices {
        obj += &format!("v {:.6} {:.6} {:.6}\n", v[0], v[1], v[2]);
    }
    for t in &faces {
        obj += &format!("f {} {} {}\n", t[0], t[1], t[2]);
    }
    obj
}

/// Reads `steps` of the active hand setup, each with a cursor of its own:
/// the machinist's place never moves, nor the selection or the revision.
fn read_steps(s: &mut Server) -> Vec<Value> {
    let before = s.tool("inspect", json!({}));
    let first = s.tool("inspect", json!({"what": "steps"}));
    assert_eq!(first["outcome"], "done", "{first}");
    let place = first["machinist_place"].clone();
    let n = first["steps"].as_u64().unwrap();
    assert!(n > 2, "{first}");
    let mut steps = vec![first["step"].clone()];
    for k in 1..n {
        let read = s.tool("inspect", json!({"what": "steps", "step": k}));
        assert_eq!(read["outcome"], "done", "{read}");
        assert_eq!(read["step"]["index"], k, "{read}");
        assert_eq!(read["machinist_place"], place, "{read}");
        steps.push(read["step"].clone());
    }
    let beyond = s.tool("inspect", json!({"what": "steps", "step": n}));
    assert_eq!(beyond["outcome"], "refused", "{beyond}");
    let after = s.tool("inspect", json!({}));
    assert_eq!(after["selection"], before["selection"]);
    assert_eq!(after["revision"], before["revision"]);
    steps
}

#[test]
fn an_agent_builds_a_manual_mill_setup_with_commands_only() {
    let data = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/four-setup-block.3mf");
    let mut s = Server::start(data.path(), None);
    s.run("import_model", json!({"path": fixture}));

    // The manual mill, made in the wizard, and what works on it: its
    // cutters and the vise, each ticked for it.
    let mill = make_machine(
        &mut s,
        "Manual mill",
        &[
            ("name", "Manual Mill"),
            ("speeds", "90-1600"),
            ("edge_finder", "5"),
        ],
    );
    let aluminium = library_id(&mut s, "materials", "Aluminium 5052-H32");
    s.run("set_project_material", json!({"id": aluminium}));
    make_tool(
        &mut s,
        "Flat end mill",
        &[
            ("name", "Flat end mill 12mm"),
            ("diameter", "12"),
            ("flutes", "3"),
            ("length", "26"),
            ("stickout", "35"),
            ("center_cutting", "Yes"),
        ],
        Some(&mill),
    );
    make_tool(
        &mut s,
        "Drill",
        &[
            ("name", "Drill 6.6mm HSS"),
            ("diameter", "6.6"),
            ("flutes", "2"),
            ("length", "63"),
            ("stickout", "70"),
        ],
        Some(&mill),
    );
    make_tool(
        &mut s,
        "Centre drill",
        &[
            ("name", "Centre drill A2.5x6.3"),
            ("diameter", "6.3"),
            ("flutes", "2"),
            ("length", "7"),
            ("stickout", "30"),
            ("pilot", "2.5"),
            ("pilot_length", "3.1"),
        ],
        Some(&mill),
    );
    let vise = library_id(&mut s, "holders", "100 mm machine vise");
    s.batch(vec![
        step("edit_holder", json!({"id": vise})),
        step("holder_machine", json!({"id": mill, "checked": true})),
        step("save_holder", json!({})),
    ]);

    let added = s.run("add_manual_setup", json!({}));
    assert_eq!(added["outcome"], "done", "{added}");
    let state = s.tool("inspect", json!({}));
    assert_eq!(state["setups"][0]["machine"], "Manual Mill", "{state}");
    s.batch(
        [
            ("stock_top", "2"),
            ("stock_bottom", "2"),
            ("stock_left", "3"),
            ("stock_right", "3"),
            ("stock_front", "3"),
            ("stock_back", "3"),
            ("holding", "Vise"),
        ]
        .iter()
        .map(|(k, v)| field("setup", k, v))
        .collect(),
    );

    // A Box filled by a click on the top face: the 2 mm of stock above it.
    let report = s.tool("part_report", json!({}));
    let top = &report["report"]["orientations"][0];
    let planes = top["planes"].as_array().unwrap();
    let face_top = ids(plane(planes, "top", 0.))[0];
    let through: Vec<&Value> = (plane(planes, "top", 0.)["holes"].as_array().unwrap().iter())
        .filter(|h| h["through"] == true)
        .collect();
    let boxed = s.run("add_operation", json!({"strategy": "Box"}));
    let box_op = boxed["selection"]["operation"]
        .as_str()
        .unwrap()
        .to_string();
    let filled = s.run(
        "pick_surfaces",
        json!({"operation": box_op, "ids": [face_top], "mode": "replace"}),
    );
    assert_eq!(filled["clicks"][0]["did"], "filled", "{filled}");
    // The material's speed for the 12 mm cutter, as far as the mill's
    // 1600 rpm goes.
    s.batch(vec![
        step("recommend_speeds", json!({"operation": box_op})),
        op_field(&box_op, "stepdown", "1"),
    ]);
    let listed = s.tool(
        "list_fields",
        json!({"target": "operation", "operation": box_op}),
    );
    let value = |key: &str| {
        (listed["fields"].as_array().unwrap().iter())
            .find(|f| f["key"] == key)
            .unwrap()["value"]
            .clone()
    };
    assert_eq!(value("z_top"), "0");
    assert_eq!(value("z_bottom"), "-2");

    // The through holes drilled: the drill brings its centre drilling.
    let drilled = s.run("add_operation", json!({"strategy": "Drill"}));
    let drill_op = drilled["selection"]["operation"]
        .as_str()
        .unwrap()
        .to_string();
    let listed = s.tool("list_surfaces", json!({"limit": 5000}));
    let walls = hole_walls(listed["surfaces"].as_array().unwrap(), &through, 33.);
    s.run(
        "pick_surfaces",
        json!({"operation": drill_op, "ids": walls, "mode": "replace"}),
    );
    let state = s.tool("inspect", json!({}));
    let ops = state["setups"][0]["operations"].as_array().unwrap();
    let names: Vec<&str> = ops.iter().map(|o| o["name"].as_str().unwrap()).collect();
    assert_eq!(ops.len(), 3, "{names:?}");
    assert_eq!(ops[1]["tool"], "Centre drill A2.5x6.3", "{names:?}");
    assert_eq!(ops[2]["name"], "Drill Ø6.6 · 4 holes", "{names:?}");
    // add_centring is for a drill that has none: refused, nothing done.
    let again = s.tool(
        "execute",
        json!({"command": "add_centring", "args": {"operation": drill_op}}),
    );
    assert_eq!(again["outcome"], "refused", "{again}");

    s.run("suggest_layout", json!({}));
    let generated = s.run("generate_all", json!({}));
    assert!(
        generated["status"]
            .as_str()
            .unwrap()
            .contains("0 need attention"),
        "{generated}"
    );
    let check = s.tool("check_job", json!({"wait_s": 50}));
    assert_eq!(check["problems"], json!([]), "{check}");
    assert_eq!(check["passed"], true, "{check}");

    // Its steps, read without moving the machinist's place.
    let steps = read_steps(&mut s);
    let holes = (steps.iter())
        .find(|st| st["kind"] == "Holes" && st["operation"]["id"] == drill_op.as_str())
        .unwrap_or_else(|| panic!("no Holes step of the drill: {steps:?}"));
    assert_eq!(holes["holes"].as_array().unwrap().len(), 4, "{holes}");
    s.end();
}

#[test]
fn an_agent_builds_a_lathe_setup_with_commands_only() {
    let data = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let part = work.path().join("stepped-shaft.obj");
    std::fs::write(&part, shaft_obj()).unwrap();
    let mut s = Server::start(data.path(), None);
    s.run("import_model", json!({"path": part}));

    make_machine(&mut s, "Manual lathe", &[("name", "Lathe")]);
    make_tool(
        &mut s,
        "Turning tool",
        &[("name", "Turning tool R0.4"), ("nose_radius", "0.4")],
        None,
    );
    let aluminium = library_id(&mut s, "materials", "Aluminium 6061-T6");
    s.run("set_project_material", json!({"id": aluminium}));
    let added = s.run("add_lathe_setup", json!({}));
    assert_eq!(added["outcome"], "done", "{added}");
    let state = s.tool("inspect", json!({}));
    assert_eq!(state["setups"][0]["machine"], "Lathe", "{state}");

    // The pin's round face fills an Outside; the free end fills a Face.
    let listed = s.tool("list_surfaces", json!({"limit": 5000}));
    let surfaces = listed["surfaces"].as_array().unwrap();
    let pin = (surfaces.iter())
        .find(|s| {
            f(&s["normal"][2]).abs() < 0.1
                && (f(&s["centre"][0]).hypot(f(&s["centre"][1])) - 8.).abs() < 0.5
        })
        .unwrap()["id"]
        .clone();
    let end = (surfaces.iter())
        .find(|s| f(&s["normal"][2]) > 0.99 && (f(&s["centre"][2]) - 50.).abs() < 0.01)
        .unwrap()["id"]
        .clone();
    let outside = s.run("add_turn", json!({"kind": "Outside"}));
    let outside = outside["selection"]["operation"]
        .as_str()
        .unwrap()
        .to_string();
    let filled = s.run(
        "pick_surfaces",
        json!({"operation": outside, "ids": [pin], "mode": "replace"}),
    );
    assert_eq!(filled["clicks"][0]["did"], "filled", "{filled}");
    s.batch(vec![op_field(&outside, "depth", "1")]);
    let listed = s.tool(
        "list_fields",
        json!({"target": "operation", "operation": outside}),
    );
    let value = |key: &str| {
        (listed["fields"].as_array().unwrap().iter())
            .find(|f| f["key"] == key)
            .unwrap()["value"]
            .clone()
    };
    assert_eq!(value("diameter"), "16");
    assert_eq!(value("depth"), "1");
    let faced = s.run("add_turn", json!({"kind": "Face"}));
    let face = faced["selection"]["operation"]
        .as_str()
        .unwrap()
        .to_string();
    let filled = s.run(
        "pick_surfaces",
        json!({"operation": face, "ids": [end], "mode": "replace"}),
    );
    assert_eq!(filled["clicks"][0]["did"], "filled", "{filled}");
    // Faced first, then turned: one place up.
    s.run("move_operation", json!({"operation": face, "delta": -1}));
    let state = s.tool("inspect", json!({}));
    let order: Vec<&Value> = (state["setups"][0]["operations"].as_array().unwrap().iter())
        .map(|o| &o["id"])
        .collect();
    assert_eq!(order, [&json!(face), &json!(outside)]);
    // Two places up from the first would leave the list: refused.
    let refused = s.tool(
        "execute",
        json!({"command": "move_operation", "args": {"operation": face, "delta": -2}}),
    );
    assert_eq!(refused["outcome"], "refused", "{refused}");

    // The chuck as suggested for the bar: already the one chosen.
    let chuck = s.run("suggest_layout", json!({}));
    assert!(
        chuck["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("3-jaw chuck")
            || chuck["outcome"] == "done",
        "{chuck}"
    );
    let generated = s.run("generate_all", json!({}));
    assert!(
        generated["status"]
            .as_str()
            .unwrap()
            .contains("0 need attention"),
        "{generated}"
    );
    let check = s.tool("check_job", json!({"wait_s": 50}));
    assert_eq!(check["problems"], json!([]), "{check}");
    assert_eq!(check["passed"], true, "{check}");

    let steps = read_steps(&mut s);
    assert!(
        (steps.iter()).any(|st| st["operation"]["id"] == outside.as_str()),
        "{steps:?}"
    );
    s.end();
}

/// A session the agent only looked at is resumed as it was: its workspace
/// is on disk from the start, so the starter library keeps its ids and
/// nothing reads as the user's change.
#[test]
fn a_session_resumed_untouched_reports_no_change() {
    let data = tempfile::tempdir().unwrap();
    let mut s = Server::start(data.path(), None);
    let first = s.tool("inspect", json!({}));
    let session = first["session"]["id"].as_str().unwrap().to_string();
    let libraries = s.tool("inspect", json!({"what": "libraries"}));
    assert!(
        !libraries["materials"].as_array().unwrap().is_empty(),
        "{libraries}"
    );
    let unknown = s.request(
        "tools/call",
        json!({"name": "inspect", "arguments": {"what": "everything"}}),
    );
    assert_eq!(unknown["isError"], true, "{unknown}");
    s.end();
    for _ in 0..2 {
        let mut s = Server::start(data.path(), Some(&session));
        let state = s.tool("inspect", json!({}));
        assert_eq!(state["user_changed"], json!([]), "{state}");
        assert_eq!(state["revision"], first["revision"], "{state}");
        let again = s.tool("inspect", json!({"what": "libraries"}));
        assert_eq!(again["materials"], libraries["materials"]);
        assert_eq!(again["holders"], libraries["holders"]);
        s.end();
    }
}
