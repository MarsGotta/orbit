//! The engine's capture travels where the helper's capture travels.
//!
//! Paging the engine's catalogue closed one hole and left this one in plain
//! sight. When
//! the native helper takes the picture, the PNG goes up out of band
//! (`PUT /v1/captures/<id>`) and the frame carries only a `capture_id`. When
//! the engine takes it, `answer_bridge_from_computer_screenshot` put the whole
//! PNG in base64 inside the reply: `MAX_SOURCE_BYTES` admits 700 000 bytes of
//! PNG, which is about 933 KiB of base64, against a link that admits 131 072.
//! Seven times over, on every full-screen capture.
//!
//! These cases drive the engine path with a PNG at that ceiling and a
//! synthetic hub that records the upload, so the size is the real one rather
//! than the 1x1 image every earlier fixture used.
use super::*;
use crate::daemon::audio::tests::actor::{
    capture_hub, capture_hub_at_capacity, maximal_png, measure_status_rounds, sha256_hex,
};
use crate::hub::MAX_OUTGOING_FRAME_BYTES;
use base64::Engine as _;

/// A Mac with the engine connected and macOS permissions granted, a hub
/// channel the test reads frames off, and a synthetic capture upload hub.
async fn mac_capturing(
    png: Vec<u8>,
    reject_upload: bool,
) -> Result<(
    Mac,
    mpsc::Receiver<Value>,
    mpsc::Receiver<crate::daemon::audio::tests::actor::RecordedPut>,
    std::sync::Arc<tokio::sync::Notify>,
)> {
    let mut h = Mac::with_engine(FakeEngine::with_png(&png)).await?;
    let (address, puts, release) = capture_hub(reject_upload).await?;
    h.runtime.settings.hub_url = Some(format!("http://{address}"));
    h.runtime.token = Some("synthetic device credential".to_owned());
    let (hub, frames) = mpsc::channel(16);
    h.runtime.hub.install(hub);
    Ok((h, frames, puts, release))
}

/// Asks for a window capture and pumps the engine round trips the answer needs.
async fn ask_for_a_capture(h: &mut Mac, request_id: &str) -> Result<()> {
    h.runtime
        .device_request_raw(&json!({
            "request_id": request_id,
            "capability": "screenshot",
            "parameters": {"application": "Fixture"},
        }))
        .await?;
    // The window listing, then the capture of the window it chose.
    h.finish().await?;
    h.finish().await?;
    Ok(())
}

#[tokio::test]
async fn engine_capture_returns_to_actor_while_upload_is_held() -> Result<()> {
    let (mut h, mut frames, mut puts, release) = mac_capturing(maximal_png()?, false).await?;
    h.runtime
        .apply(Event::StartMeeting {
            id: "engine-capture-recording".into(),
            title: "Fixture".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
        })
        .await?;
    let start = loop {
        let command = tokio::time::timeout(Duration::from_secs(5), h._native.recv())
            .await?
            .context("the helper did not receive audio_start")?;
        if command["command"] == "audio_start" {
            break command;
        }
    };
    let stream = start["stream_id"]
        .as_str()
        .context("audio_start without stream_id")?
        .to_owned();
    h.runtime
        .native_event(json!({"event":"response", "id":start["id"], "ok":true,
            "result":{"stream_id":stream}}))
        .await?;
    let idle_status = measure_status_rounds(&mut h.runtime).await?;
    h.runtime
        .device_request_raw(&json!({
            "request_id": "engine-capture",
            "capability": "screenshot",
            "parameters": {"application": "Fixture"},
        }))
        .await?;
    h.finish().await?;
    let started = std::time::Instant::now();
    let mut finishing = Box::pin(h.finish());
    tokio::time::timeout(Duration::from_secs(5), &mut finishing)
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "the engine held the actor {:.3} s while the upload was still held",
                std::time::Instant::now()
                    .duration_since(started)
                    .as_secs_f64()
            )
        })??;
    drop(finishing);
    let put = tokio::time::timeout(Duration::from_secs(5), puts.recv())
        .await?
        .context("the fake hub closed before the PUT")?;
    assert_eq!(put.method, "PUT");
    for sequence in 0..30_u64 {
        h.runtime
            .native_event(json!({
                "event":"audio", "source":"mic_local", "stream_id":stream,
                "sample_rate":8000, "channels":1, "frames":100, "sequence":sequence,
                "monotonic_ns":sequence.saturating_mul(10_000),
                "pcm_s16le_base64":base64::engine::general_purpose::STANDARD.encode(vec![1_u8; 200])
            }))
            .await?;
    }
    let held_status = measure_status_rounds(&mut h.runtime).await?;
    eprintln!(
        "engine_capture_status_us idle_p99={} idle_worst={} held_p99={} held_worst={} n=100",
        idle_status.0, idle_status.1, held_status.0, held_status.1
    );
    assert_eq!(
        h.runtime
            .state
            .durable
            .capture
            .recording
            .as_ref()
            .context("no recording")?
            .gaps,
        0,
        "the engine capture introduced gaps in the helper's 30 frames"
    );
    assert!(
        frames.try_recv().is_err(),
        "Deimos was answered before the PUT was confirmed"
    );
    release.notify_one();
    h.finish().await?;
    let reply = tokio::time::timeout(Duration::from_secs(5), frames.recv())
        .await?
        .context("the upload's reply did not arrive")?;
    assert_eq!(reply["params"]["request_id"], "engine-capture");
    assert!(reply["params"]["result"]["capture_id"].is_string());
    assert!(h.runtime.conversation.bridge_request.is_none());
    h.runtime
        .revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::ComputerRevoked,
        })
        .await;
    Ok(())
}

