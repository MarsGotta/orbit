use super::fake_sentry::FakeSentry;
use super::*;
use orbit_core::notice::{Notice, NoticeCode, NoticeKey, Op, ReportSource};

/// Bounds a hang of the loopback, not a delivery budget: `flush` returns as
/// soon as the queue is empty.
const DELIVERY: Duration = Duration::from_secs(10);

fn timer_failure(op: &str) -> Report {
    Report::of_notice(
        &Notice::new(NoticeCode::CallDetectionFailed)
            .with(NoticeKey::Count, 1_u64)
            .with_token(NoticeKey::Cause, "timer_failed"),
        Op::parse(op).unwrap(),
        ReportSource::Daemon,
    )
}

/// Criterion 3: a code repeated every 100 ms for 5 s leaves once; the
/// repetitions are counted and leave with the next send, once its minute
/// ended.
#[test]
fn a_code_repeated_every_100_ms_for_5_s_leaves_once_with_its_count() {
    let sentry = FakeSentry::start().unwrap();
    let reports = ErrorReports::start(Some(&sentry.dsn()), true);
    let start = Instant::now();
    for tick in 0..50_u64 {
        let now = start + Duration::from_millis(100 * tick);
        reports.report(&timer_failure(&format!("op-repeat-{tick:04}")), now);
        reports.send_due(now);
    }
    assert!(reports.flush(DELIVERY));
    let events = sentry.events();
    assert_eq!(events.len(), 1, "{events:#?}");
    assert_eq!(events[0]["tags"]["op"], "op-repeat-0000");
    assert!(events[0]["extra"].get("repeated").is_none());

    reports.send_due(start + REPEAT_WINDOW + Duration::from_millis(100));
    assert!(reports.flush(DELIVERY));
    let events = sentry.events();
    assert_eq!(events.len(), 2, "{events:#?}");
    assert_eq!(events[1]["message"], "call_detection_failed");
    assert_eq!(events[1]["extra"]["repeated"], 49);
    // The count leaves with the op of the last repetition.
    assert_eq!(events[1]["tags"]["op"], "op-repeat-0049");
}

#[test]
fn the_same_code_with_other_data_is_another_report() {
    let sentry = FakeSentry::start().unwrap();
    let reports = ErrorReports::start(Some(&sentry.dsn()), true);
    let now = Instant::now();
    let other = Report::of_notice(
        &Notice::new(NoticeCode::CallDetectionFailed).with(NoticeKey::Count, 2_u64),
        Op::parse("op-other-data").unwrap(),
        ReportSource::Daemon,
    );
    reports.report(&timer_failure("op-first-data"), now);
    reports.report(&other, now);
    assert!(reports.flush(DELIVERY));
    assert_eq!(sentry.events().len(), 2, "{:?}", sentry.requests());
}

/// Only failures leave: the silent trail and a condition she can fix stay
/// in the local log; a failed code and a silent failure leave as errors.
#[test]
fn only_failures_leave_and_as_errors() {
    let sentry = FakeSentry::start().unwrap();
    let reports = ErrorReports::start(Some(&sentry.dsn()), true);
    let now = Instant::now();
    let line = |code: NoticeCode, op: &str| {
        Report::of_notice(
            &Notice::new(code),
            Op::parse(op).unwrap(),
            ReportSource::Daemon,
        )
    };
    reports.report(&line(NoticeCode::WakePhraseListening, "op-trail-0001"), now);
    reports.report(&line(NoticeCode::Paired, "op-trail-0002"), now);
    reports.report(
        &line(NoticeCode::MicrophonePermissionMissing, "op-fixable-01"),
        now,
    );
    reports.report(&line(NoticeCode::StoreProblem, "op-failed-001"), now);
    reports.report(&line(NoticeCode::WakePhraseFailed, "op-silentfail"), now);
    reports.report(
        &line(NoticeCode::TranscriptReadFailed, "op-sim24-fail"),
        now,
    );
    assert!(reports.flush(DELIVERY));
    let events = sentry.events();
    let sent: Vec<(&str, &str)> = events
        .iter()
        .map(|event| {
            (
                event["tags"]["code"].as_str().unwrap(),
                event["level"].as_str().unwrap_or("error"),
            )
        })
        .collect();
    assert_eq!(
        sent,
        [
            ("store_problem", "error"),
            ("wake_phrase_failed", "error"),
            ("transcript_read_failed", "error")
        ],
        "{events:#?}"
    );
}

