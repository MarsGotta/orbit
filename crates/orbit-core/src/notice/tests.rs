#![allow(clippy::unwrap_used)]
use super::*;
use serde::Deserialize;

#[test]
fn a_log_scopes_each_notice_once_and_numbers_identical_notices_separately() {
    let first = Op::parse("first-producing-turn").unwrap();
    let second = Op::parse("next-producing-turn").unwrap();
    let mut log = NoticeLog::default();
    log.begin_turn(first.clone());
    let saved = log.record(Notice::new(NoticeCode::AssistantThinking));
    assert_eq!(saved.op, Some(first));
    assert_eq!(saved.sequence, Some(1));
    let before = log.clone();
    log.begin_turn(second.clone());
    assert_eq!(log, before, "a context change is not a published change");
    let next = log.record(Notice::new(NoticeCode::AssistantThinking));
    assert_eq!(next.op, Some(second));
    assert_eq!(next.sequence, Some(2));
    assert_eq!(log.records()[0], saved);
}

#[test]
fn a_restored_log_keeps_its_sequence_after_eviction_and_leaves_legacy_ops_unknown() {
    let mut log = NoticeLog::default();
    for _ in 0..CAPACITY + 3 {
        log.record(Notice::new(NoticeCode::AudioGap));
    }
    let saved = serde_json::to_string(&log).unwrap();
    let mut restored: NoticeLog = serde_json::from_str(&saved).unwrap();
    restored.begin_turn(Op::parse("restored-producing-turn").unwrap());
    assert!(restored.records().iter().all(|notice| notice.op.is_none()));
    let next = restored.record(Notice::new(NoticeCode::AssistantThinking));
    assert_eq!(next.sequence, Some((CAPACITY + 4) as u64));
    let legacy: NoticeLog =
        serde_json::from_str(r#"{"records":[{"code":"audio_gap"},{"code":"voice_note_saved"}]}"#)
            .unwrap();
    assert_eq!(
        legacy
            .records()
            .iter()
            .map(|notice| notice.sequence)
            .collect::<Vec<_>>(),
        vec![None, None]
    );
}

#[test]
fn an_op_is_eight_to_sixty_four_ascii_letters_digits_or_hyphens() {
    assert!(Op::parse("0123abcd").is_some());
    assert!(Op::parse("550e8400-e29b-41d4-a716-446655440000").is_some());
    assert!(Op::parse("short").is_none());
    assert!(Op::parse(&"a".repeat(65)).is_none());
    assert!(Op::parse("con espacio").is_none());
    assert!(Op::parse("ñandú-123").is_none());
    assert_eq!(
        Op::parse("7f3a2c9e-0000").map(|op| op.short()),
        Some("7F3A2C".to_owned())
    );
}

#[test]
fn notice_data_refuses_free_text_on_the_wire() {
    let text = r#"{"code":"capture_cut","data":{"cause":"Alguien dijo algo privado"}}"#;
    assert!(serde_json::from_str::<Notice>(text).is_err());
    let unknown_key = r#"{"code":"capture_cut","data":{"transcript":"x"}}"#;
    assert!(serde_json::from_str::<Notice>(unknown_key).is_err());
    let report = r#"{"op":"0123abcd","code":"ui_render_failed","class":"failed","source":"ui","data":{"cause":"Hola Lucía"}}"#;
    assert!(serde_json::from_str::<Report>(report).is_err());
    let fine = r#"{"code":"capture_cut","op":"0123abcd","data":{"cause":"helper_lost","fragments":3,"closing":false}}"#;
    let notice: Notice = serde_json::from_str(fine).unwrap();
    assert_eq!(notice.code, NoticeCode::CaptureCut);
    assert_eq!(
        notice.data.get(&NoticeKey::Fragments),
        Some(&DataValue::Int(3))
    );
    assert_eq!(
        notice.data.get(&NoticeKey::Closing),
        Some(&DataValue::Bool(false))
    );
}

#[test]
fn a_value_that_is_not_a_token_is_kept_only_as_unrecognized() {
    let notice =
        Notice::new(NoticeCode::CaptureRefused).with_token(NoticeKey::Cause, "Hola, Alina");
    assert_eq!(
        notice.data.get(&NoticeKey::Cause),
        Some(&DataValue::Token(Token::unrecognized()))
    );
}

#[test]
fn notices_survive_in_the_order_they_arrived_up_to_capacity() {
    let mut log = NoticeLog::default();
    for index in 0..(CAPACITY + 4) {
        log.record(Notice::new(NoticeCode::AudioGap).with(NoticeKey::Count, index));
    }
    assert_eq!(log.records().len(), CAPACITY);
    assert_eq!(
        log.records()
            .first()
            .and_then(|notice| notice.data.get(&NoticeKey::Count)),
        Some(&DataValue::from(4_usize))
    );
    assert_eq!(
        log.latest()
            .and_then(|notice| notice.data.get(&NoticeKey::Count)),
        Some(&DataValue::from(CAPACITY + 3))
    );
}

#[test]
fn a_saved_state_from_before_typed_notices_still_loads() {
    let old = r#"{"records":[{"text":"Grabación interrumpida"},{"code":"audio_gap"}]}"#;
    let log: NoticeLog = serde_json::from_str(old).unwrap();
    assert_eq!(log.records().len(), 1);
    assert_eq!(
        log.latest().map(|notice| notice.code),
        Some(NoticeCode::AudioGap)
    );
    #[derive(Deserialize)]
    struct Saved {
        #[serde(deserialize_with = "lenient_notice")]
        notice: Option<Notice>,
    }
    let saved: Saved =
        serde_json::from_str(r#"{"notice":"Tu espacio, bajo tu control."}"#).unwrap();
    assert_eq!(saved.notice, None);
}

#[test]
fn a_saved_notice_with_an_unknown_code_keeps_its_op() {
    #[derive(Deserialize)]
    struct Topic {
        #[serde(deserialize_with = "lenient_notice")]
        notice: Option<Notice>,
    }
    let newer: Topic = serde_json::from_str(
        r#"{"notice":{"code":"code_from_the_future","op":"7f3a2c9e","data":{"new_key":1}}}"#,
    )
    .unwrap();
    let notice = newer.notice.unwrap();
    assert_eq!(notice.code, NoticeCode::Unknown);
    assert_eq!(notice.class(), NoticeClass::Failed);
    assert_eq!(notice.op.map(|op| op.short()), Some("7F3A2C".to_owned()));
}

/// Every `failed` code is a failure; nothing the user can fix,
/// nothing in progress and nothing done is; the silent trail is not,
/// except the silent codes that are failures.
#[test]
fn failures_are_the_failed_codes_and_the_silent_failures() {
    for code in NoticeCode::ALL {
        match code.class() {
            NoticeClass::Failed => assert!(code.is_failure(), "{code:?}"),
            NoticeClass::Fixable | NoticeClass::Working | NoticeClass::Done => {
                assert!(!code.is_failure(), "{code:?}")
            }
            NoticeClass::Silent => {}
        }
    }
    assert!(NoticeCode::HelperCommandFailed.is_failure());
    assert!(NoticeCode::ConnectFailed.is_failure());
    assert!(NoticeCode::CalendarWriteFailed.is_failure());
    assert!(!NoticeCode::WakePhraseListening.is_failure());
    assert!(!NoticeCode::Paired.is_failure());
    let line = |code: &str, class: &str| -> Report {
        serde_json::from_value(serde_json::json!({
            "op": "0123abcd", "code": code, "class": class, "source": "ui"
        }))
        .unwrap()
    };
    assert!(line("ui_render_failed", "failed").is_failure());
    assert!(line("wake_phrase_failed", "silent").is_failure());
    assert!(line("note_read_problem", "silent").is_failure());
    assert!(!line("wake_phrase_listening", "silent").is_failure());
    assert!(!line("microphone_permission_missing", "fixable").is_failure());
}

#[test]
fn every_code_serializes_as_a_report_token() {
    for code in NoticeCode::ALL {
        let report = Report::of_notice(
            &Notice::new(*code),
            Op::parse("0123abcd").unwrap(),
            ReportSource::Daemon,
        );
        assert_ne!(report.code, Token::unrecognized(), "{code:?}");
    }
}

/// Places are the codes' order on the wire: no gap, so the next code's place
/// is the number of known codes; `unknown` stays last whatever comes after.
#[test]
fn every_place_is_taken_once_from_zero_and_unknown_is_last() {
    let (unknown, known) = NoticeCode::ALL.split_last().unwrap();
    assert_eq!(*unknown, NoticeCode::Unknown);
    for (place, code) in known.iter().enumerate() {
        assert_eq!(*code as usize, place, "{code:?}");
    }
}
