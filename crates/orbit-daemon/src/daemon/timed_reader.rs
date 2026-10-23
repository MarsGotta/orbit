//! The loop's SQLite reader, each read timed and named by its call site.
//!
//! These reads run synchronously on the loop's thread, and the connection
//! waits up to its 2 s `busy_timeout` when the database is busy
//! (`orbit-store/src/reader.rs`), so any of them can hold every domain. Each
//! one reports how long it took to the turn in progress (`turn_watch`); a
//! slow turn then names the read as `read@<file>:<line>`. The methods mirror
//! the [`ReadOnlyStore`] ones the daemon calls, so call sites do not change.
use super::turn_watch::TurnWatch;
use orbit_store::{ChunkReceipt, Job, PendingPrivateNotes, ReadOnlyStore, Result};
use std::{panic::Location, time::Instant};

pub(super) struct TimedReader {
    store: ReadOnlyStore,
    watch: TurnWatch,
}

impl TimedReader {
    pub(super) fn new(store: ReadOnlyStore, watch: TurnWatch) -> Self {
        Self { store, watch }
    }

    fn timed<T>(
        &self,
        at: &'static Location<'static>,
        read: impl FnOnce(&ReadOnlyStore) -> T,
    ) -> T {
        let started = Instant::now();
        let output = read(&self.store);
        self.watch.read_waited(at, started.elapsed());
        output
    }

    #[track_caller]
    pub(super) fn get_job(&self, id: &str) -> Result<Option<Job>> {
        self.timed(Location::caller(), |store| store.get_job(id))
    }

    #[track_caller]
    pub(super) fn list_jobs_after(&self, after_id: Option<&str>, limit: usize) -> Result<Vec<Job>> {
        self.timed(Location::caller(), |store| {
            store.list_jobs_after(after_id, limit)
        })
    }

    #[track_caller]
    pub(super) fn pending_jobs_of_kind(
        &self,
        kind: &str,
        after_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Job>> {
        self.timed(Location::caller(), |store| {
            store.pending_jobs_of_kind(kind, after_id, limit)
        })
    }

    /// The chunk uploads the store still owes; the capture task reads its
    /// own, a test reads them here.
    #[cfg(test)]
    #[track_caller]
    pub(super) fn pending_chunk_uploads(
        &self,
        after_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Job>> {
        self.timed(Location::caller(), |store| {
            store.pending_chunk_uploads(after_id, limit)
        })
    }

    #[track_caller]
    pub(super) fn pending_job_count(&self) -> Result<u64> {
        self.timed(Location::caller(), ReadOnlyStore::pending_job_count)
    }

    #[track_caller]
    pub(super) fn finalize_counts(&self, recording: &str) -> Result<(u64, u64)> {
        self.timed(Location::caller(), |store| store.finalize_counts(recording))
    }

    #[track_caller]
    pub(super) fn latest_chunk_receipt(
        &self,
        recording: &str,
        track: &str,
    ) -> Result<Option<ChunkReceipt>> {
        self.timed(Location::caller(), |store| {
            store.latest_chunk_receipt(recording, track)
        })
    }

    #[track_caller]
    pub(super) fn refused_chunks(
        &self,
        recording: &str,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<orbit_store::ChunkMetadata>> {
        self.timed(Location::caller(), |store| {
            store.refused_chunks(recording, after, limit)
        })
    }

    #[track_caller]
    pub(super) fn refused_chunk_count(&self, recording: &str) -> Result<u64> {
        self.timed(Location::caller(), |store| {
            store.refused_chunk_count(recording)
        })
    }

    #[cfg(test)]
    #[track_caller]
    pub(super) fn chunk_receipt(&self, id: &str) -> Result<Option<ChunkReceipt>> {
        self.timed(Location::caller(), |store| store.chunk_receipt(id))
    }

    #[track_caller]
    pub(super) fn next_uploaded_chunk(&self) -> Result<Option<String>> {
        self.timed(Location::caller(), ReadOnlyStore::next_uploaded_chunk)
    }

    #[track_caller]
    pub(super) fn list_chunks(
        &self,
        recording_id: &str,
        after_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ChunkReceipt>> {
        self.timed(Location::caller(), |store| {
            store.list_chunks(recording_id, after_id, limit)
        })
    }

    /// Reads and decrypts a whole chunk of audio.
    #[track_caller]
    pub(super) fn read_chunk(&self, id: &str) -> Result<Vec<u8>> {
        self.timed(Location::caller(), |store| store.read_chunk(id))
    }

    #[track_caller]
    pub(super) fn private_notes(&self, recording: &str) -> Result<Option<PendingPrivateNotes>> {
        self.timed(Location::caller(), |store| store.private_notes(recording))
    }

    #[track_caller]
    pub(super) fn load_assistant_folders(
        &self,
    ) -> Result<Vec<orbit_core::assistant_files::Folder>> {
        self.timed(Location::caller(), ReadOnlyStore::load_assistant_folders)
    }

    #[track_caller]
    pub(super) fn load_call_preference(
        &self,
    ) -> Result<Option<orbit_core::call_detection::CallPreference>> {
        self.timed(Location::caller(), ReadOnlyStore::load_call_preference)
    }

    #[track_caller]
    pub(super) fn pending_private_notes(&self) -> Result<Vec<String>> {
        self.timed(Location::caller(), ReadOnlyStore::pending_private_notes)
    }

    #[track_caller]
    pub(super) fn note_orders(&self) -> Result<Vec<orbit_store::WaitingOrder>> {
        self.timed(Location::caller(), ReadOnlyStore::note_orders)
    }

    #[cfg(test)]
    pub(super) fn decryption_count(&self) -> u64 {
        self.store.decryption_count()
    }
}
