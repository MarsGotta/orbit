//! What the app receives about a question to Deimos during the
//! meeting — the question and its answer joined from the conversation, the
//! button's windows from orbit-core, the detector's state — and the orders
//! the pill and the panel send.
#![allow(clippy::expect_used)]
use orbit_app_core::{
    AppSnapshot, MeetingQuestionCause, MeetingQuestionStage, VoiceOffCause, VoiceStatusSnapshot,
    features::{FeatureInput, recording},
    topics::TopicCache,
};
use orbit_core::{
    CapturePhase, Recording,
    meeting_question::{
        DEFAULT_WINDOW_MINUTES, MAX_QUESTION_BYTES, MeetingQuestion, NotListening, QuestionCause,
        QuestionOrigin, VoiceStatus, WINDOW_MINUTES,
    },
    topics::{Knowledge, Pending, TopicData, TopicFrame},
};
use serde_json::json;

const START: i64 = 1_800_000_000_000;

fn recording_head(
    phase: CapturePhase,
    question: Option<MeetingQuestion>,
    trigger: Option<VoiceStatus>,
) -> TopicFrame {
    TopicFrame {
        epoch: "meeting-question".into(),
        revision: 1,
        data: TopicData::Recording(Knowledge::Present {
            value: orbit_core::topics::recording::Value {
                recording: Some(Box::new(Recording {
                    id: "recording-1".into(),
                    title: "Comité".into(),
                    started_at_ms: START,
                    ended_at_ms: None,
                    expires_at_ms: START + 3_600_000,
                    application_pid: None,
                    audio: orbit_core::protocol::MeetingAudio::System,
                    phase,
                    sealed_chunks: 0,
                    gaps: 0,
                    tracks: Vec::new(),
                    finalizing_since_ms: None,
                    ceiling_notice_given: false,
                    muted_spans: Vec::new(),
                })),
                recording_signal: None,
                pending_jobs: 0,
                offline: false,
                call: None,
                question: question.map(Box::new),
                voice_trigger: trigger,
                microphone_muted: false,
                applications: Knowledge::Unknown {
                    why: Pending::NotObservedYet,
                },
            },
            as_of_ms: None,
        }),
    }
}

fn assistant(turn: serde_json::Value) -> TopicFrame {
    let value: orbit_core::topics::assistant::Value = serde_json::from_value(json!({
        "session_id": null, "turn": turn,
        "pending": null, "voice_active": false, "turn_in_flight": true, "activity": null,
        "voice": {"state": "unknown", "why": "not_observed_yet"},
        "call": {"state": "present", "value": null}
    }))
    .expect("typed assistant fixture");
    TopicFrame {
        epoch: "meeting-question".into(),
        revision: 1,
        data: TopicData::Assistant(Knowledge::Present {
            value,
            as_of_ms: None,
        }),
    }
}

fn asked() -> MeetingQuestion {
    MeetingQuestion::thinking(
        "turn-1".into(),
        "recording-1".into(),
        "¿Qué dijo Lucía?".into(),
        2,
        QuestionOrigin::Voice,
        START,
    )
}

fn view(frames: Vec<TopicFrame>) -> orbit_app_core::MeetingSnapshot {
    let mut topics = TopicCache::default();
    for frame in frames {
        topics.observe_at(frame, Some(1));
    }
    let local = AppSnapshot::default();
    recording::project(&FeatureInput::new(&local, &topics)).meeting
}

#[test]
fn the_panel_says_deimos_is_thinking_then_shows_the_answer_as_it_arrives() {
    let thinking = view(vec![recording_head(
        CapturePhase::Recording,
        Some(asked()),
        None,
    )]);
    let question = thinking.question.expect("the question reaches the app");
    assert_eq!(question.stage, MeetingQuestionStage::Thinking);
    assert_eq!(question.question, "¿Qué dijo Lucía?");
    assert_eq!(question.answer, None);

    let partial = view(vec![
        recording_head(CapturePhase::Recording, Some(asked()), None),
        assistant(json!({
            "user": {"id": "turn-1", "role": "user", "text": "¿Qué dijo Lucía?", "complete": true},
            "answer": {"id": "turn-1:assistant", "role": "assistant", "text": "Que el presupuesto",
                "complete": false}
        })),
    ]);
    let question = partial.question.expect("question");
    assert_eq!(question.stage, MeetingQuestionStage::Responding);
    assert_eq!(question.answer.as_deref(), Some("Que el presupuesto"));

    let mut answered = asked();
    answered.follow(
        orbit_core::meeting_question::TurnFacts {
            in_flight: false,
            answer: Some("Que el presupuesto sube un 10 %."),
            failure: None,
        },
        START + 3_000,
    );
    let done = view(vec![
        recording_head(CapturePhase::Recording, Some(answered), None),
        assistant(json!({
            "user": {"id": "turn-1", "role": "user", "text": "¿Qué dijo Lucía?", "complete": true},
            "answer": {"id": "turn-1:assistant", "role": "assistant",
                "text": "Que el presupuesto sube un 10 %.", "complete": true}
        })),
    ]);
    let question = done.question.expect("question");
    assert_eq!(question.stage, MeetingQuestionStage::Answered);
    assert_eq!(
        question.answer.as_deref(),
        Some("Que el presupuesto sube un 10 %.")
    );
    assert_eq!(
        question.shown_until_ms,
        Some(START + 3_000 + orbit_core::meeting_question::ANSWER_SHOWN_MS)
    );
}

