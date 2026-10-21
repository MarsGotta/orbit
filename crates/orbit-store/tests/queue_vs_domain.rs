//! The jobs table mixes daemon queue rows with domain rows, such as a
//! meeting's finalized transcript. Queue traversals must count only queued
//! work, while an oversized queue-kind row stays an explicit error.
//!
//! All texts below are synthetic fixtures; no provider runs here.
#![allow(clippy::unwrap_used)]
use orbit_core::wire::sha256;
use orbit_store::{ChunkMetadata, Error, Job, JobState, Store};
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;
const KEY: [u8; 32] = [0x42; 32];
const DEVICE: &str = "11111111-1111-4111-8111-111111111111";
const RECORDING: &str = "22222222-2222-4222-8222-222222222222";

fn record_meeting(store: &mut Store) -> TestResult {
    let mut segments = Vec::new();
    for index in 0..20 {
        let chunk_id = format!("chunk-{index:03}");
        let chunk = store.seal_chunk(
            &ChunkMetadata {
                id: chunk_id.clone(),
                recording_id: RECORDING.into(),
                track: "mic_local".into(),
                capture_epoch_id: None,
                sequence: index as u64,
                sample_rate: 16000,
                channels: 1,
                frames: 1,
                monotonic_ns: index as u64,
                first_sample: index as u64,
                started_at_ms: 1000 + index as i64,
            },
            chunk_id.as_bytes(),
        )?;
        let sources =
            json!([{"chunk_id":chunk_id,"sequence":index,"sha256":chunk.sha256,"frames":1}]);
        let group_id = format!(
            "group_{}",
            sha256(&serde_json::to_vec(
                &json!({"device_id":DEVICE,"recording_id":RECORDING,
                "track":"mic_local","sources":sources,"pipeline_version":1})
            )?)
        );
        let text = format!("g{index:03}: {}\n", "x".repeat(8 * 1024 - 8));
        segments.push(
            json!({"group_id":group_id,"track":"mic_local","sequence":index,
            "last_sequence":index,"source_chunk_ids":[chunk_id],"sources":sources,
            "timing_precision":"chunk","speaker":"unknown","pipeline_version":1,"frames":1,
            "flush_reason":"legacy","state":"completed","error_code":null,"text":text,
            "chunk_id":chunk_id,"source_sha256":chunk.sha256}),
        );
    }
    let manifest = json!({"tracks":[{"track":"mic_local","chunk_count":20,"last_sequence":19,
        "total_frames":20}],"gaps":0,"started_at_ms":1000,"finalized_at_ms":1900});
    for job in [
        Job {
            id: format!("finalize-{RECORDING}"),
            kind: "recording_finalize".into(),
            state: JobState::Succeeded,
            created_at: 1900,
            payload: json!({"recording_id":RECORDING,"manifest":manifest}),
        },
        Job {
            id: format!("transcript-{RECORDING}"),
            kind: "transcript".into(),
            state: JobState::Succeeded,
            created_at: 1901,
            payload: json!({"recording_id":RECORDING,"publication":"local_draft",
                "pending_chunks":0,"capture_finalized":true,"capture_complete":false,
                "transcript_ready":true,"manifest":manifest,"missing_tracks":["remote_app"],
                "segments":segments}),
        },
    ] {
        store.enqueue_job(&job)?;
    }
    Ok(())
}

fn probe_chunk(sequence: u64) -> ChunkMetadata {
    ChunkMetadata {
        id: format!("probe-{sequence:03}"),
        recording_id: "probe-recording".into(),
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
fn oversized_queue_kind_row_is_still_an_explicit_error() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    store.enqueue_job(&Job {
        id: "upload-corrupt-probe".into(),
        kind: "chunk_upload".into(),
        payload: json!({"probe":true}),
        state: JobState::Queued,
        created_at: 1,
    })?;
    // Simulate on-disk corruption: a queue row above the payload limit must
    // stay an explicit error instead of being skipped like a domain row.
    let database = rusqlite::Connection::open(directory.path().join("orbit.sqlite3"))?;
    database.execute(
        "UPDATE jobs SET payload = payload || zeroblob(1048576) \
         WHERE id = 'upload-corrupt-probe'",
        [],
    )?;
    assert!(matches!(
        store.list_jobs_after(None, 100),
        Err(Error::Database(_))
    ));
    Ok(())
}

#[test]
fn pending_count_ignores_domain_rows_and_counts_only_queued_uploads() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut store = Store::open(directory.path(), KEY)?;
    record_meeting(&mut store)?;
    // Retire the uploads the meeting fixture queued; only domain rows remain.
    for index in 0..20 {
        store.update_job(&format!("upload-chunk-{index:03}"), JobState::Cancelled)?;
    }
    assert_eq!(store.diagnostics()?.pending_jobs, 0);
    store.seal_chunk(&probe_chunk(0), b"synthetic audio")?;
    assert_eq!(store.diagnostics()?.pending_jobs, 1);
    Ok(())
}
