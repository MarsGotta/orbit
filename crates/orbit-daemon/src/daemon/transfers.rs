//! The loop's transfers: a recording's closure and the cleanup of audio the
//! hub confirmed. Its chunks go up from the capture task (`capture_lane`),
//! which a held loop cannot hold; both share the retry gate and the rule
//! that settles a transfer (`upload_outcome`).
use super::upload_outcome::{HUB_FULL_RETRY, Settlement, prune_uploaded_audio, settle};
use super::*;
use orbit_core::notice::{Notice, NoticeCode};

/// The transfers' handle: the one durable transfer worker and what decides
/// when the next closure may go.
pub(super) struct TransfersHandle {
    /// The one durable transfer worker (upload or finalize). A second job
    /// waits for its turn; this is not a queue limit.
    pub(super) worker: ticketed::Ticketed<()>,
    pub(super) retry_gate: upload_outcome::SharedGate,
    pub(super) finalize_cursor: Option<String>,
    pub(super) finalize_ready: BTreeMap<String, bool>,
}

impl Default for TransfersHandle {
    fn default() -> Self {
        Self {
            worker: ticketed::Ticketed::logical(),
            retry_gate: upload_outcome::SharedGate::default(),
            finalize_cursor: None,
            finalize_ready: BTreeMap::new(),
        }
    }
}