/// Criterion 2: with the switch off nothing leaves, not even when a count
/// was pending from before she switched it off; without an address there
/// is no client at all.
#[test]
fn with_the_switch_off_or_without_an_address_nothing_leaves() {
    let sentry = FakeSentry::start().unwrap();
    let off = ErrorReports::start(Some(&sentry.dsn()), false);
    assert_eq!(off.choice(), Some(false));
    let start = Instant::now();
    off.report(&timer_failure("op-while-off"), start);
    off.send_due(start + REPEAT_WINDOW * 2);
    assert!(off.flush(DELIVERY));
    assert!(sentry.requests().is_empty(), "{:?}", sentry.requests());

    let switched = ErrorReports::start(Some(&sentry.dsn()), true);
    switched.report(&timer_failure("op-allowed-0"), start);
    switched.report(
        &timer_failure("op-allowed-1"),
        start + Duration::from_secs(1),
    );
    assert!(switched.flush(DELIVERY));
    assert_eq!(sentry.events().len(), 1);
    switched.set_enabled(false);
    assert_eq!(switched.choice(), Some(false));
    switched.report(&timer_failure("op-after-off"), start + REPEAT_WINDOW * 2);
    switched.send_due(start + REPEAT_WINDOW * 2);
    assert!(switched.flush(DELIVERY));
    assert_eq!(sentry.events().len(), 1, "{:#?}", sentry.events());

    let unaddressed = ErrorReports::start(None, true);
    assert_eq!(unaddressed.choice(), None);
    unaddressed.report(&timer_failure("op-no-address"), start);
    let invalid = ErrorReports::start(Some("no es una dirección"), true);
    assert_eq!(invalid.choice(), None);
}

/// The allow-list on its own: an event carrying everything the SDK or a
/// caller could put in it comes out with the code, the four tags, numbers,
/// booleans, closed tokens, the release, macOS's name and version and the
/// frames' file, function and line, and nothing else.
#[test]
fn the_scrub_rebuilds_the_event_from_its_allow_list() {
    const MARKER: &str = "marcadorprivadosim23";
    let mut event = Event {
        message: Some(MARKER.into()),
        server_name: Some(MARKER.into()),
        environment: Some(MARKER.into()),
        release: Some("orbit@0.4.0".into()),
        culprit: Some(MARKER.into()),
        transaction: Some(MARKER.into()),
        user: Some(sentry::User {
            ip_address: Some(sentry::protocol::IpAddress::Auto),
            username: Some(MARKER.into()),
            ..Default::default()
        }),
        ..Event::default()
    };
    for (key, value) in [
        ("op", "0123abcd-op"),
        ("code", "capture_cut"),
        ("class", "failed"),
        ("source", "daemon"),
        ("hub", MARKER),
    ] {
        event.tags.insert(key.into(), value.into());
    }
    for (key, value) in [
        ("fragments", Value::from(3)),
        ("closing", Value::from(false)),
        ("cause", Value::from("helper_lost")),
        ("stage", Value::from(MARKER)),
        ("code", Value::from(format!("{MARKER} con espacios"))),
        ("transcript", Value::from(MARKER)),
        ("pages", Value::from(vec![MARKER])),
    ] {
        event.extra.insert(key.into(), value);
    }
    event.contexts.insert(
        "os".into(),
        Context::Os(Box::new(OsContext {
            name: Some("macOS".into()),
            version: Some("26.0.1".into()),
            kernel_version: Some(MARKER.into()),
            build: Some(MARKER.into()),
            ..OsContext::default()
        })),
    );
    event
        .contexts
        .insert("device".into(), Context::Other(Default::default()));
    event.exception = vec![Exception {
        ty: "panic".into(),
        value: Some(MARKER.into()),
        stacktrace: Some(Stacktrace {
            frames: vec![Frame {
                function: Some("orbit_daemon::daemon::tick".into()),
                filename: Some("turn.rs".into()),
                abs_path: Some(format!("/Users/{MARKER}/orbit/turn.rs")),
                lineno: Some(42),
                context_line: Some(MARKER.into()),
                vars: [(String::from("text"), Value::from(MARKER))]
                    .into_iter()
                    .collect(),
                ..Frame::default()
            }],
            ..Stacktrace::default()
        }),
        ..Exception::default()
    }]
    .into();

    let clean = scrub(event).unwrap();
    let sent = serde_json::to_string(&clean).unwrap();
    assert!(!sent.contains(MARKER), "{sent}");
    assert_eq!(clean.message.as_deref(), Some("capture_cut"));
    let mut tags: Vec<&str> = clean.tags.keys().map(String::as_str).collect();
    tags.sort_unstable();
    assert_eq!(tags, ["class", "code", "op", "source"]);
    let mut extra: Vec<&str> = clean.extra.keys().map(String::as_str).collect();
    extra.sort_unstable();
    assert_eq!(extra, ["cause", "closing", "fragments"]);
    assert_eq!(clean.release.as_deref(), Some("orbit@0.4.0"));
    assert!(sent.contains("\"version\":\"26.0.1\""), "{sent}");
    assert!(sent.contains("\"lineno\":42"), "{sent}");
    assert!(sent.contains("turn.rs"), "{sent}");
    assert!(clean.server_name.is_none() && clean.user.is_none());
}

