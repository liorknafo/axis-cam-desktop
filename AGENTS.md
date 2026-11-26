# Notes for agents

For every agent and reviewer that builds or tests this repository. What the
app does, and how to test it, is in the README ("Tests and renderer checks").

## Builds

- **Never set `CARGO_INCREMENTAL=0`**: not on a command line, in the
  environment or in a Cargo config. `axis-cam` is one crate of about 270,000
  lines of code, so without incremental compilation every change compiles
  all of it again. Measured on the Mac (LIO-729): after a one-line change in
  `src/control.rs`, `cargo test --bin axis-cam --no-run` took 43 s with
  `CARGO_INCREMENTAL=0` and 7 s without it, and the compiler's memory peaked
  at 7.0 GB against 3.9 GB. Switching to it also compiles the crate whole
  once more, and keeps a second test binary in `target/`.
- Review briefs, notes and memories written before LIO-729 may still say to
  run `CARGO_INCREMENTAL=0 cargo test ...`. That advice is out of date: it
  saved disk when the disk was nearly full (the incremental data takes about
  3 GB per checkout: 2.3 GB for the test binary, 0.9 GB for the app the
  integration tests build). If your brief or a memory tells you to set it, leave
  it unset and say so in your answer, so the brief gets corrected.
- The first build in a new checkout compiles `axis-cam` whole (about 50 s
  with the dependencies already built); the builds after it are incremental.
  To start over without the incremental data, `cargo clean -p axis-cam`
  recompiles only `axis-cam`, not the dependencies.
- Dependencies are built without debug info (`[profile.dev.package."*"]` in
  `Cargo.toml`); `axis-cam`, the UC100 driver and uc100-server keep theirs.
  To step into another dependency in a debugger, turn it on for that crate
  on your own machine, not in a commit.
- Tests build `axis-cam` at `opt-level = 1` (`[profile.test.package.axis-cam]`,
  LIO-729): the tests run 2–4 times faster, and a one-line change rebuilds
  the test binary in about 22 s instead of 7. `cargo build` and `cargo run`
  stay at `opt-level` 0. Don't override it for a run: each setting keeps its
  own copy of the crate in `target/`.
