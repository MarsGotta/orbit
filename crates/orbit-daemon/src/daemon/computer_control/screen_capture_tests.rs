//! A screenshot without false errors (a real call on 2026-10-18,
//! Orbit 0.6.0). Deimos asked for the windows, a window capture and a full
//! capture. The engine's answer to the full capture was larger than an engine
//! message may be; Orbit killed the engine as unhealthy, the helper answered
//! instead, and the engine Orbit started again on the next turn delivered its
//! connection as if it were a window listing: that reached the screenshot
//! still pending on the helper and failed as «Hay una fuente pendiente». The
//! window capture, before all that, took macOS's cursor overlay
//! (CursorUIViewService, untitled, 128×128) as the front window.
//!
//! The engine here is synthetic: it answers over an in-memory transport, with
//! a frame the size of a Retina full screen. Nothing reaches macOS.
use super::*;
use crate::daemon::audio::tests::actor::capture_hub;
use crate::hub::MAX_OUTGOING_FRAME_BYTES;
use base64::Engine as _;
use orbit_core::notice::NoticeClass;
use std::sync::atomic::Ordering;

/// The base64 a Retina full-screen PNG takes: a 3024×1964 desktop is several
/// MiB of PNG, so 6 MiB of base64 is the order of size that call met.
const RETINA_BASE64_BYTES: usize = 6 * 1024 * 1024;

/// macOS's cursor overlay as the engine lists it, in front of everything,
/// and the window the user was looking at behind it.
fn windows_with_the_cursor_overlay_in_front() -> Value {
    json!({"windows": [
        {"window_id": 9, "pid": 401, "app_name": "CursorUIViewService",
         "bundle_id": "com.apple.CursorUIViewService", "title": "", "z_index": 2000},
        {"window_id": 22, "pid": 11, "app_name": "Safari",
         "bundle_id": "com.apple.Safari", "title": "Correo", "z_index": 5},
        {"window_id": 31, "pid": 12, "app_name": "Reproductor",
         "bundle_id": "com.example.player", "title": "", "z_index": 3},
    ]})
}

fn small_png() -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder
            .write_header()?
            .write_image_data(&[0, 0, 0, 255, 255, 255, 255, 255])?;
    }
    Ok(bytes)
}

/// A connected Mac with that engine, a hub channel the test reads frames off,
/// a hub that speaks typed refusals, and a capture hub that takes uploads.
async fn mac(
    windows: Value,
    desktop_file: Option<DesktopFile>,
) -> Result<(
    Mac,
    EngineRecord,
    mpsc::Receiver<Value>,
    mpsc::Receiver<crate::daemon::audio::tests::actor::RecordedPut>,
    Arc<tokio::sync::Notify>,
)> {
    let engine = FakeEngine {
        tools: Value::Array(tools_named(&[
            "list_apps",
            "list_windows",
            "get_window_state",
            "get_desktop_state",
        ])),
        windows,
        png: base64::engine::general_purpose::STANDARD.encode(small_png()?),
        desktop_png: Some("A".repeat(RETINA_BASE64_BYTES)),
        desktop_file,
        window_dimensions: (2, 1),
        desktop_dimensions: (3024, 1964),
        timeout: Duration::from_secs(5),
        ..FakeEngine::default()
    };
    let record = engine.record.clone();
    let mut h = Mac::with_engine(engine).await?;
    let (address, puts, release) = capture_hub(false).await?;
    h.runtime.settings.hub_url = Some(format!("http://{address}"));
    h.runtime.token = Some("synthetic device credential".to_owned());
    let (hub, frames) = mpsc::channel(16);
    h.runtime.hub.install(hub);
    Ok((h, record, frames, puts, release))
}

async fn reply(frames: &mut mpsc::Receiver<Value>, request_id: &str) -> Result<Value> {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(2), frames.recv())
            .await
            .map_err(|_| anyhow::anyhow!("request {request_id} got no reply"))?
            .context("the hub channel closed")?;
        if frame["method"] == "device.reply" && frame["params"]["request_id"] == request_id {
            return Ok(frame);
        }
    }
}

