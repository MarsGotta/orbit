use orbit_app_core::{DaemonClient, features::FeatureFrame};
use tauri::Emitter;

pub const FEATURE_EVENT_PREFIX: &str = "orbit://feature/";

fn emit_feature_frames<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    frames: impl IntoIterator<Item = FeatureFrame>,
) {
    for frame in frames {
        let menu = match &frame {
            FeatureFrame::Local { view, .. } => crate::tray::refresh_update(app, &view.update),
            FeatureFrame::Recording { view, .. } => {
                crate::tray::refresh_recording(app, &view.meeting, view.call.as_ref())
            }
            FeatureFrame::Assistant { view, .. } => crate::tray::refresh_assistant(app, view),
            _ => Ok(()),
        };
        if let Err(error) = menu {
            eprintln!("Orbit: no se pudo actualizar la barra de menús: {error}");
        }
        // A long turn that ended, or a question from the agent, while
        // Orbit is in the background.
        if let FeatureFrame::Assistant { view, .. } = &frame {
            crate::notify::follow(app, view);
        }
        let event = format!("{FEATURE_EVENT_PREFIX}{}", frame.feature().as_str());
        // The recording frame also says whether the floating panel
        // has something to say; it is shown or hidden after the frame went out.
        let panel = crate::panel::panel_visible(&frame);
        crate::panel_hover::follow(&frame);
        let height = crate::panel::panel_height(&frame).unwrap_or(crate::panel::PANEL_HEIGHT);
        if let Err(error) = app.emit(&event, frame) {
            eprintln!("No se pudo publicar el marco {event}: {error}");
        }
        if let Some(visible) = panel {
            crate::panel::apply(app, visible, height);
        }
    }
}

/// Generic over the Tauri runtime so a test can drive it with
/// `tauri::test::MockRuntime` instead of the Wry window runtime.
pub fn forward_frames<R: tauri::Runtime>(app: tauri::AppHandle<R>, client: DaemonClient) {
    let mut frame_changes = client.watch_frames();
    tauri::async_runtime::spawn(async move {
        while frame_changes.changed().await.is_ok() {
            frame_changes.borrow_and_update();
            emit_feature_frames(&app, client.take_frames());
        }
    });
}

