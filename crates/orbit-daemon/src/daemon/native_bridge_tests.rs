//! The native bridge at the daemon actor: a returning helper restores the
//! native capability and the recording its loss cut goes on, and
//! `native_send` carries the 500 ms
//! deadline whose expiry is classified with the core taxonomy:
//! non-critical commands degrade with Backpressure, and an
//! undelivered stop cuts or is said, never latches. Synthetic channels only;
//! no helper process is contacted.
use super::*;
use crate::daemon::audio::tests::fixture;
use crate::daemon::turn::said;
use orbit_core::notice::NoticeCode;

#[tokio::test]
async fn stopped_helper_ready_delivers_stop_when_snapshot_writer_fails() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut commands) = fixture(directory.path())?;
    runtime.state = orbit_core::decide(&runtime.state, Event::PrivacyStop, 1)?.state;
    runtime.state = orbit_core::decide(&runtime.state, Event::HelperLost, 2)?.state;
    assert!(runtime.state.privacy_stopped());
    assert!(!runtime.state.capabilities().native_helper());
    runtime.store_actor.snapshot(&runtime.state).await?;
    let database = rusqlite::Connection::open(runtime.paths.store.join("orbit.sqlite3"))?;
    database.execute_batch(
        "CREATE TRIGGER fail_ready_snapshot BEFORE UPDATE ON settings
         WHEN NEW.key='snapshot' BEGIN
           SELECT RAISE(ABORT, 'synthetic ready snapshot failure');
         END;",
    )?;
    let result = runtime
        .native_event(json!({"event":"ready","capabilities":["local_privacy_stop"]}))
        .await;
    let mut sent = Vec::new();
    while let Ok(command) = commands.try_recv() {
        sent.push(command["command"].as_str().unwrap_or_default().to_owned());
    }
    assert!(
        sent.contains(&"privacy_stop".to_owned()),
        "returning helper missed the user's privacy_stop: {sent:?}; event: {result:?}"
    );
    result?;
    assert!(runtime.state.capabilities().native_helper());
    Ok(())
}

#[tokio::test]
async fn hello_response_from_bridge_does_not_create_a_permissions_notice() -> Result<()> {
    use crate::ipc::read_frame;
    use tokio::{
        io::{AsyncWriteExt, BufReader},
        net::unix::pipe,
    };

    let current: Value = serde_json::from_str(include_str!(
        "../../../../contracts/fixtures/native/results/hello.json"
    ))?;
    for (version, result) in [("current", current["result"].clone())] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, _commands) = fixture(directory.path())?;
        said::trace(&mut runtime, directory.path());
        let (mut helper_output, bridge_input) = pipe::pipe()?;
        let (bridge_output, helper_input) = pipe::pipe()?;
        let (send, commands) = mpsc::channel(4);
        let (events, mut received) = mpsc::channel(4);
        let bridge = tokio::spawn(crate::native::bridge(
            BufReader::new(bridge_input),
            bridge_output,
            commands,
            events,
        ));
        let mut helper_input = BufReader::new(helper_input);
        let hello: Value = serde_json::from_slice(&read_frame(&mut helper_input).await?)?;
        assert_eq!(hello["command"], "hello");
        helper_output
            .write_all(
                format!(
                    "{}\n",
                    json!({
                        "v": 1, "event": "response", "id": hello["id"], "ok": true,
                        "result": result
                    })
                )
                .as_bytes(),
            )
            .await?;
        let response = tokio::time::timeout(Duration::from_secs(2), received.recv())
            .await?
            .context("hello response absent")?;
        let microphone = runtime.state.capabilities().microphone_state();
        runtime.handle_native_event(response).await;
        assert_eq!(
            runtime.state.capabilities().microphone_state(),
            microphone,
            "{version}: a capture-active bool was read as a permission"
        );
        assert!(
            !said::told(&runtime).contains("response permissions field=microphone type=boolean"),
            "{version}: {:?}",
            runtime.state.durable.notice
        );
        assert!(
            !said::told(&runtime).contains("una orden desconocida"),
            "{version}: {:?}",
            runtime.state.durable.notice
        );
        drop(send);
        tokio::time::timeout(Duration::from_secs(2), bridge).await???;
    }
    Ok(())
}

