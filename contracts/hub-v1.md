# Orbit Hub protocol v1

TLS is mandatory except explicit localhost development. Tokens are device-scoped;
server-side settings choose the Hermes profile. Never trust a client device ID.

`POST /v1/pair` accepts `{code,name}` and returns `{device_id,token}`. Codes are
one-use and short-lived; create/revoke through the hub's local admin CLI.

`GET /v1/sessions?before=&limit=` uses the device bearer token and returns
`{"sessions":[{"id":string,"agent_id"?:string,"created_at_ms":integer,"last_at_ms":integer,
"message_count":integer}],"total":integer,"next":string|null}`, newest first,
at most 128 rows per page. `next` is the next page's `before`. `last_at_ms` is
the last message's creation time, or the session's if empty. The app renders
the date as the visible title.

`GET /v1/sessions/{id}/messages?before=&limit=` returns
`{"messages":[{"id":integer,"role":"user"|"assistant","text":string,
"at_ms":integer|null,"kind":"text"|"voice","duration_ms"?:integer,
"client_id"?:string}],"next":integer|null,"total":integer,"agent_id"?:string}` in order.
`agent_id` is the session's agent; see [Agents](#agents).
Pages allow 200 messages and 512 KiB of JSON without cutting a message; `next`
reads older messages. A message unable to fit alone yields
`413 session_message_too_large` with `bytes`/`max_bytes`; another device's
session yields `404 session_not_found`.

`POST /v1/recordings/{uuid}/transcript/retry` uses the same authentication and
returns `{"requeued":nonnegative-integer}`. The recording ID is a canonical UUID.

## Local daemon IPC

The same-user IPC envelope remains `{"v":1,"id":string,...}`. These additive
commands are available to the app; unknown or repeated fields remain invalid:

```json
{"command":"pair","hub_url":"https://hub.example","code":"one-use-code","device_name":"Example Mac"}
{"command":"setup","hub_url":"https://hub.example"}
{"command":"session_open","id":"Orbit-session-id"}
{"command":"transcript_retry","recording_id":"11111111-1111-4111-8111-111111111111"}
{"command":"voice_note_audio","message_id":"30000000-0000-4000-8000-000000000003"}
{"command":"speak","message_id":"30000000-0000-4000-8000-000000000004","text":"Texto de la respuesta"}
{"command":"speak_stop"}
```

`pair`/`setup` validate the hub origin and restart only the daemon's hub
connection. The UI never stores its own credential/configuration.
`remote_switch` is retired: macOS permissions decide, and the daemon rejects
that command as unknown. `session_open` changes the selected conversation without
disturbing the engine or its activity. Messages are read later through
`session_messages` from the hub. `transcript_retry` publishes `requeued` in
the transcript detail. Retired `activity`/`activity_revert` are unknown commands.

`speak` («Escuchar» on an answer) fetches `text` through `POST /v1/audio/speech`
and plays it on the Mac; a new one replaces the one being read. The assistant
topic names the answer as `speaking` until the helper reports that playback
played or cancelled, or `speak_stop` stops it. It is refused with its cause
while privacy is stopped, without the helper, unpaired, during a recording or
a call, or for empty text or text over 32 KiB (`text_length` with both sizes).

`voice_note_audio` is local-only. The daemon finds the encrypted WAV recording
bound to `message_id` and replies outside `State` with additive envelope fields:

```json
{"v":1,"id":"request-uuid","ok":true,"state":null,"error":null,"message_id":"30000000-0000-4000-8000-000000000003","format":"wav","bytes_base64":"UklGRg..."}
```

Decoded WAV is at most 8 MiB. Missing audio yields `voice_note_missing`; oversize
yields `voice_note_too_large`. Only this response permits Base64 expansion within
a 12 MiB frame budget; other IPC requests/replies retain the general 1 MiB bound.
This read never requests or sends audio to the hub.

Response `State` adds `remote:{observation:boolean,action:boolean,meetings:boolean}`
and `helper_connected:boolean`. `daemon_version` is retired because the daemon
runs inside the app at its version; older snapshots containing it are read
without it. Missing fields decode to empty/false for v1 compatibility; empty
additive fields may be omitted on reserialization. Credentials never appear in
state, notices or logs. `session_messages` adds `audio_available` to voice notes
when this Mac still has their encrypted audio, found by `client_id`.

`GET /v1/ws`, header `Authorization: Bearer <device token>`, exchanges JSON:

Client: `{v:1,id:<uuid>,method:<method>,params:{...}}`.
Methods: `chat.send` (`text`, `session_id` optional Orbit ID, `call_active`
optional boolean defaulting to false; also optional are `client_id`, the
Mac's id for the message, `voice:{duration_ms}` for a voice note, and
`meeting:{recording_id,minutes,title}` for a question about a meeting, and
`agent_id` for the agent the turn is for (see [Agents](#agents)): the hub
adds the meeting's last `minutes` of transcript and keeps only the question and
«[Durante «título» · últimos N min]»; a recording this device does not hold is
`meeting_not_found`), `chat.interrupt` (`session_id`,`turn_id`), `approval.respond`
(`session_id`,`request_id`,`approved`,`turn_id`), `clarify.respond`
(`session_id`,`request_id`,`answer`,`question_id` optional), `ping`.
`calendar.ask` and the routes of the hub's copy of the agenda are retired;
since 2026-10-15 the hub no longer keeps a notice for them and refuses
them like any other method or route outside this contract. The only way
anything reads the Mac's calendar is the live `calendar_read` below.
`turn_id` is the UUID of the original `chat.send`, never a newly generated control
ID. Interrupt and deny require it; the new client also sends it for approval.
The server rejects a control for another device, session or stale turn. Legacy
positive approvals may omit it but still require an exact live pending request.

Server: `{v:1,id:<request uuid or null>,event:<event>,data:{...}}`.
Events: `ready` (`device_id`, `assistant_name`, `agents`, `default_agent_id`), `chat.session` (`session_id`), `chat.delta`
(`session_id`,`text`), `chat.completed` (`session_id`,`text`,`status`,
`persisted`,`warning`), `approval.required`
(`session_id`,`request_id`,`description`), `clarify.required`
(`session_id`,`request_id`,`question`), `error` (`code`,`message`), `pong`,
`revoked`. Unknown fields/methods and oversized messages are rejected.

Asynchronous turn events carry the original `chat.send` UUID in `id`. A command
acknowledgement carries that command's UUID. A failed completion does not persist
its text as an authoritative assistant history message. Terminal Hermes errors
without completion become `error.code:upstream_turn_failed` on the same turn;
ambiguous errors remain warnings unless read-only reconciliation proves no agent
is running. Neither path resubmits a prompt.

When persisting the assistant's final reply fails inside `message.complete`
(`append_message`, session status update or quota), the turn still ends with
`chat.completed`: the frame adds `persisted:false` and `warning:"<HubError code>"`
(additive, both absent when persistence succeeded). The session leaves
`streaming`, only this session is affected, and the Hermes connection stays
open; a subsequent `chat.send` is rejected only by the original cause, never by
`session_busy`. A payload that is not a JSON object is ignored with a
`chat.status` warning (`kind:"warning"`), never propagated to the transport
reader, and leaves the session in its current state.

`state.snapshot` has `id:null` and data:

```json
{"revision":1,"sessions":[{"session_id":"Orbit-session-id","turn_id":"original-chat-UUID","status":"streaming","pending":[{"request_id":"Hermes-request-id","kind":"approval","state":"pending","expires_at":null,"description":"bounded description","choices":["once","deny"]}]}],"omitted":false}
```

The snapshot is scoped to the authenticated device's live runtime and follows
pending-request, resolution, expiry and completion events. Reconnection receives
one before new events if the runtime has sessions. Revisions are monotonic within
that runtime; clients reset revision tracking on a new WebSocket connection and
reconcile only their selected session, never pick an arbitrary session or resume.
History also carries the current selected `session_snapshot`, or null when no
live snapshot exists. Snapshots include at most 64 sessions and 128 pending items
per session, approximately 100 KiB combined data; `omitted:true` exposes bounds.
Unknown expiry stays null. Clarify items use `question`, `choices`, and optionally
bounded `questions` with qids; answered qids leave the next snapshot.

`interaction.resolved` or `interaction.expired` replies use the response-command
UUID, `session_id` and `request_id`. Approval success requires
`status:"ok",resolved:1,applied:true`; zero returns
`status:"expired",resolved:0,applied:false`. Other counts/types are protocol
errors, not permission grants. Clarify uses its separate
`status:"ok"|"expired",remaining:[qids],applied:boolean` contract. Clients must not
clear a pending interaction optimistically. The result precedes its updated
snapshot; the snapshot is authoritative when several requests are pending.

An RPC acknowledgement can arrive after an interaction expires, its turn ends,
or another request replaces it (even with an identical request ID). Such replies
include `stale_interaction:true` and the historical `turn_id`. Their `applied`,
`resolved` and `remaining` describe the prior RPC receipt, not current pending
state. Clients consume that response without clearing or restoring any current
interaction; the current snapshot remains authoritative. Deduplicated responses
retain this marker and never repeat the upstream action.

Interrupt and explicit deny use a separate encrypted durable safety ledger that
does not consult content admission quota. It deduplicates by bound target, with
128 active targets per device; new retry UUID aliases do not create more rows.
Resolved/obsolete receipts retire only when another valid control is admitted;
removed targets cannot pass live turn/request validation. Uncertain targets are
not reapplied automatically. Physical disk exhaustion remains a deployment
constraint requiring reserved metadata capacity; revocation is still local.

The hub persists Orbit→Hermes bindings per authenticated device and never allows
access to a Hermes session ID supplied by the client. Reconnecting does not
resubmit a prompt automatically. Uncertain completion must remain visible.
When it opens an upstream session, the hub sends `source:"orbit"` and a title
`Orbit · <device name>` in `session.create`. A `chat.send` with
`call_active:true` uses `llamada de voz en curso` as the private Hermes header;
the stored message and public events retain only the original text.
`prompt.submit` omits `source` unless support was established earlier on that
same connection; the current adapter performs no destructive prompt probe and
therefore submits once without the field.

An answered or refused `/internal/v1/device/request` updates the active device's
durable `last_seen`; a timeout does not fabricate activity.

A refusal travels as `{"refusal":{"code":<code>,"retry":<policy>,…}}`. The code
identifies the cause and fields preserve its numbers. The UI and translator
render those fields in Spanish. `code` remains 1–64 characters `[a-z0-9_]` and
`retry` retains its four values. An HTTP request body over its ceiling is
`413 too_large` with `resource` (`capture`, `recording`, `audio`, or
`hub_request` for any other body), the `bytes` received when it was cut and
`max_bytes`. The storage quota refuses with `507 quota_exceeded` and `bytes`
(the write), `reserved_bytes` (what admission reserves for it), `used_bytes`
and `max_bytes`. The hub/translator no longer accept the old
0.6.x `refused` + `code` response; connected Macs must upgrade together.
Historical journal receipts remain readable.

Typed refusals remain structured up to the caller. An already dispatched action
keeps `outcome_unknown` and `operation_id`; if authorization is withdrawn before
its receipt, the bridge adds `interruption` with the typed refusal. That cause
never turns an uncertain outcome into a safely repeatable action.

The parameters that
travel in the other direction are pinned by the shared fixture
`fixtures/device-bridge-request-v1.json`: the translator's suite asserts that
what leaves it is exactly that document and the Mac's suite feeds the same
document to its real handler, so a change of type on either side fails a test
instead of shipping a capability that refuses every call, as `calendar_read`
did throughout 0.3.11.

`POST /v1/audio/transcribe` accepts bounded WAV binary, returns `{text}`.
`POST /v1/audio/speech` accepts `{text}` and returns the complete Hermes clip
(at most 4 MiB), retaining `Content-Type` and declaring
`X-Orbit-Speech-Kind: clip`/`X-Orbit-Speech-Provider`. Failures yield
`speech_unavailable`; oversize yields `speech_too_large` with `bytes` and
`max_bytes` (the 4 MiB clip, or Hermes's 6 MiB answer around it). The server profile
selects voice, format and provider; credentials stay server-side.

## Private meetings for the assistant

Meetings have no sharing switch. Retired `PUT /v1/meetings-sharing` is no longer
called by the daemon.

`POST /internal/v1/meetings/list`, `…/transcript` and `…/note` use
`orbit_meetings_bridge_v1`, a dedicated key and `X-Orbit-Meetings-` headers.
Signatures cover exact method, path and body. Each route has its own
`Authenticator`; a signature for one route cannot open another. Invalid
signatures yield 401. A valid HMAC key is the sole requirement.

A recording is `capture:"open"` while the hub has chunks but no final manifest.
This does not assert that anyone is still speaking; include the last audio
receipt time.

`meetings/list` takes `{device_id,since_ms,limit,before_ms?}` (`limit` 1–100) and
returns `{"meetings":[…],"more":bool}`, latest start first, including open
recordings. Rows contain
`{recording_id,title,started_at_ms,duration_ms,capture,transcript,note}`.
Open rows add `last_audio_received_ms`; duration is received audio and start
may be null (unknown, not zero). `transcript` is `ready|pending|partial|failed`.
`note` contains `state`:
`not_requested`, `not_configured`, `waiting_transcript` (`retry_at_ms`),
`queued` (`retry_at_ms,attempts,cause` and last-failure numbers),
`generating` (`attempt`), `failed` (`attempts,cause` and numbers), or `ready`.
Written notes also include the UI's `title`/`short_summary` and `revision`.
Continue `more:true` with `before_ms` equal to the last start.

`meetings/transcript` takes `{device_id,recording_id,format,after_index?}` and
returns `{text,after_index,next_after_index,total_chars,incomplete_groups,
capture,speaker_names}`. Open recordings add `pending_chunks` and
`pending_audio_ms` (received untranscribed audio on the most delayed track),
rather than refusing `transcription_pending`.
`format:"text"` gives one `[hh:mm:ss] Name: text` line per turn;
`format:"segments"` gives a JSON object per transcribed nonempty group,
`{at_ms,track,speaker,first_index,last_index,text}`.
Names/times match the Mac's `GET /v1/recordings/{uuid}/turns`: relative start,
no empty groups, consecutive same-voice turns merged, local mic named after
the owner, explicit user renames, otherwise “Otra persona”.
`speaker_names:"profile"` means the owner is known; otherwise `"tracks"` with
“Micrófono local”. Pages allow 1 MiB; `after_index` continues without mixing
devices. Past-end requests yield `after_index_out_of_range`
`{after_index,total_chars}`.

`meetings/note` takes `{device_id,recording_id}` and returns the same note the
UI reads: `{recording_id,state,revision,progress,note,edited,owner_name,
private_notes,superseded_edits,updated_at_ms}`. Owner is null until supplied;
private notes come from the hub. `note` reflects edited title/sections
(`edited:{title,sections}`) and each task's `done`. Regeneration keeps state and
progress visible. Unwritten notes refuse with their state/numbers:
`404 note_not_requested`, or 503 `note_not_configured`,
`note_waiting_transcript` (`retry_at_ms,capture,pending_chunks`),
`note_generating` (`attempt`), `note_queued` (`retry_at_ms,attempts,cause,…`),
`note_failed` (`attempts,cause` and numbers). Unknown recordings yield
`404 recording_not_found`. Transcript/note content never enters logs.

`PUT /v1/captures/{capture_id}` accepts an authenticated device's `image/png`
body of at most 700,000 bytes. `capture_id` is a UUID; `X-Orbit-Request` binds
the upload to the outstanding device-bridge request and `X-Orbit-SHA256` is the
lowercase SHA-256 of the exact body. The hub retains at most `MAX_PENDING`
captures per device in memory for five minutes. A capture is delivered once:
when a `device.reply` result contains its `capture_id`, the private bridge route
removes that field and supplies the exact bytes as `data_base64`. A missing,
expired, already consumed, cross-device or differently bound capture produces a
`capture_missing` refusal.

Remote `screenshot`/`screen_sources`/`selected_text` depend on macOS Screen
Recording and, for text, Accessibility. There is no additional Orbit observation
permission or switch.

Screenshot replies from the Mac carry
`capture_id`, `application`, `title`, `width`, `height`, `mime_type` and
`sha256`, and never the image itself. This holds whichever side took the
picture: the native helper and Cua both upload the PNG by
`PUT /v1/captures/{capture_id}` and then name it, because a validated PNG may
be 700,000 bytes and a frame may be 131,072. A capture whose upload does not
complete is refused with `upstream_failed` (`service: hub`) rather than left unanswered; the translator names the screenshot using the requested capability.
`full_screen:true` is exclusive with `application` and
`title` and asks Cua for the complete desktop; ordinary window requests fall
back to the native helper when Cua is absent or cannot provide a valid image.
Selected-text replies carry `application`, `title` and `text` and never use
Cua.

Remote `calendar_read` depends only on full macOS Calendar permission. This is
the user's sole calendar consent, with no switch or second confirmation.

It takes `{"start_ms":<integer>,"end_ms":<integer>}`, milliseconds since the Unix
epoch, both required and `end_ms > start_ms`; any range, a month or a year. Any
other shape is refused with `invalid_parameter` naming what is wrong. The Mac
reads EventKit at that moment over **every** calendar, stores nothing, and does
not disturb the week the desktop app is showing.

The answer is paged, and the pages belong to one walk. A page's result is
`{"read_id":<string>,"requested":{start_unix_ms,end_unix_ms},
"read_at_unix_ms":<integer>,"system_timezone","provider_sync_status",
"enumeration","events":[…],"events_returned":<integer>,
"omitted":[{reason,count}],"next_after_ms":<integer>?,
"next_after_index":<integer>?,"next_after_invitee":<integer>?}`. The last page
carries no `next_*`. Every other page is followed by a request with the same
range plus `read_id`, `after_ms`, `after_index` and, when present,
`after_invitee` — the continuation `fixtures/device-bridge-request-v1.json`
pins in `continuations`. A page carries at most 96 KiB of events
(`MAX_CALENDAR_PAGE_BYTES`) so the reply clears the 128 KiB frame, and always
at least one event while events remain. One event whose own invitees do not fit
a page travels in fragments — the same event, each with the next part of its
invitee list, all but the last marked `invitees_continued:true` — and
`next_after_invitee` says where its list goes on. An event that cannot fit a
page even alone — without its guests, or with its largest guest — is left out
and named in the page's `oversized` list (its calendar, `calendar_item_id`,
dates, the `field` to blame and its `bytes` against the `limit` that fits), and
the rest of the read arrives; so does what the helper itself could not carry.
Nothing is trimmed and
nothing is reported as incomplete because of a ceiling of Orbit's: the
translator's `orbit_calendar` walks every page and joins them, so Deimos asks
once. A page the Mac takes longer than 10 s to fill is sent with what it has.

`read_interrupted` carries `calendar_read_restarted` or `capture_stopped` in
`cause.kind`; `privacy_stopped` is separate. The first means the reader restarted
or the read was replaced; `helper_restarted` identifies helper restart.
`capture_stopped` means a local macOS capture stop, such as lid closure/inactive
session. The translator restarts these reads from the beginning at most twice
and reports attempt counts on failure. Privacy stop is never retried: the user
must rearm locally.

Refusals remain distinct from empty results. `permission_denied`,
`permission_not_granted` and `permission_unknown` name the permission;
`calendar_backend_unavailable` names the absent reader; `helper_reply_invalid`
keeps field/expected shape; `helper_refused` keeps `helper_code`;
`calendar_read_limit` keeps limit/counts. `privacy_stopped` and
`invalid_parameter` retain typed causes. Current Macs send only typed refusals,
not loose restart codes. Historical `legacy.text` receipts remain readable with
bounded detail. Missing permission never becomes zero events.

A Deimos read decides nothing, asks nobody and leaves no local record: the
activity log that once showed it was retired.

Remote `control_tools` reads the Mac engine catalogue by page, without an
additional Orbit observation permission. `{}` starts; `{"after_index":integer ≥ 0}`
continues. Other shapes yield `invalid_parameter` naming the field. Its result

is `{"tools":[{name,description,input_schema}],"after_index":<integer>,
"total":<integer>,"next_after_index":<integer>?}`. `after_index` echoes the index
asked for, `total` is the number of tools in the whole catalogue, and
`next_after_index` is absent exactly on the last page. A page carries at most
96 KiB of tools so the reply clears the 128 KiB frame limit with its envelope,
and a page always carries at least one tool while tools remain. Nothing between
the engine and the caller removes a tool: the length of a tool's description is
not a reason to drop it, and a tool whose own detail cannot fit a page travels
with its name, a null `input_schema` and a notice in `description` naming why.
`total` is therefore the number the engine declares. The caller walks the pages and
assembles the catalogue; the tool `orbit_control_tools` does this walk itself,
so an assistant still asks once. `after_index` and `total` are required;
`next_after_index` may be absent on the last page. An empty catalogue is valid.
`perform_action` depends on macOS permissions and accepts
`{"action":{"tool":"<tool>","arguments":{...}}}`. `tool` must be declared by the
engine, 1–128 ASCII bytes (letters, digits, `_` or `-`);

the whole nested action is bounded to 16 KiB and the Mac validates the exact
typed arguments, then runs it: the macOS permissions are the only consent and
there is no confirmation on top ([control policy](control-policy-v1.md)). One
attempt only. The `{"adapter_id":"<uuid>","effect_id":"<uuid>"}` form went with
the native adapters.
The request route does not wait for the action to finish: after
reserving the request in `operations`, it returns HTTP 202 with
`{"state":"pending","outcome":"in_progress","operation_id":"<uuid>"}`.
`in_progress` means the Mac has the action and it has not finished — it never
means somebody is being asked to permit it. The old `awaiting_approval` outcome
is no longer accepted from the hub.

`POST /internal/v1/device/action-result`, signed with the same bridge key over
`{protocol,device_id,operation_id}`, reads that operation without dispatching or
retrying it. It returns one of exactly six outcomes: `in_progress`,
`rejected`, `expired`, `performed` with its receipt, `unknown` after an attempted
send, or `failed` with a stable code, notice and next step. An `in_progress`
answer also says where to read the outcome, so nothing about it invites the
conclusion that a permission was needed. A device-side
admission refusal may still return synchronously before a human proposal exists.
The receipt keeps both IDs, `attempted` and `retry:"no_repetir"`; `unknown` is
never retried. Current refusals include `permission_denied`, `permission_unknown`,
`permission_not_granted`, `invalid_parameter`, `busy` and `privacy_stopped`,
with cause, numbers and retry policy.

The hub validates the envelope (`protocol`, identifiers, parameter/reply objects,
exactly one result or refusal), size and HMAC/nonces. It does not model each
capability's content or discard unknown/null fields. Capture storage, device
correlation, quotas/deadlines and the action journal remain hub responsibilities.
Delivering captures replaces `capture_id` with `data_base64`.
New replies/receipts include `device_version`, null if unreported. The translator
validates results and names the field, expected shape and version on violations;
the Mac receives envelope errors, not consumer result-validation errors.

Capability models live in `server/orbit_mcp/bridge.py` and the Mac.
`contracts/fixtures/bridge/` is generated from Rust and consumed by both.
`read_id` allows 128 bytes and travels only with `after_ms`/`after_index`.
Extra keys are rejected by MCP and the corresponding Mac boundary. MCP window
filters retain a 1024-character budget. Actions are measured as canonical JSON,
at most 16384 bytes. Uncertain outcomes never authorize repetition.

If encoded `device.reply` exceeds 131072 bytes (128 KiB, the outgoing WebSocket
JSON frame limit), the device sends a replacement with the same `id`/`request_id`
and a typed `too_large` refusal: `resource:"message"`, original `bytes` and
`max_bytes:131072`. It excludes the original result. Existing refusals and replies
without `request_id` are not replaced; the replacement must itself fit.
An oversized `chat.send` is reported locally with size/limit and neither it nor
a substitute is sent.

`PUT /v1/chunks/{chunk_id}` accepts bounded binary audio and headers
`X-Orbit-Recording`, `X-Orbit-Track`, `X-Orbit-Sequence`, `X-Orbit-SHA256`.
Returns `{chunk_id,sha256,durable:true}` only after durable persistence. Same
ID+same bytes is idempotent; same ID+different bytes is a conflict.

IDs are stable non-path ASCII strings of up to 121 characters. Audio is a
self-contained WAV PCM16, at most 8 MiB. Optional capture metadata must arrive
as a complete set: `X-Orbit-Started-At-Ms`, `X-Orbit-Monotonic-Ns`,
`X-Orbit-First-Sample`, `X-Orbit-Frames`, `X-Orbit-Sample-Rate`,
`X-Orbit-Channels`. Format/count must match WAV; repeated bytes with changed
metadata also conflict. Wall-clock start is estimated, not word alignment.
New clients additionally send optional `X-Orbit-Capture-Epoch-Id` (UUID), requiring
the complete capture metadata set. It identifies one native capture epoch, not a
server timestamp or session. The hub preserves it in encrypted metadata, group
manifests and each source, and never groups different epochs or an epoch with
legacy untagged audio. Absent legacy epochs stay absent: old audio, metadata and
receipts are not rewritten. An opaque keyed equality index supports scoped reads.

`POST /v1/recordings/{uuid}/finalize` accepts
`{tracks:[{track,chunk_count,last_sequence,total_frames}],gaps,started_at_ms,finalized_at_ms,title?,kind?,event?}`.
`title` is optional for compatibility with older devices and, when present, is
a string of at most 256 characters. The hub stores it with the encrypted
recording manifest. `kind` is `meeting` (the default) or `voice_call`;
`event` is the calendar event the meeting was started from, in the shape of a
note's `event` (`{title,starts_at_ms,ends_at_ms,attendees,calendar_item_id}`).
The body is bounded to 512 KiB, the largest valid event escaped.
It verifies durable chunk counts, contiguous sequences and sample totals. The
same manifest is idempotent; a different one or new chunks after it conflict.
Finalizing a `meeting` asks for its note at once, with the manifest's title and
event and the series' template; a `voice_call` gets no note and is not listed.

`GET /v1/recordings/{uuid}/transcript`, paged `…/archive` and `…/live-transcript`
are retired and return 404: the Mac no longer copies hub meetings. The assistant
uses `POST /internal/v1/meetings/transcript`.

Finalized means capture ended. Complete requires both tracks and zero declared
gaps regardless of ASR state; ready additionally requires all chunks transcribed.

Pipeline v2 ASR uses immutable, non-overlapping groups of approximately 20 seconds
from one contiguous track, capped at 128 original sources and 20 MiB. Finalize,
source boundary or a 30-second durable receipt deadline flushes partial groups.
A sequence hole can be an upload still in transit: it waits for that same
deadline or finalization, rather than immediately declaring an audio boundary.
Adjacent sources with incompatible epoch, format, sample counter or native clock
still close a boundary immediately. No group crosses a sequence hole; a source
arriving after a group was sealed never rewrites that group's transcript.
Segments expose `group_id`, `track`, `sequence`, `last_sequence`, `frames`,
`source_chunk_ids`, `sources:[{chunk_id,sequence,sha256,capture_epoch_id?,first_sample,frames,sample_rate,channels,monotonic_ns,started_at_ms,wall_time_precision}]`,
`timing_precision:"group"`, `speaker:"unknown"`, `pipeline_version:2`,
`flush_reason`, `state`, `error_code` and `text` only when completed. One text
belongs to the whole group, never each one-second source. Multi-source segments
do not have a singular `chunk_id` or `source_sha256`. Legacy ASR remains a
one-source pipeline-v1 segment with `timing_precision:"chunk"`, without rerunning
the provider. Unassigned chunks have `state:"awaiting_group"` and no text.

### Note templates and assignment

All routes use the device bearer token and expose only that device's data.

- `GET /v1/note-templates` →
  `{"templates":[{id,name,context,sections:[{name,instructions}],builtin,forked_from,updated_at_ms}]}`.
  The six built-ins come first (`builtin:true`, null origin/time), then custom
  templates in creation order. `forked_from` is `{id,name}` or null.
- `PUT /v1/note-templates/{id}` with `{name,context,sections,forked_from?}` creates
  (201) or replaces (200) a custom template. Client-selected IDs
  (`^[a-z0-9][a-z0-9_-]*$`, up to 64; daemon uses UUID) make a lost-receipt retry
  idempotent. Bounds: 1–12 sections, names 1–80 characters, context 4000,
  instructions 2000. Invalid fields yield `400 invalid_note_template`
  `{field,max,actual?}`. Built-in PUT/DELETE yields `409 note_template_builtin`
  `{template_id}`; edit by forking, preserving `forked_from` on replacement.
  At most 32 custom templates (`409 note_templates_full`
  `{templates,max_templates}`) and 768 KiB total list, to fit the daemon's
  1 MiB response budget (`413 note_templates_too_large` `{bytes,max_bytes}`).
  Content is encrypted.
- `DELETE /v1/note-templates/{id}` → `{template_id,deleted,links_released}`;
  idempotent, `deleted:false` if absent. Assignments revert to General;
  existing notes retain template name/sections.
- `GET /v1/note-template-links` →
  `{"links":[{calendar_item_id,template_id,updated_at_ms}]}`.
- `PUT /v1/note-template-links` with `{calendar_item_id,template_id}` assigns or
  changes a series template. Unknown template: 404. At most 256 assignments:
  `409 note_template_links_full` `{links,max_links}`.
- `DELETE /v1/note-template-links` with `{calendar_item_id}` →
  `{calendar_item_id,deleted}`.

`calendar_item_id` is the stable calendar event/series identifier, 1–1024 UTF-8
bytes without control characters, matching core's `MAX_LABEL_BYTES`.

`POST /v1/recordings/{uuid}/note` takes `{template_id?}`, replacing the retired
`{inputs,regenerate}` shape. It regenerates or first creates an older note.
Absent template selects the manifest event series' assignment, otherwise General;
an explicit template wins. `note.template_source` is `requested|series|default`
(absent on older notes). Document `template_id` is the requested value or null
when the hub chose. Title/event come from the manifest; owner/voices/private
notes are read from the hub when generating.

### Meetings for the Mac

Device bearer authentication; errors include `code`, numbered `refusal` and `op`.

- `PUT /v1/device/profile` `{owner_name}` (1–128 characters, one line) →
  `{owner_name}`. The local mic uses it, otherwise “Micrófono local”.
- `GET /v1/recordings?before_ms=&before_id=&limit=` (`limit` 1–50, default 50) →
  `{meetings:[{recording_id,title,started_at_ms,duration_ms,capture,
  transcript,note}],more}`. Newest start first, ties by recording ID; includes
  open recordings, excludes voice calls. Legacy unfinalized voice notes are
  excluded when their only chunk is microphone sequence 0 lasting at least two
  seconds. Shorter legacy notes cannot be distinguished from a meeting's first
  second and remain listed. Title uses the explicit override, then manifest;
  open recordings use requested note title or a dated “Reunión del…” fallback.
  `transcript` is `none|pending|ready|failed`; `failed` includes terminal partial
  failures. `note` is `{state,short_summary}` or null. Continue with both the
  last `started_at_ms` and `recording_id` to avoid dropping tied starts.
  Without `before_id` only strictly earlier starts are returned.
- `GET /v1/recordings/{uuid}` →
  `{recording_id,title,started_at_ms,duration_ms,capture,language,
  speakers:[{key,track,name}],transcript:{state,pending_chunks,
  failed_groups:[{track,cause,attempts}],retranscribing?},
  note,private_notes:{text,revision}}`.
  Language is explicit user selection, otherwise most frequent among groups
  with text, otherwise configured `[meeting_transcription] language`.
  Null means neither text nor configured language; never “auto”.
  During language change, `retranscribing:{language,done,total}` counts newly
  returned groups against those requested by that change.
  `note` is null or `{state,revision,attempts,next_attempt_ms,error,
  template_id,note,edited:{sections},superseded_available,updated_at_ms}`.
- `GET /v1/recordings/{uuid}/turns?offset=` →
  `{turns:[{at_ms,speaker_key,text}],next_offset,total}`. Names come from the
  document's `speakers`. Pages allow 256 KiB JSON turns and at least one turn;
  `offset > total` yields `400 invalid_parameters` `{field,actual,max}`.
- `PUT /v1/recordings/{uuid}/speakers` `{names:{key:name|null}}` → `{speakers}`.
  Null restores the default. Unknown keys yield `400 unknown_speaker`.
  Renames affect reads, assistant tools and future notes, not existing note text.
- `PUT /v1/recordings/{uuid}/title` `{title}` → `{title}`. Changes list/document/
  assistant list and future notes, not existing notes. Trimmed to one line;
  empty removes the override. Over 256 characters yields `400 invalid_parameters`
  `{field:"title",actual,max}`; controls yield the same code with `{field:"title",max}`.
- `PUT /v1/recordings/{uuid}/language` `{language}` → `{language,requeued}`.
  Requires two lowercase ISO 639-1 letters, not “auto” or “es-ES”:
  `400 invalid_parameters` `{field:"language"}` otherwise.
  Repeats groups with text in another language and recognition in flight;
  old text remains while `transcript.retranscribing` reports progress.
  If any repeat and a note exists, regenerate with its template after transcript
  completion, preserving edits in `superseded_edits`.
  Same language: `requeued:0`. Failed textless groups wait for explicit Retry,
  which uses the new language. Retained audio is never deleted; unreadable
  `invalid_group_audio` leaves everything unchanged and yields
  `409 meeting_audio_unreadable` `{groups,total,cause:"invalid_group_audio"}`.
- `PUT /v1/recordings/{uuid}/private-notes` `{text,base_revision}` → `{revision}`.
  At most 65536 characters. A stale revision yields `409 stale_revision`
  `{revision,base_revision}`. Generation reads these notes. Legacy note-request
  private text is preserved at revision 0.
- WebSocket `recording.changed {recording_id}` reaches only the owning device's
  connections after closure, group completion/failure, note state/edit changes,
  names, private notes, title or language changes; retranscribed groups notify
  on return. Fixture: `contracts/fixtures/hub/events/recording.changed.json`,
  consumed as `HubEventKind::RecordingChanged`.

Unknown or cross-device recordings yield `404 recording_not_found`.
Template content never enters logs.

### Recording context (retired)

`GET` and `POST /v1/recordings/{uuid}/context` were retired on 2026-10-15
with their only consumer, the terminal's context query. They answer
404. The index table a hub kept for them is left in place, unread.

Device revocation closes active sockets and rejects uploads. Administrative
operations are local CLI commands, not exposed to the device transport.

## Meeting dictionaries

Both routes use the device bearer token and access only its dictionaries.
Names, terms, wrong forms and series titles/keys share an AES-GCM body; neither
errors nor logs include that content.

- `GET /v1/meeting-dictionaries` →
  `{dictionaries:[{id,name,terms:[{id,correct,wrong:[string]}],
  series:[{calendar_item_id,title}]}]}`.
- `POST /v1/meeting-dictionaries/orders` takes one order:
  - `{action:"save",dictionary_id,name}` creates/renames. The client chooses a
    UUID; repetition preserves entries/series.
  - `{action:"select",recording_id,dictionary_id:string|null,use_for_series:false}`
    selects for a meeting; null removes its override and inherits the series.
    With `use_for_series:true` assign the series and clear the meeting override;
    null releases the series. One dictionary per series; multiple series per dictionary.
  - `{action:"correct",recording_id,dictionary_id,wrong,correct,name?:string|null}`
    saves a correction and preserves an existing explicit/inherited selection of
    that dictionary; otherwise selects it for this meeting. `name` can create
    the dictionary and first entry in one transaction. Wrong forms for the same
    correct form share an entry; the corrected entry moves to the most recent end.
  - `{action:"remove",dictionary_id,entry_id}` removes the entry and all wrong
    forms; repetition is harmless.
  Orders return HTTP 200 with the updated dictionary, or `{dictionary_id:null}`.

The series becomes known at finalization. Open drafts have no series hint;
after closure, retries and final-pass windows can use it. Chunk uploads carry
no dictionary or series data.

Meeting documents add `dictionary:{dictionary_id:string|null,
source:"requested"|"series"|null,series:{calendar_item_id,title}|null}`.
A present null ID means no dictionary; an absent field means unknown, for example
an older hub or a Mac without the hub read.

`turns.py` replaces whole words/phrases in one case-insensitive pass, longest
phrase first. App, notes, questions and `orbit_transcript` share this read.
Segment output applies corrections after final-pass words are joined.
Originals are unchanged; removing an entry reverses the correction.
Existing notes retain text until regenerated.

The note model receives correct forms in all parts and reductions. Draft groups
and final windows send complete, most-recent terms in a multipart `prompt`
bounded to 160 characters including `Glosario: `, separators and final period:
`Glosario: term, term.`. No terms means no prompt. This conservatively budgets
for Whisper's last 223 tokens without a tokenizer; it does not guarantee a token
count for arbitrary Unicode. Repeated recognition rereads the hint.

Hub-only bounds: 32 dictionaries, 128 terms each, eight forms per term,
128 characters per form, 80 per name, 256 total series; 192 KiB per dictionary
(one item of the 256 KiB library read), 768 KiB per HTTP list, 16 KiB per order.
Refusals include `code`, a content-free message and numbers:
`dictionary_full {terms,max_terms}`,
`dictionary_text_too_long {field,characters,max_characters}`,
`dictionary_forms_full {forms,max_forms}`,
`dictionaries_full {dictionaries,max_dictionaries}`,
`dictionary_too_large`/`dictionaries_too_large {bytes,max_bytes}`,
`dictionary_series_full {series,max_series}`.
`dictionary_series_missing` means the series is unknown, not a zero count.
The Mac carries numbers to the UI. On order confirmation the daemon invalidates
all meeting/library reads once, as for templates, rather than one WebSocket event
per historical meeting. app-core does not reuse closed turns when a dictionary
exists or that fact is unknown. Unconfirmed orders retain the written correction
for resending and report the transport cause.

## Request transcription again

`POST /v1/recordings/{id}/final-pass` uses the owning device's bearer token.
It creates a pass or requeues a terminal `completed`/`failed` pass.
An already `queued`/`running` pass is not duplicated and does not request another
note. Reply: `{recording_id,requested:true}`. It uses the selected meeting language
or configured fallback, keeps prior text until a complete new publication, and
reuses voices already separated from the same audio.

Only an already-requested note is requested again with its template; it waits
for the pass and preserves edits as superseded, as for language changes.
`recording.changed` announces queue/start/end. The document carries
`transcript.final_pass:{state,error}` (`queued|running|completed|failed`);
absence means no registered pass. `transcript.unmatched_speaker_names` lists
names whose voices no longer exist; they are not assigned to new voices.
The microphone track name is retained.

Before queueing, refusals are `400 recording_still_open`,
`409 meeting_audio_unreadable` (`groups,total` for invalid audio), and
`409 diarization_not_configured`. Unknown/cross-device IDs remain
`404 recording_not_found`. Failures retain prior text and carry code, phase and
the cause's measured numbers.

## Final pass and recording voices

HTTP finalization stores the manifest and `recording_final_passes` job in one
transaction. The block draft remains readable while queued/running.
`transcript.final_pass:{state,error}` errors contain `code`, `phase` and cause
numbers (`provider_status`, `timeout_ms`, `exit_status`, sizes, etc.).
Without local separation configuration, admission marks the job failed with
`diarization_not_configured` rather than simulating voices.

The pass reads retained hub audio, converts to mono 16 kHz and constant 24 kbit/s
Opus. Windows are at most 45 seconds, cut at the midpoint of the longest silence
between 15 and 45 seconds after the previous cut (−48 dB and 0.3 seconds with
`ffmpeg silencedetect`). Without eligible silence, cut at 45 seconds.
All audio is retained; both word endpoints shift into track time.
Provider size limits remain checked. Requests use `verbose_json`,
`timestamp_granularities[]=word`, `vad_filter=true` without VAD parameter overrides.
The service applies its own faster-whisper defaults.
Responses supply `words:[{start,end,word|text}]` in seconds relative to each
window; `segments` may accompany them. No `initial_prompt` is used.
Voices are never merged automatically.

Both tracks publish atomically, encrypted, after the whole job finishes:
`recording_final_transcripts` holds words/times; `recording_voices` holds only
`{start_ms,end_ms,voice}` intervals. The separator returns no stored embeddings.
Voices are immutable. Language changes repeat text and reuse voices; stale-language
results cannot replace current results. Audio is never deleted.
`mic_local` retains the owner's name. Remote keys `voice_1`, `voice_2`, etc.,
are numbered by first appearance and displayed as “Persona 1”, “Persona 2”,
even for brief voices. Only the user merges them by renaming.
A remote word uses maximum overlap, otherwise the nearest voice within 250 ms;
more distant words are discarded. This does not filter the local microphone.

Before attribution/publication, each track is compared with its own draft,
including groups with text awaiting retranscription. Timing matches
`meetings/turns.py` (`started_at`, `_length_ms` and group manifest start).
Only groups with at least six words count. If word-start times within the
group interval ±1000 ms inclusive contain fewer than one quarter of its draft
words, that span is lost. Groups without text do not count: absence is not proof
of loss. Sound is not used to infer missing speech; the pass does not retry ASR.

A lost span, or a repetition loop over both 66 words and 223 permille of words,
fails `final_pass_incomplete` without publishing either track or new voices.
`phase=whisper_mic_local|whisper_remote_app` identifies the track.
Fields are `compared_groups`, `lost_groups`, `lost_draft_words` (sum of draft
words in lost spans, not a WER difference), `first_lost_group_ms` (null if none),
`loop_words` and `word_count`. The response also states
`min_draft_words=6`, `min_final_word_divisor=4`, `draft_margin_ms=1000`,
`max_loop_words=66` and `max_loop_permille=223`.
Loop counting ignores punctuation and case. Windowing failures use
`windows_<track>` and preserve ffmpeg causes/numbers.
These checks guarantee neither WER nor semantic accuracy.

The note waits for the final job. On failure it waits for the draft to finish and
uses it, including terminally failed groups, preserving `final_pass_error` and
its numbers for assistant reads. An unnamed “Persona N” is never invented as a
task assignee. Existing notes retain text until explicitly regenerated.

Older recordings are not queued by migration or repeated finalization. Use
`POST /v1/recordings/{uuid}/final-pass` to request or retry a pass.

`PUT …/speakers` still accepts `{names:{key:name|null}}` for an entire voice.
Naming one voice after another records `same_as` and merges them in reads.
Optional `turn:{at_ms,speaker_key}` applies the single supplied name only to that
turn. The hub validates the pair against current turns inside the write
transaction and stores `{track,start_ms,end_ms,speaker_key}`, never list position.
A disappeared turn yields `409 turn_no_longer_exists` with `at_ms` and current
`turns` count. Null restores the interval's original voice.

`meetings/transcript` adds `final_pass:{state,error}`. After a final pass,
`segments` adds `end_ms` and `speaker_key` and each segment is one turn with
word-measured endpoints. Draft/legacy formats are unchanged.
`first_index`/`last_index` are 0 in these segments and do not identify a turn.
Corrections use the `{at_ms,speaker_key}` pair from the device's **turns** page.
App, notes and assistant tools apply names, merges and corrections through
`meetings/turns.py`.

The hub selects `assistant_name` from configuration (default `Deimos`) and sends
it on every authenticated `ready`. It is independent of the Hermes profile ID.
The Mac retains the last learned name for offline use; an older hub omitting
this optional field does not erase it. Before learning any name it uses Deimos.
Invalid names are refused during hub configuration loading, before any socket
can advertise them; see the [self-hosting guide](../docs/self-hosting.md).

## Agents

A hub can offer several agents, each an assistant with its own backend (today
a Hermes profile per agent). The addition is optional on both sides: a 0.9.0
Mac keeps working against a hub with agents, and a Mac with agents keeps
working against a 0.9.0 hub.

**`ready`** adds `agents`, the hub's agents in its order, and
`default_agent_id`. Each agent is `{id, name, kind, capabilities}`: `id` is
stable, `[a-z0-9-]`, 1 to 32 characters; `name` is what the app shows and what
the wake phrase hears («Oye Atlas»); `kind` is `hermes` today;
`capabilities` lists what the agent accepts today, out of `images`, `voice`
and `resume`. `assistant_name` stays, with the default agent's name, for Macs
that do not read `agents`.

```json
{"v":1,"id":null,"event":"ready","data":{
  "device_id":"device-1","hub_protocol":1,
  "assistant_name":"Atlas",
  "agents":[
    {"id":"atlas","name":"Atlas","kind":"hermes","capabilities":["images","voice","resume"]},
    {"id":"vega","name":"Vega","kind":"hermes","capabilities":["resume"]}],
  "default_agent_id":"atlas"}}
```

A hub that omits `agents` (0.9.0) has one agent: the Mac calls it by
`assistant_name` (or the last name it learned, or Deimos) under the id
`default`, and never sends it `agent_id`, because that hub refuses unknown
`chat.send` parameters. A hub that omits `default_agent_id` has its first agent
as default.

The Mac keeps the last list it was given, with `assistant_name`, in
`hub-agents.toml` beside `config.toml` (never in `config.toml`, which older
Orbit versions must still load) and uses it to start without the hub; a `ready` without `agents` replaces it with the one agent of
that hub. The Mac does not take a list that is empty, has an agent with a blank
`id` or `name`, repeats an `id`, or whose `default_agent_id` is not one of its
ids: it logs `hub_agents_refused` with the cause (`agents_empty`,
`agent_id_blank`, `agent_name_blank`, `agent_id_repeated`,
`default_agent_unknown`) and its positions and count, never a name, and keeps
the list it had. It does not repeat the hub's bounds on an id's syntax or a
name's length; the hub refuses those when it loads its configuration, and the
hub frame bounds the list's size.

**`chat.send`** accepts `agent_id`. Without it the turn is for the default
agent. A session belongs to one agent:

```json
{"v":1,"id":"00000000-0000-4000-8000-000000000901","method":"chat.send",
 "params":{"text":"¿Qué tengo mañana?","client_id":"00000000-0000-4000-8000-000000000903",
  "call_active":false,"agent_id":"vega"}}
```

- A `session_id` of another agent's session is refused with
  `error.code:"agent_mismatch"` and
  `numbers:{"agent_id":"vega","session_agent_id":"atlas"}`.
- An `agent_id` the hub does not have is refused with
  `error.code:"agent_not_found"` and
  `numbers:{"agent_id":"orion","agent_ids":"atlas,vega"}`, the ids there are
  in the hub's order, joined by commas.

`numbers` values stay integers, strings or booleans: a Mac decodes no array
there and would refuse the whole frame. Neither refusal runs a turn. The Mac sends `agent_id` on every `chat.send` to
a hub that listed agents: the open conversation's agent, or the default one
for a conversation opened without one. It refuses locally, before anything
changes, a new or opened conversation with an agent the last list does not
have, naming the id asked for and the ids there are.

**Sessions** say their agent: `agent_id` on each row of `GET /v1/sessions`
and on the page of `GET /v1/sessions/{id}/messages`. Sessions from before
agents are the default agent's; a row without `agent_id` (a 0.9.0 hub) is too.

**On the Mac.** A conversation is bound to its agent when it begins (the
agent the user picked, or the default one) or when it is opened from the list
(the row's `agent_id`); it never changes agent. With two or more agents, the
app offers the choice when a conversation begins and says whose an open one
is; with one, it offers nothing, as before. Every text names the open
conversation's agent. The wake phrase hears every agent's name
([native-v1](native-v1.md)); the question goes to the agent named, in a
conversation of its own when the open one is another agent's.

