# Changelog

All notable changes to Orbit are documented in this file. The format is based
on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Orbit uses
[semantic versioning](https://semver.org/spec/v2.0.0.html). The app supports
English and Spanish; labels quoted below may use either language.

## [Unreleased]

### Fixed

- Preserve call recordings when macOS cannot observe microphone users. Keep
  available global shortcuts after a conflict and show failed combinations
  with their macOS status in Settings.
- Ignore retired wake-phrase listener reports, decode speech clips off the
  main thread with stop cancellation, and reduce idle watchdog work and
  capture allocation/timer overhead.
- Give the test app its own compiled identity for macOS integrations.
- Read notification authorization and delivery failures from macOS, and show the
  permission and System Settings remedy. Use the main app's modern login item
  registration and report when system approval is required.
- Request screen and Accessibility access before offering System Settings in
  Settings and onboarding. Suspend panel hover checks while no question is shown.

### Changed

- Assistant answer and skill editors keep dismissed drafts until saved or
  explicitly discarded. Meeting questions wait for acceptance, preserve refused
  text, explain available causes and limits, and support arrow-key context
  selection. Speech failures and composer hover help use the selected language.
- Unchanged Assistant Markdown avoids repeated parsing during recording updates;
  chat images load near the viewport and release their bytes when scrolled away.
- Desktop feature publications reuse unchanged typed views and shared topic
  payloads. Idle daemon topics stop periodic recording and notes refreshes;
  live recording, call and question state keeps its refresh cadence.
- Meeting renames consistently trim edges and collapse repeated whitespace in
  the app, daemon and hub. Hub conformance and device diagnostics use English
  while retaining their error codes and measurements. Automatic recording titles
  from the keyboard shortcut follow the selected interface language.

- The real-test harness supports an English pass with `run --language en`,
  alongside its default Spanish pass. Both use checked exports of Orbit’s own
  catalogues, including native menus, and retain all 21 checks and screen
  evidence. English speech uses two installed system voices and en-US
  on-device recognition.

- The real-test harness drives Orbit Pruebas's windows through the in-app
  bridge, preserving semantic screen evidence and reconnecting after the
  Dock's quit. CuaDriver is scoped to a new read-only accessibility check;
  real pointer motion, audio and the Dock's quit keep their native paths.

### Added

- Orbit Pruebas has an owner-only local test bridge for DOM presses, typing,
  visible elements, native menu snapshots and window numbers. The real-test
  harness uses it for interface actions and native menu snapshots. The published app excludes the bridge at compile time.

## [0.13.0] - 2026-10-29

### Added

- The interface speaks English and Spanish. Settings › General chooses the
  language; by default it follows the first of macOS's preferred languages that
  is English or Spanish, else English. Dates, times, numbers and plurals follow
  its locale, and the choice applies at once. The menu bar, the application
  menus, the macOS notifications, the floating panel, the permission texts and
  what Orbit says aloud follow it too: the wake phrase is «Oye {name}» in
  Spanish and «Hey {name}» in English, with its recognizer in that language.
- A vault's `SCHEMA.md` frontmatter defines its own types, each with a role
  (person, project, meeting, topic, source, journal, note) and an optional
  folder, and renames the project fields Orbit reads (`stage`, `start`).
  Without it, English defaults apply. The app decides by role, not by name.
- macOS notifications say when a recording is cut, comes back or is closed on
  its own.

### Changed

- The vault format is English. Tasks and dependencies use the Obsidian Tasks
  syntax, with a subset of its query blocks. The reader follows OKF 0.2:
  `status` is the lifecycle, the last change is `generated.at`, review is
  `verified`, unknown keys are kept, `index.md` and `log.md` keep their reserved
  shapes, and a note without a `type` is read but marked not conformant. Update
  the hub before the Macs.
- Refusals and failures from the hub and the daemon travel as codes with their
  numbers, and the interface writes them in the chosen language. Their English
  text remains for logs and for an older Mac.
- The assistants' tool descriptions, results and refusals, the instructions to
  models, logs, internal errors and comments are in English. Meeting notes stay
  in the meeting's language.

### Fixed

- A hole in the audio, a damaged frame or a frame in another format never cuts
  a recording: the gap is counted and the capture goes on. A real cut resumes
  with no limit on tries, waiting 1, 2, 5, 10 and then 30 s; only a capture gone
  for two minutes closes it.
- A freshly started capture task counts as running from its start, so the loop
  waits for the frames queued for it under load.
- The meeting list no longer shows voice notes, whose deletion as a meeting the
  store refused forever; a permanent refusal is not retried.

## [0.12.9] - 2026-10-28

### Fixed

- The macOS build compiles again: the app's capability test builds Tauri's
  test context instead of embedding the Info.plist a second time. 0.12.8
  was tagged but never built for the Mac; 0.12.9 carries its fixes.

## [0.12.8] - 2026-10-28

### Fixed

- Texts outside the «Asistente» view name the primary agent, who answers
  them: the floating panel, «Preguntar» in a meeting, «Oye…», Ajustes and the
  palette. Before, they named the agent of the conversation left open. The
  palette's question now goes to the primary too; inside a conversation,
  every text still names its agent.
- «Ajustes › Notas» and the «Notas» section reach the hub again: the main
  window's capability now allows the vault order, which it refused before
  the request left the app. A test checks every order each window's
  interface sends against the real capabilities.

## [0.12.7] - 2026-10-28

### Fixed

- Secret discovery skips folders the hub's user can neither list nor enter,
  such as database volumes owned by another user, instead of failing the
  whole scan and refusing every official assistant. A folder that can be
  entered but not listed still refuses, since its secrets could be opened by
  name without being masked.

## [0.12.6] - 2026-10-27

### Added

- «Ajustes › IA» and the «Tu IA» onboarding step: encrypted hub provider
  credentials, bounded OpenAI-compatible discovery and editable capabilities,
  independent model choices for meetings, listening, speech, meeting notes,
  future vault maintenance and compatible assistants, and explicit off-hub
  privacy notices. Existing OpenRouter credentials migrate on hub startup;
  operator defaults and assistant subscriptions remain available.
- Read-only «Notas»: connect a hub folder or Fast Note Sync, browse the vault,
  read fiches and original Markdown with sources and backlinks, inspect tasks
  and dependency/link maps, and choose the link style and future hub-job model.
  Credentials and the hub working copy are encrypted. Assistants get three
  read tools behind the new `notes` access group. No note writes or model jobs run.
- Choose hub Whisper or MAI-Transcribe-2 through OpenRouter in AI settings,
  with an encrypted, validated hub credential, an explicit audio privacy
  notice, and per-meeting model comparisons.

### Fixed

- Meeting voice separation uses the linked calendar's invitees, excluding the
  owner, as its remote-speaker ceiling. «Volver a transcribir» also separates
  voices again and preserves user names by matching earlier word times; merged
  voices keep the larger named voice and retain the other names as aliases.
  Existing meetings are left alone until retranscription is requested.
- Official assistants mask secret files discovered by name under optional
  `[assistant_tools.jail] secret_roots`, including secrets in a working folder
  within a configured root. Discovery skips dependency and cache trees, bounds
  depth and visited entries, and reports partial scans in Spanish while
  preserving startup and ordinary neighboring files.
- Secret discovery runs in the background at hub startup and every ten minutes,
  caching readable files per root. The first assistant start waits up to thirty
  seconds for discovery and refuses if it is unavailable; warm starts use the
  cache and skip deleted files. Registered discovery bounds now accommodate the
  measured service tree with room for growth.

## [0.12.5] - 2026-10-27

### Added

- Mac action history keeps the requesting assistant's hub-authenticated identity
  and display name. Settings shows the author on each attributed action when
  more than one assistant appears in the history; older hubs and stored entries
  remain readable without a name. Retention fits both the existing row count
  and file size budgets, evicting the oldest entries as needed.

### Fixed

- Capture task restart exhaustion preserves audio already accepted by its
  channel and hands it to the daemon loop before later fallback events.
- The installer log parser test isolates its shell, tools, environment and
  scratch directory; failed fixture installs report status, output and log.
- Every paired Mac sees the personal hub's meetings, notes, transcripts and
  previous-meeting tasks, including meetings recorded by revoked devices.
  Renaming, deleting, retranscribing and correcting them works from any Mac;
  recordings keep their original device storage keys. Reuniones names the
  recording Mac when the hub has several, and explains where pending audio
  must be uploaded. Assistant reads share the same history and preserve
  meetings that start together across paginated results.
- Assistant sign-in distinguishes an explicit subscription requirement and names
  the eligible ChatGPT or Claude plans in Spanish.
- Reopening a handed-over conversation restores its continuation line from the
  hub's message read, without enlarging the conversation list. Paired Macs can
  list and read the owner's conversations across devices.

## [0.12.4] - 2026-10-26

### Changed

- Calls and meeting recordings have no four-hour ceiling or ten-minute warning.
  Calls ask after 30 seconds without conversation or assistant work and hang up
  after 15 unanswered seconds; short presence replies stay local. Recordings
  ask after two minutes of quiet on both sides and finish and save after 30
  unanswered seconds. Voice or «Seguir» keeps them going; missing observations
  keep a recording open. Muting the microphone counts only the meeting audio.
  Recording silence uses the call speech threshold (500 RMS S16), so ordinary
  room noise does not keep a forgotten recording open. Mac real-test check 19
  verifies an unanswered question saves the meeting and its hub chunks.

### Fixed

- Pending silence questions survive missing or stale audio without repeating or
  restarting their answer window. Recordings close only on fresh quiet after
  expiry; positive voice or «Seguir grabando» keeps them going. Call questions
  likewise wait for usable observations, and noise cannot answer them. Their
  notices are waiting questions, rather than failures.

- Meeting transcripts keep each track's utterances whole during overlapping
  speech and short interjections, including in already recorded meetings.
  Pauses longer than one second start a new turn only when another voice
  speaks during the gap; monologues retain the minute paragraph window even
  through long pauses. Saved corrections, voice names and named-turn
  intervals remain attached to their words in the app and assistant reads.
- Conversations can be opened or started while another assistant turn continues
  on the hub. Returning restores its live answer, pending permission or question;
  the conversation list marks turns waiting for their owner. Voice notes still
  being recorded or transcribed explain the wait without an error code. Vendor
  retries appear as waits instead of «está pensando…».
- «Nueva conversación» starts without notes from earlier new-conversation
  views. Unsent notes stay on this Mac and reach their own hub conversation
  when delivery becomes possible, without changing the open view.
- Joined call fragments leave no failed note; their audio remains in the
  next turn. Refused or empty clips shorter than three seconds disappear
  silently, while longer refused notes keep «Descartar». Loading saved state
  removes old session-less fragments of these kinds and their local audio.

## [0.12.3] - 2026-10-26

A hub-only release: the Mac app stays at 0.12.2.

### Fixed

- Hub Codex assistants disable account plugins/connectors, browser/computer
  integrations, external hooks, automatic installs and approval reviewers at
  startup and every thread start/resume. Cached plugins are ignored, ancestor
  project configuration stays untrusted, and connection retries are finite.
  Codex and Claude turns can remain silent while reasoning without a hub
  inactivity deadline.
- Claude assistants explicitly disable claude.ai connectors, exclude user and
  project settings/plugins/hooks, and retain only Orbit's configured MCP and
  its SDK permission hook, while preserving subscription sign-in.
- Codex provider connection failures retain their structured cause and HTTP status.

### Upgrading

- Republish the hub to install the complete pinned official Codex CLI package,
  including `codex-code-mode-host`, package metadata, bundled paths and resources.
  Publication verifies its checksum and every file before sealing the release.
  Upgrade the hub first; the Mac protocol is unchanged.

## [0.12.2] - 2026-10-26

### Fixed

- Short Mac actions can return their durable result in the first tool call.
  The hub waits up to 2.5 seconds, configurable with
  `device_bridge.action_inline_wait_seconds`; slower actions keep their
  operation id and the existing result lookup.
- Speaking over a call answer pauses its reading at detected speech onset.
  Words cancel that reading; noise resumes it without creating a turn. Short
  candidates preserve carried audio, including the start of a joined turn.
- Long answers are read in sentence parts, advancing on playback receipts.
  Interruption cancels the remaining parts and the next answer stays audible.
  The daemon and bundled native helper must be upgraded together for pause/resume.

### Changed

- Assistants added from Orbit use Anthropic's official Python Claude Agent
  SDK and the official Codex app-server. Streaming, activity, approvals,
  clarifications, stop, account sign-in and the plan-limit handover keep the
  existing hub/Mac contract. Third-party ACP adapters are removed.
- Claude and Codex assistants can be added from Orbit without hub tool tables.
  Claude uses the CLI bundled with Claude Agent SDK 0.2.174; hub publication
  ships official Codex 0.164.0 after checking its pinned SHA-256. The hub
  generates MCP configuration and private bridge keys; existing tool overrides
  still work. Missing tools name their path and cause and suggest republishing.
- Official assistants work on the hub with their own tools, in a working
  folder chosen on their page in Orbit, inside a required bubblewrap jail.
  - Host files are read-only. Hub and owner credentials and control sockets
    are hidden, and `/tmp` and `/proc` are private. Only the working folder
    and the vendor home can be written.
  - Startup and hub publication fail closed if the jail cannot be created.
  - Each assistant has separate read-only MCP bridge credentials, bound to
    its authenticated identity.
  - Commands and edits request Mac cards (Codex `untrusted`, with its
    internal sandbox disabled). Claude's local reads and searches, and its
    own to-do list, tasks and subagents, don't.
  - Working folders reject protected paths, the owner's home and the
    installed hub code.
  - **Trabajó…** keeps the commands and the edited filenames.
  - The self-hosting guide documents the remaining limits: vendor approval,
    credentials, network and later execution.
- Hermes setup copy explains that assistants are created on the hub from a
  Hermes profile and then appear in Orbit.

### Upgrading

- **Self-hosters: upgrade the hub first.**
  - `[acp_adapters]` becomes optional `[assistant_tools]` overrides.
  - Publishing the hub supplies the official tools; Node ACP packages are
    no longer needed.
  - Existing `orbit_mcp_config`, `env`, agent ids and account folders keep
    working.
  - Manually configured vendor agents use `kind = "claude-code"` or
    `kind = "codex"` and drop `agent`. Adapter-specific flags must be removed.
  - Some old adapter conversation ids can't be resumed by the vendor; their
    stored Orbit history stays available.
  - See the self-hosting guide.
- **The hub host needs bubblewrap.** It must be able to create unprivileged
  user namespaces. On Ubuntu, that is the `bwrap-userns-restrict` AppArmor
  profile.

## [0.12.1] - 2026-10-25

### Upgrading

- **Upgrade the hub first, then the Macs.** Nothing in the hub's schema or
  wire changes; a 0.12.0 Mac keeps working with a 0.12.1 hub and the other
  way round.

### Changed

- **Every file and function is within the gate's limits**: no production
  file over 500 lines of code and no function over 80, in Rust, Python and
  Swift, and no domain of the daemon touches another's fields. The split
  changes no behaviour, wire, text or generated TypeScript.

- **A call lasts what a recording lasts, however it began.** From the app or
  with ⌃⌥D, a call with the assistant may last up to 4 h, and ten minutes
  before it hangs up it says so as a recording does («La llamada se colgará
  en 10 min: lleva 3 h 50 min, el máximo es 4 h…»). The call view and the
  sidebar card say the hours left («quedan 3 h 59 min»), and «Cuánto dura
  cada cosa» shows one length for both.
- **Words, not sound, keep a call going.** A turn the hub finds no words in
  (a fan, a television) no longer counts as you speaking, so a forgotten call
  in a noisy room hangs up a minute after your last words. A turn still being
  heard holds the hang-up for one turn at most; muted time and the time the
  assistant works or speaks still never count.
- **One notice per meeting, from its end to its note.** «Reunión guardada»
  now stays while the note is written, then becomes «Tu nota está lista ·
  Abrir», or says «No se pudo escribir la nota» or «No se pudo transcribir»,
  until you close it. «Grabación guardada.» and «Llamada terminada: grabación
  guardada.» are no longer said beside it.
- **The assistant's requests to the Mac wait only for their own kind.** A
  slow read of your folders no longer makes a screenshot asked from another
  conversation fail as busy: the folders, the screen, the computer's tools and
  actions, and the calendar each serve one request at a time on their own.

### Fixed

- **A voice note sounds only when you press it, and the same button stops
  it** («Reproducir nota de voz» turns to «Parar nota de voz»). Opening a
  conversation again no longer plays the last note you heard, and leaving it
  stops the note.
- **An answer no longer begins with a `MEDIA:` line naming a file on the
  hub.** When the assistant made audio with Hermes's `text_to_speech`, its
  answer carried Hermes's tag for a messaging platform; the hub now takes
  Hermes's delivery markers (`MEDIA:` tags, `[[audio_as_voice]]`,
  `[[as_document]]`) out of the answer before it stores it or sends it to the
  Mac, and its log says only how many. A voice note's turn tells the
  assistant that Orbit reads its answer aloud, so that it answers in text.
