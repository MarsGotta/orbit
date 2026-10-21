#[path = "support/integrity.rs"]
mod integrity_support;
#[path = "support/recovery.rs"]
mod recovery_support;
use orbit_store::{ChunkMetadata, Error, Job, JobState, Store};
use serde_json::{Value, json};
use std::{error::Error as StdError, fs};

type TestResult = Result<(), Box<dyn StdError>>;
const KEY: [u8; 32] = [0x42; 32];

#[test]
fn unchanged_snapshots_do_not_rewrite_ciphertext_or_wal() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    let snapshot = json!({"privacy_stopped":true,"text":"synthetic".repeat(8192)});
    store.save_snapshot(&snapshot)?;
    let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    let cipher = || {
        database.query_row::<Vec<u8>, _, _>(
            "SELECT value FROM settings WHERE key = 'snapshot'",
            [],
            |row| row.get(0),
        )
    };
    let original = cipher()?;
    let wal = fs::read(directory.path().join("orbit.sqlite3-wal"))?;
    for _ in 0..20 {
        store.save_snapshot(&snapshot)?;
    }
    assert_eq!(cipher()?, original);
    assert_eq!(fs::read(directory.path().join("orbit.sqlite3-wal"))?, wal);
    store.save_snapshot(&json!({"privacy_stopped":false}))?;
    assert_ne!(cipher()?, original);
    store.save_snapshot(&snapshot)?;
    assert_ne!(
        cipher()?,
        original,
        "a changed state must use fresh encryption"
    );
    // Authenticated comparison must notice modification by another connection.
    database.execute(
        "UPDATE settings SET value = zeroblob(length(value)) WHERE key = 'snapshot'",
        [],
    )?;
    assert!(matches!(
        store.save_snapshot(&snapshot),
        Err(Error::Authentication)
    ));
    Ok(())
}

fn chunk(id: &str, sequence: u64) -> ChunkMetadata {
    ChunkMetadata {
        id: id.into(),
        recording_id: "meeting-one".into(),
        track: "mic_local".into(),
        capture_epoch_id: None,
        sequence,
        sample_rate: 48_000,
        channels: 1,
        frames: 4,
        monotonic_ns: 5_000_000,
        first_sample: sequence * 4,
        started_at_ms: 1000,
    }
}

#[test]
fn the_snapshot_survives_restart_without_plaintext() -> TestResult {
    let directory = tempfile::tempdir()?;
    let secret = "SYNTHETIC_PRIVATE_CONTENT_ORBIT_7b90";
    {
        let mut store = Store::open(directory.path(), KEY)?;
        assert!(store.load_snapshot::<Value>()?.is_none());
        store.save_snapshot(&json!({"text": secret, "privacy_stopped": true}))?;
        integrity_support::assert_integrity(directory.path())?;
        for entry in fs::read_dir(directory.path())? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let bytes = fs::read(entry.path())?;
                assert!(
                    !bytes
                        .windows(secret.len())
                        .any(|window| window == secret.as_bytes())
                );
            }
        }
    }
    let store = Store::open(directory.path(), KEY)?;
    assert_eq!(
        store.load_snapshot::<Value>()?.ok_or("missing snapshot")?["text"],
        secret
    );
    assert!(matches!(
        Store::open(directory.path(), [9; 32]),
        Err(Error::Authentication)
    ));
    Ok(())
}

/// SIM-30: the copy of the conversations an older daemon kept here goes, rows
/// and table, when the store opens; the rest of the store stays.
#[test]
fn opening_drops_the_conversation_copy_an_older_daemon_wrote() -> TestResult {
    let directory = tempfile::tempdir()?;
    {
        let mut store = Store::open(directory.path(), KEY)?;
        store.save_snapshot(&json!({"privacy_stopped": false}))?;
    }
    {
        let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
        database.execute_batch(
            "CREATE TABLE messages (id TEXT PRIMARY KEY NOT NULL, session_id TEXT NOT NULL,
             role TEXT NOT NULL, created_at INTEGER NOT NULL, payload BLOB NOT NULL);
             INSERT INTO messages VALUES ('one', 'session-one', 'user', 1, x'00');",
        )?;
    }
    let store = Store::open(directory.path(), KEY)?;
    let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    let tables: i64 = database.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE name = 'messages'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(tables, 0, "the messages table is still there");
    assert!(store.load_snapshot::<Value>()?.is_some());
    Ok(())
}

#[test]
fn job_replay_never_resurrects_completed_or_unknown_work() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    assert!(store.get_job("absent")?.is_none());
    let job = Job {
        id: "job-one".into(),
        kind: "publish".into(),
        payload: json!({"private": "draft"}),
        state: JobState::Queued,
        created_at: 5,
    };
    store.enqueue_job(&job)?;
    assert_eq!(store.get_job(&job.id)?, Some(job.clone()));
    store.update_job(&job.id, JobState::Unknown)?;
    assert_eq!(
        store.get_job(&job.id)?.ok_or("missing job")?.state,
        JobState::Unknown
    );
    store.enqueue_job(&job)?;
    assert_eq!(store.list_jobs(10)?[0].state, JobState::Unknown);
    store.update_job(&job.id, JobState::Succeeded)?;
    store.enqueue_job(&job)?;
    assert_eq!(store.list_jobs(10)?[0].state, JobState::Succeeded);
    let mut changed = job;
    changed.payload = json!({"different": true});
    assert!(matches!(store.enqueue_job(&changed), Err(Error::Conflict)));
    Ok(())
}

