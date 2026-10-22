use super::*;
use crate::hub::HubEvent;
use orbit_core::{
    Message, Turn,
    notice::{Notice, NoticeCode},
    topics::Knowledge,
    wire::hub::HubEvent as WireHubEvent,
};

#[test]
fn unsubscribed_topics_are_not_projected_even_for_commands() {
    let state = State::default();
    let mut board = TopicBoard::new("unsubscribed".into());
    let source = TopicSource {
        state: &state,
        runtime: None,
        now_ms: 0,
    };
    assert!(board.publish(&source).is_empty());
    assert!(board.publish_command(&source).is_empty());
    assert!(
        board
            .cells
            .values()
            .all(|cell| cell.last_projected_ms.is_none() && cell.revision == 0)
    );
    assert_eq!(board.next_due_ms(), None);
}

#[tokio::test]
async fn event_sequence_preserves_frames_revisions_and_recomputation() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut helper) = crate::daemon::audio::tests::fixture(directory.path())?;
    crate::daemon::calendar::tests::seed(&mut runtime, &mut helper).await?;
    let mut board = TopicBoard::new("sequence".into());
    let channels = board.channels();
    let _receivers: Vec<_> = channels.values().map(watch::Sender::subscribe).collect();
    for step in 0..=24_u64 {
        let now_ms = step * 125;
        match step {
            1 => {
                let event = serde_json::json!({"event":"calendar_invalidated"});
                runtime.native_event(event).await?;
            }
            2 => {
                let event = HubEvent::Decoded(Box::new(
                    WireHubEvent::decode(serde_json::from_str(include_str!(
                        "../../../../../contracts/fixtures/hub/events/chat.delta.json"
                    ))?)
                    .map_err(anyhow::Error::msg)?,
                ));
                runtime.hub_event(event).await?;
            }
            3 => {
                runtime.state.durable.turn = Some(Turn {
                    user: Message {
                        attachments: Vec::new(),
                        id: "question".into(),
                        role: "user".into(),
                        text: "Prueba".into(),
                        complete: true,
                        at_ms: 0,
                        voice: None,
                    },
                    answer: Some(Message {
                        attachments: Vec::new(),
                        id: "answer".into(),
                        role: "assistant".into(),
                        text: "Respuesta".into(),
                        complete: false,
                        at_ms: 0,
                        voice: None,
                    }),
                });
            }
            4 => {
                runtime
                    .state
                    .set_notice(Notice::new(NoticeCode::NoteChangeUnsent));
            }
            5 => {
                runtime
                    .features
                    .notes
                    .head
                    .touch_recording("synthetic-recording");
            }
            8 => {
                crate::daemon::authority_fixture::set(
                    &mut runtime.state,
                    "privacy_stopped",
                    serde_json::json!(true),
                );
            }
            10 => {
                crate::daemon::authority_fixture::set(
                    &mut runtime.state,
                    "privacy_stopped",
                    serde_json::json!(false),
                );
            }
            11 => {
                let event = HubEvent::Decoded(Box::new(
                    WireHubEvent::decode(serde_json::from_str(include_str!(
                        "../../../../../contracts/fixtures/hub/events/pong.json"
                    ))?)
                    .map_err(anyhow::Error::msg)?,
                ));
                runtime.hub_event(event).await?;
            }
            12..=24 => {}
            0 | 6 | 7 | 9 => {}
            _ => unreachable!("all sequence steps are covered"),
        }
        board.state_changed();
        let source = TopicSource {
            state: &runtime.state,
            runtime: Some(&runtime),
            now_ms,
        };
        if step == 8 || step == 10 {
            board.publish_command(&source);
        } else {
            board.publish(&source);
        }
        for (id, expected) in board.recompute_all(&source) {
            let frame = channels[&id].borrow();
            assert_eq!(frame.data, expected, "step={step} topic={id:?}");
            assert_eq!(frame.revision, board.revisions()[&id]);
        }
        for (id, sender) in &channels {
            let frame = sender.borrow();
            assert!(
                matches!(frame.data, TopicData::Status(Knowledge::Present { .. }))
                    || *id != TopicId::Status
            );
        }
    }
    Ok(())
}