#[tokio::test]
async fn status_and_permissions_use_their_own_result_shapes() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut commands) = fixture(directory.path())?;
    {
        let fixture_value = serde_json::json!(orbit_core::PermissionState::Unknown);
        crate::daemon::authority_fixture::set(
            &mut runtime.state,
            "capabilities.microphone_state",
            fixture_value,
        );
    }
    for name in ["status", "permissions"] {
        runtime.native_send(name, json!({})).await?;
        let command = commands.recv().await.context("native command absent")?;
        assert_eq!(command["command"], name);
        let fixture: Value = serde_json::from_str(match name {
            "status" => include_str!("../../../../contracts/fixtures/native/results/status.json"),
            "permissions" => {
                include_str!("../../../../contracts/fixtures/native/results/permissions.json")
            }
            _ => unreachable!(),
        })?;
        runtime
            .handle_native_event(json!({
                "event":"response", "id":command["id"], "ok":true,
                "result":fixture
            }))
            .await;
        assert!(
            !said::told(&runtime).contains("field=microphone type=boolean"),
            "{name}: {:?}",
            runtime.state.durable.notice
        );
    }
    assert_eq!(
        runtime.state.capabilities().microphone_state(),
        orbit_core::PermissionState::Granted
    );
    Ok(())
}

#[tokio::test]
async fn unknown_response_id_does_not_raise_a_user_notice() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _commands) = fixture(directory.path())?;
    said::trace(&mut runtime, directory.path());
    let original = runtime.state.durable.notice.clone();
    runtime
        .handle_native_event(json!({
            "event":"response", "id":"never-sent", "ok":true,
            "result":{"microphone":false}
        }))
        .await;
    assert_eq!(runtime.state.durable.notice, original);
    Ok(())
}

#[tokio::test]
async fn bridge_owned_response_keeps_its_real_command_name_and_capture_class() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _commands) = fixture(directory.path())?;
    said::trace(&mut runtime, directory.path());
    let hello = json!({"event":"response","id":"bridge-hello",
        "_bridge_command":"hello","ok":true,"result":{"microphone":false}});
    runtime.handle_native_event(hello).await;
    assert!(
        said::told(&runtime).contains("respuesta a hello"),
        "{:?}",
        runtime.state.durable.notice
    );
    assert!(
        said::told(&runtime).contains("field="),
        "the notice must name the cause: {:?}",
        runtime.state.durable.notice
    );
    assert!(
        !said::told(&runtime).contains("una orden desconocida"),
        "{:?}",
        runtime.state.durable.notice
    );
    let stop = json!({"event":"response","id":"bridge-stop",
        "_bridge_command":"privacy_stop","ok":true,"result":{}});
    assert!(runtime.native_event_touches_capture(&stop));
    Ok(())
}

#[tokio::test]
async fn playback_progress_is_consumed_as_diagnostic() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _receiver) = fixture(directory.path())?;
    said::trace(&mut runtime, directory.path());
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory.path()));
    runtime
        .native_event(json!({
            "v": 1,
            "event": "playback_progress",
            "playback_id": "fixture-playback",
            "state": "played",
            "playback_active": false,
            "played_frames": 48
        }))
        .await?;
    let log = std::fs::read_to_string(directory.path().join("logs/daemon.log"))?;
    assert!(log.contains("native_playback_progress"), "{log}");
    assert!(log.contains("played"), "{log}");
    Ok(())
}

