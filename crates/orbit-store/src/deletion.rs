//! A meeting deleted from this Mac: «Borrar reunión», or «Terminar sin
//! guardar» while it records.
//!
//! What goes is found by the meeting's identity only: its chunks by their
//! `recording_id` column, its jobs by their exact ids or, for the upload
//! receipts of audio already released, by their authenticated payload. No
//! other recording is matched by a pattern, and a voice note or a voice call
//! is refused, never deleted here (`Store::discard_voice_note` has its own).
//!
//! The audio goes `limit` chunks at a time, so one meeting of hours never
//! holds the writer for long; the caller repeats until `done`. Each step
//! unlinks before its commit, as a prune does: a crash leaves rows without
//! files, which the next step removes, never a file no row names.
use crate::{Error, MAX_PAGE_SIZE, Result, Store, recording_notes, validation::validate_id};
use rusqlite::{TransactionBehavior, params};
use std::{fs, path::PathBuf};

/// How much of the meeting goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletionScope {
    /// Its audio and every job and receipt of its capture and closure: what
    /// «Terminar sin guardar» deletes at once.
    Audio,
    /// That, and what the person wrote that waits for the hub (its orders
    /// and private notes): once the hub deleted it, or never had it.
    Everything,
}

/// What one step deleted, and whether nothing of the scope is left.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Deleted {
    pub chunks: u64,
    pub jobs: u64,
    pub records: u64,
    pub orders: u64,
    pub done: bool,
}

impl Store {
    /// One step of deleting `recording_id`: up to `limit` chunks, or, when
    /// none is left, the rest of the scope in one transaction.
    pub fn delete_recording(
        &mut self,
        recording_id: &str,
        scope: DeletionScope,
        limit: usize,
    ) -> Result<Deleted> {
        validate_id(recording_id)?;
        if limit == 0 || limit > MAX_PAGE_SIZE {
            return Err(Error::CountOutOfRange {
                what: "Fragmentos borrados por paso",
                count: limit,
                min_count: 1,
                max_count: MAX_PAGE_SIZE,
            });
        }
        if let Some(manifest) = self.get_job(&format!("recording-{recording_id}"))?
            && manifest.kind == "recording_manifest"
            && matches!(
                manifest.payload["recording_kind"].as_str(),
                Some("voice_turn" | "voice_call")
            )
        {
            return Err(Error::Invalid("not a meeting"));
        }
        let chunks = self.chunk_ids(recording_id, limit)?;
        let audio = self.delete_chunks(&chunks)?;
        if chunks.len() == limit {
            // There may be more: the next step goes on with them.
            return Ok(audio);
        }
        let rest = self.delete_rest(recording_id, scope)?;
        Ok(Deleted {
            chunks: audio.chunks,
            jobs: audio.jobs + rest.jobs,
            ..rest
        })
    }

    fn chunk_ids(&self, recording_id: &str, limit: usize) -> Result<Vec<String>> {
        let mut statement = self
            .connection
            .prepare("SELECT id FROM chunks WHERE recording_id=?1 ORDER BY id LIMIT ?2")?;
        let ids = statement
            .query_map(params![recording_id, limit as i64], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids)
    }

    fn delete_chunks(&mut self, chunks: &[String]) -> Result<Deleted> {
        if chunks.is_empty() {
            return Ok(Deleted::default());
        }
        let paths: Vec<PathBuf> = chunks.iter().map(|id| self.chunk_path(id)).collect();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut deleted = Deleted::default();
        for id in chunks {
            deleted.chunks += transaction.execute("DELETE FROM chunks WHERE id=?1", [id])? as u64;
            deleted.jobs += transaction
                .execute("DELETE FROM jobs WHERE id=?1", [format!("upload-{id}")])?
                as u64;
        }
        for path in &paths {
            crate::validation::regular_file_if_exists(path)?;
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        fs::File::open(&self.blobs)?.sync_all()?;
        transaction.commit()?;
        Ok(deleted)
    }

    /// The jobs of the meeting's capture and closure, by exact id or by an
    /// authenticated payload naming it.
    fn job_ids(&self, recording_id: &str) -> Result<Vec<String>> {
        let mut ids = vec![
            format!("recording-{recording_id}"),
            format!("finalize-{recording_id}"),
        ];
        // Released audio keeps its upload receipt; its id starts with the
        // chunk's, which starts with the recording's. The range only finds
        // candidates: each is kept unless its payload names this recording.
        let candidates: Vec<String> = {
            let mut statement = self.connection.prepare(
                "SELECT id FROM jobs WHERE kind='chunk_upload' AND id > ?1 AND id < ?2
                 UNION ALL SELECT id FROM jobs WHERE kind='capture_epoch'",
            )?;
            statement
                .query_map(
                    params![
                        format!("upload-{recording_id}-"),
                        format!("upload-{recording_id}.")
                    ],
                    |row| row.get(0),
                )?
                .collect::<rusqlite::Result<Vec<String>>>()?
        };
        for id in candidates {
            let Some(job) = self.get_job(&id)? else {
                continue;
            };
            let named = match job.kind.as_str() {
                "chunk_upload" => &job.payload["metadata"]["recording_id"],
                _ => &job.payload["recording_id"],
            };
            if named == recording_id {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    fn delete_rest(&mut self, recording_id: &str, scope: DeletionScope) -> Result<Deleted> {
        let jobs = self.job_ids(recording_id)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut deleted = Deleted {
            done: true,
            ..Deleted::default()
        };
        for id in &jobs {
            deleted.jobs += transaction.execute("DELETE FROM jobs WHERE id=?1", [id])? as u64;
        }
        deleted.records += transaction.execute(
            "DELETE FROM records WHERE namespace='uploaded_chunks:' || ?1",
            [recording_id],
        )? as u64;
        if scope == DeletionScope::Everything {
            deleted.orders += transaction.execute(
                "DELETE FROM note_outbox WHERE lane=?1",
                [format!("meeting:{recording_id}")],
            )? as u64;
            deleted.records += transaction.execute(
                "DELETE FROM records WHERE namespace=?1 AND id=?2",
                params![
                    recording_notes::NAMESPACE,
                    recording_notes::id_of(recording_id)
                ],
            )? as u64;
        }
        transaction.commit()?;
        Ok(deleted)
    }
}
