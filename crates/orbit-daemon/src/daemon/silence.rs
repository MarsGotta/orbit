//! Read existing observations in the actor; a blocked actor cannot close a
//! meeting from a window timer. Delayed/missing track delivery is unknown.
use super::*;
use orbit_core::silence::{CallQuiet, PresenceReply, presence_reply};

impl Runtime {
    pub(super) async fn check_silence(&mut self) -> Result<()> {
        self.check_silence_at(now()).await
    }

    pub(super) async fn check_silence_at(&mut self, at: i64) -> Result<()> {
        let pending = self.state.views.silence_questions.call.pending();
        let call = self
            .audio
            .call
            .as_ref()
            .filter(|c| c.closing_at_ms.is_none())
            .and_then(|c| {
                let prompt = self.presence_question_playback();
                let speaking = !prompt
                    && (c.playback_active
                        || self.assistant.speech_task.active().is_some()
                        || self.assistant.read_aloud.is_some()
                        || self.assistant.speech_paused);
                let silent_ms = at.saturating_sub(c.detector.last_activity_ms());
                let user_turn = !pending
                    && (c.detector.is_capturing() || self.assistant.voice_task.active().is_some())
                    && silent_ms
                        < orbit_core::silence::CALL_SILENCE_MS
                            + orbit_core::voice::VoicePolicy::default().maximum_turn_ms as i64;
                let busy = speaking
                    || user_turn
                    || self.state.durable.assistant.active_request.is_some()
                    || self.state.durable.assistant.pending_approval.is_some()
                    || self.computer.work.active().is_some();
                let observed = !self.microphone_muted()
                    && self.audio.queued_native_events == 0
                    && c.last_audio.elapsed() < call::CALL_AUDIO_SILENCE_LIMIT;
                (observed || busy).then_some(CallQuiet { silent_ms, busy })
            });
        let recording = (self.audio.queued_native_events == 0)
            .then(|| self.recording_signal_view())
            .flatten()
            .map(Box::new);
        if call.is_some() || self.state.durable.capture.recording.is_some() {
            let event = Event::CheckSilence {
                call,
                recording,
                microphone_muted: self.microphone_muted(),
            };
            self.execute_decision(decide(&self.state, event, at)?)
                .await?;
            if pending
                && !self.state.views.silence_questions.call.pending()
                && self.presence_question_speaking()
            {
                self.stop_playback().await?;
            }
        }
        Ok(())
    }

    pub(super) fn call_assistant_working(&self) -> bool {
        self.state.durable.assistant.active_request.is_some()
            || self.state.durable.assistant.pending_approval.is_some()
            || self.computer.work.active().is_some()
    }

    pub(super) async fn keep_call_command(&mut self) -> Result<()> {
        self.audio.note_call_words();
        if self.presence_question_speaking() {
            self.stop_playback().await?;
        }
        self.apply(Event::CallKeep).await
    }

    /// Playback flags can arrive after the question's played receipt. Keep
    /// their identity until the question resolves, or another reading starts.
    pub(super) fn presence_question_playback(&self) -> bool {
        self.state.views.silence_questions.call.pending()
            && (self.assistant.read_aloud.is_none() || self.presence_question_speaking())
    }

    /// True only for the local question, never an assistant answer.
    pub(super) fn presence_question_speaking(&self) -> bool {
        self.assistant
            .read_aloud
            .as_ref()
            .is_some_and(|r| r.message_id == "call-presence")
    }

    /// Called after words were transcribed, before interruption or submission.
    pub(super) async fn answer_call_presence(&mut self, text: &str) -> Result<bool> {
        if self.state.durable.voice.call.is_none() {
            return Ok(false);
        }
        let pending = self.state.views.silence_questions.call.pending();
        self.audio.note_call_words();
        let reply = presence_reply(text);
        if reply == PresenceReply::HangUp {
            self.apply(Event::CallStop).await?;
            return Ok(true);
        }
        if pending {
            self.apply(Event::CallKeep).await?;
            if self.presence_question_speaking() {
                self.stop_playback().await?;
            }
        }
        Ok(pending && reply == PresenceReply::Keep)
    }
}

/// Synthetic positive observations through the public decision entry point.
/// Used by drain/retention tests; no provider, clock wait or host audio.
#[cfg(test)]
pub(super) async fn finish_silent_meeting_fixture(runtime: &mut Runtime) -> Result<()> {
    use orbit_core::audio_signal::{RecordingSignalView, SignalCondition, TrackSignalView};
    let recording_id = runtime
        .state
        .durable
        .capture
        .recording
        .as_ref()
        .context("recording")?
        .id
        .clone();
    let track = TrackSignalView {
        condition: SignalCondition::LowSignal,
        received_frames: 1_200_000,
        since_last_samples_ms: Some(0),
        low_signal_ms: 150_000,
        speech_quiet_ms: Some(150_000),
        voiced_frames: Some(0),
    };
    let signal = RecordingSignalView {
        recording_id,
        capture_epoch_id: "synthetic-epoch".into(),
        mic_local: track.clone(),
        remote_app: Some(track),
    };
    let at = now();
    for time in [at - orbit_core::silence::RECORDING_ANSWER_MS, at] {
        let event = Event::CheckSilence {
            call: None,
            recording: Some(Box::new(signal.clone())),
            microphone_muted: false,
        };
        runtime
            .execute_decision(decide(&runtime.state, event, time)?)
            .await?;
    }
    Ok(())
}
