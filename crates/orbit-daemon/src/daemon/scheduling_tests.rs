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
        .transfers
        .worker
        .start_logical(generation, orbit_ipc::new_op(), (), |_| {
            tokio::spawn(std::future::pending::<()>())
        })
        .context("worker slot")
}

/// Closures go up one at a time from the loop's worker; chunks go up from
/// the capture task. Neither waits for the other: an eligible closure
/// progresses amid unrelated uploads, and the next closure follows the first
/// without a chunk taking its turn.
#[tokio::test]
async fn an_eligible_closure_progresses_amid_unrelated_uploads() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    runtime.settings.hub_url = Some("http://127.0.0.1:9".into());
    runtime.link.token = Some("synthetic-not-a-credential".into());
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
    let state = |runtime: &Runtime, id: &str| -> Result<JobState> {
        Ok(runtime
            .fixture_store
            .get_job(id)?
            .with_context(|| format!("missing {id}"))?
            .state)
    };
    runtime.schedule_upload().await?;
    runtime.transfers.worker.cancel();
    assert_eq!(state(&runtime, "finalize-old-a")?, JobState::Running);
    // The loop's scheduling leaves the chunk to the capture task.
    assert_eq!(state(&runtime, "upload-new-mic_local-0")?, JobState::Queued);
    let (mut task, _bridge) = crate::daemon::capture_lane::test_support::spawn_task(&runtime)?;
    crate::daemon::capture_lane::test_support::job_reaches(
        &runtime,
        "upload-new-mic_local-0",
        JobState::RetryWait,
    )
    .await?;
    task.stop();
    runtime
        .fixture_store
        .update_job("finalize-old-a", JobState::Succeeded)?;
    // The chunk's failure held the shared gate: the next closure waits for
    // it, and then goes, with no chunk taking its turn.
    while !runtime
        .transfers
        .retry_gate
        .ready(std::time::Instant::now())
    {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    runtime.schedule_upload().await?;
    runtime.transfers.worker.cancel();
    assert_eq!(state(&runtime, "finalize-old-b")?, JobState::Running);
    Ok(())
}

#[tokio::test]
async fn a_missing_own_receipt_cannot_be_finalized_and_a_refused_one_is_declared() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    runtime.settings.hub_url = Some("http://127.0.0.1:9".into());
    runtime.link.token = Some("synthetic-not-a-credential".into());
    runtime
        .fixture_store
        .seal_chunk(&source("old"), b"synthetic")?;
    runtime.queue_capture_finalize("old", 1, 1000, 0).await?;
    // Its upload has not been confirmed: the closure waits for it.
    assert!(!runtime.schedule_finalize().await?);
    assert!(runtime.transfers.worker.active().is_none());
    // The hub refused it for good: the closure no longer waits for it, and
    // goes out declaring it lost.
    runtime
        .fixture_store
        .update_job("upload-old-mic_local-0", JobState::Blocked)?;
    runtime.transfers.finalize_ready.clear();
    assert!(runtime.schedule_finalize().await?);
    assert!(runtime.transfers.worker.active().is_some());
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
    runtime.link.task = Some(tokio::spawn(std::future::pending::<Result<()>>()));
    runtime.command(Command::Rearm).await?;
    assert_eq!(runtime.state.generation(), generation);
    runtime
        .work_event(WorkResult::Store(crate::daemon::StoreWork::Uploaded {
            ticket: worker,
            job_id: "upload-old".into(),
            result: Ok(()),
        }))
        .await?;
    assert!(runtime.transfers.worker.active().is_none());
    assert_eq!(
        runtime
            .fixture_store
            .get_job("upload-old")?
            .context("missing job")?
            .state,
        JobState::Succeeded
    );
    runtime.link.task.take().context("missing hub")?.abort();
    Ok(())
}

#[tokio::test]
async fn failed_ack_write_releases_worker_and_the_same_upload_can_reconcile() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    runtime.settings.hub_url = Some(format!("http://{}", listener.local_addr()?));
    runtime.link.token = Some("synthetic-not-a-credential".into());
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
        runtime.transfers.worker.active().is_none(),
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
    runtime.transfers.worker.cancel();
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
    assert!(runtime.transfers.worker.active().is_none());
    Ok(())
}
