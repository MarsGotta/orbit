# Orbit Hub

Private bridge between Orbit devices and an existing assistant profile in Hermes.
Importing it does not install another agent, change the profile or start services.

The hub keeps no calendar copy. The assistant reads it directly from
the Mac through the device bridge's `calendar_read` capability; macOS Calendar
permission is the sole authority.

On 2026-10-15 the hub removed people search, voiceprints, proposals
and their distillation, publications and “activity”, diarization, live voice,
the vault, search, memory and preparation for a meeting series, and assistant
notifications when transcription finishes. Their SQLite tables remain in place,
without being read or deleted. Remove retired configuration tables before
starting the new hub: it rejects unknown sections and keys. See the
[self-hosting guide](../docs/self-hosting.md) for current configuration.
Speaker separation returned in 0.8.0 in a different form: local, after each
meeting, without storing voiceprints.

From `server/`, prepare the environment with `uv sync --frozen` (for tests,
`uv sync --frozen --group dev`). Deployments use a published version through
`scripts/hub/publish.sh` rather than this development environment; see
[self-hosting](../docs/self-hosting.md#install-and-configure-the-hub).
Copy `config.example.toml` to a private path and adjust the server destinations.
The example paths and Hermes port are illustrative, not autodetected.

```sh
uv run orbit-hub --config /path/to/orbit.toml init
uv run orbit-hub --config /path/to/orbit.toml pair-code
uv run orbit-hub --config /path/to/orbit.toml serve --host 0.0.0.0 --tls-cert /path/to/cert.pem --tls-key /path/to/key.pem
```

`init` creates an AES-256 key with mode 0600 and a private SQLite database.
It is an explicit one-time operation: it never replaces an existing key.
The administrator must provision the Hermes backend credential in the mode-0600
file specified by `hermes_credential_file`; it is not obtained or copied automatically.
The Mac receives only its revocable device token.

The pairing code expires after five minutes, allows one use and appears only
in administrator output. Tokens and codes are stored as hashes of random secrets.
Names, messages, receipts, metadata, audio and transcripts use AES-GCM with
authenticated context; there is no plaintext fallback.

```sh
uv run orbit-hub --config /path/to/orbit.toml devices
uv run orbit-hub --config /path/to/orbit.toml revoke DEVICE_ID
```

Revocation is checked every second by a global monitor and before accepting
new work, including when the device is already disconnected. The hub requests
interruption of that device's sessions with a bounded deadline and closes its
Hermes connection. This neither confirms nor undoes actions already performed.
Quitting Orbit on the Mac does not trigger this interruption. Administrative
commands are local; there is no remote administration API. Only one process can
own the database runtime. The local development port requires explicit configuration;
the Mac uses HTTPS/WSS with valid certificates. Tailscale does not replace
pairing. Forwarded proxy headers are not trusted.

The contract is in `../contracts/hub-v1.md`. Each turn's events carry the
`chat.send` UUID, including deltas, the final response and questions.
`chat.completed` includes `text` and `status`; an error response does not imply
success. Text from a failed completion is not stored as an authoritative history
response. Approval decisions only allow once or deny.

`chat.interrupt` also requires the original send's `turn_id` UUID: a late repeat
cannot act on a later turn. Denial uses the same fence. Explicit stop and denial
have independent durable encrypted receipts outside the content quota, limited
to 128 live targets per device. Repeating a target does not duplicate the RPC;
uncertain responses are not retried automatically. Receipts for resolved requests
are retired when another control is admitted: they do not exhaust capacity during
a long turn. A retired stale target still cannot be applied. This does not overcome
physical ENOSPC: deployments must reserve room for safety metadata.

A reconnection to a live runtime receives `state.snapshot` with its revision,
sessions, turns and pending requests for that device. Individual events precede
the updated snapshot. `expires_at:null` means Hermes supplied no verifiable
deadline, not indefinite authorization. Inactive conversations resume from their
stored Hermes identity when the next message arrives. In-flight turns with
uncertain outcomes are not resent after losing the runtime.
Orbit waits for Hermes to finish loading history before sending a new message.
If loading fails, is not confirmed or times out, it rejects the send without
sending the message and explains the cause; another send can retry resumption.
Orbit resumes Hermes with `defer_history=true` and no transcript in the response:
the response's 4 MiB limit does not truncate the model's history. If the active
segment exceeds `sessions.max_resume_messages` (20,000 by default), Hermes rejects
resumption with the count and limit. Its later trimming affects the display copy
of history, not the active segment the model receives.
An approval only confirms application if Hermes responds `resolved:1`; zero means
expired, and an ambiguous contract is not reported as success. Clarifications
respect their `status` and remaining-question list separately.

Known terminal Hermes errors end the turn without promoting partial text.
Other errors only end it if `session.status` establishes that the agent is no
longer running. That query is read-only: it does not resume or replay.
If reconciliation fails, a warning is retained and the user can interrupt.

The hub is the sole source of conversations. `GET /v1/sessions` lists
those belonging to the device, newest first, with
`{id, created_at_ms, last_at_ms, message_count}` and `next` for the next page.
`GET /v1/sessions/{id}/messages?before=&limit=` returns an ordered page of messages,
`{id, role, text, at_ms, kind, duration_ms?, client_id?}`, `next` for earlier
messages and `total` for the count. It first queries IDs and sizes in SQL and
only decrypts what it returns: up to 200 messages and 512 KiB of JSON, without
truncating any message. A message that cannot fit on a page by itself is rejected
with `session_message_too_large`, its size and the ceiling. A conversation belonging
to another device is `session_not_found`.

Each message stores when it was written, whether it was a voice note (with its
duration) and the identifier assigned by the Mac. A meeting question (`chat.send`
with `meeting`) is built on the hub using its transcript: the user row stores
only the question and the Spanish marker “[Durante «título» · últimos N min]”.

Meeting chunks must be self-contained PCM16 WAV files. IDs allow 121 ASCII
characters from letters, digits, `_`, `.`, `:`, `-`; they are not paths.
The recording has a UUID. Uploads allow up to 8 MiB per chunk, and the ACK is
emitted after a SQLite FULL commit, with encrypted audio and a durable STT job.
Optional clock headers must all be present and match the WAV:
`X-Orbit-Started-At-Ms`, `X-Orbit-Monotonic-Ns`, `X-Orbit-First-Sample`,
`X-Orbit-Frames`, `X-Orbit-Sample-Rate`, `X-Orbit-Channels`.
Repeating an ID with different bytes or metadata causes a conflict.
`X-Orbit-Capture-Epoch-Id` is an optional UUID that requires this complete
metadata. It is retained encrypted in each source and group; different epochs
and legacy audio without an epoch are not mixed. Earlier receipts are not rewritten.

The queue shares a single Whisper inference worker. It prioritizes interactive
voice between meeting groups; it does not interrupt an inference already running.
It retains jobs and retries with increasing backoff, up to five attempts, and
recovers interrupted jobs on restart. A revoked credential does not schedule new
inferences. Each confirmed transcript is immutable and encrypted.

The v2 pipeline groups about 20 contiguous seconds from a single track while
retaining the original one-second chunks. It closes partial windows on finalization,
on a discontinuity or 30 seconds after the first receipt; each group is limited
to 128 sources and 20 MiB. Text carries provenance for the whole group.
An upload sequence gap waits for the same deadline or closure: out-of-order
arrival does not prove a capture gap. Adjacent sources with incompatible epochs,
formats, samples or clocks do close the group. It never crosses a gap or reopens
a sealed group for a late source.
Word or chunk times are not fabricated. Groups are immutable and do not overlap:
the planned temporal overlap/deduplication is not yet implemented, and ASR quality
has not been validated with real meetings. Earlier transcripts remain as v1 groups
with one source, without requesting ASR again.

`GET /v1/recordings/{uuid}/transcript`, paginated `/archive` and
`/live-transcript` were removed: the Mac no longer copies meetings,
and these routes return 404; the assistant reads transcripts through internal routes.
`POST /v1/recordings/{uuid}/finalize` verifies a closure manifest against durable
tracks, sequences and frames. The payload contains
`tracks:[{track,chunk_count,last_sequence,total_frames}]`, `gaps`,
`started_at_ms` and `finalized_at_ms`, and optionally `title`, `kind`
(`meeting` or `voice_call`) and `event`. An identical repeat is idempotent;
a different manifest or subsequent new chunks cause a conflict.
`capture_finalized` means verified closure; `capture_complete` requires both
tracks without discontinuities, and `transcript_ready` requires all STT jobs to
be confirmed. Closing a meeting requests its note. External notes are neither
published nor modified.

The Mac reads its meetings from the hub (`GET /v1/recordings`,
each document and its turns) and stores the owner's name, voice names and private
notes there; the WebSocket notifies it with `recording.changed`.
Turns are computed once in `orbit_hub/meetings/turns.py`, for the Mac,
the assistant and the note.

The `/v1/recordings/{uuid}/context` routes were removed along with their sole
consumer, a development command; they return 404.
Contract: `../contracts/hub-v1.md`.

Keep a consistent SQLite backup together with its key. Without the key,
recordings cannot be read; a new key is not silently generated as recovery.
Data must stay out of Git. There is no automatic audio/transcript deletion policy yet.
Before admitting new names, sessions, messages, operations, audio, transcripts or
manifests, the quota adds the observed sizes of SQLite, WAL and SHM, plus a
conservative reserve of three times the payload and 256 KiB for growth.
Existing content and identical retries are not deleted. Revocation and state
changes to existing jobs remain allowed to stop activity even at the limit.

This is a preventive check, not a hard physical quota: pages, indexes,
checkpoints and control updates can add bytes. A strict physical ceiling requires
an administrator-provisioned filesystem quota with sufficient room for that
metadata. Monitor space and export before designing an authorized retention
policy; the hub does not delete or rotate data.

## `orbit-mcp`: Orbit for the assistant

`orbit_mcp` is the translator through which the assistant reaches Orbit. Hermes
launches it over `stdio` on the hub's machine (this package's `orbit-mcp` command),
and the translator signs each request to the hub's private routes with its keys:
Hermes does not see them. Configure the profile to launch it from the active
hub version.

