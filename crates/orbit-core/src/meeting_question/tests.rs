//! Questions during a meeting: admission and following the turn, from facts
//! passed in. No clock, no daemon. The turn's text is the hub's (the hub's
//! chat tests cover it).
use super::*;

const START: i64 = 1_800_000_000_000;
const RECORDING: &str = "6f2b7d5a-9c41-4b7e-8d3f-1a2b3c4d5e6f";

fn ready() -> AskFacts {
    AskFacts {
        privacy_stopped: false,
        connection: Connection::Online,
        busy: None,
        in_meeting: true,
    }
}

#[test]
fn a_question_with_a_meeting_on_and_nothing_in_progress_is_admitted() {
    assert_eq!(
        admit(&ready(), "¿Qué dijo Lucía del presupuesto?", 2),
        Ok(())
    );
    for minutes in WINDOW_MINUTES {
        assert_eq!(admit(&ready(), "¿Y ahora?", u16::from(minutes)), Ok(()));
    }
}

#[test]
fn the_user_stop_comes_first_whatever_else_is_wrong() {
    let facts = AskFacts {
        privacy_stopped: true,
        connection: Connection::Offline,
        busy: Some(BusyTurn::AssistantTurn),
        in_meeting: false,
    };
    assert_eq!(admit(&facts, "", 3), Err(QuestionCause::PrivacyStopped));
}

#[test]
fn each_refusal_names_its_cause_and_its_numbers() {
    assert_eq!(admit(&ready(), "   ", 2), Err(QuestionCause::QuestionEmpty));
    let long = "a".repeat(MAX_QUESTION_BYTES + 1);
    assert_eq!(
        admit(&ready(), &long, 2),
        Err(QuestionCause::QuestionTooLong {
            bytes: (MAX_QUESTION_BYTES + 1) as u64,
            max_bytes: MAX_QUESTION_BYTES as u64,
        })
    );
    assert_eq!(
        admit(&ready(), &"a".repeat(MAX_QUESTION_BYTES), 2),
        Ok(()),
        "the ceiling itself fits"
    );
    assert_eq!(
        admit(&ready(), "¿Y?", 3),
        Err(QuestionCause::WindowNotOffered {
            minutes: 3,
            offered: WINDOW_MINUTES.to_vec(),
        })
    );
    let no_meeting = AskFacts {
        in_meeting: false,
        ..ready()
    };
    assert_eq!(admit(&no_meeting, "¿Y?", 2), Err(QuestionCause::NoMeeting));
    for connection in [Connection::Offline, Connection::Connecting] {
        let offline = AskFacts {
            connection: connection.clone(),
            ..ready()
        };
        assert_eq!(
            admit(&offline, "¿Y?", 2),
            Err(QuestionCause::HubUnreachable { connection })
        );
    }
    let revoked = AskFacts {
        connection: Connection::Revoked,
        ..ready()
    };
    assert_eq!(admit(&revoked, "¿Y?", 2), Err(QuestionCause::Revoked));
    for turn in [
        BusyTurn::AssistantTurn,
        BusyTurn::PendingApproval,
        BusyTurn::VoiceNote,
        BusyTurn::VoiceTranscription,
        BusyTurn::Call,
    ] {
        let busy = AskFacts {
            busy: Some(turn),
            ..ready()
        };
        assert_eq!(
            admit(&busy, "¿Y?", 2),
            Err(QuestionCause::TurnInFlight { turn })
        );
    }
}

/// The pure half: the question «¿qué dijo Lucía del
/// presupuesto?» goes with the meeting and its last 2 minutes, and nothing
/// older.
fn asked() -> MeetingQuestion {
    MeetingQuestion::thinking(
        "request-1".into(),
        RECORDING.into(),
        "¿Qué dijo Lucía?".into(),
        2,
        QuestionOrigin::Voice,
        START,
    )
}

