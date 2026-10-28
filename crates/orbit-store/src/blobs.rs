use super::{
    Store,
    validation::{
        bounded_json, context, job_bytes, regular_file_if_exists, validate_id, validate_metadata,
    },
};
use crate::{
    ChunkMetadata, ChunkReceipt, Error, Job, JobState, MAX_CHUNK_BYTES, RecoveryReport, Result,
    crypto::{OVERHEAD, digest},
};
use rusqlite::params;
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(super) const MAX_METADATA_BYTES: usize = 4096;
const MAX_SEALED_CHUNK_BYTES: usize = MAX_CHUNK_BYTES + MAX_METADATA_BYTES + 4 + OVERHEAD;

/// A forward-only, bounded-memory scan owned by the caller until EOF. It must
/// not be shared between stores. Capture stays stopped during reconciliation.
pub struct RecoveryScan {
    entries: std::iter::Peekable<fs::ReadDir>,
    directory: PathBuf,
    manifest_after: String,
    manifests_complete: bool,
}

impl Store {
    /// Synchronize immutable encrypted audio, then commit its manifest and upload
    /// job in one transaction. Reusing an ID requires identical content and facts.
    pub fn seal_chunk(&mut self, metadata: &ChunkMetadata, bytes: &[u8]) -> Result<ChunkReceipt> {
        validate_metadata(metadata)?;
        if bytes.is_empty() {
            return Err(Error::Invalid("empty audio chunk"));
        }
        if bytes.len() > MAX_CHUNK_BYTES {
            return Err(Error::TooLarge {
                what: "Audio chunk",
                bytes: bytes.len() as u64,
                max_bytes: MAX_CHUNK_BYTES as u64,
            });
        }
        let receipt = ChunkReceipt {
            metadata: metadata.clone(),
            bytes: bytes.len() as u64,
            sha256: digest(bytes),
        };
        if let Some(existing) = self.chunk_receipt(&metadata.id)? {
            if existing != receipt {
                return Err(Error::Conflict);
            }
            self.read_chunk(&metadata.id)?;
            return Ok(existing);
        }
        // A delayed seal receipt may be retried after the upload already
        // released its audio. Its immutable upload input still authenticates
        // the content; never recreate a confirmed blob.
        if let Some(job) = self.get_job(&format!("upload-{}", metadata.id))? {
            return if job.kind == "chunk_upload"
                && job.state == JobState::Succeeded
                && job.payload == serde_json::to_value(&receipt)?
            {
                Ok(receipt)
            } else {
                Err(Error::Conflict)
            };
        }
        let path = self.chunk_path(&metadata.id);
        regular_file_if_exists(&path)?;
        if path.try_exists()? {
            let (existing, existing_bytes) = self.read_sealed(&path, &metadata.id)?;
            if existing != *metadata || existing_bytes != bytes {
                return Err(Error::Conflict);
            }
        } else {
            let metadata_bytes = bounded_json(metadata, MAX_METADATA_BYTES)?;
            let mut plaintext = Vec::with_capacity(4 + metadata_bytes.len() + bytes.len());
            plaintext.extend_from_slice(&(metadata_bytes.len() as u32).to_le_bytes());
            plaintext.extend_from_slice(&metadata_bytes);
            plaintext.extend_from_slice(bytes);
            let sealed = self
                .cipher
                .encrypt(&context("chunk", &metadata.id), &plaintext)?;
            let mut temporary = tempfile::Builder::new()
                .prefix(".pending-")
                .tempfile_in(&self.blobs)?;
            temporary.write_all(&sealed)?;
            temporary.as_file().sync_all()?;
            // Exclusive publication prevents an existing original being replaced.
            temporary
                .persist_noclobber(&path)
                .map_err(|error| Error::Io(error.error))?;
            File::open(&self.blobs)?.sync_all()?;
        }
        self.commit_chunk(&receipt)?;
        Ok(receipt)
    }

    /// Authenticate the manifest and file before returning the original audio.
    pub fn read_chunk(&self, id: &str) -> Result<Vec<u8>> {
        validate_id(id)?;
        let receipt = self.chunk_receipt(id)?.ok_or(Error::NotFound)?;
        let (metadata, bytes) = self.read_sealed(&self.chunk_path(id), id)?;
        if metadata != receipt.metadata
            || bytes.len() as u64 != receipt.bytes
            || digest(&bytes) != receipt.sha256
        {
            return Err(Error::CorruptChunk);
        }
        Ok(bytes)
    }

