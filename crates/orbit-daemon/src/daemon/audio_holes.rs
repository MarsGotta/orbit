//! Damaged audio is a hole, never a capture stop. Both ingest lanes report
//! through this boundary; consecutive failures are summarized on recovery.
use super::*;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey};

impl Runtime {
    pub(super) fn note_audio_hole(&mut self, source: &str, cause: &str) -> Result<()> {
        self.note_bound_audio_gap(source)?;
        self.log(
            "audio_gap",
            &[
                ("source", source),
                ("cause", cause),
                ("duration_unknown", "true"),
            ],
        );
        self.say(
            Notice::new(NoticeCode::AudioGap)
                .with_token(NoticeKey::Origin, source)
                .with_token(NoticeKey::Cause, cause),
        );
        Ok(())
    }

    pub(super) fn drop_audio_event(&mut self, source: Option<&str>, error: &str) {
        // Only two known sources can own recording audio. Never retain an
        // external source name (or one entry per frame) in the failure tally.
        let source = source.filter(|s| matches!(*s, "mic_local" | "remote_app"));
        if let Some(source) = source {
            // Seal the valid tail before the dropped frame. If sealing itself
            // fails, retain it for the existing durable seal retry path.
            if let Ok(segments) = self.capture.buffer.restart_source(source) {
                let _ = self.seal(segments);
            }
            let _ = self.note_bound_audio_gap(source);
        } else if let Some(recording) = self.state.durable.capture.recording.as_mut() {
            recording.gaps = recording.gaps.saturating_add(1);
            let _ = self.store_actor.defer_snapshot(&self.state);
        }
        if self
            .capture
            .frame_failures
            .as_ref()
            .is_some_and(|(cause, _)| cause != error)
        {
            self.finish_audio_failures();
        }
        let failure = self
            .capture
            .frame_failures
            .get_or_insert_with(|| (error.to_owned(), 0));
        failure.1 = failure.1.saturating_add(1);
    }

    /// One diagnostic per failed burst, carrying its complete dropped count.
    pub(super) fn finish_audio_failures(&mut self) {
        if self.capture.seal_failure_active && self.recording_active() {
            return;
        }
        if let Some((cause, count)) = self.capture.frame_failures.take() {
            self.log(
                "audio_frames_dropped",
                &[("cause", &cause), ("count", &count.to_string())],
            );
        }
    }
}
