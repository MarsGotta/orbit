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
            WorkResult::Store(crate::daemon::StoreWork::ChunkPruned { ticket, result }) => {
                let settlement = self.work.worker.settle(&ticket, self.state.generation());
                if self.settled_value("chunk_cleanup", settlement).is_some()
                    && let Err(error) = result
                {
                    self.log(
                        "uploaded_chunk_delete_failed",
                        &[("error", &error.to_string())],
                    );
                }
            }
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
            WorkResult::Capture(crate::daemon::CaptureWork::VoiceProblem {
                ticket,
                operation_id,
                failure,
            }) => {
                if self.settle_voice(&ticket, &operation_id, "voice").await? {
                    self.on_work_voice_problem(operation_id, failure).await?;
                }
            }
            WorkResult::Capture(crate::daemon::CaptureWork::Transcribed {
                ticket,
                source,
                text,
            }) => {
                if self
                    .settle_voice(&ticket, &source.message_id, "voice_transcription")
                    .await?
                {
                    self.on_work_transcribed(source, text).await?;
                }
            }
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
            WorkResult::SpeechAvailability { ticket, available } => {
                let settlement = self
                    .conversation
                    .speech_task
                    .settle(&ticket, self.state.generation());
                if self.settled_value("speech", settlement).is_some() {
                    self.conversation.speech_available = Some(available);
                    if !available {
                        self.conversation.read_aloud = None;
                        self.on_work_problem(
                            crate::speech::SpeechUnavailable {
                                agent_name: crate::speech::agent_name(
                                    &self.settings,
                                    self.turn_agent_id().as_deref(),
                                ),
                            }
                            .to_string(),
                        )
                        .await?;
                    }
                }
            }
            WorkResult::Store(crate::daemon::StoreWork::Uploaded {
                ticket,
                job_id,
                result,
            }) => {
                let settlement = self.work.worker.settle(&ticket, self.state.generation());
                if self.settled_value("upload", settlement).is_some()
                    && !self.state.privacy_stopped()
                {
                    self.on_work_uploaded(job_id, result).await?;
                }
            }
            WorkResult::Store(crate::daemon::StoreWork::UploadSettled {
                ticket,
                succeeded,
                error,
                result,
            }) => {
                let settlement = self.work.worker.settle(&ticket, self.state.generation());
                if self
                    .settled_value("upload_settlement", settlement)
                    .is_some()
                    && !self.state.privacy_stopped()
                {
                    self.on_work_upload_settled(succeeded, error, result)
                        .await?;
                }
            }
            WorkResult::Store(crate::daemon::StoreWork::Sealed {
                recording,
                sequence,
            }) => {
                self.on_work_sealed(recording, sequence).await?;
            }
            WorkResult::Store(crate::daemon::StoreWork::StoreProblem { message }) => {
                self.on_work_store_problem(message).await?;
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

    pub(super) async fn schedule_upload(&mut self) -> Result<()> {
        if !self.state.privacy_stopped()
            && self.state.connection() != Connection::Online
            && self.work.worker.active().is_none()
        {
            self.schedule_chunk_cleanup()?;
        }
        // While seals are pending, only a chunk upload may enter the
        // ordered actor. Its durable admission waits on the worker, so prior
        // seals keep their order without holding the daemon loop; the
        // closure path retains its own eligibility rules once sealing is
        // caught up. This is not a second audio queue or an upload-rate limit.
        if !self.capture.pending_seals.is_empty() {
            if self.state.privacy_stopped()
                || self.state.connection() != Connection::Online
                || self.work.worker.active().is_some()
                || !self.work.retry_gate.ready(std::time::Instant::now())
            {
                return Ok(());
            }
            self.schedule_chunk_upload().await?;
            return Ok(());
        }
        self.queue_finalize().await?;
        if self.state.privacy_stopped()
            || self.state.connection() != Connection::Online
            || self.work.worker.active().is_some()
            || !self.work.retry_gate.ready(std::time::Instant::now())
        {
            return Ok(());
        }
        // Alternate closure and upload: neither can monopolize the worker.
        if self.work.prefer_finalize && self.schedule_finalize().await? {
            self.work.prefer_finalize = false;
            return Ok(());
        }
        if self.schedule_chunk_upload().await? {
            self.work.prefer_finalize = true;
            return Ok(());
        }
        if self.schedule_finalize().await? {
            self.work.prefer_finalize = false;
        } else {
            self.schedule_chunk_cleanup()?;
        }
        Ok(())
    }

    fn schedule_chunk_cleanup(&mut self) -> Result<()> {
        let Some(id) = self.reader.next_uploaded_chunk()? else {
            return Ok(());
        };
        let root = self.paths.root.clone();
        let cleanup = self.store_actor.begin_run(move |store| {
            prune_uploaded_audio(store, &id, &root);
            Ok(())
        })?;
        let events = self.work.work_events.clone();
        self.work
            .worker
            .start_logical(
                self.state.generation(),
                self.turn.op.clone(),
                (),
                move |ticket| {
                    tokio::spawn(async move {
                        let result = cleanup.wait().await;
                        crate::daemon::deliver_work(
                            &events,
                            WorkResult::Store(crate::daemon::StoreWork::ChunkPruned {
                                ticket,
                                result,
                            }),
                        )
                        .await;
                    })
                },
            )
            .context("No se pudo reservar el borrado del audio confirmado")?;
        Ok(())
    }

    /// Consume at most one upload candidate, including a locally quarantined
    /// source. This bound prevents cancellation cleanup from monopolizing actor
    /// time; the other classes receive their turn on the next admission.
    pub(super) async fn schedule_chunk_upload(&mut self) -> Result<bool> {
        let mut jobs = self.reader.pending_jobs_of_kind(
            "chunk_upload",
            self.work.upload_cursor.as_deref(),
            1,
        )?;
        if jobs.is_empty() {
            self.work.upload_cursor = None;
            jobs = self.reader.pending_jobs_of_kind("chunk_upload", None, 1)?;
        }
        let Some(job) = jobs.pop() else {
            return Ok(false);
        };
        self.work.upload_cursor = Some(job.id.clone());
        let chunk_id = job.payload["metadata"]["id"]
            .as_str()
            .context("Trabajo sin chunk_id")?;
        let receipt = self
            .reader
            .chunk_receipt(chunk_id)?
            .context("Fragmento pendiente sin recibo local")?;
        if let Some(manifest) = self
            .reader
            .get_job(&format!("recording-{}", receipt.metadata.recording_id))?
            && manifest.kind == "recording_manifest"
            && manifest.payload["upload_allowed"] == false
        {
            let job_id = job.id.clone();
            self.store_actor
                .run(move |store| Ok(store.update_job(&job_id, JobState::Cancelled)?))
                .await?;
            return Ok(true);
        }
        let bytes = self.reader.read_chunk(chunk_id)?;
        let token = self.token.clone().context("Sin credencial")?;
        let base = crate::config::validate_hub_url(
            self.settings.hub_url.as_deref().context("Sin hub")?,
            self.development,
        )?;
        let generation = self.state.generation();
        let events = self.work.work_events.clone();
        let job_id = job.id.clone();
        let admission = if self.recording_active() || !self.capture.pending_seals.is_empty() {
            Some(
                self.store_actor
                    .begin_run(move |store| Ok(store.update_job(&job_id, JobState::Running)?))?,
            )
        } else {
            self.store_actor
                .run(move |store| Ok(store.update_job(&job_id, JobState::Running)?))
                .await?;
            None
        };
        self.work
            .worker
            .start_logical(generation, self.turn.op.clone(), (), move |ticket| {
                tokio::spawn(async move {
                    let result: Result<()> = async {
                        if let Some(receipt) = admission {
                            receipt.wait().await?;
                        }
                        let mut request = hub::request(
                            reqwest::Method::PUT,
                            base.join(&format!("v1/chunks/{}", receipt.metadata.id))?,
                            Some(&token),
                            ticket.op(),
                        )?
                        .header("Content-Type", "audio/wav")
                        .header("X-Orbit-Recording", &receipt.metadata.recording_id)
                        .header("X-Orbit-Track", &receipt.metadata.track)
                        .header("X-Orbit-Sequence", receipt.metadata.sequence)
                        .header("X-Orbit-Started-At-Ms", receipt.metadata.started_at_ms)
                        .header("X-Orbit-Monotonic-Ns", receipt.metadata.monotonic_ns)
                        .header("X-Orbit-First-Sample", receipt.metadata.first_sample)
                        .header("X-Orbit-Frames", receipt.metadata.frames)
                        .header("X-Orbit-Sample-Rate", receipt.metadata.sample_rate)
                        .header("X-Orbit-Channels", receipt.metadata.channels)
                        .header("X-Orbit-SHA256", &receipt.sha256);
                        if let Some(epoch) = &receipt.metadata.capture_epoch_id {
                            request = request.header("X-Orbit-Capture-Epoch-Id", epoch);
                        }
                        let response = request.body(bytes).send().await?;
                        crate::retry::check_status(response.status())?;
                        let ack = hub::bounded_json(response).await?;
                        if ack["durable"] != true
                            || ack["sha256"] != receipt.sha256
                            || ack["chunk_id"] != receipt.metadata.id
                        {
                            bail!("El servidor no confirmó un recibo durable válido");
                        }
                        Ok(())
                    }
                    .await;
                    crate::daemon::deliver_work(
                        &events,
                        WorkResult::Store(crate::daemon::StoreWork::Uploaded {
                            ticket,
                            job_id: job.id,
                            result,
                        }),
                    )
                    .await;
                })
            })
            .context("No se pudo reservar la subida del fragmento")?;
        Ok(true)
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

    async fn on_work_uploaded(&mut self, job_id: String, result: Result<()>) -> Result<()> {
        // This transfer has delivered its outcome; the settle above already
        // released the single worker before any fallible local receipt
        // write: a finished handle must not block the queue after a
        // recoverable disk error. Running/Unknown durable jobs remain
        // eligible for idempotent PUT/finalize reconciliation; no prompt is
        // replayed here.
        let succeeded = result.is_ok();
        self.work
            .retry_gate
            .finish(succeeded, std::time::Instant::now());
        let stored_job = self
            .reader
            .get_job(&job_id)?
            .context("Recibo sin trabajo local")?;
        if stored_job.kind == "chunk_upload"
            && let Some(recording) = stored_job.payload["metadata"]["recording_id"].as_str()
        {
            self.work.finalize_ready.remove(recording);
        }
        let next_state = if result.is_ok() {
            JobState::Succeeded
        } else {
            if result
                .as_ref()
                .err()
                .is_some_and(|error| error.downcast_ref::<crate::retry::Refused>().is_some())
            {
                JobState::Blocked
            } else {
                JobState::RetryWait
            }
        };
        let chunk_id = (succeeded && stored_job.kind == "chunk_upload")
            .then(|| {
                stored_job.payload["metadata"]["id"]
                    .as_str()
                    .map(str::to_owned)
            })
            .flatten();
        let root = self.paths.root.clone();
        if stored_job.kind == "chunk_upload"
            && (self.recording_active() || !self.capture.pending_seals.is_empty())
        {
            let error = result.as_ref().err().map(ToString::to_string);
            let updated_job = job_id.clone();
            let settlement = self.store_actor.begin_run(move |store| {
                store.update_job(&updated_job, next_state)?;
                if let Some(id) = chunk_id {
                    prune_uploaded_audio(store, &id, &root);
                }
                Ok(())
            })?;
            let events = self.work.work_events.clone();
            self.work
                .worker
                .start_logical(
                    self.state.generation(),
                    self.turn.op.clone(),
                    (),
                    move |ticket| {
                        tokio::spawn(async move {
                            let result = settlement.wait().await.map(|_| ()).with_context(|| {
                                format!("No se pudo guardar el resultado de subida {job_id}")
                            });
                            // This send waits off the daemon loop, on a
                            // worker, so a slow actor cannot hold IPC. Its failure
                            // has no local recovery to check: it only happens when
                            // the receiver that owns the runtime is gone, the
                            // settlement above is already durable, and there is no
                            // loop left to wake.
                            crate::daemon::deliver_work(
                                &events,
                                WorkResult::Store(crate::daemon::StoreWork::UploadSettled {
                                    ticket,
                                    succeeded,
                                    error,
                                    result,
                                }),
                            )
                            .await;
                        })
                    },
                )
                .context("No se pudo reservar la espera del resultado de subida")?;
            return Ok(());
        }
        if succeeded && stored_job.kind == "recording_finalize" {
            let recording = stored_job.payload["recording_id"]
                .as_str()
                .context("Cierre sin identidad")?;
            // The hub has it whole and asks for its note itself;
            // the list reads it from there.
            self.recording_uploaded(recording);
        }
        let updated_job = job_id.clone();
        self.store_actor
            .run(move |store| {
                store.update_job(&updated_job, next_state)?;
                if let Some(id) = chunk_id {
                    prune_uploaded_audio(store, &id, &root);
                }
                Ok(())
            })
            .await?;
        if let Err(error) = result {
            self.log("upload_failed", &[("error", &error.to_string())]);
            self.say(NoticeCode::UploadRetrying);
        }
        if succeeded {
            self.schedule_upload().await?;
        }
        Ok(())
    }

    async fn on_work_upload_settled(
        &mut self,
        succeeded: bool,
        error: Option<String>,
        result: Result<()>,
    ) -> Result<()> {
        result?;
        if succeeded {
            self.schedule_upload().await?;
        } else if let Some(error) = error {
            self.log("upload_failed", &[("error", &error)]);
            self.say(NoticeCode::UploadRetrying);
        }
        Ok(())
    }

    async fn on_work_sealed(&mut self, recording: String, sequence: u64) -> Result<()> {
        let id = self
            .capture
            .pending_seals
            .iter()
            .find(|segment| {
                segment.metadata.recording_id == recording && segment.metadata.sequence == sequence
            })
            .map(|segment| segment.metadata.id.clone());
        if let Some(id) = id
            && self.apply_sealed_receipt(&id, &recording, sequence)
        {
            self.defer_sealed_snapshot()?;
        }
        // Outside the `if`. This used to be the only place that
        // resumed a deferred finish, and it only ran when this path was
        // the one that applied the receipt. When `collect_ready_seals`
        // had already taken the same completion there was no `id`, the
        // resume never happened, and the recording was interrupted
        // fifteen seconds later for a confirmation it had already had.
        self.resume_deferred_drain().await?;
        Ok(())
    }

    async fn on_work_store_problem(&mut self, message: String) -> Result<()> {
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

fn prune_uploaded_audio(store: &mut Store, id: &str, root: &std::path::Path) {
    let log = crate::diagnostics::DiagnosticLog::in_root(root);
    match store.prune_uploaded_chunk(id) {
        Ok(Some(bytes)) => log.record(
            "uploaded_chunk_deleted",
            &[("chunks", "1"), ("bytes", &bytes.to_string())],
        ),
        Ok(None) => {}
        Err(error) => log.record(
            "uploaded_chunk_delete_failed",
            &[
                ("chunks", "0"),
                ("pending_chunks", "1"),
                ("error", &error.to_string()),
            ],
        ),
    }
}