fn failed_notices(h: &Mac) -> Vec<String> {
    h.runtime
        .state
        .durable
        .notice_log
        .records()
        .iter()
        .filter(|notice| notice.class() == NoticeClass::Failed)
        .map(|notice| format!("{notice:?}"))
        .collect()
}

fn engine_failures(h: &Mac) -> usize {
    h.runtime
        .state
        .durable
        .notice_log
        .records()
        .iter()
        .filter(|notice| notice.code == NoticeCode::EngineFailed)
        .count()
}

/// The engine's full-screen answer of a Retina screen, larger
/// than an engine message may be, is not an engine failure and is not a work
/// failure. The engine stays connected, the helper (which cannot capture a
/// screen) is not asked for a window in its place, and Deimos is told the
/// size and the ceiling.
#[tokio::test]
async fn a_retina_full_screen_answer_is_refused_by_its_size_without_failing_the_engine()
-> Result<()> {
    let (mut h, record, mut frames, _puts, _release) =
        mac(windows_with_the_cursor_overlay_in_front(), None).await?;
    h.runtime
        .device_request_raw(&json!({
            "request_id": "full-screen-1",
            "capability": "screenshot",
            "parameters": {"full_screen": true},
        }))
        .await?;
    h.finish().await?;
    let frame = reply(&mut frames, "full-screen-1").await?;

    let frame_bytes = record.full_screen_frame_bytes.load(Ordering::SeqCst);
    assert!(
        frame_bytes > 1_048_576,
        "the test uses an answer larger than the ceiling: {frame_bytes} bytes"
    );
    let refusal = &frame["params"]["refusal"];
    assert_eq!(refusal["code"], "too_large", "{frame}");
    assert_eq!(refusal["resource"], "capture", "{frame}");
    assert_eq!(refusal["bytes"], frame_bytes, "{frame}");
    assert_eq!(refusal["max_bytes"], 1_048_576, "{frame}");
    assert_eq!(
        engine_failures(&h),
        0,
        "{:?}",
        h.runtime.state.durable.notice_log
    );
    assert!(failed_notices(&h).is_empty(), "{:?}", failed_notices(&h));
    assert!(
        h.runtime.engine_is_ready(),
        "a large answer left the engine disconnected"
    );
    assert!(h.runtime.features.computer.engine_failure.is_none());
    assert!(
        h.runtime.state.views.source_request.is_none(),
        "the helper was asked for a window instead of the screen"
    );
    assert_eq!(record.calls_of("get_desktop_state").len(), 1);
    assert!(h.runtime.conversation.bridge_request.is_none());
    Ok(())
}

