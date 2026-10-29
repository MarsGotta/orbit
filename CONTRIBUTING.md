# Contributing to Orbit

Thank you for considering a contribution. Orbit is a macOS companion app for a
personal assistant running in [Hermes Agent](https://github.com/NousResearch/hermes-agent),
with its own hub. It is beta software for macOS on Apple Silicon, and its
interface catalogue supports English and Spanish.

[AGENTS.md](AGENTS.md) is the engineering contract: component boundaries, rules
for effects, privacy and bounds, and the failure modes that have cost the most
time. Read it, and the [architecture](docs/architecture/README.md), before
changing code. This guide covers the practical side: the environment, the gate,
what only a Mac can check, and how to propose a change.

## Set up the environment

Development needs:

- Bash 5 or later (on macOS, `brew install bash`).
- Python 3.11 or later and [`uv`](https://docs.astral.sh/uv/).
- The Rust toolchain pinned in [rust-toolchain.toml](rust-toolchain.toml), with
  `clippy` and `rustfmt`. Install it with `rustup toolchain install` using that
  channel; if it is missing, the gate prints the exact command.
- **Node 22** and pnpm 9.15.4 (the version in `apps/desktop/package.json`).
  Put Node 22 first on `PATH`: Node 26 breaks the current Vitest setup.
- `ffmpeg`, which the hub's final-pass tests run for real.
- `git`, which the guards use to list tracked files and build fixture
  repositories.

On Linux, the Tauri shell also needs a C/C++ toolchain, `pkg-config` and the
development packages for GTK 3, WebKitGTK 4.1, Ayatana AppIndicator, librsvg,
OpenSSL and D-Bus. On Debian or Ubuntu:

```sh
sudo apt-get install build-essential pkg-config libgtk-3-dev \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
  libssl-dev libdbus-1-dev ffmpeg
```

On macOS, the native helper and the app bundle also need Xcode tools with the
macOS 26 SDK (for `SpeechAnalyzer`) and Swift 6. `bash scripts/macos/check-sdk.sh sdk`
and `bash scripts/macos/check-sdk.sh xctest` report, without changing anything,
whether the active toolchain provides them. Install GNU coreutils for `gtimeout`.

Development never touches a live Hermes installation, profile, service or
vault. Use an isolated Hermes profile and an isolated hub with explicit
configuration; see [self-hosting](docs/self-hosting.md).

## The gate

The gate is one command that checks the whole repository on Linux or macOS:

```sh
# First time, or after a lockfile changes: install locked Python and frontend
# dependencies into server/.venv and apps/desktop/node_modules, then run it.
timeout 3600 bash scripts/check.sh --install
# Afterwards:
timeout 3600 bash scripts/check.sh
```

It ends with a table of every step, its result and its duration, and exits
non-zero if any step fails. `bash scripts/check.sh --plan` lists the steps
without running them. Pass one or more zones to run only those:

| Zone | What it checks |
|---|---|
| `rust` | `cargo fmt`, strict Clippy, `cargo test` for every workspace crate in two Cargo feature groups (the daemon and its crates; the app), `orbit-core` and the test bridge with `pruebas`, and that the generated TypeScript is current |
| `guards` | The size limits and the domain boundaries (below), then the Python guards in `scripts/tests`: shared constants and their copies, packaging, installer and release scripts. The `scripts/tests` step runs after the hub's environment step and is skipped, saying why, without it: the hub publish guards build a release with the real `uv` offline, from the cache that syncing `server/.venv` fills |
| `hub` | Ruff and Ruff format over the hub and the harness, the hub's pytest suite, including end-to-end tests against the real daemon binary, and the real-test harness's dry run |
| `ui` | Lint, typecheck, Vitest and the production build of `apps/desktop` |

Run the whole gate before asking for review; a single zone can miss what
another zone reads (the hub tests, for example, read constants from the Rust
source). `CARGO_TARGET_DIR` selects where Cargo builds, and
`ORBIT_CHECK_LOG_DIR` where each step's output is kept (by default
`target/check/<timestamp>/`).

**Size limits.** A production file in Rust, Python or Swift has at most 500
lines of code and a function at most 80, as ESLint already enforces (more
tightly) in the interface. Comments, docstrings and blank lines do not count;
tests and generated files are not measured. `scripts/check/sizes.py` says
exactly what is measured and how. What was already over is listed, at its size,
in `scripts/check/size-ratchet.toml`: an entry may only go down, a new entry is
refused, and an entry that shrank must be lowered
(`python3 scripts/check/sizes.py --shrink`).

**Domain boundaries.** A domain uses another only through its handle and its
types: in `orbit-core`, each module of `transition/` is a domain; in the
daemon, each module that owns a handle of `Runtime`. `scripts/check/boundaries.toml`
lists the allowed edges between domains and the reaches into another domain's
internals that already existed; `scripts/check/boundaries.py` fails on any new
one and on entries the code no longer has.

A skipped test is not a passed test. The gate fails on any skip that is not
listed, with its reason, in `scripts/check/allowed-skips.toml`. Never weaken,
skip or delete a test to get green. The TypeScript in
`apps/desktop/src/bridge/generated/` is written by `cargo test` of
`orbit-app-core`; commit it exactly as the test writes it.

Every pull request and every push to `main` runs the whole gate on Linux in
GitHub Actions ([check.yml](.github/workflows/check.yml)). Nothing that needs a
Mac runs there.

## What needs a Mac

Linux runs the whole gate, and passing it is necessary, never sufficient. These
need macOS on Apple Silicon:

- The Swift helper: `gtimeout 600 swift test --package-path native/macos`.
- The app bundle: `ORBIT_CUA_DRIVER_BINARY=/absolute/path/to/cua-driver bash scripts/macos/bundle.sh`
  (see [installation](docs/INSTALL.md#build-a-macos-package)).
- An isolated launch of a built helper:
  `python3 -B scripts/macos/smoke-native.py --binary <built OrbitNative>`. It
  checks stdio and the privacy latch, not permissions or audio.

Mock and Linux tests cannot establish macOS permissions (Microphone, Screen
Recording, Accessibility, Calendars), audio capture or echo cancellation,
Keychain access, focus, how macOS quits and reopens the app, signing and
notarization, or that a real Hermes profile works. Say which of these your
change touches and what you measured on a Mac; "skipped" is not "passed".

## The real-test harness and Orbit Pruebas

“Orbit Pruebas” (Spanish for “Orbit Tests”) is a separate build of the app with
its own bundle identifier, data directory and Keychain service, so tests never
touch a real installation. The Cargo feature `pruebas` and `ORBIT_PRUEBAS=1`
select it. On the Mac, `scripts/e2e-mac/build-test-app.sh` builds it from the
tree and installs it in `~/Applications/Orbit Pruebas.app`, signed with a stable
identity of your own so the macOS permissions you grant it once survive the next
build. It is never notarized or published.

`scripts/e2e-mac/real_test.py` is the real acceptance test. It runs on a Linux
machine that hosts an isolated hub and Hermes profile, and drives the Mac over
SSH:

- `prepare` creates, once, the test's private credentials under
  `~/.orbit-pruebas/`, and deletes only the drafts (`orbit.draft.` keys) from
  Orbit Pruebas's own WebKit storage (`~/Library/WebKit/app.orbit.companion.pruebas`),
  which also keeps the first run's completion and the theme.
- `run` (Spanish by default) or `run --language en` is the real test: it records a synthetic meeting in Orbit Pruebas by
  clicking and reading its interface (past a first run's welcome, skipped with
  «Configurarlo más tarde» and said in the report), with part of it muted, sends a voice
  note, opens a call, throws a second recording away, deletes the first
  meeting, runs the checks and cleans up, also when something fails. It reads
  the test hub's database to check that a deleted recording left nothing there.
  It moves the system pointer over the floating panel and away, and quits and
  reopens Orbit Pruebas once (the Dock's quit) to check a draft
  survives. Nobody should be using the Mac while it runs. The speech is synthesized
  with `say` into the BlackHole 2ch virtual input, so the Mac needs
  `blackhole-2ch`, `switchaudio-osx` and voices for the selected language
  (two English voices for the English pass), plus the corresponding on-device
  speech recognition; the run
  switches the audio input and restores it afterwards. All presses, typing and readings inside
  Orbit Pruebas use its [test bridge](docs/testing/test-bridge.md), including
  check 16's native menu snapshot. CuaDriver is started only around check 21,
  which reads macOS accessibility before and during recording. Active labels
  come from checked catalogue exports; see [language passes](docs/testing/test-bridge.md#language-passes)
  for regeneration commands and Mac acceptance.
  Check 18 still moves the real pointer through the Swift script. Orbit itself must be
  closed, because both builds register the same global shortcuts.
- `dry-run --language en` covers the English pass; `dry-run` defaults to Spanish.
  Both walk the same orchestrator and checks on Linux against the example
  states in `scripts/e2e-mac/dry-run-examples/`. It proves the logic of the
  checks, not Orbit, and the gate runs it.

**`run` sends real keystrokes and clicks to the Mac.** Use it
only with the Mac owner's consent, on a Mac with nobody at the keyboard, with
the separate Orbit Pruebas identity and an isolated hub and Hermes profile.
Turn off Bluetooth audio devices that could switch the route mid-run. Calls and
the meeting wake phrase also need trials with a real voice: macOS can duck
synthetic audio. Keep the evidence you collect outside the repository and save
only what a check needs; accessibility output can contain personal document
names.

## Data in tests, examples and documentation

Tests, fixtures, examples and documentation use invented people,
organizations, meetings and addresses. Use documentation addresses such as
`100.64.0.1`, `hub.example` and `example.test`, never a real host, account,
meeting title, colleague or customer. Commit no credentials, private keys or
tokens, not even expired ones; mark a synthetic key that looks real so secret
scanners can recognize it.

## Language

Code, identifiers, comments, commit messages and repository documentation are
in English. Interface copy belongs in the typed English and Spanish catalogues. The app
chooses its language in Settings › General, following the system by default.
Inline feature copy and native/server messages are being migrated separately;
see [interface language](docs/architecture/interface-language.md). Developer tooling
(the gate, the scripts, the harness and the hub's operator messages) is in
English.

## Propose a change

1. For anything beyond a small fix, open an issue first describing the problem
   or the behaviour you want, so the approach can be agreed before you write it.
2. Keep pull requests small and focused on one change. A refactor and a
   behaviour change go in separate pull requests.
3. Add or adapt the tests that fix the behaviour, and show that they fail
   without the change.
4. Run the whole gate and make it green. Note anything you could not check
   (for example, macOS behaviour from a Linux machine).
5. Stage files by name. Sweeping commits (`git add -A`, `git commit -a`) have
   included build output and personal tool settings before.
6. Describe what changed, why, and how you verified it.

Changes to a wire protocol in `contracts/`, to stored data or to a shared bound
need special care: the hub and the Mac are upgraded separately (hub first), and
a published version must keep working with its counterpart. A constant mirrored
across languages is registered in `contracts/constants.json` with a guard that
compares the copies.

Report security vulnerabilities privately as described in
[SECURITY.md](SECURITY.md), never in a public issue.
