# Self-host Orbit

An Orbit installation has a Mac app, an Orbit hub, a Hermes assistant profile,
and a meeting transcription service. The hub owns conversations, meetings,
transcripts and notes. Hermes owns the assistant's models, identity and provider
credentials. `orbit-mcp` is a stdio translator launched by Hermes or by an ACP
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
kind = "acp"                                      # see "Claude Code and Codex agents"
agent = "claude-code"                             # required: "claude-code" or "codex"
command = ["/opt/orbit/agents/node_modules/.bin/claude-agent-acp", "--hide-claude-auth"]
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
or one of its key files is missing, empty or not `0600`, or an `acp` agent's
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
read-aloud; an `acp` agent has no voice. Without one, the hub refuses the
request with `speech_not_configured`, naming the agent and why, and `ready`
does not list `voice` among that agent's capabilities.

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
stdio, not a second HTTP listener. Its seventeen tools are `orbit_devices`,
`orbit_windows`, `orbit_screenshot`, `orbit_look`, `orbit_selected_text`,
`orbit_control_tools`, `orbit_perform_action`, `orbit_action_result`,
`orbit_calendar`, `orbit_meetings`, `orbit_transcript`, `orbit_note`,
`orbit_files_list`, `orbit_files_search`, `orbit_files_read`,
`orbit_attachment` and `orbit_skill`; `orbit_skill` loads an `openai` or
`acp` agent's skills and is not offered to a Hermes agent.
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

An agent of kind `acp` runs Claude Code or Codex through its official Agent
Client Protocol adapter, unmodified, with the account its operator signs in
on the server. Orbit never offers a sign-in, and never stores or sees a token.
Install Node 22 or later and the pinned adapters outside the repository:

```sh
mkdir -p /opt/orbit/agents && cd /opt/orbit/agents
npm install --save-exact @agentclientprotocol/claude-agent-acp@0.94.0 \
  @agentclientprotocol/codex-acp@2.2.2
```

