//! A finalized capture is distinct from uploaded audio and a ready transcript.
use super::{Runtime, WorkResult};
use anyhow::{Context, Result, bail};
use orbit_core::{CapturePhase, Event};
use orbit_store::{Job, JobState};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[path = "finalize_schedule.rs"]
mod schedule;

#[cfg(test)]
#[path = "finalize_tests.rs"]
mod tests;

#[derive(Default, Serialize)]
struct TrackSummary {
    track: String,
    chunk_count: u64,
    last_sequence: u64,
    total_frames: u64,
}

impl Runtime {
    /// A durable closure is an irrevocable capture fence, including closures
    /// left beside an Interrupted snapshot by an older version of the daemon.
    pub(super) fn validate_recording_start(&self, decision: &orbit_core::Decision) -> Result<()> {
        for effect in &decision.effects {
            if let orbit_core::Effect::StartCapture { recording_id, .. } = effect
                && self.recording_closure(recording_id)?.is_some()
            {
                return Err(orbit_core::refusal::Refusal::InvalidParameter {
                    parameter: orbit_core::refusal::Parameter::Request,
                    expected: orbit_core::refusal::Expected::OpenRecording,
                }
                .into());
            }
        }
        Ok(())
    }

    fn recording_closure(&self, recording_id: &str) -> Result<Option<Job>> {
        let job = self.reader.get_job(&format!("finalize-{recording_id}"))?;
        if let Some(job) = &job
            && (job.kind != "recording_finalize" || job.payload["recording_id"] != recording_id)
        {
            bail!("The durable closure does not match this recording");
        }
        Ok(job)
    }

    /// Prepare the retiring identity's closure without writing it. Interrupted
    /// and drained Idle recordings both own retries until this immutable input
    /// and the replacing/removing snapshot commit in one transaction.
    pub(super) fn preserve_replaced_recording(
        &self,
        next: &orbit_core::State,
    ) -> Result<Option<Job>> {
        let Some(previous) = self.state.durable.capture.recording.as_ref() else {
            return Ok(None);
        };
        let replacing = next.durable.capture.recording.as_ref();
        if replacing.is_some_and(|recording| recording.id == previous.id) {
            return Ok(None);
        }
        let (ended_at_ms, gaps) = match (&previous.phase, previous.ended_at_ms) {
            (CapturePhase::Interrupted, Some(ended_at_ms)) => (ended_at_ms, previous.gaps),
            (CapturePhase::Interrupted, None) => (
                super::now().max(previous.started_at_ms),
                previous.gaps.saturating_add(1),
            ),
            // A successful drain already established this boundary. A failed
            // job INSERT neither changes its end time nor creates an audio gap.
            (CapturePhase::Idle, Some(ended_at_ms)) => (ended_at_ms, previous.gaps),
            _ => return Ok(None),
        };
        if self.audio.audio_is_draining()
            || self
                .capture
                .pending_seals
                .iter()
                .any(|segment| segment.metadata.recording_id == previous.id)
        {
            return Err(orbit_core::refusal::Refusal::Busy {
                operation: orbit_core::refusal::Operation::AudioDrain,
                retry_after_ms: None,
            }
            .into());
        }
        self.capture_finalize_job(&previous.id, previous.started_at_ms, ended_at_ms, gaps)
            .map(Some)
    }

    pub(super) fn empty_interrupted_closure(
        &self,
        next: &orbit_core::State,
    ) -> Result<Option<Job>> {
        let previous = self.state.durable.capture.recording.as_ref();
        let Some(previous) = previous.filter(|recording| {
            recording.phase == CapturePhase::Interrupted
                && recording.sealed_chunks == 0
                && recording.ended_at_ms.is_none()
        }) else {
            return Ok(None);
        };
        let Some(recording) = next.durable.capture.recording.as_ref().filter(|recording| {
            recording.id == previous.id && recording.phase == CapturePhase::Interrupted
        }) else {
            return Ok(None);
        };
        let Some(ended_at_ms) = recording.ended_at_ms else {
            return Ok(None);
        };
        if self.recording_closure(&previous.id)?.is_some() {
            return Ok(None);
        }
        let mut job = self.capture_finalize_job(
            &previous.id,
            previous.started_at_ms,
            ended_at_ms,
            previous.gaps,
        )?;
        let interruption = self
            .capture
            .interruption
            .as_ref()
            .context("Interrupted closure has no typed cause")?;
        job.payload["interruption"] = serde_json::to_value(interruption)?;
        Ok(Some(job))
    }

