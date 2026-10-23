//! The only stop that lasts is the one the user pressed.
//!
//! Previously every failure the actor loop saw, a sleeping Mac, a lost
//! helper, an audio route change or an unverifiable inventory ran the same
//! privacy stop the user's button runs, and every restart invented one more. Now
//! there are three doors and each says what it is:
//!
//! - [`Runtime::privacy_stop`] is the user's stop. It latches, it survives a
//!   restart, and only the user's controls can reach it: its argument is a
//!   [`UserStop`], which has no variant for an automatic cause.
//! - [`Runtime::cut_capture`] cuts the live recording, call or voice note
//!   with its cause and what was kept. Orbit keeps working, nothing latches
//!   and the helper's durable latch is not touched. A recording cut by
//!   anything but the person resumes once the capture is back, or is closed
//!   (`cut_recovery`).
//! - [`Runtime::close_for_exit`] is Orbit closing: the capture is cut, the
//!   network and every vertical are retired, and the next start comes back
//!   working.
//!
//! Every former automatic cause is routed by [`Runtime::contain`] through the
//! containment table in `orbit_core::privacy_stop`, never by hand.
use super::audio::{CallStopVoice, VoiceCancellationCause};
use super::retire::RetirePhase;
use super::*;
use orbit_core::authority::{AuthorityChange, RetirePlan, retire};
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_core::{
    failure::is_capture_command,
    privacy_stop::{StopCause, StopClass, UserStop},
    wire::native::{NativeEventHeader, NativeResponseIdRef},
};

mod authority;
mod cuts;
mod events;
mod maintenance;

/// Orders whose answers are the capture's own: the capture commands of the
/// failure taxonomy and the user's stop.
fn capture_command(command: &str) -> bool {
    is_capture_command(command) || command == "privacy_stop"
}

fn recording_interruption_cause(
    cause: StopCause,
) -> Option<orbit_core::RecordingInterruptionCause> {
    use orbit_core::RecordingInterruptionCause as Interruption;
    match cause {
        StopCause::HelperCaptureStop => Some(Interruption::HelperCaptureStop),
        StopCause::HelperIntegrityFailure => Some(Interruption::HelperIntegrityFailure),
        StopCause::HelperLost => Some(Interruption::HelperLost),
        StopCause::CaptureEventFailed => Some(Interruption::CaptureEventFailed),
        StopCause::CaptureMaintenanceFailed => Some(Interruption::CaptureMaintenanceFailed),
        StopCause::AudioStopUnconfirmed => Some(Interruption::AudioStopUnconfirmed),
        StopCause::CaptureStopUndelivered => Some(Interruption::CaptureStopUndelivered),
        StopCause::Shutdown => Some(Interruption::Shutdown),
        StopCause::ListenerStopped => Some(Interruption::ListenerStopped),
        StopCause::Order
        | StopCause::Hotkey
        | StopCause::HelperUserStop
        | StopCause::HelperLatch
        | StopCause::HelperRefusedStopped
        | StopCause::NativeEventFailed
        | StopCause::VerticalMaintenanceFailed
        | StopCause::PrivacyStopUndelivered
        | StopCause::AudioInventoryUnverifiable => None,
    }
}

fn unsaved_interruption_cause() -> orbit_core::RecordingInterruption {
    orbit_core::RecordingInterruption {
        cause: orbit_core::RecordingInterruptionCause::CauseNotSaved,
        detail: None,
        audio_available: false,
    }
}

/// Said when the hub task ended without an unsent-frames error, which is the
/// only one that carries a count.
const HUB_FRAMES_UNCOUNTED: &str = "recuento de tramas no disponible";

