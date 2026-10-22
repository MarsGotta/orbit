#![allow(clippy::unwrap_used)]
//! Real subprocess/Unix socket checks; no native device or provider is mocked as verified.
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[path = "support/ipc.rs"]
mod ipc;
#[path = "daemon_lifecycle/ipc_admission.rs"]
mod ipc_admission;
use orbit_core::protocol::Command as IpcCommand;

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn command(path: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_orbit"));
    command.args(["--dev", "--data-dir"]).arg(path);
    command
}

fn start(path: &Path) -> Daemon {
    let mut child = command(path)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(
            child.try_wait().unwrap().is_none(),
            "daemon exited before IPC readiness"
        );
        assert!(Instant::now() < deadline, "daemon did not become ready");
        // The socket answers `status` while the store is still recovering
        // and every other order is refused until it ends with
        // «El almacén se está recuperando…». On macOS, with the tests of this
        // binary running in parallel, a restart's recovery outlived the first
        // `status` and the next order was refused (0.3.13 build, 29-09). Ready
        // is the recovery's end, not a clock.
        if ipc::status(path).is_ok_and(|state| state["recovery"]["state"] == "ready") {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    Daemon(child)
}

fn status(path: &Path) -> serde_json::Value {
    ipc::status(path).unwrap()
}

#[test]
fn privacy_stop_survives_real_daemon_restart_without_network() {
    let directory = tempfile::Builder::new()
        .prefix("orbit-lifecycle-")
        .tempdir_in("/tmp")
        .unwrap();
    let mut first = start(directory.path());
    assert_eq!(status(directory.path())["privacy_stopped"], false);
    ipc::request(directory.path(), IpcCommand::PrivacyStop).unwrap();
    assert_eq!(status(directory.path())["privacy_stopped"], true);
    first.0.kill().unwrap();
    first.0.wait().unwrap();
    // Stale socket from SIGKILL is reconciled only if it is owned by this user.
    let _second = start(directory.path());
    let state = status(directory.path());
    assert_eq!(state["privacy_stopped"], true);
    assert_eq!(state["voice_active"], false);
    ipc::request(directory.path(), IpcCommand::Rearm).unwrap();
    assert_eq!(status(directory.path())["privacy_stopped"], false);
}

/// With the real binary: a crash or an update that nobody asked to
/// stop comes back working, and the recording it cut stays interrupted with
/// its fragments, not resumed.
#[test]
fn a_restart_nobody_stopped_comes_back_working() -> anyhow::Result<()> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use orbit_core::{CapturePhase, Recording, State, protocol::MeetingAudio};
    use orbit_store::Store;

    let directory = tempfile::Builder::new()
        .prefix("orbit-restart-")
        .tempdir_in("/tmp")?;
    let path = directory.path();
    let mut first = start(path);
    assert_eq!(status(path)["privacy_stopped"], false);
    first.0.kill()?;
    first.0.wait()?;
    // The last snapshot says a recording was running when the process died.
    let key: [u8; 32] = STANDARD
        .decode(std::fs::read(path.join("development-storage.key"))?)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid fixture key"))?;
    let mut store = Store::open(path.join("store"), key)?;
    let mut snapshot = store
        .load_snapshot::<State>()?
        .ok_or_else(|| anyhow::anyhow!("the first daemon left no snapshot"))?;
    snapshot.durable.recording = Some(Recording {
        id: "crashed-recording".into(),
        title: "Reunión sintética".into(),
        started_at_ms: 1_000,
        ended_at_ms: None,
        expires_at_ms: i64::MAX / 2,
        application_pid: None,
        audio: MeetingAudio::MicrophoneOnly,
        phase: CapturePhase::Recording,
        sealed_chunks: 3,
        gaps: 0,
        tracks: Vec::new(),
        finalizing_since_ms: None,
    });
    store.save_snapshot(&snapshot)?;
    drop(store);

    let _second = start(path);
    let state = status(path);
    assert_eq!(state["privacy_stopped"], false, "{state}");
    assert_eq!(state["recording"]["phase"], "interrupted", "{state}");
    assert_eq!(state["recording"]["sealed_chunks"], 3, "{state}");
    assert!(
        !state["notice"]
            .as_str()
            .unwrap_or_default()
            .contains("Rearma"),
        "{state}"
    );
    Ok(())
}