- **«Escuchar» says «Parar» while Orbit reads an answer aloud by itself**
  after a voice note, as it does when you asked to hear it, and «Parar» stops
  that reading.
- **Tooltips stay inside the window.** At an edge they move in or go to the
  other side of their control, and no panel or header hides them, as «Añadir
  asistente» next to «Tus asistentes» was.
- **A request the Mac's helper refused because its previous order passed its
  deadline** (`native_command_stalled`) reaches the assistant as a busy Mac it
  can ask again, not as a reply it could not read.
- **«No entendí bien «Oye …»» no longer flashes** when the recognizer revises
  the phrase and hears it again at once: the line waits 1.5 s, and appears
  only if the phrase was not heard again. The daemon's log keeps every
  abandonment as before.
- **A short call no longer says it is about to hang up as soon as it
  starts.** The notice ten minutes before a call's 4 h ceiling is now said
  only to a call longer than those ten minutes; a call asked for two minutes
  hangs up at its end without it.

## [0.12.0] - 2026-10-24

### Upgrading

- **Upgrade the hub first, then the Macs.** The hub's changes are additive: a
  0.11 Mac keeps working with a 0.12.0 hub, and a 0.12.0 Mac with a 0.11 hub,
  without what needs both: deleting meetings, the personal reminder on both
  Macs, each answer's «Trabajó…» and the muted stretches in the transcript.
