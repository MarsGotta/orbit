//! The user's stop: the first one latches and cuts what is live before any
//! store write; a stop on top of one only adds its control and retires again.
use super::*;

/// The core event for the user's stop: the helper's latch adopted, or a
/// stop the user gave.
pub(super) fn privacy_event(origin: &UserStop) -> Event {
    match origin {
        UserStop::HelperLatch { .. } => Event::HelperLatched,
        UserStop::Order | UserStop::Hotkey | UserStop::Helper { .. } => Event::PrivacyStop,
    }
}

impl Runtime {
    /// The user's stop, and the only function that latches one.
    /// Every caller names the control they used; nothing automatic can be
    /// written as a [`UserStop`].
    pub(in crate::daemon) async fn privacy_stop(&mut self, origin: UserStop) {
        let plan = retire(AuthorityChange::UserStop);
        // Logged at the single point every one of the user's stops passes through;
        // the cause is Orbit's own name for the control, never content.
        self.log("privacy_stopped", &[("cause", origin.cause().name())]);
        // The retirement is idempotent and runs first so a route added later
        // cannot bypass it; on re-entry it also catches anything reopened
        // after the first call. The flag is captured up front because only the
        // core decision below latches it.
        if self.state.privacy_stopped() {
            self.repeat_privacy_stop(origin, plan).await;
        } else {
            self.first_privacy_stop(origin, plan).await;
        }
    }

    /// The stop that latches: the helper's order and the live audio go
    /// before any store write, and the durable tail runs within its budget.
    pub(super) async fn first_privacy_stop(&mut self, origin: UserStop, plan: RetirePlan) {
        let message = origin.notice();
        let interrupted_request = self.state.durable.assistant.active_request.clone();
        self.note_privacy_interruption();
        let Ok(mut decision) = decide(&self.state, privacy_event(&origin), now()) else {
            return;
        };
        decision.state.set_notice(self.scoped(message));
        // The latch and the helper order precede every store write in
        // retirement and halt. A stuck writer must not keep the Mac's
        // microphone open or let a late frame restart this generation.
        self.state = decision.state.clone();
        if let Err(error) = self
            .execute_retire_plan(
                plan,
                AuthorityChange::UserStop,
                RetirePhase::ReadEpoch,
                None,
            )
            .await
        {
            self.log("retire_read_epoch_failed", &[("error", &error.to_string())]);
        }
        self.stop_live_audio_for_privacy(interrupted_request.as_deref())
            .await;
        // These core effects were performed above. Retirement below owns
        // CancelNetwork's volatile half before its first durable await.
        decision.effects.retain(|effect| {
            !matches!(
                effect,
                Effect::StopCapture
                    | Effect::StopPlayback
                    | Effect::StopCall
                    | Effect::Interrupt
                    | Effect::CancelNetwork
            )
        });
        self.retire_authority_volatile(plan, AuthorityChange::UserStop)
            .await;
        if let Err(error) = self
            .execute_retire_plan(
                plan,
                AuthorityChange::UserStop,
                RetirePhase::Link,
                interrupted_request.as_deref(),
            )
            .await
        {
            self.log("retire_link_failed", &[("error", &error.to_string())]);
        }
        if let Err(error) = self.store_actor.defer_snapshot(&self.state) {
            self.say_step_failure("stop_snapshot", &error);
        }
        self.privacy_durable_tail(plan, decision).await;
    }

    /// Why the recording is interrupted, unless something already said so.
    pub(super) fn note_privacy_interruption(&mut self) {
        if self.capture.interruption.is_some() {
            return;
        }
        let earlier = self
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .is_some_and(|recording| {
                recording.phase == CapturePhase::Interrupted && recording.ended_at_ms.is_none()
            });
        self.capture.interruption = Some(if earlier {
            unsaved_interruption_cause()
        } else {
            orbit_core::RecordingInterruption {
                cause: orbit_core::RecordingInterruptionCause::PrivacyStop,
                detail: None,
                audio_available: false,
            }
        });
    }

