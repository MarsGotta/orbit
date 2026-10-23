//! The cuts: what a cause that is not the person's stop does to what is live.
use super::*;

impl Runtime {
    /// Cuts the live capture with its cause. The helper is asked
    /// to stop capturing without its latch; the recording stays interrupted
    /// with its fragments until `cut_recovery` resumes or closes it. With
    /// nothing live, only the cause is said.
    pub(in crate::daemon) async fn cut_capture(
        &mut self,
        cause: StopCause,
        code: &str,
        text: &str,
    ) {
        let voice_cause = if matches!(
            cause,
            StopCause::HelperCaptureStop
                | StopCause::HelperIntegrityFailure
                | StopCause::HelperLost
                | StopCause::CaptureEventFailed
        ) {
            VoiceCancellationCause::HelperCut
        } else {
            VoiceCancellationCause::CaptureCut
        };
        if self.state.privacy_stopped() || !self.capture_live() {
            self.say_durably(contained(cause, code)).await;
            return;
        }
        if let Some(typed) = recording_interruption_cause(cause)
            && self.capture.interruption.is_none()
        {
            self.capture.interruption = Some(orbit_core::RecordingInterruption {
                cause: typed,
                detail: Some(text.to_owned()),
                audio_available: false,
            });
        }
        let Ok(decision) = decide(
            &self.state,
            Event::CaptureCut {
                cause: code.to_owned(),
            },
            now(),
        ) else {
            self.say_durably(contained(cause, code)).await;
            return;
        };
        self.note_cut(cause, code);
        self.log("capture_cut", &[("reason", text)]);
        Box::pin(self.halt(decision, CallStopVoice::Cancel(voice_cause))).await;
    }

    /// Orbit is closing: the same retirement the user's stop performs,
    /// and the capture cut with its cause, but nothing latches, so the next
    /// start comes back working. Only the closing door calls this.
    pub(in crate::daemon) async fn close_for_exit(&mut self, cause: StopCause, text: &str) {
        self.closing = true;
        self.log(
            "closing",
            &[
                ("cause", cause.name()),
                ("class", cause.class().name()),
                ("reason", text),
            ],
        );
        if self.state.privacy_stopped() {
            if self.audio.full_stop_send_owed()
                && let Err(error) = self.retry_full_stop_send().await
            {
                self.say_step_failure("helper_stop_send", &error);
            }
            Box::pin(self.retire_authority(AuthorityChange::Closing, None)).await;
            // The user's stop is already in force and already stopped everything;
            // it stays theirs across the restart that follows.
            self.say(
                Notice::new(NoticeCode::OrbitClosing).with_token(NoticeKey::Cause, cause.name()),
            );
            if let Err(error) = self.store_actor.snapshot(&self.state).await {
                self.log("close_snapshot_failed", &[("error", &error.to_string())]);
            }
            return;
        }
        let Ok(mut decision) = decide(
            &self.state,
            Event::Close {
                cause: cause.name().to_owned(),
            },
            now(),
        ) else {
            self.say_durably(
                Notice::new(NoticeCode::OrbitClosing).with_token(NoticeKey::Cause, cause.name()),
            )
            .await;
            return;
        };
        self.state = decision.state.clone();
        if let Err(error) = self
            .request_full_audio_stop(super::audio::FullStop::Cut)
            .await
        {
            self.log("stop_effect_failed", &[("error", &error.to_string())]);
            self.say_step_failure("audio_stop_send", &error);
        }
        if let Err(error) = self.store_actor.defer_snapshot(&self.state) {
            self.say_step_failure("close_snapshot", &error);
        }
        Box::pin(self.retire_authority(AuthorityChange::Closing, None)).await;
        decision.state = self.state.clone();
        Box::pin(self.halt(
            decision,
            CallStopVoice::Cancel(VoiceCancellationCause::Authority(AuthorityChange::Closing)),
        ))
        .await;
    }

    /// The helper stopped its capture on its own, or announced a stop with a
    /// code that is not one of the user's controls: the live capture is cut and the
    /// code says why. The echo of a stop the daemon itself asked for is not a
    /// new cause.
    pub(in crate::daemon) async fn helper_capture_stop(&mut self, code: &str) {
        if orbit_core::privacy_stop::DAEMON_ECHO_CODES.contains(&code) {
            self.log("helper_stop_echo", &[("reason", code)]);
            return;
        }
        if !self.capture_live() {
            // With nothing recording nothing was cut: only recorded.
            self.log("helper_capture_stop_idle", &[("reason", code)]);
            self.say_durably(
                Notice::new(NoticeCode::MacStoppedIdleCapture).with_token(NoticeKey::Code, code),
            )
            .await;
            return;
        }
        self.contain_coded(
            StopCause::HelperCaptureStop,
            code,
            &format!("El Mac cortó la captura ({code})."),
        )
        .await;
    }
}