- **The hub's schema gains tables and columns and loses nothing**, so going
  back with `activate.sh anterior` keeps working.

### Added

- **«Silenciar»** in the «Grabando» card, the floating panels, the call view
  and the menu bar. In a recording only the microphone goes silent and the
  other side keeps recording; in a call the assistant stops hearing you. The
  helper replaces the samples with silence before they leave it, so nothing
  said is stored, transcribed or heard by «Oye Deimos», and the recording
  keeps its time. The note marks each stretch («Micrófono silenciado · 12:04
  – 12:30»); muted time never hangs a call up.
- **A new brand: the two-moon symbol.** The app icon, the menu bar mark (a
  template at rest, a red moon with the time while recording, an amber one
  while an assistant waits for you), the sidebar's logo, whose moon turns
  while an assistant works, and the README's logotype. The Orbit name, logo,
  app icon, menu bar icons and assistant avatars are not covered by the MIT
  License.
- **«Terminar sin guardar»**, next to «Terminar» in the «Grabando» card, the
  floating panel and the menu bar, while it records or is paused: after
  «¿Terminar sin guardar? Se borra lo grabado y no se escribe nota.» and
  «Borrar y terminar», the recording stops, nothing of it is kept on the Mac
  or the hub, no note is written, and «Reunión descartada» says so. With the
  hub away it is deleted there when the hub returns, also after a restart.
