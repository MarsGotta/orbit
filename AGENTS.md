# Orbit engineering contract

Orbit is the macOS companion for a personal assistant running in Hermes, with
its own hub for meetings, notes and conversations. Product copy is Spanish;
repository documentation is English; code uses clear English names. Read the
[architecture](docs/architecture/README.md) and [self-hosting guide](docs/self-hosting.md).
The historical Orbit agent harness is unrelated.

## Boundaries

- **The hub is the single source** of chat, meetings, transcripts and notes
  (`server/orbit_hub`). The Mac records, uploads and reads back. It retains
  unconfirmed work and local voice-note audio; see [storage](docs/architecture/storage.md).
- **One app process on the Mac.** The daemon runs on a thread of Orbit.app
  (`apps/desktop/src-tauri/src/daemon_thread.rs`), with the Swift helper
  `OrbitNative` over stdio and the bundled `cua-driver` engine. The daemon owns
  capture, local durable state and device effects. Presentation never talks to providers.
- **Closing the window never stops recording; quitting always closes the daemon
  in order.** Every exit path reaches that closure: tray, ⌘Q, Dock, update, signal.
- Pure domain decisions live in `crates/orbit-core`: no clocks, network, files,
  processes or global state. Pass time and facts explicitly.
- Server provider credentials stay on the server. Device credentials and storage
  keys use macOS Keychain. No plaintext fallback for sensitive payloads.
- Effects are adapters. Validate before executing, record uncertain outcomes,
  and never retry an input action merely because its receipt was lost.
- Privacy stop is local, durable and latched; remote traffic cannot rearm it.
- Passing mock tests do not establish macOS permissions, audio or real Hermes acceptance.
- Development must not modify live Hermes checkouts, profiles, services or vaults.
  Use isolated fixtures and explicit configuration.

## Recurring failure modes

These have caused lengthy investigations or broken releases. Look for them actively.

- **One generic word covering several causes.** `Unknown`, `unavailable`,
  `no_answer`, `Timeout`, “over the limit”, or “TLS connection” for a stopped
  hub conceal the actionable cause. Every refusal names its cause and carries
  its numbers—size, ceiling, counts and window—all the way to the caller, not
  just a log. An assistant can correct its request when it has those facts.
- **A name that is untrue.** An action in progress must never be described as
  awaiting consent. Stops caused by different rules must have distinct causes;
  a shared `idle` label once concealed which call rule fired.
- **A ceiling with no stated purpose.** State in source what each bound protects,
  why it has that size and what it does not limit. Different resources need
  separate bounds: sharing a 1 MiB engine-frame limit with screenshots broke Retina capture.
- **A constant mirrored across languages without a guard.** Before changing a
  shared bound, search `.rs`, `.py`, `.swift`, `.ts` and `contracts/`. Register
  every copy in `contracts/constants.json` with a guard comparing them.
- **Absence read as a negative fact.** An evicted view is unknown, never “no”.
  Model three values; a boolean can revoke permission before any view arrives.
  Assistant work or speech is not user silence; queued audio is not a silent microphone.
- **A test whose result is decided by the clock.** Measure the failure rate first.
  Derive budgets from p99 and worst observed, wait for events, then re-measure.
  Never soften an assertion to get green. Paused virtual time mixed with real I/O races.
- **A test with effects on its host.** Test decisions against synthetic listings,
  not actions against the machine. Strip fixture profiles of every power the
  test does not require, including terminals and inherited pre-approved commands.
- **Evidence carrying private data.** Save only what a check needs. Whole-window
  accessibility output can include Recent Items and personal document names.
  Remove unrelated app and system content before saving evidence.
- **A fake external tool that never changes.** A fake command once kept passing
  after Hermes changed its CLI. Verify external commands against the actual
  installed tool for releases, without disrupting live services during development.
- **A second layer of consent over a system permission.** The macOS permission
  is the user's decision. Do not add expiring reviews, activation switches or
  confirmations for that same permission.
- **Assuming Linux and macOS agree.** Socket flags, headerless executable files,
  `ps` and `/proc` differ. Passing on Linux is necessary, never sufficient.
- **Assuming how macOS quits or reopens.** ⌘Q and Dock Quit reach Tauri as
  `RunEvent::Exit`, not `ExitRequested`; Dock activation and reopening from Finder
  arrive as `RunEvent::Reopen`. A quit Apple Event addressed by name may reach
  the helper in the same bundle. Verify these paths on macOS.
- **A sweeping commit.** Stage files by name. Broad staging can include stale
  browser output and personal tool settings.

## Working practices

Keep modules focused and contracts typed. Prefer small vertical features over
empty scaffolding. Do not panic on external input. Bound messages, audio and
queues. Keep conversation content and secrets out of logs. Tests and examples
use invented people, organizations, meetings and addresses; use documentation
addresses such as `100.64.0.1`, `hub.example` and `example.test`.

**The gate** is `bash scripts/check.sh`; `--plan` lists its steps. Put **Node 22**
first on `PATH`; Node 26 breaks the current Vitest setup. `--instalar` prepares
locked Python/frontend dependencies. The zones are `rust`, `guards`, `hub` and
`ui`. The full gate covers every workspace crate in two Cargo feature groups,
hub e2e, Ruff over the hub and native harness, guards and frontend checks.
Excluding an additional crate can hide a defect from the app build. Run the
whole gate before integration; a macOS release build alone does not check the
hub and frontend. Use timeouts for test commands.

**On macOS**, native acceptance requires consented fixtures. Skipped is not
passed. The `scripts/e2e-mac` harness sends real keys: use the separate “Orbit
Pruebas” identity, an isolated hub/profile, and a Mac with nobody at the keyboard.
Calls and meeting wake-phrase turns also need real voice trials; macOS can duck
synthetic audio. Record what was measured, with repetitions for rate claims,
and keep private evidence outside the public repository.

**Configuration and delivery.** Infrastructure belongs in explicit configuration,
not source defaults or examples. Credentials and private operator instructions
stay outside Git. A deployment must review hub/Mac compatibility; the normal
upgrade order is hub then Mac. Development checks must not publish builds or
restart shared services.

**Specs and agents.** The product is the desktop app. The standalone `orbit`
(`crates/orbit-daemon/src/main.rs`) exists for development and tests and is not
shipped; there is no product CLI or TUI. A spec can be wrong: if evidence refutes
its stated cause, report the measurement instead of implementing that mechanism.
Each implementer uses its own worktree and does not stage, commit, branch,
merge or rebase. The reviewer integrates the work.