async fn start_meeting(
    runtime: &mut Runtime,
    receiver: &mut mpsc::Receiver<Value>,
) -> Result<String> {
    runtime
        .apply(Event::StartMeeting {
            id: "native-bridge-meeting".into(),
            title: "Synthetic fixture".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        })
        .await?;
    let command = receiver.recv().await.context("start command absent")?;
    assert_eq!(command["command"], "audio_start");
    runtime
        .native_event(json!({"event":"response", "id":command["id"], "ok":true,
        "result":{"stream_id":command["stream_id"]}}))
        .await?;
    Ok(command["stream_id"]
        .as_str()
        .context("stream absent")?
        .to_owned())
}

/// A one-slot command channel whose single slot is already occupied stands in
/// for a hung helper: the bridge is not draining anything the daemon sends.
fn stalled_native_channel() -> Result<(mpsc::Sender<Value>, mpsc::Receiver<Value>)> {
    let (native, stalled) = mpsc::channel(1);
    native
        .try_send(json!({"fixture":"stalled-helper"}))
        .map_err(|error| anyhow::anyhow!("fixture channel must accept one value: {error}"))?;
    Ok((native, stalled))
}

/// Losing the helper cuts the live recording with its cause and latches
/// nothing. The relaunched helper's ready restores the native capability,
/// and once its status says the microphone is authorized the same recording
/// goes on by itself: nothing waits for the person to start it again.
#[tokio::test]
async fn a_returning_helper_restores_the_capability_and_resumes_the_cut_recording() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start_meeting(&mut runtime, &mut receiver).await?;
    for sequence in 0..2 {
        runtime
            .handle_native_event(super::meeting_continuity_tests::frame(
                &stream, sequence, 8_000,
            ))
            .await;
    }
    crate::daemon::audio::tests::settle_seal_receipts(&mut runtime).await?;
    runtime
        .handle_native_event(json!({"event":"helper_disconnected"}))
        .await;
    assert!(
        !runtime.state.capabilities().native_helper(),
        "the lost helper must clear the native capability"
    );
    assert!(
        !runtime.state.privacy_stopped(),
        "a lost helper cuts the recording, it does not latch a stop"
    );
    let recording = runtime
        .state
        .durable
        .capture
        .recording
        .as_ref()
        .context("recording absent")?;
    assert_eq!(
        recording.phase,
        CapturePhase::Interrupted,
        "the recording the helper was feeding is cut"
    );
    assert_eq!(recording.ended_at_ms, None, "the cut recording stays open");
    assert!(
        said::told(&runtime).contains("helper"),
        "the cut says why: {:?}",
        runtime.state.durable.notice
    );
    assert!(
        !runtime.audio.audio_is_draining(),
        "with the helper gone nothing is left draining"
    );
    // Without a helper, the upkeep waits for it and reopens nothing.
    runtime
        .capture
        .advance_recovery_clock(std::time::Duration::from_secs(1));
    runtime.run_maintenance().await;
    while let Ok(command) = receiver.try_recv() {
        assert_ne!(command["command"], "audio_start", "no helper to open it");
    }
    // The relaunched helper comes back with its own ready and status.
    runtime
        .native_event(json!({"event":"ready","capabilities":[]}))
        .await?;
    assert!(
        runtime.state.capabilities().native_helper(),
        "the returning helper's ready must restore the native capability"
    );
    loop {
        let command = receiver
            .recv()
            .await
            .context("status command after the ready absent")?;
        if command["command"] == "status" {
            runtime
                .native_event(super::meeting_continuity_tests::granted_status(
                    &command["id"],
                ))
                .await?;
            break;
        }
    }
    assert!(
        runtime.state.capabilities().microphone_state() == orbit_core::PermissionState::Granted,
        "the returning helper's status must re-observe the permissions"
    );
    runtime
        .capture
        .advance_recovery_clock(std::time::Duration::from_secs(30));
    runtime.run_maintenance().await;
    let reopened = loop {
        let command = receiver
            .try_recv()
            .context("the cut recording was not resumed")?;
        if command["command"] == "audio_start" {
            break command;
        }
    };
    let recording = runtime
        .state
        .durable
        .capture
        .recording
        .as_ref()
        .context("recording absent")?;
    assert_eq!(recording.id, "native-bridge-meeting", "the same recording");
    assert_eq!(recording.phase, CapturePhase::Starting);
    assert_ne!(reopened["stream_id"], json!(stream), "a new segment");
    runtime
        .handle_native_event(json!({"event":"response", "id":reopened["id"], "ok":true,
        "result":{"stream_id":reopened["stream_id"]}}))
        .await;
    assert!(said::has(
        &runtime.state,
        NoticeCode::RecordingResumedAfterCut
    ));
    Ok(())
}