#[tokio::test]
async fn engine_observation_reports_full_and_retired_hub_after_upload() -> Result<()> {
    for retired in [false, true] {
        let (mut h, _old_frames, mut puts, release) = mac_capturing(maximal_png()?, false).await?;
        let (sender, mut frames) = mpsc::channel(1);
        sender.try_send(json!({"fixture":"occupied"}))?;
        h.runtime.hub.install(sender);
        if retired {
            h.runtime
                .hub
                .retire(crate::daemon::hub_link::Retirement::AuthorityRetired);
        }
        release.notify_one();
        ask_for_a_capture(&mut h, "engine-observation").await?;
        let put = tokio::time::timeout(Duration::from_secs(5), puts.recv())
            .await?
            .context("the capture was not uploaded")?;
        assert_eq!(put.method, "PUT");
        h.finish().await?;
        // Not delivered is said; its cause stays in daemon.log.
        let notice = &h.runtime.state.durable.notice;
        assert_eq!(
            h.runtime.state.notice_code(),
            Some(orbit_core::notice::NoticeCode::BridgeReplyUndelivered),
            "{notice:?}"
        );
        if !retired {
            assert_eq!(frames.try_recv()?["fixture"], "occupied");
            assert!(frames.try_recv().is_err());
        }
    }
    Ok(())
}