// A retry occupies at most one quarter of the actor's one-second tick. It
// protects IPC and the other verticals from a writer held in fsync; it is not
// a durability deadline, since the owed flag remains set for another tick.
// Ten held-writer ticks returned within a 1 s guard at load 12.91–45.26;
// the worst complete fixture took 39.42 s setting up SQLite under that load.
// The old ten-second retry failed the same one-second guard in the mutation
// copy. This slice is not a bound on a whole maintenance turn or disk fsync.
const RETIREMENT_RETRY_SLICE: std::time::Duration = std::time::Duration::from_millis(250);

// The stop, the cut and the closing await large futures (every vertical's
// retirement, every core effect). They are boxed where they are awaited so the
// callers that embed them — `command`, the loop's branches — keep a stack
// frame small enough for the test threads' default stack.
impl Runtime {
    /// Every cause that used to stop Orbit on its own lands here, and does
    /// what the containment table says: cut the live capture or say the cause.
    /// It never latches; a cause of the user's class reaching this door is a misroute, is
    /// logged as one and only said.
    pub(super) async fn contain(&mut self, cause: StopCause, text: &str) {
        Box::pin(self.contain_coded(cause, cause.name(), text)).await;
    }

    /// [`Self::contain`] with the code a cut names: the helper's own code
    /// when it gave one, the cause's name otherwise. `text` stays in
    /// `daemon.log`.
    pub(super) async fn contain_coded(&mut self, cause: StopCause, code: &str, text: &str) {
        let class = cause.class();
        self.log(
            "stop_contained",
            &[
                ("cause", cause.name()),
                ("class", class.name()),
                ("reason", text),
            ],
        );
        match class {
            StopClass::CutCapture if !self.state.privacy_stopped() && self.capture_live() => {
                Box::pin(self.cut_capture(cause, code, text)).await;
            }
            StopClass::UserStop => {
                self.log("stop_misrouted", &[("cause", cause.name())]);
                self.say_durably(contained(cause, code)).await;
            }
            StopClass::CutCapture => {
                self.say_durably(contained(cause, code)).await;
                if !self.capture_live() {
                    let earlier = self
                        .capture
                        .interruption
                        .clone()
                        .unwrap_or_else(unsaved_interruption_cause);
                    if let Err(error) = Box::pin(self.close_empty_interrupted(earlier)).await {
                        self.log(
                            "earlier_interruption_close_failed",
                            &[("error", &error.to_string())],
                        );
                    }
                }
            }
            StopClass::Notice => self.say_durably(contained(cause, code)).await,
        }
    }

    /// Something can still be capturing: the state says so, or the helper
    /// still holds a stream or a start the daemon asked for.
    pub(super) fn capture_live(&self) -> bool {
        orbit_core::privacy_stop::capture_live(&self.state)
            || self.capture.mic_stream.is_some()
            || self.capture.remote_stream.is_some()
            || !self.capture.pending_starts.is_empty()
    }

    /// A cause said once: a maintenance that fails every second says it once,
    /// instead of pushing everything else out of the notice log. Returns
    /// whether anything new was said.
    pub(super) fn say_once(&mut self, notice: Notice) -> bool {
        if self
            .state
            .durable
            .notice
            .as_ref()
            .is_some_and(|said| said.code == notice.code && said.data == notice.data)
        {
            return false;
        }
        self.say(notice);
        true
    }

    /// Says a contained cause and keeps it in the snapshot, so the next
    /// process still knows why, as the old stop did. Best effort: when the disk
    /// is the thing failing, the cause stays in memory and on screen.
    async fn say_durably(&mut self, notice: Notice) {
        if self.say_once(notice)
            && let Err(error) = self.store_actor.snapshot(&self.state).await
        {
            self.log("durable_notice_failed", &[("error", &error.to_string())]);
        }
    }