/// A non-critical command that cannot reach the helper fails within
/// the 500 ms deadline and is classified as a Backpressure degradation: an
/// additive notice lands and nothing latches.
#[tokio::test(start_paused = true)]
async fn native_send_to_a_stalled_helper_times_out_at_the_deadline_as_backpressure() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _commands) = fixture(directory.path())?;
    said::trace(&mut runtime, directory.path());
    let (native, _stalled) = stalled_native_channel()?;
    runtime.helper.sender = native;
    let notices_before = runtime.state.durable.notice_log.records().len();
    let started = tokio::time::Instant::now();
    let sent = tokio::time::timeout(
        Duration::from_secs(2),
        runtime.native_send("permissions", json!({})),
    )
    .await
    .context("native_send must not wait without a deadline on a stalled helper")?;
    assert!(
        sent.is_err(),
        "a send that cannot reach the helper within the deadline must fail"
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(500) && elapsed < Duration::from_secs(2),
        "the deadline is 500 ms, not {elapsed:?}"
    );
    assert!(
        !runtime.state.privacy_stopped(),
        "a non-critical command is a backpressure degradation, never a latch"
    );
    assert!(
        runtime.state.durable.notice_log.records().len() > notices_before,
        "the degradation must land as an additive notice"
    );
    Ok(())
}

/// A stall guard on the real clock, not a budget. The privacy-stop
/// case below returns as soon as `native_send` does, so this number never
/// decides a passing run. It is only here so that a `native_send` which lost
/// its deadline — and would therefore wait on the stalled helper forever —
/// fails the case by name instead of hanging the suite.
///
/// Measured, not chosen round: over 80 runs under load (48
/// busy-loop processes plus 8 fsync writers on a 32-core box) the call took
/// p50 519.9 ms, p95 1 037.1 ms, p99 1 168.7 ms, worst 1 171.9 ms — the 500 ms
/// deadline plus the stop that followed it then (that stop is now a
/// notice, which only shortens the call). The guard is 40× that worst
/// observation, far outside the distribution it must never arbitrate.
const STALLED_HELPER_STALL_GUARD: Duration = Duration::from_millis(46_876);

/// The user's stop that cannot reach the helper within
/// the deadline is said; the helper remains present and nothing new latches
/// (the user's stop, when it is in force, already is).
///
/// This case deliberately does *not* run on tokio's paused clock,
/// unlike its backpressure sibling above. The stop branch of `native_send`
/// runs `contain`, which may await the store actor's receipt
/// from a real OS thread. Under `start_paused` every task parks during that
/// await, tokio decides the runtime is idle and auto-advances the virtual
/// clock to the next timer — the case's own outer deadline — so under load it
/// failed for the exact opposite of the reason it watches. Nothing here reads
/// a clock, not even the real one: the deadline is proven by the classification
/// `native_send` returns, and the guard below only converts a lost deadline
/// from a hang into a named failure.
#[tokio::test]
async fn native_send_of_a_privacy_stop_to_a_stalled_helper_is_said_and_latches_nothing()
-> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _commands) = fixture(directory.path())?;
    said::trace(&mut runtime, directory.path());
    let (native, _stalled) = stalled_native_channel()?;
    runtime.helper.sender = native;
    let sent = tokio::time::timeout(
        STALLED_HELPER_STALL_GUARD,
        runtime.native_send("privacy_stop", json!({})),
    )
    .await
    .map_err(|_| {
        anyhow::anyhow!(
            "native_send stayed on the stalled helper for {STALLED_HELPER_STALL_GUARD:?} \
             waiting for a privacy_stop it will never confirm: it has lost its 500 ms deadline"
        )
    })?;
    let refusal = sent
        .err()
        .context("a send that cannot reach the helper within the deadline must fail")?
        .to_string();
    // What the case watches is that the refusal came from the
    // deadline expiring, not from some other failure that happened to be
    // quicker. Read off the classification, never off a clock.
    assert!(
        refusal.contains("privacy_stop") && refusal.contains("500 ms"),
        "the refusal must be the expiry of the 500 ms deadline on privacy_stop, not {refusal:?}"
    );
    assert!(
        !runtime.state.privacy_stopped(),
        "an undelivered order is said; only the user's own stop latches"
    );
    assert!(
        runtime.state.capabilities().native_helper(),
        "a full channel is not evidence that the helper is gone"
    );
    assert!(
        said::told(&runtime).contains("500 ms"),
        "{:?}",
        runtime.state.durable.notice
    );
    Ok(())
}

