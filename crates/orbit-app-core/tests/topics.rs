//! The real daemon publishes watches inside this process, over isolated fixture data.
#![allow(clippy::unwrap_used)]
use orbit_app_core::{
    AppCommand, DaemonClient, DaemonConnection, MeetingApplicationsState, ReceiptDelivery,
    features::FeatureFrame,
};
use orbit_core::protocol::Command as OrbitCommand;

use embedded_daemon::request;
use orbit_ipc::Paths;
use std::{error::Error, time::Duration};

#[path = "support/embedded_daemon.rs"]
mod embedded_daemon;

// Existing daemon_client.rs measured 3,969 ms at worst under load and uses
// 30 seconds for a real daemon's cold start. This bounds a hang, not speed.
const START_BUDGET: Duration = Duration::from_secs(30);
// A daemon recovery was measured at 36.4 s worst on a loaded disk and uses 120 s to catch a
// jammed recovery. This only bounds the test wait; it is not startup latency.
const RECOVERY_JAM_GUARD: Duration = Duration::from_secs(120);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordered_shutdown_keeps_the_closing_notice_and_ends_reads_explicitly()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().to_owned()))?;
    let daemon = embedded_daemon::TestDaemon::start(directory.path(), None)?;
    daemon.ready(RECOVERY_JAM_GUARD).await?;
    let client = DaemonClient::connect(paths, daemon.client.clone());
    let mut changes = client.watch_frames();
    daemon
        .client
        .request_command(OrbitCommand::Shutdown)
        .await?;
    tokio::time::timeout(START_BUDGET, async {
        while client.composed_snapshot().connection.daemon != DaemonConnection::Lost {
            changes.changed().await?;
        }
        Ok::<_, Box<dyn Error>>(())
    })
    .await??;
    assert!(client.bootstrap().frames.iter().any(|frame| matches!(frame,
        FeatureFrame::Status { view, .. } if view.notice.as_ref().is_some_and(|notice| notice.code == orbit_core::notice::NoticeCode::OrbitClosing)
    )));
    let error = client
        .read_sessions_page(None)
        .await
        .err()
        .ok_or("closed daemon accepted a read")?;
    assert!(error.message.contains("orbit_closing"), "{}", error.message);
    assert!(
        !error
            .message
            .contains("Conexión cerrada antes del fin del mensaje")
    );
    Ok(())
}

