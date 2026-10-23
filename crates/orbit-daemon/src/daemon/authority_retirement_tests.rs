use super::*;
use crate::daemon::audio::tests::fixture;

/// A call on offer, the call detection's own state. It is local: only
/// the user's stop, a revocation and closing retire it.
fn offer_a_call(runtime: &mut Runtime) {
    use orbit_core::call_detection::{CallApp, CallFacts, CallInput, CallPreference};
    let facts = CallFacts {
        now_ms: 0,
        preference: CallPreference::Notify,
        privacy_stopped: false,
        recording_live: false,
    };
    let zoom = CallApp {
        application_pid: 4242,
        bundle_id: "us.zoom.xos".into(),
        name: "zoom.us".into(),
    };
    runtime.features.call.detector.apply(
        CallInput::Microphone {
            applications: vec![zoom],
        },
        &facts,
    );
    runtime.features.call.detector.apply(
        CallInput::Tick,
        &CallFacts {
            now_ms: 3_000,
            ..facts
        },
    );
    assert!(call_offered(runtime), "the fixture offers a call");
}

fn call_offered(runtime: &Runtime) -> bool {
    matches!(
        runtime.features.call.detector.panel(3_000),
        Some(orbit_core::call_detection::CallPanel::Offer { .. })
    )
}

#[tokio::test]
async fn revocation_retires_sources() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    runtime.state.views.source_inventory = Some(orbit_core::sources::WindowInventory {
        native_instance_id: "fixture".into(),
        privacy_generation: 1,
        inventory_id: "fixture".into(),
        clock_domain: "fixture".into(),
        observed_monotonic_ns: 0,
        expires_monotonic_ns: 1,
        selected_text_available: false,
        windows: Vec::new(),
        omitted_count: 0,
    });
    runtime.apply(Event::Connect(Connection::Revoked)).await?;
    assert!(runtime.state.views.source_inventory.is_none());
    Ok(())
}

#[tokio::test]
async fn new_and_open_session_retire_the_same_local_verticals() -> Result<()> {
    for open in [false, true] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _native) = fixture(directory.path())?;
        {
            let fixture_source: Option<String> = Some("before".into());
            let fixture_value = serde_json::json!(fixture_source);
            crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
        }
        runtime.state.durable.assistant.pending_approval = Some(Approval {
            request_id: "approval".into(),
            description: "fixture".into(),
            clarification: false,
            choices: Vec::new(),
            questions: Vec::new(),
            deletes: None,
        });
        if open {
            runtime
                .command(Command::SessionOpen {
                    id: "after".into(),
                    agent_id: None,
                })
                .await?;
            assert_eq!(runtime.state.session_id().as_deref(), Some("after"));
        } else {
            runtime
                .command(Command::NewSession { agent_id: None })
                .await?;
            assert!(runtime.state.session_id().is_none());
        }
        assert!(runtime.state.durable.assistant.pending_approval.is_none());
    }
    Ok(())
}

