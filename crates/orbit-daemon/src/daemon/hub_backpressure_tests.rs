//! A saturated hub command channel must fail the actor's send immediately
//! while native audio keeps flowing. Synthetic fixtures only; the channel that
//! stays full for the whole test stands in for a stalled hub connection, and
//! the refused frame is never queued or replayed.
use super::*;
use crate::daemon::audio::tests::{fixture, settle_seal_receipts};
use crate::daemon::turn::said;
use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::notice::NoticeCode;

/// A send's cause is no longer a sentence in the notice; it is
/// logged in `daemon.log` under the turn's op. The fixture logs there.
fn logging(runtime: &mut Runtime, directory: &std::path::Path) {
    runtime.diagnostics = Some(crate::diagnostics::DiagnosticLog::in_root(directory));
}

/// The last `daemon.log` line of `event`, as written.
fn last_logged(directory: &std::path::Path, event: &str) -> Result<String> {
    let log = std::fs::read_to_string(directory.join("logs/daemon.log"))?;
    log.lines()
        .rev()
        .find(|line| line.contains(&format!("\"event\":\"{event}\"")))
        .map(str::to_owned)
        .context("event absent from daemon.log")
}

fn assert_undelivered_cause(directory: &std::path::Path, expected: &str) -> Result<()> {
    let record: Value = serde_json::from_str(&last_logged(directory, "bridge_reply_undelivered")?)?;
    assert!(
        record["fields"]["cause"]
            .as_str()
            .is_some_and(|cause| cause.contains(expected)),
        "{record}"
    );
    Ok(())
}

#[tokio::test]
async fn privacy_stop_keeps_its_notice_with_a_pending_bridge_request() -> Result<()> {
    use crate::daemon::bridge::BridgeAsk;
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    logging(&mut runtime, directory.path());
    runtime.assistant.bridge_requests.put(BridgeAsk {
        agent: None,
        for_attachment: false,
        id: "pending-during-stop".into(),
        capability: orbit_core::device_bridge::Capability::ScreenSources,
        application: None,
        title: None,
        full_screen: false,
        source_id: None,
        tool_call: None,
        catalogue_after: 0,
        calendar_window: None,
        calendar_cursor: None,
        files: None,
    });
    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;
    assert!(runtime.state.privacy_stopped());
    assert!(runtime.assistant.bridge_requests.is_empty());
    assert!(
        said::has(&runtime.state, NoticeCode::PrivacyStopped),
        "{:?}",
        runtime.state.durable.notice_log
    );
    assert!(said::has(
        &runtime.state,
        NoticeCode::BridgeReplyUndelivered
    ));
    let cause = last_logged(directory.path(), "bridge_reply_undelivered")?;
    assert!(cause.contains("parada de privacidad"), "{cause}");
    assert!(!cause.contains("sin conexión con el hub"), "{cause}");
    Ok(())
}

