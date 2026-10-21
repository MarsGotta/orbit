//! LC1: losing the Idle snapshot owner must never lose its pending closure.
use super::*;
use orbit_core::{Connection, Decision};

/// What the actor loop does with the error (REF-12): it is contained by its
/// class, cut or said, and never latches a stop. Both failures here are the
/// capture's own (closing the recording it sealed), so they are cut causes.
async fn contained_like_event_loop(
    runtime: &mut Runtime,
    cause: orbit_core::privacy_stop::StopCause,
    error: anyhow::Error,
) {
    runtime.contain(cause, &error.to_string()).await;
}

async fn drained_idle_with_insert_fault(
    runtime: &mut Runtime,
    receiver: &mut mpsc::Receiver<Value>,
) -> Result<rusqlite::Connection> {
    let stream = start(runtime, receiver, "old").await?;
    runtime.native_event(frame(&stream, 100)).await?;
    let database = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    database.execute_batch(
        "CREATE TRIGGER lc1_close_failure BEFORE INSERT ON jobs
        WHEN NEW.id='finalize-old' BEGIN SELECT RAISE(ABORT,'synthetic LC1 closure failure'); END;",
    )?;
    let error = finish(runtime, receiver, &stream)
        .await
        .err()
        .context("closure INSERT should fail")?;
    assert!(error.to_string().contains("synthetic LC1 closure failure"));
    contained_like_event_loop(
        runtime,
        orbit_core::privacy_stop::StopCause::CaptureEventFailed,
        error,
    )
    .await;
    // The fixture says Online with no credential. The stop this failure used
    // to latch kept the maintenance off the network; now nothing is stopped,
    // and a Mac without a connection is what keeps it local, as it would for
    // one that is not paired.
    {
        let fixture_value = serde_json::json!(Connection::Offline);
        crate::daemon::authority_fixture::set(&mut runtime.state, "connection", fixture_value);
    }
    runtime.store_actor.snapshot(&runtime.state).await?;
    assert_eq!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .phase,
        CapturePhase::Idle
    );
    assert!(
        runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("recording absent")?
            .ended_at_ms
            .is_some()
    );
    // REF-12: a closure that could not be written is said, not a stop.
    assert!(!runtime.state.privacy_stopped());
    assert!(runtime.fixture_store.get_job("finalize-old")?.is_none());
    assert_eq!(runtime.features.notes.head.revision, 0);
    assert!(!runtime.audio_is_draining() && runtime.capture.pending_seals.is_empty());
    assert_eq!(
        runtime.fixture_store.load_snapshot::<State>()?,
        Some(runtime.state.clone())
    );
    Ok(database)
}

async fn reopen(directory: &std::path::Path) -> Result<(Runtime, mpsc::Receiver<Value>)> {
    let (mut runtime, receiver) = fixture(directory)?;
    runtime.state = runtime
        .fixture_store
        .load_snapshot::<State>()?
        .context("snapshot absent")?
        .recovered();
    crate::daemon::recover_store(&mut runtime.fixture_store).await?;
    runtime.fixture_store.recover_upload_jobs()?;
    runtime.fixture_store.save_snapshot(&runtime.state)?;
    runtime.recover_call_closures(crate::daemon::now()).await?;
    runtime.connect(); // No configured hub, credentials or native effects.
    Ok((runtime, receiver))
}

async fn closure_completes_its_upload(runtime: &mut Runtime) -> Result<()> {
    runtime.native_event(json!({"event":"local_rearm"})).await?;
    assert_eq!(acknowledge_uploads(runtime, "old")?, 100);
    let closure = runtime
        .fixture_store
        .get_job("finalize-old")?
        .context("closure absent")?;
    assert!(runtime.finalize_is_ready(&closure)?);
    runtime.settings.hub_url = Some("http://127.0.0.1:9".into());
    runtime.token = Some("SYNTHETIC-NOT-A-CREDENTIAL".into());
    assert!(runtime.schedule_finalize().await?);
    // Tokio current-thread fixture: abort before the spawned task is ever polled.
    // The synthetic receipt below exercises actor handling; no HTTP is executed.
    let ticket = runtime
        .work
        .worker
        .ticket()
        .cloned()
        .context("eligible closure ticket")?;
    let worker = runtime
        .work
        .worker
        .task_handle_mut()
        .context("eligible closure not scheduled")?;
    worker.abort();
    assert!(worker.await.is_err());
    assert_eq!(
        runtime
            .fixture_store
            .get_job("finalize-old")?
            .context("closure absent")?
            .state,
        JobState::Running
    );
    {
        let fixture_value = serde_json::json!(Connection::Offline);
        crate::daemon::authority_fixture::set(&mut runtime.state, "connection", fixture_value);
    }
    runtime.settings.hub_url = None;
    runtime.token = None;
    let before = runtime.features.notes.head.changed_at("old");
    runtime
        .work_event(WorkResult::Store(crate::daemon::StoreWork::Uploaded {
            ticket,
            job_id: "finalize-old".into(),
            result: Ok(()),
        }))
        .await?;
    assert_eq!(
        runtime
            .fixture_store
            .get_job("finalize-old")?
            .context("closure absent")?
            .state,
        JobState::Succeeded
    );
    assert!(
        runtime.features.notes.head.changed_at("old") > before,
        "the uploaded closure did not tell the app to read its note again"
    );
    Ok(())
}

