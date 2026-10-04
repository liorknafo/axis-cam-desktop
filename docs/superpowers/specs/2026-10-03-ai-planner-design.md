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

1. Bind a listener on `127.0.0.1` with an ephemeral port. The redirect URI is `http://127.0.0.1:<port>/auth/callback`, the path OpenAI's reference kit uses.
2. Generate fresh `state`, `nonce` and a PKCE verifier (32 random bytes each, base64url). The challenge is base64url(SHA-256(verifier)) without padding.
3. Open the system browser at the authorize URL with these parameters:
   - `client_id`: `dynamic_agent_client` on first registration, otherwise the saved issued client ID
   - `agent_name_hint`: `AXIS CAM Studio` (first registration only)
   - `ext_agent_host_id`: the installation's stable host ID, in the form `urn:uuid:<lowercase version-4 UUID>`. OpenAI rejects other forms (found in the first live test)
   - `id_token_hint`: the retained ID token, when re-authorizing. Supported by the URL builder; M1 never has one to send, because sign-out deletes the tokens and re-authorizing while signed in is not offered.
   - `response_type=code`, `redirect_uri`, `state`, `nonce`, `code_challenge`, `code_challenge_method=S256`
   - `scope`: `openid profile email offline_access resource.invoke chatgpt.tokens.use.direct`
   - `resource`: `https://api.openai.com/v1`
4. Wait up to 5 minutes for the callback. A request to any other path, or one whose `state` does not match, gets 404 and is ignored, so a stray local request cannot end the sign-in. The user can cancel. An `error` parameter (for example `access_denied`) with the right `state` ends sign-in with a plain message. The response page says the user can return to the app.
5. Save the `client_id` returned by the callback (first registration). It must be 1–200 characters of letters, digits, `_` or `-`, and not `dynamic_agent_client`.
6. POST the token endpoint (form-encoded): `grant_type=authorization_code`, `client_id`, `code`, `code_verifier`, `redirect_uri`, `resource`. There is no client secret.
7. Validate the ID token: RS256 signature against the JWKS, `iss` = `https://auth.openai.com`, `aud` = the issued client ID, not expired, `nonce` matches, and `azp` equals the client ID when present.
8. If the granted `scope` lacks `chatgpt.tokens.use.direct`, the session is kept but marked "plan usage not granted", and the planner stays unavailable with that reason.

Refresh: access tokens last 1 hour and refresh tokens 30 days, rotating on each use. The client refreshes when less than 5 minutes remain, or once after a 401. Refreshes are single-flight (one mutex per session). The refresh and its save run to completion even if the request that needed them is cancelled. The new token set is persisted before it is used; if saving fails twice, it is kept in memory for this run. `invalid_grant`, `token_expired`, `refresh_token_expired`, `refresh_token_reused` and `invalid_client` clear the tokens and return the app to signed-out with a message, as does any other OAuth error code on a 400 or 401 that is not a transient one. Connect errors, 5xx, 408, 429 and code-less errors are retried with bounded backoff. Requests to the sign-in endpoints time out after 30 seconds and do not follow redirects.

Sign-out: POST the revoke endpoint with `token=<refresh token>`, `token_type_hint=refresh_token`, `client_id`. Local tokens are cleared whether or not revocation succeeds; if it fails, the status says remote revocation was not confirmed.

### Storage

- **Tokens** (access, refresh, ID token, expiry, scope, name, email, subject): one macOS Keychain item, service `AXIS CAM Studio`, account `chatgpt-session`. On other platforms: a file in the app data folder with owner-only permissions, written atomically. Tokens never go into project files, the workspace file, logs or status text. The session type's `Debug` output redacts them.
- The saved sign-in is read the first time the planner page opens, because macOS may ask for Keychain permission. Until that read finishes, signing in is disabled, and a result that arrives when no read is pending is ignored. If a refresh is rejected but the store holds a newer sign-in (another running copy of the app refreshed it), the newer one is used instead of being deleted.
- **Non-secret settings**: `settings.json` in the app data folder: host ID (`urn:uuid:` plus a random UUID, created once; a bare UUID saved by an earlier build is prefixed), issued client ID, chosen model, and whether the first-sign-in notice was shown. Smoke and test runs use their isolated folder.

