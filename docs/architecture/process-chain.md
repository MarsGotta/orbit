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
 ├─ Contents/MacOS/OrbitNative --stdio --managed
 ├─ Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver serve --socket <data>/cua/driver.sock
 └─ Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver mcp --embedded --socket <data>/cua/driver.sock
```

See the [overview](README.md) for component boundaries.

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
  daemon completion closes the app and logs the cause. There is no relauncher.
- **Crash:** memory-only audio is lost. Next startup marks the recording
  interrupted and reports how many chunks were saved
  (`RecordingInterruptedByRestart` in `orbit-core/src/model.rs`).
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

`ORBIT_PRUEBAS=1 bash scripts/bundle-macos.sh` enables the `pruebas` feature and
builds “Orbit Pruebas”, with a separate bundle identity, data directory and
Keychain service, plus its restricted HTTP test-network exception. See
`crates/orbit-core/src/orbit_bundle.rs` for exact identities. The helper's own
lock/log paths are independent of the feature. Use your own stable test signing
identity; acceptance is not established by an ad hoc build.

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