#[test]
fn rearm_projects_recording_without_waiting_for_periodic_tick() {
    let mut state = State::default();
    crate::daemon::authority_fixture::set(&mut state, "privacy_stopped", serde_json::json!(true));
    let mut board = TopicBoard::new("rearm".into());
    let receiver = board.channels()[&TopicId::Recording].subscribe();
    board.publish(&TopicSource {
        state: &state,
        runtime: None,
        now_ms: 0,
    });
    crate::daemon::authority_fixture::set(&mut state, "privacy_stopped", serde_json::json!(false));
    board.state_changed();
    let source = TopicSource {
        state: &state,
        runtime: None,
        now_ms: 125,
    };
    board.publish_command(&source);
    assert_eq!(receiver.borrow().data, project(TopicId::Recording, &source));
    assert_eq!(receiver.borrow().revision, 2);
}

#[test]
fn control_retry_deadline_and_cause_publish_but_countdown_does_not() -> anyhow::Result<()> {
    use orbit_core::topics::control::engine_knowledge_with_permissions;
    use orbit_core::{PermissionState, computer_control::engine::Status, refusal::EngineCause};
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = crate::daemon::audio::tests::fixture(directory.path())?;
    crate::daemon::authority_fixture::set(
        &mut runtime.state,
        "capabilities.accessibility_state",
        serde_json::json!(PermissionState::Granted),
    );
    runtime.features.computer.engine_failure = Some("synthetic".into());
    runtime.features.computer.engine_failure_kind = Some(EngineCause::InitializationFailed);
    runtime.features.computer.engine_retry_at_ms = Some(10_000);
    let mut board = TopicBoard::new("retry".into());
    let receiver = board.channels()[&TopicId::Control].subscribe();
    let publish = |board: &mut TopicBoard, runtime: &Runtime| {
        board.state_changed();
        board.publish(&TopicSource {
            state: &runtime.state,
            runtime: Some(runtime),
            now_ms: 0,
        });
    };
    publish(&mut board, &runtime);
    let first = receiver.borrow().clone();
    let failed = |retry_in_ms| {
        engine_knowledge_with_permissions(
            Status::Failed {
                cause: "synthetic".into(),
                retry_in_ms,
            },
            Some(EngineCause::InitializationFailed),
            PermissionState::Granted,
            PermissionState::Granted,
        )
    };
    assert_eq!(failed(5_000), failed(3_000));
    runtime.features.computer.engine_failure = Some("same cause, different diagnostic".into());
    publish(&mut board, &runtime);
    assert_eq!(*receiver.borrow(), first);
    runtime.features.computer.engine_retry_at_ms = Some(15_000);
    publish(&mut board, &runtime);
    assert_eq!(receiver.borrow().revision, first.revision + 1);
    runtime.features.computer.engine_failure_kind = Some(EngineCause::Unhealthy);
    publish(&mut board, &runtime);
    assert_eq!(receiver.borrow().revision, first.revision + 2);
    let frame = serde_json::to_value(&*receiver.borrow())?;
    assert_eq!(
        frame["data"]["knowledge"]["value"]["engine"]["refusal"]["cause"],
        "unhealthy"
    );
    runtime.features.computer.engine_failure = None;
    runtime.features.computer.engine_failure_kind = None;
    runtime.features.computer.engine_retry_at_ms = None;
    publish(&mut board, &runtime);
    assert_eq!(receiver.borrow().revision, first.revision + 3);
    for (id, expected) in board.recompute_all(&TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 0,
    }) {
        assert_eq!(board.cells[&id].value, expected);
    }
    Ok(())
}

