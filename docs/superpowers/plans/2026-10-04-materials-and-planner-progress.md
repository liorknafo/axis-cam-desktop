# Material Library, Speed/Feed Checks and Planner Progress — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A material library like the tool library (with cutting data and starter materials), speed and feed checks against it (warnings for the user's operations, refusals for AI plans), and a live timeline of the AI planner's progress.

**Architecture:** A new `src/material.rs` holds the `Material` type, the starter data, validation and the pure speed/feed check. `Document` gains `materials` and `material_id`; loading adopts legacy material names. The Materials page mirrors the Tools page. The planner reports `Event`s (stage starts, check results) instead of bare stages, and the OpenAI backend counts streamed reply characters in a shared counter the App reads on its existing tick.

**Tech Stack:** Rust 2024, Iced 0.14, existing crates only.

**Spec:** `docs/superpowers/specs/2026-10-04-materials-and-planner-progress-design.md`

## Global Constraints

- Repository `/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/outputs/axis-cam-desktop`, branch `materials-progress`. Before every cargo command: `export CARGO_TARGET_DIR='/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/work/native-target'`. Use `cargo test --locked --offline`. No new crates, no `#[allow]`.
- Never read, write or delete anything under `~/Library/Application Support/AXIS CAM Studio` or `~/.codex`. Never run the `#[ignore]` Keychain test. Unit tests never run Iced tasks, open a browser, touch the Keychain, use the network (except the existing loopback fake) or build an `App` on the real workspace path (use the existing temp-folder helpers in `src/smoke.rs`). Do not launch the app; do not run the renderer smoke test except in Task 7, into a scratch folder.
- Model text stays cleaned and bounded (`proposal::clip`); nothing new is sent to OpenAI except the material's name, group, hardness, notes and cutting data.
- UI text never claims fixture, clamp or machine collision checking, cutting-load checking or exact gouge detection.
- Follow the codebase's style: parameter maps (`Fields`) like `Tool`, compact functions, few comments.
- Commit messages end with a blank line and `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Stage only the files each task names.
- Test counts are relative: run the suite before Task 1 and call the passing total N.

---

### Task 1: The material type, starters and the speed/feed check

**Files:**
- Create: `src/material.rs`
- Modify: `src/model.rs` (`Document`: `materials`, `material_id`), `src/main.rs` (module list)

**Interfaces:**
- Produces: `material::{Material, CuttingData, GROUPS, BANDS, STARTER_NOTE, starters() -> Vec<Material>, SpeedFeed, check(&Material, &Tool, &Operation, &Machine) -> SpeedFeed, band(diameter) -> usize}`; `Material::{new(name), name(), cutting_data() -> Option<CuttingData>, issues() -> Vec<String>}`; `Document.materials: Vec<Material>` and `Document.material_id: String` (both `#[serde(default)]`).

