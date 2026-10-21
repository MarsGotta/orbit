//! The app does not call it recording until the daemon says so.
//!
//! `send(MeetingStart)` used to promote `Starting` to `Recording` the moment
//! the IPC answer came back, but that answer leaves the daemon while the
//! capture is still `Starting`: the helper has not acknowledged anything yet
//! (`start_track` only writes to the helper's channel and
//! `capture_started_if_ready` does nothing while starts are pending). Saying
//! "recording" there is a state in progress wearing the name of a fact, and it
//! is what made `daemon_client::a2…` read "recording" for the starting
//! snapshot under load.
//!
//! The channel holds a command reply to observe the phase before acknowledgement,
//! and checks the client clock at the outgoing command boundary.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use orbit_app_core::{AppCommand, DaemonClient, MeetingPhase};
use orbit_core::{
    PROTOCOL_VERSION,
    protocol::{Command, Response},
};
use orbit_ipc::{Paths, TopicChannels};
use std::{collections::BTreeMap, error::Error, sync::Arc, time::Duration};
use tokio::sync::mpsc;

#[path = "support/fake_daemon.rs"]
mod fake_daemon;

/// Bounds a wait so a hang fails with the condition's name; the fact itself is
/// awaited on the watch channel, never on a clock. The fake daemon is
/// in-process, so anything slower than this budget is a hang, not load.
const HANG_BUDGET: Duration = Duration::from_secs(5);

#[tokio::test]
async fn sending_either_start_command_does_not_write_a_phase_before_the_daemon_answers()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let (calls_tx, mut calls_rx) = mpsc::channel(8);
    let listener = fake_daemon::channel(calls_tx, TopicChannels::new());
    let client = DaemonClient::connect(paths, listener.client.clone());
    let commands = [
        orbit_app_core::features::recording::RecordingCommand::MeetingStart {
            title: "Primera".into(),
            minutes: 10,
        },
        orbit_app_core::features::recording::RecordingCommand::MeetingStartSelected {
            title: "Segunda".into(),
            minutes: 10,
            audio: None,
            event: None,
            event_id: None,
        },
    ];
    for command in commands {
        let sending = client.clone();
        let task = tokio::spawn(async move { sending.send(AppCommand::Recording(command)).await });
        let call = loop {
            let call = tokio::time::timeout(HANG_BUDGET, calls_rx.recv())
                .await?
                .ok_or("start command did not reach the fake daemon")?;
            if matches!(call.request.body, Command::MeetingStart { .. }) {
                break call;
            }
            let _ = call.reply.send(Response {
                v: PROTOCOL_VERSION,
                id: call.request.id,
                ok: true,
                status: None,
                error: None,
                refusal: None,
                epoch: Some("recording-phase".into()),
                revisions: Some(BTreeMap::new()),
            });
        };
        assert_eq!(client.composed_snapshot().meeting.phase, MeetingPhase::Idle);
        let _ = call.reply.send(Response {
            v: PROTOCOL_VERSION,
            id: call.request.id,
            ok: true,
            status: None,
            error: None,
            refusal: None,
            epoch: Some("recording-phase".into()),
            revisions: Some(BTreeMap::new()),
        });
        task.await??;
    }
    listener.abort();
    Ok(())
}

struct MadridClock;

impl orbit_app_core::calendar::CalendarClock for MadridClock {
    fn now_ms(&self) -> i64 {
        1_792_405_200_000 // 19 October 2026, 10:20 UTC.
    }
    fn local_zone(&self, _now_ms: i64) -> orbit_app_core::LocalZone {
        orbit_app_core::LocalZone::fixed(7_200_000)
    }
    fn timezone_name(&self) -> Result<String, String> {
        Ok("Europe/Madrid".into())
    }
}

#[tokio::test]
async fn automatic_recording_titles_reach_the_daemon_in_the_clients_local_zone()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let (calls_tx, mut calls_rx) = mpsc::channel(8);
    let listener = fake_daemon::channel(calls_tx, TopicChannels::new());
    let client =
        DaemonClient::connect_with_clock(paths, Arc::new(MadridClock), listener.client.clone());
    for (command, expected) in [
        (
            AppCommand::Recording(
                orbit_app_core::features::recording::RecordingCommand::MeetingStart {
                    title: String::new(),
                    minutes: 60,
                },
            ),
            "Reunión 12:20",
        ),
        (
            serde_json::from_value(
                serde_json::json!({"command": "meeting_start", "title": "", "minutes": 60, "audio": null}),
            )?,
            "Reunión 12:20",
        ),
        (
            serde_json::from_value(
                serde_json::json!({"command": "meeting_start", "title": "Actas", "minutes": 60}),
            )?,
            "Actas",
        ),
    ] {
        let sending = client.clone();
        let task = tokio::spawn(async move { sending.send(command).await });
        let call = loop {
            let call = tokio::time::timeout(HANG_BUDGET, calls_rx.recv())
                .await?
                .ok_or("recording start did not reach the fake daemon")?;
            if matches!(call.request.body, Command::MeetingStart { .. }) {
                break call;
            }
            let _ = call.reply.send(Response {
                v: PROTOCOL_VERSION,
                id: call.request.id,
                ok: true,
                status: None,
                error: None,
                refusal: None,
                epoch: Some("recording-phase".into()),
                revisions: Some(BTreeMap::new()),
            });
        };
        assert!(
            matches!(&call.request.body,
                Command::MeetingStart { title, duration_minutes: 60, event: None, .. } if title == expected
            ),
            "{:?}",
            call.request.body
        );
        let _ = call.reply.send(Response {
            v: PROTOCOL_VERSION,
            id: call.request.id,
            ok: true,
            status: None,
            error: None,
            refusal: None,
            epoch: Some("recording-phase".into()),
            revisions: Some(BTreeMap::new()),
        });
        task.await??;
    }
    listener.abort();
    Ok(())
}
