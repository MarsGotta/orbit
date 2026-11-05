//! Orbit closing delivers a meeting it cut before it leaves.
//!
//! Closing retires the network at once: the core takes the link offline and
//! the uploads are cancelled before the helper has even drained the last
//! audio (`orbit_core::transition::stops::close`), and the loop leaves as
//! soon as nothing is left to store. A meeting recorded up to ⌘Q kept its
//! last chunks and its closure on this Mac until the next start, so the hub
//! knew nothing of its end while Orbit was closed (the real test's check 23
//! never passed).
//!
//! Once everything is stored, and only then, the meeting the quit cut is
//! closed at the moment of the quit, and what the queue owes the hub goes up
//! in order: the chunks, then each closure they make ready. One request at a
//! time and only within what is left of the one closing budget
//! ([`super::SHUTDOWN_BUDGET`]); the first failure, or the deadline, leaves
//! the rest queued for the next start, as before. The retry gate's backoff
//! is not waited for: one bounded attempt per job is not a retry storm, and a
//! hub that failed a moment ago may answer now. A privacy stop sends nothing,
//! and neither does a Mac without a hub or whose hub link was offline at the
//! quit.
use super::upload_outcome::{self, ChunkPut};
use super::*;
use std::collections::BTreeSet;
use std::time::Instant;

/// Chunk jobs read per page. A read size, not a limit on what goes up.
const PAGE: usize = 16;

/// What the way out sent, for its one log line: counts only.
#[derive(Default)]
struct Sent {
    chunks: usize,
    closures: usize,
}

impl Runtime {
    /// Called by the loop once nothing is left to store, with the closing's
    /// deadline. It never outlives that deadline and never fails the exit:
    /// what it could not send stays queued, and the line in `daemon.log`
    /// says what went up and why it stopped.
    pub(super) async fn deliver_before_exit(&mut self, deadline: Instant) {
        if self.state.privacy_stopped() {
            return;
        }
        let mut sent = Sent::default();
        let outcome = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            Box::pin(self.deliver_owed(&mut sent)),
        )
        .await;
        let (outcome, error) = match outcome {
            Ok(Ok(outcome)) => (outcome, None),
            Ok(Err(error)) => ("failed", Some(format!("{error:#}"))),
            Err(_) => ("deadline", None),
        };
        let (chunks, closures) = (sent.chunks.to_string(), sent.closures.to_string());
        let mut fields = vec![
            ("outcome", outcome),
            ("chunks", chunks.as_str()),
            ("closures", closures.as_str()),
        ];
        if let Some(error) = &error {
            fields.push(("error", error.as_str()));
        }
        self.log("exit_delivery", &fields);
    }

    /// Closes the cut meeting, then sends what the queue owes. Returns why
    /// it ended when nothing failed.
    async fn deliver_owed(&mut self, sent: &mut Sent) -> Result<&'static str> {
        Box::pin(self.close_unresumable_cut_recording()).await?;
        self.queue_finalize().await?;
        // Closing took the link offline; whether the hub answered before is
        // what it knew. A hub out of reach would hold the quit for the rest
        // of the budget (its connect timeout alone is ten seconds).
        if !self.online_at_close {
            return Ok("hub_offline");
        }
        let (Some(token), Some(hub)) = (self.link.token.clone(), self.settings.hub_url.clone())
        else {
            return Ok("no_hub");
        };
        let base = crate::config::validate_hub_url(&hub, self.development)?;
        let mut tried = BTreeSet::new();
        while let Some(job) = self.next_exit_chunk(&mut tried)? {
            self.send_exit_chunk(job, &base, &token).await?;
            sent.chunks += 1;
        }
        // Chunks settled here, not through the loop: their closures'
        // readiness is read again.
        self.transfers.finalize_ready.clear();
        while let Some(closure) = self.next_closure()? {
            let job_id = closure.job.id.clone();
            self.mark_running(&job_id).await?;
            let result =
                super::finalize::post_closure(&base, &token, &closure, &orbit_ipc::new_op()).await;
            self.settle_exit_transfer(job_id, result).await?;
            sent.closures += 1;
        }
        Ok("delivered")
    }

    /// The next chunk job not tried on the way out, skipping those of a
    /// meeting thrown away or not allowed up: the loop decides those.
    fn next_exit_chunk(&self, tried: &mut BTreeSet<String>) -> Result<Option<Job>> {
        let discarded = self.notes.outbox.discarded();
        for job in self.reader.pending_chunk_uploads(None, PAGE)? {
            if !tried.insert(job.id.clone()) {
                continue;
            }
            let recording = job.payload["metadata"]["recording_id"]
                .as_str()
                .context("Chunk job has no recording")?;
            let forbidden = self
                .reader
                .get_job(&format!("recording-{recording}"))?
                .is_some_and(|manifest| manifest.payload["upload_allowed"] == false);
            if !discarded.contains(recording) && !forbidden {
                return Ok(Some(job));
            }
        }
        Ok(None)
    }

    async fn send_exit_chunk(&mut self, job: Job, base: &reqwest::Url, token: &str) -> Result<()> {
        let chunk = job.payload["metadata"]["id"]
            .as_str()
            .context("Job has no chunk_id")?
            .to_owned();
        let receipt = self
            .reader
            .chunk_receipt(&chunk)?
            .context("Pending chunk has no local receipt")?;
        let bytes = self.reader.read_chunk(&chunk)?;
        self.mark_running(&job.id).await?;
        let put = ChunkPut {
            base: base.clone(),
            token: token.to_owned(),
            receipt,
            bytes,
        };
        let result = upload_outcome::put_chunk(put, &orbit_ipc::new_op()).await;
        self.settle_exit_transfer(job.id, result).await
    }

    async fn mark_running(&mut self, job_id: &str) -> Result<()> {
        let job_id = job_id.to_owned();
        self.store_actor
            .run(move |store| Ok(store.update_job(&job_id, JobState::Running)?))
            .await
    }

    /// The same settlement as every transfer (`upload_outcome`), and the
    /// transfer's error, which ends the way out.
    async fn settle_exit_transfer(&mut self, job_id: String, result: Result<()>) -> Result<()> {
        let stored = self
            .reader
            .get_job(&job_id)?
            .context("Receipt has no local job")?;
        let settlement = upload_outcome::settle(&stored, &result);
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
        result
    }
}

#[cfg(test)]
#[path = "exit_delivery_tests.rs"]
mod tests;
