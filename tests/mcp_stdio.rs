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
    assert!(!folder.join("lock").exists());
}
