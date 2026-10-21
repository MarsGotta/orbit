//! DEI-15: the engine's capture travels where the helper's capture travels.
//!
//! DEI-14 closed the catalogue's hole and left this one in plain sight. When
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
    h.runtime.token = Some("credencial sintética del dispositivo".to_owned());
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
async fn ref306_engine_capture_returns_to_actor_while_upload_is_held() -> Result<()> {
    let (mut h, mut frames, mut puts, release) = mac_capturing(maximal_png()?, false).await?;
    h.runtime
        .apply(Event::StartMeeting {
            id: "ref306-engine-recording".into(),
            title: "Fixture".into(),
            audio: orbit_core::protocol::MeetingAudio::MicrophoneOnly,
            duration_ms: 60_000,
        })
        .await?;
    let start = loop {
        let command = tokio::time::timeout(Duration::from_secs(5), h._native.recv())
            .await?
            .context("el helper no recibió audio_start")?;
        if command["command"] == "audio_start" {
            break command;
        }
    };
    let stream = start["stream_id"]
        .as_str()
        .context("audio_start sin stream_id")?
        .to_owned();
    h.runtime
        .native_event(json!({"event":"response", "id":start["id"], "ok":true,
            "result":{"stream_id":stream}}))
        .await?;
    let idle_status = measure_status_rounds(&mut h.runtime).await?;
    h.runtime
        .device_request_raw(&json!({
            "request_id": "ref306-engine",
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
                "el motor retuvo el actor {:.3} s mientras la subida seguía retenida",
                std::time::Instant::now()
                    .duration_since(started)
                    .as_secs_f64()
            )
        })??;
    drop(finishing);
    let put = tokio::time::timeout(Duration::from_secs(5), puts.recv())
        .await?
        .context("el hub falso cerró antes del PUT")?;
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
        "ref306_engine_status_us idle_p99={} idle_worst={} held_p99={} held_worst={} n=100",
        idle_status.0, idle_status.1, held_status.0, held_status.1
    );
    assert_eq!(
        h.runtime
            .state
            .durable
            .recording
            .as_ref()
            .context("sin grabación")?
            .gaps,
        0,
        "la captura del motor introdujo huecos en las 30 tramas del helper"
    );
    assert!(
        frames.try_recv().is_err(),
        "se respondió a Deimos antes de confirmar el PUT"
    );
    release.notify_one();
    h.finish().await?;
    let reply = tokio::time::timeout(Duration::from_secs(5), frames.recv())
        .await?
        .context("no llegó la respuesta de la subida")?;
    assert_eq!(reply["params"]["request_id"], "ref306-engine");
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
async fn ref303_a2_engine_observation_reports_full_and_retired_hub_after_upload() -> Result<()> {
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
            .context("la captura no se subió")?;
        assert_eq!(put.method, "PUT");
        h.finish().await?;
        // SIM-21: not delivered is said; its cause stays in daemon.log.
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

/// A1/A3: a capture at the PNG ceiling — the size that never fitted — reaches
/// Deimos, and the frame that announces it carries the identifier and the
/// measurements rather than the image.
#[tokio::test]
async fn dei15_a1_a_capture_that_never_fitted_now_arrives_by_the_upload() -> Result<()> {
    let png = maximal_png()?;
    assert_eq!(png.len(), orbit_core::sources::MAX_SOURCE_BYTES);
    let inline = base64::engine::general_purpose::STANDARD.encode(&png);
    assert!(
        inline.len() > MAX_OUTGOING_FRAME_BYTES * 7,
        "la prueba usa el tamaño que nunca cupo: {} bytes en base64 contra {} de techo",
        inline.len(),
        MAX_OUTGOING_FRAME_BYTES
    );

    let (mut h, mut frames, mut puts, release) = mac_capturing(png.clone(), false).await?;

    // The engine finishes while the synthetic hub still holds its PUT. The
    // reply remains absent until that same maximal image is confirmed.
    ask_for_a_capture(&mut h, "captura-1").await?;
    // Hang guard, not an upload budget: 60 s exceeds 2× the 28.1 s worst
    // complete run measured in REF-28B under 16 spinners and 8 fsync writers.
    const PUT_HANG_GUARD: Duration = Duration::from_secs(60);
    let upload_wait_started = std::time::Instant::now();
    let record = tokio::time::timeout(PUT_HANG_GUARD, puts.recv())
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "A1: no se observó el PUT tras {:.1} s (guarda: {} s)",
                std::time::Instant::now()
                    .duration_since(upload_wait_started)
                    .as_secs_f64(),
                PUT_HANG_GUARD.as_secs()
            )
        })?
        .context("el hub falso se apagó")?;
    assert!(
        frames.try_recv().is_err(),
        "llegó respuesta antes de confirmar la subida"
    );
    release.notify_one();
    h.finish().await?;

    let frame = tokio::time::timeout(Duration::from_secs(2), frames.recv())
        .await
        .map_err(|_| anyhow::anyhow!("A1: la petición remota quedó sin respuesta"))?
        .context("el canal del hub se cerró")?;
    assert_eq!(frame["method"], "device.reply");
    assert_eq!(frame["params"]["request_id"], "captura-1");
    assert!(
        frame["params"]["refusal"].is_null(),
        "A1: la captura ya no se rechaza: {frame}"
    );

    // A1: the frame fits the link that refused it before.
    let wire = serde_json::to_vec(&frame)?;
    assert!(
        wire.len() < MAX_OUTGOING_FRAME_BYTES,
        "A1: la trama de {} bytes tiene que caber en el techo del enlace ({} bytes)",
        wire.len(),
        MAX_OUTGOING_FRAME_BYTES
    );

    // A3: the identifier and the measurements travel; the image does not.
    let result = &frame["params"]["result"];
    let capture_id = result["capture_id"]
        .as_str()
        .context("A3: la respuesta no lleva capture_id")?
        .to_owned();
    assert!(
        uuid::Uuid::parse_str(&capture_id).is_ok(),
        "A3: el capture_id tiene que ser un UUID, como el de la otra subida: {capture_id}"
    );
    assert_eq!(result["application"], "Fixture");
    assert_eq!(result["width"], 2);
    assert_eq!(result["height"], 1);
    assert_eq!(result["mime_type"], "image/png");
    assert_eq!(result["sha256"], sha256_hex(&png));
    assert!(
        result.get("data_base64").is_none(),
        "A3: la respuesta del motor sigue llevando los bytes dentro: {result}"
    );
    assert!(
        !String::from_utf8_lossy(&wire).contains(&inline),
        "A3: la trama contiene el PNG entero"
    );

    // A2: the bytes went up the same way the helper's bytes go up.
    assert_eq!(record.method, "PUT");
    assert_eq!(record.path, format!("/v1/captures/{capture_id}"));
    assert_eq!(
        record.header("x-orbit-request"),
        Some("captura-1"),
        "A2: la subida no va atada a la petición del puente"
    );
    assert_eq!(
        record.header("x-orbit-sha256"),
        Some(sha256_hex(&png).as_str())
    );
    assert_eq!(record.header("content-type"), Some("image/png"));
    assert_eq!(
        record.header("authorization"),
        Some("Bearer credencial sintética del dispositivo")
    );
    assert_eq!(record.body, png, "A2: no se subieron los bytes exactos");

    assert!(h.runtime.conversation.bridge_request.is_none());
    h.runtime
        .revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::ComputerRevoked,
        })
        .await;
    Ok(())
}

