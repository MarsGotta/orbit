//! SIM-23 in the actor: the report path ends in the one error-report client,
//! the switch is a setting, and what the helper reports reaches the local
//! report log with its op.
#![allow(clippy::unwrap_used)]
use super::*;
use crate::error_reports::{ErrorReports, fake_sentry::FakeSentry};
use orbit_core::notice::{Notice, NoticeCode, Op, Report, ReportSource};

const DELIVERY: Duration = Duration::from_secs(10);

/// The report log's lines, as JSON.
fn lines(runtime: &Runtime) -> Vec<Value> {
    let path = runtime.report_log.as_ref().unwrap().path();
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn line<'a>(lines: &'a [Value], code: &str) -> &'a Value {
    lines
        .iter()
        .find(|line| line["code"] == code)
        .unwrap_or_else(|| panic!("no line {code} in {lines:#?}"))
}

/// Criterion 1, the daemon's half: a notice it says and an error the
/// interface reports through `Command::Report` leave through the same
/// client, each with its code and its op.
#[tokio::test]
async fn a_notice_and_an_interface_report_leave_through_the_one_client() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, _helper) = super::audio::tests::fixture(directory.path()).unwrap();
    let sentry = FakeSentry::start().unwrap();
    runtime.error_reports = ErrorReports::start(Some(&sentry.dsn()), true);

    runtime.begin_turn(
        Op::parse("op-said-by-daemon").unwrap(),
        ReportSource::Daemon,
    );
    runtime.say(Notice::new(NoticeCode::StoreProblem));
    let interface: Report = serde_json::from_value(json!({
        "op": "op-said-by-interface", "code": "ui_uncaught_error",
        "class": "failed", "source": "ui"
    }))
    .unwrap();
    runtime
        .command(Command::Report { report: interface })
        .await
        .unwrap();
    assert!(runtime.error_reports.flush(DELIVERY));

    let events = sentry.events();
    let sent: Vec<(&str, &str)> = events
        .iter()
        .map(|event| {
            (
                event["tags"]["code"].as_str().unwrap(),
                event["tags"]["op"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        sent,
        [
            ("store_problem", "op-said-by-daemon"),
            ("ui_uncaught_error", "op-said-by-interface")
        ]
    );
}

/// The switch is a daemon setting: saved, and in force at once; a build
/// without an address does not offer it (the status topic says so, see
/// `topics/status.rs`).
#[tokio::test]
async fn the_switch_is_saved_in_force_and_shown_only_when_offered() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, _helper) = super::audio::tests::fixture(directory.path()).unwrap();
    let offered = |runtime: &Runtime| runtime.error_reports.choice();
    assert_eq!(offered(&runtime), None);

    let sentry = FakeSentry::start().unwrap();
    runtime.error_reports = ErrorReports::start(Some(&sentry.dsn()), false);
    assert_eq!(offered(&runtime), Some(false));
    runtime
        .command(Command::ErrorReports { enabled: true })
        .await
        .unwrap();
    assert_eq!(offered(&runtime), Some(true));
    assert!(runtime.paths.load_settings().unwrap().error_reports);
    runtime.say(NoticeCode::StoreProblem);
    assert!(runtime.error_reports.flush(DELIVERY));
    assert_eq!(sentry.events().len(), 1);

    runtime
        .command(Command::ErrorReports { enabled: false })
        .await
        .unwrap();
    assert!(!runtime.paths.load_settings().unwrap().error_reports);
    runtime.say(NoticeCode::RecordingSealFailed);
    assert!(runtime.error_reports.flush(DELIVERY));
    assert_eq!(sentry.events().len(), 1);
}

