# Material library, speed and feed checks, and planner progress — design

Date: 4 October 2026. Branch: `materials-progress` (from `main` 121b0a2).

## Goal

1. Replace the free-text project material with a **material library** that works like the tool library and holds cutting data.
2. **Check speeds and feeds** against the selected material: a warning for the user's own operations, a refusal (sent back to the model) for AI planner plans.
3. Show the **AI planner's progress** as a live timeline while it runs.

## Decisions made by the user

- A material holds a name, group, hardness, notes and cutting data (surface speed range; chip load per tooth by tool diameter).
- Out-of-range speeds and feeds: warn for the user's own operations (export still allowed); refuse planner plans.
- The library starts with conservative starter materials, marked as starting points.
- Progress view: a timeline of every step with elapsed times, a live count of the model's reply, and each check's result with the problems sent back.

## Non-goals

Cutting-load, power or deflection models; coated/uncoated or HSS tool distinctions (cutting data is for carbide end mills and drills); per-tool material overrides; showing the model's raw reply text.

## Material library

### Data

A material is stored like a tool: `Material { id, p: Fields }` in `Document.materials`. Keys:

| Key | Meaning |
| --- | --- |
| `name` | Display name, required, unique in the library (case-insensitive). |
| `group` | One of: Aluminium, Steel, Stainless steel, Brass and copper, Plastic, Wood and composite, Other. |
| `hardness` | Free text, for example "95 HB" (may be empty). |
| `notes` | Free text, for example coolant advice (may be empty, at most 500 characters). |
| `vc_min`, `vc_max` | Surface speed range in m/min for carbide tools. |
| `fz3_min`, `fz3_max` | Chip load per tooth in mm for tool diameters up to 3 mm. |
| `fz6_…`, `fz10_…`, `fz16_…` | The same for diameters over 3 up to 6, over 6 up to 10, over 10 up to 16 mm. |
| `fzbig_min`, `fzbig_max` | The same for diameters over 16 mm. |

A material **has cutting data** when all twelve numbers are present, finite, positive, and each min ≤ max. A material may exist without cutting data (for example one created from a legacy name); it can be selected but the checks and the planner then report that its cutting data is missing.

The project's material is a reference: `Document.material_id` (the selected material's id, empty for none). `Document.material` (the name) is kept for the setup sheet, the G-code header and old readers, and is always set from the selected material's name.

Like tools and machines, `materials` are carried to a new project and merged by **Explore example** without duplicates (match by name, case-insensitive).

### Starter materials

A new workspace, a document without any materials, and **Explore example** get these entries, each with the note "Starting point for carbide tools: adjust to your machine and your tool maker's data." followed by the material-specific note. Values (Vc m/min; fz mm for ≤3 / ≤6 / ≤10 / ≤16 / >16 mm):

| Name | Group | Hardness | Vc | fz | Note |
| --- | --- | --- | --- | --- | --- |
| Aluminium 6061-T6 | Aluminium | 95 HB | 150–300 | 0.010–0.030 / 0.020–0.050 / 0.030–0.080 / 0.050–0.100 / 0.060–0.120 | Flood or mist; clear chips. |
| Aluminium 7075-T6 | Aluminium | 150 HB | 150–280 | 0.010–0.025 / 0.020–0.045 / 0.030–0.070 / 0.040–0.090 / 0.050–0.110 | Flood or mist; clear chips. |
| Mild steel 1018 | Steel | 126 HB | 80–150 | 0.005–0.015 / 0.010–0.030 / 0.020–0.050 / 0.030–0.070 / 0.040–0.090 | Flood; avoid rubbing at low feed. |
| Stainless steel 304 | Stainless steel | 201 HB | 50–100 | 0.005–0.012 / 0.010–0.025 / 0.015–0.040 / 0.020–0.050 / 0.030–0.070 | Flood; keep the tool cutting, it work-hardens. |
| Brass C360 | Brass and copper | 78 HRB | 150–300 | 0.010–0.030 / 0.020–0.050 / 0.030–0.070 / 0.040–0.090 / 0.050–0.110 | Dry or air; sharp tools. |
| POM (Delrin) | Plastic | | 150–400 | 0.020–0.050 / 0.040–0.100 / 0.060–0.150 / 0.080–0.200 / 0.100–0.250 | Air; avoid melting with high feed per tooth. |
| Acrylic (PMMA) | Plastic | | 100–300 | 0.020–0.050 / 0.030–0.080 / 0.050–0.120 / 0.060–0.150 / 0.080–0.180 | Air; single-flute tools give the best finish. |
| HDPE | Plastic | | 150–400 | 0.030–0.070 / 0.050–0.120 / 0.080–0.180 / 0.100–0.220 / 0.120–0.250 | Air; it deflects, use light finishing passes. |
| MDF | Wood and composite | | 200–500 | 0.030–0.080 / 0.050–0.150 / 0.080–0.200 / 0.100–0.250 / 0.120–0.300 | Dust extraction; no coolant. |

