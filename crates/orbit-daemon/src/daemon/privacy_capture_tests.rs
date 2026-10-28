//! The privacy stop at the daemon actor: the only stop that lasts is the one
//! the user pressed, and every other cause that used to stop Orbit cuts the live
//! capture or says its cause. Synthetic helper channel and real encrypted
//! store; every event goes through the loop's own handlers
//! (`handle_native_event`, `run_maintenance`), never a hand-written copy.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;
use crate::daemon::audio::tests::fixture;
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::protocol::MeetingAudio;

#[tokio::test]
async fn typed_permission_frames_name_the_actual_mac_permission() -> Result<()> {
    use orbit_core::{PermissionState, calendar::CalendarAuthorization};
    let cases = [
        (
            "calendar_read",
            CalendarAuthorization::Denied,
            PermissionState::Granted,
            PermissionState::Granted,
            "permission_denied",
            "calendar",
        ),
        (
            "calendar_read",
            CalendarAuthorization::Unknown,
            PermissionState::Granted,
            PermissionState::Granted,
            "permission_unknown",
            "calendar",
        ),
        (
            "screen_sources",
            CalendarAuthorization::FullAccess,
            PermissionState::Denied,
            PermissionState::Granted,
            "permission_denied",
            "screen_recording",
        ),
        (
            "screen_sources",
            CalendarAuthorization::FullAccess,
            PermissionState::Unknown,
            PermissionState::Granted,
            "permission_unknown",
            "screen_recording",
        ),
        (
            "control_tools",
            CalendarAuthorization::FullAccess,
            PermissionState::Granted,
            PermissionState::Denied,
            "permission_denied",
            "accessibility",
        ),
        (
            "control_tools",
            CalendarAuthorization::FullAccess,
            PermissionState::Granted,
            PermissionState::Unknown,
            "permission_unknown",
            "accessibility",
        ),
        (
            "control_tools",
            CalendarAuthorization::FullAccess,
            PermissionState::Denied,
            PermissionState::Denied,
            "permission_denied",
            "screen_and_accessibility",
        ),
        (
            "control_tools",
            CalendarAuthorization::FullAccess,
            PermissionState::Unknown,
            PermissionState::Unknown,
            "permission_unknown",
            "screen_and_accessibility",
        ),
    ];
    for (capability, calendar, screen, accessibility, code, permission) in cases {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _helper) = fixture(directory.path())?;
        {
            let fixture_value = serde_json::json!(true);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.native_helper",
                fixture_value,
            );
        }
        {
            let fixture_value = serde_json::json!(screen);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.screen_state",
                fixture_value,
            );
        }
        {
            let fixture_value = serde_json::json!(accessibility);
            crate::daemon::authority_fixture::set(
                &mut runtime.state,
                "capabilities.accessibility_state",
                fixture_value,
            );
        }
        runtime.calendar.view.authorization = calendar;
        let (sender, mut receiver) = mpsc::channel(16);
        runtime.link.outbound.install(sender);
        runtime
            .device_request_raw(
                &json!({"request_id":"ask","capability":capability,"parameters":{}}),
            )
            .await?;
        let reply = receiver.try_recv()?;
        assert_eq!(reply["params"]["refusal"]["code"], code, "{capability}");
        assert_eq!(
            reply["params"]["refusal"]["permission"], permission,
            "{capability}"
        );
    }
    Ok(())
}

/// Every order the helper was sent since the last call, by name.
fn sent(helper: &mut mpsc::Receiver<Value>) -> Vec<Value> {
    let mut commands = Vec::new();
    while let Ok(command) = helper.try_recv() {
        commands.push(command);
    }
    commands
}