#[tokio::test]
async fn privacy_stop_queues_refusal_before_retiring_hub() -> Result<()> {
    use crate::daemon::bridge::BridgeAsk;
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    logging(&mut runtime, directory.path());
    let (sender, mut frames) = mpsc::channel(2);
    runtime.link.outbound.install(sender);
    {
        let fixture_value = serde_json::json!(orbit_core::Connection::Online);
        crate::daemon::authority_fixture::set(&mut runtime.state, "connection", fixture_value);
    }
    runtime.assistant.bridge_requests.put(BridgeAsk {
        agent: None,
        for_attachment: false,
        id: "pending-during-stop".into(),
        capability: orbit_core::device_bridge::Capability::ScreenSources,
        application: None,
        title: None,
        full_screen: false,
        source_id: None,
        tool_call: None,
        catalogue_after: 0,
        calendar_window: None,
        calendar_cursor: None,
        files: None,
    });

    runtime
        .privacy_stop(orbit_core::privacy_stop::UserStop::Order)
        .await;

    assert!(runtime.state.privacy_stopped());
    assert!(runtime.link.outbound.is_none());
    let frame = frames.try_recv()?;
    assert_eq!(frame["method"], "device.reply");
    assert_eq!(frame["params"]["request_id"], "pending-during-stop");
    assert_eq!(
        frame["params"]["refusal"]["code"], "privacy_stopped",
        "{frame}"
    );
    assert!(said::has(&runtime.state, NoticeCode::PrivacyStopped));
    assert!(said::has(&runtime.state, NoticeCode::BridgeReplySent));
    let sent = last_logged(directory.path(), "bridge_reply_queued")?;
    assert!(sent.contains("(privacy_stopped)"), "{sent}");
    assert!(frames.try_recv().is_err());
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn privacy_stop_orders_native_cut_before_hub_refusal() -> Result<()> {
    use crate::daemon::bridge::BridgeAsk;
    use futures_util::FutureExt;
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut native_frames) = fixture(directory.path())?;
    let (sender, mut hub_frames) = mpsc::channel(2);
    runtime.link.outbound.install(sender);
    runtime.assistant.bridge_requests.put(BridgeAsk {
        agent: None,
        for_attachment: false,
        id: "cut-before-refusal".into(),
        capability: orbit_core::device_bridge::Capability::ScreenSources,
        application: None,
        title: None,
        full_screen: false,
        source_id: None,
        tool_call: None,
        catalogue_after: 0,
        calendar_window: None,
        calendar_cursor: None,
        files: None,
    });
    for _ in 0..runtime.helper.sender.max_capacity() {
        runtime
            .helper
            .sender
            .try_send(json!({"fixture":"occupied"}))?;
    }

    let stop = runtime.privacy_stop(orbit_core::privacy_stop::UserStop::Order);
    tokio::pin!(stop);
    assert!(stop.as_mut().now_or_never().is_none());
    assert!(
        hub_frames.try_recv().is_err(),
        "the refusal left before the cut order was admitted"
    );
    while native_frames.try_recv().is_ok() {}
    stop.await;

    assert_eq!(native_frames.try_recv()?["command"], "privacy_stop");
    assert_eq!(
        hub_frames.try_recv()?["params"]["request_id"],
        "cut-before-refusal"
    );
    assert!(hub_frames.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn work_delivery_waits_for_capacity_and_handles_closed_receiver() {
    use futures_util::FutureExt;
    let (sender, mut receiver) = mpsc::channel(1);
    assert!(
        sender
            .send(WorkResult::SpeechAvailability {
                ticket: crate::daemon::ticketed::WorkTicket::test(1),
                voice: "default".into(),
                available: true,
            })
            .await
            .is_ok()
    );
    let delivery = deliver_work(
        &sender,
        WorkResult::SpeechAvailability {
            ticket: crate::daemon::ticketed::WorkTicket::test(2),
            voice: "default".into(),
            available: false,
        },
    );
    tokio::pin!(delivery);
    assert!(
        delivery.as_mut().now_or_never().is_none(),
        "a full channel must not lose the result"
    );
    assert!(matches!(
        receiver.recv().await,
        Some(WorkResult::SpeechAvailability { ticket, .. }) if ticket.generation() == 1
    ));
    delivery.await;
    assert!(matches!(
        receiver.recv().await,
        Some(WorkResult::SpeechAvailability { ticket, .. }) if ticket.generation() == 2
    ));
    drop(receiver);
    deliver_work(
        &sender,
        WorkResult::SpeechAvailability {
            ticket: crate::daemon::ticketed::WorkTicket::test(3),
            voice: "default".into(),
            available: false,
        },
    )
    .await;
}

#[tokio::test]
async fn hub_encoding_failure_is_reported_without_queue_admission() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    logging(&mut runtime, directory.path());
    let (sender, mut frames) = mpsc::channel(1);
    runtime.link.outbound.install(sender);
    let result = runtime.send_frame(crate::hub::command(
        "invalid-chat-fixture",
        "chat.send",
        json!({}),
    ));
    assert!(matches!(
        result,
        hub_link::SendResult::EncodingFailed { .. }
    ));
    runtime.note_bridge_send(&result, "the test frame");
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::BridgeReplyUndelivered)
    );

    assert!(frames.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn full_hub_report_names_capacity_and_does_not_queue_reply() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    let (sender, mut frames) = mpsc::channel(1);
    sender.try_send(json!({"fixture":"occupied"}))?;
    runtime.link.outbound.install(sender);
    let Err(error) = runtime.send(json!({"method":"device.reply"})).await else {
        bail!("the full channel should have refused the reply")
    };
    assert!(error.to_string().contains("1"), "{error}");
    assert!(error.to_string().contains("lleno"), "{error}");
    assert_eq!(frames.try_recv()?["fixture"], "occupied");
    assert!(frames.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn action_receipt_is_reported_only_after_queue_admission() -> Result<()> {
    use crate::daemon::bridge::BridgeAsk;
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    logging(&mut runtime, directory.path());
    let (sender, mut frames) = mpsc::channel(1);
    sender.try_send(json!({"fixture":"occupied"}))?;
    runtime.link.outbound.install(sender);
    let ask = BridgeAsk {
        agent: None,
        for_attachment: false,
        id: "remote-action".into(),
        capability: orbit_core::device_bridge::Capability::PerformAction,
        application: None,
        title: None,
        full_screen: false,
        source_id: None,
        tool_call: Some(orbit_core::computer_control::ToolCall {
            tool: "click".into(),
            arguments: json!({}),
        }),
        catalogue_after: 0,
        calendar_window: None,
        calendar_cursor: None,
        files: None,
    };
    runtime.assistant.bridge_requests.put(ask.clone());
    runtime.finish_remote_computer_action("one-attempt", &json!({"ok":true}));
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::BridgeReplyUndelivered),
        "{:?}",
        runtime.state.durable.notice
    );
    let cause = last_logged(directory.path(), "bridge_reply_undelivered")?;
    assert!(cause.contains("no se entregó"), "{cause}");
    assert!(!cause.contains("recibió"), "{cause}");
    assert_undelivered_cause(directory.path(), "capacidad 1")?;
    assert_eq!(frames.try_recv()?["fixture"], "occupied");
    assert!(frames.try_recv().is_err());
    let (sender, mut admitted) = mpsc::channel(1);
    runtime.link.outbound.install(sender);
    runtime.assistant.bridge_requests.put(ask.clone());
    runtime.finish_remote_computer_action("second-attempt", &json!({"ok":true}));
    // Queued, never «received»: there is no receipt from Deimos.
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::BridgeReplySent),
        "{:?}",
        runtime.state.durable.notice
    );
    assert_eq!(
        admitted.try_recv()?["params"]["result"]["receipt"]["operation_id"],
        "second-attempt"
    );
    runtime
        .link
        .outbound
        .retire(crate::daemon::hub_link::Retirement::PrivacyStop);
    runtime.assistant.bridge_requests.put(ask);
    runtime.finish_remote_computer_action("third-attempt", &json!({"ok":true}));
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::BridgeReplyUndelivered),
        "{:?}",
        runtime.state.durable.notice
    );
    let cause = last_logged(directory.path(), "bridge_reply_undelivered")?;
    assert!(cause.contains("parada de privacidad"), "{cause}");
    assert!(cause.contains("no se entregó"), "{cause}");
    assert_undelivered_cause(directory.path(), "parada de privacidad")?;
    Ok(())
}