- **«Borrar reunión»** in a note's «Más» menu, after «¿Borrar esta reunión?»:
  the audio, the transcript and the note leave the Mac and the hub, with its
  tasks, speakers and corrections; the series' dictionaries stay. A meeting
  still closing says «Se borrará al terminar de cerrarse». The hub must be of
  this version (`DELETE /v1/recordings/{id}`): an older one deletes nothing
  and the Mac says «Actualiza el hub para borrar reuniones». A meeting that
  no longer exists says «Esta reunión ya no existe».

### Changed

- **The final pass keeps the better of the two transcripts, span by span**,
  instead of keeping the live draft whole when the final pass lost a span;
  voices are always separated, so the other side reads «Persona 1» and
  «Persona 2».
- **The floating panel's answer stays while the pointer is over it**, and
  goes five seconds after it leaves.
- **The personal reminder is kept in the hub**, so both Macs show it: «Solo
  lo ves tú. No va a la nota.» Without the hub it stays on the Mac and is sent
  later.
- **Each answer keeps its «Trabajó…» summary** after later messages and after
  reopening the conversation.
- **Wording from the brand guide:** «Todo está parado» and «Reactivar»
  instead of the privacy stop and rearm; «Rehacer el resumen» and «Rehacer la
  transcripción».
- **Inside, Orbit is split by domain.** Recording runs apart from the
  daemon's loop, on its own task and clock: with the loop held for 15 s a
  recording keeps sealing and uploading. Calendar work leaves the helper's
  main thread. The core's state and decisions, the daemon's runtime, its
  bridge, its catalogues of refusals and notices, and the hub's storage and
  runtime each have one module per domain. A panic in one of the daemon's
  actors relaunches only that actor. The gate holds 500 lines per file and
  80 per function, with a ratchet for what is over, and the boundaries
  between domains.

### Fixed