#[test]
fn real_restart_closes_a_user_stopped_empty_recording_before_ready() -> anyhow::Result<()> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use orbit_core::{
        CapturePhase, Connection, Event, PermissionState, Recording, State, decide,
        protocol::MeetingAudio,
    };
    use orbit_store::Store;

    let directory = tempfile::Builder::new()
        .prefix("orbit-stopped-")
        .tempdir_in("/tmp")?;
    let path = directory.path();
    let mut first = start(path);
    ipc::request(path, IpcCommand::PrivacyStop)?;
    first.0.kill()?;
    first.0.wait()?;

    let key: [u8; 32] = STANDARD
        .decode(std::fs::read(path.join("development-storage.key"))?)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid fixture key"))?;
    let mut store = Store::open(path.join("store"), key)?;
    let mut snapshot = store
        .load_snapshot::<State>()?
        .ok_or_else(|| anyhow::anyhow!("the first daemon left no snapshot"))?;
    assert!(snapshot.privacy_stopped(), "the fixture lost Alina's stop");
    snapshot.durable.recording = Some(Recording {
        id: "user-interrupted-empty".into(),
        title: "Reunión de Alina".into(),
        started_at_ms: 1_000,
        ended_at_ms: None,
        expires_at_ms: i64::MAX / 2,
        application_pid: None,
        audio: MeetingAudio::MicrophoneOnly,
        phase: CapturePhase::Interrupted,
        sealed_chunks: 0,
        gaps: 1,
        tracks: Vec::new(),
        finalizing_since_ms: None,
    });
    store.save_snapshot(&snapshot)?;
    drop(store);

    // start waits for recovery.state == ready, not just a responding socket.
    let _second = start(path);
    let state = status(path);
    assert_eq!(state["privacy_stopped"], true, "{state}");
    assert!(state["recording"]["ended_at_ms"].is_i64(), "{state}");
    let store = Store::open(path.join("store"), key)?;
    let closure = store
        .get_job("finalize-user-interrupted-empty")?
        .ok_or_else(|| anyhow::anyhow!("durable closure absent after ready"))?;
    assert_eq!(closure.payload["interruption"]["cause"], "cause_not_saved");

    // The test supplies the same explicit local facts a consented Mac would
    // supply after rearm; no native permission or provider is exercised here.
    let admission = State::from(
        store
            .load_snapshot::<orbit_core::DurableState>()?
            .ok_or_else(|| anyhow::anyhow!("recovered durable state absent"))?,
    );
    let rearmed = decide(&admission, Event::LocalRearm, 1_999)?.state;
    let online = decide(&rearmed, Event::Connect(Connection::Online), 1_999)?.state;
    let ready = decide(&online, Event::HelperReady, 1_999)?.state;
    let mut admission = decide(
        &ready,
        Event::PermissionObserved {
            microphone: PermissionState::Granted,
            screen: PermissionState::Unknown,
            accessibility: PermissionState::Unknown,
            calendar: orbit_core::calendar::CalendarAuthorization::Unknown,
        },
        1_999,
    )?
    .state;
    admission.set_voice_processing_supported(true);
    let call = decide(
        &admission,
        Event::CallStart {
            id: "next-call".into(),
            duration_ms: 60_000,
            origin: orbit_core::CallOrigin::App,
        },
        2_000,
    )?;
    assert!(call.state.durable.call.is_some());
    Ok(())
}

#[test]
fn real_restart_names_a_capture_that_was_live_in_the_snapshot() -> anyhow::Result<()> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use orbit_core::{CapturePhase, Recording, State, protocol::MeetingAudio};
    use orbit_store::Store;

    let directory = tempfile::Builder::new()
        .prefix("orbit-live-")
        .tempdir_in("/tmp")?;
    let path = directory.path();
    let mut first = start(path);
    first.0.kill()?;
    first.0.wait()?;
    let key: [u8; 32] = STANDARD
        .decode(std::fs::read(path.join("development-storage.key"))?)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid fixture key"))?;
    let mut store = Store::open(path.join("store"), key)?;
    let mut snapshot = store
        .load_snapshot::<State>()?
        .ok_or_else(|| anyhow::anyhow!("the first daemon left no snapshot"))?;
    snapshot.durable.recording = Some(Recording {
        id: "live-before-restart".into(),
        title: "Reunión activa".into(),
        started_at_ms: 1_000,
        ended_at_ms: None,
        expires_at_ms: i64::MAX / 2,
        application_pid: None,
        audio: MeetingAudio::MicrophoneOnly,
        phase: CapturePhase::Recording,
        sealed_chunks: 0,
        gaps: 0,
        tracks: Vec::new(),
        finalizing_since_ms: None,
    });
    store.save_snapshot(&snapshot)?;
    drop(store);

    let _second = start(path);
    let state = status(path);
    assert_eq!(state["recording"]["phase"], "interrupted", "{state}");
    assert!(state["recording"]["ended_at_ms"].is_i64(), "{state}");
    let store = Store::open(path.join("store"), key)?;
    let closure = store
        .get_job("finalize-live-before-restart")?
        .ok_or_else(|| anyhow::anyhow!("durable closure absent after ready"))?;
    assert_eq!(closure.payload["interruption"]["cause"], "restart");
    Ok(())
}