#[tokio::test]
async fn bridge_release_reports_full_and_retired_link() -> Result<()> {
    use crate::daemon::bridge::{BridgeAsk, BridgeReleaseReason};
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    logging(&mut runtime, directory.path());
    let ask = BridgeAsk {
        agent: None,
        for_attachment: false,
        id: "release-fixture".into(),
        capability: orbit_core::device_bridge::Capability::ScreenSources,
        application: None,
        title: None,
        full_screen: false,
        source_id: None,
        tool_call: None,
        catalogue_after: 0,
        calendar_window: None,
        calendar_cursor: None,
        files: None,
    };
    let (sender, mut frames) = mpsc::channel(1);
    sender.try_send(json!({"fixture":"occupied"}))?;
    runtime.link.outbound.install(sender);
    runtime.assistant.bridge_requests.put(ask.clone());
    runtime.release_bridge_requests(BridgeReleaseReason::Cancelled);
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::BridgeReplyUndelivered)
    );

    assert!(runtime.assistant.bridge_requests.is_empty());
    assert_undelivered_cause(directory.path(), "capacidad 1")?;
    assert_eq!(frames.try_recv()?["fixture"], "occupied");
    assert!(frames.try_recv().is_err());

    runtime
        .link
        .outbound
        .retire(crate::daemon::hub_link::Retirement::AuthorityRetired);
    runtime.assistant.bridge_requests.put(ask);
    runtime.release_bridge_requests(BridgeReleaseReason::Cancelled);

    assert!(runtime.assistant.bridge_requests.is_empty());
    assert_undelivered_cause(directory.path(), "retirada de autoridad")?;
    Ok(())
}

