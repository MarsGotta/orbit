# Transcription model selection

The hub owns one preference shared by every paired Mac. Ajustes › Reuniones ›
Transcripción reads and writes it through the existing authenticated, typed app
bridge. `whisper` is the default; `mai` selects `microsoft/mai-transcribe-2`.
There is no device-side provider client or credential cache.

## Audio paths

| Entry | Whisper selected | MAI selected |
|---|---|---|
| Live meeting draft (`TranscriptionWorker`) | Existing hub STT, its grouping and glossary prompt | OpenRouter JSON STT, the same groups, dictionary phrases |
| Final pass (`FinalPassWorker`) | Existing Opus windows, word timestamps and Whisper VAD | Same windows of at most 45 seconds, MAI word timestamps |
| Voice notes, dictated input and derived call utterances (`POST /v1/audio/transcribe`) | Existing default agent STT (Hermes or its configured speech adapter) | OpenRouter JSON STT, each existing utterance segment |
| Meeting question wake phrase (`SpeechPhraseListener.swift`) | Apple's on-device SpeechAnalyzer | Unchanged: on-device SpeechAnalyzer |

No path must remain on Whisper when MAI is selected. The wake-phrase listener
remains on Apple speech recognition because it detects the local trigger and
collects the spoken question with low latency; it does not generate the hub's
saved transcript. Meeting speaker separation remains local pyannote on the
remote meeting track. MAI diarization is disabled; the hub assigns its normalized
words to tracks and pyannote voices with the existing timestamp rules.

The existing read-time dictionary corrections remain. MAI receives all resolved
meeting/series dictionary terms as `provider.options.azure.phraseList.phrases`,
independently of Whisper's shorter glossary prompt. Transcription style is
`verbatim` under `enhancedMode.modelOptions.transcribeStyle`.

## Credentials and failure

`transcription_preferences` contains one AES-GCM encrypted JSON document,
using the hub storage key and its own authenticated context, just like assistant
credentials. Only the selected model and a `key_configured` boolean leave the
hub. A replacement key is validated by a real transcription of 250 ms of
synthetic silence before saving. This can incur a small provider charge. A failed
validation preserves the existing key and preference; provider bodies are never
returned or logged. Key, permission, balance, throttling and transport failures
have separate sanitized causes. Rust debug output redacts the key and app-core
removes it from the returned call envelope.

Draft groups pin their model when first claimed, including retries. Final passes
pin it when claimed or explicitly requested. A preference change cannot reroute
a retry or later window of the same job. Existing group backoff/attempt limits
and final-pass retry behavior remain. A MAI failure never calls Whisper. Existing
final-pass integrity checks may retain already available draft spans and report
`kept_draft`; this does not send audio to another provider.

Every MAI request sends `provider.zdr: true`. Per OpenRouter's
[ZDR policy](https://openrouter.ai/docs/guides/features/zdr), this restricts
routing to endpoints classified as zero data retention. It does not keep audio
on the hub: audio still reaches Microsoft through OpenRouter, and routing can
fail if no eligible endpoint is available. OpenRouter treats transient in-memory
caching separately from retention; its account logging settings still matter.
Orbit does not independently verify either company's retention behavior.

The request shape follows the [STT guide](https://openrouter.ai/docs/guides/overview/multimodal/stt)
and [model documentation](https://openrouter.ai/microsoft/mai-transcribe-2),
checked on 2026-10-27. It uses JSON/base64, never multipart.

## HTTP compatibility

- `GET /v1/settings/transcription` returns
  `{ "model": "whisper", "key_configured": false }` before configuration.
- `PUT /v1/settings/transcription` accepts `model` (`whisper` or `mai`) and
  optional `api_key`. Omitting the key retains the encrypted one. Selecting MAI
  requires a valid key. Every paired, active Mac can change this per-hub setting.
- `POST /v1/recordings/{id}/final-pass` accepts an optional `{ "model": "mai" }`
  or `{ "model": "whisper" }`. An empty body keeps older Macs compatible and
  uses the hub preference. This comparison does not change the preference or
  regenerate the note. `transcript.final_pass.model` identifies the pinned model;
  older stored jobs omit it until claimed.

Deploy the hub before the app. Schema additions preserve existing data; older
Macs ignore the new response fields. Keep the existing hub Whisper service
configuration so that selecting it continues to work.

## Reviewer acceptance (manual, uses real providers)

Do not use private recordings. On a consenting test Mac, synthesize **all three
parts** of `scripts/e2e-mac/meeting-script.txt`, in order, omitting comments,
`A:`/`B:` prefixes and `---` separators. `say -f` can write an AIFF without playing
it; convert that file with `ffmpeg -ar 16000 -ac 1 -c:a pcm_s16le` to a WAV. No
Orbit session, microphone, SSH or live Hermes profile is needed for this check.

Run from this checkout with the locked hub environment, an explicit isolated
Whisper endpoint and the reviewer's environment key:

```sh
# Set OPENROUTER_API_KEY outside shell history; never put it in an argument.
server/.venv/bin/python scripts/e2e-mac/compare-transcription.py \
  --audio /tmp/synthetic-meeting.wav \
  --whisper-endpoint https://whisper.example.test \
  --whisper-model whisper-large-v3
```

The script reads that synthetic WAV in 45-second windows and uses both production
adapters. It prints only each model's percentage of script words recovered
(case/accent-insensitive multiset recall, counting repetitions). It prints no
transcript, response body, credential or audio. A failure exits with a sanitized
cause and no percentages. These are recall percentages for one synthetic script,
not WER, diarization accuracy, a model benchmark or real-microphone acceptance.
The automated suite uses local fake endpoints and never executes this script
against an account.
