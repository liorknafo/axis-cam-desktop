//! MCP over stdio (LIO-470): JSON-RPC 2.0, one message a line, protocol
//! version `2025-06-18`. Written by hand on `serde_json`, since only
//! `initialize`, `notifications/initialized`, `ping`, `tools/list` and
//! `tools/call` are needed.
//!
//! Stdin is read on a thread of its own, so a call is read while another is
//! still working. Every answer is written by one writer thread, and standard
//! output carries nothing else: on macOS the process's own standard output
//! is pointed at standard error before anything runs, so nothing the app
//! prints can land among the protocol's lines.
use super::{
    engine::{self, Answer},
    reads,
    session::Sessions,
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, Write},
    sync::mpsc,
};

/// The MCP version this server speaks.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// What `initialize` tells the agent, in the SolidCraft style.
pub const INSTRUCTIONS: &str = "\
AXIS CAM Studio plans machining jobs for a CNC mill, a manual mill and a lathe. These tools drive it with the user's own commands. Units are millimetres and degrees.

Start with `inspect`: the project, its setups and their operations (each with a stable `id`), the selection with its page and tab, the status line, the work still pending, and the session.

Workflow: inspect, import the part, add setups, add operations, pick their surfaces, generate_all, check_job, review the NC, save, export. `list_commands {filter?, group?}` lists the commands this server runs now; `describe_command {name}` gives one's help, its `args` schema, what it needs first and the effect it is checked for. `list_fields {target, setup?, operation?}` lists the fields `edit_field` takes, with their choices and units. Run one with `execute {command, args, wait_s?}`. `guide {topic}` gives the machining rules the checks hold a plan to (workflow, stepdowns, cutters, holding, chip_load, minimum_rpm): read `guide {topic: \"workflow\"}` first.

Reading the part and the job: `part_report` (the part, the machine's limits, the material's cutting data, the tools by number), `list_surfaces {setup?}` (the surface ids `pick_surfaces` takes, with each orientation's planes; on the built-in sample, its features), `surface_info {id}`, `cutting_data {tool, op?}`, and `view_model {view, …}`, a picture of the part, its stock and holding, or an operation's toolpaths and the stock it leaves. A manual mill or lathe setup's steps: `inspect {what: \"steps\", setup, step?}`, which never moves the machinist's place.

Checking: `check_job` runs every check on the job exactly as the user sees it; nothing is filled in. A setup with clamps but no positions is a problem: place them with `suggest_layout` or `set_clamps`. `check_holding {setup}` checks one setup's holder and placement as saved, `stock_after {setup}` gives the stock it starts from. These read a snapshot of the job; when the job changed while they worked the result says `stale` (check_job is then refused: run it again). They simulate toolpaths, so while the machine runs they are refused, and Cycle Start waits for them. A read that takes long answers still_running with a `command_id`, as a command does.

Ids: setups and operations are addressed by `id`. An index from 0, in inspect's order, is accepted too and turned into the id when the call arrives.

Outcomes: every `execute` result has `outcome`: `done` (its effect was checked and holds), `no_change` (it was already so), `refused` (nothing was done; `reason` says what is missing), `busy` (the previous command's work is still settling; nothing was done: call `wait`, then retry), `failed` (it was sent and did not take; `reason` gives the app's status line or says what did not happen), `still_running` (it was sent and its work, listed in `pending`, had not settled within `wait_s`: 25 s by default, at most 50), or `cancelled`. A command that was sent has a `command_id`: `wait {command_id}` gives its final result, and `cancel {command_id}` stops a generation. `batch {steps}` runs commands in order and stops at the first that does not go; it is not all-or-nothing. Reads, `wait`, `cancel` and the stops answer while a command's work settles. The status line alone does not tell whether a command worked; the outcome does.

Session: with no AXIS CAM window to work in, this server runs a headless session of its own, which starts from a copy of the user's workspace (their libraries and last job). It never writes the user's workspace. Everything a result reports is on disk in the session's folder before the result is sent, and it stays there when the client ends: `resume_session {id}` (or `axis-cam --mcp --session <id>`) continues it, `list_sessions` lists them, and `delete_session` deletes one; nothing deletes them otherwise. Every result names its backend and its session, whether the job is saved to a project file, and the last saves and exports.

Checkpoints: `checkpoint {name}` keeps the job as it is; `restore_checkpoint {name}` puts it back as one new revision, refused when the user changed the job since (unless `discard_user_changes: true`), for another project, or when the part file changed. Take one before a risky batch.

The machine: moving it, turning the spindle or coolant on, connecting, and what its controller holds stay with the user in the app; no command here does them.";

/// Serves MCP on stdin and stdout until stdin ends, the tools carried out
/// in the headless session of `sessions`; the exit code. The session is
/// left on disk.
pub fn serve(mut sessions: Sessions) -> i32 {
    let out = protocol_output();
    let (lines, written) = mpsc::channel::<String>();
    let writer = std::thread::Builder::new()
        .name("mcp-stdout".into())
        .spawn(move || write_lines(out, written))
        .expect("the MCP writer starts");
    let reader = std::thread::Builder::new()
        .name("mcp-stdin".into())
        .spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                handle_line(&line, &mut sessions, &lines);
            }
            // The calls already read are answered before the session ends.
            sessions.stop();
        })
        .expect("the MCP reader starts");
    let _ = reader.join();
    // Every sender has gone with the reader and the session: the writer
    // ends once it has written what they sent.
    let _ = writer.join();
    0
}