#[tokio::test]
async fn engine_windows_reply_reports_full_and_retired_link() -> Result<()> {
    use crate::daemon::bridge::BridgeAsk;
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    logging(&mut runtime, directory.path());
    let ask = BridgeAsk {
        agent: None,
        for_attachment: false,
        id: "engine-windows".into(),
        capability: orbit_core::device_bridge::Capability::ScreenSources,
        application: None,
        title: None,
        full_screen: false,
        source_id: None,
        tool_call: None,
        catalogue_after: 0,
        calendar_window: None,
        calendar_cursor: None,
        files: None,
    };
    let observation = json!({"structured":{"windows":[]}});
    let (sender, mut frames) = mpsc::channel(1);
    sender.try_send(json!({"fixture":"occupied"}))?;
    runtime.link.outbound.install(sender);
    runtime.assistant.bridge_requests.put(ask.clone());
    assert!(
        runtime
            .answer_bridge_from_computer_windows(&observation)
            .await?
    );
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::BridgeReplyUndelivered)
    );

    assert_undelivered_cause(directory.path(), "capacidad 1")?;
    assert_eq!(frames.try_recv()?["fixture"], "occupied");
    assert!(frames.try_recv().is_err());
    runtime
        .link
        .outbound
        .retire(crate::daemon::hub_link::Retirement::NetworkCancelled);
    runtime.assistant.bridge_requests.put(ask);
    assert!(
        runtime
            .answer_bridge_from_computer_windows(&observation)
            .await?
    );

    assert_undelivered_cause(directory.path(), "cancelación de red")?;
    Ok(())
}