/// That real call in order — the windows, a window capture and
/// a full capture — leaves no notice of class `failed`. The window capture
/// takes the window the user was looking at, not the cursor overlay in front of it.
#[tokio::test]
async fn the_sequence_of_a_real_call_leaves_no_failed_notice() -> Result<()> {
    let (mut h, record, mut frames, mut puts, release) = mac(
        windows_with_the_cursor_overlay_in_front(),
        Some(DesktopFile::Png(Arc::new(desktop_png(RETINA.0, RETINA.1)?))),
    )
    .await?;

    h.runtime
        .device_request_raw(&json!({"request_id": "windows-1", "capability": "screen_sources"}))
        .await?;
    h.finish().await?;
    let windows = reply(&mut frames, "windows-1").await?;
    assert!(
        windows["params"]["result"]["windows"].is_array(),
        "{windows}"
    );

    h.runtime
        .device_request_raw(&json!({"request_id": "window-1", "capability": "screenshot"}))
        .await?;
    // The listing, then the capture of the window it chose.
    h.finish().await?;
    h.finish().await?;
    tokio::time::timeout(Duration::from_secs(5), puts.recv())
        .await?
        .context("the window capture was not uploaded")?;
    release.notify_one();
    h.finish().await?;
    let window = reply(&mut frames, "window-1").await?;
    assert_eq!(
        window["params"]["result"]["application"], "Safari",
        "{window}"
    );
    assert_eq!(window["params"]["result"]["title"], "Correo", "{window}");
    let captured = record.calls_of("get_window_state");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0]["arguments"]["window_id"], 22);

    h.runtime
        .device_request_raw(&json!({
            "request_id": "full-screen-1",
            "capability": "screenshot",
            "parameters": {"full_screen": true},
        }))
        .await?;
    h.finish().await?;
    let uploaded = tokio::time::timeout(Duration::from_secs(5), puts.recv())
        .await?
        .context("the screen capture was not uploaded")?;
    assert_eq!(png_size(&uploaded.body)?, (1280, 831));
    release.notify_one();
    h.finish().await?;
    let full = reply(&mut frames, "full-screen-1").await?;
    assert!(full["params"]["result"]["capture_id"].is_string(), "{full}");

    assert!(failed_notices(&h).is_empty(), "{:?}", failed_notices(&h));
    assert_eq!(engine_failures(&h), 0);
    assert!(h.runtime.engine_is_ready());
    Ok(())
}

/// The second failure of that call: while the helper still had the screenshot
/// in hand, the engine Orbit had started again connected. Its connection is
/// not a window listing, so it must not reach the request the helper is
/// answering, and nothing fails.
#[tokio::test]
async fn an_engine_that_connects_while_the_helper_captures_fails_nothing() -> Result<()> {
    let mut h = Mac::new(false).await?;
    let service = h
        .runtime
        .features
        .computer
        .service
        .take()
        .context("fixture service")?;
    h.runtime.conversation.bridge_request = Some(crate::daemon::BridgeAsk {
        id: "full-screen-1".into(),
        capability: orbit_core::device_bridge::Capability::Screenshot,
        application: None,
        title: None,
        full_screen: true,
        source_id: None,
        tool_call: None,
        catalogue_after: 0,
        calendar_window: None,
        calendar_cursor: None,
        files: None,
    });
    h.runtime.source_windows().await?;
    assert!(h.runtime.source_operation_pending());

    let generation = h.runtime.state.generation();
    let ticket = h.runtime.begin_computer(
        ComputerStage::Connecting,
        "Conectando al motor CUA local; no se inicia ningún navegador.",
        generation,
        |_| tokio::spawn(std::future::pending()),
    )?;
    // What `connect_computer`'s worker delivers when the engine comes up.
    h.runtime
        .computer_event(&ticket, connected(service))
        .await?;

    assert!(
        h.runtime.source_operation_pending(),
        "the engine's connection touched the capture the helper had in progress"
    );
    assert!(h.runtime.engine_is_ready());
    assert!(failed_notices(&h).is_empty(), "{:?}", failed_notices(&h));
    Ok(())
}

/// The addendum: macOS's cursor overlay is untitled and sits in front of every
/// window. The front window a capture takes is the frontmost titled one; an
/// untitled window is taken only when it is the only one that matches.
#[tokio::test]
async fn the_front_window_is_not_the_cursor_overlay() -> Result<()> {
    let h = Mac::new(false).await?;
    let mut runtime = h.runtime;
    let listing = json!({"structured": windows_with_the_cursor_overlay_in_front()});
    let ask = |application: Option<&str>| crate::daemon::BridgeAsk {
        id: "window-1".into(),
        capability: orbit_core::device_bridge::Capability::Screenshot,
        application: application.map(str::to_owned),
        title: None,
        full_screen: false,
        source_id: None,
        tool_call: None,
        catalogue_after: 0,
        calendar_window: None,
        calendar_cursor: None,
        files: None,
    };
    runtime.conversation.bridge_request = Some(ask(None));
    let front = runtime
        .bridge_screenshot_target(&listing)
        .context("no window chosen")?;
    assert_eq!(
        (front.0, front.1, front.2.as_str()),
        (11, 22, "Safari"),
        "{front:?}"
    );
    runtime.conversation.bridge_request = Some(ask(Some("Reproductor")));
    let only = runtime
        .bridge_screenshot_target(&listing)
        .context("no window chosen")?;
    assert_eq!((only.0, only.1), (12, 31), "{only:?}");
    Ok(())
}

