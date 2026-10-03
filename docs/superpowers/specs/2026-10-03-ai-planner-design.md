# One-button AI planner — design

Date: 3 October 2026. Approved in conversation before this spec was written. Branch: `ai-planner`, from `linuxcnc-post`.

## Goal

After the user imports a part and configures the first setup's machine, stock and material, one click on **Plan** produces a complete machining job: setups, operations, tools, surface selections and cutting parameters. The plan is proposed by an OpenAI model running on the user's ChatGPT plan. The app verifies it with the real generators, the stock simulation and the LinuxCNC export checks, and asks the model to repair failures. The user sees a preview and applies it with one more click.

## Decisions already made by the user

- Model access uses **Sign in with ChatGPT** (subscription), not an API key.
- Default model: `gpt-6.1-sol`, selectable from the models the plan allows.
- The planner uses **only tools already in the library**. Features it cannot machine with them are reported.
- The result is shown as a **preview**; nothing changes until the user clicks **Apply**.

## Non-goals

- New machining strategies, multi-axis setups, or orientations that are not multiples of 90°.
- Adding tools, choosing the material, or sizing the stock.
- Modelling clamps or fixtures. Workholding notes are advice.
- Validating feeds and speeds against cutter load. They are capped to machine limits only.
- An API-key fallback, a headless planning command, multiple ChatGPT accounts, image input.
- The parked LinuxCNC items (install, merge, the non-90° work-zero gap).

## Delivery

Three milestones. Each has its own implementation plan and review cycle, and leaves working, tested software.

1. **M1 — Sign-in and connection.** The user can sign in, see their plan's models and run a connection check.
2. **M2 — Part analysis and plan checker.** No AI. A hand-written plan file can be checked from the command line.
3. **M3 — The Plan button.** Planner loop, preview and apply. The sample-housing demo planner is removed.

---

## M1 — Sign in with ChatGPT and the OpenAI client

### Eligibility and policy

OpenAI documents plan usage as available to "open-source projects, personal projects that run locally, and selected private apps". AXIS CAM Studio is a personal, locally run, MIT-licensed project. The app must follow OpenAI's UI/UX guidelines for plan usage (below). OpenAI's terms and brand guidelines apply to the sign-in button asset.

### OAuth flow (authorization code + PKCE, loopback redirect)

Endpoints (from `https://auth.openai.com/.well-known/openid-configuration`):

| Purpose | URL |
| --- | --- |
| Authorize | `https://auth.openai.com/api/accounts/authorize` |
| Token | `https://auth.openai.com/api/accounts/oauth/token` |
| Revoke | `https://auth.openai.com/api/accounts/oauth/revoke` |
| JWKS | `https://auth.openai.com/.well-known/jwks.json` |
| API base | `https://api.openai.com/v1` |

Sign-in steps:

1. Bind a listener on `127.0.0.1` with an ephemeral port. The redirect URI is `http://127.0.0.1:<port>/callback`.
2. Generate fresh `state`, `nonce` and a PKCE verifier (32 random bytes each, base64url). The challenge is base64url(SHA-256(verifier)) without padding.
3. Open the system browser at the authorize URL with these parameters:
   - `client_id`: `dynamic_agent_client` on first registration, otherwise the saved issued client ID
   - `agent_name_hint`: `AXIS CAM Studio` (first registration only)
   - `ext_agent_host_id`: the installation's stable host ID
   - `id_token_hint`: the retained ID token, when re-authorizing. Supported by the URL builder; M1 never has one to send, because sign-out deletes the tokens and re-authorizing while signed in is not offered.
   - `response_type=code`, `redirect_uri`, `state`, `nonce`, `code_challenge`, `code_challenge_method=S256`
   - `scope`: `openid profile email offline_access resource.invoke chatgpt.tokens.use.direct`
   - `resource`: `https://api.openai.com/v1`
