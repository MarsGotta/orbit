use crate::{
    ChunkReceipt, Error, Job, JobState, MAX_PAGE_BYTES, MAX_PAYLOAD_BYTES, QUEUE_KINDS, Result,
    crypto::OVERHEAD,
};
use crate::{
    Store,
    blobs::MAX_METADATA_BYTES,
    validation::{
        bounded_blob, context, decode_job, validate_id, validate_metadata, validate_page,
    },
};
use rusqlite::{OptionalExtension, params, params_from_iter};
impl Store {
    /// Indexed readiness facts for a recording finalization.
    ///
    /// The first count includes receipt-only uploaded chunks. The second is the subset whose
    /// deterministic upload job is terminal-successful. Neither query reads or
    /// decrypts a manifest or job payload.
    pub fn finalize_counts(&self, recording_id: &str) -> Result<(u64, u64)> {
        validate_id(recording_id)?;
        let chunks: i64 = self.connection.query_row(
            "SELECT (SELECT COUNT(*) FROM chunks WHERE recording_id = ?1)
                  + (SELECT COUNT(*) FROM records WHERE namespace='uploaded_chunks:' || ?1)",
            [recording_id],
            |row| row.get(0),
        )?;
        let uploaded: i64 = self.connection.query_row(
            "SELECT (SELECT COUNT(*) FROM chunks AS chunk
             JOIN jobs AS job ON job.id = 'upload-' || chunk.id
             WHERE chunk.recording_id = ?1
               AND job.kind = 'chunk_upload'
               AND job.state = 'succeeded')
                  + (SELECT COUNT(*) FROM records WHERE namespace='uploaded_chunks:' || ?1)",
            [recording_id],
            |row| row.get(0),
        )?;
        Ok((
            u64::try_from(chunks).map_err(|_| Error::Invalid("database count"))?,
            u64::try_from(uploaded).map_err(|_| Error::Invalid("database count"))?,
        ))
    }

    /// Chunks of a recording whose upload the hub refused for good
    /// (`blocked`), at most `limit`, by track and sequence. They stay on the
    /// Mac; the recording's closure declares them lost instead of waiting.
    pub fn refused_chunks(
        &self,
        recording_id: &str,
        limit: usize,
    ) -> Result<Vec<crate::ChunkMetadata>> {
        validate_id(recording_id)?;
        validate_page(limit)?;
        let mut statement = self.connection.prepare(
            "SELECT chunk.id FROM chunks AS chunk
             JOIN jobs AS job ON job.id = 'upload-' || chunk.id
             WHERE chunk.recording_id = ?1 AND job.kind = 'chunk_upload'
               AND job.state = 'blocked'
             ORDER BY chunk.track, chunk.sequence LIMIT ?2",
        )?;
        let ids = statement
            .query_map(params![recording_id, limit as i64], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut chunks = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(receipt) = self.chunk_receipt(&id)? {
                chunks.push(receipt.metadata);
            }
        }
        Ok(chunks)
    }

    /// Queue count used by the daemon's once-per-tick presentation refresh.
    /// Unlike `diagnostics`, this does not collect SQLite build/runtime facts.
    pub fn pending_job_count(&self) -> Result<u64> {
        let placeholders = std::iter::repeat_n("?", QUEUE_KINDS.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT COUNT(*) FROM jobs WHERE kind IN ({placeholders})
             AND state IN ('queued','retry_wait','running','unknown')"
        );
        let count: i64 = self.connection.query_row(
            &sql,
            params_from_iter(QUEUE_KINDS.iter().copied()),
            |row| row.get(0),
        )?;
        u64::try_from(count).map_err(|_| Error::Invalid("database count"))
    }

    /// Highest numeric sequence for one recording/track, using the existing
    /// unique index rather than scanning audio or sorting IDs/native clocks.
    /// Every selected index column is checked against the authenticated receipt.
    /// This is a local durable frontier, not proof of upload, live freshness,
    /// blob integrity (use read_chunk), or resistance to whole-store rollback.
    pub fn latest_chunk_receipt(
        &self,
        recording_id: &str,
        track: &str,
    ) -> Result<Option<ChunkReceipt>> {
        validate_id(recording_id)?;
        validate_id(track)?;
        let row = self
            .connection
            .query_row(
                "SELECT id, recording_id, track, sequence, bytes, length(payload), payload
             FROM chunks WHERE recording_id = ?1 AND track = ?2
             UNION ALL
             SELECT id, ?1, kind, created_at, NULL, length(payload), payload FROM records
             WHERE namespace='uploaded_chunks:' || ?1 AND kind=?2
             ORDER BY sequence DESC LIMIT 1",
                params![recording_id, track],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        bounded_blob(
                            row,
                            6,
                            MAX_METADATA_BYTES + OVERHEAD,
                            "El manifiesto de audio guardado",
                        )?,
                    ))
                },
            )
            .optional()?;
        let Some((id, indexed_recording, indexed_track, sequence, bytes, payload)) = row else {
            return Ok(None);
        };
        let receipt: ChunkReceipt =
            serde_json::from_slice(&self.cipher.decrypt(&context("manifest", &id), &payload)?)?;
        validate_metadata(&receipt.metadata)?;
        if receipt.metadata.id != id
            || receipt.metadata.recording_id != indexed_recording
            || indexed_recording != recording_id
            || receipt.metadata.track != indexed_track
            || indexed_track != track
            || i64::try_from(receipt.metadata.sequence).ok() != Some(sequence)
            || bytes.is_some_and(|bytes| i64::try_from(receipt.bytes).ok() != Some(bytes))
        {
            return Err(Error::Authentication);
        }
        Ok(Some(receipt))
    }
    /// Cursor page of nonterminal work of one kind. Running/unknown work still needs an idempotent protocol or reconciliation before execution.
    pub fn pending_jobs_of_kind(
        &self,
        kind: &str,
        after_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Job>> {
        validate_id(kind)?;
        if let Some(after_id) = after_id {
            validate_id(after_id)?;
        }
        validate_page(limit)?;
        let mut statement = self.connection.prepare(
            "SELECT id, state, payload, length(payload) FROM jobs
             WHERE kind = ?1 AND state IN ('queued','retry_wait','running','unknown')
             AND (?2 IS NULL OR id > ?2) ORDER BY id LIMIT ?3",
        )?;
        let rows = statement.query_map(params![kind, after_id, limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                bounded_blob(row, 2, MAX_PAYLOAD_BYTES + OVERHEAD, "El trabajo guardado")?,
            ))
        })?;
        let mut page_bytes = 0;
        let mut page = Vec::new();
        for row in rows {
            let (id, state, payload) = row?;
            if payload.len() > MAX_PAGE_BYTES - page_bytes {
                break;
            }
            page_bytes += payload.len();
            let job = decode_job(&self.cipher, &id, Some(&state), Some(kind), None, &payload)?;
            page.push(job);
        }
        Ok(page)
    }
    /// Cursor page of nonterminal chunk uploads in capture order: the chunk's
    /// start time (the job's `created_at`), then its numeric sequence, then
    /// the id. Text order over `{recording}-{track}-{sequence}` would send
    /// chunk 10 before chunk 2, and the hub closes a transcription group
    /// after a hole. The cursor is a job id in any state (an upload that just
    /// succeeded still marks the place); an unknown one yields an empty page,
    /// which restarts the scan.
    pub fn pending_chunk_uploads(&self, after_id: Option<&str>, limit: usize) -> Result<Vec<Job>> {
        if let Some(after_id) = after_id {
            validate_id(after_id)?;
        }
        validate_page(limit)?;
        let mut statement = self.connection.prepare(
            "WITH ordered AS (
                 SELECT jobs.id AS id, jobs.state AS state, jobs.payload AS payload,
                        jobs.created_at AS created_at, coalesce(chunks.sequence, 0) AS sequence
                 FROM jobs LEFT JOIN chunks ON jobs.id = 'upload-' || chunks.id
                 WHERE jobs.kind = 'chunk_upload'
                 AND jobs.state IN ('queued','retry_wait','running','unknown'))
             SELECT id, state, payload FROM ordered
             WHERE ?1 IS NULL OR (created_at, sequence, id) >
                 (SELECT jobs.created_at, coalesce(chunks.sequence, 0), jobs.id
                  FROM jobs LEFT JOIN chunks ON jobs.id = 'upload-' || chunks.id
                  WHERE jobs.id = ?1)
             ORDER BY created_at, sequence, id LIMIT ?2",
        )?;
        let rows = statement.query_map(params![after_id, limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                bounded_blob(row, 2, MAX_PAYLOAD_BYTES + OVERHEAD, "El trabajo guardado")?,
            ))
        })?;
        let mut page_bytes = 0;
        let mut page = Vec::new();
        for row in rows {
            let (id, state, payload) = row?;
            if payload.len() > MAX_PAGE_BYTES - page_bytes {
                break;
            }
            page_bytes += payload.len();
            let job = decode_job(
                &self.cipher,
                &id,
                Some(&state),
                Some("chunk_upload"),
                None,
                &payload,
            )?;
            page.push(job);
        }
        Ok(page)
    }
    /// Read one authenticated job by its immutable identity, without scanning a queue. A missing identity is not an error or proof that an effect ran.
    pub fn get_job(&self, id: &str) -> Result<Option<Job>> {
        validate_id(id)?;
        let mut matches = self.query_jobs(
            "SELECT id,state,payload,payload_length FROM (
                 SELECT id,state,payload,length(payload) AS payload_length
                 FROM jobs WHERE id=?1
                 UNION ALL
                 SELECT id,state,payload,length(payload) AS payload_length
                 FROM records WHERE id=?1 AND namespace NOT GLOB 'uploaded_chunks:*'
             ) LIMIT ?2",
            Some(id),
            2,
        )?;
        if matches.len() > 1 {
            return Err(Error::Conflict);
        }
        Ok(matches.pop())
    }
    /// Cursor page ordered by immutable ID; pass the last returned ID for the next page. Pages may be shorter than `limit` to bound memory; only an empty page marks EOF. A consistent export concerns finalized data without new inserts. Only kinds the daemon enqueues as queue work are returned, so domain records cannot enter the traversal or make it fail because of their padded size.
    pub fn list_jobs_after(&self, after_id: Option<&str>, limit: usize) -> Result<Vec<Job>> {
        self.query_queue_jobs(after_id, limit)
    }
    /// Authenticated recording receipts, including uploads whose audio has
    /// left. Closure needs all capture facts; this is not an audio inventory.
    pub fn list_chunks(
        &self,
        recording_id: &str,
        after_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ChunkReceipt>> {
        validate_id(recording_id)?;
        if let Some(after_id) = after_id {
            validate_id(after_id)?;
        }
        validate_page(limit)?;
        let mut statement = self.connection.prepare(
            "SELECT id, payload, length(payload) FROM chunks
             WHERE recording_id = ?1 AND (?2 IS NULL OR id > ?2)
             UNION ALL
             SELECT id, payload, length(payload) FROM records
             WHERE namespace='uploaded_chunks:' || ?1 AND (?2 IS NULL OR id > ?2)
             ORDER BY id LIMIT ?3",
        )?;
        let rows = statement.query_map(params![recording_id, after_id, limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                bounded_blob(
                    row,
                    1,
                    MAX_METADATA_BYTES + OVERHEAD,
                    "El manifiesto de audio guardado",
                )?,
            ))
        })?;
        rows.map(|row| {
            let (id, payload) = row?;
            let receipt: ChunkReceipt =
                serde_json::from_slice(&self.cipher.decrypt(&context("manifest", &id), &payload)?)?;
            if receipt.metadata.id != id || receipt.metadata.recording_id != recording_id {
                return Err(Error::Authentication);
            }
            Ok(receipt)
        })
        .collect()
    }
    /// On daemon startup only, make interrupted immutable chunk uploads retryable. The receiver must deduplicate the unchanged chunk ID and verify its hash. Other job kinds, including desktop input, remain untouched.
    pub fn recover_upload_jobs(&mut self) -> Result<usize> {
        let mut recovered = 0;
        loop {
            let jobs = self.query_jobs(
                "SELECT id, state, payload, length(payload) FROM jobs
                 WHERE kind = 'chunk_upload' AND state IN ('running', 'unknown')
                   AND (?1 IS NULL OR id > ?1) ORDER BY id LIMIT ?2",
                None,
                100,
            )?;
            if jobs.is_empty() {
                break;
            }
            for job in jobs {
                if job.kind != "chunk_upload" {
                    return Err(Error::Authentication);
                }
                self.update_job(&job.id, JobState::RetryWait)?;
                recovered += 1;
            }
        }
        Ok(recovered)
    }
    fn query_jobs(&self, sql: &str, after_id: Option<&str>, limit: usize) -> Result<Vec<Job>> {
        validate_page(limit)?;
        if let Some(after_id) = after_id {
            validate_id(after_id)?;
        }
        let limit_value = limit as i64;
        self.collect_jobs(sql, &[&after_id as &dyn rusqlite::ToSql, &limit_value])
    }
    /// Cursor page over queue-kind rows only. Domain records live in a separate table, so their padded ciphertext cannot make the traversal fail or shadow queue work. The payload-size guard at the closure still fires for oversized queue-kind rows, keeping the defence against corruption.
    fn query_queue_jobs(&self, after_id: Option<&str>, limit: usize) -> Result<Vec<Job>> {
        validate_page(limit)?;
        if let Some(after_id) = after_id {
            validate_id(after_id)?;
        }
        let placeholders = std::iter::repeat_n("?", QUEUE_KINDS.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT id, state, payload, length(payload) FROM jobs
             WHERE kind IN ({placeholders})
               AND (?{next} IS NULL OR id > ?{next})
             ORDER BY id LIMIT ?{last}",
            next = QUEUE_KINDS.len() + 1,
            last = QUEUE_KINDS.len() + 2,
        );
        let limit_value = limit as i64;
        let mut parameters: Vec<&dyn rusqlite::ToSql> = QUEUE_KINDS
            .iter()
            .map(|kind| kind as &dyn rusqlite::ToSql)
            .collect();
        parameters.push(&after_id);
        parameters.push(&limit_value);
        self.collect_jobs(&sql, &parameters)
    }
    /// One authenticated, memory-bounded job traversal over a caller-built query. Every selected index column is checked against the payload.
    pub(super) fn collect_jobs(
        &self,
        sql: &str,
        parameters: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<Job>> {
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(parameters), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                bounded_blob(row, 2, MAX_PAYLOAD_BYTES + OVERHEAD, "El trabajo guardado")?,
            ))
        })?;
        let mut page_bytes = 0;
        let mut page = Vec::new();
        for row in rows {
            let (id, state, payload) = row?;
            if payload.len() > MAX_PAGE_BYTES - page_bytes {
                break;
            }
            page_bytes += payload.len();
            let job = decode_job(&self.cipher, &id, Some(&state), None, None, &payload)?;
            page.push(job);
        }
        Ok(page)
    }
}