const INSTANCE: &str = "11111111-1111-4111-8111-111111111111";
const INVENTORY: &str = "22222222-2222-4222-8222-222222222222";
const OVERLAY_TOKEN: &str = "33333333-3333-4333-8333-333333333333";
const SAFARI_TOKEN: &str = "55555555-5555-4555-8555-555555555555";

fn helper_row(
    token: &str,
    window_id: u64,
    pid: u64,
    name: &str,
    bundle: &str,
    title: &str,
) -> Value {
    json!({
        "binding": {
            "native_instance_id": INSTANCE,
            "privacy_generation": 0,
            "inventory_id": INVENTORY,
            "selection_token": token,
            "window_id": window_id,
            "application_pid": pid,
            "application_instance_id": "44444444-4444-4444-8444-444444444444",
            "binding_quality": "observed_owner_and_window",
            "descriptor_hash": "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        },
        "window_id": window_id,
        "application_pid": pid,
        "application_name": name,
        "bundle_id": bundle,
        "title": title,
        "title_truncated": false,
        "unavailable_reason": null
    })
}

/// The same rule where the helper answers: its inventory lists the overlay
/// first, and the capture it is asked for is Safari's.
#[tokio::test]
async fn the_helper_does_not_capture_the_cursor_overlay_either() -> Result<()> {
    let mut h = Mac::new(false).await?;
    drop(h.runtime.features.computer.service.take());
    let (hub, _frames) = mpsc::channel(16);
    h.runtime.hub.install(hub);
    h.runtime
        .device_request_raw(&json!({"request_id": "window-1", "capability": "screenshot"}))
        .await?;
    let list = tokio::time::timeout(Duration::from_secs(2), h._native.recv())
        .await?
        .context("the helper did not receive the listing")?;
    assert_eq!(list["command"], "screen_sources", "{list}");
    let generation = h.runtime.state.generation();
    let mut overlay = helper_row(
        OVERLAY_TOKEN,
        9,
        401,
        "CursorUIViewService",
        "com.apple.CursorUIViewService",
        "",
    );
    let mut safari = helper_row(SAFARI_TOKEN, 22, 11, "Safari", "com.apple.Safari", "Correo");
    overlay["binding"]["privacy_generation"] = json!(generation);
    safari["binding"]["privacy_generation"] = json!(generation);
    h.runtime
        .native_event(
            json!({"event":"response","id":list["id"],"ok":true,"result":{
                "native_instance_id": INSTANCE,
                "privacy_generation": generation,
                "inventory_id": INVENTORY,
                "clock_domain": "mach_absolute_time",
                "observed_monotonic_ns": 1_000_000u64,
                "expires_monotonic_ns": 1_000_000u64 + orbit_core::sources::INVENTORY_TTL_NS,
                "selected_text_available": false,
                "windows": [overlay, safari],
                "omitted_count": 0u64
            }}),
        )
        .await?;
    let shot = tokio::time::timeout(Duration::from_secs(2), h._native.recv())
        .await?
        .context("the helper did not receive the capture")?;
    assert_eq!(shot["command"], "screenshot", "{shot}");
    assert_eq!(
        shot["window_binding"]["selection_token"], SAFARI_TOKEN,
        "the helper captured the cursor overlay: {shot}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Part 2: the full screen through a file, fitted, and desktop coordinates.
// ---------------------------------------------------------------------------

/// The width and height of a MacBook display at true pixels.
const RETINA: (u32, u32) = (3024, 1964);

/// A synthetic desktop: smooth bands of colour, which compress like a real
/// screen of windows rather than like noise.
fn desktop_png(width: u32, height: u32) -> Result<Vec<u8>> {
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&[
                (x / 64 % 256) as u8,
                (y / 64 % 256) as u8,
                ((x / 256 + y / 256) % 2 * 200) as u8,
                255,
            ]);
        }
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.write_header()?.write_image_data(&pixels)?;
    }
    Ok(bytes)
}