4. Wait up to 5 minutes for the callback. A request to any other path, or one whose `state` does not match, gets 404 and is ignored, so a stray local request cannot end the sign-in. The user can cancel. An `error` parameter (for example `access_denied`) with the right `state` ends sign-in with a plain message. The response page says the user can return to the app.
5. Save the `client_id` returned by the callback (first registration).
6. POST the token endpoint (form-encoded): `grant_type=authorization_code`, `client_id`, `code`, `code_verifier`, `redirect_uri`, `resource`. There is no client secret.
7. Validate the ID token: RS256 signature against the JWKS, `iss` = `https://auth.openai.com`, `aud` = the issued client ID, not expired, `nonce` matches.
8. If the granted `scope` lacks `chatgpt.tokens.use.direct`, the session is kept but marked "plan usage not granted", and the planner stays unavailable with that reason.

Refresh: access tokens last 1 hour and refresh tokens 30 days, rotating on each use. The client refreshes when less than 5 minutes remain, or once after a 401. Refreshes are single-flight (one mutex per session). The refresh and its save run to completion even if the request that needed them is cancelled. The new token set is persisted before it is used; if saving fails twice, it is kept in memory for this run. `invalid_grant`, `token_expired`, `refresh_token_expired`, `refresh_token_reused` and `invalid_client` clear the tokens and return the app to signed-out with a message, as does any other OAuth error code on a 400 or 401 that is not a transient one. Connect errors, 5xx, 408, 429 and code-less errors are retried with bounded backoff. Requests to the sign-in endpoints time out after 30 seconds and do not follow redirects.

Sign-out: POST the revoke endpoint with `token=<refresh token>`, `token_type_hint=refresh_token`, `client_id`. Local tokens are cleared whether or not revocation succeeds; if it fails, the status says remote revocation was not confirmed.

### Storage

- **Tokens** (access, refresh, ID token, expiry, scope, name, email, subject): one macOS Keychain item, service `AXIS CAM Studio`, account `chatgpt-session`. On other platforms: a file in the app data folder with owner-only permissions, written atomically. Tokens never go into project files, the workspace file, logs or status text. The session type's `Debug` output redacts them.
- **Non-secret settings**: `settings.json` in the app data folder: host ID (a random UUID created once), issued client ID, chosen model, and whether the first-sign-in notice was shown. Smoke and test runs use their isolated folder.

### API client

- `GET /v1/models` with the bearer token. Show models whose `visibility` is `list`, or all when the field is absent. The default is `gpt-6.1-sol` when listed, otherwise the first listed model.
- `POST /v1/responses` with `model`, `input` (a system message and user messages), `store: false`, `stream: true`, and `text.format = { type: "json_schema", name, schema, strict: true }`.
- Server-sent events: accumulate `response.output_text.delta` (`delta`). The request succeeds only on `response.completed`. `response.failed`, `response.incomplete`, `error`, a refusal, or a stream that ends early are errors. Unknown event types are ignored.
- If the API answers `subscription_sharing_unsupported_capability`, the client retries once without `text.format`, with the schema restated in the instructions. Output is always parsed and validated locally, so both paths give the same guarantees.
- Timeouts: 15 s to connect, 5 minutes per request. A request can be cancelled.

Error handling:

| Error | What the user sees |
| --- | --- |
| `subscription_sharing_usage_limit_exceeded` (429) | "Usage limit reached" with a **Manage usage** link to `https://chatgpt.com/settings/usage` |
| `subscription_sharing_user_not_eligible` (403) | The plan cannot be used in this app; no retry |
| `subscription_sharing_usage_unavailable` (503), other 5xx | Retried with backoff, then "OpenAI is unavailable; try again" |
| `subscription_sharing_invalid_user` (401), repeated 401 | Signed out; asks the user to sign in again |
| `subscription_sharing_route_not_supported`, other 4xx | The error message from OpenAI, verbatim |
| No network | "Could not reach OpenAI" |

### UI (AI planner page, account panel)

Required by OpenAI's guidelines; exact strings are copied from the guidelines page at implementation time:

- Signed out: a text-only **Continue with ChatGPT** button. OpenAI's devkit license grants no rights to its logos or branded assets, so none is bundled, and the app never draws its own imitation.
- A sentence under the button states what is shared (name, email, profile picture) and that planning requests count against the ChatGPT plan.
- First sign-in only: a notice titled **You're using your ChatGPT plan**, dismissed with **Got it**.
- Signed in: the account name and email, the label **Using ChatGPT plan** with a **Manage usage** link, a model picker, **Check connection** and **Sign out**.
- **Check connection** sends one tiny structured-output request and reports success or the mapped error. It is the live proof that plan tokens, streaming and structured output work.
- While waiting for the browser: "Waiting for sign-in in your browser…" with **Cancel**.

### New dependencies

An HTTPS client with rustls (no OpenSSL), a JWT/JWKS validator, base64, a secure random source, and Keychain access. `tokio` gains the features the listener and timers need. `Cargo.lock` changes in this milestone only. Licenses are added to `THIRD_PARTY.md` and `licenses/`.

### Testing (M1)

A local fake of OpenAI's servers (token, JWKS, revoke, models, responses) on a loopback port, injected through an `Endpoints` value that production code never changes. Tests use a test-only RSA key to sign ID tokens.

- Full sign-in with a simulated browser callback; issued client ID saved and reused.
- Refusals: wrong `state`, `error=access_denied`, bad signature, wrong `aud`, wrong `nonce`, expired token, timeout, cancel.
- Scope without `chatgpt.tokens.use.direct` marks plan usage as not granted.
- Refresh before expiry, refresh after 401, single-flight under concurrent requests, rotation persisted, each fatal refresh error clears tokens.
- Sign-out revokes and clears; failed revocation still clears and reports it.
- Streaming: deltas accumulate, completion required, each failure event, early end, unsupported-capability fallback, every error-table row.
- No token appears in `Debug` output, status text or the settings file.
- Session store round-trip against the file store; the Keychain store is exercised by one ignored-by-default test.

Live check by the user: sign in, see the account and models, run **Check connection**, sign out.

---

## M2 — Part analysis and plan checker

### Preconditions for planning

Shown as a checklist on the planner page; **Plan** is disabled until all pass.

1. A part is imported (not the built-in example) and loaded.
2. Setup 1 exists, with a 3-axis LinuxCNC machine, valid stock, and rotations that are multiples of 90°.
3. A material is entered.
4. The tool library has at least one tool, and tool numbers are valid and unique.
5. (M3) The user is signed in and plan usage is granted.

### Orientations

The planner works with six orientations, named by the part axis that points up: `+Z`, `-Z`, `+X`, `-X`, `+Y`, `-Y`. Their setup rotations (rx, ry, rz) are (0,0,0), (180,0,0), (0,−90,0), (0,90,0), (90,0,0) and (−90,0,0). The orientation whose up axis matches setup 1 uses setup 1's actual rotation. The first planned setup is always setup 1's orientation, because the stock is defined there.

### Part report (`analysis.rs`)

A deterministic summary of the part, built from the loaded mesh. All lengths are mm, rounded to 0.01. It contains:

- Part size, material, stock size and the stock margin on each side in setup 1.
- Machine limits: maximum rpm, maximum feed, travel.
- Tools: number, name, type, diameter, flute length, stickout.
- For each orientation:
  - Model height, and stock above the model top.
  - **Planar faces visible from above**: surface IDs, height below the model top, area, bounding rectangle, and whether it is the highest plane. A face is visible when nothing in the mesh is above its triangles' centroids. Coplanar faces at one height are listed as one plane with several IDs.
  - **Pocket floors**: visible planes below the top and above the model bottom, with depth and the largest flat cutter diameter that fits (found by shrinking the floor outline until it vanishes).
  - **Holes** in each visible plane, from the existing hole detector: centre, diameter, opening diameter, depth, and whether it goes through.
  - **Freeform area**: the visible area that is neither horizontal nor vertical, which only surface finishing can follow.
- Limits to keep the report bounded: at most 40 planes and 100 holes per orientation, faces under 1 mm² dropped, with a `truncated` flag when anything is cut.

### Proposal (`proposal.rs`) — the model's output

A strict JSON schema (every field required, no extra properties, optional numbers nullable):

