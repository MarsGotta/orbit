use super::*;
use crate::daemon::audio::tests::fixture;
use anyhow::{Context, Result};
use serde_json::json;

#[tokio::test]
async fn expired_close_wait_reports_its_budget_cause() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    let mut shutdown = Shutdown::default();
    shutdown.signal();
    shutdown.deadline = Some(Instant::now());
    let cause = shutdown
        .close_until_exit(&mut runtime, std::future::pending())
        .await;
    assert!(matches!(cause, CloseWait::BudgetExpired));
    Ok(())
}

#[tokio::test]
async fn a_failed_shutdown_command_still_latches_the_exit_intent() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(false);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.native_helper",
            fixture_value,
        );
    }
    runtime.capture.buffer.begin_voice()?;
    let voice = runtime
        .capture
        .buffer
        .voice
        .as_mut()
        .context("voice absent")?;
    voice.pcm = vec![3; 200];
    voice.sample_rate = 8000;
    let database = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    database.execute_batch("CREATE TRIGGER fixture_manifest_failure BEFORE INSERT ON jobs BEGIN SELECT RAISE(ABORT, 'synthetic write failure'); END;")?;
    let mut shutdown = Shutdown::default();
    assert!(
        shutdown
            .command(Command::Shutdown, &mut runtime)
            .await
            .is_err()
    );
    assert!(shutdown.requested);
    assert!(!shutdown.ready(&runtime));
    assert!(
        shutdown
            .command(Command::Rearm, &mut runtime)
            .await
            .is_err()
    );
    // Closing latches no stop; what keeps the capture closed while
    // Orbit leaves is the closing gate itself.
    assert!(!runtime.state.privacy_stopped());
    assert!(!shutdown.allows_command(&Command::MeetingStart {
        title: "Reunión".into(),
        audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        duration_minutes: None,
        event: None,
    }));
    assert!(!shutdown.allows_command(&Command::VoiceStart));
    database.execute_batch("DROP TRIGGER fixture_manifest_failure;")?;
    runtime.capture.seal_retry_at = None;
    runtime.maintain_capture().await?;
    assert!(shutdown.ready(&runtime));
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 1);
    Ok(())
}

#[tokio::test]
async fn listener_failure_cannot_bypass_an_unsaved_voice_tail() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(false);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.native_helper",
            fixture_value,
        );
    }
    runtime.capture.buffer.begin_voice()?;
    let voice = runtime
        .capture
        .buffer
        .voice
        .as_mut()
        .context("voice absent")?;
    voice.pcm = vec![3; 200];
    voice.sample_rate = 8000;
    let database = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    database.execute_batch("CREATE TRIGGER fixture_manifest_failure BEFORE INSERT ON jobs BEGIN SELECT RAISE(ABORT, 'synthetic write failure'); END;")?;
    let listener = tokio::spawn(async {});
    listener.await?;
    let mut shutdown = Shutdown::default();
    shutdown.observe_listener(true, &mut runtime).await;
    assert!(shutdown.requested && shutdown.listener_failed);
    // Losing the listener closes Orbit without latching a stop.
    assert!(!runtime.state.privacy_stopped());
    assert!(!shutdown.ready(&runtime));
    assert!(runtime.maintain_capture().await.is_err());
    assert!(!shutdown.ready(&runtime));
    assert_eq!(
        runtime
            .capture
            .buffer
            .voice
            .as_ref()
            .context("voice lost")?
            .pcm,
        vec![3; 200]
    );
    assert!(!shutdown.allows_command(&Command::Rearm));
    assert!(!shutdown.allows_native_event(&json!({"event":"local_rearm"})));
    assert!(shutdown.allows_native_event(&json!({"event":"audio"})));
    database.execute_batch("DROP TRIGGER fixture_manifest_failure;")?;
    runtime.capture.seal_retry_at = None;
    runtime.maintain_capture().await?;
    assert!(shutdown.ready(&runtime));
    assert_eq!(runtime.fixture_store.diagnostics()?.chunk_count, 1);
    assert!(runtime.transfers.worker.active().is_none());
    Ok(())
}
