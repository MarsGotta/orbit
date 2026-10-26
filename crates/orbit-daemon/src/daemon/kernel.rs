//! The kernel routes a decision: it adopts the core's state around the
//! retirements an authority change asks for, stores it, and hands each
//! effect to the domain that executes it.
use super::audio::{CallStopVoice, FullStop, VoiceCancellationCause};
use super::*;
use orbit_core::authority::{AuthorityChange, retire};

mod adopt;
mod persist;

pub(super) use persist::{DurableSnapshot, Persisted};

impl Runtime {
    pub(super) async fn apply(&mut self, event: Event) -> Result<()> {
        let at_ms = now();
        let voice = call_stop_voice_for(&event);
        self.apply_with_call_stop_voice_at(event, voice, at_ms)
            .await
    }

    pub(super) async fn apply_with_call_stop_voice(
        &mut self,
        event: Event,
        voice: CallStopVoice,
    ) -> Result<()> {
        self.apply_with_call_stop_voice_at(event, voice, now())
            .await
    }

    async fn apply_with_call_stop_voice_at(
        &mut self,
        event: Event,
        voice: CallStopVoice,
        at_ms: i64,
    ) -> Result<()> {
        let force_snapshot = matches!(event, Event::Completed { .. });
        let local_rearm = matches!(event, Event::LocalRearm);
        self.validate_audio_transition(&event)?;
        let decision = decide(&self.state, event, at_ms)?;
        let voice = call_stop_voice_after(&self.state, &decision, voice);
        self.persisted.force_once = force_snapshot;
        let result = self
            .execute_decision_with_call_stop_voice(decision, voice)
            .await;
        if local_rearm && !self.state.privacy_stopped() {
            self.privacy_retirement_owed = false;
        }
        result?;
        Ok(())
    }

    pub(super) fn execute_decision(
        &mut self,
        decision: orbit_core::Decision,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + '_>> {
        self.execute_decision_with_call_stop_voice(
            decision,
            CallStopVoice::Cancel(VoiceCancellationCause::Authority(
                AuthorityChange::Interrupt,
            )),
        )
    }

    fn execute_decision_with_call_stop_voice(
        &mut self,
        decision: orbit_core::Decision,
        voice: CallStopVoice,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + '_>> {
        // Before anything of it takes effect: the capture task stops
        // serving frames alone when this changes what that depends on.
        self.disarm_capture_for(&decision.state);
        Box::pin(self.execute_boxed_decision(Box::new(decision), voice))
    }

    async fn execute_boxed_decision(
        &mut self,
        decision: Box<orbit_core::Decision>,
        voice: CallStopVoice,
    ) -> Result<()> {
        let mut force_snapshot = std::mem::take(&mut self.persisted.force_once);
        let change = decision.authority_change;
        let plan = change.map(retire);
        // An in-place closure (FinishMeeting on Interrupted or under the
        // privacy brake) leaves the recording identity untouched, so
        // `preserve_replaced_recording` cannot author its durable closure.
        // Detect that transition before the snapshot overwrites the prior
        // state and mirror the path the existing CaptureFinalized drain
        // already takes: enqueue the closure so `finalize-<id>` exists.
        let previous_was_closed = persist::closed(&self.state);
        self.validate_recording_start(&decision)?;
        let closure = self
            .preserve_replaced_recording(&decision.state)?
            .or(self.empty_interrupted_closure(&decision.state)?);
        let closure_committed = closure.is_some();
        let interrupted_request = self.state.durable.assistant.active_request.clone();
        let orbit_core::Decision {
            mut state, effects, ..
        } = *decision;
        if let Some((change, plan)) = change.zip(plan) {
            self.retire_before_decision(&mut state, change, plan)
                .await?;
        }
        // A failed decision snapshot can restore this point: the source
        // receipts have happened, but the new turn has not begun.
        let previous_state = self.state.clone();
        self.state = state;
        // A turn no longer in flight has ended, however it did: interrupted,
        // lost with the hub's link or stopped. Its work says when.
        if let Some(turn) = interrupted_request.as_deref()
            && self.state.durable.assistant.active_request.as_deref() != Some(turn)
        {
            self.end_work(turn);
            if change == Some(AuthorityChange::Conversation) {
                self.assistant.work = None;
                self.assistant.retry = None;
                if self.assistant.detached_requests.len() == 128 {
                    self.assistant.detached_requests.pop_front();
                }
                self.assistant.detached_requests.push_back(turn.to_owned());
            }
        }
        self.retire_before_snapshot(change, plan).await?;
        force_snapshot |= std::mem::take(&mut self.persisted.force_once);
        self.persist_decision(closure, force_snapshot, change, previous_state)
            .await?;
        if !previous_was_closed && persist::closed(&self.state) && !closure_committed {
            self.queue_finalize().await?;
        }
        for effect in effects {
            self.route_effect(effect, interrupted_request.as_deref(), voice)
                .await?;
        }
        if let Some((change, plan)) = change.zip(plan) {
            self.retire_after_effects(change, plan).await?;
        }
        Ok(())
    }