#[test]
fn a_second_daemon_cannot_take_over_the_running_instance() {
    let directory = tempfile::Builder::new()
        .prefix("orbit-single-")
        .tempdir_in("/tmp")
        .unwrap();
    let _first = start(directory.path());
    let second = command(directory.path()).arg("daemon").output().unwrap();
    assert!(!second.status.success());
    assert_eq!(status(directory.path())["privacy_stopped"], false);
}

#[test]
fn disconnected_probes_do_not_kill_the_ipc_listener() {
    let directory = tempfile::Builder::new()
        .prefix("orbit-probe-")
        .tempdir_in("/tmp")
        .unwrap();
    let _daemon = start(directory.path());
    for _ in 0..100 {
        let probe =
            std::os::unix::net::UnixStream::connect(directory.path().join("orbit.sock")).unwrap();
        drop(probe);
    }
    assert_eq!(status(directory.path())["privacy_stopped"], false);
}

#[test]
fn real_startup_recovers_a_durable_call_lifecycle_without_restoring_authority() -> anyhow::Result<()>
{
    use base64::{Engine, engine::general_purpose::STANDARD};
    use orbit_core::{CallPhase, CallState, State};
    use orbit_store::{ChunkMetadata, Job, JobState, Store};
    use serde_json::json;

    let directory = tempfile::Builder::new()
        .prefix("orbit-call-")
        .tempdir_in("/tmp")?;
    let path = directory.path();
    // First install creates only this fixture's development key, never Keychain.
    let mut first = start(path);
    first.0.kill()?;
    first.0.wait()?;
    let key: [u8; 32] = STANDARD
        .decode(std::fs::read(path.join("development-storage.key"))?)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid fixture key"))?;
    let store_path = path.join("store");
    let mut store = Store::open(&store_path, key)?;
    let mut snapshot = store.load_snapshot::<State>()?.unwrap();
    snapshot.durable.call = Some(CallState {
        id: "crashed-call".into(),
        phase: CallPhase::Listening,
        started_at_ms: 10,
        expires_at_ms: 60010,
        turns: 0,
        origin: orbit_core::CallOrigin::App,
    });
    snapshot.durable.active_request = Some("uncertain-do-not-replay".into());
    store.save_snapshot(&snapshot)?;
    store.enqueue_job(&Job {
        id: "recording-crashed-call".into(),
        kind: "recording_manifest".into(),
        state: JobState::Succeeded,
        created_at: 10,
        payload: json!({"recording_id":"crashed-call","recording_kind":"voice_call",
            "started_at_ms":10,"expires_at_ms":60010}),
    })?;
    let original = vec![0_u8; 16000]; // Store fixture, not acoustic/ASR acceptance.
    let receipt = store.seal_chunk(
        &ChunkMetadata {
            id: "crashed-call-mic-0".into(),
            recording_id: "crashed-call".into(),
            capture_epoch_id: None,
            track: "mic_local".into(),
            sequence: 0,
            sample_rate: 8000,
            channels: 1,
            frames: 8000,
            monotonic_ns: 1,
            first_sample: 0,
            started_at_ms: 10,
        },
        &original,
    )?;
    drop(store);
    // This is the actual binary's startup, not a hand-reimplemented recovery call.
    let mut second = start(path);
    let state = status(path);
    // The restart invents no stop; the call's authority is still gone.
    assert_eq!(state["privacy_stopped"], false);
    assert!(state["call"].is_null() && state["active_request"].is_null());
    assert_eq!(state["voice_active"], false);
    assert_eq!(state["connection"], "offline");
    let store = Store::open(&store_path, key)?;
    let closure = store.get_job("finalize-crashed-call")?.unwrap();
    assert_eq!(closure.state, JobState::Queued);
    assert_eq!(
        closure.payload["manifest"]["tracks"][0]["total_frames"],
        8000
    );
    assert_eq!(closure.payload["manifest"]["gaps"], 1);
    assert_eq!(store.read_chunk(&receipt.metadata.id)?, original);
    let boundary = store.get_job("call-recovery-crashed-call")?.unwrap();
    drop(store);
    second.0.kill()?;
    second.0.wait()?;
    let _third = start(path);
    let store = Store::open(&store_path, key)?;
    assert_eq!(
        store.get_job(&closure.id)?.unwrap().payload,
        closure.payload
    );
    assert_eq!(
        store.get_job(&boundary.id)?.unwrap().payload,
        boundary.payload
    );
    assert_eq!(store.read_chunk(&receipt.metadata.id)?, original);
    assert_eq!(status(path)["privacy_stopped"], false);
    Ok(())
}
