use orbit_store::{ChunkMetadata, Error, JobState, Store};
use std::{error::Error as StdError, fs};

type TestResult = Result<(), Box<dyn StdError>>;
const KEY: [u8; 32] = [0x42; 32];

fn chunk(id: &str, sequence: u64) -> ChunkMetadata {
    ChunkMetadata {
        id: id.into(),
        recording_id: "meeting".into(),
        track: "mic_local".into(),
        capture_epoch_id: Some("synthetic-epoch".into()),
        sequence,
        sample_rate: 8000,
        channels: 1,
        frames: 8000,
        monotonic_ns: 0,
        first_sample: sequence * 8000,
        started_at_ms: 1,
    }
}

#[test]
fn a_commit_failure_after_unlink_leaves_a_retryable_confirmation_without_reuploading() -> TestResult
{
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    let metadata = chunk("source", 0);
    store.seal_chunk(&metadata, b"synthetic")?;
    store.update_job("upload-source", JobState::Succeeded)?;
    let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    database.execute_batch(
        "PRAGMA foreign_keys=ON;
        CREATE TABLE cleanup_fault(id TEXT REFERENCES chunks(id) DEFERRABLE INITIALLY DEFERRED);
        INSERT INTO cleanup_fault VALUES('source');",
    )?;
    // A deferred constraint fails COMMIT after the filesystem unlink. The
    // confirmed upload must stay terminal; recovery must never requeue it.
    assert!(matches!(
        store.prune_uploaded_chunk("source"),
        Err(Error::Database(_))
    ));
    assert!(!directory.path().join("chunks/source.orbit").exists());
    assert!(store.chunk_receipt("source")?.is_some());
    assert_eq!(
        store.get_job("upload-source")?.ok_or("job missing")?.state,
        JobState::Succeeded
    );
    drop(store);
    database.execute_batch("DROP TABLE cleanup_fault")?;
    let mut store = Store::open(directory.path(), KEY)?;
    let mut scan = store.begin_recovery()?;
    let report = store.recover_next(&mut scan, 10)?;
    assert_eq!((report.recovered, report.missing), (0, 0));
    assert_eq!(store.recover_upload_jobs()?, 0);
    assert_eq!(store.prune_uploaded_chunk("source")?, Some(0));
    assert!(store.chunk_receipt("source")?.is_none());
    assert_eq!(store.finalize_counts("meeting")?, (1, 1));
    assert!(
        store
            .pending_jobs_of_kind("chunk_upload", None, 10)?
            .is_empty()
    );
    assert_eq!(
        store
            .recover_next(&mut store.begin_recovery()?, 10)?
            .missing,
        0
    );
    Ok(())
}

#[test]
fn missing_audio_requires_a_matching_confirmed_upload_to_avoid_a_loss_report() -> TestResult {
    for (name, state, keep_job, matching_receipt, expected_missing) in [
        ("queued", JobState::Queued, true, true, 1),
        ("running", JobState::Running, true, true, 1),
        ("retry", JobState::RetryWait, true, true, 1),
        ("unknown", JobState::Unknown, true, true, 1),
        ("blocked", JobState::Blocked, true, true, 1),
        ("cancelled", JobState::Cancelled, true, true, 1),
        ("no-job", JobState::Queued, false, true, 1),
        ("different-receipt", JobState::Succeeded, true, false, 1),
        ("confirmed", JobState::Succeeded, true, true, 0),
    ] {
        let directory = tempfile::tempdir()?;
        let mut store = Store::open(directory.path(), KEY)?;
        store.seal_chunk(&chunk("source", 0), b"synthetic")?;
        store.update_job("upload-source", state)?;
        if !keep_job || !matching_receipt {
            let mut job = store.get_job("upload-source")?.ok_or("job missing")?;
            let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
            database.execute("DELETE FROM jobs WHERE id='upload-source'", [])?;
            if keep_job {
                job.payload["sha256"] = serde_json::json!("different-hash");
                store.enqueue_job(&job)?;
            }
        }
        fs::remove_file(directory.path().join("chunks/source.orbit"))?;
        drop(store);
        let mut store = Store::open(directory.path(), KEY)?;
        let report = store.recover_next(&mut store.begin_recovery()?, 1)?;
        assert_eq!(report.missing, expected_missing, "{name}");
        assert_eq!(
            (
                report.recovered,
                report.invalid,
                report.incomplete_temporary
            ),
            (0, 0, 0),
            "{name}"
        );
        assert!(store.chunk_receipt("source")?.is_some(), "{name}");
        let rescanned = store.recover_next(&mut store.begin_recovery()?, 1)?;
        assert_eq!(
            rescanned.missing, expected_missing,
            "{name}: reopening a scan must preserve the distinction"
        );
    }
    Ok(())
}

