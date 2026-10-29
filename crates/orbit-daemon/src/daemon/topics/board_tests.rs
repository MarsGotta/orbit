use super::*;

#[test]
fn oversized_topic_is_absent_before_any_transport_reads_it() -> anyhow::Result<()> {
    use orbit_core::{
        AccountSummary,
        protocol::TopicEvent,
        refusal::{Refusal, Resource},
        topics::TOPIC_ENVELOPE_BYTES,
    };
    let mut state = State::default();
    state.views.account = Some(AccountSummary {
        // Each backslash is two bytes of JSON: twice the topic's ceiling.
        device_name: Some("\\".repeat(TopicId::Status.max_bytes()).into()),
        ..AccountSummary::default()
    });
    let mut board = TopicBoard::new("bounded-topic".into());
    let channels = board.channels();
    let receiver = channels[&TopicId::Status].subscribe();
    let source = TopicSource {
        state: &state,
        runtime: None,
        now_ms: 0,
    };
    let unbounded = TopicFrame {
        epoch: board.epoch().into(),
        revision: 1,
        data: project(TopicId::Status, &source),
    };
    let bytes = serde_json::to_vec(&TopicEvent::Changed { frame: unbounded })?.len() + 1;
    board.publish(&source);
    let published = receiver.borrow().clone();
    assert_eq!(published.revision, 1);
    assert!(
        published.data
            == TopicData::absent(
                TopicId::Status,
                Refusal::TooLarge {
                    resource: Resource::IpcResponse,
                    bytes: bytes as u64,
                    max_bytes: (TopicId::Status.max_bytes() + TOPIC_ENVELOPE_BYTES) as u64,
                }
            )
    );
    Ok(())
}

#[test]
fn thousand_audio_frames_keep_unchanged_revisions_at_the_audio_cadence() {
    let state = State::default();
    let source = |now_ms| TopicSource {
        state: &state,
        runtime: None,
        now_ms,
    };
    let mut board = TopicBoard::new("synthetic-epoch".into());
    let channels = board.channels();
    let receivers: Vec<_> = channels.values().map(watch::Sender::subscribe).collect();
    assert_eq!(board.publish(&source(0)).len(), TopicId::ALL.len());
    let mut projections = 0;
    for frame in 0..1_000 {
        board.state_changed();
        let now_ms = frame;
        let before = board.cells[&TopicId::Recording].last_projected_ms;
        assert!(board.publish(&source(now_ms)).is_empty());
        if board.cells[&TopicId::Recording].last_projected_ms != before {
            projections += 1;
        }
        for (id, expected) in board.recompute_all(&source(now_ms)) {
            assert_eq!(board.cells[&id].value, expected);
            let published = channels[&id].borrow();
            assert_eq!(published.data, expected);
            assert_eq!(published.revision, board.cells[&id].revision);
        }
    }
    assert_eq!(projections, 3);
    for sender in channels.values() {
        assert_eq!(sender.borrow().revision, 1);
    }
    assert_eq!(receivers.len(), TopicId::ALL.len());
    assert_eq!(board.revisions()[&TopicId::Recording], 1);
}

#[test]
fn topics_have_independent_cells_and_epoch() {
    let state = State::default();
    let source = |now_ms| TopicSource {
        state: &state,
        runtime: None,
        now_ms,
    };
    let mut board = TopicBoard::new("one".into());
    let channels = board.channels();
    let recording = channels[&TopicId::Recording].subscribe();
    let status = channels[&TopicId::Status].subscribe();
    board.publish(&source(0));
    board.state_changed();
    board.publish(&source(250));
    assert_eq!(status.borrow().revision, 1);
    assert_eq!(recording.borrow().revision, 1);
    assert_eq!(
        board.cells[&TopicId::Recording].last_projected_ms,
        Some(250)
    );
    let next = TopicBoard::new("two".into());
    assert_ne!(
        recording.borrow().epoch,
        next.channels()[&TopicId::Recording]
            .subscribe()
            .borrow()
            .epoch
    );
}

