#![allow(clippy::unwrap_used)]
use super::*;
use orbit_core::{
    State,
    topics::{Knowledge, TopicData},
    voice::{CallPhase, CallState},
};

fn source(state: &State) -> TopicSource<'_> {
    TopicSource {
        state,
        runtime: None,
        now_ms: 0,
    }
}

#[test]
fn notices_publish_every_new_notice() {
    use orbit_core::notice::NoticeCode;
    let mut state = State::default();
    let initial = project(TopicId::Notices, &source(&state));
    assert!(matches!(
        initial,
        TopicData::Notices(Knowledge::Present { .. })
    ));
    let mut board = TopicBoard::new("notices".into());
    let receiver = board.channels()[&TopicId::Notices].subscribe();
    board.publish(&source(&state));
    let initial_revision = receiver.borrow().revision;
    state.set_notice(NoticeCode::PrivacyStopped);
    board.state_changed();
    board.publish(&source(&state));
    assert!(receiver.borrow().revision > initial_revision);
    let after_stop = receiver.borrow().revision;
    state.set_notice(NoticeCode::StopStepFailed);
    board.state_changed();
    board.publish(&source(&state));
    assert!(receiver.borrow().revision > after_stop);
    let frame = serde_json::to_value(&receiver.borrow().data).unwrap();
    assert_eq!(
        frame["knowledge"]["value"]["notice"]["code"],
        "stop_step_failed"
    );
}

/// A notice is a code, an op and closed data, so its frame has a
/// ceiling of its own. Every key filled with the longest token and the
/// longest op still fits the registered frame.
#[test]
fn notices_frame_fits_with_every_key_at_its_longest() {
    use orbit_core::notice::{Notice, NoticeCode, NoticeKey, Op};
    let mut notice = Notice::new(NoticeCode::NoteChangeUnsent);
    for key in NoticeKey::ALL {
        notice = notice.with_token(*key, &"x".repeat(orbit_core::notice::Token::MAX_LEN));
    }
    notice.op = Op::parse(&"a".repeat(Op::MAX_LEN));
    let mut state = State::default();
    state.set_notice(notice);
    let data = project(TopicId::Notices, &source(&state));
    assert!(matches!(
        data,
        TopicData::Notices(Knowledge::Present { .. })
    ));
    let bytes = serde_json::to_vec(&data).unwrap().len();
    eprintln!(
        "notice frame: keys={} frame_bytes={bytes}",
        NoticeKey::ALL.len()
    );
    assert!(bytes <= TopicId::Notices.max_bytes());
}

#[test]
fn assistant_call_is_known_and_missing_wire_defaults_to_unknown() {
    let mut state = State::default();
    let call = CallState {
        id: "synthetic".into(),
        phase: CallPhase::Starting,
        started_at_ms: 12,
        expires_at_ms: 60_012,
        turns: 0,
        origin: orbit_core::CallOrigin::App,
    };
    state.durable.voice.call = Some(call.clone());
    let active = serde_json::to_value(project(TopicId::Assistant, &source(&state))).unwrap();
    assert_eq!(
        active["knowledge"]["value"]["call"]["value"]["phase"],
        "starting"
    );
    state.durable.voice.call = None;
    let stopped = serde_json::to_value(project(TopicId::Assistant, &source(&state))).unwrap();
    assert_eq!(
        stopped["knowledge"]["value"]["call"]["value"],
        serde_json::Value::Null
    );
    let mut old = active["knowledge"]["value"].clone();
    old.as_object_mut().unwrap().remove("call");
    let decoded: orbit_core::topics::assistant::Value = serde_json::from_value(old).unwrap();
    let decoded = serde_json::to_value(decoded).unwrap();
    assert_eq!(decoded["call"]["state"], "unknown");

    let mut board = TopicBoard::new("call".into());
    let receiver = board.channels()[&TopicId::Assistant].subscribe();
    board.publish(&source(&state));
    let initial_revision = receiver.borrow().revision;
    state.durable.voice.call = Some(call);
    board.state_changed();
    board.publish_command(&source(&state));
    assert!(receiver.borrow().revision > initial_revision);
    let active_revision = receiver.borrow().revision;
    state.durable.voice.call = None;
    board.state_changed();
    board.publish_command(&source(&state));
    assert!(receiver.borrow().revision > active_revision);
    let stopped = serde_json::to_value(&receiver.borrow().data).unwrap();
    assert_eq!(
        stopped["knowledge"]["value"]["call"]["value"],
        serde_json::Value::Null
    );
}