    /// The microphone, the turn in flight, the playback and the call's live
    /// voice link stop before a store receipt can wait on a blocked writer.
    pub(super) async fn stop_live_audio_for_privacy(&mut self, interrupted_request: Option<&str>) {
        if let Err(error) = self.stop_recording_audio().await {
            self.log("stop_effect_failed", &[("error", &error.to_string())]);
            self.say_step_failure("audio_stop", &error);
        }
        self.interrupt_turn(interrupted_request, true).await;
        for effect in [Effect::StopPlayback, Effect::StopCall] {
            let result = if matches!(effect, Effect::StopCall) {
                self.stop_call_audio(CallStopVoice::Cancel(VoiceCancellationCause::Authority(
                    AuthorityChange::UserStop,
                )))
                .await
            } else {
                self.effect(effect).await
            };
            if let Err(error) = result {
                self.say_step_failure("playback_or_call_stop", &error);
            }
        }
    }

    /// The existing ten-second closure budget covers two five-second seal
    /// retries under a failing store. Here it bounds only the later durable
    /// work; it is not a microphone deadline or proof that the snapshot
    /// landed. The helper was ordered before.
    pub(super) async fn privacy_durable_tail(
        &mut self,
        plan: RetirePlan,
        mut decision: orbit_core::Decision,
    ) {
        let durable_tail = async {
            Box::pin(self.retire_authority_durable(plan, AuthorityChange::UserStop)).await;
            decision.state = self.state.clone();
            Box::pin(self.halt(
                decision,
                CallStopVoice::Cancel(VoiceCancellationCause::Authority(AuthorityChange::UserStop)),
            ))
            .await;
        };
        if tokio::time::timeout(shutdown::SHUTDOWN_BUDGET, durable_tail)
            .await
            .is_err()
        {
            self.privacy_retirement_still_owed();
        } else {
            self.privacy_retirement_owed = false;
        }
    }

    /// The durable retirement did not finish in its budget: it stays owed,
    /// its counts are logged and the snapshot is deferred.
    pub(super) fn privacy_retirement_still_owed(&mut self) {
        self.privacy_retirement_owed = true;
        self.report_pending_stop_work();
        if let Err(error) = self.store_actor.defer_snapshot(&self.state) {
            self.log(
                "stop_snapshot_defer_failed",
                &[("error", &error.to_string())],
            );
        }
    }

    /// Re-entry: the helper already heard the user's stop. Asking again
    /// would be a duplicate; the new control is appended so the first stays
    /// visible, and the snapshot is rewritten only if the retirement changed
    /// it.
    pub(super) async fn repeat_privacy_stop(&mut self, origin: UserStop, plan: RetirePlan) {
        let message = origin.notice();
        let before_closure = self.state.clone();
        if self.audio.full_stop_send_owed()
            && let Err(error) = self.retry_full_stop_send().await
        {
            self.say_step_failure("helper_stop_send", &error);
        }
        // The attribution is the control the user pressed, independent of whether
        // the writer subsequently responds within the retirement budget.
        match decide(&self.state, privacy_event(&origin), now()) {
            Ok(decision) => self.state = decision.state,
            Err(error) => {
                self.log(
                    "privacy_stop_attribution_failed",
                    &[("error", &error.to_string())],
                );
            }
        }
        self.retire_authority_volatile(plan, AuthorityChange::UserStop)
            .await;
        if let Err(error) = self
            .execute_retire_plan(plan, AuthorityChange::UserStop, RetirePhase::Link, None)
            .await
        {
            self.log("retire_link_failed", &[("error", &error.to_string())]);
        }
        if tokio::time::timeout(
            shutdown::SHUTDOWN_BUDGET,
            self.retire_authority_durable(plan, AuthorityChange::UserStop),
        )
        .await
        .is_err()
        {
            self.privacy_retirement_still_owed();
            return;
        }
        self.privacy_retirement_owed = false;
        // Pressed on top of a stop the user did not ask for (a revocation),
        // it becomes theirs, so it survives the next restart as theirs. A latch
        // found on disk makes a revocation's stop one nobody can attribute,
        // which is kept too.
        if self.state != before_closure
            && let Err(error) = self.store_actor.snapshot(&self.state).await
        {
            self.log("stop_snapshot_failed", &[("error", &error.to_string())]);
        }
        self.say(message);
    }
}