### API client

- `GET /v1/models` with the bearer token. Show models whose `visibility` is `list`, or all when the field is absent. The default is `gpt-6.1-sol` when listed, otherwise the first listed model.
- `POST /v1/responses` with `model`, `input` (a system message and user messages), `store: false`, `stream: true`, and `text.format = { type: "json_schema", name, schema, strict: true }`.
- Server-sent events: accumulate `response.output_text.delta` (`delta`). The request succeeds only on `response.completed`. `response.failed`, `response.incomplete`, `error`, a refusal, or a stream that ends early are errors. Unknown event types are ignored.
- If the API answers `subscription_sharing_unsupported_capability`, the client retries once without `text.format`, with the schema restated in the instructions. Output is always parsed and validated locally, so both paths give the same guarantees.
- Timeouts: 15 s to connect, 2 minutes without any data, 15 minutes per request (planning replies can take minutes). A reply that completed before the connection dropped is kept. A request can be cancelled.

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
- Limits to keep the report bounded: at most 40 planes and 100 holes per orientation, faces under 1 mm² dropped, with a `truncated` flag when anything is cut. A plane's holes are listed completely or not at all: when a plane has more hole candidates than remain under the limit, its holes are not searched or listed and `truncated` is set, so the work is bounded too.
- The part must be a closed solid. The report is refused, before any model request, when the mesh has an open edge, when it encloses no volume, when more than 2% of a grid of vertical test rays cross the surface inconsistently (a ray counts only when it stays inconsistent at four points 0.002 mm around it), or when no ray hits the part, because the plan checks cannot tell inside from outside on such a mesh. Edge test, in the part's own coordinates in mm (where setup 1 puts the part does not matter), with a tolerance that absorbs float rounding and text written with 7 significant digits: 8 f32 steps at the part's largest absolute coordinate, in its file or around its centre, or, if that is more, √3 × 10^(⌊log10 reach⌋ − 6) mm, where the reach is that coordinate and at least 1 mm. 6 significant digits are not covered (a T-junction mesh written that way is refused). Vertices within the tolerance of each other are grouped, chains included, and a group is welded only when its points fit in a box whose diagonal is at most twice the tolerance; the points of a wider group stay distinct, so a row of nearby points cannot weld an opening shut; zero-area triangles are ignored and a triangle written more than once with the same winding counts once (with opposite windings both count, and their edges cancel); an edge used by an odd number of triangles is open; each open edge is split at the ends of other open edges that lie within the tolerance of it and farther than that from both its own ends (a T-junction), and the pieces then used an odd number of times are the open edges left. The refusal names the number of open edges. Coordinates more than 1,000,000 mm from the file origin refuse the part with their own message: "The part lies more than 1 000 000 mm from its file origin. Move it closer to the origin and re-export it." A part whose enclosed volume is under 1e-9 of its bounding box's (a sheet written with both windings) is refused as enclosing no volume. A test ray's crossings within 0.0001 mm of one height are taken facing down first, so coincident faces facing both ways read the same whatever the triangle order. A ray whose solid spans are all of zero length (a crossing facing down and one facing up at one height, with nothing solid on either side, as through a double-sided sheet whose sides are triangulated differently) is inconsistent; a zero-length span with solid elsewhere in its column (a void whose top lies on the part's top face) is not. Where a column stays inconsistent (flipped triangles, overlapping bodies, a zero-length span alone), the plan checks treat it as solid from its lowest crossing if that faces down, otherwise from the model's bottom, up to its highest crossing if that faces up, otherwise up to the model top; crossings at one height facing both ways count as facing the way that makes more of the column solid. Known limit: part analysis can take tens of seconds on meshes with tens of thousands of long sliver triangles in one plane (synthetic fan or T-junction slivers); realistic meshes up to 200,000 triangles take a few seconds.

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

- Setup 1 is a copy of the user's setup 1 (name, machine, stock, origin, fixture, offset), with its operations replaced. The proposal's `origin` and `name` apply to later setups only.
- Later setups get the same machine snapshot, the next free work offset (at most six setups), the orientation's rotation, the proposed name and origin, the workholding text as notes, and inherited stock.
- Each operation starts from the app's defaults for its strategy. Non-null proposal values replace defaults, with one exception: a null `radial_allowance`, `axial_allowance` or `bottom_offset` means 0, so one operation machines to size unless the proposal leaves stock. A nullable field that is missing is read as null. Combinations the generators require are fixed by the app, not the model: Pocket uses the selected floor with the tool inside; outside Contour, Parallel and Waterline use the model silhouette; compensation is in computer; spindle clockwise; climb milling.
- Names are limited to 60 characters and notes to 500. Control and format characters (including bidirectional overrides) are removed. Free text from the model is only ever displayed or written as sanitized comments.
- Limits on a proposal: at most 12 operations per setup and 40 in total; at most 500 surface IDs per operation, deduplicated; rpm from 100 to the machine maximum; feed from 1 mm/min to the machine maximum; every number finite; `bottom_offset`, `radial_allowance` and `axial_allowance` 0 or more, so a plan never cuts below the model's bottom except by a through drill's breakthrough.
- Drilling: `through` may be true only in setup 1, and only when every hole the operation selects (the plane's holes, narrowed by `hole_diameter`) is a through hole in the report. In later setups the part rests on the face where the drill would exit, and no tool may go below the part's underside, so a hole that goes through along X or Y is drilled with `through` false and its exit listed in `unmachined`. `breakthrough` may be at most half the drill diameter plus 1 mm.

### Plan check (`plan_check.rs`)

Run in this order. A structure problem stops everything. A generation failure stops the export stage. The tool axis check and the coarse gouge check run for every operation that generated, whatever the export stage says, so one round of feedback carries every known problem.

1. **Structure.** Setup 1's orientation is first; orientations are unique; every tool number exists; tool type suits the strategy; every surface ID belongs to that orientation's report and suits the strategy; rpm, feed and the limits above hold. The built job must pass the same structure validation as a saved project, so an applied plan can always be saved and reopened. Each problem names its setup and operation.
2. **Generation.** `simulation::sequence` on the built job. Every enabled operation must generate, with inherited stock.
3. **Export checks.** `post::post_job` must be ready: limits, travel, rapids into material, holder clearance, work zeros, round-trip replay.
4. **Tool axis check (exact).** For every move of every operation, in its setup's frame, the tool's centre line from the tip upward must not pass through the part: positions are sampled every 1 mm along the move and tested against the mesh with a vertical ray. More than 0.1 mm of part material above the tip fails the check for every strategy, as does any move below the bottom of the stock box in setup 1, or below the part's underside in later setups (earlier setups may have removed the stock there). This catches a tool of any size that goes deeper than the surface under its centre, such as a drill sent past a blind hole's bottom. It does not see a cutter's edge cutting a wall beside its centre; the generators' own rules and the coarse check cover that.
5. **Gouge check (coarse).** The part is voxelized on the stock grid (vertical ray parity per column). A part cell more than one cell below the part surface that the stock simulation has removed is a gouge. The check fails once three or more such cells exist in total, attributed to each operation that adds to them, with their bounding box. This detects cuts deeper than about one grid cell (the grid has up to 96 cells on the stock's longest side) by cutters wide enough to cover a cell centre; walls and floors thinner than about three cells have no such cells. The preview says "no gouging detected (coarse check)".
6. **Coverage (report, not pass/fail).**
   - Covered surfaces: for each surface, sample its triangles' centroids, step 1.5 cells outward along the outward normal, and test whether stock remains there. Report planes more than half covered are listed with IDs, area and covered share. The remaining surface (walls and freeform, not the report's planes) is summed by the direction it faces and listed when more than 10% of it is covered.
   - Exposed share: uncovered area divided by total area.
   - Leftover stock volume outside the part, split into: above the part, around the silhouette, inside the part's bounding box, below the part.
   - All coverage numbers are approximate and labelled so.
7. **Totals.** Estimated machining time per operation and in total.

The result is a `CheckReport`: pass or fail, problems grouped by setup and operation, the coverage numbers and the totals. It is shown to the user and, in M3, sent to the model. The built job is returned only when the plan passes.

### Command line (M2)

`axis-cam --check-plan <project.json> <proposal.json>` loads the project and its model, builds the job from the proposal, runs the check and prints the report as JSON on standard output (the one-line summary goes to standard error). Exit code 0 means the plan passes, 1 that it does not, 2 that the check could not run (usage, unreadable files, unmet preconditions). `--part-report <project.json>` and `--proposal-schema` print JSON and use exit code 2 for errors.

### Testing (M2)

- Report on the example block: the top plane, the pocket floor with its depth and fitting diameter, the holes with diameters and depths, all six orientations, truncation.
- Visibility: a face under an overhang is not listed; a flipped orientation lists the underside.
- Proposal parsing: the schema accepts a valid proposal and rejects missing fields, unknown strategies and extra properties.
- Building: defaults, overrides, fixed combinations, setup 1 preserved, later setups created with offsets G55 onward.
- A hand-written good plan for the example block passes all stages (face, pocket, contour, drill; flipped facing).
- Each structural error; a generation failure; an export refusal (for example a stock-based work zero on a cut face); each reported with its setup and operation.
- Gouge: a synthetic cut into the part is detected and attributed; a clean job reports none. Tool axis: a drill sent 25 mm past a blind hole's bottom with a 1 mm drill, and a drill sent below the stock, are each refused end to end; the good plan and a plan with Parallel and Waterline finishing report nothing.
- A passing plan's job survives saving and reopening. Duplicate surface IDs, too many operations, a spindle speed under 100 rpm, `through` on a blind hole and an oversized breakthrough are each refused. An open mesh is refused by the report.
- Coverage: before machining the top is covered; after facing it is exposed; leftover categories sum to the total.
- The command exits 0 for the good plan and non-zero with the report for a bad one.

---

## M3 — The Plan button

### Planner loop (`planner.rs`)

A `Backend` trait with one method (messages and schema in, text out) has two implementations: the OpenAI client, and a scripted one for tests.

1. Build the part report, on a background thread.
2. Request 1: instructions plus the report. The instructions cover:
   - the role and the output schema
   - the six strategies with the exact rules their generators enforce (for example: Facing only on the highest plane with a flat end mill or face mill; Pocket on a recessed flat floor with a flat end mill that fits, stepover at most 50%, depth within stickout; Drill with a drill no wider than the hole; depths within tool reach)
   - planning rules: library tools only; surfaces only from the chosen orientation's list; face, then pockets, then contour, then holes, then finishing; add a setup only for features that cannot be reached otherwise; never invent IDs; list what cannot be machined with the reason
   - feeds and speeds within machine limits and conservative for the material
   - every limit the builder and the generators enforce, stated where the model can act on it: each orientation once and at most 6 setups; at most 12 operations per setup and 40 in total; finite, non-negative numbers; `finish_passes` 0-20; `ramp_angle` null or 0.1-15; Face only with stock above the face beyond `bottom_offset`; Contour's stepdown within the flute length; inside-floor Contour under Pocket's tool and depth rules; Drill depth measured from the stock top; `hole_diameter` matching a hole's `diameter`; `through` only in setup 1; `breakthrough` (null means 1 mm, at most half the drill diameter plus 1 mm and the stock below the model); Parallel's 30,000-sample limit with its formula and an example; Waterline's depth and 300 layers; every operation's 30,000-move limit, how Waterline (layers × outline points), Pocket (levels × offset-ring points) and outside Contour (levels × outline points) moves grow, and that on a part of about 120 × 80 mm, with Face first, Pocket, Contour and Waterline need a stepdown of about 0.5 mm or more, a guide that assumes `stepover_percent` 40, 1 finish pass and a `ramp_angle` of 1° or more, with curved walls and round islands adding hundreds of moves per level; in the Pocket and inside-floor Contour lines, that each finish pass repeats the outermost ring at every level and a smaller `ramp_angle` adds ramp moves on every ring and level, that a smaller stepover adds Pocket rings, and that an inside-floor Contour's stepover is fixed at 40 %; Face's 2,000 strokes (levels × passes) with the formula and an example; `peck` at least the drilling depth from the stock top ÷ 1,000; at least one operation in every setup; that only a through drill's breakthrough in setup 1 goes below the model; `facing_area`; the report's `truncated`; the defaults of null fields; and that the check report names `covered_other` directions in setup 1's frame. A test checks that the instructions mention each of these, so a change to a limit is noticed, and each numeric example is backed by a behavioural test on the example block, generating the operation with the stock the ones before it leave as the check does: Parallel 0.6 mm passes and 0.5 fails; Pocket 0.5 and 0.45, outside Contour 0.35 and 0.3, Waterline 0.45 and 0.4 (all pass at 0.5); Pocket, and an inside-floor Contour after it, at 0.5 mm pass with 1 finish pass and with a 1° ramp and fail with 2 finish passes or a 0.3° ramp; Pocket at 0.5 mm fails at stepover 30 %, while an inside-floor Contour at stepover 10 % passes (its stepover stays 40 %); Face at 5% stepover 0.4 and 0.38; Face at 12.5% stepover 0.165 passes and 0.155 fails, a pair only the formula's "+1" separates; `facing_area` `model` at 5% passes at 0.38 (where the stock's width fails) and fails at 0.36; Waterline on a 20 mm high box generates 300 layers (well under the move limit) and refuses 301; peck 2% above and below the depth ÷ 1,000; an empty later setup is refused.
3. Parse and validate the proposal locally. A parse failure counts as a failed attempt and is reported back.
4. Build the job and run the plan check in a background thread.
5. If the check fails, send the check report and ask for the full corrected proposal. If it passes but report planes or other surface are still covered by stock, ask once per run for those to be machined or listed in `unmachined`.
6. At most **4 model requests per click**. The result is the passing proposal with the highest exposed share. Once a proposal has passed, a later request that fails, cannot be parsed or does not pass ends the run with the plan already in hand. If none passes, the planner reports failure with the last problems and offers to try again. A run that ends early, on an API error or a check that fails to run, after a proposal failed the checks keeps that proposal's problems (bounded) with the failure. Nothing can be applied in that case.

Progress is shown live, one stage at a time: Analyzing (the part report), Asking(n) (the first request), Repairing(n) (a request after a failed check or an unreadable reply), Extending(n) (the one request about faces still covered), Checking(n) (the plan check), Previewing (the 3D view after the check passed); n counts requests out of 4, and the stage names the model the run was started with (stored when Plan is clicked, so picking another model during the run does not change it). The user can cancel at any time.

- The outcome records how many requests were made (shown in the preview) and which request's plan won.
- A follow-up request's API error is kept with the plan: the preview says "A follow-up request failed: …", and a sign-out or usage limit reaches the account panel (only while the client the run started with is current).
- A check that fails to run after a plan already passed ends the run with that plan.
- Before parsing, a surrounding Markdown code fence and any text outside the outermost braces are removed. Once the plan route has refused structured output, later requests of the run ask for plain JSON at once.

### What is sent to OpenAI

The part report (dimensions and feature list), the tool library entries listed above, machine limits, material and stock size, and on repair rounds the check report. The CAD file, file paths, project title, setup names and account data are not sent. Every check runs on an anonymized copy of the job: the title is empty and every setup of the user's job is named "Setup {n}", so no problem message (generation, export round trip) can carry them. A passing job gets the user's setup-1 name back before it is previewed and applied. So that the anonymized check and the applied job cannot differ in validity, the planner's checklist shows setup 1's own issues (`Setup::issues`: its name, work offset, machine, stock and position fields) as an unmet item, and Plan is refused until they are fixed. Export messages name a setup by its number, never by the program's file name. Requests set `store: false`. The page states this next to the Plan button: "Sent to OpenAI: the part's dimensions and feature list, your tools, machine limits, material and stock size, and on correction rounds the app's check report. The CAD file, file paths, the project title and your setup names are not sent."

### Preview and apply (AI planner page)

- Left: the account panel, the preconditions checklist and what is sent to OpenAI.
- Right, before a run: the steps the planner takes and **Plan this job**, shown when the checklist is met. During a run: the current stage and **Cancel**.
- Right, after a run:
  - the totals (operations, setups, machining time, model requests made) and the check results: all operations generate; export checks pass; no tool centre line cuts into the part and nothing goes below the stock; no gouging detected on the stock grid (coarse check)
  - **Apply plan** and **Discard**, directly after the check lines (and again at the bottom)
  - "The planner's summary"
  - a card per setup, titled "Setup 1: name" for the user's setup 1 and "Setup n · named by the planner: “name”" for later setups, with the planner's workholding note, labelled "Workholding (from the planner; on Apply it goes at the end of setup 1's notes, replacing an earlier planner note if it is unchanged)" on setup 1 and "Workholding (from the planner)" on later setups (where it becomes the setup's notes), and per operation three lines: the strategy in the app's words, the planner's operation name in quotes, the tool and the time; the values that will run, read from the built operation's parameters (rpm as posted, feed, and per strategy: stepdown, stepover %, allowances and bottom offset when not zero, entry and ramp angle for Pocket and for an inside-floor Contour (which also shows "stepover 40% (fixed)", the floor profile's stepover the app sets), finish passes, drill cycle and peck, through and breakthrough, hole diameter, surface stepover and tolerance for Parallel); "The planner's reason"
  - coverage: exposed share, covered surfaces, leftover stock by region
  - "Not machined (as listed by the planner)", with reasons; "Warnings from the planner"; each list says how many entries were left out
  - a 3D view of the stock after the whole plan, with its own camera: it starts framed on the part for each preview, and orbit, pan and zoom in it never change the machining viewport's camera
  - **Apply plan** and **Discard**
- The preview states that Apply replaces setup 1's operations and removes the later setups (with their counts), and that the previous job is kept in History and can be undone on this page.
- **Apply** first saves the current job to History with the app's archive (as New project does; if that fails, nothing is applied). Applying is not a project switch, so **Recover previous project** keeps reopening the project from before the last switch. Apply then replaces the job's setups and active setup with the planned ones, appends the planner's setup-1 workholding note to setup 1's own notes on the same line (as "… · Planner: …", or "Planner: …" alone when setup 1 has no notes; the planner's line breaks become spaces and "Planner: …" is at most 500 characters; the user's notes are never cut). Setup 1 keeps the note Apply added in its `planner_note` parameter, saved with the project. A later Apply first removes that exact text where it still stands whole, also when the new plan has no note for setup 1, so notes do not pile up: the last " · Planner: <that note>" followed by the end of the notes or by white space, or else the last "Planner: <that note>" that is at the start of the notes or after white space and has the same ending, together with the white space after it. When the note does not end in . ! ? or 。, only the end of the notes or a line break may follow it, because words after it on the same line may go on with its sentence ("Planner: Vise grips here" stays). Otherwise it appends, so text the user typed before or after the planner's note, notes the user starts with "Planner: " and a planner note the user changed or went on with mid-word ("Vises", "sides, 40 Nm") all stay. Apply then opens the machining page. The job is then an ordinary editable job; export is on the NC program page.
- **Undo** first keeps a History copy of the applied job, with any edits since, with the app's archive (if that fails, nothing is undone and the status says why); its status says the applied job is in History. It then restores only the setups and active setup from before Apply, and re-adds any tool their operations use that was deleted since; first every re-added tool whose number no tool in the library has keeps it (the first, if two share one), then each re-added tool whose number is taken gets the lowest number neither used nor kept, and the status names each re-added tool with its number ("Ball 6 is back in the library as T4."). Title, material, tools and machines edited after Apply stay. Importing a part, opening a project, creating a new project and restoring the model clear the Undo (the job stays in History).
- Model text is displayed as plain text only, with control and format characters removed, and labelled as the planner's. After parsing, and before the check builds the job, setup and operation names keep only an allow-list. Fullwidth forms U+FF01–FF5E are first read as their ASCII counterparts, except the fullwidth quotation marks U+FF02 and U+FF07; dashes and minus signs (U+2010–2015, U+2212, U+FE58, U+FE63, U+FF0D) become "-", the wave dash U+301C (and U+FF5E through its ASCII form) becomes "~", U+066B, U+066C and U+066A become ".", "," and "%", U+2044 and U+2215 become "/", U+2205 becomes "⌀" and "×" becomes "x" ("Holes 1-4", "Z-0.5", "1~4", "۳.۵", "1/4", "粗加工(左)"). A "*" becomes "x" only between two digits, white space aside ("10x10", "10 x 10"); any other "*" is removed like other symbols ("**Roughing** pass" becomes "Roughing pass"). The format characters U+00AD, U+200B–200F, U+2060–2064 and U+FEFF are dropped without a space before anything else reads the name, so a Persian word written with a zero-width non-joiner stays one word, and U+00B7 inside Catalan l·l (between two letters l or L) is dropped without a space ("paral·lel" becomes "parallel"); elsewhere it becomes a space ("드릴·탭" becomes "드릴 탭"). Names then keep letters and digits of any script (letters drawn as quotes or primes excluded: the modifier letters such as ʺ ˮ ʼ, the Canadian syllabics finals U+141F, U+1420 and U+1425, U+0149 and the Duployan affixes U+1BC80–1BC93), combining marks on a kept letter, and - _ . , / ( ) # + : & % ~ < > = ± ° ⌀ ≤ ≥; an apostrophe (' or ’) stays between a letter or digit and a letter, judged on the base character before any combining marks ("Bob's", a decomposed "José's", "Op 2's"). Quotation and check marks and the listed look-alikes are removed from names; other symbols become spaces, so words and numbers never run together (each removed character, white space included, leaves a space, and runs of spaces become one), and no name can close the app's quotes around it or pass for a check line. The proposal's text is bounded once: summary 600 characters, at most 12 warnings of 240, at most 20 unmachined entries (feature 80, reason 240), operation names 60, reasons 160, workholding 500. Every text an API error carries (OpenAI's own error messages and a refusal) is shown as one cleaned line of at most 300 characters, a failed run's problem messages to 300 (twelve shown, then a count), and the client refuses a reply larger than 4 MB.
- A preview and a running plan describe the job as it was when Plan was clicked. Any edit to the job, opening another project, or (for a running plan) signing out cancels the run and discards the preview. An edit that drops a running plan sets the status "Planning stopped because the job changed."; one that discards a preview "The plan preview was discarded because the job changed." (only when a plan was actually running or previewed). Signing out, or a sign-in found gone, while a plan runs sets "Planning stopped because you are signed out." When the handler of that message also set a status of its own (Undo, an import, saving a tool), both are shown: its own status, a space, then the drop sentence.
- **Cancel** aborts the task and its requests at once. The blocking stages (part report, plan check, 3D view) cannot be interrupted and finish in the background. A guard counts each running blocking stage, also those of cancelled runs, and decrements when the work itself ends; while any is still running, **Plan** is refused with "The previous plan check is still finishing; try again in a moment."
- A successful, non-empty model list clears the note about the model used without a list, whatever came before it in the note. A plan that finishes with the client it started with clears the usage-limit banner and an account note that starts with the usage-limit message, including one that carries the fallback-model sentence after it (a follow-up request that hit the limit sets both again).
- Every count the planner prints says "1 setup" or "2 setups" (operations, problems, faces and requests likewise).
- The planner needs a model. When OpenAI's model list is empty or cannot be loaded, the saved model is kept, or `gpt-6.1-sol` is used when none is saved, and the account panel says so. **Check connection** shows whether that model answers; **Load models** tries the list again.

### Removed

The sample-housing demo planner: its coverage, priority and brief controls, `Document::sample_plan`, and their tests and smoke steps.

### Testing (M3)

With the scripted backend:

- A good proposal on the first request gives a passing preview; Apply replaces the job; Undo restores the previous setups.
- The Plan task's body (`run_plan`) sends every stage and the result with the run's epoch; Cancel and edits abort its task handle and move the epoch on.
- A bad proposal followed by a good one exercises repair; the repair message contains the check report.
- Four bad proposals give a failure with the last problems and nothing to apply.
- Unparseable output counts as an attempt.
- A passing plan with uncovered, unlisted surfaces triggers one more round; the best passing proposal is kept.
- Cancel stops the run and leaves the job unchanged.
- Dropping a run while its part report, a plan check or (in the Plan task) the 3D view is running keeps that stage counted until the stage itself ends, then nothing. The tests hold the stage with a gate that a failing test always releases.
- The request contains the report and no tokens, paths, title or account data.
- Each mapped API error appears in the UI state.
- Renderer smoke screens: signed out, signed in with preconditions unmet, running, preview, failure.

Live check by the user: sign in, click Plan on the four-setup block, review the preview, apply, play the simulation and export.

## Documentation

README and VALIDATION gain sections for sign-in, what is sent, the planner's checks and their limits. Help text covers every new control. No text claims fixture checking, cutting-load validation, or exact gouge detection.