/// A transiently stalled helper stays present. The
/// `capture_stop` which did not enter its full channel is retried by the real
/// maintenance path, closes the microphone when acknowledged, and does not
/// make the following meeting look as if Orbit.app were absent.
#[tokio::test]
async fn a_capture_stop_stalled_once_is_retried_and_the_next_meeting_can_start() -> Result<()> {
    use base64::Engine as _;
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    said::trace(&mut runtime, directory.path());
    let stream = start_meeting(&mut runtime, &mut receiver).await?;
    let (native, _stalled) = stalled_native_channel()?;
    runtime.helper.sender = native;
    tokio::time::timeout(
        STALLED_HELPER_STALL_GUARD,
        runtime.contain(
            orbit_core::privacy_stop::StopCause::CaptureEventFailed,
            "Fallo sintético de la captura.",
        ),
    )
    .await
    .map_err(|_| {
        anyhow::anyhow!("the cut stayed on the stalled helper past its 500 ms deadline")
    })?;
    let recording = runtime
        .state
        .durable
        .capture
        .recording
        .as_ref()
        .context("recording absent")?;
    assert_eq!(recording.phase, CapturePhase::Interrupted);
    assert!(!runtime.state.privacy_stopped());
    assert!(
        runtime.state.capabilities().native_helper(),
        "a send deadline is not evidence that Orbit.app died"
    );
    let frame = json!({"event":"audio","source":"mic_local","stream_id":stream,
        "sample_rate":8_000,"channels":1,"frames":100,"sequence":0,"monotonic_ns":1,
        "pcm_s16le_base64":base64::engine::general_purpose::STANDARD.encode(vec![1_u8; 200])});
    assert!(
        runtime.accepts_audio(&frame),
        "tail audio remains attributable until the retried stop is acknowledged"
    );
    let notice = said::told(&runtime);
    assert!(
        notice.contains("no recibió la orden capture_stop en 500 ms"),
        "{notice}"
    );
    assert!(!notice.contains("no pudo confirmar la parada"), "{notice}");
    assert!(!notice.contains("6 s"), "{notice}");

    let (native, mut recovered) = mpsc::channel(8);
    runtime.helper.sender = native;
    runtime.run_maintenance().await;
    let retry = crate::daemon::audio::tests::take_helper_command(&mut recovered, "capture_stop")?;
    runtime
        .handle_native_event(json!({"event":"response","id":retry["id"],"ok":true,
            "result":{"drain_complete":true}}))
        .await;
    assert!(runtime.audio.mic_stream.is_none());
    assert!(!runtime.audio.audio_is_draining());
    assert!(
        !runtime.accepts_audio(&frame),
        "the acknowledged capture_stop closes the old stream"
    );

    runtime
        .apply(Event::StartMeeting {
            id: "after-transient-stall".into(),
            title: "Synthetic fixture".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        })
        .await?;
    crate::daemon::audio::tests::take_helper_command(&mut recovered, "audio_start")
        .context("the next meeting was refused after the transient stall")?;
    Ok(())
}

