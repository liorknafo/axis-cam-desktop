# UC100 over the network: "Pi (network)" connection

The owner, 2026-10-30:

> I want to create with raspbery pi a server that will be connected to the CNC
> as we are doing now with this mac, and we will connect with this software to
> the server that will control the CNC, so the CNC will be able to run even if
> the mac is disconnected

## Pieces

- **The server.** The Pi runs `uc100-server` (github.com/liorknafo/uc100-server),
  which owns the UC100 through the driver's `Controller`.
- **The client.** AXIS CAM drives it with `uc100_remote::RemoteController`, which
  has the same shape as `Controller` (`send`, `snapshot`, `shutdown`), plus
  `link()`, `pair()`, `take_control()` and `stop_after()`.
- **The shared driver.** Both pin the same driver commit, so `Command` and
  `Snapshot` are one type.

## The owner's decisions

- **When the Mac drops, the Pi finishes the current tool run.** The job is on
  the Pi as one `Command::Job`, so this is the driver's normal behaviour. A
  held jog still stops within 0.4 s.
- **Pairing.** Pair once with the code `sudo uc100-server pair` shows. One
  computer controls; other paired ones watch and may still E-stop, Hold and
  Stop.
- **Stop after step: the Pi decides.** When armed, AXIS CAM sends the Pi the
  job-line ranges where a stop is due (`RemoteController::stop_after`). The
  Pi stops there by itself, with or without the Mac.

## AXIS CAM changes

### Machine settings

- **`uc100_connection`** gains a third value, "Pi (network)", beside "USB" and
  "Simulated".
- **`uc100_pi`** is the server's address, by default `uc100.local` (port
  7110).
- **The pairing token** is not a project setting: it is a credential for
  driving the machine. On macOS it goes in the Keychain, as the item
  "AXIS CAM Studio" / `uc100-pi-<machine id>`. An isolated (test or smoke)
  workspace keeps it in a 0600 file beside the workspace instead, as
  `session.rs` does for the sign-in.

### `control::Connection`

- **The controller.** `Connection` holds a `Driver`: either `Local(Controller)`
  or `Remote(RemoteController)`.
  - `send` returns whether the command went. A local one always goes.
  - `close` shuts a remote link down and never sends `Disconnect`. The Pi keeps
    the UC100, and the job keeps running.
- **Remote state.**
  - The driver's state file (offsets, last position) lives on the Pi:
    `pad_state` and the local state file are not used.
  - The run journal stays on the Mac (`<machine>-pi-journal.json`).
  - `manual()` also needs the link live: with the link down, the snapshot is
    the last one received and nothing can be sent.

### Runs

- **A Job not sent.** `send` returns false when the link is down or another
  computer controls the machine. Cycle Start then puts the run back as it
  was, as for a job the driver refused (`NOT_STARTED`), and says why.
- **A Job lost on the way.** The link can fail while a job is still crossing.
  Then the Pi has no new outcome and isn't busy. If the link is live, that
  state lasts 3 s after the send, and the machine never showed the job
  running, the run is put back the same way: "The tool run did not reach the
  Pi".
- **Disconnecting while a job runs on the Pi** doesn't mark the run stopped
  (`leaving`): it carries on.
- **Stop after step.** On a Pi connection, arming it sends the ranges; the Pi
  stops and reports it. On USB and the simulator it stays as it is
  (`run::Run::stop_due`).

### Control page

- **Connection choice.** Real (USB) / Simulated / Pi (network).
- **With Pi (network) chosen:**
  - the server's address;
  - "Paired as <name>" with Forget, or a code field and Pair.
- **Connected to the Pi**, the link's state:
  - Connecting…;
  - Refused: why (pair again, or update one side);
  - Watching (another computer controls), with Take control;
  - Controlling.
- **Link down:** "Lost the link to the Pi: the machine carries on. Reconnecting…".
  The server's and the client's notes (a command refused or not sent) show
  in the status line.
