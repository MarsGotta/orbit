# Changelog

All notable changes to Orbit are documented in this file. The format is based
on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Orbit uses
[semantic versioning](https://semver.org/spec/v2.0.0.html). The app's interface
is in Spanish; the labels quoted below are the ones it shows.

## [Unreleased]

### Added

- `CONTRIBUTING.md`, `SECURITY.md` and this changelog.
- A GitHub Actions workflow that runs the whole Linux gate on every pull
  request and every push to `main`.
- The hub can offer several agents: `[[agents]]` in `orbit.toml`, each a
  Hermes profile for now. `ready` lists them, `chat.send` takes an
  `agent_id`, and each conversation keeps its agent. A 0.9.0 `orbit.toml`
  is still one agent.
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
  hub's operator. Dictation and read-aloud are the default agent's; an agent
  without a voice is refused with `speech_not_configured` and its id.

### Changed

- Developer tooling is in English. The gate's `--instalar` option is now
  `--install`; the macOS scripts moved to `scripts/macos/` (`bundle.sh`,
  `check-sdk.sh`, `smoke-native.py`) and the release scripts to
  `scripts/release/`; the real-test harness is `scripts/e2e-mac/real_test.py`
  with the subcommands `prepare`, `run` and `dry-run`, and Orbit Pruebas is
  built with `scripts/e2e-mac/build-test-app.sh`.
- The `orbit-update` crate is now a module of `orbit-app-core`.

### Fixed

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
