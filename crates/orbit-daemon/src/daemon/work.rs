//! Background work completion and scheduling.
use super::ticketed::Settlement;
use super::*;
use orbit_core::notice::NoticeCode;

#[cfg(test)]
#[path = "uploaded_chunk_tests.rs"]
mod tests;

impl Runtime {
    pub(super) async fn work_event(&mut self, event: WorkResult) -> Result<()> {
        match event {
            WorkResult::Store(store) => Box::pin(self.store_work(store)).await?,
            WorkResult::BridgeCaptureUpload {
                ticket,
                request_id,
                capture,
                outcome,
            } => {
                self.on_bridge_capture_upload(ticket, request_id, capture, outcome)
                    .await?;
            }
            WorkResult::Control(crate::daemon::ControlWork::Computer { ticket, event }) => {
                self.on_work_computer(ticket, event).await?;
            }
            WorkResult::Capture(capture) => Box::pin(self.capture_work(capture)).await?,
            WorkResult::Problem { ticket, message } => {
                let settlement = self
                    .assistant
                    .speech_task
                    .settle(&ticket, self.state.generation());
                if self.settled_value("speech", settlement).is_some() {
                    // The clip never reached the helper: nothing is being read.
                    self.assistant.read_aloud = None;
                    self.on_work_problem(message).await?;
                }
            }
            WorkResult::SpeechClip {
                ticket,
                voice,
                command,
            } => {
                Box::pin(self.on_speech_availability(ticket, voice, true, Some(command))).await?;
            }
            WorkResult::SpeechAvailability {
                ticket,
                voice,
                available,
            } => {
                Box::pin(self.on_speech_availability(ticket, voice, available, None)).await?;
            }
            WorkResult::PhotoJpeg {
                source_path,
                destination_path,
                asked,
            } => {
                // An order a privacy stop already ended asks nothing more.
                if !asked.is_closed() {
                    let _ = asked.send(self.ask_helper(
                        "photo_jpeg",
                        json!({"source_path": source_path, "destination_path": destination_path}),
                        orbit_core::refusal::Operation::Files,
                    ));
                }
            }
        }
        Ok(())
    }

    async fn capture_work(&mut self, work: crate::daemon::CaptureWork) -> Result<()> {
        match work {
            crate::daemon::CaptureWork::VoiceProblem {
                ticket,
                operation_id,
                failure,
            } => {
                if self.settle_voice(&ticket, &operation_id, "voice").await? {
                    self.on_work_voice_problem(operation_id, failure).await?;
                }
            }
            crate::daemon::CaptureWork::Transcribed {
                ticket,
                source,
                text,
            } => {
                if self
                    .settle_voice(&ticket, &source.message_id, "voice_transcription")
                    .await?
                {
                    self.on_work_transcribed(source, text).await?;
                }
            }
        }
        Ok(())
    }

    async fn on_speech_availability(
        &mut self,
        ticket: ticketed::WorkTicket,
        voice: String,
        available: bool,
        command: Option<Value>,
    ) -> Result<()> {
        let settlement = self
            .assistant
            .speech_task
            .settle(&ticket, self.state.generation());
        if self.settled_value("speech", settlement).is_some() {
            // The voice that did not arrive is the one that was reading.
            let agent_name = crate::speech::agent_name(&self.settings, Some(&voice));
            self.assistant.speech_voices.insert(voice, available);
            if let Some(command) = command {
                self.helper.record_pending(&command);
                tokio::time::timeout(Duration::from_secs(2), self.helper.sender.send(command))
                    .await??;
            }
            if !available {
                self.assistant.read_aloud = None;
                self.on_work_problem(crate::speech::SpeechUnavailable { agent_name }.to_string())
                    .await?;
            }
        }
        Ok(())
    }

    pub(super) fn settled_value<T>(&self, work: &str, settlement: Settlement<T>) -> Option<T> {
        match settlement {
            Settlement::Current(value) => Some(value),
            Settlement::Foreign => None,
            Settlement::Expired {
                started_generation,
                current_generation,
            } => {
                self.note_expired_work(work, started_generation, current_generation);
                None
            }
        }
    }

    async fn settle_voice(
        &mut self,
        ticket: &ticketed::WorkTicket,
        operation_id: &str,
        work: &str,
    ) -> Result<bool> {
        let cause = match self
            .assistant
            .voice_task
            .settle(ticket, self.state.generation())
        {
            Settlement::Current(())
                if !self.state.privacy_stopped()
                    && self.assistant.voice_operation.as_deref() == Some(operation_id) =>
            {
                return Ok(true);
            }
            Settlement::Current(()) => {
                if self.state.privacy_stopped() {
                    "voice_transcription_privacy_stopped"
                } else {
                    "voice_transcription_superseded"
                }
            }
            Settlement::Foreign => return Ok(false),
            Settlement::Expired {
                started_generation,
                current_generation,
            } => {
                self.note_expired_work(work, started_generation, current_generation);
                "voice_transcription_generation_changed"
            }
        };
        self.on_work_voice_problem(
            operation_id.to_owned(),
            orbit_core::VoiceTranscriptionFailure::local(cause),
        )
        .await?;
        Ok(false)
    }

