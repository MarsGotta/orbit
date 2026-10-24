//! The call's frames: each one feeds the turn detector, moves the call's
//! phase and, once in both buffers, finishes a turn; an interrupt cuts the
//! voice being captured.
use super::*;

#[derive(Deserialize)]
pub(super) struct CallFrameFacts {
    pub(super) frames: u32,
    pub(super) sample_rate: u32,
    pub(super) monotonic_ns: u64,
    pub(super) playback_active: bool,
}

impl Runtime {
    pub(in crate::daemon) fn interrupt_call_turn(&mut self) {
        if let Some(call) = self
            .audio
            .call
            .as_mut()
            .filter(|call| call.closing_at_ms.is_none())
        {
            // Cutting the voice is all this does. The request, the tool and the
            // turn behind it continue, and the answer still arrives as text.
            call.detector = call.detector.cancelled_turn();
            call.preroll.clear();
            call.preroll_ns = 0;
            call.joined = None;
            call.carried = None;
            call.capturing_over_answer = false;
            self.capture.buffer.voice = None;
            if let Some(state) = self.state.durable.voice.call.as_mut() {
                state.phase = CallPhase::Listening;
            }
        }
    }

    pub(in crate::daemon) async fn prepare_call_frame(
        &mut self,
        frame: &Value,
        received_at_ms: i64,
    ) -> Result<TurnAction> {
        let Some(call) = self.audio.call.as_ref() else {
            return Ok(TurnAction::None);
        };
        if call.closing_at_ms.is_some() || self.state.durable.voice.call.is_none() {
            return Ok(TurnAction::None);
        }
        let Some(expected_pipeline) = call.pipeline_id.as_deref() else {
            return Ok(TurnAction::None);
        };
        let facts = CallFrameFacts::deserialize(frame);
        if processing_pipeline(frame) != Some(expected_pipeline) || facts.is_err() {
            if let Some(call) = self.audio.call.as_mut() {
                call.gaps = call.gaps.saturating_add(1);
            }
            bail!(
                "Cambió o se perdió la evidencia de procesamiento/referencia de la llamada; requiere reinicio explícito"
            );
        }
        let facts = facts?;
        let pcm = crate::capture::frame_pcm(frame)?;
        let frames = u64::from(facts.frames);
        let rate = u64::from(facts.sample_rate);
        let duration_ns = frames * 1_000_000_000 / rate;
        let policy = VoicePolicy::default();
        let responding = facts.playback_active || self.assistant.speech_task.active().is_some();
        // A turn's words being asked for is not Deimos at work: noise
        // transcribed into nothing must not keep a forgotten call open.
        let transcribing = self.assistant.voice_task.active().is_some();
        let working = self.state.durable.assistant.active_request.is_some();
        let waiting = transcribing || working;
        let observation = VoiceObservation {
            generation: self.state.generation(),
            authorized: !self.state.privacy_stopped() && self.state.durable.voice.call.is_some(),
            processing_observed: true,
            monotonic_ns: facts.monotonic_ns,
            duration_ns,
            received_at_ms,
            voiced: energy_exceeds(&pcm, policy.rms_threshold),
            response_active: responding || working,
            transcribing,
            muted: self.microphone_muted(),
        };
        let (action, preroll, began) = self.audio.observe_call_audio(
            frame,
            received_at_ms,
            observation,
            &pcm,
            facts.playback_active,
        )?;
        match action {
            TurnAction::Begin { interrupt } => {
                self.begin_call_turn(interrupt, responding, began, preroll)
                    .await?;
            }
            TurnAction::Stop(reason) => {
                if reason == StopReason::Idle {
                    self.log_call_idle_stop(reason, received_at_ms);
                }
                self.stop_call_for_reason(reason).await?;
            }
            TurnAction::None
                if !self
                    .audio
                    .call
                    .as_ref()
                    .is_some_and(|call| call.detector.is_capturing()) =>
            {
                self.update_call_phase(responding, waiting).await?;
            }
            TurnAction::None | TurnAction::Finish { .. } => {}
        }
        Ok(action)
    }

    /// The call's phase while no turn is being captured.
    pub(super) async fn update_call_phase(
        &mut self,
        responding: bool,
        waiting: bool,
    ) -> Result<()> {
        let phase = if responding {
            CallPhase::Responding
        } else if waiting {
            CallPhase::Waiting
        } else {
            CallPhase::Listening
        };
        if self
            .state
            .durable
            .voice
            .call
            .as_ref()
            .is_some_and(|call| call.phase != phase)
        {
            self.apply(Event::CallPhase(phase)).await?;
        }
        Ok(())
    }

    /// Runs after the current frame has reached both original and derived buffers.
    pub(in crate::daemon) async fn finish_call_frame(&mut self, action: TurnAction) -> Result<()> {
        if let TurnAction::Finish { submit } = action {
            self.finish_call_voice(submit).await?;
            if let Some(call) = self.state.durable.voice.call.as_mut() {
                call.phase = if submit {
                    CallPhase::Waiting
                } else {
                    CallPhase::Listening
                };
            }
            self.store_actor.snapshot(&self.state).await?;
        }
        Ok(())
    }
}