#[test]
fn the_question_waits_while_the_turn_is_in_flight_and_shows_its_answer() {
    let mut question = asked();
    assert_eq!(question.stage, QuestionStage::Thinking);
    assert_eq!(question.shown_until_ms(), None, "thinking has no deadline");
    let unchanged = question.follow(
        TurnFacts {
            in_flight: true,
            answer: Some("parcial"),
            failure: None,
        },
        START + 1_000,
    );
    assert!(!unchanged);
    assert_eq!(question.stage, QuestionStage::Thinking);
    assert!(question.follow(
        TurnFacts {
            in_flight: false,
            answer: Some("Dijo que sube un diez por ciento."),
            failure: None,
        },
        START + 4_000,
    ));
    assert_eq!(question.stage, QuestionStage::Answered);
    assert_eq!(question.finished_at_ms, Some(START + 4_000));
    assert_eq!(
        question.shown_until_ms(),
        Some(START + 4_000 + ANSWER_SHOWN_MS)
    );
    assert!(!question.expired(START + 4_000 + ANSWER_SHOWN_MS - 1));
    assert!(question.expired(START + 4_000 + ANSWER_SHOWN_MS));
    assert!(
        !question.follow(
            TurnFacts {
                in_flight: false,
                answer: None,
                failure: Some(&QuestionCause::Interrupted),
            },
            START + 5_000,
        ),
        "an answered question stays answered"
    );
}

#[test]
fn a_turn_that_ends_without_an_answer_keeps_the_cause_it_heard() {
    let mut question = asked();
    let cause = QuestionCause::DeimosUnavailable {
        code: "upstream_unavailable".into(),
    };
    assert!(question.follow(
        TurnFacts {
            in_flight: false,
            answer: None,
            failure: Some(&cause),
        },
        START + 2_000,
    ));
    assert_eq!(question.stage, QuestionStage::Failed);
    assert_eq!(question.cause, Some(cause));
    assert_eq!(
        question.shown_until_ms(),
        Some(START + 2_000 + FAILURE_SHOWN_MS)
    );

    let mut silent = asked();
    assert!(silent.follow(
        TurnFacts {
            in_flight: false,
            answer: Some("  "),
            failure: None,
        },
        START + 2_000,
    ));
    assert_eq!(silent.cause, Some(QuestionCause::NoAnswer));
}

#[test]
fn a_refused_question_is_shown_with_its_cause_for_eight_seconds() {
    let refused = MeetingQuestion::refused(
        "request-2".into(),
        None,
        "¿Y?".into(),
        2,
        QuestionOrigin::Typed,
        QuestionCause::NoMeeting,
        START,
    );
    assert_eq!(refused.stage, QuestionStage::Failed);
    assert_eq!(refused.shown_until_ms(), Some(START + FAILURE_SHOWN_MS));
}

#[test]
fn causes_travel_with_their_names_and_numbers() -> Result<(), serde_json::Error> {
    assert_eq!(
        serde_json::to_value(QuestionCause::TurnInFlight {
            turn: BusyTurn::VoiceTranscription
        })?,
        serde_json::json!({"cause": "turn_in_flight", "turn": "voice_transcription"})
    );
    assert_eq!(
        serde_json::to_value(QuestionCause::HubUnreachable {
            connection: Connection::Connecting
        })?,
        serde_json::json!({"cause": "hub_unreachable", "connection": "connecting"})
    );
    Ok(())
}

/// A refused question is stored at most at the ceiling, cut on a
/// character, and its cause keeps the size it arrived with.
#[test]
fn a_refused_question_is_kept_at_the_ceiling_with_its_original_size() {
    let long = "ñ".repeat(10_000);
    let cause = QuestionCause::QuestionTooLong {
        bytes: long.len() as u64,
        max_bytes: MAX_QUESTION_BYTES as u64,
    };
    let refused = MeetingQuestion::refused(
        "request-3".into(),
        Some(RECORDING.into()),
        long,
        2,
        QuestionOrigin::Typed,
        cause.clone(),
        START,
    );
    assert!(
        refused.question.len() <= MAX_QUESTION_BYTES,
        "{}",
        refused.question.len()
    );
    assert!(refused.question.chars().all(|character| character == 'ñ'));
    assert_eq!(refused.cause, Some(cause));
}

#[test]
fn a_refusal_never_reached_deimos() {
    let refused = MeetingQuestion::refused(
        "request-4".into(),
        None,
        "¿Y?".into(),
        2,
        QuestionOrigin::Voice,
        QuestionCause::PrivacyStopped,
        START,
    );
    assert!(!refused.sent);
    assert!(asked().sent);
}

