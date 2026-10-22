//! «Adjuntar ventana» with the engine running: the capture and, in a call of
//! its own, the window's tree, which goes next to it as a `.txt` without the
//! app's menu bar. The engine and the hub are synthetic; the window and its
//! text are invented.
use super::*;
use crate::daemon::attachments_tests::{Request, http_hub};
use base64::Engine as _;
use orbit_core::attachments::{AttachmentSource, WindowText};

const CAPTURE: &str = "40000000-0000-4000-8000-0000000000b1";
const TEXT: &str = "40000000-0000-4000-8000-0000000000b2";

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

/// Orbit's window in front, as when the user presses the button, and the
/// editor behind it.
fn orbit_in_front() -> Value {
    json!({"windows": [
        {"window_id": 7, "pid": 70, "app_name": "Orbit", "bundle_id": "app.orbit.companion",
         "title": "Orbit", "z_index": 50},
        {"window_id": 22, "pid": 11, "app_name": "Editor", "bundle_id": "com.example.editor",
         "title": "Nota sintética", "z_index": 5},
    ]})
}

async fn attach_with_the_engine(
    tree: &str,
) -> Result<(
    orbit_core::protocol::AttachmentResponse,
    Vec<Request>,
    EngineRecord,
)> {
    let engine = FakeEngine {
        tools: Value::Array(tools_named(&[
            "list_apps",
            "list_windows",
            "get_window_state",
        ])),
        windows: orbit_in_front(),
        png: base64::engine::general_purpose::STANDARD.encode(small_png()?),
        window_dimensions: (2, 1),
        timeout: Duration::from_secs(5),
        tree: Some(tree.to_owned()),
        ..FakeEngine::default()
    };
    let record = engine.record.clone();
    let mut h = Mac::with_engine(engine).await?;
    let (url, recorded) = http_hub(|request| {
        let body = if request.path == "/v1/attachments/capture" {
            json!({"attachment": {"id": CAPTURE, "name": "Editor — Nota sintética.png",
                "kind": "image", "mime": "image/png", "bytes": 70}})
        } else if request.path == "/v1/attachments" {
            json!({"attachment": {"id": TEXT, "name": "Editor — Nota sintética.txt",
                "kind": "text", "mime": "text/plain", "bytes": request.body.len()}})
        } else {
            json!({"capture_id": "stored", "stored": true})
        };
        (
            200,
            "application/json",
            Vec::new(),
            body.to_string().into_bytes(),
        )
    })
    .await?;
    h.runtime.settings.hub_url = Some(url);
    h.runtime.token = Some("synthetic device credential".to_owned());
    h.runtime.conversation.hub_attachments = Some(orbit_core::attachments::AttachmentLimits {
        max_file_bytes: 10_000_000,
        max_message_files: 10,
        max_message_bytes: 10_000_000,
        max_session_bytes: 100_000_000,
    });
    let (hub, _frames) = mpsc::channel(16);
    h.runtime.hub.install(hub);
    let (calls, mut received) = mpsc::channel(4);
    let (_ended, ended) = tokio::sync::watch::channel(None);
    let client = orbit_ipc::LocalClient::new(calls, &std::collections::BTreeMap::new(), ended);
    let asked = tokio::spawn(async move {
        client
            .attachment(orbit_core::protocol::Command::AttachmentAdd {
                source: AttachmentSource::Window,
            })
            .await
    });
    let call = received.recv().await.context("no order")?;
    h.runtime.attach_window(call.request.id, call.reply).await;
    // The listing, the capture with its tree, and the capture's upload.
    h.finish().await?;
    h.finish().await?;
    let uploaded = tokio::time::timeout(Duration::from_secs(10), h.events.recv())
        .await?
        .context("the capture upload never ended")?;
    h.runtime.work_event(uploaded).await?;
    let answered = tokio::time::timeout(Duration::from_secs(10), asked)
        .await
        .context("no answer")???;
    let requests = recorded.lock().map(|seen| seen.clone()).unwrap_or_default();
    Ok((answered, requests, record))
}

#[tokio::test]
async fn the_engine_s_tree_of_the_window_goes_next_to_its_capture() -> Result<()> {
    let tree = "- AXWindow \"Nota sintética\"\n  - AXMenuBar\n    - [1] AXMenuBarItem \"Archivo\"\n      - AXMenuItem \"Abrir reciente\"\n  - AXStaticText = \"Lista de la compra\"\n  - AXTextArea = \"Pan y agua\"\n";
    let (answered, requests, record) = attach_with_the_engine(tree).await?;
    assert!(answered.ok, "{:?}", answered.refusal);
    // One call for the picture, one for the tree, both of the editor.
    let calls = record.calls_of("get_window_state");
    let [picture, text] = calls.as_slice() else {
        bail!("expected two reads of the window, got {calls:?}");
    };
    assert_eq!(picture["arguments"]["window_id"], 22);
    assert_eq!(picture["arguments"]["include_accessibility_tree"], false);
    assert_eq!(text["arguments"]["window_id"], 22);
    assert_eq!(text["arguments"]["include_screenshot"], false);
    assert_eq!(text["arguments"]["include_accessibility_tree"], true);
    let Some(file) = requests
        .iter()
        .find(|request| request.path == "/v1/attachments")
    else {
        bail!("the text was not uploaded: {requests:?}");
    };
    assert_eq!(
        String::from_utf8_lossy(&file.body),
        "- AXWindow \"Nota sintética\"\n  - AXStaticText = \"Lista de la compra\"\n  - AXTextArea = \"Pan y agua\""
    );
    assert!(matches!(
        answered.window_text,
        Some(WindowText::Attached { attachment }) if attachment.id == TEXT
    ));
    Ok(())
}

#[tokio::test]
async fn a_window_the_engine_reads_no_text_from_sends_its_capture_alone() -> Result<()> {
    let (answered, requests, _record) =
        attach_with_the_engine("  - AXMenuBar\n    - AXMenuItem \"Archivo\"\n").await?;
    assert_eq!(
        answered.attachment.map(|attachment| attachment.id),
        Some(CAPTURE.to_owned())
    );
    assert_eq!(
        answered.window_text,
        Some(WindowText::Missing {
            cause: "empty".into()
        })
    );
    assert!(
        requests
            .iter()
            .all(|request| request.path != "/v1/attachments"),
        "{requests:?}"
    );
    Ok(())
}