#[cfg(test)]
#[path = "../../../../crates/orbit-app-core/tests/support/embedded_daemon.rs"]
mod embedded_daemon;

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_app_core::{
        DaemonConnection, MeetingApplicationsState,
        features::{self, FeatureFrame, FeatureInput, FrameRevision},
        topics::TopicCache,
    };
    use orbit_core::protocol::Command;
    use orbit_ipc::Paths;
    use std::{
        error::Error,
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tauri::Listener;

    #[test]
    fn changed_status_frame_is_emitted_on_the_store_event() {
        let app = tauri::test::mock_app();
        let snapshot = orbit_app_core::AppSnapshot::empty(1, DaemonConnection::Ready);
        let topics = TopicCache::default();
        let frame = features::project_all(
            &FeatureInput::new(&snapshot, &topics),
            FrameRevision {
                epoch: 2,
                counter: 3,
            },
        )
        .into_iter()
        .find(|frame| matches!(frame, FeatureFrame::Status { .. }));
        let Some(frame) = frame else {
            panic!("status feature is registered")
        };
        let observed = std::sync::Arc::new(std::sync::Mutex::new(None));
        let received = observed.clone();
        app.listen_any("orbit://feature/status", move |event| {
            if let Ok(mut slot) = received.lock() {
                *slot = serde_json::from_str::<FeatureFrame>(event.payload()).ok();
            }
        });
        emit_feature_frames(app.handle(), [frame.clone()]);
        let delivered = observed.lock().ok().and_then(|slot| slot.clone());
        assert_eq!(delivered, Some(frame));
    }

    #[tokio::test]
    async fn real_recording_and_control_changes_reach_the_mock_runtime_store()
    -> Result<(), Box<dyn Error>> {
        // Existing daemon-client measurements observed 3,969 ms at worst on
        // the loaded machine. Thirty seconds bounds a hung fixture, not speed.
        let budget = Duration::from_secs(30);
        let directory = tempfile::tempdir()?;
        let paths = Paths::resolve(Some(directory.path().to_path_buf()))?;
        let daemon = embedded_daemon::TestDaemon::start(directory.path(), None)?;
        daemon.ready(budget).await?;
        let client = DaemonClient::connect(paths.clone(), daemon.client.clone());
        let mut frames = client.watch_frames();
        tokio::time::timeout(budget, async {
            while client.composed_snapshot().connection.daemon != DaemonConnection::Ready {
                frames.changed().await?;
            }
            Ok::<_, tokio::sync::watch::error::RecvError>(())
        })
        .await??;
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))?;
        let store = Arc::new(Mutex::new(
            std::collections::HashMap::<String, FeatureFrame>::new(),
        ));
        let observed = Arc::clone(&store);
        let (sent, mut received) = tokio::sync::mpsc::unbounded_channel();
        for event in ["orbit://feature/recording", "orbit://feature/control"] {
            let observed = Arc::clone(&observed);
            let sent = sent.clone();
            app.listen(event, move |event| {
                if let Ok(frame) = serde_json::from_str::<FeatureFrame>(event.payload()) {
                    if let Ok(mut frames) = observed.lock() {
                        frames.insert(frame.feature().as_str().to_owned(), frame.clone());
                    }
                    let _ = sent.send(frame);
                }
            });
        }
        forward_frames(app.handle().clone(), client);
        tokio::time::timeout(budget, async {
            loop {
                let frame = received.recv().await.ok_or("event stream closed")?;
                if let FeatureFrame::Control { view, .. } = &frame
                    && view.as_ref().as_ref().is_some_and(|view| {
                        matches!(
                            view.engine,
                            orbit_app_core::EngineSnapshot::HelperNotConnected
                                | orbit_app_core::EngineSnapshot::PermissionUnknown { .. }
                        )
                    })
                {
                    assert_eq!(serde_json::to_value(&frame)?["feature"], "control");
                    return Ok::<_, Box<dyn Error>>(());
                }
            }
        })
        .await??;
        let reply = daemon.client.request_command(Command::PrivacyStop).await?;
        assert!(reply.ok, "privacy stop refused: {:?}", reply.refusal);
        let mut recording_seen = false;
        let mut control_seen = false;
        tokio::time::timeout(budget, async {
            while !recording_seen || !control_seen {
                let frame = received.recv().await.ok_or("event stream closed")?;
                if let FeatureFrame::Recording { view, .. } = &frame
                    && view.meeting.applications_state == MeetingApplicationsState::PrivacyStopped
                {
                    recording_seen = true;
                }
                if let FeatureFrame::Control { view, .. } = &frame
                    && view.as_ref().as_ref().is_some_and(|view| {
                        view.engine == orbit_app_core::EngineSnapshot::PrivacyStopped
                    })
                {
                    assert_eq!(serde_json::to_value(&frame)?["feature"], "control");
                    control_seen = true;
                }
            }
            Ok::<_, Box<dyn Error>>(())
        })
        .await??;
        daemon.shutdown(budget).await?;
        let stored = store.lock().map_err(|_| "store lock poisoned")?;
        let Some(FeatureFrame::Recording { view, .. }) = stored.get("recording") else {
            return Err("recording frame was not stored".into());
        };
        assert_eq!(
            view.meeting.applications_state,
            MeetingApplicationsState::PrivacyStopped
        );
        let Some(FeatureFrame::Control { view, .. }) = stored.get("control") else {
            return Err("control frame was not stored".into());
        };
        assert_eq!(
            view.as_ref().as_ref().ok_or("control view missing")?.engine,
            orbit_app_core::EngineSnapshot::PrivacyStopped
        );
        Ok(())
    }
}