fn names(commands: &[Value]) -> Vec<String> {
    commands
        .iter()
        .map(|command| command["command"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// Every notice said so far, oldest first, including the current one, as
/// JSON so its code and data read; then every line of `daemon.log`, where
/// the cause sentences are kept under the same op.
fn said(runtime: &Runtime) -> Vec<String> {
    let mut texts: Vec<String> = runtime
        .state
        .durable
        .notice_log
        .records()
        .iter()
        .chain(runtime.state.durable.notice.iter())
        .map(|record| serde_json::to_string(record).unwrap_or_default())
        .collect();
    if let Some(log) = &runtime.diagnostics
        && let Ok(text) = std::fs::read_to_string(log.path())
    {
        texts.extend(text.lines().map(str::to_owned));
    }
    texts
}

async fn start_recording(runtime: &mut Runtime, helper: &mut mpsc::Receiver<Value>) -> String {
    runtime
        .apply(Event::StartMeeting {
            id: "privacy-meeting".into(),
            title: "Reunión sintética".into(),
            audio: MeetingAudio::MicrophoneOnly,
        })
        .await
        .unwrap();
    let start = helper.recv().await.expect("audio_start absent");
    assert_eq!(start["command"], "audio_start");
    runtime
        .handle_native_event(json!({"event":"response","id":start["id"],"ok":true,
            "result":{"stream_id":start["stream_id"]}}))
        .await;
    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .unwrap()
            .phase,
        CapturePhase::Recording
    );
    start["stream_id"].as_str().unwrap().to_owned()
}

fn frame(stream: &str, sequence: u64, sample_rate: u32) -> Value {
    json!({"event":"audio", "source":"mic_local", "stream_id":stream,
        "sample_rate":sample_rate, "channels":1, "frames":100, "sequence":sequence,
        "monotonic_ns":sequence.saturating_mul(12_500_000),
        "pcm_s16le_base64":STANDARD.encode(vec![1_u8; 200])})
}

/// The state a daemon starts with after its process restarted with `stopped`
/// as the last snapshot, and a helper that already reported itself.
fn restarted(runtime: &mut Runtime, stopped: bool) {
    let mut durable = runtime.state.durable();
    if stopped {
        durable = decide(&State::from(durable), Event::PrivacyStop, 0)
            .unwrap()
            .state
            .durable();
    }
    let capabilities = runtime.state.capabilities().clone();
    runtime.state = State::from(durable.recovered());
    {
        let fixture_value = serde_json::json!(capabilities);
        crate::daemon::authority_fixture::set(&mut runtime.state, "capabilities", fixture_value);
    }
}

fn helper_state(privacy_stopped: bool) -> Value {
    json!({"event":"state","privacy_stopped":privacy_stopped,"connected":true,
        "native_instance_id":"00000000-0000-4000-8000-000000000001","privacy_generation":1,
        "microphone":false,"system_audio":false,"drain_unconfirmed":false,"playing":false})
}

#[tokio::test]
async fn a_restarted_daemon_without_the_users_stop_comes_back_working() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, mut helper) = fixture(directory.path()).unwrap();
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    restarted(&mut runtime, false);
    runtime
        .handle_native_event(json!({"event":"ready","capabilities":["local_privacy_stop"]}))
        .await;
    runtime.handle_native_event(helper_state(false)).await;
    assert!(!runtime.state.privacy_stopped(), "{:?}", said(&runtime));
    assert!(
        !names(&sent(&mut helper)).contains(&"privacy_stop".to_owned()),
        "the helper is not asked to latch a stop nobody pressed"
    );
    for text in said(&runtime) {
        assert!(!text.contains("privacy_stop_kept"), "{text}");
    }
    // Working means the user's next order is accepted, not refused as stopped.
    runtime
        .apply(Event::StartMeeting {
            id: "after-restart".into(),
            title: "Reunión sintética".into(),
            audio: MeetingAudio::MicrophoneOnly,
        })
        .await
        .unwrap();
    assert!(names(&sent(&mut helper)).contains(&"audio_start".to_owned()));
}