#[tokio::test]
async fn decoded_chat_events_publish_their_facts_and_pong_keeps_the_revision() -> anyhow::Result<()>
{
    const REQUEST: &str = "00000000-0000-4000-8000-000000000901";
    const SESSION: &str = "00000000-0000-4000-8000-000000000902";
    let cases = [
        (
            include_str!("../../../../../contracts/fixtures/hub/events/chat.session.json"),
            "/session_id",
            SESSION,
        ),
        (
            include_str!("../../../../../contracts/fixtures/hub/events/chat.delta.json"),
            "/turn/answer/text",
            "parcial",
        ),
        (
            include_str!("../../../../../contracts/fixtures/hub/events/chat.completed.json"),
            "/turn/answer/text",
            "final",
        ),
        (
            include_str!("../../../../../contracts/fixtures/hub/events/approval.required.json"),
            "/pending/request_id",
            REQUEST,
        ),
        (
            include_str!("../../../../../contracts/fixtures/hub/events/clarify.required.json"),
            "/pending/request_id",
            REQUEST,
        ),
    ];
    for (json, pointer, expected) in cases {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _helper) = crate::daemon::audio::tests::fixture(directory.path())?;
        if pointer != "/session_id" {
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "session_id",
                serde_json::json!(SESSION),
            );
        }
        runtime.state.durable.active_request = Some(REQUEST.into());
        runtime.state.durable.turn = Some(Turn {
            user: Message {
                attachments: Vec::new(),
                id: REQUEST.into(),
                role: "user".into(),
                text: "Pregunta".into(),
                complete: true,
                at_ms: 0,
                voice: None,
            },
            answer: None,
        });
        let mut board = TopicBoard::new("decoded".into());
        let receiver = board.channels()[&TopicId::Assistant].subscribe();
        board.publish(&TopicSource {
            state: &runtime.state,
            runtime: Some(&runtime),
            now_ms: 0,
        });
        let mut event: serde_json::Value = serde_json::from_str(json)?;
        if pointer == "/pending/request_id" {
            // The decoder fixtures use an independent interaction ID. This
            // active-turn case needs the core's matching request correlation.
            event["data"]["request_id"] = serde_json::json!(REQUEST);
        }
        let decoded = WireHubEvent::decode(event).map_err(anyhow::Error::msg)?;
        runtime
            .hub_event(HubEvent::Decoded(Box::new(decoded)))
            .await?;
        board.state_changed();
        let source = TopicSource {
            state: &runtime.state,
            runtime: Some(&runtime),
            now_ms: 100,
        };
        board.publish(&source);
        assert_eq!(receiver.borrow().revision, 2, "{pointer}");
        let frame = serde_json::to_value(&receiver.borrow().data)?;
        assert_eq!(
            frame["knowledge"]["value"]
                .pointer(pointer)
                .and_then(serde_json::Value::as_str),
            Some(expected)
        );
        let before = receiver.borrow().clone();
        let pong = WireHubEvent::decode(serde_json::from_str(include_str!(
            "../../../../../contracts/fixtures/hub/events/pong.json"
        ))?)
        .map_err(anyhow::Error::msg)?;
        runtime.hub_event(HubEvent::Decoded(Box::new(pong))).await?;
        board.state_changed();
        board.publish(&TopicSource {
            state: &runtime.state,
            runtime: Some(&runtime),
            now_ms: 200,
        });
        assert_eq!(*receiver.borrow(), before);
    }
    Ok(())
}

#[test]
fn a_short_recording_does_not_publish_an_intermediate_notes_head_before_its_cadence()
-> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = crate::daemon::audio::tests::fixture(directory.path())?;
    let mut board = TopicBoard::new("notes-cadence".into());
    let receiver = board.channels()[&TopicId::Notes].subscribe();
    board.publish(&TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 0,
    });
    let recording_id = "00000000-0000-4000-8000-000000000001";
    runtime.state.durable.recording = Some(orbit_core::Recording {
        id: recording_id.into(),
        title: "Prueba".into(),
        started_at_ms: 0,
        ended_at_ms: None,
        expires_at_ms: 60_000,
        application_pid: None,
        audio: orbit_core::protocol::MeetingAudio::System,
        phase: orbit_core::CapturePhase::Starting,
        sealed_chunks: 0,
        gaps: 0,
        tracks: Vec::new(),
        finalizing_since_ms: None,
        ceiling_notice_given: false,
    });
    board.state_changed();
    board.publish_command(&TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 1,
    });
    assert_eq!(
        receiver.borrow().revision,
        1,
        "a recording command must keep the Notes cadence"
    );
    runtime.state.durable.recording = None;
    runtime.features.notes.head.touch_recording(recording_id);
    board.state_changed();
    board.publish(&TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 999,
    });
    assert_eq!(receiver.borrow().revision, 1);
    assert_eq!(board.next_due_ms(), Some(1_000));
    board.publish(&TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 1_000,
    });
    assert_eq!(receiver.borrow().revision, 2);
    let frame = serde_json::to_value(&receiver.borrow().data)?;
    assert_eq!(frame["knowledge"]["value"]["revision"], 1);
    assert!(frame["knowledge"]["value"].get("recording").is_none());
    Ok(())
}
