//! The loop's workers that panic: each one's slot is freed and its owner
//! starts it again, and the hub's connection is connected again.
//! Synthetic runtime; the hub is a closed loopback port.
use super::*;
use crate::daemon::audio::tests::fixture;
use std::time::{Duration, Instant};

/// A jam guard for a task to end, not a measured budget.
const JAM: Duration = Duration::from_secs(30);

#[allow(clippy::panic, reason = "the worker panics on purpose")]
fn panicking() -> JoinHandle<()> {
    tokio::spawn(async { panic!("injected worker panic") })
}

async fn ended(task: Option<&JoinHandle<()>>) -> Result<()> {
    let started = Instant::now();
    while task.is_some_and(|task| !task.is_finished()) {
        anyhow::ensure!(started.elapsed() < JAM, "the worker never ended");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    Ok(())
}

#[tokio::test]
async fn a_transfer_worker_that_panics_frees_its_slot() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let generation = runtime.state.generation();
    runtime
        .transfers
        .worker
        .attach_test(generation, (), panicking());
    ended(runtime.transfers.worker.task_handle()).await?;
    runtime.supervise_workers().await;
    assert!(
        runtime.transfers.worker.active().is_none(),
        "the closure and the cleanup wait for ever behind a dead worker"
    );
    assert_eq!(runtime.supervision.restarts(Actor::TransferWorker), 1);
    Ok(())
}

#[tokio::test]
async fn a_voice_turn_that_panics_fails_as_its_own_failure() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let generation = runtime.state.generation();
    runtime
        .assistant
        .voice_task
        .attach_test(generation, (), panicking());
    runtime.assistant.voice_operation = Some("nota-de-voz-sintetica".into());
    ended(runtime.assistant.voice_task.task_handle()).await?;
    runtime.supervise_workers().await;
    assert!(
        runtime.assistant.voice_task.active().is_none(),
        "a new voice note is refused for ever behind a dead turn"
    );
    assert_eq!(runtime.assistant.voice_operation, None);
    assert_eq!(runtime.supervision.restarts(Actor::VoiceTurn), 1);
    Ok(())
}

#[tokio::test]
async fn a_hub_connection_that_panics_is_connected_again() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    // A port nothing listens on: the relaunched link keeps retrying it.
    runtime.settings.hub_url = Some("http://127.0.0.1:9".into());
    runtime.settings.device_id = Some("11111111-1111-4111-8111-111111111111".into());
    runtime.link.token = Some("synthetic-not-a-credential".into());
    crate::daemon::authority_fixture::set(&mut runtime.state, "connection", json!("offline"));
    runtime.supervision.inject(Actor::HubConnection, 1);
    runtime.connect();
    let started = Instant::now();
    while runtime
        .link
        .task
        .as_ref()
        .is_some_and(|task| !task.is_finished())
    {
        anyhow::ensure!(started.elapsed() < JAM, "the hub task never fell");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    runtime.supervise_workers().await;
    assert_eq!(runtime.supervision.restarts(Actor::HubConnection), 1);
    let relaunched = runtime
        .link
        .task
        .as_ref()
        .context("the hub's connection was not launched again")?;
    assert!(!relaunched.is_finished(), "the relaunched connection ended");
    if let Some(task) = runtime.link.task.take() {
        task.abort();
    }
    Ok(())
}

#[tokio::test]
async fn a_reading_that_panics_frees_its_slot() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let generation = runtime.state.generation();
    runtime
        .assistant
        .speech_task
        .attach_test(generation, (), panicking());
    ended(runtime.assistant.speech_task.task_handle()).await?;
    runtime.supervise_workers().await;
    assert!(runtime.assistant.speech_task.active().is_none());
    assert_eq!(runtime.supervision.restarts(Actor::Playback), 1);
    Ok(())
}

#[tokio::test]
async fn a_control_engine_work_that_panics_settles_as_a_failure() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = fixture(directory.path())?;
    let generation = runtime.state.generation();
    runtime
        .computer
        .work
        .attach_test(generation, (), panicking());
    ended(runtime.computer.work.task_handle()).await?;
    runtime.supervise_workers().await;
    assert!(
        runtime.computer.work.active().is_none(),
        "the next control order is refused for ever behind a dead worker"
    );
    assert_eq!(runtime.supervision.restarts(Actor::ComputerControl), 1);
    Ok(())
}