#[tokio::test]
async fn the_users_stop_after_a_restart_reaches_the_helper_and_both_agree() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, mut helper) = fixture(directory.path()).unwrap();
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    restarted(&mut runtime, true);
    assert!(runtime.state.privacy_stopped());
    runtime
        .handle_native_event(json!({"event":"ready","capabilities":["local_privacy_stop"]}))
        .await;
    assert!(
        names(&sent(&mut helper)).contains(&"privacy_stop".to_owned()),
        "a helper without the latch is told, so both say the same"
    );
    runtime.handle_native_event(helper_state(true)).await;
    assert!(runtime.state.privacy_stopped());
    // "mars" is the user's stop as stored: wire/stored name kept for compatibility with 0.9.0.
    assert_eq!(
        serde_json::to_value(&runtime.state).unwrap()["privacy_stop_by"],
        "mars",
        "the stop that survived is recorded as the user's"
    );
}

#[tokio::test]
async fn a_latch_the_helper_kept_is_adopted_so_both_agree() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, _helper) = fixture(directory.path()).unwrap();
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    restarted(&mut runtime, false);
    runtime.handle_native_event(helper_state(true)).await;
    assert!(runtime.state.privacy_stopped());
    assert!(
        said(&runtime)
            .iter()
            .any(|text| text.contains("\"privacy_stopped\"") && text.contains("helper_latch")),
        "{:?}",
        said(&runtime)
    );
}

#[tokio::test]
async fn nothing_remote_rearms_the_users_stop() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, _helper) = fixture(directory.path()).unwrap();
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    runtime.settings.device_id = Some("expected-device".into());
    runtime
        .command(orbit_core::protocol::Command::PrivacyStop)
        .await
        .unwrap();
    for event in [
        HubEvent::Connected,
        HubEvent::Frame(json!({"event":"ready","data":{"device_id":"expected-device"}})),
        HubEvent::Frame(json!({"v":1,"event":"state.snapshot","data":{"privacy_stopped":false}})),
        HubEvent::Frame(json!({"v":1,"event":"local_rearm","data":{}})),
    ] {
        let _ = runtime.hub_event(event).await;
        assert!(runtime.state.privacy_stopped());
    }
}

#[tokio::test]
async fn a_privacy_stopped_refusal_is_not_taken_for_the_users_stop() {
    // The helper refuses with `privacy_stopped` while its latch is set and
    // also while a capture stop is still cleaning up. Only its `state` says
    // which; a refusal alone must never become a durable stop.
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, mut helper) = fixture(directory.path()).unwrap();
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    runtime.native_send("permissions", json!({})).await.unwrap();
    let permissions = helper.recv().await.unwrap();
    runtime
        .handle_native_event(json!({"event":"response","id":permissions["id"],"ok":false,
            "error":{"code":"privacy_stopped","message":"Orbit está terminando de parar la captura"}}))
        .await;
    assert!(!runtime.state.privacy_stopped(), "{:?}", said(&runtime));
    assert!(crate::daemon::turn::said::mentions(
        &runtime.state,
        "privacy_stopped"
    ));
}

fn inventory(rows: usize, omitted: u64) -> Value {
    let applications: Vec<Value> = (0..rows)
        .map(|index| {
            json!({"application_pid": 1_000 + index, "application_name": format!("App {index}"),
                "bundle_id":"com.google.Chrome","binding":null,
                "unavailable_reason":"application_identity_unavailable"})
        })
        .collect();
    json!({"native_instance_id":"00000000-0000-4000-8000-000000000001",
        "privacy_generation":0,"clock_domain":"mach_absolute_time",
        "observed_monotonic_ns":1_000,"completed_monotonic_ns":2_000,
        "omitted_count":omitted,"applications":applications})
}

