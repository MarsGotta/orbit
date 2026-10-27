# Orbit architecture

Orbit is a macOS companion for a personal assistant running in Hermes. This
document describes the current implementation; where code and documentation
disagree, correct the documentation against the code.

- **The hub is the single source** of meetings, transcripts, notes and conversations.
  The Mac records and uploads, then reads the hub's results.
- **One app process on the Mac**, plus child helpers: Orbit.app contains the UI
  and daemon. The Swift helper and control engine are managed by the daemon.
- **Decisions are pure**, in `crates/orbit-core`; effects are adapters.

Details: [process chain](process-chain.md), [local storage](storage.md),
[transcripts](transcripts.md), [continuous voice](continuous-voice.md),
[installation](../INSTALL.md) and [self-hosting](../self-hosting.md).

## Components

The [read-only Markdown vault](vault.md) owns connector configuration, the
hub's encrypted working copy, its reading index and the `notes` tool group.

| Component | Location | Owns | Must not do |
|---|---|---|---|
| UI | `apps/desktop/src`, React/Vite in WKWebView | Views and Spanish product copy in `src/i18n/` | Business decisions, hub traffic, `@tauri-apps/*` imports outside `src/bridge/` |
| Tauri shell | `apps/desktop/src-tauri` | App instance, window, tray, floating call panel, commands and daemon thread | Business rules or separate signal handlers |
| app-core | `crates/orbit-app-core` | In-memory daemon client, topic subscriptions, reads, commands, view projections, generated TypeScript and updater (release manifest and installer download) | Open the store, talk to the hub or depend on Tauri |
| Daemon | `crates/orbit-daemon` | Capture, durable state/jobs, helper/engine lifecycle, hub connection, Keychain, privacy stop and assistant bridge | Cache hub meetings/notes/conversations, decide presentation or hold provider credentials |
| Native helper | `native/macos`, `OrbitNative --stdio --managed` | macOS effects and global shortcuts | Network, launch a daemon, or create another managed-mode icon/login item/instance lock |
| Control engine | Bundled `cua-driver` 0.28.2 | Its declared computer-use tools | Network access or arbitrary executable/socket selection |
| Hub | `server/orbit_hub`, Python/FastAPI | Pairing, devices, chat, durable audio, transcription, turns, notes/templates, dictionaries, voices, titles/languages, private notes, transient screenshots and assistant bridge | Keep a calendar copy or expose remote administration |
| Hermes | Outside this repository | Assistant identity, sessions, interactive STT and TTS, models and provider credentials | Receive the Mac's device credentials |
| Translator | `server/orbit_mcp`, launched by Hermes over stdio | Assistant tools, signed private hub requests and result validation | Contact the Mac directly |
| Meeting transcriber | Operator-configured OpenAI-compatible service | Recognize meeting audio | Receive plaintext HTTP audio outside loopback |
| Note model | Operator-configured `[processing]` service | Write notes from transcript, private notes and meeting metadata | Receive meeting audio |
| Update origin | Optional build configuration | `latest.txt`, packages and `install.sh` | Assume a shared operator's download host |

The native helper handles microphone/system/app audio, VoiceProcessing for calls,
TTS playback, TCC permissions, EventKit, global shortcuts, local speech recognition,
window screenshots/selected text, microphone-using app observations and its own
privacy latch. `orbit-mcp` exposes `orbit_devices`, `orbit_windows`,
`orbit_screenshot`, `orbit_look`, `orbit_selected_text`, `orbit_control_tools`,
`orbit_perform_action`, `orbit_action_result`, `orbit_calendar`,
`orbit_meetings`, `orbit_transcript`, `orbit_note`, `orbit_files_list`,
`orbit_files_search`, `orbit_files_read`, `orbit_attachment` and, for an
`openai` or official vendor agent, `orbit_skill`.

| Crate | Responsibility |
|---|---|
| `orbit-core` | Pure policies, state and typed contracts, with facts/time supplied |
| `orbit-store` | Encrypted storage; receives its key, never opens Keychain or reads clocks |
| `orbit-ipc` | Typed local transports, paths, configuration and reports |
| `orbit-control` | The control engine's adapter: the verified CUA MCP client, the engine's processes and their cleanup; its decisions are `orbit-core`'s |
| `orbit-daemon` | Runtime composition, capture, hub, helper and engine; standalone binary is development-only |
| `orbit-app-core` | App client, projections and updater, without Tauri |
| `orbit-desktop` | Tauri shell linking the daemon into the app |