    pub(super) async fn queue_finalize(&mut self) -> Result<()> {
        let Some(recording) = self.state.durable.capture.recording.as_ref().filter(|r| {
            r.ended_at_ms.is_some()
                && matches!(r.phase, CapturePhase::Idle | CapturePhase::Interrupted)
        }) else {
            return Ok(());
        };
        let recording = recording.clone();
        self.queue_capture_finalize(
            &recording.id,
            recording.started_at_ms,
            recording.ended_at_ms.unwrap_or(recording.started_at_ms),
            recording.gaps,
        )
        .await
    }

    pub(super) async fn close_empty_interrupted(
        &mut self,
        interruption: orbit_core::RecordingInterruption,
    ) -> Result<()> {
        let recording = self.state.durable.capture.recording.as_ref();
        let Some(recording) = recording.filter(|recording| {
            recording.phase == CapturePhase::Interrupted
                && recording.ended_at_ms.is_none()
                && recording.sealed_chunks == 0
        }) else {
            return Ok(());
        };
        if self.cut_owns(&recording.id) {
            // A cut that goes on: the recording resumes, or is closed
            // with its tries (`cut_recovery`).
            return Ok(());
        }
        if self.audio.audio_is_draining()
            || self.audio.mic_stream.is_some()
            || self.audio.remote_stream.is_some()
            || !self.audio.pending_starts.is_empty()
            || !self.capture.pending_seals.is_empty()
            || self.capture.buffer.has_pending_audio()
        {
            return Ok(());
        }
        if !self.reader.list_chunks(&recording.id, None, 1)?.is_empty() {
            return Ok(());
        }
        if self.capture.interruption.is_none() {
            self.capture.interruption = Some(interruption);
        }
        let result = self.apply(Event::EmptyCaptureClosed).await;
        if result.is_ok() {
            self.capture.interruption = None;
        }
        result
    }

    pub(super) async fn queue_capture_finalize(
        &mut self,
        recording_id: &str,
        started_at_ms: i64,
        ended_at_ms: i64,
        gaps: u64,
    ) -> Result<()> {
        let job = self.capture_finalize_job(recording_id, started_at_ms, ended_at_ms, gaps)?;
        self.store_actor
            .run(move |store| Ok(store.enqueue_job(&job)?))
            .await?;
        // The local enqueue is the durable closure, including while the hub
        // is disconnected or privacy-stopped. The topic sees this generation
        // in the same actor turn after the write receipt.
        self.notes.head.touch();
        Ok(())
    }

    fn capture_finalize_job(
        &self,
        recording_id: &str,
        started_at_ms: i64,
        ended_at_ms: i64,
        gaps: u64,
    ) -> Result<Job> {
        if self
            .capture
            .pending_seals
            .iter()
            .any(|segment| segment.metadata.recording_id == recording_id)
        {
            return Err(orbit_core::refusal::Refusal::Busy {
                operation: orbit_core::refusal::Operation::AudioDrain,
                retry_after_ms: None,
            }
            .into());
        }
        let job_id = format!("finalize-{recording_id}");
        if let Some(existing) = self.recording_closure(recording_id)? {
            return Ok(existing);
        }
        let manifest = self.closure_manifest(recording_id, started_at_ms, ended_at_ms, gaps)?;
        Ok(Job {
            id: job_id,
            kind: orbit_store::QueueKind::RecordingFinalize.into(),
            state: JobState::Queued,
            created_at: ended_at_ms,
            // No op here: the store checks this closure against the
            // transcript's manifest field by field (ops stay on the
            // transcript fetch, whose notices the person reads).
            payload: json!({"recording_id":recording_id,"manifest":manifest}),
        })
    }