### Migration

When a document is loaded (workspace, project file, recovery) and `material_id` is empty while `material` (a name) is not:
- if a library material has that name (case-insensitive), it is selected;
- otherwise a material with that name, group Other and no cutting data is added and selected.
Either way the status says what happened ("The project material "X" was linked to the library." / "… was added to the material library without cutting data; add it on the Materials page."). Documents saved before this change also get the starter materials added (without duplicating names).

### UI

- A **Materials** page beside **Tools**, built like it: a list (name, group, Vc range), an editor with every field, **+ New material**, **Save material**, **Delete material** (refused while the project uses it). Field help explains Vc and fz.
- The setup editor's **Project material** text field becomes a picker of library materials, with "Open material library". When the selected material has no cutting data, a note says so.
- Setup sheet and G-code header: unchanged text, still the material name.

## Speed and feed checks

For an operation whose tool has a positive diameter D (mm) and flutes z ≥ 1, with spindle speed n (rpm) and feed F (mm/min):
- surface speed Vc = π · D · n / 1000 (m/min);
- chip load per tooth fz = F / (n · z) (mm), using the tool's flutes for drills too;
- the chip-load band is chosen by D (≤3, ≤6, ≤10, ≤16, >16 mm).

The operation is **in range** when both hold (1 % slack on each limit for rounding):
- Vc ≤ vc_max, and Vc ≥ vc_min **or** n is at the machine's maximum rpm (a small tool cannot reach the minimum surface speed);
- fz ≤ band max, and fz ≥ band min **or** F is at the machine's maximum feed.

Ball end mills use the nominal diameter (the effective diameter at shallow depth is smaller, so the check is conservative on surface speed only in one direction; documented). Plunge and ramp feeds are not checked. Operations without a tool, a material without cutting data, or a tool without diameter or flutes are not checked; the check reports why.

**User's own operations:** the operation editor shows a warning line with the computed values and the material's range ("Surface speed 452 m/min is above Aluminium 6061-T6's 150–300 m/min."). The NC program page lists the same warnings per operation; export still goes ahead.

**AI planner:**
- The part report's `material` becomes an object: name, group, hardness, notes, vc range and the five fz bands.
- The planning preconditions require a selected material **with cutting data** ("Material: add cutting data to X on the Materials page.").
- A new plan-check stage, after structure and before generation, refuses any operation out of range, naming the value, the range and the operation (stage "cutting data"); its problems go back to the model like any other.
- The instructions explain the two formulas, the bands, the machine-limit exceptions, and that out-of-range plans are refused.
- The preview's "Not checked" note becomes: "Not checked: cutting loads, tool deflection, fixtures, clamps and the machine body. Speeds and feeds are checked against the material's cutting data, which are starting points."

## Planner progress

### Events

The planner reports a timeline instead of a single stage:
- each **step** starts and ends: Analyzing, Asking(n), Repairing(n), Extending(n), Checking(n), Previewing;
- while a request streams, the number of reply characters received so far;
- when a check ends: passed, or the problems that go back to the model (the same cleaned, bounded text as on a failed run: at most 5 shown per check, then "and N more"), or that the reply could not be read (the parse error, cleaned and bounded).

The OpenAI client reports streamed text length through a counter shared with the planner backend; no reply text is shown.

### UI

While running, the planner pane shows the timeline: one row per step with a tick or a spinner, its elapsed time (seconds, updated by the existing 500 ms tick), and for the current request "received N characters". Check rows expand to their result. **Cancel** stays. When the run ends, the timeline stays above the preview or the failure ("How the plan was made" / "What the planner tried"), collapsed to one line per step.