Inside the daemon one actor owns the runtime (`daemon/runtime.rs`): a
coordinator over one handle per domain (the helper, the hub link, transfers,
the assistant, the capture, the audio of meetings, voice notes and calls,
screen sources, the calendar, control of the Mac, notes, call detection, the
meeting question and the assistant's folders). A handle owns its domain's
runtime state and the methods that touch only that state. What crosses
domains stays on the coordinator, including every core decision:
`daemon/kernel.rs` adopts the decision and hands each effect to the domain
that executes it, and `daemon/native_events/` routes each helper event and
answer the same way. A domain uses another only through its handle and its
types; `scripts/check/boundaries.toml` lists the edges between domains, here
and in `orbit-core`'s `transition/`, and the gate refuses new ones.

## Boundaries and contracts

| Boundary | Transport | Contract |
|---|---|---|
| UI ↔ shell | Tauri `orbit_snapshot`, `orbit_read_sessions_page`, `orbit_read_session_messages`, `orbit_agent`, `orbit_send`, `orbit_panel_send`, `orbit_report`; `orbit://feature/<feature>` events | ts-rs types from app-core in `apps/desktop/src/bridge/generated/` |
| app-core ↔ daemon | In-process `Call`/oneshot and a `watch` per topic | `orbit-core/src/topics/`, `reads.rs`, `protocol.rs`, three-valued `Knowledge` and typed `Refusal` |
| Daemon ↔ helper | Two private inherited pipes, NDJSON | [Native protocol](../../contracts/native-v1.md) |
| Daemon ↔ engine | MCP JSON-RPC via `cua-driver mcp --embedded` stdio, proxy to a private socket | [CUA](../../contracts/cua-v1.md), [control policy](../../contracts/control-policy-v1.md) |
| Daemon ↔ hub | HTTPS and WSS `/v1/ws` with device token | [Hub protocol](../../contracts/hub-v1.md) |
| Translator ↔ hub | HMAC-signed private HTTP routes for device requests/results and meetings | [Hub protocol](../../contracts/hub-v1.md) |
| Hub ↔ assistant | Typed `AssistantBackend` and `SpeechBackend` interfaces; Hermes over `/api/ws` JSON-RPC and HTTP audio endpoints | `server/orbit_hub/assistant/`; [setup](../self-hosting.md) |
| Hub ↔ transcriber | `POST <endpoint>/v1/audio/transcriptions` | [Transcripts](transcripts.md) |
| App ↔ update origin | HTTPS | [Release manifest](../../contracts/release-manifest-v1.md) |

Boundary limits have their purposes in source:

- The development/acceptance `orbit.sock` request line is at most 1 MiB
  (`MAX_IPC_LINE_BYTES`). Each topic/read has its own bound; transport allows
  the largest plus envelope. Voice-note audio replies allow 12 MiB for Base64
  (`orbit-ipc/src/ipc/budget.rs`). Producer limits also apply in memory;
  `Absent { TooLarge }` preserves its counts.
- Helper lines: 6 MiB; 64 commands waiting for the helper's main thread (past
  them a command is refused `native_main_thread_busy` and the channel stays
  open; stops have 16 more) and 2 MiB pending output. Heartbeat every 2
  seconds; capture stops after 6 seconds without one.
- Engine frames: 1 MiB; daemon requests normally have a 5-second timeout.
- Hub: Mac outgoing JSON frames are at most 128 KiB; the Mac's incoming hub
  JSON frame/ordinary HTTP response bound is 1 MiB. Audio has separate limits.

Shared constants and their copy checks are registered in
`contracts/constants.json` and `scripts/tests/test_constants_copies.py`.

## Process lifecycle

```text
Orbit.app/Contents/MacOS/Orbit
 │  UI + shell + app-core
 │  app-core ── Call / watch ── daemon thread "orbit-daemon"
 ├─ Contents/MacOS/OrbitNative --stdio --managed
 ├─ Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver serve --socket <data>/cua/driver.sock
 └─ Contents/Helpers/Orbit Engine.app/Contents/MacOS/cua-driver mcp --embedded --socket <data>/cua/driver.sock
 daemon ── HTTPS + WSS ── hub ── Hermes profile ── orbit-mcp
```

