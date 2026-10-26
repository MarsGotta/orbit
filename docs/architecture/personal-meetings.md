# Meetings in a personal hub

A hub belongs to one person. Every authenticated paired Mac reads and edits
that person's meeting history. The recording device remains the storage and
encryption key for its audio, manifest, transcript, note and edits. There is
no data migration, rekeying, schema change or new sharing permission.

Revocation removes access for that device, not ownership of its meetings.
Background transcription can still process audio already on the hub, and
another paired Mac can request retranscription or delete the meeting. A
delete retains the tombstone under the original recording device, so late
chunks and closure retries cannot resurrect it.

The list and open meeting carry optional `recorded_on` metadata when more
than one device has ever paired, including revoked devices. The app displays
«Grabada en …». An open meeting explains that pending audio must upload from
that Mac; another Mac does not gain control of its microphone or local files.
Calendar context comes from the shared manifest, with this Mac's pending
context overlaid only when present.

The hub must upgrade first. Existing Macs ignore the additive fields and
continue using the same authenticated URLs. New Macs tolerate their absence
from an older hub. The assistant bridge adds optional `before_id` alongside
`before_ms`; the MCP tool passes and displays both so meetings starting at
the same time on different Macs remain reachable on later pages.

## Read-scope audit

Every remaining device predicate below is either an authentication boundary
or the original storage key, never a restriction to the requesting Mac's
meetings.

| Entry point / storage reads | Decision |
| --- | --- |
| `meetings/routes.py:170,226,247` and `meetings/views.py:67,123,235` | Authentication stays per device. Listings gather open and closed meetings across all stored devices, sort once across the hub and paginate the combined history. |
| `meetings/routes.py:272,296,313,340,351,375,395,408,425` | Resolve a recording UUID to its original device before document/turn reads, series assignment, deletion, speaker/private-note/title/language changes or final-pass requests. |
| `meetings/ownership.py:10` | Resolve through device-indexed manifests, chunks and deletion tombstones, including revoked devices. Unknown UUIDs retain the caller's key so discarding before upload remains idempotent. A legacy UUID collision is refused with an explicit cause and match count. |
| `notes/routes.py:48,62,68` recording note request/read/edit | Resolve the recording device after authenticating the caller. Notes and revisions remain in their existing rows. |
| `routes/recordings.py:85` transcript retry | Resolve before retrying groups. Finalize deliberately retains the authenticated recording device: uploading and closing another Mac's recording is not a shared editing operation. |
| `meetings/series.py:51` | Search manifests across devices for the prior series or title, decrypt each with its own key, and read that meeting's note/tasks with the matching original device. |
| `meetings/dictionaries.py:212,228,244,282,443,557` | List the person's dictionaries; resolve recording orders to their original device. Meeting-specific dictionary selections remain device-keyed. Updates preserve every dictionary's original encryption/storage key. Legacy duplicate dictionary UUIDs receive an explicit refusal on mutation rather than an arbitrary overwrite. |
| `notes/custom_templates.py:133,148,156,243,325` template and series-link listing/find/resolve/mutations | Templates and series assignments are shared, allowing a template selected on one Mac to write another Mac's meeting. Preserve original row keys. Legacy client-chosen template/link IDs use the most recently updated row, with device ID as a deterministic tie-break; deletion removes every version of that public ID. |
| `meetings/repository.py`, `turns.py`, `muted.py`, `open_recordings.py`, `views.py` document/note/state helpers | Keep device predicates for the resolved original device. Audio groups, source tracks, speaker names, microphone profile, private notes, title, language, final transcript and muted/missing spans must all use the same storage/encryption context. Open-recording discovery runs once for every stored device. |
| `notes/repository.py`, `notes/worker.py`, `group_storage.py`, `meeting_final_pass.py`, `final_pass_audio.py` | Keep device-keyed jobs, reads and receipts. Drop revocation filtering from group preparation/claim and final-pass claim: the person's stored audio still belongs to her. No provider, model or device permission is added. |
| `storage/recordings.py`, `routes/captures.py`, upload/finalize status reads | Retain the recording device scope for chunk admission, upload status, finalization and late-write rejection. These are recording/upload operations, not history reads. |
| `meetings_bridge/service.py:162,199,282` list/note/transcript | Keep signed authentication and agent meetings-access checks. List the whole hub; resolve notes/transcripts independently of the request's device context. Use original microphone/speaker/private-note keys. |
| `meetings_bridge/service.py:104` attachment reads | Retain device and conversation-agent scope. Message attachments are conversation data, outside meeting history. |
| `meeting_context.py:103` (runtime chat questions during a meeting) | Resolve the original recording device before reading named transcript lines and pending counts. The conversation and its authentication still belong to the asking device. |
| `runtime/registry.py:56` | Broadcast recording-change invalidation to every connected Mac. Never send meeting content in the event. |
| `routes/reminders.py` / `storage/reminders.py` | Already shared by title digest; authentication stays per device. No change needed. |
| `crates/orbit-daemon/src/hub_notes.rs:249,263,269,324,455,465` list/document/turns/previous-meeting/library reads | Already address the hub using a device bearer token and recording/series IDs, with no device query filter. Keep request paths and credentials. Additive provenance survives deserialization. |
| `crates/orbit-daemon/src/daemon/recording_notes/reads.rs:73,255,277,324,407` local rows, document overlays and local transcript fallback | Keep local capture, unuploaded audio, pending private notes/outbox and pending calendar context scoped to this Mac. They supplement shared history. Preserve hub provenance when local progress replaces a list row; do not erase another Mac's shared event. |
| `crates/orbit-app-core/src/client/note_read.rs`, projections and `apps/desktop/src/features/notes/` | No device filter exists. Preserve `recorded_on` in the typed projections and generated TypeScript, show it in the list/detail, and explain the recording Mac's pending upload. Commands still address recordings by UUID. |
| Pairing, device credentials, active-device views and owner-profile publication | Retain existing device scope. Revoked tokens remain refused; reading a revoked recorder's meeting does not reactivate it. |