/// A capture at the PNG ceiling — the size that never fitted — reaches
/// Deimos, and the frame that announces it carries the identifier and the
/// measurements rather than the image.
#[tokio::test]
async fn a_capture_that_never_fitted_now_arrives_by_the_upload() -> Result<()> {
    let png = maximal_png()?;
    assert_eq!(png.len(), orbit_core::sources::MAX_SOURCE_BYTES);
    let inline = base64::engine::general_purpose::STANDARD.encode(&png);
    assert!(
        inline.len() > MAX_OUTGOING_FRAME_BYTES * 7,
        "the test uses the size that never fitted: {} bytes of base64 against a {} ceiling",
        inline.len(),
        MAX_OUTGOING_FRAME_BYTES
    );

    let (mut h, mut frames, mut puts, release) = mac_capturing(png.clone(), false).await?;

    // The engine finishes while the synthetic hub still holds its PUT. The
    // reply remains absent until that same maximal image is confirmed.
    ask_for_a_capture(&mut h, "capture-1").await?;
    // Hang guard, not an upload budget: 60 s exceeds 2× the 28.1 s worst
    // complete run measured under 16 spinners and 8 fsync writers.
    const PUT_HANG_GUARD: Duration = Duration::from_secs(60);
    let upload_wait_started = std::time::Instant::now();
    let record = tokio::time::timeout(PUT_HANG_GUARD, puts.recv())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "no PUT was observed after {:.1} s (guard: {} s)",
                std::time::Instant::now()
                    .duration_since(upload_wait_started)
                    .as_secs_f64(),
                PUT_HANG_GUARD.as_secs()
            )
        })?
        .context("the fake hub shut down")?;
    assert!(
        frames.try_recv().is_err(),
        "a reply arrived before the upload was confirmed"
    );
    release.notify_one();
    h.finish().await?;

    let frame = tokio::time::timeout(Duration::from_secs(2), frames.recv())
        .await
        .map_err(|_| anyhow::anyhow!("the remote request got no reply"))?
        .context("the hub channel closed")?;
    assert_eq!(frame["method"], "device.reply");
    assert_eq!(frame["params"]["request_id"], "capture-1");
    assert!(
        frame["params"]["refusal"].is_null(),
        "the capture must no longer be refused: {frame}"
    );

    // The frame fits the link that refused it before.
    let wire = serde_json::to_vec(&frame)?;
    assert!(
        wire.len() < MAX_OUTGOING_FRAME_BYTES,
        "the {}-byte frame has to fit the link's ceiling ({} bytes)",
        wire.len(),
        MAX_OUTGOING_FRAME_BYTES
    );

    // The identifier and the measurements travel; the image does not.
    let result = &frame["params"]["result"];
    let capture_id = result["capture_id"]
        .as_str()
        .context("the reply carries no capture_id")?
        .to_owned();
    assert!(
        uuid::Uuid::parse_str(&capture_id).is_ok(),
        "the capture_id has to be a UUID, like the other upload's: {capture_id}"
    );
    assert_eq!(result["application"], "Fixture");
    assert_eq!(result["width"], 2);
    assert_eq!(result["height"], 1);
    assert_eq!(result["mime_type"], "image/png");
    assert_eq!(result["sha256"], sha256_hex(&png));
    assert!(
        result.get("data_base64").is_none(),
        "the engine's reply still carries the bytes inside: {result}"
    );
    assert!(
        !String::from_utf8_lossy(&wire).contains(&inline),
        "the frame contains the whole PNG"
    );

    // The bytes went up the same way the helper's bytes go up.
    assert_eq!(record.method, "PUT");
    assert_eq!(record.path, format!("/v1/captures/{capture_id}"));
    assert_eq!(
        record.header("x-orbit-request"),
        Some("capture-1"),
        "the upload is not tied to the bridge request"
    );
    assert_eq!(
        record.header("x-orbit-sha256"),
        Some(sha256_hex(&png).as_str())
    );
    assert_eq!(record.header("content-type"), Some("image/png"));
    assert_eq!(
        record.header("authorization"),
        Some("Bearer synthetic device credential")
    );
    assert_eq!(record.body, png, "the exact bytes were not uploaded");

    assert!(h.runtime.conversation.bridge_request.is_none());
    h.runtime
        .revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::ComputerRevoked,
        })
        .await;
    Ok(())
}

/// An upload that truly cannot happen is explained. Paging the catalogue had
/// just cured a thirty-second silence; moving the bytes must not open a new
/// door to it.
#[tokio::test]
async fn an_upload_that_fails_is_refused_by_name_and_not_by_silence() -> Result<()> {
    let png = maximal_png()?;
    let (mut h, mut frames, mut puts, _release) = mac_capturing(png, true).await?;

    // The hub gives a bridge request 30 s. The refusal has to be back long
    // before that, or the caller learns nothing but "no answer".
    tokio::time::timeout(
        Duration::from_secs(10),
        ask_for_a_capture(&mut h, "capture-2"),
    )
    .await
    .map_err(|_| anyhow::anyhow!("the daemon hung on the upload"))??;

    let record = tokio::time::timeout(Duration::from_secs(2), puts.recv())
        .await
        .map_err(|_| anyhow::anyhow!("the engine did not try to upload the capture"))?
        .context("the fake hub shut down")?;
    assert!(record.path.starts_with("/v1/captures/"));

    h.finish().await?;

    let frame = tokio::time::timeout(Duration::from_secs(2), frames.recv())
        .await
        .map_err(|_| anyhow::anyhow!("the remote request was left in silence"))?
        .context("the hub channel closed")?;
    assert_eq!(frame["method"], "device.reply");
    assert_eq!(frame["params"]["request_id"], "capture-2");
    assert_eq!(frame["params"]["refusal"]["code"], "upstream_failed");
    assert_eq!(frame["params"]["refusal"]["service"], "hub");
    assert!(frame["params"]["result"].is_null());
    assert!(h.runtime.conversation.bridge_request.is_none());
    h.runtime
        .revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::ComputerRevoked,
        })
        .await;
    Ok(())
}

