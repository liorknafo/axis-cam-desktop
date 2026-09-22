# Validated LinuxCNC Postprocessor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the comments-only `.nc` export with executable, checked and round-trip-verified 3-axis LinuxCNC G-code, one `.ngc` program per setup.

**Architecture:** `post.rs` turns the exact `Plan.moves` that `simulation::sequence` plays into a `Block` list (plan coordinates), then emits text. `post_check.rs` validates the blocks: limits, travel, rapids into material, holder clearance. `ngc.rs` strictly re-reads the text and `ngc::verify` compares it one-to-one with the blocks. Files are written only when every setup passes. The UI runs this in the background on the NC program page. A headless `--export-gcode` flag runs the same path.

**Tech Stack:** Rust edition 2024, Iced 0.14, existing crates only (no new dependencies).

**Spec:** `docs/superpowers/specs/2026-09-22-linuxcnc-post-design.md`

## Global Constraints

- Source folder: `/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/outputs/axis-cam-desktop` (all paths below are relative to it). Every `cargo` command runs from there with `export CARGO_TARGET_DIR='/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/work/native-target'`.
- Never write to `~/Library/Application Support/AXIS CAM Studio/workspace.json` (the user's live job). Read-only use is allowed in Task 12.
- No new crates. `Cargo.lock` must not change (`--locked`).
- Output: mm only (`G21`), `G0`/`G1`/`G4` only, coordinates `{:.3}`, feeds `{:.1}`, dwell `P{:.3}`, blend `G64 P{:.3}` or `G61`.
- Offsets accepted: `G54` `G55` `G56` `G57` `G58` `G59` `G59.1` `G59.2` `G59.3`.
- File names: `<job> - <n> <setup name> (<offset>).ngc`, ASCII hyphen.
- Post settings keys and defaults: `post_tool_length` = `Tool table (G43 H#)` | `Touch off each tool (one program per tool)`; `post_retract` = `Machine Z home (G53 G0 Z0)` | `Setup clearance plane`; `post_blend` = `0.01` (0–1 mm); `post_dwell` = `2` (0–60 s); `post_air` = `M7` | `M8` | `Not available`.
- Tool numbers: whole numbers 1–9999, key `number`.
- Style: match the surrounding code, including compact functions, the `get`/`num` helpers, few comments and user-facing sentences in plain language. Never claim fixture, clamp or machine-body collision checking.
- Every task ends with `cargo fmt` and `cargo test --locked --quiet` passing (dead-code warnings are allowed until Task 8 wires everything up).

---

### Task 0: Version control and green baseline

**Files:**
- Create: `.git/` (via `git init`)
- Modify: `src/choice_help.rs:551-553`

- [ ] **Step 1: Initialise git and commit the current state**

```bash
cd '/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/outputs/axis-cam-desktop'
git init -q && git add -A && git commit -q -m "Baseline: AXIS CAM Studio before LinuxCNC post" && git log --oneline | head -1
```
Expected: one commit hash line. `.gitignore` already excludes `/target/`, `/dist/`, `/smoke-output/` and `.DS_Store`.

- [ ] **Step 2: Confirm the known stale failure**

Run: `cargo test --locked --quiet choice_help_keeps 2>&1 | tail -5`
Expected: FAIL at `src/choice_help.rs:552` (`rest.body.contains("inherits earlier cuts")`).

- [ ] **Step 3: Update the two stale assertions to the current, accurate wording**

In `src/choice_help.rs`, replace:
```rust
        assert!(rest.body.contains("inherits earlier cuts"));
        assert!(rest.body.contains("path generation is not implemented"));
```
with:
```rust
        assert!(rest.body.contains("Fully cleared top layers are skipped automatically"));
        assert!(rest.body.contains("does not yet isolate leftover islands or corners"));
```

- [ ] **Step 4: Run the full suite**

Run: `cargo test --locked --quiet 2>&1 | tail -3`
Expected: `test result: ok. 119 passed; 0 failed`

- [ ] **Step 5: Commit**

```bash
git commit -qam "Test: match current rest-machining help wording"
```

---

### Task 1: Moves record their programmed feed

**Files:**
- Modify: `src/simulation.rs` (`Move` struct ~line 17; Facing `push` closure ~line 341; tests module ~line 1175)
- Modify: `src/cam.rs` (`Builder::push` ~line 194; drilling dwell ~line 641)
- Modify: `src/pocket.rs` (`push` ~line 79)

**Interfaces:**
- Produces: `simulation::Move { .., pub feed: f32, .. }`. The value is mm/min for `Rapid` and `Feed`, and `0.` for `Dwell`. It is the same rate the move's timing uses.

- [ ] **Step 1: Write the failing test** (append inside `mod tests` in `src/simulation.rs`, the one that has `fn fixture() -> (Document, Mesh)`)

```rust
    #[test]
    fn every_move_records_the_rate_its_timing_uses() {
        let (doc, mesh) = fixture();
        let mut plans = 0;
        sequence(&doc, &mesh, None, |_, _, result| {
            let plan = result.unwrap().plan;
            plans += 1;
            for m in &plan.moves {
                let distance = (0..3)
                    .map(|i| (m.to[i] - m.from[i]).powi(2))
                    .sum::<f32>()
                    .sqrt();
                if m.motion == Motion::Dwell {
                    assert_eq!(m.feed, 0.);
                } else {
                    assert!(m.feed > 0.);
                    assert!(((m.end - m.start) - distance / m.feed * 60.).abs() < 0.01, "{m:?}");
                }
            }
            if plan.strategy == Strategy::Face {
                assert!(
                    plan.moves
                        .iter()
                        .filter(|m| m.motion == Motion::Feed)
                        .all(|m| m.feed == 1000. || m.feed == 250.)
                );
            }
        });
        assert_eq!(plans, 3);
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test --locked --quiet every_move_records 2>&1 | tail -5`
Expected: compile error `no field feed on type &Move`.

- [ ] **Step 3: Add the field and set it at all four construction sites**

`src/simulation.rs`, `Move`:
```rust
pub struct Move {
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub motion: Motion,
    pub cutting_pass: bool,
    /// Programmed rate (mm/min) that timed this move; 0 for dwells.
    pub feed: f32,
    pub start: f32,
    pub end: f32,
}
```
`src/simulation.rs` Facing `push` closure, `src/cam.rs` `Builder::push` and `src/pocket.rs` `push`: add `feed: rate,` after `cutting_pass: ...,` in the `Move { .. }` literal.
`src/cam.rs` Counterboring dwell literal (`motion: Motion::Dwell`): add `feed: 0.,` after `cutting_pass: false,`.

- [ ] **Step 4: Run the tests**

Run: `cargo test --locked --quiet 2>&1 | tail -3`
Expected: `120 passed; 0 failed`.

- [ ] **Step 5: Commit**

```bash
cargo fmt && git commit -qam "Record each simulated move's programmed feed"
```

---

### Task 2: Meshes remember the source origin

**Files:**
- Modify: `src/mesh.rs` (`Mesh` struct ~line 41, `normalize` ~line 198, `sample` ~line 280, tests ~line 410)

**Interfaces:**
- Produces: `mesh::Mesh { pub origin: [f32; 3], .. }`, the display-space position of the source file's (0,0,0). `Placement::transform(mesh.origin) / display_scale` gives it in plan mm.

- [ ] **Step 1: Write the failing test** (append inside `mod tests` in `src/mesh.rs`)

```rust
    #[test]
    fn normalize_remembers_the_source_origin() {
        let mesh = normalize(vec![[[10., 20., 30.], [20., 20., 30.], [10., 40., 35.]]]).unwrap();
        let s = mesh.display_scale();
        let expected = [-15. * s, -30. * s, -30. * s + 4.];
        for k in 0..3 {
            assert!((mesh.origin[k] - expected[k]).abs() < 1e-3, "{:?}", mesh.origin);
        }
        let sources = [[10., 20., 30.], [20., 20., 30.], [10., 40., 35.]];
        for p in &mesh.faces[0].points {
            assert!(sources.iter().any(|q| (0..3).all(|k| (p[k] - (mesh.origin[k] + q[k] * s)).abs() < 1e-3)));
        }
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test --locked --quiet normalize_remembers 2>&1 | tail -5`
Expected: compile error `no field origin`.

- [ ] **Step 3: Implement**

In `pub struct Mesh`, after `pub units: &'static str,`:
```rust
    /// Display-space position of the source file's (0,0,0).
    pub origin: [f32; 3],
```
In `normalize`, after `let scale = 140. / extent;`:
```rust
    let origin = [
        -(min[0] + dimensions[0] / 2.) * scale,
        -(min[1] + dimensions[1] / 2.) * scale,
        -min[2] * scale + 4.,
    ];
```
Add `origin,` to the `Ok(Mesh { .. })` literal, and `origin: [0., 0., 0.],` to the `Mesh { .. }` literal in `sample()`.

- [ ] **Step 4: Run the tests**

Run: `cargo test --locked --quiet 2>&1 | tail -3`
Expected: `121 passed; 0 failed`.

- [ ] **Step 5: Commit**

```bash
cargo fmt && git commit -qam "Keep the source model origin through mesh normalization"
```

---

### Task 3: Tool numbers and machine post settings

**Files:**
- Modify: `src/model.rs` (Machine::new ~line 184; Tool impl ~line 286; Document::sample ~line 803; Document impl; new constants/functions near `facing_setting` ~line 373; tests ~line 1170)
- Modify: `src/storage.rs` (`load` ~line 13; tests ~line 85)

**Interfaces:**
- Produces:
  - `pub const TOOL_NUMBER_MAX: u32 = 9999;`
  - `Tool::number(&self) -> Option<u32>`
  - `Document::next_tool_number(&self) -> u32`, `Document::number_tools(&mut self) -> bool`, `Document::tool_number_issues(&self, tool: &Tool) -> Vec<String>`
  - `pub const POST_TOOL_LENGTH: [&str; 2]`, `POST_RETRACT: [&str; 2]`, `POST_AIR: [&str; 3]`, `POST_KEYS: [&str; 5]`
  - `pub fn post_setting<'a>(p: &'a Fields, key: &str) -> &'a str`, `pub fn post_setting_issues(p: &Fields) -> Vec<String>`
  - `Machine::fill_post_defaults(&mut self)`

- [ ] **Step 1: Write the failing tests** (append inside `mod tests` in `src/model.rs`)

```rust
    #[test]
    fn tool_numbers_migrate_once_and_flag_duplicates() {
        let mut d = Document::sample();
        let numbers = |d: &Document| d.tools.iter().map(|t| t.number()).collect::<Vec<_>>();
        assert_eq!(numbers(&d), vec![Some(1), Some(2), Some(3)]);
        for t in &mut d.tools {
            t.p.remove("number");
        }
        d.tools[1].p.insert("number".into(), "1".into());
        assert!(d.number_tools());
        assert_eq!(numbers(&d), vec![Some(2), Some(1), Some(3)]);
        assert!(!d.number_tools());
        d.tools[2].p.insert("number".into(), "2".into());
        assert!(d.tool_number_issues(&d.tools[2]).iter().any(|e| e.contains("T2")));
        d.tools[2].p.insert("number".into(), "0".into());
        assert!(!d.tool_number_issues(&d.tools[2]).is_empty());
        assert!(!d.number_tools());
        assert_eq!(d.next_tool_number(), 3);
    }
    #[test]
    fn legacy_machines_use_post_defaults_and_bad_values_are_reported() {
        let mut m = Machine::new(3);
        assert!(post_setting_issues(&m.p).is_empty());
        for k in POST_KEYS {
            m.p.remove(k);
        }
        assert_eq!(post_setting(&m.p, "post_tool_length"), "Tool table (G43 H#)");
        assert_eq!(post_setting(&m.p, "post_retract"), "Machine Z home (G53 G0 Z0)");
        assert_eq!(post_setting(&m.p, "post_blend"), "0.01");
        assert!(post_setting_issues(&m.p).is_empty());
        m.fill_post_defaults();
        assert_eq!(get(&m.p, "post_dwell"), "2");
        m.p.insert("post_blend".into(), "-1".into());
        m.p.insert("post_dwell".into(), "90".into());
        m.p.insert("post_air".into(), "M50".into());
        assert_eq!(post_setting_issues(&m.p).len(), 3);
        assert!(m.issues().is_empty(), "post settings never block simulation");
    }
```
Append inside `mod tests` in `src/storage.rs`:
```rust
    #[test]
    fn loading_numbers_legacy_tools_once() {
        let p = std::env::temp_dir().join(format!("axis-{}.json", uuid::Uuid::new_v4()));
        let mut d = Document::sample();
        for t in &mut d.tools {
            t.p.remove("number");
        }
        save(&p, &d).unwrap();
        let loaded = load(&p).unwrap();
        let numbers: Vec<_> = loaded.tools.iter().map(|t| t.number()).collect();
        assert_eq!(numbers, vec![Some(1), Some(2), Some(3)]);
        fs::remove_file(p).unwrap();
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --locked --quiet tool_numbers legacy_machines loading_numbers 2>&1 | tail -5`
Expected: compile errors (`number`, `POST_KEYS` … not found).

- [ ] **Step 3: Implement in `src/model.rs`**

After `facing_setting_issues` add:
```rust
pub const TOOL_NUMBER_MAX: u32 = 9999;
pub const POST_TOOL_LENGTH: [&str; 2] = ["Tool table (G43 H#)", "Touch off each tool (one program per tool)"];
pub const POST_RETRACT: [&str; 2] = ["Machine Z home (G53 G0 Z0)", "Setup clearance plane"];
pub const POST_AIR: [&str; 3] = ["M7", "M8", "Not available"];
pub const POST_KEYS: [&str; 5] = [
    "post_tool_length",
    "post_retract",
    "post_blend",
    "post_dwell",
    "post_air",
];
/// Machines saved before the postprocessor settings use these defaults.
pub fn post_setting<'a>(p: &'a Fields, key: &str) -> &'a str {
    p.get(key).map(String::as_str).unwrap_or(match key {
        "post_tool_length" => POST_TOOL_LENGTH[0],
        "post_retract" => POST_RETRACT[0],
        "post_blend" => "0.01",
        "post_dwell" => "2",
        "post_air" => POST_AIR[0],
        _ => "",
    })
}
/// Export-only checks; they are deliberately not part of `Machine::issues`.
pub fn post_setting_issues(p: &Fields) -> Vec<String> {
    let mut e = vec![];
    for (key, values) in [
        ("post_tool_length", &POST_TOOL_LENGTH[..]),
        ("post_retract", &POST_RETRACT[..]),
        ("post_air", &POST_AIR[..]),
    ] {
        if !values.contains(&post_setting(p, key)) {
            e.push(format!("Choose a valid {} setting.", label(key)));
        }
    }
    let value = |k: &str| post_setting(p, k).trim().parse::<f32>().ok().filter(|v| v.is_finite());
    if !value("post_blend").is_some_and(|v| (0. ..=1.).contains(&v)) {
        e.push("Path blending tolerance must be between 0 and 1 mm.".into());
    }
    if !value("post_dwell").is_some_and(|v| (0. ..=60.).contains(&v)) {
        e.push("Spindle start dwell must be between 0 and 60 seconds.".into());
    }
    e
}
```
In `Machine::new`'s `fields(&[...])` list, after `("tcp", "No"),` add:
```rust
            ("post_tool_length", "Tool table (G43 H#)"),
            ("post_retract", "Machine Z home (G53 G0 Z0)"),
            ("post_blend", "0.01"),
            ("post_dwell", "2"),
            ("post_air", "M7"),
```
In `impl Machine`, add:
```rust
    pub fn fill_post_defaults(&mut self) {
        for key in POST_KEYS {
            if !self.p.contains_key(key) {
                let value = post_setting(&self.p, key).to_string();
                self.p.insert(key.into(), value);
            }
        }
    }
```
In `impl Tool`, add:
```rust
    /// Controller tool number (T#), when it is a whole number 1–9999.
    pub fn number(&self) -> Option<u32> {
        get(&self.p, "number")
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|n| (1..=TOOL_NUMBER_MAX).contains(n))
    }
```
In the second `impl Document` block (the one with `has_model`), add:
```rust
    pub fn next_tool_number(&self) -> u32 {
        (1..=TOOL_NUMBER_MAX)
            .find(|n| !self.tools.iter().any(|t| t.number() == Some(*n)))
            .unwrap_or(TOOL_NUMBER_MAX)
    }
    /// Number tools saved before tool numbers existed. Entered values are kept.
    pub fn number_tools(&mut self) -> bool {
        let mut changed = false;
        for i in 0..self.tools.len() {
            if get(&self.tools[i].p, "number").trim().is_empty() {
                let n = self.next_tool_number();
                self.tools[i].p.insert("number".into(), n.to_string());
                changed = true;
            }
        }
        changed
    }
    pub fn tool_number_issues(&self, tool: &Tool) -> Vec<String> {
        let Some(n) = tool.number() else {
            return vec![format!(
                "{}: enter a tool number from 1 to {TOOL_NUMBER_MAX}.",
                tool.name()
            )];
        };
        self.tools
            .iter()
            .filter(|t| t.id != tool.id && t.number() == Some(n))
            .map(|t| format!("{}: tool number T{n} is also used by {}.", tool.name(), t.name()))
            .collect()
    }
```
In `Document::sample()`, replace the final `Self { .. }` expression with a binding, then number the tools:
```rust
        let mut doc = Self {
            schema: 1,
            sample_model: true,
            title: "Bearing housing".into(),
            material: "Aluminum 6061".into(),
            file: None,
            active: s.id.clone(),
            machines,
            tools,
            setups: vec![s],
        };
        doc.number_tools();
        doc
```

- [ ] **Step 4: Migrate on load in `src/storage.rs`**

In `load`, immediately before `d.validate_structure()?;` add:
```rust
    d.number_tools();
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --locked --quiet 2>&1 | tail -3`
Expected: `124 passed; 0 failed`.

- [ ] **Step 6: Commit**

```bash
cargo fmt && git commit -qam "Add tool numbers and LinuxCNC post settings to the model"
```

---

### Task 4: Strict LinuxCNC reader (`ngc.rs`)

**Files:**
- Create: `src/ngc.rs`
- Modify: `src/main.rs:1-19` (add `mod ngc;`)

**Interfaces:**
- Produces:
  - `pub enum Spindle { Off, Cw, Ccw }`, `pub enum Blend { Unset, Exact, Tolerance(f64) }`, `pub enum StepKind { Rapid, Feed, Dwell }`
  - `pub struct State { pub metric, absolute, xy_plane, per_minute, comp_off, cycle_off: bool, pub blend: Blend, pub offset: Option<String>, pub selected: Option<u32>, pub tool: Option<u32>, pub length: Option<u32>, pub touched_off: bool, pub spindle: Spindle, pub rpm: f64, pub mist: bool, pub flood: bool, pub feed: f64 }`
  - `pub struct Step { pub line: usize, pub kind: StepKind, pub machine: bool, pub from: [Option<f64>; 3], pub to: [Option<f64>; 3], pub seconds: f64, pub state: State }` (for a `G53` step, `to` holds the machine values and work Z becomes unknown)
  - `pub struct Replay { pub steps: Vec<Step>, pub messages: Vec<String>, pub ended: bool, pub end_state: State }`
  - `pub fn replay(text: &str) -> Result<Replay, String>`

- [ ] **Step 1: Write the failing tests** (create `src/ngc.rs` with only the test module, and add `mod ngc;` after `mod model;` in `src/main.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use StepKind::*;
    const SAMPLE: &str = "(header)\nG17 G21 G40 G49 G80 G90 G94\nG64 P0.010\nG54\nG53 G0 Z0\nT3 M6\nG43 H3\nS8000 M3\nG4 P2.000\nM8\nG0 X1.000 Y2.000\nZ5.000\nG1 Z-1.000 F250.0\nX4.000 F1000.0\nG0 Z5.000\nM9\nM5\nG53 G0 Z0\nM2\n";
    #[test]
    fn replays_motion_and_modal_state() {
        let r = replay(SAMPLE).unwrap();
        assert!(r.ended);
        let kinds: Vec<_> = r.steps.iter().map(|s| (s.kind, s.machine)).collect();
        assert_eq!(
            kinds,
            vec![
                (Rapid, true),
                (Dwell, false),
                (Rapid, false),
                (Rapid, false),
                (Feed, false),
                (Feed, false),
                (Rapid, false),
                (Rapid, true)
            ]
        );
        assert_eq!(r.steps[0].to, [None, None, Some(0.)]);
        assert_eq!(r.steps[2].to, [Some(1.), Some(2.), None]);
        let cut = &r.steps[5];
        assert_eq!(cut.from, [Some(1.), Some(2.), Some(-1.)]);
        assert_eq!(cut.to, [Some(4.), Some(2.), Some(-1.)]);
        assert_eq!(cut.state.feed, 1000.);
        assert_eq!((cut.state.tool, cut.state.length), (Some(3), Some(3)));
        assert_eq!((cut.state.spindle, cut.state.rpm, cut.state.flood), (Spindle::Cw, 8000., true));
        assert_eq!(cut.state.offset.as_deref(), Some("G54"));
        assert_eq!(cut.state.blend, Blend::Tolerance(0.01));
        assert_eq!(r.steps[1].seconds, 2.);
        assert_eq!((r.end_state.spindle, r.end_state.flood), (Spindle::Off, false));
    }
    #[test]
    fn touch_off_mode_is_visible_in_state() {
        let text = "G17 G21 G40 G49 G80 G90 G94\nG61\nG59.1\nT2 M6\n(MSG, Touch off Z for T2)\nM0\nS1000 M4\nG0 X0 Y0 Z10\nG1 Z0 F100\nM5\nM2\n";
        let r = replay(text).unwrap();
        let cut = r.steps.last().unwrap();
        assert!(cut.state.touched_off);
        assert_eq!(cut.state.length, None);
        assert_eq!(cut.state.spindle, Spindle::Ccw);
        assert_eq!(cut.state.blend, Blend::Exact);
        assert_eq!(cut.state.offset.as_deref(), Some("G59.1"));
        assert_eq!(r.messages, vec!["Touch off Z for T2".to_string()]);
    }
    #[test]
    fn rejects_anything_the_post_does_not_emit() {
        let preamble = "G17 G21 G40 G49 G80 G90 G94\nG54\n";
        for bad in [
            "G2 X1 Y1",
            "G1 X1",
            "N10 G0 X1",
            "G0 X1.2.3",
            "G0 X1 (open",
            "G0 X1 X2",
            "X1",
            "M6",
            "G43 H1.5",
            "M98 P1",
            "G0 X1 ; note",
            "g0 x1",
            "G53 Z0",
        ] {
            assert!(replay(&format!("{preamble}{bad}\nM2\n")).is_err(), "{bad}");
        }
        assert!(replay("G0 X1\nM2\n").is_err());
        assert!(replay(&format!("{preamble}M2\nG0 X1\n")).is_err());
        assert!(replay("G17 G21 G40 G49 G80 G90 G94\nG0 X1\nM2\n").is_err());
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --locked --quiet ngc:: 2>&1 | tail -5`
Expected: compile errors (`replay`, `StepKind` not found).

- [ ] **Step 3: Implement the reader** (insert above the test module in `src/ngc.rs`)

```rust
//! Strict reader for the LinuxCNC subset that `post` writes. It replays modal
//! state so each exported program can be compared with the simulated moves.
//! Any word the postprocessor never emits is an error, not ignored.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spindle {
    Off,
    Cw,
    Ccw,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Blend {
    Unset,
    Exact,
    Tolerance(f64),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    Rapid,
    Feed,
    Dwell,
}
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub metric: bool,
    pub absolute: bool,
    pub xy_plane: bool,
    pub per_minute: bool,
    pub comp_off: bool,
    pub cycle_off: bool,
    pub blend: Blend,
    pub offset: Option<String>,
    pub selected: Option<u32>,
    pub tool: Option<u32>,
    pub length: Option<u32>,
    pub touched_off: bool,
    pub spindle: Spindle,
    pub rpm: f64,
    pub mist: bool,
    pub flood: bool,
    pub feed: f64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            metric: false,
            absolute: false,
            xy_plane: false,
            per_minute: false,
            comp_off: false,
            cycle_off: false,
            blend: Blend::Unset,
            offset: None,
            selected: None,
            tool: None,
            length: None,
            touched_off: false,
            spindle: Spindle::Off,
            rpm: 0.,
            mist: false,
            flood: false,
            feed: 0.,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub line: usize,
    pub kind: StepKind,
    /// G53 move: `to` holds machine values and the work position becomes unknown.
    pub machine: bool,
    pub from: [Option<f64>; 3],
    pub to: [Option<f64>; 3],
    pub seconds: f64,
    pub state: State,
}
#[derive(Debug, Clone, Default)]
pub struct Replay {
    pub steps: Vec<Step>,
    pub messages: Vec<String>,
    pub ended: bool,
    pub end_state: State,
}
/// Supported G codes ×10: G0 G1 G4 G17 G21 G40 G43 G49 G53 G54–G59.3 G61 G64 G80 G90 G94.
const G_CODES: [i64; 23] = [
    0, 10, 40, 170, 210, 400, 430, 490, 530, 540, 550, 560, 570, 580, 590, 591, 592, 593, 610,
    640, 800, 900, 940,
];
const M_CODES: [i64; 9] = [0, 2, 3, 4, 5, 6, 7, 8, 9];

fn strip_comments(raw: &str, messages: &mut Vec<String>) -> Result<String, String> {
    let mut code = String::new();
    let mut rest = raw;
    while let Some(open) = rest.find('(') {
        code.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after
            .find(')')
            .ok_or_else(|| "unterminated comment".to_string())?;
        let comment = &after[..close];
        if comment.contains('(') {
            return Err("nested comment".into());
        }
        if let Some(message) = comment.strip_prefix("MSG,") {
            messages.push(message.trim().to_string());
        }
        rest = &after[close + 1..];
    }
    if rest.contains(')') {
        return Err("unmatched )".into());
    }
    code.push_str(rest);
    Ok(code)
}
fn number(text: &str) -> Option<f64> {
    let digits = text.strip_prefix(|c| c == '-' || c == '+').unwrap_or(text);
    if digits.is_empty()
        || digits.matches('.').count() > 1
        || !digits.chars().all(|c| c.is_ascii_digit() || c == '.')
        || !digits.chars().any(|c| c.is_ascii_digit())
    {
        return None;
    }
    text.parse::<f64>().ok().filter(|v| v.is_finite())
}
fn words(code: &str) -> Result<Vec<(char, f64)>, String> {
    let chars: Vec<char> = code.chars().filter(|c| *c != ' ' && *c != '\t').collect();
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        let letter = chars[i];
        if !"GMTSFHPXYZ".contains(letter) {
            return Err(format!("unsupported word {letter}"));
        }
        let mut end = i + 1;
        while end < chars.len() && (chars[end].is_ascii_digit() || matches!(chars[end], '.' | '-' | '+')) {
            end += 1;
        }
        let text: String = chars[i + 1..end].iter().collect();
        out.push((letter, number(&text).ok_or_else(|| format!("malformed number after {letter}"))?));
        i = end;
    }
    Ok(out)
}
pub fn replay(text: &str) -> Result<Replay, String> {
    let mut out = Replay::default();
    let mut state = State::default();
    let mut position: [Option<f64>; 3] = [None; 3];
    let mut motion: Option<StepKind> = None;
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let fail = |why: &str| format!("line {line}: {why}: {raw}");
        let code = strip_comments(raw, &mut out.messages).map_err(|e| fail(&e))?;
        if code.trim().is_empty() {
            continue;
        }
        if out.ended {
            return Err(fail("code after M2"));
        }
        let mut g: Vec<i64> = vec![];
        let mut m: Vec<i64> = vec![];
        let mut axes: [Option<f64>; 3] = [None; 3];
        let (mut f, mut s, mut t, mut h, mut p) = (None, None, None, None, None);
        for (letter, value) in words(&code).map_err(|e| fail(&e))? {
            let slot = match letter {
                'G' => {
                    g.push((value * 10.).round() as i64);
                    continue;
                }
                'M' => {
                    if value.fract() != 0. {
                        return Err(fail("unsupported M code"));
                    }
                    m.push(value as i64);
                    continue;
                }
                'X' => &mut axes[0],
                'Y' => &mut axes[1],
                'Z' => &mut axes[2],
                'F' => &mut f,
                'S' => &mut s,
                'T' => &mut t,
                'H' => &mut h,
                _ => &mut p,
            };
            if slot.replace(value).is_some() {
                return Err(fail("repeated word"));
            }
        }
        if let Some(code) = g.iter().find(|c| !G_CODES.contains(c)) {
            return Err(fail(&format!("unsupported G{}", *code as f64 / 10.)));
        }
        if let Some(code) = m.iter().find(|c| !M_CODES.contains(c)) {
            return Err(fail(&format!("unsupported M{code}")));
        }
        let has = |c: i64| g.contains(&c);
        let motions = g.iter().filter(|c| **c == 0 || **c == 10).count();
        if motions > 1
            || m.iter().filter(|c| (3..=5).contains(*c)).count() > 1
            || g.iter().filter(|c| (540..=593).contains(*c)).count() > 1
            || (has(430) && has(490))
            || (has(40) && has(640))
            || (has(610) && has(640))
        {
            return Err(fail("conflicting codes on one line"));
        }
        if h.is_some() && !has(430) {
            return Err(fail("H without G43"));
        }
        if p.is_some() && !has(40) && !has(640) {
            return Err(fail("P without G4 or G64"));
        }
        if let Some(v) = f {
            if v <= 0. {
                return Err(fail("F must be positive"));
            }
            state.feed = v;
        }
        if let Some(v) = s {
            if v < 0. {
                return Err(fail("S cannot be negative"));
            }
            state.rpm = v;
        }
        if let Some(v) = t {
            if v < 0. || v.fract() != 0. {
                return Err(fail("T must be a whole number"));
            }
            state.selected = Some(v as u32);
        }
        if m.contains(&6) {
            state.tool = Some(state.selected.ok_or_else(|| fail("M6 without a selected tool"))?);
            state.touched_off = false;
        }
        if m.contains(&3) || m.contains(&4) {
            if state.rpm <= 0. {
                return Err(fail("spindle start without S"));
            }
            state.spindle = if m.contains(&3) { Spindle::Cw } else { Spindle::Ccw };
        }
        if m.contains(&5) {
            state.spindle = Spindle::Off;
        }
        if m.contains(&9) {
            if m.contains(&7) || m.contains(&8) {
                return Err(fail("M9 with M7 or M8"));
            }
            state.mist = false;
            state.flood = false;
        }
        state.mist |= m.contains(&7);
        state.flood |= m.contains(&8);
        if has(40) {
            if axes.iter().any(Option::is_some) {
                return Err(fail("G4 with axis words"));
            }
            let seconds = p.filter(|v| *v >= 0.).ok_or_else(|| fail("G4 needs P"))?;
            out.steps.push(Step {
                line,
                kind: StepKind::Dwell,
                machine: false,
                from: position,
                to: position,
                seconds,
                state: state.clone(),
            });
        }
        state.xy_plane |= has(170);
        state.metric |= has(210);
        state.comp_off |= has(400);
        state.cycle_off |= has(800);
        state.absolute |= has(900);
        state.per_minute |= has(940);
        if has(430) {
            // A bare G43 uses the loaded tool, as LinuxCNC does.
            state.length = match h {
                None => state.tool,
                Some(v) if v >= 0. && v.fract() == 0. => Some(v as u32),
                Some(_) => return Err(fail("G43 needs a whole-number H")),
            };
        }
        if has(490) {
            state.length = None;
        }
        if let Some(code) = g.iter().find(|c| (540..=593).contains(*c)) {
            state.offset = Some(match code {
                591..=593 => format!("G59.{}", code - 590),
                c => format!("G{}", c / 10),
            });
        }
        if has(610) {
            state.blend = Blend::Exact;
        }
        if has(640) {
            state.blend = Blend::Tolerance(p.filter(|v| *v >= 0.).ok_or_else(|| fail("G64 needs P"))?);
        }
        if let Some(code) = g.iter().find(|c| **c == 0 || **c == 10) {
            motion = Some(if *code == 0 { StepKind::Rapid } else { StepKind::Feed });
        }
        let machine = has(530);
        if machine && (motions == 0 || axes.iter().all(Option::is_none)) {
            return Err(fail("G53 needs G0 or G1 and an axis word"));
        }
        if axes.iter().any(Option::is_some) {
            let kind = motion.ok_or_else(|| fail("axis words without G0 or G1"))?;
            if !(state.metric
                && state.absolute
                && state.xy_plane
                && state.per_minute
                && state.comp_off
                && state.cycle_off)
            {
                return Err(fail("motion before G17 G21 G40 G80 G90 G94"));
            }
            if kind == StepKind::Feed && state.feed <= 0. {
                return Err(fail("G1 without F"));
            }
            let from = position;
            let to = if machine {
                for k in 0..3 {
                    if axes[k].is_some() {
                        position[k] = None;
                    }
                }
                axes
            } else {
                if state.offset.is_none() {
                    return Err(fail("motion before a work offset"));
                }
                for k in 0..3 {
                    if axes[k].is_some() {
                        position[k] = axes[k];
                    }
                }
                position
            };
            out.steps.push(Step {
                line,
                kind,
                machine,
                from,
                to,
                seconds: 0.,
                state: state.clone(),
            });
        }
        state.touched_off |= m.contains(&0);
        out.ended |= m.contains(&2);
    }
    out.end_state = state;
    Ok(out)
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --locked --quiet 2>&1 | tail -3`
Expected: `127 passed; 0 failed`.

- [ ] **Step 5: Commit**

```bash
cargo fmt && git add -A && git commit -qm "Add a strict LinuxCNC reader for exported programs"
```

---

### Task 5: Emitter: settings, datum, blocks and program text (`post.rs`)

**Files:**
- Create: `src/post.rs`
- Modify: `src/main.rs` (add `mod post;` after `mod pocket;`)

**Interfaces:**
- Consumes: `Move.feed` (Task 1), `Mesh.origin` (Task 2), `post_setting*`/`POST_*` (Task 3).
- Produces:
  - `pub const OFFSETS: [&str; 9]`
  - `pub struct PostSettings { pub tool_table: bool, pub machine_home: bool, pub blend: f32, pub dwell: f32, pub air: Option<(bool, bool)> }` and `PostSettings::from_machine(&Machine) -> Result<Self, Vec<String>>`. `air` is `(mist, flood)`.
  - `pub fn datum(setup: &Setup, stock: Bounds, mesh: &Mesh) -> Result<[f32; 3], String>`
  - `pub fn coord(v: f32) -> String`, `pub fn comment(s: &str) -> String`, `pub fn preview(text: &str, lines: usize) -> String`
  - `pub struct OpState { pub name: String, pub strategy: &'static str, pub tool: u32, pub tool_id: String, pub tool_name: String, pub diameter: f32, pub stickout: f32, pub rpm: f32, pub clockwise: bool, pub coolant: (bool, bool) }`
  - `pub enum Kind { Rapid, Feed, Dwell }`
  - `pub enum Block { Stop, Retract, Start { op: usize, change: bool }, Run { op: usize, spindle: bool, coolant: bool }, Wait { seconds: f32 }, Approach { op: usize, xy: [f32; 2] }, Descend { op: usize, xy: [f32; 2], z: f32 }, Move { op: usize, kind: Kind, from: [f32; 3], to: [f32; 3], feed: f32, seconds: f32 }, End }`
  - `pub fn blocks(ops: &[OpState], plans: &[&Plan], settings: &PostSettings) -> Vec<Block>`
  - `pub fn clearance_plane(blocks: &[Block]) -> f32`, `pub fn extents(blocks: &[Block]) -> Bounds` (plan coordinates)
  - `pub struct Header<'a> { pub job: &'a str, pub setup: &'a Setup, pub number: usize, pub part: Option<(usize, usize)>, pub stock: Bounds, pub datum: [f32; 3], pub disabled: Vec<String> }`
  - `pub fn emit(h: &Header, ops: &[OpState], blocks: &[Block], settings: &PostSettings) -> String`
  - `#[cfg(test)] pub(crate) mod fixture { pub fn plan(start, moves) -> Plan; pub fn facing_op() -> OpState; pub fn job() -> (Document, Mesh); pub fn flipped_job() -> (Document, Mesh) }`

- [ ] **Step 1: Write the failing tests** (create `src/post.rs` with the fixture and test modules below. Add `mod post;` to `src/main.rs`.)

```rust
#[cfg(test)]
pub(crate) mod fixture {
    use super::OpState;
    use crate::{
        mesh::{Bounds, Mesh},
        model::*,
        simulation::{CutterShape, Motion, Move, Plan},
        stock::Placement,
        surface::Selection,
    };
    /// A Facing-shaped plan on a 20 × 20 × 10 mm stock, starting at `start`.
    /// For dwells the third tuple value is seconds, otherwise mm/min.
    pub fn plan(start: [f32; 3], moves: &[(Motion, [f32; 3], f32)]) -> Plan {
        let mut plan = Plan {
            shape: CutterShape::Flat,
            strategy: Strategy::Face,
            operation: "op".into(),
            setup: "setup".into(),
            moves: vec![],
            stock: Bounds {
                min: [-10., -10., 0.],
                max: [10., 10., 10.],
            },
            radius: 5.,
            flute_length: 25.,
            tool_length: 35.,
            holder_radius: 16.,
            display_scale: 1.,
            duration: 0.,
            target: 9.,
            levels: 1,
            rest_top: None,
            spindle_clockwise: true,
            milling_direction: "Climb",
        };
        let mut at = start;
        for (motion, to, rate) in moves {
            let seconds = if *motion == Motion::Dwell {
                *rate
            } else {
                (0..3).map(|k| (to[k] - at[k]).powi(2)).sum::<f32>().sqrt() / rate * 60.
            };
            plan.moves.push(Move {
                from: at,
                to: *to,
                motion: *motion,
                cutting_pass: *motion == Motion::Feed,
                feed: if *motion == Motion::Dwell { 0. } else { *rate },
                start: plan.duration,
                end: plan.duration + seconds,
            });
            plan.duration += seconds;
            at = *to;
        }
        plan
    }
    pub fn facing_op() -> OpState {
        OpState {
            name: "Face top".into(),
            strategy: "Facing",
            tool: 1,
            tool_id: "t1".into(),
            tool_name: "Flat end mill · Ø10".into(),
            diameter: 10.,
            stickout: 35.,
            rpm: 8000.,
            clockwise: true,
            coolant: (false, true),
        }
    }
    /// All six implemented strategies on the example block, in one G54 setup.
    pub fn job() -> (Document, Mesh) {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = manifest.join("examples/four-setup-block.3mf");
        let mesh = crate::mesh::load(&path).unwrap();
        let mut doc = Document::default();
        doc.title = "Fixture block".into();
        let mut tools = vec![
            Tool::new("Flat 10", "Flat end mill", "10"),
            Tool::new("Ball 6", "Ball end mill", "6"),
            Tool::new("Drill 4", "Drill", "4"),
        ];
        for (n, t) in tools.iter_mut().enumerate() {
            t.p.insert("length".into(), "60".into());
            t.p.insert("stickout".into(), "65".into());
            t.p.insert("number".into(), (n + 1).to_string());
        }
        let mut setup = Setup::new(Machine::new(3), "G54");
        setup.p.insert("name".into(), "Top".into());
        for (k, v) in [
            ("stock_top", "4"),
            ("stock_bottom", "3"),
            ("stock_left", "2"),
            ("stock_right", "2"),
            ("stock_front", "2"),
            ("stock_back", "2"),
        ] {
            setup.p.insert(k.into(), v.into());
        }
        let placement = Placement::new(&mesh, Some(&setup));
        let top_z = placement.bounds.max[2];
        let flats: Vec<(usize, f32, f32)> = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| s.normal[2].abs() > 0.999999)
            .map(|(i, s)| (i, placement.transform(s.center)[2], s.area))
            .collect();
        let tops: Vec<usize> = flats
            .iter()
            .filter(|f| (f.1 - top_z).abs() < 0.01)
            .map(|f| f.0)
            .collect();
        let top = flats
            .iter()
            .filter(|f| (f.1 - top_z).abs() < 0.01)
            .max_by(|a, b| a.2.total_cmp(&b.2))
            .unwrap()
            .0;
        let floor = flats
            .iter()
            .filter(|f| f.1 < top_z - 0.1 && f.1 > placement.bounds.min[2] + 0.1)
            .max_by(|a, b| a.2.total_cmp(&b.2))
            .unwrap()
            .0;
        let selection = |ids: Vec<usize>| {
            Some(Selection {
                mesh_fingerprint: mesh.fingerprint.clone(),
                ids,
            })
        };
        let (flat, ball, drill) = (tools[0].id.clone(), tools[1].id.clone(), tools[2].id.clone());
        let mut face = Operation::new(Strategy::Face, flat.clone());
        face.p.insert("boundary".into(), "Stock silhouette".into());
        face.p.insert("containment".into(), "Tool outside boundary".into());
        face.p.insert("allowance".into(), "0".into());
        face.surfaces = selection(vec![top]);
        let mut pocket = Operation::new(Strategy::Pocket, flat.clone());
        pocket.surfaces = selection(vec![floor]);
        let contour = Operation::new(Strategy::Contour, flat.clone());
        let mut drilling = Operation::new(Strategy::Drill, drill);
        drilling.surfaces = selection(tops);
        drilling.p.insert("cycle".into(), "Deep drilling".into());
        let mut parallel = Operation::new(Strategy::Parallel, ball);
        parallel.p.insert("surface_stepover".into(), "2".into());
        parallel.p.insert("tolerance".into(), "0.1".into());
        let waterline = Operation::new(Strategy::Waterline, flat);
        setup.operations = vec![face, pocket, contour, drilling, parallel, waterline];
        for op in &mut setup.operations {
            op.geometry.clear();
        }
        doc.machines = vec![setup.machine.clone()];
        doc.tools = tools;
        doc.active = setup.id.clone();
        doc.setups = vec![setup];
        doc.file = Some(ModelFile {
            name: "four-setup-block.3mf".into(),
            path: path.to_string_lossy().into_owned(),
            format: "3mf".into(),
            bytes: 0,
        });
        doc.sync_stock_sources();
        (doc, mesh)
    }
    /// Facing and Pocket on G54, then the part flipped about X and faced on G55.
    pub fn flipped_job() -> (Document, Mesh) {
        let (mut doc, mesh) = job();
        doc.setups[0].operations.truncate(2);
        let mut flipped = Setup::new(Machine::new(3), "G55");
        flipped.p.insert("name".into(), "Flipped".into());
        flipped.p.insert("rx".into(), "180".into());
        let placement = Placement::new(&mesh, Some(&flipped));
        let top = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| (placement.transform(s.center)[2] - placement.bounds.max[2]).abs() < 0.01)
            .max_by(|(_, a), (_, b)| a.area.total_cmp(&b.area))
            .unwrap()
            .0;
        let mut face = doc.setups[0].operations[0].clone();
        face.id = id();
        face.surfaces = Some(Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: vec![top],
        });
        flipped.operations.push(face);
        doc.setups.push(flipped);
        doc.sync_stock_sources();
        (doc, mesh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::Motion;
    fn golden_plan() -> Plan {
        fixture::plan(
            [-15., -15., 25.],
            &[
                (Motion::Rapid, [-15., -15., 11.], 6000.),
                (Motion::Feed, [-15., -15., 9.], 250.),
                (Motion::Feed, [15., -15., 9.], 1000.),
                (Motion::Rapid, [15., -15., 25.], 6000.),
            ],
        )
    }
    fn program(machine: Machine) -> (String, Vec<Block>, PostSettings) {
        let settings = PostSettings::from_machine(&machine).unwrap();
        let mut setup = Setup::new(machine, "G54");
        setup.p.insert("name".into(), "Top side".into());
        let plan = golden_plan();
        let ops = vec![fixture::facing_op()];
        let blocks = blocks(&ops, &[&plan], &settings);
        let header = Header {
            job: "Golden",
            setup: &setup,
            number: 1,
            part: None,
            stock: plan.stock,
            datum: [0., 0., 10.],
            disabled: vec![],
        };
        (emit(&header, &ops, &blocks, &settings), blocks, settings)
    }
    const GOLDEN: &str = "(AXIS CAM Studio - LinuxCNC 3-axis program)
(Job: Golden - Setup 1: Top side - G54)
(Machine: Workshop Mill v1)
(Work zero: Stock top center. Touch off X0 Y0 Z0 there.)
(Stock: 20.000 x 20.000 x 10.000 mm. Part rotation X0 Y0 Z0 deg.)
(Fixture: Low profile clamps)
(T1: Flat end mill - D10 - diameter 10.000 mm - stickout 35.000 mm)
(Operation 1: Face top - Facing - T1 - S8000)
(Extents: X-15.000 to 15.000 Y-15.000 to -15.000 Z-1.000 to 15.000 mm)
(Not checked: fixtures, clamps and the machine body. LinuxCNC soft limits check absolute position. Preview in LinuxCNC and air-cut first.)
G17 G21 G40 G49 G80 G90 G94
G64 P0.010
G54
G53 G0 Z0
(Operation 1/1: Face top - T1)
T1 M6
G43 H1
G53 G0 Z0
S8000 M3
M8
G4 P2.000
G0 X-15.000 Y-15.000
G0 Z15.000
Z1.000
G1 Z-1.000 F250.0
X15.000 F1000.0
G0 Z15.000
M9
M5
G53 G0 Z0
M2
";
    #[test]
    fn golden_facing_program() {
        assert_eq!(program(Machine::new(3)).0, GOLDEN);
    }
    #[test]
    fn machine_settings_change_tool_length_retract_blend_and_dwell() {
        let mut m = Machine::new(3);
        m.p.insert("post_tool_length".into(), POST_TOOL_LENGTH[1].into());
        m.p.insert("post_retract".into(), POST_RETRACT[1].into());
        m.p.insert("post_blend".into(), "0".into());
        m.p.insert("post_dwell".into(), "0".into());
        let (text, _, _) = program(m);
        assert!(text.contains("(Load T1 Flat end mill - D10 and touch off Z at the work zero before starting this program.)\nG0 Z15.000\nS8000 M3\n"), "{text}");
        assert!(!text.contains("M6") && !text.contains("\nM0\n") && !text.contains("G43"));
        assert!(!text.contains("G53") && !text.contains("G4 P"));
        assert!(text.contains("\nG61\n"));
        assert_eq!(text.matches("G0 Z15.000").count(), 5, "{text}");
    }
    #[test]
    fn clearance_retracts_use_the_loaded_tool_length() {
        let mut m = Machine::new(3);
        m.p.insert("post_retract".into(), POST_RETRACT[1].into());
        let (text, _, _) = program(m);
        assert!(
            text.contains("\nG54\nG43\nG0 Z15.000\n(Operation 1/1: Face top - T1)\nT1 M6\nG43 H1\nG0 Z15.000\nS8000 M3\n"),
            "{text}"
        );
    }
    #[test]
    fn datum_follows_the_setup_origin() {
        let mesh = crate::mesh::normalize(vec![[[10., 20., 30.], [20., 20., 30.], [10., 40., 35.]]]).unwrap();
        let stock = Bounds {
            min: [-7., -9., 1.],
            max: [5., 11., 21.],
        };
        let mut setup = Setup::new(Machine::new(3), "G54");
        assert_eq!(datum(&setup, stock, &mesh).unwrap(), [-1., 1., 21.]);
        setup.p.insert("origin".into(), "Stock top front-left".into());
        assert_eq!(datum(&setup, stock, &mesh).unwrap(), [-7., -9., 21.]);
        setup.p.insert("origin".into(), "Model origin".into());
        let sources = [[10., 20., 30.], [20., 20., 30.], [10., 40., 35.]];
        for rx in ["0", "180"] {
            setup.p.insert("rx".into(), rx.into());
            let zero = datum(&setup, stock, &mesh).unwrap();
            let placement = Placement::new(&mesh, Some(&setup));
            for p in &mesh.faces[0].points {
                let v = placement.transform(*p).map(|v| v / mesh.display_scale());
                let rel: [f32; 3] = std::array::from_fn(|k| v[k] - zero[k]);
                let sign = if rx == "0" { [1., 1., 1.] } else { [1., -1., -1.] };
                assert!(
                    sources
                        .iter()
                        .any(|q| (0..3).all(|k| (rel[k] - sign[k] * q[k]).abs() < 1e-3)),
                    "{rx}: {rel:?}"
                );
            }
        }
        setup.p.insert("origin".into(), "Selected point (planned)".into());
        assert!(datum(&setup, stock, &mesh).is_err());
    }
    #[test]
    fn comments_cannot_escape_or_carry_non_ascii() {
        assert_eq!(comment("bad)\nG0 X1\n(Ø·×é"), "bad G0 X1 D-x?");
    }
    #[test]
    fn controller_and_axes_are_refused_before_any_output() {
        let mut m = Machine::new(3);
        m.p.insert("controller".into(), "GRBL".into());
        assert!(PostSettings::from_machine(&m).unwrap_err()[0].contains("Only the LinuxCNC"));
        assert!(PostSettings::from_machine(&Machine::new(4)).unwrap_err().iter().any(|e| e.contains("3-axis")));
    }
    #[test]
    fn tools_change_only_between_different_tools() {
        let (doc, mesh) = fixture::job();
        let mut plans = vec![];
        crate::simulation::sequence(&doc, &mesh, None, |_, _, r| plans.push(r.unwrap().plan));
        let tool_of = |i: usize| doc.setups[0].operations[i].tool.clone();
        let ops: Vec<OpState> = (0..plans.len())
            .map(|i| OpState {
                tool_id: tool_of(i),
                ..fixture::facing_op()
            })
            .collect();
        let settings = PostSettings::from_machine(&Machine::new(3)).unwrap();
        let refs: Vec<&Plan> = plans.iter().collect();
        let blocks = blocks(&ops, &refs, &settings);
        let changes: Vec<bool> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Start { change, .. } => Some(*change),
                _ => None,
            })
            .collect();
        assert_eq!(changes, vec![true, false, false, true, true, true]);
        for pair in blocks.windows(2) {
            if let [Block::Move { to, .. }, Block::Move { from, .. }] = pair {
                assert_eq!(to, from);
            }
        }
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --locked --quiet post:: 2>&1 | tail -5`
Expected: compile errors (`OpState`, `blocks`, `emit` … not found).

- [ ] **Step 3: Implement** (insert at the top of `src/post.rs`, above the fixture module)

```rust
//! LinuxCNC postprocessor for the simulated 3-axis moves. A program is written
//! only after `post_check` accepts its motion and `ngc::verify` replays the
//! text back to the same moves.
use crate::{
    mesh::{Bounds, Mesh},
    model::*,
    simulation::{Motion, Plan},
    stock::Placement,
};

pub const OFFSETS: [&str; 9] = [
    "G54", "G55", "G56", "G57", "G58", "G59", "G59.1", "G59.2", "G59.3",
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PostSettings {
    pub tool_table: bool,
    pub machine_home: bool,
    pub blend: f32,
    pub dwell: f32,
    /// (mist, flood) outputs used for Air; None when the machine has none.
    pub air: Option<(bool, bool)>,
}
impl PostSettings {
    pub fn from_machine(m: &Machine) -> Result<Self, Vec<String>> {
        let mut e = vec![];
        if get(&m.p, "controller") != "LinuxCNC" {
            e.push(format!(
                "{} uses the {} controller. Only the LinuxCNC postprocessor is implemented.",
                m.name(),
                get(&m.p, "controller")
            ));
        }
        if m.axes() != 3 || get(&m.p, "mode") != "3-axis" {
            e.push(format!(
                "{} is not a 3-axis machine. Only 3-axis output is implemented.",
                m.name()
            ));
        }
        e.extend(m.issues());
        e.extend(post_setting_issues(&m.p));
        if !e.is_empty() {
            return Err(e);
        }
        let value = |k: &str| post_setting(&m.p, k).trim().parse::<f32>().unwrap_or(0.);
        Ok(Self {
            tool_table: post_setting(&m.p, "post_tool_length") == POST_TOOL_LENGTH[0],
            machine_home: post_setting(&m.p, "post_retract") == POST_RETRACT[0],
            blend: value("post_blend"),
            dwell: value("post_dwell"),
            air: match post_setting(&m.p, "post_air") {
                "M7" => Some((true, false)),
                "M8" => Some((false, true)),
                _ => None,
            },
        })
    }
}
/// Program zero in the setup's plan coordinates (mm).
pub fn datum(setup: &Setup, stock: Bounds, mesh: &Mesh) -> Result<[f32; 3], String> {
    match get(&setup.p, "origin") {
        "Stock top center" => Ok([
            (stock.min[0] + stock.max[0]) / 2.,
            (stock.min[1] + stock.max[1]) / 2.,
            stock.max[2],
        ]),
        "Stock top front-left" => Ok([stock.min[0], stock.min[1], stock.max[2]]),
        "Model origin" => Ok(Placement::new(mesh, Some(setup))
            .transform(mesh.origin)
            .map(|v| v / mesh.display_scale())),
        "Selected point (planned)" => Err("Selected-point work zeros are not implemented. Choose Stock top center, Stock top front-left or Model origin.".into()),
        other => Err(format!("Unsupported work origin \"{other}\".")),
    }
}
pub fn coord(v: f32) -> String {
    let s = format!("{v:.3}");
    if s == "-0.000" { "0.000".into() } else { s }
}
/// Comment text limited to printable ASCII on one line, without parentheses.
pub fn comment(s: &str) -> String {
    let mapped: String = s
        .chars()
        .map(|c| match c {
            '(' | ')' => ' ',
            'Ø' | 'ø' => 'D',
            '·' | '–' | '—' => '-',
            '×' => 'x',
            c if c.is_ascii_graphic() || c == ' ' => c,
            c if c.is_whitespace() => ' ',
            _ => '?',
        })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect()
}
pub fn preview(text: &str, lines: usize) -> String {
    let total = text.lines().count();
    let mut out = text.lines().take(lines).collect::<Vec<_>>().join("\n");
    if total > lines {
        out += &format!("\n(… {} more lines)", total - lines);
    }
    out
}
/// What must be active while one operation runs.
#[derive(Debug, Clone, PartialEq)]
pub struct OpState {
    pub name: String,
    pub strategy: &'static str,
    pub tool: u32,
    pub tool_id: String,
    pub tool_name: String,
    pub diameter: f32,
    pub stickout: f32,
    pub rpm: f32,
    pub clockwise: bool,
    /// (mist, flood)
    pub coolant: (bool, bool),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Rapid,
    Feed,
    Dwell,
}
/// One setup's program in plan coordinates: every motion and state change.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// M9/M5 for whatever is running.
    Stop,
    /// G53 G0 Z0, or G0 Z to the setup clearance plane.
    Retract,
    /// Begin an operation: its comment, and when `change`, T# M6 then G43 H# or MSG + M0.
    Start { op: usize, change: bool },
    /// (Re)issue the spindle (S M3/M4) and coolant for an operation.
    Run { op: usize, spindle: bool, coolant: bool },
    /// Spindle acceleration dwell from the machine settings.
    Wait { seconds: f32 },
    /// Rapid in X/Y at the retract height after a tool change.
    Approach { op: usize, xy: [f32; 2] },
    /// Rapid down from the retract height to the operation's first point.
    Descend { op: usize, xy: [f32; 2], z: f32 },
    Move { op: usize, kind: Kind, from: [f32; 3], to: [f32; 3], feed: f32, seconds: f32 },
    End,
}
pub fn blocks(ops: &[OpState], plans: &[&Plan], settings: &PostSettings) -> Vec<Block> {
    let mut out = vec![];
    for (k, (op, plan)) in ops.iter().zip(plans).enumerate() {
        let first = plan.moves[0].from;
        let previous = k.checked_sub(1);
        let change = previous.is_none_or(|p| ops[p].tool_id != op.tool_id);
        let spindle = change
            || previous.is_some_and(|p| ops[p].rpm != op.rpm || ops[p].clockwise != op.clockwise);
        let coolant = change || previous.is_some_and(|p| ops[p].coolant != op.coolant);
        if change {
            out.extend([Block::Stop, Block::Retract]);
        }
        out.push(Block::Start { op: k, change });
        if change {
            // The new tool's tip (after G43 or a manual touch-off) goes to a safe height first.
            out.push(Block::Retract);
        }
        out.push(Block::Run { op: k, spindle, coolant });
        if spindle && settings.dwell > 0. {
            out.push(Block::Wait { seconds: settings.dwell });
        }
        if change {
            out.push(Block::Approach { op: k, xy: [first[0], first[1]] });
            out.push(Block::Descend { op: k, xy: [first[0], first[1]], z: first[2] });
        } else {
            // Same tool: climb, cross and descend between the two clearance points.
            let from = plans[k - 1].moves.last().unwrap().to;
            let high = from[2].max(first[2]);
            let mut at = from;
            for to in [[from[0], from[1], high], [first[0], first[1], high], first] {
                if to != at {
                    out.push(Block::Move { op: k, kind: Kind::Rapid, from: at, to, feed: 0., seconds: 0. });
                    at = to;
                }
            }
        }
        for m in &plan.moves {
            let kind = match m.motion {
                Motion::Rapid => Kind::Rapid,
                Motion::Feed => Kind::Feed,
                Motion::Dwell => Kind::Dwell,
            };
            out.push(Block::Move {
                op: k,
                kind,
                from: m.from,
                to: m.to,
                feed: if kind == Kind::Feed { m.feed } else { 0. },
                seconds: if kind == Kind::Dwell { m.end - m.start } else { 0. },
            });
        }
    }
    out.extend([Block::Stop, Block::Retract, Block::End]);
    out
}
/// Highest tool-tip Z of the setup's motion (plan coordinates).
pub fn clearance_plane(blocks: &[Block]) -> f32 {
    blocks.iter().fold(f32::NEG_INFINITY, |z, b| match b {
        Block::Move { from, to, .. } => z.max(from[2]).max(to[2]),
        Block::Descend { z: d, .. } => z.max(*d),
        _ => z,
    })
}
/// Tool-tip extents of the setup's motion (plan coordinates).
pub fn extents(blocks: &[Block]) -> Bounds {
    let mut b = Bounds {
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
    };
    let points = blocks.iter().flat_map(|block| match block {
        Block::Move { from, to, .. } => vec![*from, *to],
        Block::Descend { xy, z, .. } => vec![[xy[0], xy[1], *z]],
        _ => vec![],
    });
    for p in points {
        for k in 0..3 {
            b.min[k] = b.min[k].min(p[k]);
            b.max[k] = b.max[k].max(p[k]);
        }
    }
    b
}
pub struct Header<'a> {
    pub job: &'a str,
    pub setup: &'a Setup,
    pub number: usize,
    /// (program, count) when touch-off mode splits a setup into one program per tool run.
    pub part: Option<(usize, usize)>,
    pub stock: Bounds,
    pub datum: [f32; 3],
    pub disabled: Vec<String>,
}
struct Writer(String);
impl Writer {
    fn line(&mut self, s: impl AsRef<str>) {
        self.0.push_str(s.as_ref());
        self.0.push('\n');
    }
    fn note(&mut self, s: &str) {
        self.line(format!("({})", comment(s)));
    }
}
pub fn emit(h: &Header, ops: &[OpState], blocks: &[Block], settings: &PostSettings) -> String {
    let (d, setup) = (h.datum, h.setup);
    let offset = get(&setup.p, "offset");
    let mut w = Writer(String::new());
    w.line("(AXIS CAM Studio - LinuxCNC 3-axis program)");
    w.note(&format!("Job: {} - Setup {}: {} - {offset}", h.job, h.number, setup.name()));
    if let Some((i, count)) = h.part {
        w.note(&format!("Program {i} of {count} for this setup, one per tool"));
    }
    w.note(&format!("Machine: {} v{}", setup.machine.name(), setup.machine.version));
    w.note(&format!("Work zero: {}. Touch off X0 Y0 Z0 there.", get(&setup.p, "origin")));
    let size = h.stock.dimensions();
    w.note(&format!(
        "Stock: {} x {} x {} mm. Part rotation X{} Y{} Z{} deg.",
        coord(size[0]),
        coord(size[1]),
        coord(size[2]),
        get(&setup.p, "rx"),
        get(&setup.p, "ry"),
        get(&setup.p, "rz")
    ));
    w.note(&format!("Fixture: {}", get(&setup.p, "fixture")));
    let mut listed = vec![];
    for op in ops {
        if !listed.contains(&op.tool) {
            listed.push(op.tool);
            w.note(&format!(
                "T{}: {} - diameter {} mm - stickout {} mm",
                op.tool,
                op.tool_name,
                coord(op.diameter),
                coord(op.stickout)
            ));
        }
    }
    for (k, op) in ops.iter().enumerate() {
        w.note(&format!("Operation {}: {} - {} - T{} - S{:.0}", k + 1, op.name, op.strategy, op.tool, op.rpm));
    }
    for name in &h.disabled {
        w.note(&format!("Suppressed: {name}"));
    }
    let e = extents(blocks);
    w.line(format!(
        "(Extents: X{} to {} Y{} to {} Z{} to {} mm)",
        coord(e.min[0] - d[0]),
        coord(e.max[0] - d[0]),
        coord(e.min[1] - d[1]),
        coord(e.max[1] - d[1]),
        coord(e.min[2] - d[2]),
        coord(e.max[2] - d[2])
    ));
    w.line("(Not checked: fixtures, clamps and the machine body. LinuxCNC soft limits check absolute position. Preview in LinuxCNC and air-cut first.)");
    w.line("G17 G21 G40 G49 G80 G90 G94");
    w.line(if settings.blend > 0. { format!("G64 P{:.3}", settings.blend) } else { "G61".into() });
    w.line(offset);
    if settings.tool_table && !settings.machine_home {
        // The first clearance retract must position the loaded tool's tip, not the gauge line.
        w.line("G43");
    }
    let clearance = coord(clearance_plane(blocks) - d[2]);
    let mut position: [Option<String>; 3] = [None, None, None];
    let mut motion: Option<Kind> = None;
    let mut feed: Option<String> = None;
    let mut spindle_on = false;
    let mut coolant = (false, false);
    for block in blocks {
        match block {
            Block::Stop => {
                if coolant != (false, false) {
                    w.line("M9");
                    coolant = (false, false);
                }
                if spindle_on {
                    w.line("M5");
                    spindle_on = false;
                }
            }
            Block::Retract => {
                if settings.machine_home {
                    w.line("G53 G0 Z0");
                    position[2] = None;
                } else {
                    w.line(format!("G0 Z{clearance}"));
                    position[2] = Some(clearance.clone());
                }
                motion = Some(Kind::Rapid);
            }
            Block::Start { op, change } => {
                let s = &ops[*op];
                w.note(&format!("Operation {}/{}: {} - T{}", op + 1, ops.len(), s.name, s.tool));
                if *change && settings.tool_table {
                    w.line(format!("T{} M6", s.tool));
                    w.line(format!("G43 H{}", s.tool));
                } else if *change {
                    // AXIS cannot touch off while a program is paused, so each tool gets its own program.
                    w.note(&format!("Load T{} {} and touch off Z at the work zero before starting this program.", s.tool, s.tool_name));
                }
            }
            Block::Run { op, spindle, coolant: set } => {
                let s = &ops[*op];
                if *spindle {
                    w.line(format!("S{:.0} {}", s.rpm, if s.clockwise { "M3" } else { "M4" }));
                    spindle_on = true;
                }
                if *set && s.coolant != coolant {
                    if coolant != (false, false) {
                        w.line("M9");
                    }
                    if s.coolant.0 {
                        w.line("M7");
                    }
                    if s.coolant.1 {
                        w.line("M8");
                    }
                    coolant = s.coolant;
                }
            }
            Block::Wait { seconds } => w.line(format!("G4 P{seconds:.3}")),
            Block::Approach { xy, .. } => {
                let (x, y) = (coord(xy[0] - d[0]), coord(xy[1] - d[1]));
                w.line(format!("G0 X{x} Y{y}"));
                position[0] = Some(x);
                position[1] = Some(y);
                motion = Some(Kind::Rapid);
            }
            Block::Descend { z, .. } => {
                let z = coord(*z - d[2]);
                w.line(format!("G0 Z{z}"));
                position[2] = Some(z);
                motion = Some(Kind::Rapid);
            }
            Block::Move { kind: Kind::Dwell, seconds, .. } => w.line(format!("G4 P{seconds:.3}")),
            Block::Move { kind, to, feed: rate, .. } => {
                let mut words = String::new();
                for (k, axis) in ["X", "Y", "Z"].iter().enumerate() {
                    let v = coord(to[k] - d[k]);
                    if position[k].as_deref() != Some(v.as_str()) {
                        words += &format!(" {axis}{v}");
                        position[k] = Some(v);
                    }
                }
                if words.is_empty() {
                    continue;
                }
                let mut line = String::new();
                if motion != Some(*kind) {
                    line += if *kind == Kind::Rapid { "G0" } else { "G1" };
                    motion = Some(*kind);
                }
                line += &words;
                if *kind == Kind::Feed {
                    let f = format!("{rate:.1}");
                    if feed.as_deref() != Some(f.as_str()) {
                        line += &format!(" F{f}");
                        feed = Some(f);
                    }
                }
                w.line(line.trim_start());
            }
            Block::End => w.line("M2"),
        }
    }
    w.0
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --locked --quiet 2>&1 | tail -3`
Expected: `135 passed; 0 failed` (including one new `ngc` test from the Task 5 fix round). If `golden_facing_program` fails, print both strings and fix the emitter until the golden text matches. Do not edit `GOLDEN` unless the spec requires it.

- [ ] **Step 5: Commit**

```bash
cargo fmt && git add -A && git commit -qm "Emit LinuxCNC programs from simulated moves"
```

---

### Task 6: Pre-export checks (`post_check.rs`)

**Files:**
- Create: `src/post_check.rs`
- Modify: `src/main.rs` (add `mod post_check;` after `mod post;`)

**Interfaces:**
- Consumes: `post::{Block, Kind, OpState, PostSettings, clearance_plane, extents, fixture}`, `rest::RestStock::{new, top}`, `simulation::Simulation { plan, initial_volume, frame }`.
- Produces: `pub fn machine_mm(m: &Machine, key: &str) -> f32` and `pub fn check_setup(setup: &Setup, ops: &[OpState], sims: &[&Simulation], blocks: &[Block], settings: &PostSettings, datum: [f32; 3]) -> Vec<String>`. Returns an empty list when the setup is safe to emit.

- [ ] **Step 1: Write the failing tests** (create `src/post_check.rs` with this test module. Add `mod post_check;` to `src/main.rs`.)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        post::{OpState, PostSettings, blocks, fixture},
        simulation::Motion,
    };
    fn check(machine: Machine, op: OpState, moves: &[(Motion, [f32; 3], f32)]) -> Vec<String> {
        let plan = fixture::plan([-15., -15., 25.], moves);
        let settings = PostSettings::from_machine(&machine).unwrap();
        let blocks = blocks(&[op.clone()], &[&plan], &settings);
        let setup = Setup::new(machine, "G54");
        let sim = Simulation::new(plan);
        check_setup(&setup, &[op], &[&sim], &blocks, &settings, [0., 0., 10.])
    }
    const FACING: [(Motion, [f32; 3], f32); 4] = [
        (Motion::Rapid, [-15., -15., 11.], 6000.),
        (Motion::Feed, [-15., -15., 9.], 250.),
        (Motion::Feed, [15., -15., 9.], 1000.),
        (Motion::Rapid, [15., -15., 25.], 6000.),
    ];
    #[test]
    fn a_normal_facing_pass_is_accepted() {
        assert_eq!(check(Machine::new(3), fixture::facing_op(), &FACING), Vec::<String>::new());
    }
    #[test]
    fn retracts_and_peck_re_entry_at_the_same_xy_are_accepted() {
        let pecks = [
            (Motion::Rapid, [-15., -15., 11.], 6000.),
            (Motion::Feed, [-15., -15., 4.], 100.),
            (Motion::Rapid, [-15., -15., 11.], 6000.),
            (Motion::Rapid, [-15., -15., 4.1], 6000.),
            (Motion::Feed, [-15., -15., 2.], 100.),
            (Motion::Rapid, [-15., -15., 25.], 6000.),
        ];
        let problems = check(Machine::new(3), fixture::facing_op(), &pecks);
        assert!(problems.is_empty(), "{problems:?}");
    }
    #[test]
    fn rapids_into_unproven_material_or_sideways_below_the_top_are_refused() {
        let down = [
            (Motion::Rapid, [-15., -15., 5.], 6000.),
            (Motion::Feed, [-15., -15., 4.], 100.),
            (Motion::Rapid, [-15., -15., 25.], 6000.),
        ];
        let problems = check(Machine::new(3), fixture::facing_op(), &down);
        assert!(problems.iter().any(|p| p.contains("not proven removed")), "{problems:?}");
        let sideways = [
            (Motion::Rapid, [-15., -15., 11.], 6000.),
            (Motion::Feed, [-15., -15., 5.], 100.),
            (Motion::Rapid, [0., -15., 5.], 6000.),
            (Motion::Rapid, [0., -15., 25.], 6000.),
        ];
        let problems = check(Machine::new(3), fixture::facing_op(), &sideways);
        assert!(problems.iter().any(|p| p.contains("sideways")), "{problems:?}");
    }
    #[test]
    fn holder_feed_rpm_and_travel_limits_are_enforced() {
        let short = OpState { stickout: 0.5, ..fixture::facing_op() };
        assert!(check(Machine::new(3), short, &FACING).iter().any(|p| p.contains("holder")));
        let mut slow = Machine::new(3);
        slow.p.insert("feed".into(), "500".into());
        assert!(check(slow, fixture::facing_op(), &FACING).iter().any(|p| p.contains("F1000.0")));
        let fast = OpState { rpm: 20000., ..fixture::facing_op() };
        assert!(check(Machine::new(3), fast, &FACING).iter().any(|p| p.contains("S20000")));
        let mut small = Machine::new(3);
        small.p.insert("x".into(), "10".into());
        assert!(check(small, fixture::facing_op(), &FACING).iter().any(|p| p.contains("X travel")));
        let mut inch = Machine::new(3);
        inch.p.insert("units".into(), "inch".into());
        inch.p.insert("x".into(), "1".into());
        assert!(check(inch, fixture::facing_op(), &FACING).iter().any(|p| p.contains("25.400 mm")));
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --locked --quiet post_check:: 2>&1 | tail -5`
Expected: compile error (`check_setup` not found).

- [ ] **Step 3: Implement** (insert above the test module)

```rust
//! Checks that must pass before G-code is written. Positions are plan
//! coordinates (setup frame, mm). The tool is modelled as the simulation
//! models it: a cylinder of cutter diameter up to its stickout, then the holder.
use crate::{
    mesh::Bounds,
    model::{Machine, Setup, get, num},
    post::{Block, Kind, OpState, PostSettings, clearance_plane, extents},
    rest::RestStock,
    simulation::Simulation,
};
use std::collections::{HashMap, HashSet};

const EPS: f32 = 0.0005;
/// Machine-profile travel or feed in mm (or mm/min).
pub fn machine_mm(m: &Machine, key: &str) -> f32 {
    let v = num(&m.p, key).unwrap_or(0.);
    if get(&m.p, "units") == "inch" { v * 25.4 } else { v }
}
#[derive(Default)]
struct Problems {
    seen: HashSet<(Option<usize>, &'static str)>,
    list: Vec<String>,
}
impl Problems {
    /// Keep the first message for each operation and rule.
    fn add(&mut self, op: Option<usize>, rule: &'static str, message: String) {
        if self.seen.insert((op, rule)) {
            self.list.push(message);
        }
    }
}
fn key(x: f32, y: f32) -> (i64, i64) {
    ((x * 1000.).round() as i64, (y * 1000.).round() as i64)
}
pub fn check_setup(
    setup: &Setup,
    ops: &[OpState],
    sims: &[&Simulation],
    blocks: &[Block],
    settings: &PostSettings,
    datum: [f32; 3],
) -> Vec<String> {
    let mut problems = Problems::default();
    let machine = &setup.machine;
    let stock = sims[0].plan.stock;
    if sims.iter().any(|s| s.plan.stock != stock) {
        problems.add(None, "stock", "Operations in this setup report different stock boxes, so the work zero is ambiguous.".into());
    }
    let top = stock.max[2];
    let max_rpm = num(&machine.p, "rpm").unwrap_or(0.);
    let max_feed = machine_mm(machine, "feed");
    for (k, op) in ops.iter().enumerate() {
        if !(op.rpm > 0. && op.rpm <= max_rpm) {
            problems.add(Some(k), "rpm", format!("{}: S{:.0} is outside the machine's 1-{max_rpm:.0} rpm range.", op.name, op.rpm));
        }
    }
    let span = extents(blocks);
    for (k, (axis, rule)) in [("X", "travel X"), ("Y", "travel Y"), ("Z", "travel Z")].into_iter().enumerate() {
        let travel = machine_mm(machine, &axis.to_lowercase());
        let size = span.max[k] - span.min[k];
        if size > travel + 0.001 {
            problems.add(None, rule, format!("The program spans {size:.3} mm in {axis}, more than the machine's {axis} travel of {travel:.3} mm."));
        }
    }
    if !settings.machine_home && clearance_plane(blocks) < top - EPS {
        problems.add(None, "retract", "The setup clearance plane is below the stock top, so tool changes would retract into material. Raise the operations' clearance heights.".into());
    }
    let rests: Vec<Option<RestStock>> = sims
        .iter()
        .map(|s| s.initial_volume.as_ref().zip(s.frame).map(|(v, f)| RestStock::new(v, f)))
        .collect();
    let at = |p: [f32; 3]| format!("X{:.3} Y{:.3} Z{:.3}", p[0] - datum[0], p[1] - datum[1], p[2] - datum[2]);
    // Lowest tip Z reached at each XY by the loaded tool since its tool change.
    let mut visited: HashMap<(i64, i64), f32> = HashMap::new();
    for block in blocks {
        let (op, from, to, rapid) = match block {
            Block::Start { change: true, .. } => {
                visited.clear();
                continue;
            }
            Block::Descend { op, xy, z } => (*op, [xy[0], xy[1], f32::INFINITY], [xy[0], xy[1], *z], true),
            Block::Move { op, kind, from, to, feed, .. } => {
                if *kind == Kind::Feed && *feed > max_feed + 0.001 {
                    problems.add(Some(*op), "feed", format!("{}: F{feed:.1} exceeds the machine's {max_feed:.1} mm/min maximum feed.", ops[*op].name));
                }
                (*op, *from, *to, *kind == Kind::Rapid)
            }
            _ => continue,
        };
        let o = &ops[op];
        let low = from[2].min(to[2]);
        if low + o.stickout < top - EPS {
            problems.add(Some(op), "holder", format!("{}: the holder would drop below the stock top at {} (stickout {:.3} mm). Use a longer stickout or a shallower cut.", o.name, at([to[0], to[1], low]), o.stickout));
        }
        if rapid && low < top - EPS {
            let vertical = (from[0] - to[0]).abs() <= EPS && (from[1] - to[1]).abs() <= EPS;
            let revisited = visited.get(&key(to[0], to[1])).is_some_and(|z| *z <= low + EPS);
            let proven = || {
                rests[op].as_ref().is_some_and(|r| {
                    let radius = o.diameter / 2.;
                    let column = Bounds {
                        min: [to[0] - radius, to[1] - radius, low],
                        max: [to[0] + radius, to[1] + radius, top],
                    };
                    r.top(column, low - 0.001, top) <= low + EPS
                })
            };
            if !vertical {
                problems.add(Some(op), "rapid", format!("{}: a rapid moves sideways below the stock top at {}.", o.name, at(to)));
            } else if !revisited && !proven() {
                problems.add(Some(op), "rapid", format!("{}: a rapid descends to {} through material that is not proven removed.", o.name, at([to[0], to[1], low])));
            }
        }
        for p in [from, to] {
            if p[2].is_finite() {
                let z = visited.entry(key(p[0], p[1])).or_insert(p[2]);
                *z = z.min(p[2]);
            }
        }
    }
    problems.list
}
```
- [ ] **Step 4: Run the tests**

Run: `cargo test --locked --quiet 2>&1 | tail -3`
Expected: `140 passed; 0 failed` (includes the touch-off single-tool test added by ruling).

- [ ] **Step 5: Commit**

```bash
cargo fmt && git add -A && git commit -qm "Check limits, travel, rapids and holder clearance before export"
```

---

### Task 7: Job posting, round-trip verification, writing files and CLI

**Files:**
- Modify: `src/ngc.rs` (add `verify`)
- Modify: `src/post.rs` (add `Program`, `SetupPost`, `JobPost`, `file_name`, `op_state`, `Prepared`, `prepare`, `post_job`, `write_job`)
- Modify: `src/main.rs` (`--export-gcode` in `main`)

**Interfaces:**
- Consumes: `ngc::replay`, `post_check::check_setup`, `simulation::sequence`.
- Produces:
  - `ngc::verify(text: &str, blocks: &[post::Block], ops: &[post::OpState], datum: [f32; 3], settings: &post::PostSettings, offset: &str) -> Result<(), String>`
  - `post::Program { pub file_name: String, pub text: String, pub lines: usize, pub tools: Vec<String>, pub operations: Vec<String> }`
  - `post::SetupPost { pub setup: usize, pub name: String, pub result: Result<Vec<Program>, Vec<String>> }` (one program per setup; one per tool run in touch-off mode)
  - `post::JobPost { pub errors: Vec<String>, pub setups: Vec<SetupPost> }` with `ready(&self) -> bool` and `blocked(message: &str) -> Self`
  - `post::post_job(doc: &Document, mesh: &Mesh) -> JobPost`
  - `post::write_job(job: &JobPost, folder: &Path) -> Result<(PathBuf, usize), String>`

- [ ] **Step 1: Write the failing tests** (append inside `mod tests` in `src/post.rs`)

```rust
    fn errors(job: &JobPost) -> Vec<String> {
        job.errors
            .iter()
            .cloned()
            .chain(job.setups.iter().filter_map(|s| s.result.as_ref().err()).flatten().cloned())
            .collect()
    }
    fn prepared(doc: &Document, mesh: &Mesh, si: usize) -> Prepared {
        let mut results: Vec<Vec<(usize, Result<crate::simulation::Simulation, String>)>> =
            vec![vec![]; doc.setups.len()];
        crate::simulation::sequence(doc, mesh, None, |s, o, r| results[s].push((o, r)));
        prepare(doc, mesh, si, &results[si]).unwrap().remove(0)
    }
    #[test]
    fn all_six_strategies_post_and_replay_exactly() {
        let (doc, mesh) = fixture::job();
        let job = post_job(&doc, &mesh);
        assert!(job.ready(), "{:?}", errors(&job));
        let programs = job.setups[0].result.as_ref().unwrap();
        assert_eq!(programs.len(), 1);
        let program = &programs[0];
        assert_eq!(program.file_name, "Fixture block - 1 Top (G54).ngc");
        for needle in ["T1 M6", "T3 M6", "T2 M6", "\nG54\n", "G43 H3", "\nM2\n"] {
            assert!(program.text.contains(needle), "{needle}");
        }
        assert_eq!(program.operations.len(), 6);
        assert!(program.lines > 1000);
    }
    #[test]
    fn corrupted_programs_fail_verification() {
        let (doc, mesh) = fixture::job();
        let p = prepared(&doc, &mesh, 0);
        let check = |text: &str| crate::ngc::verify(text, &p.blocks, &p.ops, p.datum, &p.settings, &p.offset);
        assert_eq!(check(&p.text), Ok(()));
        let lines: Vec<&str> = p.text.lines().collect();
        let first_feed = lines.iter().position(|l| l.starts_with("G1 ")).unwrap();
        let with = |i: usize, replacement: &str| {
            let mut l = lines.clone();
            l[i] = replacement;
            l.join("\n") + "\n"
        };
        assert!(check(&with(first_feed, "G1 Z-50.000 F250.0")).is_err());
        assert!(check(&with(first_feed, lines[first_feed].split(" F").next().unwrap())).is_err());
        assert!(check(&p.text.replace("T3 M6", "T4 M6")).is_err());
        assert!(check(&p.text.replacen("G54\n", "G54\nG2 X0 Y0\n", 1)).is_err());
        let cut = lines.iter().rposition(|l| l.starts_with('X') || l.starts_with('Y')).unwrap();
        let mut dropped = lines.clone();
        dropped.remove(cut);
        assert!(check(&(dropped.join("\n") + "\n")).is_err());
    }
    #[test]
    fn golden_programs_verify_in_every_mode() {
        for (key, value) in [
            ("post_tool_length", POST_TOOL_LENGTH[0]),
            ("post_tool_length", POST_TOOL_LENGTH[1]),
            ("post_retract", POST_RETRACT[1]),
            ("post_blend", "0"),
            ("post_dwell", "0"),
        ] {
            let mut m = Machine::new(3);
            m.p.insert(key.into(), value.into());
            let (text, blocks, settings) = program(m);
            let ops = vec![fixture::facing_op()];
            assert_eq!(crate::ngc::verify(&text, &blocks, &ops, [0., 0., 10.], &settings, "G54"), Ok(()), "{key}");
        }
    }
    #[test]
    fn a_flipped_setup_gets_its_own_program_and_offset() {
        let (doc, mesh) = fixture::flipped_job();
        let job = post_job(&doc, &mesh);
        assert!(job.ready(), "{:?}", errors(&job));
        let names: Vec<_> = job
            .setups
            .iter()
            .map(|s| s.result.as_ref().unwrap()[0].file_name.as_str())
            .collect();
        assert_eq!(names, vec!["Fixture block - 1 Top (G54).ngc", "Fixture block - 2 Flipped (G55).ngc"]);
        let second = &job.setups[1].result.as_ref().unwrap()[0].text;
        assert!(second.contains("\nG55\n") && !second.contains("\nG54\n"));
        assert!(second.contains("Part rotation X180"));
    }
    #[test]
    fn touch_off_mode_writes_one_program_per_tool_run() {
        let (mut doc, mesh) = fixture::job();
        doc.setups[0]
            .machine
            .p
            .insert("post_tool_length".into(), POST_TOOL_LENGTH[1].into());
        let job = post_job(&doc, &mesh);
        assert!(job.ready(), "{:?}", errors(&job));
        let programs = job.setups[0].result.as_ref().unwrap();
        let names: Vec<_> = programs.iter().map(|p| p.file_name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Fixture block - 1 Top (G54) - 1 of 4 T1.ngc",
                "Fixture block - 1 Top (G54) - 2 of 4 T3.ngc",
                "Fixture block - 1 Top (G54) - 3 of 4 T2.ngc",
                "Fixture block - 1 Top (G54) - 4 of 4 T1.ngc",
            ]
        );
        assert_eq!(programs[0].operations.len(), 3);
        for p in programs {
            assert!(!p.text.contains("M6") && !p.text.contains("G43") && p.text.contains("touch off Z"));
        }
    }
    #[test]
    fn unsafe_or_unsupported_jobs_are_refused_with_reasons() {
        let refused = |edit: &dyn Fn(&mut Document), needle: &str| {
            let (mut doc, mesh) = fixture::job();
            edit(&mut doc);
            let job = post_job(&doc, &mesh);
            assert!(!job.ready());
            assert!(errors(&job).iter().any(|e| e.contains(needle)), "{needle}: {:?}", errors(&job));
        };
        refused(&|d| { d.setups[0].machine.p.insert("controller".into(), "GRBL".into()); }, "Only the LinuxCNC");
        refused(&|d| d.setups[0].machine = Machine::new(4), "3-axis");
        refused(&|d| { d.setups[0].machine.p.insert("x".into(), "50".into()); }, "travel");
        refused(&|d| { d.tools[1].p.insert("number".into(), "1".into()); }, "also used");
        refused(&|d| { d.setups[0].p.insert("origin".into(), "Selected point (planned)".into()); }, "Selected-point");
        refused(&|d| { d.setups[0].p.insert("offset".into(), "G92".into()); }, "G54-G59.3");
        refused(&|d| {
            d.setups[0].machine.p.insert("post_air".into(), "Not available".into());
            d.setups[0].operations[0].p.insert("coolant".into(), "Air".into());
        }, "coolant");
        refused(&|d| d.setups[0].operations[1].surfaces = None, "Cannot inherit");
        refused(&|d| d.setups[0].operations.iter_mut().for_each(|o| o.enabled = false), "no enabled operations");
        let sample = post_job(&Document::sample(), &crate::mesh::sample());
        assert!(!sample.ready() && sample.errors[0].contains("Import a part"));
    }
    #[test]
    fn write_job_writes_every_program_or_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (doc, mesh) = fixture::flipped_job();
        let (folder, count) = write_job(&post_job(&doc, &mesh), dir.path()).unwrap();
        assert_eq!((folder.as_path(), count), (dir.path(), 2));
        let written = std::fs::read_to_string(dir.path().join("Fixture block - 2 Flipped (G55).ngc")).unwrap();
        assert!(written.ends_with("M2\n"));
        let empty = tempfile::tempdir().unwrap();
        let (mut bad, mesh) = fixture::flipped_job();
        bad.setups[1].p.insert("origin".into(), "Selected point (planned)".into());
        assert!(write_job(&post_job(&bad, &mesh), empty.path()).unwrap_err().contains("Nothing was written"));
        assert_eq!(std::fs::read_dir(empty.path()).unwrap().count(), 0);
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --locked --quiet post:: 2>&1 | tail -5`
Expected: compile errors (`post_job`, `prepare`, `write_job` … not found).

- [ ] **Step 3: Implement `ngc::verify`** (append above the test module in `src/ngc.rs`)

```rust
use crate::post::{Block, Kind, OpState, PostSettings};
const TOL: f64 = 0.0006;
fn take<'a>(steps: &'a [Step], i: &mut usize, what: &str) -> Result<&'a Step, String> {
    let s = steps.get(*i).ok_or_else(|| format!("the program ends before {what}"))?;
    *i += 1;
    Ok(s)
}
fn near(a: Option<f64>, b: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() <= TOL)
}
/// State that must be active whenever an operation moves.
fn active(s: &Step, op: &OpState, settings: &PostSettings, offset: &str) -> Result<(), String> {
    let st = &s.state;
    let spindle = if op.clockwise { Spindle::Cw } else { Spindle::Ccw };
    let blend = match st.blend {
        Blend::Exact => settings.blend == 0.,
        Blend::Tolerance(p) => settings.blend > 0. && (p - f64::from(settings.blend)).abs() <= TOL,
        Blend::Unset => false,
    };
    let problem = if settings.tool_table && st.tool != Some(op.tool) {
        format!("T{} is not loaded", op.tool)
    } else if settings.tool_table && st.length != Some(op.tool) {
        format!("G43 H{} is not active", op.tool)
    } else if !settings.tool_table && (st.tool.is_some() || st.length.is_some()) {
        "a touch-off program must not change tools or apply G43".into()
    } else if st.spindle != spindle || (st.rpm - f64::from(op.rpm.round())).abs() > 0.5 {
        format!("the spindle is not running {} at S{:.0}", if op.clockwise { "M3" } else { "M4" }, op.rpm)
    } else if (st.mist, st.flood) != op.coolant {
        "the coolant state is wrong".into()
    } else if st.offset.as_deref() != Some(offset) {
        format!("{offset} is not active")
    } else if !blend {
        "the path blending mode is wrong".into()
    } else {
        return Ok(());
    };
    Err(format!("line {}: {problem} for {}", s.line, op.name))
}
/// Replay `text` and require exactly the motion in `blocks` (plan coordinates).
pub fn verify(
    text: &str,
    blocks: &[Block],
    ops: &[OpState],
    datum: [f32; 3],
    settings: &PostSettings,
    offset: &str,
) -> Result<(), String> {
    let r = replay(text)?;
    if !r.ended {
        return Err("the program has no M2 end".into());
    }
    let d = datum.map(f64::from);
    let work = |p: [f32; 3]| -> [f64; 3] { std::array::from_fn(|k| f64::from(p[k]) - d[k]) };
    let clearance = f64::from(crate::post::clearance_plane(blocks)) - d[2];
    let steps = &r.steps;
    let mut i = 0;
    let mut at: [Option<f64>; 3] = [None; 3];
    for block in blocks {
        match block {
            Block::Stop | Block::Start { .. } | Block::Run { .. } | Block::End => {}
            Block::Retract => {
                let s = take(steps, &mut i, "a safe retract")?;
                let ok = s.kind == StepKind::Rapid
                    && if settings.machine_home {
                        s.machine && s.to == [None, None, Some(0.)]
                    } else {
                        !s.machine && near(s.to[2], clearance) && s.to[..2] == s.from[..2]
                    };
                if !ok {
                    return Err(format!("line {}: expected the safe retract", s.line));
                }
                if s.machine {
                    at[2] = None;
                } else {
                    at = s.to;
                }
            }
            Block::Wait { seconds } => {
                let s = take(steps, &mut i, "the spindle dwell")?;
                if s.kind != StepKind::Dwell || (s.seconds - f64::from(*seconds)).abs() > TOL {
                    return Err(format!("line {}: expected a {seconds:.3} s spindle dwell", s.line));
                }
            }
            Block::Approach { op, xy } => {
                let s = take(steps, &mut i, "the approach")?;
                let w = work([xy[0], xy[1], 0.]);
                if s.machine || s.kind != StepKind::Rapid || !near(s.to[0], w[0]) || !near(s.to[1], w[1]) || s.to[2] != s.from[2] {
                    return Err(format!("line {}: expected the rapid approach in X/Y", s.line));
                }
                active(s, &ops[*op], settings, offset)?;
                at = s.to;
            }
            Block::Descend { op, xy, z } => {
                let s = take(steps, &mut i, "the descent")?;
                let w = work([xy[0], xy[1], *z]);
                if s.machine || s.kind != StepKind::Rapid || (0..3).any(|k| !near(s.to[k], w[k])) || s.to[..2] != s.from[..2] {
                    return Err(format!("line {}: expected the rapid descent to the first point", s.line));
                }
                active(s, &ops[*op], settings, offset)?;
                at = s.to;
            }
            Block::Move { op, kind: Kind::Dwell, seconds, .. } => {
                let s = take(steps, &mut i, "a dwell")?;
                if s.kind != StepKind::Dwell || (s.seconds - f64::from(*seconds)).abs() > TOL {
                    return Err(format!("line {}: expected a {seconds:.3} s dwell", s.line));
                }
                active(s, &ops[*op], settings, offset)?;
            }
            Block::Move { op, kind, to, feed, .. } => {
                let w = work(*to);
                let expected = if *kind == Kind::Rapid { StepKind::Rapid } else { StepKind::Feed };
                let found = steps.get(i).filter(|s| {
                    !s.machine && s.kind == expected && (0..3).all(|k| near(s.to[k], w[k]))
                });
                if let Some(s) = found {
                    i += 1;
                    if expected == StepKind::Feed && (s.state.feed - f64::from(*feed)).abs() > 0.051 {
                        return Err(format!("line {}: F{:.1} does not match the simulated {feed:.1} mm/min", s.line, s.state.feed));
                    }
                    active(s, &ops[*op], settings, offset)?;
                    at = s.to;
                } else if !(0..3).all(|k| near(at[k], w[k])) {
                    // Only a move shorter than the output resolution may be absent.
                    let line = steps.get(i).map_or(0, |s| s.line);
                    return Err(format!("line {line}: expected a {kind:?} move to X{:.3} Y{:.3} Z{:.3}", w[0], w[1], w[2]));
                }
            }
        }
    }
    if let Some(s) = steps.get(i) {
        return Err(format!("line {}: unexpected extra motion", s.line));
    }
    if r.end_state.spindle != Spindle::Off || r.end_state.mist || r.end_state.flood {
        return Err("the spindle or coolant is still on at M2".into());
    }
    Ok(())
}
```

- [ ] **Step 4: Implement job posting in `src/post.rs`** (add below `emit`. Also add `simulation::Simulation` to the `use crate::{..}` list.)

```rust
#[derive(Debug, Clone)]
pub struct Program {
    pub file_name: String,
    pub text: String,
    pub lines: usize,
    pub tools: Vec<String>,
    pub operations: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct SetupPost {
    pub setup: usize,
    pub name: String,
    /// One program per setup, or one per tool run in touch-off mode.
    pub result: Result<Vec<Program>, Vec<String>>,
}
#[derive(Debug, Clone, Default)]
pub struct JobPost {
    pub errors: Vec<String>,
    pub setups: Vec<SetupPost>,
}
impl JobPost {
    pub fn blocked(message: &str) -> Self {
        Self {
            errors: vec![message.into()],
            setups: vec![],
        }
    }
    pub fn ready(&self) -> bool {
        self.errors.is_empty() && !self.setups.is_empty() && self.setups.iter().all(|s| s.result.is_ok())
    }
}
pub fn file_name(doc: &Document, index: usize, part: Option<(usize, usize, u32)>) -> String {
    let clean = |s: &str| -> String {
        let s: String = s
            .chars()
            .map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '_' } else { c })
            .collect();
        s.trim().chars().take(60).collect()
    };
    let setup = &doc.setups[index];
    let job = clean(&doc.title);
    let base = format!(
        "{} - {} {} ({})",
        if job.is_empty() { "AXIS job".into() } else { job },
        index + 1,
        clean(setup.name()),
        clean(get(&setup.p, "offset"))
    );
    match part {
        Some((i, count, tool)) => format!("{base} - {i} of {count} T{tool}.ngc"),
        None => format!("{base}.ngc"),
    }
}
fn op_state(doc: &Document, op: &Operation, plan: &Plan, settings: Option<&PostSettings>) -> Result<OpState, Vec<String>> {
    let Some(tool) = doc.tools.iter().find(|t| t.id == op.tool) else {
        return Err(vec![format!("{}: choose a saved tool.", op.name())]);
    };
    let mut e: Vec<String> = doc
        .tool_number_issues(tool)
        .into_iter()
        .map(|x| format!("{}: {x}", op.name()))
        .collect();
    let coolant = match get(&op.p, "coolant") {
        "Flood" => Some((false, true)),
        "Mist" => Some((true, false)),
        "Off" => Some((false, false)),
        "Air" => settings.and_then(|s| s.air),
        _ => None,
    };
    if coolant.is_none() {
        e.push(format!(
            "{}: coolant \"{}\" has no LinuxCNC output on this machine.",
            op.name(),
            get(&op.p, "coolant")
        ));
    }
    if !e.is_empty() {
        return Err(e);
    }
    Ok(OpState {
        name: op.name().into(),
        strategy: op.strategy.name(),
        tool: tool.number().unwrap(),
        tool_id: tool.id.clone(),
        tool_name: tool.name().into(),
        diameter: plan.radius * 2.,
        stickout: plan.tool_length,
        rpm: num(&op.p, "rpm").unwrap_or(0.),
        clockwise: plan.spindle_clockwise,
        coolant: coolant.unwrap(),
    })
}
pub(crate) struct Prepared {
    pub file_name: String,
    pub text: String,
    pub blocks: Vec<Block>,
    pub ops: Vec<OpState>,
    pub datum: [f32; 3],
    pub settings: PostSettings,
    pub offset: String,
}
/// Check and emit one setup from its `sequence` results (enabled operations only):
/// one program, or one per run of the same tool in touch-off mode.
pub(crate) fn prepare(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    sims: &[(usize, Result<Simulation, String>)],
) -> Result<Vec<Prepared>, Vec<String>> {
    let setup = &doc.setups[si];
    let offset = get(&setup.p, "offset").to_string();
    let mut errors = vec![];
    let settings = PostSettings::from_machine(&setup.machine)
        .map_err(|e| errors.extend(e))
        .ok();
    if !OFFSETS.contains(&offset.as_str()) {
        errors.push(format!("Work offset \"{offset}\" is not a LinuxCNC register (G54-G59.3)."));
    }
    if sims.is_empty() {
        errors.push("This setup has no enabled operations. Add one or remove the setup.".into());
    }
    let mut ops = vec![];
    let mut simulations = vec![];
    for (oi, result) in sims {
        let op = &setup.operations[*oi];
        match result {
            Err(e) => errors.push(format!("{}: {e}", op.name())),
            Ok(sim) => match op_state(doc, op, &sim.plan, settings.as_ref()) {
                Ok(state) => {
                    ops.push(state);
                    simulations.push(sim);
                }
                Err(e) => errors.extend(e),
            },
        }
    }
    let zero = simulations
        .first()
        .and_then(|sim| datum(setup, sim.plan.stock, mesh).map_err(|e| errors.push(e)).ok());
    let (Some(settings), Some(zero), true) = (settings, zero, errors.is_empty()) else {
        return Err(errors);
    };
    // Touch-off mode: one program per run of the same tool, touched off before it starts.
    let mut groups: Vec<std::ops::Range<usize>> = vec![];
    for k in 0..ops.len() {
        match groups.last_mut() {
            Some(g) if settings.tool_table || ops[g.start].tool_id == ops[k].tool_id => g.end = k + 1,
            _ => groups.push(k..k + 1),
        }
    }
    let count = groups.len();
    let mut out = vec![];
    for (i, g) in groups.into_iter().enumerate() {
        let group_ops = &ops[g.clone()];
        let group_sims = &simulations[g];
        let plans: Vec<&Plan> = group_sims.iter().map(|s| &s.plan).collect();
        let blocks = blocks(group_ops, &plans, &settings);
        let problems = crate::post_check::check_setup(setup, group_ops, group_sims, &blocks, &settings, zero);
        if !problems.is_empty() {
            return Err(problems);
        }
        let part = (!settings.tool_table).then(|| (i + 1, count, group_ops[0].tool));
        let header = Header {
            job: &doc.title,
            setup,
            number: si + 1,
            part: part.map(|(i, count, _)| (i, count)),
            stock: plans[0].stock,
            datum: zero,
            disabled: setup
                .operations
                .iter()
                .filter(|o| !o.enabled)
                .map(|o| o.name().to_string())
                .collect(),
        };
        let text = emit(&header, group_ops, &blocks, &settings);
        out.push(Prepared {
            file_name: file_name(doc, si, part),
            text,
            blocks,
            ops: group_ops.to_vec(),
            datum: zero,
            settings,
            offset: offset.clone(),
        });
    }
    Ok(out)
}
pub fn post_job(doc: &Document, mesh: &Mesh) -> JobPost {
    if doc.file.is_none() {
        return JobPost::blocked("Import a part before posting. The example project's paths are illustrative and cannot be machined.");
    }
    if doc.setups.is_empty() {
        return JobPost::blocked("Create a machining setup.");
    }
    let mut results: Vec<Vec<(usize, Result<Simulation, String>)>> = vec![vec![]; doc.setups.len()];
    crate::simulation::sequence(doc, mesh, None, |si, oi, r| results[si].push((oi, r)));
    let setups = (0..doc.setups.len())
        .map(|si| {
            let result = prepare(doc, mesh, si, &results[si]).and_then(|parts| {
                parts
                    .into_iter()
                    .map(|p| {
                        crate::ngc::verify(&p.text, &p.blocks, &p.ops, p.datum, &p.settings, &p.offset)
                            .map_err(|e| vec![format!("{}: round-trip verification failed: {e}", p.file_name)])?;
                        let mut tools: Vec<String> = vec![];
                        for op in &p.ops {
                            let tool = format!("T{} {}", op.tool, op.tool_name);
                            if !tools.contains(&tool) {
                                tools.push(tool);
                            }
                        }
                        Ok(Program {
                            file_name: p.file_name,
                            lines: p.text.lines().count(),
                            operations: p.ops.iter().map(|o| o.name.clone()).collect(),
                            tools,
                            text: p.text,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            });
            SetupPost {
                setup: si,
                name: doc.setups[si].name().to_string(),
                result,
            }
        })
        .collect();
    JobPost { errors: vec![], setups }
}
/// Write every program, or nothing when any setup failed.
pub fn write_job(job: &JobPost, folder: &std::path::Path) -> Result<(std::path::PathBuf, usize), String> {
    if !job.ready() {
        let reason = job
            .errors
            .first()
            .cloned()
            .or_else(|| {
                job.setups
                    .iter()
                    .find_map(|s| s.result.as_ref().err().and_then(|e| e.first().map(|x| format!("{}: {x}", s.name))))
            })
            .unwrap_or_else(|| "No programs were produced.".into());
        return Err(format!("Nothing was written. {reason}"));
    }
    let mut count = 0;
    for program in job.setups.iter().flat_map(|s| s.result.as_ref().unwrap()) {
        crate::storage::write_atomic(&folder.join(&program.file_name), program.text.as_bytes())?;
        count += 1;
    }
    Ok((folder.to_path_buf(), count))
}
```

- [ ] **Step 5: Add the headless command in `src/main.rs`**

At the start of `fn main()`:
```rust
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--export-gcode") {
        match export_gcode(args.get(2), args.get(3)) {
            Ok(message) => {
                println!("{message}");
                std::process::exit(0)
            }
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1)
            }
        }
    }
```
Above `fn main()`:
```rust
/// Headless export of a saved project with the same checks and files as the UI.
fn export_gcode(project: Option<&String>, folder: Option<&String>) -> Result<String, String> {
    let (Some(project), Some(folder)) = (project, folder) else {
        return Err("Usage: axis-cam --export-gcode <project.json> <output folder>".into());
    };
    let mut doc = storage::load(std::path::Path::new(project))?;
    doc.sync_stock_sources();
    let file = doc.file.clone().ok_or("The project has no imported part.")?;
    let mesh = mesh::load(std::path::Path::new(&file.path))?;
    let job = post::post_job(&doc, &mesh);
    for e in &job.errors {
        eprintln!("! {e}");
    }
    for setup in &job.setups {
        match &setup.result {
            Ok(programs) => programs
                .iter()
                .for_each(|p| println!("✓ {} ({} lines)", p.file_name, p.lines)),
            Err(problems) => problems.iter().for_each(|e| eprintln!("! {}: {e}", setup.name)),
        }
    }
    let (path, count) = post::write_job(&job, std::path::Path::new(folder))?;
    Ok(format!("Wrote {count} LinuxCNC program(s) to {}", path.display()))
}
```
Add a test in `src/post.rs` `mod tests`:
```rust
    #[test]
    fn headless_export_matches_the_ui_path() {
        let dir = tempfile::tempdir().unwrap();
        let (doc, _) = fixture::job();
        let project = dir.path().join("job.axis.json");
        crate::storage::save(&project, &doc).unwrap();
        let out = dir.path().join("out");
        let message = crate::export_gcode(
            Some(&project.to_string_lossy().into_owned()),
            Some(&out.to_string_lossy().into_owned()),
        )
        .unwrap();
        assert!(message.starts_with("Wrote 1 LinuxCNC program"));
        assert!(out.join("Fixture block - 1 Top (G54).ngc").exists());
    }
```

- [ ] **Step 6: Run the tests**

Run: `cargo test --locked --quiet 2>&1 | tail -3`
Expected: `148 passed; 0 failed`. If `all_six_strategies_post_and_replay_exactly` reports a check problem, read the message. A real generator defect (for example a rapid into unproven material) must be reported to the user; do not weaken `post_check`. A verifier or emitter mismatch should be fixed in `post.rs`/`ngc.rs`.

- [ ] **Step 7: Commit**

```bash
cargo fmt && git add -A && git commit -qm "Post whole jobs with round-trip verification and headless export"
```

---

### Task 8: App integration (Program page, tool numbers, machine post settings)

**Files:**
- Modify: `src/main.rs` (App fields; `Message`; `touch`, `replace_document`, `ModelRestored`, `Page`, `Tick`, `NewTool`, `SaveTool`, `EditMachine`, `Export`; new `refresh_post`, `invalidate_post`)
- Modify: `src/ui.rs` (`post` page ~line 2151; `tools` ~line 1997; machine wizard step 3 and review ~line 1957-1960; status line ~line 318; audit notes ~line 2217 and ~2247)
- Modify: `src/model.rs` (delete `preview_nc` and test `nc_export_is_comments_only`)

**Interfaces:**
- Consumes: `post::{post_job, write_job, JobPost, preview}`, `Document::{next_tool_number, tool_number_issues}`, `Machine::fill_post_defaults`, `post_setting`, `post_setting_issues`, `POST_*`.
- Produces: `App { pub post: Option<Arc<post::JobPost>>, pub post_busy: bool, post_epoch: u64 }`; `Message::{PostChecked(u64, Arc<post::JobPost>), ExportGcode, GcodeExported(Result<(PathBuf, usize), String>)}`; `App::invalidate_post(&mut self)`; `App::refresh_post(&mut self) -> Task<Message>`.

- [ ] **Step 1: Write the failing test** (append inside `mod tests` in `src/smoke.rs`)

```rust
    #[test]
    fn program_page_rechecks_after_edits_and_ignores_stale_results() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        let (doc, mesh) = crate::post::fixture::job();
        a.replace_document(doc, None);
        a.imported_mesh = Some(mesh);
        a.smoke_dir = Some(dir.path().to_path_buf());
        let _ = a.update(Message::Page(Page::Post));
        assert!(a.post.as_ref().is_some_and(|j| j.ready()));
        let _ = a.view();
        let stale = a.post_epoch;
        let _ = a.update(Message::Edit(Target::Setup, "offset", "G92".into()));
        assert!(a.post.is_none() && a.post_epoch != stale);
        let _ = a.update(Message::PostChecked(stale, std::sync::Arc::new(crate::post::JobPost::default())));
        assert!(a.post.as_ref().is_none_or(|j| !j.setups.is_empty()));
        let _ = a.update(Message::NewTool);
        assert_eq!(a.tool_draft.number(), Some(4));
    }