The retained storage predicates were checked individually:

- `meetings/repository.py:44,95,118,271,342,378,387`: existence, recording
  microphone profile, speaker changes, private notes/revision, title,
  language and repeated-group counts. Write-side revision/turn/group checks
  retain this same original-device key.
- `meetings/turns.py:218,324,341,400` and `meetings/muted.py:45`: the manifest,
  wall-clock origin, final transcript, draft groups/tracks and muted spans.
- `meetings/open_recordings.py:19,31,50,73,93,126`: indexed open-recording
  discovery, chunk metadata, voice-note exclusion, capture and pending counts.
- `meetings/views.py:160,210,221,323,354,366,431`: transcript state, notes,
  heading, failed groups, complete document and retranscription progress.
- `notes/repository.py:86,108,191,292` and `notes/worker.py:135`: planned
  inputs, request, worker claim, revision-checked edits and generation material.
- `group_storage.py:28,71,106,232,242`, `meeting_final_pass.py:36,106,150`
  and `final_pass_audio.py:55`: encrypted group manifests, preparation/claim,
  retry, upload capture status, final-pass status/request/claim and audio source.
- `storage/recordings.py:22,67,120,161,173,188` and
  `routes/captures.py:82`: chunk identity/receipt, closure equality and track
  validation remain recorder-only; they authenticate uploads rather than
  restrict the person's history.

## Changes outside the meeting package and list UI

The adjacent lines needed to carry this behavior through existing entry
points are `notes/routes.py:33`, `notes/custom_templates.py:133`,
`routes/recordings.py:92`, `meetings_bridge/models.py:36`,
`meetings_bridge/service.py:181`, `runtime/registry.py:56`,
`group_storage.py:71,106`, `meeting_final_pass.py:150` and
`meeting_context.py:111` (paths relative to `server/orbit_hub`). The signed
MCP adapter also needs `server/orbit_mcp/hub.py:478`,
`meeting_requests.py:4`, `meetings.py:166`, `refusal_messages.py:113` and
`tools/recordings.py:73` for personal-hub wording and the pagination cursor.

The Mac transport carries additive provenance through
`crates/orbit-core/src/notes/meetings.rs:81`,
`crates/orbit-daemon/src/daemon/recording_notes/reads.rs:356,467`,
`crates/orbit-app-core/contract/notes.rs:37,304`,
`crates/orbit-app-core/src/projection/notes.rs:50` and
`crates/orbit-app-core/src/projection/note.rs:134`, with their generated
TypeScript. The `briefing.rs` and projection test-support changes only supply
`None` for the new optional field in existing test fixtures. The device
bridge, control history, assistant settings, sign-in and handover production
code were left untouched.

