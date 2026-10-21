//! Existing-store sessions never initialize, migrate or repair permissions. Read and write access share all admission checks; neither switches journal mode. SQLite may maintain its ordinary WAL coordination files, so opening is not an immutable-file snapshot or an OS write prohibition.
use crate::{Error, Result, crypto::Cipher};
use crate::{Store, store::KEY_CHECK, store::KEY_CONTEXT, store::SCHEMA_VERSION};
use rusqlite::{Connection, OpenFlags};
use std::{fs, path::Path, time::Duration};
/// A deliberately narrow read capability. No blob, journal or snapshot writes are exposed, independently of the SQLite connection's read-only enforcement.
pub struct ReadOnlyStore {
    store: Store,
}

#[derive(Clone, Copy)]
enum Access {
    ReadOnly,
    ReadWrite,
}
impl ReadOnlyStore {
    pub fn decryption_count(&self) -> u64 {
        self.store.decryption_count()
    }

    pub fn private_notes(&self, recording: &str) -> Result<Option<crate::PendingPrivateNotes>> {
        self.store.private_notes(recording)
    }
    pub fn pending_private_notes(&self) -> Result<Vec<String>> {
        self.store.pending_private_notes()
    }
    pub fn load_snapshot<T: serde::de::DeserializeOwned>(&self) -> Result<Option<T>> {
        self.store.load_snapshot()
    }
    pub fn pending_job_count(&self) -> Result<u64> {
        self.store.pending_job_count()
    }
    pub fn pending_jobs_of_kind(
        &self,
        kind: &str,
        after_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<crate::Job>> {
        self.store.pending_jobs_of_kind(kind, after_id, limit)
    }
    pub fn list_jobs_after(&self, after_id: Option<&str>, limit: usize) -> Result<Vec<crate::Job>> {
        self.store.list_jobs_after(after_id, limit)
    }
    pub fn finalize_counts(&self, recording: &str) -> Result<(u64, u64)> {
        self.store.finalize_counts(recording)
    }
    pub fn latest_chunk_receipt(
        &self,
        recording: &str,
        track: &str,
    ) -> Result<Option<crate::ChunkReceipt>> {
        self.store.latest_chunk_receipt(recording, track)
    }
    pub fn chunk_receipt(&self, id: &str) -> Result<Option<crate::ChunkReceipt>> {
        self.store.chunk_receipt(id)
    }
    pub fn next_uploaded_chunk(&self) -> Result<Option<String>> {
        self.store.next_uploaded_chunk()
    }
    pub fn load_call_preference(
        &self,
    ) -> Result<Option<orbit_core::call_detection::CallPreference>> {
        self.store.load_call_preference()
    }
    pub fn get_job(&self, id: &str) -> Result<Option<crate::Job>> {
        self.store.get_job(id)
    }
    pub fn list_chunks(
        &self,
        recording_id: &str,
        after_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<crate::ChunkReceipt>> {
        self.store.list_chunks(recording_id, after_id, limit)
    }
    pub fn read_chunk(&self, id: &str) -> Result<Vec<u8>> {
        self.store.read_chunk(id)
    }
}
impl Store {
    /// Open an already initialized current-schema store with a read-only API. SQLite also forbids data writes on this connection. Every result needs its domain's authenticated parent checks; opening is not proof of archive completion.
    pub fn open_reader(directory: impl AsRef<Path>, key: [u8; 32]) -> Result<ReadOnlyStore> {
        Ok(ReadOnlyStore {
            store: Self::existing_session(directory.as_ref(), key, Access::ReadOnly)?,
        })
    }
    /// Open a current-schema WAL store for explicit writes without creating or migrating any application data, journal, directory or key. Missing domain ledgers are not initialized here; each operation authenticates its own parents and admission records before modifying them.
    pub fn open_existing(directory: impl AsRef<Path>, key: [u8; 32]) -> Result<Self> {
        Self::existing_session(directory.as_ref(), key, Access::ReadWrite)
    }
    fn existing_session(directory: &Path, key: [u8; 32], access: Access) -> Result<Self> {
        if rusqlite::version_number() < 3_051_003 {
            return Err(Error::SqliteVersion);
        }
        let selected_directory = directory;
        let selected_identity = existing_private(selected_directory, true)?;
        // macOS exposes ordinary user/temp directories through ancestor aliases such as /var -> /private/var. SQLite NOFOLLOW rejects *any* component reported by its full-path resolver. Resolve only the already validated directory, then revalidate its identity; never follow a store leaf or database symlink. Keep NOFOLLOW on the actual database open.
        let directory = fs::canonicalize(selected_directory)?;
        same_identity(&selected_identity, &existing_private(&directory, true)?)?;
        let blobs = directory.join("chunks");
        existing_private(&blobs, true)?;
        let database = directory.join("orbit.sqlite3");
        let database_identity = existing_private(&database, false)?;
        for filename in ["orbit.sqlite3-wal", "orbit.sqlite3-shm"] {
            super::validation::regular_file_if_exists(&directory.join(filename))?;
        }
        let flags = match access {
            Access::ReadOnly => OpenFlags::SQLITE_OPEN_READ_ONLY,
            Access::ReadWrite => OpenFlags::SQLITE_OPEN_READ_WRITE,
        } | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        // Intentionally no SQLITE_OPEN_CREATE in either mode.
        let connection = Connection::open_with_flags(&database, flags)?;
        // Detect ordinary path replacement during admission. These checks are not an OS sandbox against a malicious same-user process swapping paths back and forth; authenticated domain reads remain mandatory.
        same_identity(
            &selected_identity,
            &existing_private(selected_directory, true)?,
        )?;
        same_identity(&selected_identity, &existing_private(&directory, true)?)?;
        same_identity(&database_identity, &existing_private(&database, false)?)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON;")?;
        if matches!(access, Access::ReadOnly) {
            connection.pragma_update(None, "query_only", true)?;
        }
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != SCHEMA_VERSION {
            return Err(Error::SchemaVersion {
                version,
                min_version: SCHEMA_VERSION,
                max_version: SCHEMA_VERSION,
            });
        }
        let journal: String =
            connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        if journal != "wal" {
            return Err(Error::Invalid("existing WAL store"));
        }
        let cipher = Cipher::new(&key);
        let verifier: Vec<u8> = connection.query_row(
            "SELECT value FROM settings WHERE key='key_check' AND typeof(value)='blob' AND length(value)<=256",
            [], |row| row.get(0))?;
        if cipher.decrypt(KEY_CONTEXT, &verifier)? != KEY_CHECK {
            return Err(Error::Authentication);
        }
        if matches!(access, Access::ReadWrite) {
            // Connection-local durability options, never journal_mode=WAL or a schema/ledger initializer. Configure only after key validation.
            connection.execute_batch(
                "PRAGMA synchronous=FULL;
                 PRAGMA fullfsync=ON;
                 PRAGMA checkpoint_fullfsync=ON;",
            )?;
            for (name, expected) in [
                ("synchronous", 2i64),
                ("foreign_keys", 1),
                ("fullfsync", 1),
                ("checkpoint_fullfsync", 1),
            ] {
                let actual: i64 = connection.pragma_query_value(None, name, |row| row.get(0))?;
                if actual != expected {
                    return Err(Error::Invalid("existing writer durability"));
                }
            }
        }
        Ok(Self {
            connection,
            cipher,
            blobs,
        })
    }
}
fn existing_private(path: &Path, directory: bool) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if (directory && !metadata.file_type().is_dir())
        || (!directory && !metadata.file_type().is_file())
    {
        return Err(Error::UnsafePath);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(Error::UnsafePath);
        }
    }
    Ok(metadata)
}
fn same_identity(before: &fs::Metadata, after: &fs::Metadata) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return Err(Error::UnsafePath);
        }
    }
    #[cfg(not(unix))]
    let _ = (before, after);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reader_exposes_the_local_read_surface_without_materializing_a_store() -> Result<()> {
        let empty = tempfile::tempdir()?;
        assert!(Store::open_reader(empty.path().join("missing"), [7; 32]).is_err());
        assert_eq!(fs::read_dir(empty.path())?.count(), 0);
        let directory = tempfile::tempdir()?;
        let mut writer = Store::open(directory.path(), [7; 32])?;
        writer.enqueue_job(&crate::Job {
            id: "reader-surface".into(),
            kind: "synthetic".into(),
            state: crate::JobState::Succeeded,
            created_at: 1,
            payload: serde_json::json!({"synthetic":true}),
        })?;
        let reader = Store::open_reader(directory.path(), [7; 32])?;
        assert_eq!(
            reader.get_job("reader-surface")?.map(|job| job.id),
            Some("reader-surface".into())
        );
        assert!(
            reader
                .list_chunks("00000000-0000-4000-8000-000000000000", None, 100)?
                .is_empty()
        );
        Ok(())
    }
    #[test]
    fn sqlite_also_rejects_mutation_beneath_the_narrow_read_api() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let _writer = Store::open(directory.path(), [7; 32])?;
        let mut reader = Store::open_reader(directory.path(), [7; 32])?;
        assert!(
            reader
                .store
                .save_snapshot(&serde_json::json!({"synthetic":true}))
                .is_err()
        );
        assert!(
            reader
                .store
                .connection
                .execute("DELETE FROM settings", [])
                .is_err()
        );
        Ok(())
    }
    #[test]
    fn reader_rejects_non_wal_without_switching_the_database_mode() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let writer = Store::open(directory.path(), [7; 32])?;
        drop(writer);
        let connection = Connection::open(directory.path().join("orbit.sqlite3"))?;
        connection.pragma_update(None, "journal_mode", "DELETE")?;
        assert!(Store::open_reader(directory.path(), [7; 32]).is_err());
        let mode: String = connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        assert_eq!(mode, "delete");
        Ok(())
    }
    #[test]
    fn existing_writer_and_reader_enforce_their_actual_connection_modes() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let _initial = Store::open(directory.path(), [7; 32])?;
        let mut writer = Store::open_existing(directory.path(), [7; 32])?;
        for (name, expected) in [
            ("query_only", 0i64),
            ("synchronous", 2),
            ("foreign_keys", 1),
            ("fullfsync", 1),
            ("checkpoint_fullfsync", 1),
            ("trusted_schema", 0),
        ] {
            let actual: i64 = writer
                .connection
                .pragma_query_value(None, name, |row| row.get(0))?;
            assert_eq!(actual, expected, "{name}");
        }
        writer.save_snapshot(&serde_json::json!({"revision":2}))?;
        let mut reader = Store::open_reader(directory.path(), [7; 32])?;
        assert!(
            reader
                .store
                .save_snapshot(&serde_json::json!({"revision":3}))
                .is_err()
        );
        assert_eq!(
            writer.load_snapshot::<serde_json::Value>()?,
            Some(serde_json::json!({"revision":2}))
        );
        Ok(())
    }
}