    /// One effect of a decision in its turn: the interrupt goes for the
    /// request that was in flight before it, and the call's stop carries
    /// the voice this decision chose.
    async fn route_effect(
        &mut self,
        effect: Effect,
        interrupted_request: Option<&str>,
        voice: CallStopVoice,
    ) -> Result<()> {
        match effect {
            Effect::AskCallPresence => {
                self.ask_call_presence();
                Ok(())
            }
            Effect::Interrupt => {
                self.interrupt_turn(interrupted_request, false).await;
                Ok(())
            }
            Effect::StopCall => self.stop_call_audio(voice).await,
            effect @ (Effect::SendText { .. }
            | Effect::StopPlayback
            | Effect::StartCapture { .. }
            | Effect::StopCapture
            | Effect::CutCapture
            | Effect::StartCall
            | Effect::StartVoice
            | Effect::FinishVoice
            | Effect::CancelVoice
            | Effect::CancelNetwork) => self.effect(effect).await,
        }
    }

    /// Executes one effect in the domain that owns it: the conversation's
    /// in `assistant` and `speech`, the microphone's and the screen's in
    /// `audio` and `call`.
    pub(super) async fn effect(&mut self, effect: Effect) -> Result<()> {
        match effect {
            Effect::AskCallPresence => {
                self.ask_call_presence();
                Ok(())
            }
            Effect::SendText {
                id,
                text,
                voice,
                meeting,
                attachments,
            } => {
                self.send_turn_text(id, text, voice, meeting, attachments)
                    .await
            }
            Effect::Interrupt => {
                let turn = self.state.durable.assistant.active_request.clone();
                self.interrupt_turn(turn.as_deref(), false).await;
                Ok(())
            }
            Effect::StopPlayback => self.stop_playback().await,
            Effect::StartCapture {
                recording_id,
                audio,
            } => self.start_capture(recording_id, audio).await,
            Effect::StopCapture => self.stop_recording_audio().await,
            Effect::CutCapture => self.request_full_audio_stop(FullStop::Cut).await,
            Effect::StartCall => self.start_call_audio().await,
            Effect::StopCall => {
                self.stop_call_audio(CallStopVoice::Cancel(VoiceCancellationCause::Authority(
                    AuthorityChange::Interrupt,
                )))
                .await
            }
            Effect::StartVoice => self.start_voice_audio().await,
            Effect::FinishVoice => self.finish_voice_audio().await,
            Effect::CancelVoice => self.cancel_voice_audio().await,
            Effect::CancelNetwork => Ok(()),
        }
    }
}

/// A link change keeps the transcription; a cut names itself; an explicit
/// hangup or unanswered question cancels it through the same interruption.
fn call_stop_voice_for(event: &Event) -> CallStopVoice {
    if matches!(
        event,
        Event::Connect(Connection::Offline | Connection::Connecting | Connection::Online)
            | Event::HubDisconnected
    ) {
        CallStopVoice::KeepTranscription
    } else if matches!(event, Event::CaptureCut { .. }) {
        CallStopVoice::Cancel(VoiceCancellationCause::CaptureCut)
    } else {
        CallStopVoice::Cancel(VoiceCancellationCause::Authority(
            AuthorityChange::Interrupt,
        ))
    }
}

/// The voice once the decision is known: the user's stop, closing and a
/// revocation name themselves, and a lost link that stops a live call names
/// the link.
fn call_stop_voice_after(
    state: &State,
    decision: &orbit_core::Decision,
    voice: CallStopVoice,
) -> CallStopVoice {
    match decision.authority_change {
        Some(
            change @ (AuthorityChange::UserStop
            | AuthorityChange::Closing
            | AuthorityChange::Revoked),
        ) => CallStopVoice::Cancel(VoiceCancellationCause::Authority(change)),
        Some(
            change @ (AuthorityChange::HubLink
            | AuthorityChange::HubLinkOffline
            | AuthorityChange::HubDisconnected),
        ) if state.durable.voice.call.is_some()
            && decision
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::StopCall)) =>
        {
            CallStopVoice::Cancel(VoiceCancellationCause::Authority(change))
        }
        _ => voice,
    }
}
