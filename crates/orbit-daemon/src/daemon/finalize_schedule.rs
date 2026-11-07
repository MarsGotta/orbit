//! The closures' turn to go up: the next one ready, the chunks the hub refused
//! for good declared in it, and the upload off the loop.
use super::*;

/// A closure ready to go up: its job, its recording and the manifest the
/// hub receives, with the chunks it refused for good declared.
pub(in crate::daemon) struct Closure {
    pub(in crate::daemon) job: Job,
    pub(in crate::daemon) recording_id: String,
    pub(in crate::daemon) manifest: Value,
}

impl Runtime {
    pub(in crate::daemon) async fn schedule_finalize(&mut self) -> Result<bool> {
        let Some(closure) = self.next_closure()? else {
            return Ok(false);
        };
        let token = self.link.token.clone().context("No credential")?;
        let base = crate::config::validate_hub_url(
            self.settings.hub_url.as_deref().context("No hub")?,
            self.development,
        )?;
        self.spawn_finalize(closure, token, base).await?;
        Ok(true)
    }

    /// The next closure ready to go, with the chunks the hub refused for
    /// good declared in its manifest.
    pub(in crate::daemon) fn next_closure(&mut self) -> Result<Option<Closure>> {
        let Some(job) = self.next_finalize_candidate()? else {
            return Ok(None);
        };
        let recording_id = job.payload["recording_id"]
            .as_str()
            .context("Manifest has no recording_id")?
            .to_owned();
        let refused = self.refused_chunks_of(&recording_id)?;
        let mut manifest = job.payload["manifest"].clone();
        if !refused.is_empty() {
            declare_refused_chunks(&mut manifest, &refused)?;
            self.log(
                "closure_declares_refused_chunks",
                &[("chunks", &refused.len().to_string())],
            );
        }
        Ok(Some(Closure {
            job,
            recording_id,
            manifest,
        }))
    }

    /// The next closure ready to go. Bound candidate work and advance a
    /// cursor: an earlier blocked recording must not conceal later eligible
    /// closures indefinitely.
    pub(super) fn next_finalize_candidate(&mut self) -> Result<Option<Job>> {
        let mut jobs = self.reader.pending_jobs_of_kind(
            "recording_finalize",
            self.transfers.finalize_cursor.as_deref(),
            16,
        )?;
        if jobs.is_empty() {
            self.transfers.finalize_cursor = None;
            jobs = self
                .reader
                .pending_jobs_of_kind("recording_finalize", None, 16)?;
        }
        let mut candidate = None;
        for job in jobs {
            self.transfers.finalize_cursor = Some(job.id.clone());
            if self.finalize_is_ready(&job)? {
                candidate = Some(job);
                break;
            }
        }
        Ok(candidate)
    }

    /// Chunks the hub refused for good are declared, not waited for.
    pub(super) fn refused_chunks_of(
        &self,
        recording_id: &str,
    ) -> Result<Vec<orbit_store::ChunkMetadata>> {
        let mut refused = Vec::new();
        loop {
            let page = self.reader.refused_chunks(
                recording_id,
                refused
                    .last()
                    .map(|chunk: &orbit_store::ChunkMetadata| chunk.id.as_str()),
                orbit_store::MAX_PAGE_SIZE,
            )?;
            if page.is_empty() {
                break;
            }
            refused.extend(page);
        }
        Ok(refused)
    }

    /// The closure goes up off the loop; its result comes back as work.
    pub(super) async fn spawn_finalize(
        &mut self,
        closure: Closure,
        token: String,
        base: reqwest::Url,
    ) -> Result<()> {
        let generation = self.state.generation();
        let events = self.work_events.clone();
        let job_id = closure.job.id.clone();
        let job_op = super::super::turn::job_op(&closure.job.payload)
            .unwrap_or_else(|| self.turn.op.clone());
        self.store_actor
            .run(move |store| Ok(store.update_job(&job_id, JobState::Running)?))
            .await?;
        self.transfers
            .worker
            .start_logical(generation, job_op, (), move |ticket| {
                tokio::spawn(async move {
                    let result = crate::hub::post_closure(
                        &base,
                        &token,
                        &closure.recording_id,
                        &closure.manifest,
                        ticket.op(),
                    )
                    .await;
                    crate::daemon::deliver_work(
                        &events,
                        WorkResult::Store(crate::daemon::StoreWork::Uploaded {
                            ticket,
                            job_id: closure.job.id,
                            result,
                        }),
                    )
                    .await;
                })
            })
            .context("Could not reserve recording closure")?;
        Ok(())
    }

    pub(super) fn finalize_is_ready(&mut self, job: &Job) -> Result<bool> {
        let recording = job.payload["recording_id"]
            .as_str()
            .context("Closure has no identity")?;
        if self
            .capture
            .pending_seals
            .iter()
            .any(|segment| segment.metadata.recording_id == recording)
        {
            return Ok(false);
        }
        if let Some(ready) = self.transfers.finalize_ready.get(recording) {
            return Ok(*ready);
        }
        let expected = job.payload["manifest"]["tracks"]
            .as_array()
            .context("Closure has no track summary")?
            .iter()
            .try_fold(0u64, |total, track| {
                total
                    .checked_add(
                        track["chunk_count"]
                            .as_u64()
                            .context("Track summary has no chunks")?,
                    )
                    .context("Contador de fragmentos agotado")
            })?;
        let (chunks, uploaded) = self.reader.finalize_counts(recording)?;
        // A chunk the hub refused for good is settled too: the closure
        // declares it lost instead of waiting for it for ever.
        let refused = self.reader.refused_chunk_count(recording)?;
        let ready = chunks == expected && uploaded.saturating_add(refused) == expected;
        self.transfers
            .finalize_ready
            .insert(recording.to_owned(), ready);
        Ok(ready)
    }
}