#[tokio::test]
async fn helper_windows_reply_reports_full_and_retired_link() -> Result<()> {
    use crate::daemon::bridge::BridgeAsk;
    let directory = tempfile::tempdir()?;
    let (mut runtime, _) = fixture(directory.path())?;
    logging(&mut runtime, directory.path());
    let ask = BridgeAsk {
        agent: None,
        for_attachment: false,
        id: "helper-windows".into(),
        capability: orbit_core::device_bridge::Capability::ScreenSources,
        application: None,
        title: None,
        full_screen: false,
        source_id: None,
        tool_call: None,
        catalogue_after: 0,
        calendar_window: None,
        calendar_cursor: None,
        files: None,
    };
    let (sender, mut frames) = mpsc::channel(1);
    sender.try_send(json!({"fixture":"occupied"}))?;
    runtime.link.outbound.install(sender);
    runtime.assistant.bridge_requests.put(ask.clone());
    runtime
        .answer_bridge_from_sources(&json!({"id":"source-list-fixture","ok":false,"error":{"code":"source_inventory_failed","message":"No se pudo leer el inventario"}}))
        .await?;
    assert_eq!(
        runtime.state.notice_code(),
        Some(NoticeCode::BridgeReplyUndelivered)
    );

    assert_undelivered_cause(directory.path(), "capacidad 1")?;
    assert_eq!(frames.try_recv()?["fixture"], "occupied");
    assert!(frames.try_recv().is_err());
    runtime
        .link
        .outbound
        .retire(crate::daemon::hub_link::Retirement::AuthorityRetired);
    runtime.assistant.bridge_requests.put(ask);
    runtime
        .answer_bridge_from_sources(&json!({"id":"source-list-fixture","ok":false,"error":{"code":"source_inventory_failed","message":"No se pudo leer el inventario"}}))
        .await?;

    assert_undelivered_cause(directory.path(), "retirada de autoridad")?;
    Ok(())
}

async fn start_meeting(
    runtime: &mut Runtime,
    receiver: &mut mpsc::Receiver<Value>,
) -> Result<String> {
    runtime
        .apply(Event::StartMeeting {
            id: "backpressure-meeting".into(),
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

fn audio_frame(stream: &str, sequence: u64, frames: usize) -> Value {
    json!({"event":"audio", "source":"mic_local", "stream_id":stream,
        "sample_rate":8000, "channels":1, "frames":frames, "sequence":sequence,
        "monotonic_ns":sequence.saturating_mul(10_000),
        "pcm_s16le_base64":STANDARD.encode(vec![1; frames * 2])})
}

#[tokio::test(start_paused = true)]
async fn full_command_channel_fails_the_send_immediately_and_audio_keeps_processing() -> Result<()>
{
    let directory = tempfile::tempdir()?;
    let (mut runtime, mut receiver) = fixture(directory.path())?;
    let stream = start_meeting(&mut runtime, &mut receiver).await?;
    let (hub, hub_commands) = mpsc::channel(16);
    for _ in 0..16 {
        hub.try_send(json!({"fixture":"saturated"})).ok();
    }
    runtime.link.outbound.install(hub);
    let notices_before = runtime.state.durable.notice_log.records().len();
    let sent = tokio::time::timeout(
        Duration::from_secs(1),
        runtime.send(hub::command(
            "backpressure-send",
            "chat.send",
            json!({"text":"synthetic","session_id":Value::Null}),
        )?),
    )
    .await
    .context("with the command channel full, send must fail immediately, not block the actor")?;
    let Err(_refused) = sent else {
        bail!("with the command channel full, send must return an error")
    };
    assert!(
        runtime.state.durable.notice_log.records().len() > notices_before
            && runtime.state.notice_code() == Some(NoticeCode::HubSendFailed),
        "the refused turn must be marked unsent with a notice: {:?}",
        runtime.state.durable.notice
    );
    // The actor keeps processing native audio while the queue stays full.
    runtime.native_event(audio_frame(&stream, 0, 100)).await?;
    runtime.apply(Event::FinishMeeting).await?;
    let stop = receiver.recv().await.context("stop command absent")?;
    assert_eq!(stop["command"], "audio_stop");
    runtime.native_event(audio_frame(&stream, 1, 100)).await?;
    runtime
        .native_event(json!({"event":"response", "id":stop["id"], "ok":true,
        "result":{"stream_id":stream,"drain_complete":true}}))
        .await?;
    settle_seal_receipts(&mut runtime).await?;
    let chunks = runtime
        .fixture_store
        .list_chunks("backpressure-meeting", None, 10)?;
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].metadata.frames, 200);
    drop(hub_commands); // the receiver stays open through every assertion above
    Ok(())
}