    /// Read an authenticated manifest. This alone does not verify the audio file.
    pub fn chunk_receipt(&self, id: &str) -> Result<Option<ChunkReceipt>> {
        validate_id(id)?;
        self.record_payload("chunks", id)?
            .map(|payload| {
                let receipt: ChunkReceipt = serde_json::from_slice(
                    &self.cipher.decrypt(&context("manifest", id), &payload)?,
                )?;
                if receipt.metadata.id != id {
                    return Err(Error::Authentication);
                }
                Ok(receipt)
            })
            .transpose()
    }

    /// Release only a durably acknowledged upload. Keep its small encrypted
    /// receipt for recording closure and capture-position restoration, outside
    /// the audio inventory. The upload job remains the durable confirmation.
    /// Returns the encrypted bytes unlinked, or None when not eligible.
    pub fn prune_uploaded_chunk(&mut self, id: &str) -> Result<Option<u64>> {
        let Some(receipt) = self.chunk_receipt(id)? else {
            return Ok(None);
        };
        let Some(job) = self.get_job(&format!("upload-{id}"))? else {
            return Ok(None);
        };
        if job.kind != "chunk_upload" || job.state != JobState::Succeeded {
            return Ok(None);
        }
        if job.payload != serde_json::to_value(&receipt)? {
            return Err(Error::Authentication);
        }
        if let Some(manifest) =
            self.get_job(&format!("recording-{}", receipt.metadata.recording_id))?
            && manifest.kind == "recording_manifest"
            && manifest.payload["upload_allowed"] == false
        {
            return Ok(None);
        }
        let path = self.chunk_path(id);
        regular_file_if_exists(&path)?;
        let closed = self
            .get_job(&format!("finalize-{}", receipt.metadata.recording_id))?
            .is_some_and(|job| {
                job.kind == "recording_finalize"
                    && job.state == JobState::Succeeded
                    && job.payload["recording_id"] == receipt.metadata.recording_id
            });
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // Move metadata, rather than keeping an audio row with a missing file.
        // The existing records index serves recording/track/sequence reads.
        if !closed {
            transaction.execute(
                "INSERT INTO records(namespace,id,kind,state,created_at,payload)
             SELECT 'uploaded_chunks:' || ?2,id,?3,'succeeded',?4,payload
             FROM chunks WHERE id=?1",
                params![
                    id,
                    receipt.metadata.recording_id,
                    receipt.metadata.track,
                    receipt.metadata.sequence as i64
                ],
            )?;
        }
        transaction.execute("DELETE FROM chunks WHERE id=?1", [id])?;
        let bytes = match fs::metadata(&path) {
            Ok(metadata) => {
                fs::remove_file(&path)?;
                metadata.len()
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.into()),
        };
        // Unlink before SQL commit: a crash may leave a confirmed row with no
        // file, which this operation retries. The reverse order would leave an
        // orphan that recovery could mistake for audio still needing upload.
        File::open(&self.blobs)?.sync_all()?;
        transaction.commit()?;
        Ok(Some(bytes))
    }