/// A5: an upload that truly cannot happen is explained. DEI-14 has just
/// finished curing the thirty-second silence; moving the bytes must not open a
/// new door to it.
#[tokio::test]
async fn dei15_a5_an_upload_that_fails_is_refused_by_name_and_not_by_silence() -> Result<()> {
    let png = maximal_png()?;
    let (mut h, mut frames, mut puts, _release) = mac_capturing(png, true).await?;

    // The hub gives a bridge request 30 s. The refusal has to be back long
    // before that, or the caller learns nothing but "no answer".
    tokio::time::timeout(
        Duration::from_secs(10),
        ask_for_a_capture(&mut h, "captura-2"),
    )
    .await
    .map_err(|_| anyhow::anyhow!("A5: el daemon se quedó colgado en la subida"))??;

    let record = tokio::time::timeout(Duration::from_secs(2), puts.recv())
        .await
        .map_err(|_| anyhow::anyhow!("A5: el motor no intentó subir la captura"))?
        .context("el hub falso se apagó")?;
    assert!(record.path.starts_with("/v1/captures/"));

    h.finish().await?;

    let frame = tokio::time::timeout(Duration::from_secs(2), frames.recv())
        .await
        .map_err(|_| anyhow::anyhow!("A5: la petición remota quedó en silencio"))?
        .context("el canal del hub se cerró")?;
    assert_eq!(frame["method"], "device.reply");
    assert_eq!(frame["params"]["request_id"], "captura-2");
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
    h.runtime.token = Some("credencial sintética del dispositivo".to_owned());
    let (hub, mut frames) = mpsc::channel(16);
    h.runtime.hub.install(hub);

    h.runtime
        .device_request_raw(&json!({
            "request_id":"captura-capacity",
            "capability":"screenshot",
            "parameters":{"application":"Fixture"},
        }))
        .await?;
    h.finish().await?;
    h.finish().await?;
    assert!(
        h.runtime.features.sources.bridge_upload.is_some(),
        "la subida real no quedó pendiente hasta conocer la respuesta del hub"
    );

    let put = tokio::time::timeout(Duration::from_secs(2), puts.recv())
        .await?
        .context("el motor no subió la captura")?;
    assert!(put.path.starts_with("/v1/captures/"));
    h.finish().await?;
    let frame = tokio::time::timeout(Duration::from_secs(2), frames.recv())
        .await?
        .context("el motor no respondió al solicitante")?;
    assert_eq!(frame["method"], "device.reply");
    assert_eq!(frame["params"]["request_id"], "captura-capacity");
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

/// A6: the PNG ceiling stays where it was. This round moves bytes; it does not
/// raise a bound, so a PNG above `MAX_SOURCE_BYTES` is refused exactly where it
/// was refused before and never reaches the upload.
#[tokio::test]
async fn dei15_a6_a_png_above_the_ceiling_is_still_refused_and_never_uploaded() -> Result<()> {
    let mut oversized = maximal_png()?;
    oversized.extend(std::iter::repeat_n(0u8, 1024));
    assert!(oversized.len() > orbit_core::sources::MAX_SOURCE_BYTES);

    let (mut h, _frames, mut puts, _release) = mac_capturing(oversized, false).await?;
    // The completed engine response falls back to a helper window listing.
    // Observe that event before asserting absence; no scheduling delay proves it.
    ask_for_a_capture(&mut h, "captura-3").await?;
    let fallback = tokio::time::timeout(Duration::from_secs(2), h._native.recv())
        .await?
        .context("el PNG rechazado no pidió el fallback al helper")?;
    assert_eq!(fallback["command"], "screen_sources");
    assert!(h.runtime.features.sources.bridge_upload.is_none());
    assert!(puts.try_recv().is_err(), "el PNG rechazado llegó a subirse");
    h.runtime
        .revoke_computer(&orbit_core::refusal::Refusal::ReadInterrupted {
            cause: orbit_core::refusal::ReadInterruption::ComputerRevoked,
        })
        .await;
    Ok(())
}
