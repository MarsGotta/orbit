# Changelog

All notable changes to Orbit are documented in this file. The format is based
on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Orbit uses
[semantic versioning](https://semver.org/spec/v2.0.0.html). The app's interface
is in Spanish; the labels quoted below are the ones it shows.

## [Unreleased]

### Fixed

- **A recording no longer stops at its calendar event's end:** a recording
  started from Hoy or the calendar stopped fifteen minutes after the event, and
  one started by hand, from the menu bar or with ⌃⌥M after an hour. Every
  recording now runs until it is finished, with one safety ceiling of four
  hours against a forgotten recording, whatever started it.
- **Ten minutes before that ceiling, Orbit says so once**, with how long the
  recording has run and what is left. «Nueva grabación», the recording card
  and Hoy say the same ceiling («Graba hasta 4 h»), and Ajustes › «Cuánto dura
  cada cosa» says what lasts how long: a recording up to 4 h, a call from the
  app up to 1 h, one begun with ⌃⌥D up to 15 min.
- **A hub that stops answering during a recording does not stop it:** the
  recording goes on, its audio waits on the Mac and reaches the hub once,
  whole, when the hub answers again (now covered by an end-to-end test).

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
