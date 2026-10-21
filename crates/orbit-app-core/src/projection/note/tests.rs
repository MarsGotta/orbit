use super::*;

#[test]
fn an_unverifiable_inventory_is_unavailable_with_its_numbers_not_empty() {
    let head = orbit_core::topics::recording::Value {
        recording: None,
        recording_signal: None,
        pending_jobs: 0,
        offline: false,
        call: None,
        question: None,
        voice_trigger: None,
        applications: orbit_core::topics::Knowledge::Absent {
            refusal: orbit_core::refusal::Refusal::TooLarge {
                resource: orbit_core::refusal::Resource::Source,
                bytes: 40,
                max_bytes: 32,
            },
        },
    };
    let meeting = crate::projection::project_meeting_topic(&head);
    assert_eq!(
        meeting.applications_state,
        MeetingApplicationsState::Unavailable
    );
    assert!(
        meeting
            .applications_detail
            .as_deref()
            .is_some_and(|detail| detail.contains("40") && detail.contains("32"))
    );
}

fn hub_document() -> orbit_core::notes::MeetingDocument {
    serde_json::from_value(serde_json::json!({
        "recording_id": "6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f",
        "title": "Comité de dirección", "started_at_ms": 1_800_000_000_000_i64,
        "duration_ms": 120_000, "capture": "closed", "language": "es",
        "speakers": [
            {"key": "mic_local", "track": "mic_local", "name": "Alina"},
            {"key": "remote_app", "track": "remote_app", "name": "Otra persona"}
        ],
        "transcript": {"state": "ready", "pending_chunks": 0, "failed_groups": []},
        "note": {
            "state": "ready", "revision": 3, "attempts": 1, "error": null,
            "template_id": "general",
            "note": {"short_summary": "Resumen.", "sections": [{"name": "Resumen", "markdown": "- a"}],
                "action_items": [{"id": "a0123456789ab", "owner": "Alina", "text": "Acta", "done": true}],
                "decisions": ["Sí"], "open_topics": [], "template_id": "general",
                "template_name": "General", "model": "qwen", "generated_at_ms": 5},
            "edited": {"sections": true}, "superseded_available": true, "updated_at_ms": 5
        },
        "private_notes": {"text": "- mías", "revision": 2}
    }))
    .expect("a document of the contract")
}

/// Defects 1–4 at app-core: the open note carries the hub's times, names
/// and language as they came, and the turns it was given.
#[test]
fn the_open_note_carries_what_the_hub_computed() {
    let turns = vec![
        orbit_core::notes::Turn {
            at_ms: 0,
            speaker_key: "mic_local".into(),
            text: "Empezamos.".into(),
        },
        orbit_core::notes::Turn {
            at_ms: 12_000,
            speaker_key: "remote_app".into(),
            text: "Vale.".into(),
        },
    ];
    let note = crate::projection::note::project_open_note(hub_document(), Ok(turns), None);
    assert_eq!(note.title, "Comité de dirección");
    assert_eq!(note.transcript.language.as_deref(), Some("es"));
    assert_eq!(
        note.transcript
            .turns
            .iter()
            .map(|turn| (turn.at_ms, turn.speaker_key.as_str()))
            .collect::<Vec<_>>(),
        [(0, "mic_local"), (12_000, "remote_app")]
    );
    assert_eq!(note.speakers[0].name, "Alina");
    assert_eq!(note.speakers[0].share_pct, 50);
    assert_eq!(note.owner_name, "Alina");
    assert_eq!(note.ended_at_ms, 1_800_000_120_000);
    assert_eq!(note.note_revision, Some(3));
    assert_eq!(note.private_notes, "- mías");
    assert_eq!(note.private_notes_revision, 2);
    let summary = note.summary.expect("the note is written");
    assert!(summary.sections[0].edited);
    assert!(summary.superseded_available);
    assert!(note.actions[0].done);
    assert_eq!(note.progress.summary.state, crate::NoteStepState::Done);
}