```
(This sets `smoke_dir` so `refresh_post` runs synchronously. After the `offset` edit, the stale `PostChecked` must not be stored, and a fresh synchronous check replaces it.)

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test --locked --quiet program_page_rechecks 2>&1 | tail -5`
Expected: compile errors (`post`, `post_epoch`, `PostChecked` not found).

- [ ] **Step 3: Wire state and messages in `src/main.rs`**

Add fields to `pub struct App` after `pub audit_report: ...`:
```rust
    pub post: Option<std::sync::Arc<post::JobPost>>,
    pub post_busy: bool,
    post_epoch: u64,
```
Initialise in `from_workspace` after `audit_report: vec![],`:
```rust
            post: None,
            post_busy: false,
            post_epoch: 0,
```
Add to `enum Message` after `ExportError(String),`:
```rust
    PostChecked(u64, std::sync::Arc<post::JobPost>),
    ExportGcode,
    GcodeExported(Result<(PathBuf, usize), String>),
```
Add methods in `impl App` next to `refresh_stock_surface`:
```rust
    fn invalidate_post(&mut self) {
        self.post = None;
        self.post_epoch = self.post_epoch.wrapping_add(1);
    }
    /// Check and emit the whole job while the NC program page is open.
    fn refresh_post(&mut self) -> Task<Message> {
        if self.page != Page::Post || self.post.is_some() || self.post_busy {
            return Task::none();
        }
        let doc = self.doc.clone();
        let mesh = self.part_mesh().cloned();
        let run = move || match mesh {
            Some(mesh) => post::post_job(&doc, &mesh),
            None => post::JobPost::blocked("Load the imported part before posting."),
        };
        if self.smoke_dir.is_some() {
            self.post = Some(std::sync::Arc::new(run()));
            return Task::none();
        }
        let epoch = self.post_epoch;
        self.post_busy = true;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(run)
                    .await
                    .unwrap_or_else(|e| post::JobPost::blocked(&format!("Postprocessor failed: {e}")))
            },
            move |job| Message::PostChecked(epoch, std::sync::Arc::new(job)),
        )
    }
```
Call `self.invalidate_post();`:
- in `touch()`, after `self.audit_report.clear();`
- in `replace_document()`, after `self.audit_report.clear();`
- in `Message::ModelRestored` → `Ok(mesh)`, after `self.imported_mesh = Some(mesh);`

