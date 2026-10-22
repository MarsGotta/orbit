//! Daemon decision execution and durable state.
use super::audio::{CallStopVoice, VoiceCancellationCause};
use super::retire::RetirePhase;
use super::*;
use orbit_core::authority::{AuthorityChange, retire, retire_stopped_decision};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct DurableSnapshot {
    recording: Option<orbit_core::Recording>,
    privacy_stopped: bool,
    privacy_stop_by: Option<orbit_core::privacy_stop::StopRequester>,
    generation: u64,
    connection: Connection,
    session_id: Option<String>,
    agent_id: Option<String>,
    pending_approval: Option<Approval>,
    call: Option<orbit_core::voice::CallState>,
}

impl From<&State> for DurableSnapshot {
    fn from(state: &State) -> Self {
        Self {
            recording: state.durable.recording.clone(),
            privacy_stopped: state.privacy_stopped(),
            privacy_stop_by: state.privacy_stop_by(),
            generation: state.generation(),
            connection: state.connection().clone(),
            session_id: state.session_id().clone(),
            agent_id: state.agent_id().map(str::to_owned),
            pending_approval: state.durable.pending_approval.clone(),
            call: state.durable.call.clone(),
        }
    }
}
impl Runtime {
    /// Records one diagnostic event. Never fails and never carries conversation
    /// content: the log module truncates every field it is given.
    pub(super) fn log(&self, event: &str, fields: &[(&str, &str)]) {
        if let Some(diagnostics) = &self.diagnostics {
            // Every line names the turn's op, which joins it to the report
            // log, the notice and the hub's own logs.
            let mut with_op = Vec::with_capacity(fields.len() + 1);
            with_op.push(("op", self.turn.op.as_str()));
            with_op.extend_from_slice(fields);
            diagnostics.record(event, &with_op);
        }
    }

    pub(super) fn connect(&mut self) {
        if self.state.privacy_stopped() || self.hub_task.as_ref().is_some_and(|t| !t.is_finished())
        {
            return;
        }
        if let (Some(token), Some(_), Some(_)) = (
            &self.token,
            &self.settings.hub_url,
            &self.settings.device_id,
        ) {
            // Existing 16-frame command queue: bounds local frames while the
            // websocket is blocked. It is not a Deimos receipt or a retry queue.
            match decide(&self.state, Event::Connecting, now()) {
                Ok(decision) => self.state = decision.state,
                Err(error) => {
                    self.log("connect_decision_failed", &[("error", &error.to_string())]);
                    return;
                }
            }
            let commands = self.hub.open(16);
            self.hub_task = Some(tokio::spawn(hub::run(
                self.settings.clone(),
                token.clone(),
                self.development,
                commands,
                hub::Events {
                    generation: self.state.generation(),
                    sender: self.hub_events.clone(),
                },
            )));
        }
    }

