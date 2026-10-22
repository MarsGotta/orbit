# Meeting transcription

Transcription belongs to the hub. The Mac uploads audio and reads results without
keeping a hub transcript copy. See [self-hosting](../self-hosting.md) for setup
and the [hub contract](../../contracts/hub-v1.md) for fields and errors.

**Finalized**, **complete** and **ready** are distinct: finalized means the closure
manifest arrived; complete requires both tracks and no declared gaps, regardless
of recognition state; ready additionally requires all chunks transcribed.
Recognition failure does not make a complete capture incomplete. Automatic text
is a draft, not verification of what was said.

## Mac responsibilities

- Seal one second per track (`mic_local`/`remote_app`) and upload with
  `PUT /v1/chunks/{id}` plus time, monotonic clock, first sample, frame count,
  format and epoch. Finalize with tracks, gaps, title, kind and optional event.
- Read `GET /v1/recordings`, at most 50 per page, adding only local in-progress
  or not-yet-uploaded recordings. Open documents with `GET /v1/recordings/{id}`
  and turns with `GET …/turns?offset=`. Turn pages carry 256 KiB; app-core reads
  at most 64 pages (`MAX_TURN_PAGES`). The daemon passes documents/turns through;
  it has no speaker, timing or language rules.
- On `recording.changed` advance the `notes` topic revision. app-core rereads
  the list/open note; there is no periodic polling.
- Send user commands for voices, title, language, private notes, note edits,
  regeneration, failed-group retry, dictionaries and final-pass retry.
- Publish `PUT /v1/device/profile` with the Mac account's owner name for the
  local microphone (`daemon/recording_notes.rs`).

Voice notes and derived call turns use Hermes' interactive
`POST /v1/audio/transcribe`; their audio stays on the Mac. The continuous call
original uploads as `voice_call` without a meeting note.

## Draft groups

`server/orbit_hub/grouping.py` groups contiguous audio from one track, targeting
20 seconds (`TARGET_SECONDS`), at most 128 sources/20 MiB. It does not mix tracks,
formats or epochs, insert silence or resample. Sequence, sample counters, format,
epoch and native-clock continuity are checked with 50 ms jitter tolerance
(`MAX_CLOCK_SKEW_NS`). Discontinuity closes the group.

A sequence gap may be an upload in transit, so it waits for the deadline or closure.
Partial groups flush on finalization, discontinuity or 30 seconds after the first
durable receipt (`FLUSH_AFTER_SECONDS`). This is queue admission, not a latency
promise.

Sealed groups are immutable. `group_id` derives from device, recording, track,
pipeline version and ordered source provenance. A chunk belongs to at most one
group. Late sources can form new groups but never reopen a sealed transcript.
Draft groups do not overlap; there is no boundary-word deduplication or word-level
alignment in this draft path. Failed groups become terminal after five attempts;
Retry queues them again. Legacy pipeline-v1 ASR reads as a one-source group with
`chunk` timing, without another provider call.

One inference worker serves interactive dictation ahead of meeting groups between
inferences; it does not interrupt an active inference.

## Transcription service

`MeetingTranscriber` calls `POST <endpoint>/v1/audio/transcriptions` with model,
`verbose_json`, recording language and an optional bounded prompt. The
`[meeting_transcription]` table is mandatory. HTTP is permitted only on loopback;
otherwise HTTPS. Requests allow 300 seconds, connection 10 seconds, audio 24 MiB.
Provider error text is excluded from logs because it can contain transcript text.

Before draft recognition, `trim_digital_silence` removes leading/trailing PCM
magnitude ≤4 of 32767 (about −78 dBFS). This prevents observed hallucinations on
near-zero padding; it does not remove ordinary microphone noise. Entirely digital
silence is stored as empty text without calling the service.

## Final pass and voices

Closing a meeting also durably queues a final pass. The hub keeps the draft
available while it transcribes retained audio and separates remote voices locally.
Without `[diarization]`, this pass explicitly fails `diarization_not_configured`;
it does not fabricate voices.

Audio becomes mono 16 kHz, constant 24 kbit/s Opus. Windows end at the midpoint of
the longest silence between 15 and 45 seconds (−48 dB, at least 0.3 seconds,
`ffmpeg silencedetect`), or at 45 seconds if none qualifies. All audio is retained.
Requests use word timestamps and `vad_filter=true`. Relative word times shift to
track time. The local separator returns intervals without stored embeddings.
Voices are reused for language changes and repeated recognition of the same audio.

Remote voices are numbered `voice_1`, `voice_2` by first appearance, displayed as
“Persona 1”, “Persona 2”. The user can rename/merge them. Remote words use the
largest overlap or closest voice within 250 ms; more distant words are omitted.
This filtering does not affect the local microphone.