```
summary: string
setups: [ {
  orientation: "+Z" | "-Z" | "+X" | "-X" | "+Y" | "-Y"
  name: string
  origin: "Stock top center" | "Stock top front-left" | "Model origin"
  workholding: string
  operations: [ {
    strategy: "Face" | "Pocket" | "Contour" | "Drill" | "Parallel" | "Waterline"
    name: string
    tool: integer                      (library tool number)
    surfaces: [integer]                (surface IDs from this orientation's report)
    rpm, feed: number
    stepdown, stepover_percent, radial_allowance, axial_allowance, bottom_offset: number | null
    finish_passes: integer | null
    facing_area: "stock" | "model" | null
    contour_mode: "outside" | "inside_floor" | null
    pocket_entry: "Profile ramp" | "Plunge" | null,  ramp_angle: number | null
    surface_stepover, tolerance: number | null
    drill_cycle: "Drilling" | "Chip breaking" | "Deep drilling" | "Counterboring" | null
    peck, breakthrough, hole_diameter: number | null,  through: boolean | null
    coolant: "Flood" | "Mist" | "Air" | "Off"
    reason: string
  } ]
} ]
unmachined: [ { feature: string, reason: string } ]
warnings: [string]
```

Building a job from a proposal:

- Setup 1 is a copy of the user's setup 1 (machine, stock, origin, fixture, offset), with its operations replaced. The proposal's `origin` applies to later setups only.
- Later setups get the same machine snapshot, the next free work offset (at most six setups), the orientation's rotation, the proposed name and origin, the workholding text as notes, and inherited stock.
- Each operation starts from the app's defaults for its strategy. Non-null proposal values replace defaults. Combinations the generators require are fixed by the app, not the model: Pocket uses the selected floor with the tool inside; outside Contour, Parallel and Waterline use the model silhouette; compensation is in computer; spindle clockwise; climb milling.
- Names are limited to 60 characters. Free text from the model is only ever displayed or written as sanitized comments.

### Plan check (`plan_check.rs`)

Run in this order; later stages run only if earlier ones pass.

