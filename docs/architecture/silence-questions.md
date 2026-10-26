# Silence questions

Calls and recordings have no duration ceiling. `orbit-core::silence` owns four
constants, registered in `contracts/constants.json`: call quiet 30 s and answer
15 s; meeting quiet 120 s and answer 30 s. They protect forgotten capture without
cutting an active conversation. They do not limit audio retention or upload size.

The daemon supplies facts to `Event::CheckSilence`. Questions are ephemeral,
scoped to the call or recording and capture epoch; recovery needs new positive
observations. No wall-clock or device effect is read in core. Backward time invalidates a countdown. Missing/stale samples and unread helper
events preserve a pending question and its original deadline, but cannot close it. The app
shows notices and sends choices; it never runs a capture-stop timer.

Calls reuse `TurnDetector::last_activity_ms` and transcription results. Assistant
speech/synthesis, an open turn, a pending approval or Mac action pause the clock.
A captured candidate or transcription may hold the initial question for at most
one maximum turn (20 s); otherwise continuous noise could postpone it forever.
Only transcribed words answer. Exact short presence replies («sí», «sigo»,
«aquí», «espera», «mantén», «no cuelgues») are consumed locally while asked;
«Cuelga» hangs up; other words become a normal assistant turn. Noise retains the
existing pause/resume guarantee and never cancels the assistant or opens a turn.
The local question uses the existing agent speech reader, without a model turn.
It does not reset its own countdown. Unread audio or missing call observations
preserve the question without authorizing a hangup; the next usable observation
can finish an expired countdown. Detector noise never answers a pending question. Missing voice falls back to the notice and
the existing macOS notification channel and system sound (`src-tauri/src/notify.rs`).

Recordings reuse `TrackSignalMonitor`, fed from the validated PCM already
measured by `measure_s16le` in capture ingestion. Each requested side must have
120 s below `VoicePolicy::default().rms_threshold` (500 RMS S16) and a recent
delivery (under the existing 3 s timeout). Energy at or above that call speech
threshold resets the quiet counter on either side. The separate low-signal
diagnostic threshold (32) still detects a dead or muted input; room noise
between 32 and 500 is quiet for this question without raising that diagnostic.
This reuses the existing squared-energy measurement, not a speech classifier. No new native measurement or Swift change is
needed. Only the microphone is required for microphone-only recordings; a muted
microphone-only recording cannot establish silence and stays open. With a remote
side, muting the microphone makes that side the only silence input.

A missing speech-quiet or voiced-frame observation, including one from an older
producer, is unknown and cannot close a recording. The monitor retains a
cumulative voiced-frame counter using the same measurement: speech followed by
a quiet buffer between daemon checks must still withdraw the question. A keep
choice or positive voice on either requested side restarts the quiet interval.
A muted microphone does not withdraw it. Unknown data neither withdraws the
question nor starts another; a missing view cannot replace its epoch with an
empty one. The notice is emitted once, as `working` (waiting for the person),
rather than a failure.

At the original 30 s deadline, both requested sides must have fresh quiet
observations. A delivery gap can restart a track's initial two-minute counter,
but a pending question does not need another two minutes. If data is missing at
expiry, the question remains open until fresh quiet permits the normal close or
positive voice/«Seguir grabando» withdraws it. Nothing closes just because data
is missing. This invokes exactly `FinishMeeting`: the helper
drains, every final chunk is sealed, closure is queued durably, and the existing
hub finalization writes the note. `close_unconfirmed` still bounds the Mac's
close acknowledgement, independently of silence or meeting duration.

Retired `recording_ceiling_near`, `recording_ceiling_reached`, `call_ceiling_near`
and `call_expired` codes retain their wire names and positions and are silent,
as other retired notices are. Old duration arguments and saved deadline fields
still decode; new records use the unbounded sentinel and presentation exposes
no end time. Mixed-version hub/Mac operation adds no hub message or schema.
Older Macs keep their previous behavior until upgraded.

Linux tests use synthetic frames, transcription results and supplied time.
Real-test check 19 starts a microphone-only recording without playing audio,
observes its question without answering, then requires normal capture closure,
the saved meeting in Reuniones and chunks plus its closure manifest on the hub.
Its wait budgets read the production constants; the gate checks its dry run.
The saved wait includes the answer window, one further answer interval to observe
fresh quiet after expiry, the existing sample-delivery tick margin, audio drain
and hub HTTP budgets. This finite test budget does not impose a production
delivery deadline: indefinitely missing samples keep capture open. Real
Mac acceptance still needs both call and meeting trials: audibility and timing
of the question, short replies under AEC, notice sound when voice is unavailable,
real room energy, both capture sources and mute, and complete saved audio after
a silent meeting closes. No mock test establishes those properties.
