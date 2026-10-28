//! The engine and nothing else. The macOS permission is the
//! only decision (the harness is a Mac that grants it), the catalogue comes
//! from the engine, and an absent engine is said out loud.
use super::*;
use orbit_core::computer_control::ToolCall;

fn launch() -> ToolCall {
    ToolCall {
        tool: "launch_app".into(),
        arguments: json!({"bundle_id": "com.example.never-granted"}),
    }
}

/// The catalogue is the engine's. A tool the fixture engine declares that
/// Orbit has never heard of reaches the engine untouched.
#[tokio::test]
async fn a_tool_orbit_does_not_know_still_reaches_the_engine() -> Result<()> {
    let mut h = Mac::new(false).await?;
    let names: Vec<String> = h
        .runtime
        .computer
        .engine_tools()
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    assert!(
        names.iter().any(|name| name == INVENTED_TOOL),
        "the engine's own catalogue reaches Orbit: {names:?}"
    );
    h.runtime.call_engine_tool(
        ToolCall {
            tool: INVENTED_TOOL.into(),
            arguments: json!({}),
        },
        "local",
        None,
    )?;
    h.finish().await?;
    assert_eq!(h.calls(INVENTED_TOOL), 1);
    Ok(())
}

/// Without an engine a remote action says so, by name, and never diverts.
#[tokio::test]
async fn without_an_engine_a_remote_action_answers_the_cause() -> Result<()> {
    let mut h = Mac::new(false).await?;
    h.runtime
        .revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::ComputerRevoked,
        })
        .await;
    assert!(!h.runtime.computer.engine_is_ready());
    let error = h
        .runtime
        .call_engine_tool(launch(), "local", None)
        .expect_err("no engine means no action");
    let message = error.to_string();
    assert!(
        message.contains("The engine is unavailable"),
        "the cause must be named: {message}"
    );
    // Nothing was attempted anywhere: no engine, no other path.
    assert_eq!(h.calls("launch_app"), 0);
    Ok(())
}

#[tokio::test]
async fn the_privacy_stop_still_cuts_an_engine_action() -> Result<()> {
    let mut h = Mac::new(true).await?;
    h.runtime.call_engine_tool(launch(), "local", None)?;
    tokio::task::yield_now().await;
    h.runtime.command(Command::PrivacyStop).await?;
    assert!(h.runtime.computer.work.active().is_none());
    assert!(!h.runtime.computer.engine_is_ready());
    let history = h.runtime.control_history()?;
    assert_eq!(
        history.first().map(|entry| entry.outcome),
        Some(orbit_core::computer_control::ControlOutcome::Unknown)
    );
    Ok(())
}

/// Withdrawing Accessibility in macOS — the last thing the old
/// switch did here — revokes the work in flight, records the uncertain end and says
/// which permission went.
#[tokio::test]
async fn withdrawing_accessibility_marks_inflight_work_unknown_and_records_it() -> Result<()> {
    let mut h = Mac::new(true).await?;
    h.runtime.call_engine_tool(launch(), "local", None)?;
    tokio::task::yield_now().await;
    {
        let fixture_value = serde_json::json!(orbit_core::PermissionState::Denied);
        crate::daemon::authority_fixture::set(
            &mut h.runtime.state,
            "capabilities.accessibility_state",
            fixture_value,
        );
    }
    h.runtime.maintain_computer().await?;
    assert!(h.runtime.computer.work.active().is_none());
    assert!(
        serde_json::from_str::<Value>(&permission_reason(&h.runtime))?["permission"]
            == "accessibility",
        "{}",
        permission_reason(&h.runtime)
    );
    let history = h.runtime.control_history()?;
    assert_eq!(
        history.first().map(|entry| entry.outcome),
        Some(orbit_core::computer_control::ControlOutcome::Unknown)
    );
    Ok(())
}

/// The record survives every one of those endings.
#[tokio::test]
async fn the_record_keeps_what_was_asked_and_how_it_ended() -> Result<()> {
    let mut h = Mac::new(false).await?;
    h.runtime.call_engine_tool(launch(), "local", None)?;
    h.finish().await?;
    let history = h.runtime.control_history()?;
    let entry = history.first().context("one recorded action")?;
    assert_eq!(entry.requested, "launch_app");
    assert_eq!(entry.application, "com.example.never-granted");
    assert_eq!(
        entry.outcome,
        orbit_core::computer_control::ControlOutcome::Performed
    );
    Ok(())
}
