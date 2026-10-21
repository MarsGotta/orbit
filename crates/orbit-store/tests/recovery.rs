#[path = "support/recovery.rs"]
mod recovery_support;
use orbit_store::{ChunkMetadata, Error, Job, JobState, Store};
use serde_json::json;
use std::{error::Error as StdError, fs};

type TestResult = Result<(), Box<dyn StdError>>;
const KEY: [u8; 32] = [0x42; 32];

#[test]
fn incremental_recovery_reaches_orphans_and_missing_blobs_beyond_500() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    for sequence in 0..503 {
        store.seal_chunk(
            &chunk(&format!("chunk-{sequence:04}"), sequence),
            b"synthetic audio",
        )?;
    }
    let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    // Every original is an orphan: enumeration order cannot make this pass
    // accidentally by putting the interesting file inside the first batch.
    database.execute("DELETE FROM chunks WHERE id <> 'chunk-0502'", [])?;
    database.execute("DELETE FROM jobs WHERE id <> 'upload-chunk-0502'", [])?;
    fs::remove_file(directory.path().join("chunks/chunk-0502.orbit"))?;
    fs::write(
        directory.path().join("chunks/.pending-synthetic"),
        b"incomplete",
    )?;
    fs::write(directory.path().join("chunks/not-a-chunk"), b"invalid")?;
    let mut scan = store.begin_recovery()?;
    let mut recovered = 0;
    let mut missing = 0;
    let mut invalid = 0;
    let mut incomplete = 0;
    let mut batches = 0;
    loop {
        let report = store.recover_next(&mut scan, 127)?;
        batches += 1;
        assert!(report.inspected <= 127);
        recovered += report.recovered;
        missing += report.missing;
        invalid += report.invalid;
        incomplete += report.incomplete_temporary;
        if !report.limit_reached {
            break;
        }
        assert!(batches < 20, "recovery must make forward progress");
    }
    assert!(batches >= 4);
    assert_eq!((recovered, missing, invalid, incomplete), (502, 1, 1, 1));
    assert_eq!(store.diagnostics()?.chunk_count, 503);
    assert_eq!(
        store
            .pending_jobs_of_kind("chunk_upload", Some("upload-chunk-0500"), 10)?
            .len(),
        2
    );
    let eof = store.recover_next(&mut scan, 127)?;
    assert_eq!((eof.inspected, eof.recovered, eof.missing), (0, 0, 0));
    assert!(!eof.limit_reached);
    Ok(())
}

fn chunk(id: &str, sequence: u64) -> ChunkMetadata {
    ChunkMetadata {
        id: id.into(),
        recording_id: "recording".into(),
        track: "mic_local".into(),
        capture_epoch_id: None,
        sequence,
        sample_rate: 48_000,
        channels: 1,
        frames: 4,
        monotonic_ns: 1000,
        first_sample: sequence * 4,
        started_at_ms: 1000,
    }
}

#[test]
fn completed_jobs_do_not_hide_pending_pages() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    for id in ["a", "b", "c", "d"] {
        store.enqueue_job(&Job {
            id: id.into(),
            kind: "chunk_upload".into(),
            payload: json!({}),
            state: if id == "d" {
                JobState::Queued
            } else {
                JobState::Succeeded
            },
            created_at: 1,
        })?;
    }
    assert_eq!(store.list_jobs(2)?.len(), 2);
    let pending = store.pending_jobs_of_kind("chunk_upload", None, 2)?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "d");
    let first = store.list_jobs_after(None, 2)?;
    let second = store.list_jobs_after(Some(&first[1].id), 2)?;
    assert_eq!(
        first.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(
        second.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(),
        ["c", "d"]
    );
    assert!(store.list_jobs_after(Some("d"), 2)?.is_empty());
    Ok(())
}

#[test]
fn chunk_export_paginates_and_missing_original_is_reported() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    for (sequence, id) in ["a", "b", "c"].into_iter().enumerate() {
        store.seal_chunk(&chunk(id, sequence as u64), b"synthetic audio")?;
    }
    assert_eq!(store.list_chunks("recording", None, 2)?.len(), 2);
    assert_eq!(
        store.list_chunks("recording", Some("b"), 2)?[0].metadata.id,
        "c"
    );
    assert!(store.list_chunks("another", None, 2)?.is_empty());
    fs::remove_file(directory.path().join("chunks/b.orbit"))?;
    assert_eq!(recovery_support::recover_page(&mut store, 100)?.missing, 1);
    assert!(matches!(store.read_chunk("b"), Err(Error::CorruptChunk)));
    Ok(())
}

#[test]
fn interrupted_upload_recovery_never_retries_unknown_desktop_effects() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    store.seal_chunk(&chunk("one", 0), b"synthetic audio")?;
    store.update_job("upload-one", JobState::Unknown)?;
    store.enqueue_job(&Job {
        id: "desktop-action".into(),
        kind: "desktop_input".into(),
        payload: json!({}),
        state: JobState::Unknown,
        created_at: 5,
    })?;
    assert_eq!(store.recover_upload_jobs()?, 1);
    assert_eq!(store.recover_upload_jobs()?, 0);
    assert_eq!(
        store.get_job("upload-one")?.ok_or("missing upload")?.state,
        JobState::RetryWait
    );
    assert_eq!(
        store
            .get_job("desktop-action")?
            .ok_or("missing desktop action")?
            .state,
        JobState::Unknown
    );
    Ok(())
}