#[test]
fn a_failure_reaches_the_app_with_its_cause_and_numbers() {
    let refused = MeetingQuestion::refused(
        "turn-2".into(),
        Some("recording-1".into()),
        "a".into(),
        2,
        QuestionOrigin::Typed,
        QuestionCause::QuestionTooLong {
            bytes: 1_100,
            max_bytes: 1_024,
        },
        START,
    );
    let meeting = view(vec![recording_head(
        CapturePhase::Recording,
        Some(refused),
        None,
    )]);
    let question = meeting.question.expect("question");
    assert_eq!(question.stage, MeetingQuestionStage::Failed);
    assert_eq!(
        question.cause,
        Some(MeetingQuestionCause::QuestionTooLong {
            bytes: 1_100,
            max_bytes: 1_024
        })
    );
}

#[test]
fn the_button_offers_core_windows_only_while_recording() {
    let recording_now = view(vec![recording_head(CapturePhase::Recording, None, None)]);
    let ask = recording_now
        .ask
        .expect("«Preguntar a Deimos» while recording");
    assert_eq!(
        ask.windows
            .iter()
            .map(|window| window.minutes)
            .collect::<Vec<_>>(),
        WINDOW_MINUTES.to_vec()
    );
    assert_eq!(ask.default_minutes, DEFAULT_WINDOW_MINUTES);
    assert_eq!(ask.max_question_bytes as usize, MAX_QUESTION_BYTES);
    assert!(
        view(vec![recording_head(CapturePhase::Paused, None, None)])
            .ask
            .is_some()
    );
    for phase in [CapturePhase::Finalizing, CapturePhase::Idle] {
        assert!(view(vec![recording_head(phase, None, None)]).ask.is_none());
    }
}

#[test]
fn the_voice_status_reaches_the_app_and_its_absence_says_nothing() {
    let meeting = view(vec![recording_head(
        CapturePhase::Recording,
        None,
        Some(VoiceStatus::NotListening {
            why: NotListening::MacosVersion {
                os_version: Some("15.4".into()),
            },
        }),
    )]);
    assert_eq!(
        meeting.voice_trigger,
        Some(VoiceStatusSnapshot::NotListening {
            why: VoiceOffCause::MacosVersion {
                os_version: Some("15.4".into())
            }
        })
    );
    let downloading = view(vec![recording_head(
        CapturePhase::Recording,
        None,
        Some(VoiceStatus::Downloading { percent: Some(42) }),
    )]);
    assert_eq!(
        downloading.voice_trigger,
        Some(VoiceStatusSnapshot::Downloading { percent: Some(42) })
    );
    let unknown = view(vec![recording_head(CapturePhase::Recording, None, None)]);
    assert_eq!(unknown.voice_trigger, None);
}

/// «Oye Deimos» heard and its question abandoned reaches the panel with the
/// helper's cause, so its line can say why and what to do.
#[test]
fn an_abandoned_phrase_reaches_the_app_with_its_cause() {
    use orbit_app_core::PhraseAbandonedCause as App;
    use orbit_core::wire::native::WakePhraseAbandonmentCause as Core;
    for (core, app) in [
        (Core::PhraseRevised, App::PhraseRevised),
        (Core::QuestionWaitExpired, App::QuestionWaitExpired),
        (Core::QuestionTooLong, App::QuestionTooLong),
    ] {
        let meeting = view(vec![recording_head(
            CapturePhase::Recording,
            None,
            Some(VoiceStatus::Abandoned { cause: core }),
        )]);
        assert_eq!(
            meeting.voice_trigger,
            Some(VoiceStatusSnapshot::Abandoned { cause: app })
        );
    }
}

/// The app knows whether the question ever reached Deimos.
#[test]
fn the_app_knows_whether_the_question_reached_deimos() {
    let refused = MeetingQuestion::refused(
        "turn-3".into(),
        None,
        "¿Y?".into(),
        2,
        QuestionOrigin::Typed,
        QuestionCause::NoMeeting,
        START,
    );
    let meeting = view(vec![recording_head(
        CapturePhase::Recording,
        Some(refused),
        None,
    )]);
    assert!(!meeting.question.expect("question").sent);
    let sent = view(vec![recording_head(
        CapturePhase::Recording,
        Some(asked()),
        None,
    )]);
    assert!(sent.question.expect("question").sent);
}