    /// The user's stop, and the only function that latches one.
    /// Every caller names the control they used; nothing automatic can be
    /// written as a [`UserStop`].
    pub(super) async fn privacy_stop(&mut self, origin: UserStop) {
        let message = origin.notice();
        let plan = retire(AuthorityChange::UserStop);
        // Logged at the single point every one of the user's stops passes through;
        // the cause is Orbit's own name for the control, never content.
        self.log("privacy_stopped", &[("cause", origin.cause().name())]);
        // The retirement is idempotent and runs first so a route added later
        // cannot bypass it; on re-entry it also catches anything reopened
        // after the first call. The flag is captured up front because only the
        // core decision below latches it.
        let privacy_already_stopped = self.state.privacy_stopped();
        let before_closure = self.state.clone();
        let interrupted_request = self.state.durable.assistant.active_request.clone();
        if !privacy_already_stopped {
            if self.capture.interruption.is_none() {
                let earlier =
                    self.state
                        .durable
                        .capture
                        .recording
                        .as_ref()
                        .is_some_and(|recording| {
                            recording.phase == CapturePhase::Interrupted
                                && recording.ended_at_ms.is_none()
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
            let event = match origin {
                UserStop::HelperLatch { .. } => Event::HelperLatched,
                UserStop::Order | UserStop::Hotkey | UserStop::Helper { .. } => Event::PrivacyStop,
            };
            let Ok(mut decision) = decide(&self.state, event, now()) else {
                return;
            };
            decision.state.set_notice(self.scoped(message.clone()));
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
            if let Err(error) = self.stop_recording_audio().await {
                self.log("stop_effect_failed", &[("error", &error.to_string())]);
                self.say_step_failure("audio_stop", &error);
            }
            self.interrupt_turn(interrupted_request.as_deref(), true)
                .await;
            // Native playback and the call's live voice link are retired
            // before a store receipt can wait on a blocked writer.
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
            // The existing ten-second closure budget covers two five-second
            // seal retries under a failing store. Here it bounds only the
            // later durable work; it is not a microphone deadline or proof
            // that the snapshot landed. The helper was ordered above.
            let durable_tail = async {
                Box::pin(self.retire_authority_durable(plan, AuthorityChange::UserStop)).await;
                decision.state = self.state.clone();
                Box::pin(self.halt(
                    decision,
                    CallStopVoice::Cancel(VoiceCancellationCause::Authority(
                        AuthorityChange::UserStop,
                    )),
                ))
                .await;
            };
            if tokio::time::timeout(shutdown::SHUTDOWN_BUDGET, durable_tail)
                .await
                .is_err()
            {
                self.privacy_retirement_owed = true;
                self.report_pending_stop_work();
                if let Err(error) = self.store_actor.defer_snapshot(&self.state) {
                    self.log(
                        "stop_snapshot_defer_failed",
                        &[("error", &error.to_string())],
                    );
                }
            } else {
                self.privacy_retirement_owed = false;
            }
            return;
        }
        if self.full_stop_send_owed()
            && let Err(error) = self.retry_full_stop_send().await
        {
            self.say_step_failure("helper_stop_send", &error);
        }
        // The attribution is the control the user pressed, independent of whether
        // the writer subsequently responds within the retirement budget.
        let event = match origin {
            UserStop::HelperLatch { .. } => Event::HelperLatched,
            UserStop::Order | UserStop::Hotkey | UserStop::Helper { .. } => Event::PrivacyStop,
        };
        match decide(&self.state, event, now()) {
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
            self.privacy_retirement_owed = true;
            self.report_pending_stop_work();
            if let Err(error) = self.store_actor.defer_snapshot(&self.state) {
                self.log(
                    "stop_snapshot_defer_failed",
                    &[("error", &error.to_string())],
                );
            }
            return;
        }
        self.privacy_retirement_owed = false;
        // Re-entry: the helper already heard the user's stop. Asking again would be a
        // duplicate; the new control is appended so the first stays visible,
        // and the snapshot is rewritten only if the retirement changed it.
        if privacy_already_stopped {
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

    /// One step of a stop or a close failed; its error stays in
    /// `daemon.log` under the turn's op.
    fn say_step_failure(&mut self, step: &str, error: &dyn std::fmt::Display) {
        self.say_failure(
            Notice::new(NoticeCode::StopStepFailed).with_token(NoticeKey::Step, step),
            error,
        );
    }

    /// A stop still owes durable work. The counts always go to `daemon.log`;
    /// only closing asks the user to keep Orbit open.
    fn report_pending_stop_work(&mut self) {
        let pending = self.pending_stop_counts();
        self.log(
            "stop_durable_pending",
            &[
                ("budget_s", &shutdown::SHUTDOWN_BUDGET.as_secs().to_string()),
                ("pending", &pending),
            ],
        );
        if self.closing {
            self.say(NoticeCode::PendingWorkKeepOpen);
        }
    }

    /// Adopts a stopping decision and carries it out, whatever the disk says.
    ///
    /// The state is taken in memory before any persistence: a failed snapshot
    /// write must not leave a Mac that has not stopped, only one whose stop is
    /// not yet durable. The core's effects then run in order;
    /// one that fails is recorded and the drain is abandoned so the audio
    /// that arrived is kept locally.
    async fn halt(&mut self, decision: orbit_core::Decision, voice: CallStopVoice) {
        self.state = decision.state.clone();
        let interrupted_request = self.state.durable.assistant.active_request.clone();
        let persisted_before_effects = self.state.clone();
        let snapshot_failed = if let Err(error) = self.store_actor.snapshot(&self.state).await {
            self.log("stop_snapshot_failed", &[("error", &error.to_string())]);
            self.say_step_failure("stop_snapshot", &error);
            true
        } else {
            false
        };
        for effect in decision.effects {
            if matches!(effect, Effect::Interrupt) {
                self.interrupt_turn(interrupted_request.as_deref(), false)
                    .await;
            } else if let Err(error) = if matches!(effect, Effect::StopCall) {
                self.stop_call_audio(voice).await
            } else {
                self.effect(effect).await
            } {
                // The effect's own error says what failed (an order the Mac
                // never received, a closed channel); the cause is already
                // said above it, so it is not repeated.
                self.log("stop_effect_failed", &[("error", &error.to_string())]);
                self.say_step_failure("stop_effect", &error);
                // A timed-out whole-helper stop never entered its full
                // channel. Keep its exact ID and drain ownership so the next
                // maintenance turn can resend it; abandoning here was what
                // left the microphone open indefinitely after a brief stall.
                if !self.full_stop_send_owed()
                    && let Err(abandon_error) = self.abandon_audio_drain().await
                {
                    self.log(
                        "audio_drain_abandon_failed",
                        &[("error", &abandon_error.to_string())],
                    );
                }
            }
        }
        if !self.has_any_pending().is_empty() {
            let summary = self.has_any_pending().join(", ");
            self.log("pending_in_memory", &[("summary", &summary)]);
            if self.closing {
                self.say(
                    Notice::new(NoticeCode::PendingWorkKeepOpen)
                        .with(NoticeKey::Count, self.has_any_pending().len()),
                );
            }
        }
        let final_snapshot_failed = if self.state != persisted_before_effects {
            if let Err(error) = self.store_actor.snapshot(&self.state).await {
                self.say_step_failure("stop_snapshot", &error);
                true
            } else {
                false
            }
        } else {
            snapshot_failed
        };
        if final_snapshot_failed {
            self.say(
                Notice::new(NoticeCode::StopStepFailed)
                    .with_token(NoticeKey::Step, "final_snapshot"),
            );
        }
    }
}

/// What a contained cause says when nothing live was cut: a failure the
/// person cannot fix, with the cause's name.
/// The helper's own code, when it gave one, travels with the cause's name.
fn contained(cause: StopCause, code: &str) -> Notice {
    let notice =
        Notice::new(NoticeCode::ContainedFailure).with_token(NoticeKey::Cause, cause.name());
    if code == cause.name() {
        notice
    } else {
        notice.with_token(NoticeKey::Code, code)
    }
}