- **An update was blocked by a recording an older version left open.** Only
  a live capture blocks it now, and such a recording is closed when Orbit
  starts, at its last audio, so its note is written.
- **Audio that arrives late after a stall keeps its real times.**
- **A failed seal says its cause**; an unconfirmed close or a refused start
  has its own words.
- **Headphones connecting while a voice note runs and a meeting starts**
  cancel only the voice note, not the meeting.
- **«Oye Deimos» is heard when the recognizer joins earlier words** to the
  phrase, and when a question is still abandoned the panel says why.
- **A helper whose main thread is held no longer drops its channel** and cuts
  the recording: past 64 waiting commands they are refused as busy.
- **Stop, approve or clarify no longer wait for another conversation**, a
  slow agent no longer holds the others, and an agent that fails to start
  shows it.
- **A hub restart in the middle of an answer no longer closes that
  conversation for ever**, and a transcription group left running by a
  stopped worker is transcribed again.
- **«Pendiente de la última vez»** also finds the previous meeting by its
  title when the recording had no calendar link.
- **A question asked about a meeting keeps its form until it is taken**, a
  new event refused with the view closed gives its text back, and «Reiniciar
  Orbit» restarts it.

- **A voice note that waits on the Mac shows only in its own conversation.**
  0.11.0 showed every waiting note at the end of every conversation with the
  assistant, days-old ones included, and could send one into the conversation
  open when the hub came back. A note spoken in a new conversation belongs to
  the conversation that new one becomes; notes kept by 0.11.0, whose
  conversation is unknown, show only in a new conversation.
- **«Descartar» on a voice note that was not transcribed**, next to
  «Reintentar transcripción», with a question in place: the note leaves the
  conversation and its audio leaves the Mac.
- **Noise during a call no longer cuts the assistant off or leaves failed
  notes.** While the assistant answers, what the microphone hears stops him
  only once its words are known; a clip with no words leaves nothing. A turn
  in which the hub hears no words is dropped without a word in any case, and
  in a call it goes on in what the user says next. The hub answers a clip
  under 3 s that the speech-to-text behind Hermes refuses as no speech; a
  longer one refused says «El servicio de transcripción no aceptó este audio»
  and is not offered «Reintentar», which could not work.
- **A note that could not be transcribed says so calmly**, with what to do;
  its codes and numbers are its detail, not the line.

## [0.11.0] - 2026-10-23

### Upgrading

- **Upgrade the hub first, then the Macs.** A 0.10 Mac keeps working with a
  0.11.0 hub, and a 0.11.0 Mac with a 0.10 hub, without the new features that
  need both: several dictionaries, assistants added from Orbit, the hub's
  storage numbers and a declared missing chunk.
- **`activate.sh` now copies the database and its key before it activates a
  release**, under `$ORBIT_HUB_BASE/backups/activation`. Going back with
  `activate.sh anterior` puts that copy back when the schema changed, and sets
  the newer database aside without deleting it. The copy needs room for one
  more database on the same disk, and `python3` 3.11 or newer on the server.
- **The hub's storage limit is no longer a fixed 10 GiB.** Without
  `storage_quota_bytes`, it is what the disk allows: half of the database and
  the disk's free space together, after keeping 5 GiB for the rest of the
  server, so a copy of the database always fits beside it. Set
  `storage_quota_bytes` to cap it lower. Nothing is ever deleted on its own.
- **A nightly copy of the database ships** as `orbit-hub backup` with a
  systemd service and timer; it is not installed by the upgrade. See
  `docs/self-hosting.md` («Backups»).
- **Assistants added from Orbit need `[acp_adapters]`** in `orbit.toml`, with
  the adapters' installed paths. See «Assistants added from Orbit».

### Added

- **A new interface.** A sidebar with names (Hoy, Calendario, Reuniones,
  Asistente, Ajustes) replaces the rail; while recording or in a call, a card
  with its clock and controls takes the button's place. The top bar shows
  Orbit's notices one at a time, each with its fix. Every screen is redrawn:
  Hoy, the calendar's day, three days, week, month and agenda, Nueva
  grabación and the floating panel, Reuniones with its list column and the
  note, the Asistente, Ajustes in five sections, the first run and the command
  palette. Nothing is shown blank while it loads.
- **«Notas» is now «Reuniones».**
- **One Markdown editor for every note:** private notes, the summary's
  sections, «Editar la respuesta», an assistant's personality and a
  template's context. It stores exactly what was typed and shows it
  formatted on every line but the cursor's, with «/», «[[», «@», a format bar
  and ⌘/ for the raw text.
- **Correcting a word under the pointer**, in a menu beside it, every time it
  is heard, into several dictionaries at once, with «Deshacer».
- **Several dictionaries for a meeting or a series**; the first one wins when
  two disagree, and the hub says so.
- **«Rehacer transcripción» asks first and shows its progress**, and no
  longer rewrites the note by itself; «Rehacer resumen» sits in the summary.
- **Several assistants:** «Tus asistentes» in the Asistente's column, each
  with its own page, a pencil for its name, description, colour and whether
  it is the primary, and a gear for the rest. Claude Code and Codex
  assistants can be added from Orbit, signed in through the vendor's own
  tool (the token stays on the hub), and removed. A plan's limit keeps the
  question and offers to go on with the primary. «Oye {nombre}» calls each
  assistant; «Oye» alone calls nobody.
- **Answers:** icons under each one (Copiar, Escuchar, Volver a preguntar,
  Editar y copiar), code blocks with their language and file, and shell
  commands copied without the `$`.

### Changed