#[tokio::test]
async fn each_session_blocker_names_its_cause() -> Result<()> {
    for (cause, operation) in [
        ("turn", orbit_core::refusal::Operation::Chat),
        ("buffer", orbit_core::refusal::Operation::Audio),
        (
            "transcription",
            orbit_core::refusal::Operation::VoiceTranscription,
        ),
    ] {
        for open in [false, true] {
            let directory = tempfile::tempdir()?;
            let (mut runtime, _native) = fixture(directory.path())?;
            match cause {
                "turn" => runtime.state.durable.assistant.active_request = Some("turn".into()),
                "buffer" => runtime.capture.buffer.begin_voice()?,
                "transcription" => {
                    runtime.conversation.voice_task.attach_test(
                        runtime.state.generation(),
                        (),
                        tokio::spawn(std::future::pending()),
                    );
                }
                other => bail!("unexpected test case: {other}"),
            }
            let command = if open {
                Command::SessionOpen {
                    id: "saved".into(),
                    agent_id: None,
                }
            } else {
                Command::NewSession { agent_id: None }
            };
            let Err(error) = runtime.command(command).await else {
                bail!("blocker admitted");
            };
            let refusal = refusal_from_error(&error);
            assert_eq!(
                refusal,
                Refusal::Busy {
                    operation,
                    retry_after_ms: None
                },
                "{cause}: IPC lost the cause"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn rejected_session_identity_names_bytes_and_ceiling_or_control() -> Result<()> {
    for (id, cause) in [
        (String::new(), "0 bytes"),
        ("x".repeat(122), "122 bytes"),
        ("bad\u{0007}id".to_owned(), "carácter de control"),
    ] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _native) = fixture(directory.path())?;
        let Err(error) = runtime
            .command(Command::SessionOpen { id, agent_id: None })
            .await
        else {
            bail!("invalid id admitted");
        };
        let text = error.to_string();
        assert!(text.contains(cause), "{text}");
        assert!(text.contains("121"), "{text}");
    }
    Ok(())
}

#[tokio::test]
async fn revocation_stops_capture_without_a_source_retirement_write() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut native) = fixture(directory.path())?;
    runtime.source_windows().await?;
    assert_eq!(
        native.try_recv().context("source listing")?["command"],
        "screen_sources"
    );
    runtime.capture.mic_stream = Some("synthetic-open-mic".into());
    let database = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    database.execute_batch(
        "CREATE TABLE review_snapshot_writes(n INTEGER NOT NULL);
         INSERT INTO review_snapshot_writes VALUES (0);
         CREATE TRIGGER review_fail_second_snapshot BEFORE UPDATE ON settings
         WHEN NEW.key='snapshot' BEGIN
           UPDATE review_snapshot_writes SET n=n+1;
           SELECT RAISE(ABORT, 'synthetic source retirement failure')
             WHERE (SELECT n FROM review_snapshot_writes) > 1;
         END;",
    )?;
    let result = runtime.apply(Event::Connect(Connection::Revoked)).await;
    result?;
    let writes: i64 =
        database.query_row("SELECT n FROM review_snapshot_writes", [], |row| row.get(0))?;
    assert_eq!(writes, 1, "revocation must persist its decision only");
    assert_eq!(
        runtime
            .state
            .views
            .source_request
            .as_ref()
            .context("source progress")?
            .stage,
        orbit_core::sources::SourceStage::Unknown,
        "the source cancellation must finish in memory"
    );
    let mut commands = Vec::new();
    while let Ok(command) = native.try_recv() {
        commands.push(command["command"].as_str().unwrap_or_default().to_owned());
    }
    assert!(
        commands.iter().any(|command| command == "capture_stop"),
        "revocation left capture open: {commands:?}"
    );
    Ok(())
}

#[tokio::test]
async fn returning_online_keeps_the_bridge_and_the_detection() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(Connection::Offline);
        crate::daemon::authority_fixture::set(&mut runtime.state, "connection", fixture_value);
    }
    offer_a_call(&mut runtime);
    runtime.conversation.bridge_request = Some(BridgeAsk {
        for_attachment: false,
        id: "still-pending".into(),
        capability: orbit_core::device_bridge::Capability::PerformAction,
        application: None,
        title: None,
        full_screen: false,
        source_id: None,
        tool_call: None,
        catalogue_after: 0,
        calendar_window: None,
        calendar_cursor: None,
        files: None,
    });
    runtime.apply(Event::Connect(Connection::Online)).await?;
    assert!(runtime.conversation.bridge_request.is_some());
    assert!(call_offered(&runtime), "Online retired the call on offer");
    Ok(())
}

#[tokio::test]
async fn a_lost_hub_still_retires_the_bridge_and_keeps_the_call() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    offer_a_call(&mut runtime);
    runtime.conversation.bridge_request = Some(BridgeAsk {
        for_attachment: false,
        id: "lost-link".into(),
        capability: orbit_core::device_bridge::Capability::PerformAction,
        application: None,
        title: None,
        full_screen: false,
        source_id: None,
        tool_call: None,
        catalogue_after: 0,
        calendar_window: None,
        calendar_cursor: None,
        files: None,
    });
    runtime.hub_event(HubEvent::Disconnected).await?;
    assert!(runtime.conversation.bridge_request.is_none());
    // A call on this Mac does not depend on the hub.
    assert!(
        call_offered(&runtime),
        "a lost hub withdrew the call on offer"
    );
    Ok(())
}