async fn desktop_mac(
    file: DesktopFile,
) -> Result<(
    Mac,
    EngineRecord,
    mpsc::Receiver<Value>,
    mpsc::Receiver<crate::daemon::audio::tests::actor::RecordedPut>,
    Arc<tokio::sync::Notify>,
)> {
    let engine = FakeEngine {
        tools: Value::Array(tools_named(&[
            "list_apps",
            "list_windows",
            "get_window_state",
            "get_desktop_state",
            "click",
        ])),
        desktop_file: Some(file),
        desktop_dimensions: RETINA,
        timeout: Duration::from_secs(5),
        ..FakeEngine::default()
    };
    let record = engine.record.clone();
    let mut h = Mac::with_engine(engine).await?;
    let (address, puts, release) = capture_hub(false).await?;
    h.runtime.settings.hub_url = Some(format!("http://{address}"));
    h.runtime.token = Some("synthetic device credential".to_owned());
    let (hub, frames) = mpsc::channel(16);
    h.runtime.hub.install(hub);
    Ok((h, record, frames, puts, release))
}

async fn ask_for_the_full_screen(h: &mut Mac, request_id: &str) -> Result<()> {
    h.runtime
        .device_request_raw(&json!({
            "request_id": request_id,
            "capability": "screenshot",
            "parameters": {"full_screen": true},
        }))
        .await?;
    h.finish().await
}

/// The file the engine was told to write, from the call it received.
fn out_file(record: &EngineRecord) -> Result<std::path::PathBuf> {
    let calls = record.calls_of("get_desktop_state");
    let path = calls
        .first()
        .and_then(|call| call["arguments"]["screenshot_out_file"].as_str())
        .context("get_desktop_state without screenshot_out_file")?;
    Ok(std::path::PathBuf::from(path))
}

fn png_size(bytes: &[u8]) -> Result<(u32, u32)> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let reader = decoder.read_info()?;
    Ok((reader.info().width, reader.info().height))
}

/// A synthetic Retina desktop is delivered to Deimos fitted to what a
/// capture can carry, the image never travels in the engine's frame, the
/// result says what its coordinates mean, and the file is gone.
#[tokio::test]
async fn a_retina_desktop_is_delivered_fitted_and_its_file_is_gone() -> Result<()> {
    let png = desktop_png(RETINA.0, RETINA.1)?;
    let (mut h, record, mut frames, mut puts, release) =
        desktop_mac(DesktopFile::Png(Arc::new(png))).await?;
    ask_for_the_full_screen(&mut h, "full-screen-1").await?;
    let put = tokio::time::timeout(Duration::from_secs(10), puts.recv())
        .await?
        .context("the screen was not uploaded")?;
    release.notify_one();
    h.finish().await?;
    let frame = reply(&mut frames, "full-screen-1").await?;

    let path = out_file(&record)?;
    assert!(
        path.starts_with(h._directory.path()),
        "the engine wrote outside Orbit's directory: {}",
        path.display()
    );
    assert!(
        !path.exists(),
        "the capture file is still there: {}",
        path.display()
    );
    let result = &frame["params"]["result"];
    assert_eq!(result["width"], 1280, "{frame}");
    assert_eq!(result["height"], 831, "{frame}");
    assert_eq!(png_size(&put.body)?, (1280, 831));
    assert!(
        put.body.len() <= orbit_core::sources::MAX_SOURCE_BYTES,
        "{} bytes",
        put.body.len()
    );
    let coordinates = result["coordinates"]
        .as_str()
        .context("the reply does not say what its coordinates are")?;
    for number in ["1280", "831", "3024", "1964"] {
        assert!(coordinates.contains(number), "{coordinates}");
    }
    assert!(failed_notices(&h).is_empty(), "{:?}", failed_notices(&h));
    assert_eq!(engine_failures(&h), 0);
    Ok(())
}

