# Self-host Orbit

An Orbit installation has a Mac app, an Orbit hub, a Hermes assistant profile,
and a meeting transcription service. The hub owns conversations, meetings,
transcripts and notes. Hermes owns the assistant's models, identity and provider
credentials. `orbit-mcp` is a stdio translator launched by Hermes; it reaches
the Mac through the hub, never directly.

All hostnames, paths and model names below are examples. Keep your actual
configuration and secrets outside the repository. The Mac UI is Spanish. The hub selects the assistant name used in the UI and
meeting wake phrase; the default is Deimos.

## Requirements

- A server with Python 3.11 or later and `uv`, durable local storage, and a TLS
  certificate trusted by the Mac. The native app requires HTTPS/WSS.
- A Hermes installation exposing the WebSocket `/api/ws` JSON-RPC interface and
  HTTP audio endpoints `/api/audio/transcribe` and `/api/audio/speak`. Configure
  the profile's models, STT and TTS in Hermes.
- An OpenAI-compatible transcription service accepting
  `POST /v1/audio/transcriptions` with `verbose_json`. Final-pass transcription
  also requires word timestamps and `vad_filter=true` support.
- `ffmpeg` for final-pass audio conversion/windowing. Optional local diarization
  needs a separate Python environment with PyTorch and `pyannote.audio`, plus an
  already downloaded community-1 model bundle; the hub does not download models.
- Optional: an OpenAI-compatible chat completions service supporting
  `response_format: json_schema` for meeting notes.

Hermes and provider compatibility must be checked against your installed
versions. Passing tests against synthetic peers does not establish that a live
Hermes profile, audio device or provider works.

## Environment and configuration

The hub loads TOML with `orbit-hub --config /absolute/path/orbit.toml`. The
translator uses `ORBIT_MCP_CONFIG` to locate its own TOML. These are separate
configuration files: the hub uses `[meetings_bridge]`, the translator `[meetings]`.
Unknown fields and retired configuration sections are rejected.

Infrastructure destinations are explicit configuration:

| Variable | Consumer and purpose |
|---|---|
| `ORBIT_UPDATE_BASE_URL` | Build-time update origin for the Rust app; omit to disable update checks |
| `VITE_ORBIT_HUB_URL` | Build-time suggested pairing origin; omit to leave the entry empty |
| `ORBIT_HUB_ASSISTANT_NAME` | Runtime assistant name override; takes precedence over TOML `assistant_name` |
| `ORBIT_HUB_PROFILE` | Runtime hub Hermes profile override; takes precedence over TOML `profile` |
| `ORBIT_HUB_BASE` | Required by release scripts/unit rendering: absolute directory containing `releases/` and `current` |
| `ORBIT_HUB_URL` | Release-script local health-check origin; defaults to `http://127.0.0.1:8700` |
| `ORBIT_HUB_HERMES` | Hermes CLI path for release scripts; defaults to `~/.local/bin/hermes` |
| `ORBIT_DEIMOS_PROFILE` | Required profile configuration path checked for the translator command |
| `ORBIT_HUB_CONFIG` | Required absolute hub TOML path when rendering the systemd unit |
| `SENTRY_ORG`, `SENTRY_PROJECT` | Required symbol-upload destination when using your own Sentry project |
| `ORBIT_RELEASE_DIR` | Absolute release-artifact root for symbol upload |
| `ORBIT_MCP_CONFIG` | Translator TOML path, supplied in the Hermes MCP entry |

`ORBIT_UPDATE_BASE_URL` and `VITE_ORBIT_HUB_URL` must be present in the build
environment, not just the shell that later launches the app. For example:

```sh
ORBIT_UPDATE_BASE_URL=https://updates.example.test \
VITE_ORBIT_HUB_URL=https://hub.example \
ORBIT_CUA_DRIVER_BINARY=/absolute/path/to/cua-driver \
bash scripts/macos/bundle.sh
```