Tools: `orbit_devices`, `orbit_windows`, `orbit_screenshot`,
`orbit_look`, `orbit_selected_text`, `orbit_control_tools`,
`orbit_perform_action`, `orbit_action_result`, `orbit_calendar`,
`orbit_meetings`, `orbit_transcript` and `orbit_note`.

See [Hermes and orbit-mcp setup](../docs/self-hosting.md#hermes-and-orbit-mcp)
and [hub deployment](../docs/self-hosting.md#install-and-configure-the-hub).

## Failures, `op` and Sentry

Every failure goes through `orbit_hub.log.report`: a content-free JSON line with
`event`, `op`, route template, `code`, status and numbers, plus a Sentry event
if `orbit.toml` has `[error_reports] dsn`. An unexpected route exception returns
`500 internal_error`; failed background tasks are reported the same way.
The `op` arrives in `X-Orbit-Op` or the WebSocket frame's `op` field (falling back
to its `id`); if invalid (8 to 64 characters from `[A-Za-z0-9-]`), another is
generated, and every error response returns it.
`GET /health/deps` reports which dependencies do not respond, without credentials.
See [self-hosting](../docs/self-hosting.md) for reporting and deployment setup.

## Validation and limits

The gate (`scripts/check.sh hub`, from the root) runs these checks, including e2e:

```sh
uv run --frozen ruff check orbit_hub orbit_mcp tests ../scripts/e2e-mac ../scripts/tests/mac_harness_dry_run_test.py
uv run --frozen ruff format --check orbit_hub orbit_mcp tests ../scripts/e2e-mac ../scripts/tests/mac_harness_dry_run_test.py
uv run --frozen pytest -q
uv run --frozen pytest -q -c pyproject.toml --rootdir . ../scripts/tests/mac_harness_dry_run_test.py
```

The e2e tests also exercise the real Rust daemon. Build its binary from the root
with `cargo build -p orbit-daemon --bin orbit --locked` and pass it to pytest
from `server/`:

```sh
ORBIT_TEST_BINARY=../target/debug/orbit uv run --frozen pytest -q tests/test_client_e2e.py
```

This test uses real loopback networking, a temporary `--dev` device and synthetic
Hermes/native peers. Without `ORBIT_TEST_BINARY`, it is skipped, and the gate counts
a skip as a failure. Its scope is the client/hub boundary with those isolated peers.

Tests use an explicit synthetic upstream, real SQLite, real encryption and
ASGI HTTP/WebSocket. They do not verify a real Hermes turn, real Hermes voice,
macOS capture or performance. Real Mac acceptance requires an isolated real
Hermes test profile and a Mac. Verify Hermes contracts and integration limitations
against the installed version; see [self-hosting](../docs/self-hosting.md#hermes-and-orbit-mcp).