#[tokio::test]
async fn review_rearm_with_a_latch_closes_generation_bound_work() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _native) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(Connection::Offline);
        crate::daemon::authority_fixture::set(&mut runtime.state, "connection", fixture_value);
    }
    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", fixture_value);
    }
    runtime.conversation.bridge_request = Some(BridgeAsk {
        for_attachment: false,
        id: "old".into(),
        capability: orbit_core::device_bridge::Capability::PerformAction,
        application: None,
        title: None,
        full_screen: false,
        source_id: None,
        tool_call: None,
        catalogue_after: 0,
        calendar_window: None,
        calendar_cursor: None,
        files: None,
    });

    runtime.apply(Event::LocalRearm).await?;
    assert_eq!(runtime.state.generation(), 1);
    assert!(runtime.conversation.bridge_request.is_none());
    Ok(())
}

async fn assert_failed_decision_snapshot_keeps_decision_pending(command: Command) -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut native) = fixture(directory.path())?;
    let interrupt = matches!(command, Command::Interrupt);
    if interrupt {
        // CallTurn still runs before the decision snapshot. A failed snapshot
        // must preserve that completed local cancellation.
        crate::daemon::audio::tests::call::start_call(&mut runtime, &mut native).await?;
        runtime
            .state
            .durable
            .voice
            .call
            .as_mut()
            .context("call missing before the source listing")?
            .phase = orbit_core::CallPhase::UserSpeaking;
    }
    runtime.source_windows().await?;
    assert_eq!(
        native.try_recv().context("source listing")?["command"],
        "screen_sources"
    );
    {
        let fixture_source: Option<String> = Some("before".into());
        let fixture_value = serde_json::json!(fixture_source);
        crate::daemon::authority_fixture::set(&mut runtime.state, "session_id", fixture_value);
    }
    if interrupt {
        runtime.state.durable.assistant.active_request = Some("turn".into());
    }
    runtime.store_actor.snapshot(&runtime.state).await?;
    runtime.work.durable_snapshot = DurableSnapshot::from(&runtime.state);
    let mut previous = runtime.state.clone();
    if interrupt {
        previous
            .durable
            .voice
            .call
            .as_mut()
            .context("call missing")?
            .phase = orbit_core::CallPhase::Listening;
    }
    let mut frames = runtime.hub.open(4);
    let database = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    database.execute_batch(
        "CREATE TRIGGER review_fail_decision_snapshot BEFORE UPDATE ON settings
         WHEN NEW.key='snapshot' BEGIN
           SELECT RAISE(ABORT, 'synthetic decision snapshot failure');
         END;",
    )?;
    let name = format!("{command:?}");
    let error = runtime
        .command(command)
        .await
        .err()
        .context("decision snapshot unexpectedly succeeded")?;
    assert!(
        error
            .to_string()
            .contains("synthetic decision snapshot failure"),
        "{name}: {error}"
    );
    assert_eq!(
        runtime
            .state
            .views
            .source_request
            .as_ref()
            .context("source progress")?
            .stage,
        orbit_core::sources::SourceStage::Unknown,
        "{name}: fixture failed before cancel_sources"
    );
    assert_eq!(
        runtime.state.durable.assistant.active_request, previous.durable.assistant.active_request,
        "{name}"
    );
    assert_eq!(runtime.state.session_id(), previous.session_id(), "{name}");
    assert_eq!(
        runtime.state.durable.assistant.turn, previous.durable.assistant.turn,
        "{name}"
    );
    assert_eq!(
        runtime.state.durable.assistant.pending_approval,
        previous.durable.assistant.pending_approval,
        "{name}"
    );
    assert_eq!(
        runtime.state.durable.voice.call, previous.durable.voice.call,
        "{name}: failed snapshot lost the post-cancellation call state"
    );
    if interrupt {
        assert_eq!(
            runtime
                .state
                .durable
                .voice
                .call
                .as_ref()
                .context("call lost")?
                .phase,
            orbit_core::CallPhase::Listening,
            "{name}: the completed local call cancellation was undone"
        );
    }
    assert!(
        frames.try_recv().is_err(),
        "{name}: hub received a skipped effect"
    );
    while let Ok(frame) = native.try_recv() {
        assert_ne!(
            frame["command"], "playback_stop",
            "{name}: playback effect ran"
        );
    }
    Ok(())
}

#[tokio::test]
async fn review_interrupt_snapshot_failure_keeps_the_turn_until_effects_can_run() -> Result<()> {
    assert_failed_decision_snapshot_keeps_decision_pending(Command::Interrupt).await
}

#[tokio::test]
async fn review_new_session_snapshot_failure_keeps_the_current_session() -> Result<()> {
    assert_failed_decision_snapshot_keeps_decision_pending(Command::NewSession { agent_id: None })
        .await
}

