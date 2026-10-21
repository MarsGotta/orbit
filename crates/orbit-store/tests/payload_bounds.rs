//! Size refusals at the real serialization and SQLite boundaries.
use orbit_store::{Error, Job, JobState, MAX_PAYLOAD_BYTES, Store};
use serde_json::json;
type Result = std::result::Result<(), Box<dyn std::error::Error>>;

#[test]
fn oversized_snapshot_reports_exact_serialized_bytes_without_writing() -> Result {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), [7; 32])?;
    store.save_snapshot(&json!({"revision": 1}))?;
    let value = "\n".repeat(MAX_PAYLOAD_BYTES);
    let expected = serde_json::to_vec(&value)?.len() as u64;
    let error = store
        .save_snapshot(&value)
        .err()
        .ok_or("accepted oversized snapshot")?;
    assert!(
        matches!(error, Error::TooLarge { bytes, max_bytes, .. }
        if bytes == expected && max_bytes == MAX_PAYLOAD_BYTES as u64),
        "{error:?}"
    );
    assert_eq!(
        store.load_snapshot::<serde_json::Value>()?,
        Some(json!({"revision": 1}))
    );
    Ok(())
}

#[test]
fn oversized_job_reports_exact_serialized_bytes_without_enqueuing() -> Result {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), [7; 32])?;
    let job = Job {
        id: "bounded-job".into(),
        kind: "capture_epoch".into(),
        state: JobState::Queued,
        created_at: 1,
        payload: json!({"text": "ñ".repeat(MAX_PAYLOAD_BYTES)}),
    };
    let expected = serde_json::to_vec(&job)?.len() as u64;
    let error = store
        .enqueue_job(&job)
        .err()
        .ok_or("accepted oversized job")?;
    assert!(
        matches!(error, Error::TooLarge { bytes, max_bytes, .. }
        if bytes == expected && max_bytes == MAX_PAYLOAD_BYTES as u64),
        "{error:?}"
    );
    assert_eq!(store.get_job(&job.id)?, None);
    Ok(())
}

#[test]
fn oversized_sqlite_job_reports_bytes_before_decryption() -> Result {
    let directory = tempfile::tempdir()?;
    let store = Store::open(directory.path(), [7; 32])?;
    let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    // A maximum valid payload's AEAD envelope is measured from an ordinary row,
    // rather than mirroring the cipher's private overhead constant.
    drop(store);
    let mut store = Store::open(directory.path(), [7; 32])?;
    let job = Job {
        id: "bounded-job".into(),
        kind: "capture_epoch".into(),
        state: JobState::Queued,
        created_at: 1,
        payload: json!({}),
    };
    store.enqueue_job(&job)?;
    let sealed: i64 = database.query_row(
        "SELECT length(payload) FROM jobs WHERE id=?1",
        [&job.id],
        |r| r.get(0),
    )?;
    let max = MAX_PAYLOAD_BYTES as u64 + sealed as u64 - serde_json::to_vec(&job)?.len() as u64;
    database.execute(
        "UPDATE jobs SET payload=zeroblob(?1) WHERE id=?2",
        rusqlite::params![(max + 1) as i64, job.id],
    )?;
    let before = store.decryption_count();
    for result in [
        store.get_job(&job.id),
        store.list_jobs_after(None, 10).map(|_| None),
        store
            .pending_jobs_of_kind(&job.kind, None, 10)
            .map(|_| None),
    ] {
        let error = result.err().ok_or("accepted oversized ciphertext")?;
        assert!(
            matches!(error, Error::TooLarge { bytes, max_bytes, .. }
            if bytes == max + 1 && max_bytes == max),
            "{error:?}"
        );
    }
    assert_eq!(store.decryption_count(), before);
    Ok(())
}