1. **Structure.** Setup 1's orientation is first; orientations are unique; every tool number exists; tool type suits the strategy; every surface ID belongs to that orientation's report and suits the strategy; rpm and feed are positive and within machine limits. Each problem names its setup and operation.
2. **Generation.** `simulation::sequence` on the built job. Every enabled operation must generate, with inherited stock.
3. **Export checks.** `post::post_job` must be ready: limits, travel, rapids into material, holder clearance, work zeros, round-trip replay.
4. **Gouge check (coarse).** The part is voxelized on the stock grid (vertical ray parity per column). A part cell more than one cell below the part surface that the stock simulation has removed is a gouge. Three or more such cells fail the check, attributed to the operation after which they first appear, with their bounding box. This detects cuts deeper than about one grid cell (the grid has up to 96 cells on the stock's longest side); finer accuracy relies on the generators' own guarantees. The preview says "no gouging detected (coarse check)".
5. **Coverage (report, not pass/fail).**
   - Covered surfaces: for each surface, sample its triangles' centroids, step 1.5 cells outward along the outward normal, and test whether stock remains there. Surfaces more than half covered are listed with ID, area and covered share.
   - Exposed share: uncovered area divided by total area.
   - Leftover stock volume outside the part, split into: above the part, around the silhouette, inside the part's bounding box, below the part.
   - All coverage numbers are approximate and labelled so.
6. **Totals.** Estimated machining time per operation and in total.

The result is a `CheckReport`: pass or fail, problems grouped by setup and operation, the coverage numbers and the totals. It is shown to the user and, in M3, sent to the model.

### Command line (M2)

`axis-cam --check-plan <project.json> <proposal.json>` loads the project and its model, builds the job from the proposal, runs the check and prints the report. Exit code 0 means pass.

### Testing (M2)

- Report on the example block: the top plane, the pocket floor with its depth and fitting diameter, the holes with diameters and depths, all six orientations, truncation.
- Visibility: a face under an overhang is not listed; a flipped orientation lists the underside.
- Proposal parsing: the schema accepts a valid proposal and rejects missing fields, unknown strategies and extra properties.
- Building: defaults, overrides, fixed combinations, setup 1 preserved, later setups created with offsets G55 onward.
- A hand-written good plan for the example block passes all stages (face, pocket, contour, drill; flipped facing).
- Each structural error; a generation failure; an export refusal (for example a stock-based work zero on a cut face); each reported with its setup and operation.
- Gouge: a synthetic cut into the part is detected and attributed; a clean job reports none.
- Coverage: before machining the top is covered; after facing it is exposed; leftover categories sum to the total.
- The command exits 0 for the good plan and non-zero with the report for a bad one.

---

## M3 — The Plan button

### Planner loop (`planner.rs`)

A `Backend` trait with one method (messages and schema in, text out) has two implementations: the OpenAI client, and a scripted one for tests.

1. Build the part report.
2. Request 1: instructions plus the report. The instructions cover:
   - the role and the output schema
   - the six strategies with the exact rules their generators enforce (for example: Facing only on the highest plane with a flat end mill or face mill; Pocket on a recessed flat floor with a flat end mill that fits, stepover at most 50%, depth within stickout; Drill with a drill no wider than the hole; depths within tool reach)
   - planning rules: library tools only; surfaces only from the chosen orientation's list; face, then pockets, then contour, then holes, then finishing; add a setup only for features that cannot be reached otherwise; never invent IDs; list what cannot be machined with the reason
   - feeds and speeds within machine limits and conservative for the material
3. Parse and validate the proposal locally. A parse failure counts as a failed attempt and is reported back.
4. Build the job and run the plan check in a background thread.
5. If the check fails, or it passes but covered surfaces are neither machined nor listed in `unmachined`, send the check report and ask for the full corrected proposal.
6. At most **4 model requests per click**. The result is the passing proposal with the highest exposed share. If none passes, the planner reports failure with the last problems and offers to try again. Nothing can be applied in that case.

Progress is shown live: analyzing, asking the model (n of 4), checking, repairing. The user can cancel at any time.

### What is sent to OpenAI

The part report (dimensions and feature list), the tool library entries listed above, machine limits, material and stock size, and on repair rounds the check report. The CAD file, file paths, project title and account data are not sent. Requests set `store: false`. The page states this next to the Plan button.

### Preview and apply (AI planner page)

- Left: the account panel, the preconditions checklist, **Plan**, progress and **Cancel**.
- Right, after a run:
  - the summary, and a card per setup with its operations (tool, key parameters, time)
  - check results: all operations generate; export checks pass; no gouging detected (coarse check)
  - coverage: exposed share, covered surfaces, leftover stock by region
  - what could not be machined, with reasons; warnings; total time
  - a 3D view of the stock after the whole plan, which the user can orbit
  - **Apply plan** and **Discard**
- The preview states how many existing operations and setups Apply will replace.
- **Apply** replaces the job's setups with the planned ones, keeps the previous job for **Undo**, and opens the machining page. The job is then an ordinary editable job; export is on the NC program page.
- Model text is displayed as plain text only.

### Removed

The sample-housing demo planner: its coverage, priority and brief controls, `Document::sample_plan`, and their tests and smoke steps.

### Testing (M3)

With the scripted backend:

- A good proposal on the first request gives a passing preview; Apply replaces the job; Undo restores it exactly.
- A bad proposal followed by a good one exercises repair; the repair message contains the check report.
- Four bad proposals give a failure with the last problems and nothing to apply.
- Unparseable output counts as an attempt.
- A passing plan with uncovered, unlisted surfaces triggers one more round; the best passing proposal is kept.
- Cancel stops the run and leaves the job unchanged.
- The request contains the report and no tokens, paths, title or account data.
- Each mapped API error appears in the UI state.
- Renderer smoke screens: signed out, signed in with preconditions unmet, running, preview, failure.

Live check by the user: sign in, click Plan on the four-setup block, review the preview, apply, play the simulation and export.

## Documentation

README and VALIDATION gain sections for sign-in, what is sent, the planner's checks and their limits. Help text covers every new control. No text claims fixture checking, cutting-load validation, or exact gouge detection.
