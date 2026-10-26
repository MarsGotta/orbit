# Self-host Orbit

An Orbit installation has a Mac app, an Orbit hub, a Hermes assistant profile,
and a meeting transcription service. The hub owns conversations, meetings,
transcripts and notes. Hermes owns the assistant's models, identity and provider
credentials. `orbit-mcp` is a stdio translator launched by Hermes or by a vendor
agent; it reaches the Mac through the hub, never directly.

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

### Several agents

A hub can offer more than one agent. List them as `[[agents]]` tables after
the root keys; without any, the hub is the single agent `default` made of
`assistant_name` and `profile`, exactly as above. Each agent has an `id`, a
`name` and a `kind`, plus that kind's own keys. One file with an agent of each
kind:

```toml
default_agent = "deimos"   # optional; without it, the first entry

[[agents]]
id = "deimos"              # stable: 1 to 32 characters of a-z, 0-9 and -
name = "Deimos"            # what the Mac shows and what you say: «Oye Deimos»
kind = "hermes"
profile = "deimos"         # required: the Hermes profile that answers

[[agents]]
id = "lyra"
name = "Lyra"
kind = "openai"                                   # see "An assistant without Hermes"
base_url = "http://127.0.0.1:11434/v1"            # required unless provider = "anthropic"
model = "example-model"                           # required
orbit_mcp_config = "/opt/orbit/config/mcp.toml"   # required: orbit-mcp's TOML, absolute
# api_key_file = "/opt/orbit/secrets/lyra.key"    # 0600; required with provider = "anthropic"
# provider = "anthropic"                          # "openai" (the default) or "anthropic"
# images = true                                   # the model reads images; default false
# [agents.speech]                                 # read-aloud; without it, no voice
# base_url = "https://speech.example/v1"          # required in the table: POST {base_url}/audio/speech
# model = "example-tts"                           # required in the table
# voice = "example-voice"                         # required in the table
# api_key_file = "/opt/orbit/secrets/speech.key"  # 0600, optional

[[agents]]
id = "claude"
name = "Claude"
kind = "claude-code"                              # or "codex"
command = ["/opt/orbit/bin/claude"]                 # official executable, absolute
orbit_mcp_config = "/opt/orbit/config/mcp-claude.toml"  # required, absolute
# env = ["ANTHROPIC_API_KEY"]                     # variables passed from the hub's environment
```

Every `hermes` agent is a profile of the same Hermes (`hermes_url`,
`hermes_credential_file`); `orbit-mcp` is configured in each profile as below.
The hub refuses to load the configuration, naming the entry and the cause, when
an id or a name repeats (names compare ignoring case and accents, because they
are spoken), an id is not in that form, a `kind` is unknown, a kind's key is
missing, unknown or invalid (a key of another kind is unknown), `default_agent`
names no agent, or `profile`, `assistant_name`, `ORBIT_HUB_PROFILE` or
`ORBIT_HUB_ASSISTANT_NAME` are set beside `[[agents]]`. Names follow the rules
of `assistant_name`. Loading reads none of the files an entry names; the hub
checks them when it starts and refuses to start, naming the agent, the key, the
path and the cause, when an `openai` agent's `orbit_mcp_config` is not a file
or one of its key files is missing, empty or not `0600`, or a Claude Code or Codex agent's
`command` does not exist or is not executable.

A conversation belongs to the agent it was opened with and keeps it; it stores
the agent's `id`, so renaming an agent keeps its conversations and changing an
`id` leaves them with an agent the hub no longer has, which the Mac is told.
Conversations stored before agents existed, and those opened with the single
agent `default` of a hub without `[[agents]]`, belong to the default agent
(unless an entry keeps the id `default`): keep the old profile's agent first,
or as `default_agent`, with the same `profile`, so they resume where they were
kept.

Dictation and read-aloud use the default agent's voice, because neither audio
request names a conversation: a `hermes` agent's profile; an `openai` agent's
`[meeting_transcription]` for dictation and its `[agents.speech]` for
read-aloud; a Claude Code or Codex agent has no voice. Without one, the hub refuses the
request with `speech_not_configured`, naming the agent and why, and `ready`
does not list `voice` among that agent's capabilities.

### Host tools and working folders

Claude Code assistants expose the SDK's complete `claude_code` tool preset,
including Bash, Read, Edit, Write, Glob and Grep. Orbit keeps `default`
permissions, installs a `PreToolUse` hook that requests permission for commands,
edits and network tools, and sends the decision through `can_use_tool` to the
existing Mac approval cards. Local reads and searches run without a card.
Commands and edits must receive **Permitir** before Orbit allows them;
**No**, interruption and disconnection do not grant access.
Orbit's MCP tools keep their existing access groups and macOS permissions.