    /// «Descartar», or a voice note in which nothing was heard: its audio,
    /// receipts and jobs leave this Mac. Only a recording whose manifest says
    /// it is a voice note that never uploads (`voice_turn`,
    /// `upload_allowed:false`) is deleted; anything else is refused. Returns
    /// the chunks deleted; none left is not an error.
    pub fn discard_voice_note(&mut self, recording_id: &str) -> Result<u64> {
        validate_id(recording_id)?;
        let manifest_id = format!("recording-{recording_id}");
        let Some(manifest) = self.get_job(&manifest_id)? else {
            return Ok(0);
        };
        if manifest.kind != "recording_manifest"
            || manifest.payload["recording_kind"] != "voice_turn"
            || manifest.payload["upload_allowed"] != false
        {
            return Err(Error::Invalid("not a voice note"));
        }
        // A voice note is one chunk; two is enough to see one too many,
        // which a voice note never has and is deleted all the same.
        let chunks: Vec<String> = self
            .list_chunks(recording_id, None, 2)?
            .into_iter()
            .map(|receipt| receipt.metadata.id)
            .collect();
        let paths: Vec<PathBuf> = chunks.iter().map(|id| self.chunk_path(id)).collect();
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for id in &chunks {
            transaction.execute("DELETE FROM chunks WHERE id=?1", [id])?;
            transaction.execute("DELETE FROM jobs WHERE id=?1", [format!("upload-{id}")])?;
        }
        transaction.execute("DELETE FROM jobs WHERE id=?1", [&manifest_id])?;
        // Unlink before the commit, as a prune does: a crash leaves rows
        // without files, which discarding again removes; never a file no row
        // names, which recovery would take for audio to keep.
        for path in &paths {
            regular_file_if_exists(path)?;
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        File::open(&self.blobs)?.sync_all()?;
        transaction.commit()?;
        Ok(chunks.len() as u64)
    }

    /// One old confirmation per maintenance turn, so cleanup cannot occupy
    /// the writer for a whole historical recording. No audio is read here.
    pub fn next_uploaded_chunk(&self) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .connection
            .query_row(
                "SELECT c.id FROM chunks c JOIN jobs j ON j.id='upload-' || c.id
             WHERE j.kind='chunk_upload' AND j.state='succeeded' ORDER BY c.id LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Begin a bounded scan of complete files left before journal commit.
    /// Invalid artifacts remain on disk; the caller advances it to EOF.
    pub fn begin_recovery(&self) -> Result<RecoveryScan> {
        Ok(RecoveryScan {
            entries: fs::read_dir(&self.blobs)?.peekable(),
            directory: self.blobs.clone(),
            manifest_after: String::new(),
            manifests_complete: false,
        })
    }

    /// Inspect at most `limit` directory entries and `limit` indexed manifests.
    /// The iterator and keyset cursor advance even past invalid/temporary files.
    pub fn recover_next(
        &mut self,
        scan: &mut RecoveryScan,
        limit: usize,
    ) -> Result<RecoveryReport> {
        // Keep the existing 100,000-entry per-step work budget. This bounds
        // one scan call, not the total recoverable recording or store.
        if limit == 0 || limit > 100_000 {
            return Err(Error::CountOutOfRange {
                what: "Entries per recovery step",
                count: limit,
                min_count: 1,
                max_count: 100_000,
            });
        }
        if scan.directory != self.blobs {
            return Err(Error::Invalid("recovery store"));
        }
        let mut report = RecoveryReport::default();
        self.recover_files(scan, limit, &mut report)?;
        self.check_manifests(scan, limit, &mut report)?;
        report.limit_reached = scan.entries.peek().is_some() || !scan.manifests_complete;
        Ok(report)
    }

    /// The next `limit` directory entries: a sealed chunk whose manifest is
    /// missing gets it back; temporary, foreign and unreadable files are
    /// counted and left where they are.
    fn recover_files(
        &mut self,
        scan: &mut RecoveryScan,
        limit: usize,
        report: &mut RecoveryReport,
    ) -> Result<()> {
        for entry in scan.entries.by_ref().take(limit) {
            report.inspected += 1;
            let entry = entry?;
            let file_type = entry.file_type()?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                report.invalid += 1;
                continue;
            };
            if !file_type.is_file() {
                report.invalid += 1;
                continue;
            }
            if name.starts_with(".pending-") {
                report.incomplete_temporary += 1;
                continue;
            }
            let Some(id) = name.strip_suffix(".orbit") else {
                report.invalid += 1;
                continue;
            };
            if validate_id(id).is_err() {
                report.invalid += 1;
                continue;
            }
            self.recover_sealed(&entry.path(), id, report)?;
        }
        Ok(())
    }

    /// One sealed chunk on disk: its manifest is committed when absent, and a
    /// chunk that does not open or disagrees with its manifest is invalid.
    fn recover_sealed(&mut self, path: &Path, id: &str, report: &mut RecoveryReport) -> Result<()> {
        match self.read_sealed(path, id) {
            Ok((metadata, bytes)) => {
                let receipt = ChunkReceipt {
                    metadata,
                    bytes: bytes.len() as u64,
                    sha256: digest(&bytes),
                };
                match self.chunk_receipt(id)? {
                    Some(existing) if existing == receipt => {}
                    Some(_) => report.invalid += 1,
                    None => match self.commit_chunk(&receipt) {
                        Ok(()) => report.recovered += 1,
                        Err(Error::Conflict) => report.invalid += 1,
                        Err(error) => return Err(error),
                    },
                }
            }
            Err(
                Error::Authentication
                | Error::CorruptChunk
                | Error::Invalid(_)
                | Error::Serialization
                | Error::TooLarge { .. },
            ) => report.invalid += 1,
            Err(error) => return Err(error),
        }
        Ok(())
    }

    /// File enumeration alone cannot reveal a manifest with a missing blob.
    /// Bound this second scan independently; it reads identifiers, not audio.
    fn check_manifests(
        &self,
        scan: &mut RecoveryScan,
        limit: usize,
        report: &mut RecoveryReport,
    ) -> Result<()> {
        let mut statement = self
            .connection
            .prepare("SELECT id FROM chunks WHERE id > ?1 ORDER BY id LIMIT ?2")?;
        let mut rows = statement.query(params![scan.manifest_after, limit as i64 + 1])?;
        let mut manifests = 0;
        scan.manifests_complete = true;
        while let Some(row) = rows.next()? {
            if manifests == limit {
                scan.manifests_complete = false;
                break;
            }
            manifests += 1;
            let id: String = row.get(0)?;
            validate_id(&id)?;
            if !self.chunk_path(&id).try_exists()? {
                // An interrupted cleanup can leave a confirmed row without
                // its file. Leave it for cleanup, without reporting audio loss.
                let confirmed = if let Some(job) = self.get_job(&format!("upload-{id}"))?
                    && job.kind == "chunk_upload"
                    && job.state == JobState::Succeeded
                {
                    let receipt = self.chunk_receipt(&id)?.ok_or(Error::NotFound)?;
                    job.payload == serde_json::to_value(receipt)?
                } else {
                    false
                };
                if !confirmed {
                    report.missing += 1;
                }
            }
            scan.manifest_after = id;
        }
        Ok(())
    }

    pub(crate) fn chunk_path(&self, id: &str) -> PathBuf {
        self.blobs.join(format!("{id}.orbit"))
    }

    fn read_sealed(&self, path: &Path, id: &str) -> Result<(ChunkMetadata, Vec<u8>)> {
        regular_file_if_exists(path)?;
        let file = File::open(path).map_err(|error| {
            // EACCES (PermissionDenied) is reported as a corrupt chunk so the
            // recovery scan can register the file as invalid and continue with
            // the rest of the directory; sealing and live reads still observe
            // the unreadable file as corrupt.
            match error.kind() {
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
                    Error::CorruptChunk
                }
                _ => Error::Io(error),
            }
        })?;
        let bytes = file.metadata()?.len();
        if bytes > MAX_SEALED_CHUNK_BYTES as u64 {
            return Err(Error::TooLarge {
                what: "Encrypted audio chunk",
                bytes,
                max_bytes: MAX_SEALED_CHUNK_BYTES as u64,
            });
        }
        let mut sealed = Vec::new();
        file.take(MAX_SEALED_CHUNK_BYTES as u64 + 1)
            .read_to_end(&mut sealed)?;
        if sealed.len() > MAX_SEALED_CHUNK_BYTES {
            return Err(Error::TooLarge {
                what: "Encrypted audio chunk",
                bytes: sealed.len() as u64,
                max_bytes: MAX_SEALED_CHUNK_BYTES as u64,
            });
        }
        let plaintext = self.cipher.decrypt(&context("chunk", id), &sealed)?;
        let header: [u8; 4] = plaintext
            .get(..4)
            .ok_or(Error::CorruptChunk)?
            .try_into()
            .map_err(|_| Error::CorruptChunk)?;
        let metadata_length = u32::from_le_bytes(header) as usize;
        if metadata_length > MAX_METADATA_BYTES {
            return Err(Error::CorruptChunk);
        }
        let metadata: ChunkMetadata = serde_json::from_slice(
            plaintext
                .get(4..4 + metadata_length)
                .ok_or(Error::CorruptChunk)?,
        )?;
        validate_metadata(&metadata)?;
        if metadata.id != id {
            return Err(Error::Authentication);
        }
        let bytes = plaintext
            .get(4 + metadata_length..)
            .ok_or(Error::CorruptChunk)?;
        if bytes.is_empty() || bytes.len() > MAX_CHUNK_BYTES {
            return Err(Error::CorruptChunk);
        }
        Ok((metadata, bytes.to_vec()))
    }