#[tokio::test]
async fn an_unverifiable_inventory_is_shown_with_its_numbers_and_stops_nothing() {
    for (rows, omitted, numbers) in [(40, 0, ["40", "32"]), (3, 1, ["1", "omit"])] {
        let directory = tempfile::tempdir().unwrap();
        let (mut runtime, mut helper) = fixture(directory.path()).unwrap();
        runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
        runtime
            .command(orbit_core::protocol::Command::AudioSourcesRefresh)
            .await
            .unwrap();
        let request = helper.recv().await.unwrap();
        assert_eq!(request["command"], "application_sources");
        runtime
            .handle_native_event(json!({"event":"response","id":request["id"],"ok":true,
                "result":inventory(rows, omitted)}))
            .await;
        assert!(
            !runtime.state.privacy_stopped(),
            "{rows}/{omitted}: {:?}",
            said(&runtime)
        );
        assert!(runtime.state.views.audio_sources.is_empty());
        let orbit_core::topics::TopicData::Recording(orbit_core::topics::Knowledge::Present {
            value,
            ..
        }) = super::topic(&runtime, orbit_core::topics::TopicId::Recording)
        else {
            panic!("recording topic absent")
        };
        let orbit_core::topics::Knowledge::Absent { refusal } = value.applications else {
            panic!("inventory refusal absent")
        };
        let unavailable = refusal.to_string();
        for number in numbers {
            assert!(
                unavailable.contains(number),
                "{rows}/{omitted}: the view must say {number}: {unavailable:?}"
            );
        }
        assert!(
            !names(&sent(&mut helper)).contains(&"privacy_stop".to_owned()),
            "{rows}/{omitted}"
        );
        // The rest of Orbit keeps working: the next recording starts.
        runtime
            .apply(Event::StartMeeting {
                id: format!("after-inventory-{rows}"),
                title: "Reunión sintética".into(),
                audio: MeetingAudio::MicrophoneOnly,
            })
            .await
            .unwrap();
        assert!(names(&sent(&mut helper)).contains(&"audio_start".to_owned()));
    }
}

#[tokio::test]
async fn a_format_change_starts_a_new_stretch_without_cutting() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, mut helper) = fixture(directory.path()).unwrap();
    let stream = start_recording(&mut runtime, &mut helper).await;
    runtime.handle_native_event(frame(&stream, 0, 8_000)).await;
    runtime.handle_native_event(frame(&stream, 1, 16_000)).await;
    assert!(!runtime.state.privacy_stopped());
    let recording = runtime.state.durable.capture.recording.as_ref().unwrap();
    assert_eq!(recording.phase, CapturePhase::Recording);
    assert_eq!(recording.gaps, 1);
    assert!(!said(&runtime).join(" | ").contains("capture_cut"));
    assert!(!names(&sent(&mut helper)).contains(&"capture_stop".to_owned()));
}

#[tokio::test]
async fn a_sleeping_mac_cuts_the_recording_and_orbit_comes_back_working() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, mut helper) = fixture(directory.path()).unwrap();
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    start_recording(&mut runtime, &mut helper).await;
    runtime
        .handle_native_event(
            json!({"event":"capture_stopped","reason":"session_inactive",
            "privacy_stopped":false,"drain_complete":false}),
        )
        .await;
    assert!(!runtime.state.privacy_stopped(), "{:?}", said(&runtime));
    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .unwrap()
            .phase,
        CapturePhase::Interrupted,
        "the recording is cut, not left running on a sleeping Mac"
    );
    let notice = said(&runtime).join(" | ");
    assert!(notice.contains("session_inactive"), "{notice}");
    // A cut, not the user's stop: nothing to rearm.
    assert!(notice.contains("\"capture_cut\""), "{notice}");
    assert!(!notice.contains("\"privacy_stopped\""), "{notice}");
    assert!(!notice.contains("rearm"), "{notice}");
    assert!(!names(&sent(&mut helper)).contains(&"privacy_stop".to_owned()));
    // The session resumes: the helper says it is not latched, nothing asks
    // for a rearm and nothing restarts the recording.
    runtime.handle_native_event(helper_state(false)).await;
    assert!(!runtime.state.privacy_stopped());
    assert!(!names(&sent(&mut helper)).contains(&"audio_start".to_owned()));
}