/// Criterion 4: every failed order and every step of «Oye Deimos» the
/// helper reports reaches the local report log under the op of its turn,
/// without becoming a notice on screen.
#[tokio::test]
async fn what_the_helper_reports_reaches_the_report_log_with_its_op() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, mut helper) = super::audio::tests::fixture(directory.path()).unwrap();
    runtime.report_log = Some(orbit_ipc::report::ReportLog::daemon(directory.path()));
    let shown_before = runtime.state.durable.notice.clone();

    runtime.begin_turn(Op::parse("op-order-0001").unwrap(), ReportSource::Daemon);
    runtime
        .native_send("screen_sources", json!({}))
        .await
        .unwrap();
    let order = helper.try_recv().unwrap();
    runtime.begin_turn(Op::parse("op-answer-0001").unwrap(), ReportSource::Daemon);
    runtime
        .native_event(json!({"v":1,"event":"response","id":order["id"],"ok":false,
            "error":{"code":"screen_permission_required","message":"Texto del helper"}}))
        .await
        .unwrap();

    runtime.begin_turn(Op::parse("op-trigger-0001").unwrap(), ReportSource::Daemon);
    for event in [
        json!({"v":1,"event":"voice_trigger","state":"downloading","percent":10}),
        json!({"v":1,"event":"voice_trigger","state":"downloading","percent":60}),
        json!({"v":1,"event":"voice_trigger","state":"listening"}),
        json!({"v":1,"event":"notice","code":"wake_phrase_heard"}),
        json!({"v":1,"event":"notice","code":"wake_phrase_abandoned"}),
        json!({"v":1,"event":"notice","code":"wake_phrase_question_dropped"}),
        json!({"v":1,"event":"voice_trigger","state":"failed","cause":"analyzer_failed",
            "detail":"Apple dijo algo largo"}),
        json!({"v":1,"event":"voice_trigger","state":"unavailable","cause":"locale_unsupported"}),
        json!({"v":1,"event":"voice_trigger","state":"idle"}),
        json!({"v":1,"event":"assistant_question","stream_id":"stream-x",
            "text":"¿Qué dijo Lucía?","started_ms":1,"ended_ms":2}),
    ] {
        runtime.native_event(event).await.unwrap();
    }
    runtime.begin_turn(Op::parse("op-unknown-0001").unwrap(), ReportSource::Daemon);
    runtime
        .native_event(json!({"v":1,"event":"notice","code":"code_from_a_newer_helper"}))
        .await
        .unwrap();
    runtime.begin_fresh_turn(ReportSource::Daemon);

    let lines = lines(&runtime);
    let failed = line(&lines, "helper_command_failed");
    assert_eq!(failed["op"], "op-answer-0001");
    assert_eq!(failed["class"], "silent");
    assert_eq!(failed["data"]["step"], "screen_sources");
    assert_eq!(failed["data"]["code"], "screen_permission_required");
    for code in [
        "wake_phrase_downloading",
        "wake_phrase_listening",
        "wake_phrase_heard",
        "wake_phrase_abandoned",
        "wake_phrase_question_dropped",
        "wake_phrase_failed",
        "wake_phrase_unavailable",
        "wake_phrase_off",
        "wake_phrase_question_ignored",
    ] {
        let said = line(&lines, code);
        assert_eq!(said["op"], "op-trigger-0001", "{code}");
        assert_eq!(said["class"], "silent", "{code}");
    }
    // One step per state: the download's percent records nothing new.
    let downloads = lines
        .iter()
        .filter(|line| line["code"] == "wake_phrase_downloading")
        .count();
    assert_eq!(downloads, 1);
    assert_eq!(
        line(&lines, "wake_phrase_failed")["data"]["cause"],
        "analyzer_failed"
    );
    assert_eq!(
        line(&lines, "wake_phrase_unavailable")["data"]["cause"],
        "locale_unsupported"
    );
    assert_eq!(
        line(&lines, "wake_phrase_question_ignored")["data"]["cause"],
        "other_stream"
    );
    let unknown = line(&lines, "helper_error");
    assert_eq!(unknown["op"], "op-unknown-0001");
    assert_eq!(unknown["data"]["code"], "code_from_a_newer_helper");
    let log = std::fs::read_to_string(runtime.report_log.as_ref().unwrap().path()).unwrap();
    for content in ["Apple dijo", "Lucía", "Texto del helper"] {
        assert!(!log.contains(content), "{content} in {log}");
    }
    // Only the helper's unknown code is said on screen; the rest is recorded.
    assert_ne!(runtime.state.durable.notice, shown_before);
    assert_eq!(
        runtime
            .state
            .durable
            .notice
            .as_ref()
            .map(|notice| notice.code),
        Some(NoticeCode::HelperError)
    );
    assert!(
        !super::turn::said::has(&runtime.state, NoticeCode::WakePhraseHeard),
        "a recorded step is not a notice on screen"
    );
}

