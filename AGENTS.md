# Notes for agents

For every agent and reviewer that builds or tests this repository. What the
app does, and how to test it, is in the README ("Tests and renderer checks").

## Builds

- **Two crates, one workspace** (LIO-729): the app `axis-cam` (`src/`: UI,
  machine control, runs, MCP, planner, tablet, smoke tests) and the core
  `axis-core` (`crates/axis-core/`: model, geometry, toolpath strategies,
  simulation, posting, analysis, proposal, plan check, storage, importers, and
  the STEP helper's `build.rs` and `native/`). The app reaches a core module as
  `crate::<module>` (`pub(crate) use axis_core::<module>;` in `src/main.rs`).
  `cargo test -p axis-core` builds and runs the core's tests without the app;
  `cargo test -p axis-cam` tests the app; plain `cargo test` at the root, or
  `--workspace`, both (`default-members`). A filter such as `cargo test --bin
  axis-cam -- post::` runs only the app's tests: the core's are in
  `cargo test -p axis-core -- post::`. The core's test-only helpers the
  app's tests use sit behind its `test-support` feature, which the app's
  dev-dependency turns on: `cargo test -p axis-core` builds the core without
  it and `cargo test` (or `-p axis-cam`) with it, so switching between the two
  recompiles the core once (about 35 s). `cargo fmt --all` and `cargo clippy --workspace`
  cover both. Tests build the core at `opt-level` 1 and the app at 0
  (below).

- **Never set `CARGO_INCREMENTAL=0`**: not on a command line, in the
  environment or in a Cargo config. Each crate is large (the app about
  180,000 lines of code, the core about 127,000), so without incremental
  compilation every change compiles a whole crate again. Measured on the Mac
  (LIO-729, while tests still built at `opt-level` 0): after a one-line
  change in `src/control.rs`, `cargo test --bin axis-cam --no-run` took 43 s
  with `CARGO_INCREMENTAL=0` and 7 s without it, and the compiler's memory
  peaked at 7.0 GB against 3.9 GB.
  Switching to it also compiles the crate whole once more, and keeps a
  second test binary in `target/`.
- Review briefs, notes and memories written before LIO-729 may still say to
  run `CARGO_INCREMENTAL=0 cargo test ...`. That advice is out of date: it
  saved disk when the disk was nearly full (the incremental data takes about
  4 GB per checkout after `cargo test`, and about 5 GB once `cargo build` has
  run too). If your brief or a memory tells you to set it, leave it unset and
  say so in your answer, so the brief gets corrected.
- The first build in a new checkout compiles both crates whole, with the
  dependencies already built: about a minute for `cargo test --no-run`
  (53–69 s measured, peaking at about 5 GB) and 24 s for `cargo build`. The
  builds after it are incremental. To start over
  without the incremental data, `cargo clean -p axis-cam -p axis-core`
  recompiles only the two crates, not the dependencies.
- Dependencies are built without debug info (`[profile.dev.package."*"]` in
  `Cargo.toml`); `axis-cam`, `axis-core`, the UC100 driver and uc100-server
  keep theirs.
  To step into another dependency in a debugger, turn it on for that crate
  on your own machine, not in a commit.
- Tests build the core at `opt-level = 1` (`[profile.test.package.axis-core]`,
  LIO-729) and the app at 0: most of the tests' time is the core's
  toolpath, simulation and posting code, so the test groups run about twice
  as fast as unoptimised, and a one-line change in the app still rebuilds
  the test binary in about 5 s (11 s in the core, which recompiles it at
  `opt-level` 1). `cargo build` and `cargo run` build both crates at
  `opt-level` 0 (a one-line rebuild 2–4 s). Don't override it for a run:
  each setting keeps its own copy of a crate in `target/`.