Before publishing both tracks atomically, the pass compares each track with its
draft groups. Groups with at least six words must retain at least a quarter of
their words by word-start time within the group's interval ±1000 ms. A missing
draft is unknown, not proof of loss. A lost span or repetition over both 66 words
and 223 permille fails `final_pass_incomplete`, retaining prior text/voices.
The error exposes track phase, measured counts and every rule threshold. See the
[hub contract](../../contracts/hub-v1.md#final-pass-and-recording-voices) for all fields.

`POST …/final-pass` queues another pass for a completed or failed recording.
An already queued/running job is not duplicated. Language changes and retries
keep the prior text until a whole new result is published. Transcribing again
leaves the note as it is; rewriting it is the separate `POST …/note`, which
keeps the hand-edited version. A running pass reports the audio it has
transcribed of the total and, after its first window, an estimate of the time
left. Speaker corrections refer to actual turn time/key, not list position.

## One turn-rendering rule

`server/orbit_hub/meetings/turns.py` supplies the Mac, private assistant bridge,
note model and meeting-question context.

- Times are relative to the manifest start, or first chunk while open. Unknown
  origin uses 0 as a read representation, not measured zero.
- In the draft, speaker key is the recognizer label if present, otherwise track.
  Current draft groups have `speaker:unknown`, yielding `mic_local`/`remote_app`.
- Names prefer user overrides, then device owner for the microphone (otherwise
  “Micrófono local”), otherwise “Otra persona”. Final-pass voices use their
  numbered names. Renaming affects reads and future notes, not existing note text.
- Empty groups are omitted. Adjacent draft groups of one voice starting within
  60 seconds of the turn's first group merge (`MERGE_WINDOW_MS`).
- Text renders `[hh:mm:ss] Name: text`. Final-pass turns use measured word endpoints.
- Meeting dictionaries apply whole-word/phrase corrections on read in one
  case-insensitive pass, longest phrase first. A meeting or series uses
  several; when two correct a heard form differently, the first in the
  device's dictionary order wins. Originals remain intact; removing an entry
  reverses its correction. All consumers share this read.

Dictionary terms also supply a conservative 160-character glossary prompt
(including `Glosario: `, separators and final period), most recent complete terms
first, the first dictionary's counting as the most recent. This does not
guarantee a token count for arbitrary Unicode.

## Language

Each meeting uses its explicit two-letter ISO 639-1 override, otherwise the most
frequent language among groups with text, otherwise configured language.
Unknown remains null if neither recognized nor configured; it is never “auto”.
Whisper receives the recording's language; returned language normalizes to two
letters, falling back to the sent one.

Changing language repeats groups with text in another language and in-flight
recognition, retaining old text meanwhile. The document reports
`transcript.retranscribing {language,done,total}`. Existing notes regenerate,
preserving edits as superseded. The same language does not repeat work.
Unreadable retained group audio returns `409 meeting_audio_unreadable` without
changing the recording. Failed groups without text use the new language when
explicitly retried. The UI offers Spanish, English, Catalan, French, Portuguese,
German and Italian.

## Notes and meeting questions

The hub's `server/orbit_hub/notes/` worker uses `[processing]`. Without it,
transcription still works and notes report no model configured.

Finalizing a meeting requests its note; calls do not. `POST …/note` selects a
template and regenerates. The worker waits for the final pass; if it fails,
it waits for the draft to finish and uses it, including terminal failures.
It preserves `final_pass_error` and its numbers. Waiting notes are revisited every
10 seconds; only one model generation runs at once.

Inputs include title/event, device owner, voice names, private notes, glossary,
series/default template and rendered turns. Long transcripts split into contiguous
parts and are reduced. The final prompt block specifies the meeting language:
putting that instruction only at the beginning proved insufficient with a
Spanish surrounding prompt. Unnamed “Persona N” is never invented as a task owner.

Transient provider slowness/unavailability/busy/unreadable responses retry after
30 seconds, 2 minutes and 10 minutes; other failures wait for intervention and
expose causes/counts. Every requested/generating/completed/failed/edited note
change emits `recording.changed`.

Meeting questions arrive as `chat.send` with `meeting {recording_id,minutes,title}`.
`meeting_context.py` includes the newest complete rendered lines within the requested
window and 31 KiB, counting omitted material. Stored history keeps only the
question and Spanish meeting marker. Another device's recording yields
`meeting_not_found`.

## Verification limits

Synthetic PCM does not establish real ASR quality, bilingual quality, speaker
identity, word alignment or acoustic capture completeness. Consent-based real
audio evaluation is separate from protocol and storage tests.