## Verification

Tests were added before behavior changes and exercised public HTTP routes,
signed private routes, the MCP tools, daemon reads and rendered React UI.
All data, devices, audio and model responses are synthetic.

- Initial personal-hub acceptance: six failures (empty foreign list,
  no previous meeting, isolated assistant list and refused edits/deletion).
- Additional regressions failed for a cross-device series template, missing
  shared event, a revoked recording's unclaimable retranscription, chat's
  meeting context, same-start assistant pagination and lost local-row provenance.
- Both Rust projection tests failed because provenance was discarded. UI
  tests failed for missing pending-audio copy; the source-label assertion also
  required the real shell portal host to render its rows. Final React tests
  check list/detail provenance and absence of labels for single-Mac snapshots.
- Existing isolation assertions were updated to the new ownership contract;
  authentication, invalid input, encryption, revision, deletion completeness,
  tombstone and unrelated-meeting checks remain.

Linux verification does not establish a real two-Mac pairing/session,
Keychain credentials, native capture, offline upload completion, permissions,
real audio or real provider/assistant acceptance. Those require consented,
isolated Mac/hub/account fixtures; none were contacted for this change.

The first full gate completed 21 of 22 steps successfully. Its hub suite found
three remaining assertions for the retired isolation policy in
`tests/test_final_pass.py:742` and `tests/test_transcription.py:26,54`.
They now check the same closed-audio cause from either device, an identical
immutable transcript from either device, and continued retry of the person's
work after recorder revocation. Their focused suites passed all 41 tests.
The full gate was then rerun against the final tree.

## Final gate

`timeout 3600 bash scripts/check.sh` finished with exit code 0: **22/22 steps
passed**, in 12 min 45 s. Node 22 and the supplied isolated Cargo target were
used. The hub ran **1,462 passed, 1 skipped**; the harness dry run ran **57
passed**. The one allowlisted skip was
`test_assistant_jail::test_real_jail_hides_secrets_and_confines_writes`: this
container denies nested user namespaces. That native jail acceptance was not
passed and must be run outside this sandbox. No ratchet or skip allowance was
changed. No stray `crates/apps/` directory was present.

| Gate step | Result | Duration |
| --- | --- | ---: |
| environment · Rust 1.98.1 | ok | 0.0 s |
| build the daemon (before any test) | ok | 0.4 s |
| rust · format | ok | 1.6 s |
| rust · strict clippy | ok | 2.6 s |
| rust · cargo test of the daemon and its crates | ok | 168.5 s |
| rust · target/debug/orbit is the daemon without test features again | ok | 0.2 s |
| rust · cargo test of the app | ok | 37.6 s |
| rust · orbit-core with the Orbit Pruebas identity | ok | 16.3 s |
| rust · generated TypeScript up to date | ok | 0.1 s |
| environment · hub (server/.venv) | ok | 0.0 s |
| guards · 500 lines per file, 80 per function, and their ratchet | ok | 2.9 s |
| guards · a domain uses another only through its handle and types | ok | 1.5 s |
| guards | ok | 126.8 s |
| hub · ruff | ok | 0.0 s |
| hub · format | ok | 0.0 s |
| hub · pytest with the e2e | ok | 321.9 s |
| hub · dry run of the Mac's real-test harness | ok | 34.1 s |
| environment · interface (node, pnpm) | ok | 0.3 s |
| interface · lint | ok | 15.3 s |
| interface · typecheck | ok | 6.6 s |
| interface · vitest | ok | 15.2 s |
| interface · build | ok | 13.1 s |

Red/green evidence, both full-gate attempts and `results.tsv` are retained in
`$CARGO_TARGET_DIR/check`. The focused personal-hub acceptance finished with
10 passing cases; both provenance projection regressions pass, the daemon's
meeting-note suite passes all 28 cases, and the three recording-device UI
cases pass. The first full gate's three obsolete isolation expectations and
their corrected 41-test suites are recorded above.