- **No recording stops because of the calendar.** Every recording runs to one
  safety ceiling of four hours, announced ten minutes before.
- **A command that deletes files is approved once at a time**; the hub
  refuses a wider choice before the agent hears it.

### Fixed

- **A recording is no longer cut by a silent daemon:** the helper keeps
  capturing and holds up to five minutes of audio for it; the daemon's
  heartbeat runs on its own thread; a late watchdog tick is the helper's own
  stall. macOS no longer naps Orbit.
- **A declared gap no longer cuts the recording**; a meeting cut by anything
  but the person resumes by itself as a new segment, or is closed at its last
  audio so its note is written; «Terminar» with a late drain still closes; a
  daemon that falls relaunches Orbit and the recording goes on.
- **One track's failure stops only that track**, and a device change under a
  voice note during a meeting cancels only the note.
- **Delivery:** a chunk the hub refuses is said with its code and, if it
  cannot be delivered, declared missing so the meeting still closes and the
  note says which part is missing; a hub out of space is said with its
  numbers and retried, keeping the audio on the Mac; the backlog uploads in
  capture order.
- **Voice notes** spoken with the hub away, or whose transcription failed,
  wait on the Mac, survive a restart and are sent when the hub is back; they
  no longer block the next recording.
- **What is written is not lost:** the chat box empties only once the message
  is accepted; drafts survive leaving the view, ⌘Q and a crash; Orbit hands
  over pending saves before it quits; the personal reminder says «Guardado»
  only when it was; capture failures say their cause instead of a generic
  text.
- **The hub:** a full disk is said as `storage_full`; a refused chat message
  can be sent again with its id; `init` refuses an existing database; a
  second regeneration keeps the corrections the first set aside; the hub
  refuses to start with more than 32 agents.
- **A slow turn of the daemon's loop, or a held helper main thread, is logged
  with its cause.**

### Compatibility

- The hub migrates its dictionary assignments to lists and adds a progress
  column; `activate.sh anterior` restores the pre-activation copy for that.

## [0.10.1] - 2026-10-22

### Fixed

- **Voice notes in the chat could not be started after opening Orbit:** the
  microphone waited for read-aloud to be known, which it only is once something
  has been read aloud. It now depends only on the daemon, the pairing and the
  hub, and a note started from the shortcut also shows in the composer.
- **Claude Code, Codex and the generic assistant could not read attachments**
  with `orbit_attachment`; each agent now reads the files of its own
  conversations only.
- **What an agent says never reaches a log**, also when the ACP SDK cannot
  read an update; and an interrupt, an approval or a clarification no longer
  waits for another agent to open its session.
- **Conversations of the implicit agent continue** when the hub moves to
  `[[agents]]`, and a conversation keeps its agent when the hub changes its
  default.
- **Each agent's voice is known on its own**, and «Escuchar» is only offered
  where a voice can read.
- **A failed turn's work, or the previous turn's, no longer stands for the
  next one**, in the chat and in the macOS notification.
- **Attachments:** a «Ventana» chip removed while uploading no longer comes
  back with the window's text; a hub drop during «Adjuntar ventana» frees it;
  a HEIF photo is recognised by its content, also pasted or misnamed; files
  never sent are deleted after an hour; a stopped hub is named as such.
- **The assistant's folders:** a removed folder is no longer read by a request
  under way; a path outside the folders no longer reveals whether it exists; a
  missing file in another letter case is "missing"; a folder that cannot be
  kept says why; the 20 s bound counts the whole request; nested folders are
  searched once.
- **Texts and refusals that misled:** ACP agents are named as the user knows
  them, the hub's messages and orbit-mcp no longer say «Deimos» or «Hermes» for
  any agent, a skill name with «/», deleting a Hermes skill, a PDF past the
  turn's budget, an image for an agent without vision, «de el fichero».
- **The status and assistant topics** fit up to 32 agents and a turn at its
  ceilings with the agent's work.

### Changed

- The hub's administrator commands start in about 0.4 s: the ACP SDK, like
  the generic assistant's libraries, loads only when such an agent is built.

### Compatibility

- Going back to 0.10.0 works as it is: the hub only adds an index, and the Mac
  writes nothing 0.10.0 does not read.

## [0.10.0] - 2026-10-22

### Upgrading

- **Upgrade the hub first, then the Macs.** A 0.10.0 Mac works with a 0.9.0
  hub, and a 0.9.0 Mac with a 0.10.0 hub, as one agent without attachments;
  the new features need both.
- **A 0.9.0 `orbit.toml` loads unchanged** as one Hermes agent. Several agents
  are configured with `[[agents]]` (see the self-hosting guide); with them,
  `profile` and `assistant_name` move into each entry and are refused at the
  top level.
- Claude Code and Codex agents need their Agent Client Protocol adapters
  installed and signed in on the hub's server; Orbit never handles those
  credentials.

### Added

- `CONTRIBUTING.md`, `SECURITY.md` and this changelog.
- A GitHub Actions workflow that runs the whole Linux gate on every pull
  request and every push to `main`.
- «Carpetas del asistente» in Ajustes: «Añadir carpeta» opens the macOS panel
  (folders only) and the assistant can then list, search and read, never
  change, what those folders hold — text and Markdown, the text of a PDF and
  images — through the new `orbit_files_list`, `orbit_files_search` and
  `orbit_files_read` tools. Nothing outside them is read, also through links
  or `..`; a refusal names the path and the folders.