    pub(super) async fn apply(&mut self, event: Event) -> Result<()> {
        let at_ms = now();
        let voice = if matches!(event, Event::Tick)
            && self
                .state
                .durable
                .call
                .as_ref()
                .is_some_and(|call| call.expires_at_ms <= at_ms)
        {
            CallStopVoice::Cancel(VoiceCancellationCause::AutomaticCallStop(
                orbit_core::voice::StopReason::Expired,
            ))
        } else if matches!(
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
        };
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
        let voice = if matches!(decision.authority_change, Some(AuthorityChange::UserStop)) {
            CallStopVoice::Cancel(VoiceCancellationCause::Authority(AuthorityChange::UserStop))
        } else if matches!(decision.authority_change, Some(AuthorityChange::Closing)) {
            CallStopVoice::Cancel(VoiceCancellationCause::Authority(AuthorityChange::Closing))
        } else if matches!(decision.authority_change, Some(AuthorityChange::Revoked)) {
            CallStopVoice::Cancel(VoiceCancellationCause::Authority(AuthorityChange::Revoked))
        } else if let Some(change) = decision.authority_change
            && matches!(
                change,
                AuthorityChange::HubLink
                    | AuthorityChange::HubLinkOffline
                    | AuthorityChange::HubDisconnected
            )
            && self.state.durable.call.is_some()
            && decision
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::StopCall))
        {
            CallStopVoice::Cancel(VoiceCancellationCause::Authority(change))
        } else {
            voice
        };
        self.work.force_snapshot_once = force_snapshot;
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
        Box::pin(self.execute_boxed_decision(Box::new(decision), voice))
    }

    async fn execute_boxed_decision(
        &mut self,
        decision: Box<orbit_core::Decision>,
        voice: CallStopVoice,
    ) -> Result<()> {
        let mut force_snapshot = std::mem::take(&mut self.work.force_snapshot_once);
        let change = decision.authority_change;
        let plan = change.map(retire);
        // An in-place closure (FinishMeeting on Interrupted or under the
        // privacy brake) leaves the recording identity untouched, so
        // `preserve_replaced_recording` cannot author its durable closure.
        // Detect that transition before the snapshot overwrites the prior
        // state and mirror the path the existing CaptureFinalized drain
        // already takes: enqueue the closure so `finalize-<id>` exists.
        let previous_was_closed = self.state.durable.recording.as_ref().is_some_and(|r| {
            r.ended_at_ms.is_some()
                && matches!(r.phase, CapturePhase::Idle | CapturePhase::Interrupted)
        });
        self.validate_recording_start(&decision)?;
        let closure = self
            .preserve_replaced_recording(&decision.state)?
            .or(self.empty_interrupted_closure(&decision.state)?);
        let closure_committed = closure.is_some();
        let interrupted_request = self.state.durable.active_request.clone();
        let orbit_core::Decision {
            mut state, effects, ..
        } = *decision;
        if let Some((change, plan)) = change.zip(plan)
            && matches!(
                change,
                AuthorityChange::Conversation | AuthorityChange::Interrupt
            )
        {
            // The old command path cancelled sources before it applied a new
            // session or turn decision. A failed receipt cannot commit that
            // decision or retire its other volatile workers.
            self.execute_retire_plan(plan, change, RetirePhase::BeforeDecision, None)
                .await?;
            // The decision was calculated before the fallible source receipt.
            // Keep that receipt and the source notice when adopting its other
            // fields; otherwise a successful cancellation becomes Listing.
            state
                .views
                .source_inventory
                .clone_from(&self.state.views.source_inventory);
            state
                .views
                .source_request
                .clone_from(&self.state.views.source_request);
            match change {
                AuthorityChange::Conversation => {
                    state.durable.notice.clone_from(&self.state.durable.notice);
                    state
                        .durable
                        .notice_log
                        .clone_from(&self.state.durable.notice_log);
                }
                AuthorityChange::Interrupt => {
                    // The core has already added the interruption notice to
                    // its decision. Put it after the source notices produced
                    // by BeforeDecision, in the same order as the old command
                    // path.
                    // Those local cancellations also change published state:
                    // the decision was computed before they ran, so retain
                    // their settled call phase when adopting it.
                    state.durable.call.clone_from(&self.state.durable.call);
                    state
                        .durable
                        .notice_log
                        .clone_from(&self.state.durable.notice_log);
                    if let Some(notice) = state.durable.notice.clone() {
                        state.set_notice(notice);
                    }
                }
                AuthorityChange::UserStop
                | AuthorityChange::Revoked
                | AuthorityChange::Rearm
                | AuthorityChange::RearmWithoutStop
                | AuthorityChange::Closing
                | AuthorityChange::HubLink
                | AuthorityChange::HubTransportReady
                | AuthorityChange::HubLinkOffline
                | AuthorityChange::HubDisconnected
                | AuthorityChange::Device => {}
            }
        }
        // A failed decision snapshot can restore this point: the source
        // receipts have happened, but the new turn has not begun.
        let previous_state = self.state.clone();
        self.state = state;
        // A turn no longer in flight has ended, however it did: interrupted,
        // lost with the hub's link or stopped. Its work says when.
        if let Some(turn) = interrupted_request.as_deref()
            && self.state.durable.active_request.as_deref() != Some(turn)
        {
            self.end_work(turn);
        }
        if self.state.privacy_stopped()
            && !matches!(
                change,
                Some(
                    AuthorityChange::UserStop | AuthorityChange::Revoked | AuthorityChange::Closing
                )
            )
        {
            // A later Tick or unrelated decision must still close authority
            // that appeared while stopped. The old kernel repeated these
            // nine slots, but never repeated hub or durable retirement here.
            let stopped = retire_stopped_decision();
            self.execute_retire_plan(
                stopped,
                AuthorityChange::UserStop,
                RetirePhase::Volatile,
                None,
            )
            .await?;
            self.execute_retire_plan(
                stopped,
                AuthorityChange::UserStop,
                RetirePhase::PreSnapshot,
                None,
            )
            .await?;
        }
        if let Some((change, plan)) = change.zip(plan) {
            self.execute_retire_plan(plan, change, RetirePhase::Volatile, None)
                .await?;
            // A claimed input is withdrawn before its snapshot is stored.
            self.execute_retire_plan(plan, change, RetirePhase::PreSnapshot, None)
                .await?;
        }
        force_snapshot |= std::mem::take(&mut self.work.force_snapshot_once);
        let next_durable = DurableSnapshot::from(&self.state);
        let persisted = if let Some(job) = closure {
            self.store_actor
                .snapshot_with_job(&self.state, job)
                .await
                .map(|()| true)
        } else if force_snapshot || next_durable != self.work.durable_snapshot {
            self.store_actor.snapshot(&self.state).await.map(|()| false)
        } else {
            Ok(false)
        };
        match persisted {
            Ok(with_closure) => {
                if with_closure {
                    // This receipt covers the snapshot and replacement
                    // closure in one store transaction.
                    self.features.notes.head.touch();
                }
                self.work.durable_snapshot = next_durable;
            }
            Err(error) => {
                if change.is_none()
                    || matches!(
                        change,
                        Some(AuthorityChange::Conversation | AuthorityChange::Interrupt)
                    )
                {
                    self.state = previous_state;
                }
                return Err(error);
            }
        }
        let now_closed = self.state.durable.recording.as_ref().is_some_and(|r| {
            r.ended_at_ms.is_some()
                && matches!(r.phase, CapturePhase::Idle | CapturePhase::Interrupted)
        });
        if !previous_was_closed && now_closed && !closure_committed {
            self.queue_finalize().await?;
        }
        for effect in effects {
            if matches!(effect, Effect::Interrupt) {
                self.interrupt_turn(interrupted_request.as_deref(), false)
                    .await;
            } else if matches!(effect, Effect::StopCall) {
                self.stop_call_audio(voice).await?;
            } else {
                self.effect(effect).await?;
            }
        }
        if let Some((change, plan)) = change.zip(plan) {
            // The base sent capture_stop and Interrupt before a hub drain or
            // a fallible source retirement. A failed durable slot must not
            // skip either effect after a revocation.
            let before_retirement = self.state.clone();
            self.execute_retire_plan(plan, change, RetirePhase::Link, None)
                .await?;
            self.execute_retire_plan(plan, change, RetirePhase::Durable, None)
                .await?;
            if self.state != before_retirement {
                self.store_actor.snapshot(&self.state).await?;
                self.work.durable_snapshot = DurableSnapshot::from(&self.state);
            }
        }
        Ok(())
    }

    /// The agent a turn is for, and whose voice reads its answer: the open
    /// conversation's, or the default one. Only a hub that listed its agents
    /// is told: a 0.9.0 hub refuses the field in `chat.send` and has one
    /// agent anyway.
    pub(super) fn turn_agent_id(&self) -> Option<String> {
        let roster = self.settings.hub_agents.as_ref()?;
        Some(
            self.state
                .agent_id()
                .unwrap_or(roster.default_agent_id())
                .to_owned(),
        )
    }

    /// The agent whose voice reads the open conversation's answers aloud:
    /// its own, or the default agent's when it has none (as the hub falls back).
    pub(super) fn reading_voice(&self) -> String {
        let roster = self.settings.roster();
        let agent_id = self.state.agent_id().unwrap_or(roster.default_agent_id());
        roster.reading_voice(agent_id).to_owned()
    }

    pub(super) async fn effect(&mut self, effect: Effect) -> Result<()> {
        match effect {
            Effect::SendText {
                id,
                text,
                voice,
                meeting,
                attachments,
            } => {
                // The work shown is this turn's from now on: the last
                // turn's never stands for it.
                self.conversation.work.take_if(|work| work.turn_id != id);
                // The turn's id is the Mac's id for the user's message,
                // kept in the hub's row so the app finds their turn by it.
                let params = serde_json::to_value(orbit_core::wire::hub::ChatSendParams {
                    text,
                    session_id: self.state.session_id().clone(),
                    call_active: Some(self.state.durable.call.is_some()),
                    client_id: Some(id.clone()),
                    voice,
                    meeting,
                    agent_id: self.turn_agent_id(),
                    attachments,
                })?;
                self.send(hub::command(&id, "chat.send", params)?).await?
            }
            Effect::Interrupt => {
                let turn = self.state.durable.active_request.clone();
                self.interrupt_turn(turn.as_deref(), false).await;
            }
            Effect::StartCapture {
                recording_id,
                audio,
            } => {
                self.capture.interruption = None;
                self.start_recording_audio(recording_id, audio).await?;
            }
            Effect::StopCapture => self.stop_recording_audio().await?,
            Effect::CutCapture => {
                self.request_full_audio_stop(super::audio::FullStop::Cut)
                    .await?
            }
            Effect::StartCall => self.start_call_audio().await?,
            Effect::StopCall => {
                self.stop_call_audio(CallStopVoice::Cancel(VoiceCancellationCause::Authority(
                    AuthorityChange::Interrupt,
                )))
                .await?
            }
            Effect::StartVoice => self.start_voice_audio().await?,
            Effect::FinishVoice => self.finish_voice_audio().await?,
            Effect::CancelVoice => self.cancel_voice_audio().await?,
            Effect::StopPlayback => {
                self.conversation.speech_task.cancel();
                self.conversation.speak_next = false;
                self.native_send("playback_stop", json!({})).await?;
            }
            Effect::CancelNetwork => {}
        }
        Ok(())
    }
}
