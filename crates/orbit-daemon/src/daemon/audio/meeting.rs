//! The meeting recording's audio: it owns the microphone while it records,
//! and its system's audio track when it has one. A voice note over it
//! consumes the same microphone stream; the meeting's capture never depends
//! on how that note ends.
use super::super::{Runtime, id, now};
use super::drain::FullStop;
use super::microphone::Holder;
use anyhow::Result;
use orbit_core::notice::NoticeCode;
use orbit_core::{CapturePhase, Event, decide, protocol::MeetingAudio};
use orbit_store::{Job, JobState};
use serde_json::json;

const MICROPHONE_SOURCE: &str = "mic_local";

/// The meeting recording's own state.
#[derive(Default)]
pub(in crate::daemon) struct RecordingAudio {
    /// What the recording's microphone and system's audio carry.
    pub(in crate::daemon) signal: Option<super::super::track_signal::MeetingSignal>,
    /// The recording's end once its streams drain: `Some(true)` to finish
    /// it, `Some(false)` to pause it.
    pub(in crate::daemon) recording_end: Option<bool>,
}

impl RecordingAudio {
    /// Whether a finished recording is still waiting to be closed.
    ///
    /// `complete_audio_drain` keeps this when it finds a seal still in
    /// flight, because a recording cannot be declared finalized while a chunk
    /// of it is unwritten. It is the one flag that says "the Mac has
    /// confirmed, the audio is in, and all that is left is to say so", and it
    /// is what the daemon has to come back to once the seals land.
    pub(super) fn finish_owed(&self) -> bool {
        self.recording_end.is_some()
    }
}

impl Runtime {
    pub(in crate::daemon) async fn start_recording_audio(
        &mut self,
        recording_id: String,
        audio: MeetingAudio,
    ) -> Result<()> {
        let epoch = id();
        self.capture
            .buffer
            .begin_recording_with_epoch(recording_id.clone(), epoch.clone())?;
        // Reserve exhaustion belongs to the recording whose unsealed PCM
        // overflowed. A later capture starts with a fresh 64 MiB reserve.
        if self.capture.pending_seals.is_empty() && !self.capture.buffer.has_pending_audio() {
            self.capture.reserve_exhausted = false;
        }
        for track in ["mic_local", "remote_app"] {
            if let Some(receipt) = self.reader.latest_chunk_receipt(&recording_id, track)? {
                self.capture.buffer.restore_position(&receipt.metadata)?;
            }
        }
        self.queue_capture_epoch(&recording_id, &epoch, audio)
            .await?;
        // The meeting takes the microphone: a voice note already dictating
        // keeps consuming the same stream.
        self.take_microphone(Some(audio)).await?;
        if !matches!(audio, MeetingAudio::MicrophoneOnly) {
            let stream = id();
            self.start_track("remote_app", &stream, Some(audio)).await?;
            self.capture.remote_stream = Some(stream);
        }
        self.begin_recording_signal(recording_id, epoch);
        self.capture_started_if_ready().await?;
        // «Oye Deimos» goes with the meeting, on its microphone
        // stream, whoever opened it.
        self.sync_wake_phrase().await?;
        Ok(())
    }

    /// The capture epoch the hub joins this recording's tracks by.
    async fn queue_capture_epoch(
        &mut self,
        recording_id: &str,
        epoch: &str,
        audio: MeetingAudio,
    ) -> Result<()> {
        let (expected_tracks, capture_scope, application_pid) = match audio {
            MeetingAudio::System => (vec!["mic_local", "remote_app"], Some("system"), None),
            MeetingAudio::Application { application_pid } => (
                vec!["mic_local", "remote_app"],
                Some("application"),
                Some(application_pid),
            ),
            MeetingAudio::MicrophoneOnly => (vec![MICROPHONE_SOURCE], None, None),
        };
        let mut payload = json!({"recording_id":recording_id,
            "capture_epoch_id":epoch,"expected_tracks":expected_tracks,"clock_domain":"mach_absolute_time",
            "origin":self.capture_origin(recording_id)});
        if let Some(fields) = payload.as_object_mut() {
            if let Some(scope) = capture_scope {
                fields.insert("capture_scope".into(), json!(scope));
            }
            if let Some(pid) = application_pid {
                fields.insert("application_pid".into(), json!(pid));
            }
        }
        let job = Job {
            id: format!("capture-epoch-{epoch}"),
            kind: orbit_store::QueueKind::CaptureEpoch.into(),
            state: JobState::Succeeded,
            created_at: now(),
            payload,
        };
        self.store_actor
            .run(move |store| Ok(store.enqueue_job(&job)?))
            .await?;
        Ok(())
    }

    pub(in crate::daemon) async fn stop_recording_audio(&mut self) -> Result<()> {
        // Pausing or finishing turns «Oye Deimos» off first, also
        // when a voice note keeps the stream open.
        self.sync_wake_phrase().await?;
        if self.state.privacy_stopped() {
            // The user's stop latches the helper; a revocation stops it
            // without writing a latch nobody pressed.
            let kind = if orbit_core::privacy_stop::latches_helper(&self.state) {
                FullStop::Privacy
            } else {
                FullStop::Cut
            };
            return self.request_full_audio_stop(kind).await;
        }
        let end = self
            .state
            .durable
            .recording
            .as_ref()
            .is_none_or(|recording| recording.phase != CapturePhase::Paused);
        self.capture.audio.meeting.recording_end = Some(end);
        if let Some(stream) = self.capture.remote_stream.clone() {
            self.request_stream_stop("remote_app", stream).await?;
        } else {
            let segments = self.capture.buffer.pause_source("remote_app")?;
            self.seal(segments)?;
        }
        let kept = self.release_microphone(Holder::Meeting).await?;
        if kept || self.capture.mic_stream.is_none() {
            // The native microphone stays live for the voice note (or there
            // is none). Recording stops at this explicit local boundary and
            // must not resume from later PCM.
            let segments = self.capture.buffer.pause_source("mic_local")?;
            self.seal(segments)?;
        }
        self.complete_audio_drain().await
    }