impl Runtime {
    pub(super) async fn schedule_upload(&mut self) -> Result<()> {
        if !self.state.privacy_stopped()
            && self.state.connection() != Connection::Online
            && self.transfers.worker.active().is_none()
        {
            self.schedule_chunk_cleanup()?;
        }
        // While seals are pending the recording may still get chunks: its
        // closure waits for them. The chunks themselves go up from the
        // capture task meanwhile, without the loop.
        if self.capture.seals_pending() {
            return Ok(());
        }
        self.queue_finalize().await?;
        if self.state.privacy_stopped()
            || self.state.connection() != Connection::Online
            || self.transfers.worker.active().is_some()
            || !self.transfers.retry_gate.ready(std::time::Instant::now())
        {
            return Ok(());
        }
        if !self.schedule_finalize().await? {
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
        let events = self.work_events.clone();
        self.transfers
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

    /// The transfer results the loop's worker carries back.
    pub(super) async fn store_work(&mut self, work: crate::daemon::StoreWork) -> Result<()> {
        use crate::daemon::StoreWork;
        match work {
            StoreWork::ChunkPruned { ticket, result } => {
                let settlement = self
                    .transfers
                    .worker
                    .settle(&ticket, self.state.generation());
                if self.settled_value("chunk_cleanup", settlement).is_some()
                    && let Err(error) = result
                {
                    self.log(
                        "uploaded_chunk_delete_failed",
                        &[("error", &error.to_string())],
                    );
                }
            }
            StoreWork::Uploaded {
                ticket,
                job_id,
                result,
            } => {
                let settlement = self
                    .transfers
                    .worker
                    .settle(&ticket, self.state.generation());
                if self.settled_value("upload", settlement).is_some()
                    && !self.state.privacy_stopped()
                {
                    self.on_work_uploaded(job_id, result).await?;
                }
            }
            StoreWork::UploadSettled {
                ticket,
                succeeded,
                error,
                notice,
                result,
            } => {
                let settlement = self
                    .transfers
                    .worker
                    .settle(&ticket, self.state.generation());
                if self
                    .settled_value("upload_settlement", settlement)
                    .is_some()
                    && !self.state.privacy_stopped()
                {
                    self.on_work_upload_settled(succeeded, error, notice, result)
                        .await?;
                }
            }
            StoreWork::Sealed { .. } => self.on_work_sealed().await?,
            StoreWork::StoreProblem { message } => self.on_work_store_problem(message).await?,
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
        let now = std::time::Instant::now();
        self.transfers.retry_gate.finish(result.is_ok(), now);
        let stored_job = self
            .reader
            .get_job(&job_id)?
            .context("Recibo sin trabajo local")?;
        let settlement = settle(&stored_job, &result);
        if settlement.out_of_space {
            self.transfers.retry_gate.hold(HUB_FULL_RETRY, now);
        }
        if let Some(recording) = &settlement.chunk_recording {
            self.transfers.finalize_ready.remove(recording);
        }
        if stored_job.kind == "chunk_upload"
            && (self.recording_active() || self.capture.seals_pending())
        {
            return self.settle_chunk_off_loop(job_id, settlement, result);
        }
        if settlement.succeeded && stored_job.kind == "recording_finalize" {
            let recording = stored_job.payload["recording_id"]
                .as_str()
                .context("Cierre sin identidad")?;
            // The hub has it whole and asks for its note itself;
            // the list reads it from there.
            self.notes.recording_uploaded(recording);
        }
        let root = self.paths.root.clone();
        let (next_state, prune) = (settlement.next_state, settlement.prune.clone());
        self.store_actor
            .run(move |store| {
                upload_outcome::record_settlement(
                    store,
                    &job_id,
                    next_state,
                    prune.as_deref(),
                    &root,
                )
            })
            .await?;
        if let Err(error) = result {
            self.log("upload_failed", &[("error", &format!("{error:#}"))]);
            self.say_upload_failure(settlement.notice);
        }
        if settlement.succeeded {
            self.schedule_upload().await?;
        }
        Ok(())
    }

    /// A chunk's settlement while a recording runs: admitted to the writer
    /// at once and waited for off the loop, so a slow writer cannot hold it.
    fn settle_chunk_off_loop(
        &mut self,
        job_id: String,
        settlement: Settlement,
        result: Result<()>,
    ) -> Result<()> {
        let error = result.as_ref().err().map(|error| format!("{error:#}"));
        let root = self.paths.root.clone();
        let updated_job = job_id.clone();
        let (next_state, prune) = (settlement.next_state, settlement.prune.clone());
        let receipt = self.store_actor.begin_run(move |store| {
            upload_outcome::record_settlement(
                store,
                &updated_job,
                next_state,
                prune.as_deref(),
                &root,
            )
        })?;
        let events = self.work_events.clone();
        let (succeeded, notice) = (settlement.succeeded, settlement.notice);
        self.transfers
            .worker
            .start_logical(
                self.state.generation(),
                self.turn.op.clone(),
                (),
                move |ticket| {
                    tokio::spawn(async move {
                        let result = receipt.wait().await.map(|_| ()).with_context(|| {
                            format!("No se pudo guardar el resultado de subida {job_id}")
                        });
                        // This send waits off the daemon loop, on a worker, so a
                        // slow actor cannot hold IPC. It only fails when the loop
                        // that owns the runtime is gone, and the settlement above
                        // is already durable.
                        crate::daemon::deliver_work(
                            &events,
                            WorkResult::Store(crate::daemon::StoreWork::UploadSettled {
                                ticket,
                                succeeded,
                                error,
                                notice,
                                result,
                            }),
                        )
                        .await;
                    })
                },
            )
            .context("No se pudo reservar la espera del resultado de subida")?;
        Ok(())
    }

    async fn on_work_upload_settled(
        &mut self,
        succeeded: bool,
        error: Option<String>,
        notice: Option<Notice>,
        result: Result<()>,
    ) -> Result<()> {
        result?;
        if succeeded {
            self.schedule_upload().await?;
        } else if let Some(error) = error {
            self.log("upload_failed", &[("error", &error)]);
            self.say_upload_failure(notice);
        }
        Ok(())
    }

    /// A failed transfer is said once while it lasts: the same refusal on
    /// every retry would push everything else out of the notices. A
    /// transient one (network, timeout, the hub's 5xx) stays silent.
    pub(super) fn say_upload_failure(&mut self, notice: Option<Notice>) {
        match notice {
            Some(notice) => {
                self.say_once(notice);
            }
            None => self.say(NoticeCode::UploadRetrying),
        }
    }
}