    /// What the closure tells the hub: the tracks the store holds, the
    /// gaps, the times, the title, what the recording is and the event it
    /// was started from, and where the microphone was muted.
    fn closure_manifest(
        &self,
        recording_id: &str,
        started_at_ms: i64,
        ended_at_ms: i64,
        gaps: u64,
    ) -> Result<Value> {
        let tracks = self.track_summaries(recording_id)?;
        let recording = self.reader.get_job(&format!("recording-{recording_id}"))?;
        let title = if let Some(recording) = self
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .filter(|recording| recording.id == recording_id)
        {
            Some(recording.title.clone())
        } else {
            recording
                .as_ref()
                .and_then(|job| job.payload["title"].as_str().map(str::to_owned))
        };
        // What the recording is and the event it was started from,
        // so the hub lists a meeting and asks for its note with the series'
        // template, and leaves a voice call out (`RecordingManifest`).
        let kind = match recording
            .as_ref()
            .and_then(|job| job.payload["recording_kind"].as_str())
        {
            Some("voice_call") => "voice_call",
            _ => "meeting",
        };
        let event = recording
            .as_ref()
            .filter(|_| kind == "meeting")
            .and_then(|job| job.payload.get("event"))
            .and_then(|event| {
                serde_json::from_value::<orbit_core::call_detection::LinkedEvent>(event.clone())
                    .ok()
            })
            .map(|linked| orbit_core::notes::event_context(&linked));
        let mut manifest = json!({"tracks":tracks.into_values().collect::<Vec<_>>(),"gaps":gaps,
            "started_at_ms":started_at_ms,"finalized_at_ms":ended_at_ms,"title":title,
            "kind":kind});
        if let Some(event) = event {
            manifest["event"] = serde_json::to_value(event)?;
        }
        // Where the microphone was muted; a hub that does not know it ignores it.
        if let Some(spans) =
            self.closure_muted_spans(recording_id, ended_at_ms.saturating_sub(started_at_ms))
        {
            manifest["muted_spans"] = serde_json::to_value(spans)?;
        }
        Ok(manifest)
    }

    /// What the store holds of each track of `recording_id`.
    fn track_summaries(&self, recording_id: &str) -> Result<BTreeMap<String, TrackSummary>> {
        let mut tracks: BTreeMap<String, TrackSummary> = BTreeMap::new();
        let mut cursor = None;
        loop {
            let page = self
                .reader
                .list_chunks(recording_id, cursor.as_deref(), 100)?;
            if page.is_empty() {
                break;
            }
            for chunk in &page {
                let summary = tracks
                    .entry(chunk.metadata.track.clone())
                    .or_insert_with(|| TrackSummary {
                        track: chunk.metadata.track.clone(),
                        ..TrackSummary::default()
                    });
                summary.chunk_count += 1;
                summary.last_sequence = summary.last_sequence.max(chunk.metadata.sequence);
                summary.total_frames = summary
                    .total_frames
                    .checked_add(chunk.metadata.frames)
                    .context("Audio counter outside bounds")?;
            }
            cursor = page.last().map(|chunk| chunk.metadata.id.clone());
        }
        Ok(tracks)
    }
}

/// The closure the hub can check against what it holds: each track counts
/// only the chunks it received and names, in `missing_sequences`, every chunk
/// the hub refused for good, wherever it is; each is one more gap. The hub
/// then holds every other sequence up to the track's last one
/// (`contracts/hub-v1.md`), transcribes around the holes and says them. A
/// track that lost every chunk is left out. More holes than the hub admits
/// per track are refused by it as `invalid_manifest`, which is said.
pub(super) fn declare_refused_chunks(
    manifest: &mut Value,
    refused: &[orbit_store::ChunkMetadata],
) -> Result<()> {
    let tracks = manifest["tracks"]
        .as_array_mut()
        .context("Closure has no track summary")?;
    for track in tracks.iter_mut() {
        let name = track["track"].as_str().unwrap_or_default().to_owned();
        let mut missing: Vec<u64> = refused
            .iter()
            .filter(|chunk| chunk.track == name)
            .map(|chunk| chunk.sequence)
            .collect();
        if missing.is_empty() {
            continue;
        }
        missing.sort_unstable();
        missing.dedup();
        let lost_frames = refused
            .iter()
            .filter(|chunk| chunk.track == name)
            .map(|chunk| chunk.frames)
            .sum::<u64>();
        let count = track["chunk_count"].as_u64().unwrap_or_default();
        let frames = track["total_frames"].as_u64().unwrap_or_default();
        track["chunk_count"] = json!(count.saturating_sub(missing.len() as u64));
        track["total_frames"] = json!(frames.saturating_sub(lost_frames));
        track["missing_sequences"] = json!(missing);
    }
    tracks.retain(|track| track["chunk_count"].as_u64().is_some_and(|count| count > 0));
    let gaps = manifest["gaps"].as_u64().unwrap_or_default();
    manifest["gaps"] = json!(gaps.saturating_add(refused.len() as u64));
    Ok(())
}