An assistant's settings page includes **Carpeta de trabajo en el hub**. This
is an absolute directory on the server, never a Mac folder picker. The hub
normalizes symlinks and `..`, requires an existing directory, and refuses the
data directory, storage key, another assistant's work folder and any ancestor
that would include them. Refusals appear beside the field without discarding
what was typed. Paths are encrypted in SQLite's `agent_workspaces`, beside
personality and skills. Older Macs ignore the additional configuration field;
older hubs leave the field absent. Upgrade the hub first.

The default is each assistant's private directory under
`<data_dir>-workspaces/<agent_id>`: for `data_dir = /opt/orbit/data`, that is
`/opt/orbit/data-workspaces/<agent_id>`. The hub user needs permission to create
this sibling directory. Account homes remain under `data_dir/agents`; work
must be outside that boundary so even the default cannot expose credentials.
Folder changes apply when a conversation opens or resumes; active runners keep
their original folder. The hub validates again when starting a runner.

Claude and Codex run inside a **bubblewrap jail**, including their shell
children, sign-in/out processes and Orbit's MCP translator. Install `bubblewrap`
on the Linux hub. Publication probes namespace creation and refuses to publish
if it fails. Assistant startup also probes its complete mount list and fails
closed with Spanish installation or Ubuntu AppArmor guidance, retaining the
`agent_adapter_missing` code understood by older Macs.

The jail exposes the server root read-only, a fresh `/dev`, a private `/tmp`
(`TMPDIR=/tmp`), and a PID namespace with its own `/proc`: the hub's processes
and their environments are absent. The network stays shared so both vendors
and their commands can reach the network. `/run/systemd/resolve` remains
readable for Ubuntu's `/etc/resolv.conf`; the whole of `/run` is not hidden.

Only the chosen working folder, the assistant's own vendor home under
`data_dir/agents/<id>` and its private `/tmp` are writable. Empty, read-only
mounts hide the hub's data, all configured secret locations (including its
configuration, TLS key, processing and provider keys), other assistants' work
folders and the default workspace tree. The assistant's own working folder
and vendor home are then bound back; its private `bridge` folder is bound
read-only. Existing owner credentials are hidden: `.ssh`, `.gnupg`, `.claude`,
`.claude.json`, `.codex`, `.config/gh`, `.config/gcloud`, `.aws`, `.azure`,
`.kube`, `.docker`, `.netrc`, `.npmrc`, `.pypirc`, `.git-credentials`,
`.password-store` and `.local/share/keyrings`. Files are masked with `/dev/null`.
User-session control (`/run/user/<uid>`), Docker and containerd sockets, and
LXD's `/var/snap/lxd/common/lxd` and `/var/lib/lxd` are hidden too.
A workspace cannot overlap those locations, the installed hub source or venv,
or be the owner's home or an ancestor of it. This prevents writable startup
files from affecting a later process outside the jail.

Commands and edits require a Mac approval card; reads and local searches do
not. **Trabajó…** shows each command and edited filename, including declined
or failed work. Claude's pre-tool hook asks for all tools except an explicit
list of local reads and the session's own to-do list, tasks and subagents; the same rule applies to subagent tools. Orbit MCP
tools retain their existing access groups and macOS permissions rather than
adding duplicate cards. See the [tool classification](architecture/assistant-jail.md).
Codex starts with `approvalPolicy = untrusted`, `sandbox = danger-full-access`
and the user as reviewer. Its own Linux sandbox cannot nest under Ubuntu's
AppArmor bubblewrap profile; the outer jail supplies kernel enforcement.

