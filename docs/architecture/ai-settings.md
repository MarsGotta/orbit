# Hub AI settings

Ajustes › IA owns paired-device overrides; `orbit.toml` remains the operator
configuration. The onboarding step adds no provider by default. Selecting
«Todo en tu hub» preserves the operator's configuration, including existing
remote services; it does not rewrite that configuration.

Providers and independent function choices live in one authenticated,
encrypted `ai_settings` row, using the Store's encryption and capacity check.
Keys are write-only; responses expose `key_configured`. Blank keys preserve an
existing credential only at the same URL; changing the destination never forwards
the old key. The Mac redacts credentials from Debug and from echoed
app answers, including failures. Subscriptions remain assistant-only and are
shown from their existing account status, with a link to Asistentes.

## Consumers and operator defaults

| Function | Existing configuration and consumer | Override |
|---|---|---|
| Meeting transcription | `config.Settings.meeting_transcription`, `meeting_transcription.MeetingTranscriber`; draft `transcription.TranscriptionWorker` and final `meeting_final_pass.FinalPassWorker` | `transcription_preferences.Transcriber` resolves provider/model, retains dictionary hints and pins the choice for durable jobs. Whisper uses the configured endpoint/model; MAI retains its native OpenRouter adapter. |
| Listening: voice notes and calls | `assistant/speech.SpeechBackend`; `assistant/hermes_speech.HermesSpeech` or `assistant/openai_speech.OpenAISpeech`, selected by the default assistant | `Transcriber.interactive` resolves `listening` separately; untouched listening delegates to the original speech backend. Explicit hub listening uses configured Whisper. |
| Assistant speech | Hermes profile or `assistant/openai_settings.SpeechSettings` (`base_url`, `model`, `voice`, optional key file), via `OpenAISpeech` | `ai/consumers.FunctionSpeech` wraps each configured agent and resolves speech per clip. Original voice names are preserved when available; otherwise the compatible endpoint receives `alloy`, also used by the probe. Agents without their own voice retain the default-agent fallback. |
| Meeting summary and note | `config.Settings.processing`, `processing.ProcessingClient`, `notes/worker.NotesWorker` | `ai/consumers.FunctionProcessing` resolves `meeting_notes` once per note, including all map/reduce parts, preserving the operator's reasoning, context and request budgets. An app provider also works without an operator processing entry. |
| Vault distillation and review | Existing vault `maintenance_model`; otherwise the operator processing model | `vault` is stored only. Vault settings read back the effective model; no maintenance job is introduced. |
| Compatible assistants | `assistant/openai_settings.OpenAIAgentSettings` and `assistant/openai.OpenAIBackend` | `ai/consumers.assistant_model` overrides the OpenAI-compatible model at the next turn, preserving tools, instructions and conversation history. Anthropic-key and subscription assistants retain their own configuration. |
| Voice separation | `config.Settings.diarization`, `meeting_diarization`, final-pass separator | Remains pyannote, displayed read-only. |

## Discovery, bounds and privacy

The hub reads `/v1/models` and probes each local model for text, a forced
synthetic tool call, transcription of silence and speech of invented text.
Word timestamps are detected separately; silence without timestamps requires
a manual capability correction before selecting meeting transcription.
Probe requests never contain user material. Corrections change capabilities
of discovered model IDs and cannot silently invalidate functions using them.

The whole validation has a 20-second deadline, including catalog download and
all probes, and refuses a concurrent settings write rather than building an
unbounded wait queue. Local discovery admits 32 models; cloud catalogs admit
2048 and use capability metadata. OpenRouter validates the credential with
the existing MAI silence probe because its catalog itself is public.
Catalog/probe responses are limited to 2 MiB. The complete stored registry is
limited to 512 KiB, leaving room for operator descriptions in the Mac's
existing response envelope, and there are at most 64 configured providers.
These bounds protect settings work and memory; they do not limit inference
count, meeting duration or audio. Provider labels, URLs and keys have shared
UI/hub guards in `contracts/constants.json`.

URLs require HTTPS except loopback HTTP and cannot contain userinfo, queries
or fragments. Discovery, processing and audio requests disable redirects and
environment proxy inheritance. Only loopback endpoints count as on-hub; a LAN endpoint
is explicitly off-hub. Rows identify audio versus text and the destination.
Hermes defaults name the configured Hermes host and state that its own
configuration controls any onward provider.

## Compatibility and verification

The hub creates the new table at schema version 10. On load it migrates the
older encrypted OpenRouter key and meeting/listening selection once. The old
transcription route remains available; an old save updates the compatible
function choices. New function selections do not rewrite `orbit.toml` or
assistant profiles. Durable custom transcription selections pin provider ID
and model; meeting views expose the readable model name. Upgrade hub before
Mac, as for other new settings routes.

Tests use local HTTP servers, synthetic credentials, silence, invented text
and an isolated MCP fixture. They cover discovery, two catalogs, corrections,
refusals, deadline, migration, each consumer, a next-turn provider switch,
unchanged defaults, credential redaction across the app/daemon/HTTP bridge,
privacy notices and onboarding. Native playback, permissions, Keychain,
provider-specific voice availability, real accounts and real model quality
still require consented acceptance on a Mac and an isolated hub.