    fn commit_chunk(&mut self, receipt: &ChunkReceipt) -> Result<()> {
        let id = &receipt.metadata.id;
        let job = Job {
            id: format!("upload-{id}"),
            kind: crate::QueueKind::ChunkUpload.into(),
            payload: serde_json::to_value(receipt)?,
            state: JobState::Queued,
            created_at: receipt.metadata.started_at_ms,
        };
        let receipt_payload = self.cipher.encrypt(
            &context("manifest", id),
            &bounded_json(receipt, MAX_METADATA_BYTES)?,
        )?;
        let job_payload = self
            .cipher
            .encrypt(&context("job", &job.id), &job_bytes(&job)?)?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let occupied: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM chunks WHERE recording_id=?1 AND track=?2 AND sequence=?3)
                 OR EXISTS(SELECT 1 FROM records WHERE namespace='uploaded_chunks:' || ?1
                           AND kind=?2 AND created_at=?3)",
            params![
                receipt.metadata.recording_id,
                receipt.metadata.track,
                receipt.metadata.sequence as i64
            ],
            |r| r.get(0),
        )?;
        if occupied {
            return Err(Error::Conflict);
        }
        transaction.execute(
            "INSERT INTO chunks (id, recording_id, track, sequence, bytes, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, receipt.metadata.recording_id, receipt.metadata.track, receipt.metadata.sequence as i64, receipt.bytes as i64, receipt_payload],
        )?;
        transaction.execute(
            "INSERT INTO jobs (id, kind, state, created_at, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                job.id,
                job.kind,
                job.state.as_str(),
                job.created_at,
                job_payload
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }
}
