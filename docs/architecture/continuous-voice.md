# Continuous voice: policy and boundaries

See the [overview](README.md#voice-note-and-call) for the hub/Hermes flow.

## Authorization and processing

`CallStart` (`⌃⌥D`) is explicit local authorization for 1 minute to 4 hours
(`orbit-core/src/transition/voice_call.rs`). A call lasts `CALL_MINUTES`, which is
a recording's ceiling (`RECORDING_CEILING_MINUTES`, 240), from the app unless the
order names a length and always from `⌃⌥D`. Ten minutes before it ends the call
says so once (`call_ceiling_near`, as `recording_ceiling_near`).
The call keeps its origin (`app` or `hotkey`), and the app's call snapshot
carries its deadline (`expires_at_ms`), origin, agent and the idle hang-up
(`idle_hangup_ms`) to every view. An open recording yields `PendingRecording`;
another call, voice note or active turn yields `AudioSessionBusy`.
The daemon persists Starting before requesting a VoiceProcessing microphone.
API support is not proof of actual processing: the ACK must report mode, actual
enablement, bypass off, AGC off, `shared_engine` playback reference and pipeline
identity (`CallStartReceipt`, `daemon/call_receipts.rs`). Every frame must retain that
provenance. A change/loss stops the call; privacy, disconnect, failure and expiry
never auto-resume it.

One producer feeds both the continuous original and derived turns. Silence is
retained in the original. The manifest records `voice_call`, requested processing
and `aec_hardware_validated:false`; `audio_processing_receipt` keeps native
readback. This does not certify acoustic AEC effectiveness. The original uploads
to the hub but is neither listed as a meeting nor given a meeting note.

## Turn detector

`orbit-core::voice` is a pure state machine with injected time/observations.
PCM16 energy is a segmentation cue, not speaker identity, wake phrase,
authorization or proof of echo cancellation. `VoicePolicy::default` sets:

- 300 ms preroll, 120 ms onset, 200 ms barge-in.
- Turn end after 700 ms silence; minimum speech 160 ms, maximum turn 20 seconds.
  Final audio is kept before interactive STT starts.
- Integer RMS 500/32768, still uncalibrated on real microphones.
- One minute of user silence ends the call, under the rules below.

Hermes transcribes turns through `POST /v1/audio/transcribe`. Their encrypted
audio stays local (`upload_allowed:false`). Text travels as `chat.send` with
`call_active:true`; `POST /v1/audio/speech` and native playback speak the reply,
in the voice of the call's agent (`agent_id`), or the default agent's when that
agent has no voice of its own. Transcription stays the default agent's.

## Automatic stops

`StopReason` (`orbit-core/src/voice.rs`) distinguishes `unauthorized`,
`generation_changed`, `processing_lost`, `discontinuity`, `expired`, `idle` and
`no_audio`.

- **`idle`:** 60 seconds without the user's words (`VoicePolicy::idle_timeout_ms`).
  Words, not energy: a turn's transcription with text restarts the minute
  (`TurnDetector::heard_words`); a turn the hub finds no words in (noise, a
  television) does not. Active playback, speech generation or a request the
  assistant works on advances activity on each frame, so the minute begins
  after the response ends, and muted time never counts. A turn in course
  (captured, or its words asked for) holds the hang-up for at most one turn's
  length (`maximum_turn_ms`), so unbroken noise cannot keep a call open. It
  closes a microphone left open after the conversation.
- **`no_audio`:** three seconds with no microphone frames
  (`CALL_AUDIO_SILENCE_LIMIT`, `daemon/call.rs`). This measures producer health,
  not user silence, which still generates audio. Maintenance judges it only
  after queued helper events are drained (`note_loop_tick`, `check_call_health`).
  Queued frames received during a stalled loop are not missing audio. Three
  seconds matches the helper's `no_capture_frames` window.
- The diagnostic `call_stopped_idle` carries cause, last-frame age, user silence,
  limit, queued-event count, assistant playback, last loop tick duration and
  heard-frame count.

A previous investigation's proposed queue-delay cause was refuted by
`call_audio_heard` measurements: frames arrived on time; the idle timer had
counted assistant work/speech as user silence. The queue check also prevents
a delayed loop from misclassifying queued audio.

## Barge-in

The producer remains open during a response. Sustained onset of 200 ms opens
an interrupting turn (`TurnAction::Begin {interrupt:true}`).

While the assistant answers, onset sends `playback_pause` through the helper's
immediate command lane. The scheduled buffer and its position stay intact,
including when synthesis finishes after the pause. The turn is captured and
transcribed from memory (`hear_over_answer`, `daemon/voice_dispatch.rs`); it is
not stored, shown or counted yet. Words cancel the reading and its queued parts
and become the turn (`accept_over_answer`). No words resume that same reading
and leave no note (`voice_turn_dropped`, `no_words`). A transcription failure
keeps a failed note and resumes the reading. Noise can pause playback but can
never cancel it or open an assistant turn.

Before this change, playback waited for the end of the sentence plus STT before
stopping. The previous noise guarantee is retained by separating pause from
cancellation. Speech requests for the next answer are armed after interrupt
retirement, so retirement cannot silence the next answer.

A turn the hub hears no words in leaves no note in any case; in a call its
audio waits for what the user says next, which carries it (`carried`), within
the bounds of a join. A candidate below the submission minimum returns both
its own PCM and any joined prefix to `carried`; discarding it used to leave
`joined` to be overwritten at the next onset, losing the beginning.

When the user's own previous turn is still transcribing,
`prepare_call_frame`/`joinable_turn` decide what happens to it:

- If it is still transcribing and has not reached Hermes, cancel its old STT
  with `CallTurnJoined` and prepend its audio to the new turn
  (`prepend_joined_call_turn`). Frames already counted are not counted twice.
  Its own note leaves the conversation: the new turn carries its audio.
  If the combined turn exceeds 60 seconds/the encrypted chunk ceiling or changes
  format, discard the old audio and report numbers in `call_turn_join_refused`.
- Otherwise the new turn replaces it (`CallTurnReplaced`).
- Both paths stop playback without changing global generation or restarting
  the microphone. Queued stale results are rejected.
  The frame opening the interruption remains owned by the detector.

Esc cancels the derived turn and requires a silence boundary before the next,
while preserving the session and continuous original.

## Stops are not failures

A helper command overtaken by a stop (`operation_cancelled_by_stop`) is logged
as `native_order_cancelled_by_stop`, not `mac_command_refused`. A chunk awaiting
its storage receipt means closure is waiting; `finish_call_drain` checks again
each maintenance tick. Cancelled voice notes name their specific cause.
Transcription failures carry codes and numbers to the Spanish UI instead of a
generic instruction to record again.

## Verification limits

`playback_active` observes native relay state, not audibility or sample alignment.
Fixtures test transitions and PCM ownership, not false positives, latency,
understanding, loudspeakers or acoustic AEC.

Switching between raw meeting/voice-note audio and VoiceProcessing calls is
rejected while the previous route is active. Real AEC/loopback, device routing
and segmentation still need consented macOS measurements.

Synthetic checks include `cargo test -p orbit-core voice` and
`cargo test -p orbit-daemon --lib daemon::audio::tests::call`, including
`daemon/call_health_tests.rs` and `daemon/call_turn_tests.rs`. They inject facts
and time and do not call real providers. Run tests with an explicit timeout.

## Sequential read-aloud

The daemon splits answer text at sentences, with a 240-character ceiling and
whitespace/Unicode-safe splits for longer sentences. It synthesizes one part at
a time and advances only on the helper's matching `played` receipt. This belongs
in the daemon because it owns playback position, pause and interruption; the hub
continues to synthesize one ordinary clip per request for every speech provider.
The native 60-second decoder ceiling remains independent. If even a short part
exceeds it, the daemon splits that text further before retrying synthesis.

A worker returns the synthesized command with its ticket to the loop. Only a
current ticket can reach the helper, so a completed fetch delivered after an
interrupt cannot restart playback. Cancellation retires all remaining parts;
paused and resumed receipts never count as completion.
