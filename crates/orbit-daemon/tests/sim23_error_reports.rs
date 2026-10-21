//! SIM-23 criterion 1: what leaves for Sentry carries codes and ops, never
//! content. The real Rust SDK sends to a fake Sentry on 127.0.0.1; nothing
//! here ever reaches sentry.io.
//!
//! One test in its own binary: the panic hook is process-wide, and this
//! process is the only one that installs it.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "../src/error_reports/fake_sentry.rs"]
mod fake_sentry;

use orbit_daemon::error_reports::ErrorReports;
use std::time::Duration;

/// A token-shaped marker, so it can ride anywhere a token can.
const MARKER: &str = "marcadorprivadosim23";

/// Bounds a hang of the loopback, not a delivery budget.
const DELIVERY: Duration = Duration::from_secs(10);

fn machine_name() -> Option<String> {
    let output = std::process::Command::new("hostname").output().ok()?;
    let name = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!name.is_empty()).then_some(name)
}

#[test]
fn a_panic_leaves_through_its_hook_with_frames_and_never_content() {
    let sentry = fake_sentry::FakeSentry::start().unwrap();
    let reports = ErrorReports::start(Some(&sentry.dsn()), true);
    reports.report_panics();

    // A panic whose text and thread are the marker.
    let panicked = std::thread::Builder::new()
        .name(MARKER.into())
        .spawn(|| panic!("{MARKER}: Alina dijo algo privado"))
        .unwrap()
        .join();
    assert!(panicked.is_err());
    assert!(reports.flush(DELIVERY));

    let sent = sentry.requests().join("\n");
    assert!(!sent.contains(MARKER), "the marker left:\n{sent}");
    if let Some(name) = machine_name() {
        assert!(
            !sent.contains(&name),
            "the machine's name «{name}» left:\n{sent}"
        );
    }

    let events = sentry.events();
    let codes: Vec<&str> = events
        .iter()
        .map(|event| event["message"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(codes, ["panic"], "{events:#?}");
    for event in &events {
        assert!(event["release"].as_str().unwrap().starts_with("orbit@"));
        assert!(event.get("server_name").is_none(), "{event:#}");
    }
    let frames = events[0]["exception"]["values"][0]["stacktrace"]["frames"]
        .as_array()
        .unwrap();
    assert!(
        frames
            .iter()
            .any(|frame| frame["filename"] == "sim23_error_reports.rs"
                && frame["lineno"].as_u64().is_some()
                && frame["function"].as_str().is_some()),
        "{frames:#?}"
    );
}
