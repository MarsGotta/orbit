//! Startup reconciliation of continuous originals, separate from call authority.
//! A manifest is durable before audio_start. Recovering it may queue closure,
//! but must never reconstruct a detector, microphone producer or derived prompt.
//!
//! Its errors are in English: they abort startup, and their text only reaches
//! the app's stderr (`daemon_thread.rs`, «terminó con un error: …») and the
//! development binary's. Neither the person nor the assistant reads them: the
//! app quits with the daemon, and the error reports carry codes, never text.
use super::Runtime;
use anyhow::{Context, Result, bail};
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};
use orbit_store::{Job, JobState};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct CallManifest {
    recording_id: String,
    started_at_ms: i64,
    expires_at_ms: i64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecoveryBoundary {
    recording_id: String,
    manifest_job_id: String,
    started_at_ms: i64,
    /// Recovery observation, NOT a measured last-sample time or a stop ACK.
    recovered_at_ms: i64,
    reason: String,
}

const REASON: &str = "restart_with_unconfirmed_tail";

impl Runtime {
    /// Startup only, after the entire encrypted-blob recovery scan. Scan every
    /// page, including completed manifests older than the presentation snapshot.
    /// A failure aborts startup; a later startup reconciles immutable boundaries.
    pub(super) async fn recover_call_closures(&mut self, recovered_at_ms: i64) -> Result<usize> {
        if self.state.durable.voice.call.is_some()
            || self.audio.call.is_some()
            || self.audio.mic_stream.is_some()
            || self.audio.remote_stream.is_some()
            || self.audio.audio_is_draining()
            || !self.audio.pending_starts.is_empty()
            || !self.capture.pending_seals.is_empty()
            || self.capture.buffer.has_pending_audio()
        {
            bail!("call recovery needs capture revoked and audio drained");
        }
        let mut cursor = None;
        let mut recovered = 0;
        loop {
            let page = self.reader.list_jobs_after(cursor.as_deref(), 100)?;
            if page.is_empty() {
                if recovered > 0 {
                    self.say(
                        Notice::new(NoticeCode::CallsRecovered).with(NoticeKey::Count, recovered),
                    );
                    self.store_actor.snapshot(&self.state).await?;
                }
                return Ok(recovered);
            }
            for job in &page {
                if job.kind == "recording_manifest" && job.payload["recording_kind"] == "voice_call"
                {
                    // An empty first-install store needs no recovery. Any prior
                    // call, however, must be reconciled before anything can
                    // capture: at startup, before the helper is spawned. This
                    // used to lean on the stop every restart invented; now a
                    // restart invents none, and the guarantee is the helper's
                    // absence itself.
                    if self.state.capabilities().native_helper()
                        || self.state.durable.voice.voice_active
                    {
                        bail!(
                            "call recovery runs at startup, before the helper connects; the helper or voice is active now"
                        );
                    }
                    recovered +=
                        usize::from(self.recover_call_closure(job, recovered_at_ms).await?);
                }
            }
            cursor = page.last().map(|job| job.id.clone());
            tokio::task::yield_now().await;
        }
    }

    async fn recover_call_closure(&mut self, job: &Job, recovered_at_ms: i64) -> Result<bool> {
        let manifest: CallManifest = serde_json::from_value(job.payload.clone())?;
        if job.id != format!("recording-{}", manifest.recording_id)
            || job.state != JobState::Succeeded
            || manifest.started_at_ms < 0
            || manifest.expires_at_ms < manifest.started_at_ms
        {
            bail!("inconsistent call manifest; kept without reactivating capture");
        }
        if let Some(closure) = self
            .reader
            .get_job(&format!("finalize-{}", manifest.recording_id))?
        {
            if closure.kind != "recording_finalize"
                || closure.payload["recording_id"] != manifest.recording_id
            {
                bail!("conflicting call closure identity");
            }
            // Includes running, unknown, blocked and completed closures. Never
            // rewrite a manifest that may already have reached the server.
            return Ok(false);
        }
        let boundary_id = format!("call-recovery-{}", manifest.recording_id);
        let boundary = if let Some(existing) = self.reader.get_job(&boundary_id)? {
            let boundary: RecoveryBoundary = serde_json::from_value(existing.payload)?;
            if existing.kind != "call_recovery_boundary"
                || existing.state != JobState::Succeeded
                || existing.created_at != boundary.recovered_at_ms
                || boundary.recording_id != manifest.recording_id
                || boundary.manifest_job_id != job.id
                || boundary.started_at_ms != manifest.started_at_ms
                || boundary.recovered_at_ms < manifest.started_at_ms
                || boundary.reason != REASON
            {
                bail!("conflicting call recovery boundary");
            }
            boundary
        } else {
            let boundary = RecoveryBoundary {
                recording_id: manifest.recording_id.clone(),
                manifest_job_id: job.id.clone(),
                started_at_ms: manifest.started_at_ms,
                recovered_at_ms: recovered_at_ms.max(manifest.started_at_ms),
                reason: REASON.into(),
            };
            let job = Job {
                id: boundary_id,
                kind: orbit_store::QueueKind::CallRecoveryBoundary.into(),
                state: JobState::Succeeded,
                created_at: boundary.recovered_at_ms,
                payload: serde_json::to_value(&boundary)
                    .context("invalid call recovery boundary")?,
            };
            self.store_actor
                .run(move |store| Ok(store.enqueue_job(&job)?))
                .await?;
            boundary
        };
        // At least one discontinuity: the original's unsealed tail and exact
        // stop are unknown. This does not invent silence or replay derived STT.
        self.queue_capture_finalize(
            &manifest.recording_id,
            manifest.started_at_ms,
            boundary.recovered_at_ms,
            1,
        )
        .await?;
        Ok(true)
    }
}