async fn replacement_after_insert_failure(
    maintenance_before: bool,
    restart_before: bool,
) -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let database = drained_idle_with_insert_fault(&mut runtime, &mut receiver).await?;
    let ended = runtime
        .state
        .durable
        .recording
        .clone()
        .context("old recording absent")?;
    // Exercise the real maintenance error handler while the INSERT fault remains.
    let error = runtime
        .maintain_capture()
        .await
        .err()
        .context("maintenance must still fail")?;
    contained_like_event_loop(
        &mut runtime,
        orbit_core::privacy_stop::StopCause::CaptureMaintenanceFailed,
        error,
    )
    .await;
    database.execute_batch("DROP TRIGGER lc1_close_failure")?;
    if restart_before {
        drop(runtime);
        (runtime, receiver) = reopen(directory.path()).await?;
        // REF-12 A1: the restart invents no stop.
        assert!(!runtime.state.privacy_stopped() && receiver.is_empty());
        let (observed, _) = fixture(tempfile::tempdir()?.path())?;
        {
            let fixture_value = serde_json::json!(observed.state.capabilities());
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities",
                fixture_value,
            );
        }
    }
    if maintenance_before {
        runtime.run_maintenance().await;
        assert!(runtime.fixture_store.get_job("finalize-old")?.is_some());
    }
    runtime.native_event(json!({"event":"local_rearm"})).await?;
    let next = start(&mut runtime, &mut receiver, "next").await?;
    let closure = runtime
        .fixture_store
        .get_job("finalize-old")?
        .context("LC1: drained Idle owner was replaced without preserving its closure")?;
    assert!(
        runtime.features.notes.head.revision > 0,
        "the atomic replacement closure did not mark Notes"
    );
    assert_eq!(
        closure.payload["manifest"]["tracks"][0]["total_frames"],
        100
    );
    assert_eq!(closure.payload["manifest"]["gaps"], ended.gaps);
    assert_eq!(
        closure.payload["manifest"]["finalized_at_ms"],
        ended.ended_at_ms.context("end absent")?
    );
    runtime.native_event(frame(&next, 50)).await?;
    finish(&mut runtime, &mut receiver, &next).await?;
    runtime.run_maintenance().await;
    assert_eq!(
        runtime.fixture_store.get_job("finalize-old")?,
        Some(closure.clone())
    );
    drop(runtime);
    let (mut runtime, receiver) = reopen(directory.path()).await?;
    assert!(receiver.is_empty());
    runtime.run_maintenance().await;
    assert_eq!(
        runtime.fixture_store.get_job("finalize-old")?,
        Some(closure)
    );
    assert_eq!(
        runtime
            .fixture_store
            .list_chunks("next", None, 10)?
            .iter()
            .map(|chunk| chunk.metadata.frames)
            .sum::<u64>(),
        50
    );
    // Keep the next recording's uploads pending: this probe must schedule old,
    // not merely whichever eligible closure happens to sort first.
    closure_completes_its_upload(&mut runtime).await
}

#[tokio::test]
async fn lc1_replacement_before_maintenance_preserves_drained_idle_closure() -> Result<()> {
    replacement_after_insert_failure(false, false).await
}

#[tokio::test]
async fn lc1_control_maintenance_before_replacement_preserves_same_closure() -> Result<()> {
    replacement_after_insert_failure(true, false).await
}

#[tokio::test]
async fn lc1_reopened_idle_owner_is_preserved_before_replacement() -> Result<()> {
    replacement_after_insert_failure(false, true).await
}

#[tokio::test]
async fn lc1_replacement_insert_failure_keeps_owner_snapshot_and_effects_unchanged() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let database = drained_idle_with_insert_fault(&mut runtime, &mut receiver).await?;
    runtime.native_event(json!({"event":"local_rearm"})).await?;
    let previous = runtime.state.clone();
    assert!(start(&mut runtime, &mut receiver, "next").await.is_err());
    assert_eq!(runtime.state, previous);
    assert_eq!(
        runtime.fixture_store.load_snapshot::<State>()?,
        Some(previous)
    );
    assert!(runtime.fixture_store.get_job("finalize-old")?.is_none());
    assert!(
        receiver.is_empty(),
        "no audio_start before the closure/snapshot commit"
    );
    database.execute_batch("DROP TRIGGER lc1_close_failure")?;
    start(&mut runtime, &mut receiver, "next").await?;
    assert!(runtime.fixture_store.get_job("finalize-old")?.is_some());
    assert!(runtime.features.notes.head.revision > 0);
    assert_eq!(
        runtime
            .fixture_store
            .load_snapshot::<State>()?
            .context("snapshot")?
            .durable
            .recording
            .context("recording")?
            .id,
        "next"
    );
    Ok(())
}

