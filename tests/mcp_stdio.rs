//! The MCP server over a pipe, against the real binary (LIO-470): what an
//! MCP client sees of `axis-cam --mcp`. Its data folder is a temporary one
//! (`AXIS_CAM_DATA_DIR`), so the user's workspace is never touched.
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

#[test]
fn the_server_answers_over_stdio() {
    let data = tempfile::tempdir().unwrap();
    let stderr_file = data.path().join("server-stderr.log");
    let stderr = || std::fs::read_to_string(&stderr_file).unwrap_or_default();
    let mut server = Command::new(env!("CARGO_BIN_EXE_axis-cam"))
        .arg("--mcp")
        .env("AXIS_CAM_DATA_DIR", data.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(&stderr_file).unwrap())
        .spawn()
        .unwrap();
    let out = server.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines() {
            let _ = tx.send(line.unwrap());
        }
    });
    let mut stdin = server.stdin.take().unwrap();
    let mut send = |line: &str| {
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
    };
    let initialize = json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1"},
        },
    });
    send(&initialize.to_string());
    send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    send(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"inspect","arguments":{}}}"#,
    );
    send("{this is not json");
    send(r#"{"jsonrpc":"2.0","id":"four","method":"resources/list"}"#);
    send(r#"{"jsonrpc":"2.0","id":5,"method":"ping"}"#);
    send(
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"execute","arguments":{"command":"fly"}}}"#,
    );

    // Seven answers, in any order: the notification gets none.
    let mut answers: HashMap<String, Value> = HashMap::new();
    while answers.len() < 7 {
        let line = rx.recv_timeout(Duration::from_secs(60)).unwrap_or_else(|e| {
            let status = server.try_wait();
            panic!("no answer: {e} (timeout: still running; disconnected: exited); status {status:?}; server stderr:\n{}", stderr())
        });
        // Standard output carries the protocol's lines and nothing else.
        let v: Value = serde_json::from_str(&line).expect("a JSON line");
        assert_eq!(v["jsonrpc"], "2.0", "{line}");
        answers.insert(v["id"].to_string(), v);
    }

    let init = &answers["1"]["result"];
    assert_eq!(init["protocolVersion"], "2025-06-18");
    assert_eq!(init["serverInfo"]["name"], "axis-cam");
    assert!(init["capabilities"]["tools"].is_object());
    assert!(init["instructions"].as_str().unwrap().contains("inspect"));

    let tools: Vec<&str> = (answers["2"]["result"]["tools"].as_array().unwrap().iter())
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for name in [
        "inspect",
        "list_fields",
        "list_commands",
        "describe_command",
        "execute",
        "check_job",
        "check_holding",
        "stock_after",
        "cutting_data",
        "surface_info",
        "part_report",
        "list_surfaces",
        "view_model",
        "guide",
        "checkpoint",
        "restore_checkpoint",
        "list_checkpoints",
        "list_sessions",
        "resume_session",
        "delete_session",
    ] {
        assert!(tools.contains(&name), "{tools:?}");
    }

    let inspect = &answers["3"]["result"];
    assert_eq!(inspect["isError"], false);
    let state = &inspect["structuredContent"];
    let text: Value =
        serde_json::from_str(inspect["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(&text, state);
    assert_eq!(state["backend"], "headless");
    assert!(state["setups"].is_array());
    let folder = std::path::PathBuf::from(state["session"]["folder"].as_str().unwrap());
    let sessions = data.path().join("AXIS CAM Studio").join("mcp-sessions");
    assert_eq!(
        folder,
        sessions.join(state["session"]["id"].as_str().unwrap())
    );
    assert!(folder.is_dir());

    let malformed = &answers["null"];
    assert_eq!(malformed["error"]["code"], -32700);
    assert_eq!(answers["\"four\""]["error"]["code"], -32601);
    assert_eq!(answers["5"]["result"], json!({}));
    let unknown = &answers["6"]["result"];
    assert_eq!(unknown["isError"], true);
    assert!(
        unknown["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("fly")
    );

    // A command whose work takes a while (an edit settles after a pause)
    // does not hold up a read sent after it.
    send(
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"execute","arguments":{"command":"set_project_title","args":{"title":"Slow"},"wait_s":20}}}"#,
    );
    send(
        r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"inspect","arguments":{}}}"#,
    );
    let mut order = vec![];
    while order.len() < 2 {
        let line = rx.recv_timeout(Duration::from_secs(60)).unwrap_or_else(|e| {
            let status = server.try_wait();
            panic!("no answer: {e} (timeout: still running; disconnected: exited); status {status:?}; server stderr:\n{}", stderr())
        });
        let v: Value = serde_json::from_str(&line).unwrap();
        order.push(v);
    }
    assert_eq!(order[0]["id"], 8, "the read answers first: {order:?}");
    let executed = &order[1]["result"]["structuredContent"];
    assert_eq!(executed["outcome"], "done", "{executed}");
    assert!(executed["command_id"].is_string());

    // Stdin's end ends the server, and the session stays on disk.
    drop(stdin);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = server.try_wait().unwrap() {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the server did not end"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success());
    assert!(folder.is_dir());
    // With what a resume needs beside its workspace, and let go of.
    let kept: Value =
        serde_json::from_slice(&std::fs::read(folder.join("session.json")).unwrap()).unwrap();
    assert_eq!(kept["title"], "Slow", "{kept}");
    assert!(!held(&folder.join("lock")));
}

