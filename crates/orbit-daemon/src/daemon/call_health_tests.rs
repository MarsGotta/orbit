//! A call stops for lack of audio only when no audio is coming, and for the
//! user's silence only when they are silent and Deimos is not answering.
//! The facts are given, never waited for: a past `last_audio`, a real channel
//! holding the frames the loop has not read, and the frames' own timestamps.
use super::*;
use std::time::{Duration, Instant};

fn logging(runtime: &mut Runtime, directory: &std::path::Path) {
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory));
}

fn last_logged(directory: &std::path::Path, event: &str) -> Result<Value> {
    let log = std::fs::read_to_string(directory.join("logs/daemon.log"))?;
    let line = log
        .lines()
        .rev()
        .find(|line| line.contains(&format!("\"event\":\"{event}\"")))
        .context("event absent from daemon.log")?;
    let line: Value = serde_json::from_str(line)?;
    Ok(line["fields"].clone())
}

fn call_alive(runtime: &Runtime) -> bool {
    runtime.state.durable.call.is_some()
        && runtime
            .capture
            .call_runtime
            .as_ref()
            .is_some_and(|call| call.closing_at_ms.is_none())
}

fn last_audio_at(runtime: &mut Runtime, at: Instant) -> Result<()> {
    runtime
        .capture
        .call_runtime
        .as_mut()
        .context("call runtime absent")?
        .last_audio = at;
    Ok(())
}

/// The helper kept sending (40 frames of 100 ms, four seconds),
/// the loop was held and read none of them. The tick that comes first must
/// not read the unread frames as silence.
#[tokio::test]
async fn a_held_loop_does_not_cut_a_call_whose_frames_are_waiting() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    let stream = start_call(&mut runtime, &mut helper).await?;
    runtime.native_event(call_frame(&stream, 0, false)).await?;
    let (queue, mut unread) = mpsc::channel(crate::native::NATIVE_EVENTS_CAPACITY);
    for sequence in 1..=40 {
        queue.try_send(call_frame(&stream, sequence, false))?;
    }
    // The last frame the loop processed is four seconds old.
    last_audio_at(&mut runtime, Instant::now() - Duration::from_secs(4))?;
    runtime.note_loop_tick(unread.len(), Duration::from_secs(4));
    runtime.check_call_health().await?;
    assert!(
        call_alive(&runtime),
        "the call was cut with 40 frames unread"
    );
    // The loop reads them, and the next tick finds nothing waiting.
    while let Ok(frame) = unread.try_recv() {
        runtime.native_event(frame).await?;
    }
    runtime.note_loop_tick(unread.len(), Duration::from_millis(3));
    runtime.check_call_health().await?;
    assert!(call_alive(&runtime));
    assert_eq!(
        runtime
            .capture
            .call_runtime
            .as_ref()
            .context("call runtime absent")?
            .frames_heard,
        41
    );
    Ok(())
}

/// No frame processed for four seconds and none waiting: the
/// call stops, and the log carries the numbers that say why.
#[tokio::test]
async fn a_call_with_no_audio_stops_and_says_its_numbers() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    logging(&mut runtime, directory.path());
    let stream = start_call(&mut runtime, &mut helper).await?;
    runtime.native_event(call_frame(&stream, 0, false)).await?;
    last_audio_at(&mut runtime, Instant::now() - Duration::from_secs(4))?;
    runtime.note_loop_tick(0, Duration::from_millis(1_200));
    runtime.check_call_health().await?;
    assert!(!call_alive(&runtime), "a call with no audio kept going");
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::CallStoppedAutomatically)
    );
    assert!(crate::daemon::turn::said::mentions(
        &runtime.state,
        "no_audio"
    ));
    let line = last_logged(directory.path(), "call_stopped_idle")?;
    assert_eq!(line["cause"], "no_audio", "{line}");
    let age: u64 = line["last_frame_age_ms"]
        .as_str()
        .context("age absent")?
        .parse()?;
    assert!((4_000..60_000).contains(&age), "{line}");
    assert_eq!(line["queued_native_events"], "0", "{line}");
    assert_eq!(line["deimos_speaking"], "false", "{line}");
    assert_eq!(line["last_tick_ms"], "1200", "{line}");
    assert_eq!(line["limit_ms"], "3000", "{line}");
    Ok(())
}

/// A real call on 2026-10-18 (Orbit 0.6.0): the audio kept arriving, but a
/// minute after the user's last turn the detector stopped the call `idle` while
/// Deimos was speaking. The frames carry `playback_active` and their own
/// time; nothing here waits.
#[tokio::test]
async fn deimos_speaking_a_minute_after_the_users_last_turn_keeps_the_call() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = fixture(directory.path())?;
    logging(&mut runtime, directory.path());
    let stream = start_call_for(&mut runtime, &mut helper, 600_000).await?;
    let started = runtime
        .state
        .durable
        .call
        .as_ref()
        .context("call absent")?
        .started_at_ms;
    let mut speaking = call_frame(&stream, 0, false);
    speaking["playback_active"] = json!(true);
    let action = runtime
        .prepare_call_frame(&speaking, started + 61_000)
        .await?;
    assert_eq!(action, orbit_core::voice::TurnAction::None);
    assert!(call_alive(&runtime), "the call stopped while Deimos spoke");
    // He stops at +61 s; a minute of the user's silence after that does stop it.
    let action = runtime
        .prepare_call_frame(&call_frame(&stream, 1, false), started + 121_000)
        .await?;
    assert_eq!(
        action,
        orbit_core::voice::TurnAction::Stop(orbit_core::voice::StopReason::Idle)
    );
    assert!(!call_alive(&runtime));
    let line = last_logged(directory.path(), "call_stopped_idle")?;
    assert_eq!(line["cause"], "idle", "{line}");
    assert_eq!(line["user_silent_ms"], "60000", "{line}");
    assert_eq!(line["deimos_speaking"], "false", "{line}");
    Ok(())
}
