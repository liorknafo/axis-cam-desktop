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
  dev-dependency turns on. `cargo fmt --all` and `cargo clippy --workspace`
  cover both. The opt-level note below (`[profile.test.package.*]`) covers
  both crates.

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
  4.4 GB per checkout after `cargo test`: 3.1 GB for the test binary, 1.3 GB
  for the app the integration tests build; `cargo build` adds 0.9 GB). If your
  brief or a memory tells you to set it, leave it unset and say so in your
  answer, so the brief gets corrected.
- The first build in a new checkout compiles `axis-cam` whole, with the
  dependencies already built: about 95 s for `cargo test --no-run`, which
  builds it at `opt-level` 1 (below), and 28 s for `cargo build`. The builds
  after it are incremental. To start over without the incremental data,
  `cargo clean -p axis-cam` recompiles only `axis-cam`, not the dependencies.
- Dependencies are built without debug info (`[profile.dev.package."*"]` in
  `Cargo.toml`); `axis-cam`, `axis-core`, the UC100 driver and uc100-server
  keep theirs.
  To step into another dependency in a debugger, turn it on for that crate
  on your own machine, not in a commit.
- Tests build `axis-cam` at `opt-level = 1` (`[profile.test.package.axis-cam]`,
  LIO-729): the test groups run 1.8–2.6 times faster (the full suite about
  17 min instead of 40). The cost: a one-line change rebuilds the test binary
  in about 22 s instead of 7, peaking at 6.3 GB of memory instead of 3.9, and
  in a cold `cargo test --no-run` the compiler of the test binary alone
  reaches about 10 GB (7.3 before). `cargo build` and `cargo run` stay
  at `opt-level` 0 (a one-line rebuild about 4 s). Don't override it for a
  run: each setting keeps its own copy of the crate in `target/`.
