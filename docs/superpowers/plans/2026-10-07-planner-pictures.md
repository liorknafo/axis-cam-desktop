# Planner pictures, live progress and common aluminium alloys — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The AI planner sees labelled pictures of the part (an overview plus a `view_model` tool it calls itself), the planner page shows what the model writes and looks at while it runs, and the material library gains six common, cheap aluminium alloys.

**Architecture:** The viewport's CPU rasterizer is split so a scene can be drawn to pixels off-screen; a new `src/look.rs` turns that into labelled 768 × 768 PNGs. The OpenAI client carries conversation *items* (messages with text and image parts, function calls and their outputs) and streams function calls; the planner runs a tool loop inside each of its four requests, with limits and step-by-step fallbacks. The app keeps the streamed text instead of a character count and reads it with a tolerant JSON reader for the timeline.

**Tech Stack:** Rust 2024, Iced 0.14 (canvas, image), serde/serde_json, reqwest SSE streaming, `png` 0.17, `base64` 0.23, tokio tests with the in-repo fake OpenAI server.

**Spec:** `docs/superpowers/specs/2026-10-07-planner-pictures-design.md` (the aluminium alloys are the small separate change agreed in chat; Task 1).

## Global Constraints

- Pictures: 768 × 768 PNG, sent as `data:image/png;base64,…` with `detail` `high`.
- Tool name `view_model`; fields `orientation`, `angle`, `focus_surfaces`, `highlight_surfaces`, `show_stock`, `show_workholding`, `labels`, `operation`, `purpose` — strict schema, every field required.
- Angles: `top`, `front`, `back`, `left`, `right`, `iso_front_left`, `iso_front_right`, `iso_back_left`, `iso_back_right`.
- Limits: 6 pictures per planner request, 20 per run, 8 most recent pictures re-sent; at most 8 tool rounds per planner request.
- Text placeholders, verbatim: `(Earlier picture, not resent: <description>)`, `(Picture not sent: <description>)`, `No more pictures for this request; reply with the proposal.`, `No more pictures in this run; reply with the proposal.`
- Fallback order on `Unsupported` or HTTP 400 without a known code: tool, then pictures, then (inside the client, as today) structured output.
- Every text the model wrote is shown through `proposal::clip` (one line, bounded); `purpose` at most 120 characters, the summary line at most 300.
- Setting: `Settings.plan_pictures_off: bool` (false by default ⇒ pictures on). Checkbox label: "Let the AI see pictures of the part".
- Requests keep `store: false`; replayed function calls carry no item `id`; reasoning items are not replayed.
- The material library gains six starters (Task 1 table), added once to an existing library through `Document.starter_materials`.
- `cargo test` passes and `cargo clippy --all-targets` adds no warnings at the end of every task. Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **A model that keeps calling the tool** after `tool_choice` is `none` (or that answers with calls forever): the request must end with an error after 8 tool rounds, never loop or hang. Test: Task 6 `a_model_that_never_stops_looking_fails_the_request`.
2. **Malformed tool arguments** (not JSON, missing or extra fields, unknown angle, negative or out-of-range IDs, 10 000 IDs, an operation before any proposal): a one-sentence error to the model, no panic. Test: Task 4 `bad_arguments_get_one_sentence_errors`.
3. **Pictures refused after some were sent:** every later send carries no image at all, old ones included, and the system message loses its PICTURES section. Test: Task 6 `refused_pictures_are_dropped_from_the_whole_conversation`.
4. **A huge or hostile streamed reply** (4 MB of text, deep nesting, a cut escape): the timeline reader returns quickly and never panics. Test: Task 7 `partial_is_bounded_and_never_panics`.
5. **A mirrored picture:** the viewport's projection maps right × up to the direction *away* from the viewer, so a raw frame is a mirror image; a picture must keep +X right and +Y up in a top view. Test: Task 4 `a_top_view_keeps_x_right_and_y_up`.

---

### Task 1: Six common aluminium alloys in the material library

**Files:**
- Modify: `src/material.rs` (starter rows, `ADDED_IN_SET_2`, `STARTER_SET`, test)
- Modify: `src/model.rs:850-890` (`Document.starter_materials`), `src/model.rs:977-1010` (`adopt_materials`), tests near `src/model.rs:1640-1790`
- Modify: `src/main.rs:1340-1350` (New project), `src/main.rs:3095-3125` (`merge_sample`)
- Modify: `README.md:40`, `README.md:253`, `docs/VALIDATION.md:83,90,94`

**Interfaces:**
- Produces: `material::STARTER_SET: u32 = 2`, `material::ADDED_IN_SET_2: [&str; 6]`, `Document.starter_materials: u32` (serde default 0).

- [ ] **Step 1: Write the failing tests**

In `src/material.rs` tests, replace `starters_are_valid_and_complete`'s counts and add a test:

```rust
    #[test]
    fn starters_are_valid_and_complete() {
        let all = starters();
        assert_eq!(all.len(), 15);
        for m in &all {
            assert!(m.issues().is_empty(), "{}: {:?}", m.name(), m.issues());
            assert!(m.cutting_data().is_some(), "{}", m.name());
            assert!(get(&m.p, "notes").starts_with(STARTER_NOTE));
            assert!(GROUPS.contains(&get(&m.p, "group")));
        }
        let mut names: Vec<_> = all.iter().map(|m| m.name().to_lowercase()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 15);
        let data = alu().cutting_data().unwrap();
        assert_eq!(data.vc, (150., 300.));
        assert_eq!(data.fz[1], (0.02, 0.05));
    }
    #[test]
    fn the_second_starter_set_is_six_common_aluminium_alloys() {
        let all = starters();
        for name in ADDED_IN_SET_2 {
            let m = all.iter().find(|m| m.name() == name).expect(name);
            assert_eq!(get(&m.p, "group"), "Aluminium", "{name}");
        }
        let alloy = |name: &str| all.iter().find(|m| m.name() == name).unwrap().cutting_data().unwrap();
        assert_eq!(alloy("Aluminium 6082-T6").vc, (150., 300.));
        assert_eq!(alloy("Aluminium 5083-H111").vc, (120., 250.));
        assert_eq!(alloy("Aluminium 1050A-H14").fz[0], (0.010, 0.025));
        // The first nine come first, in their old order.
        assert_eq!(all[0].name(), "Aluminium 6061-T6");
        assert_eq!(all[8].name(), "MDF");
    }
```

In `src/model.rs` tests, add:

```rust
    #[test]
    fn an_existing_library_gets_the_new_alloys_once() {
        let mut doc = Document {
            materials: vec![crate::material::Material::new("My alloy")],
            ..Document::default()
        };
        assert!(doc.adopt_materials().is_none());
        let names: Vec<_> = doc.materials.iter().map(|m| m.name().to_string()).collect();
        assert_eq!(names.len(), 7, "{names:?}");
        assert!(names.contains(&"Aluminium 6082-T6".to_string()));
        assert!(!names.contains(&"Aluminium 6061-T6".to_string()));
        assert_eq!(doc.starter_materials, crate::material::STARTER_SET);
        // Deleted after the addition: it does not come back.
        doc.materials.retain(|m| m.name() != "Aluminium 6063-T6");
        doc.adopt_materials();
        assert!(!doc.materials.iter().any(|m| m.name() == "Aluminium 6063-T6"));
        // A name the user already has (any case) is not added twice.
        let mut mine = Document {
            materials: vec![crate::material::Material::new("aluminium 5083-h111")],
            ..Document::default()
        };
        mine.adopt_materials();
        assert_eq!(
            mine.materials.iter().filter(|m| crate::material::same_name(m.name(), "Aluminium 5083-H111")).count(),
            1
        );
        // An empty library gets all fifteen and the current set.
        let mut empty = Document::default();
        empty.adopt_materials();
        assert_eq!((empty.materials.len(), empty.starter_materials), (15, 2));
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test --bin axis-cam material:: model::tests::an_existing_library 2>&1 | tail -20`
Expected: compile error (`ADDED_IN_SET_2`, `starter_materials` unknown).

- [ ] **Step 3: Implement**

In `src/material.rs`, after `STARTER_NOTE`:

```rust
/// The starter set a library has been given: 1 is the first nine, 2 adds
/// six common aluminium alloys. A library is brought up to it once.
pub const STARTER_SET: u32 = 2;
/// The starters set 2 added, by name.
pub const ADDED_IN_SET_2: [&str; 6] = [
    "Aluminium 6082-T6",
    "Aluminium 6063-T6",
    "Aluminium 5083-H111",
    "Aluminium 5052-H32",
    "Aluminium 1050A-H14",
    "Cast aluminium plate (MIC-6)",
];
```

In `starters()`, change the array length from `9` to `15` and append after the MDF row:

```rust
        (
            "Aluminium 6082-T6",
            "Aluminium",
            "95 HB",
            (150., 300.),
            [
                (0.010, 0.030),
                (0.020, 0.050),
                (0.030, 0.080),
                (0.050, 0.100),
                (0.060, 0.120),
            ],
            "Flood or mist; clear chips. The common European plate and bar alloy; machines like 6061.",
        ),
        (
            "Aluminium 6063-T6",
            "Aluminium",
            "73 HB",
            (150., 300.),
            [
                (0.010, 0.030),
                (0.020, 0.050),
                (0.030, 0.080),
                (0.050, 0.100),
                (0.060, 0.120),
            ],
            "Flood or mist; cheap extrusion alloy, softer and gummier than 6061: sharp, polished tools.",
        ),
        (
            "Aluminium 5083-H111",
            "Aluminium",
            "75 HB",
            (120., 250.),
            [
                (0.010, 0.025),
                (0.020, 0.045),
                (0.030, 0.070),
                (0.040, 0.090),
                (0.050, 0.110),
            ],
            "Flood; gummy plate alloy: sharp, polished tools and clear chips.",
        ),
        (
            "Aluminium 5052-H32",
            "Aluminium",
            "60 HB",
            (120., 250.),
            [
                (0.010, 0.025),
                (0.020, 0.045),
                (0.030, 0.070),
                (0.040, 0.090),
                (0.050, 0.110),
            ],
            "Flood; gummy sheet alloy: sharp, polished tools and clear chips.",
        ),
        (
            "Aluminium 1050A-H14",
            "Aluminium",
            "34 HB",
            (100., 250.),
            [
                (0.010, 0.025),
                (0.015, 0.040),
                (0.025, 0.060),
                (0.035, 0.080),
                (0.045, 0.100),
            ],
            "Flood with lubricant; very soft and gummy: sharp, polished one- or two-flute tools.",
        ),
        (
            "Cast aluminium plate (MIC-6)",
            "Aluminium",
            "65 HB",
            (150., 300.),
            [
                (0.010, 0.030),
                (0.020, 0.050),
                (0.030, 0.080),
                (0.050, 0.100),
                (0.060, 0.120),
            ],
            "Mist or air; stable cast tooling plate that stays flat.",
        ),
```

In `src/model.rs`, add to `Document` after `material_id`:

```rust
    /// The starter set (`material::STARTER_SET`) the library has been
    /// given; 0 for a library saved before sets were counted.
    #[serde(default)]
    pub starter_materials: u32,
```

and `starter_materials: 0,` in `impl Default for Document`. In `adopt_materials`, replace the first `if`:

```rust
        if self.materials.is_empty() {
            self.materials = crate::material::starters();
        } else if self.starter_materials < 2 {
            for m in crate::material::starters()
                .into_iter()
                .filter(|m| crate::material::ADDED_IN_SET_2.contains(&m.name()))
            {
                if !self
                    .materials
                    .iter()
                    .any(|x| crate::material::same_name(x.name(), m.name()))
                {
                    self.materials.push(m);
                }
            }
        }
        self.starter_materials = crate::material::STARTER_SET;
```

Update the doc comment above it: "Gives a document with an empty library the starter materials, brings an older library up to the current starter set once, and links …".

In `src/main.rs` `Message::NewProject`, add `starter_materials: self.doc.starter_materials,` beside `materials:`. In `merge_sample`, before `let material_note = doc.adopt_materials();` add `doc.starter_materials = user.starter_materials;`.

- [ ] **Step 4: Run the whole suite and fix count expectations**