The exact Codex 0.164.0 source differs from descriptions of older releases:
[`render_decision_for_unmatched_command_for_platform`](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/exec_policy.rs)
prompts for every unmatched command under `UnlessTrusted`. Only an explicit
execution-policy allow rule can skip that prompt; there is no hard-coded safe
command list in this function. [`assess_patch_safety`](https://github.com/openai/codex/blob/rust-v0.164.0/codex-rs/core/src/safety.rs)
returns `AskUser` for every nonempty patch with `UnlessTrusted`. Empty patches
are rejected. Orbit accepts only one-use approval responses, never policy
amendments or session-wide approvals. Vendor approval behavior remains a
vendor contract, tested locally with deterministic protocol doubles.

Each jailed assistant has separate device and meetings bridge keys and a
translator TOML in `data_dir/agents/<id>/bridge`. The hub authenticates the
agent from that key, fills an omitted agent id and refuses another claimed id.
Only that agent's bridge directory is visible in its jail. The operator's
translator configuration and existing shared bridge keys continue to serve
Hermes and operator-configured translators as before.

**Limits:** a script or project modification left in the working folder runs
with the owner's permissions if the owner later runs it outside the jail.
Each assistant can read its own vendor credentials. Other readable files
outside the hidden paths, including project `.env` files, remain readable;
network access can transmit readable content and reach network services.
The jail does not undo an approved action, enforce vendor tool semantics,
or isolate remote services reachable through the shared network. An existing
hard link inside a writable folder to a file outside it shares that file's
inode; path mounts cannot distinguish the two names. Do not
interpret synthetic CLI tests as acceptance with a real account and Mac.

Each agent starts on its own: a slow or broken one holds only the turn that
asked for it, never another agent or another conversation. A start (the
agent's connection, or a session opening or reopening) still running after
10 s is logged once as `agent_start_slow`; one that fails is logged as
`agent_start_failed` with the refusal's code and `task` `agent:<id>`, and one
still running after 300 s is cut and refused as `agent_start_timeout`. Until
a start succeeds again, `ready` and `agents.changed` list the agent with
`backend: {"state": "failed", "code": …, "at_ms": …}`
([the protocol](../contracts/hub-v1.md#assistants-from-orbit)).

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

`init` creates an AES-256 storage key and private SQLite store. It refuses to
replace an existing key, and it refuses a data directory that already holds a
database: a new key beside a database encrypted with another would leave it
with two. The hub likewise refuses to open a database with a key it was not
written with. A lost key makes the stored content unreadable; keep a copy of
it apart from the database copies (see [Backups](#backups)).

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
ORBIT_HUB_ENV_FILE=/opt/orbit/config/orbit-hub.env \
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
stdio, not a second HTTP listener. Its seventeen tools are `orbit_devices`,
`orbit_windows`, `orbit_screenshot`, `orbit_look`, `orbit_selected_text`,
`orbit_control_tools`, `orbit_perform_action`, `orbit_action_result`,
`orbit_calendar`, `orbit_meetings`, `orbit_transcript`, `orbit_note`,
`orbit_files_list`, `orbit_files_search`, `orbit_files_read`,
`orbit_attachment` and `orbit_skill`; `orbit_skill` loads an `openai` or
Claude Code or Codex agent's skills and is not offered to a Hermes agent.
Validate discovery with an isolated profile first. Apply profile changes using
the commands supported by your installed Hermes version; a gateway restart can
interrupt other profiles and must be coordinated with their users.

## An assistant without Hermes (`kind = "openai"`)

An `[[agents]]` entry of `kind = "openai"` runs the assistant inside the hub
against any OpenAI-compatible Chat Completions API (OpenAI, OpenRouter, LiteLLM,
Ollama, vLLM…) or, with `provider = "anthropic"`, Anthropic's own API. Its keys
are in the example under [Several agents](#several-agents). `base_url` is the
API root, as the OpenAI SDK takes it, and must be HTTPS, or HTTP on loopback;
with `provider = "anthropic"` it is optional and `api_key_file` is required.
Keys live only in 0600 files owned by the hub's user; a server that needs no
key (a local Ollama or vLLM) gets a placeholder bearer.

The hub launches `orbit-mcp` from its own environment for each Mac connection,
with `ORBIT_MCP_CONFIG` set to `orbit_mcp_config` and `ORBIT_MCP_AGENT` to the
agent's id; the agent gets Orbit's tools and no shell or server files. The
conversation is the one the hub stores: the model receives its newest messages
up to 65 536 characters, and is told how many older ones were left out.
Dictation uses `[meeting_transcription]`; read-aloud needs `[agents.speech]`.
The hub does not retry provider requests; a failure reaches the Mac with its
cause.

`images = true` says the model reads images: the images attached to a message
(and the pages of a PDF with almost no text) go to it as image parts after the
message's text, and Orbit's screenshots as images in the tools' results; the
agent lists `images` among its capabilities. It is false by default, because
not every OpenAI-compatible server or model takes images; then the message only
names each image and says the agent cannot see it, and a screenshot or an
image read with `orbit_attachment` reaches it as a sentence saying it was left
out. The conversation
the hub keeps for later turns holds a line naming each image's attachment, not
its bytes. A provider that refuses a message with images (HTTP 400, 413, 415
or 422 to its first request) fails the turn with `attachment_refused`, naming
the files.

## Claude Code and Codex agents

Claude and Codex work out of the box on a published Linux x86_64 hub. Add
one from Orbit and sign in with its official account flow; no
`[assistant_tools]` table, manual CLI installation or MCP file is required.

- `kind = "claude-code"` runs Anthropic's official Python Claude Agent SDK
  (`claude-agent-sdk==0.2.174`, pinned by `server/uv.lock`). Its wheel bundles
  Claude Code **2.1.310**. The hub locates `_bundled/claude` from the installed
  Python package, without searching `PATH` or assuming a venv location.
- `kind = "codex"` runs the official **codex-cli 0.164.0** `app-server` over
  stdio JSON-RPC v2. `scripts/hub/publish.sh` downloads OpenAI's
  [`codex-package-x86_64-unknown-linux-musl.tar.gz`](https://github.com/openai/codex/releases/tag/rust-v0.164.0)
  and verifies SHA-256
  `3ec55053109608d5038f7e50750f7d25f768dfd740fef071aa710e2559b63aac`.
  It preserves the complete official layout beside `.venv`: `bin/codex`,
  `bin/codex-code-mode-host`, `codex-path/rg`, `codex-package.json` and
  `codex-resources/` (bubblewrap and the voice runtime, including licenses).
  All 43 regular files are required by the pinned inventory in
  `server/codex-release.toml`; missing, extra, duplicate or linked file members
  fail publication. The standalone app-server package lacks `codex login`,
  which Orbit also needs, so the complete CLI package supplies both login and
  the `app-server` subcommand. Upstream discovers siblings and resources
  relative to `bin/codex`. The jail's read-only root already exposes the whole
  installed release; no writable bind or additional host path is needed.

The Linux x86_64 Claude SDK wheel's SHA-256 is
`728592effdfe63e96fbad7ba3b2ddf642e3aad344e73692b429898fbfcb33f12`
(verified by the lockfile installer). Its bundled CLI's SHA-256 is
`b73676430af12f97316a798b8371f81d640f90a58bdcb8f3ce56a835bb47df56`.
Other SDK platforms have their own wheel hashes in the lockfile.

Publishing fails on a Codex checksum or inventory mismatch, unsupported server platform or
failed isolated `--version` check, removes the incomplete release, and starts
no service or account session. The sealed `RELEASE` records the Codex version
and asset checksum. The Codex protocol fixtures were generated with 0.164.0;
update those fixtures and run real account acceptance when changing the pin.
No Node adapter is involved.

The hub applies a reviewed Codex feature policy at process startup **and at
every thread start/resume**. It disables account apps/connectors, plugins
(including an existing cache), browser/computer tools, hooks, dependency
installation, tool suggestions, delegation, image generation, automatic
approval reviewers, daemon auto-start and unbounded connection retries.
Core shell/unified execution, `apply_patch`, `view_image` and Orbit's MCP remain.
Intrinsic vendor bookkeeping and MCP tool discovery remain; discovery sees
only Orbit's server. No browser or desktop-control tool reaches the Mac except
through Orbit's own permissioned MCP tools. See the
[complete flag decisions and upstream exceptions](architecture/hub-tool-policy.md).

Every ancestor of the selected working folder has an invocation-local
`projects.<path>.trust_level = "untrusted"` override. The pinned loader checks
these entries before repository trust fallback, so old trusted repository
entries cannot activate parent/project `.codex/config.toml`, hooks or exec
policies. The hub does not write trust to the assistant's account configuration.
Hooks are also feature-disabled, and the legacy `notify` command is empty.

Codex may retry only within its bounded upstream retry policy (four HTTP
attempts, five stream reconnections with the pinned defaults). Codex and Claude
turns have no hub inactivity or total-duration deadline: reasoning can take
minutes without emitting a visible event. Startup and protocol request
acknowledgements keep their separate timeouts.
Terminal provider failures preserve their structured reason and HTTP status,
without returning raw provider diagnostics.

Claude has no claude.ai account connectors: the hub forces
`ENABLE_CLAUDEAI_MCP_SERVERS=false` and the CLI setting
`disableClaudeAiConnectors=true`. Empty `setting_sources` excludes user,
project and local settings, including their hooks and plugin enablement.
`plugins=[]` supplies no explicit plugins; ambient plugin-directory variables
are cleared. `strict_mcp_config=True` restricts MCP to the explicitly supplied
Orbit server. The hub's own SDK `PreToolUse` permission hook remains active.
Subscription OAuth remains available; `--bare` would disable it and is not used.
Managed machine policy still belongs to the host administrator and must be
reviewed on the real hub; synthetic tests do not establish account inventory
or organization policy.

Anthropic supports the Agent SDK with [your own Claude plan](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan).
Orbit does not read subscription tokens or implement provider authentication.
The official SDK and CLI alone use the signed-in Claude account; Codex alone
uses its ChatGPT account. The existing generic `openai` backend remains for
operators who explicitly configure a provider API and API key.

Each agent's private working directory is `<data_dir>/agents/<id>` (mode
`0700`). Claude's `CLAUDE_CONFIG_DIR` is its `claude-config` subfolder;
Codex's `CODEX_HOME` is its `codex-home` subfolder. These paths are unchanged,
so accounts already signed in there remain available. `command` is a list
starting with the official executable's absolute path (or an operator's
executable wrapper), followed by any fixed arguments. Missing or non-executable
tools are refused with the service and path. `env` still explicitly names
variables passed from the hub's environment; otherwise only basic process
variables such as `HOME` and `PATH` are inherited. Do not pass API credentials
when the agent should use its subscription.

Personality and enabled skills reach Claude as the SDK's appended system
prompt on every turn and Codex as `developerInstructions` when starting or
resuming a thread. Skills are loaded through `orbit_skill`. Neither backend
writes personality files into the account directory. An existing Codex
`AGENTS.md` from an older hub is no longer needed and can be removed by the
operator after upgrading.

Both vendors launch this hub's `orbit-mcp` over stdio inside their bubblewrap
jail. `ORBIT_MCP_CONFIG` names their own read-only translator TOML under
`data_dir/agents/<id>/bridge`, and `ORBIT_MCP_AGENT` selects that same agent.
The hub verifies the selection against the key's identity. Claude exposes its
native tools with no user/project settings and strict MCP configuration.
Codex exposes its native shell and edits, uses `untrusted` approvals and the
outer jail instead of its own nested sandbox. Approval requests use the
existing `once`/`deny` flow; questions use the existing clarification flow.
See [the jail and its limits](#host-tools-and-working-folders).

At most four runners per agent are held at once; an idle runner can be evicted.
Claude uses the SDK's `session_id` for a new conversation and `resume` for
subsequent turns. Codex uses `thread/start`, saves its thread id, and uses
`thread/resume` after reconnecting. Text, tool activity and reasoning map to
the existing hub events. Stop uses the SDK's `interrupt` or Codex's
`turn/interrupt`. The hub never repeats an uncertain action. A plan-limit
refusal keeps `agent_limit_reached`, with `resets_at_ms` when supplied; Orbit
saves the question and offers to pass it to the main assistant as before.

Sign-in remains the vendor's own command, in the same private directories:

- Claude: `<command> auth login --claudeai`. Orbit opens the address shown,
  then hands the browser's pasted code back to the CLI.
- Codex: `<command> login --device-auth`. Orbit shows the address and one-time
  code. Device sign-in must be enabled in the ChatGPT account. The CLI runs
  in a temporary home; only a successful sign-in's `auth.json` is moved whole
  into `codex-home`. Failure or cancellation preserves the earlier account.

Sign-in is stopped after 15 minutes. Sign-out runs `<command> auth logout`
or `<command> logout`. The hub never reads credentials, and tool output is
not logged or forwarded beyond the sign-in address and code.

## Assistants added from Orbit

Orbit can add a Claude Code or Codex agent without restarting the hub.
`[assistant_tools]` is optional and only overrides what the hub supplies.
Existing `command`, `env` and `orbit_mcp_config` keys keep working:

```toml
# Optional: use a different official binary or wrapper.
[assistant_tools.claude-code]
command = ["/opt/orbit/bin/claude"]
# orbit_mcp_config = "/opt/orbit/config/mcp-added.toml"  # optional, absolute
# env = ["LANG"]                                    # optional allowlist

[assistant_tools.codex]
command = ["/opt/orbit/bin/codex"]
```

`orbit-hub serve` derives `<data_dir>/assistant-mcp.toml` (mode `0600`) from
its actual listener, including its port. A wildcard TLS listener uses the
first concrete non-loopback `allowed_hosts` entry as its HTTPS hostname;
that name must resolve to this listener and match its trusted certificate.
An explicit `orbit_mcp_config` can select a different reachable origin.
Embedders calling `create_app` directly must call
`assistant.local_mcp.prepare(settings, local_origin)` before composition.
The generated file points to the hub's existing device/meeting bridge key
files. If a bridge is not configured, the hub creates and reuses a private
32-byte key in `<data_dir>/assistant-tools/` (directory `0700`, keys `0600`)
and enables that bridge with the same key. It never overwrites an existing
invalid key. No credentials are written into the release or sent to the Mac.
The translator discovers connected Macs through `orbit_devices`; aliases
need no manual configuration. An operator's MCP file is never modified.

A missing or unusable official executable keeps `agent_adapter_missing` for
existing Macs and carries the service, exact path and cause. Its Spanish
message explains that publishing the hub again installs the missing tool.
Hermes assistants are created on the hub from a Hermes profile and then
appear in Orbit; they are added there, rather than through Orbit. `openai`
agents also remain explicitly configured. The Mac chooses only the service,
profile and access groups; it supplies no command, path or agent id. Added
agents are stored beside configured agents and use the defaults or overrides
above. Removing one stops its processes and deletes its private folder and
credentials, while preserving its conversations.

A hub offers at most 32 configured and added agents together. Removing a
service override returns its existing agents to the shipped tool on restart.
Upgrade the hub before Macs;
no Mac update is required for the replacement backends. Existing manually
configured vendor entries must change their kind to `claude-code` or `codex`,
remove `agent`, and point `command` at the official executable. Keep their ids
and account directories. Old adapter-created conversation ids may not be
vendor session/thread ids: their hub history remains readable, but a vendor
that cannot resume one refuses it rather than starting an empty conversation.

**Profiles.** Orbit keeps each agent's name, description, colour and which
one is the primary in the hub's database, over `orbit.toml`'s `name` and
`default_agent`, so every Mac sees the same. The primary answers when no
agent is named. Renaming an agent changes how Orbit shows it and the name the
wake phrase listens for, nothing of the agent: a Hermes profile is not
touched. `default_agent` still owns conversations stored before agents
existed, speaks for an `orbit-mcp` that names no agent and lends its voice
to dictation, whichever agent is primary.

**What the hub publishes of each account** (`account` in
[the protocol](../contracts/hub-v1.md#assistants-from-orbit)): signed in,
signed out, or expired when a turn refuses for want of sign-in. Codex's
`account/read` supplies only account type, email and plan; the hub never asks
for tokens. Claude's SDK does not supply account email/plan, so Orbit omits
those labels unless already known. Both vendors report usage windows and
reset times when available. A refusal, rather than an inferred percentage,
marks the account as at its plan limit.

## Personality, skills and access of each agent

In Orbit, Ajustes › Agentes shows each agent's personality, its skills and its
access groups; the routes are in [the hub protocol](../contracts/hub-v1.md#agent-configuration).
What Orbit can change depends on the agent's kind:

- **`openai`**: everything, kept in the hub's database. The personality is
  sent with Orbit's own instructions on every turn; each enabled skill is
  offered by name and description, and the model loads its instructions with
  `orbit-mcp`'s `orbit_skill` tool when it needs them.
- **`claude-code` and `codex`**: everything, kept in the hub's database.
  Claude receives the text through the SDK's appended system prompt on every
  turn; Codex receives it as `developerInstructions` when opening or resuming
  its thread. Both load enabled skills through `orbit_skill`. No skills folder
  is written, and Claude loads no user or project settings.

- **`hermes`**: the profile's own `SOUL.md` and skills, through Hermes's
  panel API at `hermes_url` with the same `hermes_credential_file`
  (`GET`/`PUT /api/profiles/<profile>/soul`, `/api/skills`,
  `/api/skills/content`, `/api/skills/toggle`). Hermes gives those routes no
  version, so check them against your installed Hermes before upgrading
  either; a route Hermes refuses reaches the Mac as
  `assistant_admin_refused` with the route and Hermes's HTTP status. Hermes's
  panel cannot delete a skill: disable it in Orbit or delete it on the
  server.

Skills follow the [agentskills.io](https://agentskills.io) `SKILL.md` format:
a frontmatter with `name` (lowercase `a-z`, digits and single hyphens, at
most 64 characters, the same as the skill's name) and `description` (at most
1024 characters). A personality is at most 16 KiB, a `SKILL.md` 64 KiB, and
an agent has at most 128 skills.

**Access groups** — Pantalla (`screen`), Control del Mac (`control`),
Calendario (`calendar`), Reuniones y notas (`meetings`) and Archivos
(`files`) — choose which of `orbit-mcp`'s tools each agent has. All are on
until someone turns one off. `orbit-mcp` knows its agent from
`ORBIT_MCP_AGENT`: the hub sets it for `openai` and Claude Code or Codex agents; for a
`hermes` agent that is not the default, set it in that profile's
`mcp_servers` entry, beside `ORBIT_MCP_CONFIG`:

```yaml
mcp_servers:
  orbit:
    command: /absolute/path/to/orbit/server/.venv/bin/orbit-mcp
    env:
      ORBIT_MCP_CONFIG: /opt/orbit/config/mcp.toml
      ORBIT_MCP_AGENT: atlas
```

Without it, `orbit-mcp` speaks for the default agent. A group that is off is
neither listed nor run by `orbit-mcp`, and the hub refuses its bridge
requests with `agent_access_disabled`, the agent and the group.
`orbit_skill` belongs to no group: it reads the agent's own skills.

Jailed Claude and Codex translators have per-agent HMAC credentials: the hub
derives their identity from the key and refuses another claimed id with
`agent_identity_mismatch`. Their keys cannot select the Hermes agent. The
operator's existing bridge keys retain their agent-selection behavior for
Hermes and operator-configured translators; any process with one of those
keys can still select any agent. Those keys remain hidden from jailed
assistants. macOS permissions decide what the Mac allows.

## Short action replies

In `[device_bridge]`, `action_inline_wait_seconds` defaults to `2.5` (range
`0`–`30`). It bounds only how long `orbit_perform_action` waits for the existing
receipt worker. A completed action returns the same outcome as
`orbit_action_result`; a slower action returns `in_progress` and its operation
id. `0` keeps immediate pending replies. This does not change the 30-second
receipt deadline, repeat an input action, or remove its durable result.

Read-aloud is queued by the daemon in sentence parts, with a 240-character
ceiling per synthesis request and smaller parts when the native decoder reports
`clip_too_long`. The helper still enforces 60 seconds per decoded clip. Only a
`played` receipt advances the queue. A call's detected speech onset pauses the
current clip, including one still being fetched; words cancel the whole queue,
and an empty transcription resumes it at the paused position.

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

## Storage

The hub keeps encrypted audio, transcripts, messages and notes in one SQLite
database (`hub.sqlite3` in `data_dir`, with its WAL). Nothing is ever deleted
on its own: when the hub has no room left it refuses new content with
`507`, and the Mac keeps its copy of the audio until the hub takes it.

How much the hub admits is a setting. Without `storage_quota_bytes`, the disk
sets the limit: the database may grow to half of what it and the free space
on its disk add up to, once 5 GiB are left free. The other half is room for
one whole copy of the database beside it, which every activation takes (see
[Upgrades](#upgrades)); the 5 GiB are for the rest of the server, which keeps
writing logs and updates after the hub stops admitting audio (journald alone
keeps up to 4 GiB by default). With 168 GB free and a 1.8 GB database that is
a limit of about 82 GB, some 120 hours of two-track meetings (0.69 GB an
hour). To cap it lower, set a number of bytes at the TOML root:

```toml
storage_quota_bytes = 21474836480   # 20 GiB
```

The disk still applies when it holds less than the quota. Restart the hub
after changing it. The limit is admission, not a filesystem quota: SQLite's
own growth is estimated, so leave the disk some room beyond it.

The hub tells each Mac what it uses, its limit and what is left whenever the
Mac connects, and again when it becomes nearly full (80 % of the limit) or
stops being. The Mac shows «El hub está casi lleno» with those numbers and
what to do. A full disk the hub did not foresee is refused as `507
storage_full` with the free space, never as an internal error.

To see the numbers on the server, read them from a Mac's `ready` or look at
the files: `du -ch /opt/orbit/data/hub.sqlite3*` and `df -h /opt/orbit/data`.

## Backups

The Mac deletes its audio once the hub confirms it, so without copies the
hub's disk holds the only one. There are two kinds of copy, both taken with
SQLite's online backup API (consistent with the WAL while the hub runs):

- **Every night, off the server.** `orbit-hub backup <directory>` writes
  `daily/hub-YYYY-MM-DD.sqlite3` and the first copy of each ISO week to
  `weekly/hub-YYYY-Www.sqlite3`, and keeps the last 7 daily and 4 weekly
  copies. It removes only copies it named, and it refuses a directory that
  does not exist or that is on the same disk as `data_dir`: that is what a
  mount point looks like when the other machine's disk is not mounted, and a
  copy there would not survive the disk it is meant to replace. The
  destination needs room for eleven copies of the database: the audio is
  encrypted and does not compress. A meeting deleted from Orbit («Borrar
  reunión», «Terminar sin guardar») stays in the copies taken before the
  delete until they leave that retention of 7 daily and 4 weekly copies.
- **Before each activation, on the server.** `scripts/hub/activate.sh` copies
  the database and its key into `ORBIT_HUB_BASE/backups/activation` before it
  moves anything (see [Upgrades](#upgrades)).

**The key never travels with the nightly copies.** Whoever reads a copy and
its key reads everything. Keep the key (`key_file`, 32 bytes) somewhere else
once, for example in a password manager as base64:

```sh
base64 < /opt/orbit/secrets/storage.key    # store this line apart from the copies
```

It never changes while the data directory lives, so one copy of it is enough.

### Install the nightly copy

Choose the destination: a directory on another machine's disk mounted on the
server (for example over NFS or SSHFS at `/mnt/orbit-copies`), writable by the
hub's user. Then render and install the service and its timer next to
`orbit-hub.service`:

```sh
ORBIT_HUB_BASE=/opt/orbit/app \
ORBIT_HUB_CONFIG=/opt/orbit/config/orbit.toml \
ORBIT_HUB_ENV_FILE=/opt/orbit/config/orbit-hub.env \
ORBIT_HUB_BACKUP_DIR=/mnt/orbit-copies \
bash scripts/hub/render-unit.sh orbit-hub-backup.service > ~/.config/systemd/user/orbit-hub-backup.service
bash scripts/hub/render-unit.sh orbit-hub-backup.timer > ~/.config/systemd/user/orbit-hub-backup.timer
systemctl --user daemon-reload
systemctl --user enable --now orbit-hub-backup.timer
```

The timer runs at 03:30 (within 15 minutes), and a night the server was off
is copied when it is back. The copy runs at low CPU and I/O priority.

### Check it

```sh
systemctl --user list-timers orbit-hub-backup.timer   # the next and the last run
systemctl --user status orbit-hub-backup.service      # the last result
journalctl --user -u orbit-hub-backup.service -n 20   # what it copied and removed
ls -l /mnt/orbit-copies/daily /mnt/orbit-copies/weekly
```

A failed night leaves the service `failed` with its cause in the journal
(the destination missing or on the server's disk, no room there). Run the
copy by hand to try it: `orbit-hub --config … backup /mnt/orbit-copies`.

### Restore

On the machine that will run the hub, with the hub stopped and an empty (or
missing) `data_dir`:

1. Put the key back where `key_file` points, from where you kept it apart:
   `base64 -d > /opt/orbit/secrets/storage.key` (paste the line), then
   `chmod 600 /opt/orbit/secrets/storage.key`.
2. Restore a copy:
   `orbit-hub --config /opt/orbit/config/orbit.toml restore /mnt/orbit-copies/daily/hub-2026-10-22.sqlite3`.
   It refuses a data directory that already holds a database, and a key the
   copy was not written with (and then leaves the data directory empty).
3. Start the hub. The Macs reconnect with their own pairing; meetings
   recorded after the copy are lost on the hub unless a Mac still holds them.

Do not copy `hub.sqlite3` by hand while the hub runs: without its WAL the
copy misses the latest writes or is inconsistent.

## Upgrades

Upgrade the hub before the app. A new release migrates the database when it
first starts, so `activate.sh <version>` first takes a consistent copy of the
database and its key, as the release it leaves has them, into
`ORBIT_HUB_BASE/backups/activation` (with a `FROM` file naming that release),
replacing the copy of the activation before. If it cannot, it stops before
moving anything. `activate.sh` runs that copy with the server's `python3`
(3.11 or newer).

Going back to the release that copy was taken from (`activate.sh anterior`,
or its number) checks whether the database's schema changed since. If it
did, it stops the hub, moves the database aside to
`ORBIT_HUB_BASE/backups/set-aside-<time>` and puts the copy back, so the older
code never runs on a database migrated past it. What was written after the
copy is in the database set aside, not in the one restored; nothing is
deleted. If the schema is the same, the database stays as it is and keeps
everything. Re-activating the active release takes no copy.

The [hub contract](../contracts/hub-v1.md) describes wire compatibility; the
[release manifest contract](../contracts/release-manifest-v1.md) describes package
and installer hashes. Hosting updates is optional and independent of running a hub.