    pub(in crate::daemon) async fn capture_started_if_ready(&mut self) -> Result<()> {
        if self.capture.pending_starts.is_empty()
            && self
                .state
                .durable
                .recording
                .as_ref()
                .is_some_and(|r| r.phase == CapturePhase::Starting)
        {
            let decision = decide(&self.state, Event::CaptureStarted, now())?;
            self.store_actor.snapshot(&decision.state).await?;
            self.state = decision.state;
        }
        Ok(())
    }

    /// Close a recording whose finish was deferred because a seal was still
    /// in flight, once that seal has landed.
    ///
    /// The last thing `complete_audio_drain` does for a finished
    /// meeting is ask `decide` for `CaptureFinalized`, which is what takes the
    /// recording out of `Finalizing`. It refuses to do that while a chunk is
    /// still being written — rightly, because a recording is not finalized
    /// while part of it is unwritten — and stores the intent to come back.
    ///
    /// Coming back is what went missing. Two paths observe a seal landing: the
    /// store actor's completion queue, drained by `collect_ready_seals` from
    /// `seal` and from the tick, and the work event `StoreWork::Sealed`. Only
    /// the second re-drove the drain, and only inside `if let Some(id)` — that
    /// is, only when the segment was *still* pending by the time the work event
    /// was handled. Under load the collector usually gets there first, the work
    /// event then finds nothing to apply, and the deferred finish is dropped on
    /// the floor. The recording sits in `Finalizing` until
    /// `FINALIZING_TIMEOUT_MS` interrupts it fifteen seconds later and tells
    /// the person the Mac never confirmed the close — when the Mac had
    /// confirmed it, and the daemon had recorded that it did.
    ///
    /// Measured at four runs in twenty under load before this existed.
    ///
    /// So both observers end here instead, and the tick asks the same question
    /// once a second, because the condition — a finish is waiting and no seal
    /// is left — is cheap to read and does not depend on which path noticed.
    pub(in crate::daemon) async fn resume_deferred_drain(&mut self) -> Result<()> {
        if self.capture.audio.meeting.finish_owed()
            && !self.capture.audio.pending()
            && self.capture.pending_seals.is_empty()
        {
            self.complete_audio_drain().await?;
        }
        Ok(())
    }

    /// After the drain: a recording whose end was asked for is sealed and,
    /// once finished, finalized. False while one of its seals is still in
    /// flight; `resume_deferred_drain` comes back to it.
    pub(super) async fn close_drained_recording(&mut self) -> Result<bool> {
        let Some(end) = self.capture.audio.meeting.recording_end else {
            return Ok(true);
        };
        let segments = self.capture.buffer.flush(end);
        if let Err(error) = self.seal(segments) {
            self.say_failure(NoticeCode::RecordingSealFailed, &error);
            return Err(error);
        }
        if !self.capture.pending_seals.is_empty() {
            return Ok(false);
        }
        self.capture.audio.meeting.recording_end = None;
        if end
            && self
                .state
                .durable
                .recording
                .as_ref()
                .is_some_and(|r| r.phase == CapturePhase::Finalizing)
        {
            let decision = decide(&self.state, Event::CaptureFinalized, now())?;
            self.store_actor.snapshot(&decision.state).await?;
            self.state = decision.state;
            self.queue_finalize().await?;
        }
        Ok(true)
    }

    /// Attribute a safety fact to the actual producer owner, not the UI phase.
    /// Keep it in RAM even if persisting the snapshot fails; pretending no gap
    /// would be less safe than retaining this conservative monotonic counter.
    pub(in crate::daemon) fn note_bound_audio_gap(&mut self, source: &str) -> Result<()> {
        let Some(owner) = self
            .capture
            .buffer
            .recording_owner(source)
            .map(str::to_owned)
        else {
            return Ok(());
        };
        self.note_call_gap(&owner);
        if let Some(recording) = self
            .state
            .durable
            .recording
            .as_mut()
            .filter(|recording| recording.id == owner)
        {
            recording.gaps = recording.gaps.saturating_add(1);
            if let Some(track) = recording
                .tracks
                .iter_mut()
                .find(|track| track.name == source)
            {
                track.gaps = track.gaps.saturating_add(1);
            }
            self.store_actor.defer_snapshot(&self.state)?;
        }
        Ok(())
    }

    pub(in crate::daemon) fn note_bound_track_gap(&mut self, source: &str) -> Result<()> {
        let Some(owner) = self
            .capture
            .buffer
            .recording_owner(source)
            .map(str::to_owned)
        else {
            return Ok(());
        };
        let Some(track) = self
            .state
            .durable
            .recording
            .as_mut()
            .filter(|recording| recording.id == owner)
            .and_then(|recording| {
                recording
                    .tracks
                    .iter_mut()
                    .find(|track| track.name == source)
            })
        else {
            return Ok(());
        };
        track.gaps = track.gaps.saturating_add(1);
        self.store_actor.defer_snapshot(&self.state)?;
        Ok(())
    }
}