#[test]
fn plaintext_state_tampering_cannot_reauthorize_a_job() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    store.enqueue_job(&Job {
        id: "complete".into(),
        kind: "desktop_input".into(),
        payload: json!({}),
        state: JobState::Succeeded,
        created_at: 5,
    })?;
    let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    database.execute("UPDATE jobs SET state = 'queued' WHERE id = 'complete'", [])?;
    assert!(matches!(
        store.pending_jobs_of_kind("desktop_input", None, 10),
        Err(Error::Authentication)
    ));
    Ok(())
}

#[test]
fn pending_job_kind_and_cursor_are_authenticated_without_other_queue_starvation() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    for (id, kind, state) in [
        ("a", "chunk_upload", JobState::Queued),
        ("b", "transcript_fetch", JobState::Running),
        ("c", "transcript_fetch", JobState::RetryWait),
        ("d", "transcript_fetch", JobState::Succeeded),
    ] {
        store.enqueue_job(&Job {
            id: id.into(),
            kind: kind.into(),
            state,
            created_at: 1,
            payload: json!({}),
        })?;
    }
    assert_eq!(
        store.pending_jobs_of_kind("transcript_fetch", None, 1)?[0].id,
        "b"
    );
    assert_eq!(
        store.pending_jobs_of_kind("transcript_fetch", Some("b"), 10)?[0].id,
        "c"
    );
    assert!(
        store
            .pending_jobs_of_kind("transcript_fetch", Some("c"), 10)?
            .is_empty()
    );
    let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    database.execute("UPDATE jobs SET kind='transcript_fetch' WHERE id='a'", [])?;
    assert!(matches!(
        store.pending_jobs_of_kind("transcript_fetch", None, 10),
        Err(Error::Authentication)
    ));
    Ok(())
}

#[cfg(unix)]
#[test]
fn unreadable_blob_file_is_counted_invalid_and_recovery_completes() -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    for sequence in 0..3 {
        store.seal_chunk(
            &chunk(&format!("chunk-{sequence:04}"), sequence),
            b"synthetic audio",
        )?;
    }
    // A blob present on disk but unreadable (EACCES) must not abort the scan:
    // it is reported as invalid and the remaining files are still processed.
    let unreadable = directory.path().join("chunks/chunk-0001.orbit");
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000))?;
    let report = recovery_support::recover_page(&mut store, 100)?;
    assert_eq!(report.inspected, 3);
    assert_eq!(report.invalid, 1);
    assert_eq!(report.recovered, 0);
    assert_eq!(report.missing, 0);
    assert!(!report.limit_reached);
    Ok(())
}

#[test]
fn byte_bounded_pages_continue_without_truncating_large_jobs() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    let large = "x".repeat(orbit_store::MAX_PAYLOAD_BYTES - 512);
    for index in 0..10 {
        store.enqueue_job(&Job {
            id: format!("job-{index}"),
            kind: "chunk_upload".into(),
            payload: json!({"text":large}),
            state: JobState::Queued,
            created_at: 1,
        })?;
    }
    let first = store.list_jobs_after(None, 100)?;
    assert!(!first.is_empty() && first.len() < 10);
    let second = store.list_jobs_after(first.last().map(|job| job.id.as_str()), 100)?;
    assert_eq!(first.len() + second.len(), 10);
    for job in first.iter().chain(&second) {
        assert_eq!(job.payload["text"].as_str(), Some(large.as_str()));
    }
    let typed = store.pending_jobs_of_kind("chunk_upload", None, 100)?;
    assert_eq!(typed.len(), first.len());
    assert_eq!(
        store
            .pending_jobs_of_kind("chunk_upload", typed.last().map(|job| job.id.as_str()), 100)?
            .len(),
        second.len()
    );
    Ok(())
}

#[test]
fn oversized_blob_is_counted_invalid_and_other_chunks_are_recovered() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    store.seal_chunk(&chunk("valid", 0), b"synthetic audio")?;
    let oversized = directory.path().join("chunks/oversized.orbit");
    let file = fs::File::create(&oversized)?;
    file.set_len((2 * orbit_store::MAX_CHUNK_BYTES) as u64)?;
    let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    database.execute_batch("DELETE FROM chunks; DELETE FROM jobs;")?;
    let report = recovery_support::recover_page(&mut store, 100)?;
    assert_eq!(report.inspected, 2);
    assert_eq!(report.invalid, 1);
    assert_eq!(report.recovered, 1);
    assert_eq!(report.missing, 0);
    assert!(!report.limit_reached);
    assert!(oversized.exists());
    assert_eq!(store.read_chunk("valid")?, b"synthetic audio");
    assert_eq!(
        store.get_job("upload-valid")?.ok_or("missing upload")?.kind,
        "chunk_upload"
    );
    Ok(())
}
