//! The capture's own durable record: the chunk receipts it sealed.
//!
//! Every seal writes its chunk and its receipt in one store transaction, so
//! what was recorded is known from the store alone, whatever the whole-state
//! snapshot managed to save before Orbit stopped. A recording's counters in
//! that snapshot are a projection of this record. At start they are rebuilt
//! from it, so a recording an older version left cut with counts it never
//! saved (or saved as zero) is still closed at the end of its audio, and the
//! hub writes its note.
use super::*;
use std::collections::BTreeMap;

/// Receipts read per page while rebuilding a recording's counts at start:
/// the store's own page bound for chunk listings elsewhere (`finalize.rs`).
/// It bounds one read's memory, not how many chunks a recording may have.
const RECORD_PAGE: usize = 100;

/// What the store holds for one track of a recording.
#[derive(Default)]
struct TrackRecord {
    sealed_chunks: u64,
    last_chunk_at_ms: i64,
}

impl Runtime {
    /// Brings the saved recording's counters up to its durable record: no
    /// count is lowered, and a track the snapshot did not list is added.
    pub(super) fn restore_capture_record(&mut self) -> Result<()> {
        let Some(id) = self
            .state
            .durable
            .recording
            .as_ref()
            .map(|recording| recording.id.clone())
        else {
            return Ok(());
        };
        let tracks = self.chunk_record(&id)?;
        let Some(recording) = self.state.durable.recording.as_mut() else {
            return Ok(());
        };
        let sealed: u64 = tracks.values().map(|track| track.sealed_chunks).sum();
        recording.sealed_chunks = recording.sealed_chunks.max(sealed);
        for (name, record) in tracks {
            match recording.tracks.iter_mut().find(|track| track.name == name) {
                Some(track) => {
                    track.sealed_chunks = track.sealed_chunks.max(record.sealed_chunks);
                    track.last_chunk_at_ms = track.last_chunk_at_ms.max(record.last_chunk_at_ms);
                }
                None => recording.tracks.push(orbit_core::TrackProgress {
                    name,
                    sealed_chunks: record.sealed_chunks,
                    gaps: 0,
                    last_chunk_at_ms: record.last_chunk_at_ms,
                }),
            }
        }
        Ok(())
    }

    /// Every chunk receipt of `recording_id`, uploaded or not, by track.
    fn chunk_record(&self, recording_id: &str) -> Result<BTreeMap<String, TrackRecord>> {
        let mut tracks = BTreeMap::<String, TrackRecord>::new();
        let mut cursor: Option<String> = None;
        loop {
            let page = self
                .reader
                .list_chunks(recording_id, cursor.as_deref(), RECORD_PAGE)?;
            let Some(last) = page.last() else {
                return Ok(tracks);
            };
            cursor = Some(last.metadata.id.clone());
            for receipt in &page {
                let track = tracks.entry(receipt.metadata.track.clone()).or_default();
                track.sealed_chunks = track.sealed_chunks.saturating_add(1);
                track.last_chunk_at_ms = track.last_chunk_at_ms.max(receipt.metadata.started_at_ms);
            }
            if page.len() < RECORD_PAGE {
                return Ok(tracks);
            }
        }
    }

    /// The saved recording as this start finds it: its counts rebuilt from
    /// what it sealed, an empty interrupted one closed empty, and one an
    /// earlier run left cut that will not go on closed at its last audio.
    /// A capture this restart cut and that may resume waits for the helper.
    pub(super) async fn recover_recording_at_start(
        &mut self,
        empty: orbit_core::RecordingInterruption,
    ) -> Result<()> {
        self.restore_capture_record()?;
        self.close_empty_interrupted(empty).await?;
        Box::pin(self.close_cut_recording_at_start()).await
    }
}
