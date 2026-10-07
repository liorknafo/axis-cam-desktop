# Planner pictures and live progress — design

Date: 7 October 2026. Branch: `planner-pictures` (from `main` 33b6fef).

## Goal

1. Let the AI planner **see the part**: the first request carries a labelled overview picture, and a `view_model` tool lets the model ask for any other view, including the toolpaths and remaining stock of its own proposal.
2. Show **what the model is doing** while it runs, instead of a count of characters: the plan as it is written, and every picture it asked for, with its reason.

## Decisions made by the user

- Pictures show the part with labels (surface IDs and holes), the stock and setup 1's vise or clamps.
- The model decides what to look at, with every display mode the 3D view has (orientation, angle, zoom, stock, workholding, labels, highlighted surfaces, an operation's toolpaths and the stock left after it).
- Approach: one labelled overview up front, and the tool for everything else. If the plan route refuses the tool, the overview remains; if it refuses pictures, the run goes on with text only and says so.
- The earlier "live progress" design (the plan's summary, setups and operations shown as the reply streams) is part of this work.

## Non-goals

Streaming the model's reasoning summary; free camera angles beyond the listed ones; sending the CAD file or the mesh; enlarging a thumbnail in the app; letting the model change anything but its proposal; the new aluminium starter materials (a separate change).

## Pictures

### Drawing

`Viewport::pixels` (`src/viewport.rs`) is split so the scene can be drawn to RGBA pixels without Iced's canvas: a plain `Scene` (mesh, setup, operation, simulation, holding boxes, show stock, show paths, highlighted surfaces, camera) and a function that returns `(w, h, rgba)`. The viewport keeps drawing through it; nothing it shows changes. The hover highlight becomes one case of the highlighted surfaces.

A new module `src/look.rs` draws the planner's pictures on top of that:

- **Size:** 768 × 768 pixels, PNG (the existing `png` crate), sent as a `data:image/png;base64,…` URL (the existing `base64` crate) with `detail` `high`.
- **Frame:** the part is drawn in the chosen orientation's frame: a setup with setup 1's stock settings and that orientation's rotation from `analysis::ORIENTATIONS`.
- **Tags:** small boxes with text in a built-in 5 × 7 pixel font (digits, capitals, `+ - . , Ø …`):
  - each plane the report lists for the drawn orientation, at the centre of its `bbox` and at its depth, reading its surface IDs joined with commas (at most three, then `…`);
  - each hole of those planes, at its centre, reading its `diameter` as `Ø6.0`.
  
  A tag is drawn only where its point is visible (depth test against the drawn part) and inside the picture; overlapping tags are skipped, the larger plane first.
- **Caption strip** along the bottom: the view in words (`+Z up · isometric front-left · zoomed on 12, 14`), a scale bar with its length in mm, and the axis triad.
- **Colours:** as the 3D view: stock amber and translucent, vise and clamps grey, toolpaths as in the operation preview, highlighted surfaces in the selection tint.

Each picture comes with a one-line description for the model and the timeline, for example "+Z up, isometric front-left, zoomed on surfaces 12, 14; stock and vise shown; tags: 3, 7, 12, 14, Ø6.0". The description lists only the tags actually drawn.

### The overview

After the part analysis, and before the first request, the app draws the overview: setup 1's orientation, isometric front-left, the whole scene, stock and workholding shown, tags on. It goes into the first user message as an image part after the part report. The planner page shows it once the run has it, under the heading "The AI starts with this picture".

### The `view_model` tool

A function tool with a strict schema (every field required, nullable where marked):

| Field | Type | Meaning |
| --- | --- | --- |
| `orientation` | `+Z`, `-Z`, `+X`, `-X`, `+Y`, `-Y` or null | The report's orientation to draw the part in. Null: setup 1's, or the operation's setup when `operation` is set. |
| `angle` | `top`, `front`, `back`, `left`, `right`, `iso_front_left`, `iso_front_right`, `iso_back_left`, `iso_back_right` | Where the camera looks from, in that orientation's frame. |
| `focus_surfaces` | integers | Fit the camera to these surfaces; empty fits the whole scene. |
| `highlight_surfaces` | integers | Surfaces drawn in the highlight tint. |
| `show_stock` | boolean | The stock box, or with `operation` the stock left after it. |
| `show_workholding` | boolean | Setup 1's vise or clamps (with `operation`: that setup's laid-out ones). |
| `labels` | boolean | The surface and hole tags. |
| `operation` | `{setup, operation}` or null | An operation of the latest built proposal, counted from 1 as in the check report: its toolpaths, and the simulated stock after it. |
| `purpose` | string | A few words on why it looks; shown to the user, clipped to 120 characters. |

Each angle is a fixed `Camera` yaw and pitch (`top` looks straight down). An unknown surface ID, an `operation` outside the latest proposal, or `operation` before any proposal has been built returns a one-sentence error as the tool's output, without a picture; it still counts against the limits.

To draw an operation, the plan check returns the job it built even when the proposal fails (today `plan_check::check` drops it unless the plan passed). The picture uses `simulation::prepare` on that job and seeks to the operation's end, as `planner::final_view` does for the whole job.

## Requests

### Conversation items

`openai::Request` carries input items instead of `(Role, String)` pairs:

- a message: role and parts, each part text or a PNG image;
- a function call, replayed as the model sent it (`call_id`, `name`, `arguments`), without its item `id` (requests are sent with `store: false`, so stored ids cannot be referred to);
- a function call output: `call_id` and text.

Reasoning items are not replayed. The planner's `Backend::reply` takes the items, whether to offer the tool, and whether tool calls are still allowed, and returns a `Reply` with the text and the function calls.

### Streaming

The accumulator also folds `response.output_item.done` items of type `function_call` (and, when nothing was streamed, the `function_call` items in `response.completed`'s output). A reply may hold text, calls, or both.

The streamed reply text is kept in a shared `Arc<Mutex<String>>` that replaces the character counter; it is cleared when each attempt starts.

### The tool loop

Inside one planner request (one of the four `MAX_REQUESTS`):

1. Send the conversation, with the tool offered when pictures are on and the tool has not been refused.
2. If the reply has calls: draw each picture on the blocking pool (`busy`), append the call, its output (the description, or the error), and one user message holding the pictures of this round with their descriptions (no message when every call was an error); then send again. These rounds do not count as planner requests.
3. Otherwise the reply's text is the proposal, read and checked as today.

**Limits:** at most 6 pictures per planner request and 20 per run. Calls beyond a limit get the output "No more pictures for this request; reply with the proposal." (or "… in this run …"), and every later send sets `tool_choice` to `none`: until the next planner request for the per-request limit, for the rest of the run for the per-run limit. Only the 8 most recent pictures are sent as images; an older picture's image part is replaced with the text "(Earlier picture, not resent: <description>)". The overview counts as a picture for this rule but not for the limits.

### When OpenAI refuses something

A request that fails with `Unsupported` or HTTP 400 (a new `ApiError::BadRequest` for a 400 without a known code) while it carries something optional is sent again without the next optional thing, in this order:

1. the tool (the overview stays);
2. every picture (image parts become their descriptions);
3. structured output (as today, only on `Unsupported`).

The backend remembers for the rest of the run what it dropped, as it does today for structured output, and the run reports each drop as an event. A failure with nothing left to drop is reported as today.

### The instructions

`planner::instructions()` gains a PICTURES section: what the overview shows; the tool and its fields; what the tags mean (a plane's surface IDs; a hole's `diameter`); the colours; the limits; that pictures are drawn by the program from the same model as the report, and that the report's numbers are authoritative when they seem to disagree. With pictures off or refused, the section is left out.

## What the user sees

### Live progress

`proposal::partial(text)` reads an unfinished reply: it skips text before the first `{` and a code fence, and closes an object cut anywhere (inside a key, a string, a number or an escape) so serde can read what has arrived. It returns the summary (possibly cut), and the setups, each with its name, orientation and operations (strategy, name, tool, rpm, feed), the last one possibly incomplete.

Under the running request row, in place of "received N characters":

- before any text: "Thinking: the model has not started writing yet." (plus the looks, below);
- `Plan: <summary>` (cleaned, one line, at most 300 characters);
- one line per setup: `Setup 1 · +Z up · Face "Face top" T1 · Pocket "Main pocket" T3`, with the operation being written as `writing: Contour "Outside" T2, 9000 rpm, 1200 mm/min`.

Every text the model wrote is cleaned and clipped with `proposal::clip`.

### Looks

Each picture is reported as an event (`Event::Looked`) with its description, the model's `purpose` and the PNG. Under its request row it is a line with a 120 px wide thumbnail:

`Looked: +Z up, isometric front-left, zoomed on 12, 14 — "is the pocket open on the −Y side?"`

An error output is a line without a thumbnail ("Asked for surface 99, which the report does not list."). A dropped capability is a line with the warning mark ("OpenAI refused pictures; continuing with text only."). In the finished timeline a request's one line ends with "· N looks" when it had any.

### Control and disclosure

- The AI planner page gets a checkbox "Let the AI see pictures of the part", saved in `Settings` as `plan_pictures_off` (false by default, so pictures are on). Off: no overview, no tool, no PICTURES section.
- `SENT_TO_OPENAI` adds, while the box is ticked: "pictures the app draws of the part, the stock and setup 1's vise or clamps, with surface and hole labels, and, when the model asks, of its own toolpaths". The sentence still says the CAD file is not sent.

## Testing

- **Drawing:** a block with a pocket: the overview has the expected size; the pocket's floor tag is drawn and listed in the description; a hidden plane's tag is not; a highlighted surface's pixels carry the tint; `focus_surfaces` makes the surface larger than in the whole-scene view; the viewport's pixels are unchanged by the split.
- **The font and tags:** every character the tags use has a glyph; overlapping tags keep the larger plane's.
- **Tool arguments:** unknown surface, operation out of range, operation before any proposal, and a `purpose` that needs cleaning.
- **Streaming:** recorded event sequences with a function call, with text and a call, and with calls only in `response.completed`.
- **The loop:** calls answered with outputs and one picture message; a request id never replayed; the per-request and per-run limits and `tool_choice` `none`; older pictures replaced by their text; tool rounds not counted as planner requests.
- **Refusals:** the fake OpenAI server refuses the tool, then pictures, then structured output; each drop is remembered for the run and reported.
- **`partial`:** a real proposal cut at every byte gives no panic and a growing summary and setup list; text before the JSON and a code fence are skipped.
- **The timeline:** rows while running and finished, with thinking, plan lines, looks, errors and drops.
- **Settings and disclosure:** the checkbox's default, save and effect on the request; the `SENT_TO_OPENAI` text.
- **Smoke screens:** the running timeline with plan lines and thumbnails, the planner page with the overview, and a finished timeline with looks.
- **Not testable here:** whether the user's ChatGPT plan route accepts tools and images for their model. A real run by the user confirms it; the fallbacks cover a refusal.

README and `docs/VALIDATION.md` are updated as for earlier work.
