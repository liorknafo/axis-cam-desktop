# Workholding (Vise and Clamps) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Model the vise or clamps that hold the stock in each setup, suggest a layout from the toolpaths, refuse export (and AI plans) when any move hits them, and show them in the 3D view.

**Architecture:** A new `src/holding.rs` holds the library entry type (`Holder`), the starters, per-setup layout parsing, the box geometry and the pure collision check and layout suggestion. `Document.workholding` is a library like tools and materials. The export checks, the operation preview, the plan check and the 3D view call into `holding`.

**Tech Stack:** Rust 2024, Iced 0.14, existing crates only.

**Spec:** `docs/superpowers/specs/2026-10-04-workholding-design.md`

## Global Constraints

- Repository `/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/outputs/axis-cam-desktop`, branch `workholding` (from `main` 10bfbae). Before every cargo command: `export CARGO_TARGET_DIR='/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/work/native-target'`. Use `cargo test --locked --offline`. No new crates, no `#[allow]`.
- Never read, write or delete anything under `~/Library/Application Support/AXIS CAM Studio` or `~/.codex`. Never run the `#[ignore]` Keychain test. Unit tests never run Iced tasks, open a browser, touch the Keychain, use the network (except the existing loopback fake) or build an `App` on the real workspace path. Do not launch the app; do not run the renderer smoke test except in Task 6, into `.superpowers/`.
- Refuse only: toolpaths are never changed to avoid workholding.
- UI text: jaws and clamps are checked as boxes with a clearance gap when the setup has modelled workholding; parallels, bolts, the table and the machine body are not checked. Never claim more.
- Model text stays cleaned and bounded (`proposal::clip`); the only new data sent to OpenAI are setup 1's workholding boxes (relative to the model) and the gap.
- Follow the codebase's style (parameter maps like `Tool` and `Material`, compact functions, few comments).
- Commit messages end with a blank line and a `Co-Authored-By:` line naming the model that wrote the commit.
- Test counts are relative: run the suite before Task 1 and call the passing total N.

---

### Task 1: Holder type, layout, box geometry and the collision check

**Files:**
- Create: `src/holding.rs`
- Modify: `src/main.rs` (module list)

**Interfaces:**
- Produces: `holding::{Holder, KINDS, starters() -> Vec<Holder>, Layout, Side, layout(&Setup) -> Result<Layout, String>, HoldBox { label: String, bounds: Bounds }, boxes(&Layout, &Holder, stock: Bounds) -> Vec<HoldBox>, Hit { label: String, part: &'static str, at: [f32; 3] }, hits(&Plan, &[HoldBox], gap: f32) -> Vec<Hit>}`; `Holder::{new(kind, name), name(), kind(), issues() -> Vec<String>}`.