    /// An expired result with nobody waiting for it is noted once, with its
    /// work kind and both generations, and without content.
    pub(super) fn note_expired_work(
        &self,
        work: &str,
        started_generation: u64,
        current_generation: u64,
    ) {
        if let Some(diagnostics) = &self.diagnostics {
            diagnostics.record(
                "work_result_expired",
                &[
                    ("work", work),
                    ("cause", "generation_changed"),
                    ("started_generation", &started_generation.to_string()),
                    ("current_generation", &current_generation.to_string()),
                ],
            );
        }
    }

    async fn on_work_transcribed(
        &mut self,
        source: audio::VoiceSource,
        text: String,
    ) -> Result<()> {
        self.assistant.voice_operation = None;
        let over_answer = self.audio.take_over_answer_turn(&source.message_id);
        if text.trim().is_empty() {
            // Nothing was heard: noise, silence, or a person who had not
            // finished. Never a failure: the turn leaves no note, and in a
            // call its audio waits for what the user says next.
            let in_call = self.audio.call.is_some();
            if let Some(turn) = over_answer {
                self.carry_into_next_turn(call::call_turn_audio(turn.operation_id, turn.voice));
                self.note_dropped_voice("no_words");
            } else {
                if let Some(audio) = self.audio.take_submitted_call_turn(&source.message_id) {
                    self.carry_into_next_turn(audio);
                }
                self.drop_unheard_voice_note(&source.message_id, "no_words")
                    .await?;
            }
            self.resume_call_reading().await?;
            // A voice note pressed for, and silent, is said in a small line;
            // in a call nothing is said.
            self.say(if in_call {
                NoticeCode::VoiceNotHeard
            } else {
                NoticeCode::VoiceNotRecognized
            });
            return Ok(());
        }
        // Words keep a call going; sound alone does not (`idle_timeout_ms`).
        self.audio.note_call_words();
        if let Some(turn) = over_answer
            && !self.accept_over_answer(turn, None).await?
        {
            return Ok(());
        }
        // Interrupt retirement clears the old answer's speech request. Arm
        // this answer only after that retirement has completed.
        self.assistant.speak_next = !self.recording_active();
        if !self
            .ask_meeting_by_dictation(source.message_id.clone(), text.clone(), source.duration_ms)
            .await?
        {
            self.apply(Event::SubmitVoice {
                id: source.message_id,
                text,
                duration_ms: source.duration_ms,
            })
            .await?;
        }
        Ok(())
    }

    async fn on_work_problem(&mut self, message: String) -> Result<()> {
        let call_id = self
            .state
            .durable
            .voice
            .call
            .as_ref()
            .map(|call| call.id.clone());
        if call_id.is_none() || self.assistant.speech_notice_call_id.as_ref() != call_id.as_ref() {
            self.say_failure(NoticeCode::SpeechFailed, &message);
            self.assistant.speech_notice_call_id = call_id;
        }
        Ok(())
    }

    pub(super) async fn on_work_sealed(&mut self) -> Result<()> {
        // The receipt is counted whoever took it from the writer first: this
        // wake-up, the collector of a later seal or the capture task.
        self.collect_ready_seals(sealing::OPPORTUNISTIC_RECEIPT_WAIT, false)?;
        // Outside any condition. This used to be the only place that resumed
        // a deferred finish, and it only ran when this path was the one that
        // applied the receipt. When the collector had already taken the same
        // completion the resume never happened, and the recording was
        // interrupted fifteen seconds later for a confirmation it had
        // already had.
        self.resume_deferred_drain().await?;
        Ok(())
    }

    pub(super) async fn on_work_store_problem(&mut self, message: String) -> Result<()> {
        // Said once while it lasts: the age the message carries changes on
        // every report, the problem does not.
        self.log("store_problem", &[("message", &message)]);
        if self.state.notice_code() != Some(NoticeCode::StoreProblem) {
            self.say(NoticeCode::StoreProblem);
        }
        Ok(())
    }

    async fn on_work_voice_problem(
        &mut self,
        operation_id: String,
        failure: orbit_core::VoiceTranscriptionFailure,
    ) -> Result<()> {
        let current = self.assistant.voice_operation.as_deref() == Some(operation_id.as_str());
        if current {
            self.assistant.voice_operation = None;
        }
        // A turn spoken over Deimos's answer that could not be heard does
        // not stop him; it is kept as a note that says why.
        if let Some(turn) = self.audio.take_over_answer_turn(&operation_id) {
            self.accept_over_answer(turn, Some(failure)).await?;
            self.resume_call_reading().await?;
            if current {
                self.say(NoticeCode::VoiceTranscriptionFailed);
            }
            return Ok(());
        }
        self.apply(orbit_core::Event::VoiceFailed {
            id: operation_id,
            failure,
        })
        .await?;
        if current {
            self.say(NoticeCode::VoiceTranscriptionFailed);
        }
        // The authority fingerprint omits chat messages. This receipt must
        // persist explicitly or restart restores the pending voice note.
        self.store_actor.snapshot(&self.state).await?;
        Ok(())
    }

    async fn on_work_computer(
        &mut self,
        ticket: ticketed::WorkTicket,
        event: Box<Result<computer_control::ComputerCompletion>>,
    ) -> Result<()> {
        self.computer_event(&ticket, *event).await?;
        Ok(())
    }
}