/// The bridge emits a synthetic `gap` only when the consumer is
/// behind, and the gap must carry enough identity for the daemon's
/// `accepts_audio` gate to attribute it. With a stream id that matches the
/// live recording the gap is a degradation: the counter moves, the notice
/// is additive, and the helper is not asked to stop.
#[tokio::test]
async fn a_synthetic_backpressure_gap_for_a_live_stream_is_a_degradation() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    said::trace(&mut runtime, directory.path());
    let stream = start_meeting(&mut runtime, &mut receiver).await?;
    let gaps_before = runtime
        .state
        .durable
        .capture
        .recording
        .as_ref()
        .context("recording absent")?
        .gaps;

    let gap = json!({"v":1,"event":"gap","source":"mic_local",
        "stream_id":stream,"reason":"daemon_backpressure","frames":8});
    let decoded: orbit_core::wire::native::NativeEvent = serde_json::from_value(gap.clone())?;
    assert!(matches!(
        decoded,
        orbit_core::wire::native::NativeEvent::Gap {
            v: 1,
            frames: 8,
            ..
        }
    ));
    runtime.native_event(gap).await?;

    let recording = runtime
        .state
        .durable
        .capture
        .recording
        .as_ref()
        .context("recording absent after gap")?;
    assert_eq!(
        recording.gaps,
        gaps_before + 1,
        "a gap for a live stream must increment the counter"
    );
    assert!(
        !runtime.state.privacy_stopped(),
        "a daemon_backpressure gap is a degradation, never a privacy stop"
    );
    assert!(
        said::has(&runtime.state, NoticeCode::AudioGap),
        "the notice must declare the discontinuity: {:?}",
        runtime.state.durable.notice
    );
    let mut privacy_stops = 0;
    while let Ok(command) = receiver.try_recv() {
        if command["command"] == "privacy_stop" {
            privacy_stops += 1;
        }
    }
    assert_eq!(
        privacy_stops, 0,
        "a backpressure gap must not ask the helper to stop"
    );
    Ok(())
}

#[tokio::test]
async fn malformed_audio_bridge_loss_cuts_capture_with_the_field_visible() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    said::trace(&mut runtime, directory.path());
    let stream = start_meeting(&mut runtime, &mut receiver).await?;
    runtime
        .native_event(json!({"v":1,"event":"gap","source":"mic_local",
            "stream_id":stream,
            "reason":"native_audio_invalid event=audio field=sample_rate type=string",
            "frames":1}))
        .await?;
    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .context("recording absent")?
            .gaps,
        1
    );
    runtime
        .native_event(json!({"event":"helper_disconnected",
            "cause":"native_audio_invalid event=audio field=sample_rate type=string"}))
        .await?;
    assert_eq!(
        runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .context("recording absent")?
            .phase,
        CapturePhase::Interrupted
    );
    assert!(
        said::told(&runtime).contains("sample_rate"),
        "{:?}",
        runtime.state.durable.notice
    );
    assert!(
        said::told(&runtime).contains("string"),
        "{:?}",
        runtime.state.durable.notice
    );
    Ok(())
}