/// A desktop click Deimos places on the fitted image lands on the screen
/// pixel under it; before any desktop capture was delivered, a desktop
/// action is refused by name, since its coordinates refer to nothing.
#[tokio::test]
async fn a_desktop_click_on_the_fitted_image_lands_on_the_screen_pixel() -> Result<()> {
    let png = desktop_png(RETINA.0, RETINA.1)?;
    let (mut h, record, mut frames, mut puts, release) =
        desktop_mac(DesktopFile::Png(Arc::new(png))).await?;
    let click = |x: u64, y: u64| ToolCall {
        tool: "click".into(),
        arguments: json!({"target": {"kind": "desktop", "display_id": "primary"}, "x": x, "y": y}),
    };

    let refused = h
        .runtime
        .call_engine_tool(click(640, 415), "fixture")
        .err()
        .context("a desktop click was sent without a delivered capture")?
        .to_string();
    assert_eq!(
        serde_json::from_str::<Value>(&refused)?["expected"]["kind"],
        "desktop_frame"
    );
    assert!(record.calls_of("click").is_empty());

    ask_for_the_full_screen(&mut h, "full-screen-1").await?;
    tokio::time::timeout(Duration::from_secs(10), puts.recv())
        .await?
        .context("the screen was not uploaded")?;
    release.notify_one();
    h.finish().await?;
    reply(&mut frames, "full-screen-1").await?;

    h.runtime.call_engine_tool(click(640, 415), "fixture")?;
    h.finish().await?;
    let clicks = record.calls_of("click");
    assert_eq!(clicks.len(), 1);
    // Image pixel 640 covers screen pixels 1512.0–1514.4 of 3024; its centre
    // is 1513. Row 415 covers 980.8–983.2 of 1964; its centre is 982.
    assert_eq!(clicks[0]["arguments"]["x"], 1513, "{:?}", clicks[0]);
    assert_eq!(clicks[0]["arguments"]["y"], 982, "{:?}", clicks[0]);
    assert_eq!(clicks[0]["arguments"]["target"]["kind"], "desktop");

    let outside = h
        .runtime
        .call_engine_tool(click(1280, 10), "fixture")
        .err()
        .context("a click outside the image was sent")?
        .to_string();
    assert!(
        outside.contains("1280") && outside.contains("831"),
        "{outside}"
    );
    Ok(())
}

/// An oversized or a missing file is refused to Deimos with its numbers,
/// nothing fails on screen, and no file is left behind.
#[tokio::test]
async fn an_oversized_or_missing_file_is_refused_with_its_numbers() -> Result<()> {
    let oversized = orbit_core::sources::MAX_DESKTOP_PNG_BYTES as u64 + 1;
    let (mut h, record, mut frames, _puts, _release) =
        desktop_mac(DesktopFile::Sparse(oversized)).await?;
    ask_for_the_full_screen(&mut h, "full-screen-1").await?;
    let frame = reply(&mut frames, "full-screen-1").await?;
    let refusal = &frame["params"]["refusal"];
    assert_eq!(refusal["code"], "too_large", "{frame}");
    assert_eq!(refusal["resource"], "capture", "{frame}");
    assert_eq!(refusal["bytes"], oversized, "{frame}");
    assert_eq!(
        refusal["max_bytes"],
        orbit_core::sources::MAX_DESKTOP_PNG_BYTES,
        "{frame}"
    );
    assert!(!out_file(&record)?.exists(), "the huge file is still there");
    assert!(failed_notices(&h).is_empty(), "{:?}", failed_notices(&h));

    let (mut h, _record, mut frames, _puts, _release) = desktop_mac(DesktopFile::Nothing).await?;
    ask_for_the_full_screen(&mut h, "full-screen-2").await?;
    let frame = reply(&mut frames, "full-screen-2").await?;
    let refusal = &frame["params"]["refusal"];
    assert_eq!(refusal["code"], "not_found", "{frame}");
    assert_eq!(refusal["resource"], "capture", "{frame}");
    assert!(failed_notices(&h).is_empty(), "{:?}", failed_notices(&h));
    assert!(
        h.runtime.state.views.source_request.is_none(),
        "the helper was asked for a window"
    );
    Ok(())
}