/// 2026-10-17: a call's microphone did not open and the report said only
/// «native_operation_failed». The helper now sends the macOS error under it,
/// domain and number; the report keeps both, and never the helper's text.
#[tokio::test]
async fn a_failed_order_keeps_the_macos_error_domain_and_number() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, mut helper) = super::audio::tests::fixture(directory.path()).unwrap();
    runtime.report_log = Some(orbit_ipc::report::ReportLog::daemon(directory.path()));
    runtime.begin_turn(Op::parse("op-order-0002").unwrap(), ReportSource::Daemon);
    runtime
        .native_send("screen_sources", json!({}))
        .await
        .unwrap();
    let order = helper.try_recv().unwrap();
    runtime.begin_turn(Op::parse("op-answer-0002").unwrap(), ReportSource::Daemon);
    runtime
        .native_event(json!({"v":1,"event":"response","id":order["id"],"ok":false,
            "error":{"code":"native_operation_failed","message":"Texto del helper",
                "detail":{"domain":"com.apple.coreaudio.avfaudio","os_status":-10875}}}))
        .await
        .unwrap();
    runtime.begin_fresh_turn(ReportSource::Daemon);
    let lines = lines(&runtime);
    let failed = line(&lines, "helper_command_failed");
    assert_eq!(failed["data"]["code"], "native_operation_failed");
    assert_eq!(failed["data"]["cause"], "com.apple.coreaudio.avfaudio");
    assert_eq!(failed["data"]["status"], -10875);
    assert!(
        !lines
            .iter()
            .any(|line| line.to_string().contains("Texto del helper"))
    );
}

#[tokio::test]
async fn abandoned_wake_questions_keep_each_cause_and_audio_times_in_the_report() {
    let mut observed = Vec::new();
    let mut expected = Vec::new();
    for (cause, elapsed_ms, silent_ms) in [
        ("phrase_revised", 1800, 0),
        ("question_wait_expired", 7000, 6000),
        ("question_too_long", 45000, 0),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let (mut runtime, _helper) = super::audio::tests::fixture(directory.path()).unwrap();
        runtime.report_log = Some(orbit_ipc::report::ReportLog::daemon(directory.path()));
        runtime.begin_turn(
            Op::parse("op-abandoned-0001").unwrap(),
            ReportSource::Daemon,
        );
        runtime
            .native_event(
                json!({"v":1,"event":"notice","code":"wake_phrase_abandoned",
            "abandonment":{"cause":cause,"elapsed_ms":elapsed_ms,"silent_ms":silent_ms},
            "text":"palabras privadas que no deben viajar"}),
            )
            .await
            .unwrap();
        let lines = lines(&runtime);
        let report = line(&lines, "wake_phrase_abandoned");
        observed.push(report["data"].clone());
        expected.push(json!({"cause":cause,"elapsed_ms":elapsed_ms,"silent_ms":silent_ms}));
        assert_eq!(report["op"], "op-abandoned-0001");
        assert_eq!(report["class"], "silent");
        assert!(runtime.state.durable.notice.is_none());
        assert!(!report.to_string().contains("privadas"));
    }
    assert_eq!(observed, expected);
}