#[test]
fn command_publication_bypasses_audio_cadence() {
    let mut state = State::default();
    let mut board = TopicBoard::new("synthetic".into());
    let receiver = board.channels()[&TopicId::Recording].subscribe();
    board.publish(&source(&state));
    let before = receiver.borrow().revision;
    state.durable.pending_jobs = 1;
    board.state_changed();
    board.publish_command(&source(&state));
    assert!(
        receiver.borrow().revision > before,
        "command still waits for recording cadence"
    );
}

#[test]
fn meeting_and_privacy_frames_match_command_receipt_revisions() -> anyhow::Result<()> {
    use orbit_core::{CapturePhase, Recording, protocol::MeetingAudio};
    let directory = tempfile::tempdir()?;
    let (mut runtime, _helper) = crate::daemon::audio::tests::fixture(directory.path())?;
    let mut board = TopicBoard::new("receipt-epoch".into());
    let channels = board.channels();
    let recording = channels[&TopicId::Recording].subscribe();
    let status = channels[&TopicId::Status].subscribe();
    let control = channels[&TopicId::Control].subscribe();
    board.publish(&TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 0,
    });
    runtime.state.durable.capture.recording = Some(Recording {
        id: "synthetic-recording".into(),
        title: "Prueba".into(),
        started_at_ms: 10,
        ended_at_ms: None,
        expires_at_ms: 60_010,
        application_pid: None,
        audio: MeetingAudio::System,
        phase: CapturePhase::Starting,
        sealed_chunks: 0,
        gaps: 0,
        tracks: Vec::new(),
        finalizing_since_ms: None,
        ceiling_notice_given: false,
        muted_spans: Vec::new(),
    });
    board.state_changed();
    board.state_changed();
    board.publish_command(&TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 0,
    });
    let receipt = board.revisions();
    assert_eq!(recording.borrow().revision, receipt[&TopicId::Recording]);
    let recording_json = serde_json::to_value(&recording.borrow().data)?;
    assert_eq!(
        recording_json["knowledge"]["value"]["recording"]["phase"],
        "starting"
    );

    {
        let fixture_value = serde_json::json!(true);
        crate::daemon::authority_fixture::set(&mut runtime.state, "privacy_stopped", fixture_value);
    }
    board.state_changed();
    board.state_changed();
    board.publish_command(&TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 0,
    });
    let receipt = board.revisions();
    assert_eq!(status.borrow().revision, receipt[&TopicId::Status]);
    assert_eq!(control.borrow().revision, receipt[&TopicId::Control]);
    let status_json = serde_json::to_value(&status.borrow().data)?;
    assert_eq!(status_json["knowledge"]["value"]["privacy_stopped"], true);
    let control_json = serde_json::to_value(&control.borrow().data)?;
    assert_eq!(
        control_json["knowledge"]["value"]["engine"]["refusal"]["code"],
        "privacy_stopped"
    );
    Ok(())
}

#[test]
fn quiet_notices_assistant_and_notes_publish_no_new_revisions() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let (runtime, _helper) = crate::daemon::audio::tests::fixture(directory.path())?;
    let mut board = TopicBoard::new("quiet".into());
    let channels = board.channels();
    let _receivers: Vec<_> = [TopicId::Notices, TopicId::Assistant, TopicId::Notes]
        .into_iter()
        .map(|id| channels[&id].subscribe())
        .collect();
    board.publish(&TopicSource {
        state: &runtime.state,
        runtime: Some(&runtime),
        now_ms: 0,
    });
    let before = board.revisions();
    for second in 1..=10 {
        board.publish(&TopicSource {
            state: &runtime.state,
            runtime: Some(&runtime),
            now_ms: second * 1_000,
        });
    }
    assert_eq!(board.revisions(), before);
    Ok(())
}
