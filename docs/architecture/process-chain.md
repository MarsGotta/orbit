# App process chain

Orbit.app contains the UI and daemon in one app process. The daemon runs on its
`orbit-daemon` thread with a Tokio runtime and an 8 MiB stack. app-core uses
in-memory calls and topic watches; only “Orbit Pruebas” and the standalone
development daemon expose `orbit.sock`. The daemon launches the managed Swift
helper and the bundled control engine. There is no shipped standalone daemon.

```text
Orbit.app/Contents/MacOS/Orbit
 │  main thread: Tauri window, tray, panel and app-core
 │  orbit-daemon thread: daemon ← Call / watch → app-core
 │  orbit-helper-bridge thread: the helper's pipes, its heartbeat and relaunches
 │  orbit-capture thread: every frame, sealing and chunk uploads
 │  orbit-store thread: every SQLite write and fsync
 ├─ Contents/MacOS/OrbitNative --stdio --managed
 ├─ Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver serve --socket <data>/cua/driver.sock
 └─ Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver mcp --embedded --socket <data>/cua/driver.sock
```

See the [overview](README.md) for component boundaries.

## Capture, apart from the loop

The daemon's loop serves orders, the helper's other events, the hub, work
results and every vertical's upkeep, one turn at a time. A recording does not
wait for any of it (`crates/orbit-daemon/src/daemon/capture_lane/`):

- The bridge numbers every helper event. `audio` and `gap` go to the capture
  task's own channel; the rest go to the loop, each with a marker at its place
  in the task's channel. The loop serves an event only once the task has
  handled every frame before it, so the helper's order holds across both.
- While a recording is in its plain steady state (nothing draining, no voice
  note or call on the microphone, the writer well), the loop arms the lane and
  the task sequences, seals and queues each frame itself. Any other frame, and
  every one after a frame it could not serve, goes to the loop in order, which
  serves it exactly as before.
- The task's own clock reads seal receipts, retries owed seals and uploads the
  chunks, one at a time, under the retry gate the loop's closures share. A
  decision that retires uploads (a privacy stop, Orbit closing) aborts the one
  in flight before it takes effect.
- A recording's chunk counters are a projection of the chunks it sealed: at
  start they are rebuilt from the store, so a recording an older version left
  cut is closed at its last audio and the hub writes its note.