/// The protocol's standard output. On macOS a copy of it, with the
/// process's own standard output pointed at standard error.
fn protocol_output() -> Box<dyn Write + Send> {
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::FromRawFd;
        let _ = std::io::stdout().flush();
        // SAFETY: plain descriptor calls on 1 and 2, before any other
        // thread starts; the copy is owned by the File from here on.
        unsafe {
            let copy = libc::dup(1);
            if copy >= 0 {
                if libc::dup2(2, 1) >= 0 {
                    return Box::new(std::fs::File::from_raw_fd(copy));
                }
                libc::close(copy);
            }
        }
    }
    Box::new(std::io::stdout())
}

/// The writer: each line as it comes, flushed, until every sender has gone.
fn write_lines(mut out: Box<dyn Write + Send>, lines: mpsc::Receiver<String>) {
    for line in lines {
        if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
            break;
        }
    }
}

/// JSON-RPC's error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

fn result(id: &Value, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}
fn error(id: &Value, code: i64, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}

/// A tool's answer as MCP's `CallToolResult`: the result object as text and
/// as structured content, or the error as text with `isError`. A picture in
/// the result (`reads::IMAGE`) goes as an image content item, before the
/// text, and is left out of the rest.
pub fn tool_result(answer: Answer) -> Value {
    match answer {
        Ok(mut v) => {
            let image = (v.as_object_mut())
                .and_then(|o| o.remove(reads::IMAGE))
                .and_then(|png| png.as_str().map(String::from));
            let text = json!({"type": "text", "text": v.to_string()});
            let content = match image {
                Some(data) => {
                    json!([{"type": "image", "data": data, "mimeType": "image/png"}, text])
                }
                None => json!([text]),
            };
            json!({
                "content": content,
                "structuredContent": v,
                "isError": false,
            })
        }
        Err(e) => json!({
            "content": [{"type": "text", "text": e}],
            "isError": true,
        }),
    }
}

/// Answers one line from the client. A notification, or a response, gets
/// no answer; a `tools/call` is answered by the session when it is done.
fn handle_line(line: &str, sessions: &mut Sessions, out: &mpsc::Sender<String>) {
    if line.trim().is_empty() {
        return;
    }
    let send = |line: String| {
        let _ = out.send(line);
    };
    let message: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return send(error(
                &Value::Null,
                PARSE_ERROR,
                &format!("Parse error: {e}"),
            ));
        }
    };
    let Some(object) = message.as_object() else {
        return send(error(
            &Value::Null,
            INVALID_REQUEST,
            "Invalid request: one JSON-RPC object a line.",
        ));
    };
    let id = object.get("id").cloned();
    let method = object.get("method").and_then(Value::as_str);
    // A notification, and the client's answer to a request of ours, need
    // no answer.
    let answer = object.contains_key("result") || object.contains_key("error");
    if (method.is_some() && id.is_none()) || (method.is_none() && answer) {
        return;
    }
    let version = object.get("jsonrpc").and_then(Value::as_str) == Some("2.0");
    let (Some(id), Some(method), true) = (id.clone(), method, version) else {
        return send(error(
            &id.unwrap_or(Value::Null),
            INVALID_REQUEST,
            "Invalid request: it needs \"jsonrpc\": \"2.0\", a method and an id.",
        ));
    };
    let params = object.get("params").cloned().unwrap_or(json!({}));
    match method {
        "initialize" => send(result(
            &id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {
                    "name": "axis-cam",
                    "title": "AXIS CAM Studio",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "instructions": INSTRUCTIONS,
            }),
        )),
        "ping" => send(result(&id, json!({}))),
        "tools/list" => send(result(&id, json!({"tools": engine::tools()}))),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return send(error(
                    &id,
                    INVALID_PARAMS,
                    "tools/call needs a tool's name.",
                ));
            };
            if !engine::is_tool(name) {
                return send(error(&id, INVALID_PARAMS, &format!("Unknown tool: {name}")));
            }
            let args = match params.get("arguments") {
                None | Some(Value::Null) => json!({}),
                Some(a @ Value::Object(_)) => a.clone(),
                Some(_) => {
                    return send(error(
                        &id,
                        INVALID_PARAMS,
                        "A tool's arguments are an object.",
                    ));
                }
            };
            let out = out.clone();
            sessions.call(
                name,
                args,
                Box::new(move |answer| {
                    let _ = out.send(result(&id, tool_result(answer)));
                }),
            );
        }
        _ => send(error(
            &id,
            METHOD_NOT_FOUND,
            &format!("Method not found: {method}"),
        )),
    }
}