- [ ] **Step 1: Write the failing tests** — create `src/material.rs` with only this test module, and add `mod material;` to `src/main.rs` (alphabetical position):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Machine, Operation, Strategy, Tool};
    fn op(rpm: &str, feed: &str) -> Operation {
        let mut o = Operation::new(Strategy::Pocket, String::new());
        o.p.insert("rpm".into(), rpm.into());
        o.p.insert("feed".into(), feed.into());
        o
    }
    fn alu() -> Material {
        starters().into_iter().find(|m| m.name() == "Aluminium 6061-T6").unwrap()
    }
    #[test]
    fn starters_are_valid_and_complete() {
        let all = starters();
        assert_eq!(all.len(), 9);
        for m in &all {
            assert!(m.issues().is_empty(), "{}: {:?}", m.name(), m.issues());
            assert!(m.cutting_data().is_some(), "{}", m.name());
            assert!(get(&m.p, "notes").starts_with(STARTER_NOTE));
            assert!(GROUPS.contains(&get(&m.p, "group")));
        }
        let mut names: Vec<_> = all.iter().map(|m| m.name().to_lowercase()).collect();
        names.dedup();
        assert_eq!(names.len(), 9);
        let data = alu().cutting_data().unwrap();
        assert_eq!(data.vc, (150., 300.));
        assert_eq!(data.fz[1], (0.02, 0.05));
    }
    #[test]
    fn cutting_data_must_be_complete_positive_and_ordered() {
        let mut m = Material::new("Test");
        assert!(m.cutting_data().is_none() && m.issues().is_empty());
        m.p.insert("vc_min".into(), "100".into());
        assert!(m.issues().iter().any(|e| e.contains("every cutting-data field")));
        let mut full = alu();
        full.p.insert("vc_min".into(), "400".into());
        assert!(full.issues().iter().any(|e| e.contains("Surface speed")));
        assert!(full.cutting_data().is_none());
        let mut negative = alu();
        negative.p.insert("fz6_min".into(), "-0.01".into());
        assert!(!negative.issues().is_empty());
        let mut blank = alu();
        blank.p.insert("name".into(), " ".into());
        assert!(blank.issues().iter().any(|e| e.contains("name")));
        let mut long = alu();
        long.p.insert("notes".into(), "x".repeat(501));
        assert!(long.issues().iter().any(|e| e.contains("500")));
    }
    #[test]
    fn bands_follow_the_tool_diameter() {
        assert_eq!([2.9, 3., 3.1, 6., 10., 16., 16.1, 50.].map(band), [0, 0, 1, 1, 2, 3, 4, 4]);
    }
    #[test]
    fn surface_speed_and_chip_load_are_checked_with_machine_limits() {
        let machine = Machine::new(3); // 12000 rpm, 5000 mm/min
        let tool = Tool::new("Flat 6", "Flat end mill", "6"); // 3 flutes
        // 8000 rpm: Vc = π·6·8000/1000 = 150.8 m/min; 1000 mm/min: fz = 1000/(8000·3) = 0.0417
        assert!(matches!(check(&alu(), &tool, &op("8000", "1000"), &machine), SpeedFeed::InRange { .. }));
        let SpeedFeed::Out(problems) = check(&alu(), &tool, &op("20000", "1000"), &machine) else { panic!() };
        assert!(problems[0].contains("Surface speed 377 m/min is above Aluminium 6061-T6's 150–300 m/min"), "{problems:?}");
        let SpeedFeed::Out(problems) = check(&alu(), &tool, &op("8000", "200"), &machine) else { panic!() };
        assert!(problems[0].contains("Chip load 0.008 mm per tooth is below"), "{problems:?}");
        // A 1 mm tool cannot reach 150 m/min under 12000 rpm: the machine's maximum counts.
        let small = Tool::new("Flat 1", "Flat end mill", "1");
        assert!(matches!(check(&alu(), &small, &op("12000", "500"), &machine), SpeedFeed::InRange { .. }));
        assert!(matches!(check(&alu(), &small, &op("11000", "500"), &machine), SpeedFeed::Out(_)));
        // 1 % slack for rounding.
        assert!(matches!(check(&alu(), &tool, &op("7900", "1000"), &machine), SpeedFeed::InRange { .. }));
        // Nothing to check against.
        assert!(matches!(check(&Material::new("Unknown"), &tool, &op("8000", "1000"), &machine), SpeedFeed::Unchecked(_)));
        let mut broken = tool.clone();
        broken.p.insert("flutes".into(), "0".into());
        assert!(matches!(check(&alu(), &broken, &op("8000", "1000"), &machine), SpeedFeed::Unchecked(_)));
    }
    #[test]
    fn documents_saved_before_the_library_still_load() {
        let old = r#"{"schema":1,"title":"Old","material":"Aluminum 6061","file":null,"machines":[],"tools":[],"setups":[],"active":""}"#;
        let doc: crate::model::Document = serde_json::from_str(old).unwrap();
        assert!(doc.materials.is_empty() && doc.material_id.is_empty());
        assert_eq!(doc.material, "Aluminum 6061");
    }
}
```

- [ ] **Step 2: Run the tests to see them fail to compile**

Run: `cargo test --locked --offline --quiet material:: 2>&1 | tail -5`
Expected: compile errors (`Material`, `starters`, `check`, `band` not found; `Document` has no `materials`).

- [ ] **Step 3: Add the fields to `Document`** (`src/model.rs`), after `pub material: String,`:

```rust
    /// The material library, carried to new projects like tools.
    #[serde(default)]
    pub materials: Vec<crate::material::Material>,
    /// The selected library material's id; `material` keeps its name.
    #[serde(default)]
    pub material_id: String,
```

and in `Default for Document` add `materials: vec![], material_id: String::new(),`. Fix every other `Document { … }` struct literal the compiler reports (use `..Document::default()` where a literal lists every field).

- [ ] **Step 4: Write the module** (above the tests in `src/material.rs`):

```rust
//! The material library: what a material holds, the starter materials, and
//! the speed and feed check against a material's cutting data.
use crate::model::{Fields, Machine, Operation, Tool, fields, get, id, num};
use serde::{Deserialize, Serialize};

pub const GROUPS: [&str; 7] = [
    "Aluminium",
    "Steel",
    "Stainless steel",
    "Brass and copper",
    "Plastic",
    "Wood and composite",
    "Other",
];
/// Chip-load bands: key prefix and the largest tool diameter (mm) in the band.
pub const BANDS: [(&str, f32); 5] = [
    ("fz3", 3.),
    ("fz6", 6.),
    ("fz10", 10.),
    ("fz16", 16.),
    ("fzbig", f32::INFINITY),
];
pub const STARTER_NOTE: &str =
    "Starting point for carbide tools: adjust to your machine and your tool maker's data.";