Each adapter brings its agent: `claude-agent-acp` the Claude Agent SDK and its
`claude` binary, `codex-acp` the `@openai/codex` package. Declare each agent in
the hub TOML as in the example under [Several agents](#several-agents), each
with its own copy of the translator configuration. A Codex agent has
`agent = "codex"` and `command = ["/opt/orbit/agents/node_modules/.bin/codex-acp"]`.

`command` starts with the adapter's absolute path; the hub refuses to start
when it is missing or not executable, and refuses a session whose process does
not start, with the path and the cause. `env` names variables passed from the
hub's environment; besides them the agent receives only `HOME`, `LOGNAME`,
`PATH`, `SHELL`, `TERM` and `USER`. `--hide-claude-auth` makes the adapter
refuse any session a claude.ai subscription would pay for: use an API key or a
Console sign-in until Anthropic confirms in writing that a subscription may be
used through Orbit.

Every agent has a private folder, `<data_dir>/agents/<id>` (mode `0700`), which
is its working directory. Claude Code's `CLAUDE_CONFIG_DIR` is its
`claude-config` subfolder and Codex's `CODEX_HOME` its `codex-home`. The hub
declares neither file-system nor terminal access over ACP and refuses such
requests. Because the adapters run their agent's own tools inside their
process, the hub also locks each agent to Orbit's tools:

- Claude Code: `tools: []`, `strictMcpConfig: true` and `settingSources: []`
  in each session's `_meta.claudeCode.options`.
- Codex: `CODEX_CONFIG` with `features.shell_tool = false`,
  `sandbox_mode = "read-only"`, `approval_policy = "never"` and
  `web_search = "disabled"`, plus `INITIAL_AGENT_MODE=read-only`. codex-acp
  sends its mode's sandbox and approval policy with every turn, overriding the
  configuration, so the read-only mode is what keeps the sandbox read-only.
  That mode's approval policy is `on-request`, and its requests reach the Mac.

The agent starts `orbit-mcp` itself, with this hub's Python, `ORBIT_MCP_CONFIG`
set to `orbit_mcp_config` and `ORBIT_MCP_AGENT` to the agent's id. At most four
processes of one agent run at once; an idle session gives its process up and
is reloaded with `session/load` on its next turn.

Sign each agent in on the server, as the hub's user, before its first
conversation. A signed-out agent's refusal names the same command:

```sh
CLAUDE_CONFIG_DIR=/opt/orbit/data/agents/claude/claude-config \
  /opt/orbit/agents/node_modules/.bin/claude-agent-acp --hide-claude-auth \
  --cli auth login --console
CODEX_HOME=/opt/orbit/data/agents/codex/codex-home \
  /opt/orbit/agents/node_modules/.bin/codex-acp cli login --device-auth
```

`--cli` and `cli` run the adapter's bundled `claude` and `codex`. Device-code
sign-in must first be enabled in your ChatGPT security settings. A Claude API
key can reach the agent through `env` instead; Codex stores one with
`printenv OPENAI_API_KEY | CODEX_HOME=… codex-acp cli login --with-api-key`.
Read the terms that apply to your account before connecting it: Anthropic's
[legal and compliance](https://code.claude.com/docs/en/legal-and-compliance)
page for Claude Code and the Agent SDK, and OpenAI's
[terms and policies](https://openai.com/policies) with the
[Codex app-server](https://learn.chatgpt.com/docs/app-server) notes.

Run the agents as a separate system user. The hub's storage key, Hermes
credential and HMAC keys are `0600` files of the hub's user, and an agent
process running as that user could read them. For example, with an
`orbit-agent` user:

- wrap the command: `command = ["/usr/bin/sudo", "-n", "-u", "orbit-agent",
  "--preserve-env=CLAUDE_CONFIG_DIR", "--", "/opt/orbit/agents/…"]` (Codex:
  `--preserve-env=CODEX_HOME,CODEX_CONFIG,INITIAL_AGENT_MODE`, plus any name
  in `env`), allowed in sudoers with `NOPASSWD:SETENV:` for exactly that adapter;
- create `<data_dir>/agents/<id>` and its `claude-config` or `codex-home`
  beforehand, owned by `orbit-agent`, give the hub's user access to the folder
  (for instance with an ACL), and let `orbit-agent` traverse `<data_dir>` and
  `<data_dir>/agents` without listing them (`setfacl -m u:orbit-agent:x`);
- give `orbit-agent` its own `orbit_mcp_config` and copies of the HMAC keys,
  owned by it with mode `0600`, and read access to the hub's installation.

## Personality, skills and access of each agent

In Orbit, Ajustes › Agentes shows each agent's personality, its skills and its
access groups; the routes are in [the hub protocol](../contracts/hub-v1.md#agent-configuration).
What Orbit can change depends on the agent's kind:

- **`openai`**: everything, kept in the hub's database. The personality is
  sent with Orbit's own instructions on every turn; each enabled skill is
  offered by name and description, and the model loads its instructions with
  `orbit-mcp`'s `orbit_skill` tool when it needs them.
- **`acp`**: everything, kept in the hub's database. The agent is told the
  same text as an `openai` agent where its lock-down still reads it, and
  loads a skill with the same `orbit_skill`. Claude Code is told in each
  session's `_meta.systemPrompt.append`, after its own system prompt
  (`settingSources: []` loads no `CLAUDE.md` or `.claude/skills`, and
  `tools: []` leaves no tool to open them). Codex is told in the `AGENTS.md`
  of its folder (`0644`, inside the agent's private folder, so an agent run
  as its own user can read it, replaced whole before each session opens),
  and `CODEX_CONFIG` sets `project_doc_max_bytes` so it reads all of it; no
  skills folder is written, because Codex opens a listed skill with its
  shell, which the lock-down removes. A conversation already open keeps what
  it started with.
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
`ORBIT_MCP_AGENT`: the hub sets it for `openai` and `acp` agents; for a
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

Access groups configure an agent's tools; they are not a security boundary.
Both checks trust the agent id that `orbit-mcp` signs with the bridge's HMAC
key, so a process on the same machine that can read that key (or run
`orbit-mcp` with another `ORBIT_MCP_AGENT`) can use any group. Keep agents
you do not trust away from those keys, as described for `acp` agents above,
and rely on macOS's permissions for what the Mac allows at all.

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
