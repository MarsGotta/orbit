use crate::{
    Error, Job, JobState, MAX_PAYLOAD_BYTES, QUEUE_KINDS, Result, StoreDiagnostics,
    crypto::{Cipher, OVERHEAD},
    migrations,
    validation::{
        bounded_blob, bounded_json, bounded_setting, context, decode_job, private_directory,
        private_file, regular_file_if_exists, validate_id, validate_page,
    },
    writes,
};
pub(crate) use migrations::SCHEMA_VERSION;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Serialize, de::DeserializeOwned};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;
pub(crate) const KEY_CONTEXT: &[u8] = b"orbit:storage:key-check:v1";
pub(crate) const KEY_CHECK: &[u8] = b"Orbit encrypted storage key is valid.";
/// Production connections retain the existing 2 s SQLite busy wait to bound
/// one external writer collision
/// (for example, an older Orbit process); the daemon now serializes its own
/// writes through one actor. A run on a loaded host (load average 30.68)
/// observed a 6151 ms refusal after three such waits, so the value remains while that external
/// case is still possible. It protects against an unbounded SQLite lock wait,
/// not the length of one actor operation or a privacy-stop deadline.
pub(crate) const PRODUCTION_BUSY_TIMEOUT: Duration = Duration::from_secs(2);
/// A single-owner SQLite connection plus encrypted immutable audio fragments.  Call from the daemon's storage worker, never an audio callback or draw loop. All IDs, timestamps, scheduling decisions and the key come from the caller.
pub struct Store {
    pub(crate) connection: Connection,
    pub(crate) cipher: Cipher,
    pub(crate) blobs: PathBuf,
}
impl Store {
    /// Counts SQLite row changes on this connection. The daemon uses it only
    /// to avoid publishing a new in-memory store version for a read-only run;
    /// a failed transaction can still contribute to this conservative count.
    pub fn mutation_count(&self) -> u64 {
        self.connection.total_changes()
    }
    /// Total encrypted records this connection has decrypted since it was opened. Wraps the single decryption point in the cipher so it counts every job, message, chunk manifest, archive page, descriptor and closure the worker reads. Permanent, cheap, useful for read budgets.
    pub fn decryption_count(&self) -> u64 {
        self.cipher.decryption_count()
    }
    pub fn set_busy_timeout(&self, timeout: Duration) -> Result<()> {
        Ok(self.connection.busy_timeout(timeout)?)
    }
    /// Open or initialize owner-only storage. A different key fails closed.
    pub fn open(directory: impl AsRef<Path>, key: [u8; 32]) -> Result<Self> {
        if rusqlite::version_number() < 3_051_003 {
            return Err(Error::SqliteVersion);
        }
        let directory = directory.as_ref();
        let database = directory.join("orbit.sqlite3");
        for filename in ["orbit.sqlite3", "orbit.sqlite3-wal", "orbit.sqlite3-shm"] {
            regular_file_if_exists(&directory.join(filename))?;
        }
        if database.try_exists()? {
            if !std::fs::symlink_metadata(directory)?.file_type().is_dir() {
                return Err(Error::UnsafePath);
            }
            // /var is an ancestor alias on macOS. Resolve the validated
            // directory only, retaining NOFOLLOW on the database leaf.
            let inspected_database = std::fs::canonicalize(directory)?.join("orbit.sqlite3");
            let inspection = Connection::open_with_flags(
                &inspected_database,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                    | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
            )?;
            let version: i64 =
                inspection.pragma_query_value(None, "user_version", |row| row.get(0))?;
            if !(3..=SCHEMA_VERSION).contains(&version) {
                return Err(Error::SchemaVersion {
                    version,
                    min_version: 3,
                    max_version: SCHEMA_VERSION,
                });
            }
        }
        private_directory(directory)?;
        let blobs = directory.join("chunks");
        private_directory(&blobs)?;
        let mut connection = Connection::open(directory.join("orbit.sqlite3"))?;
        private_file(&directory.join("orbit.sqlite3"))?;
        connection.busy_timeout(PRODUCTION_BUSY_TIMEOUT)?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(Error::SchemaVersion {
                version,
                min_version: 3,
                max_version: SCHEMA_VERSION,
            });
        }
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;
             PRAGMA fullfsync=ON;
             PRAGMA checkpoint_fullfsync=ON;
             PRAGMA trusted_schema=OFF;",
        )?;
        let cipher = Cipher::new(&key);
        if version == 0 {
            let verifier = cipher.encrypt(KEY_CONTEXT, KEY_CHECK)?;
            migrations::initialize(&mut connection, &verifier)?;
            File::open(directory)?.sync_all()?;
        }
        let verifier: Vec<u8> = connection.query_row(
            "SELECT value FROM settings WHERE key = 'key_check' AND length(value) <= 256",
            [],
            |row| row.get(0),
        )?;
        if cipher.decrypt(KEY_CONTEXT, &verifier)? != KEY_CHECK {
            return Err(Error::Authentication);
        }
        // Authenticate the key before retiring any rows or source files.
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        migrations::migrate(&mut connection, version)?;
        migrations::retire_meeting_mirror(&mut connection)?;
        migrations::retire_messages(&connection)?;
        let sources = directory.join("sources");
        // Filesystem cleanup follows the committed SQL migration and is retried
        // on every open, including after a crash with user_version already 4.
        match std::fs::symlink_metadata(&sources) {
            Ok(metadata) if metadata.file_type().is_dir() => std::fs::remove_dir_all(&sources)?,
            Ok(_) => return Err(Error::UnsafePath),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        File::open(directory)?.sync_all()?;
        let checkpoint_fullfsync: i64 =
            connection.pragma_query_value(None, "checkpoint_fullfsync", |row| row.get(0))?;
        let store = Self {
            connection,
            cipher,
            blobs,
        };
        let diagnostics = store.diagnostics()?;
        if diagnostics.journal_mode != "wal"
            || diagnostics.synchronous != 2
            || diagnostics.foreign_keys != 1
            || diagnostics.fullfsync != 1
            || checkpoint_fullfsync != 1
        {
            return Err(Error::Invalid("effective SQLite durability settings"));
        }
        Ok(store)
    }
    /// Atomically replace the daemon snapshot. Sensitive fields stay encrypted.
    pub fn save_snapshot<T: Serialize>(&mut self, snapshot: &T) -> Result<()> {
        self.persist_snapshot(snapshot, None)
    }
    /// Commit an immutable durable job and its authorizing snapshot together. Neither change survives if either write fails. An identical existing job keeps its processing state, including Running, Unknown and Succeeded.
    pub fn save_snapshot_with_job<T: Serialize>(&mut self, snapshot: &T, job: &Job) -> Result<()> {
        self.persist_snapshot(snapshot, Some(job))
    }
    fn persist_snapshot<T: Serialize>(&mut self, snapshot: &T, job: Option<&Job>) -> Result<()> {
        let bytes = bounded_json(snapshot, MAX_PAYLOAD_BYTES)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(job) = job {
            writes::enqueue_job(&transaction, &self.cipher, job)?;
        }
        writes::snapshot(&transaction, &self.cipher, &bytes)?;
        transaction.commit()?;
        Ok(())
    }
    /// `None` means no saved snapshot; oversized or unauthenticated state is an error.
    pub fn load_snapshot<T: DeserializeOwned>(&self) -> Result<Option<T>> {
        let payload = bounded_setting(
            &self.connection,
            "snapshot",
            MAX_PAYLOAD_BYTES + OVERHEAD,
            "La instantánea guardada",
        )?;
        payload
            .map(|payload| {
                Ok(serde_json::from_slice(
                    &self.cipher.decrypt(b"orbit:snapshot:v1", &payload)?,
                )?)
            })
            .transpose()
    }
    /// Persist immutable input. A duplicate preserves the existing processing state.
    pub fn enqueue_job(&mut self, job: &Job) -> Result<()> {
        writes::enqueue_job(&self.connection, &self.cipher, job)
    }
    /// Test inspection helper: bounded chronological page, including terminal work.
    /// Production dispatch uses `pending_jobs_of_kind` and `list_jobs_after`.
    pub fn list_jobs(&self, limit: usize) -> Result<Vec<Job>> {
        validate_page(limit)?;
        let limit_value = limit as i64;
        self.collect_jobs(
            "SELECT id, state, payload, length(payload) FROM jobs
             ORDER BY created_at, rowid LIMIT ?1",
            &[&limit_value],
        )
    }
    /// Persist a transition already validated by the domain; input is immutable.
    pub fn update_job(&mut self, id: &str, state: JobState) -> Result<()> {
        validate_id(id)?;
        let stored = self.record_payload("jobs", id)?.ok_or(Error::NotFound)?;
        let mut job = decode_job(&self.cipher, id, None, None, None, &stored)?;
        if matches!(
            job.state,
            JobState::Succeeded | JobState::Cancelled | JobState::Blocked
        ) && !matches!(
            state,
            JobState::Succeeded | JobState::Cancelled | JobState::Blocked
        ) {
            return Err(Error::JobTransition);
        }
        job.state = state;
        let payload = self
            .cipher
            .encrypt(&context("job", id), &bounded_json(&job, MAX_PAYLOAD_BYTES)?)?;
        let transaction = self.connection.transaction()?;
        let changed = transaction.execute(
            "UPDATE jobs SET state = ?2, payload = ?3 WHERE id = ?1",
            params![id, state.as_str(), payload],
        )?;
        // The job was read outside this transaction: a row gone in between
        // is not found.
        if changed != 1 {
            return Err(Error::NotFound);
        }
        if state == JobState::Succeeded
            && job.kind == "recording_finalize"
            && let Some(recording) = job.payload["recording_id"].as_str()
        {
            // The durable closure owns the track summaries and forbids
            // resuming this recording. Its temporary capture receipts can go.
            transaction.execute(
                "DELETE FROM records WHERE namespace='uploaded_chunks:' || ?1",
                [recording],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
    /// Runtime evidence from the actual linked SQLite and effective pragmas.
    pub fn diagnostics(&self) -> Result<StoreDiagnostics> {
        let pragma_i64 = |name| {
            self.connection
                .pragma_query_value(None, name, |r| r.get::<_, i64>(0))
        };
        let count = |sql: &str| -> Result<u64> {
            let value: i64 = self.connection.query_row(sql, [], |r| r.get(0))?;
            u64::try_from(value).map_err(|_| Error::Invalid("database count"))
        };
        let mut statement = self.connection.prepare("PRAGMA compile_options")?;
        let compile_options = statement
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let placeholders = std::iter::repeat_n("?", QUEUE_KINDS.len())
            .collect::<Vec<_>>()
            .join(",");
        let pending_sql = format!(
            "SELECT count(*) FROM jobs WHERE kind IN ({placeholders}) \
             AND state IN ('queued','retry_wait','running','unknown')"
        );
        let pending_jobs: i64 = self.connection.query_row(
            &pending_sql,
            rusqlite::params_from_iter(QUEUE_KINDS.iter().copied()),
            |r| r.get(0),
        )?;
        Ok(StoreDiagnostics {
            sqlite_version: rusqlite::version().to_owned(),
            sqlite_source_id: self
                .connection
                .query_row("SELECT sqlite_source_id()", [], |r| r.get(0))?,
            compile_options,
            schema_version: pragma_i64("user_version")?,
            journal_mode: self
                .connection
                .pragma_query_value(None, "journal_mode", |r| r.get(0))?,
            synchronous: pragma_i64("synchronous")?,
            foreign_keys: pragma_i64("foreign_keys")?,
            fullfsync: pragma_i64("fullfsync")?,
            pending_jobs: u64::try_from(pending_jobs)
                .map_err(|_| Error::Invalid("database count"))?,
            chunk_count: count("SELECT count(*) FROM chunks")?,
            chunk_bytes: count("SELECT coalesce(sum(bytes), 0) FROM chunks")?,
        })
    }
    pub(crate) fn record_payload(&self, table: &'static str, id: &str) -> Result<Option<Vec<u8>>> {
        let sql = match table {
            "jobs" => "SELECT payload, length(payload) FROM jobs WHERE id = ?1",
            "chunks" => "SELECT payload, length(payload) FROM chunks WHERE id = ?1",
            _ => return Err(Error::Invalid("record table")),
        };
        Ok(self
            .connection
            .query_row(sql, [id], |row| {
                bounded_blob(
                    row,
                    0,
                    MAX_PAYLOAD_BYTES + OVERHEAD,
                    "El contenido cifrado guardado",
                )
            })
            .optional()?)
    }
}
