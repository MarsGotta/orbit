//! Latest indexed receipt is authenticated, bounded, and does not invent an
//! epoch or rewrite existing encrypted sources. All fixtures are synthetic.
#[path = "support/recovery.rs"]
mod recovery_support;
use orbit_store::{ChunkMetadata, Error, Store};
use serde_json::json;
use std::{error::Error as StdError, fs};

type TestResult = Result<(), Box<dyn StdError>>;
const KEY: [u8; 32] = [0x42; 32];

fn chunk(id: &str, sequence: u64) -> ChunkMetadata {
    ChunkMetadata {
        id: id.into(),
        recording_id: "recording".into(),
        track: "mic_local".into(),
        capture_epoch_id: Some("capture-epoch".into()),
        sequence,
        sample_rate: 8000,
        channels: 1,
        frames: 8000,
        monotonic_ns: sequence * 1_000_000_000,
        first_sample: sequence * 8000,
        started_at_ms: 1000,
    }
}

#[test]
fn numeric_sequence_selects_the_frontier_not_id_or_native_clock_order() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    let mut previous = chunk("source-9", 9);
    previous.monotonic_ns = 900_000_000_000;
    previous.capture_epoch_id = Some("before-restart".into());
    store.seal_chunk(&previous, b"old synthetic")?;
    let mut current = chunk("source-10", 10);
    current.monotonic_ns = 1_000_000_000;
    current.capture_epoch_id = Some("after-restart".into());
    let receipt = store.seal_chunk(&current, b"new synthetic")?;
    let mut remote = chunk("remote-999", 999);
    remote.track = "remote_app".into();
    let remote_receipt = store.seal_chunk(&remote, b"remote synthetic")?;
    let mut foreign = chunk("foreign-9999", 9999);
    foreign.recording_id = "foreign".into();
    store.seal_chunk(&foreign, b"foreign synthetic")?;
    assert_eq!(
        store.latest_chunk_receipt("recording", "mic_local")?,
        Some(receipt)
    );
    assert_eq!(
        store.latest_chunk_receipt("recording", "remote_app")?,
        Some(remote_receipt)
    );
    assert!(store.latest_chunk_receipt("absent", "mic_local")?.is_none());
    assert!(store.latest_chunk_receipt("recording", "absent")?.is_none());
    Ok(())
}

#[test]
fn selected_index_columns_cannot_disagree_with_the_authenticated_receipt() -> TestResult {
    for alteration in [
        "id",
        "recording_id",
        "track",
        "sequence",
        "bytes",
        "payload",
    ] {
        let directory = tempfile::tempdir()?;
        let mut store = Store::open(directory.path(), KEY)?;
        store.seal_chunk(&chunk("source", 0), b"synthetic")?;
        let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
        let (sql, recording, track) = match alteration {
            "id" => ("UPDATE chunks SET id='forged'", "recording", "mic_local"),
            "recording_id" => (
                "UPDATE chunks SET recording_id='foreign'",
                "foreign",
                "mic_local",
            ),
            "track" => (
                "UPDATE chunks SET track='remote_app'",
                "recording",
                "remote_app",
            ),
            "sequence" => ("UPDATE chunks SET sequence=99", "recording", "mic_local"),
            "bytes" => ("UPDATE chunks SET bytes=bytes+1", "recording", "mic_local"),
            _ => (
                "UPDATE chunks SET payload=zeroblob(length(payload))",
                "recording",
                "mic_local",
            ),
        };
        database.execute(sql, [])?;
        assert!(
            matches!(
                store.latest_chunk_receipt(recording, track),
                Err(Error::Authentication)
            ),
            "untrusted column {alteration} was accepted"
        );
    }
    Ok(())
}

#[test]
fn frontier_rejects_oversized_ciphertext_before_collecting_it() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    let receipt = store.seal_chunk(&chunk("source", 0), b"synthetic")?;
    let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    let size: i64 =
        database.query_row("SELECT length(payload) FROM chunks", [], |row| row.get(0))?;
    let max = 4096 + size as u64 - serde_json::to_vec(&receipt)?.len() as u64;
    let before = store.decryption_count();
    database.execute("UPDATE chunks SET payload=zeroblob(8192)", [])?;
    assert!(matches!(
        store.latest_chunk_receipt("recording", "mic_local"),
        Err(Error::TooLarge { bytes: 8192, max_bytes, .. }) if max_bytes == max
    ));
    assert_eq!(store.decryption_count(), before);
    Ok(())
}

