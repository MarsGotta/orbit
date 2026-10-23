//! What the hub refuses or cannot store is said with its cause, never holds a
//! meeting's closure for ever, and the backlog goes up in capture order.
//! Synthetic hub and store only.
#![allow(clippy::unwrap_used)]

use super::capability_notice_tests::{TOKEN, json_response, one_shot_http};
use super::*;
use crate::daemon::audio::tests::fixture;
use orbit_core::notice::{DataValue, NoticeClass, NoticeCode, NoticeKey};
use orbit_store::ChunkMetadata;

fn chunk(recording: &str, sequence: u64) -> ChunkMetadata {
    ChunkMetadata {
        id: format!("{recording}-mic_local-{sequence}"),
        recording_id: recording.into(),
        track: "mic_local".into(),
        capture_epoch_id: None,
        sequence,
        sample_rate: 8000,
        channels: 1,
        frames: 1,
        monotonic_ns: 0,
        first_sample: sequence,
        started_at_ms: 1,
    }
}

async fn upload_outcome(runtime: &mut Runtime, id: &str, result: Result<()>) -> Result<()> {
    let (events, mut outcomes) = mpsc::channel(8);
    runtime.work_events = events;
    let generation = runtime.state.generation();
    let ticket =
        crate::daemon::audio::tests::pending_work(&mut runtime.transfers.worker, generation, ())?;
    runtime
        .work_event(WorkResult::Store(crate::daemon::StoreWork::Uploaded {
            ticket,
            job_id: format!("upload-{id}"),
            result,
        }))
        .await?;
    while runtime.transfers.worker.active().is_some() {
        runtime
            .work_event(outcomes.recv().await.context("settlement absent")?)
            .await?;
    }
    Ok(())
}

/// A chunk the hub refuses is said with its status, never as a silent retry:
/// 401 and 404 wait for a fix and are retried; 409 and 413 would get the same
/// answer for the same bytes, so they stay on the Mac without retries.
#[tokio::test]
async fn a_chunk_the_hub_refuses_is_said_with_its_status() -> Result<()> {
    for status in [401_u16, 404, 409, 413] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _) = fixture(directory.path())?;
        let metadata = chunk("reunion-rechazada", 0);
        runtime.fixture_store.seal_chunk(&metadata, b"synthetic")?;
        runtime
            .fixture_store
            .update_job(&format!("upload-{}", metadata.id), JobState::Running)?;
        upload_outcome(
            &mut runtime,
            &metadata.id,
            Err(crate::retry::Refused(status).into()),
        )
        .await?;
        let job = runtime
            .reader
            .get_job(&format!("upload-{}", metadata.id))?
            .context("job lost")?;
        let notice = runtime.state.durable.notice.as_ref();
        let shown = notice.is_some_and(|notice| notice.code.class() != NoticeClass::Silent);
        assert!(
            shown,
            "HTTP {status}: the chunk is now {:?} and the person is told nothing (notice {:?})",
            job.state, notice
        );
        assert_eq!(
            notice.and_then(|notice| notice.data.get(&NoticeKey::Status)),
            Some(&DataValue::Int(i64::from(status))),
            "HTTP {status}: said with its status"
        );
        // Pairing again (401) or a hub mid-deploy (404) is retried; a 409 or
        // 413 would get the same answer for the same bytes.
        let expected = if matches!(status, 401 | 404) {
            JobState::RetryWait
        } else {
            JobState::Blocked
        };
        assert_eq!(job.state, expected, "HTTP {status}");
    }
    Ok(())
}

/// A chunk refused for good does not hold its meeting's closure: the closure
/// goes out declaring it lost, and the hub writes the note.
#[tokio::test]
async fn a_refused_chunk_does_not_hold_the_meeting_closure_for_ever() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    // Offline while the two outcomes land, so no further upload is attempted.
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        json!(orbit_core::Connection::Offline),
    );
    let delivered = chunk("reunion-retenida", 0);
    let refused = chunk("reunion-retenida", 1);
    for metadata in [&delivered, &refused] {
        runtime.fixture_store.seal_chunk(metadata, b"synthetic")?;
        runtime
            .fixture_store
            .update_job(&format!("upload-{}", metadata.id), JobState::Running)?;
    }
    upload_outcome(
        &mut runtime,
        &refused.id,
        Err(crate::retry::Refused(409).into()),
    )
    .await?;
    upload_outcome(&mut runtime, &delivered.id, Ok(())).await?;
    runtime
        .queue_capture_finalize("reunion-retenida", 1, 2, 0)
        .await?;
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "connection",
        json!(orbit_core::Connection::Online),
    );
    runtime.link.token = Some(TOKEN.into());
    runtime.settings.hub_url = Some(
        one_shot_http(json_response(
            "200 OK",
            "{\"durable\":true,\"finalized\":true,\"recording_id\":\"reunion-retenida\"}",
        ))
        .await?
        .to_string(),
    );
    let admitted = runtime.schedule_finalize().await?;
    assert!(
        admitted,
        "the closure of a meeting with one refused chunk is never sent; (chunks, uploaded) = {:?}",
        runtime.reader.finalize_counts("reunion-retenida")?
    );
    Ok(())
}

/// A hub out of space (507) is said as something to fix, not as a silent
/// retry; the chunk stays on the Mac to be sent when there is room.
#[tokio::test]
async fn a_hub_out_of_space_is_said_not_retried_in_silence() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let metadata = chunk("reunion-sin-cuota", 0);
    runtime.fixture_store.seal_chunk(&metadata, b"synthetic")?;
    runtime
        .fixture_store
        .update_job(&format!("upload-{}", metadata.id), JobState::Running)?;
    let refusal = crate::retry::check_status(reqwest::StatusCode::INSUFFICIENT_STORAGE);
    upload_outcome(&mut runtime, &metadata.id, refusal).await?;
    let job = runtime
        .reader
        .get_job(&format!("upload-{}", metadata.id))?
        .context("job lost")?;
    let notice = runtime.state.durable.notice.as_ref();
    assert!(
        notice.is_some_and(|notice| notice.code.class() != NoticeClass::Silent),
        "HTTP 507 (hub quota): the chunk is {:?} and the person is told nothing (notice {:?})",
        job.state,
        notice
    );
    Ok(())
}

