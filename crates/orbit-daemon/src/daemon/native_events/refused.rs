//! An order the helper refused, read by the failure taxonomy: a refusal, a
//! degradation, or an integrity failure that cuts the audio at risk.
use super::super::*;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::wire::native::NativeResponseFields;

impl Runtime {
    pub(super) async fn helper_order_refused(
        &mut self,
        response: &NativeResponseFields,
        command_kind: &str,
    ) -> Result<()> {
        // A late refusal of any order sent before the brake may not
        // rewrite the cause and may not ask the helper to stop again:
        // privacy is already latched and the helper already knows.
        let error = response
            .error
            .as_ref()
            .context("response field=error type=absent")?;
        let code = error.code.as_str();
        if code == "privacy_stopped" && self.state.privacy_stopped() {
            self.log("native_rejected_after_privacy_stop", &[]);
            return Ok(());
        }
        let outcome =
            orbit_core::failure::classify_native_response(command_kind, code, &self.state);
        match outcome {
            Outcome::Refused { reason } => {
                self.order_refused_for(response, command_kind, reason).await
            }
            Outcome::Degraded { kind, detail } => {
                // The taxonomy adds its notice through the core;
                // the helper code keeps its own entry without
                // erasing the call/recording context above it.
                self.apply(Event::Degraded(kind)).await?;
                self.say(
                    Notice::new(NoticeCode::MacCommandDegraded)
                        .with_token(NoticeKey::Step, command_kind)
                        .with_token(NoticeKey::Code, &detail),
                );
                Ok(())
            }
            Outcome::IntegrityFailure { detail } => {
                // The audio at risk is cut with the
                // helper's code; nothing latches.
                let message = error.message.as_str();
                self.contain_coded(
                    orbit_core::privacy_stop::StopCause::HelperIntegrityFailure,
                    &detail,
                    &format!(
                        "El Mac no pudo garantizar el audio ({command_kind}: {detail}). {message}"
                    ),
                )
                .await;
                Ok(())
            }
        }
    }

    async fn order_refused_for(
        &mut self,
        response: &NativeResponseFields,
        command_kind: &str,
        reason: String,
    ) -> Result<()> {
        if command_kind == "audio_start" {
            self.audio_start_refused(response, reason).await
        } else if reason == "operation_cancelled_by_stop" {
            // The helper says this only when a stop it was given
            // overtook the order: the stop is what happened, and
            // it says so with its own notice. A cancelled
            // `audio_stop` under a cut is not a refusal.
            self.log(
                "native_order_cancelled_by_stop",
                &[("step", command_kind), ("id", &response.id)],
            );
            Ok(())
        } else {
            self.say(
                Notice::new(NoticeCode::MacCommandRefused)
                    .with_token(NoticeKey::Step, command_kind)
                    .with_token(NoticeKey::Code, &reason),
            );
            Ok(())
        }
    }

    /// An audio_start refused for a permission or scope reason interrupts
    /// the recording without latching; the brake stays reserved for
    /// integrity and intent. The pending start cycle is dropped so the
    /// helper's late ACK (or its absence) cannot advance the phase.
    async fn audio_start_refused(
        &mut self,
        response: &NativeResponseFields,
        reason: String,
    ) -> Result<()> {
        let refused_voice = self.state.durable.voice.voice_active
            && !self
                .state
                .durable
                .capture
                .recording
                .as_ref()
                .is_some_and(|recording| recording.phase.interrupted_on_recovery());
        let empty_voice = self.state.durable.voice.voice_active
            && self
                .capture
                .buffer
                .voice
                .as_ref()
                .is_some_and(|voice| voice.pcm.is_empty());
        self.audio.pending_starts.retain(|id| id != &response.id);
        if !refused_voice && self.capture.interruption.is_none() {
            self.capture.interruption = Some(refused_start_interruption(&reason));
        }
        self.apply(Event::CaptureRefused {
            reason: reason.clone(),
        })
        .await?;
        if empty_voice {
            // No voice transcript exists after a refused start.
            // Flushing an empty voice buffer would replace the
            // helper's permission cause with "No se capturó audio".
            self.discard_refused_voice();
        }
        if self.audio.remote_stream.is_some() {
            // A second track may have started in the helper.
            // Keep its identity and incoming tail frames until
            // the whole-capture drain is acknowledged.
            self.request_full_audio_stop(audio::FullStop::Cut).await?;
        } else if refused_voice && empty_voice {
            self.audio.mic_stream = None;
        } else {
            // A one-track start was refused; no producer can
            // still deliver audio for this recording.
            self.audio.mic_stream = None;
            Box::pin(self.flush_stopped_audio()).await?;
        }
        Ok(())
    }
}

/// Why a refused start interrupted the recording, by the helper's reason.
fn refused_start_interruption(reason: &str) -> orbit_core::RecordingInterruption {
    let cause = match reason {
        "microphone_permission_required" => {
            orbit_core::RecordingInterruptionCause::MicrophonePermissionRequired
        }
        "screen_permission_required" => {
            orbit_core::RecordingInterruptionCause::ScreenPermissionRequired
        }
        _ => orbit_core::RecordingInterruptionCause::CaptureStartRefused,
    };
    orbit_core::RecordingInterruption {
        cause,
        detail: Some(reason.to_owned()),
        audio_available: false,
    }
}