- The loop's own part of the capture's upkeep (a drain's deadline, a cut's
  recovery, a call's health) runs on its own clock, never behind the other
  verticals'.

In the helper, the daemon's beats are observed on the channel's reader queue
and its silence is judged on a queue of its own
(`native/macos/Sources/OrbitNativeState/DaemonLivenessWatch.swift`): a held main
thread is never the daemon's silence, and a capture in progress goes on
without it.

## Lifecycle

In `apps/desktop/src-tauri/src/daemon_thread.rs` and `src/lib.rs`:

- **Startup:** wait up to 15 seconds (`DAEMON_READY_WAIT`) for `Status`.
  `daemon.lock` prevents another daemon on that data directory. The open-file
  `flock` also separates simultaneous starts. Development outside a bundle
  uses `Launch::development`, dev storage and no helper.
- **Quit:** tray/update paths reach `ExitRequested`; macOS ⌘Q, app-menu Quit and
  Dock Quit reach `Exit` via `applicationWillTerminate`. Both call `quit_on_exit`:
  one `Shutdown` and a thread join bounded to 17 seconds (`DAEMON_EXIT_WAIT`).
  Its compile-time budget includes shutdown (10 seconds), snapshot receipt grace
  (1128 ms), helper exit grace (3 seconds) and final report flush (2 seconds).
  A thread cannot be killed independently; process exit ends it if the budget
  expires. Startup recovery may refuse `Shutdown`; the app reports it, waits
  and exits, and the next launch recovers again.
- **Signals:** the daemon alone installs SIGTERM/SIGINT handlers. The first
  starts the same bounded shutdown; a second requests immediate exit. Unexpected
  daemon completion (a panic, or its run ending) after its first minute
  relaunches the whole app (`DAEMON_RELAUNCH_MIN_UPTIME` in
  `daemon_thread.rs`); one that falls sooner closes the app and logs the cause.
- **Actor panic:** a panic inside one of the daemon's actors (the capture
  task, the store writer, the helper's supervision, the hub connection, the
  loop's workers) is caught there, logged as `actor_panicked` with the actor
  and its counts but never the panic's message, and only that actor is
  relaunched from durable state; a recording goes on. Each actor has three
  relaunches per minute (`RESTART_BUDGET`, `RESTART_WINDOW` in
  `orbit-daemon/src/supervision.rs`); its next panic ends the loop with a
  failure, the unexpected completion above. A panic of the loop itself is
  one too.
- **Crash:** memory-only audio is lost. Next startup marks the recording
  interrupted and reports how many chunks were saved
  (`RecordingInterruptedByRestart` in `orbit-core/src/transition/capture.rs`); once the new
  helper is ready, the same recording resumes in a new segment
  (`orbit-daemon/src/daemon/cut_recovery.rs`).
- **Cut:** a recording cut by anything but the person («Terminar», the privacy
  stop), Orbit closing or the Mac going to sleep resumes by itself when the
  capture is back; one that cannot is closed at its last audio so the hub
  writes its note. A declared gap never cuts it.
- **Window close:** hides the window, preserving recording and the menu bar app.
  Dock activation or Finder reopen shows it again.
- **Helper exit:** the daemon sends `shutdown`, closes stdin, drains stdout and
  allows 3 seconds before killing it. EOF, including parent death, makes the
  helper stop and exit on its own within 2 seconds.

## Ownership

The app owns the instance, window, tray and floating panel. The daemon owns
capture, durable state/jobs, hub connection and helper/engine lifecycle. The
helper owns macOS effects, shortcuts and sleep/session observers. In managed
mode it creates no second icon, login item or instance lock. Private pipes expose
no network listener. The engine executes computer-use tools.

Process-wide setup happens once from the daemon: signal handlers, the Sentry
panic hook (also seeing UI panics) and, when Sentry is enabled, rustls' `ring`
provider. Bundled helper/engine and live-ancestor resolution use the app
executable identity (`Member::App` in `orbit-core/src/orbit_bundle.rs`).

## Control engine

`cua-driver` is pinned by `VERIFIED_DRIVER_VERSION` in
`crates/orbit-daemon/src/cua.rs`, currently 0.28.2. See
[CUA](../../contracts/cua-v1.md) and [control policy](../../contracts/control-policy-v1.md).

- `orbit_core::computer_control::engine::decide` runs from maintenance. It starts
  the engine with both system permissions granted, helper connected and privacy
  rearmed. It needs no hub/pairing/request. Privacy stop, withdrawn permissions
  or exit stop it; an unknown permission neither starts nor stops it. Failed
  starts wait 5 seconds (`ENGINE_RETRY_DELAY`) before another attempt.
- The daemon creates `<data>/cua/` mode 0700, starts `serve` in its own process
  group (the engine forks), then the MCP stdio proxy with `--embedded` to prevent
  another engine app instance. Connection closure ends engine and proxy.
- Commands cannot select another executable or socket.
- On macOS an old socket's owner is stopped only when verified as an orphaned
  engine of the same bundle/user; otherwise only the socket name is removed.
- Full-screen capture writes `<data>/cua/desktop-<id>.png`; the daemon always
  deletes the temporary file after reading and resizing.
- TCC permissions belong to Orbit.app, without a separate engine entry.

## Hub connection

The daemon is the Mac's only hub client. Its token-authenticated `/v1/ws` carries
chat, `device.request`/`device.reply` and `recording.changed`. It reconnects after
loss without resending a submitted turn. HTTPS handles chunks, closure, meetings,
notes, templates, conversations, dictation, speech and captures.

Without a hub, recording continues and uploads queue. Plain HTTP is allowed only
for explicit localhost development and the acceptance app's restricted test
network range; see `validate_hub_url` in `crates/orbit-daemon/src/config.rs`.

## Acceptance identity and development binary

`ORBIT_PRUEBAS=1 bash scripts/macos/bundle.sh` enables the `pruebas` feature and
builds “Orbit Pruebas”, with a separate bundle identity, data directory and
Keychain service, plus its restricted HTTP test-network exception. See
`crates/orbit-core/src/orbit_bundle.rs` for exact identities. The helper's own
lock/log paths are independent of the feature. Use your own stable test signing
identity; acceptance is not established by an ad hoc build. On the Mac,
`scripts/e2e-mac/build-test-app.sh` builds and installs it with that identity
for the real-test harness described in
[CONTRIBUTING](../../CONTRIBUTING.md#the-real-test-harness-and-orbit-pruebas).

The standalone `orbit` (`crates/orbit-daemon/src/main.rs`) is used by hub e2e,
app-core and daemon process tests. `--parent-pid` prevents it outliving the test.
It is not shipped. The retired helper→daemon startup chain is rejected:
the daemon starts `OrbitNative --stdio --managed`; tests can use
`--stdio --test-state-directory <directory>` without acquiring host data.
Bare `--stdio` and no-argument launches are refused.

The installer identifies app/helper processes by executable path, including a
legacy `Contents/Resources/orbit` daemon during upgrades from 0.5.x. The engine
is not counted. A basename or command line alone cannot classify an unrelated
process as Orbit. Remaining processes are named for the user to quit.
