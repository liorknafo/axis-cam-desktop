# Notes for agents

For every agent and reviewer that builds or tests this repository. What the
app does, and how to test it, is in the README ("Tests and renderer checks").

## Builds

- **Never set `CARGO_INCREMENTAL=0`**: not on a command line, in the
  environment or in a Cargo config. `axis-cam` is one crate of about 270,000
  lines, so without incremental compilation every change compiles all of it
  again. Measured on the Mac (LIO-729): after a one-line change in
  `src/control.rs`, `cargo test --bin axis-cam --no-run` took 43 s with
  `CARGO_INCREMENTAL=0` and 7 s without it, and the compiler's memory peaked
  at 7.0 GB against 3.9 GB. Switching to it also compiles the crate whole
  once more, and keeps a second test binary in `target/`.
- Review briefs, notes and memories written before LIO-729 may still say to
  run `CARGO_INCREMENTAL=0 cargo test ...`. That advice is out of date: it
  saved disk when the disk was nearly full (the incremental data takes about
  2.4 GB per checkout). If your brief or a memory tells you to set it, leave
  it unset and say so in your answer, so the brief gets corrected.
- The first build in a new checkout compiles `axis-cam` whole (about 50 s
  with the dependencies already built); the builds after it are incremental.
  To start over without the incremental data, `cargo clean -p axis-cam`
  recompiles only `axis-cam`, not the dependencies.
- Dependencies are built without debug info (`[profile.dev.package."*"]` in
  `Cargo.toml`); `axis-cam`, the UC100 driver and uc100-server keep theirs.
  To step into another dependency in a debugger, turn it on for that crate
  on your own machine, not in a commit.