#[test]
fn progress_follows_the_recording_the_transcript_and_the_note() {
    use crate::{NoteStepCause, NoteStepState};
    let mut document = hub_document();
    document.note = None;
    document.transcript.state = orbit_core::notes::TranscriptState::Pending;
    document.transcript.pending_chunks = 4;
    document.local = Some(orbit_core::notes::LocalRecording {
        phase: orbit_core::notes::LocalPhase::Recording,
        pending_chunks: Some(1),
    });
    let progress = crate::projection::note::project_progress(&document);
    assert_eq!(progress.upload.cause, Some(NoteStepCause::Recording {}));
    assert_eq!(
        progress.transcript.cause,
        Some(NoteStepCause::Transcribing { chunks: 4 })
    );
    assert_eq!(
        progress.summary.cause,
        Some(NoteStepCause::AfterRecording {})
    );
    document.local = None;
    document.transcript.failed_groups = vec![orbit_core::notes::FailedGroup {
        track: Some("remote_app".into()),
        cause: Some("timeout".into()),
        attempts: 3,
    }];
    let progress = crate::projection::note::project_progress(&document);
    assert_eq!(progress.upload.state, NoteStepState::Done);
    assert_eq!(progress.transcript.state, NoteStepState::Failed);
    assert_eq!(
        progress.transcript.cause,
        Some(NoteStepCause::TranscriptFailed { groups: 1 })
    );
}

/// While a change of language repeats the meeting, the open note
/// says how far it got, in the hub's numbers.
#[test]
fn the_open_note_carries_the_repetition_of_a_language_change() {
    let mut document = hub_document();
    document.language = Some("en".into());
    document.transcript.retranscribing = Some(orbit_core::notes::Retranscribing {
        language: "en".into(),
        done: 1,
        total: 4,
    });
    let note = crate::projection::note::project_open_note(document, Ok(Vec::new()), None);
    assert_eq!(note.transcript.language.as_deref(), Some("en"));
    assert_eq!(
        note.transcript.retranscribing,
        Some(NoteRetranscribingSnapshot {
            language: "en".into(),
            done: 1,
            total: 4,
        })
    );
    let done = crate::projection::note::project_open_note(hub_document(), Ok(Vec::new()), None);
    assert_eq!(done.transcript.retranscribing, None);
}

#[test]
fn event_invitees_survive_the_note_projection() {
    let mut document = hub_document();
    document.event = Some(
        serde_json::from_value(serde_json::json!({
            "title": "Comité", "starts_at_ms": 1, "ends_at_ms": 2,
            "calendar_item_id": "series",
            "attendees": [{"name": "Lucía", "email": "lucia@example.invalid", "is_self": false}]
        }))
        .expect("an event of the contract"),
    );
    let note = crate::projection::note::project_open_note(document, Ok(Vec::new()), None);
    let event = serde_json::to_value(note.event).expect("serializable event");
    assert_eq!(event["attendees"][0]["name"], "Lucía");
    assert_eq!(event["attendees"][0]["email"], "lucia@example.invalid");
    assert_eq!(event["attendees"][0]["is_self"], false);
}

#[test]
fn final_pass_state_and_failure_reach_the_open_transcript() {
    for state in ["queued", "running", "completed", "failed"] {
        let mut raw = serde_json::to_value(hub_document()).unwrap();
        raw["transcript"]["final_pass"] = serde_json::json!({
            "state": state,
            "error": if state == "failed" {
                serde_json::json!({"code":"final_pass_incomplete", "draft_words":42,"final_words":2})
            } else { serde_json::Value::Null }
        });
        raw["transcript"]["unmatched_speaker_names"] = serde_json::json!(1);
        let document = serde_json::from_value(raw).unwrap();
        let projected =
            serde_json::to_value(project_open_note(document, Ok(vec![]), None)).unwrap();
        assert_eq!(projected["transcript"]["final_pass"]["state"], state);
        assert_eq!(projected["transcript"]["unmatched_speaker_names"], 1);
        if state == "failed" {
            let cause = projected["transcript"]["final_pass"]["error"]
                .as_str()
                .unwrap();
            assert_eq!(cause, "final_pass_incomplete");
            assert!(
                projected["transcript"]["final_pass"]
                    .get("timeout_ms")
                    .is_none()
            );
        }
    }
    let old = serde_json::to_value(project_open_note(hub_document(), Ok(vec![]), None)).unwrap();
    assert!(old["transcript"]["final_pass"].is_null());
}

#[test]
fn final_pass_timeout_projects_its_budget_without_other_diagnostics() {
    let mut raw = serde_json::to_value(hub_document()).unwrap();
    raw["transcript"]["final_pass"] = serde_json::json!({
        "state":"failed", "error": {
            "code":"final_pass_timeout", "timeout_ms":90000, "phase":"diarization"
        }
    });
    let document = serde_json::from_value(raw).unwrap();
    let projected = serde_json::to_value(project_open_note(document, Ok(vec![]), None)).unwrap();
    assert_eq!(
        projected["transcript"]["final_pass"],
        serde_json::json!({
            "state":"failed", "error":"final_pass_timeout", "timeout_ms":90000
        })
    );
}