fn window_inventory_reply(list: &Value, generation: u64, application: &str, title: &str) -> Value {
    let mut row = helper_row(
        SAFARI_TOKEN,
        22,
        11,
        application,
        "com.example.browser",
        title,
    );
    row["binding"]["privacy_generation"] = json!(generation);
    json!({"event":"response","id":list["id"],"ok":true,"result":{
        "native_instance_id":INSTANCE,"privacy_generation":generation,"inventory_id":INVENTORY,
        "clock_domain":"mach_absolute_time","observed_monotonic_ns":1_000_000u64,
        "expires_monotonic_ns":1_000_000u64 + orbit_core::sources::INVENTORY_TTL_NS,
        "selected_text_available":true,"windows":[row],"omitted_count":0u64
    }})
}

#[tokio::test]
async fn listed_window_is_selected_by_same_application_and_title() -> Result<()> {
    for (application, title) in [
        ("Dia", "Notes: Lighthouse Project in DEV"),
        ("Chrome", "Lighthouse Project in DEV"),
    ] {
        for capability in ["screenshot", "selected_text"] {
            let mut h = Mac::new(false).await?;
            drop(h.runtime.features.computer.service.take());
            let (hub, mut frames) = mpsc::channel(16);
            h.runtime.hub.install(hub);
            h.runtime
                .device_request_raw(&json!({"request_id":"list","capability":"screen_sources"}))
                .await?;
            let list = h._native.try_recv()?;
            let generation = h.runtime.state.generation();
            h.runtime
                .native_event(window_inventory_reply(
                    &list,
                    generation,
                    application,
                    title,
                ))
                .await?;
            let listed = reply(&mut frames, "list").await?;
            let window = &listed["params"]["result"]["windows"][0];
            assert_eq!(window["application"], application);
            assert_eq!(window["title"], title);
            h.runtime
                .device_request_raw(&json!({"request_id":"read","capability":capability,
                "parameters":{"application":window["application"],"title":window["title"]}}))
                .await?;
            let list = h._native.try_recv()?;
            h.runtime
                .native_event(window_inventory_reply(
                    &list,
                    generation,
                    application,
                    title,
                ))
                .await?;
            let capture = h._native.try_recv()?;
            assert_eq!(capture["command"], capability, "{capture}");
            assert_eq!(capture["window_binding"]["selection_token"], SAFARI_TOKEN);
        }
    }
    Ok(())
}

#[tokio::test]
async fn missing_window_names_filters_and_inventory_counts() -> Result<()> {
    let mut h = Mac::new(false).await?;
    drop(h.runtime.features.computer.service.take());
    let (hub, mut frames) = mpsc::channel(16);
    h.runtime.hub.install(hub);
    h.runtime
        .device_request_raw(&json!({"request_id":"missing","capability":"screenshot",
        "parameters":{"application":"Dia","title":"Lighthouse Project in DEV"}}))
        .await?;
    let list = h._native.try_recv()?;
    let generation = h.runtime.state.generation();
    h.runtime
        .native_event(window_inventory_reply(
            &list, generation, "Safari", "Correo",
        ))
        .await?;
    let refused = reply(&mut frames, "missing").await?;
    assert_eq!(refused["params"]["refusal"]["code"], "invalid_parameter");
    let expected = &refused["params"]["refusal"]["expected"];
    assert_eq!(expected["reason"], "window_not_found");
    assert_eq!(expected["application"], "Dia");
    assert_eq!(expected["title"], "Lighthouse Project in DEV");
    assert_eq!(expected["windows"], 1);
    assert_eq!(expected["matching"], 0);
    assert_eq!(expected["selectable"], 0);
    assert_eq!(expected["omitted"], 0);
    assert_eq!(expected["inventory"][0]["application"], "Safari");
    assert_eq!(expected["inventory"][0]["title"], "Correo");
    Ok(())
}

