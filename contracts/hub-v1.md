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
"client_id"?:string,"attachments"?:[attachment]}],"next":integer|null,"total":integer,
"agent_id"?:string}` in order. `attachments` lists a user message's files; see
[Attachments](#attachments).
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
{"command":"chat","text":"¿Qué ruta marca?","attachments":[{"id":"…","name":"mapa.png","kind":"image","mime":"image/png","bytes":120}]}
{"command":"attachment_add","source":{"from":"path","path":"/Users/example/mapa.png"}}
{"command":"attachment_add","source":{"from":"bytes","name":"Imagen pegada.png","data_base64":"iVBOR…"}}
{"command":"attachment_add","source":{"from":"window"}}
{"command":"attachment_fetch","attachment_id":"…"}
{"command":"attachment_save","attachment_id":"…"}
```

The `attachment_*` orders run off the daemon's loop and answer outside `State`
with `{"v":1,"id":…,"ok":…,"error":…,"refusal"?:…,"attachment"?:…,"content"?:
{"name","mime","bytes"},"path"?:string}` (`content.bytes` is base64 on the
socket; the app takes it in process). The app waits 100 s for one: the hub
request's own 90 s and 10 s for the daemon. See [Attachments](#attachments).

`pair`/`setup` validate the hub origin and restart only the daemon's hub
connection. The UI never stores its own credential/configuration.
`remote_switch` is retired: macOS permissions decide, and the daemon rejects
that command as unknown. `session_open` changes the selected conversation without
disturbing the engine or its activity. Messages are read later through
`session_messages` from the hub. `transcript_retry` publishes `requeued` in
the transcript detail. Retired `activity`/`activity_revert` are unknown commands.