In `Message::Page(p)`, after `self.playing = false;`, add `return self.refresh_post();`.
In `Message::Tick`, replace the final `return self.refresh_stock_surface();` with:
```rust
                return Task::batch([self.refresh_stock_surface(), self.refresh_post()]);
```
Add handlers next to `Message::Exported(p) => ...`:
```rust
            Message::PostChecked(epoch, job) => {
                self.post_busy = false;
                if epoch == self.post_epoch {
                    self.post = Some(job);
                } else {
                    return self.refresh_post();
                }
            }
            Message::ExportGcode => {
                let Some(mesh) = self.part_mesh().cloned() else {
                    self.status = "Load the imported part before exporting G-code.".into();
                    return Task::none();
                };
                let doc = self.doc.clone();
                return Task::perform(
                    async move {
                        let folder = rfd::AsyncFileDialog::new()
                            .set_title("Choose a folder for the LinuxCNC programs")
                            .pick_folder()
                            .await?
                            .path()
                            .to_path_buf();
                        Some(
                            tokio::task::spawn_blocking(move || post::write_job(&post::post_job(&doc, &mesh), &folder))
                                .await
                                .unwrap_or_else(|e| Err(format!("Export worker failed: {e}"))),
                        )
                    },
                    |r| r.map_or(Message::Noop, Message::GcodeExported),
                );
            }
            Message::GcodeExported(Ok((folder, count))) => {
                self.status = format!(
                    "Exported {count} LinuxCNC program{} to {}. Preview each in LinuxCNC and air-cut before cutting.",
                    if count == 1 { "" } else { "s" },
                    folder.display()
                )
            }
            Message::GcodeExported(Err(e)) => self.status = format!("G-code export failed: {e}"),
```
Replace the body of `Message::Export(kind)` (the only remaining kind is `"sheet"`):
```rust
            Message::Export(_) => {
                let data = self.doc.setup_sheet(self.part_mesh());
                return Task::perform(
                    async move {
                        let file = rfd::AsyncFileDialog::new()
                            .set_title("Export setup sheet")
                            .set_file_name("axis-setup-sheet.md")
                            .save_file()
                            .await?;
                        let p = file.path().to_path_buf();
                        Some(storage::write_atomic(&p, data.as_bytes()).map(|()| p))
                    },
                    |r| match r {
                        Some(Ok(p)) => Message::Exported(p),
                        Some(Err(e)) => Message::ExportError(e),
                        None => Message::Noop,
                    },
                );
            }
```
`Message::NewTool`:
```rust
            Message::NewTool => {
                let mut tool = Tool::new("New tool", "Flat end mill", "6");
                tool.p.insert("number".into(), self.doc.next_tool_number().to_string());
                self.tool_draft = tool;
                self.page = Page::Tools
            }
```
`Message::SaveTool`: replace `let issues = self.tool_draft.issues();` with:
```rust
                let mut issues = self.tool_draft.issues();
                issues.extend(self.doc.tool_number_issues(&self.tool_draft));
```
`Message::EditMachine`: after `self.machine_draft = m.clone();` add `self.machine_draft.fill_post_defaults();`.