fn facts(report: Option<&VoiceTrigger>) -> VoiceFacts<'_> {
    VoiceFacts {
        recording: Some(crate::CapturePhase::Recording),
        privacy_stopped: false,
        helper_connected: true,
        helper_listens: true,
        requested: true,
        report,
    }
}

/// During a recording the status always says whether «Oye Deimos»
/// listens and, when it does not, why; outside a recording it says nothing.
#[test]
fn the_voice_status_is_true_and_says_why_it_does_not_listen() {
    let listening = VoiceTrigger::Listening;
    let idle = VoiceTrigger::Idle;
    let downloading = VoiceTrigger::Downloading { percent: Some(42) };
    let old = VoiceTrigger::Unavailable {
        cause: "macos_version".into(),
        os_version: Some("15.4".into()),
    };
    let locale = VoiceTrigger::Unavailable {
        cause: "locale_unsupported".into(),
        os_version: None,
    };
    let failed = VoiceTrigger::Failed {
        cause: "assets_download_failed".into(),
        detail: Some("sin red".into()),
    };
    assert_eq!(
        voice_status(&facts(Some(&listening))),
        Some(VoiceStatus::Listening)
    );
    assert_eq!(
        voice_status(&facts(Some(&downloading))),
        Some(VoiceStatus::Downloading { percent: Some(42) })
    );
    for report in [None, Some(&idle)] {
        assert_eq!(
            voice_status(&facts(report)),
            Some(VoiceStatus::Starting),
            "{report:?}"
        );
    }
    let off = |facts: VoiceFacts<'_>| match voice_status(&facts) {
        Some(VoiceStatus::NotListening { why }) => Some(why),
        _ => None,
    };
    assert_eq!(
        off(facts(Some(&old))),
        Some(NotListening::MacosVersion {
            os_version: Some("15.4".into())
        })
    );
    assert_eq!(
        off(facts(Some(&locale))),
        Some(NotListening::LocaleUnsupported)
    );
    assert_eq!(
        off(facts(Some(&failed))),
        Some(NotListening::Failed {
            code: "assets_download_failed".into(),
            detail: Some("sin red".into()),
        })
    );
    assert_eq!(
        off(VoiceFacts {
            privacy_stopped: true,
            ..facts(Some(&listening))
        }),
        Some(NotListening::PrivacyStopped)
    );
    assert_eq!(
        off(VoiceFacts {
            recording: Some(crate::CapturePhase::Paused),
            requested: false,
            ..facts(Some(&idle))
        }),
        Some(NotListening::Paused)
    );
    assert_eq!(
        off(VoiceFacts {
            helper_connected: false,
            requested: false,
            ..facts(None)
        }),
        Some(NotListening::HelperDisconnected)
    );
    assert_eq!(
        off(VoiceFacts {
            helper_listens: false,
            requested: false,
            ..facts(None)
        }),
        Some(NotListening::HelperWithoutPhrase)
    );
    assert_eq!(
        off(VoiceFacts {
            requested: false,
            ..facts(Some(&idle))
        }),
        Some(NotListening::NoMicrophone)
    );
    for phase in [
        None,
        Some(crate::CapturePhase::Idle),
        Some(crate::CapturePhase::Finalizing),
        Some(crate::CapturePhase::Interrupted),
    ] {
        assert_eq!(
            voice_status(&VoiceFacts {
                recording: phase,
                ..facts(Some(&listening))
            }),
            None
        );
    }
}

/// The transcription can write one phrase twice with other marks.
#[test]
fn the_same_question_is_the_same_words_whatever_their_marks() {
    assert!(same_question(
        "¿De qué estamos hablando?",
        "de qué estamos hablando"
    ));
    assert!(same_question("  ¿Qué dijo Lucía? ", "¿qué dijo lucía?"));
    assert!(!same_question("¿Qué dijo Lucía?", "¿Qué dijo Pedro?"));
    assert!(
        !same_question("¿?", "¿?"),
        "no words is never the same question"
    );
}
