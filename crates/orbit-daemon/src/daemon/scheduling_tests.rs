use super::*;
use crate::daemon::audio::tests::fixture;
use crate::daemon::ticketed::WorkTicket;
use orbit_store::ChunkMetadata;

fn source(recording: &str) -> ChunkMetadata {
    ChunkMetadata {
        capture_epoch_id: None,
        id: format!("{recording}-mic_local-0"),
        recording_id: recording.into(),
        track: "mic_local".into(),
        sequence: 0,
        sample_rate: 8000,
        channels: 1,
        frames: 1,
        monotonic_ns: 0,
        first_sample: 0,
        started_at_ms: 1,
    }
}

/// Occupy the single worker with a pending task and hand back its ticket, so
/// a synthetic event settles exactly the work the actor owns.
fn pending_worker(runtime: &mut Runtime, generation: u64) -> Result<WorkTicket> {
    runtime
        .work
        .worker
        .start_logical(generation, orbit_ipc::new_op(), (), |_| {
            tokio::spawn(std::future::pending::<()>())
        })
        .context("worker slot")
}

#[tokio::test]
async fn an_eligible_closure_progresses_amid_unrelated_uploads_with_fair_alternation() -> Result<()>
{
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    runtime.settings.hub_url = Some("http://127.0.0.1:9".into());
    runtime.token = Some("synthetic-not-a-credential".into());
    for recording in ["old-a", "old-b"] {
        runtime
            .fixture_store
            .seal_chunk(&source(recording), b"synthetic")?;
        runtime.fixture_store.update_job(
            &format!("upload-{recording}-mic_local-0"),
            JobState::Succeeded,
        )?;
        runtime
            .queue_capture_finalize(recording, 1, 1000, 0)
            .await?;
    }
    runtime
        .fixture_store
        .seal_chunk(&source("new"), b"synthetic")?;
    runtime.schedule_upload().await?;
    runtime.work.worker.cancel();
    assert_eq!(
        runtime
            .fixture_store
            .get_job("finalize-old-a")?
            .context("missing close")?
            .state,
        JobState::Running
    );
    assert_eq!(
        runtime
            .fixture_store
            .get_job("upload-new-mic_local-0")?
            .context("missing upload")?
            .state,
        JobState::Queued
    );
    runtime
        .fixture_store
        .update_job("finalize-old-a", JobState::Succeeded)?;
    runtime.schedule_upload().await?;
    runtime.work.worker.cancel();
    assert_eq!(
        runtime
            .fixture_store
            .get_job("upload-new-mic_local-0")?
            .context("missing upload")?
            .state,
        JobState::Running
    );
    assert_eq!(
        runtime
            .fixture_store
            .get_job("finalize-old-b")?
            .context("missing close")?
            .state,
        JobState::Queued
    );
    Ok(())
}

#[tokio::test]
async fn blocked_or_missing_own_receipts_cannot_be_finalized() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    runtime
        .fixture_store
        .seal_chunk(&source("old"), b"synthetic")?;
    runtime.queue_capture_finalize("old", 1, 1000, 0).await?;
    runtime
        .fixture_store
        .update_job("upload-old-mic_local-0", JobState::Blocked)?;
    assert!(!runtime.schedule_finalize().await?);
    assert!(runtime.work.worker.active().is_none());
    Ok(())
}

#[tokio::test]
async fn redundant_rearm_preserves_generation_and_accepts_the_live_worker_receipt() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "capabilities.native_helper",
        serde_json::json!(false),
    );
    let generation = runtime.state.generation();
    runtime.fixture_store.enqueue_job(&Job {
        id: "upload-old".into(),
        kind: "chunk_upload".into(),
        payload: json!({}),
        state: JobState::Running,
        created_at: 1,
    })?;
    let worker = pending_worker(&mut runtime, generation)?;
    runtime.hub_task = Some(tokio::spawn(std::future::pending::<Result<()>>()));
    runtime.command(Command::Rearm).await?;
    assert_eq!(runtime.state.generation(), generation);
    runtime
        .work_event(WorkResult::Store(crate::daemon::StoreWork::Uploaded {
            ticket: worker,
            job_id: "upload-old".into(),
            result: Ok(()),
        }))
        .await?;
    assert!(runtime.work.worker.active().is_none());
    assert_eq!(
        runtime
            .fixture_store
            .get_job("upload-old")?
            .context("missing job")?
            .state,
        JobState::Succeeded
    );
    runtime.hub_task.take().context("missing hub")?.abort();
    Ok(())
}

#[tokio::test]
async fn failed_ack_write_releases_worker_and_the_same_upload_can_reconcile() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    runtime.settings.hub_url = Some(format!("http://{}", listener.local_addr()?));
    runtime.token = Some("synthetic-not-a-credential".into());
    let receipt = runtime
        .fixture_store
        .seal_chunk(&source("ack"), b"synthetic")?;
    let job_id = format!("upload-{}", receipt.metadata.id);
    runtime
        .fixture_store
        .update_job(&job_id, JobState::Running)?;
    let generation = runtime.state.generation();
    let first = pending_worker(&mut runtime, generation)?;
    let fault = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    fault.execute_batch("CREATE TRIGGER synthetic_ack_fault BEFORE UPDATE ON jobs WHEN NEW.state = 'succeeded' BEGIN SELECT RAISE(FAIL, 'synthetic ACK write failure'); END;")?;
    assert!(
        runtime
            .work_event(WorkResult::Store(crate::daemon::StoreWork::Uploaded {
                ticket: first,
                job_id: job_id.clone(),
                result: Ok(())
            }))
            .await
            .is_err()
    );
    assert!(
        runtime.work.worker.active().is_none(),
        "a finished handle must not block all jobs"
    );
    assert_eq!(
        runtime
            .fixture_store
            .get_job(&job_id)?
            .context("job absent")?
            .state,
        JobState::Running
    );
    fault.execute_batch("DROP TRIGGER synthetic_ack_fault;")?;
    runtime.schedule_upload().await?;
    runtime.work.worker.cancel();
    assert_eq!(
        runtime
            .fixture_store
            .get_job(&job_id)?
            .context("job absent")?
            .payload,
        serde_json::to_value(receipt)?
    );
    let generation = runtime.state.generation();
    let retry = pending_worker(&mut runtime, generation)?;
    runtime
        .work_event(WorkResult::Store(crate::daemon::StoreWork::Uploaded {
            ticket: retry,
            job_id: job_id.clone(),
            result: Ok(()),
        }))
        .await?;
    assert_eq!(
        runtime
            .fixture_store
            .get_job(&job_id)?
            .context("job absent")?
            .state,
        JobState::Succeeded
    );
    assert!(runtime.work.worker.active().is_none());
    Ok(())
}