Startup waits up to 15 seconds for daemon readiness; `daemon.lock` prevents two
daemons over the same data directory. Every quit route sends `Shutdown` once
and joins the thread for at most 17 seconds. SIGTERM/SIGINT belong to the daemon.
Unexpected daemon exit relaunches the app, and a recording that was live
resumes; a daemon that falls in its first minute closes the app instead.
Closing the window only hides it and never stops recording.

The engine starts when Accessibility and Screen Recording are granted, the
helper is connected and privacy is rearmed; no hub or pairing is required.
Privacy stop, permission withdrawal and exit stop it. An unknown permission
does not start or stop it. See [process chain](process-chain.md).

## Data flows

### Meeting

Start from a calendar event, `⌃⌥M` or the recording prompt after a call app has
used the microphone for 3 seconds (`CALL_CONFIRM_MS`). A recording accepted from
call detection stops after the app releases the microphone and a visible
countdown expires. A manually started recording does not stop for that reason.
No recording stops at its calendar event's end or at a duration ceiling.
After two minutes below the call speech threshold (500 RMS S16) in both recorded
sides, Orbit asks whether to
keep recording; thirty seconds without voice or «Seguir grabando» finishes
through the normal audio drain and save. A muted microphone is ignored. Missing,
delayed or unknown samples keep recording; the window never decides a stop.
See [silence questions](silence-questions.md). While the hub does not answer,
the recording continues: sealed chunks wait on the Mac and are
uploaded when the hub answers again.

1. The helper supplies `mic_local` and `remote_app` PCM. The daemon seals an
   encrypted chunk per track per second and its `chunk_upload` job.
2. `PUT /v1/chunks/{id}` sends capture metadata; the hub acknowledges only after
   durable storage. The Mac releases confirmed meeting audio.
3. The hub groups contiguous track audio, transcribes it and emits
   `recording.changed`. The daemon advances the `notes` topic revision; app-core
   rereads the list or open document/turns from the hub.
4. `POST /v1/recordings/{id}/finalize` records closure. The final pass and note
   worker process the stored audio/transcript; later changes emit the same event.

Recording works offline. Private notes stay in a local durable queue until
`PUT …/private-notes` is confirmed, and note edits, titles, speaker names,
languages and template orders wait in the notes outbox until the hub answers:
one lane per meeting, so a meeting the hub refuses or does not answer never
holds another one back (`daemon/recording_notes/outbox.rs`). All transcript consumers, including assistant
tools and note generation, use `server/orbit_hub/meetings/turns.py`.