- The hub can offer several agents: `[[agents]]` in `orbit.toml`. `ready`
  lists them, `chat.send` takes an `agent_id`, and each conversation keeps its
  agent. A 0.9.0 `orbit.toml` is still one agent. Three kinds: `hermes` (a
  Hermes profile); `openai`, a model the hub runs itself against any
  OpenAI-compatible API or Anthropic's, with Orbit's tools and, with
  `[agents.speech]`, read-aloud; and `acp`, Claude Code or Codex through their
  Agent Client Protocol adapters, locked to Orbit's tools and signed in by the
  hub's operator. Dictation is the default agent's. Read-aloud is in the voice
  of the conversation's agent (`agent_id` in `POST /v1/audio/speech`), or the
  default agent's for an agent without one of its own; with no voice at all
  it is refused with `speech_not_configured` and the agent asked for.
- With two or more agents the Mac shows a «Hablar con» selector for a new
  conversation; each conversation keeps and names its agent, and «Oye
  <nombre>» asks the agent named. The agents a hub lists are kept in
  `hub-agents.toml`, so `config.toml` stays readable by 0.8.0 and 0.9.0.
- What the agent does shows in the chat as it happens (`chat.item`): a line
  per running tool, in Spanish for Orbit's own, the plan's steps, its
  reasoning folded, and how long it worked. Approvals and clarifications
  offer the agent's real choices, one question at a time, also from the
  floating panel, and a macOS notification says when a long answer is ready
  or the agent needs an answer while Orbit is in the background.
- Attachments in the chat: «Adjuntar», drag and drop, pasted images,
  «Adjuntar ventana» (the window behind Orbit) and a long paste as «Texto
  pegado.txt». The hub keeps each one encrypted and bound to its
  conversation, within ceilings per file, message and conversation; every
  refusal names the file and its numbers. Images reach the agents that take
  them, a PDF goes as its text and page images, and any other file through
  `orbit-mcp`'s new `orbit_attachment`. The chat shows thumbnails, a viewer
  for images and PDFs, and «Abrir» or «Mostrar en Finder» for the rest.
  «Adjuntar ventana» sends the window's text beside its capture, as a `.txt`
  read through Accessibility (never a secure field, at most 128 KiB). A HEIC
  or HEIF photo chosen on the Mac is attached as a JPEG, upright and without
  its metadata, so without its location.
- Answers in full Markdown: code with its language and «Copiar», tables that
  scroll and copy as Markdown, no raw HTML, images never fetched. Each answer
  can be copied, retried, read aloud («Escuchar»), opened in a Markdown
  editor or quoted in part into the composer; the whole conversation copies
  as Markdown and ⌘⇧C copies the last code block.
