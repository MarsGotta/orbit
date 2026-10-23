<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/orbit-logotipo-color-oscuro.svg">
    <img alt="Orbit" height="48" src="docs/assets/orbit-logotipo-color-claro.svg">
  </picture>
</h1>

Orbit is a macOS companion for a personal assistant running in
[Hermes Agent](https://github.com/NousResearch/hermes-agent), the open-source
agent by Nous Research. It brings together the calendar, meeting recording,
notes, chat and voice, and lets the assistant see and use the Mac through macOS
permissions. The app's interface is in Spanish. The assistant name defaults to
Deimos and is configured on the hub (see
[self-hosting](docs/self-hosting.md#install-and-configure-the-hub)).

Orbit has its own hub: the single source of meetings, transcripts, notes and
conversations. The Mac records, encrypts and uploads audio, then reads results
from the hub. Hermes owns the assistant's identity, models and provider
credentials. A local, durable privacy stop cuts capture, playback and transfers.

## Status

Orbit is beta software. It runs only on macOS on Apple Silicon; there is no
Intel, Windows or Linux app (Linux runs the development checks and can host the
hub). The interface is in Spanish only. Changes are listed in the
[changelog](CHANGELOG.md).

## Requirements

- Apple Silicon and macOS 15 or later. The local “Oye Deimos” meeting wake phrase
  requires macOS 26; earlier systems use the question button.
- Your own Orbit hub, Hermes profile and transcription service for chat, voice
  and meeting processing. See [self-hosting](docs/self-hosting.md).
- Microphone permission for recording; Screen Recording for system or app audio
  and screenshots; Accessibility for computer control and selected text;
  Calendars for the local calendar. Orbit requests the relevant system permission.

Recording and the local calendar work without a connected hub. Unconfirmed audio
and private notes wait on the Mac; viewing hub notes and conversations needs a
connection.

## Install

Build your own app using the instructions below, or obtain a package from an
operator you trust. This repository does not assume an update or download host.
See [installation and use](docs/INSTALL.md) for package verification, pairing,
permissions, shortcuts, updates and removal.

## Build and check

Development needs Bash 5, Python 3.11 or later, `uv`, the Rust toolchain pinned in
[rust-toolchain.toml](rust-toolchain.toml), pnpm 9.15.4, **Node 22** and `ffmpeg`
(the hub tests run it). Put Node 22 first on `PATH`; Node 26 is incompatible with
the current Vitest setup. Linux can run the whole repository gate; the native
helper and app bundle require macOS, Xcode tools with the macOS 26 SDK (for
`SpeechAnalyzer`) and Swift 6. On Linux, building the Tauri shell also needs a
C/C++ toolchain, `pkg-config` and development packages for GTK 3, WebKitGTK 4.1,
Ayatana AppIndicator, librsvg, OpenSSL and D-Bus.

From the repository root:

```sh
# Install locked Python and frontend dependencies, then run the full gate.
timeout 3600 bash scripts/check.sh --install
# Subsequent checks, using the prepared environments:
timeout 3600 bash scripts/check.sh
```

The gate covers Rust formatting, strict Clippy, all workspace crates in two
feature groups, generated TypeScript, guards, hub tests including daemon e2e,
and frontend lint, types, tests and build. `bash scripts/check.sh --plan` lists
the steps; `rust`, `guards`, `hub` and `ui` select individual zones. A skipped
required test is a failure. [CONTRIBUTING.md](CONTRIBUTING.md) describes the
environment, the zones and what only a Mac can check.

On macOS, use a `timeout` implementation (for example GNU coreutils' `gtimeout`)
for test commands, and build with the exact `cua-driver` version pinned in
[the adapter](crates/orbit-control/src/cua.rs):

```sh
gtimeout 600 swift test --package-path native/macos
ORBIT_CUA_DRIVER_BINARY=/absolute/path/to/cua-driver bash scripts/macos/bundle.sh
```

Without a signing identity this produces an ad hoc development build, not a
notarized distribution. Signing and notarization settings are described in
[installation](docs/INSTALL.md#build-a-macos-package). Native permission, audio,
focus and real Hermes acceptance require a Mac and consented fixtures; Linux
and mock tests cannot establish those results.

## Repository map

| Location | Responsibility |
|---|---|
| `crates/orbit-core` | Pure decisions, state, policies and typed contracts; no clocks or I/O |
| `crates/orbit-store` | Encrypted Mac storage, durable jobs and pending private notes |
| `crates/orbit-ipc` | Local paths, configuration, in-memory transport and development/test socket |
| `crates/orbit-daemon` | Capture, storage, hub connection and device effects; standalone `orbit` is for development/tests |
| `crates/orbit-app-core` | The app's daemon client, view projections and updater (release manifest and installer download) |
| `apps/desktop` | Tauri/React app with the daemon on its own thread |
| `native/macos` | Swift `OrbitNative` helper |
| `server/orbit_hub` | Hub, meeting processing and assistant bridge |
| `server/orbit_mcp` | `orbit-mcp`, the assistant's Orbit tools through the hub |
| `contracts` | Versioned protocols, shared constants and fixtures |
| `scripts` | The gate (`check.sh`) and its guards (`tests/`), macOS packaging (`macos/`), release tools (`release/`), hub deployment (`hub/`) and the real-test harness (`e2e-mac/`) |
| `third-party/cua-driver` | License of the bundled `cua-driver` control engine |

The [architecture](docs/architecture/README.md) explains component boundaries,
[processes](docs/architecture/process-chain.md),
[storage](docs/architecture/storage.md),
[transcripts](docs/architecture/transcripts.md) and
[continuous voice](docs/architecture/continuous-voice.md).
Protocol references: [hub](contracts/hub-v1.md),
[native helper](contracts/native-v1.md), [CUA](contracts/cua-v1.md),
[control policy](contracts/control-policy-v1.md) and
[release manifest](contracts/release-manifest-v1.md).

## Contributing and security

Read [CONTRIBUTING.md](CONTRIBUTING.md) before proposing a change; the
engineering contract is in [AGENTS.md](AGENTS.md). Report vulnerabilities
privately as described in [SECURITY.md](SECURITY.md), never in a public issue.

## License

Orbit's code is released under the [MIT License](LICENSE). The Orbit name,
logo, app icon, menu bar icons and assistant avatars are not covered by it.
The bundled `cua-driver` engine has its own license, in
[third-party/cua-driver/LICENSE.md](third-party/cua-driver/LICENSE.md). Each
build writes the licences of the interface's packages and of the Rust crates
compiled into the app to `Orbit.app/Contents/Resources/third-party/NOTICES.txt`
(`scripts/release/third-party-notices.py`).