/// A list read the orderly closure cuts says so with the one cause the
/// interface takes for «Orbit is closing», not a failure
/// (`apps/desktop/src/state/state-failures.ts`, `CLOSED_IN_ORDER`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_notes_read_cut_by_the_orderly_close_names_that_cause() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().to_owned()))?;
    let daemon = embedded_daemon::TestDaemon::start(directory.path(), None)?;
    daemon.ready(RECOVERY_JAM_GUARD).await?;
    let client = DaemonClient::connect(paths, daemon.client.clone());
    let mut changes = client.watch_frames();
    daemon
        .client
        .request_command(OrbitCommand::Shutdown)
        .await?;
    tokio::time::timeout(START_BUDGET, async {
        while client.composed_snapshot().connection.daemon != DaemonConnection::Lost {
            changes.changed().await?;
        }
        Ok::<_, Box<dyn Error>>(())
    })
    .await??;
    let refresh: AppCommand =
        serde_json::from_value(serde_json::json!({"command": "notes_refresh"}))?;
    client.send(refresh).await?;
    let index = client
        .composed_snapshot()
        .notes
        .and_then(|notes| notes.index)
        .ok_or("the list was not read")?;
    assert_eq!(index.detail.as_deref(), Some("orbit_closing"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn real_topic_changes_reach_status_recording_control_and_assistant_frames()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
    let daemon = embedded_daemon::TestDaemon::start(directory.path(), None)?;
    daemon.ready(RECOVERY_JAM_GUARD).await?;
    let client = DaemonClient::connect(paths, daemon.client.clone());
    let mut changes = client.watch_frames();
    tokio::time::timeout(START_BUDGET, async {
        loop {
            let initial = client.bootstrap();
            let recording = initial.frames.iter().any(|frame| matches!(frame,
                FeatureFrame::Recording { view, .. } if view.meeting.applications_state == MeetingApplicationsState::HelperMissing
                && view.meeting.applications_detail.is_some()));
            let control = initial.frames.iter().any(|frame| matches!(frame,
                FeatureFrame::Control { view, .. } if view.as_ref().as_ref().is_some_and(|view|
                    matches!(view.engine, orbit_app_core::EngineSnapshot::HelperNotConnected | orbit_app_core::EngineSnapshot::PermissionUnknown { .. }))));
            if recording && control && client.composed_snapshot().connection.daemon == DaemonConnection::Ready { break; }
            changes.changed().await?;
        }
        Ok::<_, Box<dyn Error>>(())
    }).await??;
    client.take_frames();
    let receipt = client
        .send(AppCommand::Daemon(Box::new(OrbitCommand::PrivacyStop)))
        .await?;
    assert!(
        matches!(receipt.delivery, Some(ReceiptDelivery::Reflected)),
        "{receipt:?}"
    );
    // Reflected means the status fact and its frame already exist on return.
    let mut pending = client.take_frames();
    let status = pending
        .iter()
        .find(|frame| matches!(frame, FeatureFrame::Status { view, .. } if view.privacy_stopped))
        .ok_or("privacy frame missing when send returned")?;
    let serialized = serde_json::to_value(status)?;
    assert_eq!(serialized["feature"], "status");
    assert_eq!(serialized["view"]["privacy_stopped"], true);
    let mut recording_seen = false;
    let mut control_seen = false;
    tokio::time::timeout(START_BUDGET, async {
        loop {
            for frame in pending.drain(..) {
                if let FeatureFrame::Recording { view, .. } = &frame
                    && view.meeting.applications_state == MeetingApplicationsState::PrivacyStopped
                {
                    let detail = view
                        .meeting
                        .applications_detail
                        .as_deref()
                        .ok_or("missing recording refusal")?;
                    assert_eq!(
                        serde_json::from_str::<serde_json::Value>(detail)?["code"],
                        "privacy_stopped"
                    );
                    assert!(view.meeting.applications.is_empty());
                    recording_seen = true;
                }
                if let FeatureFrame::Control { view, .. } = &frame
                    && view.as_ref().as_ref().is_some_and(|view| {
                        view.engine == orbit_app_core::EngineSnapshot::PrivacyStopped
                    })
                {
                    control_seen = true;
                }
            }
            if recording_seen && control_seen {
                break;
            }
            changes.changed().await?;
            pending = client.take_frames();
        }
        Ok::<_, Box<dyn Error>>(())
    })
    .await??;
    let response = request(
        &daemon.client,
        OrbitCommand::SessionOpen {
            id: "session-e2e".into(),
            agent_id: None,
        },
    )
    .await?;
    assert!(response.ok, "session open refused: {:?}", response.refusal);
    tokio::time::timeout(START_BUDGET, async {
        loop {
            for frame in client.take_frames() {
                if let FeatureFrame::Assistant { view, .. } = &frame
                    && view.session_id.as_deref() == Some("session-e2e")
                {
                    let data = serde_json::to_value(&frame)?;
                    assert_eq!(data["view"]["session_id"], "session-e2e");
                    let mut delivered =
                        orbit_app_core::AppSnapshot::empty(0, DaemonConnection::Ready);
                    frame.apply(&mut delivered);
                    assert_eq!(delivered.chat.session_id.as_deref(), Some("session-e2e"));
                    return Ok::<_, Box<dyn Error>>(());
                }
            }
            changes.changed().await?;
        }
    })
    .await??;
    drop(client);
    daemon.shutdown(START_BUDGET).await?;
    Ok(())
}