#[tokio::test]
async fn lc1_replacement_second_write_failure_rolls_back_idle_closure_and_snapshot() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let database = drained_idle_with_insert_fault(&mut runtime, &mut receiver).await?;
    database.execute_batch("DROP TRIGGER lc1_close_failure")?;
    runtime.native_event(json!({"event":"local_rearm"})).await?;
    let previous = runtime.state.clone();
    let second_write = fail_second_write(&runtime)?;
    let error = start(&mut runtime, &mut receiver, "next")
        .await
        .err()
        .context("second write should fail")?;
    assert!(error.to_string().contains("synthetic second write failure"));
    assert_eq!(runtime.state, previous);
    assert_eq!(
        runtime.fixture_store.load_snapshot::<State>()?,
        Some(previous)
    );
    assert!(runtime.fixture_store.get_job("finalize-old")?.is_none());
    assert!(receiver.is_empty());
    second_write.execute_batch("DROP TRIGGER r2_snapshot_failure")?;
    drop(runtime);
    let (mut runtime, mut receiver) = reopen(directory.path()).await?;
    let (observed, _) = fixture(tempfile::tempdir()?.path())?;
    {
        let fixture_value = serde_json::json!(observed.state.capabilities());
        crate::daemon::authority_fixture::set(&mut runtime.state, "capabilities", fixture_value);
    }
    runtime.native_event(json!({"event":"local_rearm"})).await?;
    start(&mut runtime, &mut receiver, "next").await?;
    assert!(runtime.fixture_store.get_job("finalize-old")?.is_some());
    closure_completes_its_upload(&mut runtime).await
}

#[tokio::test]
async fn lc1_existing_closure_remains_immutable_in_every_job_state() -> Result<()> {
    for state in [
        JobState::Queued,
        JobState::Running,
        JobState::Unknown,
        JobState::Blocked,
        JobState::RetryWait,
        JobState::Succeeded,
        JobState::Cancelled,
    ] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut receiver) = fixture(directory.path())?;
        let database = drained_idle_with_insert_fault(&mut runtime, &mut receiver).await?;
        database.execute_batch("DROP TRIGGER lc1_close_failure")?;
        runtime.queue_finalize().await?;
        runtime.fixture_store.update_job("finalize-old", state)?;
        let previous = runtime
            .fixture_store
            .get_job("finalize-old")?
            .context("closure absent")?;
        runtime.native_event(json!({"event":"local_rearm"})).await?;
        start(&mut runtime, &mut receiver, "next").await?;
        assert_eq!(
            runtime.fixture_store.get_job("finalize-old")?,
            Some(previous.clone())
        );
        runtime.run_maintenance().await;
        let reopened = crate::daemon::test_store::open(&runtime.paths.store, [0x42; 32])?;
        assert_eq!(reopened.get_job("finalize-old")?, Some(previous));
    }
    Ok(())
}

#[tokio::test]
async fn lc1_removing_the_snapshot_owner_preserves_its_closure_atomically() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let database = drained_idle_with_insert_fault(&mut runtime, &mut receiver).await?;
    let previous = runtime.state.clone();
    let mut removed = previous.clone();
    removed.durable.recording = None;
    // Exercise the existing decision boundary; no new public action is introduced.
    let decision = || Decision {
        state: removed.clone(),
        effects: Vec::new(),
        authority_change: None,
    };
    assert!(runtime.execute_decision(decision()).await.is_err());
    assert_eq!(runtime.state, previous);
    assert_eq!(
        runtime.fixture_store.load_snapshot::<State>()?,
        Some(previous)
    );
    assert!(runtime.fixture_store.get_job("finalize-old")?.is_none());
    database.execute_batch("DROP TRIGGER lc1_close_failure")?;
    runtime.execute_decision(decision()).await?;
    assert!(runtime.state.durable.recording.is_none());
    assert!(
        runtime
            .fixture_store
            .load_snapshot::<State>()?
            .context("snapshot absent")?
            .durable
            .recording
            .is_none()
    );
    assert!(runtime.fixture_store.get_job("finalize-old")?.is_some());
    assert!(receiver.is_empty());
    closure_completes_its_upload(&mut runtime).await
}
