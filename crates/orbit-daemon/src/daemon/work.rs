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
                    .conversation
                    .speech_task
                    .settle(&ticket, self.state.generation());
                if self.settled_value("speech", settlement).is_some() {
                    // The clip never reached the helper: nothing is being read.
                    self.conversation.read_aloud = None;
                    self.on_work_problem(message).await?;
                }
            }
            WorkResult::SpeechAvailability {
                ticket,
                voice,
                available,
            } => {
                Box::pin(self.on_speech_availability(ticket, voice, available)).await?;
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
    ) -> Result<()> {
        let settlement = self
            .conversation
            .speech_task
            .settle(&ticket, self.state.generation());
        if self.settled_value("speech", settlement).is_some() {
            // The voice that did not arrive is the one that was reading.
            let agent_name = crate::speech::agent_name(&self.settings, Some(&voice));
            self.conversation.speech_voices.insert(voice, available);
            if !available {
                self.conversation.read_aloud = None;
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
            .conversation
            .voice_task
            .settle(ticket, self.state.generation())
        {
            Settlement::Current(())
                if !self.state.privacy_stopped()
                    && self.conversation.voice_operation.as_deref() == Some(operation_id) =>
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
        self.conversation.voice_operation = None;
        self.conversation.speak_next = !self.recording_active();
        if text.trim().is_empty() {
            self.apply(Event::VoiceFailed {
                id: source.message_id,
                failure: orbit_core::VoiceTranscriptionFailure::local("voice_transcription_empty"),
            })
            .await?;
            self.say(NoticeCode::VoiceNotRecognized);
        } else if !self
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
        let call_id = self.state.durable.call.as_ref().map(|call| call.id.clone());
        if call_id.is_none() || self.conversation.speech_notice_call_id.as_ref() != call_id.as_ref()
        {
            self.say_failure(NoticeCode::SpeechFailed, &message);
            self.conversation.speech_notice_call_id = call_id;
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
        let current = self.conversation.voice_operation.as_deref() == Some(operation_id.as_str());
        if current {
            self.conversation.voice_operation = None;
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