In `src/model.rs`, delete `pub fn preview_nc(&self) -> String { .. }` and the test `nc_export_is_comments_only`.

- [ ] **Step 4: Program page in `src/ui.rs`** (replace the whole `fn post`)

```rust
fn post(app: &App) -> Element<'_, Message> {
    let mut summary = column![
        bold("Program review", 20),
        note("LinuxCNC 3-axis output, one .ngc program per setup. Export repeats every check, replays each program against the simulated moves and writes nothing if any setup fails. Fixtures, clamps and the machine body are not checked: preview each program in LinuxCNC and air-cut first."),
    ]
    .spacing(14);
    let mut code = column![].spacing(18);
    match &app.post {
        None => {
            summary = summary.push(muted(if app.post_busy {
                "Checking every setup and replaying its program…"
            } else {
                "Checks run when this page opens."
            }))
        }
        Some(job) => {
            if !job.errors.is_empty() {
                summary = summary.push(warnings(job.errors.clone()));
            }
            for s in &job.setups {
                match &s.result {
                    Ok(programs) => {
                        for program in programs {
                            summary = summary.push(
                                column![
                                    bold(format!("✓  {}", program.file_name), 13),
                                    muted(format!(
                                        "{} lines · {} · {}",
                                        program.lines,
                                        program.tools.join(", "),
                                        program.operations.join(" → ")
                                    ))
                                ]
                                .spacing(4),
                            );
                            code = code.push(
                                text(crate::post::preview(&program.text, 120))
                                    .font(Font::MONOSPACE)
                                    .size(12)
                                    .color(Color::from_rgb8(183, 211, 221)),
                            );
                        }
                    }
                    Err(problems) => {
                        summary = summary
                            .push(bold(format!("!  {}", s.name), 13))
                            .push(warnings(problems.clone()));
                    }
                }
            }
        }
    }
    summary = if app.post.as_ref().is_some_and(|job| job.ready()) {
        summary.push(primary("Export LinuxCNC G-code…", Message::ExportGcode).width(Fill))
    } else {
        summary.push(muted("Export becomes available when every setup passes."))
    };
    summary = summary
        .push(btn("Export setup sheet (.md)", Message::Export("sheet"), false).width(Fill))
        .push(btn("Save complete project (.json)", Message::SaveProject, false).width(Fill));
    let code = container(scrollable(code).height(Fill))
        .padding(22)
        .width(Fill)
        .height(Fill)
        .style(|_| container::Style {
            background: Some(Color::from_rgb8(29, 43, 55).into()),
            border: Border {
                radius: 5.into(),
                ..Default::default()
            },
            ..Default::default()
        });
    container(
        column![
            page_header("NC program", "Check each setup, then export LinuxCNC G-code."),
            row![
                container(scrollable(summary))
                    .padding(22)
                    .width(375)
                    .height(Fill)
                    .style(panel_style),
                code
            ]
            .spacing(22)
            .height(Fill)
        ]
        .spacing(22),
    )
    .padding(28)
    .height(Fill)
    .into()
}
```