/// A route change is an explicit producer boundary, not a new meeting.
/// The helper keeps the original wire rate when the hardware rate changes.
#[tokio::test]
async fn audio_route_recovery_seals_the_tail_and_keeps_the_timeline() -> Result<()> {
    use crate::daemon::audio::tests::settle_seal_receipts;
    use base64::{Engine, engine::general_purpose::STANDARD};

    for rate in [8_000, 48_000] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut commands) = fixture(directory.path())?;
        let stream = start_meeting(&mut runtime, &mut commands).await?;
        let first = json!({"event":"audio","source":"mic_local","stream_id":stream,
            "sample_rate":rate,"channels":1,"frames":rate / 10,"sequence":0,
            "monotonic_ns":1_000_000_000_u64,"pcm_s16le_base64":STANDARD.encode(vec![1; rate / 5])});
        runtime.native_event(first).await?;
        runtime
            .native_event(
                json!({"event":"gap","source":"mic_local","stream_id":stream,
            "reason":"audio_device_changed","frames":0,"duration_ns":2_000_000_000_u64}),
            )
            .await?;
        let resumed = json!({"event":"audio","source":"mic_local","stream_id":stream,
            "sample_rate":rate,"channels":1,"frames":rate,"sequence":0,
            "monotonic_ns":3_100_000_000_u64,"pcm_s16le_base64":STANDARD.encode(vec![2; rate * 2])});
        runtime.native_event(resumed).await?;
        settle_seal_receipts(&mut runtime).await?;

        let recording = runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .context("recording absent")?;
        assert_eq!(recording.id, "native-bridge-meeting");
        assert_eq!(recording.phase, CapturePhase::Recording);
        assert_eq!(recording.gaps, 1);
        assert!(!runtime.state.privacy_stopped());
        let chunks = runtime.fixture_store.list_chunks(&recording.id, None, 10)?;
        assert_eq!(
            chunks.len(),
            2,
            "the tail and resumed audio must not be concatenated"
        );
        let before = &chunks[0].metadata;
        let after = &chunks[1].metadata;
        assert_eq!(
            (before.sequence, before.frames, before.sample_rate),
            (0, (rate / 10) as u64, rate as u32)
        );
        assert_eq!(
            (after.sequence, after.frames, after.sample_rate),
            (1, rate as u64, rate as u32)
        );
        assert_eq!(before.capture_epoch_id, after.capture_epoch_id);
        assert_eq!(after.first_sample, (rate / 10) as u64);
        assert_eq!(
            after.monotonic_ns
                - (before.monotonic_ns
                    + before.frames * 1_000_000_000 / u64::from(before.sample_rate)),
            2_000_000_000
        );
        assert_eq!(
            runtime.fixture_store.read_chunk(&chunks[0].metadata.id)?[44..],
            vec![1; rate / 5]
        );
        assert_eq!(
            runtime.fixture_store.read_chunk(&chunks[1].metadata.id)?[44..],
            vec![2; rate * 2]
        );
        while let Ok(command) = commands.try_recv() {
            assert_ne!(command["command"], "capture_stop");
            assert_ne!(command["command"], "privacy_stop");
        }
    }
    Ok(())
}

#[tokio::test]
async fn audio_route_failure_keeps_the_os_cause_and_status_in_the_notice() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _commands) = fixture(directory.path())?;
    runtime
        .native_event(json!({"event":"error","code":"audio_route_start_failed",
        "os_domain":"NSOSStatusErrorDomain","os_status":-50}))
        .await?;
    let notice = serde_json::to_value(
        runtime
            .state
            .durable
            .notice
            .as_ref()
            .context("notice absent")?,
    )?;
    assert_eq!(notice["code"], "helper_error");
    assert_eq!(notice["data"]["code"], "audio_route_start_failed");
    assert_eq!(notice["data"]["cause"], "nsosstatuserrordomain");
    assert_eq!(notice["data"]["status"], -50);
    Ok(())
}