#[test]
fn idle_recording_has_no_periodic_deadline_or_projection() {
    let state = State::default();
    let source = |now_ms| TopicSource {
        state: &state,
        runtime: None,
        now_ms,
    };
    let mut board = TopicBoard::new("one".into());
    let receiver = board.channels()[&TopicId::Recording].subscribe();
    assert!(board.publish(&source(0)).contains(&TopicId::Recording));
    assert!(!board.publish(&source(249)).contains(&TopicId::Recording));
    assert!(!board.publish(&source(250)).contains(&TopicId::Recording));
    assert_eq!(receiver.borrow().revision, 1);
    assert_eq!(board.next_due_ms(), None);
    assert_eq!(board.cells[&TopicId::Recording].last_published_ms, Some(0));
    assert_eq!(board.cells[&TopicId::Recording].last_projected_ms, Some(0));
}

#[test]
fn unchanged_periodic_projection_does_not_delay_a_dirty_audio_projection() {
    let state = State::default();
    let source = |now_ms| TopicSource {
        state: &state,
        runtime: None,
        now_ms,
    };
    let mut board = TopicBoard::new("one".into());
    let _receiver = board.channels()[&TopicId::Recording].subscribe();
    board.publish(&source(0));
    assert!(!board.publish(&source(250)).contains(&TopicId::Recording));
    board.state_changed();
    assert!(!board.publish(&source(251)).contains(&TopicId::Recording));
    assert_eq!(
        board.cells[&TopicId::Recording].last_projected_ms,
        Some(251)
    );
    assert!(!board.cells[&TopicId::Recording].dirty);
}

#[test]
fn idle_notes_have_no_deadline_but_changed_inputs_still_publish() {
    let state = State::default();
    let mut board = TopicBoard::new("idle-notes".into());
    let receiver = board.channels()[&TopicId::Notes].subscribe();
    board.publish(&TopicSource {
        state: &state,
        runtime: None,
        now_ms: 0,
    });
    assert_eq!(board.next_due_ms(), None);
    board.state_changed();
    assert_eq!(board.next_due_ms(), Some(1_000));
    board.publish(&TopicSource {
        state: &state,
        runtime: None,
        now_ms: 1_000,
    });
    assert_eq!(receiver.borrow().revision, 1);
    assert_eq!(board.next_due_ms(), None);
}

#[test]
fn a_live_recording_keeps_its_deadline_until_the_finish_input() -> anyhow::Result<()> {
    let mut state = State::default();
    let mut board = TopicBoard::new("active-recording".into());
    let receiver = board.channels()[&TopicId::Recording].subscribe();
    board.publish(&TopicSource {
        state: &state,
        runtime: None,
        now_ms: 0,
    });
    assert_eq!(board.next_due_ms(), None);
    state.durable.capture.recording = Some(serde_json::from_value(serde_json::json!({
        "id": "recording-one", "title": "Budget review", "started_at_ms": 1,
        "ended_at_ms": null, "expires_at_ms": 60_001, "application_pid": null,
        "audio": "system", "phase": "recording", "sealed_chunks": 0,
        "gaps": 0, "tracks": [], "finalizing_since_ms": null
    }))?);
    board.state_changed();
    board.publish_command(&TopicSource {
        state: &state,
        runtime: None,
        now_ms: 1,
    });
    assert_eq!(receiver.borrow().revision, 2);
    assert_eq!(board.next_due_ms(), Some(251));
    board.publish(&TopicSource {
        state: &state,
        runtime: None,
        now_ms: 251,
    });
    assert_eq!(board.next_due_ms(), Some(501));
    state.durable.capture.recording = None;
    board.state_changed();
    board.publish_command(&TopicSource {
        state: &state,
        runtime: None,
        now_ms: 252,
    });
    assert_eq!(receiver.borrow().revision, 3);
    assert_eq!(board.next_due_ms(), None);
    Ok(())
}