- [ ] **Step 5: Tool library and machine wizard in `src/ui.rs`**

In `fn tools`, change the list button label to:
```rust
                format!(
                    "T{} · {}\n{} · Ø{} mm",
                    get(&t.p, "number"),
                    t.name(),
                    get(&t.p, "type"),
                    get(&t.p, "diameter")
                ),
```
Insert `("number", "Tool number · T"),` as the first entry of the `for (k, l) in [...]` field list. Replace `form = form.push(warnings(app.tool_draft.issues())).push(` with:
```rust
    let mut tool_issues = app.tool_draft.issues();
    tool_issues.extend(app.doc.tool_number_issues(&app.tool_draft));
    form = form.push(warnings(tool_issues)).push(
```
In the machine wizard, replace the step `3 => { form=form.push(choice(p,"controller",…)) … }` note text with `"LinuxCNC has a 3-axis postprocessor. Other controllers are saved for planning; exporting G-code for them is refused."`. After that line, still inside `3 => { .. }`, add:
```rust
            if get(p, "controller") == "LinuxCNC" {
                form = form
                    .push(bold("Postprocessor", 15))
                    .push(choice_value(p, post_setting(p, "post_tool_length"), "post_tool_length", "Tool length", &POST_TOOL_LENGTH, t))
                    .push(choice_value(p, post_setting(p, "post_retract"), "post_retract", "Safe retract", &POST_RETRACT, t))
                    .push(field(p, "post_blend", "Path blending tolerance · mm", t))
                    .push(field(p, "post_dwell", "Spindle start dwell · s", t))
                    .push(choice_value(p, post_setting(p, "post_air"), "post_air", "Air blast output", &POST_AIR, t))
                    .push(warnings(post_setting_issues(p)))
                    .push(note("Setups keep a snapshot of the machine. After saving, use Assign to setup so existing setups use these settings."));
            }
```
In the review step (`_ => { … }`), after the spindle/feed `muted(..)` push, add:
```rust
.push(muted(format!("Post: {} · {} · G64 P{} · dwell {} s", post_setting(p, "post_tool_length"), post_setting(p, "post_retract"), post_setting(p, "post_blend"), post_setting(p, "post_dwell"))))
```
Status line (~318): `"mm  |  Bounded mesh previews · no NC postprocessor"` becomes `"mm  |  Bounded mesh previews · LinuxCNC 3-axis post"`.
Audit intro note (~2217): replace the sentence `No operation currently produces executable G-code or verifies machine/fixture collisions.` with `Their moves can be exported as LinuxCNC 3-axis programs after checks for limits, rapids into material and holder clearance. Fixture and machine collisions are not checked.`
Audit closing note (~2247): replace `Rest-aware toolpath optimization, tool/holder collision checks, machine kinematics and controller postprocessing remain unavailable.` with `Fully cleared top layers are skipped automatically. Leftover-region rest machining, fixture and machine collision checks, multi-axis kinematics and controllers other than LinuxCNC remain unavailable.`