#[test]
fn receipt_only_rows_preserve_paging_numeric_frontier_and_seal_idempotence() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    let latest = store.seal_chunk(&chunk("a-latest", 10), b"latest")?;
    store.seal_chunk(&chunk("z-previous", 9), b"previous")?;
    store.update_job("upload-a-latest", JobState::Succeeded)?;
    let path = directory.path().join("chunks/a-latest.orbit");
    let encrypted_bytes = fs::metadata(&path)?.len();
    assert_eq!(
        store.prune_uploaded_chunk("a-latest")?,
        Some(encrypted_bytes)
    );
    assert_eq!(store.prune_uploaded_chunk("a-latest")?, None);
    assert_eq!(store.list_chunks("meeting", None, 1)?, vec![latest.clone()]);
    assert_eq!(
        store.list_chunks("meeting", Some("a-latest"), 1)?[0]
            .metadata
            .id,
        "z-previous"
    );
    assert!(store.list_chunks("foreign", None, 1)?.is_empty());
    assert_eq!(
        store.latest_chunk_receipt("meeting", "mic_local")?,
        Some(latest.clone())
    );
    assert_eq!(store.finalize_counts("meeting")?, (2, 1));
    assert_eq!(store.diagnostics()?.chunk_count, 1);
    assert_eq!(store.seal_chunk(&latest.metadata, b"latest")?, latest);
    assert!(
        !path.exists(),
        "a delayed seal must not recreate confirmed audio"
    );
    assert!(matches!(
        store.seal_chunk(&latest.metadata, b"changed"),
        Err(Error::Conflict)
    ));
    assert!(matches!(
        store.seal_chunk(&chunk("duplicate-sequence", 10), b"other"),
        Err(Error::Conflict)
    ));
    assert!(
        store.get_job("a-latest")?.is_none(),
        "a receipt is not a job"
    );
    store.enqueue_job(&orbit_store::Job {
        id: "finalize-meeting".into(),
        kind: "recording_finalize".into(),
        state: JobState::Queued,
        created_at: 1,
        payload: serde_json::json!({"recording_id":"meeting"}),
    })?;
    store.update_job("upload-z-previous", JobState::Succeeded)?;
    store.update_job("finalize-meeting", JobState::Succeeded)?;
    store.prune_uploaded_chunk("z-previous")?;
    assert!(
        store.list_chunks("meeting", None, 10)?.is_empty(),
        "closed recordings need no local receipts"
    );
    assert_eq!(store.finalize_counts("meeting")?, (0, 0));
    Ok(())
}

#[test]
fn a_forged_success_index_and_a_forged_frontier_are_not_authority() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    store.seal_chunk(&chunk("source", 0), b"synthetic")?;
    let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    database.execute(
        "UPDATE jobs SET state='succeeded' WHERE id='upload-source'",
        [],
    )?;
    assert!(matches!(
        store.prune_uploaded_chunk("source"),
        Err(Error::Authentication)
    ));
    assert_eq!(store.read_chunk("source")?, b"synthetic");
    database.execute(
        "UPDATE jobs SET state='queued' WHERE id='upload-source'",
        [],
    )?;
    store.update_job("upload-source", JobState::Succeeded)?;
    store.prune_uploaded_chunk("source")?;
    database.execute(
        "UPDATE records SET created_at=99 WHERE namespace='uploaded_chunks:meeting'",
        [],
    )?;
    assert!(matches!(
        store.latest_chunk_receipt("meeting", "mic_local"),
        Err(Error::Authentication)
    ));
    Ok(())
}
