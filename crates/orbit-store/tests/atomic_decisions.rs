//! Persist-before-effects boundary: immutable work and its snapshot must commit
//! together, including duplicate work whose processing has already advanced.
use orbit_store::{Error, Job, JobState, Store};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;
const KEY: [u8; 32] = [0x42; 32];

fn closure() -> Job {
    Job {
        id: "finalize-old".into(),
        kind: "recording_finalize".into(),
        payload: json!({"recording_id":"old","frames":100}),
        state: JobState::Queued,
        created_at: 5,
    }
}

#[test]
fn second_write_failure_rolls_back_the_job_and_survives_reopen() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    let old = json!({"recording":"old","phase":"interrupted"});
    let next = json!({"recording":"next","phase":"starting"});
    store.save_snapshot(&old)?;
    let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    database.execute_batch(
        "CREATE TRIGGER r2_second_write BEFORE UPDATE ON settings
        WHEN OLD.key = 'snapshot' AND EXISTS (SELECT 1 FROM jobs WHERE id = 'finalize-old')
        BEGIN SELECT RAISE(ABORT, 'synthetic second write failure'); END;",
    )?;
    assert!(store.save_snapshot_with_job(&next, &closure()).is_err());
    assert!(store.get_job("finalize-old")?.is_none());
    assert_eq!(store.load_snapshot::<Value>()?, Some(old.clone()));
    drop(store);
    let mut store = Store::open(directory.path(), KEY)?;
    assert!(store.get_job("finalize-old")?.is_none());
    assert_eq!(store.load_snapshot::<Value>()?, Some(old));
    database.execute_batch("DROP TRIGGER r2_second_write")?;
    store.save_snapshot_with_job(&next, &closure())?;
    drop(store);
    let store = Store::open(directory.path(), KEY)?;
    assert_eq!(store.get_job("finalize-old")?, Some(closure()));
    assert_eq!(store.load_snapshot::<Value>()?, Some(next));
    Ok(())
}

#[test]
fn failed_job_insert_never_advances_the_snapshot() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    let old = json!({"recording":"old"});
    store.save_snapshot(&old)?;
    let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    let encrypted: Vec<u8> = database.query_row(
        "SELECT value FROM settings WHERE key='snapshot'",
        [],
        |row| row.get(0),
    )?;
    database.execute_batch(
        "CREATE TRIGGER r2_first_write BEFORE INSERT ON jobs
        BEGIN SELECT RAISE(ABORT, 'synthetic first write failure'); END;",
    )?;
    assert!(
        store
            .save_snapshot_with_job(&json!({"recording":"next"}), &closure())
            .is_err()
    );
    assert!(store.get_job("finalize-old")?.is_none());
    assert_eq!(store.load_snapshot::<Value>()?, Some(old));
    let unchanged: Vec<u8> = database.query_row(
        "SELECT value FROM settings WHERE key='snapshot'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(unchanged, encrypted);
    Ok(())
}

#[test]
fn existing_jobs_keep_advanced_state_on_commit_failure_success_and_replay() -> TestResult {
    for state in [JobState::Running, JobState::Unknown, JobState::Succeeded] {
        let directory = tempfile::tempdir()?;
        let mut store = Store::open(directory.path(), KEY)?;
        let old = json!({"recording":"old"});
        let next = json!({"recording":"next"});
        store.save_snapshot(&old)?;
        store.enqueue_job(&closure())?;
        store.update_job("finalize-old", state)?;
        let existing = store.get_job("finalize-old")?.ok_or("closure absent")?;
        let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
        database.execute_batch(
            "CREATE TRIGGER r2_existing_snapshot BEFORE UPDATE ON settings
            WHEN OLD.key='snapshot' BEGIN SELECT RAISE(ABORT, 'synthetic'); END;",
        )?;
        assert!(store.save_snapshot_with_job(&next, &closure()).is_err());
        assert_eq!(store.load_snapshot::<Value>()?, Some(old));
        assert_eq!(store.get_job("finalize-old")?, Some(existing.clone()));
        database.execute_batch("DROP TRIGGER r2_existing_snapshot")?;
        store.save_snapshot_with_job(&next, &closure())?;
        store.save_snapshot_with_job(&next, &closure())?;
        assert_eq!(store.load_snapshot::<Value>()?, Some(next.clone()));
        assert_eq!(store.get_job("finalize-old")?, Some(existing));
        let mut changed = closure();
        changed.payload["frames"] = 200.into();
        assert!(matches!(
            store.save_snapshot_with_job(&json!({"recording":"uncommitted"}), &changed),
            Err(Error::Conflict)
        ));
        assert_eq!(store.load_snapshot::<Value>()?, Some(next));
    }
    Ok(())
}

#[test]
fn authenticated_comparison_never_replaces_corrupt_snapshot_or_job_indices() -> TestResult {
    for target in ["snapshot", "kind", "state", "created_at"] {
        let directory = tempfile::tempdir()?;
        let mut store = Store::open(directory.path(), KEY)?;
        store.save_snapshot(&json!({"recording":"old"}))?;
        let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
        if target == "snapshot" {
            database.execute_batch(
                "UPDATE settings SET value=zeroblob(length(value)) WHERE key='snapshot'",
            )?;
        } else {
            store.enqueue_job(&closure())?;
            database.execute_batch(match target {
                "kind" => "UPDATE jobs SET kind='foreign' WHERE id='finalize-old'",
                "state" => "UPDATE jobs SET state='succeeded' WHERE id='finalize-old'",
                _ => "UPDATE jobs SET created_at=6 WHERE id='finalize-old'",
            })?;
        }
        assert!(
            matches!(
                store.save_snapshot_with_job(&json!({"recording":"next"}), &closure()),
                Err(Error::Authentication)
            ),
            "{target}"
        );
        if target == "snapshot" {
            assert!(
                store.get_job("finalize-old")?.is_none(),
                "the inserted job must roll back on failed snapshot authentication"
            );
        } else {
            assert_eq!(
                store.load_snapshot::<Value>()?,
                Some(json!({"recording":"old"}))
            );
        }
    }
    Ok(())
}
