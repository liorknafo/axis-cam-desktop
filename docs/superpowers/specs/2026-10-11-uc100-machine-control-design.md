# UC100 machine control, part 2: the machine control page — design

Date: 11 October 2026. The scope and the main decisions were made in conversation (listed below); the details are filled in here. The user asked for no review stops, so this spec was not reviewed before the plan.

Linear: project "AXIS CAM — UC100 machine control", part 2 is LIO-31 (LIO-74, LIO-41…47, LIO-75).

## Goal

Everything needed at the machine before running a job, on AXIS CAM's Control page: set machine zero (the JY5800 has no home switches) so the soft limits apply, home a machine that has switches, jog, set work zero and pick G54–G59, run the spindle and coolant, override feed and limits, send MDI lines, and see a model of the CNC4030 move with the live position. Plus the status-line fix (LIO-75).

Part 1 (`docs/superpowers/specs/2026-10-11-uc100-foundation-design.md`) is the base: `src/control.rs` `Connection`, the Control page, the top-bar E-STOP, Esc, the simulated UC100.

## Decisions already made

1. `Connection` keeps a copy of the loaded `MachineConfig` (the driver's snapshot has no config).
2. Machine zero (LIO-74) comes first: a panel with **Set machine zero here** (X left end, Y front end, Z top), per-axis buttons and **Keep the last position**, with a note. Until every limited axis is referenced, the page says clearly that the soft limits are OFF. **Home** only when every enabled axis has a home pin.
3. Jogging (LIO-42): X−/X+/Y−/Y+/Z+/Z− buttons, Continuous and Step (0.01/0.1/1/10 mm), a speed slider. Mouse: press → JogStart, resent every ~100 ms from the tick while held; any left-button release or the window losing focus → JogStop. Keyboard on the Control page only, not while typing: arrows = X/Y, Page Up/Down = Z.
4. Work zero (LIO-43): Zero X/Y/Z/all (SetWork 0); G54–G59 through MDI while Idle; the active WCS shown.
5. Spindle and coolant (LIO-44): CW at an RPM, Stop; CCW only with a direction output; flood/mist only for configured outputs.
6. Feed override 10–200 % (LIO-45); limit override only when a limit input is configured.
7. MDI line with Send and Enter (LIO-46).
8. Machine model (LIO-47): its own lightweight canvas on the Control page, flat-shaded faces drawn back to front; not the Work page viewport. Moving gantry (Y) over a fixed T-slot table, X carriage on the beam, Z plate, 65 mm spindle and a nominal tool. ASSUMED dimensions in one place. The travel envelope and the tool tip from the live `machine_pos`; "machine position not set" styling while unreferenced. It moves with the simulator.
9. Status line (LIO-75): once the snapshot reports a state, "Connecting to …" is replaced by it.

## Non-goals (later parts)

Running setups and the job flow, tool changes and Z touch-off (part 3); editing the UC100 profile in AXIS CAM, diagnostics, I/O test (part 4); driver changes (part 5). This part changes nothing in the driver: it pins commit `36885a1fd0ea124e494b9db7d464e0393401ce6d` (branch axis-cam-foundation), where `SetMachine` marks the axis referenced and the JY5800 profile has soft limits X 0…390, Y 0…280, Z −55…0 (ASSUMED travel).

## Driver facts this design relies on

From `src/controller.rs` at the pinned commit:

- Manual commands (jog, SetWork, MDI, spindle) need `manual_ok()`: Idle or Paused, and not re-anchoring. `Home` and `SetMachine` need Idle and not re-anchoring.
- **`SetMachine` sets the device counters from the last status read and then re-anchors.** A second `SetMachine` processed in the same worker iteration is dropped (not Idle while re-anchoring), and could not use a fresh position anyway. So AXIS CAM sends one `SetMachine` per tick (§1.2).
- `JogStart` must be resent within 400 ms or the driver stops the jog. A `JogStart` while jogging only refreshes that deadline (even for another axis). `speed` is a fraction 0.001–1 of the axis's `max_velocity`.
- `JogStep` is a distance in mm at `feed` mm/min; it is checked against the soft limits up front ("Jog refused: …").
- Soft limits apply only when every axis with a `soft_min` is referenced. A jog into a soft limit just ends the jog; any other motion into one is an E-stop.
- `Home` refuses an axis without a `home` pin, and runs the profile's `homing.order` when given no axes.
- `SetWork` makes the current position `value` in the active WCS. `active_wcs` 0 = G54. G54–G59 change only through `Mdi("G55")` etc.
- Spindle CCW without `outputs.spindle_dir` or `outputs.spindle_ccw` is an E-stop; so is CW↔CCW without stopping first.
- `FeedOverride` is a fraction (1 = 100 %), clamped to 0.01–3. `OverrideLimits(true)` only while stopped; it switches itself off after 1 s with no limit input active.
- An axis index ≥ 6 in JogStart/JogStep/Home panics the worker. AXIS CAM only sends enabled axes 0–5.
- The state file keeps work offsets, the active WCS and the last position, not `referenced`; every session starts unreferenced at the last position.

## 1. `src/control.rs`

### 1.1 Connection

```rust
pub struct Connection {
    controller, pub machine_id, pub simulated, pub snapshot, _nap,  // part 1
    pub config: MachineConfig,             // the profile it was started with (simulated flag applied)
    declare: VecDeque<(usize, f64)>,       // SetMachine still to send, one per tick
    held: Option<Held>,                    // the continuous jog held down
    pub jog_step: Option<f64>,             // None = Continuous
    pub jog_speed: f32,                    // % of max velocity, 1–100, default 50
    pub feed_override: f32,                // %, 10–200, default 100
    pub rpm: String,                       // the RPM field
    pub mdi: String,                       // the MDI field
}
```

The page's machine-control settings live on the connection, so each connection starts from the defaults.

### 1.2 Actions

`pub enum Action` (one `Message::Machine(Action)` in `main.rs`), handled by `Connection::act`:

| Action | Sends | Guard |
| --- | --- | --- |
| `Home` | `Home(vec![])` | Idle; every enabled axis has a home pin |
| `MachineZero(None)` | queues `SetMachine 0` for X, Y, Z | Idle; enabled axes only |
| `MachineZero(Some(a))` | queues `SetMachine 0` for `a` | Idle; `a` enabled |
| `KeepPosition` | queues `SetMachine machine_pos[a]` for every enabled axis | Idle |
| `JogPress(a, neg)` | Step: `JogStep ±step at speed % × max_velocity`; Continuous: `JogStart`, and holds it | Idle or Paused; `a` enabled |
| `JogRelease` | `JogStop` if a jog is held | — |
| `JogStep(s)`, `JogSpeed(%)` | — (settings) | speed clamped 1–100 |
| `WorkZero(None/Some(a))` | `SetWork 0` for every enabled axis / for `a` | Idle or Paused |
| `Wcs(i)` | `Mdi("G5{4+i}")` | Idle; i < 6 |
| `Rpm(s)` | — | — |
| `Spindle(Off)` | `Spindle Off` | — |
| `Spindle(Cw/Ccw)` | `Spindle {state, rpm}` | rpm valid; CCW only with a direction output; not while turning the other way |
| `Flood(b)`, `Mist(b)` | `Coolant {flood, mist}` (the other from the snapshot) | that output configured |
| `FeedOverride(%)` | `FeedOverride(% / 100)` | clamped 10–200 |
| `OverrideLimits(b)` | `OverrideLimits(b)` | a limit input configured |
| `MdiText(s)`, `MdiSend` | `Mdi(line)`, then the field clears | non-empty |

Guards mirror the driver's so a refused command never reaches it as an E-stop (CCW, reversal) or a panic (axis ≥ 6); the page also greys out what the guards refuse. The driver stays the authority: everything it refuses is logged by it.

`refresh()` (each 33 ms tick, after copying the snapshot):

- **Declarations:** if Idle, send the next queued `SetMachine`; if not Idle (E-stop, a move), drop the queue, so nothing is declared later by surprise.
- **Held jog:** while the snapshot says Jog, resend `JogStart` every 100 ms. If the state leaves Jog while held (soft limit, E-stop, the driver's timeout), or never reaches Jog within 400 ms (refused), the hold ends: the operator must press again. A jog therefore never restarts by itself, for example after an E-stop and Reset.

Read-only helpers for the page: `held()`, `home_switches()` (every enabled axis has a home pin), `soft_limits_configured()`, `unset_limited_axes()` (enabled axes with a `soft_min` that are not referenced; soft limits are off while non-empty), `reversible()` (direction output), `limit_inputs()`, `rpm_value()` (1…`spindle_max_rpm`), `manual()` (Idle or Paused).

## 2. Keyboard and mouse (`src/main.rs`)

- Jog buttons are `mouse_area`s: `on_press` → `JogPress`, `on_release` and `on_exit` → `JogRelease`. Releasing over the button, leaving it, releasing anywhere else, or losing window focus all stop the jog.
- The global event listener adds: left-button release (any status) → `JogRelease`; window `Unfocused` → `JogRelease`; a jog key released (any status) → `JogKeyUp`; a jog key pressed and **not captured** → `JogKey(axis, negative, repeat)`.
- Jog keys: ←/→ = X−/X+, ↑/↓ = Y+/Y− (Y+ is away from the operator), Page Up/Down = Z+/Z−.
- "Not while typing": iced's text input captures ←/→ but not ↑/↓ or Page Up/Down, so "not captured" alone would jog Y or Z while typing an MDI line. `JogKey` therefore asks the widget tree whether any text field has the focus (one custom widget operation, iced's `advanced` feature) and only then starts the jog (`JogKeyChecked`). The key must still be down when the answer comes (`App::jog_key`), so a quick tap released before the answer cannot start a jog that nothing would stop.
- Keys only on the Control page and while connected. Repeats are ignored: the tick keeps a held key jog alive like the mouse (see Deviations), and Step mode moves one step per press.

## 3. The Control page

Left column (300 px): the machine picker, Connection, Connect/Disconnect (part 1), then the log (moved here from the right column so the main area has room at the minimum window width).

Main area, scrollable (as built after a look in the app: Position beside Jog, so jogging needs no scrolling at the default window size; the model beside Work zero, Spindle and Overrides; MDI last):

1. **Head:** the state in large type, SIMULATED, Reset (in E-stop), E-STOP, the identity line (part 1).
2. **Soft limits:** an amber box "Soft limits are OFF" naming the axes without a machine position, while any limited axis is unreferenced; "Soft limits on: X 0…390, Y 0…280, Z −55…0 mm" once all are; "This profile sets no soft limits" without any.
3. **Position and machine zero** (left) beside **the machine model** (right):
   - The position table: axis, machine, work (headed with the active WCS), "Position set" yes/no.
   - Machine zero: Set machine zero here, X, Y, Z, Keep the last position, Home (only with home switches on every enabled axis), and the note: jog X to its left end, Y to its front end (gantry toward you) and Z to the top, then Set machine zero here; Keep the last position uses the position saved at the last disconnect, only if nothing moved the machine since.
4. **Jog:** Continuous / 0.01 / 0.1 / 1 / 10 mm, the six buttons (X±, Y±, Z± for the enabled axes), speed 1–100 % with the resulting X feed, and a note on the keys.
5. **Work zero:** Zero X, Y, Z, all; G54…G59 with the active one highlighted (Idle only); a note.
6. **Spindle and coolant:** RPM, CW, CCW (only with a direction output; otherwise a note says why it is missing), Stop, the current spindle state; Flood and Mist checkboxes only for configured outputs.
7. **Overrides:** feed override slider 10–200 %; Override limit switches (only with limit inputs).
8. **MDI:** a field (Enter sends) and Send.

Buttons are greyed out when the machine cannot take the command (for example everything but E-stop in E-stop). Each new button has a tooltip in `help.rs`.

## 4. The machine model (`src/machine_view.rs`)

A `canvas::Program` drawn from scratch each frame: about 120 flat-shaded polygons, back faces culled, sorted far to near. Orthographic, orbit by dragging (yaw, pitch); the scale fits a fixed box round the whole machine, so it does not change as the machine moves.

World coordinates are machine coordinates of the tool tip (mm): machine zero at X left end, Y front end, Z top. ASSUMED dimensions, all in one block at the top of the module with a comment to measure them:

| What | Value |
| --- | --- |
| Travel X × Y × Z | 390 × 280 × 55 mm (Z −55…0) |
| T-slot table | 530 × 320 mm, travel centred on it |
| Table top to underside of the gantry beam | 70 mm |
| Table top to tool tip at Z top | 60 mm |
| Spindle body | Ø65 × 185 mm |
| Nominal tool | Ø6, 30 mm below the collet nut |

Parts: base frame and T-slot table (fixed); gantry (side plates and beam) at the tool's Y; X carriage on the front of the beam at the tool's X; Z plate, spindle, collet nut and tool at X, Y, Z. The tool-tip travel box is drawn from the profile's soft limits when X, Y and Z all have them, otherwise from the assumed travel; the tool tip is a red dot. While X, Y or Z has no machine position the moving parts are tinted amber (translucent parts showed their inner faces when tried), the box amber, and a label says "Machine position not set: the model follows the step counters, not the machine".

## 5. Status line (LIO-75)

In the tick, while the status line still starts with "Connecting to " and the snapshot has a state: "Connected to <machine>: <state>" or, if the driver could not connect, "Could not connect to <machine>; see the log on the Control page."

## 6. Error handling and safety

- E-stop is unchanged: top bar, Esc, any page. Every new action is refused by AXIS CAM's guard or by the driver when the machine is not in a state to take it.
- No held jog survives an E-stop, a soft limit, a refused start or a lost release: §1.2 and §2.
- Declarations never run outside Idle and are dropped on any state change.
- Before the machine position is set, nothing stops an axis at the end of its travel: the page says so in amber, and the model shows it.
- The assumed travel is in the driver profile (soft limits) and in `machine_view.rs` (drawing); both must be corrected after measuring the machine.

## 7. Testing

All connected tests use the simulated UC100 (`uc100_connection = "Simulated"`), in `smoke.rs`'s style, with small test profiles written to a temporary directory (soft limits; home switches and a limit input; spindle and coolant outputs):

- LIO-75: the status line shows the state after connecting.
- LIO-74: soft limits OFF until X, Y and Z are set; Set machine zero here sets all three to 0 and references them; a move past a limit is then refused; Keep the last position after a reconnect keeps the position and references it.
- LIO-41: Home references every axis on a profile with home switches; nothing is sent without them.
- LIO-42: a held continuous jog keeps moving past 400 ms and stops on release; a step moves exactly one step; a jog into a soft limit ends at the limit and the hold ends; jog keys: ignored off the Control page, while typing, and when released before the focus answer.
- LIO-43: Zero X and Zero all; G55 selects the second WCS.
- LIO-44: CW at an RPM, Stop; CCW and reversal are not sent; flood only when configured.
- LIO-45: feed override 150 %; limit override on with a limit input.
- LIO-46: an MDI line moves the machine; the field clears.
- LIO-47: the model's parts follow the position (tool tip at the position, gantry moves with Y only, table fixed); the painter culls back faces and draws far to near.
- The Control page renders disconnected, in E-stop, Idle unreferenced and referenced, jogging.
- Looked at in the real app against the simulator (isolated workspace, throwaway patch as in part 1).

## Deviations from the decisions

- **Keyboard jog keepalive.** Decision 3 said key repeats alone keep a key jog alive. Windows' default key-repeat delay is 500 ms, longer than the driver's 400 ms jog timeout, so a held key would stop and restart. Key jogs therefore use the same tick keepalive as the mouse; a lost key release is covered by window focus loss, any mouse release, and the key's own release.