#[test]
fn legacy_epoch_stays_absent_and_reads_do_not_rewrite_encrypted_receipts() -> TestResult {
    let directory = tempfile::tempdir()?;
    let legacy = json!({"id":"legacy-source","recording_id":"recording","track":"mic_local",
        "sequence":0,"sample_rate":8000,"channels":1,"frames":8000,"monotonic_ns":1000,
        "first_sample":0,"started_at_ms":1000});
    let metadata: ChunkMetadata = serde_json::from_value(legacy.clone())?;
    assert!(metadata.capture_epoch_id.is_none());
    assert_eq!(serde_json::to_value(&metadata)?, legacy);
    let mut store = Store::open(directory.path(), KEY)?;
    let receipt = store.seal_chunk(&metadata, b"legacy synthetic")?;
    let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    let manifest_before: Vec<u8> = database.query_row(
        "SELECT payload FROM chunks WHERE id='legacy-source'",
        [],
        |row| row.get(0),
    )?;
    let job_before: Vec<u8> = database.query_row(
        "SELECT payload FROM jobs WHERE id='upload-legacy-source'",
        [],
        |row| row.get(0),
    )?;
    let blob = directory.path().join("chunks/legacy-source.orbit");
    let blob_before = fs::read(&blob)?;
    drop(store);
    let mut reopened = Store::open(directory.path(), KEY)?;
    assert_eq!(
        reopened.latest_chunk_receipt("recording", "mic_local")?,
        Some(receipt.clone())
    );
    assert_eq!(
        recovery_support::recover_page(&mut reopened, 10)?.recovered,
        0
    );
    assert_eq!(
        reopened.seal_chunk(&metadata, b"legacy synthetic")?,
        receipt
    );
    let mut changed = metadata;
    changed.capture_epoch_id = Some("invented-backfill".into());
    assert!(matches!(
        reopened.seal_chunk(&changed, b"legacy synthetic"),
        Err(Error::Conflict)
    ));
    let manifest_after: Vec<u8> = database.query_row(
        "SELECT payload FROM chunks WHERE id='legacy-source'",
        [],
        |row| row.get(0),
    )?;
    let job_after: Vec<u8> = database.query_row(
        "SELECT payload FROM jobs WHERE id='upload-legacy-source'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(manifest_before, manifest_after);
    assert_eq!(job_before, job_after);
    assert_eq!(fs::read(blob)?, blob_before);
    assert!(
        reopened
            .get_job("upload-legacy-source")?
            .ok_or("missing job")?
            .payload["metadata"]
            .get("capture_epoch_id")
            .is_none()
    );
    Ok(())
}

#[test]
fn invalid_capture_epochs_are_rejected_before_any_audio_is_sealed() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    for epoch in [
        String::new(),
        ".".into(),
        "..".into(),
        "../foreign".into(),
        "white space".into(),
        "é".into(),
        "x".repeat(129),
    ] {
        let mut metadata = chunk("source", 0);
        metadata.capture_epoch_id = Some(epoch);
        assert!(matches!(
            store.seal_chunk(&metadata, b"synthetic"),
            Err(Error::Invalid(_))
        ));
    }
    assert_eq!(store.diagnostics()?.chunk_count, 0);
    assert!(matches!(
        store.latest_chunk_receipt("../foreign", "mic_local"),
        Err(Error::Invalid(_))
    ));
    Ok(())
}

#[test]
fn latest_sequence_uses_the_existing_recording_track_index() -> TestResult {
    let directory = tempfile::tempdir()?;
    let _store = Store::open(directory.path(), KEY)?;
    let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    let plan: String = database.query_row(
        "EXPLAIN QUERY PLAN SELECT id,recording_id,track,sequence,bytes,length(payload),payload
         FROM chunks WHERE recording_id=?1 AND track=?2 ORDER BY sequence DESC LIMIT 1",
        ["recording", "mic_local"],
        |row| row.get(3),
    )?;
    assert!(plan.contains("SEARCH chunks USING INDEX"), "{plan}");
    assert!(
        !plan.contains("SCAN") && !plan.contains("TEMP B-TREE"),
        "{plan}"
    );
    Ok(())
}