«Terminar sin guardar», while recording or paused, stops the capture without
closing it: what drains is dropped, no chunk of it goes up, its audio leaves
the Mac and no note is written. «Borrar reunión» deletes a saved meeting.
Both go to the hub as `DELETE /v1/recordings/{id}` from the notes outbox,
also after the hub was away or the Mac restarted, and the hub deletes the
recording and all made from it in one transaction ([storage](storage.md),
[hub protocol](../../contracts/hub-v1.md#deleting-a-recording)).

### Voice note and call

Holding `⌃⌥Space` captures a voice note. In the chat, the composer's microphone
button starts one with a click and sends it with the next; holding it talks
while held, and Esc cancels the note without sending it. When the note ends the
daemon stores its encrypted
WAV locally with `upload_allowed:false`, sends it for interactive dictation to
`POST /v1/audio/transcribe`, then sends `chat.send` with `client_id` and duration.
A note shorter than `MINIMUM_SPEECH_MS` (160 ms, the speech a call turn needs)
is neither stored nor sent; the composer says its length and that minimum.
A note in which the hub hears no words leaves the conversation and its audio
the Mac, with a small line in the composer; it is not a failure. A note that
waits on the Mac (`DurableState::pending_voice`) belongs to the conversation
it was spoken in and is shown and sent only there; one spoken in a new
conversation belongs to the conversation that new one becomes. A failed note
can be discarded («Descartar», after a question in place), which deletes its
audio (`Store::discard_voice_note`).
The hub asks Hermes; text events return to the Mac. `POST /v1/audio/speech` gets
the spoken response for native playback, and «Escuchar» reads an answer the same
way. The request names the conversation's agent (`agent_id`, to a hub that lists
its agents); the hub reads in that agent's voice, or in the default agent's when
it has none of its own (`voice` absent from its `capabilities` in `ready`).
Dictation stays the default agent's. The daemon remembers whether the last
reading worked per voice that read, so one agent's failed voice is not reported
for another, and the chat offers «Escuchar» only when the conversation's agent
or the default agent has a voice; a hub without agents is offered it as before.
When the transcription fails, «Reintentar
transcripción» sends the stored WAV again; if this Mac no longer holds it, the
refusal says the audio is missing. During a meeting, dictation becomes
a question about that meeting; it does not play the response aloud.

`⌃⌥D` authorizes a call without a duration ceiling. The helper starts VoiceProcessing; the daemon
validates its ACK and every frame's provenance. A pure detector segments turns.
Derived turn audio stays local, while the continuous `voice_call` original is
uploaded; it is not listed as a meeting and gets no meeting note. Barge-in joins
a still-transcribing turn or replaces an already submitted turn, interrupts the
remote response and stops playback. See [continuous voice](continuous-voice.md).

### Meeting wake phrase

During a meeting the helper uses local macOS `SpeechAnalyzer` recognition for
“Oye <name>”, “Hola <name>” or “<name>,”. The hub selects the name (Deimos
by default), advertises it in `ready`, and the daemon retains it for offline
starts and passes it to the helper and UI. Only the question text and its times leave
the helper. Silence of 1.5 seconds or the 30-second question bound closes it.
The daemon validates privacy, connection, busy state, size and meeting window.
The hub adds the last requested minutes of transcript (default 2), bounded to
31 KiB. History retains the question and the Spanish meeting marker rather than
the expanded context. The floating panel follows the response; it is shown
during any recording, started by hand or by a detected call, so the question
can be asked from it.

### Screen and actions

The translator requests a screenshot through the signed hub bridge. For a full
desktop screenshot the engine writes a PNG to an Orbit-owned temporary file;
the daemon reads, deletes and fits it to 1280×1024 and 700,000 bytes. It never
embeds a Retina screenshot in a 1 MiB engine frame. Window requests use the engine
or, if unavailable/invalid, a native window capture.

The daemon uploads the PNG with request ID/hash, then replies with `capture_id`
and scale. The hub delivers the in-memory bytes once. Desktop actions require
a previously delivered full-screen capture; coordinates map back to screen pixels.
An action is recorded as `unknown` before execution. A lost receipt never
authorizes repeating input. The bridge returns `in_progress` and an operation ID,
then a separate read retrieves the outcome.

The daemon serves one bridge request at a time of each kind: the folders, the
screen (windows, screenshots, selected text), the computer's tools and actions,
and the calendar. A second request of a kind in course is refused as busy; one
of another kind, from any conversation, is served meanwhile. A capture's upload
belongs to its request's kind.

### Calendar

`orbit_calendar` sends a signed `calendar_read` request. The daemon reads all
EventKit calendars through the helper in 31-day windows, with remote pages of
at most 96 KiB. The translator walks continuations and joins fragmented invitees.
Calendar permission is the sole consent; there is no hub calendar copy, activation
switch or change to the week currently visible in the app. Missing permissions
are explicit refusals, never an empty calendar.

Calendar work never runs on the helper's main thread, which reads the
daemon's heartbeat and starts and stops audio: EventKit runs on its own actor,
`CalendarController` on another, and `EKEventStoreChanged` on the controller's
queue; the main thread only admits a command and reads the last published
status. In the daemon a read EventKit holds is a pending request with a
deadline, never a wait in the loop (`crates/orbit-daemon/tests/calendar_isolation.rs`).

### Agent configuration

Ajustes › Agentes lists the agents of the status topic. Each read or order of
an agent's personality, skills or access groups is one `orbit_agent` call:
app-core passes an `AgentCall` to the daemon (`Command::Agent`), which sends
it, off its loop, to the hub route `orbit_core::agent_config` names, with the
device token, and answers with the outcome: the configuration, a `SKILL.md`,
done, the hub's refusal (code, HTTP status and scalar numbers) or the Mac's
own refusal (not paired, privacy stop, no link, an answer out of contract).
Nothing of it is kept on the Mac; the screen reads the configuration again
after every order the hub takes. An access switch is configuration and is
sent as it is flipped. Importing a skill reads only the `SKILL.md` at the top
of a folder chosen in WebKit's own file panel, in the web view, when it fits
the hub's ceiling.

### Assistant folders

«Añadir carpeta» in Ajustes asks the helper for the macOS panel (folders only);
the folder chosen there is the whole permission. The helper returns its path
and a security-scoped bookmark, which the daemon keeps in the encrypted store
(`assistant_folders_v1`, at most eight folders); «Quitar» forgets it. The
control topic publishes each folder with `accessible`, `unknown` or
`unavailable` and its cause.

`orbit_files_list`, `orbit_files_search` and `orbit_files_read` send
`files_list`, `files_search` and `files_read` through the signed bridge. Each
takes two helper steps: `assistant_folders` opens every bookmark and resolves
the requested path with `realpath` (links and `..` followed), and
`orbit_core::assistant_files::admit` decides, without the disk, whether that
canonical path lies in a folder; only then does `assistant_files_list|search|
read` run on the canonical path, which the helper reads only while it is still
canonical (`O_NOFOLLOW`). A search never follows a link. Text arrives in pages
of 48,000 bytes, a PDF's text (PDFKit) by pages, and an image fitted by ImageIO
into a PNG capture uploaded like a screenshot. Refusals name the path, where it
led and every folder. Read-only; paths and content never enter the logs.

## Storage and trust

See [local storage](storage.md) for encryption, retention and recovery.

| Location | Data and lifetime |
|---|---|
| Mac store | Durable current state/turn, jobs, encrypted pending audio, upload receipts, pending private notes and note orders, call preference and the assistant's folders with their bookmarks; no hub history cache |
| Mac data directory | Configuration without tokens, instance lock, engine socket/temporary screenshot, bounded control history and logs |
| Keychain | Storage key and revocable device token |
| Helper | Durable local privacy latch and content-free native diagnostics |
| Hub SQLite | AES-GCM protected content: devices, pairing, conversations, audio, transcripts, notes/templates, dictionaries, voice intervals and meeting metadata |
| Hub memory | Screenshots, at most four per device, at most 300 seconds, delivered once |
| Hermes | Assistant sessions under Hermes retention policy |

The hub has a preventive database/WAL/SHM admission limit, not a physical
quota or deletion policy: `storage_quota_bytes`, or by default what its disk
leaves (half of database plus free space, after 5 GiB; see the
[self-hosting guide](../self-hosting.md#storage)). It tells each Mac what it
uses and what is left, and when it is nearly full. It does not automatically
delete audio or history.

| Secret/boundary | Holder and authority |
|---|---|
| `orbit.sock` in development/acceptance | Mode 0600; both peers validate same-user ownership |
| Helper pipes | Inherited by the managed helper; only local managed traffic can rearm |
| Engine socket | Private 0700 directory under Orbit data; no alternative executable/socket from commands |
| Mac storage key | Keychain `storage` account; development-only file keys require explicit dev mode |
| Device token | Keychain `device` account; hub stores a hash; locally revocable with `orbit-hub revoke` |
| Pairing code | Administrator output, hash in hub; one use, default 300 seconds, allowed 30–600 |
| Hub storage key | Private 0600 file, AES-256 |
| Hermes credential | Private 0600 server file |
| Bridge keys | Hub and translator only, separate 32-byte HMAC keys with route-bound signatures, 30-second clock-skew allowance and replay-rejected nonces |
| Provider credentials | Server only |
| App error reports | Build-time `ORBIT_SENTRY_DSN`; codes/numbers only, with anonymous reporting off by default in Settings |
| Updates | TLS plus manifest hashes; signing/notarization are separate publisher checks |

macOS attributes permissions to Orbit.app and its descendants. Network traffic
cannot clear the durable privacy latch. A remote revocation stop is separate
from the user's durable stop and does not persist through restart. Logs exclude conversation, transcript, note and
credential content.

Without a hub, recording, EventKit, privacy stop, shortcuts and queued private
notes still work. Hub history, chat, dictation, calls and meeting questions need
the connection. The engine may run locally, but the assistant cannot request
its tools without the hub.

## Verification

Run the whole gate described in [CONTRIBUTING.md](../../CONTRIBUTING.md) before integration.
The standalone daemon binary and “Orbit Pruebas” support isolated acceptance
fixtures. Tests against synthetic peers establish protocol decisions, not real
macOS permissions, audio quality, focus behavior, AEC or Hermes acceptance.