- [ ] **Step 1: Write the failing tests** — create `src/holding.rs` with only this test module; add `mod holding;` to `src/main.rs` (alphabetical):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Machine, Setup};
    use crate::simulation::{Motion, Move, Plan};
    fn stock() -> Bounds {
        Bounds { min: [-50., -30., 0.], max: [50., 30., 40.] }
    }
    fn vise() -> Holder {
        starters().into_iter().find(|h| h.kind() == "Vise").unwrap()
    }
    fn clamp() -> Holder {
        starters().into_iter().find(|h| h.kind() == "Clamp").unwrap()
    }
    fn setup(pairs: &[(&str, &str)]) -> Setup {
        let mut s = Setup::new(Machine::new(3), "G54");
        for (k, v) in pairs {
            s.p.insert((*k).into(), (*v).into());
        }
        s
    }
    /// A plan whose tool (Ø`2r`, stickout 30, holder Ø32) makes `moves`.
    fn plan(r: f32, moves: &[([f32; 3], [f32; 3])]) -> Plan {
        let mut p = crate::post::fixture::plan(moves[0].0, &[]);
        p.radius = r;
        p.flute_length = 20.;
        p.tool_length = 30.;
        p.holder_radius = 16.;
        p.moves = moves
            .iter()
            .map(|&(from, to)| Move { from, to, motion: Motion::Feed, cutting_pass: true, feed: 500., start: 0., end: 0. })
            .collect();
        p
    }
    #[test]
    fn starters_are_valid() {
        let all = starters();
        assert_eq!(all.len(), 2);
        for h in &all {
            assert!(h.issues().is_empty(), "{}: {:?}", h.name(), h.issues());
            assert!(get(&h.p, "notes").contains("Measure yours"));
        }
        let mut bad = clamp();
        bad.p.insert("bite".into(), "80".into());
        assert!(bad.issues().iter().any(|e| e.contains("bite")));
        let mut negative = vise();
        negative.p.insert("jaw_width".into(), "-1".into());
        assert!(!negative.issues().is_empty());
    }
    #[test]
    fn layouts_are_read_from_the_setup() {
        assert_eq!(layout(&setup(&[])).unwrap(), Layout::None);
        assert_eq!(
            layout(&setup(&[("holding", "Vise"), ("holder", "v1"), ("vise_axis", "Y"), ("vise_offset", "5"), ("vise_grip", "8"), ("holding_gap", "2")])).unwrap(),
            Layout::Vise { holder: "v1".into(), axis_y: true, offset: 5., grip: 8., gap: 2. }
        );
        assert_eq!(
            layout(&setup(&[("holding", "Clamps"), ("holder", "c1"), ("clamps", "-X:0;+X:10")])).unwrap(),
            Layout::Clamps { holder: "c1".into(), clamps: vec![(Side::MinusX, 0.), (Side::PlusX, 10.)], gap: 2. }
        );
        assert!(layout(&setup(&[("holding", "Clamps"), ("holder", "c1"), ("clamps", "-Z:0")])).is_err());
        assert!(layout(&setup(&[("holding", "Clamps"), ("holder", "c1"), ("clamps", "")])).is_err());
        let nine = vec!["-X:0"; 9].join(";");
        assert!(layout(&setup(&[("holding", "Clamps"), ("holder", "c1"), ("clamps", &nine)])).is_err());
        assert!(layout(&setup(&[("holding", "Vise"), ("holder", "v1"), ("vise_grip", "-1")])).is_err());
    }
    #[test]
    fn vise_jaws_and_clamps_sit_where_the_spec_says() {
        // Vise on the ±Y sides: jaw 100 wide, 20 thick, up to grip 8 above the stock bottom.
        let jaws = boxes(&Layout::Vise { holder: String::new(), axis_y: true, offset: 5., grip: 8., gap: 2. }, &vise(), stock());
        assert_eq!(jaws.len(), 2);
        let front = jaws.iter().find(|b| b.label.contains("-Y")).unwrap();
        assert_eq!(front.bounds.max[1], -30.);
        assert_eq!(front.bounds.min[1], -50.);
        assert_eq!((front.bounds.min[0], front.bounds.max[0]), (-45., 55.));
        assert_eq!(front.bounds.max[2], 8.);
        assert!(front.bounds.min[2] <= -100.);
        // A clamp on -X at offset 10: 20 wide, bite 12 onto the stock, 60 long, 25 high.
        let c = boxes(&Layout::Clamps { holder: String::new(), clamps: vec![(Side::MinusX, 10.)], gap: 2. }, &clamp(), stock());
        assert_eq!(c[0].bounds.min, [-98., 0., 40.]);
        assert_eq!(c[0].bounds.max, [-38., 20., 65.]);
        assert!(c[0].label.contains("-X"));
        assert!(boxes(&Layout::None, &vise(), stock()).is_empty());
    }
    #[test]
    fn the_check_finds_tool_and_holder_hits_and_respects_the_gap() {
        let c = boxes(&Layout::Clamps { holder: String::new(), clamps: vec![(Side::MinusX, 0.)], gap: 2. }, &clamp(), stock());
        // Clamp box: X -98..-38, Y -10..10, Z 40..65.
        // A Ø6 tool feeding at Z 41 across X -60..-30 at Y 0 cuts into the clamp.
        let into = hits(&plan(3., &[([-60., 0., 41.], [-30., 0., 41.])]), &c, 2.);
        assert_eq!(into.len(), 1);
        assert_eq!(into[0].part, "tool");
        assert!(into[0].label.contains("-X"));
        // Passing 4 mm beside it (tool edge 4 mm from the box) is clear with a 2 mm gap...
        assert!(hits(&plan(3., &[([-60., 17., 41.], [-30., 17., 41.])]), &c, 2.).is_empty());
        // ...and a hit with a 5 mm gap.
        assert_eq!(hits(&plan(3., &[([-60., 17., 41.], [-30., 17., 41.])]), &c, 5.).len(), 1);
        // A rapid high above the clamp (tip at Z 100) is clear; the holder starts at the stickout.
        assert!(hits(&plan(3., &[([-60., 0., 100.], [-30., 0., 100.])]), &c, 2.).is_empty());
        // The tip at Z 70 is above the clamp (top 65 + gap 2), but a Ø32 holder... starts at Z 100: clear.
        assert!(hits(&plan(3., &[([-60., 0., 70.], [-30., 0., 70.])]), &c, 2.).is_empty());
        // With a short stickout (holder from Z 75), a holder Ø32 passing at Y 25 (edge 9 from the box) is clear; at Y 20 it hits.
        let mut short = plan(3., &[([-60., 20., 60.], [-30., 20., 60.])]);
        short.tool_length = 3.; // tip 60 → holder from 63, which is within the clamp top 65 + 2
        let h = hits(&short, &c, 2.);
        assert!(h.iter().any(|x| x.part == "holder"), "{h:?}");
        // One hit per box and part, not per sample.
        let long = hits(&plan(3., &[([-90., 0., 41.], [-40., 0., 41.])]), &c, 2.);
        assert_eq!(long.len(), 1);
    }
}
```

- [ ] **Step 2: Run to see it fail to compile** — `cargo test --locked --offline --quiet holding:: 2>&1 | tail -5`.

- [ ] **Step 3: Write the module** (above the tests):

```rust
//! Workholding: vises and clamps from the library, where they sit in a
//! setup, and whether a toolpath comes too close to them. Toolpaths are
//! never changed; moves that hit are reported.
use crate::mesh::Bounds;
use crate::model::{Fields, Setup, fields, get, id, num};
use crate::simulation::Plan;
use serde::{Deserialize, Serialize};