Run: `cargo test 2>&1 | grep -E "^test .* FAILED|panicked|test result" | head -40`
Expected: the new tests pass; tests that counted nine starters or "a tenth material" fail. Find them with `grep -n "len(), 9\|nine\|tenth\|gets_no_starters" src/*.rs` and update each expectation to fifteen starters / a sixteenth material. `model::tests::a_library_with_entries_gets_no_starters` becomes: a library with one entry and `starter_materials: 2` gets none (set the field in that test's document); keep its legacy-name assertions. Re-run until `test result: ok` for every target.

- [ ] **Step 5: Update README and VALIDATION**

`README.md:40`: "nine starter materials" → "fifteen starter materials". `README.md:253`: list the six new names after MDF and add: "A library saved before these six were added gets them once, unless it already has a material of that name (capitals ignored); deleting one afterwards is final." `docs/VALIDATION.md:83`: "nine" → "fifteen", name `material::tests::the_second_starter_set_is_six_common_aluminium_alloys`; `:90` and `:94`: nine → fifteen, tenth → sixteenth, and add a sentence naming `model::tests::an_existing_library_gets_the_new_alloys_once`.

- [ ] **Step 6: Commit**

```bash
git add src/material.rs src/model.rs src/main.rs README.md docs/VALIDATION.md
git commit -m "Six common aluminium alloys in the material library, added once to older libraries

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Draw a viewport scene off-screen

**Files:**
- Modify: `src/viewport.rs:135-240` (`Projector`, `projector`), `src/viewport.rs:280-470` (`pixels`)
- Test: `src/viewport.rs` tests module (create `#[cfg(test)] mod tests` at the end if none)

**Interfaces:**
- Produces:
  - `pub(crate) struct Projector` with `pub(crate) scale: f32`, `pub(crate) fn fit(camera: Camera, size: (f32, f32), bounds: crate::mesh::Bounds) -> Projector`, `pub(crate) fn p(&self, v: [f32; 3]) -> Point`, `pub(crate) fn depth(&self, v: [f32; 3]) -> f32`.
  - `impl Viewport { pub(crate) fn scene_bounds(&self) -> Bounds; pub(crate) fn render(&self, p: &Projector, (w, h): (u32, u32), ratio: f32, hovered: Option<usize>, marked: &[usize]) -> (Vec<u8>, Vec<f32>) }` — RGBA (alpha 0 where nothing was drawn) and the part's depth buffer.
  - Jaws and clamps (`holding`) are drawn whenever the list is non-empty (callers already pass an empty list when stock is hidden).

- [ ] **Step 1: Write the characterisation test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    /// FNV-1a over the pixels, to pin the drawing across the refactor.
    fn digest(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf29ce484222325, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
    }
    #[test]
    fn the_viewport_draws_the_same_pixels_after_the_split() {
        let (doc, mesh) = crate::post::fixture::job();
        let setup = doc.setups.first();
        let view = Viewport {
            setup,
            operation: None,
            mesh: &mesh,
            simulation: None,
            sample: false,
            camera: Camera::default(),
            picking: false,
            show_stock: true,
            show_paths: false,
            progress: None,
            holding: vec![],
        };
        let bounds = Rectangle { x: 0., y: 0., width: 400., height: 300. };
        let p = view.projector(bounds);
        let (w, h, rgba) = view.pixels(&p, bounds, None);
        assert_eq!((w, h), (600, 450));
        assert_eq!(digest(&rgba), 0); // replace 0 with the printed value, see Step 2
    }
}
```

- [ ] **Step 2: Record the digest before refactoring**

Run: `cargo test --bin axis-cam viewport::tests 2>&1 | grep -A3 "assertion"`
Expected: FAIL showing `left: <N>`. Put `<N>` in place of `0` and re-run: PASS. This pins the current drawing.

- [ ] **Step 3: Split `projector` and `pixels`**

Make `Projector` and its methods `pub(crate)`; add:

```rust
impl Projector {
    /// The camera fitted to `bounds` (display units) in a `size` frame: the
    /// bounds' centre at the frame's centre, their bounding sphere filling it.
    pub(crate) fn fit(camera: Camera, size: (f32, f32), bounds: crate::mesh::Bounds) -> Self {
        let frame = Rectangle { x: 0., y: 0., width: size.0, height: size.1 };
        let mut p = Projector::new(camera, frame);
        p.target = std::array::from_fn(|i| bounds.min[i] + (bounds.max[i] - bounds.min[i]) / 2.);
        let diameter = bounds.dimensions().iter().map(|v| v * v).sum::<f32>().sqrt().max(1.);
        p.scale = (size.0 * 0.86).min(size.1 * 0.82) / diameter * camera.zoom;
        p
    }
}
```

Move the bounds computation of `Viewport::projector` into `pub(crate) fn scene_bounds(&self) -> crate::mesh::Bounds` (unchanged logic, except the holding loop runs when `!self.holding.is_empty()` instead of `if self.show_stock`), and make `projector` `Projector::fit(self.camera, (b.width, b.height), self.scene_bounds())`.

Rename the body of `pixels` after the `w`/`h`/`ratio` lines into:

```rust
    /// The scene drawn into a `w` × `h` frame whose points are the
    /// projector's times `ratio`: RGBA (alpha 0 where nothing was drawn) and
    /// the part's depth buffer. `marked` surfaces take the selection tint.
    pub(crate) fn render(
        &self,
        p: &Projector,
        (w, h): (u32, u32),
        ratio: f32,
        hovered: Option<usize>,
        marked: &[usize],
    ) -> (Vec<u8>, Vec<f32>) {
```

with `selected` extended to `… || face.feature.is_some_and(|f| marked.contains(&f))`, the jaws loop guarded by `!self.holding.is_empty()` (not `self.show_stock`), and a snapshot of `depths` taken right after the part faces loop (`let part_depths = depths.clone();`) returned as the second value. `pixels` becomes:

```rust
    fn pixels(&self, p: &Projector, bounds: Rectangle, hovered: Option<usize>) -> (u32, u32, Vec<u8>) {
        let ratio = (1600. / bounds.width.max(1.)).min(1200. / bounds.height.max(1.)).min(1.5);
        let w = (bounds.width * ratio).ceil().max(1.) as u32;
        let h = (bounds.height * ratio).ceil().max(1.) as u32;
        let (rgba, _) = self.render(p, (w, h), ratio, hovered, &[]);
        (w, h, rgba)
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --bin axis-cam viewport:: 2>&1 | tail -5` then `cargo test 2>&1 | grep "test result"`
Expected: the digest test passes unchanged; every suite `ok`.

- [ ] **Step 5: Commit**

```bash
git add src/viewport.rs
git commit -m "Viewport: draw a scene to pixels without the canvas

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Pixel font, tags, caption and PNG for pictures

**Files:**
- Create: `src/look.rs` (first part: drawing primitives)
- Modify: `src/main.rs` (`mod look;` beside the other `mod` lines)

**Interfaces:**
- Produces (all `pub(crate)` in `look`):
  - `struct Canvas { pub w: u32, pub h: u32, pub rgb: Vec<u8> }`
  - `fn Canvas::over(rgba: &[u8], w: u32, h: u32, flip: bool) -> Canvas` — composite on the viewport background `[239, 243, 247]`, mirrored left–right when `flip`.
  - `fn Canvas::text_width(text: &str, scale: u32) -> u32`
  - `fn Canvas::text(&mut self, x: i32, y: i32, text: &str, scale: u32, color: [u8; 3])`
  - `fn Canvas::tag(&mut self, center: (f32, f32), text: &str) -> Option<[i32; 4]>` — white box, dark border, scale-2 text centred on `center`; returns its rectangle `[x0, y0, x1, y1]`, None when it would leave the picture.
  - `fn Canvas::fill(&mut self, rect: [i32; 4], color: [u8; 3])`, `fn Canvas::line(&mut self, a: (f32, f32), b: (f32, f32), color: [u8; 3], width: f32)`
  - `fn Canvas::png(&self) -> Vec<u8>`

- [ ] **Step 1: Write the failing tests** (at the end of `src/look.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_character_a_tag_or_caption_uses_has_a_glyph() {
        for c in "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ +-.,:/()Ø×%…·".chars() {
            assert!(glyph(c).is_some(), "{c}");
        }
        assert!(glyph('ä').is_none());
        // Lower case is drawn as capitals; anything else as "?".
        let mut a = Canvas::blank(40, 20);
        let mut b = Canvas::blank(40, 20);
        a.text(0, 0, "ab", 1, [0, 0, 0]);
        b.text(0, 0, "AB", 1, [0, 0, 0]);
        assert_eq!(a.rgb, b.rgb);
    }
    #[test]
    fn a_tag_is_drawn_centred_and_refused_at_the_edge() {
        let mut c = Canvas::blank(200, 100);
        let rect = c.tag((100., 50.), "12").unwrap();
        assert!(rect[0] < 100 && rect[2] > 100 && rect[1] < 50 && rect[3] > 50);
        let dark = (rect[0]..rect[2]).any(|x| c.at(x as u32, 50) == [40, 46, 54]);
        assert!(dark, "the text is drawn inside the box");
        assert!(c.tag((2., 50.), "12").is_none());
    }
    #[test]
    fn composite_flips_and_fills_the_background() {
        // A 2 × 1 frame: left pixel red and opaque, right pixel empty.
        let rgba = [255, 0, 0, 255, 0, 0, 0, 0];
        let c = Canvas::over(&rgba, 2, 1, true);
        assert_eq!(c.at(0, 0), [239, 243, 247]);
        assert_eq!(c.at(1, 0), [255, 0, 0]);
        let png = c.png();
        assert_eq!(&png[1..4], b"PNG");
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --bin axis-cam look:: 2>&1 | tail -5`
Expected: compile errors (module empty).

- [ ] **Step 3: Implement**

`src/look.rs`:

```rust
//! Pictures of the part for the AI planner: the overview its first request
//! carries and the views the model asks for with the `view_model` tool.

/// The viewport's background, under everything a picture draws.
const BACKGROUND: [u8; 3] = [239, 243, 247];
const INK: [u8; 3] = [40, 46, 54];
/// A 5 × 7 glyph per character, one byte a row, bit 4 the leftmost pixel.
fn glyph(c: char) -> Option<[u8; 7]> {
    Some(match c {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        ' ' => [0; 7],
        '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x0C, 0x04, 0x08],
        ':' => [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00],
        '/' => [0x00, 0x01, 0x02, 0x04, 0x08, 0x10, 0x00],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        'Ø' => [0x0D, 0x12, 0x15, 0x15, 0x15, 0x09, 0x16],
        '×' => [0x00, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x00],
        '%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
        '…' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x15, 0x15],
        '·' => [0x00, 0x00, 0x00, 0x0C, 0x0C, 0x00, 0x00],
        '?' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
        _ => return None,
    })
}
/// An opaque RGB picture being drawn.
pub(crate) struct Canvas {
    pub w: u32,
    pub h: u32,
    pub rgb: Vec<u8>,
}
impl Canvas {
    #[cfg(test)]
    fn blank(w: u32, h: u32) -> Self {
        Self { w, h, rgb: BACKGROUND.repeat((w * h) as usize) }
    }
    #[cfg(test)]
    fn at(&self, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * self.w + x) * 3) as usize;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }
    /// `rgba` laid over the background; mirrored left to right when `flip`.
    pub(crate) fn over(rgba: &[u8], w: u32, h: u32, flip: bool) -> Self {
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                let sx = if flip { w - 1 - x } else { x };
                let i = ((y * w + sx) * 4) as usize;
                let a = rgba[i + 3] as f32 / 255.;
                for k in 0..3 {
                    rgb.push((rgba[i + k] as f32 * a + BACKGROUND[k] as f32 * (1. - a)).round() as u8);
                }
            }
        }
        Self { w, h, rgb }
    }
    fn put(&mut self, x: i32, y: i32, color: [u8; 3]) {
        if x >= 0 && y >= 0 && (x as u32) < self.w && (y as u32) < self.h {
            let i = ((y as u32 * self.w + x as u32) * 3) as usize;
            self.rgb[i..i + 3].copy_from_slice(&color);
        }
    }
    pub(crate) fn fill(&mut self, [x0, y0, x1, y1]: [i32; 4], color: [u8; 3]) {
        for y in y0..y1 {
            for x in x0..x1 {
                self.put(x, y, color);
            }
        }
    }
    pub(crate) fn line(&mut self, a: (f32, f32), b: (f32, f32), color: [u8; 3], width: f32) {
        let steps = ((b.0 - a.0).hypot(b.1 - a.1).ceil() as usize).clamp(1, 4000);
        let r = (width / 2.).max(0.5);
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            for yy in (y - r).floor() as i32..=(y + r).ceil() as i32 {
                for xx in (x - r).floor() as i32..=(x + r).ceil() as i32 {
                    if (xx as f32 + 0.5 - x).powi(2) + (yy as f32 + 0.5 - y).powi(2) <= r * r {
                        self.put(xx, yy, color);
                    }
                }
            }
        }
    }
    pub(crate) fn text_width(text: &str, scale: u32) -> u32 {
        (text.chars().count() as u32 * 6).saturating_sub(1) * scale
    }
    /// `text` in capitals from its top-left corner; unknown characters as "?".
    pub(crate) fn text(&mut self, x: i32, y: i32, text: &str, scale: u32, color: [u8; 3]) {
        let s = scale as i32;
        for (n, c) in text.chars().flat_map(char::to_uppercase).enumerate() {
            let rows = glyph(c).unwrap_or_else(|| glyph('?').unwrap());
            for (row, bits) in rows.iter().enumerate() {
                for col in 0..5 {
                    if bits & (0x10 >> col) != 0 {
                        let (px, py) = (x + (n as i32 * 6 + col) * s, y + row as i32 * s);
                        self.fill([px, py, px + s, py + s], color);
                    }
                }
            }
        }
    }
    /// A label box centred on `center`; None when it would leave the picture.
    pub(crate) fn tag(&mut self, center: (f32, f32), text: &str) -> Option<[i32; 4]> {
        let (tw, th) = (Self::text_width(text, 2) as i32, 14);
        let (w, h) = (tw + 8, th + 8);
        let x0 = center.0.round() as i32 - w / 2;
        let y0 = center.1.round() as i32 - h / 2;
        let rect = [x0, y0, x0 + w, y0 + h];
        if x0 < 0 || y0 < 0 || rect[2] > self.w as i32 || rect[3] > self.h as i32 {
            return None;
        }
        self.fill(rect, INK);
        self.fill([x0 + 1, y0 + 1, rect[2] - 1, rect[3] - 1], [255, 255, 255]);
        self.text(x0 + 4, y0 + 4, text, 2, INK);
        Some(rect)
    }
    pub(crate) fn png(&self) -> Vec<u8> {
        let mut out = vec![];
        let mut encoder = png::Encoder::new(&mut out, self.w, self.h);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("PNG header");
        writer.write_image_data(&self.rgb).expect("PNG data");
        drop(writer);
        out
    }
}
```

Add `mod look;` to `src/main.rs` with the other module declarations.

- [ ] **Step 4: Run the tests**

Run: `cargo test --bin axis-cam look:: 2>&1 | tail -5`
Expected: 3 passed. (`dead_code` warnings for not-yet-used items are fine until Task 4; do not commit with `#[allow]`.)

- [ ] **Step 5: Commit**

```bash
git add src/look.rs src/main.rs
git commit -m "Planner pictures: pixel font, tags and PNG output

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: The `view_model` tool: arguments, views and descriptions

**Files:**
- Modify: `src/look.rs` (scene, arguments, drawing)
- Modify: `src/planner.rs` (`view_after` split from `final_view_at`, ~line 800)
- Modify: `src/plan_check.rs:396-425` (`evaluate`)

**Interfaces:**
- Consumes: `viewport::{Viewport, Projector, Camera}`, `Viewport::render`, `Projector::fit` (Task 2); `Canvas` (Task 3); `analysis::{oriented, up_axis, ORIENTATIONS}`.
- Produces:
  - `pub struct look::Scene { pub doc: Arc<Document>, pub mesh: Arc<Mesh>, pub report: Arc<PartReport>, pub job: Option<Arc<Document>> }` (`Clone`)
  - `#[derive(Debug, Clone, PartialEq, Eq)] pub struct look::Picture { pub png: Arc<Vec<u8>>, pub description: String, pub purpose: String }`
  - `pub fn look::overview(scene: &Scene) -> Result<Picture, String>`
  - `pub fn look::draw(scene: &Scene, arguments: &str) -> Result<Picture, String>` (Err: one sentence for the model)
  - (`look::tool()` is added in Task 5, which defines `openai::Tool`.)
  - `pub const look::NAME: &str = "view_model"`
  - `pub(crate) fn planner::view_after(job: &Document, mesh: &Mesh, si: usize, oi: usize, detail: Detail) -> Option<Simulation>`
  - `pub fn plan_check::evaluate(doc, mesh, report, proposal) -> (CheckReport, Option<Document>)` — the job whenever the proposal built, passed or not.

- [ ] **Step 1: Write the failing tests** (in `src/look.rs` tests)

```rust
    use crate::proposal::tests::{block, good};
    use std::sync::Arc;
    fn scene() -> Scene {
        let (doc, mesh, report) = block();
        Scene { doc: Arc::new(doc), mesh: Arc::new(mesh), report: Arc::new(report), job: None }
    }
    fn args(json: serde_json::Value) -> String {
        let mut all = serde_json::json!({
            "orientation": null, "angle": "iso_front_left", "focus_surfaces": [],
            "highlight_surfaces": [], "show_stock": true, "show_workholding": true,
            "labels": true, "operation": null, "purpose": "look"
        });
        for (k, v) in json.as_object().unwrap() {
            all[k] = v.clone();
        }
        all.to_string()
    }
    fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).unwrap();
        (info.width, info.height, buf)
    }
    #[test]
    fn the_overview_is_a_labelled_768_pixel_png_of_setup_1() {
        let s = scene();
        let picture = overview(&s).unwrap();
        let (w, h, _) = decode(&picture.png);
        assert_eq!((w, h), (768, 768));
        assert!(picture.description.starts_with("+Z up, isometric front-left, whole part"), "{}", picture.description);
        assert!(picture.description.contains("stock shown"), "{}", picture.description);
        let top = &s.report.orientations[0].planes[0];
        assert!(picture.description.contains(&top.surfaces[0].to_string()), "{}", picture.description);
    }
    #[test]
    fn a_top_view_keeps_x_right_and_y_up() {
        let s = scene();
        let view = View::new(&s, &Args::parse(&args(serde_json::json!({"angle": "top"}))).unwrap()).unwrap();
        let (a, b, c) = (view.screen([0., 0., 0.]), view.screen([10., 0., 0.]), view.screen([0., 10., 0.]));
        assert!(b.0 > a.0 + 1., "+X is to the right: {a:?} {b:?}");
        assert!(c.1 < a.1 - 1., "+Y is up: {a:?} {c:?}");
        let front = View::new(&s, &Args::parse(&args(serde_json::json!({"angle": "front"}))).unwrap()).unwrap();
        let (a, b) = (front.screen([0., 0., 0.]), front.screen([10., 0., 0.]));
        assert!(b.0 > a.0 + 1., "seen from -Y, +X is to the right");
    }
    #[test]
    fn highlighting_and_focus_change_the_picture() {
        let s = scene();
        let pocket = s.report.orientations[0].planes.iter().find(|p| p.kind == "floor").unwrap().surfaces[0];
        let plain = draw(&s, &args(serde_json::json!({"angle": "top", "labels": false}))).unwrap();
        let marked = draw(&s, &args(serde_json::json!({"angle": "top", "labels": false, "highlight_surfaces": [pocket]}))).unwrap();
        let tinted = |png: &[u8]| decode(png).2.chunks(3).filter(|p| p == &[92, 168, 223]).count();
        assert!(tinted(&marked.png) > tinted(&plain.png) + 100);
        let near = draw(&s, &args(serde_json::json!({"angle": "top", "labels": false, "highlight_surfaces": [pocket], "focus_surfaces": [pocket]}))).unwrap();
        assert!(tinted(&near.png) > tinted(&marked.png) * 2, "the focused surface fills more of the picture");
        assert!(near.description.contains("zoomed on "), "{}", near.description);
    }
    #[test]
    fn bad_arguments_get_one_sentence_errors() {
        let s = scene();
        for (bad, says) in [
            ("not json".to_string(), "could not be read"),
            (args(serde_json::json!({"angle": "sideways"})), "could not be read"),
            (args(serde_json::json!({"focus_surfaces": [99999]})), "Surface 99999"),
            (args(serde_json::json!({"highlight_surfaces": (0..10_000).collect::<Vec<_>>()})), "at most 200"),
            (args(serde_json::json!({"operation": {"setup": 1, "operation": 1}})), "No proposal has been built yet"),
            (args(serde_json::json!({"orientation": "+Q"})), "could not be read"),
        ] {
            let error = draw(&s, &bad).unwrap_err();
            assert!(error.contains(says), "{bad}: {error}");
            assert!(!error.contains('\n') && error.chars().count() <= 300, "{error}");
        }
        let mut extra: serde_json::Value = serde_json::from_str(&args(serde_json::json!({}))).unwrap();
        extra["zoom"] = serde_json::json!(2);
        assert!(draw(&s, &extra.to_string()).unwrap_err().contains("could not be read"));
    }
    #[test]
    fn an_operation_of_the_latest_proposal_can_be_looked_at() {
        let mut s = scene();
        let (_, job) = crate::plan_check::evaluate(&s.doc, &s.mesh, &s.report, &good(&s.report));
        s.job = job.map(Arc::new);
        let picture = draw(&s, &args(serde_json::json!({"operation": {"setup": 1, "operation": 1}}))).unwrap();
        assert!(picture.description.contains("after setup 1, operation 1"), "{}", picture.description);
        let out = draw(&s, &args(serde_json::json!({"operation": {"setup": 1, "operation": 99}}))).unwrap_err();
        assert!(out.contains("Setup 1 has no operation 99"), "{out}");
    }
    #[test]
    fn the_purpose_is_cleaned_and_clipped() {
        let s = scene();
        let long = format!("why\n\u{202e}{}", "x".repeat(500));
        let p = draw(&s, &args(serde_json::json!({"purpose": long, "labels": false}))).unwrap();
        assert!(p.purpose.starts_with("why x") && p.purpose.chars().count() <= 120);
    }
```

In `src/plan_check.rs` tests add:

```rust
    #[test]
    fn evaluate_keeps_the_job_of_a_failing_plan() {
        let (doc, mesh, report) = crate::proposal::tests::block();
        let mut slow = crate::proposal::tests::good(&report);
        slow.setups[0].operations[0].rpm = 100.;
        let (check, job) = evaluate(&doc, &mesh, &report, &slow);
        assert!(!check.passed && job.is_some());
        let (check, job) = super::check(&doc, &mesh, &report, &slow);
        assert!(!check.passed && job.is_none());
    }
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --bin axis-cam look:: plan_check::tests::evaluate 2>&1 | tail -5`
Expected: compile errors.

- [ ] **Step 3: Implement `evaluate` and `view_after`**

`src/plan_check.rs`:

```rust
/// Runs every stage on a proposal. Returns the report and the job it built
/// (with the layouts suggested for its later setups), whether or not the
/// plan passed; None when the proposal could not be built.
pub fn evaluate(
    doc: &Document,
    mesh: &Mesh,
    report: &PartReport,
    proposal: &Proposal,
) -> (CheckReport, Option<Document>) {
    match proposal::build(doc, mesh, report, proposal) {
        Ok(built) => {
            let (out, laid) = run(&built, mesh, report);
            (out, Some(laid.unwrap_or(built)))
        }
        Err(problems) => (
            CheckReport { passed: false, problems, operations: vec![], coverage: None, total_minutes: 0., zeros: vec![] },
            None,
        ),
    }
}
/// `evaluate` that keeps the job only when the plan passed.
pub fn check(doc: &Document, mesh: &Mesh, report: &PartReport, proposal: &Proposal) -> (CheckReport, Option<Document>) {
    let (out, job) = evaluate(doc, mesh, report, proposal);
    let job = job.filter(|_| out.passed);
    (out, job)
}
```

`src/planner.rs`: move the body of `final_view_at` from `let mut sim = …` to its end into

```rust
/// The stock after operation `oi` of setup `si` of `job`, meshed at `detail`.
pub(crate) fn view_after(
    job: &Document,
    mesh: &Mesh,
    si: usize,
    oi: usize,
    detail: crate::stock_surface::Detail,
) -> Option<crate::simulation::Simulation> {
    let mut sim = crate::simulation::prepare(job, mesh, si, oi).ok()?;
    // … the rest of final_view_at's body, unchanged …
}
```

and make `final_view_at` compute `si`/`oi` as now and return `view_after(job, mesh, si, oi, detail)`. In `plan()`, pass `plan_check::evaluate` instead of `plan_check::check` (the planner already requires `check.passed` before it uses the job).

- [ ] **Step 4: Implement the tool in `src/look.rs`**

```rust
use crate::{
    analysis::{self, PartReport},
    mesh::{Bounds, Mesh},
    model::{Document, Setup},
    viewport::{Camera, Projector, Viewport},
};
use serde::Deserialize;
use std::sync::Arc;

pub const NAME: &str = "view_model";
pub const SIZE: u32 = 768;
/// The strip along the bottom: the caption and the scale bar.
const CAPTION: u32 = 44;
const MAX_IDS: usize = 200;
/// Each angle: name, words, yaw and pitch of `viewport::Camera`. The
/// projector puts the viewer at (sin yaw · cos pitch, cos yaw · cos pitch,
/// sin pitch); its frame is a mirror image, so pictures are flipped.
const ANGLES: [(&str, &str, f32, f32); 9] = [
    ("top", "top", std::f32::consts::PI, std::f32::consts::FRAC_PI_2),
    ("front", "front (from -Y)", std::f32::consts::PI, 0.),
    ("back", "back (from +Y)", 0., 0.),
    ("left", "left (from -X)", -std::f32::consts::FRAC_PI_2, 0.),
    ("right", "right (from +X)", std::f32::consts::FRAC_PI_2, 0.),
    ("iso_front_left", "isometric front-left", -(std::f32::consts::PI - 0.75), 0.62),
    ("iso_front_right", "isometric front-right", std::f32::consts::PI - 0.75, 0.62),
    ("iso_back_left", "isometric back-left", -0.75, 0.62),
    ("iso_back_right", "isometric back-right", 0.75, 0.62),
];
#[derive(Clone)]
pub struct Scene {
    pub doc: Arc<Document>,
    pub mesh: Arc<Mesh>,
    pub report: Arc<PartReport>,
    /// The latest proposal's built job, for `operation` views.
    pub job: Option<Arc<Document>>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub png: Arc<Vec<u8>>,
    /// What the picture shows, for the model and the timeline.
    pub description: String,
    /// The model's reason, cleaned, at most 120 characters.
    pub purpose: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpRef {
    setup: usize,
    operation: usize,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Args {
    orientation: Option<String>,
    angle: String,
    focus_surfaces: Vec<usize>,
    highlight_surfaces: Vec<usize>,
    show_stock: bool,
    show_workholding: bool,
    labels: bool,
    operation: Option<OpRef>,
    purpose: String,
}
impl Args {
    pub(crate) fn parse(arguments: &str) -> Result<Self, String> {
        let args: Self = serde_json::from_str(arguments).map_err(|e| {
            crate::proposal::clip(&format!("The view_model arguments could not be read: {e}"), 300)
        })?;
        let known_up = |u: &str| analysis::ORIENTATIONS.iter().any(|(up, _)| *up == u);
        if args.orientation.as_deref().is_some_and(|u| !known_up(u))
            || !ANGLES.iter().any(|(a, ..)| *a == args.angle)
        {
            return Err("The view_model arguments could not be read: unknown orientation or angle.".into());
        }
        if args.focus_surfaces.len() > MAX_IDS || args.highlight_surfaces.len() > MAX_IDS {
            return Err(format!("List at most {MAX_IDS} surfaces to focus on or highlight."));
        }
        Ok(args)
    }
}
```

Then `View` — the setup, the camera fit and the flip, built from the arguments:

```rust
/// What one picture draws: the setup frame, the projector and the layers.
pub(crate) struct View {
    setup: Setup,
    up: Option<&'static str>,
    projector: Projector,
    sim: Option<crate::simulation::Simulation>,
    holding: Vec<crate::holding::HoldBox>,
    angle_words: &'static str,
    focus: Vec<usize>,
    operation: Option<(usize, usize)>,
}
impl View {
    pub(crate) fn new(scene: &Scene, args: &Args) -> Result<Self, String> {
        let mesh = &*scene.mesh;
        let surfaces = mesh.surfaces.len();
        if let Some(bad) = args.focus_surfaces.iter().chain(&args.highlight_surfaces).find(|id| **id >= surfaces) {
            return Err(format!("Surface {bad} is not a surface of this part."));
        }
        let (_, angle_words, yaw, pitch) = *ANGLES.iter().find(|(a, ..)| *a == args.angle).unwrap();
        let camera = Camera { yaw, pitch, zoom: 1., pan: [0., 0.] };
        let (setup, up, sim, operation) = match &args.operation {
            Some(OpRef { setup, operation }) => {
                let job = scene.job.as_ref().ok_or("No proposal has been built yet, so there is no operation to look at.")?;
                let s = job.setups.get(setup.wrapping_sub(1)).ok_or(format!("The latest proposal has no setup {setup}."))?;
                if operation.wrapping_sub(1) >= s.operations.len() {
                    return Err(format!("Setup {setup} has no operation {operation} in the latest proposal."));
                }
                let sim = crate::planner::view_after(job, mesh, setup - 1, operation - 1, crate::stock_surface::Detail::Standard)
                    .ok_or(format!("Setup {setup}, operation {operation} could not be simulated."))?;
                (s.clone(), analysis::up_axis(s), Some(sim), Some((*setup, *operation)))
            }
            None => {
                let first = scene.doc.setups.first().ok_or("The project has no setup 1.")?;
                let up = match &args.orientation {
                    Some(u) => analysis::ORIENTATIONS.iter().find(|(name, _)| name == u).unwrap().0,
                    None => analysis::up_axis(first).ok_or("Setup 1 is not square to the part.")?,
                };
                let mut setup = analysis::oriented(first, up).ok_or("That orientation cannot be drawn.")?;
                // The stock as the report gives it in this orientation.
                if let Some(o) = scene.report.orientations.iter().find(|o| o.up == up) {
                    let scale = mesh.display_scale();
                    let model = crate::stock::Placement::new(mesh, Some(&setup)).bounds.scaled(1. / scale);
                    let m = o.stock_margin;
                    setup.preview_stock = Some(Bounds {
                        min: [model.min[0] - m[2], model.min[1] - m[4], model.min[2] - m[1]],
                        max: [model.max[0] + m[3], model.max[1] + m[5], model.max[2] + m[0]],
                    });
                    setup.p.insert("stock_source".into(), "fresh".into());
                }
                (setup, Some(up), None, None)
            }
        };
        let holding = if args.show_workholding {
            let doc = scene.job.as_deref().filter(|_| operation.is_some()).unwrap_or(&scene.doc);
            let first_up = scene.doc.setups.first().and_then(analysis::up_axis);
            let stock = sim.as_ref().map(|s| s.plan.stock).or_else(|| crate::stock::resolve(&setup, Some(mesh)).ok().map(|r| r.stock));
            match stock {
                Some(stock) if operation.is_some() || up == first_up => doc
                    .holding_boxes(&setup, stock)
                    .map(|b| crate::holding::shown(&b, stock))
                    .unwrap_or_default(),
                _ => vec![],
            }
        } else {
            vec![]
        };
        let viewport = Viewport {
            setup: Some(&setup), operation: None, mesh, simulation: sim.as_ref(), sample: false,
            camera, picking: false, show_stock: args.show_stock, show_paths: false, progress: None,
            holding: holding.clone(),
        };
        let bounds = if args.focus_surfaces.is_empty() {
            viewport.scene_bounds()
        } else {
            focus_bounds(mesh, &setup, &args.focus_surfaces)
        };
        let projector = Projector::fit(camera, (SIZE as f32, (SIZE - CAPTION) as f32), bounds);
        Ok(Self { setup, up, projector, sim, holding, angle_words, focus: args.focus_surfaces.clone(), operation })
    }
    /// Where a point (display units, setup frame) lands in the picture:
    /// x, y in pixels after the flip, and its depth.
    pub(crate) fn screen(&self, v: [f32; 3]) -> (f32, f32, f32) {
        let q = self.projector.p(v);
        (SIZE as f32 - q.x, q.y, self.projector.depth(v))
    }
}
/// The display-unit bounds of `ids`' faces in `setup`'s frame, 15 % larger.
fn focus_bounds(mesh: &Mesh, setup: &Setup, ids: &[usize]) -> Bounds {
    let placement = crate::stock::Placement::new(mesh, Some(setup));
    let mut b = Bounds { min: [f32::INFINITY; 3], max: [f32::NEG_INFINITY; 3] };
    for face in mesh.faces.iter().filter(|f| f.feature.is_some_and(|id| ids.contains(&id))) {
        for p in &face.points {
            let q = placement.transform(*p);
            for k in 0..3 {
                b.min[k] = b.min[k].min(q[k]);
                b.max[k] = b.max[k].max(q[k]);
            }
        }
    }
    let pad: [f32; 3] = std::array::from_fn(|k| (b.max[k] - b.min[k]).max(1.) * 0.15);
    Bounds { min: std::array::from_fn(|k| b.min[k] - pad[k]), max: std::array::from_fn(|k| b.max[k] + pad[k]) }
}
```

(Check `Bounds` has public `min`/`max` fields and whether it has a constructor or `Default`; use them if so. If the `screen` test shows +X still to the left in a top view, the projector is not mirrored: remove the flip in `screen` and in `Canvas::over`'s call below, and say so in the commit message.)

Then the picture itself:

```rust
fn render(scene: &Scene, args: &Args) -> Result<Picture, String> {
    let view = View::new(scene, args)?;
    let mesh = &*scene.mesh;
    let job_setup = view.operation.and_then(|(s, o)| scene.job.as_ref().map(|j| (j.clone(), s, o)));
    let operation = job_setup.as_ref().map(|(j, s, o)| &j.setups[s - 1].operations[o - 1]);
    let viewport = Viewport {
        setup: Some(&view.setup), operation, mesh, simulation: view.sim.as_ref(), sample: false,
        camera: Camera::default(), picking: false, show_stock: args.show_stock,
        show_paths: view.operation.is_some(), progress: None, holding: view.holding.clone(),
    };
    let (w, h) = (SIZE, SIZE - CAPTION);
    let (rgba, depths) = viewport.render(&view.projector, (w, h), 1., None, &args.highlight_surfaces);
    let mut canvas = Canvas::over(&rgba, w, h, true);
    canvas.rgb.extend(std::iter::repeat_n(255u8, (SIZE * CAPTION * 3) as usize));
    canvas.h = SIZE;
    let tags = if args.labels { draw_tags(&mut canvas, scene, &view, &depths) } else { vec![] };
    caption(&mut canvas, &view, mesh.display_scale());
    let description = describe(&view, args, &tags);
    Ok(Picture {
        png: Arc::new(canvas.png()),
        description,
        purpose: crate::proposal::clip(&args.purpose, 120),
    })
}
pub fn draw(scene: &Scene, arguments: &str) -> Result<Picture, String> {
    render(scene, &Args::parse(arguments)?)
}
pub fn overview(scene: &Scene) -> Result<Picture, String> {
    let args = Args {
        orientation: None, angle: "iso_front_left".into(), focus_surfaces: vec![], highlight_surfaces: vec![],
        show_stock: true, show_workholding: true, labels: true, operation: None,
        purpose: "Overview".into(),
    };
    render(scene, &args)
}
```

`draw_tags`: for the report orientation whose `up == view.up` (none ⇒ no tags), place each plane's tag at display point `[model.min[0] + (bbox[0]+bbox[2])/2 · scale, model.min[1] + (bbox[1]+bbox[3])/2 · scale, model.max[2] − depth_below_top · scale]` where `model = Placement::new(mesh, Some(&view.setup)).bounds` and `scale = mesh.display_scale()`; text = up to three surface IDs joined with "," then "…" if more. Each hole of that plane gets "Ø{diameter:.1}" at `[model.min[0] + center[0]·scale, model.min[1] + center[1]·scale, same z]`. A tag is drawn only when its point is inside the picture and visible: with `(x, y, z) = view.screen(point)`, the depth buffer is indexed at the *unflipped* column `w − 1 − x`, and visible means `z + 0.5 >= depths[y·w + (w−1−x)]` (or the buffer there is −∞). Skip a tag whose rectangle overlaps one already drawn (planes are largest first). Return the drawn texts in order.

`caption`: fill the strip `[0, SIZE−CAPTION, SIZE, SIZE]` white with a 1 px `[200, 205, 212]` top line; write `{up} UP · {ANGLE WORDS}` (and ` · AFTER S{s} OP{o}` with an operation) at `(10, SIZE−CAPTION+8)` scale 2; a scale bar on the right: the largest of `[1, 2, 5, 10, 20, 50, 100, 200, 500]` mm whose length `mm · scale · projector.scale` is at most 160 px, drawn as a 3 px INK line ending 12 px from the right edge, with "{n} MM" above it; and an axis triad at (40, 40): lines 28 px long toward the flipped screen directions of +X `[205, 81, 85]`, +Y `[76, 154, 105]`, +Z `[67, 125, 196]` (from `view.screen` differences), each with its letter.

`describe` builds, verbatim in this order:
`"{up} up, {angle words}, {whole part | zoomed on 12, 14}{, after setup S, operation O}; {stock shown | stock hidden}{; vise or clamps shown}; tags: {list or none}"` where `{up}` is the view's `up` or "rotated setup" when None, and the vise/clamps part appears only when `view.holding` is non-empty.

- [ ] **Step 5: Run the tests**

Run: `cargo test --bin axis-cam look:: plan_check:: planner:: 2>&1 | grep -E "test result|FAILED|panicked"`
Expected: all pass. If `a_top_view_keeps_x_right_and_y_up` fails only on direction, fix the angle table (yaw sign) — not the test.

- [ ] **Step 6: Commit**

```bash
git add src/look.rs src/planner.rs src/plan_check.rs
git commit -m "Planner pictures: the view_model views, tags, caption and descriptions

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Conversation items, images, tools and streamed calls in the OpenAI client

**Files:**
- Modify: `src/openai.rs` (types, `Request::body`, `Accumulator`, `attempt`, `respond*`, `api_error`, `ApiError`, tests)
- Modify: `src/smoke.rs:3642` (Request literal), `src/look.rs` (`tool()`)
- Modify: `src/planner.rs:757-800` (`OpenAiBackend` holds the stream), `src/main.rs:322,562,1137,2444` (`plan_received` → `plan_stream`), `src/ui.rs:3493-3516` (the count is the stream's length), `src/smoke.rs:746,4717-4723`

**Interfaces:**
- Produces:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part { Text(String), Image { png: Arc<Vec<u8>>, alt: String } }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall { pub call_id: String, pub name: String, pub arguments: String }
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item { Message(Role, Vec<Part>), Call(ToolCall), Output { call_id: String, output: String } }
impl Item {
    pub fn text(role: Role, text: impl Into<String>) -> Self;
    /// A message's role and its text parts joined with "\n"; None otherwise.
    pub fn as_text(&self) -> Option<(Role, String)>;
}
#[derive(Debug, Clone, PartialEq)]
pub struct Tool { pub name: &'static str, pub description: String, pub parameters: Value }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolChoice { #[default] Auto, None }
#[derive(Debug, Clone, Default)]
pub struct Request { pub model: String, pub messages: Vec<Item>, pub schema_name: &'static str, pub schema: Value, pub tools: Vec<Tool>, pub tool_choice: ToolChoice }
#[derive(Debug, Clone, PartialEq)]
pub struct Reply { pub text: String, pub structured: bool, pub calls: Vec<ToolCall> }
pub enum ApiError { …, BadRequest(String) }
pub async fn respond_streaming(&self, request: &Request, stream: &std::sync::Mutex<String>) -> Result<Reply, ApiError>;
pub async fn respond_plain_streaming(&self, request: &Request, stream: &std::sync::Mutex<String>) -> Result<Reply, ApiError>;
```

`respond_counting`/`respond_plain_counting` are replaced by the `_streaming` pair: `stream` is cleared when each attempt starts and receives every text delta.

- [ ] **Step 1: Write the failing tests** (in `src/openai.rs` tests; change `request()` to build `messages: vec![Item::text(Role::System, "Plan."), Item::text(Role::User, "Part.")]` and `..Default::default()`)

```rust
    #[tokio::test]
    async fn images_tools_and_calls_are_sent_as_items() {
        let r = rig(sample(), |_| Resp::answer("{}")).await;
        let png = Arc::new(vec![1u8, 2, 3]);
        let mut req = request();
        req.messages.push(Item::Message(Role::User, vec![
            Part::Text("Overview.".into()),
            Part::Image { png: png.clone(), alt: "the part".into() },
        ]));
        req.messages.push(Item::Call(ToolCall { call_id: "c1".into(), name: "view_model".into(), arguments: "{}".into() }));
        req.messages.push(Item::Output { call_id: "c1".into(), output: "drawn".into() });
        req.tools = vec![Tool { name: "view_model", description: "Draws.".into(), parameters: json!({"type": "object"}) }];
        req.tool_choice = ToolChoice::None;
        r.client.respond(&req).await.unwrap();
        let body = r.fake.seen("/v1/responses")[0].json();
        assert_eq!(body["input"][0], json!({"role": "system", "content": "Plan."}));
        assert_eq!(body["input"][2]["content"][0], json!({"type": "input_text", "text": "Overview."}));
        assert_eq!(body["input"][2]["content"][1], json!({"type": "input_image", "image_url": "data:image/png;base64,AQID", "detail": "high"}));
        assert_eq!(body["input"][3], json!({"type": "function_call", "call_id": "c1", "name": "view_model", "arguments": "{}"}));
        assert_eq!(body["input"][4], json!({"type": "function_call_output", "call_id": "c1", "output": "drawn"}));
        assert_eq!(body["tools"][0], json!({"type": "function", "name": "view_model", "description": "Draws.", "parameters": {"type": "object"}, "strict": true}));
        assert_eq!(body["tool_choice"], "none");
        let plain = rig(sample(), |_| Resp::answer("{}")).await;
        plain.client.respond(&request()).await.unwrap();
        let body = plain.fake.seen("/v1/responses")[0].json();
        assert!(body["tools"].is_null() && body["tool_choice"].is_null());
    }
    #[tokio::test]
    async fn function_calls_are_read_from_the_stream_or_the_final_event() {
        let call = json!({"type": "function_call", "id": "fc_1", "call_id": "c1", "name": "view_model", "arguments": "{\"angle\":\"top\"}"});
        let streamed = rig(sample(), {
            let call = call.clone();
            move |_| Resp::sse(&[
                json!({"type": "response.output_item.done", "item": call}),
                json!({"type": "response.completed", "response": {"output": [call]}}),
            ])
        }).await;
        let reply = streamed.client.respond(&request()).await.unwrap();
        assert_eq!(reply.text, "");
        assert_eq!(reply.calls, vec![ToolCall { call_id: "c1".into(), name: "view_model".into(), arguments: "{\"angle\":\"top\"}".into() }]);
        let final_only = rig(sample(), move |_| Resp::sse(&[json!({"type": "response.completed", "response": {"output": [call]}})])).await;
        assert_eq!(final_only.client.respond(&request()).await.unwrap().calls.len(), 1);
    }
    #[tokio::test]
    async fn an_http_400_without_a_known_code_is_a_bad_request() {
        let r = rig(sample(), |_| Resp::error(400, "invalid_value")).await;
        assert!(matches!(r.client.respond(&request()).await, Err(ApiError::BadRequest(_))));
    }
```

Replace `respond_counting_adds_the_streamed_characters_and_each_call_starts_at_zero` and `the_count_grows_while_the_reply_is_still_streaming` with the same scenarios on `respond_streaming`: the stream holds `"{" + "x"×2000 + "é}"` after the first call, `"{}"` after the second (a stale `"old"` is cleared), and while streaming it grows to `{"a":1` (6 characters) before the reply completes.

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --bin axis-cam openai:: 2>&1 | tail -5`
Expected: compile errors.

- [ ] **Step 3: Implement**

`Request::body`:

```rust
    fn body(&self, structured: bool) -> Value {
        use base64::Engine;
        let role_name = |role: &Role| match role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        let mut input: Vec<Value> = self
            .messages
            .iter()
            .map(|item| match item {
                Item::Message(role, parts) => match parts.as_slice() {
                    [Part::Text(text)] => json!({"role": role_name(role), "content": text}),
                    parts => json!({"role": role_name(role), "content": parts.iter().map(|part| match part {
                        Part::Text(text) => json!({"type": "input_text", "text": text}),
                        Part::Image { png, .. } => json!({
                            "type": "input_image",
                            "image_url": format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png.as_slice())),
                            "detail": "high",
                        }),
                    }).collect::<Vec<_>>()}),
                },
                Item::Call(call) => json!({"type": "function_call", "call_id": call.call_id, "name": call.name, "arguments": call.arguments}),
                Item::Output { call_id, output } => json!({"type": "function_call_output", "call_id": call_id, "output": output}),
            })
            .collect();
        let mut body = json!({"model": self.model, "store": false, "stream": true});
        // … structured / plain branch unchanged, pushing `json!({"role": "system", "content": …})` …
        if !self.tools.is_empty() {
            body["tools"] = self.tools.iter().map(|t| json!({
                "type": "function", "name": t.name, "description": t.description,
                "parameters": t.parameters, "strict": true,
            })).collect();
            body["tool_choice"] = json!(match self.tool_choice { ToolChoice::Auto => "auto", ToolChoice::None => "none" });
        }
        body["input"] = Value::Array(input);
        body
    }
```

`Accumulator` gains `calls: Vec<ToolCall>`; `apply` adds:

```rust
            "response.output_item.done" if event["item"]["type"] == "function_call" => {
                self.calls.extend(call(&event["item"]));
            }
```

and in `"response.completed"`, after the text fallback: `if self.calls.is_empty() { self.calls.extend(output.iter().filter(|i| i["type"] == "function_call").filter_map(call)); }` (iterate `event["response"]["output"]`), with

```rust
/// A function call item as the API sends it; its item `id` is not kept.
fn call(item: &Value) -> Option<ToolCall> {
    Some(ToolCall {
        call_id: item["call_id"].as_str()?.into(),
        name: item["name"].as_str()?.into(),
        arguments: item["arguments"].as_str().unwrap_or("").into(),
    })
}
```

The size guard counts `self.calls.iter().map(|c| c.arguments.len()).sum()` with the text and refusal; at most 16 calls are kept (`self.calls.truncate(16)`). `apply` returns the streamed delta `&str` length as before; `attempt` appends each delta to `stream` (lock, `push_str`) instead of a counter — change `apply` to return `Option<String>` with the delta text, or keep the delta in a local. `finish` returns `Ok((text, calls))`; a reply that is done with calls and no text is complete. `attempt` returns `Reply { text, structured, calls }`.

`ApiError::BadRequest(String)`: in `api_error`, after the `401` arm add `_ if status == 400 => ApiError::BadRequest(if message.is_empty() { "OpenAI returned HTTP 400.".into() } else { message }),`; in `message()` add it to the clipped arm. Update the existing `every_message_from_openai_is_one_clean_bounded_line` list with `(ApiError::BadRequest(hostile.clone()), "")`.

Update every other `Request { … }` literal (`check()`, `src/smoke.rs:3642`, `planner::OpenAiBackend::reply`) to the new field types (`Item::text`, `..Default::default()`); `OpenAiBackend` replaces `received: Arc<AtomicUsize>` with `stream: Arc<std::sync::Mutex<String>>` and wraps its `(Role, String)` messages with `Item::text` (the item conversation arrives in Task 6). In the app, `App.plan_received` becomes `plan_stream: Arc<std::sync::Mutex<String>>` (default empty; `drop_plan` makes a new one), `Message::Plan` passes it, and `timeline_rows` reads `let received = app.plan_stream.lock().map_or(0, |t| t.chars().count());` so the page is unchanged until Task 8. In `src/smoke.rs`, screen 84 stores `"x".repeat(1840)` in the stream, and the test at 4717 compares `Arc::ptr_eq` on `plan_stream`. In `look.rs` add `pub fn tool() -> crate::openai::Tool` returning `Tool { name: NAME, description, parameters }` with:

description: "Draws a picture of the part as the program sees it and returns it with a short description. Use it to check which faces the report's surface IDs are, where holes and pockets are, how the stock and the vise or clamps sit, and, after a proposal has been checked, what an operation's toolpaths and the stock left after it look like."

parameters (strict; every property required):

```rust
json!({
    "type": "object",
    "properties": {
        "orientation": {"type": ["string", "null"], "enum": ["+Z", "-Z", "+X", "-X", "+Y", "-Y", null],
            "description": "The report's orientation to draw the part in; null for setup 1's, or the operation's setup."},
        "angle": {"type": "string", "enum": ANGLES.map(|a| a.0)},
        "focus_surfaces": {"type": "array", "items": {"type": "integer"}, "description": "Zoom to these surface IDs; empty for the whole part."},
        "highlight_surfaces": {"type": "array", "items": {"type": "integer"}, "description": "Surface IDs drawn in blue."},
        "show_stock": {"type": "boolean"},
        "show_workholding": {"type": "boolean"},
        "labels": {"type": "boolean", "description": "Tags with each listed plane's surface IDs and each hole's diameter."},
        "operation": {"type": ["object", "null"], "properties": {"setup": {"type": "integer"}, "operation": {"type": "integer"}},
            "required": ["setup", "operation"], "additionalProperties": false,
            "description": "An operation of your latest checked proposal, counted from 1: its toolpaths and the stock left after it."},
        "purpose": {"type": "string", "description": "A few words on why you look; shown to the user."}
    },
    "required": ["orientation", "angle", "focus_surfaces", "highlight_surfaces", "show_stock", "show_workholding", "labels", "operation", "purpose"],
    "additionalProperties": false
})
```

- [ ] **Step 4: Run the tests**

Run: `cargo test 2>&1 | grep -E "test result|FAILED|panicked"`
Expected: all `ok`. A test that expected `ApiError::Other` for an HTTP 400 now sees `BadRequest`: update that expectation.

- [ ] **Step 5: Commit**

```bash
git add src/openai.rs src/look.rs src/planner.rs src/main.rs src/ui.rs src/smoke.rs
git commit -m "OpenAI client: conversation items with images, the tool, streamed calls and text

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: The planner's tool loop, limits, fallbacks and instructions

**Files:**
- Modify: `src/planner.rs` (`Backend`, `Event`, `OpenAiBackend`, `plan_with`, new `ask`, `outgoing`, `instructions_with`, tests)
- Modify: `src/main.rs` (`OpenAiBackend::new` gains `true` for pictures until Task 9; `record_plan_event` gets an arm `Event::Overview(_) | Event::Looked(..) | Event::Dropped(..) => {}` until Task 8)

**Interfaces:**
- Consumes: `openai::{Item, Part, ToolCall, ToolChoice, Reply, ApiError::BadRequest}` (Task 5); `look::{Scene, Picture, draw, overview, tool}` (Task 4/5); `plan_check::evaluate` (Task 4).
- Produces:

```rust
pub trait Backend {
    /// One send of the conversation. `tool`: None when the view_model tool
    /// is not offered, otherwise how the model may use it.
    fn reply(&self, messages: &[Item], tool: Option<ToolChoice>) -> impl Future<Output = Result<Reply, ApiError>> + Send;
    /// Whether this run may show the model pictures.
    fn pictures(&self) -> bool { false }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look { pub description: String, pub purpose: String, pub png: Option<Arc<Vec<u8>>> }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dropped { Tool, Pictures }
pub enum Event { Stage(Progress), Checked(usize, StepResult), Overview(Look), Looked(usize, Look), Dropped(usize, Dropped) }
pub const MAX_PICTURES_PER_REQUEST: usize = 6;
pub const MAX_PICTURES_PER_RUN: usize = 20;
pub const KEPT_PICTURES: usize = 8;
pub const MAX_TOOL_ROUNDS: usize = 8;
pub fn instructions_with(pictures: bool) -> String; // instructions() == instructions_with(false)
impl OpenAiBackend { pub fn new(client, model: String, stream: Arc<Mutex<String>>, pictures: bool) -> Self }
```

- [ ] **Step 1: Adapt the test `Script` and write the failing tests**

`Script` keeps `seen: Mutex<Vec<Vec<(Role, String)>>>` (each sent message as `Item::as_text`, other items skipped, so existing tests are unchanged) and adds `pub turns: Mutex<Vec<(Vec<Item>, Option<ToolChoice>)>>`, `pictures: bool`, and `Script::calls(replies: Vec<Result<Reply, ApiError>>) -> Self` (pictures on). `Script::new` maps `Ok(text)` to `Reply { text, structured: true, calls: vec![] }`. Fix `stages()` to `_ => None`.

```rust
    fn look_call(id: &str, json: serde_json::Value) -> ToolCall {
        let mut args = serde_json::json!({
            "orientation": null, "angle": "top", "focus_surfaces": [], "highlight_surfaces": [],
            "show_stock": true, "show_workholding": true, "labels": true, "operation": null, "purpose": "check"
        });
        for (k, v) in json.as_object().unwrap() { args[k] = v.clone(); }
        ToolCall { call_id: id.into(), name: crate::look::NAME.into(), arguments: args.to_string() }
    }
    fn calls(calls: Vec<ToolCall>) -> Result<Reply, ApiError> {
        Ok(Reply { text: String::new(), structured: true, calls })
    }
    fn text(proposal: &Proposal) -> Result<Reply, ApiError> {
        Ok(Reply { text: serde_json::to_string(proposal).unwrap(), structured: true, calls: vec![] })
    }
    #[tokio::test]
    async fn the_model_gets_an_overview_and_the_pictures_it_asks_for() {
        let (doc, mesh, report) = block();
        let script = Script::calls(vec![
            calls(vec![look_call("a", serde_json::json!({})), look_call("b", serde_json::json!({"focus_surfaces": [99999]}))]),
            text(&good(&report)),
            text(&good(&report)),
        ]);
        let mut events = vec![];
        let outcome = plan(&script, doc, mesh, &Busy::default(), |e| events.push(e)).await.unwrap();
        assert_eq!(outcome.requests, 2, "tool rounds are not planner requests");
        let turns = script.turns.lock().unwrap();
        let (first, tool) = &turns[0];
        assert_eq!(*tool, Some(ToolChoice::Auto));
        assert!(first[0].as_text().unwrap().1.contains("PICTURES"));
        assert!(matches!(&first[2], Item::Message(Role::User, parts) if parts.iter().any(|p| matches!(p, Part::Image { .. }))));
        let (second, _) = &turns[1];
        let n = second.len();
        assert!(matches!(&second[n - 5], Item::Call(c) if c.call_id == "a"));
        assert!(matches!(&second[n - 4], Item::Output { call_id, output } if call_id == "a" && output.contains("up,")));
        assert!(matches!(&second[n - 3], Item::Call(c) if c.call_id == "b"));
        assert!(matches!(&second[n - 2], Item::Output { output, .. } if output.contains("Surface 99999")));
        assert!(matches!(&second[n - 1], Item::Message(Role::User, parts) if parts.len() == 2));
        assert!(events.iter().any(|e| matches!(e, Event::Overview(l) if l.png.is_some())));
        let looks: Vec<_> = events.iter().filter_map(|e| match e { Event::Looked(1, l) => Some(l), _ => None }).collect();
        assert_eq!(looks.len(), 2);
        assert!(looks[0].png.is_some() && looks[1].png.is_none());
    }
    #[tokio::test]
    async fn pictures_are_limited_per_request_and_old_ones_are_not_resent() {
        let (doc, mesh, report) = block();
        let many = |from: usize| calls((from..from + 4).map(|i| look_call(&i.to_string(), serde_json::json!({}))).collect());
        let script = Script::calls(vec![many(0), many(4), text(&good(&report)), text(&good(&report))]);
        let mut events = vec![];
        plan(&script, doc, mesh, &Busy::default(), |e| events.push(e)).await.unwrap();
        let turns = script.turns.lock().unwrap();
        // After 4 + 2 pictures the per-request limit is reached.
        assert_eq!(turns[2].1, Some(ToolChoice::None));
        let outputs: Vec<_> = turns[2].0.iter().filter_map(|i| match i { Item::Output { output, .. } => Some(output.clone()), _ => None }).collect();
        assert_eq!(outputs.iter().filter(|o| *o == "No more pictures for this request; reply with the proposal.").count(), 2);
        let images = turns[2].0.iter().flat_map(|i| match i { Item::Message(_, p) => p.clone(), _ => vec![] }).filter(|p| matches!(p, Part::Image { .. })).count();
        assert_eq!(images, 7, "the overview and 6 pictures, all within the 8 kept");
        // The next planner request may look again.
        assert_eq!(turns[3].1, Some(ToolChoice::Auto));
    }
    #[tokio::test]
    async fn a_model_that_never_stops_looking_fails_the_request() {
        let (doc, mesh, _) = block();
        let replies = (0..MAX_TOOL_ROUNDS + 2).map(|i| calls(vec![look_call(&i.to_string(), serde_json::json!({"labels": false}))])).collect();
        let script = Script::calls(replies);
        let failure = plan(&script, doc, mesh, &Busy::default(), |_| {}).await.unwrap_err();
        assert!(failure.message.contains("kept asking for pictures"), "{}", failure.message);
    }
    #[tokio::test]
    async fn a_refused_tool_then_refused_pictures_are_dropped_in_order() {
        let (doc, mesh, report) = block();
        let script = Script::calls(vec![
            Err(ApiError::BadRequest("tools".into())),
            Err(ApiError::Unsupported("images".into())),
            text(&good(&report)),
            text(&good(&report)),
        ]);
        let mut events = vec![];
        plan(&script, doc, mesh, &Busy::default(), |e| events.push(e)).await.unwrap();
        let drops: Vec<_> = events.iter().filter_map(|e| match e { Event::Dropped(n, d) => Some((*n, *d)), _ => None }).collect();
        assert_eq!(drops, vec![(1, Dropped::Tool), (1, Dropped::Pictures)]);
        let turns = script.turns.lock().unwrap();
        assert_eq!(turns[1].1, None);
        assert_eq!(turns[3].1, None, "the run remembers");
        assert!(!turns[3].0[0].as_text().unwrap().1.contains("PICTURES"));
    }
    #[tokio::test]
    async fn refused_pictures_are_dropped_from_the_whole_conversation() {
        let (doc, mesh, report) = block();
        let script = Script::calls(vec![
            calls(vec![look_call("a", serde_json::json!({}))]),
            Err(ApiError::BadRequest("tool".into())),
            Err(ApiError::BadRequest("images".into())),
            text(&good(&report)),
            text(&good(&report)),
        ]);
        plan(&script, doc, mesh, &Busy::default(), |_| {}).await.unwrap();
        let turns = script.turns.lock().unwrap();
        let last = &turns.last().unwrap().0;
        assert!(!last.iter().any(|i| matches!(i, Item::Message(_, p) if p.iter().any(|p| matches!(p, Part::Image { .. })))));
        assert!(!last.iter().any(|i| matches!(i, Item::Call(_) | Item::Output { .. })), "calls become text once the tool is gone");
        assert!(last.iter().any(|i| i.as_text().is_some_and(|(_, t)| t.contains("(Picture not sent: "))));
    }
    #[tokio::test]
    async fn without_pictures_nothing_changes() {
        let (doc, mesh, report) = block();
        let script = Script::new(vec![json(&good(&report)), json(&good(&report))]);
        plan(&script, doc, mesh, &Busy::default(), |_| {}).await.unwrap();
        let turns = script.turns.lock().unwrap();
        assert!(turns.iter().all(|(_, tool)| tool.is_none()));
        assert_eq!(turns[0].0.len(), 2, "system and part report only");
    }
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --bin axis-cam planner:: 2>&1 | tail -5`
Expected: compile errors.

- [ ] **Step 3: Implement**

`outgoing` (pure, in `src/planner.rs`):

```rust
/// The conversation as it is sent: images beyond the `KEPT_PICTURES` most
/// recent, or all of them once pictures are off, become their description;
/// once the tool is gone, its calls and outputs become plain text.
fn outgoing(conversation: &[Item], pictures: bool, tool: bool) -> Vec<Item> {
    let total = conversation.iter().map(|i| match i {
        Item::Message(_, parts) => parts.iter().filter(|p| matches!(p, Part::Image { .. })).count(),
        _ => 0,
    }).sum::<usize>();
    let mut seen = 0;
    conversation.iter().map(|item| match item {
        Item::Message(role, parts) => Item::Message(*role, parts.iter().map(|part| match part {
            Part::Image { alt, .. } => {
                seen += 1;
                if !pictures {
                    Part::Text(format!("(Picture not sent: {alt})"))
                } else if total - seen >= KEPT_PICTURES {
                    Part::Text(format!("(Earlier picture, not resent: {alt})"))
                } else {
                    part.clone()
                }
            }
            text => text.clone(),
        }).collect()),
        Item::Call(call) if !tool => Item::text(Role::Assistant, format!("(Asked for a picture: {})", call.arguments)),
        Item::Output { output, .. } if !tool => Item::text(Role::User, format!("(Picture tool answered: {output})")),
        other => other.clone(),
    }).collect()
}
```

Run state and `ask`:

```rust
/// What the run may still show the model.
struct Sight {
    pictures: bool,
    tool: bool,
    shown: usize,
}
/// One planner request: sends the conversation, answers the model's calls
/// for pictures, and returns the text of the first reply without calls.
async fn ask(
    backend: &(impl Backend + Sync),
    sight: &mut Sight,
    conversation: &mut Vec<Item>,
    scene: &look::Scene,
    busy: &Busy,
    request: usize,
    progress: &mut (impl FnMut(Event) + Send),
) -> Result<String, ApiError> {
    let mut here = 0;
    let mut rounds = 0;
    loop {
        let choice = sight.tool.then(|| {
            if here >= MAX_PICTURES_PER_REQUEST || sight.shown >= MAX_PICTURES_PER_RUN { ToolChoice::None } else { ToolChoice::Auto }
        });
        let mut messages = vec![Item::text(Role::System, instructions_with(sight.pictures))];
        messages.extend(outgoing(conversation, sight.pictures, sight.tool));
        let reply = match backend.reply(&messages, choice).await {
            Ok(reply) => reply,
            Err(ApiError::Unsupported(_) | ApiError::BadRequest(_)) if sight.tool || sight.pictures => {
                let dropped = if sight.tool { sight.tool = false; Dropped::Tool } else { sight.pictures = false; Dropped::Pictures };
                progress(Event::Dropped(request, dropped));
                continue;
            }
            Err(error) => return Err(error),
        };
        if reply.calls.is_empty() {
            return Ok(reply.text);
        }
        rounds += 1;
        if rounds > MAX_TOOL_ROUNDS {
            return Err(ApiError::Other("The model kept asking for pictures without replying with a plan.".into()));
        }
        let mut pictures = vec![];
        for call in reply.calls {
            let answer = if call.name != look::NAME {
                Err(format!("There is no tool named {}.", proposal::clip(&call.name, 40)))
            } else if here >= MAX_PICTURES_PER_REQUEST {
                Err("No more pictures for this request; reply with the proposal.".into())
            } else if sight.shown >= MAX_PICTURES_PER_RUN {
                Err("No more pictures in this run; reply with the proposal.".into())
            } else {
                let (scene, arguments) = (scene.clone(), call.arguments.clone());
                blocking(busy, move || look::draw(&scene, &arguments))
                    .await
                    .unwrap_or_else(|_| Err("The picture could not be drawn.".into()))
            };
            conversation.push(Item::Call(call.clone()));
            match answer {
                Ok(picture) => {
                    here += 1;
                    sight.shown += 1;
                    conversation.push(Item::Output { call_id: call.call_id, output: picture.description.clone() });
                    progress(Event::Looked(request, Look { description: picture.description.clone(), purpose: picture.purpose.clone(), png: Some(picture.png.clone()) }));
                    pictures.push(picture);
                }
                Err(message) => {
                    conversation.push(Item::Output { call_id: call.call_id, output: message.clone() });
                    progress(Event::Looked(request, Look { description: message, purpose: String::new(), png: None }));
                }
            }
        }
        if !pictures.is_empty() {
            conversation.push(Item::Message(Role::User, pictures.into_iter().flat_map(|p| [
                Part::Text(p.description.clone()),
                Part::Image { png: p.png, alt: p.description },
            ]).collect()));
        }
    }
}
```

In `plan_with`: replace `messages` with `conversation: Vec<Item>` starting `vec![Item::text(Role::User, part)]` (the system message is added per send). After the report is ready:

```rust
    let mut scene = look::Scene { doc: Arc::new(doc.clone()), mesh: Arc::new(mesh.clone()), report: Arc::new(report.clone()), job: None };
    let mut sight = Sight { pictures: backend.pictures(), tool: backend.pictures(), shown: 0 };
    if sight.pictures {
        let s = scene.clone();
        if let Ok(Ok(picture)) = blocking(busy, move || look::overview(&s)).await {
            progress(Event::Overview(Look { description: picture.description.clone(), purpose: String::new(), png: Some(picture.png.clone()) }));
            conversation.push(Item::Message(Role::User, vec![
                Part::Text(format!("The program's overview picture: {}", picture.description)),
                Part::Image { png: picture.png, alt: picture.description },
            ]));
        }
    }
```

Each `backend.reply(&messages)` becomes `ask(backend, &mut sight, &mut conversation, &scene, busy, request, &mut progress).await`; each `messages.push((Role::X, t))` becomes `conversation.push(Item::text(Role::X, t))`. After `let (check, job) = …;` add `scene.job = job.clone().map(Arc::new);`.

`OpenAiBackend`: field `stream: Arc<Mutex<String>>` (replaces `received`), `pictures: bool`; `reply(messages, tool)` builds `Request { model, messages: messages.to_vec(), schema_name: "machining_plan", schema: proposal::schema(), tools: tool.map(|_| vec![look::tool()]).unwrap_or_default(), tool_choice: tool.unwrap_or_default() }` and calls `respond_plain_streaming` / `respond_streaming` as today's `plain` logic; `fn pictures(&self) -> bool { self.pictures }`.

`instructions_with(pictures)`: today's text; when `pictures`, insert before "RULES":

```text
PICTURES
- After the part report you get an overview picture drawn by the program: setup 1's orientation from the front-left, with the stock (amber, see-through) and setup 1's vise or clamps (grey). Tags in white boxes give each listed plane's surface IDs (at most three, then "…") and each hole's `diameter` (as Ø6.0). The caption names the view and has a scale bar in mm; the triad shows +X (red), +Y (green), +Z (blue).
- Call `view_model` to see more: another orientation or angle, zoomed on surfaces, surfaces highlighted in blue, layers on or off, and, once a proposal has been checked, one of its operations (`operation`, counted from 1 as in the check report): its toolpaths, and the stock left after it (cut stock green).
- You get at most 6 pictures per reply and 20 in the run. Older pictures are replaced by their descriptions. The pictures are drawn from the same model as the report; when they seem to disagree, the report's numbers are right.
```

Update `the_first_request_carries_the_report_and_no_private_data` only if it breaks; existing tests use `Script::new` (no pictures), so the conversation they see is unchanged.

- [ ] **Step 4: Run the tests**

Run: `cargo test 2>&1 | grep -E "test result|FAILED|panicked"`
Expected: all `ok`.

- [ ] **Step 5: Commit**

```bash
git add src/planner.rs src/main.rs
git commit -m "Planner: the overview, the view_model tool loop, its limits and fallbacks

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Read the unfinished reply

**Files:**
- Modify: `src/proposal.rs` (`partial`, `Draft`, tests)

**Interfaces:**
- Produces:

```rust
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DraftOp { pub strategy: Option<String>, pub name: Option<String>, pub tool: Option<u64>, pub rpm: Option<f64>, pub feed: Option<f64> }
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DraftSetup { pub name: Option<String>, pub orientation: Option<String>, pub operations: Vec<DraftOp> }
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Draft { pub summary: Option<String>, pub setups: Vec<DraftSetup>, /// The text is not a whole JSON object yet.
    pub writing: bool }
pub fn partial(text: &str) -> Draft;
```

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn a_reply_cut_anywhere_reads_as_far_as_it_has_come() {
        let (_, _, report) = block();
        let full = serde_json::to_string(&good(&report)).unwrap();
        let mut last_ops = 0;
        for end in (0..=full.len()).filter(|i| full.is_char_boundary(*i)) {
            let draft = partial(&full[..end]);
            let ops: usize = draft.setups.iter().map(|s| s.operations.len()).sum();
            assert!(ops + 1 >= last_ops, "at {end}: {ops} after {last_ops}");
            last_ops = ops;
        }
        let done = partial(&full);
        assert!(!done.writing);
        assert_eq!(done.summary.as_deref(), Some("Top side, then the underside."));
        assert_eq!(done.setups.len(), 2);
        let cut = partial(&full[..full.find("then").unwrap()]);
        assert_eq!(cut.summary.as_deref(), Some("Top side, "));
        assert!(cut.writing && cut.setups.is_empty());
        // A number being written is not shown until it ends.
        let at = full.find("\"rpm\":").unwrap() + 7;
        let draft = partial(&full[..at]);
        assert_eq!(draft.setups[0].operations[0].rpm, None);
    }
    #[test]
    fn text_before_the_json_and_a_fence_are_skipped() {
        let draft = partial("Here is the plan:\n```json\n{\"summary\": \"Face it\", \"setups\": [{\"name\": \"Top\"");
        assert_eq!(draft.summary.as_deref(), Some("Face it"));
        assert_eq!(draft.setups[0].name.as_deref(), Some("Top"));
        assert_eq!(partial("thinking…"), Draft::default());
        let done = partial("```json\n{\"summary\": \"x\", \"setups\": []}\n```");
        assert!(!done.writing);
    }
    #[test]
    fn partial_is_bounded_and_never_panics() {
        let deep = "{\"setups\":".to_string() + &"[".repeat(100_000);
        let _ = partial(&deep);
        let escape = "{\"summary\":\"a\\";
        assert_eq!(partial(escape).summary.as_deref(), Some("a"));
        let unicode = "{\"summary\":\"a\\u00";
        assert_eq!(partial(unicode).summary.as_deref(), Some("a"));
        let huge = format!("{{\"summary\":\"{}", "x".repeat(4 << 20));
        let started = std::time::Instant::now();
        let draft = partial(&huge);
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        assert!(draft.summary.is_some());
    }
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --bin axis-cam proposal::tests::a_reply_cut 2>&1 | tail -5`
Expected: compile error.

- [ ] **Step 3: Implement**

```rust
/// The most of a reply the timeline reads; plans are far shorter.
const PARTIAL_LIMIT: usize = 256 * 1024;
/// Nesting deeper than a proposal's is not read.
const PARTIAL_DEPTH: usize = 8;
/// The proposal as far as `text` has come: text before the first `{` is
/// skipped, a string cut short is closed, and a key, number or literal cut
/// short is left out with its member.
pub fn partial(text: &str) -> Draft {
    let Some(start) = text.find('{') else { return Draft::default() };
    let mut body = &text[start..];
    if let Some(Ok(value)) = serde_json::Deserializer::from_str(body).into_iter::<Value>().next() {
        return Draft::of(&value, false);
    }
    if body.len() > PARTIAL_LIMIT {
        let mut end = PARTIAL_LIMIT;
        while !body.is_char_boundary(end) { end -= 1; }
        body = &body[..end];
    }
    let mut end = body.len();
    for _ in 0..64 {
        let (candidate, boundary) = closed(&body[..end]);
        if let Some(candidate) = candidate
            && let Ok(value) = serde_json::from_str::<Value>(&candidate)
        {
            return Draft::of(&value, true);
        }
        match boundary {
            Some(b) if b < end => end = b,
            _ => break,
        }
    }
    Draft { writing: true, ..Draft::default() }
}
/// `prefix` closed into JSON (None when it ends inside a number, literal or
/// key), and where to cut it back to: before its last `,`, or just after its
/// last `{` or `[`, outside strings.
fn closed(prefix: &str) -> (Option<String>, Option<usize>) {
    let mut stack = vec![];
    let (mut in_string, mut escaped, mut boundary) = (false, false, None);
    for (i, c) in prefix.char_indices() {
        if in_string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' | '[' => {
                stack.push(if c == '{' { '}' } else { ']' });
                boundary = Some(i + 1);
                if stack.len() > PARTIAL_DEPTH { return (None, None) }
            }
            '}' | ']' => { stack.pop(); }
            ',' => boundary = Some(i),
            _ => {}
        }
    }
    let mut out = prefix.to_string();
    if in_string {
        // Drop an escape cut short: a lone backslash or a \u without 4 digits.
        if let Some(at) = out.rfind('\\') {
            let tail = &out[at..];
            if escaped || (tail.starts_with("\\u") && tail.len() < 6) { out.truncate(at); }
        }
        out.push('"');
    } else if out.trim_end().ends_with(|c: char| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == ':') {
        return (None, boundary);
    }
    out.extend(stack.iter().rev());
    (Some(out), boundary)
}
impl Draft {
    fn of(value: &Value, writing: bool) -> Self {
        let text = |v: &Value| v.as_str().map(String::from);
        Draft {
            summary: text(&value["summary"]),
            setups: value["setups"].as_array().into_iter().flatten().map(|s| DraftSetup {
                name: text(&s["name"]),
                orientation: text(&s["orientation"]),
                operations: s["operations"].as_array().into_iter().flatten().map(|o| DraftOp {
                    strategy: text(&o["strategy"]),
                    name: text(&o["name"]),
                    tool: o["tool"].as_u64(),
                    rpm: o["rpm"].as_f64(),
                    feed: o["feed"].as_f64(),
                }).collect(),
            }).collect(),
            writing,
        }
    }
}
```

Note: a key cut short (`{"summ`) closes to `{"summ"}`, which serde refuses, so the loop cuts back to after `{` — the expected behaviour. A string *value* cut short closes correctly. Adjust only if a test shows otherwise.

- [ ] **Step 4: Run the tests**

Run: `cargo test --bin axis-cam proposal:: 2>&1 | grep -E "test result|FAILED"`
Expected: ok.

- [ ] **Step 5: Commit**

```bash
git add src/proposal.rs
git commit -m "Read a planner reply that is still streaming

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: The timeline shows the plan being written, the looks and the drops

**Files:**
- Modify: `src/main.rs` (`plan_overview`, `Step.looks`, `record_plan_event`, `drop_plan`)
- Modify: `src/ui.rs:3420-3600` (`TimelineRow`, `timeline_rows`, `timeline_view`, running view), tests that call `timeline_rows`
- Modify: `src/smoke.rs` (screen 84's stream text)

**Interfaces:**
- Consumes: `planner::{Event, Look, Dropped}` (Task 6), `proposal::{partial, Draft}` (Task 7), `App.plan_stream` (Task 5).
- Produces:
  - `App.plan_overview: Option<Shown>`
  - `#[derive(Debug, Clone)] pub struct Shown { pub line: String, pub warn: bool, pub thumb: Option<iced::widget::image::Handle> }` in `main.rs`
  - `Step.looks: Vec<Shown>`
  - `TimelineRow.looks: Vec<Shown>`
  - `pub(crate) fn draft_lines(draft: &proposal::Draft) -> Vec<String>` in `ui.rs`
  - `pub(crate) fn look_line(look: &planner::Look) -> String` in `ui.rs`

- [ ] **Step 1: Write the failing tests** (in `src/ui.rs` tests, or the module where `timeline_rows` tests live — `grep -n "fn .*timeline" src/*.rs`)

```rust
    #[test]
    fn the_draft_reads_as_plan_and_setup_lines() {
        use crate::proposal::{Draft, DraftOp, DraftSetup};
        let op = |s: &str, n: &str, t, rpm, feed| DraftOp { strategy: Some(s.into()), name: Some(n.into()), tool: Some(t), rpm, feed };
        let draft = Draft {
            summary: Some("Face the top,\nthen pocket.".into()),
            setups: vec![
                DraftSetup { name: Some("Top".into()), orientation: Some("+Z".into()), operations: vec![op("Face", "Face top", 1, Some(8000.), Some(1200.)), op("Pocket", "Main pocket", 3, Some(9000.), None)] },
                DraftSetup { name: Some("Bottom".into()), orientation: Some("-Z".into()), operations: vec![op("Contour", "Outside", 2, Some(9000.), Some(1200.))] },
            ],
            writing: true,
        };
        assert_eq!(draft_lines(&draft), vec![
            "Plan: Face the top, then pocket.".to_string(),
            "Setup 1 · +Z up · Face \"Face top\" T1 · Pocket \"Main pocket\" T3".into(),
            "Setup 2 · -Z up · writing: Contour \"Outside\" T2, 9000 rpm, 1200 mm/min".into(),
        ]);
        assert_eq!(draft_lines(&Draft { writing: true, ..Draft::default() }), vec!["Thinking: the model has not started writing yet.".to_string()]);
    }
    #[test]
    fn a_look_reads_as_one_line() {
        let look = crate::planner::Look { description: "+Z up, top, zoomed on 12, 14; stock shown; tags: 12".into(), purpose: "is the pocket open on −Y?".into(), png: Some(std::sync::Arc::new(vec![])) };
        assert_eq!(look_line(&look), "Looked: +Z up, top, zoomed on 12, 14; stock shown; tags: 12 — \"is the pocket open on −Y?\"");
        let error = crate::planner::Look { description: "Surface 99 is not a surface of this part.".into(), purpose: String::new(), png: None };
        assert_eq!(look_line(&error), "Surface 99 is not a surface of this part.");
    }
```

and an App-level test (where the existing timeline tests build an `App`, e.g. `smoke::tests` — find with `grep -n "timeline_rows(" src/smoke.rs`):

```rust
    #[test]
    fn the_running_request_shows_the_draft_looks_and_drops() {
        use crate::planner::{Dropped, Event, Look, Progress};
        let mut a = app(); // the helper the other smoke tests use
        a.planner = crate::Planner::Running(Progress::Asking(1));
        a.record_plan_event(Event::Stage(Progress::Asking(1)));
        a.record_plan_event(Event::Looked(1, Look { description: "+Z up, top, whole part; stock shown; tags: none".into(), purpose: "overview".into(), png: Some(std::sync::Arc::new(vec![1])) }));
        a.record_plan_event(Event::Dropped(1, Dropped::Pictures));
        *a.plan_stream.lock().unwrap() = "{\"summary\":\"Face it\",\"setups\":[".into();
        let rows = crate::ui::timeline_rows(&a, std::time::Instant::now(), false);
        let row = rows.last().unwrap();
        assert_eq!(row.notes, vec!["Plan: Face it".to_string()]);
        assert_eq!(row.looks.len(), 2);
        assert!(row.looks[0].line.starts_with("Looked: "));
        assert!(row.looks[1].warn && row.looks[1].line == "OpenAI refused pictures; continuing with text only.");
        // Finished: one line, ending in the looks.
        a.record_plan_event(Event::Stage(Progress::Checking(1)));
        let rows = crate::ui::timeline_rows(&a, std::time::Instant::now(), true);
        assert!(rows[0].text.ends_with("· 1 look"), "{}", rows[0].text);
    }
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --bin axis-cam draft_lines a_look_reads the_running_request_shows 2>&1 | tail -5`
Expected: compile errors.

- [ ] **Step 3: Implement**

`main.rs`: `Step` gets `looks: Vec<Shown>` (init `vec![]`). `drop_plan` also clears `plan_overview`. `record_plan_event`:

```rust
            Event::Overview(look) => {
                self.plan_overview = Some(Shown { line: look.description.clone(), warn: false, thumb: look.png.as_ref().map(|png| iced::widget::image::Handle::from_bytes(png.to_vec())) });
            }
            Event::Looked(n, look) => {
                let line = ui::look_line(&look);
                let thumb = look.png.as_ref().map(|png| iced::widget::image::Handle::from_bytes(png.to_vec()));
                if let Some(step) = self.request_step(n) {
                    step.looks.push(Shown { line, warn: false, thumb });
                }
            }
            Event::Dropped(n, what) => {
                let line = match what {
                    planner::Dropped::Tool => "OpenAI refused the picture tool; continuing with the overview only.",
                    planner::Dropped::Pictures => "OpenAI refused pictures; continuing with text only.",
                };
                if let Some(step) = self.request_step(n) {
                    step.looks.push(Shown { line: line.into(), warn: true, thumb: None });
                }
            }
```

with `fn request_step(&mut self, n: usize) -> Option<&mut Step>` = the last step whose stage is `Asking(n) | Repairing(n) | Extending(n)` (reuse it in the `Checked` arm's `reply` branch).

`ui.rs`:

```rust
/// What the model has written of its reply so far, as timeline lines.
pub(crate) fn draft_lines(draft: &crate::proposal::Draft) -> Vec<String> {
    if draft.summary.is_none() && draft.setups.is_empty() {
        return vec!["Thinking: the model has not started writing yet.".into()];
    }
    let mut lines = vec![];
    if let Some(summary) = &draft.summary {
        lines.push(format!("Plan: {}", clip(summary, 300)));
    }
    let last = draft.setups.len().saturating_sub(1);
    for (i, setup) in draft.setups.iter().enumerate() {
        let mut parts = vec![format!("Setup {}", i + 1)];
        if let Some(up) = &setup.orientation {
            parts.push(format!("{} up", clip(up, 4)));
        }
        let n = setup.operations.len();
        for (k, op) in setup.operations.iter().enumerate() {
            let mut words = format!(
                "{} \"{}\"",
                clip(op.strategy.as_deref().unwrap_or("…"), 20),
                clip(op.name.as_deref().unwrap_or("…"), 40)
            );
            if let Some(t) = op.tool {
                words += &format!(" T{t}");
            }
            if draft.writing && i == last && k + 1 == n {
                if let Some(rpm) = op.rpm { words += &format!(", {rpm:.0} rpm"); }
                if let Some(feed) = op.feed { words += &format!(", {feed:.0} mm/min"); }
                words = format!("writing: {words}");
            }
            parts.push(words);
        }
        lines.push(parts.join(" · "));
    }
    lines
}
pub(crate) fn look_line(look: &crate::planner::Look) -> String {
    match (&look.png, look.purpose.is_empty()) {
        (Some(_), false) => format!("Looked: {} — \"{}\"", clip(&look.description, 200), clip(&look.purpose, 120)),
        (Some(_), true) => format!("Looked: {}", clip(&look.description, 200)),
        (None, _) => clip(&look.description, 300),
    }
}
```

A look is a picture when it has a PNG; a look without one is an error sentence and is shown bare.

`TimelineRow` gets `pub looks: Vec<crate::Shown>`. In `timeline_rows`, replace the "received N characters" note with: when `step.ended.is_none() && request(step.stage)`, `notes.extend(draft_lines(&crate::proposal::partial(&app.plan_stream.lock().unwrap())))`. Set `looks: step.looks.clone()`. In the finished branch append `· {plural(n, "look")}` to `text` where `n` = looks with a thumbnail, when `n > 0`. Mark a request row whose looks contain a `warn` with `Mark::Warn` only when it has no other result (keep existing marks otherwise).

`timeline_view`: under each row's notes push, for every look, `row![thumb (iced::widget::image(handle).width(120)), muted(line)]` (the thumbnail only when present; the warning colour for `warn`). In the running view, before the timeline rows, show `plan_overview` when present: `column![bold("The AI starts with this picture", 15), image(handle).width(240), muted(description)]`.

In `src/smoke.rs`, screen 84 sets a real partial reply instead of the 1840 x's: `*self.plan_stream.lock().unwrap() = "{\"summary\":\"Face the top, rough the pocket with T3, then turn the part over.\",\"setups\":[{\"orientation\":\"+Z\",\"name\":\"Top\",\"origin\":\"Stock top center\",\"workholding\":\"Vise\",\"operations\":[{\"strategy\":\"Face\",\"name\":\"Face top\",\"tool\":1".into();`.

Update existing timeline tests that expected `"received N characters"` to expect the draft lines instead (`grep -n "received" src/*.rs`).

- [ ] **Step 4: Run the tests**

Run: `cargo test 2>&1 | grep -E "test result|FAILED|panicked"`
Expected: all `ok`.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/ui.rs src/smoke.rs
git commit -m "Planner timeline: the plan as it is written, the pictures the model looked at, refusals

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: The pictures checkbox and the disclosure

**Files:**
- Modify: `src/settings.rs` (field, test), `src/main.rs` (`Message::PlanPictures(bool)`, pass setting), `src/ui.rs:2773-2800` (`SENT_TO_OPENAI`, `planner_controls`)

**Interfaces:**
- Produces: `Settings.plan_pictures_off: bool`; `Message::PlanPictures(bool)`; `pub(crate) const PICTURES_SENT: &str`; `pub(crate) fn sent_to_openai(pictures: bool) -> String`.

- [ ] **Step 1: Write the failing tests**

`src/settings.rs`:

```rust
    #[test]
    fn pictures_are_on_unless_turned_off_and_the_choice_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert!(!load(&path).plan_pictures_off);
        let mut s = load(&path);
        s.plan_pictures_off = true;
        save(&path, &s).unwrap();
        assert!(load(&path).plan_pictures_off);
        // Settings saved before the field existed read as pictures on.
        std::fs::write(&path, br#"{"host_id":"urn:uuid:3b241101-e2bb-4255-8caf-4136c566a962","model":"gpt-6.1-sol"}"#).unwrap();
        assert!(!load(&path).plan_pictures_off);
    }
```

`src/ui.rs` tests (or smoke tests, where an `App` is available):

```rust
    #[test]
    fn the_disclosure_names_pictures_only_when_they_are_on() {
        assert!(sent_to_openai(true).contains("pictures the app draws of the part"));
        assert!(!sent_to_openai(false).contains("pictures"));
        assert!(sent_to_openai(true).contains("The CAD file"));
    }
```

and in `src/smoke.rs` tests: `Message::PlanPictures(false)` sets `settings.plan_pictures_off` and saves it (follow `plan_notice_shown`'s test at `src/smoke.rs:6583`).

- [ ] **Step 2: Run to see them fail**

Run: `cargo test --bin axis-cam pictures_are_on the_disclosure 2>&1 | tail -5`
Expected: compile errors.

- [ ] **Step 3: Implement**

`Settings`: `/// The AI planner may not show the model pictures of the part. pub plan_pictures_off: bool,`.

`ui.rs`:

```rust
pub(crate) const PICTURES_SENT: &str = " With the box below ticked, pictures the app draws of the part, the stock and setup 1's vise or clamps, with surface and hole labels, are sent too, and, when the model asks, pictures of its own toolpaths.";
pub(crate) fn sent_to_openai(pictures: bool) -> String {
    let mut text = SENT_TO_OPENAI.to_string();
    if pictures {
        text.push_str(PICTURES_SENT);
    }
    text
}
```

In `planner_controls`, replace `muted(SENT_TO_OPENAI)` with `muted(sent_to_openai(!app.settings.plan_pictures_off))` followed by `checkbox(!app.settings.plan_pictures_off).label("Let the AI see pictures of the part").on_toggle(Message::PlanPictures).text_size(13)` (match how other checkboxes in `ui.rs` are built: `grep -n "checkbox(" src/ui.rs`).

`main.rs`: `Message::PlanPictures(on) => { self.settings.plan_pictures_off = !on; self.save_settings(); }`; `Message::Plan` passes `!self.settings.plan_pictures_off` to `OpenAiBackend::new`.

- [ ] **Step 4: Run the tests**

Run: `cargo test 2>&1 | grep -E "test result|FAILED|panicked"`
Expected: all `ok`.

- [ ] **Step 5: Commit**

```bash
git add src/settings.rs src/main.rs src/ui.rs src/smoke.rs
git commit -m "Planner: a setting for pictures and the disclosure that names them

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: Smoke screen, README and VALIDATION

**Files:**
- Modify: `src/smoke.rs` (screen 91, screens 84 and 87 data)
- Modify: `README.md` (AI planner section, smoke count, module list), `docs/VALIDATION.md` (new section)

- [ ] **Step 1: Add the screen**

In `src/smoke.rs`: add `114 => "91-planner-pictures".into(),` to the name table, and a step `114` that sets `planner = Running(Repairing(2))`, a timeline like screen 84's, `plan_overview` from `crate::look::overview(&crate::look::Scene { doc: Arc::new(self.doc.clone()), mesh: Arc::new(mesh), report: Arc::new(crate::analysis::report(&self.doc, &mesh).unwrap()), job: None })` (use `self.imported_mesh`, as `smoke_plan_preview` does), two looks on the open row (one with a thumbnail from a `look::draw` call with `"angle": "top"`, one error line), and a `Dropped::Tool` line on the first request. Extend the run's last step from 113 to 114 (find where the smoke run ends: `grep -n "smoke_step" src/smoke.rs | grep -E ">|==" `). Add looks to screen 87's finished timeline so a row ends "· 2 looks".

- [ ] **Step 2: Run the smoke screens and inspect them**

Run: `cargo run --release -- --smoke 2>&1 | tail -5` (use the flag the README's smoke section names) and open the PNGs for screens 84, 87 and 91 with the Read tool.
Expected: the overview and thumbnails visible, text not clipped or overlapping; note anything cosmetic as earlier VALIDATION entries do.

- [ ] **Step 3: Update the docs**

README: in the AI planner section, a paragraph "**Pictures.**" describing the overview, the `view_model` tool and its options, the limits (6 / 20 / 8), the fallbacks and the checkbox; the timeline paragraph now says it shows the plan as it is written, the pictures the model looked at and refusals (no character count); the smoke paragraph: ninety-one screens, 115 PNG files, screen 91; the module list gains `src/look.rs: pictures of the part for the AI planner (the overview and the view_model tool)`. VALIDATION: a "Planner pictures" section naming each new test, as earlier sections do, and the screens inspected with the date.

- [ ] **Step 4: Full verification**

Run: `cargo test 2>&1 | grep "test result"` and `cargo clippy --all-targets 2>&1 | grep -c warning`
Expected: every result `ok`; no new warnings compared with `main`.

- [ ] **Step 5: Commit**

```bash
git add src/smoke.rs README.md docs/VALIDATION.md
git commit -m "Document planner pictures: smoke screen 91, README and VALIDATION

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
