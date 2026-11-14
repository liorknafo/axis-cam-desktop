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
//! - `fields`: the editors' fields, which `list_fields` lists and
//!   `edit_field` checks its key against.
//! - `pick`: the picks made by clicks, each checked, and a click undone in
//!   full.
//! - `state`: the work state's snapshots, the revisions and their change
//!   log, the external events, and the machinist's records.
//! - `reads`: the reads that work something out (the checks of the job as
//!   it is, the part, pictures, a hand setup's steps), on a snapshot of the
//!   job off the `App`'s thread, heavy ones kept from a running machine.
//! - `rules`: the machining rules by topic, for `guide`.
//! - `checkpoint`: the job kept by name, and put back.
pub mod checkpoint;
pub mod class;
pub mod commands;
pub mod engine;
pub mod fields;
pub mod pick;
pub mod protocol;
pub mod reads;
pub mod rules;
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