#[tokio::test]
async fn closing_the_lid_without_a_recording_leaves_orbit_as_it_was() {
    for event in [
        json!({"event":"capture_stopped","reason":"session_inactive",
            "privacy_stopped":false,"drain_complete":true}),
        // What a helper that still named every stop a privacy stop sent.
        json!({"event":"privacy_stop","reason":"session_inactive",
            "privacy_stopped":false,"drain_complete":true}),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let (mut runtime, mut helper) = fixture(directory.path()).unwrap();
        runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
        runtime.handle_native_event(event.clone()).await;
        assert!(
            !runtime.state.privacy_stopped(),
            "{event}: {:?}",
            said(&runtime)
        );
        assert!(!names(&sent(&mut helper)).contains(&"privacy_stop".to_owned()));
        for text in said(&runtime) {
            assert!(!text.contains("rearm"), "{event}: {text}");
            // With nothing recording, nothing was cut.
            assert!(!text.contains("\"capture_cut\""), "{event}: {text}");
        }
        assert!(
            said(&runtime)
                .iter()
                .any(|text| text.contains("mac_stopped_idle_capture")),
            "{event}: {:?}",
            said(&runtime)
        );
    }
}

#[tokio::test]
async fn a_failure_in_the_loop_never_latches() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, mut helper) = fixture(directory.path()).unwrap();
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    // The helper process going away used to be an error of the helper branch.
    runtime
        .handle_native_event(json!({"event":"helper_disconnected"}))
        .await;
    assert!(!runtime.state.privacy_stopped(), "{:?}", said(&runtime));
    assert!(
        said(&runtime).iter().any(|text| text.contains("helper")),
        "the loss is said: {:?}",
        said(&runtime)
    );
    runtime
        .handle_native_event(json!({"event":"ready","capabilities":["local_privacy_stop"]}))
        .await;
    // A maintenance whose retry of an unsaved fragment fails: the disk
    // refuses the chunk's path.
    std::fs::create_dir_all(runtime.paths.store.join("chunks/privacy-mic_local-0.orbit")).unwrap();
    let segment = crate::capture::Segment {
        metadata: orbit_store::ChunkMetadata {
            capture_epoch_id: None,
            id: "privacy-mic_local-0".into(),
            recording_id: "privacy".into(),
            track: "mic_local".into(),
            sequence: 0,
            sample_rate: 8_000,
            channels: 1,
            frames: 8_000,
            monotonic_ns: 1,
            first_sample: 0,
            started_at_ms: 1_000,
        },
        pcm: vec![7; 16_000],
    };
    assert!(runtime.seal(vec![segment]).is_err());
    runtime.capture.seal_retry_at = None;
    runtime.run_maintenance().await;
    assert!(!runtime.state.privacy_stopped(), "{:?}", said(&runtime));
    assert!(!names(&sent(&mut helper)).contains(&"privacy_stop".to_owned()));
    assert_eq!(
        runtime.capture.pending_seals.len(),
        1,
        "the unsaved fragment is still held for the next retry"
    );
}

#[tokio::test]
async fn the_users_controls_are_still_the_stop_and_are_recorded_as_theirs() {
    for control in ["order", "hotkey"] {
        let directory = tempfile::tempdir().unwrap();
        let (mut runtime, mut helper) = fixture(directory.path()).unwrap();
        runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
        if control == "order" {
            runtime
                .command(orbit_core::protocol::Command::PrivacyStop)
                .await
                .unwrap();
        } else {
            runtime
                .handle_native_event(
                    json!({"event":"hotkey","action":"privacy_stop","phase":"pressed"}),
                )
                .await;
        }
        assert!(runtime.state.privacy_stopped(), "{control}");
        // Stored value of the user's stop: wire/stored name kept for compatibility with 0.9.0.
        assert_eq!(
            serde_json::to_value(&runtime.state).unwrap()["privacy_stop_by"],
            "mars",
            "{control}"
        );
        assert!(names(&sent(&mut helper)).contains(&"privacy_stop".to_owned()));
        let persisted = runtime
            .fixture_store
            .load_snapshot::<State>()
            .unwrap()
            .unwrap();
        assert!(
            persisted.privacy_stopped(),
            "{control}: the user's stop is durable"
        );
    }
}