/// Every cutting-data key, in pairs.
fn keys() -> Vec<String> {
    std::iter::once("vc")
        .chain(BANDS.iter().map(|(k, _)| *k))
        .flat_map(|k| [format!("{k}_min"), format!("{k}_max")])
        .collect()
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub id: String,
    pub p: Fields,
}
/// A material's complete cutting data: surface speed (m/min) and chip load
/// per tooth (mm) for each band of `BANDS`, as (min, max).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CuttingData {
    pub vc: (f32, f32),
    pub fz: [(f32, f32); 5],
}
impl Material {
    pub fn new(name: &str) -> Self {
        Self {
            id: id(),
            p: fields(&[("name", name), ("group", "Other")]),
        }
    }
    pub fn name(&self) -> &str {
        get(&self.p, "name")
    }
    fn pair(&self, key: &str) -> Option<(f32, f32)> {
        let min = num(&self.p, &format!("{key}_min"))?;
        let max = num(&self.p, &format!("{key}_max"))?;
        (min > 0. && min <= max).then_some((min, max))
    }
    pub fn cutting_data(&self) -> Option<CuttingData> {
        let vc = self.pair("vc")?;
        let mut fz = [(0., 0.); 5];
        for (slot, (key, _)) in fz.iter_mut().zip(BANDS) {
            *slot = self.pair(key)?;
        }
        Some(CuttingData { vc, fz })
    }
    pub fn issues(&self) -> Vec<String> {
        let mut e = vec![];
        if self.name().trim().is_empty() {
            e.push("Give the material a name.".into());
        }
        if !GROUPS.contains(&get(&self.p, "group")) {
            e.push("Choose a material group.".into());
        }
        if get(&self.p, "notes").chars().count() > 500 {
            e.push("Notes can be at most 500 characters.".into());
        }
        let keys = keys();
        let filled = keys.iter().filter(|k| !get(&self.p, k).trim().is_empty()).count();
        if filled > 0 && filled < keys.len() {
            e.push("Fill in every cutting-data field, or leave them all empty.".into());
        }
        if filled == keys.len() {
            for k in &keys {
                if !num(&self.p, k).is_some_and(|v| v > 0.) {
                    e.push(format!("{k} must be a positive number."));
                }
            }
            if self.pair("vc").is_none() {
                e.push("Surface speed: the minimum must not exceed the maximum.".into());
            }
            for (k, limit) in BANDS {
                if self.pair(k).is_none() {
                    let band = if limit.is_finite() {
                        format!("up to {limit} mm")
                    } else {
                        "over 16 mm".into()
                    };
                    e.push(format!("Chip load {band}: the minimum must not exceed the maximum."));
                }
            }
        }
        e
    }
}
/// The chip-load band of a tool diameter (index into `BANDS`).
pub fn band(diameter: f32) -> usize {
    BANDS.iter().position(|(_, limit)| diameter <= *limit).unwrap_or(4)
}
/// The result of checking one operation's speed and feed.
#[derive(Debug, Clone, PartialEq)]
pub enum SpeedFeed {
    InRange { vc: f32, fz: f32 },
    /// Each problem names the value and the material's range.
    Out(Vec<String>),
    /// Why the operation could not be checked.
    Unchecked(String),
}
/// Checks an operation's spindle speed and feed against `material`, for
/// `tool` on `machine` (whose maximum rpm and feed count as in range when the
/// material asks for more). 1 % slack on every limit absorbs rounding.
pub fn check(material: &Material, tool: &Tool, op: &Operation, machine: &Machine) -> SpeedFeed {
    let Some(data) = material.cutting_data() else {
        return SpeedFeed::Unchecked(format!("{} has no cutting data.", material.name()));
    };
    let (Some(d), Some(z)) = (num(&tool.p, "diameter"), num(&tool.p, "flutes")) else {
        return SpeedFeed::Unchecked("The tool has no diameter or flutes.".into());
    };
    let (Some(n), Some(f)) = (num(&op.p, "rpm"), num(&op.p, "feed")) else {
        return SpeedFeed::Unchecked("The operation has no spindle speed or feed.".into());
    };
    if d <= 0. || z < 1. || n <= 0. || f <= 0. {
        return SpeedFeed::Unchecked("The tool or the operation has no positive speed data.".into());
    }
    let inch = if get(&machine.p, "units") == "inch" { 25.4 } else { 1. };
    let max_rpm = num(&machine.p, "rpm").unwrap_or(f32::INFINITY);
    let max_feed = num(&machine.p, "feed").map_or(f32::INFINITY, |v| v * inch);
    let vc = std::f32::consts::PI * d * n / 1000.;
    let fz = f / (n * z);
    let (vc_min, vc_max) = data.vc;
    let (fz_min, fz_max) = data.fz[band(d)];
    let name = material.name();
    let mut out = vec![];
    if vc > vc_max * 1.01 {
        out.push(format!("Surface speed {vc:.0} m/min is above {name}'s {vc_min:.0}–{vc_max:.0} m/min."));
    } else if vc < vc_min * 0.99 && n < max_rpm - 0.5 {
        out.push(format!("Surface speed {vc:.0} m/min is below {name}'s {vc_min:.0}–{vc_max:.0} m/min."));
    }
    if fz > fz_max * 1.01 {
        out.push(format!("Chip load {fz:.3} mm per tooth is above {name}'s {fz_min:.3}–{fz_max:.3} mm for a Ø{d} mm tool."));
    } else if fz < fz_min * 0.99 && f < max_feed - 0.5 {
        out.push(format!("Chip load {fz:.3} mm per tooth is below {name}'s {fz_min:.3}–{fz_max:.3} mm for a Ø{d} mm tool."));
    }
    if out.is_empty() { SpeedFeed::InRange { vc, fz } } else { SpeedFeed::Out(out) }
}
/// The starter library (spec table "Starter materials").
pub fn starters() -> Vec<Material> {
    let rows: [(&str, &str, &str, (f32, f32), [(f32, f32); 5], &str); 9] = [
        ("Aluminium 6061-T6", "Aluminium", "95 HB", (150., 300.), [(0.010, 0.030), (0.020, 0.050), (0.030, 0.080), (0.050, 0.100), (0.060, 0.120)], "Flood or mist; clear chips."),
        ("Aluminium 7075-T6", "Aluminium", "150 HB", (150., 280.), [(0.010, 0.025), (0.020, 0.045), (0.030, 0.070), (0.040, 0.090), (0.050, 0.110)], "Flood or mist; clear chips."),
        ("Mild steel 1018", "Steel", "126 HB", (80., 150.), [(0.005, 0.015), (0.010, 0.030), (0.020, 0.050), (0.030, 0.070), (0.040, 0.090)], "Flood; avoid rubbing at low feed."),
        ("Stainless steel 304", "Stainless steel", "201 HB", (50., 100.), [(0.005, 0.012), (0.010, 0.025), (0.015, 0.040), (0.020, 0.050), (0.030, 0.070)], "Flood; keep the tool cutting, it work-hardens."),
        ("Brass C360", "Brass and copper", "78 HRB", (150., 300.), [(0.010, 0.030), (0.020, 0.050), (0.030, 0.070), (0.040, 0.090), (0.050, 0.110)], "Dry or air; sharp tools."),
        ("POM (Delrin)", "Plastic", "", (150., 400.), [(0.020, 0.050), (0.040, 0.100), (0.060, 0.150), (0.080, 0.200), (0.100, 0.250)], "Air; avoid melting with high feed per tooth."),
        ("Acrylic (PMMA)", "Plastic", "", (100., 300.), [(0.020, 0.050), (0.030, 0.080), (0.050, 0.120), (0.060, 0.150), (0.080, 0.180)], "Air; single-flute tools give the best finish."),
        ("HDPE", "Plastic", "", (150., 400.), [(0.030, 0.070), (0.050, 0.120), (0.080, 0.180), (0.100, 0.220), (0.120, 0.250)], "Air; it deflects, use light finishing passes."),
        ("MDF", "Wood and composite", "", (200., 500.), [(0.030, 0.080), (0.050, 0.150), (0.080, 0.200), (0.100, 0.250), (0.120, 0.300)], "Dust extraction; no coolant."),
    ];
    rows.into_iter()
        .map(|(name, group, hardness, vc, fz, note)| {
            let mut m = Material::new(name);
            m.p.insert("group".into(), group.into());
            m.p.insert("hardness".into(), hardness.into());
            m.p.insert("notes".into(), format!("{STARTER_NOTE} {note}"));
            m.p.insert("vc_min".into(), vc.0.to_string());
            m.p.insert("vc_max".into(), vc.1.to_string());
            for ((key, _), (min, max)) in BANDS.iter().zip(fz) {
                m.p.insert(format!("{key}_min"), min.to_string());
                m.p.insert(format!("{key}_max"), max.to_string());
            }
            m
        })
        .collect()
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --locked --offline --quiet 2>&1 | tail -3`
Expected: N + 5 passed; 0 failed. Dead-code warnings for unused `material` items are expected until Task 2.

- [ ] **Step 6: Commit**

```bash
cargo fmt && git add src/material.rs src/model.rs src/main.rs && git commit -q -m "Material type, starter materials and the speed/feed check

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

(If other files needed `..Document::default()` for Step 3, add them to `git add`.)

---

### Task 2: The library in every document: adoption, migration, carrying over

**Files:**
- Modify: `src/model.rs` (`Document::adopt_materials`, `Document::selected_material`, `Document::select_material`), `src/storage.rs` (`load_reporting` returns notes), `src/main.rs` (callers of `load_reporting`, `NewProject`, `merge_sample`, app start), `src/smoke.rs` (tests), `src/post.rs` (`export_document` caller if it reads the bool)

**Interfaces:**
- Consumes: Task 1.
- Produces:
  - `Document::adopt_materials(&mut self) -> Option<String>` — adds every starter whose name (case-insensitive) is not in the library; then, if `material_id` is empty and `material` is not, links to the library material of that name or adds `Material::new(&self.material)` (group Other, no cutting data) and selects it. Returns the status note: `"The project material \"{name}\" was linked to the library."` or `"The project material \"{name}\" was added to the material library without cutting data; add it on the Materials page."`, or `None`. If `material_id` names a material that no longer exists, clears it (and keeps `material`).
  - `Document::selected_material(&self) -> Option<&Material>`.
  - `Document::select_material(&mut self, id: &str) -> bool` — sets `material_id` and `material` (the name) when the id exists.
  - `storage::load_reporting(path) -> Result<(Document, Vec<String>), String>` — notes: `NUMBERED_TOOLS` when tools were numbered, then the material note when any. `storage::load` unchanged.

- [ ] **Step 1: Write the failing tests** (in `src/model.rs` tests, or `src/material.rs` tests if model has none for documents):

```rust
#[test]
fn adopting_materials_adds_starters_and_links_legacy_names() {
    let mut doc = crate::model::Document { material: "aluminium 6061-t6".into(), ..Default::default() };
    let note = doc.adopt_materials().unwrap();
    assert_eq!(doc.materials.len(), 9);
    assert!(note.contains("linked to the library"), "{note}");
    assert_eq!(doc.selected_material().unwrap().name(), "Aluminium 6061-T6");
    assert_eq!(doc.material, "Aluminium 6061-T6");
    assert!(doc.adopt_materials().is_none(), "a second call changes nothing");
    assert_eq!(doc.materials.len(), 9);

    let mut odd = crate::model::Document { material: "Inconel 718".into(), ..Default::default() };
    let note = odd.adopt_materials().unwrap();
    assert!(note.contains("without cutting data"), "{note}");
    assert_eq!(odd.materials.len(), 10);
    assert!(odd.selected_material().unwrap().cutting_data().is_none());

    let mut none = crate::model::Document::default();
    assert!(none.adopt_materials().is_none());
    assert_eq!(none.materials.len(), 9);
    assert!(none.material_id.is_empty());

    let mut stale = none.clone();
    stale.material_id = "gone".into();
    stale.adopt_materials();
    assert!(stale.material_id.is_empty());
}
#[test]
fn selecting_a_material_sets_its_id_and_name() {
    let mut doc = crate::model::Document::default();
    doc.adopt_materials();
    let id = doc.materials[2].id.clone();
    assert!(doc.select_material(&id));
    assert_eq!(doc.material, "Mild steel 1018");
    assert!(!doc.select_material("missing"));
}
```

And in `src/smoke.rs` tests (use the existing temp-folder helpers there):
- `loading_a_project_with_a_legacy_material_links_it_and_says_so`: save (with `storage::save`) a document whose `material` is "Brass C360" and `materials` empty into a temp folder; `Message::ProjectOpened` path via the existing open helper (or call `storage::load_reporting` directly) — assert the notes contain the linked sentence and the status after opening contains it.
- `new_project_and_explore_example_carry_materials_without_duplicates`: after `adopt_materials`, add a user material "Titanium Gr5" (valid, no data); `NewProject` keeps 10 materials and no material selected; `OpenSample` (Explore example) keeps 10 materials (no duplicate names, case-insensitive) and the sample's material is selected if it matches a library name.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --locked --offline --quiet adopting_materials 2>&1 | tail -5`
Expected: compile errors (`adopt_materials`, `selected_material`, `select_material` not found).

- [ ] **Step 3: Implement** in `src/model.rs` `impl Document`:

```rust
    pub fn selected_material(&self) -> Option<&crate::material::Material> {
        self.materials.iter().find(|m| m.id == self.material_id)
    }
    pub fn select_material(&mut self, id: &str) -> bool {
        let Some(name) = self.materials.iter().find(|m| m.id == id).map(|m| m.name().to_string()) else {
            return false;
        };
        self.material_id = id.into();
        self.material = name;
        true
    }
    /// Gives the document the starter materials it lacks and links a legacy
    /// material name to the library. Returns the note to show, if any.
    pub fn adopt_materials(&mut self) -> Option<String> {
        let has = |list: &[crate::material::Material], name: &str| {
            list.iter().any(|m| m.name().eq_ignore_ascii_case(name))
        };
        for starter in crate::material::starters() {
            if !has(&self.materials, starter.name()) {
                self.materials.push(starter);
            }
        }
        if !self.material_id.is_empty() && self.selected_material().is_none() {
            self.material_id.clear();
        }
        let name = self.material.trim().to_string();
        if !self.material_id.is_empty() || name.is_empty() {
            return None;
        }
        if let Some(m) = self.materials.iter().find(|m| m.name().eq_ignore_ascii_case(&name)) {
            let id = m.id.clone();
            self.select_material(&id);
            return Some(format!("The project material \"{}\" was linked to the library.", self.material));
        }
        let added = crate::material::Material::new(&name);
        let id = added.id.clone();
        self.materials.push(added);
        self.select_material(&id);
        Some(format!("The project material \"{name}\" was added to the material library without cutting data; add it on the Materials page."))
    }
```

In `src/storage.rs` `load_reporting`: change the return type to `Result<(Document, Vec<String>), String>`; after `number_tools`, build `notes` (push `NUMBERED_TOOLS.to_string()` when it returned true; push the `adopt_materials()` note when `Some`). `load` keeps returning only the document. Update every caller the compiler reports (`main.rs` Open task, Recover previous, workspace start, `export_document`/`--export-gcode`): where they appended `NUMBERED_TOOLS` on `true`, append each note (space-separated) instead. Update the existing tests that asserted the bool (`storage::loading_reports_whether_tools_were_numbered` and the smoke/post tests that use it) to assert the notes contain or omit `NUMBERED_TOOLS`.

In `src/main.rs`:
- App start (the fresh-workspace path in `from_workspace`, where no file exists) and `NewProject`: `materials: self.doc.materials.clone()` like tools, then `doc.adopt_materials()` (so a blank workspace has the starters); `NewProject` leaves `material_id` empty.
- `merge_sample` (Explore example): add materials the way tools are merged, matching by name case-insensitively; keep the user's entry when names match; then `adopt_materials()` on the sample document so its "Aluminum 6061"-style legacy name links.

- [ ] **Step 4: Run the tests**

Run: `cargo test --locked --offline --quiet 2>&1 | tail -3`
Expected: N + 9 passed (Task 1's 5 + 4 new); 0 failed.

- [ ] **Step 5: Commit**

```bash
cargo fmt && git add src/model.rs src/storage.rs src/main.rs src/smoke.rs src/post.rs src/material.rs && git commit -q -m "Every document gets the material library; legacy material names are linked

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: The Materials page and the project material picker

**Files:**
- Modify: `src/main.rs` (`Page::Materials`, `Target::Material`, `material_draft`, messages), `src/ui.rs` (`materials` page, nav button, setup-editor picker), `src/help.rs` (help for new controls and fields), `src/smoke.rs` (tests)

**Interfaces:**
- Consumes: Tasks 1–2.
- Produces: `Page::Materials`; `Target::Material`; `App.material_draft: Material`; messages `NewMaterial`, `EditMaterial(String)`, `SaveMaterial`, `DeleteMaterial(String)`, `PickMaterial(String)` (an id); `Message::Material(String)` (free text) is removed.

- [ ] **Step 1: Write the failing app tests** in `src/smoke.rs` tests (temp-folder `App` helpers):

```rust
#[test]
fn materials_are_created_edited_saved_and_deleted_like_tools() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = fresh_app(&dir);
    let before = a.doc.materials.len();
    let _ = a.update(Message::NewMaterial);
    assert_eq!(a.page, Page::Materials);
    let _ = a.update(Message::Edit(Target::Material, "name", "Titanium Gr5".into()));
    let _ = a.update(Message::SaveMaterial);
    assert_eq!(a.doc.materials.len(), before + 1);
    // A duplicate name is refused.
    let _ = a.update(Message::NewMaterial);
    let _ = a.update(Message::Edit(Target::Material, "name", "titanium gr5".into()));
    let _ = a.update(Message::SaveMaterial);
    assert_eq!(a.doc.materials.len(), before + 1);
    assert!(a.status.contains("already"), "{}", a.status);
    // Partial cutting data is refused with the material's issue.
    let id = a.doc.materials.last().unwrap().id.clone();
    let _ = a.update(Message::EditMaterial(id.clone()));
    let _ = a.update(Message::Edit(Target::Material, "vc_min", "40".into()));
    let _ = a.update(Message::SaveMaterial);
    assert!(a.status.contains("every cutting-data field"), "{}", a.status);
    // The project material cannot be deleted while selected.
    let _ = a.update(Message::PickMaterial(id.clone()));
    assert_eq!(a.doc.material, "Titanium Gr5");
    let _ = a.update(Message::DeleteMaterial(id.clone()));
    assert!(a.doc.materials.iter().any(|m| m.id == id));
    assert!(a.status.contains("project material"), "{}", a.status);
    let other = a.doc.materials[0].id.clone();
    let _ = a.update(Message::PickMaterial(other));
    let _ = a.update(Message::DeleteMaterial(id.clone()));
    assert!(!a.doc.materials.iter().any(|m| m.id == id));
    a.page = Page::Materials;
    let _ = a.view();
}
```

(If the helper is named differently, use the existing temp-folder `App` constructor in `src/smoke.rs`.) Extend `help::tests` so `NewMaterial`, `SaveMaterial`, `DeleteMaterial`, `PickMaterial` and `Page(Materials)` have specific help (not the generic text).

- [ ] **Step 2: Run to see it fail** — `cargo test --locked --offline --quiet materials_are_created 2>&1 | tail -5` → compile errors.

- [ ] **Step 3: Implement**, mirroring the tool library exactly (see `Message::NewTool/EditTool/SaveTool/DeleteTool` in `src/main.rs` and `fn tools` in `src/ui.rs`):
- `App.material_draft` initialised like `tool_draft` (first library material, else `Material::new("New material")`).
- `Message::Edit(Target::Material, k, v)` writes into `material_draft.p`.
- `NewMaterial`: draft `Material::new("New material")`, page Materials.
- `SaveMaterial`: refuse with `issues()` joined, or "A material named X is already in the library." when another material has the same name (case-insensitive); else insert or replace, keep `doc.material` in sync if the saved material is selected, `touch()`, status "Material saved to the library.".
- `DeleteMaterial(id)`: refuse while `doc.material_id == id` ("This is the project material. Choose another project material before deleting it."); else remove, `touch()`.
- `PickMaterial(id)`: `doc.select_material(&id)`, `touch()`.
- `ui::materials(app)`: page header "Materials", subtitle "Cutting data are for carbide tools and are starting points.", list `btn("{name}\n{group} · Vc {vc_min}–{vc_max} m/min", EditMaterial(id))` (or "no cutting data"), editor with `field`s for name, hardness, notes, `choice` for group over `material::GROUPS`, then a "Surface speed · m/min" row (vc_min, vc_max) and five "Chip load per tooth · mm" rows labelled "Ø up to 3", "Ø 3–6", "Ø 6–10", "Ø 10–16", "Ø over 16" (min, max each), `warnings(issues)`, **Save material**, **Delete material**. Add a **Materials** button next to **Tools** in the document bar and `Page::Materials => materials(app)` in the page match.
- Setup editor: replace the "Project material" `text_input` with a `pick_list` of library material names (selecting sends `PickMaterial(id)`), a "Open material library" button (`Page(Materials)`), and under it a muted note when the selected material has no cutting data ("X has no cutting data: speeds and feeds are not checked."). Update its help text: "Material from the library. Its cutting data are used to check speeds and feeds and are sent to the AI planner."
- Remove `Message::Material(String)` and its handler.

- [ ] **Step 4: Run the tests** — `cargo test --locked --offline --quiet 2>&1 | tail -3` → 0 failed.

- [ ] **Step 5: Commit**

```bash
cargo fmt && git add src/main.rs src/ui.rs src/help.rs src/smoke.rs && git commit -q -m "Materials page and the project material picker

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Speed and feed warnings for the user's operations

**Files:**
- Modify: `src/main.rs` or `src/model.rs` (`Document::speed_feed(&self, setup, op) -> SpeedFeed`), `src/ui.rs` (operation editor warning, NC page list), `src/smoke.rs` (tests)

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces: `Document::speed_feed(&self, setup: &Setup, op: &Operation) -> Option<material::SpeedFeed>` (None when no material is selected or the operation has no tool in the library); `ui::speed_feed_lines(app) -> Vec<String>` (the NC page's list: "Setup 1 · Pocket: Surface speed … ", one line per problem; tested).

- [ ] **Step 1: Failing tests** in `src/smoke.rs`: build the fixture job (`crate::post::fixture::job()`), adopt materials, select "Aluminium 6061-T6", set one operation's rpm to 30000 (above the machine max is irrelevant: check uses the op's value) — assert `doc.speed_feed(..)` is `Out` with "Surface speed"; `ui::speed_feed_lines` contains "Setup 1" and the operation name; `post::post_job` is still ready (warnings never block export); with "Unknown" material (no data) the line says it is not checked; the operation editor view builds with the warning (`a.view()` on the Work page with that operation selected).

- [ ] **Step 2: Run to fail.**

- [ ] **Step 3: Implement:**

```rust
    pub fn speed_feed(&self, setup: &Setup, op: &Operation) -> Option<crate::material::SpeedFeed> {
        let material = self.selected_material()?;
        let tool = self.tools.iter().find(|t| t.id == op.tool)?;
        Some(crate::material::check(material, tool, op, &setup.machine))
    }
```

In the operation editor (where the operation's rpm/feed fields are shown in `src/ui.rs`), push `warnings(problems)` for `Out(problems)`, a muted line for `Unchecked(reason)`, and nothing for `InRange`. On the NC program page, under the export note, list `speed_feed_lines(app)` as warnings with the heading "Speeds and feeds outside the material's cutting data (export still allowed)".

- [ ] **Step 4: Tests pass; Step 5: commit** (`src/model.rs src/ui.rs src/smoke.rs`, message "Warn when an operation's speed or feed is outside the material's cutting data").

---

### Task 5: The planner uses the material

**Files:**
- Modify: `src/analysis.rs` (`MaterialReport`, `ToolReport.flutes`, preconditions), `src/plan_check.rs` (cutting-data stage), `src/planner.rs` (instructions), `src/ui.rs` (not-checked note), tests in those files

**Interfaces:**
- Consumes: Tasks 1–4.
- Produces: `analysis::MaterialReport { name, group, hardness, notes, vc_min_m_per_min, vc_max_m_per_min, chip_load_mm: Vec<ChipLoadBand { up_to_mm: Option<f32>, min, max }> }`; `PartReport.material: MaterialReport`; `ToolReport.flutes: u32`; plan-check stage `"cutting data"`.

- [ ] **Step 1: Failing tests**:
  - `analysis`: on the example block with "Aluminium 6061-T6" selected, `report().material.vc_min_m_per_min == 150.`, five bands, the last `up_to_mm == None`; notes are clipped (`proposal::clip(…, 500)`); the tool report has flutes. The precondition line "Material" is unmet with no material ("Choose a material on the setup page.") and with a material without cutting data ("Add cutting data to X on the Materials page.").
  - `plan_check`: `good()` with every operation's rpm/feed in range passes (adjust `proposal::tests::good` values if needed so the example passes with Aluminium 6061-T6: 3-flute Ø10 at 8000 rpm gives Vc 251, fz = F/24000 → feed 1200 gives 0.05 in 0.03–0.08); setting one operation's rpm to 2000 is refused at stage `"cutting data"` naming setup and operation and "Surface speed".
  - `planner`: `instructions()` contains "surface speed", "chip load", "π" or "pi", and "outside the material's cutting data are refused".

- [ ] **Step 2: Run to fail.**

- [ ] **Step 3: Implement:**
  - `analysis::report`: build `MaterialReport` from `doc.selected_material()` (preconditions guarantee cutting data); strings through `proposal::clip` (name 60, group 40, hardness 40, notes 500).
  - Preconditions: the "Material" line requires `selected_material()` with `cutting_data()`.
  - `plan_check::check_built`: after `validate_structure`, for each setup/operation call `doc.speed_feed(setup, op)`; each `Out(problems)` → `Problem { setup, operation: Some(oi+1), stage: "cutting data", message }` (one per problem). This stage does not stop generation (like the tool-axis stage it reports alongside), so one round of feedback carries every problem.
  - `planner::instructions`: add a section "SPEEDS AND FEEDS": surface speed Vc = π × D × rpm / 1000 m/min and chip load fz = feed / (rpm × flutes) mm per tooth must lie in the report's `material` ranges (the band by tool diameter); running at the machine's maximum rpm or feed counts as in range when the material asks for more; plans outside are refused.
  - `ui`: the not-checked note becomes the spec's sentence; update its test.
  - `SENT_TO_OPENAI` sentence and README "What is sent" mention the material's cutting data.

- [ ] **Step 4: Tests pass; Step 5: commit** (`src/analysis.rs src/plan_check.rs src/planner.rs src/proposal.rs src/ui.rs src/smoke.rs`, message "The planner gets the material's cutting data; plans outside it are refused").

---

### Task 6: The planner progress timeline

**Files:**
- Modify: `src/planner.rs` (`Event`, `StepResult`, `plan`/`plan_with` emit events, `OpenAiBackend.received`), `src/openai.rs` (`respond_counting`, streamed length), `src/main.rs` (`PlanProgress` carries `Event`, App `plan_timeline`, `plan_received`), `src/ui.rs` (timeline view running and finished), `src/smoke.rs` (tests)

**Interfaces:**
- Produces:
  - `planner::Event { Stage(Progress), Checked(usize, StepResult) }` and `planner::StepResult { Passed, Problems { shown: Vec<String>, more: usize }, Unreadable(String) }` — at most 5 problem lines, each `proposal::clip(…, 300)`; `Unreadable` carries the parse error clipped to 300.
  - `plan`/`plan_with` take `progress: impl FnMut(Event) + Send` (was `FnMut(Progress)`): `Stage` when a step starts (as before), `Checked(n, …)` after each check and after each unreadable reply (attached to request n).
  - `openai::Client::respond_counting(&self, request, received: &AtomicUsize) -> Result<Reply, ApiError>` (and the plain variant): resets `received` to 0, then adds the length (in chars) of every streamed `output_text.delta`; `respond` delegates with a throwaway counter.
  - `planner::OpenAiBackend.received: Arc<AtomicUsize>`.
  - `App.plan_timeline: Vec<Step>` with `Step { stage: Progress, started: Instant, ended: Option<Instant>, result: Option<StepResult> }`, `App.plan_received: Arc<AtomicUsize>`; cleared by `drop_plan`; kept after `PlanFinished`.

- [ ] **Step 1: Failing tests**:
  - planner: update the existing stage-sequence assertions by collecting `Event::Stage` only (helper `fn stages(events: &[Event]) -> Vec<Progress>`); add `a_run_reports_each_check_result`: Script `[bad, good, good]` → events contain `Checked(1, Problems{shown: ["… T9 …"], more: 0})`, `Checked(2, Passed)`; `an_unreadable_reply_is_reported`: Script `["Here is my plan!", good]` → `Checked(1, Unreadable(..))`; problems over 5 → `more` counts the rest.
  - openai (loopback fake): `respond_counting` with a reply streamed in three deltas "{" + 2000 × "x" … → after the call `received == total chars`; counter reset to 0 at the start of a second call.
  - app (smoke tests): `Plan` then `PlanProgress(epoch, Event::Stage(Asking(1)))` adds a step with `ended == None`; a following `Stage(Checking(1))` ends it; `Checked(1, Passed)` attaches to the Checking step and ends it; an event from an older epoch changes nothing; `PlanFinished(Ok)` keeps the timeline; `DiscardPlan`/an edit (`touch`) clears it; the planner page builds while running and after finishing with the timeline.

- [ ] **Step 2: Run to fail.**

- [ ] **Step 3: Implement:**
  - In `planner::plan_with`: replace `progress(stage)` calls with `progress(Event::Stage(stage))`; after `plan_check::check` returns, emit `Checked(request, if check.passed { Passed } else { Problems{..} })` using the check's problems (format each as "Setup s, operation o: message" like the failed panel); on a parse failure emit `Checked(request, Unreadable(clip(error, 300)))`.
  - `run_plan_with`: forward events as `Message::PlanProgress(epoch, event)`; `Previewing` as `Event::Stage(Previewing)`.
  - `OpenAiBackend::reply` calls `respond_counting(&request, &self.received)` (and the plain variant).
  - In `openai::Client::attempt`, take `received: &AtomicUsize`; reset at the start; after `reply.apply(&event)`, add the delta's char count (have `Accumulator::apply` return the number of chars it appended for `output_text.delta`).
  - App: on `Plan`, `plan_timeline.clear()`, a fresh `plan_received` passed to the backend; on `PlanProgress(e, Stage(s))` with the current epoch: end the last open step (`ended = Some(now)`), push a new `Step`; on `Checked(n, r)`: attach to the last `Checking(n)` step (or the last `Asking/Repairing/Extending(n)` step for `Unreadable`), end it; on `PlanFinished`: end the last open step; `drop_plan` clears both.
  - UI (`plan_result` Running arm, and above the preview and the failure): a column of rows `"{tick or •} {stage_text} · {elapsed:.0} s"`; for the open request step add `"received {n} characters"` from `plan_received`; under a check step: "passed", or the problem lines (muted) plus "and {more} more", or "The reply could not be read: …". Finished: heading "How the plan was made" above the preview, "What the planner tried" above a failure, rows without spinners. Elapsed times use `Instant` differences; the existing 500 ms `Tick` redraws while running.
  - Renderer smoke steps: set a timeline with 4 steps (one open request with a received count) for screen 84; add a step after the ready screen that shows a finished timeline above the preview (new screen `86-planner-timeline`). Keep the existing screen names.

- [ ] **Step 4: Tests pass; Step 5: commit** (`src/planner.rs src/openai.rs src/main.rs src/ui.rs src/smoke.rs`, message "AI planner progress: a live timeline with each check's result").

---

### Task 7: Documentation, renderer check and totals

**Files:**
- Modify: `README.md`, `docs/VALIDATION.md`, `docs/superpowers/specs/2026-10-04-materials-and-planner-progress-design.md` (only if the implementation differs; record how)

- [ ] **Step 1: README**: a "Materials" section (the library, starters marked as starting points, migration of old projects, the picker), the speed/feed check (formulas, bands, machine-limit exception, warnings vs planner refusal, ball-mill note), the planner progress timeline; update "What is sent to OpenAI" and the AI planner "Limits"; code map entry for `src/material.rs`.
- [ ] **Step 2: VALIDATION**: a "Materials and speed/feed checks" section and a "Planner progress" section, each sentence naming the test behind it; update totals.
- [ ] **Step 3: Renderer smoke run** into the untracked `.superpowers/` folder (it opens a window for several minutes and closes itself):

```bash
cargo build --release --locked --offline 2>&1 | tail -1
rm -rf .superpowers/smoke-materials && "$CARGO_TARGET_DIR/release/axis-cam" --smoke-test "$PWD/.superpowers/smoke-materials" > .superpowers/smoke-materials.log 2>&1; echo "exit $?"
ls .superpowers/smoke-materials | grep -c png
```

Expected: `exit 0`; one more PNG than before (the new timeline screen) plus any Materials-page screen added in Task 3. Look at the planner screens (84, 86) and the Materials page screen; confirm nothing is clipped or overlapping. Record the counts in README and VALIDATION.
- [ ] **Step 4: Verify** — `cargo fmt --check && cargo test --locked --offline --quiet 2>&1 | tail -2`.
- [ ] **Step 5: Commit** (`README.md docs/VALIDATION.md` and the spec if changed; message "Document the material library, speed/feed checks and planner progress").