- [ ] **Step 6: Run the tests**

Run: `cargo test --locked --quiet 2>&1 | tail -3`
Expected: `148 passed; 0 failed`. The new test is added and `nc_export_is_comments_only` is removed. There are no warnings about unused `post`/`ngc` items.

- [ ] **Step 7: Commit**

```bash
cargo fmt && git add -A && git commit -qm "Program page runs the LinuxCNC post; tool numbers and post settings in the UI"
```

---

### Task 9: Help and wording

**Files:**
- Modify: `src/help.rs` (`command` descriptions ~lines 214, 260, 331-336; `field_help` ~lines 633-635, 717-719)
- Modify: `src/choice_help.rs` (`selected` origin entries ~lines 151-161; `offset`/`controller` defaults ~lines 371-376; new `post_*` entries; tests)

- [ ] **Step 1: Write the failing test** (append inside `mod tests` in `src/choice_help.rs`)

```rust
    #[test]
    fn post_settings_have_choice_specific_help() {
        let p = Fields::new();
        let cards: Vec<_> = [
            ("post_tool_length", "Tool table (G43 H#)"),
            ("post_tool_length", "Touch off each tool (one program per tool)"),
            ("post_retract", "Machine Z home (G53 G0 Z0)"),
            ("post_retract", "Setup clearance plane"),
            ("post_air", "M7"),
            ("post_air", "M8"),
            ("post_air", "Not available"),
        ]
        .iter()
        .map(|(k, v)| selected(k, "Post", v, Target::Machine, &p))
        .collect();
        for (i, a) in cards.iter().enumerate() {
            assert!(!a.body.starts_with("Current saved value"));
            for b in &cards[i + 1..] {
                assert_ne!(a.body, b.body);
            }
        }
        assert!(selected("controller", "Controller", "LinuxCNC", Target::Machine, &p).body.contains("3-axis postprocessor"));
        assert!(selected("controller", "Controller", "GRBL", Target::Machine, &p).body.contains("refused"));
        for origin in ["Stock top center", "Stock top front-left", "Model origin"] {
            assert!(!selected("origin", "Origin", origin, Target::Setup, &p).body.contains("pending"));
        }
        assert!(!crate::help::field_help("number", "Tool number", Target::Tool).body.starts_with("Saved parameter"));
        for key in ["post_blend", "post_dwell"] {
            assert!(!crate::help::field_help(key, "Post", Target::Machine).body.starts_with("Saved parameter"));
        }
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test --locked --quiet post_settings_have 2>&1 | tail -5`
Expected: FAIL (generic `Current saved value` body).