/// The helper echoes `daemon_requested` for an order the daemon sent.
/// Arriving with the daemon not stopped, it is not an order of the user's the
/// daemon can vouch for: the latch is adopted so both agree, and it is never
/// recorded as the user's.
#[tokio::test]
async fn an_echo_of_the_daemons_order_is_never_taken_for_the_users() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, mut helper) = fixture(directory.path()).unwrap();
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    runtime
        .handle_native_event(json!({"event":"privacy_stop","reason":"daemon_requested",
            "privacy_stopped":true,"drain_complete":false}))
        .await;
    assert!(runtime.state.privacy_stopped(), "both say the same");
    assert_eq!(
        serde_json::to_value(&runtime.state).unwrap()["privacy_stop_by"],
        serde_json::Value::Null,
        "an echo nobody can attribute is never the user's: {:?}",
        said(&runtime)
    );
    // The user's own control at the helper still is theirs.
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, _helper) = fixture(directory.path()).unwrap();
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    runtime
        .handle_native_event(json!({"event":"privacy_stop","reason":"local_hotkey",
            "privacy_stopped":true,"drain_complete":false}))
        .await;
    // Stored value of the user's stop: wire/stored name kept for compatibility with 0.9.0.
    assert_eq!(
        serde_json::to_value(&runtime.state).unwrap()["privacy_stop_by"],
        "mars"
    );
    assert!(!names(&sent(&mut helper)).contains(&"capture_stop".to_owned()));
}

/// The other vertical's side: a data fault outside the capture
/// is said with its cause, and the recording goes on: nothing is cut, nothing
/// is sent to the helper, fault after fault. (The publication journal that
/// used to fail here on demand is gone; the fault now enters where
/// `run_maintenance` hands a vertical's maintenance error.)
#[tokio::test]
async fn a_fault_of_another_vertical_is_said_and_the_recording_goes_on() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, mut helper) = fixture(directory.path()).unwrap();
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    start_recording(&mut runtime, &mut helper).await;
    {
        let fixture_value = serde_json::json!(orbit_core::Connection::Online);
        crate::daemon::authority_fixture::set(&mut runtime.state, "connection", fixture_value);
    }
    for _ in 0..3 {
        runtime
            .contain(
                orbit_core::privacy_stop::StopCause::VerticalMaintenanceFailed,
                "Fallo en el mantenimiento de Orbit, no de la captura: fallo sintético. No se corta nada.",
            )
            .await;
        runtime.run_maintenance().await;
        assert_eq!(
            runtime
                .state
                .durable
                .capture
                .recording
                .as_ref()
                .unwrap()
                .phase,
            CapturePhase::Recording,
            "{:?}",
            said(&runtime)
        );
    }
    let notice = said(&runtime).join(" | ");
    assert!(notice.contains("no de la captura"), "{notice}");
    assert!(notice.contains("No se corta nada"), "{notice}");
    let commands = names(&sent(&mut helper));
    assert!(
        !commands.contains(&"capture_stop".to_owned()),
        "{commands:?}"
    );
    assert!(!runtime.state.privacy_stopped());

    // The same for a helper event of another vertical that fails.
    runtime
        .contain(
            orbit_core::privacy_stop::StopCause::NativeEventFailed,
            "Fallo sintético de la agenda.",
        )
        .await;
    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .unwrap()
            .phase,
        CapturePhase::Recording
    );
    assert!(!names(&sent(&mut helper)).contains(&"capture_stop".to_owned()));
}