#[tokio::test]
async fn matching_unselectable_window_is_not_reported_as_missing() -> Result<()> {
    let mut h = Mac::new(false).await?;
    drop(h.runtime.features.computer.service.take());
    let (hub, mut frames) = mpsc::channel(16);
    h.runtime.hub.install(hub);
    h.runtime
        .device_request_raw(
            &json!({"request_id":"unselectable","capability":"selected_text",
        "parameters":{"application":"Dia","title":"Lighthouse Project in DEV"}}),
        )
        .await?;
    let list = h._native.try_recv()?;
    let mut inventory = window_inventory_reply(
        &list,
        h.runtime.state.generation(),
        "Dia",
        "Lighthouse Project in DEV",
    );
    inventory["result"]["windows"][0]["binding"] = Value::Null;
    inventory["result"]["windows"][0]["unavailable_reason"] = json!("window_identity_unavailable");
    h.runtime.native_event(inventory).await?;
    let refused = reply(&mut frames, "unselectable").await?;
    assert_eq!(refused["params"]["refusal"]["code"], "invalid_parameter");
    let expected = &refused["params"]["refusal"]["expected"];
    assert_eq!(expected["reason"], "unavailable");
    assert_eq!(expected["matching"], 1);
    assert_eq!(expected["selectable"], 0);
    assert_eq!(
        expected["inventory"][0]["unavailable_reason"],
        "window_identity_unavailable"
    );
    Ok(())
}

#[tokio::test]
async fn long_selection_filters_still_deliver_a_numbered_refusal() -> Result<()> {
    let mut h = Mac::new(false).await?;
    drop(h.runtime.features.computer.service.take());
    let (hub, mut frames) = mpsc::channel(16);
    h.runtime.hub.install(hub);
    let title = "x".repeat(MAX_OUTGOING_FRAME_BYTES - 180);
    let request = json!({"request_id":"long-filter", "capability":"screenshot", "parameters":{"title": title}});
    assert!(serde_json::to_vec(&request)?.len() < MAX_OUTGOING_FRAME_BYTES);
    h.runtime.device_request_raw(&request).await?;
    let list = h._native.try_recv()?;
    let generation = h.runtime.state.generation();
    h.runtime
        .native_event(window_inventory_reply(
            &list, generation, "Safari", "Correo",
        ))
        .await?;
    let frame = frames
        .try_recv()
        .context("the selection refusal exceeded the outgoing frame")?;
    let bytes = serde_json::to_vec(&frame)?.len();
    println!("selection_refusal bytes={bytes} max_bytes={MAX_OUTGOING_FRAME_BYTES}");
    assert!(
        bytes < MAX_OUTGOING_FRAME_BYTES,
        "refusal has {bytes} bytes; ceiling {MAX_OUTGOING_FRAME_BYTES}"
    );
    let expected = &frame["params"]["refusal"]["expected"];
    assert_eq!(expected["title_bytes"], title.len());
    assert_eq!(expected["filters_truncated"], true);
    assert_eq!(expected["windows"], 1);
    assert_eq!(expected["matching"], 0);
    assert!(serde_json::to_vec(&frame)?.len() < MAX_OUTGOING_FRAME_BYTES);
    Ok(())
}