- [ ] **Step 3: Implement the help text**

`src/choice_help.rs`, in `selected`'s `match (key, value)`, replace the four `("origin", ..)` arms with:
```rust
        ("origin", "Stock top center") => {
            "Program zero is the center of the stock's top face: the LinuxCNC program puts X0 Y0 at the stock center and Z0 on its top. Touch off the work offset there."
        }
        ("origin", "Stock top front-left") => {
            "Program zero is the stock's top corner on −X and −Y in setup coordinates, independent of camera orientation. Touch off X0 Y0 Z0 on that corner."
        }
        ("origin", "Model origin") => {
            "Program zero is the imported file's own 0,0,0, placed with this setup's rotation and translation. Check that the point can be located on the machine before choosing it."
        }
        ("origin", "Selected point (planned)") => {
            "Picking an arbitrary work zero is planned. G-code export is refused while this is selected."
        }
```
Add these arms to the same match:
```rust
        ("post_tool_length", "Tool table (G43 H#)") => {
            "Write T# M6, then G43 H# after each tool change. LinuxCNC applies the length stored for that tool number, so measure every tool into its tool table first."
        }
        ("post_tool_length", "Touch off each tool (one program per tool)") => {
            "Write one program per run of the same tool, with no tool change or G43 inside. Before each program, load its tool and touch off Z at the work zero. LinuxCNC's AXIS cannot touch off while a program is paused, so tool changes are not placed inside a program."
        }
        ("post_retract", "Machine Z home (G53 G0 Z0)") => {
            "Retract to machine Z0 with G53 before tool changes and at the end. On a standard LinuxCNC mill Z0 is the top of travel; confirm this on your machine."
        }
        ("post_retract", "Setup clearance plane") => {
            "Retract with G0 Z to the setup's highest clearance height, in work coordinates. Use it when machine Z0 is not the top of travel. There may be less room to change tools."
        }
        ("post_air", "M7") => {
            "Operations set to Air turn on M7, the mist output. Choose this when the air blast is wired to mist."
        }
        ("post_air", "M8") => {
            "Operations set to Air turn on M8, the flood output. Choose this when the air blast is wired to flood."
        }
        ("post_air", "Not available") => {
            "This machine has no air blast output. Export is refused for operations set to Air."
        }
```
In the `if body.is_empty() { body = match key { .. } }` block, replace the `"offset"` and `"controller"` arms with:
```rust
            "offset" => format!(
                "Use {value} as this setup's work offset. The LinuxCNC program selects it before any motion; touch it off at the setup's work zero."
            ),
            "controller" if value == "LinuxCNC" => "LinuxCNC has a 3-axis postprocessor. Export checks every setup and replays each program against the simulated moves before writing it.".to_string(),
            "controller" => format!(
                "Save {value} as the intended controller family for planning. Only LinuxCNC output is implemented, so G-code export is refused for this machine."
            ),
```
`src/help.rs`, `field_help`, add before the final `_ => (` arm:
```rust
        (Target::Tool, "number") => (
            "Tool number written as T# and, with a tool table, G43 H#. It must match the entry in LinuxCNC's tool table. Each library tool needs a unique number from 1 to 9999.",
            None,
        ),
        (_, "post_tool_length") => (
            "How programs apply tool length: from LinuxCNC's tool table with T# M6 and G43 H#, or one program per tool that you touch off before running it.",
            None,
        ),
        (_, "post_retract") => (
            "Where the tool goes before a tool change and at the end: machine Z0 with G53, or the setup's highest clearance height.",
            None,
        ),
        (_, "post_blend") => (
            "Path blending tolerance for LinuxCNC's trajectory planner. A positive value writes G64 P with this tolerance in mm, so corners may round by up to that much at speed. 0 writes G61 exact path.",
            None,
        ),
        (_, "post_dwell") => (
            "Seconds to wait after starting or changing the spindle before the first move. Use it when the spindle has no at-speed feedback. 0 writes no dwell.",
            None,
        ),
        (_, "post_air") => (
            "LinuxCNC output used for operations set to Air: M7 (mist) or M8 (flood), whichever is wired to the air blast.",
            None,
        ),
```
Change the `(_, "origin")` body to `"Work zero for this setup's G-code. Choose a point you can reproduce when touching off the stock; arbitrary point selection is planned."` and the `(_, "controller")` body to `"Controller family. LinuxCNC has a 3-axis postprocessor; other controllers are saved for planning and cannot export G-code yet."`.
`command()`: set the `Page(crate::Page::Post)` text to `"Check each setup and export LinuxCNC G-code or a setup sheet. Checks cover generation, machine limits, rapids into material and holder clearance; fixtures and the machine body are not checked."`. Set the `Generate` text to `"Generate the selected supported imported-part operation with sampled stock removal. The example project uses illustrative paths. G-code export is on the NC program page."`. Delete the `Export(_) => { .. }` arm and add:
```rust
        ExportGcode => {
            "Choose a folder and write one LinuxCNC .ngc program per setup. Every check and the replay run again first; nothing is written if any setup fails."
        }
```