### Rules kept

Model text shown only cleaned and bounded; nothing private sent; a run's events from an earlier epoch are ignored; the timeline is cleared with the plan (`drop_plan`).

## Testing

- Materials: data validation, starter data present and valid, migration (linked, created, starters added), Explore example and New project carry materials without duplicates, delete refused while used, picker sets `material_id` and `material`.
- Checks: Vc and fz arithmetic, bands, machine-limit exceptions, slack, unchecked cases; warnings on the operation editor and NC page; the plan-check stage refuses out-of-range plans and passes in-range ones; the part report carries the material object; preconditions require cutting data.
- Progress: the planner's events for each loop path (pass first time, repair, coverage round, parse failure, API error), reply-length updates from the client (loopback fake), epochs, timeline kept after the run and cleared on drop, page builds in every state; renderer smoke screens for the running timeline and the finished timeline.

## Documentation

README (Materials, the checks, the progress view), VALIDATION (tests behind each statement), help texts for every new control, the preview's not-checked note.

## As built (5 October 2026)

The implementation follows this design except for the points below.

- **Starters are added only to an empty library.** A new workspace, a document or workspace saved before the library existed (it has no materials), a new project that carries an empty library, and Explore example with an empty library get the nine starters. A library that has any entry is never topped up, so a starter the user deleted does not come back; the earlier wording, "documents saved before this change also get the starter materials added", holds for those documents because their library is empty. Explore example merges the user's materials by name and then links the example's material to the entry of that name, or adds it without cutting data when the user deleted it, and the status says which. A name is matched ignoring ASCII capitals.
- **The speed and feed check** also counts a spindle speed within 0.5 rpm of the machine's maximum, and a feed within 0.5 of its maximum (an inch machine's limit converted to mm/min), as at the maximum. An operation is also reported as not checked, with the reason, when it has no spindle speed or feed or when its tool is not in the library. Drills are checked with their flutes and the end-mill bands, as designed, and the README calls that a rough guide.
- **The warnings** are an amber box on the operation editor's Tool tab and a list on the NC program page for enabled operations only (a suppressed operation is not listed); its heading is "Speeds and feeds outside the material's cutting data (export still allowed)" when any line is a problem and "Speeds and feeds not checked (export still allowed)" otherwise.
- **The plan-check stage** is called "cutting data" and runs after the structure stage, but it does not stop the stages after it, and its problems are listed after the generation and export problems and before the tool-axis and gouge ones, so one correction round carries all of them. Its messages are clipped to one line of at most 300 characters.
- **The part report** gives `material` as `name`, `group`, `hardness`, `notes` (cleaned, at most 500 characters), `vc_min_m_per_min`, `vc_max_m_per_min` and `chip_load_mm`, a list of five bands `{up_to_mm, min, max}` (`up_to_mm` null for the last). Each tool in the report also carries `flutes`, which the chip load needs. The planning checklist says "Choose a material on the setup page." or "Add cutting data to X on the Materials page.".
- **Planner progress** is reported as events: a stage start, and the result of a reply's check (passed, problems, or unreadable). A step ends when the next stage starts or the run ends, so there is no separate end event. A running step is marked with a bullet, not a spinner; a check with problems or an unreadable reply gets an amber "!"; the check's result is listed under its row while the run goes (up to five problems and "and N more"), not behind an expand control. When the run is over the timeline is one line per step ("passed", "N problems", "reply could not be read"), under "How the plan was made" above a preview or "What the planner tried" above a failure. The count of reply characters belongs to the open request only and starts at zero with each request.
- **The material name is printed by the setup sheet only.** The setup sheet shows it as "Material: …" under the title. The G-code header does not print the material, and never did; the design's lines 41 ("kept for the setup sheet, the G-code header …") and 72 ("Setup sheet and G-code header: unchanged text, still the material name") were wrong about that, and nothing in the header changed.
- **What the planner is sent of the project material** is its name (at most 60 characters), group (40), hardness (40) and notes (500) and its cutting data; the in-app texts beside the Plan button, on the material picker and in the help for the name, group, hardness and notes say so.
- **Screens:** the renderer smoke run has `84-planner-running` (the running timeline), `86-materials` and `87-planner-timeline` (a finished run above its preview).