`speak` («Escuchar» on an answer) fetches `text` through `POST /v1/audio/speech`,
in the voice of the open conversation's agent (the default agent's when it
has none of its own), and plays it on the Mac; a new
one replaces the one being read. The assistant topic names the answer as
`speaking` until the helper reports that playback played or cancelled, or
`speak_stop` stops it. It is refused with its cause while privacy is stopped,
without the helper, unpaired, during a recording or a call, or for empty text
or text over 32 KiB (`text_length` with both sizes).

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
Methods: `chat.send` (`text`, empty only beside `attachments`, the ids of the
turn's files (see [Attachments](#attachments)); `session_id` optional Orbit ID, `call_active`
optional boolean defaulting to false; also optional are `client_id`, the
Mac's id for the message, `voice:{duration_ms}` for a voice note, and
`meeting:{recording_id,minutes,title}` for a question about a meeting, and
`agent_id` for the agent the turn is for (see [Agents](#agents)): the hub
adds the meeting's last `minutes` of transcript and keeps only the question and
«[Durante «título» · últimos N min]»; a recording this device does not hold is
`meeting_not_found`), `chat.interrupt` (`session_id`,`turn_id`), `approval.respond`
(`session_id`,`request_id`,`approved`,`turn_id`, and optionally `choice`: one of
the request's `choices`; see [Choices](#choices-and-questions)), `clarify.respond`
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
Events: `ready` (`device_id`, `assistant_name`, `agents`, `default_agent_id`,
`attachments`, `storage`), `storage.status`, `chat.session` (`session_id`), `chat.delta`
(`session_id`,`text`), `chat.completed` (`session_id`,`text`,`status`,
`persisted`,`warning`), `approval.required`
(`session_id`,`request_id`,`description`), `clarify.required`
(`session_id`,`request_id`,`question`), `chat.item` (see
[What the agent does](#what-the-agent-does-chatitem)), `error` (`code`,`message`), `pong`,
`revoked`. Unknown fields/methods and oversized messages are rejected.

Asynchronous turn events carry the original `chat.send` UUID in `id`. A command
acknowledgement carries that command's UUID. A failed completion does not persist
its text as an authoritative assistant history message. Terminal Hermes errors
without completion become `error.code:upstream_turn_failed` on the same turn;
ambiguous errors remain warnings unless read-only reconciliation proves no agent
is running. Neither path resubmits a prompt.
An assistant run inside the hub (`kind = "openai"`) names why its turn ended
without an answer: `error` on the same turn with `session_id`, a Spanish
`message` and a `refusal` carrying the numbers. Codes: `assistant_unreachable`
(`connect_timeout_ms`; the message names the provider URL without secrets),
`assistant_connect_timeout` and `assistant_timeout` (`connect_timeout_ms` or
`timeout_ms`), `assistant_connection_lost`, `assistant_unauthorized` and
`assistant_http_error` (`provider_status`), `assistant_rate_limited`
(`provider_status`, and `retry_after_ms` when the provider said),
`assistant_context_exceeded` (`history_chars`, `max_history_chars`, and
`max_tokens`/`tokens` when the provider gave them), `assistant_tool_failed`
(`timeout_ms` when an Orbit tool never answered; its outcome is unknown),
`assistant_turn_too_long` (`max_requests`), `assistant_provider_error` and
`assistant_failed`.
An ACP agent (`kind = "acp"`) names its causes the same way: `agent_auth_required`
(its session on the server expired or never began: sign in from Orbit, see
[Assistants from Orbit](#assistants-from-orbit)), `agent_limit_reached` (its
plan's limit; `resets_at_ms` when the adapter said when it resets, and the
message quotes the agent's own sentence, which may say it), `agent_rejected`
(`acp_code`; the message quotes the agent's reason, bounded), `agent_stopped`
(`exit_status` when its process ended, or `max_bytes` when one of its messages
crossed that ceiling), `agent_protocol`, and `agent_turn_unfinished`
(`stop_reason`: `max_tokens`, `max_turn_requests` or `refusal`). What the agent
said before it stopped was streamed live and is not stored as its answer.

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

### What the agent does: `chat.item`

Each step of the agent's work in a turn is an item. It travels as `chat.item`
with the turn's `chat.send` UUID in `id`; it is live only, neither stored in
the conversation nor carried by `state.snapshot`, so a Mac that reconnects in
the middle of a turn sees the steps that follow.

```json
{"v":1,"id":"original-chat-UUID","event":"chat.item","data":{"session_id":"Orbit-session-id","item_id":"call-1","kind":"tool","phase":"started","label":"Mirando la pantalla","subject":"la pantalla"}}
```

- `item_id`: stable across one step's phases within the turn (a tool call's
  id; `plan` for the turn's plan; a reasoning or subagent id).
- `kind`: `tool`, `reasoning`, `plan` or `subagent`.
- `phase`: `started`, `progress`, `completed` or `failed`. A `failed` item
  carries `cause`, the agent's words on one line, at most 300 characters
  (`sin causa declarada` when it gave none).
- `label`: one line of at most 500 characters (`activity.MAX_ITEM_LABEL_CHARS`):
  Hermes's own reasoning preview is 500 characters, and past that it is no
  longer a line one reads in passing. It protects the frame and the Mac's
  assistant topic from upstream text of any length; it limits neither what
  the agent does nor its answer. For Orbit's seventeen orbit-mcp tools it is
  Spanish («Mirando la pantalla», «Leyendo el calendario», «Pulsando en
  Notas», «Escribiendo» — never the text typed); any other tool keeps its own
  name. For reasoning it is the summary, when the agent sends one; for a
  subagent its goal; for a plan «Plan · N de M pasos».
- `subject` (tools): what the step used, at most 80 characters («la
  pantalla», or the tool's own name), for the summary of the turn's work.
- `steps` (plans): the whole plan on each update, at most 20 steps of
  `{text,status}`, `status` one of `pending`, `in_progress`, `completed`,
  `cancelled`; the label counts every step.

Hermes: `tool.start`/`tool.complete` (`is_error` is `failed` with the
result as its cause), `todo.updated` and the `todo` tool's result,
`reasoning.available`, `subagent.start`/`progress`/`tool`/`complete`.
`reasoning.delta` is the thought itself and `thinking.delta` a spinner's
text: neither is relayed. ACP: `tool_call`, `tool_call_update`, `plan` and
`agent_thought_chunk` (a streamed thought is one reasoning item, completed with
its whole text when anything else arrives). The OpenAI-compatible assistant
(`kind = "openai"`): each tool call is an item from the call to its result
(`failed` with the cause when the tool refused or the model must retry it),
and a streamed thought is one reasoning item, completed with its whole text
when the next part starts or the turn ends; it never asks for approval (if it
did, it would offer `once` and `deny`). A Mac without `chat.item`
(0.9.0) decodes it as an unknown event, records its name once and changes
nothing.

### Choices and questions

`approval.required` and `clarify.required` travel under the turn's `chat.send`
UUID in `id`; their `request_id` names the request (the backend's own id, or
one the hub made unique on this device) and is never the turn's. A client
takes a question as current by the turn in `id`.

`approval.required` carries `choices`, the answers the agent offers, in
the hub's vocabulary: `once`, `session` (this conversation), `always`,
`deny`. Hermes offers `once` and `deny` and, when allowed, `session` and
`always`; an ACP agent's options are `once` (`allow_once`), `always`
(`allow_always`) and `deny` (`reject_once`, else `reject_always`). `choices`
absent means `once` and `deny`. A request whose command deletes files
(`server/orbit_hub/assistant/deletion.py`: `rm`, `unlink`, `rmdir`, `shred`,
`trash`, `git rm`/`git clean`, `find -delete`, also behind `sudo`, `xargs`,
`sh -c` or in a chain; for ACP, also a tool call of kind `delete`) is approved
one at a time: the hub offers only `once` and `deny` of what the agent
offered, refuses `session` and `always` with `approval_not_allowed`, and adds
`deletes` to `approval.required` and to its snapshot item, with `paths`, the
number of files, when the command is one plain `rm` or `unlink` that names
them (`{"paths":2}`; `{}` when not known). A 0.10 Mac ignores `deletes` and
shows the two choices. The Mac answers with `approval.respond`
`choice`; `approved` must agree with it (`deny` is the only refusal, since
`approved:false` is what reaches the safety ledger), else the frame is
refused with `approval_choice_mismatch` before anything else. A choice the
request did not offer is `approval_not_allowed`, naming the choice and what
was offered. Without `choice` the hub answers `once` or `deny` by `approved`,
as a 0.9.0 Mac does; a 0.9.0 hub ignores `choice` and does the same.

`clarify.required` carries `choices` (suggestions; the answer is free text
too) and, for several questions at once, `questions` of
`{qid,question,choices?,multi_select?}`. The Mac answers each with its own
`clarify.respond` naming `question_id`; `remaining` lists the qids still
open, which the next snapshot keeps.

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

A `chat.send` id is reserved when the frame is admitted, so a resend after a
lost receipt never resubmits (`outcome_unknown`). A send the hub refuses before
it stored the message (a refusal such as `meeting_not_found`, or a full disk)
gives its id back: sending the same frame again is a new attempt. Once the
message is stored, the id stays reserved whatever follows.

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
`max_bytes`. The storage limit refuses with `507 quota_exceeded` and `bytes`
(the write), `reserved_bytes` (what admission reserves for it), `used_bytes`,
`max_bytes`, `remaining_bytes` and `limit` (`quota` when `storage_quota_bytes`
sets it, `disk` when the disk does). A write the disk itself refused is `507
storage_full` with `used_bytes` and `free_bytes` (the disk's free space).
Both are `retry:{"kind":"after_fix"}`: the same write is admitted once someone
frees room or raises the quota; the Mac keeps its copy until then. The
hub/translator no longer accept the old
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
`POST /v1/audio/speech` accepts `{text, agent_id?}` and returns the complete clip
(at most 4 MiB), retaining `Content-Type` and declaring
`X-Orbit-Speech-Kind: clip`/`X-Orbit-Speech-Provider`. Failures yield
`speech_unavailable`; oversize yields `speech_too_large` with `bytes` and
`max_bytes` (the 4 MiB clip, or Hermes's 6 MiB answer around it). The server profile
selects voice, format and provider; credentials stay server-side. Read-aloud
uses the voice of the agent `agent_id` names (one of `ready.agents`): the Mac
sends the open conversation's agent, for «Escuchar» and for spoken replies,
to a hub that listed its agents. Without the field (a 0.9.0 Mac) it is the
default agent's voice. An unknown id yields `404 agent_not_found` with
`agent_id` and `agent_ids`. Dictation (`/v1/audio/transcribe`) stays the
default agent's: it is the user's input to whichever agent they talk to, not
an agent's voice, and an agent of kind `acp` has none of its own. An agent
without a voice of its own (`voice` absent from its `capabilities` in `ready`)
is read aloud in the default agent's voice. When neither has one, read-aloud
yields `503 speech_not_configured` naming the agent asked for (`agent_id` among
the refusal's numbers), and dictation the same for the default agent.

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

### Assistant folders

Remote `files_list`, `files_search` and `files_read` read, never change, the
folders the user chose in Ajustes with the macOS panel. That choice is the only
consent: there is no confirmation, expiry or switch. No macOS permission is
consulted; the privacy stop refuses them and a missing helper is
`helper_not_running`. Parameters (pinned in `fixtures/device-bridge-request-v1.json`):

- `files_list`: `{}` lists the folders themselves, each entry named by its
  whole path, with `unavailable:{kind,…}` when its bookmark does not open;
  `{"path":<absolute>,"after_index"?:int ≥ 0}` lists one folder inside them.
  Result `{path?,entries:[{name,kind:"file"|"folder"|"link"|"other",
  size_bytes?,modified_ms?,unavailable?}],total,after_index,next_after_index?}`,
  sorted by name, at most 200 entries (`MAX_LIST_ENTRIES`) and 96 KiB
  (`MAX_PAGE_BYTES`) per page. A link is listed, never followed.
- `files_search`: `{"query":<1–256 bytes>,"path"?:<absolute>,"content"?:bool}`.
  Names match ignoring case and accents; with `content` also the lines of
  UTF-8 files without NUL. Result `{query,path?,content,results:[{path,kind,
  size_bytes?,line?,snippet?}],visited,files_read,bytes_read,partial_files,
  stopped?:{cause,limit}}`. `cause` is `results` (50), `visited` (20,000
  entries), `files_read` (1,000), `bytes_read` (32 MiB), `deadline` (10,000 ms)
  or `page_bytes`; each file is searched in its first MiB, and
  `partial_files` counts those read in part. A search never follows a link.
- `files_read`: `{"path":<absolute>,"after_index"?:int,"after_page"?:int}`.
  Result `{path,size_bytes,modified_ms?,content,…}` where `path` is canonical:
  `content:"text"` with `text,after_index,next_after_index?` (48,000 bytes per
  page, `MAX_TEXT_PAGE_BYTES`, cut at a character); `content:"pdf"` with
  `pages:[{page,text}],page_count,after_page,after_index,next_after_page?,
  next_after_index?` (PDFKit text, at most 50 pages per answer); or
  `content:"image"` with `capture_id,width,height,mime_type:"image/png",
  sha256,source_width,source_height`. The image is fitted within 1280×1024 and
  700,000 bytes and uploaded by `PUT /v1/captures/{capture_id}` like a
  screenshot; the hub swaps `capture_id` for `data_base64`. A PDF over 64 MiB
  or an image over 32 MiB is refused before it is opened. Text has no file-size
  ceiling: it is read a page at a time.

Paths are absolute, at most 1024 bytes (macOS `PATH_MAX`). The Mac resolves the
path with `realpath` and admits it only when that canonical path is a folder or
lies inside one, by whole components; a link or `..` that leads out is refused
like any other path outside. Refusals: `no_folders`; `outside_folders`
`{path,resolved,folders}` (the path asked, where it led when the path as
written lies inside a folder — a link in it — and that differs, and every
folder); `folder_unavailable {path,cause}` with `cause.kind`
`bookmark_unresolvable {domain,code}`, `not_a_folder`, `access_denied {errno}`,
`path_too_long`/`bookmark_too_large {bytes,max_bytes}`;
`file_unavailable {path,cause}` with `cause.kind` `not_found`, `not_a_file`,
`not_a_folder`, `not_text`, `pdf_unreadable`, `image_unreadable`,
`image_does_not_fit {smallest_png_bytes,max_png_bytes}`, `changed`,
`read_failed {errno}`, `past_end {after_index,size_bytes}`,
`not_character_boundary {after_index}`, `past_last_page {after_page,page_count}`;
and `file_too_large {path,content,bytes,max_bytes}`. A path written outside
every folder gets the same `outside_folders` with `resolved:null` whether it
exists or not; a missing path written inside a folder, in any letter case, is
`not_found`. A request the helper has not answered within 20 s, both of its
steps counted from its arrival, is `timeout {operation:"files",waited_ms,deadline_ms}`;
a helper that restarts mid-request is `read_interrupted` (`helper_restarted`),
one that is gone `helper_not_running`. A folder removed meanwhile is not read
or answered from: the request is decided with the folders that remain.

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

A track may add `missing_sequences`: chunks the device kept but the hub refused
for good, in increasing order within `0..last_sequence`, at most 1 024 per track
(`MAX_DECLARED_MISSING_CHUNKS`). Then `chunk_count` and `total_frames` count the
chunks the hub holds, `last_sequence` is the last one the capture produced, and
the hub must hold every other sequence up to it; anything else is
`manifest_mismatch`, and an unordered or oversized list `invalid_manifest`. The
hub transcribes around each hole. `GET /v1/recordings/{uuid}` then says, in
`transcript.missing_audio`, each hole's `track`, `at_ms` from the meeting's
start, `duration_ms` (each `null` when the chunks around it do not tell) and its
`chunks`; the note's transcript carries a line at that time saying the audio is
missing. The field is additive: a manifest without it (a 0.10.1 device) is
checked and stored exactly as before, so its retried closure still matches.
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
- A series' template and dictionary saved together, from one of its meetings:
  `PUT /v1/recordings/{uuid}/series` (Meetings for the Mac).

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
  completion, preserving edits in `superseded_edits`. A later regeneration
  keeps the sets an earlier one set aside under `superseded_edits.earlier`,
  newest first; the top-level `title`/`sections` are always the latest.
  Same language: `requeued:0`. Failed textless groups wait for explicit Retry,
  which uses the new language. Retained audio is never deleted; unreadable
  `invalid_group_audio` leaves everything unchanged and yields
  `409 meeting_audio_unreadable` `{groups,total,cause:"invalid_group_audio"}`.
- `PUT /v1/recordings/{uuid}/private-notes` `{text,base_revision}` → `{revision}`.
  At most 65536 characters. A stale revision yields `409 stale_revision`
  `{revision,base_revision}`. Generation reads these notes. Legacy note-request
  private text is preserved at revision 0.
- `GET /v1/series/previous-meeting?calendar_item_id=&before_ms=` →
  `{calendar_item_id,before_ms,previous}`: the latest closed meeting (not a
  voice call, not an open recording) whose manifest event carries that series
  key and that started before `before_ms`, latest start then highest recording
  ID. Hoy shows its open tasks as «Pendiente de la última vez» for the next
  meeting of the series. `previous` is null when the series has no such
  meeting — a fact, not an unknown — or
  `{recording_id,title,started_at_ms,note}`. `note` is null when nobody asked
  for it, otherwise `{state,revision,short_summary,action_items}`; until the
  note is written `short_summary` and `action_items` are null (unknown, never
  an empty list). Each task is `{id,owner,text,due,done}`, as the note's Actions
  tab has it with the user's marks; marking one is the note's own
  `PUT …/note/edits` `{base_revision: revision, done:{id:bool}}`, which
  announces `recording.changed` for that recording. Both query parameters are
  required: a missing one, or a key that is not one (empty, control
  characters, over 1024 bytes with `bytes,max_bytes`), yields
  `400 invalid_parameters` `{field,…}`. A hub before this route answers 404
  without a code; the Mac reads that as unknown, never as «no earlier meeting».
- `PUT /v1/recordings/{uuid}/series` `{template_id,dictionary_id:string|null}`
  or `{template_id,dictionary_ids:[string]}` →
  `{recording_id,calendar_item_id,template_id,dictionary_id,dictionary_ids}`:
  «Usar lo mismo en toda la serie». The meeting's series takes this template
  (the link of `PUT /v1/note-template-links`) and these dictionaries (`select`
  with `use_for_series:true`; null or `[]` leaves the series without one) in one
  transaction: either both are saved or neither. `dictionary_ids` in the reply
  is in the device's dictionary order and `dictionary_id` is its first, or null.
  The body carries one of the two fields, never both. The meeting's own dictionary
  choice is cleared so it follows its series; written notes keep their template
  until written again. Refusals name their cause and numbers, and nothing is
  saved: `404 recording_not_found`, `409 meeting_series_missing` (the meeting
  has no calendar series), `404 note_template_not_found` `{template_id}`,
  `409 note_template_links_full` `{links,max_links}`,
  `404 dictionary_not_found`, `400 invalid_dictionary_id`,
  `409 dictionary_series_full` `{series,max_series}`,
  `413 dictionaries_too_large`/`dictionary_too_large` `{bytes,max_bytes}`,
  `400 invalid_parameters` `{field}` (`dictionary_id` when neither field is
  sent, `dictionary_ids` when both are or the list names over 32, with
  `{ids,max_ids}`). The language is not a property of the
  series: it stays the recording's (`PUT …/language`). A hub before this route
  answers 404 without a code; the Mac names it `hub_route_missing`.
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

A meeting, or a series, uses several dictionaries. Their order is always the
device's dictionary order, the order of `GET /v1/meeting-dictionaries` (oldest
first), whatever order a request names them in. When two of a meeting's
dictionaries correct the same heard form (case-insensitively) to different
words, the first in that order wins; adding such a word answers with a
`conflicts` warning. Hubs up to 0.10.1 took one dictionary per meeting and per
series: their stored choices read as lists of one, and the single-dictionary
shapes below keep working for Macs up to 0.10.1. A Mac knows a hub takes lists
when its meeting documents carry `dictionary.dictionary_ids`; with an older hub
it sends one dictionary.

- `GET /v1/meeting-dictionaries` →
  `{dictionaries:[{id,name,terms:[{id,correct,wrong:[string]}],
  series:[{calendar_item_id,title}]}]}`. A series can appear in several.
- `POST /v1/meeting-dictionaries/orders` takes one order:
  - `{action:"save",dictionary_id,name}` creates/renames. The client chooses a
    UUID; repetition preserves entries/series.
  - `{action:"select",recording_id,dictionary_ids:[string],use_for_series:false}`
    sets the meeting's own list; `[]` removes it and the meeting inherits its
    series' list. With `use_for_series:true` the series' list becomes exactly
    these (`[]` releases the series) and the meeting's own list is cleared.
    Reply: `{dictionaries:[…]}`, the chosen ones in device order.
    The single form `{…,dictionary_id:string|null,…}` is a list of one (null:
    `[]`) and replies as before: the dictionary, or `{dictionary_id:null}`.
  - `{action:"correct",recording_id,dictionary_ids:[string],wrong,correct,name?:string|null}`
    saves one correction in every listed dictionary in one transaction: all or
    nothing. `name` creates, under that name, the listed dictionaries that do
    not exist yet; without it an unknown one is `404 dictionary_not_found`. The
    meeting then uses every listed dictionary: those it did not use are added
    to its own list (an inherited list becomes its own). Reply:
    `{dictionaries:[…],conflicts:[…]}`.
    The single form `{…,dictionary_id,…}` behaves as in 0.10.1: it keeps a
    meeting choice, explicit or inherited, that already has that dictionary,
    and otherwise makes that dictionary the meeting's only one. It replies with
    the dictionary and `conflicts`.
    Wrong forms for the same correct form share an entry; the corrected entry
    moves to the most recent end; a form moves out of any other entry of the
    same dictionary.
  - `{action:"remove_correction",dictionary_ids:[string],wrong,correct}` is
    «Deshacer» a correction: in each listed dictionary, `wrong` leaves the
    entry of `correct`, and an entry left without forms goes. It does not give
    back a mapping the correction replaced. Repetition is harmless; an unknown
    dictionary is `404 dictionary_not_found` and nothing changes. Reply:
    `{dictionaries:[…]}`.
  - `{action:"remove",dictionary_id,entry_id}` removes the entry and all wrong
    forms; repetition is harmless.
  An order names its dictionaries with `dictionary_ids` or `dictionary_id`,
  never both: `400 invalid_dictionary_order` `{field:"dictionary_ids"}`; with
  neither, `{field:"dictionary_id"}`. A list over 32 is refused with
  `{field:"dictionary_ids",ids,max_ids}`, and `correct` or
  `remove_correction` with an empty list with `{field:"dictionary_ids"}`.
  Repeated ids count once.

Each conflict is `{wrong,correct,dictionary_id,other_dictionary_id,other_name,
other_correct,applied_dictionary_id}`: the correction just saved in
`dictionary_id`, another dictionary of the meeting that hears `wrong` as
`other_correct`, and the dictionary whose correction the meeting shows (the
first in its list). `[]` when there is none.

The series becomes known at finalization. Open drafts have no series hint;
after closure, retries and final-pass windows can use it. Chunk uploads carry
no dictionary or series data.

Meeting documents add `dictionary:{dictionary_id:string|null,
dictionary_ids:[string],source:"requested"|"series"|null,
series:{calendar_item_id,title}|null}`. `dictionary_ids` is the meeting's list
in device order (its own, else its series'), `dictionary_id` its first or null.
An empty list means no dictionary; an absent `dictionary` means unknown, for
example an older hub or a Mac without the hub read, and an absent
`dictionary_ids` a hub up to 0.10.1.

`turns.py` combines the meeting's dictionaries, the first winning each heard
form, and replaces whole words/phrases in one case-insensitive pass, longest
phrase first. App, notes, questions and `orbit_transcript` share this read.
Segment output applies corrections after final-pass words are joined.
Originals are unchanged; removing an entry reverses the correction.
Existing notes retain text until regenerated.

The note model receives the correct forms of all the meeting's dictionaries in
all parts and reductions. Draft groups and final windows send complete,
most-recent terms in a multipart `prompt` bounded to 160 characters including
`Glosario: `, separators and final period: `Glosario: term, term.`. The first
dictionary's terms count as the most recent. No terms means no prompt. This
conservatively budgets for Whisper's last 223 tokens without a tokenizer; it
does not guarantee a token count for arbitrary Unicode. Repeated recognition
rereads the hint.

Hub-only bounds: 32 dictionaries, 128 terms each, eight forms per term,
128 characters per form, 80 per name, 256 total series; 192 KiB per dictionary
(one item of the 256 KiB library read), 768 KiB per HTTP list, 16 KiB per order.
Refusals include `code`, a content-free message and numbers:
`dictionary_full {terms,max_terms,dictionary_id}`,
`dictionary_text_too_long {field,characters,max_characters}`,
`dictionary_forms_full {forms,max_forms,dictionary_id}`,
`dictionaries_full {dictionaries,max_dictionaries}`,
`dictionary_too_large {bytes,max_bytes,dictionary_id}`,
`dictionaries_too_large {bytes,max_bytes}`,
`dictionary_series_full {series,max_series}`. `dictionary_id` names which
dictionary of an order into several is full.
`dictionary_series_missing` means the series is unknown, not a zero count.
The Mac carries numbers to the UI, and a correction's `conflicts` too, by the
order's target, to show beside it. On order confirmation the daemon invalidates
all meeting/library reads once, as for templates, rather than one WebSocket event
per historical meeting. app-core does not reuse closed turns when a dictionary
exists or that fact is unknown. Unconfirmed orders retain the written correction
for resending and report the transport cause.

## Request transcription again

`POST /v1/recordings/{id}/final-pass` uses the owning device's bearer token.
The hub runs the pass; the Mac only asks for it and reads its state.
It creates a pass or requeues a terminal `completed`/`failed` pass.
An already `queued`/`running` pass is not duplicated.
Reply: `{recording_id,requested:true}`. It uses the selected meeting language
or configured fallback and all the meeting's dictionaries as the pass runs,
keeps prior text until a complete new publication, and reuses voices already
separated from the same audio.

Transcribing again does not rewrite the note, nor ask for one: the note and its
edits stay as they are. Hubs up to 0.10.1 requested an already-requested note
again after the pass. Rewriting it is its own order, `POST …/note` («Rehacer
resumen»), which keeps the version edited by hand as `superseded_edits`.

`recording.changed` announces queue, start, each window transcribed and end.
The document carries `transcript.final_pass:{state,error,progress?}`
(`queued|running|completed|failed`); absence means no registered pass. A
running pass carries `progress:{done_ms,total_ms,remaining_ms?}`: `done_ms` of
`total_ms` of audio transcribed (the fraction for a progress bar), and, once a
window is done, `remaining_ms`, the time taken so far scaled to the audio
left. `total_ms` starts from the closing manifest (each track at most the
recording's length) and becomes each track's measured length when the pass
reads it. A queued pass carries no progress: its wait depends on other
passes. Hubs up to 0.10.1 send no `progress`. A `completed` pass that kept
spans of the live draft, because the new text lost them or looped over them,
carries `kept_draft:{spans,duration_ms}`: how many draft spans it kept and their
summed length. Its absence means the new text was used throughout, as from every
earlier hub, which never mixed the two; a Mac that does not know the field sees
a normal `completed`. `transcript.unmatched_speaker_names`
lists names whose voices no longer exist; they are not assigned to new voices.
The microphone track name is retained.

Before queueing, refusals are `400 recording_still_open`,
`409 meeting_audio_unreadable` (`groups,total` for invalid audio), and
`409 diarization_not_configured`. Unknown/cross-device IDs remain
`404 recording_not_found`. Failures retain prior text and carry code, phase and
the cause's measured numbers: `final_pass_timeout` `{timeout_ms}` (it took
longer than the hub allows), `final_audio_invalid` (audio the hub cannot read),
and any other code for a pass that could not complete. Hubs up to 0.11.0 also
failed `final_pass_incomplete` when the new text lost parts of the previous
one; this hub keeps those parts from the draft instead (below).

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

Before attribution/publication, each track is compared with its own draft, span
by span (`meeting_transcript_integrity.choose`), including groups with text
awaiting retranscription. Timing matches `meetings/turns.py` (`started_at`,
`_length_ms` and group manifest start). Only groups with at least six words can
be lost: if word-start times within the group interval ±1000 ms inclusive
contain fewer than one quarter of its draft words, that span is lost. A span
loops when the new words starting inside it repeat over both 66 words and 223
permille of them; a repetition that runs on into the next spans loops over them
too. Loop counting ignores punctuation and case. Groups without text do not
count: absence is not proof of loss. Sound is not used to infer missing speech;
the pass does not retry ASR.

A lost or looping span takes its draft words, spread evenly over the span
(drafts carry no word times); every other span, and any word outside all draft
spans, keeps the new words. A track whose every span that can be lost is lost
keeps its whole draft. Voices are separated for `remote_app` whatever was
kept, and attribution applies to the words that remain, by their times, so
kept draft words get their “Persona N” too. The pass completes and reports
`kept_draft` (above). Windowing failures use `windows_<track>` and preserve
ffmpeg causes/numbers. These checks guarantee neither WER nor semantic accuracy.

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

A hub can offer several agents, each an assistant with its own backend: a
Hermes profile, a model the hub runs against an OpenAI-compatible or Anthropic
API, or Claude Code or Codex over the Agent Client Protocol. The addition is
optional on both sides: a 0.9.0 Mac keeps working against a hub with agents,
and a Mac with agents keeps working against a 0.9.0 hub.

**`ready`** adds `agents`, the hub's agents in its order, and
`default_agent_id`. Each agent is `{id, name, kind, capabilities}`: `id` is
stable, `[a-z0-9-]`, 1 to 32 characters; `name` is what the app shows and what
the wake phrase hears («Oye Atlas»); `kind` is `hermes`, `openai` or `acp`,
and a Mac must accept any other text there;
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
  `refusal:{"code":"agent_mismatch","agent_id":"vega","session_agent_id":"atlas","retry":{"kind":"after_fix"}}`.
- An `agent_id` the hub does not have is refused with
  `error.code:"agent_not_found"` and
  `refusal:{"code":"agent_not_found","agent_id":"orion","agent_ids":"atlas,vega","retry":{"kind":"after_fix"}}`,
  the ids there are in the hub's order, joined by commas.

A refusal's numbers stay integers, strings or booleans, so the ids are joined
rather than listed: the Mac's reader of a refusal drops an array. Neither
refusal runs a turn. The Mac sends `agent_id` on every `chat.send` to
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


## Agent configuration

Settings › Agents reads and changes, for each agent of `ready`, its
**personality** (Markdown the agent is given as its instructions), its
**skills** (files in the [agentskills.io](https://agentskills.io) `SKILL.md`
format) and its **access groups** (which of Orbit's tools it may use). The
routes use the device bearer token, like [Meeting dictionaries](#meeting-dictionaries);
any paired Mac configures every agent of its hub. All of it is optional: a Mac
that never calls these routes leaves every agent as it was, and a hub without
them answers `404` with no `code`.

Where each kind keeps them:

- `openai` and `acp`: the hub's database (personality and skill files
  encrypted). The agent is told its personality and the name and
  description of each enabled skill, and loads a skill's instructions with
  `orbit-mcp`'s `orbit_skill` (see [Access groups](#access-groups)) when it
  needs them. An `openai` agent is told at every turn; Claude Code in each
  session's `_meta.systemPrompt.append` (`session/new` and `session/load`);
  Codex in the `AGENTS.md` of its folder, written before each session opens.
  A conversation already open keeps what it started with.
- `hermes`: the profile's own `SOUL.md` and skills, through Hermes's panel
  (see Hermes refusals below). Nothing of them is stored in the hub.
- Access groups: the hub's, for every kind.

### Read

`GET /v1/agents/{agent_id}/config` →

```json
{"agent_id":"lyra","kind":"openai",
 "personality":{"text":"Eres Lyra. Hablas claro y breve.","editable":true,"max_bytes":16384},
 "access":{"groups":{"screen":true,"control":false,"calendar":true,"meetings":true,"files":true},
           "editable":true},
 "skills":{"editable":true,"max_skills":128,"max_skill_bytes":65536,
           "items":[{"name":"resumen-semanal","description":"Resume la semana en cinco puntos.",
                     "enabled":true}]}}
```

`items` are ordered by name. An agent nobody configured has `text:""`, no
items and every group `true`. `editable` says whether this kind of agent lets
Orbit change the field; when it is `false`, the field also carries `cause`, a
Spanish sentence. Every kind lets Orbit change all three today, so it is
always `true`; a Mac must still honour `false`.

`GET /v1/agents/{agent_id}/skills/{name}` →
`{"name","description","enabled","content"}`, `content` being the whole
`SKILL.md`.

### Write

Each write answers `200` with the agent's whole configuration, exactly as
`GET …/config`. A refused write changes nothing.

- `PUT /v1/agents/{agent_id}/personality` `{"text":"…"}`. An empty text
  clears it.
- `PUT /v1/agents/{agent_id}/access`
  `{"groups":{"screen":true,"control":false,"calendar":true,"meetings":true,"files":true}}`:
  the five groups, each a boolean; a missing or extra key is refused.
- `PUT /v1/agents/{agent_id}/skills/{name}` `{"content":"---\nname: resumen-semanal\ndescription: Resume la semana en cinco puntos.\n---\n1. Lee la agenda…"}`
  creates the skill, enabled, or replaces its file keeping whether it is
  enabled. A `name` with an encoded slash (`mis%2Fskill`) is `422
  skill_invalid` with `rule:"name_characters"`, as any other character
  outside the standard's.
- `PUT /v1/agents/{agent_id}/skills/{name}/enabled` `{"enabled":false}`.
- `DELETE /v1/agents/{agent_id}/skills/{name}`. Deleting a skill that is not
  there is `skill_not_found`, so a Mac that lost the answer to a delete can
  read that refusal as already done.

Malformed JSON or a body outside these shapes is `400 invalid_agent_config`
with the field. A body over its ceiling is `413 too_large`
(`resource:"hub_request"`): 6 × the text's bound plus 1 KiB, room for a text
whose every character JSON-escapes to six bytes.

### Skills

A `SKILL.md` starts with a YAML frontmatter between `---` lines; Orbit checks
the standard's rules on its two required fields, and keeps any other key
(`license`, `allowed-tools`, `metadata`, an agent's own) as written:

| rule | what it refuses |
|---|---|
| `frontmatter_missing` | the file does not start with a `---` line |
| `frontmatter_unclosed` | no `---` line closes the frontmatter |
| `frontmatter_yaml` | the frontmatter is not YAML, or not a mapping |
| `name_missing` | no `name`, or not text |
| `name_too_long` | more than 64 characters (`characters`, `max_characters`) |
| `name_characters` | anything but `a-z`, `0-9` and `-` |
| `name_hyphen_edge` | a name that starts or ends with `-` |
| `name_consecutive_hyphens` | `--` in the name |
| `name_mismatch` | the frontmatter's `name` is not the route's `{name}` |
| `description_missing` | no `description`, or blank |
| `description_too_long` | more than 1024 characters (`characters`, `max_characters`) |

The route's `{name}` is held to the same name rules. Orbit admits only ASCII
letters, where the standard admits any lowercase letter: the name is a folder
in an `acp` agent's directory and a segment of this URL.

### Bounds and refusals

- Personality: 16 KiB of UTF-8. It is sent to an `openai` agent's model on
  every turn (about 4,000 tokens, an eighth of a 32,000-token window) and
  read by an `acp` agent at each session.
- One `SKILL.md`: 64 KiB. The standard asks for under 500 lines; Hermes's
  longest bundled skill measured 35,175 bytes.
- Skills per agent: 128. It bounds this read (under 1 MiB even at the
  standard's maxima) and the list an `openai` agent is told every turn.

Refusals carry `code`, a Spanish `message` without the personality's or
skill's text, and their numbers in `refusal`, all scalars (a Mac decodes no
array there):

| status | code | numbers |
|---|---|---|
| 404 | `agent_not_found` | `agent_id`, `agent_ids` (comma-joined, in the hub's order) |
| 403 | `skill_delete_unsupported` | `agent_id`, `kind` (`hermes`: its panel cannot delete a skill) |
| 422 | `skill_invalid` | `name`, `rule`, and for the length rules `characters`, `max_characters` |
| 404 | `skill_not_found` | `name` |
| 413 | `skill_too_large` | `name`, `bytes`, `max_bytes` |
| 409 | `too_many_skills` | `count`, `max` |
| 413 | `personality_too_large` | `bytes`, `max_bytes` |

```json
{"code":"skill_invalid","message":"La skill «resumen-semanal» no cumple el estándar SKILL.md: la cabecera dice name: otra, y la skill se llama resumen-semanal",
 "refusal":{"code":"skill_invalid","name":"resumen-semanal","rule":"name_mismatch","retry":{"kind":"after_fix"}},
 "op":"…"}
```

### A Hermes agent

The hub reaches the profile's `SOUL.md` and skills through Hermes's panel API
with the credential it already holds for Hermes (`X-Hermes-Session-Token`):
`GET`/`PUT /api/profiles/{name}/soul`, `GET /api/skills`,
`GET /api/skills/content`, `POST /api/skills` (create), `PUT /api/skills/content`
(replace) and `PUT /api/skills/toggle`. Those routes carry no version, so they
are checked against the installed Hermes at each release. The panel has no
route to delete a skill: `DELETE` on a `hermes` agent's skill is `403
skill_delete_unsupported` with `kind:"hermes"`. Orbit's own rules
and bounds apply before anything is sent to Hermes; creating a skill counts
the profile's skills against the 128.

| status | code | numbers |
|---|---|---|
| 502 | `assistant_admin_refused` | `route` (`"PUT /api/profiles/{name}/soul"`, as written above, never the profile or skill), `status` (Hermes's HTTP status) |
| 503 | `assistant_admin_unreachable` | `route` |
| 504 | `assistant_admin_timeout` | `route`, `timeout_ms` |
| 502 | `assistant_admin_invalid` | `route`: Hermes answered something the hub cannot read |
| 502 | `assistant_admin_too_large` | `route`, `bytes`, `max_bytes` (1 MiB per answer) |

### Access groups

| group | orbit-mcp tools | bridge |
|---|---|---|
| `screen` | `orbit_windows`, `orbit_screenshot`, `orbit_look`, `orbit_selected_text` | `screen_sources`, `screenshot`, `selected_text` |
| `control` | `orbit_control_tools`, `orbit_perform_action`, `orbit_action_result` | `control_tools`, `perform_action`, `/internal/v1/device/action-result` |
| `calendar` | `orbit_calendar` | `calendar_read` |
| `meetings` | `orbit_meetings`, `orbit_transcript`, `orbit_note` | `/internal/v1/meetings/*` |
| `files` | `orbit_files_list`, `orbit_files_search`, `orbit_files_read` | `files_list`, `files_search`, `files_read` |

`orbit_devices` and `orbit_skill` belong to none: the first names the Macs,
the second reads the agent's own skills. Every group is on by default. A group off is
a configuration of the agent's tools: nothing is asked at each use, nothing
expires, and macOS's permission stays the decision about the Mac.

It is enforced twice. `orbit-mcp` knows its agent from `ORBIT_MCP_AGENT`,
which the hub sets for `openai` and `acp` agents and a Hermes profile's MCP
configuration sets for a `hermes` agent; without it, the default agent. It
asks `POST /internal/v1/agent/access`, signed with the device-bridge key over
`{"protocol":"orbit_device_bridge_v1","agent_id"?:"lyra"}`, which answers
`{"protocol":"orbit_device_bridge_result_v1","state":"answered","agent_id":"lyra","groups":{…},"skills_in_hub":true}`
(no `agent_id`: the default agent, named in the answer). `orbit-mcp` then
lists no tool of a group that is off, and refuses one called anyway with
`agent_access_disabled`; it lists `orbit_skill` only when `skills_in_hub`
is true (`openai` and `acp` agents; a `hermes` agent's skills are its
profile's). When the hub cannot say, it offers every tool and the hub
decides. It reads the answer again after 10 s.

The hub checks each request: the device-bridge request, the action-result
read and the meeting reads take an optional `agent_id`, which `orbit-mcp`
sends when it has one (a request without it is the default agent's, as
before). A request of a group that is off is `403 agent_access_disabled` with
`agent_id` and `group`; an `agent_id` the hub does not have is `404
agent_not_found` with `agent_id` and `agent_ids`. Neither reaches the Mac.

Both checks trust the agent id `orbit-mcp` signs with the bridge key: the
groups configure a cooperative agent's tools, and they are not a boundary
against a hostile process on the same machine that holds that key.

## Storage

**`ready`** adds `storage`, what the hub's database uses and its limit:
`{"used_bytes":1810000000,"max_bytes":82000000000,"remaining_bytes":80190000000,
"limit":"disk","nearly_full":false}`. `limit` is `quota` when
`storage_quota_bytes` sets it and `disk` when the disk does; `nearly_full`
is the hub's own reading (from 80 % of the limit). When `nearly_full`
changes while a Mac is connected the hub sends `storage.status` with the same
object and `id:null`. A 0.10.1 hub sends neither; a 0.10.1 Mac ignores the
field and records the event as unrecognized. A Mac that cannot read `storage`
leaves it out and keeps the `ready`. The Mac says a nearly full hub once,
with the numbers and the fix, and again only after the hub has had room.

## Assistants from Orbit

Each agent has a **profile** the hub keeps over what its configuration says
(how Orbit shows and calls it, never the agent itself: renaming a Hermes agent
does not touch Hermes), and Claude Code and Codex agents can be **added,
signed in and out and removed** from Orbit. Everything here is additive: a Mac
that knows none of it keeps working, and a hub that sends no `addable_agents`
in `ready` has none of these routes (`404` with no `code`) and takes no
`handover_from`.

**`ready`** and **`agents.changed`** carry, for each agent, besides `id`,
`name`, `kind` and `capabilities`:

| field | |
|---|---|
| `service` | what answers: `hermes`, `openai`, `claude-code` or `codex`; a Mac accepts any other text |
| `description` | at most 90 characters, possibly empty |
| `color` | `c1` to `c4`; until someone chooses one, the four in turn by position |
| `removable` | added from Orbit, so Orbit can remove it |
| `account` | only for `acp` agents: `signed_in`, `signed_out`, `expired` (it refused for want of a sign-in after having one) or `unknown` (the hub has not learned it yet) |
| `account_email`, `plan` | only when the agent's adapter said them (`_auth/status_update`) |
| `limit` | only while the plan is at its limit: `{"until_ms": integer\|null}`, null when the provider did not say when it resets; a Mac treats one whose time has passed as over |
| `usage` | only when the provider reported it: `{"used_percent": 0-100, "window": "five_hour"\|"seven_day"\|…\|null, "resets_at_ms": integer\|null}`. Today only Claude Code with a claude.ai subscription reports it, when it changes; Codex reports none, so a Codex agent has no usage |

`default_agent_id` is the **primary**: the agent a turn without `agent_id`
goes to, and the one the wake phrase without a name, the floating panel and
«Preguntar» use. `assistant_name` is the primary's name. `addable_agents`
lists the services the hub has an adapter for, possibly none.

`agents.changed` (`id:null`) is sent to every connected device whenever the
list changes: a profile, an agent added or removed, a sign-in or sign-out, an
account the hub learned (expired, at its limit, its address), a usage report.
Its `data` is `{"agents":[…],"default_agent_id":…,"addable_agents":[…]}`,
exactly as `ready` gives them; a Mac before it ignores the event.

```json
{"v":1,"id":null,"event":"agents.changed","data":{
  "agents":[
    {"id":"atlas","name":"Atlas","kind":"hermes","capabilities":["images","voice","resume"],
     "service":"hermes","description":"Tu asistente de siempre","color":"c1","removable":false},
    {"id":"codex-1a2b3c4d","name":"Nova","kind":"acp","capabilities":["images","resume"],
     "service":"codex","description":"Programa conmigo","color":"c2","removable":true,
     "account":"signed_in","limit":{"until_ms":1790000000000}}],
  "default_agent_id":"atlas","addable_agents":["codex"]}}
```

### Routes

All use the device bearer token. Each write answers the whole list as
`agents.changed` carries it (`POST /v1/agents` adds the new `agent_id`).

- `GET /v1/agents` → the list.
- `PUT /v1/agents/{agent_id}/profile`
  `{"name":"Nova","description":"Programa conmigo","color":"c2","primary":false}`:
  all four. The name is trimmed, 1 to 24 characters without control
  characters, and no other agent's said the same (ignoring case and accents,
  because the wake phrase hears it). The description is at most 90
  characters on one line. `primary:true` makes it the primary; the primary
  stops being it only when another one is made primary. A name set in
  `orbit.toml` longer than 24 stays until renamed here.
- `POST /v1/agents`
  `{"service":"codex","name":"Nova","description":"","color":"c2","access":{"screen":true,"control":false,"calendar":true,"meetings":true,"files":true}}`
  → `201` with `agent_id`. The Mac chooses only the service, the profile and
  the access groups: the command comes from the hub's `[acp_adapters]`, and
  the id (`codex-` or `claude-` and eight hex digits) and its folder are the
  hub's. It starts signed out. A hub offers at most 32 agents, its
  configuration's and the added ones together, and refuses to start with
  more.
- `DELETE /v1/agents/{agent_id}`: only an agent added from Orbit. Its
  processes stop, its folder (with its account's credentials) is removed, and
  the hub forgets its profile, access, personality and skills. Its
  conversations stay, listed with its `agent_id`; a turn in one is
  `agent_not_found`.
- `POST /v1/agents/{agent_id}/sign-in` → a sign-in (below), started or the
  one already waiting. `GET` reads it, `DELETE` cancels it: the vendor's tool
  is stopped and the agent's account stays as it was. The Mac reads it again
  until it is no longer `waiting`; `done` is also published as
  `account:"signed_in"`.
- `POST /v1/agents/{agent_id}/sign-in/code` `{"code":"<code>#<state>"}`:
  for Claude Code, the code its page shows after signing in, only URL-safe
  characters around one `#`.
- `POST /v1/agents/{agent_id}/sign-out`: the vendor's own sign-out, which
  revokes the account and deletes its credentials; the agent's processes
  stop. → the list, the agent `signed_out`.

A sign-in:

```json
{"agent_id":"codex-1a2b3c4d","state":"waiting","expires_at_ms":1790000900000,
 "expects_code":false,"url":"https://auth.openai.com/codex/device","code":"ABCD-1234"}
```

`state` is `waiting`, `done`, `cancelled` or `failed`; a Mac reads another as
unknown. While waiting: `url`, the address the Mac opens in the browser;
`code`, Codex's one-time code the user types there; `expects_code`, true for
Claude Code, whose page shows a code the user pastes back into Orbit; and
`expires_at_ms`, when the hub stops it (15 minutes, the life of Codex's code).
A failed one has `cause` and, when the tool ended with one, `exit_status`:

| cause | |
|---|---|
| `device_code_disabled` | the ChatGPT account does not allow sign-in with a device code (its security settings) |
| `expired` | the vendor's code expired before the user finished |
| `code_rejected` | the code pasted into Orbit was not accepted |
| `login_disabled` | the vendor's configuration forbids this way of signing in |
| `timed_out` | 15 minutes passed |
| `no_url` | the tool showed no address in 30 seconds |
| `no_credentials` | the tool said it finished but wrote no credentials |
| `exited` | the tool ended with an error the hub does not tell apart |
| `not_started` | the tool did not start (also refused at once as `sign_in_failed`) |

The hub runs the vendor's tool through the agent's own adapter, in the
agent's folder; the tool writes the account's credentials there. The hub
never reads, logs or returns them, and nothing of the tool's output reaches a
Mac or a log but the address, the code and the cause.

### Handing a conversation over

`chat.send` takes `handover_from`, a conversation of this device, beside
`agent_id` and without `session_id` or `meeting`: the turn opens a new
conversation with `agent_id` whose first message to the agent carries what
was said in the old one (its newest messages that fit in 32,768 characters,
about 8,000 tokens, a quarter of a 32,000-token window, oldest first; those
left out are counted in it), and then the question. An empty `text` carries over the old
conversation's unanswered question (the user's messages after its last
answer); a `text` that repeats it is carried once. The new conversation keeps
only the question as the user's message, and its rows (`GET /v1/sessions`, the
page of `GET /v1/sessions/{id}/messages`) carry
`"handed_over_from":{"session_id":"…","agent_id":"…"}`. The old one stays as
it was. Nothing to carry is `handover_no_question` (`messages`).

### Refusals

| status | code | numbers |
|---|---|---|
| 422 | `agent_profile_invalid` | `field` (`name`, `description`, `color`), `rule` (`empty`, `too_long`, `control_characters`, `unknown_color`), `characters`, `max_characters` |
| 409 | `agent_name_taken` | `field`, `agent_id` (the agent already called that) |
| 409 | `agent_primary_required` | `agent_id` |
| 403 | `agent_add_unsupported` | `service` (`hermes`, `openai`: the UI says «Pídeselo a quien gestiona tu hub») |
| 409 | `agent_adapter_missing` | `service`: no `[acp_adapters]` table for it, or its command does not start |
| 409 | `too_many_agents` | `count`, `max` |
| 403 | `agent_not_removable` | `agent_id`: from the hub's configuration |
| 500 | `agent_folder_not_removed` | `agent_id`, `errno`; the agent stays |
| 409 | `sign_in_unsupported` | `agent_id`, `kind` (not an `acp` agent) |
| 404 | `sign_in_not_found` | `agent_id` |
| 409 | `sign_in_not_waiting_for_code` | `agent_id` |
| 422 | `sign_in_code_invalid` | `agent_id`, `characters` |
| 502 | `sign_in_failed` | `agent_id`, `cause` |
| 502 | `sign_out_failed` | `agent_id`, `exit_status` |
| 504 | `sign_out_timeout` | `agent_id`, `timeout_ms` |
| 400 | `invalid_agent_config` | `field`: a body outside these shapes, a `command` or an `id` included |

## Attachments

A user's message can carry files. The addition is optional on both
sides, as agents are: a 0.9.0 Mac sends no `attachments` and reads pages
without them; a Mac with attachments offers none to a hub whose `ready` has no
`attachments` (0.9.0), because that hub ignores unknown `chat.send` fields and
the turn would reach the agent without its files. The Mac refuses such a turn
locally (`attachment` refusal, cause `unsupported`) before anything leaves.

**`ready`** adds `attachments`, the hub's ceilings:
`{"max_file_bytes":20971520,"max_message_files":10,"max_message_bytes":33554432,
"max_session_bytes":268435456}`. Each is registered in `contracts/constants.json`
with what it protects (`server/orbit_hub/attachments.py`).

**Upload.** The Mac reads a file by its path (or takes the bytes of a pasted
image or text) and uploads it before the turn, over HTTP like audio, because a
frame is at most 128 KiB:

`POST /v1/attachments`, device bearer token, body the file's bytes,
`X-Orbit-Attachment-Name` (the name, percent-encoded UTF-8, 1–255 characters,
no control characters and no `/`) and `X-Orbit-SHA256` (lowercase SHA-256 of
the body). It answers `{"attachment":{"id","name","kind","mime","bytes",
"pages"?}}`. `kind` is what the hub found in the content, never the name:
`image` (PNG, JPEG, GIF or WebP), `pdf` (opened and its text extracted with
pypdfium2; `pages` is its page count) or `text` (UTF-8 without NUL, `mime`
`text/plain`). The file is stored encrypted, unbound, and deleted when no turn
names it within an hour: the hub sweeps every device's every minute.

`POST /v1/attachments/capture` with `{capture_id,request_id,name}` keeps as an
attachment a window capture the Mac uploaded (`PUT /v1/captures/{capture_id}`
bound to `request_id`) for «Adjuntar ventana»: the PNG crosses the network
once and is taken once (`404 capture_missing` `{capture_id}` afterwards). The
window's text, when it has some, follows as an ordinary `POST /v1/attachments`
of a `.txt` with the capture's name. A HEIC photo is turned into JPEG on the
Mac before its upload, so this hub refuses one only from another client.

`GET /v1/attachments/{id}` returns the bytes with their `Content-Type` and the
name in `X-Orbit-Attachment-Name`, for the device that uploaded them.

**`chat.send`** adds `attachments`, the ids in the order the user attached
them (at most 64 in the frame); with them `text` may be empty. The hub binds
them to the message, and so to its session, in the message's own transaction.

**Refusals**, each with its numbers and the file's name where there is one:

- `413 attachments_too_large` `{ceiling:"file",name,bytes,max_bytes}` at upload,
  before the body is read when `Content-Length` says it;
  `{ceiling:"message",files,bytes,max_bytes}` and
  `{ceiling:"session",bytes,session_bytes,max_bytes}` at `chat.send`;
- `attachments_too_many` `{files,max_files}` at `chat.send`;
- `415 attachment_type_unsupported` `{name,mime}` (what the content looks like:
  `application/zip`, `image/heic`, `application/octet-stream`…) at upload;
- `422 attachment_unreadable` `{name,cause}` (`pdf_locked`, `pdf_damaged`) at
  upload, or `{cause:"render_failed"}` at `chat.send`;
- `404 attachment_not_found` `{attachment_id}`: another device's, expired, or
  already sent with another message; for `orbit_attachment`, also one that is
  not in a conversation of the asking agent;
- `attachment_refused` `{name,upstream_code}`: the agent refused an image
  (Hermes `image.attach_bytes` 4016, 4017, 4018); the prompt is not sent.
  From an `openai` agent it is the turn's `error` event, `{name,provider_status}`:
  the provider answered the turn's first request, which carried its images,
  with HTTP 400, 413, 415 or 422 for a cause other than the context; `name`
  lists the turn's images, because the provider does not say which one.

On the Mac these are one typed refusal, `attachment` with `cause.kind`
`too_large`, `too_many`, `type_unsupported`, `unreadable`, `refused`,
`not_found` or `unsupported`; a turn refused for its files keeps the sizes and
counts in its notice (`bytes`, `max_bytes`, `stored`, `count`, `max`).

**What reaches the agent.** The turn's text names every file with its kind,
size and id. An image goes as an image to an agent that takes them (Hermes:
`image.attach_bytes` with a data URL before `prompt.submit`; ACP: an image
content block when the agent declares `promptCapabilities.image`; `openai`:
an image part after the text when the agent has `images = true`; otherwise
the text names it and says the agent cannot see it, since `orbit_attachment`
hands such an agent a note in its place). A PDF goes as its text,
page by page, up to 48,000 characters per turn, and its pages with fewer than
32 characters of text (a scan, a figure) as PNG images, at most 8 per turn,
1,568 px on the long side. A text file is read with `orbit_attachment`.

**`orbit_attachment`** (orbit-mcp) reads one by id through
`POST /internal/v1/attachments/read`, signed with the meetings key
(`orbit_meetings_bridge_v1`), `{device_id,attachment_id,after_index?,agent_id?}`:
an image whole (`data_base64`), the text of a PDF or a text file in pages of
48,000 characters with `after_index`, `next_after_index` and `total_chars`.
`agent_id` is the agent orbit-mcp serves (`ORBIT_MCP_AGENT`; none is the
default agent): only a file sent in a conversation of that agent is read,
anything else is `404 attachment_not_found`, and an unknown agent is `404
agent_not_found`.

`orbit_skill(name)` asks `POST /internal/v1/agent/skill`, signed the same way
over `{"protocol":"orbit_device_bridge_v1","agent_id"?:"lyra","name":"resumen-semanal"}`
(`name` 1–64 characters), which answers
`{"protocol":"orbit_device_bridge_result_v1","state":"answered","agent_id":"lyra","name":"resumen-semanal","instructions":"…"}`
with the `SKILL.md` body after its frontmatter. Nothing reaches a Mac.

| status | code | numbers |
|---|---|---|
| 404 | `skill_not_enabled` | `agent_id`, `name`, `enabled` (the agent's enabled skills, comma-joined in name order; empty when none) |
| 409 | `skills_not_in_hub` | `agent_id`, `kind` (`hermes`: its skills are its profile's) |
| 404 | `agent_not_found` | `agent_id`, `agent_ids` |