#[test]
fn an_event_that_was_not_made_here_is_not_sent() {
    assert!(scrub(Event::default()).is_none());
}

/// A panic of a release build has no names: the binary is stripped. Since
/// SIM-51 that binary is the app, `Contents/MacOS/Orbit`, with the daemon
/// inside; the image keeps its file name, whatever it is. Sentry
/// names its frames from the build's dSYM, uploaded at release, by the
/// frames' addresses and the image's identifier and load address. Those are
/// kept; the image's paths, which carry the account's name, never.
#[test]
fn a_panic_keeps_the_addresses_and_the_image_sentry_symbolicates_with() {
    use sentry::protocol::{DebugImage, DebugMeta, SymbolicDebugImage};
    const MARKER: &str = "marcadorprivadosim54";
    let mut event = Event {
        release: Some("orbit@0.5.1".into()),
        ..Event::default()
    };
    event.tags.insert("code".into(), "panic".into());
    event.exception = vec![Exception {
        ty: "panic".into(),
        stacktrace: Some(Stacktrace {
            frames: vec![Frame {
                instruction_addr: Some(0x1_0000_4f20_u64.into()),
                ..Frame::default()
            }],
            ..Stacktrace::default()
        }),
        ..Exception::default()
    }]
    .into();
    let id: sentry::types::DebugId = "4c4c4459-5555-3144-a1a1-a1a1a1a1a1a1".parse().unwrap();
    event.debug_meta = std::borrow::Cow::Owned(DebugMeta {
        sdk_info: None,
        images: vec![DebugImage::Symbolic(SymbolicDebugImage {
            name: format!("/Users/{MARKER}/Applications/Orbit.app/Contents/MacOS/Orbit"),
            arch: Some("arm64".into()),
            image_addr: 0x1_0000_0000_u64.into(),
            image_size: 0x80_0000,
            image_vmaddr: 0x1_0000_0000_u64.into(),
            id,
            code_id: None,
            debug_file: Some(format!("/Users/{MARKER}/orbit-desktop.dSYM")),
        })],
    });

    let clean = scrub(event).unwrap();
    let sent = serde_json::to_string(&clean).unwrap();
    assert!(!sent.contains(MARKER), "{sent}");
    assert!(
        sent.contains("\"instruction_addr\":\"0x100004f20\""),
        "{sent}"
    );
    let images = &clean.debug_meta.images;
    assert_eq!(images.len(), 1);
    let DebugImage::Symbolic(image) = &images[0] else {
        panic!("{images:?}");
    };
    assert_eq!(image.id, id);
    assert_eq!(image.name, "Orbit");
    assert_eq!(u64::from(image.image_addr), 0x1_0000_0000);
    assert!(image.debug_file.is_none());
}