/// The capture's side: its own upkeep failing (a fragment the
/// disk will not take) cuts the live recording with its cause.
#[tokio::test]
async fn a_seal_failure_counts_a_gap_and_keeps_recording() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, mut helper) = fixture(directory.path()).unwrap();
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    let stream = start_recording(&mut runtime, &mut helper).await;
    runtime.handle_native_event(frame(&stream, 0, 8_000)).await;
    std::fs::create_dir_all(
        runtime
            .paths
            .store
            .join("chunks/privacy-cut-mic_local-0.orbit"),
    )
    .unwrap();
    let segment = crate::capture::Segment {
        metadata: orbit_store::ChunkMetadata {
            capture_epoch_id: None,
            id: "privacy-cut-mic_local-0".into(),
            recording_id: "privacy-meeting".into(),
            track: "mic_local".into(),
            sequence: 0,
            sample_rate: 8_000,
            channels: 1,
            frames: 8_000,
            monotonic_ns: 1,
            first_sample: 0,
            started_at_ms: 1_000,
        },
        pcm: vec![7; 16_000],
    };
    assert!(runtime.seal(vec![segment]).is_err());
    runtime.capture.seal_retry_at = None;
    runtime.run_maintenance().await;
    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .unwrap()
            .phase,
        CapturePhase::Recording,
        "{:?}",
        said(&runtime)
    );
    assert!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .unwrap()
            .gaps
            > 0
    );
    assert!(!said(&runtime).join(" | ").contains("capture_cut"));
}

#[tokio::test]
async fn only_the_capture_s_own_events_count_as_capture_failures() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, mut helper) = fixture(directory.path()).unwrap();
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    runtime
        .native_send(
            "audio_start",
            json!({"source":"mic_local","stream_id":"fixture-stream"}),
        )
        .await
        .unwrap();
    let audio = helper.recv().await.unwrap();
    runtime
        .native_send(
            "application_sources",
            json!({"bundle_ids":["com.example.meeting"]}),
        )
        .await
        .unwrap();
    let sources = helper.recv().await.unwrap();
    for (event, capture) in [
        (json!({"event":"audio"}), true),
        (json!({"event":"gap"}), true),
        (json!({"event":"helper_disconnected"}), true),
        (json!({"event":"capture_stopped"}), true),
        (json!({"event":"response","id":audio["id"]}), true),
        (json!({"event":"response","id":sources["id"]}), false),
        (json!({"event":"response","id":"calendar-read-1"}), false),
        (json!({"event":"calendar_invalidated"}), false),
        (json!({"event":"error","code":"x"}), false),
    ] {
        assert_eq!(
            runtime.native_event_touches_capture(&event),
            capture,
            "{event}"
        );
    }
}

/// `pending_native` is only a bounded diagnostic index. Losing
/// an entry there cannot turn an `audio_stop` failure into another vertical's
/// fault while the capture still owns that exact ID.
#[tokio::test]
async fn an_evicted_audio_stop_ack_still_cuts_and_closes_the_microphone() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, mut helper) = fixture(directory.path()).unwrap();
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    start_recording(&mut runtime, &mut helper).await;
    runtime.apply(Event::FinishMeeting).await.unwrap();
    let stop = helper.recv().await.expect("audio_stop absent");
    assert_eq!(stop["command"], "audio_stop");

    for index in 0..MAX_PENDING_NATIVE {
        runtime.helper.record_pending(&json!({
            "id": format!("zz-noncapture-{index:03}"),
            "command": "application_sources"
        }));
    }
    assert!(
        !runtime
            .helper
            .pending
            .contains_key(stop["id"].as_str().unwrap()),
        "the fixture must reproduce eviction from the generic index"
    );
    runtime
        .handle_native_event(json!({"event":"response","id":stop["id"],"ok":true,
            "result":{"stream_id":stop["stream_id"],"drain_complete":false}}))
        .await;
    let cut = helper.try_recv().expect("capture_stop absent");
    assert_eq!(cut["command"], "capture_stop");
    runtime
        .handle_native_event(json!({"event":"response","id":cut["id"],"ok":true,
            "result":{"drain_complete":true}}))
        .await;
    assert!(runtime.audio.mic_stream.is_none());
    assert!(!runtime.audio.audio_is_draining());
    let notice = said(&runtime).join(" | ");
    assert!(notice.contains("audio_stop"), "{notice}");
    assert!(!notice.contains("No se corta nada"), "{notice}");
}