#[test]
fn chunks_are_encrypted_deduplicated_and_checked_on_read() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    let metadata = chunk("chunk-one", 0);
    let audio = b"SYNTHETIC-PCM-NOT-REAL-RECORDING";
    let receipt = store.seal_chunk(&metadata, audio)?;
    assert_eq!(store.seal_chunk(&metadata, audio)?, receipt);
    assert_eq!(store.read_chunk(&metadata.id)?, audio);
    assert_eq!(store.list_jobs(10)?.len(), 1);
    assert_eq!(store.diagnostics()?.chunk_count, 1);
    assert!(matches!(
        store.seal_chunk(&metadata, b"changed"),
        Err(Error::Conflict)
    ));
    assert!(matches!(
        store.seal_chunk(&chunk("other-id", 0), audio),
        Err(Error::Conflict)
    ));
    let path = directory.path().join("chunks/chunk-one.orbit");
    let mut sealed = fs::read(&path)?;
    assert!(!sealed.windows(audio.len()).any(|window| window == audio));
    let last = sealed.last_mut().ok_or("empty blob")?;
    *last ^= 1;
    fs::write(path, sealed)?;
    assert!(matches!(
        store.read_chunk(&metadata.id),
        Err(Error::Authentication)
    ));
    assert_eq!(recovery_support::recover_page(&mut store, 100)?.invalid, 2);
    Ok(())
}

#[test]
fn orphan_recovery_reconstructs_manifest_and_outbox_idempotently() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    store.seal_chunk(&chunk("chunk-one", 0), b"synthetic audio")?;
    drop(store);
    // A durable blob without either row represents the crash window between
    // file sync and the single SQLite transaction. This is not a power-loss test.
    let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    database.execute_batch("BEGIN; DELETE FROM chunks; DELETE FROM jobs; COMMIT;")?;
    drop(database);
    let mut recovered = Store::open(directory.path(), KEY)?;
    let report = recovery_support::recover_page(&mut recovered, 100)?;
    assert_eq!(report.recovered, 1);
    assert_eq!(
        recovery_support::recover_page(&mut recovered, 100)?.recovered,
        0
    );
    assert_eq!(recovered.list_jobs(10)?.len(), 1);
    assert_eq!(recovered.read_chunk("chunk-one")?, b"synthetic audio");
    integrity_support::assert_integrity(directory.path())?;
    Ok(())
}

#[test]
fn oversized_and_unsafe_input_fails_before_durable_acceptance() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    assert!(matches!(
        store.list_chunks("../outside", None, 1),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        store.list_jobs(501),
        Err(Error::CountOutOfRange {
            count: 501,
            min_count: 1,
            max_count: orbit_store::MAX_PAGE_SIZE,
            ..
        })
    ));
    assert!(matches!(
        store.save_snapshot(&"x".repeat(orbit_store::MAX_PAYLOAD_BYTES + 1)),
        Err(Error::TooLarge { bytes, max_bytes, .. })
            if bytes == orbit_store::MAX_PAYLOAD_BYTES as u64 + 3
                && max_bytes == orbit_store::MAX_PAYLOAD_BYTES as u64
    ));
    assert!(matches!(
        store.seal_chunk(
            &chunk("too-big", 0),
            &vec![0; orbit_store::MAX_CHUNK_BYTES + 1]
        ),
        Err(Error::TooLarge { bytes, max_bytes, .. })
            if bytes == orbit_store::MAX_CHUNK_BYTES as u64 + 1
                && max_bytes == orbit_store::MAX_CHUNK_BYTES as u64
    ));
    assert_eq!(store.diagnostics()?.chunk_count, 0);
    Ok(())
}

#[test]
fn damaged_snapshot_is_not_treated_as_a_fresh_install() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    store.save_snapshot(&json!({"privacy_stopped": true}))?;
    let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    database.execute(
        "UPDATE settings SET value = x'000000' WHERE key = 'snapshot'",
        [],
    )?;
    assert!(matches!(
        store.load_snapshot::<Value>(),
        Err(Error::Authentication)
    ));
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlink_storage_is_rejected_and_permissions_are_private() -> TestResult {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory = tempfile::tempdir()?;
    let actual = directory.path().join("actual");
    fs::create_dir(&actual)?;
    let linked = directory.path().join("linked");
    symlink(&actual, &linked)?;
    assert!(matches!(Store::open(&linked, KEY), Err(Error::UnsafePath)));
    let _store = Store::open(&actual, KEY)?;
    assert_eq!(fs::metadata(&actual)?.permissions().mode() & 0o777, 0o700);
    assert_eq!(
        fs::metadata(actual.join("orbit.sqlite3"))?
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    integrity_support::assert_integrity(&actual)?;
    Ok(())
}

#[test]
fn sqlite_runtime_evidence_records_durable_configuration() -> TestResult {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path(), KEY)?;
    let diagnostics = store.diagnostics()?;
    assert_eq!(diagnostics.journal_mode, "wal");
    assert_eq!(diagnostics.synchronous, 2);
    assert_eq!(diagnostics.foreign_keys, 1);
    assert_eq!(diagnostics.fullfsync, 1);
    assert_eq!(diagnostics.schema_version, 4);
    assert!(!diagnostics.sqlite_source_id.is_empty());
    eprintln!(
        "sqlite={} source={} mode={} sync={}",
        diagnostics.sqlite_version,
        diagnostics.sqlite_source_id,
        diagnostics.journal_mode,
        diagnostics.synchronous
    );
    Ok(())
}