#[tokio::test]
async fn engine_capture_capacity_reaches_device_reply_and_clears_pending_upload() -> Result<()> {
    let mut h = Mac::with_engine(FakeEngine::with_png(&maximal_png()?)).await?;
    let (address, mut puts) = capture_hub_at_capacity().await?;
    h.runtime.settings.hub_url = Some(format!("http://{address}"));
    h.runtime.token = Some("synthetic device credential".to_owned());
    let (hub, mut frames) = mpsc::channel(16);
    h.runtime.hub.install(hub);

    h.runtime
        .device_request_raw(&json!({
            "request_id":"capture-capacity",
            "capability":"screenshot",
            "parameters":{"application":"Fixture"},
        }))
        .await?;
    h.finish().await?;
    h.finish().await?;
    assert!(
        h.runtime.features.sources.bridge_upload.is_some(),
        "the real upload did not stay pending until the hub's answer was known"
    );

    let put = tokio::time::timeout(Duration::from_secs(2), puts.recv())
        .await?
        .context("the engine did not upload the capture")?;
    assert!(put.path.starts_with("/v1/captures/"));
    h.finish().await?;
    let frame = tokio::time::timeout(Duration::from_secs(2), frames.recv())
        .await?
        .context("the engine did not answer the requester")?;
    assert_eq!(frame["method"], "device.reply");
    assert_eq!(frame["params"]["request_id"], "capture-capacity");
    assert_eq!(frame["params"]["refusal"]["code"], "too_many");
    assert_eq!(frame["params"]["refusal"]["resource"], "capture");
    assert_eq!(frame["params"]["refusal"]["count"], 5);
    assert_eq!(frame["params"]["refusal"]["max"], 4);
    assert!(h.runtime.features.sources.bridge_upload.is_none());
    assert!(h.runtime.conversation.bridge_request.is_none());
    h.runtime
        .revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::ComputerRevoked,
        })
        .await;
    Ok(())
}

/// The PNG ceiling stays where it was. Moving the bytes out of band does not
/// raise a bound, so a PNG above `MAX_SOURCE_BYTES` is refused exactly where it
/// was refused before and never reaches the upload.
#[tokio::test]
async fn a_png_above_the_ceiling_is_still_refused_and_never_uploaded() -> Result<()> {
    let mut oversized = maximal_png()?;
    oversized.extend(std::iter::repeat_n(0u8, 1024));
    assert!(oversized.len() > orbit_core::sources::MAX_SOURCE_BYTES);

    let (mut h, _frames, mut puts, _release) = mac_capturing(oversized, false).await?;
    // The completed engine response falls back to a helper window listing.
    // Observe that event before asserting absence; no scheduling delay proves it.
    ask_for_a_capture(&mut h, "capture-3").await?;
    let fallback = tokio::time::timeout(Duration::from_secs(2), h._native.recv())
        .await?
        .context("the refused PNG did not ask the helper for the fallback")?;
    assert_eq!(fallback["command"], "screen_sources");
    assert!(h.runtime.features.sources.bridge_upload.is_none());
    assert!(puts.try_recv().is_err(), "the refused PNG got uploaded");
    h.runtime
        .revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::ComputerRevoked,
        })
        .await;
    Ok(())
}
