//! The call's health: a call with no audio, or past its authorization, stops,
//! and the log says why with the numbers that tell the causes apart.
use super::*;

/// The cause token a stop is said with: one word per cause.
pub(super) fn stop_cause(reason: StopReason) -> &'static str {
    match reason {
        StopReason::Unauthorized => "unauthorized",
        StopReason::GenerationChanged => "generation_changed",
        StopReason::ProcessingLost => "processing_lost",
        StopReason::Discontinuity => "discontinuity",
        StopReason::Expired => "expired",
        StopReason::Idle => "idle",
        StopReason::NoAudio => "no_audio",
    }
}

impl Runtime {
    pub(in crate::daemon) async fn check_call_health(&mut self) -> Result<()> {
        let Some(call) = self
            .audio
            .call
            .as_ref()
            .filter(|call| call.closing_at_ms.is_none())
        else {
            return Ok(());
        };
        let authorized = Duration::from_millis(
            call.authorization
                .expires_at_ms
                .saturating_sub(call.authorization.started_at_ms) as u64,
        );
        // A frame still in the loop's queue was sent; absence is judged only
        // once nothing is waiting.
        let no_audio =
            call.last_audio.elapsed() > CALL_AUDIO_SILENCE_LIMIT && call.queued_native_events == 0;
        let reason = if no_audio {
            StopReason::NoAudio
        } else if call.started.elapsed() >= authorized {
            StopReason::Expired
        } else {
            return Ok(());
        };
        if no_audio {
            self.log_call_idle_stop(reason, now());
        }
        self.stop_call_for_reason(reason).await
    }

    /// Why a call stopped for lack of audio or of conversation, with the
    /// numbers that tell one from the other and a held loop from both.
    pub(super) fn log_call_idle_stop(&self, reason: StopReason, now_ms: i64) {
        let Some(call) = self.audio.call.as_ref() else {
            return;
        };
        let deimos_speaking = call.playback_active || self.assistant.speech_task.active().is_some();
        let limit_ms = if reason == StopReason::NoAudio {
            CALL_AUDIO_SILENCE_LIMIT.as_millis().to_string()
        } else {
            VoicePolicy::default().idle_timeout_ms.to_string()
        };
        self.log(
            "call_stopped_idle",
            &[
                ("cause", stop_cause(reason)),
                (
                    "last_frame_age_ms",
                    &call.last_audio.elapsed().as_millis().to_string(),
                ),
                (
                    "user_silent_ms",
                    &now_ms
                        .saturating_sub(call.detector.last_activity_ms())
                        .to_string(),
                ),
                ("limit_ms", &limit_ms),
                (
                    "queued_native_events",
                    &call.queued_native_events.to_string(),
                ),
                (
                    "deimos_speaking",
                    if deimos_speaking { "true" } else { "false" },
                ),
                ("last_tick_ms", &call.last_tick.as_millis().to_string()),
                ("frames", &call.frames_heard.to_string()),
            ],
        );
    }
}