/// A retired deadline must not send even a failed stop to a stalled helper.
#[tokio::test]
async fn retired_expiry_never_stops_audio_even_when_the_helper_is_stalled() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, mut helper) = fixture(directory.path()).unwrap();
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    start_recording(&mut runtime, &mut helper).await;
    runtime
        .state
        .durable
        .capture
        .recording
        .as_mut()
        .unwrap()
        .expires_at_ms = now() - 1;
    let (stalled_native, mut stalled) = mpsc::channel(1);
    stalled_native
        .try_send(json!({"fixture":"occupied"}))
        .unwrap();
    runtime.helper.sender = stalled_native;

    runtime.run_maintenance().await;
    assert!(stalled.try_recv().is_ok());
    assert!(
        stalled.try_recv().is_err(),
        "a retired deadline sent an audio stop"
    );
    assert!(runtime.audio.mic_stream.is_some());
    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .unwrap()
            .phase,
        orbit_core::CapturePhase::Recording
    );
}

#[tokio::test]
async fn privacy_stop_stalled_once_is_retried_until_the_microphone_closes() {
    let directory = tempfile::tempdir().unwrap();
    let (mut runtime, mut helper) = fixture(directory.path()).unwrap();
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    start_recording(&mut runtime, &mut helper).await;
    let (stalled_native, _occupied) = mpsc::channel(1);
    stalled_native
        .try_send(json!({"fixture":"occupied"}))
        .unwrap();
    runtime.helper.sender = stalled_native;

    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    assert!(runtime.state.privacy_stopped());
    assert!(
        runtime.audio.full_stop_send_owed(),
        "privacy_stop was never enqueued"
    );
    assert!(
        runtime.audio.mic_stream.is_some(),
        "no native drain was acknowledged"
    );

    let (native, mut recovered) = mpsc::channel(8);
    runtime.helper.sender = native;
    runtime.run_maintenance().await;
    let retry =
        crate::daemon::audio::tests::take_helper_command(&mut recovered, "privacy_stop").unwrap();
    runtime
        .handle_native_event(json!({"event":"response","id":retry["id"],"ok":true,
        "result":{"privacy_stopped":true,"drain_complete":true}}))
        .await;
    assert!(runtime.audio.mic_stream.is_none());
    assert!(!runtime.audio.audio_is_draining());
    assert!(runtime.state.privacy_stopped());
}

/// What can still be capturing is also what the helper holds
/// open, not only what the state says. A stream or a start left in the
/// helper with no recording in the state is still cut.
#[tokio::test]
async fn a_stream_still_open_in_the_helper_is_cut_whatever_the_state_says() {
    for open in ["mic", "remote", "start"] {
        let directory = tempfile::tempdir().unwrap();
        let (mut runtime, mut helper) = fixture(directory.path()).unwrap();
        runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
        match open {
            "mic" => runtime.audio.mic_stream = Some("orphan-mic".into()),
            "remote" => runtime.audio.remote_stream = Some("orphan-remote".into()),
            _ => runtime.audio.pending_starts.push("orphan-start".into()),
        }
        assert!(!orbit_core::privacy_stop::capture_live(&runtime.state));
        runtime
            .contain(
                orbit_core::privacy_stop::StopCause::CaptureEventFailed,
                "Fallo sintético de la captura.",
            )
            .await;
        assert!(
            names(&sent(&mut helper)).contains(&"capture_stop".to_owned()),
            "{open}: the helper is told to stop what it still holds"
        );
        assert!(!runtime.state.privacy_stopped(), "{open}");
    }
}
