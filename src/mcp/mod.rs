//! The MCP server (LIO-470): `axis-cam --mcp` lets an AI agent (Claude
//! Code, Codex or any MCP client) plan a job with the user's own commands.
//! The design is `docs/superpowers/specs/2026-11-13-mcp-server-design.md`.
//!
//! - `protocol`: JSON-RPC 2.0 over stdin and stdout, one message a line.
//! - `runner`: the headless session, an `App` with no window on a thread
//!   of its own.
//! - `engine`: the tools, carried out against that `App`.
//! - `class`: what each `Message` is to an agent, with no wildcard.
//! - `commands`: the commands `execute` runs.
//! - `state`: the work state's snapshots, the revisions and their change
//!   log, the external events, and the machinist's records.
pub mod class;
pub mod commands;
pub mod engine;
pub mod protocol;
pub mod runner;
pub mod state;

#[cfg_attr(not(test), allow(unused_imports))]
pub use state::{NOT_WORK_STATE, WORK_STATE};

/// `axis-cam --mcp [--session <id>]`, given the arguments after `--mcp`:
/// serves MCP on stdin and stdout until stdin ends, in a new headless
/// session or in session `id`. The process's exit code.
pub fn main(args: &[String]) -> i32 {
    let id = match args {
        [] => None,
        [flag, id] if flag == "--session" => Some(id.as_str()),
        _ => {
            eprintln!("Usage: axis-cam --mcp [--session <id>]");
            return 2;
        }
    };
    crate::fault::install();
    let workspace = crate::storage::state_path();
    match runner::Session::open(&runner::Session::root(), id, &workspace) {
        Ok(session) => protocol::serve(session),
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}