- [ ] **Step 4: Confirm no stale export wording remains**

Run: `grep -rn "comments-only\|no NC postprocessor\|NC output remain\|future postprocessor's datum" src/ || echo clean`
Expected: `clean`

- [ ] **Step 5: Run the tests**

Run: `cargo test --locked --quiet 2>&1 | tail -3`
Expected: `149 passed; 0 failed`.

- [ ] **Step 6: Commit**

```bash
cargo fmt && git commit -qam "Help and wording for LinuxCNC output"
```

---

### Task 10: Renderer smoke, documentation and full verification

**Files:**
- Modify: `src/smoke.rs` (names 98–100, steps 98–100)
- Modify: `README.md`, `docs/VALIDATION.md`

- [ ] **Step 1: Add smoke screens**

In `capture_smoke`'s `match self.smoke_step` name table, add:
```rust
                98 => "75-program-blocked".into(),
                99 => "76-program-ready".into(),
                100 => "77-back-to-machining".into(),
```
In the actions `match`, replace `91..=97 => vec![],` with:
```rust
            91..=97 => vec![],
            98 => {
                self.help_animating = false;
                self.invalidate_post();
                vec![Message::Page(Page::Post)]
            }
            99 => {
                use crate::model::Strategy;
                self.doc.number_tools();
                self.doc.setups.truncate(1);
                self.doc.setups[0].machine = Machine::new(3);
                self.doc.setups[0]
                    .operations
                    .retain(|o| matches!(o.strategy, Strategy::Face | Strategy::Pocket));
                self.doc.active = self.doc.setups[0].id.clone();
                self.doc.sync_stock_sources();
                self.invalidate_post();
                vec![Message::Page(Page::Post)]
            }
            100 => {
                let job = self.post.as_ref().expect("smoke post result");
                assert!(job.ready(), "{:?}", job.setups.iter().filter_map(|s| s.result.as_ref().err()).collect::<Vec<_>>());
                vec![Message::Page(Page::Work), Message::SelectOp(0), Message::Generate]
            }
```

- [ ] **Step 2: Run the renderer smoke**

Run:
```bash
rm -rf '/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/work/post-smoke'
cargo run --release --locked -- --smoke-test '/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/work/post-smoke' 2>&1 | tail -4
ls '/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/work/post-smoke' | grep -c png
```
Expected: the process exits 0, and the count is `101` (77 named screens plus 24 motion frames). Open `75-program-blocked.png`, `76-program-ready.png` and `09-nc.png` with the Read tool. Confirm the reasons are readable, the ready page shows the program preview and export button, and nothing overlaps.

- [ ] **Step 3: Update `README.md`**

- Replace the paragraph starting `**This version does not produce executable G-code.**` with: `**G-code output is LinuxCNC 3-axis only.** Six imported-part operations have bounded motion previews with approximate stock removal: Facing, Pocket, inside/outside Contour, Drilling, Parallel and Waterline. Their exact simulated moves can be exported as one .ngc program per setup after checks and a round-trip replay (see below). Adaptive, Scallop, Indexed, Rotary and 5-axis remain explicitly unavailable for imported machining. Fixture, clamp and machine-body collisions are not checked, and production CAM remains pending. The example project's other paths and its planner are demonstrations. OpenAI is not connected, as requested for the prototype phase.`
- In the "What works" table, replace `Markdown setup sheet and comments-only \`.nc\` outline` with `LinuxCNC 3-axis \`.ngc\` per setup (checked and replayed) and a Markdown setup sheet`.
- In "Relative and fixed stock", replace `Stock inherited from previous setups remains an intended dependency; removal and transfer of remaining material are not simulated.` with `Later setups start from the material left by earlier enabled operations and setups, including after flips.`
- Add a section before `## Persistence`:

```markdown
## LinuxCNC G-code export

Open **NC program**. Checks run in the background for every setup: generation, controller (LinuxCNC, 3-axis), work offset (G54–G59.3), tool numbers, spindle and feed limits, machine travel span, rapids into material and holder clearance. Each program is then read back by a strict G-code reader and compared move by move with the simulation. **Export LinuxCNC G-code…** writes `<job> - <n> <setup> (<offset>).ngc` for every setup (touch-off mode: `… - <i> of <k> T<t>.ngc` per tool run), or nothing if any setup fails.

- Output: G21, G90, G17, G94, G0/G1/G4 only. Drilling pecks are expanded, with no canned cycles or arcs. Coordinates are relative to the setup's work zero (Stock top center, Stock top front-left or Model origin).
- Machine profile → Controller step → Postprocessor: tool table (`T# M6` + `G43 H#`) or touch off each tool (one program per tool run, touched off before each; AXIS cannot touch off while a program is paused), safe retract (`G53 G0 Z0` or the setup clearance plane), path blending (`G64 P`, or `G61` at 0), spindle start dwell and the Air output (M7/M8). Setups keep a machine snapshot, so assign the profile again after editing it.
- Tools have a number (T#) in the tool library. Older libraries are numbered 1, 2, 3… once when opened.
- Not checked: fixtures, clamps and the machine body. LinuxCNC's own soft limits check absolute position. Preview each program in LinuxCNC and air-cut first.
- Headless: `axis-cam --export-gcode <project.json> <folder>` runs the same checks and writes the same files.
```
- In "Next engineering stages", replace item 4 with `4. Add more postprocessors (GRBL, Fanuc…), arcs and canned cycles, each with the same replay verification, and validate them on known machines.`
- Replace `seventy-four screens` with `seventy-seven screens`.

- [ ] **Step 4: Full verification**

Run:
```bash
cargo fmt --check && cargo test --locked --quiet 2>&1 | tail -2 && cargo build --release --locked 2>&1 | tail -1
```
Expected: no fmt output, `test result: ok. N passed; 0 failed` (N = 149) and `Finished`.

- [ ] **Step 5: Update `docs/VALIDATION.md`**

- In the intro, replace `executable G-code` with `production G-code beyond the checked LinuxCNC 3-axis subset`.
- Automated checks: set the test count to the Step 4 result, and the smoke to **77 static screens and 24 camera-motion samples**.
- Add a section:

```markdown
## LinuxCNC post

- Golden program for a small Facing job; post settings (tool table/touch-off, retract, G61/G64, dwell, Air) change the output as specified.
- All six implemented strategies post as one program and replay exactly: every position within 0.0006 mm, feeds within 0.05 mm/min, and tool, H offset, spindle, coolant and work offset active on every move.
- Corrupted programs are rejected: a moved coordinate, a missing F, the wrong tool, an arc and a dropped line.
- Flipped setups produce separate G54/G55 programs.
- Refusals: other controllers, 4/5-axis, travel, duplicate tool numbers, selected-point origin, non-LinuxCNC offsets, Air without an output, upstream failures, a setup without enabled operations and the example project.
- Rapid-safety rule tested for accepted retracts and peck re-entry, and for refused unproven descents, sideways rapids, holder depth, feed, rpm and travel (including inch profiles).
```

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -qm "Smoke screens and documentation for LinuxCNC export"
```

---

### Task 11: Package, install and export the user's job

**Files:**
- Output: `outputs/AXIS CAM Studio.app`, `outputs/AXIS-CAM-Studio-macOS-arm64.zip`, `outputs/axis-cam-desktop-source.zip`, `outputs/linuxcnc/`

- [ ] **Step 1: Back up the live workspace (read-only copy)**

```bash
cp "$HOME/Library/Application Support/AXIS CAM Studio/workspace.json" '/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/work/workspace-before-linuxcnc-post.json'
```

- [ ] **Step 2: Export the user's saved job headlessly with the freshly built binary**

```bash
'/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/work/native-target/release/axis-cam' --export-gcode "$HOME/Library/Application Support/AXIS CAM Studio/workspace.json" '/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/outputs/linuxcnc'
```
Expected: `✓ AXIS_FourSetup_Block - 1 Setup 1 (G54).ngc (… lines)` and `Wrote 1 LinuxCNC program(s)`. If it is refused, report the listed reasons to the user verbatim. Do not change their job to make it pass.

- [ ] **Step 3: Ask the user to quit the running app, then package and install**

Ask the user to quit AXIS CAM Studio (it is running from `outputs/AXIS CAM Studio.app`). After they confirm, run:
```bash
cd '/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/outputs/axis-cam-desktop'
AXIS_PACKAGE_ZIP='/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/outputs/AXIS-CAM-Studio-macOS-arm64.zip' scripts/package-macos.sh '/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/outputs/AXIS CAM Studio.app'
codesign --verify --deep --strict '/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/outputs/AXIS CAM Studio.app' && echo signed
```
Expected: the script completes and prints `signed`.

- [ ] **Step 4: Regenerate the source archive (without .git, target or .DS_Store)**

```bash
cd '/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/outputs'
rm -f axis-cam-desktop-source.zip
zip -qr axis-cam-desktop-source.zip axis-cam-desktop -x '*/.git/*' '*/target/*' '*/dist/*' '*/smoke-output/*' '*.DS_Store'
unzip -l axis-cam-desktop-source.zip | tail -1
```
Expected: the file count grows from 83, adding `src/post.rs`, `src/post_check.rs`, `src/ngc.rs` and the two docs.

- [ ] **Step 5: Launch the installed app and confirm the NC program page**

Open `outputs/AXIS CAM Studio.app`. Continue the project, open **NC program**, and confirm the setup passes and the preview matches `outputs/linuxcnc/*.ngc`. If screen control is unavailable, say so and rely on the headless export from Step 2 plus the renderer smoke.

- [ ] **Step 6: Update the root handoff**

In `/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/HANDOFF.md`:
- Status: LinuxCNC 3-axis post implemented.
- Remaining work item 1: narrow it to other controllers, arcs and canned cycles.
- Add the git repo note.
- Add the new modules to the implementation map.
- Record the test count and smoke count.
- Add the exported file path.