/// Whether some process holds the lock file `lock`: it cannot be held
/// shared now.
fn held(lock: &std::path::Path) -> bool {
    let file = std::fs::File::open(lock).unwrap();
    matches!(
        file.try_lock_shared(),
        Err(std::fs::TryLockError::WouldBlock)
    )
}

/// `axis-cam --mcp` with `args`, its data folder `data`.
fn server(data: &std::path::Path, args: &[&str]) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_axis-cam"))
        .arg("--mcp")
        .args(args)
        .env("AXIS_CAM_DATA_DIR", data)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

/// Sends `initialize` to `server` and waits up to `within` for its answer:
/// whether it answered.
fn answers(server: &mut std::process::Child, within: Duration) -> bool {
    let mut stdin = server.stdin.take().unwrap();
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    // A server that refused the session has gone: its pipe may be closed.
    let _ = writeln!(stdin, "{init}").and_then(|()| stdin.flush());
    server.stdin = Some(stdin);
    let out = server.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let mut out = BufReader::new(out);
        let read = out.read_line(&mut line).map(|n| n > 0).unwrap_or(false);
        let _ = tx.send((read, line));
    });
    match rx.recv_timeout(within) {
        Ok((true, line)) => {
            let v: Value = serde_json::from_str(&line).unwrap();
            v["result"]["serverInfo"]["name"] == "axis-cam"
        }
        _ => false,
    }
}

/// The reviewer's race (LIO-470), with real processes: eight servers
/// started at once on one session, its lock file left by a dead process.
/// Exactly one opens it and serves; the others are refused at once,
/// naming the session, and write nothing (the lock file holds the
/// winner's pid). Killed, the winner's lock goes with its process, and the
/// next server opens the session.
#[test]
fn of_servers_racing_for_a_session_one_opens_it() {
    let data = tempfile::tempdir().unwrap();
    // A session to race for: made, worked in, and ended.
    let mut maker = server(data.path(), &[]);
    let mut stdin = maker.stdin.take().unwrap();
    let call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"execute","arguments":{"command":"set_project_title","args":{"title":"Raced"}}}}"#;
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    writeln!(stdin, "{init}\n{call}").unwrap();
    stdin.flush().unwrap();
    let mut out = BufReader::new(maker.stdout.take().unwrap());
    let made = loop {
        let mut line = String::new();
        assert!(out.read_line(&mut line).unwrap() > 0, "the server ended");
        let v: Value = serde_json::from_str(&line).unwrap();
        if v["id"] == 2 {
            break v;
        }
    };
    let state = &made["result"]["structuredContent"];
    assert_eq!(state["outcome"], "done", "{made}");
    let id = state["session"]["id"].as_str().unwrap().to_string();
    let folder = std::path::PathBuf::from(state["session"]["folder"].as_str().unwrap());
    drop(stdin);
    assert!(maker.wait().unwrap().success());
    let lock = folder.join("lock");
    let dead = json!({"pid": 2_147_483_000u32, "started": "dead", "at": 0});
    for round in 0..3 {
        std::fs::write(&lock, dead.to_string()).unwrap();
        let record = std::fs::read(folder.join("session.json")).unwrap();
        let mut racers: Vec<_> = (0..8)
            .map(|_| server(data.path(), &["--session", &id]))
            .collect();
        let served: Vec<bool> = (racers.iter_mut())
            .map(|r| answers(r, Duration::from_secs(60)))
            .collect();
        assert_eq!(
            served.iter().filter(|s| **s).count(),
            1,
            "round {round}: {served:?}"
        );
        let winner = served.iter().position(|s| *s).unwrap();
        for (i, racer) in racers.iter_mut().enumerate() {
            if i == winner {
                continue;
            }
            let status = racer.wait().unwrap();
            assert_eq!(status.code(), Some(1), "round {round}");
            let mut why = String::new();
            std::io::Read::read_to_string(racer.stderr.as_mut().unwrap(), &mut why).unwrap();
            assert!(
                why.contains(&format!("Session {id} is open in another MCP server")),
                "{why}"
            );
        }
        // The losers wrote nothing: the lock file is the winner's, the
        // record as it was.
        let owner: Value = serde_json::from_slice(&std::fs::read(&lock).unwrap()).unwrap();
        assert_eq!(owner["pid"], racers[winner].id(), "round {round}");
        assert_eq!(std::fs::read(folder.join("session.json")).unwrap(), record);
        assert!(held(&lock));
        // Killed: no stale lock is left to tell.
        racers[winner].kill().unwrap();
        racers[winner].wait().unwrap();
        assert!(!held(&lock), "round {round}");
    }
    let mut next = server(data.path(), &["--session", &id]);
    assert!(answers(&mut next, Duration::from_secs(60)));
    next.kill().unwrap();
    next.wait().unwrap();
}