Omit the two origin variables for a build without automatic updates or a
suggested hub. A saved pairing origin still takes precedence over the suggestion.
`ORBIT_HUB_PROFILE` overrides an explicit TOML profile; without the variable,
TOML selects the profile, defaulting to `default` when omitted. The example below
selects `deimos` explicitly. The profile and assistant name are independent settings.

These variables are not interpolated inside TOML. The examples use explicit
`--config`, `--host` and `--port` CLI options. The systemd template binds loopback
port 8700; choose your own unit if a different port or backend service is needed.
Packaging and signing variables are listed in
[installation](INSTALL.md#build-a-macos-package).

## Install and configure the hub

From the repository root, install the locked server package:

```sh
cd server
uv sync --frozen
```

Choose private directories, for example `/opt/orbit/config`,
`/opt/orbit/secrets` and `/opt/orbit/data`, owned by the service user. Create the
configuration and credential files with mode `0600` and private parent directories
(`0700`). Provision the Hermes session token through your own Hermes deployment;
Orbit neither discovers nor copies it.

A minimal `/opt/orbit/config/orbit.toml`:

```toml
data_dir = "/opt/orbit/data"
key_file = "/opt/orbit/secrets/storage.key"
hermes_url = "http://127.0.0.1:8643"
hermes_credential_file = "/opt/orbit/secrets/hermes-token"
hermes_ws_auth = "token"
profile = "deimos"
assistant_name = "Atlas"
allow_insecure_localhost = true
allowed_hosts = ["localhost", "127.0.0.1", "hub.example"]

[meeting_transcription]
endpoint = "http://127.0.0.1:6394"
model = "openai/whisper-large-v3"
language = "es"
```

Set `assistant_name` at the TOML root, or set `ORBIT_HUB_ASSISTANT_NAME` in the
hub process environment to override it. With neither setting, it is `Deimos`.
The hub refuses an empty or whitespace-only name, control characters, or more
than 48 Unicode characters (including spaces), reporting the cause, length and
ceiling before startup. This keeps the spoken invocation and UI label short;
it does not limit the Hermes profile or prompts.

The Mac learns the name when it connects and retains the last learned name for
offline starts; before its first connection it uses Deimos. Restart the hub
after changing the setting and reconnect the Mac. Meeting activation accepts
“Oye Atlas”, “Hola Atlas” and “Atlas,” at the start of a sentence, ignoring case
and accents. Compound names also work. Activation depends on how macOS
transcribes your chosen name. Only Deimos has the measured recognition alias
“Demos”; custom names do not inherit that alias. The wake phrase requires
macOS 26 and does not change its existing pause or question deadlines.

`allow_insecure_localhost` permits the loopback Hermes URL; it does not permit
plaintext public access. `allowed_hosts` must include the hostname the Mac uses.
The hub does not trust forwarded proxy headers. The transcription table is
required even if you initially use only chat. Its endpoint permits HTTP only
on loopback; use HTTPS elsewhere.

Initialize the encrypted store once, then start the hub explicitly:

```sh
.venv/bin/orbit-hub --config /opt/orbit/config/orbit.toml init
.venv/bin/orbit-hub --config /opt/orbit/config/orbit.toml serve \
  --host 0.0.0.0 --port 8788 \
  --tls-cert /opt/orbit/secrets/cert.pem \
  --tls-key /opt/orbit/secrets/tls.key
```

`init` creates an AES-256 storage key and private SQLite store and refuses to
replace an existing key. Back up the key together with a consistent database
backup. A lost key makes the stored content unreadable.

For a TLS-terminating proxy, bind the hub to `127.0.0.1` and explicitly allow
loopback development HTTP; expose only the HTTPS proxy to Macs. Preserve the
original Host header and WebSocket upgrade. Internal HMAC routes must remain
private to the server. Use a dedicated service user and your own service
supervision; configure all executable, working-directory and TOML paths for that
deployment rather than copying another operator's unit.

For the supplied user-service layout, run this from the repository root to
render the unit with your paths before installing it:

```sh
ORBIT_HUB_BASE=/opt/orbit/app \
ORBIT_HUB_CONFIG=/opt/orbit/config/orbit.toml \
bash scripts/hub/render-unit.sh > /opt/orbit/config/orbit-hub.service
```

The renderer requires absolute paths without spaces or systemd metacharacters.
Review the rendered unit and its Hermes service dependency before installing it.
Release scripts build immutable versions under `ORBIT_HUB_BASE/releases/` and
select one with `current`; they also require `ORBIT_DEIMOS_PROFILE`.
An activation can restart the hub and shared Hermes gateway, so coordinate it
with users. Rendering a unit alone does not install or start services.

`GET /health` checks liveness; `GET /health/deps` reports dependency failures
without credentials. A live HTTP process alone does not verify chat, transcription
or TTS. Errors carry stable codes, operation IDs and relevant counts.

## Transcription and notes

Run the transcription service separately and confirm that its endpoint/model
accept the required formats. Meeting groups use a 300-second request timeout,
a 10-second connection timeout and at most 24 MiB of audio. The hub has one
inference worker; interactive dictation takes priority between meeting groups
without interrupting an inference already running. See
[transcript processing](architecture/transcripts.md).

For final-pass speaker separation add this hub table, adjusting all three paths:

```toml
[diarization]
python = "/opt/orbit/diarization/bin/python"
bundle = "/opt/orbit/models/community-1"
gpu_lock = "/opt/orbit/data/inference.lock"
```

The bundle contains `config.yaml` and its local models. The subprocess runs
offline and returns intervals, not stored speaker embeddings. Share the GPU
lock with the transcription service if they compete for the same device.
Without this table, the draft transcript remains available but the final pass
reports `diarization_not_configured`. Voice names do not prove a person's identity.

For generated notes, add:

```toml
[processing]
endpoint = "https://model.example.test/v1/chat/completions"
model = "example-model"
credential_file = "/opt/orbit/secrets/processing-token"
reasoning = "provider_default"
timeout_seconds = 300
max_completion_tokens = 8192
context_tokens = 131072
```

Choose the model's actual context size. The credential stays on the hub.
Without `[processing]`, transcription still works and notes report
`not_configured`. Optional `[error_reports] dsn = "…"` enables your Sentry
destination; omit the table to send no reports.

## Hermes and orbit-mcp

Install Hermes separately using its upstream installation instructions; it is
not bundled with Orbit. Choose an isolated profile and configure its model,
provider credentials, STT and TTS before connecting the hub. Check your installed
CLI's help before using the backend command used by Orbit's acceptance harness:

```sh
hermes --profile deimos serve --isolated --host 127.0.0.1 --port 8643
```

Provision that backend's session token through Hermes' supported configuration
and place the same token in the hub's private `hermes_credential_file`. The
backend's authentication environment is a Hermes contract, not an Orbit variable;
do not invent a token setting or start an unauthenticated public backend.
Keep the backend supervised separately from the hub.

The hub creates and resumes device-scoped sessions; the Mac cannot supply a
Hermes session ID.
In token mode the WebSocket uses Hermes token authentication and HTTP audio uses
`X-Hermes-Session-Token`. Do not use a one-use WebSocket ticket as an HTTP token.
The current hub accepts only `hermes_ws_auth = "token"`.

Generate **two distinct** 32-byte HMAC keys as the service user, outside Git:

```sh
umask 077
head -c 32 /dev/urandom > /opt/orbit/secrets/device-bridge.key
head -c 32 /dev/urandom > /opt/orbit/secrets/meetings.key
chmod 0600 /opt/orbit/secrets/device-bridge.key /opt/orbit/secrets/meetings.key
```

The key files must be regular files owned by the process user, not symlinks.
Add to the hub TOML:

```toml
[device_bridge]
key_id = "assistant-device"
credential_file = "/opt/orbit/secrets/device-bridge.key"

[meetings_bridge]
key_id = "assistant-meetings"
credential_file = "/opt/orbit/secrets/meetings.key"
```

Create a separate `/opt/orbit/config/mcp.toml`:

```toml
hub_url = "http://127.0.0.1:8788"
default_device = "Example Mac"

[device_bridge]
key_id = "assistant-device"
key_file = "/opt/orbit/secrets/device-bridge.key"

[meetings]
key_id = "assistant-meetings"
key_file = "/opt/orbit/secrets/meetings.key"

[devices]
```

This example assumes the loopback HTTP hub/proxy arrangement described above.
For direct TLS use the matching HTTPS origin instead. Key IDs and key bytes
must match the corresponding hub tables. `[devices]` is required but may be
empty: `orbit_devices` discovers paired names. Its optional entries are fixed
aliases to device IDs. Without `[meetings]` the translator omits meeting tools.
It reloads changed TOML and refreshes device names without a gateway restart.

In your Hermes profile's `mcp_servers` configuration:

```yaml
mcp_servers:
  orbit:
    command: /absolute/path/to/orbit/server/.venv/bin/orbit-mcp
    env:
      ORBIT_MCP_CONFIG: /opt/orbit/config/mcp.toml
```

Use the executable from the hub version you actually run. The translator uses
stdio, not a second HTTP listener. Its twelve tools are `orbit_devices`,
`orbit_windows`, `orbit_screenshot`, `orbit_look`, `orbit_selected_text`,
`orbit_control_tools`, `orbit_perform_action`, `orbit_action_result`,
`orbit_calendar`, `orbit_meetings`, `orbit_transcript` and `orbit_note`.
Validate discovery with an isolated profile first. Apply profile changes using
the commands supported by your installed Hermes version; a gateway restart can
interrupt other profiles and must be coordinated with their users.

## Pair and revoke a Mac

With the hub initialized, run these local administrator commands from `server/`:

```sh
.venv/bin/orbit-hub --config /opt/orbit/config/orbit.toml pair-code
.venv/bin/orbit-hub --config /opt/orbit/config/orbit.toml devices
.venv/bin/orbit-hub --config /opt/orbit/config/orbit.toml revoke DEVICE_ID
```

A code is one-use and expires after 300 seconds by default; `pair-code --ttl`
accepts 30–600 seconds. Enter `https://hub.example:8788` (or your proxy's HTTPS
origin) and the code on the Mac. Network access alone does not pair a device.
The token is revocable and device-scoped; the hub stores its hash, and the Mac
stores the token in Keychain.

Revocation rejects new work/uploads and closes the device's sockets. It requests
interruption of the device's Hermes sessions; it cannot undo effects already
performed. Quitting the Mac app does not itself revoke the token.

Confirm with consented fixtures: pairing, a chat reply, a short recorded meeting,
transcription and note readiness, then the local privacy stop and explicit local
rearm. Computer control follows macOS Accessibility and Screen Recording grants;
there is no second Orbit approval switch.

## Storage and upgrades

The hub keeps encrypted audio, transcripts, messages and notes. There is no
automatic retention/deletion policy. Its preventive admission limit defaults to
10 GiB across SQLite, WAL and SHM; it is not a hard filesystem quota. Reserve
space for safety metadata and monitor free space. Never back up only the main
SQLite file while its WAL is live.

Upgrade the hub before the app and review schema/protocol compatibility before
rolling back. Keep a database/key backup and the previous executable environment.
The [hub contract](../contracts/hub-v1.md) describes wire compatibility; the
[release manifest contract](../contracts/release-manifest-v1.md) describes package
and installer hashes. Hosting updates is optional and independent of running a hub.