/// The backlog goes up in capture order: chunk 2 before chunk 10, where text
/// order over `{recording}-{track}-{sequence}` put 10 (and 100, 1000…) first,
/// and the hub closes a transcription group after a 30 s hole.
#[tokio::test]
async fn a_backlog_is_uploaded_in_capture_order() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    for sequence in [2_u64, 10] {
        runtime
            .fixture_store
            .seal_chunk(&chunk("reunion-atrasada", sequence), b"synthetic")?;
    }
    let first = runtime
        .reader
        .pending_chunk_uploads(None, 1)?
        .pop()
        .context("no upload queued")?;
    assert_eq!(
        first.payload["metadata"]["sequence"], 2,
        "the first chunk offered for upload is {}",
        first.id
    );
    Ok(())
}

/// The hub's own code and numbers travel with its refusal: a 507 says how
/// much the hub uses and allows, and uploads wait minutes, not seconds.
#[tokio::test]
async fn a_hub_out_of_space_is_said_with_its_numbers_and_waits() -> Result<()> {
    let body = r#"{"message":"Cuota","code":"quota_exceeded","refusal":{"code":"quota_exceeded","bytes":96044,"reserved_bytes":550356,"used_bytes":10737000000,"max_bytes":10737418240,"retry":{"kind":"never"}}}"#;
    let url = one_shot_http(json_response("507 Insufficient Storage", body)).await?;
    let refusal = crate::retry::check_response(reqwest::get(url).await?)
        .await
        .err()
        .context("a 507 passed")?;
    let full = refusal
        .downcast_ref::<crate::retry::OutOfSpace>()
        .context("not typed as out of space")?
        .clone();
    assert_eq!(full.code, "quota_exceeded");
    assert_eq!(full.used_bytes, Some(10_737_000_000));
    assert_eq!(full.max_bytes, Some(10_737_418_240));
    // The hub's other 507, a disk that refused the write, reads the same way.
    let disk = r#"{"message":"Disco lleno","code":"storage_full","refusal":{"code":"storage_full","used_bytes":80000000000,"free_bytes":0,"retry":{"kind":"after_fix"}}}"#;
    let url = one_shot_http(json_response("507 Insufficient Storage", disk)).await?;
    let refusal = crate::retry::check_response(reqwest::get(url).await?)
        .await
        .err()
        .context("a 507 passed")?;
    let disk = refusal
        .downcast_ref::<crate::retry::OutOfSpace>()
        .context("storage_full not typed as out of space")?;
    assert_eq!(disk.code, "storage_full");
    assert_eq!(disk.used_bytes, Some(80_000_000_000));
    assert_eq!(disk.free_bytes, Some(0));

    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let metadata = chunk("reunion-sin-espacio", 0);
    runtime.fixture_store.seal_chunk(&metadata, b"synthetic")?;
    runtime
        .fixture_store
        .update_job(&format!("upload-{}", metadata.id), JobState::Running)?;
    upload_outcome(&mut runtime, &metadata.id, Err(full.into())).await?;
    let notice = runtime.state.durable.notice.clone().context("unsaid")?;
    assert_eq!(notice.code, NoticeCode::HubStorageFull);
    assert_eq!(
        notice.data.get(&NoticeKey::Bytes),
        Some(&DataValue::Int(10_737_000_000))
    );
    assert_eq!(
        notice.data.get(&NoticeKey::MaxBytes),
        Some(&DataValue::Int(10_737_418_240))
    );
    let job = runtime
        .reader
        .get_job(&format!("upload-{}", metadata.id))?
        .context("job lost")?;
    assert_eq!(
        job.state,
        JobState::RetryWait,
        "kept to retry, never dropped"
    );
    let now = std::time::Instant::now();
    assert!(
        !runtime
            .transfers
            .retry_gate
            .ready(now + std::time::Duration::from_secs(61)),
        "a full hub is not asked again every minute"
    );
    assert!(
        runtime
            .transfers
            .retry_gate
            .ready(now + std::time::Duration::from_secs(301))
    );
    Ok(())
}

/// The closure counts only what the hub received and declares every chunk
/// it refused for good, wherever it is in its track, as a missing sequence
/// and a gap. A track that lost every chunk is left out.
#[test]
fn a_closure_declares_the_chunks_the_hub_refused() -> Result<()> {
    let mut manifest = json!({"tracks":[
        {"track":"mic_local","chunk_count":4,"last_sequence":3,"total_frames":4},
        {"track":"remote_app","chunk_count":1,"last_sequence":0,"total_frames":1}],
        "gaps":1,"started_at_ms":1,"finalized_at_ms":2});
    let mut middle = chunk("reunion-retenida", 1);
    middle.frames = 1;
    let mut last = chunk("reunion-retenida", 3);
    last.frames = 1;
    let mut remote = chunk("reunion-retenida", 0);
    remote.track = "remote_app".into();
    crate::daemon::finalize::declare_refused_chunks(&mut manifest, &[last, middle, remote])?;
    assert_eq!(
        manifest["tracks"],
        json!([{"track":"mic_local","chunk_count":2,"last_sequence":3,"total_frames":2,
            "missing_sequences":[1, 3]}])
    );
    assert_eq!(manifest["gaps"], 4);
    Ok(())
}