#[tokio::test]
async fn audio_route_recovery_announces_the_gap_with_known_or_unknown_duration() -> Result<()> {
    for duration_ns in [Some(2_499_999_999_u64), Some(0), None] {
        let directory = tempfile::tempdir()?;
        let (mut runtime, mut commands) = fixture(directory.path())?;
        said::trace(&mut runtime, directory.path());
        let stream = start_meeting(&mut runtime, &mut commands).await?;
        let mut event = json!({"event":"gap","source":"mic_local","stream_id":stream,
            "reason":"audio_device_changed","frames":0});
        if let Some(duration_ns) = duration_ns {
            event["duration_ns"] = json!(duration_ns);
        }
        runtime.native_event(event).await?;
        let notice = runtime
            .state
            .durable
            .notice
            .as_ref()
            .context("notice absent")?;
        let encoded = serde_json::to_value(notice)?;
        assert_eq!(encoded["code"], "audio_route_recovered");
        assert_eq!(notice.code.class(), orbit_core::notice::NoticeClass::Done);
        assert_eq!(
            encoded["data"]["elapsed_ms"],
            json!(duration_ns.map(|ns| ns / 1_000_000))
        );
        assert!(notice.op.is_some());
        if let Some(duration_ns) = duration_ns {
            assert!(said::told(&runtime).contains(&duration_ns.to_string()));
        }
        runtime
            .native_event(
                json!({"event":"gap","source":"mic_local","stream_id":stream,
            "reason":"audio_ring_overflow","frames":8}),
            )
            .await?;
        let visible = runtime
            .state
            .durable
            .notice_log
            .records()
            .iter()
            .filter(|notice| notice.code.name() == "audio_route_recovered")
            .count();
        assert_eq!(visible, 1, "other gaps must keep their silent notice");
    }
    Ok(())
}

#[tokio::test]
async fn audio_route_recovery_during_privacy_drain_never_announces_continuation() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut commands) = fixture(directory.path())?;
    let stream = start_meeting(&mut runtime, &mut commands).await?;
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    runtime
        .native_event(
            json!({"event":"gap","source":"mic_local","stream_id":stream,
        "reason":"audio_device_changed","frames":0,"duration_ns":2_000_000_000_u64}),
        )
        .await?;
    assert!(runtime.state.privacy_stopped());
    assert!(
        runtime
            .state
            .durable
            .notice_log
            .records()
            .iter()
            .all(|notice| notice.code.name() != "audio_route_recovered")
    );
    Ok(())
}

/// The helper refused a meeting's start because its main thread was holding a
/// full backlog: that start did not run. The recording is interrupted as a
/// refused start, with the helper's code; nothing is cut, nothing latches.
#[tokio::test]
async fn a_start_refused_behind_a_held_main_thread_is_a_refused_start_not_a_cut() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut commands) = fixture(directory.path())?;
    runtime
        .apply(Event::StartMeeting {
            id: "meeting-held-main".into(),
            title: "Reunión sintética".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        })
        .await?;
    let audio_start = commands.recv().await.context("start command absent")?;
    assert_eq!(audio_start["command"], "audio_start");
    runtime
        .handle_native_event(json!({"event":"response","id":audio_start["id"],"ok":false,
            "error":{"code":"native_main_thread_busy",
                "message":"El hilo principal de Orbit en este Mac tiene 64 órdenes pendientes",
                "detail":{"pending_commands":64,"max_pending_commands":64}}}))
        .await;
    let recording = runtime
        .state
        .durable
        .capture
        .recording
        .as_ref()
        .context("recording absent")?;
    assert_eq!(recording.phase, orbit_core::CapturePhase::Interrupted);
    let closure = runtime
        .fixture_store
        .get_job("finalize-meeting-held-main")?
        .context("closure absent")?;
    assert_eq!(
        closure.payload["interruption"]["cause"],
        "capture_start_refused"
    );
    assert_eq!(
        closure.payload["interruption"]["detail"],
        "native_main_thread_busy"
    );
    assert!(!runtime.state.privacy_stopped());
    assert!(
        said::mentions(&runtime.state, "native_main_thread_busy"),
        "{:?}",
        runtime.state.durable.notice
    );
    let mut sent = Vec::new();
    while let Ok(command) = commands.try_recv() {
        sent.push(command["command"].as_str().unwrap_or_default().to_owned());
    }
    assert!(
        !sent
            .iter()
            .any(|name| name == "capture_stop" || name == "privacy_stop"),
        "a refused start is not a cut: {sent:?}"
    );
    Ok(())
}