#[tokio::test]
async fn review_open_session_snapshot_failure_keeps_the_current_session() -> Result<()> {
    assert_failed_decision_snapshot_keeps_decision_pending(Command::SessionOpen {
        id: "after".into(),
        agent_id: None,
    })
    .await
}

#[tokio::test]
async fn conversation_decisions_keep_the_source_cancellation_receipt() -> Result<()> {
    for command in [
        Command::Interrupt,
        Command::NewSession { agent_id: None },
        Command::SessionOpen {
            id: "after".into(),
            agent_id: None,
        },
    ] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut native) = fixture(directory.path())?;
        runtime.source_windows().await?;
        assert_eq!(
            native.try_recv().context("source listing")?["command"],
            "screen_sources"
        );
        let interrupted = matches!(&command, Command::Interrupt);
        runtime.command(command).await?;
        assert_eq!(
            runtime
                .state
                .views
                .source_request
                .as_ref()
                .context("source progress")?
                .stage,
            orbit_core::sources::SourceStage::Unknown,
            "the completed source cancellation must survive the conversation decision"
        );
        if interrupted {
            assert!(crate::daemon::turn::said::has(
                &runtime.state,
                orbit_core::notice::NoticeCode::SourceFailed
            ));
            assert_eq!(
                runtime.state.notice_code(),
                Some(orbit_core::notice::NoticeCode::AssistantInterruptRequested)
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn interrupt_snapshot_failure_keeps_the_post_predecision_state() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut native) = fixture(directory.path())?;
    runtime.source_windows().await?;
    assert_eq!(
        native.try_recv().context("source listing")?["command"],
        "screen_sources"
    );
    runtime.state.durable.assistant.active_request = Some("turn-in-flight".into());
    runtime.state.durable.assistant.pending_approval = Some(Approval {
        request_id: "turn-in-flight".into(),
        description: "Permiso pendiente".into(),
        clarification: false,
        choices: Vec::new(),
        questions: Vec::new(),
        deletes: None,
    });
    runtime.store_actor.snapshot(&runtime.state).await?;
    runtime.work.durable_snapshot = DurableSnapshot::from(&runtime.state);
    let previous = runtime.state.clone();
    let mut frames = runtime.hub.open(4);
    let database = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    database.execute_batch(
        "CREATE TABLE review_interrupt_snapshot_calls(n INTEGER NOT NULL);
         INSERT INTO review_interrupt_snapshot_calls VALUES (0);
         CREATE TRIGGER review_fail_interrupt_snapshot BEFORE UPDATE ON settings
         WHEN NEW.key='snapshot' BEGIN
           UPDATE review_interrupt_snapshot_calls SET n=n+1;
           SELECT RAISE(ABORT, 'synthetic interrupt decision snapshot failure')
             WHERE (SELECT n FROM review_interrupt_snapshot_calls) = 1;
         END;",
    )?;

    let error = runtime
        .command(Command::Interrupt)
        .await
        .err()
        .context("interrupt snapshot unexpectedly succeeded")?;
    assert!(
        error
            .to_string()
            .contains("synthetic interrupt decision snapshot failure"),
        "fixture failed before the decision snapshot: {error}"
    );
    assert_eq!(
        runtime.state.durable.assistant.active_request,
        previous.durable.assistant.active_request
    );
    assert_eq!(
        runtime.state.durable.assistant.pending_approval,
        previous.durable.assistant.pending_approval
    );
    let source = runtime
        .state
        .views
        .source_request
        .as_ref()
        .context("source receipt")?;
    assert_eq!(source.stage, orbit_core::sources::SourceStage::Unknown);
    assert_eq!(
        runtime.state.notice_code(),
        Some(orbit_core::notice::NoticeCode::SourceFailed)
    );
    let writes: i64 =
        database.query_row("SELECT n FROM review_interrupt_snapshot_calls", [], |row| {
            row.get(0)
        })?;
    assert_eq!(
        writes, 0,
        "the refused decision snapshot must roll back its counter; sources write nothing"
    );
    assert!(
        frames.try_recv().is_err(),
        "failed decision sent a hub frame"
    );
    while let Ok(frame) = native.try_recv() {
        assert_ne!(frame["command"], "playback_stop");
    }
    Ok(())
}