pub const KINDS: [&str; 2] = ["Vise", "Clamp"];
const NOTE: &str = "Measure yours: these are typical sizes.";
/// How far below the stock bottom a vise jaw is modelled to reach.
const JAW_DEPTH: f32 = 100.;
pub const MAX_CLAMPS: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Holder {
    pub id: String,
    pub p: Fields,
}
impl Holder {
    pub fn new(kind: &str, name: &str) -> Self {
        Self { id: id(), p: fields(&[("kind", kind), ("name", name)]) }
    }
    pub fn name(&self) -> &str {
        get(&self.p, "name")
    }
    pub fn kind(&self) -> &str {
        get(&self.p, "kind")
    }
    fn keys(&self) -> &'static [&'static str] {
        if self.kind() == "Vise" {
            &["jaw_width", "jaw_height", "jaw_thickness", "max_opening", "grip"]
        } else {
            &["length", "width", "height", "bite"]
        }
    }
    pub fn issues(&self) -> Vec<String> {
        let mut e = vec![];
        if self.name().trim().is_empty() {
            e.push("Give the workholding a name.".into());
        }
        if !KINDS.contains(&self.kind()) {
            e.push("Choose Vise or Clamp.".into());
        }
        for k in self.keys() {
            if !num(&self.p, k).is_some_and(|v| v > 0.) {
                e.push(format!("{} must be a positive number.", k.replace('_', " ")));
            }
        }
        if self.kind() == "Clamp"
            && num(&self.p, "bite").zip(num(&self.p, "length")).is_some_and(|(b, l)| b >= l)
        {
            e.push("The clamp's bite must be shorter than its length.".into());
        }
        e
    }
}
pub fn starters() -> Vec<Holder> {
    let mut v = Holder::new("Vise", "100 mm machine vise");
    for (k, x) in [("jaw_width", "100"), ("jaw_height", "30"), ("jaw_thickness", "20"), ("max_opening", "120"), ("grip", "8")] {
        v.p.insert(k.into(), x.into());
    }
    let mut c = Holder::new("Clamp", "Toe clamp 60 mm");
    for (k, x) in [("length", "60"), ("width", "20"), ("height", "25"), ("bite", "12")] {
        c.p.insert(k.into(), x.into());
    }
    for h in [&mut v, &mut c] {
        h.p.insert("notes".into(), NOTE.into());
    }
    vec![v, c]
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    MinusX,
    PlusX,
    MinusY,
    PlusY,
}
impl Side {
    pub const ALL: [Side; 4] = [Side::MinusX, Side::PlusX, Side::MinusY, Side::PlusY];
    pub fn label(self) -> &'static str {
        match self {
            Side::MinusX => "-X",
            Side::PlusX => "+X",
            Side::MinusY => "-Y",
            Side::PlusY => "+Y",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        Side::ALL.into_iter().find(|side| side.label() == s.trim())
    }
}
/// A setup's modelled workholding, read from its parameters.
#[derive(Debug, Clone, PartialEq)]
pub enum Layout {
    None,
    Vise { holder: String, axis_y: bool, offset: f32, grip: f32, gap: f32 },
    Clamps { holder: String, clamps: Vec<(Side, f32)>, gap: f32 },
}
/// Encodes clamp positions as stored on the setup ("-X:0;+X:10").
pub fn clamps_text(clamps: &[(Side, f32)]) -> String {
    clamps.iter().map(|(s, o)| format!("{}:{}", s.label(), o)).collect::<Vec<_>>().join(";")
}
pub fn layout(setup: &Setup) -> Result<Layout, String> {
    let p = &setup.p;
    let gap = match get(p, "holding_gap").trim() {
        "" => 2.,
        _ => num(p, "holding_gap").filter(|g| *g >= 0.).ok_or("The clearance gap must be 0 or more.")?,
    };
    let holder = get(p, "holder").to_string();
    match get(p, "holding") {
        "" | "None" => Ok(Layout::None),
        "Vise" => {
            let grip = num(p, "vise_grip").filter(|g| *g > 0.).ok_or("The vise grip must be a positive number.")?;
            let offset = match get(p, "vise_offset").trim() {
                "" => 0.,
                _ => num(p, "vise_offset").ok_or("The vise offset must be a number.")?,
            };
            Ok(Layout::Vise { holder, axis_y: get(p, "vise_axis") == "Y", offset, grip, gap })
        }
        "Clamps" => {
            let mut clamps = vec![];
            for part in get(p, "clamps").split(';').filter(|s| !s.trim().is_empty()) {
                let (side, offset) = part.split_once(':').ok_or("Write each clamp as side:offset, for example -X:0.")?;
                let side = Side::parse(side).ok_or("A clamp side must be -X, +X, -Y or +Y.")?;
                let offset = offset.trim().parse::<f32>().ok().filter(|v| v.is_finite()).ok_or("A clamp offset must be a number.")?;
                clamps.push((side, offset));
            }
            if clamps.is_empty() || clamps.len() > MAX_CLAMPS {
                return Err(format!("Place 1 to {MAX_CLAMPS} clamps."));
            }
            Ok(Layout::Clamps { holder, clamps, gap })
        }
        other => Err(format!("\"{other}\" is not a workholding choice.")),
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct HoldBox {
    pub label: String,
    pub bounds: Bounds,
}
/// The jaws or clamps of `layout`, in the frame of `stock` (the setup's
/// starting stock box). Without the clearance gap.
pub fn boxes(layout: &Layout, holder: &Holder, stock: Bounds) -> Vec<HoldBox> {
    let n = |k| num(&holder.p, k).unwrap_or(0.);
    let centre = [(stock.min[0] + stock.max[0]) / 2., (stock.min[1] + stock.max[1]) / 2.];
    match layout {
        Layout::None => vec![],
        Layout::Vise { axis_y, offset, grip, .. } => {
            let (w, t) = (n("jaw_width"), n("jaw_thickness"));
            let (z0, z1) = (stock.min[2] - JAW_DEPTH, stock.min[2] + grip);
            let (across, along) = if *axis_y { (1, 0) } else { (0, 1) };
            [(false, "-"), (true, "+")]
                .into_iter()
                .map(|(plus, sign)| {
                    let mut min = [0.; 3];
                    let mut max = [0.; 3];
                    if plus {
                        (min[across], max[across]) = (stock.max[across], stock.max[across] + t);
                    } else {
                        (min[across], max[across]) = (stock.min[across] - t, stock.min[across]);
                    }
                    let c = centre[along] + offset;
                    (min[along], max[along]) = (c - w / 2., c + w / 2.);
                    (min[2], max[2]) = (z0, z1);
                    HoldBox {
                        label: format!("vise jaw on {sign}{}", if *axis_y { "Y" } else { "X" }),
                        bounds: Bounds { min, max },
                    }
                })
                .collect()
        }
        Layout::Clamps { clamps, .. } => {
            let (l, w, h, bite) = (n("length"), n("width"), n("height"), n("bite"));
            clamps
                .iter()
                .map(|&(side, offset)| {
                    let (across, along, plus) = match side {
                        Side::MinusX => (0, 1, false),
                        Side::PlusX => (0, 1, true),
                        Side::MinusY => (1, 0, false),
                        Side::PlusY => (1, 0, true),
                    };
                    let mut min = [0.; 3];
                    let mut max = [0.; 3];
                    if plus {
                        (min[across], max[across]) = (stock.max[across] - bite, stock.max[across] + l - bite);
                    } else {
                        (min[across], max[across]) = (stock.min[across] - (l - bite), stock.min[across] + bite);
                    }
                    let c = centre[along] + offset;
                    (min[along], max[along]) = (c - w / 2., c + w / 2.);
                    (min[2], max[2]) = (stock.max[2], stock.max[2] + h);
                    HoldBox { label: format!("clamp on {} at {offset}", side.label()), bounds: Bounds { min, max } }
                })
                .collect()
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub label: String,
    /// "tool" (cutter and shank, tip to stickout) or "holder" (above the stickout).
    pub part: &'static str,
    pub at: [f32; 3],
}
/// Distance in XY from `p` to the rectangle of `b`.
fn xy_distance(b: &Bounds, p: [f32; 3]) -> f32 {
    let dx = (b.min[0] - p[0]).max(0.).max(p[0] - b.max[0]);
    let dy = (b.min[1] - p[1]).max(0.).max(p[1] - b.max[1]);
    dx.hypot(dy)
}
/// Moves of `plan` whose tool or holder comes within `gap` of a box. The
/// tool is a vertical cylinder of the cutter radius from the tip to the tip
/// plus the stickout, the holder one of the holder radius from there
/// upward. Moves are sampled every 1 mm (at least both ends, at most 2 000
/// samples); one hit per box and part.
pub fn hits(plan: &Plan, boxes: &[HoldBox], gap: f32) -> Vec<Hit> {
    let mut out: Vec<Hit> = vec![];
    for m in &plan.moves {
        let length = (0..3).map(|k| (m.to[k] - m.from[k]).powi(2)).sum::<f32>().sqrt();
        let steps = (length.ceil() as usize).clamp(1, 2000);
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let tip: [f32; 3] = std::array::from_fn(|k| m.from[k] + (m.to[k] - m.from[k]) * t);
            for b in boxes {
                let parts = [
                    ("tool", plan.radius, tip[2], tip[2] + plan.tool_length),
                    ("holder", plan.holder_radius, tip[2] + plan.tool_length, f32::INFINITY),
                ];
                for (part, radius, low, high) in parts {
                    let overlaps = low <= b.bounds.max[2] + gap && high >= b.bounds.min[2] - gap;
                    if overlaps
                        && xy_distance(&b.bounds, tip) <= radius + gap
                        && !out.iter().any(|h| h.label == b.label && h.part == part)
                    {
                        out.push(Hit { label: b.label.clone(), part, at: tip });
                    }
                }
            }
        }
    }
    out
}
```

Check the test expectations against this code; if a hand-computed number in a test is wrong, fix the test's number (not the rule) and say so in the report. Note in particular: the clamp test's "beside it" case puts the tool edge 4 mm from the box (Y 17 − 3 = 14 vs box Y max 10), so a 2 mm gap is clear and a 5 mm gap hits.

- [ ] **Step 4: Run** — `cargo test --locked --offline --quiet 2>&1 | tail -3` → N + 5 passed (dead-code warnings for unused `holding` items are expected until Task 2).
- [ ] **Step 5: Commit** — `cargo fmt && git add src/holding.rs src/main.rs && git commit -q -m "Workholding: holder type, layout, boxes and the collision check"` (plus the trailer).

---

### Task 2: The workholding library and the setup controls

**Files:**
- Modify: `src/model.rs` (`Document.workholding`, `adopt_holders`, validation), `src/storage.rs` (adopt on load), `src/main.rs` (carry on New project, Explore merge, workholding page messages, setup messages), `src/ui.rs` (Workholding page, setup editor controls), `src/help.rs` (help), `src/smoke.rs` (tests)

**Interfaces:**
- Consumes: Task 1.
- Produces: `Document.workholding: Vec<Holder>` (`#[serde(default)]`); `Document::adopt_holders(&mut self)` (adds the starters only when the library is empty); `validate_structure` refuses duplicate or empty holder ids; `Page::Workholding`; `Target::Holder`; `App.holder_draft`; messages `NewHolder(kind)`, `EditHolder(id)`, `SaveHolder`, `DeleteHolder(id)` (refused while any setup uses it); setup keys written through the existing `Message::Edit(Target::Setup, key, value)` for `holding`, `holder`, `vise_axis`, `vise_offset`, `vise_grip`, `holding_gap`, plus `Message::SetClamps(String)` writing `clamps` (validated by `holding::layout`).

- [ ] **Step 1: Failing tests** in `src/smoke.rs` (temp-folder App helpers): the library starts with the two starters after load/adopt (and keeps a user's edits; a library with entries gets no starters); New project carries it; Explore merges without duplicate names (case-insensitive, the shared `material::same_name` style helper or a local equivalent); a vise and a clamp are created, edited, saved, refused when invalid (bite ≥ length), and deletion is refused while a setup uses the holder; choosing Vise on a setup with the starter vise writes `holding`/`holder` and the setup's `holding::layout` is `Layout::Vise{..}`; `SetClamps("-X:0;+X:0")` writes valid clamps and an invalid text is refused with the layout error in the status; the setup editor shows the layout's issues (for example a stock wider than the vise's maximum opening: "The stock is 140 mm across the jaws; this vise opens to 120 mm."); the Workholding page and the setup editor build in each state.

- [ ] **Step 2: Run to fail; Step 3: implement**, mirroring the Materials page (see `Message::NewMaterial/EditMaterial/SaveMaterial/DeleteMaterial` and `fn materials` in `src/ui.rs`). Setup editor, in the existing "Workholding" section above the fixture description:
  - **Modelled workholding**: `pick_list` None / Vise / Clamps (key `holding`).
  - **Vise**: picker of library vises (key `holder`), "Jaws face" X sides / Y sides (`vise_axis` X/Y), offset (`vise_offset`, mm), grip (`vise_grip`, mm, default the holder's grip when the vise is first chosen).
  - **Clamps**: picker of library clamps (`holder`), a text field "Clamp positions" (`clamps`, "side:offset; …", help explains the format and lists the sides), validated on every edit with the error shown under it.
  - **Clearance gap** (`holding_gap`, mm, default 2).
  - **Suggest layout** button (wired in Task 3; in this task it may be absent).
  - Under the controls: `holding::layout` errors and the setup issue for the vise opening.
  - A Workholding page button next to Materials in the document bar; the page lists vises and clamps (name, kind, key sizes), an editor with every field for the chosen kind, **+ New vise**, **+ New clamp**, **Save**, **Delete**.
  - `Message::Edit(Target::Setup, …)` on these keys calls `touch()` like other setup edits (it already does).
- [ ] **Step 4: Tests pass; Step 5: commit** (message "Workholding library and setup controls").

---

### Task 3: The suggested layout

**Files:**
- Modify: `src/holding.rs` (`suggest_vise`, `suggest_clamps`), `src/main.rs` (`Message::SuggestLayout`, automatic suggestion when Vise/Clamps or the holder is first chosen), `src/ui.rs` (button and status), `src/smoke.rs` and `src/holding.rs` tests

**Interfaces:**
- Produces:
  - `holding::suggest_vise(stock: Bounds, holder: &Holder, plans: &[&Plan], gap: f32) -> (bool /* axis_y */, f32 /* grip */)` — for each axis (X sides, Y sides) compute the jaws (offset 0) at the holder's grip, reducing the grip in 1 mm steps down to 3 mm while `hits` finds any hit; pick the axis with no hits at the largest grip; if neither is clear, pick the axis whose jaws have the larger smallest XY distance to any move below the jaw top + gap, at grip 3 mm.
  - `holding::suggest_clamps(stock: Bounds, holder: &Holder, plans: &[&Plan], gap: f32) -> Vec<(Side, f32)>` — candidates are each side's middle and quarter points (offsets −L/4, 0, +L/4 along the side, L = the side's length); score = the smallest XY distance from the clamp box (grown by gap) to any move whose tip is below the clamp top + gap (infinite when none); choose 4 clamps (2 when both stock sides are shorter than 3 × the clamp width) maximising the smallest score, preferring opposite sides on ties (greedy is fine: best candidate first, then the best on the opposite side, then the best of the remaining sides, at most one clamp per side).
  - `Message::SuggestLayout`: generates the setup's enabled operations (the same way **Generate all** does for one setup; reuse `simulation::sequence` or the existing helper the audit uses), calls the suggest function for the setup's kind and holder, writes the keys, `touch()`es, and sets the status: "Suggested: vise jaws on ±Y, grip 8 mm. Every operation is clear of them." or "… N operations come within the gap; see the warnings." Choosing Vise or Clamps (or a different holder) for the first time on a setup does the same automatically; later edits never move anything until **Suggest layout** is pressed again.
- [ ] **Step 1: Failing tests** (pure, in `src/holding.rs`): with moves only near the ±X sides (contours at X ±48), the vise is suggested on Y; with moves near ±Y, on X; with a pocket that goes down to Z 5 near every side, the grip drops to 3 and the clearer axis is kept; clamps avoid a side whose middle is crossed by moves at the top (they move to quarter points or other sides); a small stock gets 2 clamps on opposite sides. App tests: SuggestLayout writes the keys and the status names the result; choosing Vise the first time suggests; editing the offset afterwards is kept until SuggestLayout.
- [ ] **Step 2: Run to fail; Step 3: implement; Step 4: pass; Step 5: commit** ("Suggested workholding layout from the toolpaths").

---

### Task 4: Checks, 3D view, setup sheet and G-code header

**Files:**
- Modify: `src/post.rs` / `src/post_check.rs` (export refusal), `src/main.rs` or `src/model.rs` (a `Document::holding_hits(setup_index, sims)` helper), `src/ui.rs` (operation preview warning, NC page), `src/viewport.rs` (draw boxes), `src/model.rs` (setup sheet), tests in those files

**Interfaces:**
- Produces: `Document::holding_boxes(&self, setup: &Setup, stock: Bounds) -> Result<Vec<HoldBox>, String>` (layout + library holder; an unknown holder id is an error "The workholding entry was deleted; choose another."), used everywhere below.
- Export: in `post::prepare` (where the stock box of the setup is known), for each operation's plan call `holding::hits(plan, &boxes, gap)`; each hit is an export problem: "{op name}: the {part} hits the {label} at X{x:.1} Y{y:.1} Z{z:.1} (within the {gap} mm gap)." in work coordinates (subtract the datum like other post messages). A layout error refuses the setup with that error.
- Operation preview: after an operation is generated, the inspector shows the same messages as a warning (amber box, like the speed/feed warnings), never blocking.
- NC page: setups with modelled workholding list hits in the refusal list (they already appear as export problems); setups with "None" keep today's not-checked wording.
- 3D view: when a setup has modelled workholding, draw its boxes (without the gap) semi-transparent in a neutral grey, in the setup's frame, like the stock box is drawn (look at how `show_stock` draws the stock); only when the stock is shown.
- Setup sheet and G-code header: after the fixture line, "Workholding: 100 mm machine vise on ±Y, grip 8 mm, offset 0 mm, gap 2 mm." or "Workholding: 4 × Toe clamp 60 mm at -X 0, +X 0, -Y 0, +Y 0, gap 2 mm." (header line starts with the fixed word "Workholding:" like the other header comments; the existing comment-safety test must still pass).
- [ ] **Step 1: Failing tests**: `post_job` on the fixture job with a clamp placed over the pocket path is refused with the exact message pattern; with the clamp moved clear it is ready; the header and setup sheet carry the workholding line; a deleted holder refuses with its message; the preview warning appears in the inspector for an operation that hits (view builds; the message list helper is asserted); the 3D view builds with boxes.
- [ ] **Step 2–5** as usual (commit "Workholding checks at export and preview; drawn in 3D; listed in the program").

---

### Task 5: The planner respects workholding

**Files:**
- Modify: `src/analysis.rs` (report), `src/proposal.rs` (later setups copy the holding), `src/plan_check.rs` (suggest for later setups, stage "workholding"), `src/planner.rs` (instructions), `src/ui.rs` (preview card lines), tests

**Interfaces:**
- Part report: `workholding: Option<WorkholdingReport { kind, boxes: Vec<[f32; 6]> /* x0,y0,z0,x1,y1,z1 relative to the model's -X/-Y/bottom corner in setup 1 */, gap }>`, from setup 1's layout (None when "None").
- `proposal::build`: later setups copy setup 1's `holding`, `holder` and `holding_gap` (no positions yet).
- `plan_check::check_built`: after generation, for each later setup whose holding is Vise/Clamps without positions, run the Task 3 suggestion on that setup's generated plans and write the keys into the built job before the export stage; then stage "workholding": every hit in any setup is a problem ("Setup s, operation o: the tool hits the clamp on -X at 0 …") going back to the model. The returned job carries the suggested layouts, so the preview shows them.
- Instructions: a WORKHOLDING section: the report's boxes are keep-out zones; moves within the gap are refused; later setups get the same kind of workholding laid out by the app, which avoids the toolpaths where it can; keep outside contours above the jaw top or move them to a setup where the sides are free.
- Preview card: each setup with modelled workholding shows the Task 4 workholding line.
- [ ] **Step 1: Failing tests**: the report carries setup 1's boxes and gap; a proposal whose contour runs through setup 1's clamp is refused at stage "workholding" naming setup and operation; `good()` with setup 1 holding Clamps placed clear passes and its setup 2 gets a suggested layout in the returned job; the instructions mention "WORKHOLDING" and "gap".
- [ ] **Step 2–5** as usual (commit "The AI planner respects the vise and clamps").

---

### Task 6: Texts, documentation, renderer check

**Files:**
- Modify: every "Not checked: … fixtures, clamps …" text (preview note, NC page note, G-code header "Not checked" note, help texts, README, VALIDATION), `README.md`, `docs/VALIDATION.md`, `src/smoke.rs` (renderer screens)

- [ ] **Step 1**: Texts: when the setup (or every setup of the plan) has modelled workholding, the not-checked sentences say "vise jaws and clamps are checked as boxes with a clearance gap; parallels, bolts, the table and the machine body are not"; setups with "None" keep the old wording. Update the pinned tests.
- [ ] **Step 2**: README "Workholding" section (library, setup controls, suggestion, the check and its limits, the planner); VALIDATION section with each sentence naming its test; totals.
- [ ] **Step 3**: Renderer smoke screens appended after the existing last static screen (numbering continues from 88): `88-workholding-vise` (a setup with the vise drawn in 3D), `89-workholding-clamps` (clamps drawn; one operation's warning visible in the inspector), `90-workholding-page`. Run the smoke into `.superpowers/smoke-workholding` after a release build; look at 88–90 and 10-minimum-window (the document bar gains a Workholding button) and report clipping or overlap.
- [ ] **Step 4**: `cargo fmt --check`, full suite; commit ("Document workholding; renderer screens").