- Each agent's personality, skills and access groups can be read and changed
  from Orbit (`/v1/agents/{agent_id}/config`, `…/personality`, `…/access`
  and `…/skills/{name}`). Skills are agentskills.io `SKILL.md` files,
  validated by the standard's rules on `name` and `description`, at most
  64 KiB each and 128 per agent; a personality is at most 16 KiB. An `openai`
  or `acp` agent is told its personality and each enabled skill's name and
  description (an `openai` agent at every turn, Claude Code in its session's
  system prompt, Codex in its folder's `AGENTS.md`) and loads a skill's
  instructions with `orbit-mcp`'s new `orbit_skill`, which belongs to no
  access group and refuses a skill not enabled with the enabled ones; a
  `hermes` agent's are its profile's `SOUL.md` and skills, through Hermes's
  panel API, and it is not offered `orbit_skill`. The five access groups
  (screen, control, calendar, meetings, files; all on by default) are
  enforced by `orbit-mcp`, which no longer lists or runs a tool of a group
  that is off, and by the hub, which refuses that group's bridge requests
  with `agent_access_disabled`, the agent and the group. They configure an
  agent's tools and are not a boundary against a hostile local process.
- Ajustes › Agentes: each agent's personality (in the Markdown editor), its
  skills (switch on or off, view, edit, create, paste or import a folder's
  `SKILL.md`, delete) and its access groups («Pantalla», «Control del Mac»,
  «Calendario», «Reuniones y notas», «Archivos»), each switch naming the
  tools it opens. What an agent's kind does not let Orbit change is shown
  disabled with its cause; every refusal of the hub says its cause and its
  numbers. The Mac keeps none of it: it is read from the hub and changed there.

### Changed

- Developer tooling is in English. The gate's `--instalar` option is now
  `--install`; the macOS scripts moved to `scripts/macos/` (`bundle.sh`,
  `check-sdk.sh`, `smoke-native.py`) and the release scripts to
  `scripts/release/`; the real-test harness is `scripts/e2e-mac/real_test.py`
  with the subcommands `prepare`, `run` and `dry-run`, and Orbit Pruebas is
  built with `scripts/e2e-mac/build-test-app.sh`.
- The `orbit-update` crate is now a module of `orbit-app-core`.
- The hub's messages name the conversation's agent («Se perdió la conexión
  con Vega…»), or «el asistente» when it has left the configuration, instead
  of «Hermes».
- The hub's administrator commands (`pair-code`, `devices`, `revoke`) start
  in about 0.6 s instead of 2.5 s: the generic assistant's libraries load only
  when an `openai` agent is built.

### Fixed

- A Claude Code or Codex turn that fails ends with its cause and numbers:
  `agent_auth_required`, `agent_rejected` (`acp_code`), `agent_stopped`
  (`exit_status` or `max_bytes`), `agent_protocol`, or `agent_turn_unfinished`
  (`stop_reason`), instead of a completed message with status `error`.
- A failed turn says why in the chat, naming the conversation's agent and
  what to do, with its numbers: every `assistant_*` and `agent_*` cause and
  `upstream_turn_failed`. Before, all but the attachment refusals showed «Algo
  ha fallado. Contacta con el administrador (código …)».
- A live approval is pending as soon as it arrives: the Mac compared the
  agent's request id with the turn's, so every approval looked stale until
  the next snapshot of the state.
- Voice notes in the chat: a click on the microphone starts a note and the next
  click, or «Enviar nota de voz», sends it; holding the button still talks while
  held, and releasing it outside the button now counts. Esc cancels the note
  without sending it. While it records, the composer says «Grabando nota de voz»
  with its time. A note shorter than 160 ms is not sent, and the composer says
  its length and that minimum. A note whose transcription failed shows
  «Reintentar transcripción», which sends the audio this Mac kept again.
- When a meeting note is queued for a retry, the refusal the assistant gets
  now says how long until that retry, instead of a fixed 45 seconds.
- Three hub refusals now carry their numbers (fields added, codes unchanged):
  every storage-quota refusal says the bytes of the write, what admission
  reserves for it, what the hub uses and the quota; an HTTP body over its
  ceiling names what it was (`capture`, `recording`, `audio` or
  `hub_request`) instead of always `ipc_request`; and `speech_too_large` says
  the clip's size and its ceiling.
- On a fresh machine the gate prepares the hub's environment before the guards,
  whose hub publish tests build a release with `uv` offline from its cache.
### Compatibility

- The 0.10.0 hub adds an `agent_id` column to its conversations and tables for
  each agent's settings and for attachments. A 0.9.0 hub cannot create
  conversations in that database («table sessions has 8 columns but 7 values
  were supplied», measured on a synthetic database). To go back, stop the hub
  and run `ALTER TABLE sessions DROP COLUMN agent_id` on its database (SQLite
  3.35 or newer): 0.9.0 then works, every conversation belongs to the default
  agent, and attachments and agent settings are no longer read.
- The Mac's saved state only gains optional fields that 0.9.0 ignores; the
  hub's agent list goes in a new file, `hub-agents.toml`, and `config.toml` is
  written as 0.9.0 reads it. To go back, quit Orbit and install 0.9.0. It does
  not show the assistant folders, which 0.10.0 finds again.
- Downgrading the Mac has not been tested.

## [0.9.0] - 2026-10-21

The first open beta.

### Upgrading

- **Upgrade the hub first, then the Macs.**
- **The hub's default Hermes profile is now `default`** (it was `deimos`). Set
  `profile` in `orbit.toml` to the profile your hub uses before starting 0.9.0.
  The assistant name is set separately with `assistant_name` and still defaults
  to Deimos.
- **Orbit now installs in `/Applications`** (previously the user's
  `~/Applications`). The copy in `~/Applications` is removed once the new app
  has opened your data. macOS permissions carry over when the new build has the
  same signature.

### Added

- **Transcribe again** («Volver a transcribir»): in the Transcription tab of
  any closed meeting, the hub repeats the final pass with separated voices and
  rewrites the note. This is meant for meetings recorded before 0.8.0. The
  previous transcript stays visible meanwhile, with whether the pass is queued
  or transcribing.
- **Dictionaries.** Select a mistranscribed word and type the right one: it is
  corrected in every meeting that uses that dictionary, without changing what
  was transcribed (removing the correction undoes it). The note receives the
  terms, and the transcription service receives them as a hint. Each
  dictionary has a name and is chosen per meeting series; one dictionary can
  serve several series. Settings lists them.
- **Update notice.** Orbit checks for a new version at startup and every six
  hours. When there is one, it shows a notice with «Actualizar» and a line in
  the menu bar menu. It never installs by itself, nor during a recording or a
  call.
- **The assistant's name is configurable** on the hub; it defaults to Deimos.

### Changed

- **Previous copies of the app are cleaned up:** the previous version is kept
  until the new one starts correctly, and is then removed.
- **Meeting audio leaves the Mac once the hub has it.** Audio already uploaded
  is removed gradually; voice notes stay on the Mac.
- **If audio drops in the middle of a meeting** (for example, wireless
  headphones that disconnect and come back), the recording continues and says
  how much was lost. A voice note is cut once, with its cause.
- **Quitting Orbit by name** (AppleScript, Shortcuts) closes it in order with a
  single command, also while the control engine is running.

### Fixed

- Renaming a speaker turn that the transcript no longer has now says which
  turn and how many there are now.
- When the “Oye Deimos” wake phrase abandons a question, the log says why.
- «Orbit sigue guardando. No lo cierres todavía.» (“Orbit is still saving.
  Don't close it yet.”) appears only when closing, and a repeated notice no
  longer stacks.
- A voice note no longer waits up to 5 seconds for the transcription service.
- If uploading a screenshot fails, the assistant is told that the screenshot
  was the cause.

### Compatibility

- The 0.9.0 hub only adds the two dictionary tables to its database, which
  0.8.0 does not read, so a 0.8.0 hub can open it again; corrections are then
  hidden (the transcript underneath is intact) and transcribing again is not
  available. Meetings already transcribed again keep the new transcript.
- The Mac's data format does not change in 0.9.0, so 0.8.0 can open it. To go
  back, quit Orbit, delete `/Applications/Orbit.app` and install 0.8.0 with its
  installer, which places it in `~/Applications`. Audio that 0.9.0 already
  removed from the Mac is not restored: it is on the hub.
- Downgrading has not been tested.
